//! Shared gateway ports prevent self-routing and cross-client loops.
use super::Shared;
use std::sync::{Arc, Mutex, Weak};
#[derive(Default)]
pub struct Registry {
    pub mutation: Mutex<()>,
    pub gateways: Mutex<Vec<Weak<Shared>>>,
    pub ports: Arc<Mutex<Vec<u16>>>,
}
impl Registry {
    pub fn register(&self, shared: &Arc<Shared>) {
        self.gateways.lock().unwrap().push(Arc::downgrade(shared));
        self.update_ports();
    }
    pub fn update_ports(&self) {
        let all = self.gateways.lock().unwrap().clone();
        *self.ports.lock().unwrap() = all
            .iter()
            .filter_map(Weak::upgrade)
            .map(|g| g.inner.lock().unwrap().store.settings.port)
            .collect();
    }
}
