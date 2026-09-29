//! Shared proxy configuration. Runtime health and request pools stay client-local.
use super::{
    model::{Edit, Proxy, Store},
    Shared,
};
use crate::storage::{self, AppError, Result};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex, Weak},
};

pub struct Network {
    path: PathBuf,
    pub mutation: Mutex<()>,
    state: Mutex<(Vec<Proxy>, String)>,
    pub gateways: Mutex<Vec<Weak<Shared>>>,
    pub ports: Arc<Mutex<Vec<u16>>>,
}
impl Network {
    pub fn new(data: &Path, legacy: &[Proxy]) -> Result<Arc<Self>> {
        let path = data.join("proxy-profiles.json");
        let raw = storage::read_optional(&path)?;
        let proxies: Vec<Proxy> = raw
            .as_deref()
            .map(serde_json::from_slice)
            .transpose()
            .map_err(|_| AppError::new("PROXY", "代理配置无法读取"))?
            .unwrap_or_else(|| legacy.to_vec());
        let revision = if raw.is_none() && !proxies.is_empty() {
            let bytes = serde_json::to_vec_pretty(&proxies).unwrap();
            storage::atomic_write(&path, &bytes, Some("missing"))?;
            storage::digest(&bytes)
        } else {
            storage::revision(raw.as_deref())
        };
        Ok(Arc::new(Self {
            path,
            mutation: Mutex::new(()),
            state: Mutex::new((proxies, revision)),
            gateways: Mutex::new(Vec::new()),
            ports: Arc::new(Mutex::new(Vec::new())),
        }))
    }
    pub fn snapshot(&self) -> (Vec<Proxy>, String) {
        self.state.lock().unwrap().clone()
    }
    pub fn register(&self, shared: &Arc<Shared>) {
        self.gateways.lock().unwrap().push(Arc::downgrade(shared));
        self.update_ports();
    }
    pub fn update_ports(&self) {
        let all = self.gateways.lock().unwrap().clone();
        let ports = all
            .iter()
            .filter_map(Weak::upgrade)
            .map(|g| g.inner.lock().unwrap().store.settings.port)
            .collect();
        *self.ports.lock().unwrap() = ports;
    }
    // Called under mutation, before taking any client lock.
    pub fn edit(&self, edit: Edit) -> Result<()> {
        let all: Vec<_> = self
            .gateways
            .lock()
            .unwrap()
            .iter()
            .filter_map(Weak::upgrade)
            .collect();
        let (proxies, expected) = self.snapshot();
        let providers = all
            .iter()
            .flat_map(|g| g.inner.lock().unwrap().store.providers.clone())
            .collect();
        let mut store = Store {
            proxies,
            providers,
            ..Default::default()
        };
        store.edit(edit, false)?;
        let bytes = serde_json::to_vec_pretty(&store.proxies).unwrap();
        storage::atomic_write(&self.path, &bytes, Some(&expected))?;
        *self.state.lock().unwrap() = (store.proxies.clone(), storage::digest(&bytes));
        for g in all {
            g.inner.lock().unwrap().store.proxies = store.proxies.clone();
            g.clients.lock().unwrap().clear();
            let _ = g.events.send(());
        }
        Ok(())
    }
}
