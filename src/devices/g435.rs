//! Logitech G435 on the 046D:0ADF LIGHTSPEED dongle. Not HID++: the vendor collection
//! FF00:0001 (65-byte unnumbered reports) carries CRC-16/XMODEM framed messages; "PA" commands
//! inside reach the headset's radio SoC and GET_BATTERY returns millivolts.
//! Protocol: github.com/portyporty/g435-battery-percentage (MIT). Does not interrupt audio.

use crate::hid::{HidDevice, HidInfo};
use crate::model::{interpolate, DeviceKind, DeviceStatus};

const VID: u16 = 0x046D;
const PID: u16 = 0x0ADF;
const VOLTS: [u16; 16] = [4200, 4150, 4100, 4050, 4000, 3950, 3900, 3850, 3800, 3750, 3700, 3650, 3600, 3550, 3500, 3400];
const PCTS: [u8; 16] = [100, 95, 90, 85, 80, 75, 70, 62, 55, 45, 35, 25, 15, 10, 5, 0];

const CAT_OTA: u8 = 0x06;
const CAT_WIRELESS: u8 = 0x0E;
const GET_MODE: u8 = 0xE0;
const SET_MODE: u8 = 0xE1;
/// OTA_CMD_IOCTL_READ_MAX_LEN session init, as captured (sequence 1 embedded).
const SESSION_INIT: [u8; 19] = [0x50, 0x41, 0x06, 0x01, 0x0D, 0x00, 0x25, 0x34, 0x12, 0x5A, 0x5A, 0x01, 0x00, 0x00, 0x00, 0xF0, 0x00, 0x00, 0x00];

/// CRC-16/XMODEM (poly 0x1021, init 0).
pub fn crc16(d: &[u8]) -> u16 {
    let mut crc: u16 = 0;
    for &b in d {
        crc ^= (b as u16) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 { (crc << 1) ^ 0x1021 } else { crc << 1 };
        }
    }
    crc
}

/// 64-byte frame `[cmd, body len, crc lo, crc hi, body…]`; CRC over the frame with the CRC field zero.
pub fn frame(cmd: u8, body: &[u8]) -> [u8; 64] {
    let mut f = [0u8; 64];
    f[0] = cmd;
    f[1] = body.len() as u8;
    let n = body.len().min(60);
    f[4..4 + n].copy_from_slice(&body[..n]);
    let crc = crc16(&f);
    f[2..4].copy_from_slice(&crc.to_le_bytes());
    f
}

pub fn pa(cat: u8, seq: u8, len: u8, payload: &[u8]) -> Vec<u8> {
    let mut v = vec![0x50, 0x41, cat, seq, len];
    v.extend_from_slice(payload);
    v
}

/// Finds `00 07 00 06 <seq> 00 <mV lo> <mV hi>` and returns a plausible Li-Po voltage.
pub fn parse_mv(d: &[u8]) -> Option<u16> {
    d.windows(8).find_map(|w| {
        if w[0] == 0 && w[1] == 7 && w[2] == 0 && w[3] == 6 && w[5] == 0 {
            let mv = u16::from_le_bytes([w[6], w[7]]);
            (2500..=4500).contains(&mv).then_some(mv)
        } else {
            None
        }
    })
}

pub fn percent(mv: u16) -> u8 {
    interpolate(mv, &VOLTS, &PCTS)
}

/// Sends one frame and reads input reports for `wait_ms`; with `want_mv`, stops at a battery reply.
fn transact(dev: &HidDevice, cmd: u8, body: &[u8], wait_ms: u64, want_mv: bool) -> Option<u16> {
    let f = frame(cmd, body);
    let mut out = [0u8; 65];
    out[1..].copy_from_slice(&f);
    dev.flush();
    if !dev.write(&out) {
        return None;
    }
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(wait_ms);
    let mut rx = [0u8; 65];
    loop {
        let left = deadline.saturating_duration_since(std::time::Instant::now()).as_millis() as u32;
        if left == 0 {
            return None;
        }
        let Some(n) = dev.read_timeout(&mut rx, left) else { return None };
        if want_mv {
            if let Some(mv) = parse_mv(&rx[..n]) {
                return Some(mv);
            }
        }
    }
}

/// Closes the session (85 02) whatever happens after it was opened.
struct SessionGuard<'a>(&'a HidDevice);
impl Drop for SessionGuard<'_> {
    fn drop(&mut self) {
        transact(self.0, 0x85, &[0x02], 50, false);
    }
}

fn read_mv(dev: &HidDevice) -> Option<u16> {
    let f = frame(0x84, &[0x01]);
    let mut out = [0u8; 65];
    out[1..].copy_from_slice(&f);
    dev.flush();
    if !dev.write(&out) {
        return None;
    }
    let _guard = SessionGuard(dev);
    std::thread::sleep(std::time::Duration::from_millis(50));
    transact(dev, 0x85, &[0x01], 50, false);
    transact(dev, 0x80, &SESSION_INIT, 450, false);
    transact(dev, 0x80, &pa(CAT_WIRELESS, 2, 1, &[GET_MODE]), 120, false);
    transact(dev, 0x80, &pa(CAT_WIRELESS, 3, 2, &[SET_MODE, 0x01]), 120, false);
    transact(dev, 0x80, &pa(CAT_WIRELESS, 4, 1, &[GET_MODE]), 120, false);
    transact(dev, 0x80, &pa(CAT_OTA, 5, 1, &[0x00, 0x31]), 250, true)
}

pub fn poll(infos: &[HidInfo]) -> Option<DeviceStatus> {
    let info = infos.iter().find(|i| {
        i.vid == VID && i.pid == PID && i.usage_page == 0xFF00 && i.usage == 0x0001 && i.in_len == 65 && i.out_len == 65
    })?;
    let dev = HidDevice::open(info)?;
    let mv = read_mv(&dev);
    Some(DeviceStatus {
        key: "g435".into(),
        kind: DeviceKind::Headset,
        name: "Logitech G435".into(),
        percent: mv.map(percent),
        millivolts: mv,
        charging: false,
        connected: mv.is_some(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc_xmodem_check() {
        assert_eq!(crc16(b"123456789"), 0x31C3);
    }

    #[test]
    fn frame_layout() {
        let f = frame(0x84, &[1]);
        assert_eq!((f[0], f[1], f[4]), (0x84, 1, 1));
        let mut zeroed = f;
        zeroed[2] = 0;
        zeroed[3] = 0;
        assert_eq!(u16::from_le_bytes([f[2], f[3]]), crc16(&zeroed));
        assert_ne!(u16::from_le_bytes([f[2], f[3]]), 0);
    }

    #[test]
    fn pa_layout() {
        assert_eq!(pa(0x0E, 3, 2, &[0xE1, 0x01]), vec![0x50, 0x41, 0x0E, 3, 2, 0xE1, 0x01]);
    }

    #[test]
    fn parse_finds_pattern() {
        assert_eq!(parse_mv(&[9, 9, 0, 7, 0, 6, 5, 0, 0x68, 0x10]), Some(4200));
    }

    #[test]
    fn parse_rejects_implausible() {
        assert_eq!(parse_mv(&[0, 7, 0, 6, 5, 0, 0xE8, 0x03]), None); // 1000 mV
    }

    #[test]
    fn percent_points() {
        assert_eq!(percent(4000), 80);
        assert_eq!(percent(3625), 20);
        assert_eq!(percent(3300), 0);
    }
}
