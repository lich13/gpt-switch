use crate::storage::{self, AppError, Result};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};
use toml_edit::{value, DocumentMut, Item, Table};
pub const PROVIDER: &str = "gpt_switch_gateway";
#[derive(Serialize, Deserialize)]
struct Journal {
    path: PathBuf,
    original: Option<String>,
    applied: String,
    profile: Option<String>,
}
fn parse(text: &str) -> Result<DocumentMut> {
    text.parse()
        .map_err(|_| AppError::new("TOML", "配置不是有效的 TOML，请先修复配置"))
}
fn selector(doc: &DocumentMut, profile: Option<&str>) -> Option<Item> {
    match profile {
        Some(p) => doc.get("profiles")?.get(p)?.get("model_provider"),
        None => doc.get("model_provider"),
    }
    .cloned()
}
fn put_selector(doc: &mut DocumentMut, profile: Option<&str>, item: Option<Item>) {
    let table: &mut dyn toml_edit::TableLike = match profile {
        Some(p) => doc["profiles"][p]
            .as_table_like_mut()
            .expect("existing profile"),
        None => doc.as_table_mut(),
    };
    if let Some(item) = item {
        table.insert("model_provider", item);
    } else {
        table.remove("model_provider");
    }
}
fn owned(doc: &DocumentMut, profile: Option<&str>) -> String {
    format!(
        "{:?}|{:?}",
        selector(doc, profile).map(|x| x.to_string()),
        doc.get("model_providers")
            .and_then(|t| t.get(PROVIDER))
            .map(ToString::to_string)
    )
}
fn render(doc: &DocumentMut, reference: &str) -> String {
    let raw = doc.to_string();
    if reference.contains("\r\n") {
        raw.replace("\r\n", "\n").replace('\n', "\r\n")
    } else {
        raw
    }
}
pub fn import(home: &Path) -> Result<(String, String)> {
    let bytes = storage::read_optional(&home.join("config.toml"))?.unwrap_or_default();
    let text = String::from_utf8(bytes).map_err(|_| AppError::new("TOML", "配置不是 UTF-8"))?;
    let doc = parse(&text)?;
    let profile = doc.get("profile").and_then(Item::as_str);
    let id = selector(&doc, profile)
        .or_else(|| selector(&doc, None))
        .and_then(|i| i.as_str().map(str::to_owned))
        .unwrap_or_else(|| "openai".into());
    if id == PROVIDER {
        return Err(AppError::new(
            "MANAGED",
            "当前配置由本网关接管，请先停用网关再导入原供应商",
        ));
    }
    let p = doc.get("model_providers").and_then(|t| t.get(&id));
    let read = |k| {
        p.and_then(|p| p.get(k))
            .and_then(Item::as_str)
            .map(str::to_owned)
    };
    let base =
        read("base_url").ok_or_else(|| AppError::new("IMPORT", "当前 provider 没有 base_url"))?;
    let token = read("experimental_bearer_token")
        .filter(|s| !s.is_empty())
        .ok_or_else(|| AppError::new("IMPORT", "当前 provider 没有 experimental_bearer_token"))?;
    if token.starts_with("gs_") && base.contains("127.0.0.1") {
        return Err(AppError::new("MANAGED", "不能导入网关的本地凭据"));
    }
    Ok((base, token))
}
pub fn attach(data: &Path, home: &Path, port: u16, token: &str) -> Result<()> {
    let journal = data.join("gateway-recovery.json");
    if journal.exists() {
        return Err(AppError::new(
            "RECOVERY",
            "有尚未恢复的配置接管记录，请先停止接管并处理冲突",
        ));
    }
    let path = home.join("config.toml");
    let raw = storage::read_optional(&path)?;
    let original = raw
        .as_ref()
        .map(|b| String::from_utf8(b.clone()))
        .transpose()
        .map_err(|_| AppError::new("TOML", "配置不是 UTF-8"))?;
    let text = original.as_deref().unwrap_or("");
    let mut doc = parse(text)?;
    if doc
        .get("model_providers")
        .and_then(|p| p.get(PROVIDER))
        .is_some()
    {
        return Err(AppError::new(
            "MANAGED",
            "配置中已存在同名受管 provider，请先处理原记录",
        ));
    }
    let profile = doc
        .get("profile")
        .and_then(Item::as_str)
        .filter(|p| {
            doc.get("profiles")
                .and_then(|t| t.get(*p))
                .and_then(|t| t.get("model_provider"))
                .is_some()
        })
        .map(str::to_owned);
    let old_id = selector(&doc, profile.as_deref())
        .or_else(|| selector(&doc, None))
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_else(|| "openai".into());
    let mut provider = doc
        .get("model_providers")
        .and_then(|t| t.get(&old_id))
        .and_then(|item| item.clone().into_table().ok())
        .unwrap_or_default();
    for key in [
        "env_key",
        "env_key_instructions",
        "auth",
        "experimental_bearer_token",
    ] {
        provider.remove(key);
    }
    for key in ["http_headers", "env_http_headers"] {
        if let Some(headers) = provider.get_mut(key).and_then(Item::as_table_like_mut) {
            let keys: Vec<_> = headers
                .iter()
                .filter(|(k, _)| {
                    [
                        "authorization",
                        "proxy-authorization",
                        "api-key",
                        "x-api-key",
                    ]
                    .contains(&k.to_ascii_lowercase().as_str())
                })
                .map(|(k, _)| k.to_owned())
                .collect();
            for key in keys {
                headers.remove(&key);
            }
        }
    }
    provider.insert("name", value("gpt-Switch"));
    provider.insert("base_url", value(format!("http://127.0.0.1:{port}/v1")));
    provider.insert("experimental_bearer_token", value(token));
    provider.insert("requires_openai_auth", value(false));
    if !provider.contains_key("wire_api") {
        provider.insert("wire_api", value("responses"));
    }
    if doc.get("model_providers").is_none() {
        let mut parent = Table::new();
        parent.set_implicit(true);
        doc.insert("model_providers", Item::Table(parent));
    }
    doc["model_providers"][PROVIDER] = if doc["model_providers"].is_inline_table() {
        value(provider.into_inline_table())
    } else {
        Item::Table(provider)
    };
    put_selector(&mut doc, profile.as_deref(), Some(value(PROVIDER)));
    let applied = render(&doc, text);
    let record = Journal {
        path: path.clone(),
        original,
        applied: applied.clone(),
        profile,
    };
    storage::atomic_write(
        &journal,
        &serde_json::to_vec(&record).map_err(|_| AppError::new("RECOVERY", "无法生成恢复记录"))?,
        Some("missing"),
    )?;
    // The journal is durable before the config write. A crash at either step is recoverable.
    storage::atomic_write(
        &path,
        applied.as_bytes(),
        Some(&storage::revision(raw.as_deref())),
    )
}
pub fn detach(data: &Path) -> Result<()> {
    let journal = data.join("gateway-recovery.json");
    let Some(raw) = storage::read_optional(&journal)? else {
        return Ok(());
    };
    let record: Journal = serde_json::from_slice(&raw)
        .map_err(|_| AppError::new("RECOVERY", "恢复记录损坏，请保留该记录并手动检查配置"))?;
    let current = storage::read_optional(&record.path)?;
    let text = current
        .as_deref()
        .map(std::str::from_utf8)
        .transpose()
        .map_err(|_| AppError::new("CONFLICT", "配置编码已变化，请手动检查接管记录"))?;
    if text == record.original.as_deref() {
        fs::remove_file(journal).map_err(storage::io_error)?;
        return Ok(());
    }
    let output = if text == Some(&record.applied) {
        record.original.clone()
    } else {
        let mut doc = parse(text.unwrap_or_default())?;
        let applied = parse(&record.applied)?;
        if owned(&doc, record.profile.as_deref()) != owned(&applied, record.profile.as_deref()) {
            return Err(AppError::new("CONFLICT", "受管 provider 或选择器已被外部修改，已保留配置与恢复记录；恢复受管字段后可再次停止接管"));
        }
        let original = parse(record.original.as_deref().unwrap_or_default())?;
        put_selector(
            &mut doc,
            record.profile.as_deref(),
            selector(&original, record.profile.as_deref()),
        );
        if let Some(providers) = doc
            .get_mut("model_providers")
            .and_then(Item::as_table_like_mut)
        {
            providers.remove(PROVIDER);
        }
        if original.get("model_providers").is_none()
            && doc
                .get("model_providers")
                .and_then(Item::as_table)
                .is_some_and(Table::is_empty)
        {
            doc.remove("model_providers");
        }
        Some(render(&doc, text.unwrap_or_default()))
    };
    if let Some(output) = output {
        storage::atomic_write(
            &record.path,
            output.as_bytes(),
            Some(&storage::revision(current.as_deref())),
        )?;
    } else {
        if storage::revision(storage::read_optional(&record.path)?.as_deref())
            != storage::revision(current.as_deref())
        {
            return Err(AppError::new("CONFLICT", "配置在恢复时发生变化"));
        }
        fs::remove_file(&record.path).map_err(storage::io_error)?;
    }
    fs::remove_file(journal).map_err(storage::io_error)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn takeover_roundtrip_and_external_edits() {
        let t = tempfile::tempdir().unwrap();
        let data = t.path().join("data");
        let home = t.path().join("home");
        storage::private_dir(&data).unwrap();
        storage::private_dir(&home).unwrap();
        let original = "# keep\r\nmodel = 'custom-model'\r\nmodel_provider = 'custom'\r\n[model_providers.custom]\r\nbase_url = 'https://example.test/sub/v1'\r\nexperimental_bearer_token = 'fixture'\r\nsupports_websockets = true\r\n";
        let path = home.join("config.toml");
        fs::write(&path, original).unwrap();
        fs::write(home.join("auth.json"), "auth-unchanged").unwrap();
        attach(&data, &home, 15722, "local-test").unwrap();
        let doc = parse(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(
            doc["model_providers"][PROVIDER]["supports_websockets"].as_bool(),
            Some(true)
        );
        assert!(import(&home).is_err());
        detach(&data).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), original);
        attach(&data, &home, 15722, "local-test").unwrap();
        let live = fs::read_to_string(&path)
            .unwrap()
            .replace("custom-model", "external-model");
        fs::write(&path, live).unwrap();
        detach(&data).unwrap();
        let result = fs::read_to_string(&path).unwrap();
        assert!(result.contains("external-model"));
        assert!(!result.contains(PROVIDER));
        assert!(result.contains("# keep\r\n"));
        assert_eq!(
            fs::read_to_string(home.join("auth.json")).unwrap(),
            "auth-unchanged"
        );
    }
    #[test]
    fn inline_tables_and_profile_selector_roundtrip() {
        let t = tempfile::tempdir().unwrap();
        let path = t.path().join("config.toml");
        let original = "# inline example\nprofile='work'\nprofiles={work={model_provider='original',model='keep'}}\nmodel_providers={original={base_url='https://example.test/v1',experimental_bearer_token='fixture',supports_websockets=true}}\n";
        fs::write(&path, original).unwrap();
        attach(t.path(), t.path(), 15722, "local-test").unwrap();
        let doc = parse(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(
            doc["profiles"]["work"]["model_provider"].as_str(),
            Some(PROVIDER)
        );
        assert_eq!(
            doc["model_providers"][PROVIDER]["supports_websockets"].as_bool(),
            Some(true)
        );
        detach(t.path()).unwrap();
        assert_eq!(fs::read_to_string(path).unwrap(), original);
    }
    #[test]
    fn conflict_and_crash_before_write() {
        let t = tempfile::tempdir().unwrap();
        attach(t.path(), t.path(), 15722, "local-test").unwrap();
        let path = t.path().join("config.toml");
        let applied = fs::read_to_string(&path).unwrap();
        fs::write(&path, applied.replace("local-test", "edited")).unwrap();
        assert!(detach(t.path()).is_err());
        fs::write(&path, applied).unwrap();
        detach(t.path()).unwrap();
        assert!(!path.exists());
    }
}
