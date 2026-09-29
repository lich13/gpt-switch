use super::{parser::Tokens, LogicalDetail, LogicalRecord, Record};
use crate::{
    pricing::{
        self,
        model::{calculate, decimal},
    },
    storage::{self, AppError, Result},
};
use chrono::{Local, TimeZone};
use rusqlite::{params, params_from_iter, types::Value, Connection, OpenFlags};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};
fn error(_: impl std::fmt::Display) -> AppError {
    AppError::new("USAGE_DB", "统计数据库操作失败")
}
fn json<T: Serialize>(value: &T) -> Result<String> {
    serde_json::to_string(value).map_err(error)
}
fn parse<T: serde::de::DeserializeOwned>(value: &str) -> Result<T> {
    serde_json::from_str(value).map_err(error)
}

pub fn initialize(path: &Path) -> Result<Connection> {
    if std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err(AppError::new("USAGE_DB", "统计数据库不能是符号链接"));
    }
    let mut db = Connection::open(path).map_err(error)?;
    storage::protect(path, false)?;
    db.busy_timeout(std::time::Duration::from_secs(3))
        .map_err(error)?;
    let version: u32 = db
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .map_err(error)?;
    if version > 2 {
        return Err(AppError::new("USAGE_DB", "统计数据库版本较新，请升级应用"));
    }
    db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;
        CREATE TABLE IF NOT EXISTS requests(id TEXT PRIMARY KEY, at INTEGER NOT NULL, provider TEXT NOT NULL,
        model TEXT NOT NULL, status INTEGER, finished INTEGER NOT NULL, payload TEXT NOT NULL);
        CREATE INDEX IF NOT EXISTS usage_time ON requests(at);
        CREATE INDEX IF NOT EXISTS usage_provider_model ON requests(provider,model,at);
        CREATE TABLE IF NOT EXISTS rollups(id TEXT PRIMARY KEY, at INTEGER NOT NULL, ends INTEGER NOT NULL,
        provider TEXT NOT NULL, model TEXT NOT NULL, status INTEGER, count INTEGER NOT NULL, latency INTEGER NOT NULL, payload TEXT NOT NULL);
        CREATE INDEX IF NOT EXISTS rollup_time ON rollups(at,ends);").map_err(error)?;
    let tx = db.transaction().map_err(error)?;
    if version < 2 {
        tx.execute_batch("ALTER TABLE requests ADD COLUMN logical_id TEXT NOT NULL DEFAULT ''; ")
            .map_err(error)?;
    }
    tx.execute_batch("CREATE INDEX IF NOT EXISTS usage_logical ON requests(logical_id);
        CREATE TABLE IF NOT EXISTS logical_requests(id TEXT PRIMARY KEY,at INTEGER NOT NULL,provider TEXT NOT NULL,model TEXT NOT NULL,
        status INTEGER,class TEXT NOT NULL,finished INTEGER NOT NULL,explicit INTEGER NOT NULL,final_payload TEXT NOT NULL,payload TEXT NOT NULL);
        CREATE INDEX IF NOT EXISTS logical_time ON logical_requests(at);
        CREATE INDEX IF NOT EXISTS logical_filters ON logical_requests(provider,model,at);
        CREATE TABLE IF NOT EXISTS logical_rollups(id TEXT PRIMARY KEY,at INTEGER NOT NULL,ends INTEGER NOT NULL,provider TEXT NOT NULL,
        model TEXT NOT NULL,status INTEGER,class TEXT NOT NULL,count INTEGER NOT NULL,latency INTEGER NOT NULL,payload TEXT NOT NULL);
        CREATE INDEX IF NOT EXISTS logical_rollup_time ON logical_rollups(at,ends);").map_err(error)?;
    let mut changed = BTreeSet::new();
    let rows: Vec<(String, String, bool)> = {
        let mut q = tx
            .prepare(if version < 2 {
                "SELECT id,payload,finished FROM requests"
            } else {
                "SELECT id,payload,finished FROM requests WHERE finished=0"
            })
            .map_err(error)?;
        let collected = q
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .map_err(error)?
            .collect::<std::result::Result<_, _>>()
            .map_err(error)?;
        collected
    };
    for (id, payload, finished) in rows {
        let mut r: Record = parse(&payload)?;
        if r.logical_id.is_empty() {
            r.logical_id = id.clone();
        }
        if !finished {
            r.outcome = "INTERRUPTED".into();
            r.incomplete = true;
        }
        tx.execute(
            "UPDATE requests SET logical_id=?,payload=?,finished=1 WHERE id=?",
            params![r.logical_id, json(&r)?, id],
        )
        .map_err(error)?;
        changed.insert(r.logical_id);
    }
    for id in changed {
        rebuild(&tx, &id)?;
    }
    let pending: Vec<(String, String)> = {
        let mut q = tx
            .prepare("SELECT id,final_payload FROM logical_requests WHERE finished=0")
            .map_err(error)?;
        let collected = q
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .map_err(error)?
            .collect::<std::result::Result<_, _>>()
            .map_err(error)?;
        collected
    };
    for (id, payload) in pending {
        let original: Record = parse(&payload)?;
        let mut r = attempts(&tx, &id)?
            .pop()
            .unwrap_or_else(|| original.clone());
        r.id = id.clone();
        r.logical_id = id;
        r.created_at = original.created_at;
        r.outcome = "INTERRUPTED".into();
        r.incomplete = true;
        write_logical(&tx, &r, true)?;
    }
    tx.execute_batch("PRAGMA user_version=2").map_err(error)?;
    tx.commit().map_err(error)?;
    Ok(db)
}
fn reader(path: &Path) -> Result<Connection> {
    let db = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(error)?;
    db.busy_timeout(std::time::Duration::from_secs(3))
        .map_err(error)?;
    Ok(db)
}
fn attempts(db: &Connection, id: &str) -> Result<Vec<Record>> {
    let mut q = db.prepare("SELECT payload FROM requests WHERE logical_id=? ORDER BY at,json_extract(payload,'$.attempt'),rowid").map_err(error)?;
    let payloads = q
        .query_map([id], |r| r.get::<_, String>(0))
        .map_err(error)?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(error)?;
    payloads.into_iter().map(|s| parse(&s)).collect()
}
fn summarize(mut final_record: Record, rows: &[Record]) -> LogicalRecord {
    let mut amounts = Aggregate::default();
    let mut attempt_count = 0;
    for r in rows.iter().filter(|r| !r.provider_id.is_empty()) {
        amounts.add_usage(r, 1);
        attempt_count += 1;
    }
    final_record.tokens = amounts.tokens;
    final_record.cost.total = amounts.cost;
    final_record.cost.status = if amounts.unpriced > 0 {
        "unpriced"
    } else if amounts.partial > 0 {
        "partial"
    } else if final_record.cost.total.is_none() {
        "unreported"
    } else {
        "priced"
    }
    .into();
    final_record.id = final_record.logical_id.clone();
    LogicalRecord {
        outcome_class: final_record.outcome_class().into(),
        record: final_record,
        attempt_count,
        semantics_version: 2,
    }
}
fn rebuild(db: &Connection, id: &str) -> Result<()> {
    use rusqlite::OptionalExtension;
    let rows = attempts(db, id)?;
    let existing: Option<(String, bool, bool)> = db
        .query_row(
            "SELECT final_payload,finished,explicit FROM logical_requests WHERE id=?",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()
        .map_err(error)?;
    let (mut final_record, finished, explicit) = if let Some((payload, finished, true)) = existing {
        (parse::<Record>(&payload)?, finished, true)
    } else if let Some(last) = rows.last() {
        let mut r = last.clone();
        r.created_at = rows[0].created_at;
        r.latency_ms = rows
            .iter()
            .map(|r| r.created_at.saturating_sub(rows[0].created_at) * 1000 + r.latency_ms)
            .max()
            .unwrap_or(0);
        (r, last.outcome != "IN_PROGRESS", false)
    } else {
        return Ok(());
    };
    if !finished {
        if let Some(last) = rows.last() {
            final_record.provider_id = last.provider_id.clone();
            final_record.provider_name = last.provider_name.clone();
            final_record.request_model = last.request_model.clone();
            final_record.response_model = last.response_model.clone();
            final_record.billing_model = last.billing_model.clone();
        }
        final_record.outcome = "IN_PROGRESS".into();
    }
    final_record.id = id.into();
    final_record.logical_id = id.into();
    let summary = summarize(final_record.clone(), &rows);
    db.execute("INSERT INTO logical_requests(id,at,provider,model,status,class,finished,explicit,final_payload,payload) VALUES(?,?,?,?,?,?,?,?,?,?)
        ON CONFLICT(id) DO UPDATE SET at=excluded.at,provider=excluded.provider,model=excluded.model,status=excluded.status,class=excluded.class,
        finished=excluded.finished,explicit=excluded.explicit,final_payload=excluded.final_payload,payload=excluded.payload",
        params![id,final_record.created_at,final_record.provider_id,final_record.model(),final_record.status,summary.outcome_class,finished,explicit,json(&final_record)?,json(&summary)?]).map_err(error)?;
    Ok(())
}
pub fn insert(db: &Connection, r: &Record, finished: bool) -> Result<()> {
    let tx = db.unchecked_transaction().map_err(error)?;
    tx.execute("INSERT INTO requests(id,at,provider,model,status,finished,payload,logical_id) VALUES(?,?,?,?,?,?,?,?)
        ON CONFLICT(id) DO UPDATE SET model=excluded.model,status=excluded.status,finished=excluded.finished,payload=excluded.payload WHERE requests.finished=0",
        params![r.id,r.created_at,r.provider_id,r.model(),r.status,finished,json(r)?,r.logical_id]).map_err(error)?;
    rebuild(&tx, &r.logical_id)?;
    tx.commit().map_err(error)
}
fn write_logical(db: &Connection, r: &Record, finished: bool) -> Result<()> {
    let summary = summarize(r.clone(), &[]);
    db.execute("INSERT INTO logical_requests(id,at,provider,model,status,class,finished,explicit,final_payload,payload) VALUES(?,?,?,?,?,?,?,1,?,?)
        ON CONFLICT(id) DO UPDATE SET finished=excluded.finished,explicit=1,final_payload=excluded.final_payload WHERE logical_requests.finished=0 OR logical_requests.explicit=0",
        params![r.logical_id,r.created_at,r.provider_id,r.model(),r.status,summary.outcome_class,finished,json(r)?,json(&summary)?]).map_err(error)?;
    rebuild(db, &r.logical_id)
}
pub fn logical(db: &Connection, r: &Record, finished: bool) -> Result<()> {
    let tx = db.unchecked_transaction().map_err(error)?;
    write_logical(&tx, r, finished)?;
    tx.commit().map_err(error)
}
#[derive(Default, Clone, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Filters {
    pub start: Option<u64>,
    pub end: Option<u64>,
    pub provider_id: Option<String>,
    pub model: Option<String>,
    pub status: Option<u16>,
    pub outcome: Option<String>,
}
fn predicates(f: &Filters, rollup: bool, alias: &str) -> (String, Vec<Value>) {
    let mut conditions = vec!["1=1".to_string()];
    let mut values = vec![];
    if let Some(start) = f.start {
        conditions.push(format!("{alias}at>=?"));
        values.push(Value::Integer(start.min(i64::MAX as u64) as i64));
    }
    if let Some(end) = f.end {
        conditions.push(format!(
            "{alias}{}",
            if rollup { "ends<=?" } else { "at<?" }
        ));
        values.push(Value::Integer(end.min(i64::MAX as u64) as i64));
    }
    if let Some(id) = &f.provider_id {
        conditions.push(format!("{alias}provider=?"));
        values.push(Value::Text(id.clone()));
    }
    if let Some(model) = &f.model {
        conditions.push(format!("{alias}model=?"));
        values.push(Value::Text(model.clone()));
    }
    if let Some(status) = f.status {
        conditions.push(format!("{alias}status=?"));
        values.push(Value::Integer(status as i64));
    }
    if let Some(outcome) = &f.outcome {
        conditions.push(format!("{alias}class=?"));
        values.push(Value::Text(outcome.clone()));
    }
    (conditions.join(" AND "), values)
}
fn day(at: u64) -> (u64, u64) {
    let date = Local
        .timestamp_opt(at.min(i64::MAX as u64) as i64, 0)
        .single()
        .unwrap_or_else(Local::now)
        .date_naive();
    let start = date
        .and_hms_opt(0, 0, 0)
        .unwrap()
        .and_local_timezone(Local)
        .earliest()
        .unwrap()
        .timestamp()
        .max(0) as u64;
    let end = date
        .succ_opt()
        .unwrap()
        .and_hms_opt(0, 0, 0)
        .unwrap()
        .and_local_timezone(Local)
        .earliest()
        .unwrap()
        .timestamp()
        .max(0) as u64;
    (start, end)
}
#[derive(Default, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Aggregate {
    pub requests: u64,
    pub successes: u64,
    pub failures: u64,
    pub rejected: u64,
    pub cancelled: u64,
    pub unknown: u64,
    pub pending: u64,
    pub attempts: u64,
    pub tokens: Tokens,
    pub cost: Option<String>,
    pub unpriced: u64,
    pub partial: u64,
    pub missing_usage: u64,
    pub latency_ms: u64,
    pub gateway_errors: u64,
    pub legacy_attempts: u64,
}
impl Aggregate {
    fn add_result(&mut self, r: &LogicalRecord, count: u64, latency: u64) {
        self.requests += count;
        self.latency_ms += latency;
        match r.outcome_class.as_str() {
            "success" => self.successes += count,
            "failure" => self.failures += count,
            "rejected" => self.rejected += count,
            "cancelled" => self.cancelled += count,
            "pending" => self.pending += count,
            _ => self.unknown += count,
        }
        if r.record.provider_id.is_empty() && r.outcome_class == "failure" {
            self.gateway_errors += count;
        }
    }
    fn add_usage(&mut self, r: &Record, count: u64) {
        macro_rules! tokens {($($f:ident),*)=>{$(if let Some(n)=r.tokens.$f {self.tokens.$f=Some(self.tokens.$f.unwrap_or(0).saturating_add(n.saturating_mul(count)));})*};}
        tokens!(
            input,
            output,
            cache_read,
            cache_write,
            cache_write_hour,
            input_images,
            output_images,
            images
        );
        if let Some(amount) = r.cost.total.as_deref().and_then(decimal) {
            let total = self.cost.as_deref().and_then(decimal).unwrap_or_default()
                + amount * Decimal::from(count);
            self.cost = Some(total.normalize().to_string());
        }
        if r.cost.status == "unpriced" {
            self.unpriced += count;
        }
        if r.cost.status == "partial" {
            self.partial += count;
        }
        if !r.tokens.reported() {
            self.missing_usage += count;
        }
    }
    fn add(&mut self, r: &LogicalRecord, count: u64, latency: u64) {
        self.add_result(r, count, latency);
        self.add_usage(&r.record, count);
        self.attempts += r.attempt_count * count;
    }
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Group {
    pub id: String,
    pub name: String,
    #[serde(flatten)]
    pub summary: Aggregate,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Trend {
    pub at: u64,
    #[serde(flatten)]
    pub summary: Aggregate,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Dashboard {
    pub summary: Aggregate,
    pub trends: Vec<Trend>,
    pub providers: Vec<Group>,
    pub models: Vec<Group>,
    pub effective_start: Option<u64>,
    pub effective_end: Option<u64>,
    pub precision: String,
    pub available_providers: Vec<(String, String)>,
    pub available_models: Vec<String>,
    pub semantics_version: u32,
}
type Groups = BTreeMap<String, (String, Aggregate)>;
fn group<'a>(groups: &'a mut Groups, id: &str, name: &str) -> &'a mut Aggregate {
    &mut groups
        .entry(id.into())
        .or_insert_with(|| (name.into(), Aggregate::default()))
        .1
}
fn consumption(r: &Record, count: u64, providers: &mut Groups, models: &mut Groups) {
    if r.provider_id.is_empty() {
        return;
    }
    for a in [
        group(providers, &r.provider_id, &r.provider_name),
        group(models, r.model(), r.model()),
    ] {
        a.add_usage(r, count);
        a.attempts += count;
    }
}
pub fn dashboard(path: &Path, mut filters: Filters) -> Result<Dashboard> {
    let db = reader(path)?;
    let oldest: Option<u64> = db
        .query_row("SELECT MIN(at) FROM logical_requests", [], |r| r.get(0))
        .map_err(error)?;
    let boundary: Option<u64> = db.query_row("SELECT MAX(ends) FROM (SELECT ends FROM logical_rollups UNION ALL SELECT ends FROM rollups)",[],|r| r.get(0)).map_err(error)?;
    let mut precision = "hour";
    if boundary.is_some_and(|b| filters.start.is_none_or(|s| s < b)) {
        precision = "day";
        if let Some(start) = filters.start {
            filters.start = Some(day(start).0);
        }
        if let Some(end) = filters.end.filter(|e| *e < boundary.unwrap()) {
            filters.end = Some(if day(end).0 == end { end } else { day(end).1 });
        }
    }
    if filters
        .end
        .unwrap_or_else(pricing::now)
        .saturating_sub(filters.start.or(oldest).unwrap_or_else(pricing::now))
        > 86400
    {
        precision = "day";
    }
    let mut summary = Aggregate::default();
    let mut providers = Groups::new();
    let mut models = Groups::new();
    let mut trends = BTreeMap::<u64, Aggregate>::new();
    for rollup in [false, true] {
        let (predicate, values) = predicates(&filters, rollup, "");
        let sql = if rollup {
            format!("SELECT payload,count,latency FROM logical_rollups WHERE {predicate}")
        } else {
            format!("SELECT payload,1,0 FROM logical_requests WHERE {predicate} ORDER BY at")
        };
        let mut q = db.prepare(&sql).map_err(error)?;
        let mut rows = q.query(params_from_iter(values)).map_err(error)?;
        while let Some(row) = rows.next().map_err(error)? {
            let payload: String = row.get(0).map_err(error)?;
            let count = row.get(1).map_err(error)?;
            let (r, usage) = if rollup {
                let d: Archived = parse(&payload)?;
                (d.summary, d.attempts)
            } else {
                (parse::<LogicalRecord>(&payload)?, vec![])
            };
            let latency = if rollup {
                row.get(2).map_err(error)?
            } else {
                r.record.latency_ms
            };
            summary.add(&r, count, latency);
            let at = if precision == "day" {
                day(r.record.created_at).0
            } else {
                r.record.created_at / 3600 * 3600
            };
            trends.entry(at).or_default().add(&r, count, latency);
            group(
                &mut providers,
                &r.record.provider_id,
                &r.record.provider_name,
            )
            .add_result(&r, count, latency);
            group(&mut models, r.record.model(), r.record.model()).add_result(&r, count, latency);
            for attempt in usage {
                consumption(&attempt, count, &mut providers, &mut models);
            }
        }
    }
    let (predicate, values) = predicates(&filters, false, "l.");
    let mut q = db.prepare(&format!("SELECT r.payload FROM requests r JOIN logical_requests l ON l.id=r.logical_id WHERE {predicate}")).map_err(error)?;
    let mut rows = q.query(params_from_iter(values)).map_err(error)?;
    while let Some(row) = rows.next().map_err(error)? {
        consumption(
            &parse::<Record>(&row.get::<_, String>(0).map_err(error)?)?,
            1,
            &mut providers,
            &mut models,
        );
    }
    // v1 daily rows have no logical IDs. Keep their usage, never invent a logical success rate.
    let mut legacy_filter = filters.clone();
    legacy_filter.outcome = None;
    if filters.outcome.is_some() {
        legacy_filter.end = Some(0);
    }
    let (predicate, values) = predicates(&legacy_filter, true, "");
    let mut q = db
        .prepare(&format!(
            "SELECT payload,count FROM rollups WHERE {predicate}"
        ))
        .map_err(error)?;
    let mut rows = q.query(params_from_iter(values)).map_err(error)?;
    while let Some(row) = rows.next().map_err(error)? {
        let r: Record = parse(&row.get::<_, String>(0).map_err(error)?)?;
        let count: u64 = row.get(1).map_err(error)?;
        summary.legacy_attempts += count;
        summary.add_usage(&r, count);
        trends
            .entry(day(r.created_at).0)
            .or_default()
            .add_usage(&r, count);
        consumption(&r, count, &mut providers, &mut models);
    }
    let mut available_providers = BTreeMap::new();
    let mut available_models = BTreeSet::new();
    for table in ["logical_requests", "logical_rollups", "rollups"] {
        let mut q = db
            .prepare(&format!(
                "SELECT provider,model,{} FROM {table} GROUP BY provider,model",
                if table == "logical_rollups" {
                    "json_extract(payload,'$.summary.providerName')"
                } else {
                    "json_extract(payload,'$.providerName')"
                }
            ))
            .map_err(error)?;
        let mut rows = q.query([]).map_err(error)?;
        while let Some(row) = rows.next().map_err(error)? {
            available_providers.insert(
                row.get::<_, String>(0).map_err(error)?,
                row.get::<_, String>(2).map_err(error)?,
            );
            available_models.insert(row.get::<_, String>(1).map_err(error)?);
        }
    }
    let groups = |g: Groups| {
        g.into_iter()
            .map(|(id, (name, summary))| Group { id, name, summary })
            .collect()
    };
    Ok(Dashboard {
        summary,
        trends: trends
            .into_iter()
            .map(|(at, summary)| Trend { at, summary })
            .collect(),
        providers: groups(providers),
        models: groups(models),
        effective_start: filters.start,
        effective_end: filters.end,
        precision: precision.into(),
        available_providers: available_providers.into_iter().collect(),
        available_models: available_models.into_iter().collect(),
        semantics_version: 2,
    })
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogPage {
    pub records: Vec<LogicalRecord>,
    pub total: u64,
    pub page: u32,
    pub page_size: u32,
}
pub fn logs(path: &Path, filters: Filters, page: u32) -> Result<LogPage> {
    let db = reader(path)?;
    let (predicate, mut values) = predicates(&filters, false, "");
    let total = db
        .query_row(
            &format!("SELECT COUNT(*) FROM logical_requests WHERE {predicate}"),
            params_from_iter(values.clone()),
            |r| r.get(0),
        )
        .map_err(error)?;
    values.push(Value::Integer(i64::from(page) * 20));
    let mut q = db.prepare(&format!("SELECT payload FROM logical_requests WHERE {predicate} ORDER BY at DESC,rowid DESC LIMIT 20 OFFSET ?")).map_err(error)?;
    let mut rows = q.query(params_from_iter(values)).map_err(error)?;
    let mut records = vec![];
    while let Some(row) = rows.next().map_err(error)? {
        records.push(parse(&row.get::<_, String>(0).map_err(error)?)?);
    }
    Ok(LogPage {
        records,
        total,
        page,
        page_size: 20,
    })
}
pub fn detail(path: &Path, id: &str) -> Result<Option<LogicalDetail>> {
    use rusqlite::OptionalExtension;
    let db = reader(path)?;
    let payload: Option<String> = db
        .query_row(
            "SELECT payload FROM logical_requests WHERE id=?",
            [id],
            |r| r.get(0),
        )
        .optional()
        .map_err(error)?;
    payload
        .map(|p| {
            Ok(LogicalDetail {
                summary: parse(&p)?,
                attempts: attempts(&db, id)?,
            })
        })
        .transpose()
}
#[derive(Serialize, Deserialize)]
struct Archived {
    summary: LogicalRecord,
    attempts: Vec<Record>,
}
pub fn prune(db: &mut Connection, days: u16) -> Result<()> {
    let cutoff = day(pricing::now().saturating_sub(u64::from(days) * 86400)).0;
    loop {
        let rows: Vec<(String, String)> = {
            let mut q = db
                .prepare(
                    "SELECT id,payload FROM logical_requests WHERE at<? AND finished=1 LIMIT 250",
                )
                .map_err(error)?;
            let collected = q
                .query_map([cutoff], |r| Ok((r.get(0)?, r.get(1)?)))
                .map_err(error)?
                .collect::<std::result::Result<_, _>>()
                .map_err(error)?;
            collected
        };
        if rows.is_empty() {
            break;
        }
        let tx = db.transaction().map_err(error)?;
        for (id, payload) in rows {
            let mut summary: LogicalRecord = parse(&payload)?;
            let (at, ends) = day(summary.record.created_at);
            let latency = summary.record.latency_ms;
            let mut rows = attempts(&tx, &id)?;
            for r in std::iter::once(&mut summary.record).chain(rows.iter_mut()) {
                r.id.clear();
                r.logical_id.clear();
                r.created_at = at;
                r.latency_ms = 0;
                r.first_token_ms = None;
                r.duration_ms = None;
                r.routing.clear();
            }
            let archived = Archived {
                summary,
                attempts: rows,
            };
            let payload = json(&archived)?;
            let r = &archived.summary;
            tx.execute("INSERT INTO logical_rollups(id,at,ends,provider,model,status,class,count,latency,payload) VALUES(?,?,?,?,?,?,?,1,?,?)
                ON CONFLICT(id) DO UPDATE SET count=count+1,latency=latency+excluded.latency",
                params![storage::digest(payload.as_bytes()),at,ends,r.record.provider_id,r.record.model(),r.record.status,r.outcome_class,latency,payload]).map_err(error)?;
            tx.execute("DELETE FROM requests WHERE logical_id=?", [&id])
                .map_err(error)?;
            tx.execute("DELETE FROM logical_requests WHERE id=?", [&id])
                .map_err(error)?;
        }
        tx.commit().map_err(error)?;
    }
    Ok(())
}
fn price(r: &mut Record, prices: &pricing::Snapshot) -> bool {
    if r.cost.status != "unpriced" {
        return false;
    }
    let Some(p) = r.billing_model.as_deref().and_then(|m| prices.find(m)) else {
        return false;
    };
    r.cost = calculate(
        &r.tokens,
        Some(p),
        r.service_tier.as_deref(),
        &r.multiplier,
        &prices.version,
    );
    true
}
pub fn backfill(db: &mut Connection, prices: &pricing::Snapshot) -> Result<()> {
    let tx = db.transaction().map_err(error)?;
    let mut changed = BTreeSet::new();
    for table in ["requests", "rollups"] {
        let rows: Vec<(String, String)> = {
            let mut q = tx.prepare(&format!("SELECT id,payload FROM {table} WHERE json_extract(payload,'$.cost.status')='unpriced'")).map_err(error)?;
            let collected = q
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
                .map_err(error)?
                .collect::<std::result::Result<_, _>>()
                .map_err(error)?;
            collected
        };
        for (id, payload) in rows {
            let mut r: Record = parse(&payload)?;
            if price(&mut r, prices) {
                tx.execute(
                    &format!("UPDATE {table} SET payload=? WHERE id=?"),
                    params![json(&r)?, id],
                )
                .map_err(error)?;
                if table == "requests" {
                    changed.insert(r.logical_id);
                }
            }
        }
    }
    for id in changed {
        rebuild(&tx, &id)?;
    }
    let rows: Vec<(String, String)> = {
        let mut q = tx.prepare("SELECT id,payload FROM logical_rollups WHERE json_extract(payload,'$.summary.cost.status')='unpriced'").map_err(error)?;
        let collected = q
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .map_err(error)?
            .collect::<std::result::Result<_, _>>()
            .map_err(error)?;
        collected
    };
    for (id, payload) in rows {
        let mut archived: Archived = parse(&payload)?;
        for r in &mut archived.attempts {
            price(r, prices);
        }
        archived.summary = summarize(archived.summary.record, &archived.attempts);
        tx.execute(
            "UPDATE logical_rollups SET payload=? WHERE id=?",
            params![json(&archived)?, id],
        )
        .map_err(error)?;
    }
    tx.commit().map_err(error)
}

#[cfg(test)]
#[path = "database_tests.rs"]
mod tests;
