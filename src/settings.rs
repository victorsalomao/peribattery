//! Per-user settings and install: HKCU only, never needs admin.

use std::path::{Path, PathBuf};

pub const APP: &str = "PeriBattery";
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const APP_KEY: &str = r"Software\PeriBattery";

/// Value written to the Run key: the quoted exe path.
pub fn run_value(path: &Path) -> String {
    format!("\"{}\"", path.display())
}

fn split_hidden(s: &str) -> Vec<String> {
    s.split(';').filter(|k| !k.is_empty()).map(String::from).collect()
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn get_string(key: &str, name: &str) -> Option<String> {
    use windows_sys::Win32::System::Registry::*;
    let (k, n) = (wide(key), wide(name));
    let mut buf = [0u16; 1024];
    let mut size = (buf.len() * 2) as u32;
    let rc = unsafe {
        RegGetValueW(HKEY_CURRENT_USER, k.as_ptr(), n.as_ptr(), RRF_RT_REG_SZ, std::ptr::null_mut(), buf.as_mut_ptr() as _, &mut size)
    };
    if rc != 0 {
        return None;
    }
    let len = (size as usize / 2).saturating_sub(1).min(buf.len());
    Some(String::from_utf16_lossy(&buf[..len]))
}

fn set_string(key: &str, name: &str, value: &str) {
    use windows_sys::Win32::System::Registry::*;
    let (k, n, v) = (wide(key), wide(name), wide(value));
    unsafe {
        RegSetKeyValueW(HKEY_CURRENT_USER, k.as_ptr(), n.as_ptr(), REG_SZ, v.as_ptr() as _, (v.len() * 2) as u32);
    }
}

fn delete_value(key: &str, name: &str) {
    let (k, n) = (wide(key), wide(name));
    unsafe {
        windows_sys::Win32::System::Registry::RegDeleteKeyValueW(windows_sys::Win32::System::Registry::HKEY_CURRENT_USER, k.as_ptr(), n.as_ptr());
    }
}

pub fn autostart_enabled() -> bool {
    get_string(RUN_KEY, APP).is_some()
}

/// Points the Run entry at the installed copy (or the running exe if not installed).
pub fn set_autostart(on: bool) {
    if on {
        let exe = install_path().filter(|p| p.exists()).or_else(|| std::env::current_exe().ok());
        if let Some(exe) = exe {
            set_string(RUN_KEY, APP, &run_value(&exe));
        }
    } else {
        delete_value(RUN_KEY, APP);
    }
}

pub fn hidden() -> Vec<String> {
    get_string(APP_KEY, "Hidden").map(|s| split_hidden(&s)).unwrap_or_default()
}

pub fn set_hidden(key: &str, hide: bool) {
    let mut list = hidden();
    list.retain(|k| k != key);
    if hide {
        list.push(key.to_string());
    }
    set_string(APP_KEY, "Hidden", &list.join(";"));
}

pub fn is_installed() -> bool {
    get_string(APP_KEY, "Installed").is_some()
}

pub fn mark_installed() {
    set_string(APP_KEY, "Installed", "1");
}

pub fn install_path() -> Option<PathBuf> {
    let base = std::env::var_os("LOCALAPPDATA")?;
    Some(PathBuf::from(base).join(APP).join("PeriBattery.exe"))
}

/// What to do at startup, looking at the running exe and the installed copy.
pub fn startup_action() -> (InstallAction, Option<PathBuf>) {
    let (Some(target), Ok(me)) = (install_path(), std::env::current_exe()) else {
        return (InstallAction::Run, None);
    };
    let canon = |p: &Path| std::fs::canonicalize(p).ok();
    let me_is_target = me == target || (target.exists() && canon(&me) == canon(&target));
    let identical = !me_is_target
        && target.exists()
        && matches!((std::fs::read(&me), std::fs::read(&target)), (Ok(a), Ok(b)) if a == b);
    (install_action(me_is_target, target.exists(), identical), Some(target))
}

/// Copies the running exe over the installed one. Retries while a just-closed copy still
/// holds the file.
pub fn copy_self_to(target: &Path) -> bool {
    let Ok(me) = std::env::current_exe() else { return false };
    if let Some(dir) = target.parent() {
        if std::fs::create_dir_all(dir).is_err() {
            return false;
        }
    }
    for _ in 0..20 {
        if std::fs::copy(&me, target).is_ok() {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(150));
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_value_quotes_path_with_spaces() {
        let p = Path::new(r"C:\Users\Some One\AppData\Local\PeriBattery\PeriBattery.exe");
        assert_eq!(run_value(p), r#""C:\Users\Some One\AppData\Local\PeriBattery\PeriBattery.exe""#);
    }

    #[test]
    fn hidden_list_roundtrip() {
        assert_eq!(split_hidden("zowie;logi:c547:1;"), vec!["zowie".to_string(), "logi:c547:1".to_string()]);
        assert!(split_hidden("").is_empty());
    }
}

#[cfg(test)]
mod install_tests {
    use super::*;

    #[test]
    fn install_decisions() {
        assert_eq!(install_action(true, true, true), InstallAction::Run); // running the installed copy
        assert_eq!(install_action(false, false, false), InstallAction::Install); // first run / folder deleted
        assert_eq!(install_action(false, true, false), InstallAction::Install); // newer build
        assert_eq!(install_action(false, true, true), InstallAction::Run); // same build elsewhere
    }
}

#[derive(Debug, PartialEq)]
pub enum InstallAction {
    Run,
    Install,
}

/// `me_is_target`: running from the install path; `identical`: installed exe has our bytes.
pub fn install_action(me_is_target: bool, target_exists: bool, identical: bool) -> InstallAction {
    if me_is_target || (target_exists && identical) {
        InstallAction::Run
    } else {
        InstallAction::Install
    }
}

const RATES_KEY: &str = r"Software\PeriBattery\Rates";

/// "dis=1.42;chg=38.5" ⇄ Learned.
fn parse_rates(s: &str) -> crate::estimate::Learned {
    let mut l = crate::estimate::Learned::default();
    for part in s.split(';') {
        let Some((k, v)) = part.split_once('=') else { continue };
        let v = v.trim().parse::<f32>().ok().filter(|r| r.is_finite() && *r > 0.0);
        match k.trim() {
            "dis" => l.discharge = v,
            "chg" => l.charge = v,
            _ => {}
        }
    }
    l
}

fn format_rates(l: &crate::estimate::Learned) -> String {
    let mut parts = Vec::new();
    if let Some(d) = l.discharge {
        parts.push(format!("dis={d:.2}"));
    }
    if let Some(c) = l.charge {
        parts.push(format!("chg={c:.2}"));
    }
    parts.join(";")
}

/// Rates learned for a device in earlier runs.
pub fn learned(key: &str) -> crate::estimate::Learned {
    get_string(RATES_KEY, key).map(|s| parse_rates(&s)).unwrap_or_default()
}

/// Blends a freshly measured rate into the stored one.
pub fn save_rate(key: &str, charging: bool, rate: f32) {
    let mut l = learned(key);
    if charging {
        l.charge = Some(crate::estimate::blend(l.charge, rate));
    } else {
        l.discharge = Some(crate::estimate::blend(l.discharge, rate));
    }
    set_string(RATES_KEY, key, &format_rates(&l));
}

#[cfg(test)]
mod rate_tests {
    use super::*;

    #[test]
    fn rates_roundtrip() {
        let l = parse_rates("dis=1.42;chg=38.5");
        assert_eq!((l.discharge, l.charge), (Some(1.42), Some(38.5)));
        assert_eq!(format_rates(&l), "dis=1.42;chg=38.50");
        let l = parse_rates("chg=12");
        assert_eq!((l.discharge, l.charge), (None, Some(12.0)));
        assert_eq!(parse_rates("lixo").discharge, None);
    }
}
