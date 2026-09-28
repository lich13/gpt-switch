//! Responses uses per-generation slots, following Sub2API's BeforeTurn/AfterTurn
//! behavior (a3eb7ef3). Payloads are observed, never rewritten or replayed after upgrade.
use super::{
    admission::{Admission, Budget, Rejected},
    circuit, forward,
    model::Settings,
    replay,
    routing::Requirement,
    Active, Gateway, Route,
};
use base64::{engine::general_purpose::STANDARD, Engine};
use futures_util::{SinkExt, StreamExt};
use hyper::{body::Incoming, header, HeaderMap, Request, Response, StatusCode, Uri};
use hyper_util::rt::TokioIo;
use sha1::{Digest, Sha1};
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncRead, AsyncWrite},
    sync::{mpsc, oneshot, watch},
};
use yawc::{Frame, OpCode, Options, WebSocket};
type Failure = (u16, &'static str);
const MAX_MESSAGE: usize = 64 * 1024 * 1024;
fn options() -> Options {
    Options::default()
        .with_balanced_compression()
        .with_limits(MAX_MESSAGE, MAX_MESSAGE)
        .with_fragment_timeout(Duration::from_secs(120))
        .with_utf8()
}
struct Outgoing {
    frame: Frame,
    ack: oneshot::Sender<bool>,
}
struct Peer {
    incoming: mpsc::Receiver<Frame>,
    outgoing: mpsc::Sender<Outgoing>,
    closed: watch::Receiver<bool>,
    tasks: Vec<tokio::task::JoinHandle<()>>,
}
impl Peer {
    fn new<S: AsyncRead + AsyncWrite + Unpin + Send + 'static>(socket: WebSocket<S>) -> Self {
        let (mut sink, mut stream) = socket.split();
        let (tx, incoming) = mpsc::channel(2);
        let (outgoing, mut rx) = mpsc::channel::<Outgoing>(1);
        let (closed_tx, closed) = watch::channel(false);
        let writer_closed = closed_tx.clone();
        let reader = tokio::spawn(async move {
            while let Some(frame) = stream.next().await {
                let close = frame.opcode() == OpCode::Close;
                if tx.send(frame).await.is_err() || close {
                    break;
                }
            }
            let _ = closed_tx.send(true);
        });
        let writer = tokio::spawn(async move {
            while let Some(Outgoing { frame, ack }) = rx.recv().await {
                let ok = matches!(
                    tokio::time::timeout(Duration::from_secs(120), sink.send(frame)).await,
                    Ok(Ok(()))
                );
                let _ = ack.send(ok);
                if !ok {
                    break;
                }
            }
            let _ = writer_closed.send(true);
        });
        Self {
            incoming,
            outgoing,
            closed,
            tasks: vec![reader, writer],
        }
    }
    async fn send(&self, frame: Frame) -> Result<(), Failure> {
        let (ack, rx) = oneshot::channel();
        self.outgoing
            .send(Outgoing { frame, ack })
            .await
            .map_err(|_| (1011, "connection closed"))?;
        if rx.await.unwrap_or(false) {
            Ok(())
        } else {
            Err((1011, "connection write failed"))
        }
    }
    async fn close(&self, code: u16, reason: &str) {
        let _ = tokio::time::timeout(
            Duration::from_secs(2),
            self.send(Frame::close(code.into(), reason)),
        )
        .await;
    }
}
impl Drop for Peer {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
    }
}
fn value(frame: &Frame) -> Option<serde_json::Value> {
    if matches!(frame.opcode(), OpCode::Text | OpCode::Binary) {
        serde_json::from_slice(frame.payload()).ok()
    } else {
        None
    }
}
fn creates(frame: &Frame) -> bool {
    value(frame).is_some_and(|v| v["type"] == "response.create")
}
fn turn_failed(value: &serde_json::Value) -> bool {
    let error = value
        .get("error")
        .or_else(|| value.pointer("/response/error"));
    if let Some(status) = error
        .and_then(|e| e.get("status").or_else(|| e.get("status_code")))
        .and_then(|v| v.as_u64())
    {
        return circuit::retryable(status as u16);
    }
    let code = error
        .and_then(|e| e.get("type").or_else(|| e.get("code")))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    !matches!(
        code,
        "invalid_request_error"
            | "invalid_request"
            | "context_length_exceeded"
            | "response_not_found"
    )
}
fn rejected(reason: Rejected) -> Failure {
    match reason {
        Rejected::Model => (1008, "MODEL_NOT_ALLOWED or MODEL_UNDETERMINED"),
        Rejected::Stopped => (1012, "gateway stopped"),
        Rejected::Unavailable => (1013, "no available provider"),
        Rejected::Full | Rejected::Timeout => (1013, "provider concurrency full; retry later"),
    }
}
pub(super) fn accept(
    gateway: Gateway,
    mut request: Request<Incoming>,
    settings: Settings,
    manual: bool,
    ids: Vec<String>,
    routes: HashMap<String, Route>,
    active: Active,
) -> Response<replay::WireBody> {
    if request.method() != hyper::Method::GET {
        return forward::error(
            StatusCode::BAD_REQUEST,
            "WEBSOCKET",
            "WebSocket 握手必须使用 GET",
        );
    }
    let headers = request.headers().clone();
    let uri = request.uri().clone();
    let upgraded = WebSocket::upgrade_with_options(&mut request, options());
    let Ok((response, future)) = upgraded else {
        return forward::error(StatusCode::BAD_REQUEST, "WEBSOCKET", "WebSocket 握手无效");
    };
    let stop = gateway
        .0
        .inner
        .lock()
        .unwrap()
        .shutdown
        .as_ref()
        .map(|s| s.subscribe());
    tokio::spawn(async move {
        let _active = active;
        let Ok(socket) = future.await else {
            return;
        };
        let mut client = Peer::new(socket);
        let Some(mut stop) = stop else {
            return;
        };
        let result = tokio::select! {
            result = session(&gateway, &mut client, &settings, manual, ids, routes, headers, uri) => result,
            _ = stop.changed() => Err((1012, "gateway stopped")),
        };
        if let Err((code, reason)) = result {
            client.close(code, reason).await;
        }
    });
    response.map(|_| replay::empty())
}
async fn take_slot(
    g: &Gateway,
    client: &mut Peer,
    routes: &[Route],
    manual: bool,
    settings: &Settings,
    budget: &mut Budget,
    requirement: &Requirement,
) -> Result<Admission, Failure> {
    if *client.closed.borrow() {
        return Err((1000, "client closed"));
    }
    tokio::select! {
        admission = g.0.admission.acquire_for(routes, manual, settings.max_waiting, budget, requirement) => admission.map_err(|reason| {
            if matches!(reason, Rejected::Model) { (1008, requirement.code()) } else { rejected(reason) }
        }),
        _ = client.closed.changed() => Err((1000, "client closed")),
    }
}
async fn upstream(
    route: &Route,
    uri: &Uri,
    original: &HeaderMap,
    settings: &Settings,
) -> Result<Peer, (Option<u16>, bool, Option<Duration>)> {
    let mut request = Request::new(replay::empty());
    *request.method_mut() = hyper::Method::GET;
    *request.uri_mut() =
        forward::target(&route.provider.base_url, uri).map_err(|_| (None, false, None))?;
    *request.headers_mut() = original.clone();
    let headers = request.headers_mut();
    forward::clean_headers(headers, true);
    headers.remove(header::HOST);
    headers.remove(header::CONTENT_LENGTH);
    headers.insert(
        header::AUTHORIZATION,
        format!("Bearer {}", route.provider.token)
            .parse()
            .map_err(|_| (None, false, None))?,
    );
    let key = STANDARD.encode(uuid::Uuid::new_v4().as_bytes());
    headers.insert(header::SEC_WEBSOCKET_KEY, key.parse().unwrap());
    headers.insert(
        header::SEC_WEBSOCKET_EXTENSIONS,
        "permessage-deflate; client_max_window_bits"
            .parse()
            .unwrap(),
    );
    let mut response = match tokio::time::timeout(
        Duration::from_secs(settings.first_byte_seconds),
        route.client.request(request),
    )
    .await
    {
        Ok(Ok(response)) => response,
        Ok(Err(e)) => {
            return Err((
                None,
                super::connector::classify(&e).is_some_and(|e| e.is_proxy()),
                None,
            ))
        }
        Err(_) => return Err((None, false, None)),
    };
    if response.status() != StatusCode::SWITCHING_PROTOCOLS {
        let retry = response
            .headers()
            .get(header::RETRY_AFTER)
            .and_then(|v| v.to_str().ok())
            .and_then(circuit::retry_after);
        return Err((Some(response.status().as_u16()), false, retry));
    }
    let expected = STANDARD.encode(Sha1::digest(
        format!("{key}258EAFA5-E914-47DA-95CA-C5AB0DC85B11").as_bytes(),
    ));
    if response
        .headers()
        .get(header::SEC_WEBSOCKET_ACCEPT)
        .and_then(|v| v.to_str().ok())
        != Some(&expected)
    {
        return Err((Some(502), false, None));
    }
    let extensions = response
        .headers()
        .get(header::SEC_WEBSOCKET_EXTENSIONS)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let io = hyper::upgrade::on(&mut response)
        .await
        .map_err(|_| (None, false, None))?;
    let socket = WebSocket::from_stream_with_extensions(
        TokioIo::new(io),
        yawc::Role::Client,
        extensions.as_deref(),
        options(),
    )
    .map_err(|_| (None, false, None))?;
    Ok(Peer::new(socket))
}
fn turn_model(
    g: &Gateway,
    frame: &Frame,
    last: Option<&str>,
    pinned: Option<&str>,
) -> Result<Option<String>, Failure> {
    let v = value(frame).ok_or((1008, "invalid response.create"))?;
    let previous = v.get("previous_response_id").and_then(|v| v.as_str());
    let remembered = previous.and_then(|id| {
        g.0.inner
            .lock()
            .unwrap()
            .affinity
            .get(id)
            .filter(|(_, _, at)| at.elapsed() < Duration::from_secs(3600))
            .cloned()
    });
    if let Some((owner, _, _)) = &remembered {
        if pinned.is_some_and(|p| p != owner) {
            return Err((1008, "response context belongs to another provider"));
        }
    }
    if let Some(model) = v.get("model") {
        return Ok(model
            .as_str()
            .filter(|m| !m.is_empty() && m.len() <= 256 && !m.chars().any(char::is_control))
            .map(str::to_owned));
    }
    Ok(remembered
        .and_then(|(_, model, _)| model)
        .or_else(|| last.map(str::to_owned)))
}
#[allow(clippy::too_many_arguments)]
async fn session(
    g: &Gateway,
    client: &mut Peer,
    cfg: &Settings,
    manual: bool,
    mut ids: Vec<String>,
    routes: HashMap<String, Route>,
    headers: HeaderMap,
    uri: Uri,
) -> Result<(), Failure> {
    let first_deadline = tokio::time::Instant::now() + Duration::from_secs(cfg.first_byte_seconds);
    let first = loop {
        let frame = tokio::time::timeout_at(first_deadline, client.incoming.recv())
            .await
            .map_err(|_| (1008, "missing first response.create"))?
            .ok_or((1000, "client closed"))?;
        if frame.opcode() == OpCode::Close {
            return Ok(());
        }
        if matches!(frame.opcode(), OpCode::Ping | OpCode::Pong) {
            continue;
        }
        if !creates(&frame) {
            return Err((1008, "expected response.create"));
        }
        break frame;
    };
    if let Some(previous) = value(&first).and_then(|v| {
        v.get("previous_response_id")
            .and_then(|v| v.as_str())
            .map(str::to_owned)
    }) {
        let s = g.0.inner.lock().unwrap();
        if let Some((owner, _, _)) = s
            .affinity
            .get(&previous)
            .filter(|(_, _, at)| at.elapsed() < Duration::from_secs(3600))
        {
            ids = vec![owner.clone()];
        } else {
            ids.truncate(1);
        }
    }
    let mut current_model = turn_model(g, &first, None, None)?;
    let requirement = Requirement::model(current_model.as_deref());
    let mut budget = Budget::new(cfg.queue_seconds);
    let mut attempts = 0;
    let (mut upstream_peer, admission) = loop {
        if ids.is_empty() || attempts > cfg.max_retries {
            return Err((1013, "all providers failed"));
        }
        let candidates: Vec<_> = ids
            .iter()
            .filter_map(|id| routes.get(id).cloned())
            .collect();
        let mut admission = take_slot(
            g,
            client,
            &candidates,
            manual,
            cfg,
            &mut budget,
            &requirement,
        )
        .await?;
        ids.retain(|id| id != &admission.route.provider.id);
        attempts += 1;
        let began = Instant::now();
        let mut closed = client.closed.clone();
        let result = tokio::select! {
            result = upstream(&admission.route, &uri, &headers, cfg) => result,
            _ = closed.changed() => return Ok(()),
        };
        match result {
            Ok(peer) => {
                admission.permits.proxy_success(cfg);
                break (peer, admission);
            }
            Err((status, proxy, retry)) => {
                let retryable = status.is_none_or(circuit::retryable);
                if retryable {
                    admission.permits.failure(cfg, proxy, retry);
                } else {
                    admission.permits.neutral(cfg);
                }
                g.record(
                    &admission.route,
                    status,
                    began,
                    attempts - 1,
                    if proxy { "PROXY" } else { "WS_HANDSHAKE" },
                );
                if !retryable {
                    return Err((1008, "upstream rejected websocket handshake"));
                }
            }
        }
    };
    let route = admission.route.clone();
    let mut turn = Some(admission);
    let mut pending: Option<Frame> = None;
    let mut confirmed_model: Option<String> = None;
    let mut began = Instant::now();
    let mut received = false;
    let mut deadline = tokio::time::Instant::now() + Duration::from_secs(cfg.first_byte_seconds);
    if let Err(e) = upstream_peer.send(first).await {
        if let Some(mut admission) = turn.take() {
            admission.permits.failure(cfg, false, None);
        }
        return Err(e);
    }
    loop {
        if turn.is_none() {
            if let Some(frame) = pending.take() {
                let model = turn_model(
                    g,
                    &frame,
                    confirmed_model.as_deref(),
                    Some(&route.provider.id),
                )?;
                let requirement = Requirement::model(model.as_deref());
                let mut budget = Budget::new(cfg.queue_seconds);
                turn = Some(
                    take_slot(
                        g,
                        client,
                        std::slice::from_ref(&route),
                        manual,
                        cfg,
                        &mut budget,
                        &requirement,
                    )
                    .await?,
                );
                current_model = model;
                began = Instant::now();
                received = false;
                deadline =
                    tokio::time::Instant::now() + Duration::from_secs(cfg.first_byte_seconds);
                if let Err(e) = upstream_peer.send(frame).await {
                    if let Some(mut admission) = turn.take() {
                        admission.permits.failure(cfg, false, None);
                    }
                    return Err(e);
                }
            }
        }
        tokio::select! {
            biased;
            frame = upstream_peer.incoming.recv() => {
                let Some(frame) = frame else {
                    if let Some(mut admission) = turn.take() { admission.permits.failure(cfg, false, None); g.record(&route, Some(101), began, 0, "STREAM_INTERRUPTED"); }
                    return Err((1011, "upstream disconnected"));
                };
                let closing = frame.opcode() == OpCode::Close;
                if closing {
                    if let Some(mut admission) = turn.take() { admission.permits.failure(cfg, false, None); g.record(&route, Some(101), began, 0, "STREAM_INTERRUPTED"); }
                }
                if !frame.opcode().is_control() {
                    received = true;
                    deadline = tokio::time::Instant::now() + Duration::from_secs(cfg.idle_seconds);
                }
                if let Some(v) = value(&frame) {
                    if let Some(id) = v.pointer("/response/id").or_else(|| v.get("response_id")).and_then(|v| v.as_str()) { g.remember_model(id, &route.provider.id, current_model.as_deref()); }
                    let kind = v.get("type").and_then(|v| v.as_str()).unwrap_or("");
                    if matches!(kind, "response.created" | "response.completed" | "response.done") { confirmed_model = current_model.clone(); }
                    if matches!(kind, "response.completed" | "response.done" | "response.failed" | "response.incomplete" | "response.cancelled" | "response.canceled" | "error") {
                        if let Some(mut admission) = turn.take() {
                            if matches!(kind, "response.failed" | "error") {
                                if turn_failed(&v) { admission.permits.failure(cfg, false, None); } else { admission.permits.neutral(cfg); }
                            } else if matches!(kind, "response.cancelled" | "response.canceled") {
                                admission.permits.neutral(cfg);
                            } else { admission.permits.success(cfg); g.successful_response(&route.provider); }
                            g.record(&route, Some(101), began, 0, "WEBSOCKET_TURN");
                        }
                    }
                }
                client.send(frame).await?;
                if closing { return Ok(()); }
            },
            frame = client.incoming.recv() => {
                let Some(frame) = frame else { return Ok(()); };
                let closing = frame.opcode() == OpCode::Close;
                if creates(&frame) {
                    if pending.is_some() { return Err((1013, "too many pending turns")); }
                    pending = Some(frame);
                } else { upstream_peer.send(frame).await?; }
                if closing { return Ok(()); }
            },
            _ = tokio::time::sleep_until(deadline), if turn.is_some() => {
                if let Some(mut admission) = turn.take() { admission.permits.failure(cfg, false, None); }
                g.record(&route, Some(101), began, 0, if received { "STREAM_TIMEOUT" } else { "FIRST_BYTE_TIMEOUT" });
                upstream_peer.close(1011, "upstream timeout").await;
                return Err((1011, "upstream timeout"));
            },
        }
    }
}
