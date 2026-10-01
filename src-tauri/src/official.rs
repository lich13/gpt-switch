//! Codex-only official ChatGPT account mode.
//!
//! The mode owns only three standalone assignments in the existing
//! `model_providers.custom` table.  It comments those exact lines while the
//! official account is active and restores the original bytes when disabled.

use crate::storage::{self, AppError, Result};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};
use toml_edit::Document;

const FILE: &str = "official-mode.json";
const KEYS: [&str; 3] = [
    "base_url",
    "experimental_bearer_token",
    "supports_websockets",
];

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OfficialModeView {
    pub enabled: bool,
    pub state: String,
    pub account_id: Option<String>,
    pub error: Option<String>,
}

impl Default for OfficialModeView {
    fn default() -> Self {
        Self {
            enabled: false,
            state: "disabled".into(),
            account_id: None,
            error: None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct FieldRecord {
    key: String,
    original: Option<String>,
    managed: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct Record {
    version: u32,
    enabled: bool,
    path: PathBuf,
    before_revision: String,
    managed_revision: Option<String>,
    fields: Vec<FieldRecord>,
    account_id: String,
}

#[derive(Clone)]
struct Line {
    start: usize,
    end: usize,
    body: String,
    raw: String,
}

struct Scan {
    lines: Vec<Line>,
    fields: Vec<Option<usize>>,
    comments: Vec<Option<usize>>,
}

fn lines(text: &str) -> Vec<Line> {
    let mut out = Vec::new();
    let mut start = 0;
    for (offset, ch) in text.char_indices() {
        if ch == '\n' {
            let end = offset + 1;
            let raw = text[start..end].to_owned();
            let body = raw
                .strip_suffix('\n')
                .unwrap_or(&raw)
                .strip_suffix('\r')
                .unwrap_or_else(|| raw.strip_suffix('\n').unwrap_or(&raw))
                .to_owned();
            out.push(Line {
                start,
                end,
                body,
                raw,
            });
            start = end;
        }
    }
    if start < text.len() {
        let raw = text[start..].to_owned();
        out.push(Line {
            start,
            end: text.len(),
            body: raw.clone(),
            raw,
        });
    }
    out
}

fn active_custom(text: &str) -> Result<()> {
    let doc = text
        .parse::<Document<String>>()
        .map_err(|_| AppError::new("TOML", "配置不是有效的 TOML，请先修复配置"))?;
    let profile = doc.get("profile").and_then(|v| v.as_str());
    let selector = profile
        .and_then(|p| doc.get("profiles")?.get(p)?.get("model_provider"))
        .or_else(|| doc.get("model_provider"))
        .and_then(|v| v.as_str());
    if selector != Some("custom") {
        return Err(AppError::new(
            "CUSTOM",
            "当前生效的 provider 不是 custom，请在配置编辑器中处理",
        ));
    }
    let has_custom = doc
        .get("model_providers")
        .and_then(|v| v.get("custom"))
        .is_some_and(|v| v.as_table_like().is_some());
    if !has_custom {
        return Err(AppError::new(
            "CUSTOM",
            "缺少现有 model_providers.custom，请在配置编辑器中处理",
        ));
    }
    Ok(())
}

fn assignment(body: &str, key: &str) -> bool {
    let trimmed = body.trim_start();
    if trimmed.starts_with('#') || !trimmed.starts_with(key) {
        return false;
    }
    let rest = &trimmed[key.len()..];
    rest.chars()
        .next()
        .is_some_and(|c| c.is_whitespace() || c == '=')
        && rest.trim_start().starts_with('=')
}

fn comment_assignment(body: &str, key: &str) -> bool {
    let trimmed = body.trim_start();
    let Some(comment) = trimmed.strip_prefix('#') else {
        return false;
    };
    assignment(comment.trim_start(), key)
}

fn scan(text: &str) -> Result<Scan> {
    active_custom(text)?;
    let all = lines(text);
    let mut start = None;
    let mut end = all.len();
    for (i, line) in all.iter().enumerate() {
        let t = line.body.trim();
        if start.is_some()
            && (t.starts_with("[[")
                || (t.starts_with('[') && !t.starts_with("[model_providers.custom]")))
        {
            end = i;
            break;
        }
        if t.starts_with("[model_providers.custom]") {
            if start.is_some() {
                return Err(AppError::new(
                    "OFFICIAL_MODE",
                    "custom 表重复，无法安全接管",
                ));
            }
            start = Some(i);
        }
    }
    let start =
        start.ok_or_else(|| AppError::new("OFFICIAL_MODE", "custom 不是独立表，无法安全接管"))?;
    let mut fields = vec![None; KEYS.len()];
    let mut comments = vec![None; KEYS.len()];
    for (i, line) in all.iter().enumerate().take(end).skip(start + 1) {
        for (n, key) in KEYS.iter().enumerate() {
            if assignment(&line.body, key) {
                if fields[n].is_some() {
                    return Err(AppError::new("OFFICIAL_MODE", "受管字段重复，无法安全接管"));
                }
                fields[n] = Some(i);
            } else if comment_assignment(&line.body, key) {
                if comments[n].is_some() {
                    return Err(AppError::new("OFFICIAL_MODE", "受管注释重复，无法安全接管"));
                }
                comments[n] = Some(i);
            }
        }
    }
    if fields
        .iter()
        .zip(&comments)
        .any(|(a, b)| a.is_some() && b.is_some())
    {
        return Err(AppError::new(
            "OFFICIAL_MODE",
            "受管字段同时存在赋值和注释，无法安全接管",
        ));
    }
    Ok(Scan {
        lines: all,
        fields,
        comments,
    })
}

fn comment_line(raw: &str) -> String {
    let newline = if raw.ends_with("\r\n") {
        "\r\n"
    } else if raw.ends_with('\n') {
        "\n"
    } else {
        ""
    };
    let body = raw.strip_suffix(newline).unwrap_or(raw);
    let indent = body.len() - body.trim_start().len();
    format!("{}# {}{}", &body[..indent], &body[indent..], newline)
}

fn replace_lines(text: &str, scan: &Scan, replacements: &[(usize, String)]) -> String {
    let mut edits = replacements.to_vec();
    edits.sort_by_key(|(i, _)| std::cmp::Reverse(*i));
    let mut out = text.to_owned();
    for (i, value) in edits {
        out.replace_range(scan.lines[i].start..scan.lines[i].end, &value);
    }
    out
}

fn record_path(data: &Path) -> PathBuf {
    data.join(FILE)
}

fn load(data: &Path) -> Result<Option<Record>> {
    let Some(raw) = storage::read_optional(&record_path(data))? else {
        return Ok(None);
    };
    let record: Record = serde_json::from_slice(&raw)
        .map_err(|_| AppError::new("OFFICIAL_MODE", "官方账号事务记录损坏，请保留现场"))?;
    if record.version != 1
        || record.fields.len() != KEYS.len()
        || record.path.as_os_str().is_empty()
    {
        return Err(AppError::new(
            "OFFICIAL_MODE",
            "官方账号事务记录格式无效，请保留现场",
        ));
    }
    Ok(Some(record))
}

fn save(data: &Path, record: &Record) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(record)
        .map_err(|_| AppError::new("OFFICIAL_MODE", "无法保存官方账号事务记录"))?;
    let prior = storage::read_optional(&record_path(data))?;
    storage::atomic_write(
        &record_path(data),
        &bytes,
        Some(&storage::revision(prior.as_deref())),
    )
}

fn verify_record(text: &str, record: &Record) -> Result<Scan> {
    let scan = scan(text)?;
    for (n, field) in record.fields.iter().enumerate() {
        match (
            &field.original,
            &field.managed,
            scan.fields[n],
            scan.comments[n],
        ) {
            (Some(_), Some(expected), None, Some(i)) if scan.lines[i].raw == *expected => (),
            (None, None, None, None) => (),
            _ => {
                return Err(AppError::new(
                    "CONFLICT",
                    "官方账号受管字段已被外部修改，请先处理冲突",
                ))
            }
        }
    }
    Ok(scan)
}

pub fn view(data: &Path, home: &Path) -> OfficialModeView {
    let record = match load(data) {
        Ok(Some(record)) => record,
        Ok(None) => return OfficialModeView::default(),
        Err(e) => {
            return OfficialModeView {
                state: "unavailable".into(),
                error: Some(e.message),
                ..Default::default()
            }
        }
    };
    let raw = match storage::read_optional(&record.path) {
        Ok(Some(raw)) => raw,
        Ok(None) => {
            return OfficialModeView {
                state: "conflict".into(),
                account_id: Some(record.account_id),
                error: Some("Codex 配置不存在".into()),
                ..Default::default()
            }
        }
        Err(e) => {
            return OfficialModeView {
                state: "unavailable".into(),
                account_id: Some(record.account_id),
                error: Some(e.message),
                ..Default::default()
            }
        }
    };
    if record.path != home.join("config.toml") {
        return OfficialModeView {
            state: "conflict".into(),
            account_id: Some(record.account_id),
            error: Some("Codex 配置目录已变化".into()),
            ..Default::default()
        };
    }
    let text = match String::from_utf8(raw) {
        Ok(v) => v,
        Err(_) => {
            return OfficialModeView {
                state: "conflict".into(),
                account_id: Some(record.account_id),
                error: Some("Codex 配置不是 UTF-8".into()),
                ..Default::default()
            }
        }
    };
    if record.enabled
        && record.managed_revision.as_deref() == Some(&storage::revision(Some(text.as_bytes())))
        && verify_record(&text, &record).is_ok()
    {
        OfficialModeView {
            enabled: true,
            state: "enabled".into(),
            account_id: Some(record.account_id),
            error: None,
        }
    } else {
        OfficialModeView {
            state: "conflict".into(),
            account_id: Some(record.account_id),
            error: Some("官方账号配置已被外部修改".into()),
            ..Default::default()
        }
    }
}

pub fn is_enabled(data: &Path, home: &Path) -> bool {
    view(data, home).enabled
}

pub fn blocks_gateway(data: &Path, home: &Path) -> bool {
    view(data, home).state != "disabled"
}

pub fn enable(
    data: &Path,
    home: &Path,
    account_id: &str,
    expected: &str,
) -> Result<OfficialModeView> {
    if load(data)?.is_some() {
        return Err(AppError::new(
            "OFFICIAL_MODE",
            "已有官方账号事务，请先处理现有状态",
        ));
    }
    let path = home.join("config.toml");
    let prior = storage::read_optional(&path)?;
    if storage::revision(prior.as_deref()) != expected {
        return Err(AppError::new("CONFLICT", "Codex 配置已变化，请刷新后重试"));
    }
    let raw = prior.ok_or_else(|| {
        AppError::new("CUSTOM", "缺少 config.toml，请先在配置编辑器中设置 custom")
    })?;
    let text =
        String::from_utf8(raw).map_err(|_| AppError::new("CONFIG", "Codex 配置不是 UTF-8"))?;
    let scan = scan(&text)?;
    if scan.fields.iter().all(Option::is_none) {
        return Err(AppError::new(
            "OFFICIAL_MODE",
            "三个受管字段均已注释或缺失，无法确认原始配置",
        ));
    }
    if scan.comments.iter().any(Option::is_some) {
        return Err(AppError::new(
            "OFFICIAL_MODE",
            "受管字段已存在注释，无法安全接管",
        ));
    }
    let mut fields = Vec::with_capacity(KEYS.len());
    let mut replacements = Vec::new();
    for (n, key) in KEYS.iter().enumerate() {
        let original = scan.fields[n].map(|i| scan.lines[i].raw.clone());
        let managed = original.as_deref().map(comment_line);
        if let Some(i) = scan.fields[n] {
            replacements.push((i, managed.clone().unwrap_or_default()));
        }
        fields.push(FieldRecord {
            key: (*key).into(),
            original,
            managed,
        });
    }
    let output = replace_lines(&text, &scan, &replacements);
    let managed_revision = storage::revision(Some(output.as_bytes()));
    let mut record = Record {
        version: 1,
        enabled: false,
        path: path.clone(),
        before_revision: expected.into(),
        managed_revision: None,
        fields,
        account_id: account_id.into(),
    };
    save(data, &record)?;
    storage::atomic_write(&path, output.as_bytes(), Some(expected))?;
    record.enabled = true;
    record.managed_revision = Some(managed_revision);
    save(data, &record)?;
    if view(data, home).enabled {
        Ok(view(data, home))
    } else {
        Err(AppError::new(
            "VERIFY",
            "官方账号配置写入后校验失败，请处理现场",
        ))
    }
}

pub fn disable(data: &Path, home: &Path, expected: &str) -> Result<OfficialModeView> {
    let record = load(data)?.ok_or_else(|| AppError::new("OFFICIAL_MODE", "官方账号模式未启用"))?;
    let path = home.join("config.toml");
    if record.path != path {
        return Err(AppError::new(
            "CONFLICT",
            "Codex 配置目录已变化，请处理官方模式冲突",
        ));
    }
    let raw = storage::read_optional(&path)?
        .ok_or_else(|| AppError::new("CONFLICT", "Codex 配置不存在，无法恢复"))?;
    let text =
        String::from_utf8(raw).map_err(|_| AppError::new("CONFIG", "Codex 配置不是 UTF-8"))?;
    if expected != storage::revision(Some(text.as_bytes())) { /* unrelated edits are allowed; managed lines are checked below */
    }
    let scan = verify_record(&text, &record)?;
    let mut replacements = Vec::new();
    for (n, field) in record.fields.iter().enumerate() {
        if let (Some(i), Some(original)) = (scan.comments[n], &field.original) {
            replacements.push((i, original.clone()));
        }
    }
    let output = replace_lines(&text, &scan, &replacements);
    storage::atomic_write(
        &path,
        output.as_bytes(),
        Some(&storage::revision(Some(text.as_bytes()))),
    )?;
    fs::remove_file(record_path(data)).map_err(storage::io_error)?;
    Ok(OfficialModeView::default())
}

pub fn guard_save(data: &Path, home: &Path, text: &str) -> Result<()> {
    let Some(record) = load(data)? else {
        return Ok(());
    };
    if !view(data, home).enabled {
        return Err(AppError::new(
            "OFFICIAL_MODE",
            "官方账号事务存在冲突，请先关闭官方连接",
        ));
    }
    verify_record(text, &record).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    const TEXT: &str = "model_provider='custom'\r\n[model_providers.custom]\r\nbase_url = 'https://x.test' # keep\r\nexperimental_bearer_token = 'secret'\r\nsupports_websockets = true\r\nwire_api='responses'\r\n";
    #[test]
    fn comments_and_restores_exact_lines() {
        let t = tempfile::tempdir().unwrap();
        let home = t.path().join("codex");
        let data = t.path().join("data");
        fs::create_dir_all(&home).unwrap();
        storage::private_dir(&data).unwrap();
        fs::write(home.join("config.toml"), TEXT).unwrap();
        let before = storage::revision(Some(TEXT.as_bytes()));
        let enabled = enable(&data, &home, "acct", &before).unwrap();
        assert!(enabled.enabled);
        let managed = fs::read_to_string(home.join("config.toml")).unwrap();
        assert!(managed.contains("# base_url = 'https://x.test' # keep"));
        disable(&data, &home, &storage::revision(Some(managed.as_bytes()))).unwrap();
        assert_eq!(fs::read_to_string(home.join("config.toml")).unwrap(), TEXT);
    }
    #[test]
    fn refuses_inline_custom_and_external_managed_edits() {
        let t = tempfile::tempdir().unwrap();
        let home = t.path().join("codex");
        let data = t.path().join("data");
        fs::create_dir_all(&home).unwrap();
        storage::private_dir(&data).unwrap();
        let inline = "model_provider='custom'\nmodel_providers={custom={base_url='x',experimental_bearer_token='k'}}\n";
        fs::write(home.join("config.toml"), inline).unwrap();
        assert!(enable(
            &data,
            &home,
            "acct",
            &storage::revision(Some(inline.as_bytes()))
        )
        .is_err());
    }
}
