use super::{parser::Tokens, Record, Service, WriteEvent};
use crate::pricing::{self, Cost};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use std::time::Instant;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RoutingDecision {
    pub provider_id: String,
    pub provider_name: String,
    pub reason: String,
    pub active: usize,
    pub limit: u32,
    pub retry_in: u64,
    pub at: u64,
}
#[derive(Clone, Default)]
pub struct RoutingTrace(Arc<Mutex<Vec<RoutingDecision>>>);
impl RoutingTrace {
    pub fn push(
        &self,
        id: &str,
        name: &str,
        reason: &str,
        active: usize,
        limit: u32,
        retry_in: u64,
    ) {
        let mut trace = self.0.lock().unwrap();
        // Countdown ticks do not create an unbounded sequence of duplicate decisions.
        if trace
            .iter()
            .rev()
            .find(|d| d.provider_id == id)
            .is_some_and(|d| d.reason == reason && d.active == active && d.limit == limit)
        {
            return;
        }
        if trace.len() >= 64 {
            trace.remove(1);
        }
        trace.push(RoutingDecision {
            provider_id: id.into(),
            provider_name: name.into(),
            reason: reason.into(),
            active,
            limit,
            retry_in,
            at: pricing::now(),
        });
    }
    pub fn snapshot(&self) -> Vec<RoutingDecision> {
        self.0.lock().unwrap().clone()
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LogicalRecord {
    #[serde(flatten)]
    pub record: Record,
    pub outcome_class: String,
    pub attempt_count: u64,
    pub semantics_version: u32,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogicalDetail {
    #[serde(flatten)]
    pub summary: LogicalRecord,
    pub log: super::RequestLog,
    pub attempts: Vec<Record>,
}
struct Context {
    record: Record,
    finished: bool,
}
struct Inner {
    service: Service,
    state: Mutex<Context>,
    started: Instant,
    enabled: bool,
    trace: RoutingTrace,
}
#[derive(Clone)]
pub struct LogicalSpan(Arc<Inner>);
impl Service {
    pub fn logical(&self, id: &str, tracked: bool) -> LogicalSpan {
        let settings = self.0.settings.lock().unwrap().0.clone();
        let record = Record {
            response_id: None,
            id: id.into(),
            logical_id: id.into(),
            attempt: 0,
            provider_id: String::new(),
            provider_name: "网关".into(),
            request_model: None,
            response_model: None,
            billing_model: None,
            service_tier: None,
            model_source: settings.model_source,
            multiplier: settings.cost_multiplier,
            created_at: pricing::now(),
            status: None,
            outcome: "IN_PROGRESS".into(),
            streaming: false,
            latency_ms: 0,
            first_token_ms: None,
            duration_ms: None,
            tokens: Tokens::default(),
            incomplete: false,
            cost: Cost::default(),
            routing: vec![],
            terminal_evidence: None,
        };
        let enabled = settings.enabled && tracked;
        if enabled {
            self.send(WriteEvent::Logical(Box::new(record.clone()), false));
        }
        LogicalSpan(Arc::new(Inner {
            service: self.clone(),
            state: Mutex::new(Context {
                record,
                finished: false,
            }),
            started: Instant::now(),
            enabled,
            trace: RoutingTrace::default(),
        }))
    }
}
impl LogicalSpan {
    pub fn id(&self) -> String {
        self.0.state.lock().unwrap().record.logical_id.clone()
    }
    pub fn trace(&self) -> RoutingTrace {
        self.0.trace.clone()
    }
    pub fn model(&self, model: Option<&str>) {
        self.0.state.lock().unwrap().record.request_model = model.map(str::to_owned);
    }
    pub fn record(&self, record: &Record, final_attempt: bool) {
        let mut state = self.0.state.lock().unwrap();
        if state.finished {
            return;
        }
        let created_at = state.record.created_at;
        state.record = record.clone();
        state.record.id = record.logical_id.clone();
        state.record.created_at = created_at;
        state.record.latency_ms = self.0.started.elapsed().as_millis() as u64;
        state.record.first_token_ms = record
            .first_token_ms
            .map(|n| n.saturating_add(state.record.latency_ms.saturating_sub(record.latency_ms)));
        state.record.routing = self.0.trace.snapshot();
        if final_attempt {
            self.0.finish(&mut state);
        }
    }
    pub fn finish_last(&self, status: u16) {
        let mut state = self.0.state.lock().unwrap();
        if state.finished {
            return;
        }
        state.record.status = Some(status);
        if state.record.outcome == "IN_PROGRESS" {
            state.record.outcome = if status < 400 { "OK" } else { "HTTP" }.into();
        }
        self.0.finish(&mut state);
    }
    pub fn reject(&self, status: u16, category: &str) {
        let mut state = self.0.state.lock().unwrap();
        if state.finished {
            return;
        }
        state.record.status = Some(status);
        state.record.outcome = category.into();
        self.0.finish(&mut state);
    }
}
impl Inner {
    fn finish(&self, state: &mut Context) {
        state.finished = true;
        state.record.latency_ms = self.started.elapsed().as_millis() as u64;
        state.record.routing = self.trace.snapshot();
        if self.enabled {
            self.service
                .send(WriteEvent::Logical(Box::new(state.record.clone()), true));
        }
    }
}
impl Drop for Inner {
    fn drop(&mut self) {
        let mut state = self.state.lock().unwrap();
        if !state.finished {
            state.record.outcome = "CANCELLED".into();
            self.finish(&mut state);
        }
    }
}
