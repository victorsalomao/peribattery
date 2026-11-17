//! Tiny change log for diagnostics: one line whenever a device's reading changes.
//! %LOCALAPPDATA%\PeriBattery\diag.log, restarted when it passes 256 KB.

use crate::model::DeviceStatus;

const MAX_BYTES: u64 = 256 * 1024;

/// "14:31:28 zowie 20% 3758mV chg=1 on=1"
pub fn line(hms: &str, d: &DeviceStatus) -> String {
    let pct = d.percent.map(|p| format!("{p}%")).unwrap_or_else(|| "-".into());
    let mv = d.millivolts.map(|v| format!("{v}mV")).unwrap_or_else(|| "-".into());
    format!("{hms} {} {pct} {mv} chg={} on={}", d.key, d.charging as u8, d.connected as u8)
}

/// Appends a line for `d` (errors are ignored: diagnostics must never disturb the app).
pub fn log(d: &DeviceStatus) {
    let Some(base) = std::env::var_os("LOCALAPPDATA") else { return };
    let path = std::path::PathBuf::from(base).join(crate::settings::APP).join("diag.log");
    if std::fs::metadata(&path).map(|m| m.len() > MAX_BYTES).unwrap_or(false) {
        let _ = std::fs::remove_file(&path);
    }
    let hms = now_hms();
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        use std::io::Write;
        let _ = writeln!(f, "{}", line(&hms, d));
    }
}

fn now_hms() -> String {
    let mut st: windows_sys::Win32::Foundation::SYSTEMTIME = unsafe { std::mem::zeroed() };
    unsafe { windows_sys::Win32::System::SystemInformation::GetLocalTime(&mut st) };
    format!("{:02}/{:02} {:02}:{:02}:{:02}", st.wDay, st.wMonth, st.wHour, st.wMinute, st.wSecond)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::DeviceKind;

    #[test]
    fn line_format() {
        let d = DeviceStatus { key: "zowie".into(), kind: DeviceKind::Mouse, name: "ZOWIE".into(), percent: Some(20), millivolts: Some(3758), charging: true, connected: true, estimate: None };
        assert_eq!(line("17/11 14:31:28", &d), "17/11 14:31:28 zowie 20% 3758mV chg=1 on=1");
    }
}
