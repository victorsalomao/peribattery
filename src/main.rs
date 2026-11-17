#![cfg_attr(not(test), windows_subsystem = "windows")]

mod devices;
mod hid;
mod model;
mod poller;
mod settings;
mod ui;

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
        "--once" => {
            attach_console();
            for s in devices::poll_all() {
                let pct = s.percent.map(|p| format!("{p}%")).unwrap_or_else(|| "—".into());
                let mv = s.millivolts.map(|v| format!(" {v} mV")).unwrap_or_default();
                let state = if !s.connected { " (desligado)" } else if s.charging { " (carregando)" } else { "" };
                println!("{:<9?} {:<22} {}{}{}", s.kind, s.name, pct, mv, state);
            }
        }
        _ => run_app(),
    }
}

/// First run installs a copy to %LOCALAPPDATA%\PeriBattery, turns on autostart and starts the
/// installed copy. Every later start goes straight to the widget (one instance only).
fn run_app() {
    if !settings::is_installed() {
        let copied = settings::ensure_installed();
        settings::mark_installed();
        settings::set_autostart(true);
        if let Some(exe) = copied {
            if std::process::Command::new(exe).spawn().is_ok() {
                return;
            }
        }
    }
    let name: Vec<u16> = "Local\\PeriBattery".encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        use windows_sys::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS};
        let m = windows_sys::Win32::System::Threading::CreateMutexW(std::ptr::null(), 0, name.as_ptr());
        if m.is_null() || GetLastError() == ERROR_ALREADY_EXISTS {
            return; // already running
        }
    }
    std::process::exit(ui::widget::run());
}
