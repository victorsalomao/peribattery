//! Taskbar widget: a layered child window of Shell_TrayWnd, left of the notification area
//! (the "POR / PTB" language indicator sits at its left edge).
//!
//! A hidden top-level "controller" window receives the broadcasts a child of Explorer would
//! not: TaskbarCreated (Explorer restarted), WM_DEVICECHANGE, WM_POWERBROADCAST and theme
//! changes. Everything runs on the UI thread; the poller only posts WM_APP_STATUS.

use std::cell::RefCell;
use std::mem::{size_of, zeroed};
use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Registry::*;
use windows_sys::Win32::UI::Controls::*;
use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
use windows_sys::Win32::UI::Shell::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use crate::model::DeviceStatus;
use crate::poller::{self, PollerHandle, WM_APP_STATUS};
use crate::settings;
use crate::ui::render;

const TIMER_PLACE: usize = 1;
const TIMER_DEVCHANGE: usize = 2;
const PLACE_EVERY_MS: u32 = 3000;
const DEVCHANGE_DEBOUNCE_MS: u32 = 2000;
const WM_TRAY: u32 = WM_APP + 2;

const CMD_REFRESH: usize = 1;
const CMD_AUTOSTART: usize = 2;
const CMD_EXIT: usize = 3;
const CMD_HIDE_BASE: usize = 100;

#[derive(Default)]
struct App {
    controller: HWND,
    widget: HWND,
    tooltip: HWND,
    poller: Option<PollerHandle>,
    items: Vec<DeviceStatus>,
    width: i32,
    taskbar_created: u32,
    tool_count: usize,
    tray_icon: bool,
    menu_keys: Vec<String>,
    trimmed: bool,
    drawn: Option<(u32, i32, bool)>,
}

thread_local! {
    static APP: RefCell<App> = RefCell::new(App::default());
}

fn with<R>(f: impl FnOnce(&mut App) -> R) -> R {
    APP.with(|a| f(&mut a.borrow_mut()))
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Taskbar text follows the *system* theme, not the apps theme.
fn light_theme() -> bool {
    let key = wide(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize");
    let name = wide("SystemUsesLightTheme");
    let mut v: u32 = 0;
    let mut sz = 4u32;
    unsafe {
        RegGetValueW(HKEY_CURRENT_USER, key.as_ptr(), name.as_ptr(), RRF_RT_REG_DWORD, null_mut(), &mut v as *mut u32 as _, &mut sz) == 0 && v != 0
    }
}

unsafe fn find_tray() -> HWND {
    let c = wide("Shell_TrayWnd");
    FindWindowW(c.as_ptr(), null())
}

/// Builds the UI and runs the message loop until "Sair".
pub fn run() -> i32 {
    unsafe {
        let inst = GetModuleHandleW(null());
        let ctrl_class = wide("PeriBatteryController");
        let widget_class = wide("PeriBatteryWidget");
        let mut wc: WNDCLASSW = zeroed();
        wc.hInstance = inst;
        wc.lpfnWndProc = Some(controller_proc);
        wc.lpszClassName = ctrl_class.as_ptr();
        RegisterClassW(&wc);
        wc.lpfnWndProc = Some(widget_proc);
        wc.lpszClassName = widget_class.as_ptr();
        wc.hCursor = LoadCursorW(null_mut(), IDC_ARROW);
        RegisterClassW(&wc);

        let tb = wide("TaskbarCreated");
        let taskbar_created = RegisterWindowMessageW(tb.as_ptr());
        // Top-level (not message-only) so it receives broadcasts; never shown.
        let controller = CreateWindowExW(WS_EX_TOOLWINDOW, ctrl_class.as_ptr(), null(), WS_POPUP, 0, 0, 0, 0, null_mut(), null_mut(), inst, null());
        if controller.is_null() {
            return 1;
        }
        // Explorer runs elevated-integrity-free; let its broadcast through UIPI anyway.
        ChangeWindowMessageFilterEx(controller, taskbar_created, MSGFLT_ALLOW, null_mut());
        with(|a| {
            a.controller = controller;
            a.taskbar_created = taskbar_created;
            a.items = Vec::new();
        });
        let poller = poller::start(controller as isize);
        with(|a| a.poller = Some(poller));
        ensure_widget();
        SetTimer(controller, TIMER_PLACE, PLACE_EVERY_MS, None);

        let mut msg: MSG = zeroed();
        while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        remove_tray_icon();
        0
    }
}

unsafe fn ensure_widget() {
    let widget = with(|a| a.widget);
    if !widget.is_null() && IsWindow(widget) != 0 {
        return;
    }
    let tray = find_tray();
    if tray.is_null() {
        add_tray_icon(); // no taskbar (Explorer down / replaced): notification icon fallback
        return;
    }
    remove_tray_icon();
    let class = wide("PeriBatteryWidget");
    let w = CreateWindowExW(
        WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
        class.as_ptr(),
        null(),
        WS_CHILD | WS_CLIPSIBLINGS,
        0,
        0,
        1,
        1,
        tray,
        null_mut(),
        GetModuleHandleW(null()),
        null(),
    );
    if w.is_null() {
        add_tray_icon();
        return;
    }
    let (controller, old_tip) = with(|a| (a.controller, a.tooltip));
    if !old_tip.is_null() {
        DestroyWindow(old_tip);
    }
    let tip = create_tooltip(controller, w);
    with(|a| {
        a.widget = w;
        a.tooltip = tip;
        a.width = 0;
        a.tool_count = 0;
    });
    redraw();
}

unsafe fn create_tooltip(owner: HWND, widget: HWND) -> HWND {
    let icc = INITCOMMONCONTROLSEX { dwSize: size_of::<INITCOMMONCONTROLSEX>() as u32, dwICC: ICC_WIN95_CLASSES };
    InitCommonControlsEx(&icc);
    let class = wide("tooltips_class32");
    let tip = CreateWindowExW(
        WS_EX_TOPMOST,
        class.as_ptr(),
        null(),
        WS_POPUP | TTS_NOPREFIX | TTS_ALWAYSTIP,
        CW_USEDEFAULT,
        CW_USEDEFAULT,
        CW_USEDEFAULT,
        CW_USEDEFAULT,
        owner,
        null_mut(),
        GetModuleHandleW(null()),
        null(),
    );
    if tip.is_null() {
        return tip;
    }
    let _ = widget; // tools (one per device) are added after each draw
    SendMessageW(tip, TTM_SETMAXTIPWIDTH, 0, 600);
    tip
}

/// One tooltip zone per device: hovering the mouse shows only the mouse, and so on.
unsafe fn set_device_tools(spans: &[(i32, i32)], texts: &[String], height: i32) {
    let (tip, widget, old) = with(|a| (a.tooltip, a.widget, a.tool_count));
    if tip.is_null() || widget.is_null() {
        return;
    }
    for id in 0..old {
        let mut ti: TTTOOLINFOW = zeroed();
        ti.cbSize = size_of::<TTTOOLINFOW>() as u32;
        ti.hwnd = widget;
        ti.uId = id;
        SendMessageW(tip, TTM_DELTOOLW, 0, &mut ti as *mut _ as LPARAM);
    }
    for (id, (&(x0, x1), text)) in spans.iter().zip(texts).enumerate() {
        let mut t = wide(text); // the control keeps its own copy
        let mut ti: TTTOOLINFOW = zeroed();
        ti.cbSize = size_of::<TTTOOLINFOW>() as u32;
        ti.uFlags = TTF_SUBCLASS;
        ti.hwnd = widget;
        ti.uId = id;
        ti.rect = RECT { left: x0, top: 0, right: x1, bottom: height };
        ti.lpszText = t.as_mut_ptr();
        SendMessageW(tip, TTM_ADDTOOLW, 0, &mut ti as *mut _ as LPARAM);
    }
    with(|a| a.tool_count = spans.len().min(texts.len()));
}

fn tooltip_text(items: &[DeviceStatus]) -> String {
    if items.is_empty() {
        return "PeriBattery — procurando dispositivos…".into();
    }
    items
        .iter()
        .map(device_line)
        .collect::<Vec<_>>()
        .join("\r\n")
}

/// "Nome — ⚡ 45% — carregando · cheio em ~1 h 10 min (medido)"
pub fn device_line(d: &DeviceStatus) -> String {
    use crate::estimate::{format_minutes, Estimate, Source};
    let Some(pct) = d.percent else {
        let state = if d.connected { "conectado, nível não informado" } else { "desligado" };
        return format!("{} — {state}", d.name);
    };
    if !d.connected {
        return format!("{} — {pct}% — desligado (último valor)", d.name);
    }
    let label = |s: Source| match s {
        Source::Measured => "medido",
        Source::Learned => "aprendido",
        Source::Factory => "média de fábrica",
    };
    let (bolt, state) = if d.charging { ("⚡ ", "carregando") } else { ("", "descarregando") };
    let eta = match d.estimate {
        Some(Estimate::Full { minutes, source }) => format!(" · cheio em {} ({})", format_minutes(minutes), label(source)),
        Some(Estimate::Empty { minutes, source }) => format!(" · acaba em {} ({})", format_minutes(minutes), label(source)),
        Some(Estimate::Calculating) => " · calculando tempo para encher…".to_string(),
        None => String::new(),
    };
    format!("{} — {bolt}{pct}% — {state}{eta}", d.name)
}

/// Redraws the strip and re-places it. Called on new data, theme or DPI change.
unsafe fn redraw() {
    let (widget, items) = with(|a| (a.widget, a.items.clone()));
    let shown = render::visible(&items, &settings::hidden());
    update_tooltip(&tooltip_text(&shown));
    if widget.is_null() || IsWindow(widget) == 0 {
        return;
    }
    let Some(env) = draw_env() else { return };
    let (width, spans) = render::draw(widget, &shown, env.0, env.1, env.2);
    let texts: Vec<String> = shown.iter().map(device_line).collect();
    set_device_tools(&spans, &texts, env.1);
    with(|a| {
        a.width = width;
        a.drawn = Some(env);
    });
    place();
}

/// What the bitmap depends on: (taskbar dpi, taskbar height, light theme).
unsafe fn draw_env() -> Option<(u32, i32, bool)> {
    let tray = find_tray();
    if tray.is_null() {
        return None;
    }
    let mut tr: RECT = zeroed();
    GetWindowRect(tray, &mut tr);
    Some((GetDpiForWindow(tray).max(96), tr.bottom - tr.top, light_theme()))
}

/// Periodic check: redraw if scale / taskbar height / theme changed, otherwise just re-place.
unsafe fn refresh_if_stale() {
    let drawn = with(|a| a.drawn);
    match draw_env() {
        Some(env) if render::needs_redraw(drawn, env) => redraw(),
        _ => place(),
    }
}

/// Fallback tray icon: one tooltip with every device.
unsafe fn update_tooltip(text: &str) {
    if with(|a| a.tray_icon) {
        let mut nid = notify_data();
        nid.uFlags = NIF_TIP | NIF_SHOWTIP;
        copy_tip(&mut nid, text);
        Shell_NotifyIconW(NIM_MODIFY, &nid);
    }
}

/// Keeps the widget glued to the left of the notification area.
unsafe fn place() {
    let (widget, width) = with(|a| (a.widget, a.width));
    if widget.is_null() || IsWindow(widget) == 0 {
        return;
    }
    if width == 0 {
        ShowWindow(widget, SW_HIDE);
        return;
    }
    let tray = find_tray();
    if tray.is_null() {
        return;
    }
    let mut tr: RECT = zeroed();
    GetWindowRect(tray, &mut tr);
    let cls = wide("TrayNotifyWnd");
    let notify = FindWindowExW(tray, null_mut(), cls.as_ptr(), null());
    let mut right = tr.right;
    if !notify.is_null() {
        let mut nr: RECT = zeroed();
        GetWindowRect(notify, &mut nr);
        if nr.right > nr.left {
            right = nr.left;
        }
    }
    let margin = 4 * GetDpiForWindow(tray).max(96) as i32 / 96;
    let (x, h) = (right - tr.left - width - margin, tr.bottom - tr.top);
    let mut cur: RECT = zeroed();
    GetWindowRect(widget, &mut cur);
    let moved = cur.left - tr.left != x || cur.top != tr.top || cur.right - cur.left != width || cur.bottom - cur.top != h;
    if moved || GetWindow(tray, GW_CHILD) != widget || IsWindowVisible(widget) == 0 {
        SetWindowPos(widget, HWND_TOP, x, 0, width, h, SWP_NOACTIVATE | SWP_SHOWWINDOW);
    }
}

unsafe fn show_menu() {
    let (controller, items) = with(|a| (a.controller, a.items.clone()));
    let menu = CreatePopupMenu();
    let hide = CreatePopupMenu();
    let hidden = settings::hidden();
    let add = |m: HMENU, flags: u32, id: usize, text: &str| {
        let t = wide(text);
        AppendMenuW(m, flags, id, t.as_ptr());
    };
    let mut keys = Vec::new();
    for (i, d) in items.iter().enumerate() {
        let flags = MF_STRING | if hidden.contains(&d.key) { MF_CHECKED } else { 0 };
        add(hide, flags, CMD_HIDE_BASE + i, &d.name);
        keys.push(d.key.clone());
    }
    with(|a| a.menu_keys = keys);
    add(menu, MF_STRING, CMD_REFRESH, "Atualizar agora");
    add(menu, MF_STRING | if settings::autostart_enabled() { MF_CHECKED } else { 0 }, CMD_AUTOSTART, "Iniciar com o Windows");
    if !items.is_empty() {
        add(menu, MF_POPUP, hide as usize, "Ocultar");
    }
    add(menu, MF_SEPARATOR, 0, "");
    add(menu, MF_STRING, CMD_EXIT, "Sair");
    let mut pt: POINT = zeroed();
    GetCursorPos(&mut pt);
    SetForegroundWindow(controller);
    let cmd = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_BOTTOMALIGN, pt.x, pt.y, 0, controller, null()) as usize;
    PostMessageW(controller, WM_NULL, 0, 0);
    DestroyMenu(menu); // also destroys the attached submenu
    match cmd {
        CMD_REFRESH => {
            if let Some(p) = with(|a| a.poller) {
                p.wake();
            }
        }
        CMD_AUTOSTART => settings::set_autostart(!settings::autostart_enabled()),
        CMD_EXIT => {
            let w = with(|a| a.widget);
            if !w.is_null() {
                DestroyWindow(w);
            }
            PostQuitMessage(0);
        }
        c if c >= CMD_HIDE_BASE => {
            if let Some(key) = with(|a| a.menu_keys.get(c - CMD_HIDE_BASE).cloned()) {
                let now_hidden = settings::hidden().contains(&key);
                settings::set_hidden(&key, !now_hidden);
                redraw();
            }
        }
        _ => {}
    }
}

unsafe fn notify_data() -> NOTIFYICONDATAW {
    let mut nid: NOTIFYICONDATAW = zeroed();
    nid.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
    nid.hWnd = with(|a| a.controller);
    nid.uID = 1;
    nid
}

fn copy_tip(nid: &mut NOTIFYICONDATAW, text: &str) {
    for (dst, src) in nid.szTip.iter_mut().zip(text.encode_utf16().take(127)) {
        *dst = src;
    }
}

unsafe fn add_tray_icon() {
    if with(|a| a.tray_icon) {
        return;
    }
    let mut nid = notify_data();
    nid.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP | NIF_SHOWTIP;
    nid.uCallbackMessage = WM_TRAY;
    nid.hIcon = LoadIconW(GetModuleHandleW(null()), 1 as _);
    let text = tooltip_text(&with(|a| a.items.clone()));
    copy_tip(&mut nid, &text);
    if Shell_NotifyIconW(NIM_ADD, &nid) != 0 {
        with(|a| a.tray_icon = true);
    }
}

unsafe fn remove_tray_icon() {
    if with(|a| a.tray_icon) {
        let nid = notify_data();
        Shell_NotifyIconW(NIM_DELETE, &nid);
        with(|a| a.tray_icon = false);
    }
}

unsafe extern "system" fn controller_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_APP_STATUS => {
            let items = Box::from_raw(lp as *mut Vec<DeviceStatus>);
            let first = with(|a| {
                let first = a.items.is_empty() && !a.trimmed;
                a.items = *items;
                a.trimmed = true;
                first
            });
            ensure_widget();
            redraw();
            if first {
                // Startup pages (loader, fonts, first scan) are never touched again: give them back.
                windows_sys::Win32::System::Threading::SetProcessWorkingSetSize(
                    windows_sys::Win32::System::Threading::GetCurrentProcess(),
                    usize::MAX,
                    usize::MAX,
                );
            }
            0
        }
        WM_TIMER if wp == TIMER_PLACE => {
            ensure_widget();
            refresh_if_stale();
            0
        }
        WM_TIMER if wp == TIMER_DEVCHANGE => {
            KillTimer(hwnd, TIMER_DEVCHANGE);
            if let Some(p) = with(|a| a.poller) {
                p.wake();
            }
            0
        }
        WM_DEVICECHANGE => {
            // DBT_DEVNODES_CHANGED arrives in bursts: wait for it to settle.
            SetTimer(hwnd, TIMER_DEVCHANGE, DEVCHANGE_DEBOUNCE_MS, None);
            1
        }
        WM_POWERBROADCAST => {
            if wp as u32 == PBT_APMRESUMEAUTOMATIC {
                SetTimer(hwnd, TIMER_DEVCHANGE, DEVCHANGE_DEBOUNCE_MS, None);
            }
            1
        }
        WM_SETTINGCHANGE | WM_THEMECHANGED | WM_DISPLAYCHANGE | WM_DPICHANGED => {
            redraw();
            DefWindowProcW(hwnd, msg, wp, lp)
        }
        WM_TRAY => {
            let ev = (lp as u32) & 0xFFFF;
            if ev == WM_RBUTTONUP || ev == WM_LBUTTONUP {
                show_menu();
            }
            0
        }
        m if m != 0 && m == with(|a| a.taskbar_created) => {
            // Explorer restarted — or just re-broadcast (e.g. scale change) with the taskbar alive.
            // Destroy a surviving widget so there is never a second, stale strip.
            let old = with(|a| a.widget);
            if !old.is_null() && IsWindow(old) != 0 {
                DestroyWindow(old);
            }
            remove_tray_icon();
            with(|a| {
                a.widget = null_mut();
                a.tray_icon = false;
                a.drawn = None;
            });
            ensure_widget();
            0
        }
        WM_CLOSE => {
            // taskkill without /f, or a newer build asking us to step aside.
            let w = with(|a| a.widget);
            if !w.is_null() && IsWindow(w) != 0 {
                DestroyWindow(w);
            }
            DestroyWindow(hwnd);
            0
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

unsafe extern "system" fn widget_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_RBUTTONUP => {
            show_menu();
            0
        }
        WM_MOUSEACTIVATE => MA_NOACTIVATE as LRESULT,
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::estimate::{Estimate, Source};
    use crate::model::DeviceKind;

    fn dev(name: &str, pct: Option<u8>, charging: bool, connected: bool, estimate: Option<Estimate>) -> DeviceStatus {
        DeviceStatus { key: name.into(), kind: DeviceKind::Headset, name: name.into(), percent: pct, millivolts: Some(3675), charging, connected, estimate }
    }

    #[test]
    fn tooltip_discharging_with_estimate() {
        let d = dev("Logitech G435", Some(30), false, true, Some(Estimate::Empty { minutes: 320, source: Source::Measured }));
        assert_eq!(tooltip_text(&[d]), "Logitech G435 — 30% — descarregando · acaba em ~5 h 20 min (medido)");
    }

    #[test]
    fn tooltip_charging_with_estimate_and_bolt() {
        let d = dev("ZOWIE EC2-CW", Some(45), true, true, Some(Estimate::Full { minutes: 70, source: Source::Learned }));
        assert_eq!(tooltip_text(&[d]), "ZOWIE EC2-CW — ⚡ 45% — carregando · cheio em ~1 h 10 min (aprendido)");
    }

    #[test]
    fn tooltip_charging_calculating_and_factory() {
        let d = dev("DualSense", Some(20), true, true, Some(Estimate::Calculating));
        assert_eq!(tooltip_text(&[d]), "DualSense — ⚡ 20% — carregando · calculando tempo para encher…");
        let d = dev("G915", Some(92), false, true, Some(Estimate::Empty { minutes: 2220, source: Source::Factory }));
        assert_eq!(tooltip_text(&[d]), "G915 — 92% — descarregando · acaba em ~37 h (média de fábrica)");
    }

    #[test]
    fn tooltip_off_and_unknown() {
        let d = dev("Logitech G435", Some(30), false, false, None);
        assert_eq!(tooltip_text(&[d]), "Logitech G435 — 30% — desligado (último valor)");
        let d = dev("8BitDo", None, false, true, None);
        assert_eq!(tooltip_text(&[d]), "8BitDo — conectado, nível não informado");
    }
}
