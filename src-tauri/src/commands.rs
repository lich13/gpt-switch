use crate::{
    power, pricing, process_control,
    storage::{AppError, Result},
    usage, Runtime,
};
use std::{
    collections::BTreeSet,
    sync::{atomic::Ordering, Arc},
};
use tauri::Emitter;
type R<'a> = tauri::State<'a, Arc<Runtime>>;
#[tauri::command]
pub fn get_usage_state(r: R<'_>) -> usage::State {
    r.gateway.usage().state()
}
#[tauri::command]
pub fn set_usage_settings(
    r: R<'_>,
    settings: usage::Settings,
    expected_revision: String,
) -> Result<usage::State> {
    r.gateway.usage().configure(settings, &expected_revision)
}
#[tauri::command]
pub async fn get_usage_dashboard(r: R<'_>, filters: usage::Filters) -> Result<usage::Dashboard> {
    let svc = r.gateway.usage();
    tauri::async_runtime::spawn_blocking(move || svc.dashboard(filters))
        .await
        .map_err(|_| AppError::new("USAGE", "统计查询失败"))?
}
#[tauri::command]
pub async fn get_usage_logs(
    r: R<'_>,
    filters: usage::Filters,
    page: u32,
) -> Result<usage::LogPage> {
    let svc = r.gateway.usage();
    tauri::async_runtime::spawn_blocking(move || svc.logs(filters, page))
        .await
        .map_err(|_| AppError::new("USAGE", "日志查询失败"))?
}
#[tauri::command]
pub async fn get_usage_detail(r: R<'_>, id: String) -> Result<Option<usage::LogicalDetail>> {
    let svc = r.gateway.usage();
    tauri::async_runtime::spawn_blocking(move || svc.detail(&id))
        .await
        .map_err(|_| AppError::new("USAGE", "详情查询失败"))?
}
#[tauri::command]
pub fn get_pricing(r: R<'_>) -> pricing::View {
    r.gateway.usage().prices().view()
}
#[tauri::command]
pub async fn update_pricing(
    r: R<'_>,
    edit: pricing::Edit,
    expected_revision: String,
) -> Result<pricing::View> {
    let svc = r.gateway.usage();
    tauri::async_runtime::spawn_blocking(move || {
        let v = svc.prices().edit(edit, &expected_revision)?;
        svc.backfill();
        Ok(v)
    })
    .await
    .map_err(|_| AppError::new("PRICING", "定价更新失败"))?
}
#[tauri::command]
pub async fn sync_pricing(r: R<'_>) -> Result<pricing::View> {
    let svc = r.gateway.usage();
    let v = svc.prices().sync(true).await?;
    svc.backfill();
    Ok(v)
}
#[tauri::command]
pub async fn list_models_dev(r: R<'_>, force: bool) -> Result<Vec<pricing::RemoteModel>> {
    r.gateway.usage().prices().discover(force).await
}
#[tauri::command]
pub async fn import_models_dev(
    r: R<'_>,
    keys: BTreeSet<String>,
    expected_revision: String,
) -> Result<pricing::View> {
    let svc = r.gateway.usage();
    let v = svc.prices().import(keys, &expected_revision).await?;
    svc.backfill();
    Ok(v)
}
#[tauri::command]
pub fn reload_pricing(r: R<'_>) -> Result<pricing::View> {
    let svc = r.gateway.usage();
    let v = svc.prices().reload()?;
    svc.backfill();
    Ok(v)
}
#[tauri::command]
pub fn open_pricing_folder(r: R<'_>) -> Result<()> {
    r.gateway.usage().prices().open_folder()
}
#[tauri::command]
pub async fn get_clamshell_state(r: R<'_>) -> Result<power::State> {
    let r = r.inner().clone();
    tauri::async_runtime::spawn_blocking(move || r.power.state())
        .await
        .map_err(|_| AppError::new("POWER", "电源状态读取失败"))?
}
#[tauri::command]
pub async fn set_clamshell_awake(
    app: tauri::AppHandle,
    r: R<'_>,
    enabled: bool,
    expected_revision: String,
) -> Result<power::State> {
    let r = r.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let result = r.power.set(enabled, &expected_revision);
        if let Ok(state) = r.power.state() {
            let _ = app.emit("clamshell-state", state);
        }
        result
    })
    .await
    .map_err(|_| AppError::new("POWER", "电源设置失败"))?
}
#[tauri::command]
pub async fn force_quit_codex_clients(r: R<'_>) -> Result<process_control::Outcome> {
    if r.force_quitting.swap(true, Ordering::AcqRel) {
        return Err(AppError::new("BUSY", "正在退出客户端"));
    }
    let runtime = r.inner().clone();
    let result = tauri::async_runtime::spawn_blocking(process_control::force_quit)
        .await
        .map_err(|_| AppError::new("PROCESS", "退出客户端失败"));
    runtime.force_quitting.store(false, Ordering::Release);
    result?
}
#[tauri::command]
pub fn get_startup_error(r: R<'_>) -> Option<AppError> {
    r.startup_error.lock().unwrap().clone()
}
