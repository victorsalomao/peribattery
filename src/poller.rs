//! Background poller: one low-priority thread, sleeps on an event between due reads,
//! posts results to the UI only when something changed.

use std::time::Duration;

use crate::model::{DeviceKind, DeviceStatus};

pub const WM_APP_STATUS: u32 = windows_sys::Win32::UI::WindowsAndMessaging::WM_APP + 1;

/// How often a source is read. Fullscreen game ⇒ ×5.
pub fn interval_for(key: &str, gaming: bool) -> Duration {
    let base = if key == "g435" { 120 } else { 60 };
    Duration::from_secs(if gaming { base * 5 } else { base })
}

fn is_controller(key: &str) -> bool {
    key == "ds" || key.starts_with("pad:")
}

/// Folds a source's fresh reading into its previous one: a device that went silent keeps its
/// last level, dimmed; controllers simply disappear.
pub fn merge(prev: &[DeviceStatus], fresh: Vec<DeviceStatus>) -> Vec<DeviceStatus> {
    let mut out: Vec<DeviceStatus> = fresh
        .into_iter()
        .map(|mut d| {
            if !d.connected {
                if let Some(p) = prev.iter().find(|p| p.key == d.key) {
                    d.percent = d.percent.or(p.percent);
                    d.millivolts = d.millivolts.or(p.millivolts);
                }
                d.charging = false;
            }
            d
        })
        .collect();
    for p in prev {
        if !is_controller(&p.key) && !out.iter().any(|d| d.key == p.key) {
            let mut gone = p.clone();
            gone.connected = false;
            gone.charging = false;
            out.push(gone);
        }
    }
    out
}

fn kind_rank(k: DeviceKind) -> u8 {
    match k {
        DeviceKind::Gamepad => 0,
        DeviceKind::Headset => 1,
        DeviceKind::Keyboard => 2,
        DeviceKind::Mouse => 3,
    }
}

/// Display order left→right: controllers, headset, keyboard, mouse.
pub fn sorted(mut v: Vec<DeviceStatus>) -> Vec<DeviceStatus> {
    v.sort_by(|a, b| kind_rank(a.kind).cmp(&kind_rank(b.kind)).then_with(|| a.key.cmp(&b.key)));
    v
}

fn gaming() -> bool {
    use windows_sys::Win32::UI::Shell::*;
    let mut st: QUERY_USER_NOTIFICATION_STATE = 0;
    unsafe { SHQueryUserNotificationState(&mut st) == 0 && (st == QUNS_RUNNING_D3D_FULL_SCREEN || st == QUNS_BUSY) }
}

/// Wakes the poller for an immediate full read (device plugged, resume, menu "refresh").
#[derive(Clone, Copy)]
pub struct PollerHandle {
    event: isize,
}

impl PollerHandle {
    pub fn wake(&self) {
        unsafe {
            windows_sys::Win32::System::Threading::SetEvent(self.event as _);
        }
    }
}

/// Starts the poller thread; results go to `ui` as `WM_APP_STATUS` with a `Box<Vec<DeviceStatus>>`
/// in lParam (the receiver takes ownership).
pub fn start(ui: isize) -> PollerHandle {
    use crate::devices::Source;
    use std::time::Instant;
    use windows_sys::Win32::System::Threading::*;
    use windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW;

    let event = unsafe { CreateEventW(std::ptr::null(), 0, 0, std::ptr::null()) } as isize;
    std::thread::Builder::new()
        .name("poller".into())
        .stack_size(256 * 1024)
        .spawn(move || unsafe {
            SetThreadPriority(GetCurrentThread(), THREAD_MODE_BACKGROUND_BEGIN);
            let mut last: [Vec<DeviceStatus>; 5] = Default::default();
            let mut due = [Instant::now(); 5];
            let mut sent: Vec<DeviceStatus> = Vec::new();
            let mut last_forced: Option<Instant> = None;
            loop {
                let now = Instant::now();
                if due.iter().any(|d| *d <= now) {
                    let infos = crate::hid::enumerate(&crate::VIDS);
                    let g = gaming();
                    for (i, src) in Source::ALL.iter().enumerate() {
                        if due[i] <= now {
                            last[i] = merge(&last[i], src.poll(&infos));
                            due[i] = Instant::now() + interval_for(src.key(), g);
                        }
                    }
                    let all = sorted(last.iter().flatten().cloned().collect());
                    if all != sent {
                        sent = all.clone();
                        let ptr = Box::into_raw(Box::new(all));
                        if PostMessageW(ui as _, WM_APP_STATUS, 0, ptr as isize) == 0 {
                            drop(Box::from_raw(ptr));
                        }
                    }
                }
                let next = due.iter().min().copied().unwrap_or(now);
                let ms = next.saturating_duration_since(Instant::now()).as_millis().min(u32::MAX as u128) as u32;
                if event == 0 {
                    std::thread::sleep(Duration::from_millis(ms as u64)); // no event: never spin
                } else if WaitForSingleObject(event as _, ms) == 0 {
                    // Woken (device change / resume / refresh): read everything now, but at most
                    // once a minute while a game is fullscreen (Steam pads, BT endpoints churn).
                    let now = Instant::now();
                    if wake_allowed(gaming(), last_forced, now) {
                        due = [now; 5];
                        last_forced = Some(now);
                    }
                }
            }
        })
        .ok();
    PollerHandle { event }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dev(key: &str, kind: DeviceKind, pct: Option<u8>, connected: bool) -> DeviceStatus {
        DeviceStatus { key: key.into(), kind, name: key.into(), percent: pct, millivolts: pct.map(|_| 3900), charging: false, connected }
    }

    #[test]
    fn interval_normal_and_gaming() {
        assert_eq!(interval_for("zowie", false), Duration::from_secs(60));
        assert_eq!(interval_for("zowie", true), Duration::from_secs(300));
        assert_eq!(interval_for("g435", false), Duration::from_secs(120));
        assert_eq!(interval_for("g435", true), Duration::from_secs(600));
        assert_eq!(interval_for("pad", false), Duration::from_secs(60));
        assert_eq!(interval_for("pad", true), Duration::from_secs(300));
    }

    #[test]
    fn merge_keeps_last_known_when_off() {
        let prev = vec![dev("g435", DeviceKind::Headset, Some(31), true)];
        let out = merge(&prev, vec![dev("g435", DeviceKind::Headset, None, false)]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].percent, Some(31));
        assert!(!out[0].connected);
    }

    #[test]
    fn merge_keeps_vanished_device_dimmed() {
        // Receiver unplugged: the source returns nothing at all.
        let prev = vec![dev("zowie", DeviceKind::Mouse, Some(10), true)];
        let out = merge(&prev, vec![]);
        assert_eq!(out.len(), 1);
        assert_eq!((out[0].percent, out[0].connected), (Some(10), false));
    }

    #[test]
    fn merge_new_device_appears() {
        let out = merge(&[], vec![dev("logi:c547:1", DeviceKind::Keyboard, Some(92), true)]);
        assert_eq!(out.len(), 1);
        assert!(out[0].connected);
    }

    #[test]
    fn merge_dualsense_absent_removed() {
        let prev = vec![dev("ds", DeviceKind::Gamepad, Some(75), true), dev("pad:0", DeviceKind::Gamepad, None, true)];
        assert!(merge(&prev, vec![]).is_empty());
    }
}

#[cfg(test)]
mod wake_tests {
    use super::*;
    use std::time::Instant;

    #[test]
    fn wake_always_allowed_outside_games() {
        let now = Instant::now();
        assert!(wake_allowed(false, Some(now), now));
    }

    #[test]
    fn wake_rate_limited_while_gaming() {
        let now = Instant::now();
        assert!(wake_allowed(true, None, now));
        assert!(!wake_allowed(true, Some(now), now + Duration::from_secs(10)));
        assert!(wake_allowed(true, Some(now), now + Duration::from_secs(61)));
    }
}

/// A forced (device-change / resume) read while gaming is allowed at most once a minute.
pub fn wake_allowed(gaming: bool, last_forced: Option<std::time::Instant>, now: std::time::Instant) -> bool {
    !gaming || last_forced.map_or(true, |t| now.saturating_duration_since(t) >= Duration::from_secs(60))
}
