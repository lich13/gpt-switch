//! Battery clamshell control. Only the two pmset operations are elevated.
use crate::storage::{self, AppError, Result};
use serde::{Deserialize, Serialize};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct State {
    pub supported: bool,
    pub enabled: bool,
    pub battery_sleep: u16,
    pub revision: String,
}
impl State {
    fn new(supported: bool, enabled: bool, minutes: u16) -> Self {
        Self {
            supported,
            enabled,
            battery_sleep: minutes,
            revision: storage::digest(format!("{supported}:{enabled}:{minutes}").as_bytes()),
        }
    }
}
#[derive(Serialize, Deserialize)]
struct Restore {
    version: u32,
    minutes: u16,
}
trait System: Send + Sync {
    fn read(&self) -> Result<State>;
    fn apply(&self, before: &State, enabled: bool, minutes: u16) -> Result<()>;
}
struct Native;
pub struct Service {
    path: PathBuf,
    legacy: PathBuf,
    system: Arc<dyn System>,
    lock: Mutex<()>,
}
impl Service {
    pub fn new(data: &std::path::Path) -> Self {
        Self {
            path: data.join("clamshell-restore.json"),
            legacy: dirs::home_dir()
                .unwrap_or_default()
                .join("Library/Application Support/battery-clamshell-awake/battery-sleep-minutes"),
            system: Arc::new(Native),
            lock: Mutex::new(()),
        }
    }
    pub fn state(&self) -> Result<State> {
        self.system.read()
    }
    pub fn set(&self, enabled: bool, expected: &str) -> Result<State> {
        let _lock = self.lock.lock().unwrap();
        let before = self.state()?;
        if !before.supported {
            return Err(AppError::new("UNSUPPORTED", "此设备不支持电池合盖控制"));
        }
        if before.revision != expected {
            return Err(AppError::new("CONFLICT", "电源状态已变化，请重新操作"));
        }
        if before.enabled == enabled {
            return Ok(before);
        }
        let old = storage::read_optional(&self.path)?;
        let restore = old
            .as_deref()
            .map(serde_json::from_slice::<Restore>)
            .transpose()
            .map_err(|_| AppError::new("POWER", "休眠恢复记录无效"))?;
        if restore
            .as_ref()
            .is_some_and(|r| r.version != 1 || r.minutes > 1440)
        {
            return Err(AppError::new("POWER", "休眠恢复记录无效"));
        }
        let minutes = if enabled {
            0
        } else if let Some(r) = restore {
            r.minutes
        } else {
            storage::read_optional(&self.legacy)?
                .and_then(|b| String::from_utf8(b).ok())
                .and_then(|s| s.trim().parse::<u16>().ok())
                .filter(|n| *n <= 1440)
                .unwrap_or(1)
        };
        if enabled {
            let bytes = serde_json::to_vec(&Restore {
                version: 1,
                minutes: before.battery_sleep,
            })
            .unwrap();
            storage::atomic_write(&self.path, &bytes, Some(&storage::revision(old.as_deref())))?;
        }
        let result = self.system.apply(&before, enabled, minutes);
        let actual = self.state();
        if let Err(e) = result {
            // An authorization cancellation must not leave a newly-created restore record.
            if enabled && actual.as_ref().is_ok_and(|s| s == &before) {
                if let Some(bytes) = old {
                    storage::atomic_write(&self.path, &bytes, None)?;
                } else {
                    std::fs::remove_file(&self.path).map_err(storage::io_error)?;
                }
            }
            return Err(e);
        }
        let actual = actual?;
        if actual.enabled != enabled || actual.battery_sleep != minutes {
            return Err(AppError::new("POWER", "电源设置回读不一致，恢复记录已保留"));
        }
        if !enabled && self.path.exists() {
            std::fs::remove_file(&self.path).map_err(storage::io_error)?;
        }
        Ok(actual)
    }
}
#[cfg(target_os = "macos")]
fn output(args: &[&str]) -> Result<String> {
    let result = std::process::Command::new("/usr/bin/pmset")
        .args(args)
        .output()
        .map_err(storage::io_error)?;
    if !result.status.success() {
        return Err(AppError::new("POWER", "无法读取系统电源状态"));
    }
    String::from_utf8(result.stdout).map_err(|_| AppError::new("POWER", "无法读取系统电源状态"))
}
#[cfg(any(target_os = "macos", test))]
fn parse(general: &str, custom: &str) -> Result<State> {
    let flag = general.lines().find_map(|l| {
        let mut words = l.split_whitespace();
        (words.next() == Some("SleepDisabled"))
            .then(|| words.next())
            .flatten()
    });
    if !custom.lines().any(|l| l.trim() == "Battery Power:") {
        return Ok(State::new(false, false, 0));
    }
    let mut battery = false;
    let mut minutes = None;
    for line in custom.lines() {
        if line.trim() == "Battery Power:" {
            battery = true;
            continue;
        }
        if !line.starts_with(char::is_whitespace) {
            battery = false;
        }
        if battery {
            let mut words = line.split_whitespace();
            if words.next() == Some("sleep") {
                minutes = words
                    .next()
                    .and_then(|s| s.parse::<u16>().ok())
                    .filter(|n| *n <= 1440);
            }
        }
    }
    match (flag, minutes) {
        (Some("0" | "1"), Some(n)) => Ok(State::new(true, flag == Some("1"), n)),
        _ => Err(AppError::new("POWER", "无法读取合盖休眠状态")),
    }
}
impl System for Native {
    fn read(&self) -> Result<State> {
        #[cfg(target_os = "macos")]
        {
            parse(&output(&["-g"])?, &output(&["-g", "custom"])?)
        }
        #[cfg(not(target_os = "macos"))]
        {
            Ok(State::new(false, false, 0))
        }
    }
    fn apply(&self, before: &State, enabled: bool, minutes: u16) -> Result<()> {
        #[cfg(target_os = "macos")]
        {
            // Only validated integers are substituted. Recheck inside the authorized
            // operation because the system dialog may remain open for a long time.
            let verify = format!(
                r#"test "$(/usr/bin/pmset -g | /usr/bin/awk '$1=="SleepDisabled"{{print $2;exit}}')" = "{}" && test "$(/usr/bin/pmset -g custom | /usr/bin/awk '/^Battery Power:/{{b=1;next}} /^[^ \t]/{{b=0}} b && $1=="sleep"{{print $2;exit}}')" = "{}" || exit 73; "#,
                u8::from(before.enabled),
                before.battery_sleep
            );
            let commands = if enabled {
                "/usr/bin/pmset -b sleep 0 && /usr/bin/pmset -b disablesleep 1".to_string()
            } else {
                format!("/usr/bin/pmset -b disablesleep 0 && /usr/bin/pmset -b sleep {minutes}")
            };
            let rollback = format!(
                "/usr/bin/pmset -b disablesleep {} ; /usr/bin/pmset -b sleep {} ; exit 74",
                u8::from(before.enabled),
                before.battery_sleep
            );
            let shell = format!("{verify}if {commands}; then exit 0; else {rollback}; fi");
            let script = format!(
                "do shell script \"{}\" with administrator privileges",
                shell.replace('\\', "\\\\").replace('"', "\\\"")
            );
            let result = std::process::Command::new("/usr/bin/osascript")
                .args(["-e", &script])
                .stdin(std::process::Stdio::null())
                .output()
                .map_err(storage::io_error)?;
            if result.status.success() {
                Ok(())
            } else {
                let message = String::from_utf8_lossy(&result.stderr);
                Err(AppError::new(
                    "POWER",
                    if message.contains("-128") {
                        "已取消系统授权"
                    } else if message.contains("(73)") {
                        "电源状态已变化，请重新操作"
                    } else {
                        "电源设置失败，请检查系统授权；原设置已尝试恢复"
                    },
                ))
            }
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = (before, enabled, minutes);
            Err(AppError::new("UNSUPPORTED", "此设备不支持电池合盖控制"))
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    struct Mock {
        state: Mutex<State>,
        fail: bool,
    }
    impl System for Mock {
        fn read(&self) -> Result<State> {
            Ok(self.state.lock().unwrap().clone())
        }
        fn apply(&self, _: &State, enabled: bool, minutes: u16) -> Result<()> {
            if self.fail {
                return Err(AppError::new("POWER", "cancelled"));
            }
            *self.state.lock().unwrap() = State::new(true, enabled, minutes);
            Ok(())
        }
    }
    #[test]
    fn zero_minutes_roundtrip_and_conflict() {
        let t = tempfile::tempdir().unwrap();
        let mut s = Service::new(t.path());
        s.system = Arc::new(Mock {
            state: Mutex::new(State::new(true, false, 0)),
            fail: false,
        });
        let a = s.state().unwrap();
        let b = s.set(true, &a.revision).unwrap();
        assert!(s.set(false, &a.revision).is_err());
        let c = s.set(false, &b.revision).unwrap();
        assert_eq!(c, a);
        assert!(!s.path.exists());
    }
    #[test]
    fn cancellation_and_legacy_record() {
        let t = tempfile::tempdir().unwrap();
        let mut s = Service::new(t.path());
        s.system = Arc::new(Mock {
            state: Mutex::new(State::new(true, false, 3)),
            fail: true,
        });
        assert!(s.set(true, &s.state().unwrap().revision).is_err());
        assert!(!s.path.exists());
        s.legacy = t.path().join("legacy");
        std::fs::write(&s.legacy, "7").unwrap();
        s.system = Arc::new(Mock {
            state: Mutex::new(State::new(true, true, 0)),
            fail: false,
        });
        assert_eq!(
            s.set(false, &s.state().unwrap().revision)
                .unwrap()
                .battery_sleep,
            7
        );
        assert_eq!(std::fs::read_to_string(&s.legacy).unwrap(), "7");
    }
    #[test]
    fn native_status_parsing_does_not_mix_power_sources() {
        assert_eq!(
            parse(
                " SleepDisabled 1\n",
                "Battery Power:\n sleep 0\nAC Power:\n sleep 9\n"
            )
            .unwrap(),
            State::new(true, true, 0)
        );
        assert!(
            !parse("SleepDisabled 0", "AC Power:\n sleep 9")
                .unwrap()
                .supported
        );
        assert!(parse("", "Battery Power:\n sleep 0").is_err());
    }
}
