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
    tip_text: Vec<u16>,
    tray_icon: bool,
    menu_keys: Vec<String>,
    trimmed: bool,
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
    let mut empty = wide("");
    let mut ti = tool_info(widget, owner, empty.as_mut_ptr());
    SendMessageW(tip, TTM_ADDTOOLW, 0, &mut ti as *mut _ as LPARAM);
    SendMessageW(tip, TTM_SETMAXTIPWIDTH, 0, 600);
    tip
}

unsafe fn tool_info(widget: HWND, owner: HWND, text: *mut u16) -> TTTOOLINFOW {
    let mut ti: TTTOOLINFOW = zeroed();
    ti.cbSize = size_of::<TTTOOLINFOW>() as u32;
    ti.uFlags = TTF_IDISHWND | TTF_SUBCLASS;
    ti.hwnd = owner;
    ti.uId = widget as usize;
    ti.lpszText = text;
    ti
}

fn tooltip_text(items: &[DeviceStatus]) -> String {
    if items.is_empty() {
        return "PeriBattery — procurando dispositivos…".into();
    }
    items
        .iter()
        .map(|d| {
            let pct = d.percent.map(|p| format!("{p}%")).unwrap_or_else(|| "nível não informado".into());
            let mv = d.millivolts.map(|v| format!(" ({v} mV)")).unwrap_or_default();
            let state = if !d.connected {
                " — desligado (último valor)"
            } else if d.charging {
                " — carregando"
            } else {
                ""
            };
            format!("{} — {pct}{mv}{state}", d.name)
        })
        .collect::<Vec<_>>()
        .join("\r\n")
}

/// Redraws the strip and re-places it. Called on new data, theme or DPI change.
unsafe fn redraw() {
    let (widget, items, tip, controller) = with(|a| (a.widget, a.items.clone(), a.tooltip, a.controller));
    let shown = render::visible(&items, &settings::hidden());
    update_tooltip(&tooltip_text(&shown));
    if widget.is_null() || IsWindow(widget) == 0 {
        return;
    }
    let tray = find_tray();
    if tray.is_null() {
        return;
    }
    let mut tr: RECT = zeroed();
    GetWindowRect(tray, &mut tr);
    let dpi = GetDpiForWindow(tray).max(96);
    let width = render::draw(widget, &shown, dpi, tr.bottom - tr.top, light_theme());
    with(|a| a.width = width);
    let _ = (tip, controller);
    place();
}

unsafe fn update_tooltip(text: &str) {
    let (tip, widget, controller) = with(|a| {
        a.tip_text = wide(text);
        (a.tooltip, a.widget, a.controller)
    });
    if !tip.is_null() {
        let ptr = with(|a| a.tip_text.as_mut_ptr());
        let ti = tool_info(widget, controller, ptr);
        SendMessageW(tip, TTM_UPDATETIPTEXTW, 0, &ti as *const _ as LPARAM);
    }
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
            place();
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
            // Explorer restarted: the old widget died with the old taskbar.
            with(|a| {
                a.widget = null_mut();
                a.tray_icon = false;
            });
            ensure_widget();
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
