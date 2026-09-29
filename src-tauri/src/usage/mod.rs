mod database;
mod logical;
pub mod parser;
use crate::{
    pricing::{
        self,
        model::{calculate, decimal},
        Cost,
    },
    storage::{self, AppError, Result},
};
pub use database::{Dashboard, Filters, LogPage, RequestLog};
pub use logical::{LogicalDetail, LogicalRecord, LogicalSpan, RoutingDecision, RoutingTrace};
use parser::{Observation, Observer, Tokens};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{mpsc, Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::sync::broadcast;

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub enabled: bool,
    pub cost_multiplier: String,
    pub provider_multipliers: BTreeMap<String, String>,
    pub model_source: String,
    pub refresh_seconds: u16,
    pub retention_days: u16,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            enabled: true,
            cost_multiplier: "1".into(),
            provider_multipliers: BTreeMap::new(),
            model_source: "response".into(),
            refresh_seconds: 30,
            retention_days: 30,
        }
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Record {
    #[serde(default)]
    pub response_id: Option<String>,
    pub id: String,
    pub logical_id: String,
    pub attempt: usize,
    pub provider_id: String,
    pub provider_name: String,
    pub request_model: Option<String>,
    pub response_model: Option<String>,
    pub billing_model: Option<String>,
    pub service_tier: Option<String>,
    pub model_source: String,
    pub multiplier: String,
    pub created_at: u64,
    pub status: Option<u16>,
    pub outcome: String,
    pub streaming: bool,
    pub latency_ms: u64,
    pub first_token_ms: Option<u64>,
    pub duration_ms: Option<u64>,
    pub tokens: Tokens,
    pub incomplete: bool,
    pub cost: Cost,
    #[serde(default)]
    pub routing: Vec<RoutingDecision>,
    #[serde(default)]
    pub terminal_evidence: Option<String>,
}
impl Record {
    pub fn successful(&self) -> bool {
        matches!(self.outcome.as_str(), "OK" | "OUTPUT_LIMIT")
            && self
                .status
                .is_some_and(|s| (200..300).contains(&s) || s == 101)
    }
    pub fn outcome_class(&self) -> &'static str {
        if self.successful() {
            "success"
        } else if self.outcome == "IN_PROGRESS" {
            "pending"
        } else if self.outcome == "CANCELLED" {
            "cancelled"
        } else if matches!(self.outcome.as_str(), "INTERRUPTED" | "UNKNOWN_TERMINAL") {
            "unknown"
        } else if matches!(
            self.outcome.as_str(),
            "CAPACITY"
                | "MODEL_NOT_ALLOWED"
                | "MODEL_UNDETERMINED"
                | "BODY"
                | "LOCAL_AUTH"
                | "WEBSOCKET"
                | "BUSINESS_REJECTED"
        ) || self
            .status
            .is_some_and(|s| [400, 404, 405, 406, 413, 414, 415, 422].contains(&s))
        {
            "rejected"
        } else {
            "failure"
        }
    }
    pub fn model(&self) -> &str {
        self.billing_model
            .as_deref()
            .or(self.response_model.as_deref())
            .or(self.request_model.as_deref())
            .unwrap_or("未识别")
    }
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct State {
    pub settings: Settings,
    pub revision: String,
    pub error: Option<String>,
}
enum WriteEvent {
    Start(Box<Record>),
    Finish(Box<Record>),
    Logical(Box<Record>, bool),
    Backfill(Arc<pricing::Snapshot>),
    Maintain(u16),
    Barrier(mpsc::Sender<()>),
}
struct Inner {
    path: PathBuf,
    settings_path: PathBuf,
    settings: Mutex<(Settings, String)>,
    prices: pricing::Service,
    tx: mpsc::SyncSender<WriteEvent>,
    error: Arc<Mutex<Option<String>>>,
    events: broadcast::Sender<()>,
}
#[derive(Clone)]
pub struct Service(Arc<Inner>);
impl Service {
    pub fn new(data: &Path, prices: pricing::Service) -> Self {
        let settings_path = data.join("usage-settings.json");
        let (settings, revision) = storage::read_optional(&settings_path)
            .ok()
            .flatten()
            .map(|bytes| {
                let settings = serde_json::from_slice::<Settings>(&bytes).unwrap_or_default();
                (settings, storage::digest(&bytes))
            })
            .unwrap_or((Settings::default(), "missing".into()));
        let path = data.join("usage.sqlite");
        let error = Arc::new(Mutex::new(None));
        let (events, _) = broadcast::channel(32);
        let (tx, rx) = mpsc::sync_channel(4096);
        let connection = database::initialize(&path);
        if connection.is_err() {
            *error.lock().unwrap() = Some("统计数据库无法打开，网关仍可使用".into());
        }
        let service = Self(Arc::new(Inner {
            path,
            settings_path,
            settings: Mutex::new((settings.clone(), revision)),
            prices,
            tx,
            error: error.clone(),
            events: events.clone(),
        }));
        let db_path = service.0.path.clone();
        std::thread::Builder::new()
            .name("gpt-switch-usage".into())
            .spawn(move || {
                let Ok(mut db) = connection else {
                    while let Ok(e) = rx.recv() {
                        if let WriteEvent::Barrier(tx) = e {
                            let _ = tx.send(());
                        }
                    }
                    return;
                };
                let mut last = Instant::now() - Duration::from_secs(2);
                let mut writes = 0;
                while let Ok(event) = rx.recv() {
                    let result = match event {
                        WriteEvent::Start(r) => database::insert(&db, &r, false),
                        WriteEvent::Finish(r) => database::insert(&db, &r, true),
                        WriteEvent::Logical(r, finished) => database::logical(&db, &r, finished),
                        WriteEvent::Backfill(p) => database::backfill(&mut db, &p),
                        WriteEvent::Maintain(days) => database::prune(&mut db, days),
                        WriteEvent::Barrier(tx) => {
                            let _ = tx.send(());
                            continue;
                        }
                    };
                    if result.is_err() {
                        *error.lock().unwrap() =
                            Some("用量写入失败，请检查磁盘和统计数据库".into());
                    }
                    writes += 1;
                    if writes % 100 == 0 {
                        for suffix in ["-wal", "-shm"] {
                            let p = PathBuf::from(format!("{}{suffix}", db_path.to_string_lossy()));
                            if p.exists() {
                                let _ = storage::protect(&p, false);
                            }
                        }
                    }
                    if last.elapsed() >= Duration::from_millis(250) {
                        let _ = events.send(());
                        last = Instant::now();
                    }
                }
                let _ = db.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)");
            })
            .expect("usage writer");
        service.send(WriteEvent::Maintain(settings.retention_days));
        service
    }
    fn send(&self, event: WriteEvent) {
        if self.0.tx.try_send(event).is_err() {
            *self.0.error.lock().unwrap() = Some("统计写入队列已满，部分记录未保存".into());
            let _ = self.0.events.send(());
        }
    }
    pub fn subscribe(&self) -> broadcast::Receiver<()> {
        self.0.events.subscribe()
    }
    pub fn prices(&self) -> pricing::Service {
        self.0.prices.clone()
    }
    pub fn state(&self) -> State {
        let (settings, revision) = self.0.settings.lock().unwrap().clone();
        State {
            settings,
            revision,
            error: self.0.error.lock().unwrap().clone(),
        }
    }
    pub fn configure(&self, settings: Settings, expected: &str) -> Result<State> {
        if !["request", "response"].contains(&settings.model_source.as_str())
            || ![0, 5, 10, 30, 60].contains(&settings.refresh_seconds)
            || settings.retention_days < 1
            || settings.retention_days > 3650
            || decimal(&settings.cost_multiplier).is_none()
            || settings.provider_multipliers.len() > 10000
            || settings
                .provider_multipliers
                .values()
                .any(|s| decimal(s).is_none())
        {
            return Err(AppError::new("USAGE", "统计设置无效"));
        }
        {
            let mut current = self.0.settings.lock().unwrap();
            if current.1 != expected {
                return Err(AppError::new("CONFLICT", "统计设置已变化"));
            }
            let bytes = serde_json::to_vec_pretty(&settings).unwrap();
            storage::atomic_write(&self.0.settings_path, &bytes, Some(expected))?;
            *current = (settings.clone(), storage::digest(&bytes));
        }
        self.send(WriteEvent::Maintain(settings.retention_days));
        let _ = self.0.events.send(());
        Ok(self.state())
    }
    #[allow(clippy::too_many_arguments)]
    pub fn begin(
        &self,
        logical_id: &str,
        attempt: usize,
        provider_id: &str,
        name: &str,
        model: Option<&str>,
        tier: Option<&str>,
        streaming: bool,
    ) -> Span {
        let settings = self.0.settings.lock().unwrap().0.clone();
        let record = Record {
            response_id: None,
            id: uuid::Uuid::new_v4().to_string(),
            logical_id: logical_id.into(),
            attempt,
            provider_id: provider_id.into(),
            provider_name: name.into(),
            request_model: model.map(str::to_owned),
            response_model: None,
            billing_model: None,
            service_tier: tier.map(str::to_owned),
            model_source: settings.model_source,
            multiplier: settings
                .provider_multipliers
                .get(provider_id)
                .cloned()
                .unwrap_or(settings.cost_multiplier),
            created_at: pricing::now(),
            status: None,
            outcome: "IN_PROGRESS".into(),
            streaming,
            latency_ms: 0,
            first_token_ms: None,
            duration_ms: None,
            tokens: Tokens::default(),
            incomplete: false,
            cost: Cost::default(),
            routing: vec![],
            terminal_evidence: None,
        };
        if settings.enabled {
            self.send(WriteEvent::Start(Box::new(record.clone())));
        }
        Span {
            service: self.clone(),
            record,
            prices: self.0.prices.snapshot(),
            started: Instant::now(),
            observer: None,
            observation: Observation::default(),
            enabled: settings.enabled,
            finished: false,
            logical: None,
            final_attempt: false,
            trace: None,
        }
    }
    pub fn rejected(&self, id: &str, code: u16, category: &str, model: Option<&str>) {
        let mut span = self.begin(id, 0, "", "网关", model, None, false);
        span.finish(Some(code), category);
    }
    pub fn backfill(&self) {
        self.send(WriteEvent::Backfill(self.0.prices.snapshot()));
    }
    pub fn flush(&self) {
        let (tx, rx) = mpsc::channel();
        if self.0.tx.send(WriteEvent::Barrier(tx)).is_ok() {
            let _ = rx.recv_timeout(Duration::from_secs(5));
        }
    }
    pub fn dashboard(&self, filters: Filters) -> Result<Dashboard> {
        database::dashboard(&self.0.path, filters)
    }
    pub fn logs(&self, filters: Filters, page: u32) -> Result<LogPage> {
        database::logs(&self.0.path, filters, page)
    }
    pub fn detail(&self, id: &str) -> Result<Option<LogicalDetail>> {
        database::detail(&self.0.path, id)
    }
}
pub struct Span {
    service: Service,
    record: Record,
    prices: Arc<pricing::Snapshot>,
    started: Instant,
    observer: Option<Observer>,
    observation: Observation,
    enabled: bool,
    finished: bool,
    logical: Option<LogicalSpan>,
    final_attempt: bool,
    trace: Option<RoutingTrace>,
}
impl Span {
    pub fn attach(&mut self, logical: &LogicalSpan, trace: &RoutingTrace) {
        self.logical = Some(logical.clone());
        self.trace = Some(trace.clone());
    }
    pub fn final_attempt(&mut self) {
        self.final_attempt = true;
    }
    pub fn terminal(&mut self) -> Option<parser::Terminal> {
        if let Some(o) = &mut self.observer {
            self.observation = o.snapshot(false);
        }
        self.observation.terminal
    }
    pub fn outcome_class(&self) -> &'static str {
        self.record.outcome_class()
    }
    pub fn finished(&self) -> bool {
        self.finished
    }
    pub fn response(&mut self, status: u16, stream: bool, encoding: &str) {
        self.record.status = Some(status);
        self.record.streaming = stream;
        self.observer = Some(Observer::new(stream, encoding));
    }
    pub fn feed(&mut self, bytes: &[u8]) {
        if let Some(o) = &mut self.observer {
            o.feed(bytes, self.started.elapsed().as_millis() as u64);
        }
    }
    pub fn value(&mut self, value: &serde_json::Value) {
        self.observation
            .value(value, self.started.elapsed().as_millis() as u64);
    }
    pub fn finish(&mut self, status: Option<u16>, outcome: &str) {
        if self.finished {
            return;
        }
        self.finished = true;
        if let Some(observer) = &mut self.observer {
            self.observation = observer.snapshot(outcome == "OK" || outcome == "HTTP");
        }
        let o = &self.observation;
        self.record.status = status.or(self.record.status);
        self.record.outcome = if self.record.status.is_none_or(|s| s < 400) {
            o.terminal
                .map(|t| t.outcome())
                .unwrap_or_else(|| {
                    if outcome == "OK" && self.record.streaming && o.expects_terminal {
                        if o.incomplete {
                            "UNKNOWN_TERMINAL"
                        } else {
                            "STREAM_INTERRUPTED"
                        }
                    } else {
                        outcome
                    }
                })
                .into()
        } else {
            outcome.into()
        };
        self.record.terminal_evidence = o.terminal.map(|t| t.outcome().into());
        if let Some(trace) = &self.trace {
            self.record.routing = trace.snapshot();
        }
        self.record.latency_ms = self.started.elapsed().as_millis() as u64;
        self.record.first_token_ms = o.first_token_ms;
        self.record.duration_ms = o
            .first_token_ms
            .map(|n| self.record.latency_ms.saturating_sub(n));
        self.record.response_model = o.model.clone();
        self.record.response_id = o.response_id.clone();
        self.record.tokens = o.tokens.clone();
        self.record.service_tier = o.service_tier.clone().or(self.record.service_tier.clone());
        self.record.incomplete =
            o.incomplete || !matches!(self.record.outcome.as_str(), "OK" | "OUTPUT_LIMIT");
        self.record.billing_model = if self.record.model_source == "request" {
            self.record.request_model.clone().or(o.model.clone())
        } else {
            o.model.clone().or(self.record.request_model.clone())
        };
        let price = self
            .record
            .billing_model
            .as_deref()
            .and_then(|id| self.prices.find(id));
        self.record.cost = calculate(
            &self.record.tokens,
            price,
            self.record.service_tier.as_deref(),
            &self.record.multiplier,
            &self.prices.version,
        );
        if self.enabled {
            self.service
                .send(WriteEvent::Finish(Box::new(self.record.clone())));
        }
        if let Some(logical) = &self.logical {
            logical.record(&self.record, self.final_attempt);
        }
    }
}
impl Drop for Span {
    fn drop(&mut self) {
        self.finish(None, "CANCELLED");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn idempotent_span_missing_usage_and_price_snapshot() {
        let t = tempfile::tempdir().unwrap();
        let prices = pricing::Service::new(t.path()).unwrap();
        let svc = Service::new(t.path(), prices);
        let mut a = svc.begin("logical", 0, "p", "Provider", Some("unknown"), None, true);
        a.response(200, true, "");
        a.feed(b"data: {\"response\":{\"usage\":{\"input_tokens\":100,\"output_tokens\":10}}}\n\n");
        a.finish(Some(200), "OK");
        a.finish(Some(200), "OK");
        drop(a);
        drop(svc.begin("cancelled", 0, "p", "Provider", None, None, false));
        svc.flush();
        let data = svc.dashboard(Filters::default()).unwrap();
        assert_eq!(data.summary.requests, 2);
        assert_eq!(data.summary.successes, 1);
        assert_eq!(data.summary.unpriced, 1);
        assert_eq!(data.summary.tokens.total(), 110);
        assert_eq!(svc.logs(Filters::default(), 0).unwrap().total, 2);
    }
    #[test]
    fn confirmed_terminal_survives_drop_but_partial_usage_does_not_imply_success() {
        let t = tempfile::tempdir().unwrap();
        let svc = Service::new(t.path(), pricing::Service::new(t.path()).unwrap());
        for (id, event, finish) in [
            (
                "complete",
                r#"{"type":"response.completed","response":{"usage":{"input_tokens":3,"output_tokens":2}}}"#,
                false,
            ),
            (
                "cancel",
                r#"{"type":"response.created","response":{"usage":{"input_tokens":3}}}"#,
                false,
            ),
            (
                "truncated",
                r#"{"type":"response.output_text.delta","delta":"text"}"#,
                true,
            ),
            (
                "error",
                r#"{"type":"response.failed","response":{"error":{"code":"server_error"}}}"#,
                true,
            ),
            (
                "limit",
                r#"{"type":"response.incomplete","response":{"incomplete_details":{"reason":"max_output_tokens"}}}"#,
                true,
            ),
        ] {
            let logical = svc.logical(id, true);
            let mut span = svc.begin(id, 0, "p", "Provider", None, None, true);
            span.attach(&logical, &logical.trace());
            span.final_attempt();
            span.response(200, true, "");
            span.feed(format!("data: {event}\n\n").as_bytes());
            if finish {
                span.finish(Some(200), "OK");
                span.finish(Some(200), "CANCELLED");
            }
            drop(span);
        }
        svc.flush();
        let s = svc.dashboard(Filters::default()).unwrap().summary;
        assert_eq!(
            (s.requests, s.successes, s.failures, s.cancelled, s.attempts),
            (5, 2, 2, 1, 5)
        );
        assert_eq!(s.tokens.total(), 8);
    }
}
