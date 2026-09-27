// Adapted from cc-switch circuit_breaker.rs and forwarder.rs, commit 1ee2fdc3.
// Copyright (c) 2025 JasonYoung. MIT; see THIRD_PARTY_NOTICES.md.
use super::model::Settings;
use serde::Serialize;
use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

#[derive(Clone, Copy, PartialEq, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CircuitState {
    Closed,
    Open,
    HalfOpen,
}
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Health {
    pub state: CircuitState,
    pub failures: u32,
    pub requests: u32,
    pub retry_in: u64,
}
struct State {
    phase: CircuitState,
    failures: u32,
    successes: u32,
    total: u32,
    failed: u32,
    until: Option<Instant>,
    probe: bool,
    generation: u64,
}
impl Default for State {
    fn default() -> Self {
        Self {
            phase: CircuitState::Closed,
            failures: 0,
            successes: 0,
            total: 0,
            failed: 0,
            until: None,
            probe: false,
            generation: 0,
        }
    }
}
#[derive(Clone, Default)]
pub struct Circuit(Arc<Mutex<State>>);
pub enum Outcome {
    Success,
    Failure(Option<Duration>),
    Neutral,
}
pub struct Permit {
    circuit: Circuit,
    generation: u64,
    half_open: bool,
    complete: bool,
}
impl Circuit {
    pub fn health(&self) -> Health {
        let s = self.0.lock().unwrap();
        Health {
            state: s.phase,
            failures: s.failures,
            requests: s.total,
            retry_in: s
                .until
                .map(|t| t.saturating_duration_since(Instant::now()).as_secs())
                .unwrap_or(0),
        }
    }
    pub fn acquire(&self, manual: bool) -> Option<Permit> {
        let mut s = self.0.lock().unwrap();
        if s.phase == CircuitState::Open {
            if s.until.is_none_or(|t| t <= Instant::now()) {
                s.phase = CircuitState::HalfOpen;
                s.successes = 0;
                s.probe = false;
                s.generation += 1;
            } else if !manual {
                return None;
            }
        }
        let half_open = s.phase == CircuitState::HalfOpen;
        if half_open && s.probe {
            return None;
        }
        if half_open {
            s.probe = true;
        }
        Some(Permit {
            circuit: self.clone(),
            generation: s.generation,
            half_open,
            complete: false,
        })
    }
    pub fn reset(&self) {
        let mut s = self.0.lock().unwrap();
        let generation = s.generation + 1;
        *s = State {
            generation,
            ..Default::default()
        };
    }
}
impl Permit {
    pub fn finish(mut self, outcome: Outcome, cfg: &Settings) {
        let mut s = self.circuit.0.lock().unwrap();
        self.complete = true;
        if s.generation != self.generation {
            return;
        }
        if self.half_open {
            s.probe = false;
        }
        match outcome {
            Outcome::Neutral => (),
            Outcome::Success => {
                s.failures = 0;
                s.total = s.total.saturating_add(1);
                if s.phase == CircuitState::HalfOpen {
                    s.successes += 1;
                    if s.successes >= cfg.success_threshold {
                        let generation = s.generation + 1;
                        *s = State {
                            generation,
                            ..Default::default()
                        };
                    }
                }
            }
            Outcome::Failure(retry_after) => {
                s.failures = s.failures.saturating_add(1);
                s.failed = s.failed.saturating_add(1);
                s.total = s.total.saturating_add(1);
                let trip = s.phase != CircuitState::Closed
                    || s.failures >= cfg.failure_threshold
                    || (s.total >= cfg.min_requests
                        && f64::from(s.failed) / f64::from(s.total) >= cfg.error_rate)
                    || retry_after.is_some();
                if trip {
                    s.phase = CircuitState::Open;
                    s.probe = false;
                    s.generation += 1;
                    s.until = Some(
                        Instant::now()
                            + retry_after
                                .unwrap_or_default()
                                .max(Duration::from_secs(cfg.cooldown_seconds)),
                    );
                }
            }
        }
    }
}
impl Drop for Permit {
    fn drop(&mut self) {
        if !self.complete && self.half_open {
            let mut s = self.circuit.0.lock().unwrap();
            if s.generation == self.generation {
                s.probe = false;
            }
        }
    }
}
pub fn retryable(status: u16) -> bool {
    status >= 400 && ![400, 405, 406, 413, 414, 415, 422, 501].contains(&status)
}
pub fn retry_after(value: &str) -> Option<Duration> {
    value
        .trim()
        .parse::<u64>()
        .ok()
        .map(Duration::from_secs)
        .or_else(|| {
            httpdate::parse_http_date(value)
                .ok()?
                .duration_since(std::time::SystemTime::now())
                .ok()
        })
        .filter(|d| !d.is_zero())
        .map(|d| d.min(Duration::from_secs(86400)))
}
#[cfg(test)]
mod tests {
    use super::*;
    // State-transition cases adapted from the pinned upstream circuit breaker tests.
    #[test]
    fn consecutive_failure_and_single_probe_with_cancel() {
        let c = Circuit::default();
        let cfg = Settings::default();
        for _ in 0..4 {
            c.acquire(false)
                .unwrap()
                .finish(Outcome::Failure(None), &cfg);
        }
        assert!(c.acquire(false).is_none());
        c.0.lock().unwrap().until = Some(Instant::now());
        let p = c.acquire(false).unwrap();
        assert!(c.acquire(false).is_none());
        drop(p);
        c.acquire(false).unwrap().finish(Outcome::Success, &cfg);
        assert_eq!(c.health().state, CircuitState::HalfOpen);
        c.acquire(false).unwrap().finish(Outcome::Success, &cfg);
        assert_eq!(c.health().state, CircuitState::Closed);
    }
    #[test]
    fn stale_success_cannot_close_new_circuit_and_rate_threshold() {
        let c = Circuit::default();
        let cfg = Settings {
            failure_threshold: 20,
            ..Default::default()
        };
        let stale = c.acquire(false).unwrap();
        for i in 0..10 {
            c.acquire(false).unwrap().finish(
                if i < 4 {
                    Outcome::Success
                } else {
                    Outcome::Failure(None)
                },
                &cfg,
            );
        }
        assert_eq!(c.health().state, CircuitState::Open);
        stale.finish(Outcome::Success, &cfg);
        assert_eq!(c.health().state, CircuitState::Open);
    }
    #[test]
    fn classification_and_retry_after() {
        for s in [400, 405, 406, 413, 414, 415, 422, 501] {
            assert!(!retryable(s));
        }
        for s in [401, 403, 408, 429, 500, 502, 503, 504] {
            assert!(retryable(s));
        }
        let c = Circuit::default();
        c.acquire(false)
            .unwrap()
            .finish(Outcome::Failure(retry_after("120")), &Settings::default());
        assert!(c.health().retry_in >= 119);
        assert!(retry_after("bad").is_none());
    }
}
