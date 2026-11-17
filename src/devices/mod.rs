//! One module per protocol. Each `poll` opens, queries and closes; nothing stays open.

pub mod dualsense;
pub mod g435;
pub mod hidpp;
pub mod xbox;
pub mod zowie;

use crate::hid;
use crate::model::DeviceStatus;

/// Polls every hardware source once (used by `--once`).
pub fn poll_all() -> Vec<DeviceStatus> {
    let infos = hid::enumerate(&crate::VIDS);
    let mut out = Vec::new();
    out.extend(zowie::poll(&infos));
    out.extend(hidpp::poll(&infos));
    out.extend(g435::poll(&infos));
    out.extend(dualsense::poll(&infos));
    let pad_candidate = infos.iter().any(|i| i.vid == 0x045E || i.vid == xbox::VID_8BITDO);
    out.extend(xbox::poll(pad_candidate));
    out
}
