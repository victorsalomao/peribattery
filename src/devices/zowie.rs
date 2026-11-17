//! BenQ ZOWIE EC-CW series (EC1/EC2/EC3-CW). Receiver 04A5:8006, cable 04A5:8005.
//! Community-documented, read-only status query: output report [00] A5 03 on the vendor
//! collection (page >= FF00 or 00FF, 65-byte reports). Reply: [mV lo][mV hi][percent]...
//! mV = 0 means the mouse is off / not linked. Source: github.com/yleshchuk/zowie-battery.
//! NEVER send any other command: A5 09 reboots the receiver and unpairs the mouse.

use crate::hid::{HidDevice, HidInfo};
use crate::model::{interpolate, DeviceKind, DeviceStatus};

const VID: u16 = 0x04A5;
const PID_RECEIVER: u16 = 0x8006;
const PID_CABLE: u16 = 0x8005;
const VOLTS: [u16; 6] = [4200, 4000, 3890, 3760, 3680, 3500];
const PCTS: [u8; 6] = [100, 75, 50, 25, 10, 0];

/// Decodes the status reply. `Some((0, None))` = mouse not linked; `None` = not a battery reply.
pub fn parse(reply: &[u8]) -> Option<(u16, Option<u8>)> {
    let (lo, hi) = (*reply.first()?, *reply.get(1)?);
    let mv = u16::from_le_bytes([lo, hi]);
    if mv == 0 {
        return Some((0, None));
    }
    if !(3000..=4500).contains(&mv) {
        return None;
    }
    let pct = match reply.get(2) {
        Some(&p) if (1..=100).contains(&p) => p,
        _ => interpolate(mv, &VOLTS, &PCTS),
    };
    Some((mv, Some(pct)))
}

pub fn poll(infos: &[HidInfo]) -> Option<DeviceStatus> {
    for pid in [PID_RECEIVER, PID_CABLE] {
        for info in infos.iter().filter(|i| {
            i.vid == VID && i.pid == pid && (i.usage_page >= 0xFF00 || i.usage_page == 0x00FF) && i.out_len == 65
        }) {
            let Some(dev) = HidDevice::open(info) else { continue };
            dev.flush();
            if !dev.write(&[0x00, 0xA5, 0x03]) {
                continue;
            }
            let mut buf = [0u8; 65];
            let Some(n) = dev.read_timeout(&mut buf, 1000) else { continue };
            // Unnumbered reports: Windows prepends a 0 report id.
            let Some((mv, pct)) = parse(buf.get(1..n).unwrap_or(&[])) else { continue };
            return Some(DeviceStatus {
                key: "zowie".into(),
                kind: DeviceKind::Mouse,
                name: "ZOWIE EC2-CW".into(),
                percent: pct,
                millivolts: (mv != 0).then_some(mv),
                charging: pid == PID_CABLE,
                connected: mv != 0,
                estimate: None,
            });
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_ok() {
        assert_eq!(parse(&[0x68, 0x10, 72, 0xEC]), Some((4200, Some(72))));
    }
    #[test]
    fn parse_off() {
        assert_eq!(parse(&[0, 0, 0, 0]), Some((0, None)));
    }
    #[test]
    fn parse_garbage_rejected() {
        assert_eq!(parse(&[0x10, 0x27, 5]), None); // 10000 mV
    }
    #[test]
    fn parse_bad_pct_falls_back_to_curve() {
        assert_eq!(parse(&[0x32, 0x0F, 0]), Some((3890, Some(50))));
    }
    #[test]
    fn parse_short() {
        assert_eq!(parse(&[0x68]), None);
    }
}

/// Raw status reply (diagnostics: `--dump`).
pub fn raw(infos: &[HidInfo]) -> Option<Vec<u8>> {
    let info = infos.iter().find(|i| {
        i.vid == VID && (i.pid == PID_RECEIVER || i.pid == PID_CABLE) && (i.usage_page >= 0xFF00 || i.usage_page == 0x00FF) && i.out_len == 65
    })?;
    let dev = HidDevice::open(info)?;
    dev.flush();
    if !dev.write(&[0x00, 0xA5, 0x03]) {
        return None;
    }
    let mut buf = [0u8; 65];
    let n = dev.read_timeout(&mut buf, 1000)?;
    Some(buf[..n.min(17)].to_vec())
}
