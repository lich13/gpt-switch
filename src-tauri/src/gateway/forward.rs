use super::{
    circuit::{self, Outcome, Permit},
    connector::{self, BoxError},
    model::Settings,
    replay::{self, Replay, WireBody},
    Active, Gateway, Route,
};
use http_body_util::{BodyExt, StreamBody};
use hyper::{body::Incoming, header, HeaderMap, Request, Response, StatusCode, Uri};
use hyper_util::rt::TokioIo;
use std::{
    convert::Infallible,
    time::{Duration, Instant},
};
use tokio::sync::watch;
pub async fn serve(
    gateway: Gateway,
    listener: tokio::net::TcpListener,
    mut shutdown: watch::Receiver<bool>,
) {
    loop {
        tokio::select! {
            _=shutdown.changed()=>break,
            result=listener.accept()=>{
                let Ok((socket,_))=result else{break};let g=gateway.clone();let mut stop=shutdown.clone();
                tokio::spawn(async move{
                    let service=hyper::service::service_fn(move|request|{let g=g.clone();async move{Ok::<_,Infallible>(forward(g,request).await)}});
                    let connection=hyper::server::conn::http1::Builder::new().preserve_header_case(true)
                        .serve_connection(TokioIo::new(socket),service).with_upgrades();
                    tokio::pin!(connection);
                    tokio::select!{_= &mut connection=>(),_=stop.changed()=>{connection.as_mut().graceful_shutdown();let _=connection.await;}}
                });
            }
        }
    }
}
fn error(status: StatusCode, code: &str, message: &str) -> Response<WireBody> {
    Response::builder().status(status).header(header::CONTENT_TYPE,"application/json")
        .body(replay::full(serde_json::to_vec(&serde_json::json!({"error":{"type":"gpt_switch_gateway","code":code,"message":message}})).unwrap())).unwrap()
}
pub(super) fn target(base: &str, incoming: &Uri) -> Result<Uri, BoxError> {
    let path = incoming.path();
    let suffix = if path == "/v1" {
        ""
    } else if let Some(s) = path.strip_prefix("/v1/") {
        return format!(
            "{}/{}{}",
            base.trim_end_matches('/'),
            s,
            incoming
                .query()
                .map(|q| format!("?{q}"))
                .unwrap_or_default()
        )
        .parse()
        .map_err(Into::into);
    } else {
        path
    };
    format!(
        "{}{}{}",
        base.trim_end_matches('/'),
        suffix,
        incoming
            .query()
            .map(|q| format!("?{q}"))
            .unwrap_or_default()
    )
    .parse()
    .map_err(Into::into)
}
pub(super) fn clean_headers(headers: &mut HeaderMap, upgrade: bool) {
    let nominated: Vec<_> = headers
        .get_all(header::CONNECTION)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|s| s.split(',').map(str::trim).map(str::to_owned))
        .collect();
    for name in nominated {
        if !(upgrade && name.eq_ignore_ascii_case("upgrade")) {
            headers.remove(&name);
        }
    }
    for name in [
        "connection",
        "proxy-connection",
        "keep-alive",
        "proxy-authenticate",
        "proxy-authorization",
        "te",
        "trailer",
        "transfer-encoding",
    ] {
        headers.remove(name);
    }
    if upgrade {
        headers.insert(
            header::CONNECTION,
            header::HeaderValue::from_static("upgrade"),
        );
    } else {
        headers.remove(header::UPGRADE);
    }
}
fn authorized(headers: &HeaderMap, token: &str) -> bool {
    if headers.get_all(header::AUTHORIZATION).iter().count() != 1 {
        return false;
    }
    let expected = format!("Bearer {token}");
    let actual = headers
        .get(header::AUTHORIZATION)
        .map(|h| h.as_bytes())
        .unwrap_or_default();
    actual.len() == expected.len()
        && actual
            .iter()
            .zip(expected.as_bytes())
            .fold(0u8, |v, (a, b)| v | (a ^ b))
            == 0
}
struct Observe {
    buffer: Vec<u8>,
    stream: bool,
    gateway: Gateway,
    provider: String,
    overflow: bool,
}
impl Observe {
    fn new(stream: bool, gateway: Gateway, provider: String) -> Self {
        Self {
            buffer: vec![],
            stream,
            gateway,
            provider,
            overflow: false,
        }
    }
    fn parse(&self, bytes: &[u8]) {
        if let Ok(v) = serde_json::from_slice::<serde_json::Value>(bytes) {
            for id in [v.get("id"), v.pointer("/response/id"), v.get("response_id")]
                .into_iter()
                .flatten()
                .filter_map(|v| v.as_str())
            {
                if !id.is_empty() && id.len() <= 1024 {
                    self.gateway.remember(id, &self.provider);
                }
            }
        }
    }
    fn feed(&mut self, bytes: &[u8]) {
        if !self.stream {
            if !self.overflow && self.buffer.len() + bytes.len() <= 2 * 1024 * 1024 {
                self.buffer.extend_from_slice(bytes);
            } else {
                self.overflow = true;
                self.buffer.clear();
            }
            return;
        }
        for segment in bytes.split_inclusive(|b| *b == b'\n') {
            if self.buffer.len() + segment.len() <= 256 * 1024 && !self.overflow {
                self.buffer.extend_from_slice(segment);
            } else {
                self.overflow = true;
                self.buffer.clear();
            }
            if segment.ends_with(b"\n") {
                if !self.overflow {
                    if let Some(data) = self.buffer.strip_prefix(b"data:") {
                        self.parse(data);
                    }
                }
                self.buffer.clear();
                self.overflow = false;
            }
        }
    }
    fn finish(&self) {
        if !self.stream && !self.overflow {
            self.parse(&self.buffer);
        }
    }
}
async fn forward(gateway: Gateway, mut request: Request<Incoming>) -> Response<WireBody> {
    let began = Instant::now();
    let (settings, mode, mut ids, token, running) = {
        let s = gateway.0.inner.lock().unwrap();
        let ids = if s.store.mode == "auto" {
            s.store
                .providers
                .iter()
                .filter(|p| p.queued)
                .map(|p| p.id.clone())
                .collect::<Vec<_>>()
        } else {
            s.store.selected.iter().cloned().collect()
        };
        (
            s.store.settings.clone(),
            s.store.mode.clone(),
            ids,
            s.store.local_token.clone(),
            s.running,
        )
    };
    if !running {
        return error(StatusCode::SERVICE_UNAVAILABLE, "STOPPED", "网关已停止");
    }
    if !authorized(request.headers(), &token) {
        return error(StatusCode::UNAUTHORIZED, "LOCAL_AUTH", "本地网关认证失败");
    }
    // Capture all route versions before receiving the request body: edits only affect new requests.
    let provider_ids: Vec<_> = gateway
        .0
        .inner
        .lock()
        .unwrap()
        .store
        .providers
        .iter()
        .map(|p| p.id.clone())
        .collect();
    let routes: std::collections::HashMap<_, _> = provider_ids
        .into_iter()
        .filter_map(|id| gateway.route(&id).map(|r| (id, r)))
        .collect();
    let active = Active::new(gateway.clone());
    let websocket = request
        .headers()
        .get(header::UPGRADE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.eq_ignore_ascii_case("websocket"));
    let downstream_upgrade = websocket.then(|| hyper::upgrade::on(&mut request));
    let (parts, body) = request.into_parts();
    let replay = match tokio::time::timeout(
        Duration::from_secs(settings.total_seconds),
        Replay::capture(body, gateway.0.spool.path()),
    )
    .await
    {
        Ok(Ok(body)) => body,
        _ => {
            return error(
                StatusCode::BAD_REQUEST,
                "BODY",
                "请求未完整接收、超过 1 GiB 或已取消",
            )
        }
    };
    let json = parts
        .headers
        .get(header::CONTENT_TYPE)
        .and_then(|h| h.to_str().ok())
        .is_some_and(|s| s.contains("json"));
    let hints = if json {
        replay
            .hints(
                parts
                    .headers
                    .get(header::CONTENT_ENCODING)
                    .and_then(|h| h.to_str().ok())
                    .unwrap_or("identity"),
            )
            .await
    } else {
        Ok(replay::RequestHints::default())
    };
    let stream_hint = websocket
        || hints.as_ref().is_ok_and(|h| h.stream)
        || parts
            .headers
            .get(header::ACCEPT)
            .is_some_and(|h| h.as_bytes().windows(17).any(|w| w == b"text/event-stream"));
    let affinity = hints.map(|h| h.previous_response_id);
    let pinned = match affinity {
        Ok(Some(id)) => {
            let owner = gateway
                .0
                .inner
                .lock()
                .unwrap()
                .affinity
                .get(&id)
                .filter(|(_, at)| at.elapsed() < Duration::from_secs(3600))
                .map(|(id, _)| id.clone());
            if let Some(owner) = owner {
                ids = vec![owner];
            } else {
                ids.truncate(1);
            }
            true
        }
        Err(_) => {
            ids.truncate(1);
            true
        }
        Ok(None) => false,
    };
    let mut last = None;
    let mut attempted = 0usize;
    let mut last_category = "NO_PROVIDER";
    for id in ids {
        if attempted > settings.max_retries || (pinned && attempted >= 1) {
            break;
        }
        let Some(route) = routes.get(&id).cloned() else {
            continue;
        };
        let Some(mut permits) = Permits::acquire(&route, mode == "manual") else {
            continue;
        };
        let uri = match target(&route.provider.base_url, &parts.uri) {
            Ok(uri) => uri,
            Err(_) => continue,
        };
        let mut upstream = Request::new(replay.body());
        *upstream.method_mut() = parts.method.clone();
        *upstream.uri_mut() = uri;
        *upstream.headers_mut() = parts.headers.clone();
        clean_headers(upstream.headers_mut(), websocket);
        upstream.headers_mut().remove(header::HOST);
        upstream.headers_mut().remove(header::AUTHORIZATION);
        let Ok(auth) = header::HeaderValue::from_str(&format!("Bearer {}", route.provider.token))
        else {
            continue;
        };
        upstream.headers_mut().insert(header::AUTHORIZATION, auth);
        if !replay.has_trailers() {
            upstream.headers_mut().insert(
                header::CONTENT_LENGTH,
                header::HeaderValue::from(replay.length),
            );
        } else {
            upstream.headers_mut().remove(header::CONTENT_LENGTH);
        }
        let attempt = attempted;
        attempted += 1;
        let started = Instant::now();
        let deadline = tokio::time::Instant::now()
            + Duration::from_secs(if stream_hint {
                settings.first_byte_seconds
            } else {
                settings.total_seconds
            });
        let response = tokio::time::timeout_at(deadline, route.client.request(upstream)).await;
        let mut response = match response {
            Ok(Ok(response)) => response,
            Ok(Err(e)) => {
                let category = connector::classify(&e);
                let proxy_failure = category.is_some_and(|e| e.is_proxy());
                last_category = if proxy_failure { "PROXY" } else { "NETWORK" };
                permits.failure(&settings, proxy_failure, None);
                gateway.record(&route, None, started, attempt, last_category);
                continue;
            }
            Err(_) => {
                last_category = "FIRST_BYTE_TIMEOUT";
                permits.failure(&settings, false, None);
                gateway.record(&route, None, started, attempt, last_category);
                continue;
            }
        };
        permits.proxy_success(&settings);
        let status = response.status();
        if status == StatusCode::SWITCHING_PROTOCOLS && websocket {
            permits.success(&settings);
            let upstream_upgrade = hyper::upgrade::on(&mut response);
            let (mut response_parts, _) = response.into_parts();
            clean_headers(&mut response_parts.headers, true);
            let downstream = downstream_upgrade.expect("upgrade exists");
            let g = gateway.clone();
            let cfg = settings.clone();
            tokio::spawn(async move {
                let _active = active;
                let connected = tokio::try_join!(upstream_upgrade, downstream);
                if let Ok((a, b)) = connected {
                    // Opaque tunnel preserves every data/control/close frame, including binary payloads.
                    let result =
                        tokio::io::copy_bidirectional(&mut TokioIo::new(a), &mut TokioIo::new(b))
                            .await;
                    permits.neutral(&cfg);
                    g.record(
                        &route,
                        Some(101),
                        began,
                        attempt,
                        if result.is_ok() {
                            "WEBSOCKET"
                        } else {
                            "CLOSED"
                        },
                    );
                }
            });
            return Response::from_parts(response_parts, replay::empty());
        }
        if circuit::retryable(status.as_u16()) {
            let cooldown = response
                .headers()
                .get(header::RETRY_AFTER)
                .and_then(|h| h.to_str().ok())
                .and_then(circuit::retry_after);
            let (mut response_parts, body) = response.into_parts();
            clean_headers(&mut response_parts.headers, false);
            let captured = tokio::time::timeout(
                Duration::from_secs(settings.total_seconds),
                Replay::capture(body, gateway.0.spool.path()),
            )
            .await;
            permits.failure(&settings, false, cooldown);
            gateway.record(&route, Some(status.as_u16()), started, attempt, "HTTP");
            if let Ok(Ok(body)) = captured {
                last = Some(Response::from_parts(response_parts, body.body()));
            }
            last_category = "HTTP";
            continue;
        }
        let stream = response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.starts_with("text/event-stream"));
        let (mut response_parts, mut body) = response.into_parts();
        clean_headers(&mut response_parts.headers, false);
        let first_deadline = if stream {
            tokio::time::Instant::now()
                + Duration::from_secs(settings.first_byte_seconds).saturating_sub(started.elapsed())
        } else {
            deadline
        };
        let first = tokio::time::timeout_at(first_deadline, body.frame()).await;
        let first = match first {
            Ok(Some(Ok(frame))) => Some(frame),
            Ok(None) => None,
            _ => {
                permits.failure(&settings, false, None);
                gateway.record(
                    &route,
                    Some(status.as_u16()),
                    started,
                    attempt,
                    "FIRST_BYTE_TIMEOUT",
                );
                last_category = "FIRST_BYTE_TIMEOUT";
                continue;
            }
        };
        let total_deadline = tokio::time::Instant::now()
            + Duration::from_secs(settings.total_seconds).saturating_sub(began.elapsed());
        let mut observe = Observe::new(stream, gateway.clone(), route.provider.id.clone());
        let g = gateway.clone();
        let cfg = settings.clone();
        let neutral = status.as_u16() >= 400;
        let output = async_stream::try_stream! {
            let _active=active;
            if let Some(frame)=first {if let Some(data)=frame.data_ref(){observe.feed(data);}yield frame;}
            loop {
                let limit=if stream {tokio::time::Instant::now()+Duration::from_secs(cfg.idle_seconds)}else{total_deadline};
                match tokio::time::timeout_at(limit,body.frame()).await {
                    Ok(Some(Ok(frame)))=>{if let Some(data)=frame.data_ref(){observe.feed(data);}yield frame;}
                    Ok(None)=>{observe.finish();if neutral{permits.neutral(&cfg);}else{permits.success(&cfg);}
                        g.record(&route,Some(status.as_u16()),began,attempt,if neutral{"CLIENT_ERROR"}else{"OK"});break;}
                    result=>{
                        let category=if result.is_err(){"STREAM_TIMEOUT"}else{"STREAM_INTERRUPTED"};
                        permits.failure(&cfg,false,None);g.record(&route,Some(status.as_u16()),began,attempt,category);
                        Err::<(),BoxError>(std::io::Error::other("上游流中断").into())?;
                    }
                }
            }
        };
        return Response::from_parts(response_parts, StreamBody::new(output).boxed_unsync());
    }
    last.unwrap_or_else(|| {
        error(
            if attempted == 0 {
                StatusCode::SERVICE_UNAVAILABLE
            } else {
                StatusCode::BAD_GATEWAY
            },
            last_category,
            if attempted == 0 {
                "没有可用供应商，请检查队列、代理和熔断状态"
            } else {
                "所有可用供应商均请求失败"
            },
        )
    })
}
struct Permits {
    provider: Option<Permit>,
    proxy: Option<Permit>,
}
impl Permits {
    fn acquire(route: &Route, manual: bool) -> Option<Self> {
        let proxy = match &route.proxy_circuit {
            Some(c) => Some(c.acquire(manual)?),
            None => None,
        };
        Some(Self {
            provider: Some(route.provider_circuit.acquire(manual)?),
            proxy,
        })
    }
    fn proxy_success(&mut self, cfg: &Settings) {
        if let Some(p) = self.proxy.take() {
            p.finish(Outcome::Success, cfg);
        }
    }
    fn success(&mut self, cfg: &Settings) {
        if let Some(p) = self.provider.take() {
            p.finish(Outcome::Success, cfg);
        }
    }
    fn neutral(&mut self, cfg: &Settings) {
        if let Some(p) = self.provider.take() {
            p.finish(Outcome::Neutral, cfg);
        }
    }
    fn failure(&mut self, cfg: &Settings, proxy: bool, retry: Option<Duration>) {
        if proxy {
            // A failed shared proxy is unavailable immediately; it must not poison each provider.
            if let Some(p) = self.proxy.take() {
                p.finish(
                    Outcome::Failure(Some(Duration::from_secs(cfg.cooldown_seconds))),
                    cfg,
                );
            }
            self.neutral(cfg);
        } else {
            self.proxy_success(cfg);
            if let Some(p) = self.provider.take() {
                p.finish(Outcome::Failure(retry), cfg);
            }
        }
    }
}
