//! Raw Win32 HID: enumeration (SetupAPI) and overlapped I/O with timeouts.
//! No hidapi: only the few calls we need, so nothing extra is loaded or allocated.

use std::mem::{size_of, zeroed};
use std::ptr::{null, null_mut};

use windows_sys::core::GUID;
use windows_sys::Win32::Devices::DeviceAndDriverInstallation::*;
use windows_sys::Win32::Devices::HumanInterfaceDevice::*;
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Storage::FileSystem::*;
use windows_sys::Win32::System::Threading::{CreateEventW, ResetEvent, WaitForSingleObject};
use windows_sys::Win32::System::IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED};

#[derive(Clone, Debug)]
pub struct HidInfo {
    pub path: Vec<u16>, // NUL-terminated
    pub vid: u16,
    pub pid: u16,
    pub usage_page: u16,
    pub usage: u16,
    pub in_len: u16,
    pub out_len: u16,
    pub feature_len: u16,
}

impl HidInfo {
    pub fn path_string(&self) -> String {
        let end = self.path.iter().position(|&c| c == 0).unwrap_or(self.path.len());
        String::from_utf16_lossy(&self.path[..end])
    }
}

/// Parses `vid_XXXX` / `pid_XXXX` from a lowercase device path.
fn hex_after(path: &str, tag: &str) -> Option<u16> {
    let i = path.find(tag)? + tag.len();
    u16::from_str_radix(path.get(i..i + 4)?, 16).ok()
}

/// (vid, pid) from a lowercase HID path: USB `vid_xxxx&pid_xxxx`, Bluetooth `vid&0002xxxx_pid&xxxx`.
fn vid_pid(path: &str) -> (Option<u16>, Option<u16>) {
    let bt = |tag: &str| {
        let i = path.find(tag)? + tag.len();
        u16::from_str_radix(path.get(i + 4..i + 8)?, 16).ok() // skip the 000x source prefix
    };
    (hex_after(path, "vid_").or_else(|| bt("vid&")), hex_after(path, "pid_").or_else(|| hex_after(path, "pid&")))
}

/// Lists HID collections whose vendor id is in `filter_vid`. Devices of other vendors are
/// skipped by path before being opened.
pub fn enumerate(filter_vid: &[u16]) -> Vec<HidInfo> {
    let mut out = Vec::new();
    unsafe {
        let mut guid: GUID = zeroed();
        HidD_GetHidGuid(&mut guid);
        let set = SetupDiGetClassDevsW(&guid, null(), null_mut(), DIGCF_PRESENT | DIGCF_DEVICEINTERFACE);
        if set == INVALID_HANDLE_VALUE as HDEVINFO {
            return out;
        }
        // Detail buffer reused for every interface (u32-aligned for cbSize).
        let mut buf = [0u32; 512];
        let mut idx = 0u32;
        loop {
            let mut ifd: SP_DEVICE_INTERFACE_DATA = zeroed();
            ifd.cbSize = size_of::<SP_DEVICE_INTERFACE_DATA>() as u32;
            if SetupDiEnumDeviceInterfaces(set, null(), &guid, idx, &mut ifd) == 0 {
                break;
            }
            idx += 1;
            let detail = buf.as_mut_ptr() as *mut SP_DEVICE_INTERFACE_DETAIL_DATA_W;
            (*detail).cbSize = size_of::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>() as u32;
            let size = (buf.len() * 4) as u32;
            if SetupDiGetDeviceInterfaceDetailW(set, &ifd, detail, size, null_mut(), null_mut()) == 0 {
                continue;
            }
            let p = std::ptr::addr_of!((*detail).DevicePath) as *const u16;
            let mut len = 0usize;
            let max = buf.len() * 2 - 2;
            while len < max && *p.add(len) != 0 {
                len += 1;
            }
            let wide = std::slice::from_raw_parts(p, len);
            let lower = String::from_utf16_lossy(wide).to_ascii_lowercase();
            let (Some(vid), pid) = vid_pid(&lower) else { continue };
            if !filter_vid.contains(&vid) {
                continue;
            }
            let mut path = wide.to_vec();
            path.push(0);
            if let Some(info) = caps(path, vid, pid.unwrap_or(0)) {
                out.push(info);
            }
        }
        SetupDiDestroyDeviceInfoList(set);
    }
    out
}

unsafe fn caps(path: Vec<u16>, vid: u16, pid: u16) -> Option<HidInfo> {
    // Access 0: query-only open, never blocked by exclusive owners.
    let h = CreateFileW(path.as_ptr(), 0, FILE_SHARE_READ | FILE_SHARE_WRITE, null(), OPEN_EXISTING, 0, null_mut());
    if h == INVALID_HANDLE_VALUE {
        return None;
    }
    let mut pre: PHIDP_PREPARSED_DATA = 0;
    let mut result = None;
    if HidD_GetPreparsedData(h, &mut pre) != 0 {
        let mut c: HIDP_CAPS = zeroed();
        if HidP_GetCaps(pre, &mut c) == HIDP_STATUS_SUCCESS {
            result = Some(HidInfo {
                path,
                vid,
                pid,
                usage_page: c.UsagePage,
                usage: c.Usage,
                in_len: c.InputReportByteLength,
                out_len: c.OutputReportByteLength,
                feature_len: c.FeatureReportByteLength,
            });
        }
        HidD_FreePreparsedData(pre);
    }
    CloseHandle(h);
    result
}

/// An open HID collection. Handle and event are closed on drop.
pub struct HidDevice {
    handle: HANDLE,
    event: HANDLE,
    out_len: usize,
}

// The handle is only ever used from one thread at a time.
unsafe impl Send for HidDevice {}

impl HidDevice {
    pub fn open(info: &HidInfo) -> Option<HidDevice> {
        unsafe {
            let share = FILE_SHARE_READ | FILE_SHARE_WRITE;
            let mut h = CreateFileW(info.path.as_ptr(), GENERIC_READ | GENERIC_WRITE, share, null(), OPEN_EXISTING, FILE_FLAG_OVERLAPPED, null_mut());
            if h == INVALID_HANDLE_VALUE {
                h = CreateFileW(info.path.as_ptr(), GENERIC_READ, share, null(), OPEN_EXISTING, FILE_FLAG_OVERLAPPED, null_mut());
            }
            if h == INVALID_HANDLE_VALUE {
                return None;
            }
            let event = CreateEventW(null(), 1, 0, null());
            if event.is_null() {
                CloseHandle(h);
                return None;
            }
            Some(HidDevice { handle: h, event, out_len: info.out_len as usize })
        }
    }

    /// Runs one overlapped transfer and waits up to `ms`. Returns bytes transferred.
    unsafe fn transfer(&self, write: bool, data: *mut u8, len: u32, ms: u32) -> Option<usize> {
        let mut ov: OVERLAPPED = zeroed();
        ov.hEvent = self.event;
        ResetEvent(self.event);
        let mut n = 0u32;
        let ok = if write {
            WriteFile(self.handle, data, len, &mut n, &mut ov)
        } else {
            ReadFile(self.handle, data, len, &mut n, &mut ov)
        };
        if ok == 0 && GetLastError() != ERROR_IO_PENDING {
            return None;
        }
        if WaitForSingleObject(self.event, ms) != WAIT_OBJECT_0 {
            CancelIoEx(self.handle, &ov);
            GetOverlappedResult(self.handle, &ov, &mut n, 1); // wait for the cancel to land
            return None;
        }
        if GetOverlappedResult(self.handle, &ov, &mut n, 0) == 0 {
            return None;
        }
        Some(n as usize)
    }

    /// Writes an output report (first byte = report id), zero-padded to the collection's length.
    pub fn write(&self, data: &[u8]) -> bool {
        let mut buf = [0u8; 128];
        let len = self.out_len.max(data.len()).min(buf.len());
        if len == 0 {
            return false;
        }
        let n = data.len().min(len);
        buf[..n].copy_from_slice(&data[..n]);
        unsafe { self.transfer(true, buf.as_mut_ptr(), len as u32, 1000).is_some() }
    }

    /// Reads one input report into `buf`. `None` on timeout or error.
    pub fn read_timeout(&self, buf: &mut [u8], ms: u32) -> Option<usize> {
        unsafe { self.transfer(false, buf.as_mut_ptr(), buf.len() as u32, ms) }
    }

    /// HidD_GetFeature: `buf[0]` must hold the report id.
    #[allow(dead_code)]
    pub fn get_feature(&self, buf: &mut [u8]) -> bool {
        unsafe { HidD_GetFeature(self.handle, buf.as_mut_ptr() as _, buf.len() as u32) != 0 }
    }

    /// Drops input reports queued before our request.
    pub fn flush(&self) {
        unsafe {
            HidD_FlushQueue(self.handle);
        }
    }
}

impl Drop for HidDevice {
    fn drop(&mut self) {
        unsafe {
            CancelIoEx(self.handle, null());
            CloseHandle(self.handle);
            CloseHandle(self.event);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_vid_pid_from_path() {
        let p = r"\\?\hid#vid_046d&pid_c547&mi_02&col01#8&2a2b3c4d&0&0000#{4d1e55b2}";
        assert_eq!(hex_after(p, "vid_"), Some(0x046D));
        assert_eq!(hex_after(p, "pid_"), Some(0xC547));
        assert_eq!(hex_after(r"\\?\hid#vhf", "vid_"), None);
    }
}

#[cfg(test)]
mod bt_path_tests {
    use super::*;

    #[test]
    fn bluetooth_hid_paths_carry_vid_after_ampersand() {
        let p = r"\?\hid#{00001124-0000-1000-8000-00805f9b34fb}_vid&0002054c_pid&0ce6&col01#9&2b7c9b0a&0&0000#{4d1e55b2}";
        assert_eq!(vid_pid(p), (Some(0x054C), Some(0x0CE6)));
        let usb = r"\?\hid#vid_046d&pid_c547&mi_02&col02#8&24348baf&0&0001#{4d1e55b2}";
        assert_eq!(vid_pid(usb), (Some(0x046D), Some(0xC547)));
    }
}
