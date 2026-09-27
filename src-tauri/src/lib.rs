mod core;
mod login;
mod storage;
use core::{ConfigDocument, Core, Preferences, ViewState};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};
use storage::{AppError, Result};
use tauri::{
    menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem},
    tray::TrayIconBuilder,
    Emitter, Manager,
};
use tauri_plugin_dialog::DialogExt;

struct Runtime {
    core: Mutex<Core>,
    login: Mutex<login::Session>,
    quitting: AtomicBool,
    smoke: Option<PathBuf>,
    fixture: Mutex<Option<tempfile::TempDir>>,
}
fn login_active(s: &login::LoginState) -> bool {
    ["starting", "waiting", "cancelling"].contains(&s.phase.as_str())
}
fn lock<T>(m: &Mutex<T>) -> Result<std::sync::MutexGuard<'_, T>> {
    m.lock()
        .map_err(|_| AppError::new("STATE", "应用状态异常，请重新启动 gpt-Switch"))
}
fn show(app: &tauri::AppHandle, page: &str) -> Result<()> {
    #[cfg(target_os = "macos")]
    app.set_activation_policy(tauri::ActivationPolicy::Regular)
        .map_err(|_| AppError::new("WINDOW", "无法显示主窗口"))?;
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        w.show()
            .map_err(|_| AppError::new("WINDOW", "无法显示主窗口"))?;
        let _ = w.set_focus();
        let _ = app.emit("navigate", page);
    }
    Ok(())
}
fn tray_menu(app: &tauri::AppHandle, state: &ViewState) -> tauri::Result<Menu<tauri::Wry>> {
    let menu = Menu::new(app)?;
    menu.append(&MenuItem::with_id(
        app,
        "heading",
        "gpt-Switch · 账号",
        false,
        None::<&str>,
    )?)?;
    if state.accounts.is_empty() {
        menu.append(&MenuItem::with_id(
            app,
            "empty",
            "暂无已保存账号",
            false,
            None::<&str>,
        )?)?;
    }
    for a in &state.accounts {
        menu.append(&CheckMenuItem::with_id(
            app,
            format!("account:{}", a.id),
            a.name.replace('&', "&&"),
            true,
            a.current,
            None::<&str>,
        )?)?;
    }
    menu.append(&PredefinedMenuItem::separator(app)?)?;
    if state.auth_source.warning.is_some() {
        menu.append(&MenuItem::with_id(
            app,
            "source",
            "当前配置另有认证设置",
            false,
            None::<&str>,
        )?)?;
    }
    for (id, label) in [
        ("open", "打开 gpt-Switch"),
        ("config", "编辑配置"),
        ("quit", "退出 gpt-Switch"),
    ] {
        menu.append(&MenuItem::with_id(app, id, label, true, None::<&str>)?)?;
    }
    Ok(menu)
}
fn publish(app: &tauri::AppHandle, state: ViewState) {
    if let Some(tray) = app.tray_by_id("switch") {
        if let Ok(menu) = tray_menu(app, &state) {
            let _ = tray.set_menu(Some(menu));
        }
        let current = state
            .accounts
            .iter()
            .find(|a| a.current)
            .map(|a| a.name.as_str())
            .unwrap_or("未保存账号");
        let _ = tray.set_tooltip(Some(format!("gpt-Switch · {current}")));
    }
    let _ = app.emit("switch-state", &state);
}
fn refresh(app: &tauri::AppHandle, r: &Runtime) -> Result<ViewState> {
    let state = lock(&r.core)?.state()?;
    publish(app, state.clone());
    Ok(state)
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
fn read_config(r: tauri::State<'_, Arc<Runtime>>) -> Result<ConfigDocument> {
    lock(&r.core)?.read_config()
}
#[tauri::command]
fn validate_config(text: String) -> Result<()> {
    core::validate_config(&text)
}
#[tauri::command]
fn save_config(
    app: tauri::AppHandle,
    r: tauri::State<'_, Arc<Runtime>>,
    text: String,
    expected_revision: String,
) -> Result<ConfigDocument> {
    let doc = lock(&r.core)?.save_config(&text, &expected_revision)?;
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
    let s = lock(&r.core)?.set_preferences(preferences)?;
    publish(&app, s.clone());
    Ok(s)
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
fn quit(app: &tauri::AppHandle, r: &Runtime) {
    r.quitting.store(true, Ordering::Relaxed);
    if let Ok(mut s) = r.login.lock() {
        if let Some(tx) = s.cancel.take() {
            let _ = tx.send(());
            return;
        }
        if login_active(&s.state) {
            return;
        }
    }
    app.exit(0);
}
#[tauri::command]
fn frontend_ready(app: tauri::AppHandle, r: tauri::State<'_, Arc<Runtime>>) -> Result<()> {
    if let Some(output) = &r.smoke {
        let result = (|| -> Result<()> {
            let mut c = lock(&r.core)?;
            let a = c.add_api_key("Smoke A", "fixture-only-a")?;
            let b = c.add_api_key("Smoke B", "fixture-only-b")?;
            let text = "# smoke\nmodel = 'fixture'\n[future]\nkeep = true\n";
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
        let data = serde_json::json!({"ok":result.is_ok(),"version":env!("CARGO_PKG_VERSION"),"revision":env!("GPT_SWITCH_REVISION"),"webview":true,"tray":app.tray_by_id("switch").is_some(),"platform":std::env::consts::OS,"error":result.err()});
        storage::atomic_write(
            output,
            serde_json::to_string_pretty(&data).unwrap().as_bytes(),
            None,
        )?;
        if let Some(fixture) = lock(&r.fixture)?.take() {
            fixture.close().map_err(storage::io_error)?;
        }
        r.quitting.store(true, Ordering::Relaxed);
        app.exit(if data["ok"] == true { 0 } else { 1 });
    } else {
        show(&app, "accounts")?;
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
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            let _ = show(app, "accounts");
        }))
        .plugin(tauri_plugin_dialog::init())
        .setup(move |app| {
            let fixture = if smoke.is_some() {
                Some(
                    tempfile::Builder::new()
                        .prefix("gpt-switch-smoke-")
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
            let core = Core::new(data, home)?;
            let runtime = Arc::new(Runtime {
                core: Mutex::new(core),
                login: Mutex::new(Default::default()),
                quitting: AtomicBool::new(false),
                smoke,
                fixture: Mutex::new(fixture),
            });
            let state = lock(&runtime.core)?.state()?;
            app.manage(runtime.clone());
            let mut tray = TrayIconBuilder::with_id("switch")
                .menu(&tray_menu(app.handle(), &state)?)
                .show_menu_on_left_click(true)
                .tooltip("gpt-Switch");
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
            tray.on_menu_event(|app, e| {
                let r = app.state::<Arc<Runtime>>();
                let id = e.id().as_ref();
                match id {
                    "open" => {
                        let _ = show(app, "accounts");
                    }
                    "config" => {
                        let _ = show(app, "config");
                    }
                    "quit" => quit(app, &r),
                    _ => {
                        if let Some(id) = id.strip_prefix("account:") {
                            let result = lock(&r.core).and_then(|mut c| {
                                let revision = c.state()?.auth_revision;
                                c.switch_account(id, &revision)
                            });
                            match result {
                                Ok(s) => {
                                    publish(app, s);
                                    let _ =
                                        app.emit("switch-notice", "文件已切换，请重新打开 Codex");
                                }
                                Err(e) => {
                                    let _ = app.emit("switch-error", e);
                                    let _ = show(app, "accounts");
                                }
                            }
                        }
                    }
                }
            })
            .build(app)?;
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let mut last = Some(state);
                loop {
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                    if runtime.quitting.load(Ordering::Relaxed) {
                        break;
                    }
                    let result = lock(&runtime.core).and_then(|mut c| c.state());
                    match result {
                        Ok(s) => {
                            if last.as_ref() != Some(&s) {
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
        .on_window_event(|w, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let r = w.state::<Arc<Runtime>>();
                if !r.quitting.load(Ordering::Relaxed) {
                    api.prevent_close();
                    let _ = w.hide();
                    #[cfg(target_os = "macos")]
                    let _ = w
                        .app_handle()
                        .set_activation_policy(tauri::ActivationPolicy::Accessory);
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_state,
            switch_account,
            import_current,
            import_auth_file,
            add_api_key,
            rename_account,
            delete_account,
            read_config,
            validate_config,
            save_config,
            set_preferences,
            pick_path,
            get_login,
            start_login,
            cancel_login,
            open_login_url,
            frontend_ready
        ])
        .build(tauri::generate_context!())
        .expect("gpt-Switch initialization failed")
        .run(|app, event| {
            if let tauri::RunEvent::ExitRequested { api, code, .. } = event {
                if code.is_none() {
                    api.prevent_exit();
                    let r = app.state::<Arc<Runtime>>();
                    quit(app, &r);
                }
            }
        });
}
