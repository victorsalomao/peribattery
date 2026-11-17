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

/// Copies the running exe to `%LOCALAPPDATA%\PeriBattery` when it runs from elsewhere.
/// Returns the installed path when a copy was made.
pub fn ensure_installed() -> Option<PathBuf> {
    let target = install_path()?;
    let me = std::env::current_exe().ok()?;
    if me.eq(&target) || std::fs::canonicalize(&me).ok() == std::fs::canonicalize(&target).ok() {
        return None;
    }
    std::fs::create_dir_all(target.parent()?).ok()?;
    std::fs::copy(&me, &target).ok()?;
    Some(target)
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
