//! Native login items, matching lich13studio's installed-bundle/hidden-launch behavior.
use crate::storage::{self, AppError, Result};
use auto_launch::{AutoLaunch, AutoLaunchBuilder};
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    sync::Mutex,
};
pub const LOGIN_ARG: &str = "--gpt-switch-login-startup";
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Preferences {
    pub launch_to_tray: bool,
    pub restore_gateway: bool,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            launch_to_tray: true,
            restore_gateway: false,
        }
    }
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct View {
    pub launch_on_boot: bool,
    pub launch_to_tray: bool,
    pub restore_gateway: bool,
    pub revision: String,
}
pub struct Service {
    path: PathBuf,
    lock: Mutex<()>,
}
impl Service {
    pub fn new(data: &Path) -> Self {
        Self {
            path: data.join("startup.json"),
            lock: Mutex::new(()),
        }
    }
    pub fn preferences(&self) -> Result<Preferences> {
        Ok(self.read()?.0)
    }
    fn read(&self) -> Result<(Preferences, String)> {
        let raw = storage::read_optional(&self.path)?;
        let settings = raw
            .as_deref()
            .map(serde_json::from_slice)
            .transpose()
            .map_err(|_| AppError::new("STARTUP", "启动设置无法读取"))?
            .unwrap_or_default();
        Ok((settings, storage::revision(raw.as_deref())))
    }
    pub fn view(&self) -> Result<View> {
        let _lock = self.lock.lock().unwrap();
        let (prefs, revision) = self.read()?;
        let launch_on_boot = auto_launch(prefs.launch_to_tray)?
            .is_enabled()
            .map_err(system_error)?;
        Ok(View {
            launch_on_boot,
            launch_to_tray: prefs.launch_to_tray,
            restore_gateway: prefs.restore_gateway,
            revision,
        })
    }
    pub fn update(&self, enabled: bool, prefs: Preferences, expected: &str) -> Result<View> {
        let _lock = self.lock.lock().unwrap();
        let (old, revision) = self.read()?;
        if revision != expected {
            return Err(AppError::new("CONFLICT", "启动设置已变化，请重新打开设置"));
        }
        let previous = auto_launch(old.launch_to_tray)?;
        let was_enabled = previous.is_enabled().map_err(system_error)?;
        let next = auto_launch(prefs.launch_to_tray)?;
        let system_changed =
            was_enabled != enabled || (enabled && old.launch_to_tray != prefs.launch_to_tray);
        if system_changed {
            if was_enabled {
                previous.disable().map_err(system_error)?;
            }
            if enabled {
                if let Err(e) = next.enable() {
                    if was_enabled {
                        let _ = previous.enable();
                    }
                    return Err(system_error(e));
                }
            }
        }
        let actual = next.is_enabled().map_err(system_error)?;
        if actual != enabled {
            return Err(AppError::new("STARTUP", "系统登录项状态与请求不一致"));
        }
        let bytes = serde_json::to_vec_pretty(&prefs)
            .map_err(|_| AppError::new("STARTUP", "无法保存启动设置"))?;
        if let Err(e) = storage::atomic_write(&self.path, &bytes, Some(expected)) {
            if system_changed {
                let _ = next.disable();
                if was_enabled {
                    let _ = previous.enable();
                }
            }
            return Err(e);
        }
        Ok(View {
            launch_on_boot: actual,
            launch_to_tray: prefs.launch_to_tray,
            restore_gateway: prefs.restore_gateway,
            revision: storage::digest(&bytes),
        })
    }
}
fn system_error(_: auto_launch::Error) -> AppError {
    AppError::new(
        "STARTUP",
        "系统登录项操作失败，请检查系统的登录项与自动化权限",
    )
}
fn bundle_path(executable: &Path) -> PathBuf {
    executable
        .ancestors()
        .find(|p| p.extension().is_some_and(|e| e == "app"))
        .unwrap_or(executable)
        .to_owned()
}
fn auto_launch(silent: bool) -> Result<AutoLaunch> {
    let executable = std::env::current_exe().map_err(storage::io_error)?;
    let path = if cfg!(target_os = "macos") {
        bundle_path(&executable)
    } else {
        executable
    };
    if cfg!(target_os = "macos") && path.extension().is_none_or(|e| e != "app") {
        return Err(AppError::new(
            "STARTUP",
            "请在安装后的 gpt-Switch.app 中设置开机启动",
        ));
    }
    let path = path
        .to_str()
        .ok_or_else(|| AppError::new("STARTUP", "安装路径无法读取"))?;
    // auto-launch's macOS AppleScript backend quotes these strings directly.
    if path.contains(['"', '\n', '\r']) || (cfg!(target_os = "macos") && path.contains('\\')) {
        return Err(AppError::new(
            "STARTUP",
            "安装路径含不支持的字符，请移动到标准应用目录",
        ));
    }
    let args = if cfg!(target_os = "macos") {
        if silent {
            vec!["--hidden"]
        } else {
            vec![]
        }
    } else {
        vec![LOGIN_ARG]
    };
    // auto-launch writes its Windows path verbatim into the Run command.
    let path = registration_path(path, cfg!(target_os = "windows"));
    AutoLaunchBuilder::new()
        .set_app_name("gpt-Switch")
        .set_app_path(&path)
        .set_use_launch_agent(false)
        .set_args(&args)
        .build()
        .map_err(system_error)
}
fn registration_path(path: &str, windows: bool) -> String {
    if windows {
        format!("\"{path}\"")
    } else {
        path.to_owned()
    }
}
#[cfg(windows)]
pub fn smoke_registration() -> Result<()> {
    let entry = auto_launch(true)?;
    if entry.is_enabled().map_err(system_error)? {
        return Ok(());
    }
    entry.enable().map_err(system_error)?;
    let checked = (|| -> Result<()> {
        use winreg::{enums::HKEY_CURRENT_USER, RegKey};
        let value: String = RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey("SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Run")
            .and_then(|key| key.get_value("gpt-Switch"))
            .map_err(storage::io_error)?;
        let expected = format!("{} {LOGIN_ARG}", entry.get_app_path());
        if value != expected || !entry.is_enabled().map_err(system_error)? {
            return Err(AppError::new("SMOKE", "Windows 启动项校验失败"));
        }
        Ok(())
    })();
    entry.disable().map_err(system_error)?;
    checked?;
    if entry.is_enabled().map_err(system_error)? {
        return Err(AppError::new("SMOKE", "Windows 启动项未解除"));
    }
    Ok(())
}
pub fn login_source(args: &[String]) -> bool {
    if args.iter().any(|a| a == LOGIN_ARG) {
        return true;
    }
    #[cfg(target_os = "macos")]
    {
        use objc2::MainThreadMarker;
        use objc2_app_kit::NSApplication;
        MainThreadMarker::new().is_some_and(|m| NSApplication::sharedApplication(m).isHidden())
    }
    #[cfg(not(target_os = "macos"))]
    false
}
pub fn silent(args: &[String], prefs: &Preferences) -> bool {
    prefs.launch_to_tray && login_source(args)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn installed_path_and_legacy_preferences() {
        assert_eq!(
            bundle_path(Path::new(
                "/Applications/gpt-Switch.app/Contents/MacOS/gpt-switch"
            )),
            PathBuf::from("/Applications/gpt-Switch.app")
        );
        let prefs: Preferences = serde_json::from_str("{}").unwrap();
        assert!(prefs.launch_to_tray);
        assert!(!prefs.restore_gateway);
        assert_eq!(
            registration_path(r"C:\Program Files\gpt-Switch\gpt-switch.exe", true),
            r#""C:\Program Files\gpt-Switch\gpt-switch.exe""#
        );
        assert!(silent(&[LOGIN_ARG.into()], &prefs));
        assert!(!silent(
            &[LOGIN_ARG.into()],
            &Preferences {
                launch_to_tray: false,
                restore_gateway: true
            }
        ));
    }
}
