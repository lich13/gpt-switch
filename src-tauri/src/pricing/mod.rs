//! Independent price catalog. Sub2API's refresh/cache behavior is reimplemented;
//! no LGPL source is incorporated. models.dev selection follows cc-switch (MIT).
pub mod model;
use crate::storage::{self, AppError, Result};
use model::{litellm, models_dev};
pub use model::{Cost, Price, RemoteModel};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::sync::broadcast;
pub const LITELLM: &str =
    "https://raw.githubusercontent.com/BerriAI/litellm/main/model_prices_and_context_window.json";
const MODELS_DEV: &str = "https://models.dev/api.json";
const MAX_BYTES: usize = 32 * 1024 * 1024;
pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub auto_update: bool,
    pub models_dev_enabled: bool,
    pub include_common: bool,
    pub selected: BTreeSet<String>,
    pub excluded: BTreeSet<String>,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            auto_update: true,
            models_dev_enabled: false,
            include_common: true,
            selected: BTreeSet::new(),
            excluded: BTreeSet::new(),
        }
    }
}
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncInfo {
    pub checked_at: Option<u64>,
    pub updated_at: Option<u64>,
    pub error: Option<String>,
    pub etag: Option<String>,
    pub hash: Option<String>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct File {
    version: u32,
    settings: Settings,
    remote: BTreeMap<String, Price>,
    models_dev: BTreeMap<String, Price>,
    overrides: BTreeMap<String, Price>,
    disabled: BTreeSet<String>,
    sync: SyncInfo,
    models_dev_sync: SyncInfo,
}
#[derive(Clone)]
pub struct Snapshot {
    pub prices: BTreeMap<String, Price>,
    pub version: String,
}
impl Snapshot {
    pub fn find(&self, id: &str) -> Option<&Price> {
        self.prices
            .get(id)
            .or_else(|| self.prices.get(&canonical(id)))
    }
}
pub fn canonical(id: &str) -> String {
    // Exact lookup always wins. These are representation aliases, not fuzzy
    // substitutions between different models or dated model versions.
    id.rsplit('/')
        .next()
        .unwrap_or(id)
        .split(':')
        .next()
        .unwrap_or(id)
        .trim()
        .replace('@', "-")
        .trim_end_matches("[1m]")
        .trim()
        .to_ascii_lowercase()
}
struct Stored {
    file: File,
    revision: String,
    snapshot: Arc<Snapshot>,
    invalid: bool,
}
struct Inner {
    path: PathBuf,
    stored: Mutex<Stored>,
    events: broadcast::Sender<()>,
    synchronization: tokio::sync::Mutex<()>,
    discovery: tokio::sync::Mutex<Option<(u64, Vec<RemoteModel>)>>,
    client: reqwest::Client,
}
#[derive(Clone)]
pub struct Service(Arc<Inner>);
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct View {
    pub revision: String,
    pub settings: Settings,
    pub sync: SyncInfo,
    pub models_dev_sync: SyncInfo,
    pub prices: Vec<Price>,
    pub config_path: String,
}
#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "camelCase")]
pub enum Edit {
    Import { prices: Vec<Price> },
    Save { price: Box<Price> },
    Delete { id: String },
    Automatic { id: String },
    Settings { settings: Settings },
}
fn seed() -> File {
    let seed: Value = serde_json::from_str(include_str!("../../resources/pricing-seed.json"))
        .expect("validated bundled model prices");
    File {
        version: 1,
        settings: Settings::default(),
        remote: litellm(&seed["models"]),
        models_dev: BTreeMap::new(),
        overrides: BTreeMap::new(),
        disabled: BTreeSet::new(),
        sync: SyncInfo {
            hash: seed["sha256"].as_str().map(str::to_owned),
            updated_at: seed["downloadedAt"]
                .as_str()
                .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                .map(|d| d.timestamp().max(0) as u64),
            ..SyncInfo::default()
        },
        models_dev_sync: SyncInfo::default(),
    }
}
fn snapshot(file: &File) -> Arc<Snapshot> {
    let mut prices = file.remote.clone();
    if file.settings.models_dev_enabled {
        prices.extend(file.models_dev.clone());
    }
    prices.extend(file.overrides.clone());
    for id in &file.disabled {
        prices.remove(id);
    }
    let version = storage::digest(&serde_json::to_vec(&prices).unwrap());
    Arc::new(Snapshot { prices, version })
}
fn read(path: &Path) -> Result<Option<Vec<u8>>> {
    match std::fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(storage::io_error(e)),
        Ok(m) if !m.is_file() || m.file_type().is_symlink() || m.len() > MAX_BYTES as u64 => Err(
            AppError::new("PRICING", "定价文件必须是小于 32 MiB 的普通文件"),
        ),
        Ok(_) => std::fs::read(path).map(Some).map_err(storage::io_error),
    }
}
fn decode(bytes: &[u8]) -> Result<File> {
    let file: File =
        serde_json::from_slice(bytes).map_err(|_| AppError::new("PRICING", "定价文件格式无效"))?;
    if file.version != 1
        || file.remote.len() + file.models_dev.len() + file.overrides.len() > 50000
        || file
            .remote
            .values()
            .chain(file.models_dev.values())
            .chain(file.overrides.values())
            .any(|p| !p.validate())
    {
        return Err(AppError::new("PRICING", "定价文件版本或价格无效"));
    }
    Ok(file)
}
impl Service {
    pub fn new(data: &Path) -> Result<Self> {
        storage::private_dir(data)?;
        let path = data.join("model-pricing.json");
        let bytes = read(&path)?;
        let (mut file, invalid) = match bytes.as_deref().map(decode).transpose() {
            Ok(Some(file)) => (file, false),
            Ok(None) => (seed(), false),
            Err(_) => (seed(), true),
        };
        if invalid {
            file.sync.error = Some("本地定价文件无效，正在使用内置价格；修复后重新载入".into());
        }
        let revision = storage::revision(bytes.as_deref());
        let snapshot = snapshot(&file);
        let (events, _) = broadcast::channel(16);
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(20))
            .user_agent("gpt-Switch/0.6.0")
            .build()
            .map_err(|_| AppError::new("PRICING", "价格网络服务初始化失败"))?;
        let service = Self(Arc::new(Inner {
            path,
            stored: Mutex::new(Stored {
                file,
                revision,
                snapshot,
                invalid,
            }),
            events,
            synchronization: tokio::sync::Mutex::new(()),
            discovery: tokio::sync::Mutex::new(None),
            client,
        }));
        if bytes.is_none() {
            let mut s = service.0.stored.lock().unwrap();
            let file = s.file.clone();
            service.commit(&mut s, file)?;
        }
        Ok(service)
    }
    fn commit(&self, s: &mut Stored, file: File) -> Result<()> {
        if s.invalid {
            return Err(AppError::new("PRICING", "请先修复本地定价文件并重新载入"));
        }
        let bytes = serde_json::to_vec(&file)
            .map_err(|_| AppError::new("PRICING", "定价配置序列化失败"))?;
        // storage's version check uses a 2 MiB credential limit; catalogs have
        // a separate 32 MiB limit and are checked here before the atomic replace.
        if storage::revision(read(&self.0.path)?.as_deref()) != s.revision {
            return Err(AppError::new(
                "CONFLICT",
                "定价文件已在外部变化，请重新载入",
            ));
        }
        storage::atomic_write_bounded(&self.0.path, &bytes, Some(&s.revision), MAX_BYTES as u64)?;
        s.revision = storage::digest(&bytes);
        s.snapshot = snapshot(&file);
        s.file = file;
        let _ = self.0.events.send(());
        Ok(())
    }
    pub fn subscribe(&self) -> broadcast::Receiver<()> {
        self.0.events.subscribe()
    }
    pub fn snapshot(&self) -> Arc<Snapshot> {
        self.0.stored.lock().unwrap().snapshot.clone()
    }
    pub fn view(&self) -> View {
        let s = self.0.stored.lock().unwrap();
        View {
            revision: s.revision.clone(),
            settings: s.file.settings.clone(),
            sync: s.file.sync.clone(),
            models_dev_sync: s.file.models_dev_sync.clone(),
            prices: s.snapshot.prices.values().cloned().collect(),
            config_path: self.0.path.to_string_lossy().into_owned(),
        }
    }
    pub fn edit(&self, edit: Edit, expected: &str) -> Result<View> {
        {
            let mut s = self.0.stored.lock().unwrap();
            if s.revision != expected {
                return Err(AppError::new("CONFLICT", "价格已更新，请重新加载后保存"));
            }
            let mut f = s.file.clone();
            match edit {
                Edit::Import { prices } => {
                    if prices.is_empty()
                        || prices.len() > 10000
                        || prices.iter().any(|p| !p.validate())
                    {
                        return Err(AppError::new("PRICING", "批量导入价格无效"));
                    }
                    for mut p in prices {
                        p.fixed = true;
                        p.source = "manual".into();
                        f.disabled.remove(&p.model_id);
                        f.overrides.insert(p.model_id.clone(), p);
                    }
                }

                Edit::Save { price } => {
                    let mut price = *price;
                    price.model_id = price.model_id.trim().into();
                    if !price.validate() {
                        return Err(AppError::new("PRICING", "模型 ID 或非负价格无效"));
                    }
                    price.fixed = true;
                    price.source = "手动".into();
                    f.disabled.remove(&price.model_id);
                    f.overrides.insert(price.model_id.clone(), price);
                }
                Edit::Delete { id } => {
                    f.overrides.remove(&id);
                    f.disabled.insert(id);
                }
                Edit::Automatic { id } => {
                    f.overrides.remove(&id);
                    f.disabled.remove(&id);
                }
                Edit::Settings { settings } => {
                    if settings.selected.len() + settings.excluded.len() > 20000 {
                        return Err(AppError::new("PRICING", "选择的模型过多"));
                    }
                    f.settings = settings;
                }
            }
            self.commit(&mut s, f)?;
        }
        Ok(self.view())
    }
    pub fn reload(&self) -> Result<View> {
        let bytes =
            read(&self.0.path)?.ok_or_else(|| AppError::new("PRICING", "定价文件不存在"))?;
        let file = decode(&bytes)?;
        let mut s = self.0.stored.lock().unwrap();
        s.snapshot = snapshot(&file);
        s.file = file;
        s.revision = storage::digest(&bytes);
        s.invalid = false;
        drop(s);
        let _ = self.0.events.send(());
        Ok(self.view())
    }
    pub fn open_folder(&self) -> Result<()> {
        open::that(self.0.path.parent().unwrap()).map_err(storage::io_error)
    }
    async fn download(
        &self,
        url: &str,
        etag: Option<&str>,
    ) -> Result<Option<(Value, String, Option<String>)>> {
        let mut request = self.0.client.get(url);
        if let Some(etag) = etag {
            request = request.header(reqwest::header::IF_NONE_MATCH, etag);
        }
        let mut response = request
            .send()
            .await
            .map_err(|_| AppError::new("PRICING_NETWORK", "价格请求超时或网络失败"))?;
        if response.status() == reqwest::StatusCode::NOT_MODIFIED {
            return Ok(None);
        }
        if !response.status().is_success() {
            return Err(AppError::new(
                "PRICING_HTTP",
                &format!("价格服务返回 HTTP {}", response.status().as_u16()),
            ));
        }
        if response
            .content_length()
            .is_some_and(|n| n > MAX_BYTES as u64)
        {
            return Err(AppError::new("PRICING_SIZE", "价格响应超过 32 MiB"));
        }
        let etag = response
            .headers()
            .get(reqwest::header::ETAG)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        let mut bytes = vec![];
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| AppError::new("PRICING_NETWORK", "价格响应接收失败"))?
        {
            if bytes.len() + chunk.len() > MAX_BYTES {
                return Err(AppError::new("PRICING_SIZE", "价格响应超过 32 MiB"));
            }
            bytes.extend_from_slice(&chunk);
        }
        let value = serde_json::from_slice(&bytes)
            .map_err(|_| AppError::new("PRICING_JSON", "价格服务响应不是有效 JSON"))?;
        Ok(Some((value, storage::digest(&bytes), etag)))
    }
    pub async fn discover(&self, force: bool) -> Result<Vec<RemoteModel>> {
        let mut cache = self.0.discovery.lock().await;
        if let Some((at, models)) = &*cache {
            if !force && now().saturating_sub(*at) < 600 {
                return Ok(models.clone());
            }
        }
        let Some((value, _, _)) = self.download(MODELS_DEV, None).await? else {
            unreachable!()
        };
        let models = models_dev(&value);
        if models.is_empty() {
            return Err(AppError::new(
                "PRICING_JSON",
                "models.dev 没有可识别的模型价格",
            ));
        }
        *cache = Some((now(), models.clone()));
        Ok(models)
    }
    pub async fn import(&self, keys: BTreeSet<String>, expected: &str) -> Result<View> {
        let models = self.discover(false).await?;
        {
            let mut s = self.0.stored.lock().unwrap();
            if s.revision != expected {
                return Err(AppError::new("CONFLICT", "价格已更新，请重新加载"));
            }
            let mut f = s.file.clone();
            let mut seen = BTreeSet::new();
            for m in models.into_iter().filter(|m| keys.contains(&m.key)) {
                let mut price = m.price;
                price.model_id = canonical(&price.model_id);
                if !seen.insert(price.model_id.clone()) {
                    continue;
                }
                price.fixed = true;
                f.disabled.remove(&price.model_id);
                f.overrides.insert(price.model_id.clone(), price);
            }
            if seen.is_empty() {
                return Err(AppError::new("PRICING", "请选择可用模型"));
            }
            self.commit(&mut s, f)?;
        }
        Ok(self.view())
    }
    pub async fn sync(&self, force: bool) -> Result<View> {
        let _guard = match self.0.synchronization.try_lock() {
            Ok(guard) => guard,
            Err(_) => {
                let _guard = self.0.synchronization.lock().await;
                return Ok(self.view());
            }
        };
        let (settings, info, models_info, invalid) = {
            let s = self.0.stored.lock().unwrap();
            (
                s.file.settings.clone(),
                s.file.sync.clone(),
                s.file.models_dev_sync.clone(),
                s.invalid,
            )
        };
        if invalid {
            return Err(AppError::new("PRICING", "请修复本地定价文件并重新载入"));
        }
        let due = |at: Option<u64>| force || at.is_none_or(|at| now().saturating_sub(at) >= 600);
        if !force
            && (!settings.auto_update || !due(info.checked_at))
            && (!settings.models_dev_enabled || !due(models_info.checked_at))
        {
            return Ok(self.view());
        }
        if (settings.auto_update || force) && due(info.checked_at) {
            let downloaded = self.download(LITELLM, info.etag.as_deref()).await;
            let mut s = self.0.stored.lock().unwrap();
            let mut f = s.file.clone();
            f.sync.checked_at = Some(now());
            match downloaded {
                Ok(Some((v, hash, etag))) => {
                    let models = litellm(&v);
                    if models.is_empty() {
                        f.sync.error = Some("价格源未返回可识别模型，保留已有价格".into());
                    } else {
                        if f.sync.hash.as_deref() != Some(&hash) {
                            f.remote = models;
                            f.sync.updated_at = Some(now());
                        }
                        f.sync.hash = Some(hash);
                        f.sync.etag = etag;
                        f.sync.error = None;
                    }
                }
                Ok(None) => f.sync.error = None,
                Err(e) => f.sync.error = Some(e.message),
            }
            self.commit(&mut s, f)?;
        }
        if settings.models_dev_enabled && due(models_info.checked_at) {
            let downloaded = self.discover(force).await;
            let mut s = self.0.stored.lock().unwrap();
            let mut f = s.file.clone();
            // Re-read preferences after the request: an in-flight refresh cannot
            // overwrite manual edits or an updated auto-sync selection.
            let cfg = &f.settings;
            f.models_dev_sync.checked_at = Some(now());
            match downloaded {
                Ok(models) => {
                    let mut prices = BTreeMap::new();
                    for m in models.into_iter().filter(|m| {
                        cfg.selected.contains(&m.key)
                            || (cfg.include_common && m.common && !cfg.excluded.contains(&m.key))
                    }) {
                        let mut p = m.price;
                        p.model_id = canonical(&p.model_id);
                        prices.entry(p.model_id.clone()).or_insert(p);
                    }
                    f.models_dev = prices;
                    f.models_dev_sync.updated_at = Some(now());
                    f.models_dev_sync.error = None;
                }
                Err(e) => f.models_dev_sync.error = Some(e.message),
            }
            self.commit(&mut s, f)?;
        }
        Ok(self.view())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn manual_prices_survive_reload_and_invalid_file_keeps_last_snapshot() {
        let temp = tempfile::tempdir().unwrap();
        let s = Service::new(temp.path()).unwrap();
        let price = Price {
            model_id: "fixture".into(),
            display_name: "Fixture".into(),
            source: "".into(),
            fixed: false,
            rates: model::Rates {
                input: Some("0".into()),
                output: Some("2".into()),
                ..Default::default()
            },
            variants: vec![],
        };
        let view = s
            .edit(
                Edit::Save {
                    price: Box::new(price),
                },
                &s.view().revision,
            )
            .unwrap();
        assert!(s.snapshot().find("fixture").unwrap().fixed);
        assert!(s
            .edit(
                Edit::Delete {
                    id: "fixture".into()
                },
                "old"
            )
            .is_err());
        std::fs::write(&s.0.path, b"invalid").unwrap();
        assert!(s.reload().is_err());
        assert!(s.snapshot().find("fixture").is_some());
        assert!(s
            .edit(
                Edit::Delete {
                    id: "fixture".into()
                },
                &view.revision
            )
            .is_err());
    }
}
