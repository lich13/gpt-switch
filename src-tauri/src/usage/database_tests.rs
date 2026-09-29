use super::*;
use crate::pricing::Cost;

fn record(id: &str, attempt: usize, provider: &str, outcome: &str, status: u16, at: u64) -> Record {
    Record {
        response_id: None,
        id: format!("{id}-{attempt}"),
        logical_id: id.into(),
        attempt,
        provider_id: provider.into(),
        provider_name: format!("Name {provider}"),
        request_model: Some("requested".into()),
        response_model: Some(format!("model-{provider}")),
        billing_model: Some(format!("model-{provider}")),
        service_tier: None,
        model_source: "response".into(),
        multiplier: "1".into(),
        created_at: at,
        status: Some(status),
        outcome: outcome.into(),
        streaming: true,
        latency_ms: 100,
        first_token_ms: Some(10),
        duration_ms: Some(90),
        tokens: Tokens {
            input: Some(10),
            output: Some(5),
            ..Default::default()
        },
        incomplete: false,
        cost: Cost {
            status: "priced".into(),
            total: Some("0.25".into()),
            version: "frozen-v1".into(),
            ..Default::default()
        },
        routing: vec![],
        terminal_evidence: None,
    }
}
fn complete(db: &Connection, rows: &[Record]) {
    let mut pending = rows[0].clone();
    pending.outcome = "IN_PROGRESS".into();
    logical(db, &pending, false).unwrap();
    for r in rows {
        insert(db, r, true).unwrap();
    }
    let mut final_row = rows.last().unwrap().clone();
    final_row.created_at = rows[0].created_at;
    logical(db, &final_row, true).unwrap();
}

#[test]
fn logical_filters_use_final_provider_but_charge_all_attempts_once() {
    let t = tempfile::tempdir().unwrap();
    let path = t.path().join("usage.sqlite");
    let db = initialize(&path).unwrap();
    let at = pricing::now();
    let a = record("retry", 0, "p1", "HTTP", 502, at);
    let b = record("retry", 1, "p2", "OK", 200, at + 1);
    complete(&db, &[a.clone(), b.clone()]);
    insert(&db, &b, true).unwrap(); // Duplicate completion must not add usage.
    let mut late = b.clone();
    late.outcome = "CANCELLED".into();
    logical(&db, &late, true).unwrap();
    let f = Filters {
        provider_id: Some("p2".into()),
        model: Some("model-p2".into()),
        ..Default::default()
    };
    let d = dashboard(&path, f.clone()).unwrap();
    assert_eq!(
        (
            d.summary.requests,
            d.summary.successes,
            d.summary.failures,
            d.summary.attempts
        ),
        (1, 1, 0, 2)
    );
    assert_eq!(d.summary.tokens.total(), 30);
    assert_eq!(d.summary.cost.as_deref(), Some("0.5"));
    let p1 = &d.providers.iter().find(|p| p.id == "p1").unwrap().summary;
    assert_eq!((p1.requests, p1.attempts), (0, 1));
    assert_eq!(p1.cost.as_deref(), Some("0.25"));
    assert_eq!(
        d.providers
            .iter()
            .find(|p| p.id == "p2")
            .unwrap()
            .summary
            .requests,
        1
    );
    assert_eq!(logs(&path, f, 0).unwrap().total, 1);
    assert_eq!(
        dashboard(
            &path,
            Filters {
                provider_id: Some("p1".into()),
                ..Default::default()
            }
        )
        .unwrap()
        .summary
        .requests,
        0
    );
    let detail = detail(&path, "retry").unwrap().unwrap();
    assert_eq!(detail.attempts.len(), 2);
    assert_eq!(detail.summary.outcome_class, "success");
    assert_eq!(detail.attempts[0].cost.version, "frozen-v1");
}

#[test]
fn migration_preserves_historical_cancellation_and_marks_crash_unknown() {
    let t = tempfile::tempdir().unwrap();
    let path = t.path().join("usage.sqlite");
    let legacy = Connection::open(&path).unwrap();
    legacy.execute_batch("PRAGMA user_version=1; CREATE TABLE requests(id TEXT PRIMARY KEY,at INTEGER NOT NULL,provider TEXT NOT NULL,model TEXT NOT NULL,status INTEGER,finished INTEGER NOT NULL,payload TEXT NOT NULL)").unwrap();
    let at = pricing::now();
    for (r, finished) in [
        (record("retry", 0, "p1", "HTTP", 502, at), true),
        (record("retry", 1, "p2", "OK", 200, at + 1), true),
        (record("old-cancel", 0, "p1", "CANCELLED", 200, at), true),
        (record("crash", 0, "p1", "IN_PROGRESS", 200, at), false),
    ] {
        legacy
            .execute(
                "INSERT INTO requests VALUES(?,?,?,?,?,?,?)",
                params![
                    r.id,
                    r.created_at,
                    r.provider_id,
                    r.model(),
                    r.status,
                    finished,
                    json(&r).unwrap()
                ],
            )
            .unwrap();
    }
    drop(legacy);
    let db = initialize(&path).unwrap();
    assert_eq!(
        db.query_row("PRAGMA user_version", [], |r| r.get::<_, u32>(0))
            .unwrap(),
        3
    );
    let d = dashboard(&path, Filters::default()).unwrap().summary;
    assert_eq!(
        (
            d.requests,
            d.successes,
            d.failures,
            d.cancelled,
            d.unknown,
            d.attempts
        ),
        (3, 1, 0, 1, 1, 4)
    );
    assert_eq!(
        detail(&path, "old-cancel")
            .unwrap()
            .unwrap()
            .summary
            .record
            .outcome,
        "CANCELLED"
    );
    drop(db);
    let _reopen = initialize(&path).unwrap();
    assert_eq!(
        dashboard(&path, Filters::default())
            .unwrap()
            .summary
            .tokens
            .total(),
        60
    );
}

#[test]
fn terminal_classes_and_pagination_use_inbound_time_without_counting_attempt_rows() {
    let t = tempfile::tempdir().unwrap();
    let path = t.path().join("usage.sqlite");
    let db = initialize(&path).unwrap();
    let at = pricing::now();
    for i in 0..25 {
        complete(&db, &[record(&format!("r{i}"), 0, "p1", "OK", 200, at + i)]);
    }
    for (id, outcome, status) in [
        ("capacity", "CAPACITY", 429),
        ("no-provider", "NO_PROVIDER", 503),
        ("cancel", "CANCELLED", 200),
        ("unknown", "UNKNOWN_TERMINAL", 200),
    ] {
        let mut r = record(id, 0, "", outcome, status, at + 30);
        r.tokens = Tokens::default();
        r.cost = Cost::default();
        logical(&db, &r, true).unwrap();
    }
    let mut pending = record("pending", 0, "p1", "IN_PROGRESS", 200, at + 31);
    pending.tokens = Tokens::default();
    logical(&db, &pending, false).unwrap();
    let f = Filters {
        start: Some(at),
        end: Some(at + 25),
        ..Default::default()
    };
    assert_eq!(logs(&path, f.clone(), 0).unwrap().records.len(), 20);
    assert_eq!(logs(&path, f.clone(), 1).unwrap().records.len(), 5);
    assert_eq!(logs(&path, f, 1).unwrap().total, 25);
    let d = dashboard(&path, Filters::default()).unwrap().summary;
    assert_eq!(
        (
            d.requests,
            d.successes,
            d.failures,
            d.rejected,
            d.cancelled,
            d.unknown,
            d.pending
        ),
        (30, 25, 1, 1, 1, 1, 1)
    );
    assert_eq!(d.gateway_errors, 1);
    assert_eq!(
        logs(
            &path,
            Filters {
                outcome: Some("rejected".into()),
                ..Default::default()
            },
            0
        )
        .unwrap()
        .total,
        1
    );
    drop(db);
    let _db = initialize(&path).unwrap();
    assert_eq!(
        dashboard(&path, Filters::default())
            .unwrap()
            .summary
            .unknown,
        2
    );
}

#[test]
fn daily_rollup_retains_logical_results_attempt_charges_and_price_snapshots() {
    let t = tempfile::tempdir().unwrap();
    let path = t.path().join("usage.sqlite");
    let mut db = initialize(&path).unwrap();
    let at = pricing::now() - 40 * 86400;
    for id in ["a", "b"] {
        complete(
            &db,
            &[
                record(id, 0, "p1", "HTTP", 503, at),
                record(id, 1, "p2", "OK", 200, at),
            ],
        );
    }
    let before = dashboard(&path, Filters::default()).unwrap().summary;
    prune(&mut db, 30).unwrap();
    prune(&mut db, 30).unwrap();
    let d = dashboard(
        &path,
        Filters {
            start: Some(at),
            end: Some(day(at).1),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(d.precision, "day");
    assert_eq!(d.effective_start, Some(day(at).0));
    assert_eq!(
        (d.summary.requests, d.summary.successes, d.summary.attempts),
        (2, 2, 4)
    );
    assert_eq!(d.summary.tokens, before.tokens);
    assert_eq!(d.summary.cost, before.cost);
    assert_eq!(
        d.providers
            .iter()
            .find(|p| p.id == "p1")
            .unwrap()
            .summary
            .attempts,
        2
    );
    assert_eq!(logs(&path, Filters::default(), 0).unwrap().total, 0);
    let payload: String = db
        .query_row("SELECT payload FROM logical_rollups LIMIT 1", [], |r| {
            r.get(0)
        })
        .unwrap();
    let archived: Archived = parse(&payload).unwrap();
    assert_eq!(archived.attempts[0].cost.version, "frozen-v1");
    assert!(archived.attempts[0].logical_id.is_empty());
}

#[test]
fn legacy_rollups_never_enter_logical_success_rate_and_remain_filterable() {
    let t = tempfile::tempdir().unwrap();
    let path = t.path().join("usage.sqlite");
    let db = initialize(&path).unwrap();
    let at = day(pricing::now() - 50 * 86400).0;
    let r = record("legacy", 0, "deleted", "CANCELLED", 200, at);
    db.execute(
        "INSERT INTO rollups VALUES('old',?,?,?,?,?,7,700,?)",
        params![
            at,
            day(at).1,
            r.provider_id,
            r.model(),
            r.status,
            json(&r).unwrap()
        ],
    )
    .unwrap();
    let d = dashboard(&path, Filters::default()).unwrap();
    assert_eq!(d.summary.requests, 0);
    assert_eq!(d.summary.failures, 0);
    assert_eq!(d.summary.legacy_attempts, 7);
    assert_eq!(d.summary.tokens.total(), 105);
    assert!(d.available_providers.iter().any(|(id, _)| id == "deleted"));
    assert_eq!(
        dashboard(
            &path,
            Filters {
                outcome: Some("success".into()),
                ..Default::default()
            }
        )
        .unwrap()
        .summary
        .legacy_attempts,
        0
    );
}

#[test]
fn writes_and_daily_pruning_roll_back_as_one_transaction() {
    let t = tempfile::tempdir().unwrap();
    let path = t.path().join("usage.sqlite");
    let mut db = initialize(&path).unwrap();
    let r = record("atomic", 0, "p1", "OK", 200, pricing::now() - 40 * 86400);
    db.execute_batch("CREATE TRIGGER fail_logical BEFORE INSERT ON logical_requests BEGIN SELECT RAISE(ABORT,'fixture failure'); END").unwrap();
    assert!(insert(&db, &r, true).is_err());
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM requests", [], |r| r.get::<_, u32>(0))
            .unwrap(),
        0
    );
    db.execute_batch("DROP TRIGGER fail_logical").unwrap();
    complete(&db, &[r]);
    db.execute_batch("CREATE TRIGGER fail_prune BEFORE DELETE ON logical_requests BEGIN SELECT RAISE(ABORT,'fixture failure'); END").unwrap();
    assert!(prune(&mut db, 30).is_err());
    assert_eq!(logs(&path, Filters::default(), 0).unwrap().total, 1);
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM logical_rollups", [], |r| r
            .get::<_, u32>(0))
            .unwrap(),
        0
    );
    assert_eq!(detail(&path, "atomic").unwrap().unwrap().attempts.len(), 1);
}

#[test]
fn backfill_updates_unpriced_attempts_and_rollups_but_preserves_paid_history() {
    let t = tempfile::tempdir().unwrap();
    let path = t.path().join("usage.sqlite");
    let mut db = initialize(&path).unwrap();
    let price = pricing::model::Price {
        model_id: "model-p1".into(),
        display_name: "test".into(),
        source: "fixture".into(),
        fixed: false,
        rates: pricing::model::Rates {
            input: Some("2".into()),
            output: Some("4".into()),
            ..Default::default()
        },
        variants: vec![],
    };
    let prices = pricing::Snapshot {
        prices: BTreeMap::from([("model-p1".into(), price)]),
        version: "v2".into(),
    };
    for (id, at) in [
        ("recent", pricing::now()),
        ("old", pricing::now() - 40 * 86400),
    ] {
        let mut r = record(id, 0, "p1", "OK", 200, at);
        r.cost = Cost {
            status: "unpriced".into(),
            ..Default::default()
        };
        complete(&db, &[r]);
    }
    complete(&db, &[record("paid", 0, "p1", "OK", 200, pricing::now())]);
    prune(&mut db, 30).unwrap();
    backfill(&mut db, &prices).unwrap();
    backfill(&mut db, &prices).unwrap();
    assert_eq!(
        detail(&path, "paid").unwrap().unwrap().attempts[0]
            .cost
            .version,
        "frozen-v1"
    );
    assert_eq!(
        detail(&path, "recent").unwrap().unwrap().attempts[0]
            .cost
            .version,
        "v2"
    );
    let d = dashboard(&path, Filters::default()).unwrap().summary;
    assert_eq!(d.unpriced, 0);
    assert_eq!(d.cost.as_deref(), Some("0.25008"));
    assert_eq!(d.requests, 3);
}

#[test]
#[ignore = "Requires an explicitly prepared private database copy; never opens the production database"]
fn migration_of_explicitly_supplied_database_copy() {
    let path = std::path::PathBuf::from(
        std::env::var_os("GPT_SWITCH_USAGE_COPY").expect("private copy path"),
    );
    let canonical = path.canonicalize().unwrap();
    let temporary = std::env::temp_dir().canonicalize().unwrap();
    assert!(
        canonical.starts_with(&temporary)
            || std::path::Path::new("/tmp")
                .canonicalize()
                .is_ok_and(|p| canonical.starts_with(p))
    );
    assert_eq!(canonical.file_name().unwrap(), "usage-migration.sqlite");
    let before = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    let payloads: Vec<String> = before
        .prepare("SELECT payload FROM requests ORDER BY id")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<std::result::Result<_, _>>()
        .unwrap();
    let records: Vec<Record> = payloads.iter().map(|p| parse(p).unwrap()).collect();
    let count = records
        .iter()
        .map(|r| r.logical_id.as_str())
        .collect::<BTreeSet<_>>()
        .len();
    let snapshots: Vec<_> = records.iter().map(|r| json(&r.cost).unwrap()).collect();
    drop(before);
    let db = initialize(&path).unwrap();
    let after: Vec<String> = db
        .prepare("SELECT payload FROM requests ORDER BY id")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<std::result::Result<_, _>>()
        .unwrap();
    assert_eq!(records.len(), after.len());
    assert_eq!(
        snapshots,
        after
            .iter()
            .map(|s| json(&parse::<Record>(s).unwrap().cost).unwrap())
            .collect::<Vec<_>>()
    );
    let d = dashboard(&path, Filters::default()).unwrap();
    assert_eq!(d.summary.requests, count as u64);
    assert_eq!(
        d.summary.requests,
        d.summary.successes
            + d.summary.failures
            + d.summary.rejected
            + d.summary.cancelled
            + d.summary.pending
            + d.summary.unknown
    );
    println!("isolated migration: {} attempts, {} logical requests, {} cancelled, {} service failures, semantics v{}",records.len(),d.summary.requests,d.summary.cancelled,d.summary.failures,d.semantics_version);
}

#[test]
fn cc_logs_project_final_attempt_without_changing_logical_accounting() {
    let t = tempfile::tempdir().unwrap();
    let path = t.path().join("usage.sqlite");
    let db = initialize(&path).unwrap();
    let at = pricing::now();
    let a = record("retry", 0, "p1", "HTTP", 429, at);
    let mut b = record("retry", 1, "p2", "CANCELLED", 200, at + 1);
    b.tokens.input = Some(100);
    b.cost.total = Some("0.75".into());
    b.response_id = Some("resp_same".into());
    complete(&db, &[a, b.clone()]);
    let page = logs(&path, Filters::default(), 0).unwrap();
    assert_eq!(page.total, 1);
    let row = &page.records[0];
    assert_eq!(row.record.status, Some(200));
    assert_eq!(row.record.provider_id, "p2");
    assert_eq!(row.record.tokens.input, Some(100));
    assert_eq!(row.record.cost.total.as_deref(), Some("0.75"));
    assert_eq!(row.data_source, "proxy");
    let detail = detail(&path, &row.record.id).unwrap().unwrap();
    assert_eq!(detail.attempts.len(), 2);
    assert_eq!(detail.summary.record.tokens.input, Some(110));
    assert_eq!(detail.summary.record.cost.total.as_deref(), Some("1"));
    assert_eq!(detail.summary.outcome_class, "cancelled");
    let mut duplicate = b.clone();
    duplicate.logical_id = "again".into();
    duplicate.id = "again-0".into();
    duplicate.attempt = 0;
    complete(&db, &[duplicate]);
    assert_eq!(logs(&path, Filters::default(), 0).unwrap().total, 1);
    let d = dashboard(&path, Filters::default()).unwrap().summary;
    assert_eq!((d.requests, d.cancelled, d.attempts), (2, 2, 3));
    b.logical_id = "other-provider".into();
    b.id = "other-provider-0".into();
    b.provider_id = "p3".into();
    complete(&db, &[b]);
    assert_eq!(logs(&path, Filters::default(), 0).unwrap().total, 2);
    let pending = record("pending", 0, "p1", "IN_PROGRESS", 200, at);
    logical(&db, &pending, false).unwrap();
    assert_eq!(logs(&path, Filters::default(), 0).unwrap().total, 2);
    let before: Vec<String> = {
        let mut q = db
            .prepare("SELECT payload FROM requests ORDER BY id")
            .unwrap();
        q.query_map([], |r| r.get(0))
            .unwrap()
            .collect::<std::result::Result<_, _>>()
            .unwrap()
    };
    drop(db);
    let db = initialize(&path).unwrap();
    let after: Vec<String> = {
        let mut q = db
            .prepare("SELECT payload FROM requests ORDER BY id")
            .unwrap();
        q.query_map([], |r| r.get(0))
            .unwrap()
            .collect::<std::result::Result<_, _>>()
            .unwrap()
    };
    assert_eq!(before, after);
}
#[test]
fn v2_log_migration_preserves_snapshots_and_unknowns() {
    let t = tempfile::tempdir().unwrap();
    let path = t.path().join("usage.sqlite");
    let db = initialize(&path).unwrap();
    let mut r = record("old", 0, "p1", "INTERRUPTED", 200, pricing::now());
    r.status = None;
    r.tokens = Tokens::default();
    r.cost.total = None;
    complete(&db, &[r]);
    let original: String = db
        .query_row("SELECT final_payload FROM logical_requests", [], |r| {
            r.get(0)
        })
        .unwrap();
    db.execute_batch("DROP INDEX logical_log_key; ALTER TABLE logical_requests DROP COLUMN log_key; PRAGMA user_version=2;").unwrap();
    drop(db);
    let db = initialize(&path).unwrap();
    let final_payload: String = db
        .query_row("SELECT final_payload FROM logical_requests", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(original, final_payload);
    let page = logs(&path, Filters::default(), 0).unwrap();
    assert_eq!(page.total, 1);
    assert_eq!(page.records[0].record.status, None);
    assert_eq!(page.records[0].record.tokens.input, None);
}
