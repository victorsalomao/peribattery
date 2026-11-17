//! Draws the strip (icon + % per device) into a 32-bpp per-pixel-alpha DIB and pushes it with
//! UpdateLayeredWindow. Text is drawn white-on-black; glyph coverage becomes alpha.

use crate::model::{DeviceKind, DeviceStatus};

pub const LOW: u8 = 15;
const RED: u32 = 0xE5484D;
const GREEN_DARK: u32 = 0x6CCB5F;
const GREEN_LIGHT: u32 = 0x0F7B0F;
const DIM: u32 = 115; // ~45% opacity for asleep / disconnected

pub fn icon(kind: DeviceKind) -> char {
    match kind {
        DeviceKind::Mouse => '\u{E962}',
        DeviceKind::Keyboard => '\u{E765}',
        DeviceKind::Headset => '\u{E7F6}',
        DeviceKind::Gamepad => '\u{E7FC}',
    }
}
const BOLT: char = '\u{E945}';

/// One run of text drawn in one font and one color.
#[derive(Debug, Clone, PartialEq)]
pub struct Seg {
    pub text: Vec<u16>,
    pub icon: bool,
    pub color: u32, // 0xRRGGBB
    pub opacity: u32,
    pub gap_after: i32,
    /// Index of the device this segment belongs to.
    pub device: usize,
}

/// Devices to draw: everything not hidden by the user.
pub fn visible(items: &[DeviceStatus], hidden: &[String]) -> Vec<DeviceStatus> {
    items.iter().filter(|d| !hidden.contains(&d.key)).cloned().collect()
}

fn scale(v: i32, dpi: u32) -> i32 {
    v * dpi as i32 / 96
}

fn utf16(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

/// Segments for the devices, gaps already scaled to `dpi`.
pub fn segments(items: &[DeviceStatus], dpi: u32, light: bool) -> Vec<Seg> {
    let fg = if light { 0x000000 } else { 0xFFFFFF };
    let green = if light { GREEN_LIGHT } else { GREEN_DARK };
    let mut out = Vec::new();
    for (i, d) in items.iter().enumerate() {
        let gap = if i + 1 == items.len() { 0 } else { scale(12, dpi) };
        let opacity = if d.connected { 255 } else { DIM };
        let charging = d.charging && d.connected;
        let color = match d.percent {
            Some(p) if p <= LOW && !charging && d.connected => RED,
            _ => fg,
        };
        let label = d.percent.map(|p| format!("{p}%")).unwrap_or_else(|| "—".into());
        let seg = |text: Vec<u16>, icon: bool, color: u32, gap_after: i32| Seg { text, icon, color, opacity, gap_after, device: i };
        out.push(seg(vec![icon(d.kind) as u16], true, color, scale(5, dpi)));
        if charging {
            out.push(seg(utf16(&label), false, color, scale(2, dpi)));
            out.push(seg(vec![BOLT as u16], true, green, gap));
        } else {
            out.push(seg(utf16(&label), false, color, gap));
        }
    }
    out
}

/// x of each segment and the total width, given a text measurer.
pub fn layout(segs: &[Seg], dpi: u32, measure: &dyn Fn(&Seg) -> i32) -> (i32, Vec<i32>) {
    let pad = scale(6, dpi);
    let mut x = pad;
    let mut xs = Vec::with_capacity(segs.len());
    for s in segs {
        xs.push(x);
        x += measure(s) + s.gap_after;
    }
    (if segs.is_empty() { 0 } else { x + pad }, xs)
}

/// Horizontal (start, end) of each device on the strip, for per-device tooltips.
pub fn device_spans(segs: &[Seg], xs: &[i32], width: i32, devices: usize) -> Vec<(i32, i32)> {
    let mut spans = Vec::with_capacity(devices);
    for k in 0..devices {
        let start = if k == 0 {
            0
        } else {
            match segs.iter().position(|s| s.device == k) {
                // Split the gap between the previous device and this one in half.
                Some(i) if i > 0 => xs.get(i).copied().unwrap_or(width) - segs[i - 1].gap_after / 2,
                _ => width,
            }
        };
        if let Some(prev) = spans.last_mut() {
            let p: &mut (i32, i32) = prev;
            p.1 = start;
        }
        spans.push((start, width));
    }
    spans
}

/// Draws `items` into `hwnd` (a layered window); returns the width and each device's x span.
pub unsafe fn draw(hwnd: windows_sys::Win32::Foundation::HWND, items: &[DeviceStatus], dpi: u32, height: i32, light: bool) -> (i32, Vec<(i32, i32)>) {
    use std::mem::{size_of, zeroed};
    use std::ptr::{null, null_mut};
    use windows_sys::Win32::Foundation::{POINT, SIZE};
    use windows_sys::Win32::Graphics::Gdi::*;
    use windows_sys::Win32::UI::WindowsAndMessaging::{UpdateLayeredWindow, ULW_ALPHA};

    let segs = segments(items, dpi, light);
    let screen = GetDC(null_mut());
    let dc = CreateCompatibleDC(screen);
    ReleaseDC(null_mut(), screen);
    let text_font = font("Segoe UI Variable Text", -scale(12, dpi));
    let icon_font = font("Segoe Fluent Icons", -scale(15, dpi));

    let mut heights = vec![0i32; segs.len()];
    let measure = |s: &Seg| -> i32 {
        SelectObject(dc, if s.icon { icon_font } else { text_font });
        let mut sz: SIZE = zeroed();
        GetTextExtentPoint32W(dc, s.text.as_ptr(), s.text.len() as i32, &mut sz);
        sz.cx
    };
    let (width, xs) = layout(&segs, dpi, &measure);
    let spans = device_spans(&segs, &xs, width, items.len());
    for (i, s) in segs.iter().enumerate() {
        SelectObject(dc, if s.icon { icon_font } else { text_font });
        let mut sz: SIZE = zeroed();
        GetTextExtentPoint32W(dc, s.text.as_ptr(), s.text.len() as i32, &mut sz);
        heights[i] = sz.cy;
    }
    if width <= 0 || height <= 0 {
        DeleteObject(text_font);
        DeleteObject(icon_font);
        DeleteDC(dc);
        return (0, Vec::new());
    }

    let mut bi: BITMAPINFO = zeroed();
    bi.bmiHeader.biSize = size_of::<BITMAPINFOHEADER>() as u32;
    bi.bmiHeader.biWidth = width;
    bi.bmiHeader.biHeight = -height; // top-down
    bi.bmiHeader.biPlanes = 1;
    bi.bmiHeader.biBitCount = 32;
    bi.bmiHeader.biCompression = BI_RGB;
    let mut bits: *mut core::ffi::c_void = null_mut();
    let bmp = CreateDIBSection(dc, &bi, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
    if bmp.is_null() || bits.is_null() {
        DeleteObject(text_font);
        DeleteObject(icon_font);
        DeleteDC(dc);
        return (width, spans);
    }
    let old = SelectObject(dc, bmp);
    SetBkMode(dc, TRANSPARENT as _);
    SetTextColor(dc, 0x00FF_FFFF);
    for (i, s) in segs.iter().enumerate() {
        SelectObject(dc, if s.icon { icon_font } else { text_font });
        TextOutW(dc, xs[i], (height - heights[i]) / 2, s.text.as_ptr(), s.text.len() as i32);
    }
    GdiFlush();

    // Color/opacity per column, then coverage → premultiplied BGRA.
    let mut cols = vec![(0xFFFFFFu32, 255u32); width as usize];
    for (i, s) in segs.iter().enumerate() {
        let end = segs.get(i + 1).map(|_| xs[i + 1]).unwrap_or(width).min(width);
        for c in xs[i].max(0)..end {
            cols[c as usize] = (s.color, s.opacity);
        }
    }
    let px = std::slice::from_raw_parts_mut(bits as *mut u32, (width * height) as usize);
    for (i, p) in px.iter_mut().enumerate() {
        let (color, opacity) = cols[i % width as usize];
        let cov = ((*p >> 16) & 0xFF).max((*p >> 8) & 0xFF).max(*p & 0xFF);
        let a = cov * opacity / 255;
        *p = if a == 0 {
            0x0100_0000 // alpha 1: invisible but still receives the mouse (tooltip, menu)
        } else {
            let ch = |shift: u32| ((color >> shift) & 0xFF) * a / 255;
            (a << 24) | (ch(16) << 16) | (ch(8) << 8) | ch(0)
        };
    }

    let size = SIZE { cx: width, cy: height };
    let origin = POINT { x: 0, y: 0 };
    let blend = BLENDFUNCTION { BlendOp: AC_SRC_OVER as u8, BlendFlags: 0, SourceConstantAlpha: 255, AlphaFormat: AC_SRC_ALPHA as u8 };
    UpdateLayeredWindow(hwnd, null_mut(), null(), &size, dc, &origin, 0, &blend, ULW_ALPHA);

    SelectObject(dc, old);
    DeleteObject(bmp);
    DeleteObject(text_font);
    DeleteObject(icon_font);
    DeleteDC(dc);
    (width, spans)
}

unsafe fn font(face: &str, height: i32) -> windows_sys::Win32::Graphics::Gdi::HFONT {
    use windows_sys::Win32::Graphics::Gdi::*;
    let mut lf: LOGFONTW = std::mem::zeroed();
    lf.lfHeight = height;
    lf.lfWeight = FW_NORMAL as _;
    lf.lfQuality = ANTIALIASED_QUALITY as _; // grayscale AA: coverage maps cleanly to alpha
    for (dst, src) in lf.lfFaceName.iter_mut().zip(face.encode_utf16().take(31)) {
        *dst = src;
    }
    CreateFontIndirectW(&lf)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dev(key: &str, kind: DeviceKind, pct: Option<u8>, charging: bool, connected: bool) -> DeviceStatus {
        DeviceStatus { key: key.into(), kind, name: key.into(), percent: pct, millivolts: None, charging, connected, estimate: None }
    }

    fn items() -> Vec<DeviceStatus> {
        vec![dev("g435", DeviceKind::Headset, Some(31), false, true), dev("zowie", DeviceKind::Mouse, Some(10), false, true)]
    }

    #[test]
    fn layout_width_scales_with_dpi() {
        let m96 = |s: &Seg| s.text.len() as i32 * 7;
        let m192 = |s: &Seg| s.text.len() as i32 * 14;
        let (w1, _) = layout(&segments(&items(), 96, false), 96, &m96);
        let (w2, _) = layout(&segments(&items(), 192, false), 192, &m192);
        assert!(w1 > 0);
        assert_eq!(w2, w1 * 2);
    }

    #[test]
    fn layout_skips_hidden() {
        let v = visible(&items(), &["zowie".to_string()]);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].key, "g435");
    }

    #[test]
    fn low_battery_is_red_and_charging_has_bolt() {
        let s = segments(&[dev("zowie", DeviceKind::Mouse, Some(10), false, true)], 96, false);
        assert_eq!(s[1].color, RED);
        let s = segments(&[dev("zowie", DeviceKind::Mouse, Some(10), true, true)], 96, false);
        assert_ne!(s[1].color, RED); // charging: not alarming
        assert!(s.iter().any(|x| x.text == vec![BOLT as u16]));
    }

    #[test]
    fn disconnected_is_dimmed_and_unknown_shows_dash() {
        let s = segments(&[dev("g435", DeviceKind::Headset, Some(31), false, false)], 96, false);
        assert!(s.iter().all(|x| x.opacity == DIM));
        let s = segments(&[dev("pad:0", DeviceKind::Gamepad, None, false, true)], 96, false);
        assert_eq!(String::from_utf16_lossy(&s[1].text), "—");
    }
}

#[cfg(test)]
mod redraw_tests {
    use super::*;

    #[test]
    fn redraw_when_dpi_height_or_theme_changes() {
        assert!(needs_redraw(None, (96, 48, false)));
        assert!(!needs_redraw(Some((96, 48, false)), (96, 48, false)));
        assert!(needs_redraw(Some((96, 48, false)), (144, 72, false)));
        assert!(needs_redraw(Some((96, 48, false)), (96, 48, true)));
    }
}

/// (dpi, taskbar height, light theme) the bitmap was drawn for vs. now.
pub fn needs_redraw(last: Option<(u32, i32, bool)>, now: (u32, i32, bool)) -> bool {
    last != Some(now)
}

#[cfg(test)]
mod span_tests {
    use super::*;

    fn d(key: &str, kind: DeviceKind, charging: bool) -> DeviceStatus {
        DeviceStatus { key: key.into(), kind, name: key.into(), percent: Some(50), millivolts: None, charging, connected: true, estimate: None }
    }

    #[test]
    fn one_span_per_device_covering_its_segments() {
        let items = [d("g435", DeviceKind::Headset, false), d("zowie", DeviceKind::Mouse, true)];
        let segs = segments(&items, 96, false);
        let measure = |s: &Seg| s.text.len() as i32 * 10;
        let (width, xs) = layout(&segs, 96, &measure);
        let spans = device_spans(&segs, &xs, width, items.len());
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[0].0, 0); // first device starts at the left edge (incl. padding)
        assert_eq!(spans[0].1, spans[1].0); // contiguous: no dead zone between devices
        assert_eq!(spans[1].1, width); // last device reaches the right edge
        assert!(spans[1].0 > xs[1] && spans[1].0 <= xs[2]); // boundary sits in the gap before the mouse icon
    }
}
