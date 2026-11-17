//! Logitech HID++ 2.0 through a LIGHTSPEED/Unifying/Bolt receiver (G915 TKL and friends).
//! Long reports (0x11, 20 bytes) on the receiver's FF00:0002 collection.
//! Only read functions are sent: Root.getFeature, battery getters, DeviceName getters.

use crate::hid::{HidDevice, HidInfo};
use crate::model::{interpolate, DeviceKind, DeviceStatus};

const VID: u16 = 0x046D;
const RECEIVERS: [u16; 6] = [0xC539, 0xC53A, 0xC541, 0xC545, 0xC547, 0xC54D];
const SW_ID: u8 = 0x0A;
/// Paired devices answer in < 50 ms; empty receiver slots never answer on the long collection.
const REQUEST_TIMEOUT_MS: u64 = 400;
const UNIFIED_BATTERY: u16 = 0x1004;
const BATTERY_STATUS: u16 = 0x1000;
const BATTERY_VOLTAGE: u16 = 0x1001;
const DEVICE_NAME: u16 = 0x0005;
const LIION_V: [u16; 11] = [4186, 4067, 3989, 3922, 3859, 3811, 3778, 3751, 3717, 3654, 3500];
const LIION_P: [u8; 11] = [100, 90, 80, 70, 60, 50, 40, 30, 20, 10, 0];

#[derive(Debug, PartialEq)]
pub enum HidppErr {
    Timeout,
    Unsupported,
    Io,
}

/// One request/response exchange; returns the 16 parameter bytes of the reply.
pub trait Link {
    fn request(&mut self, idx: u8, feat: u8, func: u8, params: &[u8]) -> Result<[u8; 16], HidppErr>;
}

#[derive(Debug, PartialEq)]
pub enum Battery {
    Percent { pct: u8, charging: bool },
    Millivolts { mv: u16, charging: bool },
    None,
}

/// Root.getFeature: index of `feature` on device `idx`, `None` when absent.
pub fn feature_index(l: &mut dyn Link, idx: u8, feature: u16) -> Result<Option<u8>, HidppErr> {
    let r = l.request(idx, 0x00, 0x0, &feature.to_be_bytes())?;
    Ok((r[0] != 0).then_some(r[0]))
}

#[cfg(test)]
pub fn battery(l: &mut dyn Link, idx: u8) -> Result<Battery, HidppErr> {
    battery_cached(l, idx, &mut None)
}

/// Matches a received long report against a request. `None` = not ours, keep reading.
/// HID++ 2.0 error: [11 idx FF feat func_sw err]. HID++ 1.0 receiver error: [10 idx 8F ...]
/// (device unreachable / slot empty).
fn match_reply(rx: &[u8], idx: u8, feat: u8, func_sw: u8) -> Option<Result<[u8; 16], HidppErr>> {
    if rx.len() < 7 || rx[1] != idx {
        return None;
    }
    if rx[2] == 0x8F && rx[3] == feat && rx[4] == func_sw {
        return Some(Err(HidppErr::Timeout));
    }
    if rx[2] == 0xFF && rx[3] == feat && rx[4] == func_sw {
        return Some(Err(HidppErr::Unsupported));
    }
    if rx[0] == 0x11 && rx[2] == feat && rx[3] == func_sw {
        let mut p = [0u8; 16];
        let n = (rx.len() - 4).min(16);
        p[..n].copy_from_slice(&rx[4..4 + n]);
        return Some(Ok(p));
    }
    None
}

struct DevLink {
    dev: HidDevice,
}

impl Link for DevLink {
    fn request(&mut self, idx: u8, feat: u8, func: u8, params: &[u8]) -> Result<[u8; 16], HidppErr> {
        let func_sw = (func << 4) | SW_ID;
        let mut tx = [0u8; 20];
        tx[..4].copy_from_slice(&[0x11, idx, feat, func_sw]);
        let n = params.len().min(16);
        tx[4..4 + n].copy_from_slice(&params[..n]);
        self.dev.flush();
        if !self.dev.write(&tx) {
            return Err(HidppErr::Io);
        }
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(REQUEST_TIMEOUT_MS);
        let mut rx = [0u8; 20];
        loop {
            let left = deadline.saturating_duration_since(std::time::Instant::now()).as_millis() as u32;
            if left == 0 {
                return Err(HidppErr::Timeout);
            }
            let Some(got) = self.dev.read_timeout(&mut rx, left) else { return Err(HidppErr::Timeout) };
            if let Some(r) = match_reply(&rx[..got], idx, feat, func_sw) {
                return r;
            }
        }
    }
}

fn device_name(l: &mut dyn Link, idx: u8) -> Option<(String, u8)> {
    let f = feature_index(l, idx, DEVICE_NAME).ok()??;
    let count = l.request(idx, f, 0x0, &[]).ok()?[0] as usize;
    let mut name = Vec::with_capacity(count.min(48));
    while name.len() < count.min(48) {
        let r = l.request(idx, f, 0x1, &[name.len() as u8]).ok()?;
        let take = (count - name.len()).min(16);
        name.extend_from_slice(&r[..take]);
    }
    let kind = l.request(idx, f, 0x2, &[]).map(|r| r[0]).unwrap_or(0);
    let s = String::from_utf8_lossy(&name).trim_end_matches('\0').trim().to_string();
    Some((s, kind))
}

fn kind_of(t: u8) -> DeviceKind {
    match t {
        3..=5 => DeviceKind::Mouse,
        8 => DeviceKind::Headset,
        _ => DeviceKind::Keyboard,
    }
}

pub fn poll(infos: &[HidInfo]) -> Vec<DeviceStatus> {
    let mut out = Vec::new();
    for info in infos.iter().filter(|i| {
        i.vid == VID && RECEIVERS.contains(&i.pid) && i.usage_page == 0xFF00 && i.usage == 0x0002 && i.out_len == 20
    }) {
        let Some(dev) = HidDevice::open(info) else { continue };
        let mut link = DevLink { dev };
        let (slots, full) = RECEIVERS_STATE.with(|s| {
            let mut s = s.borrow_mut();
            let st = s.entry(info.pid).or_default();
            let full = full_scan_due(st.last_full, st.known.is_empty(), std::time::Instant::now());
            if full {
                st.last_full = Some(std::time::Instant::now());
            }
            (slots_to_probe(&st.known, full), full)
        });
        let mut answered = Vec::new();
        for idx in slots {
            let mut fc = RECEIVERS_STATE.with(|s| s.borrow().get(&info.pid).and_then(|st| st.features.get(&idx).copied()));
            let res = battery_cached(&mut link, idx, &mut fc);
            RECEIVERS_STATE.with(|s| {
                let mut s = s.borrow_mut();
                let st = s.entry(info.pid).or_default();
                match fc {
                    Some(v) => st.features.insert(idx, v),
                    None => st.features.remove(&idx),
                };
            });
            let Ok(bat) = res else { continue }; // empty slot or asleep
            answered.push(idx);
            let (pct, mv, charging) = match bat {
                Battery::Percent { pct, charging } => (pct, None, charging),
                Battery::Millivolts { mv, charging } => (interpolate(mv, &LIION_V, &LIION_P), Some(mv), charging),
                Battery::None => continue,
            };
            let cached = RECEIVERS_STATE.with(|s| s.borrow().get(&info.pid).and_then(|st| st.names.get(&idx).cloned()));
            let (name, t) = match cached {
                Some(n) => n,
                None => {
                    let n = device_name(&mut link, idx).map(|(n, t)| (pretty_name(&n), t)).unwrap_or_else(|| ("Logitech".into(), 0));
                    RECEIVERS_STATE.with(|s| s.borrow_mut().entry(info.pid).or_default().names.insert(idx, n.clone()));
                    n
                }
            };
            out.push(DeviceStatus {
                key: format!("logi:{:04x}:{idx}", info.pid),
                kind: kind_of(t),
                name,
                percent: Some(pct),
                millivolts: mv,
                charging,
                connected: true,
                estimate: None,
            });
        }
        RECEIVERS_STATE.with(|s| {
            let mut s = s.borrow_mut();
            let st = s.entry(info.pid).or_default();
            if full {
                st.known = merge_known(&st.known, &answered);
            }
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// Root (feat 0) answers from `features`; other (feat, func) pairs from `replies`.
    struct FakeLink {
        features: HashMap<u16, u8>,
        replies: HashMap<(u8, u8), Result<[u8; 16], HidppErr>>,
    }

    impl FakeLink {
        fn new(features: &[(u16, u8)]) -> Self {
            FakeLink { features: features.iter().copied().collect(), replies: HashMap::new() }
        }
        fn reply(mut self, feat: u8, func: u8, bytes: &[u8]) -> Self {
            let mut r = [0u8; 16];
            r[..bytes.len()].copy_from_slice(bytes);
            self.replies.insert((feat, func), Ok(r));
            self
        }
        fn fail(mut self, feat: u8, func: u8, e: HidppErr) -> Self {
            self.replies.insert((feat, func), Err(e));
            self
        }
    }

    impl Link for FakeLink {
        fn request(&mut self, _idx: u8, feat: u8, func: u8, params: &[u8]) -> Result<[u8; 16], HidppErr> {
            if feat == 0 && func == 0 {
                if let Some(Err(_)) = self.replies.get(&(0, 0)) {
                    return Err(HidppErr::Timeout);
                }
                let id = u16::from_be_bytes([params[0], params[1]]);
                let mut r = [0u8; 16];
                r[0] = *self.features.get(&id).unwrap_or(&0);
                return Ok(r);
            }
            match self.replies.remove(&(feat, func)) {
                Some(r) => r,
                None => Err(HidppErr::Unsupported),
            }
        }
    }

    #[test]
    fn unified_battery_wins() {
        let mut l = FakeLink::new(&[(0x1004, 5), (0x1000, 6)]).reply(5, 1, &[64, 0, 0]).reply(6, 0, &[10, 0, 0]);
        assert_eq!(battery(&mut l, 1), Ok(Battery::Percent { pct: 64, charging: false }));
    }

    #[test]
    fn battery_status_charging_4() {
        let mut l = FakeLink::new(&[(0x1000, 6)]).reply(6, 0, &[50, 0, 4]);
        assert_eq!(battery(&mut l, 1), Ok(Battery::Percent { pct: 50, charging: true }));
    }

    #[test]
    fn voltage_be_and_charging_bit() {
        let mut l = FakeLink::new(&[(0x1001, 7)]).reply(7, 0, &[0x0F, 0xA0, 0x80]);
        assert_eq!(battery(&mut l, 1), Ok(Battery::Millivolts { mv: 4000, charging: true }));
    }

    #[test]
    fn absent_feature_root_returns_zero() {
        let mut l = FakeLink::new(&[]);
        assert_eq!(feature_index(&mut l, 1, 0x1004), Ok(None));
        assert_eq!(battery(&mut l, 1), Ok(Battery::None));
    }

    #[test]
    fn timeout_propagates() {
        let mut l = FakeLink::new(&[]).fail(0, 0, HidppErr::Timeout);
        assert_eq!(battery(&mut l, 1), Err(HidppErr::Timeout));
    }

    #[test]
    fn reply_matching() {
        // [0x11, idx, feat, func<<4|sw, params...]
        let mut rx = [0u8; 20];
        rx[..5].copy_from_slice(&[0x11, 1, 5, 0x1A, 64]);
        assert_eq!(match_reply(&rx, 1, 5, 0x1A), Some(Ok({ let mut p = [0u8; 16]; p[0] = 64; p })));
        rx[..5].copy_from_slice(&[0x11, 1, 0xFF, 5, 0x1A]);
        assert_eq!(match_reply(&rx, 1, 5, 0x1A), Some(Err(HidppErr::Unsupported)));
        rx[..5].copy_from_slice(&[0x11, 2, 5, 0x1A, 0]);
        assert_eq!(match_reply(&rx, 1, 5, 0x1A), None);
    }
}

/// Receiver slots to query: all six on a full scan (or when none is known yet), otherwise only
/// the slots that answered last time — empty slots each cost a full timeout.
pub fn slots_to_probe(known: &[u8], full: bool) -> Vec<u8> {
    if full {
        (1..=6).collect()
    } else {
        known.to_vec()
    }
}

const FULL_SCAN_EVERY: std::time::Duration = std::time::Duration::from_secs(600);

/// Per receiver: slots that answered, when the last full scan ran, and cached names.
#[derive(Default)]
struct ReceiverState {
    known: Vec<u8>,
    last_full: Option<std::time::Instant>,
    names: std::collections::HashMap<u8, (String, u8)>,
    features: std::collections::HashMap<u8, (u16, u8)>,
}

thread_local! {
    static RECEIVERS_STATE: std::cell::RefCell<std::collections::HashMap<u16, ReceiverState>> = Default::default();
}

#[cfg(test)]
mod slot_tests {
    use super::*;

    #[test]
    fn probes_known_slots_between_full_scans() {
        assert_eq!(slots_to_probe(&[1], false), vec![1]);
        assert_eq!(slots_to_probe(&[1], true), vec![1, 2, 3, 4, 5, 6]);
        assert_eq!(slots_to_probe(&[], false), Vec::<u8>::new()); // nothing known: wait for the next full scan
    }
}

/// "G915 WIRELESS RGB MECHANICAL GAMING KEYBOARD" → "Logitech G915".
pub fn pretty_name(raw: &str) -> String {
    const FILLER: [&str; 8] = ["WIRELESS", "RGB", "MECHANICAL", "GAMING", "KEYBOARD", "MOUSE", "HEADSET", "LIGHTSPEED"];
    let model: Vec<&str> = raw.split_whitespace().filter(|w| !FILLER.contains(&w.to_ascii_uppercase().as_str())).collect();
    if model.is_empty() {
        "Logitech".into()
    } else {
        format!("Logitech {}", model.join(" "))
    }
}

#[cfg(test)]
mod name_tests {
    use super::*;

    #[test]
    fn pretty_names() {
        assert_eq!(pretty_name("G915 WIRELESS RGB MECHANICAL GAMING KEYBOARD"), "Logitech G915");
        assert_eq!(pretty_name("PRO X SUPERLIGHT"), "Logitech PRO X SUPERLIGHT");
        assert_eq!(pretty_name(""), "Logitech");
    }
}

/// Battery read that remembers which feature/index answered, so later reads skip the Root lookup.
pub fn battery_cached(l: &mut dyn Link, idx: u8, cache: &mut Option<(u16, u8)>) -> Result<Battery, HidppErr> {
    if let Some((feature, index)) = *cache {
        match read_feature(l, idx, feature, index) {
            Err(HidppErr::Unsupported) => *cache = None, // device changed: look up again
            r => return r,
        }
    }
    for feature in [UNIFIED_BATTERY, BATTERY_STATUS, BATTERY_VOLTAGE] {
        if let Some(index) = feature_index(l, idx, feature)? {
            *cache = Some((feature, index));
            return read_feature(l, idx, feature, index);
        }
    }
    Ok(Battery::None)
}

fn read_feature(l: &mut dyn Link, idx: u8, feature: u16, f: u8) -> Result<Battery, HidppErr> {
    Ok(match feature {
        UNIFIED_BATTERY => {
            let r = l.request(idx, f, 0x1, &[])?; // get_status
            Battery::Percent { pct: r[0].min(100), charging: matches!(r[2], 1 | 2) }
        }
        BATTERY_STATUS => {
            let r = l.request(idx, f, 0x0, &[])?;
            Battery::Percent { pct: r[0].min(100), charging: matches!(r[2], 1 | 2 | 4) }
        }
        _ => {
            let r = l.request(idx, f, 0x0, &[])?;
            Battery::Millivolts { mv: u16::from_be_bytes([r[0], r[1]]), charging: r[2] & 0x80 != 0 }
        }
    })
}

#[cfg(test)]
mod cache_tests {
    use super::*;

    struct Counting {
        calls: u32,
    }
    impl Link for Counting {
        fn request(&mut self, _idx: u8, feat: u8, func: u8, params: &[u8]) -> Result<[u8; 16], HidppErr> {
            self.calls += 1;
            let mut r = [0u8; 16];
            if feat == 0 && func == 0 {
                // only 0x1001 present, at index 7
                r[0] = if u16::from_be_bytes([params[0], params[1]]) == 0x1001 { 7 } else { 0 };
            } else if feat == 7 {
                r[..3].copy_from_slice(&[0x0F, 0xFB, 0x00]); // 4091 mV
            }
            Ok(r)
        }
    }

    #[test]
    fn second_read_is_a_single_request() {
        let mut l = Counting { calls: 0 };
        let mut cache = None;
        assert_eq!(battery_cached(&mut l, 1, &mut cache), Ok(Battery::Millivolts { mv: 4091, charging: false }));
        let first = l.calls; // 3 root lookups + 1 read
        assert_eq!(cache, Some((0x1001, 7)));
        assert_eq!(battery_cached(&mut l, 1, &mut cache), Ok(Battery::Millivolts { mv: 4091, charging: false }));
        assert_eq!(l.calls - first, 1);
    }
}

/// Full scan of all six slots: at start, then every 10 min — or every minute while no slot is
/// known yet (e.g. the keyboard was asleep at startup).
pub fn full_scan_due(last_full: Option<std::time::Instant>, known_empty: bool, now: std::time::Instant) -> bool {
    let every = if known_empty { std::time::Duration::from_secs(60) } else { FULL_SCAN_EVERY };
    last_full.map_or(true, |t| now.saturating_duration_since(t) >= every)
}

/// Slots to remember after a full scan: those that answered plus those known before (a sleeping
/// keyboard keeps its slot).
pub fn merge_known(old: &[u8], answered: &[u8]) -> Vec<u8> {
    let mut v: Vec<u8> = old.iter().chain(answered).copied().collect();
    v.sort_unstable();
    v.dedup();
    v
}

#[cfg(test)]
mod scan_tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn full_scan_schedule() {
        let t = Instant::now();
        assert!(full_scan_due(None, true, t));
        assert!(!full_scan_due(Some(t), true, t + Duration::from_secs(30)));
        assert!(full_scan_due(Some(t), true, t + Duration::from_secs(60)));
        assert!(!full_scan_due(Some(t), false, t + Duration::from_secs(300)));
        assert!(full_scan_due(Some(t), false, t + Duration::from_secs(600)));
    }

    #[test]
    fn sleeping_slot_stays_known() {
        assert_eq!(merge_known(&[1], &[]), vec![1]);
        assert_eq!(merge_known(&[1], &[2]), vec![1, 2]);
        assert_eq!(merge_known(&[], &[3, 1]), vec![1, 3]);
    }
}
