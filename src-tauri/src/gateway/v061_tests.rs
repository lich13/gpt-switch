use super::*;
use crate::usage::Filters;
use std::sync::atomic::{AtomicUsize, Ordering};
async fn settle(g: &Gateway) {
    for _ in 0..100 {
        g.usage().flush();
        if g.view().active_connections == 0 {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("fixture request did not finish");
}
#[tokio::test]
async fn incident_rate_limited_p1_waits_then_recovers_while_p2_is_open() {
    let hits = Arc::new(AtomicUsize::new(0));
    let h = hits.clone();
    let p1 = server(move |_| {
        let n = h.fetch_add(1, Ordering::SeqCst);
        async move {
            if n == 0 {
                Response::builder()
                    .status(429)
                    .header("retry-after", "1")
                    .body(full("rate limited"))
                    .unwrap()
            } else {
                Response::new(full("P1 recovered"))
            }
        }
    })
    .await;
    let p2 = server(|_| async {
        Response::builder()
            .status(502)
            .body(full("P2 unavailable"))
            .unwrap()
    })
    .await;
    let (t, g) = fixture(vec![
        format!("http://127.0.0.1:{p1}/v1"),
        format!("http://127.0.0.1:{p2}/v1"),
    ])
    .await;
    let mut cfg = g.view().settings;
    cfg.failure_threshold = 1;
    cfg.queue_seconds = 3;
    update(&g, &t, Edit::Settings { settings: cfg });
    update(
        &g,
        &t,
        Edit::Mode {
            mode: "auto".into(),
        },
    );
    let original_auth = std::fs::read(t.path().join("auth.json")).unwrap();
    start(&g, &t).await;
    let first = request(
        &g,
        "/v1/responses",
        br#"{"model":"gpt-test"}"#.to_vec(),
        vec![("content-type", "application/json")],
    )
    .await;
    assert_eq!(first.status(), 502);
    assert_eq!(
        first.into_body().collect().await.unwrap().to_bytes(),
        "P2 unavailable"
    );
    let view = g.view();
    assert_eq!(view.providers[0].health.failures, 0);
    assert_eq!(
        view.providers[0].health.cooldown_reason.as_deref(),
        Some("rate_limit")
    );
    assert_eq!(
        view.providers[1].health.state,
        super::super::circuit::CircuitState::Open
    );
    let g2 = g.clone();
    let queued = tokio::spawn(async move {
        request(
            &g2,
            "/v1/responses",
            br#"{"model":"gpt-test"}"#.to_vec(),
            vec![("content-type", "application/json")],
        )
        .await
    });
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert_eq!(g.view().waiting_requests, 1);
    let response = queued.await.unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(
        response.into_body().collect().await.unwrap().to_bytes(),
        "P1 recovered"
    );
    settle(&g).await;
    let svc = g.usage();
    svc.flush();
    let d = svc.dashboard(Filters::default()).unwrap();
    assert_eq!(
        (
            d.summary.requests,
            d.summary.successes,
            d.summary.failures,
            d.summary.attempts
        ),
        (2, 1, 1, 3)
    );
    let log = svc.logs(Filters::default(), 0).unwrap();
    let recovered = log.records.iter().find(|r| r.record.successful()).unwrap();
    assert_eq!(recovered.record.provider_id, g.view().providers[0].id);
    assert!(recovered
        .record
        .routing
        .iter()
        .any(|d| d.reason == "rate_limit"));
    assert!(recovered
        .record
        .routing
        .iter()
        .any(|d| d.reason == "circuit_open"));
    assert_eq!(hits.load(Ordering::SeqCst), 2);
    assert_eq!(
        std::fs::read(t.path().join("auth.json")).unwrap(),
        original_auth
    );
    g.stop().await.unwrap();
}
#[tokio::test]
async fn cooldown_timeout_is_a_real_logical_failure_without_an_upstream_attempt() {
    let p1 = server(|_| async {
        Response::builder()
            .status(429)
            .header("retry-after", "120")
            .body(full("upstream-429"))
            .unwrap()
    })
    .await;
    let (t, g) = fixture(vec![format!("http://127.0.0.1:{p1}/v1")]).await;
    let mut settings = g.view().settings;
    settings.queue_seconds = 1;
    update(&g, &t, Edit::Settings { settings });
    start(&g, &t).await;
    let first = request(&g, "/v1/responses", vec![], vec![]).await;
    assert_eq!(first.status(), 429);
    let _ = first.into_body().collect().await.unwrap();
    let response = request(&g, "/v1/responses", vec![], vec![]).await;
    assert_eq!(response.status(), 503);
    assert!(response.headers().contains_key("retry-after"));
    let body = response.into_body().collect().await.unwrap().to_bytes();
    assert!(String::from_utf8_lossy(&body).contains("PROVIDERS_COOLING_DOWN"));
    settle(&g).await;
    g.usage().flush();
    let summary = g.usage().dashboard(Filters::default()).unwrap().summary;
    assert_eq!(
        (
            summary.requests,
            summary.failures,
            summary.attempts,
            summary.gateway_errors
        ),
        (2, 2, 1, 1)
    );
    assert_eq!(g.view().providers[0].health.failures, 0);
    g.stop().await.unwrap();
}
#[tokio::test]
async fn failover_is_one_logical_request_and_keeps_each_attempts_usage_and_cost() {
    let failed = server(|_| async {
        Response::builder()
            .status(502)
            .body(full(
                r#"{"model":"gpt-4o","usage":{"input_tokens":10,"output_tokens":1}}"#,
            ))
            .unwrap()
    })
    .await;
    let ok=server(|_| async {Response::new(full(r#"{"model":"gpt-4o","status":"completed","usage":{"input_tokens":20,"output_tokens":5}}"#))}).await;
    let (t, g) = fixture(vec![
        format!("http://127.0.0.1:{failed}/v1"),
        format!("http://127.0.0.1:{ok}/v1"),
    ])
    .await;
    update(
        &g,
        &t,
        Edit::Mode {
            mode: "auto".into(),
        },
    );
    start(&g, &t).await;
    let req = br#"{"model":"gpt-4o","unknown":{"keep":true}}"#.to_vec();
    let r = request(
        &g,
        "/v1/responses",
        req,
        vec![("content-type", "application/json")],
    )
    .await;
    assert_eq!(r.status(), 200);
    let _ = r.into_body().collect().await.unwrap();
    settle(&g).await;
    let svc = g.usage();
    svc.flush();
    let s = svc.dashboard(Filters::default()).unwrap().summary;
    assert_eq!(
        (s.requests, s.successes, s.failures, s.attempts),
        (1, 1, 0, 2)
    );
    assert_eq!(s.tokens.total(), 36);
    let row = svc.logs(Filters::default(), 0).unwrap().records.remove(0);
    assert_eq!(row.record.tokens.total(), 25);
    assert_eq!(row.record.status, Some(200));
    assert_eq!(row.data_source, "proxy");
    let detail = svc.detail(&row.record.id).unwrap().unwrap();
    assert_eq!(detail.attempts.len(), 2);
    assert_eq!(detail.attempts[0].status, Some(502));
    assert_eq!(row.record.cost.total, detail.attempts[1].cost.total);
    assert_eq!(detail.summary.record.provider_id, g.view().providers[1].id);
    let detail_cost: rust_decimal::Decimal = detail
        .attempts
        .iter()
        .filter_map(|a| a.cost.total.as_deref())
        .map(|v| v.parse::<rust_decimal::Decimal>().unwrap())
        .sum();
    assert_eq!(
        s.cost.unwrap().parse::<rust_decimal::Decimal>().unwrap(),
        detail_cost
    );
    g.stop().await.unwrap();
}
#[tokio::test]
async fn sse_completed_then_client_disconnect_is_success_and_in_band_error_is_failure() {
    let p=server(|req|async move {
        let event=if req.uri().path().ends_with("failed") {b"data: {\"type\":\"response.failed\",\"response\":{\"error\":{\"code\":\"server_error\"}}}\n\n".as_slice()}
        else {b"data: {\"type\":\"response.completed\",\"response\":{\"model\":\"gpt-4o\",\"usage\":{\"input_tokens\":10,\"output_tokens\":3}}}\n\n".as_slice()};
        Response::builder().header("content-type","text/event-stream").body(StreamBody::new(async_stream::try_stream!{
            yield Frame::data(Bytes::copy_from_slice(event));
            std::future::pending::<()>().await;
            yield Frame::data(Bytes::new());
        }).map_err(|e:std::io::Error| -> connector::BoxError {Box::new(e)}).boxed_unsync()).unwrap()
    }).await;
    let (t, g) = fixture(vec![format!("http://127.0.0.1:{p}/v1")]).await;
    start(&g, &t).await;
    for path in ["/v1/responses", "/v1/failed"] {
        let mut r = request(&g, path, vec![], vec![]).await;
        assert_eq!(r.status(), 200);
        let frame = r.body_mut().frame().await.unwrap().unwrap();
        assert!(frame.data_ref().unwrap().starts_with(b"data:"));
        drop(r);
        settle(&g).await;
    }
    g.usage().flush();
    let s = g.usage().dashboard(Filters::default()).unwrap().summary;
    assert_eq!(
        (s.requests, s.successes, s.failures, s.cancelled),
        (2, 1, 1, 0)
    );
    assert_eq!(s.tokens.total(), 13);
    g.stop().await.unwrap();
}
