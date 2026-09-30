mod cleanup;
mod commands;
mod configuration;
mod core;
mod gateway;
mod links;
mod login;
mod power;
#[cfg(target_os = "macos")]
mod power_macos;
mod process_control;
mod quick;
mod startup;
#[cfg(target_os = "macos")]
mod startup_macos;
mod storage;
#[cfg(target_os = "macos")]
mod tray_macos;
use core::{Core, Preferences, ViewState};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};
use storage::{AppError, Result};
use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Emitter, Manager,
};
use tauri_plugin_dialog::DialogExt;

struct Runtime {
    core: Mutex<Core>,
    data: PathBuf,
    imports: Mutex<links::Imports>,
    gateway: gateway::Gateway,
    claude: gateway::Gateway,
    login: Mutex<login::Session>,
    quitting: AtomicBool,
    quit_pending: AtomicBool,
    smoke: Option<PathBuf>,
    smoke_result: Mutex<Option<Result<SmokeSnapshot>>>,
    fixture: Mutex<Option<tempfile::TempDir>>,
    startup: startup::Service,
    start_silently: bool,
    frontend_started: AtomicBool,
    force_quitting: AtomicBool,
    power: power::Service,
    startup_error: Mutex<Option<AppError>>,
    cleanup_error: Mutex<Option<AppError>>,
}
impl Runtime {
    fn gateway(&self, client: gateway::ClientId) -> &gateway::Gateway {
        match client {
            gateway::ClientId::Codex => &self.gateway,
            gateway::ClientId::Claude => &self.claude,
        }
    }
    fn home(&self, client: gateway::ClientId) -> Result<PathBuf> {
        let core = lock(&self.core)?;
        Ok(match client {
            gateway::ClientId::Codex => core.home(),
            gateway::ClientId::Claude => PathBuf::from(core.preferences().claude_home),
        })
    }
    async fn stop_gateways(&self) -> Result<()> {
        let a = self.gateway.stop_for_exit().await;
        let b = self.claude.stop_for_exit().await;
        a.and(b).map(|_| ())
    }
}
struct SmokeSnapshot {
    claude_home: PathBuf,
    claude_config: Option<Vec<u8>>,
    home: PathBuf,
    auth: Option<Vec<u8>>,
    config: Option<Vec<u8>>,
}

#[cfg(target_os = "macos")]
fn application_menu(app: &tauri::AppHandle) -> tauri::Result<Menu<tauri::Wry>> {
    // The native predefined Quit calls NSApplication.terminate directly.
    // Use our command so configuration restoration can finish (or report a conflict).
    let menu = Menu::default(app)?;
    menu.remove_at(0)?;
    let application = tauri::menu::Submenu::with_items(
        app,
        "lich13-switch",
        true,
        &[
            &PredefinedMenuItem::about(app, Some("关于 lich13-switch"), None)?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::services(app, None)?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::hide(app, None)?,
            &PredefinedMenuItem::hide_others(app, None)?,
            &PredefinedMenuItem::separator(app)?,
            &MenuItem::with_id(
                app,
                "app-quit",
                "退出 lich13-switch",
                true,
                Some("CmdOrCtrl+Q"),
            )?,
        ],
    )?;
    menu.insert(&application, 0)?;
    Ok(menu)
}
fn login_active(s: &login::LoginState) -> bool {
    ["starting", "waiting", "cancelling"].contains(&s.phase.as_str())
}
fn lock<T>(m: &Mutex<T>) -> Result<std::sync::MutexGuard<'_, T>> {
    m.lock()
        .map_err(|_| AppError::new("STATE", "应用状态异常，请重新启动 lich13-switch"))
}
fn show(app: &tauri::AppHandle, page: Option<&str>) -> Result<()> {
    #[cfg(target_os = "macos")]
    app.set_activation_policy(tauri::ActivationPolicy::Regular)
        .map_err(|_| AppError::new("WINDOW", "无法显示主窗口"))?;
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        w.show()
            .map_err(|_| AppError::new("WINDOW", "无法显示主窗口"))?;
        let _ = w.set_focus();
        let _ = w.emit("app-visibility", true);
        if let Some(page) = page {
            let _ = w.emit("navigate", page);
        }
    }
    Ok(())
}
fn tray_menu(app: &tauri::AppHandle) -> tauri::Result<Menu<tauri::Wry>> {
    let menu = Menu::new(app)?;
    for (id, label) in [
        ("open", "打开 lich13-switch"),
        ("config", "编辑配置"),
        ("gateway", "打开网关"),
        ("settings", "设置"),
        ("quit", "退出 lich13-switch"),
    ] {
        if id == "quit" {
            menu.append(&PredefinedMenuItem::separator(app)?)?;
        }
        menu.append(&MenuItem::with_id(app, id, label, true, None::<&str>)?)?;
    }
    Ok(menu)
}
#[tauri::command]
fn open_main(
    app: tauri::AppHandle,
    page: Option<String>,
    provider_id: Option<String>,
    client_id: Option<gateway::ClientId>,
) -> Result<()> {
    if page
        .as_ref()
        .is_some_and(|p| !["accounts", "config", "gateway", "settings"].contains(&p.as_str()))
    {
        return Err(AppError::new("WINDOW", "无效页面"));
    }
    quick::hide_quick(app.clone())?;
    if let Some(id) = provider_id {
        show(&app, None)?;
        if let Some(w) = app.get_webview_window("main") {
            let _ = w.emit(
                "provider-settings",
                serde_json::json!({"id":id,"clientId":client_id.unwrap_or_default()}),
            );
        }
        Ok(())
    } else {
        show(&app, page.as_deref())
    }
}
fn publish(app: &tauri::AppHandle, state: ViewState) {
    if let Some(tray) = app.tray_by_id("switch") {
        let current = state
            .accounts
            .iter()
            .find(|a| a.current)
            .map(|a| a.name.as_str())
            .unwrap_or("未保存账号");
        let _ = tray.set_tooltip(Some(format!("lich13-switch · {current}")));
    }
    let _ = app.emit("switch-state", &state);
}
fn refresh(app: &tauri::AppHandle, r: &Runtime) -> Result<ViewState> {
    let state = lock(&r.core)?.state()?;
    r.gateway.observe_home(&lock(&r.core)?.home());
    publish(app, state.clone());
    Ok(state)
}
#[tauri::command]
async fn list_provider_models(
    client_id: gateway::ClientId,
    r: tauri::State<'_, Arc<Runtime>>,
    provider_id: String,
    force: bool,
) -> Result<gateway::catalog::View> {
    r.gateway(client_id).list_models(&provider_id, force).await
}
#[tauri::command]
fn get_state(r: tauri::State<'_, Arc<Runtime>>) -> Result<ViewState> {
    lock(&r.core)?.state()
}
#[tauri::command]
fn switch_account(
    app: tauri::AppHandle,
    r: tauri::State<'_, Arc<Runtime>>,
    id: String,
    expected_revision: String,
) -> Result<ViewState> {
    let result = lock(&r.core)?.switch_account(&id, &expected_revision);
    match &result {
        Ok(s) => publish(&app, s.clone()),
        Err(_) => {
            let _ = refresh(&app, &r);
        }
    }
    result
}
#[tauri::command]
fn import_current(app: tauri::AppHandle, r: tauri::State<'_, Arc<Runtime>>) -> Result<ViewState> {
    let mut c = lock(&r.core)?;
    let path = c.home().join("auth.json");
    c.import_file(&path, None)?;
    drop(c);
    refresh(&app, &r)
}
#[tauri::command]
async fn import_auth_file(
    app: tauri::AppHandle,
    r: tauri::State<'_, Arc<Runtime>>,
) -> Result<Option<ViewState>> {
    let handle = app.clone();
    let path = tauri::async_runtime::spawn_blocking(move || {
        handle
            .dialog()
            .file()
            .add_filter("Codex auth JSON", &["json"])
            .blocking_pick_file()
    })
    .await
    .map_err(|_| AppError::new("DIALOG", "无法打开文件选择器"))?;
    if let Some(path) = path {
        let path = path
            .into_path()
            .map_err(|_| AppError::new("PATH", "请选择本地文件"))?;
        lock(&r.core)?.import_file(&path, None)?;
        Ok(Some(refresh(&app, &r)?))
    } else {
        Ok(None)
    }
}
#[tauri::command]
fn add_api_key(
    app: tauri::AppHandle,
    r: tauri::State<'_, Arc<Runtime>>,
    name: String,
    key: String,
) -> Result<ViewState> {
    lock(&r.core)?.add_api_key(&name, &key)?;
    refresh(&app, &r)
}
#[tauri::command]
fn rename_account(
    app: tauri::AppHandle,
    r: tauri::State<'_, Arc<Runtime>>,
    id: String,
    name: String,
) -> Result<ViewState> {
    lock(&r.core)?.rename(&id, &name)?;
    refresh(&app, &r)
}
#[tauri::command]
fn delete_account(
    app: tauri::AppHandle,
    r: tauri::State<'_, Arc<Runtime>>,
    id: String,
) -> Result<ViewState> {
    lock(&r.core)?.delete(&id)?;
    refresh(&app, &r)
}
#[tauri::command]
fn read_config(
    client_id: Option<gateway::ClientId>,
    r: tauri::State<'_, Arc<Runtime>>,
) -> Result<configuration::Document> {
    let client = client_id.unwrap_or_default();
    r.gateway(client).read_config(&r.home(client)?)
}
#[tauri::command]
fn read_previous_config(
    client_id: gateway::ClientId,
    r: tauri::State<'_, Arc<Runtime>>,
) -> Result<String> {
    r.gateway(client_id).previous_config()
}
#[tauri::command]
fn validate_config(client_id: Option<gateway::ClientId>, text: String) -> Result<()> {
    configuration::validate(client_id.unwrap_or_default(), &text)
}
#[tauri::command]
fn save_config(
    app: tauri::AppHandle,
    r: tauri::State<'_, Arc<Runtime>>,
    text: String,
    expected_revision: String,
    client_id: Option<gateway::ClientId>,
) -> Result<configuration::Document> {
    let client = client_id.unwrap_or_default();
    let doc = r
        .gateway(client)
        .save_config(&r.home(client)?, &text, &expected_revision)?;
    let _ = app.emit(
        "config-state",
        serde_json::json!({"clientId":client,"revision":doc.revision,"guarded":doc.guarded}),
    );
    refresh(&app, &r)?;
    Ok(doc)
}
#[tauri::command]
fn set_preferences(
    app: tauri::AppHandle,
    r: tauri::State<'_, Arc<Runtime>>,
    preferences: Preferences,
) -> Result<ViewState> {
    if login_active(&lock(&r.login)?.state) {
        return Err(AppError::new("LOGIN_BUSY", "请先完成或取消登录"));
    }
    if r.gateway.guarded_home() && lock(&r.core)?.preferences().codex_home != preferences.codex_home
    {
        return Err(AppError::new(
            "GATEWAY_ACTIVE",
            "更换 Codex 目录前请先停用网关并恢复配置",
        ));
    }
    if r.claude.guarded_home()
        && lock(&r.core)?.preferences().claude_home != preferences.claude_home
    {
        return Err(AppError::new(
            "GATEWAY",
            "请先停止 Claude Code 网关并处理恢复事务",
        ));
    }
    let s = lock(&r.core)?.set_preferences(preferences)?;
    r.gateway.observe_home(&r.home(gateway::ClientId::Codex)?);
    r.claude.observe_home(&r.home(gateway::ClientId::Claude)?);
    publish(&app, s.clone());
    Ok(s)
}
#[tauri::command]
async fn get_startup(r: tauri::State<'_, Arc<Runtime>>) -> Result<startup::View> {
    let r = r.inner().clone();
    tauri::async_runtime::spawn_blocking(move || r.startup.view())
        .await
        .map_err(|_| AppError::new("STARTUP", "读取启动设置失败"))?
}
#[tauri::command]
async fn set_startup(
    r: tauri::State<'_, Arc<Runtime>>,
    enabled: bool,
    preferences: startup::Preferences,
    expected_revision: String,
) -> Result<startup::View> {
    let r = r.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        r.startup.update(enabled, preferences, &expected_revision)
    })
    .await
    .map_err(|_| AppError::new("STARTUP", "更新启动设置失败"))?
}
#[tauri::command]
async fn pick_path(app: tauri::AppHandle, kind: String) -> Result<Option<String>> {
    tauri::async_runtime::spawn_blocking(move || {
        let p = if kind == "directory" {
            app.dialog().file().blocking_pick_folder()
        } else {
            app.dialog().file().blocking_pick_file()
        };
        p.map(|p| {
            p.into_path()
                .map(|p| p.to_string_lossy().into())
                .map_err(|_| AppError::new("PATH", "请选择本地路径"))
        })
        .transpose()
    })
    .await
    .map_err(|_| AppError::new("DIALOG", "无法选择路径"))?
}
#[tauri::command]
fn get_login(r: tauri::State<'_, Arc<Runtime>>) -> Result<login::LoginState> {
    Ok(lock(&r.login)?.state.clone())
}
#[tauri::command]
fn start_login(
    app: tauri::AppHandle,
    r: tauri::State<'_, Arc<Runtime>>,
    mode: String,
) -> Result<login::LoginState> {
    if !["browser", "device"].contains(&mode.as_str()) {
        return Err(AppError::new("LOGIN_MODE", "登录方式无效"));
    }
    let prefs = lock(&r.core)?.preferences();
    let cli = login::resolve_cli(&prefs.cli_path)?;
    let mut session = lock(&r.login)?;
    if login_active(&session.state) {
        return Err(AppError::new("LOGIN_BUSY", "已有登录正在进行"));
    }
    let (tx, rx) = tokio::sync::oneshot::channel();
    session.cancel = Some(tx);
    session.state = login::LoginState {
        phase: "starting".into(),
        mode: mode.clone(),
        message: "正在启动官方登录…".into(),
        ..Default::default()
    };
    let initial = session.state.clone();
    drop(session);
    let runtime = r.inner().clone();
    tauri::async_runtime::spawn(async move {
        let (sender, mut events) = tokio::sync::mpsc::channel(8);
        let task = login::run(&cli, &mode, rx, sender);
        tokio::pin!(task);
        let result = loop {
            tokio::select! {Some(state)=events.recv()=>{if let Ok(mut s)=runtime.login.lock(){s.state=state.clone();}let _=app.emit("login-state",state);},r=&mut task=>break r}
        };
        let final_state = match result {
            Ok(Some(raw)) => match lock(&runtime.core).and_then(|mut c| c.import_raw(&raw, None)) {
                Ok(_) => login::LoginState {
                    phase: "success".into(),
                    mode: mode.clone(),
                    message: "账号已添加，选择后即可切换".into(),
                    ..Default::default()
                },
                Err(e) => login::LoginState {
                    phase: "error".into(),
                    mode: mode.clone(),
                    message: e.message,
                    ..Default::default()
                },
            },
            Ok(None) => login::LoginState {
                phase: "cancelled".into(),
                mode: mode.clone(),
                message: "登录已取消".into(),
                ..Default::default()
            },
            Err(e) => login::LoginState {
                phase: "error".into(),
                mode: mode.clone(),
                message: e.message,
                ..Default::default()
            },
        };
        if let Ok(mut s) = runtime.login.lock() {
            s.state = final_state.clone();
            s.cancel = None;
        }
        let _ = app.emit("login-state", final_state);
        let _ = refresh(&app, &runtime);
        if runtime.quitting.load(Ordering::Relaxed) {
            app.exit(0);
        }
    });
    Ok(initial)
}
#[tauri::command]
fn cancel_login(r: tauri::State<'_, Arc<Runtime>>) -> Result<()> {
    let mut s = lock(&r.login)?;
    if let Some(tx) = s.cancel.take() {
        let _ = tx.send(());
        s.state.phase = "cancelling".into();
    }
    Ok(())
}
#[tauri::command]
fn open_login_url(r: tauri::State<'_, Arc<Runtime>>) -> Result<()> {
    let s = lock(&r.login)?;
    let url = s
        .state
        .url
        .as_ref()
        .ok_or_else(|| AppError::new("LOGIN_URL", "登录链接尚未生成"))?;
    open::that(url).map_err(|_| AppError::new("OPEN", "无法打开浏览器"))
}
fn quit(app: &tauri::AppHandle, r: &Arc<Runtime>) {
    if r.quit_pending.swap(true, Ordering::Relaxed) {
        return;
    }
    let runtime = r.clone();
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(e) = runtime.stop_gateways().await {
            if runtime.smoke.is_some() {
                if let Ok(mut result) = runtime.smoke_result.lock() {
                    *result = Some(Err(e));
                }
                runtime.quitting.store(true, Ordering::Relaxed);
                app.exit(1);
                return;
            }
            runtime.quit_pending.store(false, Ordering::Relaxed);
            let _ = app.emit("switch-error", e);
            let _ = show(&app, Some("gateway"));
            return;
        }
        runtime.quitting.store(true, Ordering::Relaxed);
        if let Ok(mut s) = runtime.login.lock() {
            if let Some(tx) = s.cancel.take() {
                let _ = tx.send(());
                return;
            }
            if login_active(&s.state) {
                return;
            }
        }
        app.exit(0);
    });
}
#[tauri::command]
fn get_gateway(client_id: gateway::ClientId, r: tauri::State<'_, Arc<Runtime>>) -> gateway::View {
    if let Ok(home) = r.home(client_id) {
        r.gateway(client_id).observe_home(&home);
    }
    r.gateway(client_id).view()
}
#[tauri::command]
fn update_gateway(
    client_id: gateway::ClientId,
    app: tauri::AppHandle,
    r: tauri::State<'_, Arc<Runtime>>,
    edit: gateway::Edit,
    expected_revision: String,
    expected_config_revision: Option<String>,
) -> Result<gateway::View> {
    let home = r.home(client_id)?;
    let selected = matches!(&edit, gateway::Edit::Select { .. });
    let result = r.gateway(client_id).edit_checked(
        edit,
        &expected_revision,
        &home,
        expected_config_revision.as_deref(),
    )?;
    let _ = refresh(&app, &r);
    if selected {
        let _ = app.emit(
            "switch-notice",
            if result.running {
                "已切换供应商，新请求立即生效".to_string()
            } else {
                format!("文件已切换，请重新打开 {}", client_id.name())
            },
        );
    }
    Ok(result)
}
#[tauri::command]
async fn start_gateway(
    client_id: gateway::ClientId,
    app: tauri::AppHandle,
    r: tauri::State<'_, Arc<Runtime>>,
    expected_revision: String,
    expected_config_revision: Option<String>,
) -> Result<gateway::View> {
    let home = r.home(client_id)?;
    let result = r
        .gateway(client_id)
        .start_checked(
            &expected_revision,
            &home,
            expected_config_revision.as_deref(),
        )
        .await?;
    let _ = refresh(&app, &r);
    let _ = app.emit(
        "switch-notice",
        format!("网关已启用，请重新打开 {}", client_id.name()),
    );
    Ok(result)
}
#[tauri::command]
async fn stop_gateway(
    client_id: gateway::ClientId,
    app: tauri::AppHandle,
    r: tauri::State<'_, Arc<Runtime>>,
    expected_config_revision: Option<String>,
) -> Result<gateway::View> {
    let result = r
        .gateway(client_id)
        .stop_checked(expected_config_revision.as_deref())
        .await?;
    let _ = refresh(&app, &r);
    let _ = app.emit(
        "switch-notice",
        format!("已写入当前供应商，请重新打开 {}", client_id.name()),
    );
    Ok(result)
}
#[tauri::command]
async fn query_provider_quota(
    client_id: gateway::ClientId,
    r: tauri::State<'_, Arc<Runtime>>,
    provider_id: String,
    force: bool,
) -> Result<gateway::QuotaView> {
    r.gateway(client_id).query_quota(&provider_id, force).await
}
#[tauri::command]
async fn frontend_ready(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    r: tauri::State<'_, Arc<Runtime>>,
) -> Result<()> {
    if window.label() != "main" {
        return Ok(());
    }
    if r.frontend_started.swap(true, Ordering::AcqRel) {
        return Ok(());
    }
    if r.smoke.is_some() {
        let result = (|| -> Result<()> {
            #[cfg(windows)]
            startup::smoke_registration()?;
            let mut c = lock(&r.core)?;
            let a = c.add_api_key("Smoke A", "fixture-only-a")?;
            let b = c.add_api_key("Smoke B", "fixture-only-b")?;
            let text = "# smoke\nmodel = 'fixture'\nmodel_provider = 'custom'\n[model_providers.custom]\nbase_url = \"https://example.invalid/v1\"\nexperimental_bearer_token = \"fixture-only\"\nwire_api = 'responses'\n[future]\nkeep = true\n";
            c.save_config(text, "missing")?;
            let cfg = c.state()?.config_revision;
            for id in [&a, &b, &a] {
                let rev = c.state()?.auth_revision;
                c.switch_account(id, &rev)?;
                if c.state()?.config_revision != cfg {
                    return Err(AppError::new("SMOKE", "配置发生意外变化"));
                }
            }
            Ok(())
        })();
        let result = match result {
            Ok(()) => gateway_smoke(&r).await,
            Err(e) => Err(e),
        };
        let result = match result {
            Ok(()) => prepare_exit_smoke(&r).await,
            Err(e) => Err(e),
        };
        *lock(&r.smoke_result)? = Some(result);
        // Exercise a coded exit request while the gateway is still active.
        // The report is written only after the exit handler verifies restoration.
        app.exit(0);
    } else {
        if r.startup.preferences()?.restore_gateway {
            for client in [gateway::ClientId::Codex, gateway::ClientId::Claude] {
                if let Err(e) = r.gateway(client).resume(&r.home(client)?).await {
                    let e = AppError::new(&e.code, &format!("{}：{}", client.name(), e.message));
                    *r.startup_error.lock().unwrap() = Some(e.clone());
                    let _ = app.emit("switch-error", e);
                }
            }
        }
        if !r.start_silently {
            show(&app, None)?;
        }
    }
    Ok(())
}
async fn prepare_exit_smoke(r: &Runtime) -> Result<SmokeSnapshot> {
    let home = lock(&r.core)?.home();
    let claude_home = r.home(gateway::ClientId::Claude)?;
    let snapshot = SmokeSnapshot {
        claude_config: storage::read_optional(&claude_home.join("settings.json"))?,
        claude_home,
        auth: storage::read_optional(&home.join("auth.json"))?,
        config: storage::read_optional(&home.join("config.toml"))?.map(|raw| {
            String::from_utf8(raw)
                .unwrap()
                .replace("https://backup.invalid/v1", "https://example.invalid/v1")
                .replace("fixture-backup", "fixture-only")
                .into_bytes()
        }),
        home,
    };
    r.gateway
        .start(&r.gateway.view().revision, &snapshot.home)
        .await?;
    r.gateway.edit(
        gateway::Edit::Select {
            id: r.gateway.view().providers[0].id.clone(),
        },
        &r.gateway.view().revision,
        &snapshot.home,
    )?;
    r.claude
        .start(&r.claude.view().revision, &snapshot.claude_home)
        .await?;
    Ok(snapshot)
}
fn report_exit_smoke(app: &tauri::AppHandle, r: &Runtime, restored: Result<()>) -> Result<()> {
    let result = (|| -> Result<()> {
        let snapshot = lock(&r.smoke_result)?
            .take()
            .ok_or_else(|| AppError::new("SMOKE", "退出验收未初始化"))??;
        restored?;
        if !r.quit_pending.load(Ordering::Relaxed)
            || r.gateway.guarded_home()
            || r.claude.guarded_home()
            || snapshot.claude_config
                != storage::read_optional(&snapshot.claude_home.join("settings.json"))?
            || snapshot.auth != storage::read_optional(&snapshot.home.join("auth.json"))?
            || snapshot.config != storage::read_optional(&snapshot.home.join("config.toml"))?
        {
            return Err(AppError::new("SMOKE", "正常退出未恢复配置"));
        }
        Ok(())
    })();
    let data = serde_json::json!({"safeExit":result.is_ok(),"gateway":result.is_ok(),"ok":result.is_ok(),"version":env!("CARGO_PKG_VERSION"),"revision":env!("GPT_SWITCH_REVISION"),"webview":true,"tray":app.tray_by_id("switch").is_some(),"platform":std::env::consts::OS,"error":result.err()});
    storage::atomic_write(
        r.smoke.as_ref().unwrap(),
        serde_json::to_string_pretty(&data).unwrap().as_bytes(),
        None,
    )?;
    if let Some(fixture) = lock(&r.fixture)?.take() {
        fixture.close().map_err(storage::io_error)?;
    }
    Ok(())
}
async fn gateway_smoke(r: &Runtime) -> Result<()> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let home = lock(&r.core)?.home();
    let auth = storage::read_optional(&home.join("auth.json"))?;
    let config = storage::read_optional(&home.join("config.toml"))?;
    let listener = std::net::TcpListener::bind("127.0.0.1:0").map_err(storage::io_error)?;
    let port = listener.local_addr().map_err(storage::io_error)?.port();
    drop(listener);
    r.gateway.edit(
        gateway::Edit::Settings {
            settings: gateway::Settings {
                port,
                ..Default::default()
            },
        },
        &r.gateway.view().revision,
        &home,
    )?;
    r.gateway.edit(
        gateway::Edit::SaveProvider {
            id: None,
            base_url: "https://example.invalid/v1".into(),
            token: "fixture-only".into(),
        },
        &r.gateway.view().revision,
        &home,
    )?;
    r.gateway.edit(
        gateway::Edit::SaveProvider {
            id: None,
            base_url: "https://backup.invalid/v1".into(),
            token: "fixture-backup".into(),
        },
        &r.gateway.view().revision,
        &home,
    )?;
    let claude_home = r.home(gateway::ClientId::Claude)?;
    storage::atomic_write(&claude_home.join("settings.json"), br#"{"env":{"ANTHROPIC_BASE_URL":"https://claude.example.invalid","ANTHROPIC_AUTH_TOKEN":"fixture-claude"},"model":"keep"}"#, Some("missing"))?;
    r.claude.import_initial(&claude_home)?;
    let listener = std::net::TcpListener::bind("127.0.0.1:0").map_err(storage::io_error)?;
    let claude_port = listener.local_addr().map_err(storage::io_error)?.port();
    drop(listener);
    r.claude.edit(
        gateway::Edit::Settings {
            settings: gateway::Settings {
                port: claude_port,
                ..Default::default()
            },
        },
        &r.claude.view().revision,
        &claude_home,
    )?;
    r.gateway.start(&r.gateway.view().revision, &home).await?;
    r.claude
        .start(&r.claude.view().revision, &claude_home)
        .await?;
    let live = storage::read_optional(&home.join("config.toml"))?;
    r.gateway.edit(
        gateway::Edit::Select {
            id: r.gateway.view().providers[1].id.clone(),
        },
        &r.gateway.view().revision,
        &home,
    )?;
    if live != storage::read_optional(&home.join("config.toml"))? {
        return Err(AppError::new("SMOKE", "运行中换商修改了配置"));
    }
    let config = config.map(|raw| {
        String::from_utf8(raw)
            .unwrap()
            .replace("https://example.invalid/v1", "https://backup.invalid/v1")
            .replace("fixture-only", "fixture-backup")
            .into_bytes()
    });
    let mut connection = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .map_err(storage::io_error)?;
    connection
        .write_all(b"GET /v1/models HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .await
        .map_err(storage::io_error)?;
    let mut response = Vec::new();
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        connection.read_to_end(&mut response),
    )
    .await
    .map_err(|_| AppError::new("SMOKE", "网关响应超时"))?
    .map_err(storage::io_error)?;
    r.gateway.stop().await?;
    if !r.claude.view().running {
        return Err(AppError::new("SMOKE", "双网关状态未隔离"));
    }
    r.claude.stop().await?;
    if !response.starts_with(b"HTTP/1.1 401")
        || auth != storage::read_optional(&home.join("auth.json"))?
        || config != storage::read_optional(&home.join("config.toml"))?
    {
        return Err(AppError::new("SMOKE", "网关或恢复校验失败"));
    }
    Ok(())
}
#[cfg(windows)]
fn tray_is_dark() -> bool {
    use winreg::{enums::HKEY_CURRENT_USER, RegKey};
    RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize")
        .and_then(|key| key.get_value::<u32, _>("SystemUsesLightTheme"))
        .unwrap_or(0)
        == 0
}
pub fn run() {
    let args: Vec<_> = std::env::args().collect();
    let smoke = args
        .iter()
        .position(|s| s == "--smoke-test")
        .and_then(|i| args.get(i + 1))
        .map(PathBuf::from);
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, args, _| {
            links::receive(app, args.iter().cloned());
            if !args.iter().any(|a| startup::is_login_argument(a)) {
                let _ = show(app, None);
            }
        }))
        .plugin(tauri_plugin_dialog::init())
        .setup(move |app| {
            let fixture = if smoke.is_some() {
                Some(
                    tempfile::Builder::new()
                        .prefix("lich13-switch-smoke-")
                        .tempdir()?,
                )
            } else {
                None
            };
            let data = fixture
                .as_ref()
                .map(|t| t.path().join("data"))
                .unwrap_or(app.path().app_data_dir()?);
            let home = fixture
                .as_ref()
                .map(|t| t.path().join("codex"))
                .unwrap_or_else(|| {
                    std::env::var_os("CODEX_HOME")
                        .map(PathBuf::from)
                        .unwrap_or_else(|| dirs::home_dir().unwrap_or_default().join(".codex"))
                });
            let cleanup_error = cleanup::retired_statistics(&data).err();
            let gateway = gateway::Gateway::new(data.clone())?;
            let claude = gateway.companion(data.join("claude"))?;
            let startup = startup::Service::new(&data);
            let start_silently = smoke.is_none() && startup::silent(&args, &startup.preferences()?);
            app.manage(quick::Panel::new(&data)?);
            let power = power::Service::new(&data);
            let mut core = Core::new(data.clone(), home)?;
            if let Some(fixture) = &fixture {
                storage::private_dir(&core.home())?;
                storage::private_dir(&fixture.path().join("claude"))?;
                let mut preferences = core.preferences();
                preferences.claude_home =
                    fixture.path().join("claude").to_string_lossy().into_owned();
                core.set_preferences(preferences)?;
            }
            let claude_import_error = if smoke.is_none() {
                claude
                    .import_initial(std::path::Path::new(&core.preferences().claude_home))
                    .err()
            } else {
                None
            };
            let runtime = Arc::new(Runtime {
                core: Mutex::new(core),
                data,
                imports: Mutex::new(links::Imports::default()),
                gateway,
                claude,
                login: Mutex::new(Default::default()),
                quitting: AtomicBool::new(false),
                quit_pending: AtomicBool::new(false),
                smoke,
                smoke_result: Mutex::new(None),
                fixture: Mutex::new(fixture),
                startup,
                start_silently,
                power,
                frontend_started: AtomicBool::new(false),
                force_quitting: AtomicBool::new(false),
                startup_error: Mutex::new(claude_import_error),
                cleanup_error: Mutex::new(cleanup_error),
            });
            runtime.gateway.observe_home(&lock(&runtime.core)?.home());
            runtime
                .claude
                .observe_home(&runtime.home(gateway::ClientId::Claude)?);
            let state = lock(&runtime.core)?.state()?;
            app.manage(runtime.clone());
            links::receive(app.handle(), args.iter().cloned());
            quick::Panel::create(app.handle())?;
            #[cfg(target_os = "macos")]
            if start_silently {
                app.set_activation_policy(tauri::ActivationPolicy::Accessory);
            }
            #[cfg(target_os = "macos")]
            app.set_menu(application_menu(app.handle())?)?;
            if runtime.smoke.is_none() {
                let r = runtime.clone();
                tauri::async_runtime::spawn_blocking(move || {
                    if let Err(e) = r.startup.migrate() {
                        *r.startup_error.lock().unwrap() = Some(e);
                    }
                });
            }
            for g in [runtime.gateway.clone(), runtime.claude.clone()] {
                let mut quota_events = g.quota_events();
                let quota_app = app.handle().clone();
                let client_id = g.client_id();
                tauri::async_runtime::spawn(async move {
                    loop {
                        match quota_events.recv().await {
                            Ok(view) => {
                                let _ = quota_app.emit(
                                    "provider-quota",
                                    serde_json::json!({"clientId":client_id,"quota":view}),
                                );
                            }
                            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                            Err(_) => break,
                        }
                    }
                });
                let mut events = g.subscribe();
                let handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    while let Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) =
                        events.recv().await
                    {
                        let _ = handle.emit("gateway-state", g.view());
                    }
                });
            }
            let mut tray = TrayIconBuilder::with_id("switch")
                .menu(&tray_menu(app.handle())?)
                .show_menu_on_left_click(false)
                .tooltip("lich13-switch");
            #[cfg(target_os = "macos")]
            {
                tray = tray
                    .icon(tauri::image::Image::from_bytes(include_bytes!(
                        "../icons/tray.png"
                    ))?)
                    .icon_as_template(true);
            }
            #[cfg(not(target_os = "macos"))]
            {
                let bytes = if cfg!(windows) {
                    #[cfg(windows)]
                    {
                        if tray_is_dark() {
                            include_bytes!("../icons/tray-white.png").as_slice()
                        } else {
                            include_bytes!("../icons/tray-black.png").as_slice()
                        }
                    }
                    #[cfg(not(windows))]
                    {
                        include_bytes!("../icons/tray-white.png").as_slice()
                    }
                } else {
                    include_bytes!("../icons/tray-white.png").as_slice()
                };
                tray = tray.icon(tauri::image::Image::from_bytes(bytes)?);
            }
            let tray = tray
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button,
                        button_state,
                        rect,
                        ..
                    } = event
                    {
                        if button == MouseButton::Left && button_state == MouseButtonState::Up {
                            let app = tray.app_handle().clone();
                            let handle = app.clone();
                            let _ = app.run_on_main_thread(move || {
                                if let Err(e) = quick::Panel::show(&handle, Some(rect)) {
                                    let _ = handle.emit("switch-error", e);
                                }
                            });
                        }
                        #[cfg(target_os = "macos")]
                        if button == MouseButton::Right && button_state == MouseButtonState::Down {
                            if let Err(e) = tray_macos::context_menu(tray) {
                                let _ = tray.app_handle().emit("switch-error", e);
                            }
                        }
                    }
                })
                .on_menu_event(|app, e| match e.id().as_ref() {
                    "quit" => quit(app, &app.state::<Arc<Runtime>>()),
                    "open" => {
                        let _ = open_main(app.clone(), None, None, None);
                    }
                    "config" | "gateway" | "settings" => {
                        let _ = open_main(app.clone(), Some(e.id().as_ref().into()), None, None);
                    }
                    _ => (),
                })
                .build(app)?;
            #[cfg(target_os = "macos")]
            tray_macos::install(&tray)?;
            #[cfg(not(target_os = "macos"))]
            let _ = tray;
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let mut last = Some(state);
                let mut config_versions = std::collections::HashMap::new();
                loop {
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                    if runtime.quitting.load(Ordering::Relaxed) {
                        break;
                    }
                    let result = lock(&runtime.core).and_then(|mut c| c.state());
                    match result {
                        Ok(s) => {
                            if last.as_ref() != Some(&s) {
                                runtime.gateway.observe_home(
                                    &lock(&runtime.core).map(|c| c.home()).unwrap_or_default(),
                                );
                                let _ = handle.emit("gateway-state", runtime.gateway.view());
                                last = Some(s.clone());
                                publish(&handle, s);
                            }
                        }
                        Err(e) => {
                            if let Some(mut s) = last.clone() {
                                if s.error.as_ref() != Some(&e.message) {
                                    s.error = Some(e.message);
                                    last = Some(s.clone());
                                    publish(&handle, s);
                                }
                            }
                        }
                    }
                    for client in [gateway::ClientId::Codex, gateway::ClientId::Claude] {
                        if let Ok(doc) = runtime.home(client).and_then(|home| runtime.gateway(client).read_config(&home)) {
                            let stamp = (doc.path.clone(), doc.revision.clone(), doc.guarded);
                            if config_versions.get(&client) != Some(&stamp) {
                                config_versions.insert(client, stamp);
                                let _ = handle.emit("config-state", serde_json::json!({"clientId":client,"revision":doc.revision,"guarded":doc.guarded}));
                                if client == gateway::ClientId::Claude {
                                    let _ = handle.emit("gateway-state", runtime.claude.view());
                                }
                            }
                        }
                    }
                    #[cfg(windows)]
                    if let Some(tray) = handle.tray_by_id("switch") {
                        let bytes = if tray_is_dark() {
                            include_bytes!("../icons/tray-white.png").as_slice()
                        } else {
                            include_bytes!("../icons/tray-black.png").as_slice()
                        };
                        if let Ok(image) = tauri::image::Image::from_bytes(bytes) {
                            let _ = tray.set_icon(Some(image));
                        }
                    }
                }
            });
            Ok(())
        })
        .on_menu_event(|app, event| {
            if event.id().as_ref() == "app-quit" {
                let r = app.state::<Arc<Runtime>>();
                quit(app, &r);
            }
        })
        .on_window_event(|w, event| {
            if w.label() == "quick" {
                match event {
                    tauri::WindowEvent::Focused(focused) => {
                        quick::Panel::focus(w.app_handle(), *focused)
                    }
                    tauri::WindowEvent::CloseRequested { api, .. } => {
                        api.prevent_close();
                        let _ = quick::hide_quick(w.app_handle().clone());
                    }
                    _ => (),
                }
                return;
            }
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let r = w.state::<Arc<Runtime>>();
                if !r.quitting.load(Ordering::Relaxed) {
                    api.prevent_close();
                    let _ = w.hide();
                    let _ = w.emit("app-visibility", false);
                    #[cfg(target_os = "macos")]
                    let _ = w
                        .app_handle()
                        .set_activation_policy(tauri::ActivationPolicy::Accessory);
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            links::get_link_handler_state,
            links::set_link_handler,
            links::get_provider_imports,
            links::confirm_provider_import,
            links::cancel_provider_import,
            commands::cleanup_retired_data,
            commands::get_clamshell_state,
            commands::set_clamshell_awake,
            commands::install_power_helper,
            commands::remove_power_helper,
            commands::force_quit_codex_clients,
            commands::get_startup_error,
            open_main,
            quick::get_quick,
            quick::set_quick,
            quick::hide_quick,
            quick::resize_quick,
            get_gateway,
            update_gateway,
            start_gateway,
            stop_gateway,
            query_provider_quota,
            list_provider_models,
            get_state,
            switch_account,
            import_current,
            import_auth_file,
            add_api_key,
            rename_account,
            delete_account,
            read_config,
            read_previous_config,
            validate_config,
            save_config,
            set_preferences,
            get_startup,
            set_startup,
            pick_path,
            get_login,
            start_login,
            cancel_login,
            open_login_url,
            frontend_ready
        ])
        .build(tauri::generate_context!())
        .expect("lich13-switch initialization failed")
        .run(|app, event| {
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Opened { urls } = &event {
                links::receive(app, urls.iter().map(|url| url.as_str().to_owned()));
            }
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Reopen { .. } = &event {
                if !startup::macos_login_event() {
                    let _ = show(app, None);
                }
            }
            if let tauri::RunEvent::ExitRequested { api, .. } = &event {
                let r = app.state::<Arc<Runtime>>();
                if !r.quitting.load(Ordering::Relaxed) {
                    api.prevent_exit();
                    quit(app, &r);
                }
            }
            if let tauri::RunEvent::Exit = event {
                let r = app.state::<Arc<Runtime>>();
                // macOS Dock/system termination can bypass ExitRequested.
                // Serialize restoration with any in-flight takeover before returning to the OS.
                let restored = tauri::async_runtime::block_on(r.stop_gateways());
                if r.smoke.is_some() {
                    let _ = report_exit_smoke(app, &r, restored);
                }
            }
        });
}
