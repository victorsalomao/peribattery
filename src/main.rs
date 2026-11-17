#![cfg_attr(not(test), windows_subsystem = "windows")]

mod devices;
mod estimate;
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
            for mut s in devices::poll_all() {
                let learned = settings::learned(&s.key);
                estimate::annotate(&mut s, &mut estimate::Tracker::default(), 0, &learned);
                let mv = s.millivolts.map(|v| format!(" [{v} mV]")).unwrap_or_default();
                println!("{:<9?} {}{}", s.kind, ui::widget::device_line(&s), mv);
            }
        }
        _ => run_app(),
    }
}

/// Asks a running instance to quit (WM_CLOSE to its controller) and waits up to 3 s.
fn stop_running_instance() {
    use windows_sys::Win32::UI::WindowsAndMessaging::{FindWindowW, PostMessageW, WM_CLOSE};
    let class: Vec<u16> = "PeriBatteryController".encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        let h = FindWindowW(class.as_ptr(), std::ptr::null());
        if h.is_null() {
            return;
        }
        PostMessageW(h, WM_CLOSE, 0, 0);
        for _ in 0..30 {
            std::thread::sleep(std::time::Duration::from_millis(100));
            if FindWindowW(class.as_ptr(), std::ptr::null()).is_null() {
                return;
            }
        }
    }
}

/// Running from anywhere but the install folder with a different build (or nothing installed):
/// replace the installed copy (stopping it first), turn on autostart the first time, and hand
/// over to the installed copy. Otherwise go straight to the widget (one instance only).
fn run_app() {
    let (action, target) = settings::startup_action();
    if let (settings::InstallAction::Install, Some(target)) = (action, target) {
        stop_running_instance();
        if settings::copy_self_to(&target) {
            if !settings::is_installed() || settings::autostart_enabled() {
                settings::set_autostart(true); // first install, or re-point an enabled Run entry
            }
            settings::mark_installed();
            let mut cmd = std::process::Command::new(&target);
            if let Some(dir) = target.parent() {
                cmd.current_dir(dir); // don't keep the download folder busy
            }
            if cmd.spawn().is_ok() {
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
