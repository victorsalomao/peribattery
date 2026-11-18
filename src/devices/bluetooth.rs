//! Generic Bluetooth battery: the level Windows already shows in Settings › Bluetooth
//! (device property {104EA319-…},2). No radio traffic: Windows read it, we only look it up.

use std::mem::{size_of, zeroed};
use std::ptr::{null, null_mut};

use windows_sys::core::GUID;
use windows_sys::Win32::Devices::DeviceAndDriverInstallation::*;
use windows_sys::Win32::Devices::Properties::*;
use windows_sys::Win32::Foundation::*;


use crate::model::{DeviceKind, DeviceStatus};

const BATTERY: DEVPROPKEY = DEVPROPKEY { fmtid: GUID::from_u128(0x104ea319_6ee2_4701_bd47_8ddbf425bbe5), pid: 2 };
const IS_CONNECTED: DEVPROPKEY = DEVPROPKEY { fmtid: GUID::from_u128(0x83da6326_97a6_4088_9453_a1923f573b29), pid: 15 };
const CLASS_OF_DEVICE: DEVPROPKEY = DEVPROPKEY { fmtid: GUID::from_u128(0x2bd67d8b_8beb_48d5_87e0_6cda3428040a), pid: 10 };

/// Bluetooth Class of Device → kind. Major 0x04 audio; 0x05 peripheral (minor bits:
/// 0x10 keyboard, 0x20 pointing, low nibble 1/2 joystick/gamepad).
pub fn kind_from_cod(cod: u32) -> Option<DeviceKind> {
    let major = (cod >> 8) & 0x1F;
    let minor = (cod >> 2) & 0x3F;
    match major {
        0x04 => Some(DeviceKind::Headset),
        0x05 if minor & 0x10 != 0 => Some(DeviceKind::Keyboard),
        0x05 if minor & 0x20 != 0 => Some(DeviceKind::Mouse),
        0x05 if matches!(minor & 0x0F, 1 | 2) => Some(DeviceKind::Gamepad),
        _ => None,
    }
}

/// BLE devices carry no Class of Device: fall back to words in the name.
pub fn kind_from_name(name: &str) -> DeviceKind {
    let n = name.to_lowercase();
    let has = |words: &[&str]| words.iter().any(|w| n.contains(w));
    if has(&["controller", "gamepad", "dualsense", "dualshock", "joy-con", "8bitdo"]) {
        DeviceKind::Gamepad
    } else if has(&["keyboard", "teclado", "keychron", "keys", "k380", "k780"]) {
        DeviceKind::Keyboard
    } else if has(&["mouse", "mx master", "mx anywhere", "trackball", "mx ergo"]) {
        DeviceKind::Mouse
    } else if has(&["headset", "headphone", "fone", "buds", "airpods", "earbud", "wh-", "wf-", "jbl", "bose", "qc", "beats", "soundcore"]) {
        DeviceKind::Headset
    } else {
        DeviceKind::Other
    }
}

/// Windows' container category (also on BLE devices, which have no Class of Device).
pub fn kind_from_category(cats: &[String]) -> Option<DeviceKind> {
    cats.iter().find_map(|c| {
        let c = c.to_ascii_lowercase();
        if c.starts_with("input.gaming") {
            Some(DeviceKind::Gamepad)
        } else if c.starts_with("input.mouse") || c.starts_with("input.touchpad") {
            Some(DeviceKind::Mouse)
        } else if c.starts_with("input.keyboard") {
            Some(DeviceKind::Keyboard)
        } else if c.starts_with("audio.head") || c.starts_with("communication.headset") || c.starts_with("audio.earbuds") {
            Some(DeviceKind::Headset)
        } else {
            None
        }
    })
}

unsafe fn prop_strings(set: HDEVINFO, dev: &SP_DEVINFO_DATA, key: &DEVPROPKEY) -> Vec<String> {
    let mut buf = [0u8; 512];
    let Some((ty, size)) = prop(set, dev, key, &mut buf) else { return Vec::new() };
    if ty != (DEVPROP_TYPEMOD_LIST | DEVPROP_TYPE_STRING) {
        return Vec::new();
    }
    let w: Vec<u16> = buf[..size].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
    w.split(|&c| c == 0).filter(|s| !s.is_empty()).map(String::from_utf16_lossy).collect()
}

/// The battery property is a BYTE 0–100.
pub fn battery(ty: u32, data: &[u8]) -> Option<u8> {
    (ty == DEVPROP_TYPE_BYTE).then(|| data.first().copied()).flatten().filter(|p| *p <= 100)
}

/// "bt:" + FNV-1a of the device id: stable across runs, without the Bluetooth address.
pub fn key_for(id: &str) -> String {
    let mut h: u32 = 0x811c9dc5;
    for b in id.to_ascii_uppercase().bytes() {
        h = (h ^ b as u32).wrapping_mul(0x0100_0193);
    }
    format!("bt:{h:08x}")
}

unsafe fn prop(set: HDEVINFO, dev: &SP_DEVINFO_DATA, key: &DEVPROPKEY, buf: &mut [u8]) -> Option<(u32, usize)> {
    let (mut ty, mut size) = (0u32, 0u32);
    (SetupDiGetDevicePropertyW(set, dev, key, &mut ty, buf.as_mut_ptr(), buf.len() as u32, &mut size, 0) != 0).then_some((ty, size as usize))
}

unsafe fn prop_string(set: HDEVINFO, dev: &SP_DEVINFO_DATA, key: &DEVPROPKEY) -> Option<String> {
    let mut buf = [0u8; 512];
    let (ty, size) = prop(set, dev, key, &mut buf)?;
    if ty != DEVPROP_TYPE_STRING {
        return None;
    }
    let w: Vec<u16> = buf[..size].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).take_while(|&c| c != 0).collect();
    let s = String::from_utf16_lossy(&w).trim().to_string();
    (!s.is_empty()).then_some(s)
}

unsafe fn prop_u32(set: HDEVINFO, dev: &SP_DEVINFO_DATA, key: &DEVPROPKEY) -> Option<u32> {
    let mut buf = [0u8; 8];
    let (ty, size) = prop(set, dev, key, &mut buf)?;
    (ty == DEVPROP_TYPE_UINT32 && size == 4).then(|| u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]))
}

/// One Bluetooth device node that reports a battery level.
struct Node {
    container: [u8; 16],
    id: String,
    is_device: bool, // BTHENUM\DEV_… / BTHLE\DEV_… (not a service child)
    name: Option<String>,
    cod: Option<u32>,
    category: Vec<String>,
    connected: bool,
    battery: Option<u8>,
}

unsafe fn nodes(present_only: bool) -> Vec<Node> {
    let mut out = Vec::new();
    for enumerator in ["BTHENUM", "BTHLE", "BTHLEDEVICE"] {
        let e = wide(enumerator);
        let flags = if present_only { DIGCF_PRESENT | DIGCF_ALLCLASSES } else { DIGCF_ALLCLASSES };
        let set = SetupDiGetClassDevsW(null(), e.as_ptr(), null_mut(), flags);
        if set == INVALID_HANDLE_VALUE as HDEVINFO {
            continue;
        }
        let mut i = 0;
        loop {
            let mut dev: SP_DEVINFO_DATA = zeroed();
            dev.cbSize = size_of::<SP_DEVINFO_DATA>() as u32;
            if SetupDiEnumDeviceInfo(set, i, &mut dev) == 0 {
                break;
            }
            i += 1;
            let mut b = [0u8; 16];
            let battery = prop(set, &dev, &BATTERY, &mut b).and_then(|(ty, n)| battery(ty, &b[..n]));
            let mut idbuf = [0u16; 400];
            SetupDiGetDeviceInstanceIdW(set, &dev, idbuf.as_mut_ptr(), idbuf.len() as u32, null_mut());
            let end = idbuf.iter().position(|&c| c == 0).unwrap_or(0);
            let id = String::from_utf16_lossy(&idbuf[..end]);
            let is_device = id.to_ascii_uppercase().contains("\\DEV_");
            if battery.is_none() && !is_device {
                continue; // only device nodes (for names) and nodes with a level matter
            }
            let mut g = [0u8; 16];
            let Some((ty, 16)) = prop(set, &dev, &DEVPKEY_Device_ContainerId, &mut g) else { continue };
            if ty != DEVPROP_TYPE_GUID {
                continue;
            }
            let container = g; // raw GUID bytes: only compared for equality
            let mut c = [0u8; 4];
            let connected = prop(set, &dev, &IS_CONNECTED, &mut c).map_or(false, |(ty, _)| ty == DEVPROP_TYPE_BOOLEAN && c[0] != 0);
            out.push(Node {
                container,
                id,
                is_device,
                name: prop_string(set, &dev, &DEVPKEY_Device_FriendlyName),
                cod: prop_u32(set, &dev, &CLASS_OF_DEVICE),
                category: prop_strings(set, &dev, &DEVPKEY_DeviceContainer_Category),
                connected,
                battery,
            });
        }
        SetupDiDestroyDeviceInfoList(set);
    }
    out
}

/// Connected Bluetooth devices with a battery level Windows knows. Controllers are left to the
/// DualSense / Xbox sources (exact level, no duplicates).
pub fn poll() -> Vec<DeviceStatus> {
    // Off devices only stay (dimmed) once they were seen connected.
    devices(&unsafe { nodes(true) }).into_iter().filter(|d| d.kind != DeviceKind::Gamepad && d.connected).collect()
}

/// One entry per Bluetooth device (container) that has a battery level.
fn devices(nodes: &[Node]) -> Vec<DeviceStatus> {
    let mut out: Vec<DeviceStatus> = Vec::new();
    let mut seen: Vec<[u8; 16]> = Vec::new();
    for n in nodes.iter().filter(|n| n.battery.is_some()) {
        if seen.contains(&n.container) {
            continue;
        }
        seen.push(n.container);
        let main = nodes.iter().find(|m| m.is_device && m.container == n.container).unwrap_or(n);
        let name = main.name.clone().or_else(|| n.name.clone()).unwrap_or_else(|| "Bluetooth".into());
        let kind = main
            .cod
            .or(n.cod)
            .and_then(kind_from_cod)
            .or_else(|| kind_from_category(&main.category))
            .unwrap_or_else(|| kind_from_name(&name));
        let connected = main.connected || n.connected;
        out.push(DeviceStatus { key: key_for(&main.id), kind, name, percent: n.battery, millivolts: None, charging: false, connected, estimate: None });
    }
    out
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::DeviceKind;

    #[test]
    fn class_of_device() {
        assert_eq!(kind_from_cod(0x240404), Some(DeviceKind::Headset)); // audio, wearable headset
        assert_eq!(kind_from_cod(0x002580), Some(DeviceKind::Mouse)); // peripheral, pointing
        assert_eq!(kind_from_cod(0x002540), Some(DeviceKind::Keyboard)); // peripheral, keyboard
        assert_eq!(kind_from_cod(0x0025C0), Some(DeviceKind::Keyboard)); // keyboard + pointing combo
        assert_eq!(kind_from_cod(0x002508), Some(DeviceKind::Gamepad)); // peripheral, gamepad
        assert_eq!(kind_from_cod(0x5A020C), None); // phone
        assert_eq!(kind_from_cod(0), None);
    }

    #[test]
    fn name_keywords() {
        assert_eq!(kind_from_name("MX Master 3S"), DeviceKind::Mouse);
        assert_eq!(kind_from_name("Razer Basilisk X HyperSpeed"), DeviceKind::Other);
        assert_eq!(kind_from_name("Keychron K2"), DeviceKind::Keyboard);
        assert_eq!(kind_from_name("Galaxy Buds2 Pro"), DeviceKind::Headset);
        assert_eq!(kind_from_name("WH-1000XM4"), DeviceKind::Headset);
        assert_eq!(kind_from_name("Xbox Wireless Controller"), DeviceKind::Gamepad);
        assert_eq!(kind_from_name("Fone do João"), DeviceKind::Headset);
    }

    #[test]
    fn battery_value() {
        assert_eq!(battery(DEVPROP_TYPE_BYTE, &[0x3b]), Some(59));
        assert_eq!(battery(DEVPROP_TYPE_BYTE, &[101]), None);
        assert_eq!(battery(DEVPROP_TYPE_BYTE, &[]), None);
        assert_eq!(battery(0x12, &[50, 0]), None);
    }

    #[test]
    fn keys_are_stable_and_hide_the_address() {
        let a = key_for(r"BTHLE\DEV_3CFA0652E4FB\7&2E154715&0&3CFA0652E4FB");
        assert!(a.starts_with("bt:") && !a.to_ascii_uppercase().contains("3CFA0652E4FB"));
        assert_eq!(a, key_for(r"BTHLE\DEV_3CFA0652E4FB\7&2E154715&0&3CFA0652E4FB"));
        assert_ne!(a, key_for(r"BTHENUM\DEV_001122334455\8&1"));
    }
}

/// `--bt-probe`: every paired Bluetooth device with a battery level, connected or not.
pub fn probe() -> Vec<DeviceStatus> {
    devices(&unsafe { nodes(false) })
}

#[cfg(test)]
mod category_tests {
    use super::*;
    use crate::model::DeviceKind;

    #[test]
    fn container_category() {
        assert_eq!(kind_from_category(&["Input.Gaming".into()]), Some(DeviceKind::Gamepad));
        assert_eq!(kind_from_category(&["Input.Mouse".into()]), Some(DeviceKind::Mouse));
        assert_eq!(kind_from_category(&["Input.Keyboard".into()]), Some(DeviceKind::Keyboard));
        assert_eq!(kind_from_category(&["Audio.Headphones".into()]), Some(DeviceKind::Headset));
        assert_eq!(kind_from_category(&["Communication.Headset.Bluetooth".into()]), Some(DeviceKind::Headset));
        assert_eq!(kind_from_category(&["Phone".into()]), None);
    }
}
