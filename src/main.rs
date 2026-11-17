#![cfg_attr(not(test), windows_subsystem = "windows")]

mod hid;
mod model;

/// VIDs we care about: Logitech, BenQ ZOWIE, Sony, Microsoft, 8BitDo.
pub const VIDS: [u16; 5] = [0x046D, 0x04A5, 0x054C, 0x045E, 0x2DC8];

fn attach_console() {
    unsafe {
        windows_sys::Win32::System::Console::AttachConsole(windows_sys::Win32::System::Console::ATTACH_PARENT_PROCESS);
    }
}

fn main() {
    let arg = std::env::args().nth(1).unwrap_or_default();
    match arg.as_str() {
        "--list" => {
            attach_console();
            for d in hid::enumerate(&VIDS) {
                println!(
                    "{:04x}:{:04x} page {:04x}:{:04x} in/out/feat {}/{}/{}  {}",
                    d.vid, d.pid, d.usage_page, d.usage, d.in_len, d.out_len, d.feature_len,
                    d.path_string()
                );
            }
        }
        _ => {}
    }
}
