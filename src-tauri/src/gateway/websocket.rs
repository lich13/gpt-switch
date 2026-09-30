//! Responses uses per-generation slots, following Sub2API's BeforeTurn/AfterTurn
//! behavior (a3eb7ef3). Payloads are observed, never rewritten or replayed after upgrade.
use super::{
    admission::{Admission, Budget, CapacitySource, Rejected},
    circuit, forward,
    model::Settings,
    replay,
    routing::Requirement,
    Active, Gateway, Route,
};
use async_compression::tokio::bufread::{DeflateDecoder, GzipDecoder, ZstdDecoder};
use base64::{engine::general_purpose::STANDARD, Engine};
use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use http_body_util::BodyExt;
use hyper::{body::Incoming, header, HeaderMap, Request, Response, StatusCode, Uri};
use hyper_util::rt::TokioIo;
use sha1::{Digest, Sha1};
use std::{collections::HashMap, io, time::Duration};
use tokio::{
    io::{AsyncBufRead, AsyncBufReadExt, AsyncRead, AsyncWrite, BufReader},
    sync::{mpsc, oneshot, watch},
};
use tokio_util::io::StreamReader;
use yawc::{Frame, OpCode, Options, WebSocket};
type Failure = (u16, &'static str);
type BoxReader = Box<dyn AsyncBufRead + Send + Unpin>;
const MAX_MESSAGE: usize = 64 * 1024 * 1024;
const MAX_SSE_EVENT: usize = 2 * 1024 * 1024;
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
enum Upstream {
    Native(Peer),
    Bridge(Option<BridgeTurn>),
}
struct BridgeTurn {
    first: Option<Result<Frame, Failure>>,
    incoming: mpsc::Receiver<Result<Frame, Failure>>,
    cancel: watch::Sender<bool>,
}
impl BridgeTurn {
    fn cancel(&self) {
        let _ = self.cancel.send(true);
    }
}
impl Drop for BridgeTurn {
    fn drop(&mut self) {
        let _ = self.cancel.send(true);
    }
}
async fn next_upstream(upstream: &mut Upstream) -> Option<Result<Frame, Failure>> {
    match upstream {
        Upstream::Native(peer) => peer.incoming.recv().await.map(Ok),
        Upstream::Bridge(Some(turn)) => {
            if let Some(first) = turn.first.take() {
                Some(first)
            } else {
                turn.incoming.recv().await
            }
        }
        Upstream::Bridge(None) => None,
    }
}
async fn send_to_upstream(upstream: &mut Upstream, frame: Frame) -> Result<(), Failure> {
    match upstream {
        Upstream::Native(peer) => peer.send(frame).await,
        Upstream::Bridge(Some(turn)) => {
            let Some(value) = value(&frame) else {
                return Err((1008, "bridge only accepts JSON response events"));
            };
            let kind = value.get("type").and_then(|v| v.as_str()).unwrap_or("");
            if kind == "response.cancel" {
                turn.cancel();
                return Ok(());
            }
            if kind == "response.create" {
                return Err((1013, "too many pending turns"));
            }
            Err((1008, "event is not supported by HTTP bridge"))
        }
        Upstream::Bridge(None) => Err((1011, "bridge turn is not active")),
    }
}
fn unsupported_status(status: u16) -> bool {
    matches!(status, 400 | 404 | 405 | 406 | 426 | 501)
}
fn uses_native_websocket(client: super::ClientId, supports_websocket: bool) -> bool {
    client != super::ClientId::Codex || supports_websocket
}
#[derive(Clone, Copy, Debug)]
struct AttemptFailure {
    status: Option<u16>,
    retry: Option<Duration>,
    capacity: bool,
    unsupported: bool,
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
fn rejected(reason: Rejected) -> Failure {
    match reason {
        Rejected::Model => (1008, "MODEL_NOT_ALLOWED or MODEL_UNDETERMINED"),
        Rejected::Stopped => (1012, "gateway stopped"),
        Rejected::Unavailable => (1013, "no available provider"),
        Rejected::Cooling(_) => (1013, "providers cooling down; retry later"),
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
#[allow(clippy::too_many_arguments)]
async fn take_slot(
    g: &Gateway,
    client: &mut Peer,
    routes: &[Route],
    manual: bool,
    settings: &Settings,
    budget: &mut Budget,
    requirement: &Requirement,
    immediate: bool,
) -> Result<Admission, Failure> {
    if *client.closed.borrow() {
        return Err((1000, "client closed"));
    }
    tokio::select! {
        admission = g.0.admission.acquire_for_immediate(routes, manual, settings.max_waiting, budget, requirement, immediate) => admission.map_err(|reason| {
            if matches!(reason, Rejected::Model) { (1008, requirement.code()) } else { rejected(reason) }
        }),
        _ = client.closed.changed() => Err((1000, "client closed")),
    }
}
async fn upstream_native(
    route: &Route,
    uri: &Uri,
    original: &HeaderMap,
    settings: &Settings,
) -> Result<Peer, AttemptFailure> {
    let mut request = Request::new(replay::empty());
    *request.method_mut() = hyper::Method::GET;
    *request.uri_mut() = forward::target_for(route.client_id, &route.provider.base_url, uri)
        .map_err(|_| AttemptFailure {
            status: None,
            retry: None,
            capacity: false,
            unsupported: false,
        })?;
    *request.headers_mut() = original.clone();
    let headers = request.headers_mut();
    forward::clean_headers(headers, true);
    headers.remove(header::HOST);
    headers.remove(header::CONTENT_LENGTH);
    headers.remove("x-api-key");
    headers.insert(
        header::AUTHORIZATION,
        format!("Bearer {}", route.provider.token)
            .parse()
            .map_err(|_| AttemptFailure {
                status: None,
                retry: None,
                capacity: false,
                unsupported: false,
            })?,
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
        Ok(Err(_)) => {
            return Err(AttemptFailure {
                status: None,
                retry: None,
                capacity: false,
                unsupported: false,
            })
        }
        Err(_) => {
            return Err(AttemptFailure {
                status: None,
                retry: None,
                capacity: false,
                unsupported: false,
            })
        }
    };
    if response.status() != StatusCode::SWITCHING_PROTOCOLS {
        let status = response.status();
        let retry = response
            .headers()
            .get(header::RETRY_AFTER)
            .and_then(|v| v.to_str().ok())
            .and_then(circuit::retry_after);
        let capacity = if route.client_id == super::ClientId::Codex {
            if status == StatusCode::TOO_MANY_REQUESTS {
                true
            } else {
                let encoding = response
                    .headers()
                    .get(header::CONTENT_ENCODING)
                    .and_then(|h| h.to_str().ok())
                    .unwrap_or("identity")
                    .to_owned();
                let mut body = response.into_body();
                let prefix =
                    tokio::time::timeout(Duration::from_secs(settings.first_byte_seconds), async {
                        let mut bytes = Vec::new();
                        while bytes.len() < 128 * 1024 {
                            let Some(Ok(frame)) = body.frame().await else {
                                break;
                            };
                            if let Ok(data) = frame.into_data() {
                                let count = data.len().min(128 * 1024 - bytes.len());
                                bytes.extend_from_slice(&data[..count]);
                            }
                        }
                        bytes
                    })
                    .await
                    .unwrap_or_default();
                let decoded =
                    replay::decode_prefix(&prefix, &encoding, 128 * 1024).unwrap_or_default();
                forward::capacity_message(status, &decoded)
            }
        } else {
            false
        };
        return Err(AttemptFailure {
            status: Some(status.as_u16()),
            retry,
            capacity,
            unsupported: unsupported_status(status.as_u16()),
        });
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
        return Err(AttemptFailure {
            status: Some(502),
            retry: None,
            capacity: false,
            unsupported: false,
        });
    }
    let extensions = response
        .headers()
        .get(header::SEC_WEBSOCKET_EXTENSIONS)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let io = hyper::upgrade::on(&mut response)
        .await
        .map_err(|_| AttemptFailure {
            status: None,
            retry: None,
            capacity: false,
            unsupported: false,
        })?;
    let socket = WebSocket::from_stream_with_extensions(
        TokioIo::new(io),
        yawc::Role::Client,
        extensions.as_deref(),
        options(),
    )
    .map_err(|_| AttemptFailure {
        status: None,
        retry: None,
        capacity: false,
        unsupported: false,
    })?;
    Ok(Peer::new(socket))
}
fn bridge_payload(frame: &Frame) -> Result<Bytes, Failure> {
    let mut value = value(frame).ok_or((1008, "invalid response.create"))?;
    let object = value
        .as_object_mut()
        .ok_or((1008, "response.create must be a JSON object"))?;
    if object.get("type").and_then(|v| v.as_str()) != Some("response.create") {
        return Err((1008, "expected response.create"));
    }
    object.remove("type");
    object.insert("stream".into(), serde_json::Value::Bool(true));
    serde_json::to_vec(&value)
        .map(Bytes::from)
        .map_err(|_| (1011, "cannot encode bridge request"))
}
async fn response_prefix(mut body: Incoming, limit: usize) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(limit.min(4096));
    while bytes.len() < limit {
        let Some(Ok(frame)) = body.frame().await else {
            break;
        };
        if let Ok(data) = frame.into_data() {
            let count = data.len().min(limit - bytes.len());
            bytes.extend_from_slice(&data[..count]);
        }
    }
    bytes
}
fn decoded_reader(body: Incoming, encoding: &str) -> Result<BoxReader, AttemptFailure> {
    let stream = body
        .into_data_stream()
        .map(|result| result.map_err(io::Error::other));
    let reader = BufReader::new(StreamReader::new(stream));
    let reader: BoxReader = match encoding.trim().to_ascii_lowercase().as_str() {
        "" | "identity" => Box::new(reader),
        "gzip" => Box::new(BufReader::new(GzipDecoder::new(reader))),
        "deflate" => Box::new(BufReader::new(DeflateDecoder::new(reader))),
        "zstd" => Box::new(BufReader::new(ZstdDecoder::new(reader))),
        _ => {
            return Err(AttemptFailure {
                status: None,
                retry: None,
                capacity: false,
                unsupported: true,
            })
        }
    };
    Ok(reader)
}
async fn send_bridge_event(
    data: &[u8],
    sender: &mpsc::Sender<Result<Frame, Failure>>,
) -> Result<bool, Failure> {
    if data == b"[DONE]" {
        return sender
            .send(Ok(Frame::text(r#"{"type":"response.done"}"#.to_owned())))
            .await
            .map(|_| true)
            .map_err(|_| (1000, "bridge client closed"));
    }
    if serde_json::from_slice::<serde_json::Value>(data).is_err() {
        return Err((1011, "bridge returned invalid SSE JSON"));
    }
    let payload = std::str::from_utf8(data)
        .map(str::to_owned)
        .map_err(|_| (1011, "bridge returned non-UTF8 SSE JSON"))?;
    sender
        .send(Ok(Frame::text(payload)))
        .await
        .map(|_| false)
        .map_err(|_| (1000, "bridge client closed"))
}
async fn bridge_events(
    mut reader: BoxReader,
    sender: mpsc::Sender<Result<Frame, Failure>>,
    mut cancel: watch::Receiver<bool>,
) {
    let mut line = Vec::with_capacity(256);
    let mut data = Vec::new();
    loop {
        line.clear();
        let read = tokio::select! {
            _ = cancel.changed() => return,
            result = reader.read_until(b'\n', &mut line) => result,
        };
        let Ok(size) = read else {
            let _ = sender
                .send(Err((1011, "bridge response read failed")))
                .await;
            return;
        };
        if size == 0 {
            if !data.is_empty() {
                if let Err(error) = send_bridge_event(&data, &sender).await {
                    let _ = sender.send(Err(error)).await;
                }
            }
            return;
        }
        if data.len() + line.len() > MAX_SSE_EVENT {
            let _ = sender.send(Err((1009, "bridge event too large"))).await;
            return;
        }
        let trimmed = line.strip_suffix(b"\n").unwrap_or(&line);
        let trimmed = trimmed.strip_suffix(b"\r").unwrap_or(trimmed);
        if trimmed.is_empty() {
            if !data.is_empty() {
                match send_bridge_event(&data, &sender).await {
                    Ok(done) => {
                        data.clear();
                        if done {
                            return;
                        }
                    }
                    Err(error) => {
                        let _ = sender.send(Err(error)).await;
                        return;
                    }
                }
            }
            continue;
        }
        if let Some(value) = trimmed.strip_prefix(b"data:") {
            let value = value.strip_prefix(b" ").unwrap_or(value);
            if !data.is_empty() {
                data.push(b'\n');
            }
            data.extend_from_slice(value);
        }
    }
}
async fn upstream_bridge(
    route: &Route,
    uri: &Uri,
    original: &HeaderMap,
    settings: &Settings,
    first: &Frame,
) -> Result<BridgeTurn, AttemptFailure> {
    let payload = bridge_payload(first).map_err(|(status, _)| AttemptFailure {
        status: Some(status),
        retry: None,
        capacity: false,
        unsupported: false,
    })?;
    let mut request = Request::new(replay::full(payload.clone()));
    *request.method_mut() = hyper::Method::POST;
    *request.uri_mut() = forward::target_for(route.client_id, &route.provider.base_url, uri)
        .map_err(|_| AttemptFailure {
            status: None,
            retry: None,
            capacity: false,
            unsupported: false,
        })?;
    *request.headers_mut() = original.clone();
    let headers = request.headers_mut();
    forward::clean_headers(headers, false);
    headers.remove(header::HOST);
    headers.remove(header::CONTENT_LENGTH);
    headers.remove(header::UPGRADE);
    headers.remove("x-api-key");
    for name in [
        "sec-websocket-key",
        "sec-websocket-version",
        "sec-websocket-extensions",
        "sec-websocket-protocol",
    ] {
        headers.remove(name);
    }
    headers.insert(
        header::AUTHORIZATION,
        format!("Bearer {}", route.provider.token)
            .parse()
            .map_err(|_| AttemptFailure {
                status: None,
                retry: None,
                capacity: false,
                unsupported: false,
            })?,
    );
    headers.insert(
        header::CONTENT_TYPE,
        header::HeaderValue::from_static("application/json"),
    );
    headers.insert(
        header::ACCEPT,
        header::HeaderValue::from_static("text/event-stream"),
    );
    headers.insert(
        header::CONTENT_LENGTH,
        header::HeaderValue::from(payload.len() as u64),
    );
    let response = match tokio::time::timeout(
        Duration::from_secs(settings.first_byte_seconds),
        route.client.request(request),
    )
    .await
    {
        Ok(Ok(response)) => response,
        Ok(Err(_error)) => {
            return Err(AttemptFailure {
                status: None,
                retry: None,
                capacity: false,
                unsupported: false,
            });
        }
        Err(_) => {
            return Err(AttemptFailure {
                status: None,
                retry: None,
                capacity: false,
                unsupported: false,
            })
        }
    };
    let status = response.status();
    let retry = response
        .headers()
        .get(header::RETRY_AFTER)
        .and_then(|v| v.to_str().ok())
        .and_then(circuit::retry_after);
    if !status.is_success() {
        let encoding = response
            .headers()
            .get(header::CONTENT_ENCODING)
            .and_then(|h| h.to_str().ok())
            .unwrap_or("identity")
            .to_owned();
        let prefix = response_prefix(response.into_body(), 128 * 1024).await;
        let decoded = replay::decode_prefix(&prefix, &encoding, 128 * 1024).unwrap_or_default();
        return Err(AttemptFailure {
            status: Some(status.as_u16()),
            retry,
            capacity: route.client_id == super::ClientId::Codex
                && forward::capacity_message(status, &decoded),
            unsupported: unsupported_status(status.as_u16()),
        });
    }
    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("");
    if !content_type
        .split(';')
        .next()
        .is_some_and(|value| value.trim().eq_ignore_ascii_case("text/event-stream"))
    {
        return Err(AttemptFailure {
            status: Some(status.as_u16()),
            retry,
            capacity: false,
            unsupported: true,
        });
    }
    let encoding = response
        .headers()
        .get(header::CONTENT_ENCODING)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("identity")
        .to_owned();
    let reader = decoded_reader(response.into_body(), &encoding)?;
    let (sender, mut incoming) = mpsc::channel(2);
    let (cancel, cancel_rx) = watch::channel(false);
    tokio::spawn(bridge_events(reader, sender, cancel_rx));
    let first = match tokio::time::timeout(
        Duration::from_secs(settings.first_byte_seconds),
        incoming.recv(),
    )
    .await
    {
        Ok(Some(Ok(frame))) => Ok(frame),
        Ok(Some(Err(_error))) => {
            let _ = cancel.send(true);
            return Err(AttemptFailure {
                status: None,
                retry: None,
                capacity: false,
                unsupported: false,
            });
        }
        Ok(None) | Err(_) => {
            let _ = cancel.send(true);
            return Err(AttemptFailure {
                status: None,
                retry: None,
                capacity: false,
                unsupported: false,
            });
        }
    }?;
    Ok(BridgeTurn {
        first: Some(Ok(first)),
        incoming,
        cancel,
    })
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
    mut routes: HashMap<String, Route>,
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
    let mut pinned = manual.then(|| ids.first().cloned()).flatten();
    let mut unknown_affinity = false;
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
            pinned = Some(owner.clone());
            ids = vec![owner.clone()];
        } else {
            ids.truncate(1);
            unknown_affinity = true;
        }
    }
    let mut current_model = turn_model(g, &first, None, None)?;
    let requirement = Requirement::model(current_model.as_deref());
    let mut budget = Budget::new(cfg.queue_seconds);
    let mut attempts = 0;
    let mut capacity_pending = false;
    let mut capacity_retry_after: Option<Duration> = None;
    let mut capacity_sources = Vec::new();
    let mut unsupported_seen = false;
    let mut non_unsupported_failure = false;
    let (mut upstream, admission, initial_protocol) = loop {
        if attempts > cfg.max_retries || (unknown_affinity && attempts > 0) {
            return Err(if unsupported_seen && !non_unsupported_failure {
                (1008, "WS_UNSUPPORTED")
            } else {
                (1013, "all providers failed")
            });
        }
        if ids.is_empty() {
            if !capacity_pending {
                return Err(if unsupported_seen && !non_unsupported_failure {
                    (1008, "WS_UNSUPPORTED")
                } else {
                    (1013, "all providers failed")
                });
            }
            if *client.closed.borrow() {
                return Ok(());
            }
            tokio::select! {
                result = g.0.admission.wait_capacity(&capacity_sources, forward::capacity_delay(cfg, capacity_retry_after), cfg.max_waiting) => result.map_err(rejected)?,
                _ = client.closed.changed() => return Ok(()),
            }
            ids = g.routing_ids(pinned.as_deref());
            routes = ids
                .iter()
                .filter_map(|id| g.route(id).map(|r| (id.clone(), r)))
                .collect();
            capacity_pending = false;
            capacity_retry_after = None;
            capacity_sources.clear();
            continue;
        }
        let candidates: Vec<_> = ids
            .iter()
            .filter_map(|id| routes.get(id).cloned())
            .collect();
        let mut admission = match take_slot(
            g,
            client,
            &candidates,
            manual,
            cfg,
            &mut budget,
            &requirement,
            capacity_pending,
        )
        .await
        {
            Ok(admission) => admission,
            Err((code, reason))
                if capacity_pending
                    && (code == 1013 || (code == 1008 && reason == requirement.code())) =>
            {
                ids.clear();
                continue;
            }
            Err(error) => return Err(error),
        };
        ids.retain(|id| id != &admission.route.provider.id);
        attempts += 1;
        let mut closed = client.closed.clone();
        let result = tokio::select! {
            result = async { upstream_native(&admission.route, &uri, &headers, cfg).await.map(Upstream::Native) }, if uses_native_websocket(admission.route.client_id, admission.route.provider.supports_websocket) => result,
            result = async { upstream_bridge(&admission.route, &uri, &headers, cfg, &first).await.map(|turn| Upstream::Bridge(Some(turn))) }, if !uses_native_websocket(admission.route.client_id, admission.route.provider.supports_websocket) => result,
            _ = closed.changed() => return Ok(()),
        };
        match result {
            Ok(upstream) => {
                break (upstream, admission, super::protocol::Protocol::new(true));
            }
            Err(failure) => {
                if failure.unsupported {
                    unsupported_seen = true;
                    admission.permits.neutral(cfg);
                    continue;
                }
                non_unsupported_failure = true;
                let retryable = failure.status.is_none_or(circuit::retryable);
                if failure.capacity {
                    admission.permits.capacity_limited(cfg, failure.retry);
                } else if failure.status == Some(429) {
                    admission.permits.rate_limited(cfg, failure.retry);
                } else if retryable {
                    admission.permits.failure(cfg, failure.retry);
                } else {
                    admission.permits.neutral(cfg);
                }

                if !retryable {
                    return Err((1008, "upstream rejected websocket handshake"));
                }
                if failure.capacity {
                    capacity_pending = true;
                    capacity_sources.push(CapacitySource {
                        provider_id: admission.route.provider.id.clone(),
                        reset_generation: admission.reset_generation,
                    });
                    capacity_retry_after = capacity_retry_after.max(failure.retry);
                }
            }
        }
    };
    let route = admission.route.clone();
    let mut turn = Some(admission);
    let mut protocol = Some(initial_protocol);
    let mut pending: Option<Frame> = None;
    let mut confirmed_model: Option<String> = None;
    let mut received = false;
    let mut deadline = tokio::time::Instant::now() + Duration::from_secs(cfg.first_byte_seconds);
    if let Upstream::Native(peer) = &mut upstream {
        if let Err(e) = peer.send(first).await {
            if let Some(u) = &mut protocol {
                u.finish(Some(101), "NETWORK");
            }
            if let Some(mut admission) = turn.take() {
                admission.permits.failure(cfg, None);
            }
            return Err(e);
        }
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
                        false,
                    )
                    .await?,
                );
                current_model = model;
                protocol = Some(super::protocol::Protocol::new(true));
                received = false;
                deadline =
                    tokio::time::Instant::now() + Duration::from_secs(cfg.first_byte_seconds);
                if matches!(&upstream, Upstream::Bridge(_))
                    && route.client_id == super::ClientId::Codex
                {
                    match upstream_bridge(&route, &uri, &headers, cfg, &frame).await {
                        Ok(bridge) => upstream = Upstream::Bridge(Some(bridge)),
                        Err(failure) => {
                            if let Some(u) = &mut protocol {
                                u.finish(failure.status, "HTTP");
                            }
                            if let Some(mut admission) = turn.take() {
                                if failure.capacity {
                                    admission.permits.capacity_limited(cfg, failure.retry);
                                } else if failure.status == Some(429) {
                                    admission.permits.rate_limited(cfg, failure.retry);
                                } else if failure.unsupported {
                                    admission.permits.neutral(cfg);
                                } else {
                                    admission.permits.failure(cfg, failure.retry);
                                }
                            }
                            return Err(if failure.unsupported {
                                (1008, "WS_BRIDGE_UNSUPPORTED")
                            } else {
                                (1013, "bridge request failed")
                            });
                        }
                    }
                } else if let Err(e) = send_to_upstream(&mut upstream, frame).await {
                    if let Some(u) = protocol.as_mut() {
                        u.finish(Some(101), "NETWORK");
                    }
                    if let Some(mut admission) = turn.take() {
                        admission.permits.failure(cfg, None);
                    }
                    return Err(e);
                }
            }
        }
        tokio::select! {
            biased;
            event = next_upstream(&mut upstream), if turn.is_some() => {
                let Some(event) = event else {
                    if let Some(mut u)=protocol.take(){u.finish(Some(101),"STREAM_INTERRUPTED");}
                    if let Some(mut admission) = turn.take() { admission.permits.failure(cfg, None);  }
                    return Err((1011, "upstream disconnected"));
                };
                let frame = event?;
                let closing = frame.opcode() == OpCode::Close;
                if closing {
                    if let Some(mut u)=protocol.take(){u.finish(Some(101),"STREAM_INTERRUPTED");}
                    if let Some(mut admission) = turn.take() { admission.permits.failure(cfg, None);  }
                }
                if !frame.opcode().is_control() {
                    received = true;
                    deadline = tokio::time::Instant::now() + Duration::from_secs(cfg.idle_seconds);
                }
                if let Some(v) = value(&frame) {
                    if let Some(u)=&mut protocol {u.value(&v);}
                    if let Some(id) = v.pointer("/response/id").or_else(|| v.get("response_id")).and_then(|v| v.as_str()) { g.remember_model(id, &route.provider.id, current_model.as_deref()); }
                    let kind = v.get("type").and_then(|v| v.as_str()).unwrap_or("");
                    if matches!(kind, "response.created" | "response.completed" | "response.done") { confirmed_model = current_model.clone(); }
                    if matches!(kind, "response.completed" | "response.done" | "response.failed" | "response.incomplete" | "response.cancelled" | "response.canceled" | "error") {
                        if let Some(mut admission) = turn.take() {
                            if let Some(mut u)=protocol.take() {
                                u.finish(Some(101), "UPSTREAM_ERROR");
                                forward::settle_protocol(&u, &mut admission.permits, cfg);
                                if u.succeeded() { g.successful_response(&route.provider); }
                            }

                        }
                        if let Upstream::Bridge(active) = &mut upstream {
                            active.take();
                        }
                    }
                }
                client.send(frame).await?;
                if closing { return Ok(()); }
            },
            frame = client.incoming.recv() => {
                let Some(frame) = frame else { return Ok(()); };
                let closing = frame.opcode() == OpCode::Close;
                if frame.opcode() == OpCode::Ping {
                    client.send(Frame::pong(frame.payload().to_vec())).await?;
                } else if frame.opcode() == OpCode::Pong {
                    continue;
                } else if closing {
                    if let Upstream::Bridge(Some(bridge)) = &upstream {
                        bridge.cancel();
                    }
                    if let Some(mut admission) = turn.take() {
                        if let Some(mut u) = protocol.take() {
                            u.finish(Some(1000), "CANCELLED");
                            admission.permits.neutral(cfg);
                        }
                    }
                    return Ok(());
                } else if creates(&frame) {
                    if pending.is_some() { return Err((1013, "too many pending turns")); }
                    pending = Some(frame);
                } else if value(&frame).is_some_and(|v| v["type"] == "response.cancel")
                    && matches!(&upstream, Upstream::Bridge(Some(_)))
                {
                    if let Upstream::Bridge(Some(bridge)) = &upstream {
                        bridge.cancel();
                    }
                    let _ = client
                        .send(Frame::text(r#"{"type":"response.cancelled"}"#.to_owned()))
                        .await;
                    if let Some(mut admission) = turn.take() {
                        if let Some(mut u) = protocol.take() {
                            u.finish(Some(101), "CANCELLED");
                            admission.permits.neutral(cfg);
                        }
                    }
                    if let Upstream::Bridge(active) = &mut upstream {
                        active.take();
                    }
                } else if let Err(e) = send_to_upstream(&mut upstream, frame).await {
                    if let Some(mut u) = protocol.take() { u.finish(Some(101), "NETWORK"); }
                    if let Some(mut admission) = turn.take() { admission.permits.failure(cfg, None); }
                    return Err(e);
                }
            },
            _ = tokio::time::sleep_until(deadline), if turn.is_some() => {
                if let Some(mut u)=protocol.take(){u.finish(Some(101),if received {"STREAM_TIMEOUT"}else{"FIRST_BYTE_TIMEOUT"});}
                if let Some(mut admission) = turn.take() { admission.permits.failure(cfg, None); }

                if let Upstream::Native(peer) = &upstream {
                    peer.close(1011, "upstream timeout").await;
                } else if let Upstream::Bridge(Some(bridge)) = &upstream {
                    bridge.cancel();
                }
                return Err((1011, "upstream timeout"));
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bridge_payload_only_changes_websocket_envelope() {
        let frame = Frame::text(
            r#"{"type":"response.create","model":"gpt-test","stream":false,"future":{"x":1}}"#,
        );
        let value: serde_json::Value =
            serde_json::from_slice(&bridge_payload(&frame).unwrap()).unwrap();
        assert_eq!(value["model"], "gpt-test");
        assert_eq!(value["future"]["x"], 1);
        assert_eq!(value["stream"], true);
        assert!(value.get("type").is_none());
    }

    #[tokio::test]
    async fn bridge_events_preserve_json_and_support_multiline_crlf() {
        let input = b"data: {\"type\":\"response.output_text.delta\",\"delta\":\r\ndata: \"delta\"}\r\n\r\ndata: [DONE]\r\n\r\n";
        let reader: BoxReader = Box::new(BufReader::new(std::io::Cursor::new(input.to_vec())));
        let (sender, mut receiver) = mpsc::channel(4);
        let (_cancel, cancel_rx) = watch::channel(false);
        bridge_events(reader, sender, cancel_rx).await;
        let first = receiver.recv().await.unwrap().unwrap();
        assert_eq!(
            first.payload().as_ref(),
            b"{\"type\":\"response.output_text.delta\",\"delta\":\n\"delta\"}"
        );
        let done = receiver.recv().await.unwrap().unwrap();
        assert_eq!(done.payload().as_ref(), br#"{"type":"response.done"}"#);
        assert!(receiver.recv().await.is_none());
    }

    #[test]
    fn claude_routes_are_always_native() {
        assert!(uses_native_websocket(super::super::ClientId::Claude, false));
        assert!(uses_native_websocket(super::super::ClientId::Codex, true));
        assert!(!uses_native_websocket(super::super::ClientId::Codex, false));
    }
}
