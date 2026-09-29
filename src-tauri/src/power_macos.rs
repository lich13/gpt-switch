//! The only elevation is installation of a verified, narrow helper. Normal toggles use XPC.
use crate::storage::{self, AppError, Result};
use serde_json::{json, Value};
use std::{
    ffi::{CStr, CString},
    os::raw::c_char,
    path::Path,
    process::Command,
};
const APP: &str = "/Applications/gpt-Switch.app/Contents/MacOS/gpt-switch";
const HELPER: &str = "/Library/PrivilegedHelperTools/com.lich13.gpt-switch.power-helper";
const IMAGE: &[u8] = include_bytes!(env!("GPT_POWER_HELPER"));
unsafe extern "C" {
    fn gs_power_request(json: *const c_char, requirement: *const c_char) -> *mut c_char;
    fn gs_power_identity() -> *mut c_char;
    fn gs_power_free(value: *mut c_char);
}
fn take(value: *mut c_char) -> Option<String> {
    if value.is_null() {
        return None;
    }
    let string = unsafe { CStr::from_ptr(value) }
        .to_string_lossy()
        .into_owned();
    unsafe {
        gs_power_free(value);
    }
    Some(string)
}
fn installed_app() -> bool {
    std::env::current_exe()
        .ok()
        .is_some_and(|p| p == Path::new(APP))
}
pub fn request(value: Value) -> Result<Value> {
    if !installed_app() {
        return Err(AppError::new(
            "POWER_ISOLATED",
            "请从 /Applications 中的正式应用使用电源助手",
        ));
    }
    let input = CString::new(value.to_string()).unwrap();
    let requirement = CString::new(format!(
        "identifier \"com.lich13.gpt-switch.power-helper\" and cdhash H\"{}\"",
        env!("GPT_POWER_HASH")
    ))
    .unwrap();
    let response = take(unsafe { gs_power_request(input.as_ptr(), requirement.as_ptr()) })
        .ok_or_else(|| {
            AppError::new("POWER_HELPER", "电源助手不可用或应用版本未授权，请修复助手")
        })?;
    let response: Value = serde_json::from_str(&response)
        .map_err(|_| AppError::new("POWER_HELPER", "电源助手响应无效"))?;
    if let Some(e) = response.get("error") {
        let code = e
            .get("code")
            .and_then(Value::as_str)
            .unwrap_or("POWER_HELPER");
        let message = e
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("电源操作失败");
        return Err(AppError::new(code, message));
    }
    if response.get("version") != Some(&json!(1)) {
        return Err(AppError::new(
            "POWER_HELPER",
            "电源助手版本不兼容，请修复助手",
        ));
    }
    Ok(response)
}
pub fn status() -> &'static str {
    if !installed_app() {
        return "isolated";
    }
    if !Path::new(HELPER).exists() {
        return "notInstalled";
    }
    if request(json!({"op":"state"})).is_ok() {
        "ready"
    } else {
        "needsRepair"
    }
}
fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}
pub fn ensure(force: bool) -> Result<()> {
    if !installed_app() {
        return Err(AppError::new(
            "POWER_ISOLATED",
            "隔离程序不注册系统电源助手",
        ));
    }
    if !force && status() == "ready" {
        return Ok(());
    }
    let hash = take(unsafe { gs_power_identity() })
        .filter(|s| s.len() == 40 && s.bytes().all(|b| b.is_ascii_hexdigit()))
        .ok_or_else(|| AppError::new("POWER_AUTH", "无法校验当前应用签名"))?;
    let folder = tempfile::tempdir().map_err(storage::io_error)?;
    let payload = folder.path().join("power-helper");
    storage::atomic_write(&payload, IMAGE, None)?;
    let stage = format!(
        "/Library/PrivilegedHelperTools/com.lich13.gpt-switch.power-install-{}",
        uuid::Uuid::new_v4()
    );
    let uid = unsafe { libc::geteuid() };
    // Copy into a root-owned directory, then hash the destination before ever
    // executing it. A modified user-writable staging file cannot become root code.
    let shell = format!(
        r#"set -eu
/usr/bin/test ! -L /Library/PrivilegedHelperTools
/bin/mkdir -p /Library/PrivilegedHelperTools
/usr/bin/test "$(/usr/bin/stat -f '%u' /Library/PrivilegedHelperTools)" = 0
/usr/bin/test "$(/usr/bin/stat -f '%Lp' /Library/PrivilegedHelperTools)" = 755
trap '/bin/rm -f -- {stage}' EXIT
/usr/bin/install -o root -g wheel -m 700 {source} {stage}
/usr/bin/test "$(/usr/bin/shasum -a 256 {stage} | /usr/bin/cut -d ' ' -f 1)" = {digest}
{stage} --install {uid} {app} {hash}
"#,
        source = quote(&payload.to_string_lossy()),
        digest = quote(&storage::digest(IMAGE)),
        app = quote(APP),
        hash = quote(&hash)
    );
    let script = format!(
        "do shell script \"{}\" with administrator privileges",
        shell
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\n', "\\n")
    );
    let result = Command::new("/usr/bin/osascript")
        .args(["-e", &script])
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(storage::io_error)?;
    if !result.status.success() {
        return Err(AppError::new(
            "POWER_AUTH",
            if String::from_utf8_lossy(&result.stderr).contains("-128") {
                "已取消系统授权"
            } else {
                "电源助手安装失败，原注册状态已尝试恢复"
            },
        ));
    }
    request(json!({"op":"state"}))?;
    Ok(())
}
pub fn remove() -> Result<()> {
    request(json!({"op":"remove"})).map(|_| ())
}
#[cfg(test)]
mod tests {
    #[test]
    fn native_helper_transactions_are_simulated() {
        let output = std::process::Command::new(env!("GPT_POWER_TEST"))
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    #[test]
    fn isolated_binary_never_installs_or_calls_production_service() {
        assert!(!super::installed_app());
        assert_eq!(super::status(), "isolated");
        assert_eq!(super::ensure(false).unwrap_err().code, "POWER_ISOLATED");
        assert!(super::request(serde_json::json!({"op":"set"})).is_err());
    }
    #[test]
    fn quotes_cannot_expand_shell_syntax() {
        assert_eq!(super::quote("a'b$(id)`x`"), "'a'\\''b$(id)`x`'");
    }
}
