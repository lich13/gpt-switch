//! Request slots and bounded FIFO admission. Behavior informed by Sub2API
//! a3eb7ef3 concurrency_service / account scheduler; independently implemented in Rust.
use super::{forward::Permits, model::Provider, routing::Requirement, Route};
use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::sync::{broadcast, Notify};

#[derive(Clone)]
pub struct Scheduler(Arc<Shared>);
struct Shared {
    state: Mutex<State>,
    wake: Notify,
    events: broadcast::Sender<()>,
}
#[derive(Default)]
struct State {
    running: bool,
    epoch: u64,
    next: u64,
    limits: HashMap<String, u32>,
    models: HashMap<String, Option<Vec<String>>>,
    active: HashMap<String, usize>,
    waiting: VecDeque<(u64, Vec<String>)>,
}
pub struct Slot {
    scheduler: Scheduler,
    id: String,
}
struct Waiting {
    scheduler: Scheduler,
    ticket: u64,
}
pub struct Admission {
    pub route: Route,
    pub permits: Permits,
    pub slot: Slot,
}
#[derive(Debug, PartialEq)]
pub enum Rejected {
    Stopped,
    Unavailable,
    Full,
    Timeout,
    Cooling(u64),
    Model,
}
pub struct Budget {
    remaining: Duration,
    pub trace: crate::usage::RoutingTrace,
}
impl Budget {
    pub fn new(seconds: u64) -> Self {
        Self {
            remaining: Duration::from_secs(seconds),
            trace: Default::default(),
        }
    }
}
impl Scheduler {
    pub fn new(events: broadcast::Sender<()>) -> Self {
        Self(Arc::new(Shared {
            state: Mutex::new(State::default()),
            wake: Notify::new(),
            events,
        }))
    }
    pub fn configure(&self, providers: &[Provider], running: bool) {
        let mut s = self.0.state.lock().unwrap();
        if s.running != running {
            s.epoch += 1;
        }
        s.running = running;
        s.limits = providers
            .iter()
            .map(|p| (p.id.clone(), p.max_concurrency))
            .collect();
        s.models = providers
            .iter()
            .map(|p| (p.id.clone(), p.allowed_models.clone()))
            .collect();
        drop(s);
        self.signal();
    }
    pub fn counts(&self) -> (HashMap<String, usize>, usize) {
        let s = self.0.state.lock().unwrap();
        (s.active.clone(), s.waiting.len())
    }
    pub fn signal(&self) {
        self.0.wake.notify_waiters();
        let _ = self.0.events.send(());
    }
    #[cfg(test)]
    pub async fn acquire(
        &self,
        routes: &[Route],
        manual: bool,
        max_waiting: usize,
        budget: &mut Budget,
    ) -> Result<Admission, Rejected> {
        self.acquire_for(routes, manual, max_waiting, budget, &Requirement::Resource)
            .await
    }
    pub async fn acquire_for(
        &self,
        routes: &[Route],
        manual: bool,
        max_waiting: usize,
        budget: &mut Budget,
        requirement: &Requirement,
    ) -> Result<Admission, Rejected> {
        let started = Instant::now();
        let epoch = self.0.state.lock().unwrap().epoch;
        let mut waiting: Option<Waiting> = None;
        let deadline = tokio::time::Instant::now() + budget.remaining;
        let mut cooling;
        let result = loop {
            let notified = self.0.wake.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            {
                let mut s = self.0.state.lock().unwrap();
                if !s.running || s.epoch != epoch {
                    break Err(Rejected::Stopped);
                }
                let matching: Vec<_> = routes
                    .iter()
                    .filter(|r| s.limits.contains_key(&r.provider.id))
                    .collect();
                if matching.is_empty() {
                    break Err(Rejected::Unavailable);
                }
                for r in &matching {
                    if !requirement.allows(s.models.get(&r.provider.id).and_then(|m| m.as_deref()))
                    {
                        budget.trace.push(
                            &r.provider.id,
                            &r.provider.name,
                            "model",
                            s.active.get(&r.provider.id).copied().unwrap_or(0),
                            s.limits[&r.provider.id],
                            0,
                        );
                    }
                }
                let matching: Vec<_> = matching
                    .into_iter()
                    .filter(|r| {
                        requirement.allows(s.models.get(&r.provider.id).and_then(|m| m.as_deref()))
                    })
                    .collect();
                if matching.is_empty() {
                    break Err(Rejected::Model);
                }
                let eligible = matching;
                let mut any_ready = false;
                let mut retry_in = u64::MAX;
                let ticket = waiting.as_ref().map(|w| w.ticket);
                if let Some(ticket) = ticket {
                    if let Some(entry) = s.waiting.iter_mut().find(|(id, _)| *id == ticket) {
                        entry.1 = eligible.iter().map(|r| r.provider.id.clone()).collect();
                    }
                }
                let mut accepted = None;
                let mut retry_selection = true;
                'selection: loop {
                    for (position, route) in eligible.iter().enumerate() {
                        let id = &route.provider.id;
                        let limit = s.limits[id];
                        let active = s.active.get(id).copied().unwrap_or(0);
                        let health = route.provider_circuit.health();
                        let proxy_health = route.proxy_circuit.as_ref().map(|c| c.health());
                        let blocked = proxy_health
                            .as_ref()
                            .filter(|h| !h.available)
                            .map(|h| ("proxy_cooldown", h))
                            .or_else(|| {
                                (!health.available).then(|| {
                                    (
                                        health
                                            .cooldown_reason
                                            .as_deref()
                                            .unwrap_or("half_open_probe"),
                                        &health,
                                    )
                                })
                            });
                        if let Some((reason, health)) = blocked {
                            retry_in = retry_in.min(health.retry_in.max(1));
                            budget.trace.push(
                                id,
                                &route.provider.name,
                                reason,
                                active,
                                limit,
                                health.retry_in,
                            );
                            continue;
                        }
                        any_ready = true;
                        if limit != 0 && active >= limit as usize {
                            budget.trace.push(
                                id,
                                &route.provider.name,
                                "capacity",
                                active,
                                limit,
                                0,
                            );
                            continue;
                        }
                        // A waiter pinned to another provider never blocks this provider.
                        if s.waiting
                            .iter()
                            .take_while(|(id, _)| Some(*id) != ticket)
                            .any(|(_, ids)| ids.contains(id))
                        {
                            budget
                                .trace
                                .push(id, &route.provider.name, "fifo", active, limit, 0);
                            continue;
                        }
                        // Capacity cannot change under this lock, but a higher-priority
                        // circuit may recover between its health read and this reservation.
                        if retry_selection
                            && eligible[..position].iter().any(|earlier| {
                                let id = &earlier.provider.id;
                                let limit = s.limits[id];
                                (limit == 0
                                    || s.active.get(id).copied().unwrap_or(0) < limit as usize)
                                    && earlier.provider_circuit.health().available
                                    && earlier
                                        .proxy_circuit
                                        .as_ref()
                                        .is_none_or(|c| c.health().available)
                                    && !s
                                        .waiting
                                        .iter()
                                        .take_while(|(n, _)| Some(*n) != ticket)
                                        .any(|(_, ids)| ids.contains(id))
                            })
                        {
                            retry_selection = false;
                            continue 'selection;
                        }
                        if let Some(permits) = Permits::acquire(route, manual) {
                            budget.trace.push(
                                id,
                                &route.provider.name,
                                "selected",
                                active + 1,
                                limit,
                                0,
                            );
                            *s.active.entry(id.clone()).or_default() += 1;
                            accepted = Some(Admission {
                                route: (**route).clone(),
                                permits,
                                slot: Slot {
                                    scheduler: self.clone(),
                                    id: id.clone(),
                                },
                            });
                            break;
                        }
                        budget.trace.push(
                            id,
                            &route.provider.name,
                            "state_changed",
                            active,
                            limit,
                            0,
                        );
                        if retry_selection {
                            retry_selection = false;
                            any_ready = false;
                            retry_in = u64::MAX;
                            continue 'selection;
                        }
                    }
                    break;
                }
                cooling = (!any_ready).then_some(if retry_in == u64::MAX { 1 } else { retry_in });
                if let Some(admission) = accepted {
                    break Ok(admission);
                }
                if waiting.is_none() {
                    if s.waiting.len() >= max_waiting {
                        break Err(Rejected::Full);
                    }
                    s.next += 1;
                    let ticket = s.next;
                    s.waiting.push_back((
                        ticket,
                        eligible.iter().map(|r| r.provider.id.clone()).collect(),
                    ));
                    waiting = Some(Waiting {
                        scheduler: self.clone(),
                        ticket,
                    });
                    let _ = self.0.events.send(());
                }
            }
            tokio::select! {
                _ = notified => {},
                _ = tokio::time::sleep_until(deadline) => break Err(cooling.map_or(Rejected::Timeout, Rejected::Cooling)),
                // Circuit cooldowns may expire without a separate request or UI event.
                _ = tokio::time::sleep(Duration::from_millis(200)) => {},
            }
        };
        if waiting.is_some() {
            budget.remaining = budget.remaining.saturating_sub(started.elapsed());
        }
        drop(waiting);
        self.signal();
        result
    }
}
impl Drop for Slot {
    fn drop(&mut self) {
        let mut s = self.scheduler.0.state.lock().unwrap();
        if let Some(n) = s.active.get_mut(&self.id) {
            *n = n.saturating_sub(1);
        }
        drop(s);
        self.scheduler.signal();
    }
}
impl Drop for Waiting {
    fn drop(&mut self) {
        self.scheduler
            .0
            .state
            .lock()
            .unwrap()
            .waiting
            .retain(|(id, _)| *id != self.ticket);
        self.scheduler.signal();
    }
}
