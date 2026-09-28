use super::{parser::Tokens, Record};
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
use std::{collections::BTreeMap, path::Path};
fn error(_: impl std::fmt::Display) -> AppError {
    AppError::new("USAGE_DB", "统计数据库操作失败")
}
pub fn initialize(path: &Path) -> Result<Connection> {
    if std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err(AppError::new("USAGE_DB", "统计数据库不能是符号链接"));
    }
    let db = Connection::open(path).map_err(error)?;
    storage::protect(path, false)?;
    db.busy_timeout(std::time::Duration::from_secs(3))
        .map_err(error)?;
    let version: u32 = db
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .map_err(error)?;
    if version > 1 {
        return Err(AppError::new("USAGE_DB", "统计数据库版本较新，请升级应用"));
    }
    db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;
        CREATE TABLE IF NOT EXISTS requests(id TEXT PRIMARY KEY, at INTEGER NOT NULL, provider TEXT NOT NULL,
        model TEXT NOT NULL, status INTEGER, finished INTEGER NOT NULL, payload TEXT NOT NULL);
        CREATE INDEX IF NOT EXISTS usage_time ON requests(at);
        CREATE INDEX IF NOT EXISTS usage_provider_model ON requests(provider,model,at);
        CREATE TABLE IF NOT EXISTS rollups(id TEXT PRIMARY KEY, at INTEGER NOT NULL, ends INTEGER NOT NULL,
        provider TEXT NOT NULL, model TEXT NOT NULL, status INTEGER, count INTEGER NOT NULL, latency INTEGER NOT NULL, payload TEXT NOT NULL);
        CREATE INDEX IF NOT EXISTS rollup_time ON rollups(at,ends);
        PRAGMA user_version=1;").map_err(error)?;
    let unfinished: Vec<(String, String)> = {
        let mut query = db
            .prepare("SELECT id,payload FROM requests WHERE finished=0")
            .map_err(error)?;
        let rows = query
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .map_err(error)?
            .collect::<std::result::Result<_, _>>()
            .map_err(error)?;
        rows
    };
    for (id, payload) in unfinished {
        if let Ok(mut record) = serde_json::from_str::<Record>(&payload) {
            record.outcome = "INTERRUPTED".into();
            record.incomplete = true;
            db.execute(
                "UPDATE requests SET finished=1,payload=? WHERE id=?",
                params![serde_json::to_string(&record).unwrap(), id],
            )
            .map_err(error)?;
        }
    }
    Ok(db)
}
fn reader(path: &Path) -> Result<Connection> {
    Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(error)
}
pub fn insert(db: &Connection, r: &Record, finished: bool) -> Result<()> {
    let payload = serde_json::to_string(r).map_err(error)?;
    db.execute("INSERT INTO requests(id,at,provider,model,status,finished,payload) VALUES(?,?,?,?,?,?,?)
        ON CONFLICT(id) DO UPDATE SET model=excluded.model,status=excluded.status,finished=excluded.finished,payload=excluded.payload
        WHERE requests.finished=0",
        params![r.id,r.created_at,r.provider_id,r.model(),r.status,i32::from(finished),payload]).map_err(error)?;
    Ok(())
}
#[derive(Default, Clone, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Filters {
    pub start: Option<u64>,
    pub end: Option<u64>,
    pub provider_id: Option<String>,
    pub model: Option<String>,
    pub status: Option<u16>,
}
fn predicates(f: &Filters, rollup: bool) -> (String, Vec<Value>) {
    let mut conditions: Vec<String> = vec![if rollup {
        "1=1".into()
    } else {
        "finished=1".into()
    }];
    let mut values = vec![];
    if let Some(start) = f.start {
        conditions.push("at>=?".into());
        values.push(Value::Integer(start.min(i64::MAX as u64) as i64));
    }
    if let Some(end) = f.end {
        conditions.push(if rollup {
            "ends<=?".into()
        } else {
            "at<?".into()
        });
        values.push(Value::Integer(end.min(i64::MAX as u64) as i64));
    }
    if let Some(id) = &f.provider_id {
        conditions.push("provider=?".into());
        values.push(Value::Text(id.clone()));
    }
    if let Some(model) = &f.model {
        conditions.push("model=?".into());
        values.push(Value::Text(model.clone()));
    }
    if let Some(status) = f.status {
        conditions.push("status=?".into());
        values.push(Value::Integer(status as i64));
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
    pub tokens: Tokens,
    pub cost: Option<String>,
    pub unpriced: u64,
    pub partial: u64,
    pub missing_usage: u64,
    pub latency_ms: u64,
    pub gateway_errors: u64,
}
impl Aggregate {
    fn add(&mut self, r: &Record, count: u64, latency: u64) {
        if r.provider_id.is_empty() {
            self.gateway_errors += count;
            return;
        }
        self.requests += count;
        if r.successful() {
            self.successes += count;
        }
        macro_rules! tokens {($($f:ident),*)=>{$(if let Some(n)=r.tokens.$f {self.tokens.$f=Some(self.tokens.$f.unwrap_or(0)+n.saturating_mul(count));})*};}
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
        self.latency_ms += latency;
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
}
pub fn dashboard(path: &Path, mut filters: Filters) -> Result<Dashboard> {
    let db = reader(path)?;
    let oldest_detail: Option<u64> = db
        .query_row("SELECT MIN(at) FROM requests WHERE finished=1", [], |r| {
            r.get(0)
        })
        .map_err(error)?;
    let newest_rollup: Option<u64> = db
        .query_row("SELECT MAX(ends) FROM rollups", [], |r| r.get(0))
        .map_err(error)?;
    let mut precision = "hour";
    if let Some(boundary) = newest_rollup {
        if filters.start.is_none_or(|s| s < boundary) {
            precision = "day";
            if let Some(start) = filters.start {
                filters.start = Some(day(start).0);
            }
            if let Some(end) = filters.end.filter(|e| *e < boundary) {
                filters.end = Some(if day(end).0 == end { end } else { day(end).1 });
            }
        }
    }
    if filters
        .end
        .unwrap_or_else(pricing::now)
        .saturating_sub(filters.start.or(oldest_detail).unwrap_or_else(pricing::now))
        > 86400
    {
        precision = "day";
    }
    let mut summary = Aggregate::default();
    let mut providers: BTreeMap<String, (String, Aggregate)> = BTreeMap::new();
    let mut models: BTreeMap<String, Aggregate> = BTreeMap::new();
    let mut trends: BTreeMap<u64, Aggregate> = BTreeMap::new();
    let mut add = |r: Record, count: u64, latency: u64| {
        summary.add(&r, count, latency);
        let at = if precision == "day" {
            day(r.created_at).0
        } else {
            r.created_at / 3600 * 3600
        };
        trends.entry(at).or_default().add(&r, count, latency);
        if r.provider_id.is_empty() {
            return;
        }
        let p = providers
            .entry(r.provider_id.clone())
            .or_insert_with(|| (r.provider_name.clone(), Aggregate::default()));
        p.1.add(&r, count, latency);
        models
            .entry(r.model().into())
            .or_default()
            .add(&r, count, latency);
    };
    for rollup in [false, true] {
        let (where_sql, values) = predicates(&filters, rollup);
        let sql = if rollup {
            format!("SELECT payload,count,latency FROM rollups WHERE {where_sql}")
        } else {
            format!("SELECT payload,1,0 FROM requests WHERE {where_sql} ORDER BY at")
        };
        let mut query = db.prepare(&sql).map_err(error)?;
        let mut rows = query.query(params_from_iter(values)).map_err(error)?;
        while let Some(row) = rows.next().map_err(error)? {
            let payload: String = row.get(0).map_err(error)?;
            let record: Record = serde_json::from_str(&payload).map_err(error)?;
            let count: u64 = row.get(1).map_err(error)?;
            let latency = if rollup {
                row.get(2).map_err(error)?
            } else {
                record.latency_ms
            };
            add(record, count, latency);
        }
    }
    let mut available_providers = BTreeMap::new();
    let mut available_models = std::collections::BTreeSet::new();
    for table in ["requests", "rollups"] {
        let mut q = db
            .prepare(&format!(
                "SELECT provider,model,payload FROM {table} GROUP BY provider,model"
            ))
            .map_err(error)?;
        let mut rows = q.query([]).map_err(error)?;
        while let Some(r) = rows.next().map_err(error)? {
            let id: String = r.get(0).map_err(error)?;
            let model: String = r.get(1).map_err(error)?;
            let payload: String = r.get(2).map_err(error)?;
            if let Ok(record) = serde_json::from_str::<Record>(&payload) {
                if !id.is_empty() {
                    available_providers.insert(id, record.provider_name);
                }
                available_models.insert(model);
            }
        }
    }
    Ok(Dashboard {
        summary,
        trends: trends
            .into_iter()
            .map(|(at, summary)| Trend { at, summary })
            .collect(),
        providers: providers
            .into_iter()
            .map(|(id, (name, summary))| Group { id, name, summary })
            .collect(),
        models: models
            .into_iter()
            .map(|(id, summary)| Group {
                name: id.clone(),
                id,
                summary,
            })
            .collect(),
        effective_start: filters.start,
        effective_end: filters.end,
        precision: precision.into(),
        available_providers: available_providers.into_iter().collect(),
        available_models: available_models.into_iter().collect(),
    })
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogPage {
    pub records: Vec<Record>,
    pub total: u64,
    pub page: u32,
    pub page_size: u32,
}
pub fn logs(path: &Path, filters: Filters, page: u32) -> Result<LogPage> {
    let db = reader(path)?;
    let (where_sql, values) = predicates(&filters, false);
    let total = db
        .query_row(
            &format!("SELECT COUNT(*) FROM requests WHERE {where_sql}"),
            params_from_iter(values.clone()),
            |r| r.get(0),
        )
        .map_err(error)?;
    let mut values = values;
    values.push(Value::Integer(page as i64 * 20));
    let mut q=db.prepare(&format!("SELECT payload FROM requests WHERE {where_sql} ORDER BY at DESC,rowid DESC LIMIT 20 OFFSET ?")).map_err(error)?;
    let mut rows = q.query(params_from_iter(values)).map_err(error)?;
    let mut records = vec![];
    while let Some(r) = rows.next().map_err(error)? {
        let json: String = r.get(0).map_err(error)?;
        records.push(serde_json::from_str(&json).map_err(error)?);
    }
    Ok(LogPage {
        records,
        total,
        page,
        page_size: 20,
    })
}
pub fn detail(path: &Path, id: &str) -> Result<Option<Record>> {
    use rusqlite::OptionalExtension;
    let db = reader(path)?;
    let json: Option<String> = db
        .query_row(
            "SELECT payload FROM requests WHERE id=? AND finished=1",
            [id],
            |r| r.get(0),
        )
        .optional()
        .map_err(error)?;
    json.map(|s| serde_json::from_str(&s).map_err(error))
        .transpose()
}
pub fn prune(db: &mut Connection, days: u16) -> Result<()> {
    let cutoff = day(pricing::now().saturating_sub(days as u64 * 86400)).0;
    loop {
        let records: Vec<Record> = {
            let mut q = db
                .prepare("SELECT payload FROM requests WHERE at<? AND finished=1 LIMIT 500")
                .map_err(error)?;
            let raw = q
                .query_map([cutoff], |r| r.get::<_, String>(0))
                .map_err(error)?
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(error)?;
            raw.into_iter()
                .map(|s| serde_json::from_str(&s).map_err(error))
                .collect::<Result<_>>()?
        };
        if records.is_empty() {
            break;
        }
        let tx = db.transaction().map_err(error)?;
        for mut r in records {
            let id = r.id.clone();
            let latency = r.latency_ms;
            let (at, end) = day(r.created_at);
            r.id.clear();
            r.logical_id.clear();
            r.attempt = 0;
            r.created_at = at;
            r.latency_ms = 0;
            r.first_token_ms = None;
            r.duration_ms = None;
            let payload = serde_json::to_string(&r).map_err(error)?;
            let key = storage::digest(payload.as_bytes());
            tx.execute("INSERT INTO rollups(id,at,ends,provider,model,status,count,latency,payload) VALUES(?,?,?,?,?,?,1,?,?)
                ON CONFLICT(id) DO UPDATE SET count=count+1,latency=latency+excluded.latency",
                params![key,at,end,r.provider_id,r.model(),r.status,latency,payload]).map_err(error)?;
            tx.execute("DELETE FROM requests WHERE id=?", [id])
                .map_err(error)?;
        }
        tx.commit().map_err(error)?;
    }
    Ok(())
}
pub fn backfill(db: &mut Connection, prices: &pricing::Snapshot) -> Result<()> {
    for table in ["requests", "rollups"] {
        let records: Vec<(String, Record)> = {
            let mut q=db.prepare(&format!("SELECT id,payload FROM {table} WHERE json_extract(payload,'$.cost.status') IN ('unpriced','partial')")).map_err(error)?;
            let pairs = q
                .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
                .map_err(error)?
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(error)?;
            pairs
                .into_iter()
                .map(|(id, s)| serde_json::from_str(&s).map(|r| (id, r)).map_err(error))
                .collect::<Result<_>>()?
        };
        let tx = db.transaction().map_err(error)?;
        for (id, mut r) in records {
            let price = r.billing_model.as_deref().and_then(|m| prices.find(m));
            if price.is_none() {
                continue;
            }
            r.cost = calculate(
                &r.tokens,
                price,
                r.service_tier.as_deref(),
                &r.multiplier,
                &prices.version,
            );
            tx.execute(
                &format!("UPDATE {table} SET payload=? WHERE id=?"),
                params![serde_json::to_string(&r).unwrap(), id],
            )
            .map_err(error)?;
        }
        tx.commit().map_err(error)?;
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rollback_safe_rollup_and_crash_recovery_preserve_totals() {
        let t = tempfile::tempdir().unwrap();
        let prices = pricing::Service::new(t.path()).unwrap();
        let service = super::super::Service::new(t.path(), prices);
        let mut s = service.begin("one", 0, "p", "Old name", Some("unknown"), None, false);
        s.finish(Some(200), "OK");
        service.flush();
        let path = t.path().join("usage.sqlite");
        let mut db = Connection::open(&path).unwrap();
        let mut record = detail(&path, &s.record.id).unwrap().unwrap();
        record.created_at = pricing::now() - 86400 * 40;
        db.execute(
            "UPDATE requests SET at=?,payload=?",
            params![record.created_at, serde_json::to_string(&record).unwrap()],
        )
        .unwrap();
        let before = dashboard(&path, Filters::default())
            .unwrap()
            .summary
            .requests;
        prune(&mut db, 30).unwrap();
        prune(&mut db, 30).unwrap();
        assert_eq!(
            dashboard(&path, Filters::default())
                .unwrap()
                .summary
                .requests,
            before
        );
        assert_eq!(logs(&path, Filters::default(), 0).unwrap().total, 0);
        record.id = "unfinished".into();
        record.created_at = pricing::now();
        record.outcome = "IN_PROGRESS".into();
        insert(&db, &record, false).unwrap();
        drop(db);
        let _ = initialize(&path).unwrap();
        assert_eq!(
            detail(&path, "unfinished").unwrap().unwrap().outcome,
            "INTERRUPTED"
        );
    }
}
