mod admission;
mod circuit;
mod connector;
mod forward;
mod model;
mod quota;
mod replay;
mod websocket;
pub use quota::QuotaView;
mod takeover;
#[cfg(test)]
mod tests;
use crate::storage::{self, AppError, Result};
use circuit::{Circuit, Health};
use connector::Connector;
use hyper_util::{client::legacy::Client, rt::TokioExecutor};
pub use model::{Edit, Settings};
use model::{Provider, Proxy, Store};
use replay::WireBody;
use serde::Serialize;
use std::{
    collections::{HashMap, VecDeque},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};
use tokio::sync::{broadcast, watch};
type HttpClient = Client<Connector, WireBody>;
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ProviderView {
    pub id: String,
    pub name: String,
    base_url: String,
    proxy_id: Option<String>,
    queued: bool,
    health: Health,
    quota_version: String,
    quota: Option<QuotaView>,
    pub max_concurrency: u32,
    pub active_requests: usize,
}
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ProxyView {
    id: String,
    name: String,
    host: String,
    port: u16,
    username: String,
    has_password: bool,
    health: Health,
}
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Recent {
    pub provider: String,
    pub proxy: Option<String>,
    pub status: Option<u16>,
    pub elapsed_ms: u64,
    pub retries: usize,
    pub category: String,
    pub at: u64,
}
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct View {
    pub revision: String,
    pub running: bool,
    pub address: String,
    pub mode: String,
    pub selected: Option<String>,
    pub last_successful: Option<String>,
    pub config_revision: Option<String>,
    pub config_provider: Option<String>,
    pub config_state: String,
    pub config_error: Option<String>,
    pub providers: Vec<ProviderView>,
    pub proxies: Vec<ProxyView>,
    pub settings: Settings,
    pub active_connections: usize,
    pub waiting_requests: usize,
    pub recent: Vec<Recent>,
    pub error: Option<String>,
    pub recovery_pending: bool,
}
struct Inner {
    store: Store,
    revision: String,
    running: bool,
    shutdown: Option<watch::Sender<bool>>,
    listener_task: Option<tokio::task::JoinHandle<()>>,
    error: Option<String>,
    circuits: HashMap<String, Circuit>,
    recent: VecDeque<Recent>,
    affinity: HashMap<String, (String, Instant)>,
    last_successful: Option<String>,
    home: Option<PathBuf>,
}
struct Shared {
    inner: Mutex<Inner>,
    lifecycle: tokio::sync::Mutex<()>,
    clients: Mutex<HashMap<String, HttpClient>>,
    data: PathBuf,
    spool: tempfile::TempDir,
    active: AtomicUsize,
    events: broadcast::Sender<()>,
    quota: quota::Service,
    admission: admission::Scheduler,
}
#[derive(Clone)]
pub struct Gateway(Arc<Shared>);
#[derive(Clone)]
struct Route {
    provider: Provider,
    proxy: Option<Proxy>,
    client: HttpClient,
    provider_circuit: Circuit,
    proxy_circuit: Option<Circuit>,
}
fn pkey(p: &Provider) -> String {
    format!("provider:{}:{}", p.id, p.version)
}
fn xkey(p: &Proxy) -> String {
    format!("proxy:{}:{}", p.id, p.version)
}
impl Gateway {
    pub fn new(data: PathBuf) -> Result<Self> {
        storage::private_dir(&data)?;
        let error = takeover::recover(&data).err().map(|e| e.message);
        let (store, revision) = Store::load(&data.join("gateway.json"))?;
        let spool = tempfile::Builder::new()
            .prefix("gateway-spool-")
            .tempdir_in(&data)
            .map_err(storage::io_error)?;
        storage::protect(spool.path(), true)?;
        let (events, _) = broadcast::channel(32);
        let admission = admission::Scheduler::new(events.clone());
        admission.configure(&store.providers, false);
        Ok(Self(Arc::new(Shared {
            inner: Mutex::new(Inner {
                store,
                revision,
                running: false,
                shutdown: None,
                listener_task: None,
                error,
                circuits: HashMap::new(),
                recent: VecDeque::new(),
                affinity: HashMap::new(),
                last_successful: None,
                home: None,
            }),
            lifecycle: tokio::sync::Mutex::new(()),
            clients: Mutex::new(HashMap::new()),
            data,
            spool,
            active: AtomicUsize::new(0),
            events,
            quota: quota::Service::new(),
            admission,
        })))
    }
    pub fn subscribe(&self) -> broadcast::Receiver<()> {
        self.0.events.subscribe()
    }
    fn changed(&self) {
        let _ = self.0.events.send(());
    }
    pub fn view(&self) -> View {
        let s = self.0.inner.lock().unwrap();
        let (occupied, waiting) = self.0.admission.counts();
        let health = |key: String| s.circuits.get(&key).cloned().unwrap_or_default().health();
        let config = s.home.as_ref().map(|h| takeover::read(h));
        let config_revision = config
            .as_ref()
            .and_then(|r| r.as_ref().ok())
            .map(|(r, _)| r.clone());
        let mut config_state = "unknown";
        let mut config_provider = None;
        if let Some(Ok((_, pair))) = &config {
            config_state = "unsaved";
            if pair
                == &takeover::Pair::new(
                    &format!("http://127.0.0.1:{}/v1", s.store.settings.port),
                    &s.store.local_token,
                )
            {
                config_state = "gateway";
            } else if let Some(p) = s
                .store
                .providers
                .iter()
                .find(|p| pair == &takeover::Pair::new(&p.base_url, &p.token))
            {
                config_state = "provider";
                config_provider = Some(p.id.clone());
            }
        }
        View {
            config_revision,
            config_provider,
            config_state: config_state.into(),
            config_error: config.and_then(|r| r.err()).map(|e| e.message),
            last_successful: s.last_successful.clone(),
            revision: s.revision.clone(),
            running: s.running,
            address: format!("http://127.0.0.1:{}/v1", s.store.settings.port),
            mode: s.store.mode.clone(),
            selected: s.store.selected.clone(),
            settings: s.store.settings.clone(),
            providers: s
                .store
                .providers
                .iter()
                .map(|p| ProviderView {
                    id: p.id.clone(),
                    name: p.name.clone(),
                    base_url: p.base_url.clone(),
                    proxy_id: p.proxy_id.clone(),
                    queued: p.queued,
                    health: health(pkey(p)),
                    quota_version: Self::quota_version(&s.store, p),
                    max_concurrency: p.max_concurrency,
                    active_requests: occupied.get(&p.id).copied().unwrap_or(0),
                    quota: self
                        .0
                        .quota
                        .cached(&p.id, &Self::quota_version(&s.store, p)),
                })
                .collect(),
            proxies: s
                .store
                .proxies
                .iter()
                .map(|p| ProxyView {
                    id: p.id.clone(),
                    name: p.name.clone(),
                    host: p.host.clone(),
                    port: p.port,
                    username: p.username.clone(),
                    has_password: !p.password.is_empty(),
                    health: health(xkey(p)),
                })
                .collect(),
            active_connections: self.0.active.load(Ordering::Relaxed),
            waiting_requests: waiting,
            recent: s.recent.iter().cloned().collect(),
            error: s.error.clone(),
            recovery_pending: self.0.data.join("gateway-recovery.json").exists(),
        }
    }
    pub fn observe_home(&self, home: &Path) {
        self.0.inner.lock().unwrap().home = Some(home.to_owned());
    }
    fn exit_provider<'a>(store: &'a Store, last: Option<&str>) -> Result<&'a Provider> {
        let p = if store.mode == "manual" {
            store
                .providers
                .iter()
                .find(|p| Some(&p.id) == store.selected.as_ref())
        } else {
            last.and_then(|id| store.providers.iter().find(|p| p.id == id))
                .or_else(|| store.providers.iter().find(|p| p.queued))
        };
        p.ok_or_else(|| {
            AppError::new(
                "PROVIDER",
                "没有关闭网关时可用的供应商，请选择供应商或加入故障转移队列",
            )
        })
    }
    pub fn guarded_home(&self) -> bool {
        self.view().running || self.0.data.join("gateway-recovery.json").exists()
    }
    pub fn edit(&self, edit: Edit, expected: &str, home: &Path) -> Result<View> {
        self.edit_checked(edit, expected, home, None)
    }
    pub fn edit_checked(
        &self,
        edit: Edit,
        expected: &str,
        home: &Path,
        config_revision: Option<&str>,
    ) -> Result<View> {
        let mut s = self.0.inner.lock().unwrap();
        s.home = Some(home.to_owned());
        if expected != s.revision {
            return Err(AppError::new(
                "CONFLICT",
                "网关设置已变化，请使用最新状态重试",
            ));
        }
        if !s.running && self.0.data.join("gateway-recovery.json").exists() {
            return Err(AppError::new("RECOVERY", "请先处理未完成的配置事务"));
        }
        let direct_select = !s.running && matches!(&edit, Edit::Select { .. });
        let mut next = s.store.clone();
        match edit {
            Edit::Reset { id, proxy } => {
                let prefix = format!("{}:{id}:", if proxy { "proxy" } else { "provider" });
                for (k, c) in &s.circuits {
                    if k.starts_with(&prefix) {
                        c.reset();
                    }
                }
            }
            Edit::Import => {
                let (base_url, token) = takeover::import(home)?;
                next.edit(
                    Edit::SaveProvider {
                        id: None,
                        base_url,
                        token,
                    },
                    s.running,
                )?;
            }
            edit => next.edit(edit, s.running)?,
        }
        let revision = if s.running || direct_select {
            let p = Self::exit_provider(&next, s.last_successful.as_deref())?;
            let target = takeover::Pair::new(&p.base_url, &p.token);
            next.resume = Some(model::Resume {
                home: home.to_owned(),
                desired: s.running,
                pair_hash: target.fingerprint(),
            });
            let path = self.0.data.join("gateway.json");
            let old = storage::read_optional(&path)?;
            if storage::revision(old.as_deref()) != s.revision {
                return Err(AppError::new("CONFLICT", "网关存储已被外部修改"));
            }
            let after = serde_json::to_string_pretty(&next)
                .map_err(|_| AppError::new("STORE", "无法写入网关设置"))?;
            takeover::commit_store(
                &self.0.data,
                home,
                old.map(|v| String::from_utf8(v).unwrap()),
                after.clone(),
                target,
                s.running,
                config_revision,
            )?;
            storage::digest(after.as_bytes())
        } else {
            next.persist(&self.0.data.join("gateway.json"), &s.revision)?
        };
        if s.last_successful
            .as_ref()
            .is_some_and(|id| !next.providers.iter().any(|p| &p.id == id))
        {
            s.last_successful = None;
        }
        s.store = next;
        s.revision = revision;
        self.0.admission.configure(&s.store.providers, s.running);
        self.0.quota.retain(
            &s.store
                .providers
                .iter()
                .map(|p| (p.id.clone(), Self::quota_version(&s.store, p)))
                .collect(),
        );
        drop(s);
        self.0.clients.lock().unwrap().clear();
        self.changed();
        Ok(self.view())
    }
    pub async fn start(&self, expected: &str, home: &Path) -> Result<View> {
        self.start_checked(expected, home, None).await
    }
    pub async fn start_checked(
        &self,
        expected: &str,
        home: &Path,
        config_revision: Option<&str>,
    ) -> Result<View> {
        let _guard = self.0.lifecycle.lock().await;
        let (port, token) = {
            let s = self.0.inner.lock().unwrap();
            if expected != s.revision {
                return Err(AppError::new("CONFLICT", "网关设置已变化，请重试"));
            }
            if s.running {
                drop(s);
                return Ok(self.view());
            }
            if s.store.providers.is_empty() {
                return Err(AppError::new("PROVIDER", "请先添加供应商"));
            }
            (s.store.settings.port, s.store.local_token.clone())
        };
        let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port))
            .await
            .map_err(|_| AppError::new("PORT", "无法绑定本地端口，请检查端口占用或修改端口"))?;
        {
            let s = self.0.inner.lock().unwrap();
            if s.revision != expected {
                return Err(AppError::new("CONFLICT", "绑定端口期间设置已变化，请重试"));
            }
        }
        let (tx, rx) = watch::channel(false);
        {
            let mut s = self.0.inner.lock().unwrap();
            if s.revision != expected {
                return Err(AppError::new("CONFLICT", "设置已变化，请重试"));
            }
            let p = Self::exit_provider(&s.store, None)?;
            let exit = takeover::Pair::new(&p.base_url, &p.token);
            let (current, _) = takeover::read(home)?;
            let mut next = s.store.clone();
            next.resume = Some(model::Resume {
                home: home.to_owned(),
                desired: true,
                pair_hash: exit.fingerprint(),
            });
            let before = storage::read_optional(&self.0.data.join("gateway.json"))?;
            if storage::revision(before.as_deref()) != s.revision {
                return Err(AppError::new("CONFLICT", "网关存储已被外部修改"));
            }
            let after = serde_json::to_string_pretty(&next)
                .map_err(|_| AppError::new("STORE", "无法生成网关启动事务"))?;
            takeover::attach_store(
                &self.0.data,
                home,
                port,
                &token,
                exit,
                config_revision.unwrap_or(&current),
                Some((before.map(|b| String::from_utf8(b).unwrap()), after.clone())),
            )?;
            s.store = next;
            s.revision = storage::digest(after.as_bytes());
            s.home = Some(home.to_owned());
            s.last_successful = None;
            s.running = true;
            s.shutdown = Some(tx);
            s.error = None;
            self.0.admission.configure(&s.store.providers, true);
        }
        let gateway = self.clone();
        let task = tokio::spawn(async move {
            forward::serve(gateway, listener, rx).await;
        });
        self.0.inner.lock().unwrap().listener_task = Some(task);
        self.changed();
        Ok(self.view())
    }
    pub async fn stop(&self) -> Result<View> {
        self.stop_checked(None).await
    }
    pub async fn stop_checked(&self, expected_config: Option<&str>) -> Result<View> {
        self.stop_internal(expected_config, false).await
    }
    pub async fn stop_for_exit(&self) -> Result<View> {
        self.stop_internal(None, true).await
    }
    pub async fn resume(&self, home: &Path) -> Result<()> {
        if self.0.inner.lock().unwrap().running {
            return Ok(());
        }
        let (revision, intent) = {
            let s = self.0.inner.lock().unwrap();
            (s.revision.clone(), s.store.resume.clone())
        };
        let Some(intent) = intent.filter(|i| i.desired) else {
            return Ok(());
        };
        let result = async {
            if intent.home != home {
                return Err(AppError::new(
                    "RESUME_CONFLICT",
                    "Codex 目录已变化，自动恢复已停止",
                ));
            }
            let (config_revision, pair) = takeover::read(home)?;
            if pair.fingerprint() != intent.pair_hash {
                return Err(AppError::new(
                    "RESUME_CONFLICT",
                    "custom 的地址或 Token 在退出后已被修改，自动恢复已停止",
                ));
            }
            self.start_checked(&revision, home, Some(&config_revision))
                .await
                .map(|_| ())
        }
        .await;
        if let Err(error) = &result {
            self.0.inner.lock().unwrap().error = Some(error.message.clone());
            self.changed();
        }
        result
    }
    async fn stop_internal(
        &self,
        expected_config: Option<&str>,
        preserve_intent: bool,
    ) -> Result<View> {
        let _guard = self.0.lifecycle.lock().await;
        let listener_task = {
            let mut s = self.0.inner.lock().unwrap();
            if let (Some(expected), Some(home)) = (expected_config, &s.home) {
                if takeover::read(home)?.0 != expected {
                    return Err(AppError::new("CONFLICT", "配置已变化，请刷新后重试"));
                }
            }
            if !preserve_intent {
                let mut next = s.store.clone();
                if let Some(intent) = next.resume.as_mut().filter(|r| r.desired) {
                    intent.desired = false;
                    if s.running {
                        let home = s
                            .home
                            .as_ref()
                            .ok_or_else(|| AppError::new("STATE", "缺少运行目录"))?;
                        let exit = takeover::exit_pair(&self.0.data)?;
                        let target = match exit.as_ref() {
                            Some(pair) => pair.clone(),
                            None => {
                                let (_, pair) = takeover::read(home)?;
                                if pair.fingerprint() != intent.pair_hash {
                                    return Err(AppError::new(
                                        "RECOVERY",
                                        "缺少停止目标且配置未恢复",
                                    ));
                                }
                                pair
                            }
                        };
                        intent.pair_hash = target.fingerprint();
                        let before = storage::read_optional(&self.0.data.join("gateway.json"))?;
                        if storage::revision(before.as_deref()) != s.revision {
                            return Err(AppError::new("CONFLICT", "网关存储已被外部修改"));
                        }
                        let after = serde_json::to_string_pretty(&next)
                            .map_err(|_| AppError::new("STORE", "无法生成停止事务"))?;
                        if exit.is_some() {
                            takeover::commit_store(
                                &self.0.data,
                                home,
                                before.map(|b| String::from_utf8(b).unwrap()),
                                after.clone(),
                                target,
                                true,
                                expected_config,
                            )?;
                        } else {
                            storage::atomic_write(
                                &self.0.data.join("gateway.json"),
                                after.as_bytes(),
                                Some(&s.revision),
                            )?;
                        }
                        s.revision = storage::digest(after.as_bytes());
                    } else {
                        s.revision =
                            next.persist(&self.0.data.join("gateway.json"), &s.revision)?;
                    }
                    s.store = next;
                }
            }
            if let Err(e) = takeover::recover(&self.0.data) {
                s.error = Some(e.message.clone());
                drop(s);
                self.changed();
                return Err(e);
            }
            // Recovery may have completed a store/config transaction interrupted by an I/O failure.
            let (store, revision) = Store::load(&self.0.data.join("gateway.json"))?;
            s.store = store;
            s.revision = revision;
            if let Some(tx) = s.shutdown.take() {
                let _ = tx.send(true);
            }
            s.running = false;
            self.0.admission.configure(&s.store.providers, false);
            s.error = None;
            s.listener_task.take()
        };
        if let Some(task) = listener_task {
            let _ = task.await;
        }
        self.0.clients.lock().unwrap().clear();
        self.changed();
        Ok(self.view())
    }
    fn successful_response(&self, provider: &Provider) {
        let mut s = self.0.inner.lock().unwrap();
        if !s.running
            || !s
                .store
                .providers
                .iter()
                .any(|p| p.id == provider.id && p.version == provider.version)
        {
            return;
        }
        if s.store.mode == "auto" {
            let target = takeover::Pair::new(&provider.base_url, &provider.token);
            let update = (|| -> Result<()> {
                if s.store
                    .resume
                    .as_ref()
                    .is_some_and(|r| r.pair_hash == target.fingerprint())
                {
                    return Ok(());
                }
                let home = s
                    .home
                    .as_ref()
                    .ok_or_else(|| AppError::new("STATE", "缺少运行目录"))?;
                let mut next = s.store.clone();
                next.resume = Some(model::Resume {
                    home: home.clone(),
                    desired: true,
                    pair_hash: target.fingerprint(),
                });
                let before = storage::read_optional(&self.0.data.join("gateway.json"))?;
                if storage::revision(before.as_deref()) != s.revision {
                    return Err(AppError::new("CONFLICT", "网关存储已被外部修改"));
                }
                let after = serde_json::to_string_pretty(&next)
                    .map_err(|_| AppError::new("STORE", "无法记录最近供应商"))?;
                takeover::commit_store(
                    &self.0.data,
                    home,
                    before.map(|b| String::from_utf8(b).unwrap()),
                    after.clone(),
                    target,
                    true,
                    None,
                )?;
                s.store = next;
                s.revision = storage::digest(after.as_bytes());
                Ok(())
            })();
            if let Err(e) = update {
                s.error = Some(e.message);
                drop(s);
                self.changed();
                return;
            }
        }
        s.last_successful = Some(provider.id.clone());
        drop(s);
        self.changed();
    }
    fn quota_version(store: &Store, p: &Provider) -> String {
        format!(
            "{}:{}",
            p.version,
            p.proxy_id
                .as_ref()
                .and_then(|id| store.proxies.iter().find(|x| &x.id == id))
                .map(|x| x.version.as_str())
                .unwrap_or("direct")
        )
    }
    pub fn quota_events(&self) -> broadcast::Receiver<QuotaView> {
        self.0.quota.subscribe()
    }
    pub async fn query_quota(&self, id: &str, force: bool) -> Result<QuotaView> {
        let input = {
            let s = self.0.inner.lock().unwrap();
            let p = s
                .store
                .providers
                .iter()
                .find(|p| p.id == id)
                .ok_or_else(|| AppError::new("PROVIDER", "供应商不存在"))?;
            let proxy = p
                .proxy_id
                .as_ref()
                .and_then(|id| s.store.proxies.iter().find(|x| &x.id == id))
                .cloned();
            quota::Query {
                id: p.id.clone(),
                version: Self::quota_version(&s.store, p),
                base: p.base_url.clone(),
                token: p.token.clone(),
                client: Client::builder(TokioExecutor::new())
                    .retry_canceled_requests(false)
                    .build(Connector::new(
                        proxy,
                        Duration::from_secs(10),
                        s.store.settings.port,
                    )),
            }
        };
        let version = input.version.clone();
        let result = self.0.quota.query(input, force).await?;
        let s = self.0.inner.lock().unwrap();
        if !s
            .store
            .providers
            .iter()
            .any(|p| p.id == id && Self::quota_version(&s.store, p) == version)
        {
            return Err(AppError::new("STALE", "供应商配置已变化，已丢弃旧额度结果"));
        }
        Ok(result)
    }
    pub async fn test_connection(&self, id: &str) -> Result<u64> {
        let (p, proxy, cfg) = {
            let s = self.0.inner.lock().unwrap();
            let p = s
                .store
                .providers
                .iter()
                .find(|p| p.id == id)
                .ok_or_else(|| AppError::new("PROVIDER", "供应商不存在"))?
                .clone();
            let proxy = p
                .proxy_id
                .as_ref()
                .and_then(|id| s.store.proxies.iter().find(|x| &x.id == id))
                .cloned();
            (p, proxy, s.store.settings.clone())
        };
        let connector = Connector::new(proxy, Duration::from_secs(cfg.connect_seconds), cfg.port);
        let began = Instant::now();
        let uri = p
            .base_url
            .parse()
            .map_err(|_| AppError::new("URL", "base_url 无效"))?;
        connector
            .connect(uri)
            .await
            .map_err(|e| AppError::new("CONNECT", &e.to_string()))?;
        Ok(began.elapsed().as_millis() as u64)
    }
    fn route(&self, id: &str) -> Option<Route> {
        let mut s = self.0.inner.lock().unwrap();
        let provider = s.store.providers.iter().find(|p| p.id == id)?.clone();
        let proxy = provider
            .proxy_id
            .as_ref()
            .and_then(|id| s.store.proxies.iter().find(|p| &p.id == id))
            .cloned();
        let provider_circuit = s.circuits.entry(pkey(&provider)).or_default().clone();
        let proxy_circuit = proxy
            .as_ref()
            .map(|p| s.circuits.entry(xkey(p)).or_default().clone());
        let key = format!(
            "{}:{}:{}",
            pkey(&provider),
            proxy.as_ref().map(xkey).unwrap_or_default(),
            s.store.settings.connect_seconds
        );
        let mut clients = self.0.clients.lock().unwrap();
        let client = clients
            .entry(key)
            .or_insert_with(|| {
                Client::builder(TokioExecutor::new())
                    .pool_idle_timeout(Duration::from_secs(60))
                    .pool_max_idle_per_host(8)
                    .retry_canceled_requests(false)
                    .build(Connector::new(
                        proxy.clone(),
                        Duration::from_secs(s.store.settings.connect_seconds),
                        s.store.settings.port,
                    ))
            })
            .clone();
        Some(Route {
            provider,
            proxy,
            client,
            provider_circuit,
            proxy_circuit,
        })
    }
    fn remember(&self, id: &str, provider: &str) {
        if id.is_empty() || id.len() > 1024 {
            return;
        }
        let mut s = self.0.inner.lock().unwrap();
        s.affinity
            .retain(|_, (_, at)| at.elapsed() < Duration::from_secs(3600));
        if s.affinity.len() >= 4096 {
            if let Some(old) = s
                .affinity
                .iter()
                .min_by_key(|(_, (_, at))| *at)
                .map(|(id, _)| id.clone())
            {
                s.affinity.remove(&old);
            }
        }
        s.affinity
            .insert(id.into(), (provider.into(), Instant::now()));
    }
    fn record(
        &self,
        route: &Route,
        status: Option<u16>,
        began: Instant,
        retries: usize,
        category: &str,
    ) {
        let mut s = self.0.inner.lock().unwrap();
        s.recent.push_front(Recent {
            provider: route.provider.name.clone(),
            proxy: route.proxy.as_ref().map(|p| p.name.clone()),
            status,
            elapsed_ms: began.elapsed().as_millis() as u64,
            retries,
            category: category.into(),
            at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
        });
        s.recent.truncate(40);
        drop(s);
        self.changed();
    }
}
struct Active(Gateway);
impl Active {
    fn new(g: Gateway) -> Self {
        g.0.active.fetch_add(1, Ordering::Relaxed);
        g.changed();
        Self(g)
    }
}
impl Drop for Active {
    fn drop(&mut self) {
        self.0 .0.active.fetch_sub(1, Ordering::Relaxed);
        self.0.changed();
    }
}
