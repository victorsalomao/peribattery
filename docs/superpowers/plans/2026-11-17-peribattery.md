# PeriBattery Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A single Rust `.exe` that shows battery % of ZOWIE EC2-CW, Logitech G915 TKL, Logitech G435 and PS5 DualSense in a widget on the Windows 11 taskbar, left of the language indicator, starting with Windows, with <2 MB RAM and 0% CPU between reads.

**Architecture:** One UI thread (Win32 message loop, layered child window of `Shell_TrayWnd`) and one background poller thread. The poller reads each device through raw Win32 HID (`windows-sys`), builds a `Vec<DeviceStatus>`, and `PostMessage`s it to the UI only when it differs from the last one. Protocol modules are pure functions (frame builders / parsers) plus a thin I/O function, so parsers are unit-tested without hardware.

**Tech Stack:** Rust stable (MSVC target `x86_64-pc-windows-msvc`), `windows-sys 0.59` only at runtime, `embed-resource` as build-dependency. No `hidapi`, no WinRT, no serde.

**Spec:** `docs/superpowers/specs/2026-11-17-taskbar-battery-design.md`

## Global Constraints

- Runtime dependencies: `windows-sys = "0.59"`; plus `windows = "0.58"` **only** for Windows.Gaming.Input in `devices/xbox.rs` (Task 6b). Build dependency: `embed-resource`.
- Release profile: `opt-level = 3`, `lto = true`, `codegen-units = 1`, `panic = "abort"`, `strip = true`. Because of `panic = "abort"`, **no `unwrap()`/`expect()`/indexing that can panic in poll or UI paths** — use `get()`/`?`/early return.
- `#![windows_subsystem = "windows"]`; manifest with `PerMonitorV2` DPI awareness and Common Controls v6.
- Never send to a device anything other than: Zowie `A5 03`; HID++ read functions listed in Task 4; the G435 session in Task 5. DualSense: **read only, zero writes**.
- All HID handles opened with `FILE_SHARE_READ | FILE_SHARE_WRITE`, overlapped, closed after each poll. Every read has a timeout (≤1000 ms, G435 steps as listed).
- Poller thread runs with `SetThreadPriority(THREAD_MODE_BACKGROUND_BEGIN)`.
- Intervals: Zowie/G915/DualSense 60 s; G435 120 s; all ×5 (300 s / 600 s) while `SHQueryUserNotificationState` returns `QUNS_RUNNING_D3D_FULL_SCREEN` or `QUNS_BUSY`. Immediate poll on `WM_DEVICECHANGE` (`DBT_DEVNODES_CHANGED`, debounced 2 s) and `PBT_APMRESUMEAUTOMATIC`.
- Low battery threshold: **≤15%** → red. Charging → bolt. Disconnected/asleep → dimmed last known value; DualSense hidden when absent.
- Install path `%LOCALAPPDATA%\PeriBattery\PeriBattery.exe`; autostart value `PeriBattery` under `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`; settings under `HKCU\Software\PeriBattery`.
- Icons from font **Segoe Fluent Icons** (fallback **Segoe MDL2 Assets**): mouse `U+E962`, keyboard `U+E765`, headset `U+E7F6`, gamepad `U+E7FC`, bolt `U+E945`.
- Do not copy code from krisgfx9 (GPL-3). Protocol facts come from the spec.

## Review Focus

1. Receiver unplugged / device switched off mid-read → no panic, device shows dimmed last value (covered: Tasks 3–6 parser tests on short/zero replies; Task 2 read errors return `Err`).
2. Explorer restarts (`TaskbarCreated`) → widget re-parented and re-placed (Task 8 manual check step).
3. Second instance launched (autostart + manual) → exits immediately (Task 9 test of mutex helper + manual).
4. Theme light↔dark or DPI change → redraw with right colors/size (Task 8: `WM_SETTINGCHANGE` "ImmersiveColorSet", `WM_DPICHANGED` handled; manual check).
5. Fullscreen game running → poll interval grows ×5 (Task 7 unit test of `interval_for`).

## File Structure

```
Cargo.toml, build.rs, res/app.rc, res/app.manifest, res/app.ico
src/main.rs            entry, single instance, CLI (--once, --list), starts poller + UI
src/model.rs           DeviceKind, DeviceStatus, interpolate()
src/hid.rs             HID enumeration + HidDevice (open/write/read_timeout/get_feature/flush)
src/devices/mod.rs     poll_all(&mut PollState) -> Vec<DeviceStatus>
src/devices/zowie.rs   EC-CW protocol
src/devices/hidpp.rs   Logitech HID++ 2.0 over receiver
src/devices/g435.rs    G435 0ADF protocol
src/devices/dualsense.rs
src/poller.rs          scheduling, game mode, change detection, thread
src/ui/render.rs       DIB 32bpp drawing of the strip
src/ui/widget.rs       taskbar child window, placement, tooltip, menu, tray fallback
src/settings.rs        registry autostart, hidden devices, install copy
```

---

### Task 1: Toolchain, project skeleton and model

**Files:** Create `Cargo.toml`, `build.rs`, `res/app.rc`, `res/app.manifest`, `res/app.ico`, `src/main.rs`, `src/model.rs`, `.gitignore`

**Interfaces — Produces:**
```rust
pub enum DeviceKind { Mouse, Keyboard, Headset, Gamepad }
#[derive(Clone, PartialEq, Debug)]
pub struct DeviceStatus { pub key: &'static str, pub kind: DeviceKind, pub name: String,
  pub percent: Option<u8>, pub millivolts: Option<u16>, pub charging: bool, pub connected: bool }
pub fn interpolate(mv: u16, volts: &[u16], pcts: &[u8]) -> u8   // volts descending, clamps 0..=100
```

- [ ] **Step 1:** Install Rust: `winget install --id Rustlang.Rustup -e --silent`, then `rustup default stable-x86_64-pc-windows-msvc`. Verify `cargo --version` prints a version.
- [ ] **Step 2:** `git init`, `.gitignore` with `/target`. `cargo init --name peribattery`. Fill Cargo.toml per Global Constraints; windows-sys features: `Win32_Foundation, Win32_Devices_DeviceAndDriverInstallation, Win32_Devices_HumanInterfaceDevice, Win32_Storage_FileSystem, Win32_System_IO, Win32_System_Threading, Win32_UI_WindowsAndMessaging, Win32_UI_Shell, Win32_UI_HiDpi, Win32_UI_Controls, Win32_Graphics_Gdi, Win32_System_Registry, Win32_System_Power, Win32_System_LibraryLoader, Win32_System_ProcessStatus, Win32_UI_Input_KeyboardAndMouse`. `build.rs` calls `embed_resource::compile("res/app.rc", embed_resource::NONE)`; the .rc embeds icon id 1 and the manifest (`1 24 "app.manifest"`). Generate a simple battery `.ico` with a PowerShell/System.Drawing script (16/32/48 px).
- [ ] **Step 3: Failing tests in `src/model.rs`:**
```rust
#[test] fn interp_endpoints() { let v=[4200,4000,3500]; let p=[100,75,0];
  assert_eq!(interpolate(4300,&v,&p),100); assert_eq!(interpolate(3400,&v,&p),0); assert_eq!(interpolate(4000,&v,&p),75); }
#[test] fn interp_midpoint() { assert_eq!(interpolate(4100,&[4200,4000],&[100,75]),88); } // rounds half up
```
- [ ] **Step 4:** `cargo test` → FAIL (not defined). Implement. `cargo test` → PASS. `cargo build --release` → produces `target/release/peribattery.exe` with icon.
- [ ] **Step 5:** Commit `chore: scaffold PeriBattery`.

### Task 2: Raw HID layer

**Files:** Create `src/hid.rs`

**Interfaces — Produces:**
```rust
pub struct HidInfo { pub path: Vec<u16>, pub vid: u16, pub pid: u16, pub usage_page: u16, pub usage: u16,
  pub in_len: u16, pub out_len: u16, pub feature_len: u16 }
pub fn enumerate(filter_vid: &[u16]) -> Vec<HidInfo>   // only opens devices whose path contains a wanted vid_XXXX
pub struct HidDevice { /* HANDLE + OVERLAPPED event */ }   // Drop closes both
impl HidDevice {
  pub fn open(info: &HidInfo) -> Option<HidDevice>;
  pub fn write(&self, data: &[u8]) -> bool;                       // pads to out_len with zeros
  pub fn read_timeout(&self, buf: &mut [u8], ms: u32) -> Option<usize>; // None on timeout/error; CancelIoEx on timeout
  pub fn get_feature(&self, buf: &mut [u8]) -> bool;              // HidD_GetFeature
  pub fn flush(&self);                                             // HidD_FlushQueue
}
```
Enumeration: `SetupDiGetClassDevsW(HidD_GetHidGuid, DIGCF_PRESENT|DIGCF_DEVICEINTERFACE)` → interface paths; filter by lowercase `vid_xxxx` substring **before** opening; open with access 0 for caps (`HidD_GetAttributes`, `HidD_GetPreparsedData`, `HidP_GetCaps`, `HidD_FreePreparsedData`); I/O opens with `GENERIC_READ|GENERIC_WRITE`, falls back to `GENERIC_READ` (DualSense).

- [ ] **Step 1:** Implement; add `--list` to `main.rs` printing `vid:pid page:usage in/out/feat` for VIDs `046D, 04A5, 054C` (attach to parent console with `AttachConsole(ATTACH_PARENT_PROCESS)` for CLI flags).
- [ ] **Step 2:** Run `target\release\peribattery.exe --list` (from PowerShell, `| Out-String`). Expected: lines for `04a5:8006` with a page `>=FF00` or `00FF` and out 65, `046d:c547` page `ff00` usages `0001`/`0002`, `046d:0adf` page `ff00:0001` in/out 65. Record the output in the commit message.
- [ ] **Step 3:** Commit `feat: raw HID enumeration and I/O`.

### Task 3: ZOWIE EC-CW

**Files:** Create `src/devices/mod.rs`, `src/devices/zowie.rs`

**Interfaces — Consumes:** `hid::*`, `model::*`. **Produces:** `pub fn parse(reply: &[u8]) -> Option<(u16 /*mv*/, Option<u8> /*pct*/)>`, `pub fn poll(infos: &[HidInfo]) -> Option<DeviceStatus>` (key `"zowie"`, name `"ZOWIE EC2-CW"`).

- [ ] **Step 1: Tests**
```rust
#[test] fn parse_ok() { assert_eq!(parse(&[0x68,0x10,72,0xEC]), Some((4200,Some(72)))); }
#[test] fn parse_off() { assert_eq!(parse(&[0,0,0,0]), Some((0,None))); }
#[test] fn parse_garbage_rejected() { assert_eq!(parse(&[0x10,0x27,5]), None); } // 10000 mV
#[test] fn parse_bad_pct_falls_back_to_curve() { assert_eq!(parse(&[0x32,0x0F,0]), Some((3890,Some(50)))); }
#[test] fn parse_short() { assert_eq!(parse(&[0x68]), None); }
```
Curve: `[4200,4000,3890,3760,3680,3500]` → `[100,75,50,25,10,0]`. Valid mV: 0 or 3000..=4500.
- [ ] **Step 2:** Run → FAIL. Implement. Poll: PIDs `8006` (receiver) / `8005` (cable ⇒ charging); collection with `(usage_page >= 0xFF00 || usage_page == 0x00FF) && out_len == 65`; `flush`, `write(&[0x00,0xA5,0x03])`, `read_timeout(1000)`. mv 0 ⇒ `connected=false`.
- [ ] **Step 3:** Run → PASS; add `--once` to main printing `poll_all` results; run it with the mouse on: expect `ZOWIE EC2-CW xx%`. Commit `feat: ZOWIE EC-CW battery`.

### Task 4: Logitech HID++ (G915 TKL)

**Files:** Create `src/devices/hidpp.rs`

**Interfaces — Produces:**
```rust
pub trait Link { fn request(&mut self, idx: u8, feat: u8, func: u8, params: &[u8]) -> Result<[u8;16], HidppErr>; }
pub enum HidppErr { Timeout, Unsupported, Io }
pub fn feature_index(l: &mut dyn Link, idx: u8, feature: u16) -> Result<Option<u8>, HidppErr>;
pub enum Battery { Percent{pct:u8, charging:bool}, Millivolts{mv:u16, charging:bool}, None }
pub fn battery(l: &mut dyn Link, idx: u8) -> Result<Battery, HidppErr>;
pub fn poll(infos: &[HidInfo]) -> Vec<DeviceStatus>;   // key "logi:<idx>", kind from 0x0005 type, name from 0x0005
```
Wire format: long report `[0x11, idx, feat, (func<<4)|0x0A, params…]` 20 bytes, sent on usage `FF00:0002` collection; replies read on same collection, matched on `idx, feat, func/swid`; error replies `rx[2]==0xFF` (HID++2) or `0x8F` (HID++1) ⇒ `Unsupported`; read timeout 1000 ms ⇒ `Timeout` (device asleep). Root: feat 0, func 0, params `[hi, lo]` → `r[0]` index (0 = absent). Battery priority **0x1004** func 1 (`pct=r[0]`, charging `r[2] ∈ {1,2}`), **0x1000** func 0 (`pct=r[0]`, charging `r[2] ∈ {1,2,4}`), **0x1001** func 0 (`mv=BE(r[0],r[1])`, charging `r[2]&0x80`; pct by Li-ion curve `[4186,4067,3989,3922,3859,3811,3778,3751,3717,3654,3500]` → `[100,90,80,70,60,50,40,30,20,10,0]`). Name: 0x0005 func 0 → count, func 1(offset) chunks of 16; type func 2 (`0`=keyboard, `3`/`4`=mouse, `8`=headset). Receiver PIDs: `C539, C53A, C541, C545, C547, C54D`. Device indices 1..=6; Timeout on index ⇒ keep last known (poller handles) and `connected=false`.

- [ ] **Step 1: Tests with a `FakeLink` (map `(feat,func)` → reply):** `unified_battery_wins`, `battery_status_charging_4`, `voltage_be_and_charging_bit` (`[0x0F,0xA0,0x80]` ⇒ 4000 mV, charging), `absent_feature_root_returns_zero`, `timeout_propagates`.
- [ ] **Step 2:** FAIL → implement → PASS. `--once` shows `G915 TKL xx%` (if the keyboard turns out to be on Bluetooth, stop and report — spec risk).
- [ ] **Step 3:** Commit `feat: Logitech HID++ battery`.

### Task 5: Logitech G435 (046D:0ADF)

**Files:** Create `src/devices/g435.rs`

**Interfaces — Produces:** `pub fn crc16(d:&[u8])->u16`, `pub fn frame(cmd:u8, body:&[u8])->[u8;64]`, `pub fn pa(cat:u8, seq:u8, len:u8, payload:&[u8])->Vec<u8>`, `pub fn parse_mv(d:&[u8])->Option<u16>`, `pub fn percent(mv:u16)->u8`, `pub fn poll(infos:&[HidInfo])->Option<DeviceStatus>` (key `"g435"`, name `"Logitech G435"`).

Facts: CRC-16/XMODEM (poly 0x1021, init 0). Frame = `[cmd, body.len(), crc_lo, crc_hi, body…]` zero-padded to 64, CRC over the 64 bytes with CRC field zero. `pa` = `[0x50,0x41,cat,seq,len,payload…]`. Session (write = `[0x00] + frame`, collection `FF00:0001` in/out 65): `84 [01]` wait 50 ms → (guard: always send `85 [02]` at end) → `85 [01]` 50 → `80 init` 450 where init = `50 41 06 01 0D 00 25 34 12 5A 5A 01 00 00 00 F0 00 00 00` → `80 pa(0x0E,2,1,[E0])` 120 → `80 pa(0x0E,3,2,[E1,01])` 120 → `80 pa(0x0E,4,1,[E0])` 120 → `80 pa(0x06,5,1,[00,31])` up to 250 ms, scanning each input report with `parse_mv`. `parse_mv`: find `00 07 00 06 ?? 00 lo hi`, accept 2500..=4500. Curve `[4200,4150,4100,4050,4000,3950,3900,3850,3800,3750,3700,3650,3600,3550,3500,3400]` → `[100,95,90,85,80,75,70,62,55,45,35,25,15,10,5,0]`. No reply ⇒ `connected=false`.

- [ ] **Step 1: Tests:** `crc_xmodem_check` (`crc16(b"123456789") == 0x31C3`), `frame_layout` (`frame(0x84,&[1])`: `[0]=0x84,[1]=1,[4]=1`, bytes 2..4 equal crc of the frame with them zeroed), `parse_finds_pattern` (`[9,9,0,7,0,6,5,0,0x68,0x10]` ⇒ 4200), `parse_rejects_implausible` (mv 1000 ⇒ None), `percent_points` (4000⇒80, 3625⇒20 interpolated, 3300⇒0).
- [ ] **Step 2:** FAIL → implement → PASS. `--once` with headset on → `Logitech G435 xx%`; with headset off → `disconnected`. Listen to music during a `--once` run: audio must not cut.
- [ ] **Step 3:** Commit `feat: G435 battery`.

### Task 6: DualSense

**Files:** Create `src/devices/dualsense.rs`

**Interfaces — Produces:** `pub fn parse(report:&[u8]) -> Option<(u8 /*pct*/, bool /*charging*/)>`, `pub fn poll(infos:&[HidInfo]) -> Option<DeviceStatus>` (key `"ds"`, name `"DualSense"`/`"DualSense Edge"`).

Facts: VID `054C`, PIDs `0CE6`, `0DF2`; collection usage page `0x01` usage `0x05`. Read reports for up to 300 ms (`read_timeout` 50 ms loop), no writes, no feature reports. `0x01` with len ≥64 ⇒ status = `buf[53]`; `0x31` with len ≥78 ⇒ `buf[54]`; other ⇒ skip. Low nibble level → `min(level*10+5,100)`; high nibble `1|2` ⇒ charging (2 = full ⇒ pct 100). Device absent ⇒ `None` (hidden, not dimmed). Only basic BT reports ⇒ `None`.

- [ ] **Step 1: Tests:** `usb_level_and_charging` (64-byte buf, `[0]=0x01`, `[53]=0x17` ⇒ `(75,true)`), `bt_offset` (78-byte, `[0]=0x31`, `[54]=0x05` ⇒ `(55,false)`), `full_is_100` (`0x2A` ⇒ `(100,true)`), `short_report_none`.
- [ ] **Step 2:** FAIL → implement → PASS. USB-plug the controller, `--once` → `DualSense xx%`, with Steam open (must still work in Steam).
- [ ] **Step 3:** Commit `feat: DualSense battery`.

### Task 6b: Xbox-style controllers (Windows.Gaming.Input + XInput) and 8BitDo

**Files:** Create `src/devices/xbox.rs`; modify `Cargo.toml` (add `windows = { version = "0.58", features = ["Gaming_Input", "Devices_Power", "System_Power"] }`, windows-sys feature `Win32_UI_Input_XboxController`)

**Interfaces — Produces:** `pub fn pct_from_mwh(remaining:u32, full:u32) -> Option<u8>` (None if full==0), `pub fn classify(vid:u16, wgi:Option<(u32,u32,Status)>, xinput_wired:bool) -> Reading` where `enum Reading { Percent{pct:u8,charging:bool}, Unknown, Absent }`, `pub fn poll() -> Vec<DeviceStatus>` (key `"pad:<n>"`, kind Gamepad, name `"Controle Xbox"` or `"8BitDo"` when `RawGameController.HardwareVendorId == 0x2DC8`).

Rules: WGI status `NotPresent` ⇒ wired controller ⇒ `Unknown` shown as icon + "USB". VID `0x2DC8` ⇒ **always `Unknown`** (ignore WGI's constant 1000/1000). Otherwise `Percent`. XInput fallback only when WGI lists nothing for that slot: levels EMPTY/LOW/MEDIUM/FULL ⇒ 5/30/65/100. `percent: None` + `connected: true` renders "—". Initialise WGI lazily (first poll where `hid::enumerate` saw VID 045E or 2DC8, or XInput reports a slot) to avoid loading WinRT DLLs on PCs/sessions with no controller. Call `Gamepad::Gamepads()` from the poller thread (MTA, `RoInitialize(RO_INIT_MULTITHREADED)` once).

- [ ] **Step 1: Tests:** `mwh_to_pct` (`(600,1000)` ⇒ 60, `(0,0)` ⇒ None), `eightbitdo_never_fake_100` (`classify(0x2DC8, Some((1000,1000,Discharging)), true)` ⇒ `Unknown`), `xbox_charging` (`classify(0x045E, Some((500,1000,Charging)), false)` ⇒ `Percent{50,true}`), `wired_not_present_unknown`.
- [ ] **Step 2:** FAIL → implement → PASS. With 8BitDo dongle plugged and controller on: `--once` → `8BitDo —`. Measure RAM before/after a controller appears (report delta).
- [ ] **Step 3:** Commit `feat: Xbox-style controllers`.
- [ ] **Step 4 (investigation, only with Victor present):** Install USBPcap + Wireshark, capture the 8BitDo dongle (`2DC8:3109` or model-specific PID) while 8BitDo Ultimate Software shows the battery; look for a vendor-page HID request/response that changes with charge. If found, add `devices/eightbitdo.rs` with parser tests from the captured bytes; if not, keep "—".

### Task 7: Poller

**Files:** Create `src/poller.rs`; modify `src/devices/mod.rs`

**Interfaces — Produces:**
```rust
pub const WM_APP_STATUS: u32 = WM_APP + 1;   // lParam = Box<Vec<DeviceStatus>> raw ptr, UI takes ownership
pub fn interval_for(key: &str, gaming: bool) -> Duration;   // 60s / g435 120s; ×5 when gaming
pub fn merge(prev: &[DeviceStatus], fresh: Vec<DeviceStatus>) -> Vec<DeviceStatus>; // disconnected keeps prev percent/mv, connected=false
pub fn start(ui: HWND) -> PollerHandle;     // PollerHandle::wake() sets an auto-reset event
```
Thread loop: `WaitForSingleObject(wake_event, ms_until_next_due)`; on wake or due, run only due sources (wake ⇒ all), `merge`, and `PostMessageW(ui, WM_APP_STATUS, 0, ptr)` **only if** result `!=` last sent. Gaming = `SHQueryUserNotificationState` ∈ {`QUNS_RUNNING_D3D_FULL_SCREEN`, `QUNS_BUSY`}. Enumeration (`hid::enumerate(&[0x046D,0x04A5,0x054C])`) once per cycle and shared by the device polls.

Sources polled: zowie, logi, g435, ds, pad (pad every 60 s; 300 s gaming).

- [ ] **Step 1: Tests:** `interval_normal_and_gaming` (zowie 60/300 s, g435 120/600 s, pad 60/300 s), `merge_keeps_last_known_when_off`, `merge_new_device_appears`, `merge_dualsense_absent_removed`.
- [ ] **Step 2:** FAIL → implement → PASS. Commit `feat: background poller with game mode`.

### Task 8: Taskbar widget and rendering

**Files:** Create `src/ui/render.rs`, `src/ui/widget.rs`

**Interfaces — Consumes:** `WM_APP_STATUS`, `PollerHandle::wake`. **Produces:** `pub fn layout(items:&[DeviceStatus], dpi:u32) -> (i32 /*width*/, Vec<(i32 /*x*/, usize)>)`, `pub fn run(poller: PollerHandle) -> i32` (message loop).

Behaviour: hidden top-level controller window (receives `TaskbarCreated`, `WM_DEVICECHANGE`, `WM_POWERBROADCAST`, `WM_SETTINGCHANGE`); widget = `WS_CHILD` with `WS_EX_LAYERED|WS_EX_TOOLWINDOW|WS_EX_NOACTIVATE`, parent `Shell_TrayWnd`, placed `x = TrayNotifyWnd.left - width - 4*dpi/96`, full taskbar height. Draw into 32-bpp top-down DIB with premultiplied alpha, `UpdateLayeredWindow`. Per item: icon glyph (font 14*dpi/96 px) + `NN%` (Segoe UI Variable/Segoe UI 12*dpi/96 px), 8*dpi/96 gap. Colors: text white on dark theme / black on light (`SystemUsesLightTheme` under `HKCU\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize`, the value the taskbar follows), red `#E5484D` ≤15%, dimmed 45% alpha when `!connected`, bolt glyph before % when charging. Tooltip (`TOOLTIPS_CLASS`, `TTF_TRACK` not needed — standard) lists `name — 82% (4012 mV) — conectado/desligado`. Right-click menu: `Atualizar agora`, `Iniciar com o Windows` (check), `Ocultar ▸ <device>` (check per device), separator, `Sair`. Redraw only on status/theme/DPI change. If `Shell_TrayWnd` missing or taskbar vertical ⇒ fallback `Shell_NotifyIconW` with one icon showing the lowest % (rendered digits into a 16/32 px icon).

- [ ] **Step 1: Tests:** `layout_width_scales_with_dpi` (same items at 192 dpi ≈ 2× width at 96), `layout_skips_hidden`.
- [ ] **Step 2:** FAIL → implement → PASS. Run the release exe: widget appears left of `POR/PTB`. Manual checks: hover tooltip; menu; `Stop-Process -Name explorer` → widget returns within 5 s; switch light/dark theme → colors flip; open a fullscreen game → no stutter.
- [ ] **Step 3:** Commit `feat: taskbar widget`.

### Task 9: Install, autostart, single instance, footprint

**Files:** Create `src/settings.rs`; modify `src/main.rs`

**Interfaces — Produces:** `pub fn autostart_enabled()->bool`, `pub fn set_autostart(on:bool)`, `pub fn hidden()->Vec<String>`, `pub fn set_hidden(key:&str, hide:bool)`, `pub fn ensure_installed()->Option<PathBuf>` (copies self to install path if running elsewhere, returns new path), `pub fn run_value(path:&Path)->String` (quoted path).

main flow: `CreateMutexW("Local\\PeriBattery")` + `ERROR_ALREADY_EXISTS` ⇒ exit 0. First run (no `HKCU\Software\PeriBattery\Installed`) ⇒ `ensure_installed`, `set_autostart(true)`, mark installed, relaunch from install path if copied, exit. After UI is up: `SetProcessWorkingSetSize(GetCurrentProcess(), usize::MAX, usize::MAX)` once.

- [ ] **Step 1: Tests:** `run_value_quotes_path_with_spaces`.
- [ ] **Step 2:** FAIL → implement → PASS.
- [ ] **Step 3: Verify:** double-click exe twice → one process. `Get-ItemProperty HKCU:\Software\Microsoft\Windows\CurrentVersion\Run` shows `PeriBattery`. After 10 min idle: `Get-Process peribattery | Select WorkingSet64,PrivateMemorySize64,CPU` → private memory < 2 MB target (report actual), CPU time growth ≈ 0. Reboot or sign out/in → widget appears.
- [ ] **Step 4:** Write `README.md` (what it is, devices, build command, uninstall: menu ➜ uncheck autostart, delete `%LOCALAPPDATA%\PeriBattery`). Commit `feat: install, autostart, single instance`.
