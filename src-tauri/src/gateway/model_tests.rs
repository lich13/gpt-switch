use super::*;
use crate::gateway::routing::Requirement;
fn allow(g: &Gateway, t: &tempfile::TempDir, n: usize, models: Option<Vec<&str>>) {
    update(
        g,
        t,
        Edit::ModelsProvider {
            id: g.view().providers[n].id.clone(),
            allowed_models: models.map(|m| m.into_iter().map(str::to_owned).collect()),
        },
    );
}
async fn body(response: Response<Incoming>) -> Bytes {
    response.into_body().collect().await.unwrap().to_bytes()
}

#[tokio::test]
async fn exact_rules_filter_before_attempts_and_preserve_original_json_and_resources() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let mut urls = Vec::new();
    for n in 0..5 {
        let seen = seen.clone();
        let port = server(move |req| {
            let seen = seen.clone();
            async move {
                let payload = req.into_body().collect().await.unwrap().to_bytes();
                seen.lock().unwrap().push((n, payload.clone()));
                Response::new(full(payload))
            }
        })
        .await;
        urls.push(format!("http://127.0.0.1:{port}"));
    }
    let (t, g) = fixture(urls).await;
    for n in 0..4 {
        allow(&g, &t, n, Some(vec!["other"]));
    }
    allow(&g, &t, 4, Some(vec!["Exact"]));
    update(
        &g,
        &t,
        Edit::Mode {
            mode: "auto".into(),
        },
    );
    start(&g, &t).await;
    let config = std::fs::read(t.path().join("config.toml")).unwrap();
    let payload = b"{ \"model\" : \"Exact\", \"future\" : [3, 7] }";
    assert_eq!(
        body(
            request(
                &g,
                "/v1/responses",
                payload.to_vec(),
                vec![("content-type", "application/json")]
            )
            .await
        )
        .await
        .as_ref(),
        payload
    );
    assert_eq!(seen.lock().unwrap()[0].0, 4);
    assert_eq!(g.view().providers[4].active_requests, 0);
    assert!(g.view().providers[..4]
        .iter()
        .all(|p| p.health.requests == 0 && p.active_requests == 0));
    let denied = request(
        &g,
        "/v1/responses",
        br#"{"model":"exact"}"#.to_vec(),
        vec![("content-type", "application/json")],
    )
    .await;
    assert_eq!(denied.status(), 400);
    assert!(String::from_utf8(body(denied).await.to_vec())
        .unwrap()
        .contains("MODEL_NOT_ALLOWED"));
    let unknown = request(&g, "/future", vec![0, 255], vec![]).await;
    assert_eq!(unknown.status(), 400);
    assert!(String::from_utf8(body(unknown).await.to_vec())
        .unwrap()
        .contains("MODEL_UNDETERMINED"));
    let resource = request(&g, "/v1/files", vec![0, 255], vec![]).await;
    assert_eq!(body(resource).await.as_ref(), &[0, 255]);
    assert_eq!(std::fs::read(t.path().join("config.toml")).unwrap(), config);
    assert_eq!(
        std::fs::read(t.path().join("auth.json")).unwrap(),
        b"unchanged-auth"
    );
    g.stop().await.unwrap();
}

#[tokio::test]
async fn multipart_compressed_hints_and_manual_or_context_restrictions() {
    let p = server(|r| async {
        Response::new(full(r.into_body().collect().await.unwrap().to_bytes()))
    })
    .await;
    let (t, g) = fixture(vec![
        format!("http://127.0.0.1:{p}"),
        format!("http://127.0.0.1:{p}"),
    ])
    .await;
    allow(&g, &t, 0, Some(vec!["a"]));
    allow(&g, &t, 1, Some(vec!["b"]));
    start(&g, &t).await;
    let denied = request(
        &g,
        "/v1/responses",
        br#"{"model":"b"}"#.to_vec(),
        vec![("content-type", "application/json")],
    )
    .await;
    assert_eq!(denied.status(), 400);
    let multipart = b"--test-boundary\r\nContent-Disposition: form-data; name=\"file\"; filename=\"binary\"\r\n\r\n\0\xff\r\n--test-boundary\r\nContent-Disposition: form-data; name=\"model\"\r\n\r\na\r\n--test-boundary--\r\n";
    let response = request(
        &g,
        "/v1/images/edits",
        multipart.to_vec(),
        vec![(
            "content-type",
            "multipart/form-data; boundary=\"test-boundary\"",
        )],
    )
    .await;
    assert_eq!(response.status(), 200);
    assert_eq!(body(response).await.as_ref(), multipart);
    let payload = br#"{"model":"a","unknown":"keep"}"#;
    use std::io::Write;
    let mut gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    gzip.write_all(payload).unwrap();
    let compressed = gzip.finish().unwrap();
    assert_eq!(
        body(
            request(
                &g,
                "/v1/responses",
                compressed.clone(),
                vec![
                    ("content-type", "application/json"),
                    ("content-encoding", "gzip")
                ]
            )
            .await
        )
        .await
        .as_ref(),
        compressed
    );
    let id = g.view().providers[0].id.clone();
    g.remember_model("context-a", &id, Some("a"));
    update(
        &g,
        &t,
        Edit::Mode {
            mode: "auto".into(),
        },
    );
    let inherited = request(
        &g,
        "/v1/responses",
        br#"{"previous_response_id":"context-a"}"#.to_vec(),
        vec![("content-type", "application/json")],
    )
    .await;
    assert_eq!(inherited.status(), 200);
    body(inherited).await;
    let denied = request(
        &g,
        "/v1/responses",
        br#"{"previous_response_id":"context-a","model":"b"}"#.to_vec(),
        vec![("content-type", "application/json")],
    )
    .await;
    assert_eq!(denied.status(), 400);
    g.stop().await.unwrap();
}

#[tokio::test]
async fn waiting_requests_recheck_rules_without_resetting_occupied_slots() {
    let (t, g) = fixture(vec!["https://example.invalid".into()]).await;
    let id = g.view().providers[0].id.clone();
    update(
        &g,
        &t,
        Edit::ConcurrencyProvider {
            id: id.clone(),
            max_concurrency: 1,
        },
    );
    allow(&g, &t, 0, Some(vec!["a"]));
    start(&g, &t).await;
    let routes = vec![g.route(&id).unwrap()];
    let held =
        g.0.admission
            .acquire_for(
                &routes,
                false,
                100,
                &mut admission::Budget::new(5),
                &Requirement::model(Some("a")),
            )
            .await
            .unwrap();
    let next = g.clone();
    let queued_routes = routes.clone();
    let wait = tokio::spawn(async move {
        next.0
            .admission
            .acquire_for(
                &queued_routes,
                false,
                100,
                &mut admission::Budget::new(5),
                &Requirement::model(Some("a")),
            )
            .await
    });
    tokio::time::timeout(Duration::from_secs(2), async {
        while g.view().waiting_requests != 1 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    allow(&g, &t, 0, Some(vec!["b"]));
    assert!(matches!(
        wait.await.unwrap(),
        Err(admission::Rejected::Model)
    ));
    assert_eq!(g.view().providers[0].active_requests, 1);
    assert_eq!(g.view().providers[0].health.requests, 0);
    drop(held);
    assert_eq!(g.view().providers[0].active_requests, 0);
    g.stop().await.unwrap();
}

#[tokio::test]
async fn catalog_is_cached_singleflight_bounded_and_has_no_business_side_effects() {
    let count = Arc::new(AtomicUsize::new(0));
    let hits = count.clone();
    let port = server(move |r| { let hits = hits.clone(); async move {
        assert_eq!(r.uri().path(),"/tenant/v1/models");
        assert_eq!(r.headers()["authorization"],"Bearer upstream-fixture-token");
        hits.fetch_add(1,Ordering::Relaxed); tokio::time::sleep(Duration::from_millis(60)).await;
        Response::new(full(r#"{"data":[{"id":"b"},{"id":"a"},{"id":"a"},{"id":"upstream-fixture-token"}],"ignored":"secret"}"#))
    }}).await;
    let (t, g) = fixture(vec![format!("http://127.0.0.1:{port}/tenant/v1")]).await;
    let id = g.view().providers[0].id.clone();
    let auth = std::fs::read(t.path().join("auth.json")).unwrap();
    let config = std::fs::read(t.path().join("config.toml")).unwrap();
    let (a, b) = tokio::join!(g.list_models(&id, true), g.list_models(&id, true));
    assert_eq!(a.unwrap().models, vec!["a", "b"]);
    assert_eq!(b.unwrap().models, vec!["a", "b"]);
    g.list_models(&id, false).await.unwrap();
    assert_eq!(count.load(Ordering::Relaxed), 1);
    assert_eq!(g.view().providers[0].health.requests, 0);
    assert_eq!(g.view().active_connections, 0);
    assert_eq!(std::fs::read(t.path().join("auth.json")).unwrap(), auth);
    assert_eq!(std::fs::read(t.path().join("config.toml")).unwrap(), config);
}

#[tokio::test]
async fn catalog_errors_and_changed_credentials_do_not_expose_raw_responses() {
    for status in [301, 401, 403, 429, 500] {
        let port = server(move |_| async move {
            Response::builder()
                .status(status)
                .header("retry-after", "120")
                .body(full("private-upstream-response"))
                .unwrap()
        })
        .await;
        let (t, g) = fixture(vec![format!("http://127.0.0.1:{port}")]).await;
        let id = g.view().providers[0].id.clone();
        let result = g.list_models(&id, true).await.unwrap();
        assert!(result.error.is_some());
        assert!(!serde_json::to_string(&result)
            .unwrap()
            .contains("private-upstream"));
        if status == 429 {
            assert!(result.retry_at.unwrap() >= quota::now() + 119);
        }
        assert_eq!(g.view().providers[0].health.failures, 0);
        drop(t);
    }
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    let entered = Arc::new(tokio::sync::Notify::new());
    let r = gate.clone();
    let e = entered.clone();
    let port = server(move |_| {
        let r = r.clone();
        let e = e.clone();
        async move {
            e.notify_one();
            let _ = r.acquire().await.unwrap();
            Response::new(full(r#"{"data":[{"id":"old-model"}]}"#))
        }
    })
    .await;
    let (t, g) = fixture(vec![format!("http://127.0.0.1:{port}")]).await;
    let id = g.view().providers[0].id.clone();
    let next = g.clone();
    let task_id = id.clone();
    let task = tokio::spawn(async move { next.list_models(&task_id, true).await });
    entered.notified().await;
    update(
        &g,
        &t,
        Edit::SaveProvider {
            id: Some(id),
            base_url: format!("http://127.0.0.1:{port}"),
            token: "changed-fixture".into(),
        },
    );
    gate.add_permits(1);
    assert_eq!(task.await.unwrap().unwrap_err().code, "STALE");
}

#[tokio::test]
async fn completed_http_response_keeps_model_affinity_and_health() {
    let port = server(|req| async {
        let _ = req.into_body().collect().await;
        Response::new(full(r#"{"id":"response-normal"}"#))
    })
    .await;
    let (t, g) = fixture(vec![format!("http://127.0.0.1:{port}")]).await;
    allow(&g, &t, 0, Some(vec!["allowed"]));
    start(&g, &t).await;
    let first = request(
        &g,
        "/v1/responses",
        br#"{"model":"allowed"}"#.to_vec(),
        vec![("content-type", "application/json")],
    )
    .await;
    body(first).await;
    assert_eq!(g.view().providers[0].health.requests, 1);
    assert_eq!(g.view().providers[0].health.failures, 0);
    let inherited = request(
        &g,
        "/v1/responses",
        br#"{"previous_response_id":"response-normal"}"#.to_vec(),
        vec![("content-type", "application/json")],
    )
    .await;
    assert_eq!(inherited.status(), 200);
    body(inherited).await;
    let unknown_ws = request(
        &g,
        "/future/socket",
        vec![],
        vec![
            ("upgrade", "websocket"),
            ("connection", "upgrade"),
            ("sec-websocket-key", "dGhlIHNhbXBsZSBub25jZQ=="),
            ("sec-websocket-version", "13"),
        ],
    )
    .await;
    assert_eq!(unknown_ws.status(), 400);
    assert!(String::from_utf8(body(unknown_ws).await.to_vec())
        .unwrap()
        .contains("MODEL_UNDETERMINED"));
    g.stop().await.unwrap();
}

#[tokio::test]
async fn catalog_uses_remote_dns_and_never_falls_back_after_proxy_failure() {
    let calls = Arc::new(AtomicUsize::new(0));
    let hits = calls.clone();
    let port = server(move |_| {
        hits.fetch_add(1, Ordering::Relaxed);
        async { Response::new(full(r#"{"data":[{"id":"remote"}]}"#)) }
    })
    .await;
    let (proxy, names, _) = socks(true).await;
    let (t, g) = fixture(vec![format!(
        "http://must-resolve-remotely.invalid:{port}/tenant"
    )])
    .await;
    update(
        &g,
        &t,
        Edit::SaveProxy {
            id: None,
            name: "fixture".into(),
            host: "127.0.0.1".into(),
            port: proxy,
            username: "fixture-user".into(),
            password: "fixture-password".into(),
        },
    );
    let pid = g.view().proxies[0].id.clone();
    let id = g.view().providers[0].id.clone();
    update(
        &g,
        &t,
        Edit::RouteProvider {
            id: id.clone(),
            proxy_id: Some(pid.clone()),
        },
    );
    assert_eq!(
        g.list_models(&id, true).await.unwrap().models,
        vec!["remote"]
    );
    assert_eq!(names.lock().unwrap()[0], "must-resolve-remotely.invalid");
    update(
        &g,
        &t,
        Edit::SaveProxy {
            id: Some(pid),
            name: "fixture".into(),
            host: "127.0.0.1".into(),
            port: proxy,
            username: "fixture-user".into(),
            password: "wrong".into(),
        },
    );
    let failed = g.list_models(&id, true).await.unwrap();
    assert!(failed.error.is_some());
    assert!(failed.models.is_empty());
    assert_eq!(calls.load(Ordering::Relaxed), 1);
    assert_eq!(g.view().providers[0].health.failures, 0);
}

#[tokio::test]
async fn catalog_limits_response_size_and_request_duration() {
    let port = server(|_| async { Response::new(full(vec![b'x'; 2_000_001])) }).await;
    let (_t, g) = fixture(vec![format!("http://127.0.0.1:{port}")]).await;
    let id = g.view().providers[0].id.clone();
    assert!(g
        .list_models(&id, true)
        .await
        .unwrap()
        .error
        .unwrap()
        .contains("2 MB"));
    let port = server(|_| async {
        tokio::time::sleep(Duration::from_secs(15)).await;
        Response::new(full("{}"))
    })
    .await;
    let (_t, g) = fixture(vec![format!("http://127.0.0.1:{port}")]).await;
    let id = g.view().providers[0].id.clone();
    assert!(g
        .list_models(&id, true)
        .await
        .unwrap()
        .error
        .unwrap()
        .contains("超时"));
}

#[tokio::test]
#[ignore = "Read-only model discovery against explicitly supplied private providers and Tencent proxy"]
async fn real_provider_models_direct_and_tencent_proxy() {
    let path = std::env::var_os("GPT_SWITCH_MODELS_FIXTURE").expect("private fixture required");
    let store: Store = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let proxy = store.proxies.first().expect("Tencent proxy required");
    assert!(!store.providers.is_empty());
    let mut passed = 0;
    for (index, p) in store.providers.iter().enumerate() {
        let (t, g) = fixture(vec![]).await;
        update(
            &g,
            &t,
            Edit::SaveProvider {
                id: None,
                base_url: p.base_url.clone(),
                token: p.token.clone(),
            },
        );
        let id = g.view().providers[0].id.clone();
        let auth = std::fs::read(t.path().join("auth.json")).unwrap();
        let config = std::fs::read(t.path().join("config.toml")).unwrap();
        for route in ["direct", "proxy"] {
            if route == "proxy" {
                update(
                    &g,
                    &t,
                    Edit::SaveProxy {
                        id: None,
                        name: "Tencent acceptance".into(),
                        host: proxy.host.clone(),
                        port: proxy.port,
                        username: proxy.username.clone(),
                        password: proxy.password.clone(),
                    },
                );
                update(
                    &g,
                    &t,
                    Edit::RouteProvider {
                        id: id.clone(),
                        proxy_id: Some(g.view().proxies[0].id.clone()),
                    },
                );
            }
            let result = g.list_models(&id, true).await.unwrap();
            println!(
                "provider={} route={} models={} success={} error={}",
                index + 1,
                route,
                result.models.len(),
                result.error.is_none(),
                result.error.as_deref().unwrap_or("none")
            );
            if result.error.is_none() && !result.models.is_empty() {
                passed += 1;
            }
            assert_eq!(g.view().providers[0].health.requests, 0);
            assert_eq!(std::fs::read(t.path().join("auth.json")).unwrap(), auth);
            assert_eq!(std::fs::read(t.path().join("config.toml")).unwrap(), config);
        }
    }
    assert!(passed > 0, "No real provider returned a model catalog");
}
