//! One module per protocol. Each `poll` opens, queries and closes; nothing stays open.

pub mod bluetooth;
pub mod dualsense;
pub mod g435;
pub mod hidpp;
pub mod xbox;
pub mod zowie;

use crate::hid::{self, HidInfo};
use crate::model::DeviceStatus;

/// Independent battery sources, each polled on its own interval.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Source {
    Zowie,
    Logi,
    G435,
    Ds,
    Pad,
    Bt,
}

impl Source {
    pub const ALL: [Source; 6] = [Source::Zowie, Source::Logi, Source::G435, Source::Ds, Source::Pad, Source::Bt];

    pub fn key(self) -> &'static str {
        match self {
            Source::Zowie => "zowie",
            Source::Logi => "logi",
            Source::G435 => "g435",
            Source::Ds => "ds",
            Source::Pad => "pad",
            Source::Bt => "bt",
        }
    }

    pub fn poll(self, infos: &[HidInfo]) -> Vec<DeviceStatus> {
        match self {
            Source::Zowie => zowie::poll(infos).into_iter().collect(),
            Source::Logi => hidpp::poll(infos),
            Source::G435 => g435::poll(infos).into_iter().collect(),
            Source::Ds => dualsense::poll(infos).into_iter().collect(),
            Source::Pad => xbox::poll(infos.iter().any(|i| i.vid == 0x045E || i.vid == xbox::VID_8BITDO)),
            Source::Bt => bluetooth::poll(),
        }
    }
}

/// Polls every source once (used by `--once`).
pub fn poll_all() -> Vec<DeviceStatus> {
    let infos = hid::enumerate(&crate::VIDS);
    Source::ALL.iter().flat_map(|s| s.poll(&infos)).collect()
}
