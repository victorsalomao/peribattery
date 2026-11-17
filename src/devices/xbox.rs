//! Xbox-style controllers through Windows.Gaming.Input (exact %, mWh), XInput as fallback.
//! 8BitDo (VID 2DC8) dongles fake a wired Xbox 360 pad: WGI always says 1000/1000, so for
//! them we show "connected, level unknown" instead of a fake 100%.
//! WinRT is only touched once a candidate controller exists, so idle sessions never load it.

use crate::model::{DeviceKind, DeviceStatus};

pub const VID_8BITDO: u16 = 0x2DC8;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Status {
    NotPresent,
    Discharging,
    Idle,
    Charging,
}

#[derive(Debug, PartialEq)]
pub enum Reading {
    Percent { pct: u8, charging: bool },
    Unknown,
    Absent,
}

pub fn pct_from_mwh(remaining: u32, full: u32) -> Option<u8> {
    if full == 0 {
        return None;
    }
    Some(((remaining as u64 * 100 + full as u64 / 2) / full as u64).min(100) as u8)
}

/// Decides what to show for one controller. `wgi` = (remaining mWh, full mWh, status).
pub fn classify(vid: u16, wgi: Option<(u32, u32, Status)>, xinput_wired: bool) -> Reading {
    if vid == VID_8BITDO {
        return Reading::Unknown; // dongle never reports a real level
    }
    match wgi {
        Some((_, _, Status::NotPresent)) => Reading::Absent,
        Some((rem, full, st)) => match pct_from_mwh(rem, full) {
            Some(pct) => Reading::Percent { pct, charging: st == Status::Charging },
            None => Reading::Absent,
        },
        // No WGI entry: a wired/unknown XInput pad has nothing to show either way.
        None => {
            let _ = xinput_wired;
            Reading::Absent
        }
    }
}

thread_local! {
    static COM_READY: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

fn ensure_com() {
    COM_READY.with(|c| {
        if !c.get() {
            unsafe {
                windows_sys::Win32::System::Com::CoInitializeEx(std::ptr::null(), windows_sys::Win32::System::Com::COINIT_MULTITHREADED as u32);
            }
            c.set(true);
        }
    });
}

/// XInput battery for one slot: `Some(None)` = wired, `Some(Some(pct))` = wireless level.
fn xinput_slot(slot: u32) -> Option<Option<u8>> {
    use windows_sys::Win32::UI::Input::XboxController::*;
    unsafe {
        let mut st: XINPUT_STATE = std::mem::zeroed();
        if XInputGetState(slot, &mut st) != 0 {
            return None;
        }
        let mut b: XINPUT_BATTERY_INFORMATION = std::mem::zeroed();
        if XInputGetBatteryInformation(slot, BATTERY_DEVTYPE_GAMEPAD, &mut b) != 0 {
            return Some(None);
        }
        if b.BatteryType == BATTERY_TYPE_WIRED || b.BatteryType == BATTERY_TYPE_DISCONNECTED || b.BatteryType == BATTERY_TYPE_UNKNOWN {
            return Some(None);
        }
        Some(Some(match b.BatteryLevel {
            BATTERY_LEVEL_EMPTY => 5,
            BATTERY_LEVEL_LOW => 30,
            BATTERY_LEVEL_MEDIUM => 65,
            _ => 100,
        }))
    }
}

fn wgi_pads() -> windows::core::Result<Vec<(u16, Option<(u32, u32, Status)>)>> {
    use windows::Gaming::Input::{Gamepad, RawGameController};
    use windows::System::Power::BatteryStatus;
    let pads = Gamepad::Gamepads()?;
    let mut out = Vec::new();
    for i in 0..pads.Size()? {
        let pad = pads.GetAt(i)?;
        let vid = RawGameController::FromGameController(&pad).and_then(|r| r.HardwareVendorId()).unwrap_or(0);
        let rep = pad.TryGetBatteryReport()?;
        let status = match rep.Status()? {
            BatteryStatus::NotPresent => Status::NotPresent,
            BatteryStatus::Charging => Status::Charging,
            BatteryStatus::Idle => Status::Idle,
            _ => Status::Discharging,
        };
        let rem = rep.RemainingCapacityInMilliwattHours().and_then(|r| r.Value()).unwrap_or(0).max(0) as u32;
        let full = rep.FullChargeCapacityInMilliwattHours().and_then(|r| r.Value()).unwrap_or(0).max(0) as u32;
        out.push((vid, Some((rem, full, status))));
    }
    Ok(out)
}

fn status(n: usize, vid: u16, r: Reading) -> Option<DeviceStatus> {
    let (percent, charging) = match r {
        Reading::Percent { pct, charging } => (Some(pct), charging),
        Reading::Unknown => (None, false),
        Reading::Absent => return None,
    };
    Some(DeviceStatus {
        key: format!("pad:{n}"),
        kind: DeviceKind::Gamepad,
        name: if vid == VID_8BITDO { "8BitDo".into() } else { "Controle Xbox".into() },
        percent,
        millivolts: None,
        charging,
        connected: true,
        estimate: None,
    })
}

/// `candidate`: the HID scan saw a Microsoft/8BitDo device. WinRT is loaded only then.
pub fn poll(candidate: bool) -> Vec<DeviceStatus> {
    let now = std::time::Instant::now();
    if !xinput_due(candidate, LAST_XINPUT.with(|c| c.get()), now) {
        return LAST_PADS.with(|p| p.borrow().clone()); // not probed this time: keep what we saw
    }
    LAST_XINPUT.with(|c| c.set(Some(now)));
    let xinput: Vec<(u32, Option<u8>)> = (0..4).filter_map(|s| xinput_slot(s).map(|b| (s, b))).collect();
    if !candidate && xinput.is_empty() {
        return Vec::new();
    }
    ensure_com();
    let pads = wgi_pads().unwrap_or_default();
    let mut out: Vec<DeviceStatus> = pads.iter().enumerate()
        .filter_map(|(n, &(vid, wgi))| status(n, vid, classify(vid, wgi, false)))
        .collect();
    if pads.is_empty() {
        // WGI not populated yet (fills in after first use) or unavailable: XInput levels.
        for (slot, level) in xinput {
            if let Some(pct) = level {
                out.extend(status(slot as usize, 0, Reading::Percent { pct, charging: false }));
            }
        }
    }
    LAST_PADS.with(|p| *p.borrow_mut() = out.clone());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mwh_to_pct() {
        assert_eq!(pct_from_mwh(600, 1000), Some(60));
        assert_eq!(pct_from_mwh(0, 0), None);
        assert_eq!(pct_from_mwh(1200, 1000), Some(100));
    }

    #[test]
    fn eightbitdo_never_fake_100() {
        assert_eq!(classify(0x2DC8, Some((1000, 1000, Status::Discharging)), true), Reading::Unknown);
    }

    #[test]
    fn xbox_charging() {
        assert_eq!(classify(0x045E, Some((500, 1000, Status::Charging)), false), Reading::Percent { pct: 50, charging: true });
    }

    #[test]
    fn wired_non_8bitdo_hidden() {
        // Wired pads and Steam Input's virtual pads have no battery: do not show ghosts.
        assert_eq!(classify(0x045E, Some((0, 0, Status::NotPresent)), true), Reading::Absent);
    }
}

/// Without a Microsoft/8BitDo HID device, probe XInput only once a minute (pads behind the Xbox
/// Wireless Adapter may not show up in HID); with one, every poll.
pub fn xinput_due(candidate: bool, last: Option<std::time::Instant>, now: std::time::Instant) -> bool {
    candidate || last.map_or(true, |t| now.saturating_duration_since(t) >= std::time::Duration::from_secs(60))
}

thread_local! {
    static LAST_XINPUT: std::cell::Cell<Option<std::time::Instant>> = const { std::cell::Cell::new(None) };
    static LAST_PADS: std::cell::RefCell<Vec<DeviceStatus>> = const { std::cell::RefCell::new(Vec::new()) };
}

#[cfg(test)]
mod due_tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn xinput_probe_is_rare_without_a_candidate() {
        let t = Instant::now();
        assert!(xinput_due(true, Some(t), t));
        assert!(xinput_due(false, None, t));
        assert!(!xinput_due(false, Some(t), t + Duration::from_secs(5)));
        assert!(xinput_due(false, Some(t), t + Duration::from_secs(60)));
    }
}
