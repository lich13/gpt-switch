use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

async fn until(f: impl Fn() -> bool) {
    tokio::time::timeout(Duration::from_secs(3), async {
        while !f() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("state did not settle");
}
fn configure(g: &Gateway, t: &tempfile::TempDir, max_retries: usize) {
    update(
        g,
        t,
        Edit::Settings {
            settings: Settings {
                capacity_retry_seconds: 1,
                max_retries,
                failure_threshold: 1,
                ..g.view().settings
            },
        },
    );
    update(
        g,
        t,
        Edit::Mode {
            mode: "auto".into(),
        },
    );
}

#[test]
fn retry_after_floor_and_legacy_settings() {
    let settings = Settings::default();
    for (hint, seconds) in [
        (None, 60),
        (circuit::retry_after("bad"), 60),
        (circuit::retry_after("0"), 60),
        (circuit::retry_after("5"), 60),
        (circuit::retry_after("120"), 120),
    ] {
        assert_eq!(
            forward::capacity_delay(&settings, hint),
            Duration::from_secs(seconds)
        );
    }
    let mut legacy = serde_json::to_value(&settings).unwrap();
    legacy
        .as_object_mut()
        .unwrap()
        .remove("capacityRetrySeconds");
    assert_eq!(
        serde_json::from_value::<Settings>(legacy)
            .unwrap()
            .capacity_retry_seconds,
        60
    );
    for seconds in [0, 86401] {
        assert!(Settings {
            capacity_retry_seconds: seconds,
            ..settings.clone()
        }
        .validate()
        .is_err());
    }
}

#[tokio::test]
async fn retries_use_entire_budget_and_preserve_last_real_error() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let copy = seen.clone();
    let upstream = server(move |req| {
        let copy = copy.clone();
        async move {
            let body = req.into_body().collect().await.unwrap().to_bytes();
            copy.lock().unwrap().push(body);
            Response::builder()
                .status(429)
                .header("retry-after", "0")
                .header("x-original", "preserved")
                .body(full("selected model is at capacity — original error"))
                .unwrap()
        }
    })
    .await;
    let (t, g) = fixture(vec![format!("http://127.0.0.1:{upstream}/v1")]).await;
    configure(&g, &t, 2);
    start(&g, &t).await;
    let original = br#"{ "model": "exact-ID", "input":["original"], "future": {"x": 7} }"#.to_vec();
    let response = request(
        &g,
        "/v1/responses?future=keep",
        original.clone(),
        vec![("content-type", "application/json")],
    )
    .await;
    assert_eq!(response.status(), 429);
    assert_eq!(response.headers()["retry-after"], "0");
    assert_eq!(response.headers()["x-original"], "preserved");
    assert_eq!(
        response.into_body().collect().await.unwrap().to_bytes(),
        "selected model is at capacity — original error"
    );
    assert_eq!(seen.lock().unwrap().len(), 3);
    assert!(seen
        .lock()
        .unwrap()
        .iter()
        .all(|b| b.as_ref() == original.as_slice()));
    assert_eq!(
        g.view().providers[0].health.state,
        circuit::CircuitState::Closed
    );
    assert!(g.view().providers[0].health.protected_single_provider);
    assert!(g.view().capacity_retries.is_empty());
    assert_eq!(g.view().providers[0].active_requests, 0);
    g.stop().await.unwrap();
}

async fn raw_request(g: &Gateway) -> tokio::net::TcpStream {
    let token = g.0.inner.lock().unwrap().store.local_token.clone();
    let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", g.view().settings.port))
        .await
        .unwrap();
    let body = r#"{"model":"test"}"#;
    stream.write_all(format!("POST /v1/responses HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer {token}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",body.len()).as_bytes()).await.unwrap();
    stream
}

#[tokio::test]
async fn capacity_wait_is_bounded_cancelable_and_stopped_without_holding_slots() {
    let hits = Arc::new(AtomicUsize::new(0));
    let count = hits.clone();
    let upstream = server(move |_| {
        count.fetch_add(1, Ordering::SeqCst);
        async {
            Response::builder()
                .status(429)
                .body(full("capacity"))
                .unwrap()
        }
    })
    .await;
    let (t, g) = fixture(vec![format!("http://127.0.0.1:{upstream}/v1")]).await;
    configure(&g, &t, 1);
    update(
        &g,
        &t,
        Edit::Settings {
            settings: Settings {
                capacity_retry_seconds: 60,
                max_waiting: 1,
                ..g.view().settings
            },
        },
    );
    start(&g, &t).await;
    let stream = raw_request(&g).await;
    until(|| !g.view().capacity_retries.is_empty()).await;
    let view = g.view();
    assert_eq!(view.waiting_requests, 1);
    assert!(view.capacity_retries[0].retry_in > 55);
    assert_eq!(view.capacity_retries[0].provider_id, view.providers[0].id);
    assert_eq!(view.providers[0].active_requests, 0);
    let response = request(&g, "/v1/responses", vec![], vec![]).await;
    assert_eq!(response.status(), 429);
    assert_eq!(response.headers()["retry-after"], "5");
    assert_eq!(hits.load(Ordering::SeqCst), 1);
    drop(stream);
    until(|| g.view().waiting_requests == 0 && g.view().active_connections == 0).await;
    update(
        &g,
        &t,
        Edit::Reset {
            id: g.view().providers[0].id.clone(),
        },
    );
    let mut stream = raw_request(&g).await;
    until(|| !g.view().capacity_retries.is_empty()).await;
    g.stop().await.unwrap();
    let mut bytes = Vec::new();
    tokio::time::timeout(Duration::from_secs(2), stream.read_to_end(&mut bytes))
        .await
        .unwrap()
        .unwrap();
    assert!(String::from_utf8_lossy(&bytes).contains("STOPPED"));
    until(|| g.view().waiting_requests == 0 && g.view().active_connections == 0).await;
    assert_eq!(hits.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn capacity_round_reads_new_queue_and_routes_without_touching_files() {
    let first = server(|_| async {
        Response::builder()
            .status(429)
            .body(full("limited"))
            .unwrap()
    })
    .await;
    let second = server(|_| async {
        Response::builder()
            .status(502)
            .body(full("unavailable"))
            .unwrap()
    })
    .await;
    let third = server(|_| async { Response::new(full("new priority")) }).await;
    let (t, g) = fixture(vec![
        format!("http://127.0.0.1:{first}/v1"),
        format!("http://127.0.0.1:{second}/v1"),
    ])
    .await;
    configure(&g, &t, 2);
    start(&g, &t).await;
    let config = std::fs::read(t.path().join("config.toml")).unwrap();
    let task = {
        let g = g.clone();
        tokio::spawn(async move {
            request(
                &g,
                "/v1/responses",
                br#"{"model":"test"}"#.to_vec(),
                vec![("content-type", "application/json")],
            )
            .await
        })
    };
    until(|| !g.view().capacity_retries.is_empty()).await;
    update(
        &g,
        &t,
        Edit::SaveProvider {
            id: None,
            base_url: format!("http://127.0.0.1:{third}/v1"),
            token: "new-fixture-key".into(),
        },
    );
    let mut ids: Vec<_> = g.view().providers.iter().map(|p| p.id.clone()).collect();
    let added = ids.pop().unwrap();
    ids.insert(0, added);
    update(&g, &t, Edit::Reorder { ids });
    let response = task.await.unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(
        response.into_body().collect().await.unwrap().to_bytes(),
        "new priority"
    );
    assert_eq!(std::fs::read(t.path().join("config.toml")).unwrap(), config);
    assert_eq!(
        std::fs::read(t.path().join("auth.json")).unwrap(),
        b"unchanged-auth"
    );
    g.stop().await.unwrap();
}

#[tokio::test]
async fn filtered_alternatives_retry_p1_and_wait_does_not_consume_response_timeout() {
    let hits = Arc::new(AtomicUsize::new(0));
    let h = hits.clone();
    let first = server(move |_| {
        let attempt = h.fetch_add(1, Ordering::SeqCst);
        async move {
            if attempt == 0 {
                Response::builder()
                    .status(429)
                    .body(full("capacity"))
                    .unwrap()
            } else {
                Response::new(
                    StreamBody::new(async_stream::try_stream! {
                        yield Frame::data(Bytes::from_static(b"first"));
                        tokio::time::sleep(Duration::from_millis(150)).await;
                        yield Frame::data(Bytes::from_static(b"last"));
                    })
                    .boxed_unsync(),
                )
            }
        }
    })
    .await;
    let backup_hits = Arc::new(AtomicUsize::new(0));
    let h = backup_hits.clone();
    let second = server(move |_| {
        h.fetch_add(1, Ordering::SeqCst);
        async { Response::new(full("wrong model")) }
    })
    .await;
    let (t, g) = fixture(vec![
        format!("http://127.0.0.1:{first}/v1"),
        format!("http://127.0.0.1:{second}/v1"),
    ])
    .await;
    configure(&g, &t, 1);
    update(
        &g,
        &t,
        Edit::ModelsProvider {
            id: g.view().providers[1].id.clone(),
            allowed_models: Some(vec!["other-model".into()]),
        },
    );
    update(
        &g,
        &t,
        Edit::Settings {
            settings: Settings {
                capacity_retry_seconds: 2,
                total_seconds: 1,
                ..g.view().settings
            },
        },
    );
    start(&g, &t).await;
    let response = request(
        &g,
        "/v1/responses",
        br#"{"model":"test"}"#.to_vec(),
        vec![("content-type", "application/json")],
    )
    .await;
    assert_eq!(response.status(), 200);
    assert_eq!(
        response.into_body().collect().await.unwrap().to_bytes(),
        "firstlast"
    );
    assert_eq!(hits.load(Ordering::SeqCst), 2);
    assert_eq!(backup_hits.load(Ordering::SeqCst), 0);
    g.stop().await.unwrap();
}

#[tokio::test]
async fn protection_uses_actual_queue_and_updates_existing_circuit() {
    let (t, g) = fixture(vec![
        "https://first.example.invalid/v1".into(),
        "https://second.example.invalid/v1".into(),
    ])
    .await;
    configure(&g, &t, 0);
    let first = g.view().providers[0].id.clone();
    let second = g.view().providers[1].id.clone();
    let circuit = g.route(&first).unwrap().provider_circuit;
    // A context-pinned request does not make a two-provider queue a sole-provider queue.
    circuit
        .acquire(false)
        .unwrap()
        .finish(circuit::Outcome::Failure(None), &g.view().settings);
    assert_eq!(circuit.health().state, circuit::CircuitState::Open);
    update(
        &g,
        &t,
        Edit::QueueProvider {
            id: second.clone(),
            queued: false,
        },
    );
    assert_eq!(circuit.health().state, circuit::CircuitState::Closed);
    assert!(circuit.health().protected_single_provider);
    assert!(!circuit.health().available);
    update(
        &g,
        &t,
        Edit::QueueProvider {
            id: second,
            queued: true,
        },
    );
    assert!(!circuit.health().protected_single_provider);
}

#[tokio::test]
async fn service_and_network_errors_do_not_open_sole_automatic_provider() {
    for status in [502, 503] {
        let upstream = server(move |_| async move {
            Response::builder()
                .status(status)
                .body(full("service unavailable"))
                .unwrap()
        })
        .await;
        let (t, g) = fixture(vec![format!("http://127.0.0.1:{upstream}/v1")]).await;
        configure(&g, &t, 0);
        start(&g, &t).await;
        for _ in 0..3 {
            let r = request(&g, "/v1/responses", vec![], vec![]).await;
            assert_eq!(r.status().as_u16(), status);
            r.into_body().collect().await.unwrap();
            let health = &g.view().providers[0].health;
            assert_eq!(health.state, circuit::CircuitState::Closed);
            assert_eq!(
                health.cooldown_reason.as_deref(),
                Some("single_provider_protected")
            );
        }
        assert_eq!(g.view().providers[0].health.failures, 3);
        g.stop().await.unwrap();
    }
    let socket = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = socket.local_addr().unwrap().port();
    drop(socket);
    let (t, g) = fixture(vec![format!("http://127.0.0.1:{port}/v1")]).await;
    configure(&g, &t, 0);
    start(&g, &t).await;
    assert_eq!(
        request(&g, "/v1/responses", vec![], vec![]).await.status(),
        502
    );
    assert_eq!(
        g.view().providers[0].health.state,
        circuit::CircuitState::Closed
    );
    assert!(g.view().providers[0].health.protected_single_provider);
    g.stop().await.unwrap();
}

#[tokio::test]
async fn manual_and_known_context_retry_only_owner_unknown_context_once() {
    for mode in ["manual", "known", "unknown"] {
        let hits = Arc::new(AtomicUsize::new(0));
        let h = hits.clone();
        let upstream = server(move |_| {
            h.fetch_add(1, Ordering::SeqCst);
            async {
                Response::builder()
                    .status(429)
                    .body(full("limited"))
                    .unwrap()
            }
        })
        .await;
        let backup_hits = Arc::new(AtomicUsize::new(0));
        let h = backup_hits.clone();
        let backup = server(move |_| {
            h.fetch_add(1, Ordering::SeqCst);
            async { Response::new(full("wrong provider")) }
        })
        .await;
        let (t, g) = fixture(vec![
            format!("http://127.0.0.1:{upstream}/v1"),
            format!("http://127.0.0.1:{backup}/v1"),
        ])
        .await;
        configure(&g, &t, 1);
        if mode == "manual" {
            update(
                &g,
                &t,
                Edit::Select {
                    id: g.view().providers[0].id.clone(),
                },
            );
        }
        if mode == "known" {
            g.remember("cursor-fixture", &g.view().providers[0].id);
        }
        start(&g, &t).await;
        let body = if mode == "manual" {
            br#"{"model":"test"}"#.to_vec()
        } else {
            br#"{"model":"test","previous_response_id":"cursor-fixture"}"#.to_vec()
        };
        assert_eq!(
            request(
                &g,
                "/v1/responses",
                body,
                vec![("content-type", "application/json")]
            )
            .await
            .status(),
            429
        );
        assert_eq!(
            hits.load(Ordering::SeqCst),
            if mode == "unknown" { 1 } else { 2 }
        );
        assert_eq!(backup_hits.load(Ordering::SeqCst), 0);
        assert!(!g.view().providers[0].health.protected_single_provider);
        g.stop().await.unwrap();
    }
}

#[tokio::test]
async fn claude_rate_limit_keeps_original_cooldown_and_no_capacity_retry() {
    let hits = Arc::new(AtomicUsize::new(0));
    let h = hits.clone();
    let upstream = server(move |_| {
        h.fetch_add(1, Ordering::SeqCst);
        async {
            Response::builder()
                .status(429)
                .header("retry-after", "2")
                .body(full("Selected model is at capacity"))
                .unwrap()
        }
    })
    .await;
    let (_t, _codex, g, home) =
        claude_v080::claude_fixture(&format!("http://127.0.0.1:{upstream}"), "fixture-key").await;
    g.edit(
        Edit::Mode {
            mode: "auto".into(),
        },
        &g.view().revision,
        &home,
    )
    .unwrap();
    g.start(&g.view().revision, &home).await.unwrap();
    let r = request(
        &g,
        "/v1/messages",
        br#"{"model":"claude-test"}"#.to_vec(),
        vec![("content-type", "application/json")],
    )
    .await;
    assert_eq!(r.status(), 429);
    assert_eq!(hits.load(Ordering::SeqCst), 1);
    let h = &g.view().providers[0].health;
    assert_eq!(h.cooldown_reason.as_deref(), Some("rate_limit"));
    assert!(h.retry_in <= 2);
    assert!(!h.protected_single_provider);
    assert!(g.view().capacity_retries.is_empty());
    g.stop().await.unwrap();
}

#[tokio::test]
async fn responses_ws_handshake_capacity_fails_over_and_stays_on_established_provider() {
    let bad = server(|_| async {
        Response::builder()
            .status(503)
            .body(full("Selected model is at capacity"))
            .unwrap()
    })
    .await;
    let (ok, release, seen) = concurrency::response_ws_server().await;
    release.add_permits(2);
    let (t, g) = fixture(vec![
        format!("http://127.0.0.1:{bad}/v1"),
        format!("http://127.0.0.1:{ok}/v1"),
    ])
    .await;
    configure(&g, &t, 1);
    start(&g, &t).await;
    let mut ws = concurrency::responses_client(&g).await;
    for _ in 0..2 {
        ws.send(yawc::Frame::text(
            r#"{"type":"response.create","model":"exact-ID","extra":42}"#.to_owned(),
        ))
        .await
        .unwrap();
        tokio::time::timeout(Duration::from_secs(3), async {
            while let Some(frame) = ws.next().await {
                if frame.opcode() == yawc::OpCode::Text
                    && String::from_utf8_lossy(frame.payload()).contains("response.completed")
                {
                    break;
                }
                assert_ne!(frame.opcode(), yawc::OpCode::Close);
            }
        })
        .await
        .unwrap();
    }
    assert_eq!(seen.lock().unwrap().len(), 2);
    until(|| g.view().providers.iter().all(|p| p.active_requests == 0)).await;
    drop(ws);
    g.stop().await.unwrap();
}

#[tokio::test]
async fn capacity_errors_preserve_compressed_spooled_multipart_and_stream_payloads() {
    use std::io::Write;
    let json = br#"{"model":"fixture","stream":true,"input":["unchanged"]}"#.to_vec();
    let mut gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    gzip.write_all(&json).unwrap();
    let mut deflate = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    deflate.write_all(&json).unwrap();
    let mut compressed_error =
        flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    compressed_error
        .write_all(b"{\"error\":\"SELECTED MODEL IS AT CAPACITY\"}")
        .unwrap();
    let error = compressed_error.finish().unwrap();
    let cases = vec![
        (json.clone(), "application/json", "identity"),
        (gzip.finish().unwrap(), "application/json", "gzip"),
        (deflate.finish().unwrap(), "application/json", "deflate"),
        (zstd::stream::encode_all(json.as_slice(),1).unwrap(), "application/json", "zstd"),
        (b"--test\r\nContent-Disposition: form-data; name=\"model\"\r\n\r\nfixture\r\n--test\r\nContent-Disposition: form-data; name=\"file\"; filename=\"blob\"\r\n\r\n\x00\x01\xff\r\n--test--\r\n".to_vec(), "multipart/form-data; boundary=test", "identity"),
        (format!("{{\"model\":\"fixture\",\"input\":\"{}\"}}", "x".repeat(3*1024*1024)).into_bytes(), "application/json", "identity"),
    ];
    for (body, content_type, encoding) in cases {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let a = seen.clone();
        let error = error.clone();
        let first = server(move |req| {
            let a = a.clone();
            let error = error.clone();
            async move {
                let bytes = req.into_body().collect().await.unwrap().to_bytes();
                a.lock().unwrap().push(bytes);
                Response::builder()
                    .status(503)
                    .header("content-encoding", "gzip")
                    .body(full(error))
                    .unwrap()
            }
        })
        .await;
        let a = seen.clone();
        let second = server(move |req| {
            let a = a.clone();
            async move {
                let bytes = req.into_body().collect().await.unwrap().to_bytes();
                a.lock().unwrap().push(bytes);
                Response::builder()
                    .header("content-type", "text/event-stream")
                    .body(full("data: {\"type\":\"response.completed\"}\n\n"))
                    .unwrap()
            }
        })
        .await;
        let (t, g) = fixture(vec![
            format!("http://127.0.0.1:{first}/v1"),
            format!("http://127.0.0.1:{second}/v1"),
        ])
        .await;
        configure(&g, &t, 1);
        start(&g, &t).await;
        let r = request(
            &g,
            "/v1/responses",
            body.clone(),
            vec![
                ("content-type", content_type),
                ("content-encoding", encoding),
            ],
        )
        .await;
        assert_eq!(r.status(), 200);
        r.into_body().collect().await.unwrap();
        let received = seen.lock().unwrap().clone();
        assert_eq!(received.len(), 2);
        assert!(received
            .iter()
            .all(|value| value.as_ref() == body.as_slice()));
        assert_eq!(
            g.view().providers[0].health.cooldown_reason.as_deref(),
            Some("capacity_retry")
        );
        until(|| g.view().active_connections == 0).await;
        assert_eq!(std::fs::read_dir(g.0.spool.path()).unwrap().count(), 0);
        g.stop().await.unwrap();
    }
}

#[tokio::test]
async fn responses_ws_capacity_wait_cancels_and_exhaustion_closes_1013() {
    let hits = Arc::new(AtomicUsize::new(0));
    let h = hits.clone();
    let port = server(move |_| {
        h.fetch_add(1, Ordering::SeqCst);
        async {
            Response::builder()
                .status(429)
                .body(full("limited"))
                .unwrap()
        }
    })
    .await;
    let (t, g) = fixture(vec![format!("http://127.0.0.1:{port}/v1")]).await;
    configure(&g, &t, 1);
    start(&g, &t).await;
    let mut ws = concurrency::responses_client(&g).await;
    ws.send(yawc::Frame::text(
        r#"{"type":"response.create","model":"test"}"#.to_owned(),
    ))
    .await
    .unwrap();
    until(|| !g.view().capacity_retries.is_empty()).await;
    ws.send(yawc::Frame::close(1000.into(), "cancel"))
        .await
        .unwrap();
    drop(ws);
    until(|| g.view().waiting_requests == 0 && g.view().active_connections == 0).await;
    assert_eq!(hits.load(Ordering::SeqCst), 1);
    update(
        &g,
        &t,
        Edit::Reset {
            id: g.view().providers[0].id.clone(),
        },
    );
    let mut ws = concurrency::responses_client(&g).await;
    ws.send(yawc::Frame::text(
        r#"{"type":"response.create","model":"test"}"#.to_owned(),
    ))
    .await
    .unwrap();
    let frame = tokio::time::timeout(Duration::from_secs(3), ws.next())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(frame.close_code().map(u16::from), Some(1013));
    assert_eq!(hits.load(Ordering::SeqCst), 3);
    assert_eq!(
        g.view().providers[0].health.state,
        circuit::CircuitState::Closed
    );
    drop(ws);
    g.stop().await.unwrap();
}
