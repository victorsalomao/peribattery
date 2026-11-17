//! Sony DualSense / DualSense Edge. READ ONLY: never writes, never asks for feature reports,
//! so Steam Input keeps working. USB input report 0x01 (64 bytes) has the status byte at 53,
//! Bluetooth full report 0x31 (78 bytes) at 54. Low nibble = level 0..10, high nibble =
//! 0 discharging, 1 charging, 2 full.

use crate::hid::{HidDevice, HidInfo};
use crate::model::{DeviceKind, DeviceStatus};

const VID: u16 = 0x054C;
const PID_DS: u16 = 0x0CE6;
const PID_EDGE: u16 = 0x0DF2;

/// `(percent, charging)` from one input report, `None` for reports without battery.
pub fn parse(report: &[u8]) -> Option<(u8, bool)> {
    let status = match (*report.first()?, report.len()) {
        (0x01, n) if n >= 64 => report[53],
        (0x31, n) if n >= 78 => report[54],
        _ => return None,
    };
    let level = status & 0x0F;
    let state = status >> 4;
    let pct = if state == 2 { 100 } else { (level as u16 * 10 + 5).min(100) as u8 };
    Some((pct, matches!(state, 1 | 2)))
}

pub fn poll(infos: &[HidInfo]) -> Option<DeviceStatus> {
    for info in infos.iter().filter(|i| {
        i.vid == VID && (i.pid == PID_DS || i.pid == PID_EDGE) && i.usage_page == 0x01 && i.usage == 0x05
    }) {
        let Some(dev) = HidDevice::open(info) else { continue };
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(300);
        let mut buf = [0u8; 128];
        while std::time::Instant::now() < deadline {
            let Some(n) = dev.read_timeout(&mut buf, 50) else { continue };
            if let Some((pct, charging)) = parse(&buf[..n]) {
                return Some(DeviceStatus {
                    key: "ds".into(),
                    kind: DeviceKind::Gamepad,
                    name: if info.pid == PID_EDGE { "DualSense Edge" } else { "DualSense" }.into(),
                    percent: Some(pct),
                    millivolts: None,
                    charging,
                    connected: true,
                });
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usb_level_and_charging() {
        let mut b = [0u8; 64];
        b[0] = 0x01;
        b[53] = 0x17;
        assert_eq!(parse(&b), Some((75, true)));
    }

    #[test]
    fn bt_offset() {
        let mut b = [0u8; 78];
        b[0] = 0x31;
        b[54] = 0x05;
        assert_eq!(parse(&b), Some((55, false)));
    }

    #[test]
    fn full_is_100() {
        let mut b = [0u8; 64];
        b[0] = 0x01;
        b[53] = 0x2A;
        assert_eq!(parse(&b), Some((100, true)));
    }

    #[test]
    fn short_report_none() {
        assert_eq!(parse(&[0x01, 0x80, 0x80, 0x80, 0x80, 0x08, 0, 0, 0, 0]), None);
    }
}
