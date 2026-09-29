//! Battery clamshell control through the narrowly scoped, authorized power helper.
use crate::storage::{self, AppError, Result};
use serde::{Deserialize, Serialize};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct State {
    pub helper: String,
    pub supported: bool,
    pub enabled: bool,
    pub battery_sleep: u16,
    pub revision: String,
}
impl State {
    fn new(supported: bool, enabled: bool, minutes: u16) -> Self {
        Self {
            helper: if supported { "ready" } else { "unsupported" }.into(),
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
    fn prepare(&self, _force: bool) -> Result<()> {
        Ok(())
    }
    fn remove(&self) -> Result<()> {
        Err(AppError::new("UNSUPPORTED", "此设备不支持电源助手"))
    }
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
    pub fn install(&self) -> Result<State> {
        let _lock = self.lock.lock().unwrap();
        self.system.prepare(true)?;
        self.state()
    }
    pub fn remove(&self) -> Result<State> {
        let _lock = self.lock.lock().unwrap();
        let before = self.state()?;
        if before.enabled {
            self.set_locked(false, &before.revision)?;
        }
        self.system.remove()?;
        self.state()
    }

    pub fn set(&self, enabled: bool, expected: &str) -> Result<State> {
        let _lock = self.lock.lock().unwrap();
        self.set_locked(enabled, expected)
    }
    fn set_locked(&self, enabled: bool, expected: &str) -> Result<State> {
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
        self.system.prepare(false)?;
        let before = self.state()?;
        if before.revision != expected {
            return Err(AppError::new("CONFLICT", "电源状态已变化，请重新操作"));
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
    fn prepare(&self, force: bool) -> Result<()> {
        #[cfg(target_os = "macos")]
        {
            crate::power_macos::ensure(force)
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = force;
            Err(AppError::new("UNSUPPORTED", "此设备不支持电源助手"))
        }
    }
    fn remove(&self) -> Result<()> {
        #[cfg(target_os = "macos")]
        {
            crate::power_macos::remove()
        }
        #[cfg(not(target_os = "macos"))]
        {
            Err(AppError::new("UNSUPPORTED", "此设备不支持电源助手"))
        }
    }
    fn read(&self) -> Result<State> {
        #[cfg(target_os = "macos")]
        {
            let mut state = parse(&output(&["-g"])?, &output(&["-g", "custom"])?)?;
            state.helper = if state.supported {
                crate::power_macos::status()
            } else {
                "unsupported"
            }
            .into();
            Ok(state)
        }
        #[cfg(not(target_os = "macos"))]
        {
            Ok(State::new(false, false, 0))
        }
    }
    fn apply(&self, before: &State, enabled: bool, minutes: u16) -> Result<()> {
        #[cfg(target_os = "macos")]
        {
            crate::power_macos::request(
                serde_json::json!({"op":"set", "enabled":enabled, "minutes":minutes, "beforeEnabled":before.enabled, "beforeSleep":before.battery_sleep}),
            )?;
            Ok(())
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

#[cfg(test)]
mod helper_tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    struct MockHelper {
        current: Mutex<State>,
        ready: AtomicBool,
        cancelled: AtomicBool,
        approvals: AtomicUsize,
        switches: AtomicUsize,
    }
    impl System for MockHelper {
        fn read(&self) -> Result<State> {
            Ok(self.current.lock().unwrap().clone())
        }
        fn prepare(&self, force: bool) -> Result<()> {
            if force || !self.ready.load(Ordering::SeqCst) {
                self.approvals.fetch_add(1, Ordering::SeqCst);
                if self.cancelled.load(Ordering::SeqCst) {
                    return Err(AppError::new("POWER_AUTH", "已取消系统授权"));
                }
                self.ready.store(true, Ordering::SeqCst);
            }
            Ok(())
        }
        fn apply(&self, before: &State, enabled: bool, minutes: u16) -> Result<()> {
            let mut state = self.current.lock().unwrap();
            if state.revision != before.revision {
                return Err(AppError::new("CONFLICT", "外部状态变化"));
            }
            self.switches.fetch_add(1, Ordering::SeqCst);
            *state = State::new(true, enabled, minutes);
            Ok(())
        }
        fn remove(&self) -> Result<()> {
            self.ready.store(false, Ordering::SeqCst);
            Ok(())
        }
    }
    #[test]
    fn first_install_cancel_and_upgrade_do_not_elevate_each_toggle() {
        let t = tempfile::tempdir().unwrap();
        let mut service = Service::new(t.path());
        let helper = Arc::new(MockHelper {
            current: Mutex::new(State::new(true, false, 0)),
            ready: AtomicBool::new(false),
            cancelled: AtomicBool::new(true),
            approvals: AtomicUsize::new(0),
            switches: AtomicUsize::new(0),
        });
        service.system = helper.clone();
        let original = service.state().unwrap();
        assert!(service.set(true, &original.revision).is_err());
        assert_eq!(service.state().unwrap(), original);
        assert!(!service.path.exists());
        helper.cancelled.store(false, Ordering::SeqCst);
        for _ in 0..3 {
            let on = service
                .set(true, &service.state().unwrap().revision)
                .unwrap();
            service.set(false, &on.revision).unwrap();
        }
        assert_eq!(helper.approvals.load(Ordering::SeqCst), 2);
        assert_eq!(helper.switches.load(Ordering::SeqCst), 6);
        helper.ready.store(false, Ordering::SeqCst); // upgraded app has a new approved code identity
        service
            .set(true, &service.state().unwrap().revision)
            .unwrap();
        assert_eq!(helper.approvals.load(Ordering::SeqCst), 3);
        service.remove().unwrap();
        assert!(!service.state().unwrap().enabled);
        assert!(!helper.ready.load(Ordering::SeqCst));
        assert_eq!(service.state().unwrap().battery_sleep, 0);
    }
    #[test]
    fn concurrent_toggles_reserve_one_state_revision() {
        let t = tempfile::tempdir().unwrap();
        let mut service = Service::new(t.path());
        let helper = Arc::new(MockHelper {
            current: Mutex::new(State::new(true, false, 9)),
            ready: AtomicBool::new(true),
            cancelled: AtomicBool::new(false),
            approvals: AtomicUsize::new(0),
            switches: AtomicUsize::new(0),
        });
        service.system = helper.clone();
        let service = Arc::new(service);
        let original = service.state().unwrap().revision;
        let barrier = Arc::new(std::sync::Barrier::new(3));
        let joins: Vec<_> = (0..2)
            .map(|_| {
                let service = service.clone();
                let revision = original.clone();
                let b = barrier.clone();
                std::thread::spawn(move || {
                    b.wait();
                    service.set(true, &revision).is_ok()
                })
            })
            .collect();
        barrier.wait();
        let count = joins
            .into_iter()
            .filter_map(|j| j.join().ok())
            .filter(|v| *v)
            .count();
        assert_eq!(count, 1);
        assert_eq!(helper.switches.load(Ordering::SeqCst), 1);
        assert_eq!(helper.approvals.load(Ordering::SeqCst), 0);
    }
}
