//! Device report a tester can send back: every HID collection on the machine (vendor, product,
//! usage) plus what PeriBattery currently reads. No paths or serial numbers.

type Row = (u16, u16, u16, u16, Option<String>);

pub fn vendor(vid: u16) -> &'static str {
    match vid {
        0x046D => "Logitech",
        0x04A5 => "BenQ/ZOWIE",
        0x054C => "Sony",
        0x045E => "Microsoft",
        0x2DC8 => "8BitDo",
        0x1532 => "Razer",
        0x1B1C => "Corsair",
        0x1038 => "SteelSeries",
        0x0951 | 0x03F0 => "HyperX/HP",
        0x3554 => "VXE/ATK",
        0x258A => "Sinowealth",
        0x373B => "Lamzu/Compx",
        0x1E7D => "ROCCAT",
        0x0B05 => "ASUS",
        0x1209 => "Generic",
        0x0B0E => "Jabra",
        0x1395 => "EPOS/Sennheiser",
        0x05AC => "Apple",
        0x0D8C => "C-Media",
        0x3434 => "Keychron",
        0x320F => "Glorious",
        0x093A => "Pixart",
        _ => "?",
    }
}

pub fn line(vid: u16, pid: u16, page: u16, usage: u16, name: Option<&str>) -> String {
    let mut s = format!("{vid:04x}:{pid:04x}  {:<11}  {page:04x}:{usage:04x}  {}", vendor(vid), name.unwrap_or("(sem nome)"));
    if crate::VIDS.contains(&vid) {
        s.push_str("  [suportado]");
    }
    s
}

pub fn unique(mut rows: Vec<Row>) -> Vec<Row> {
    rows.sort_by(|a, b| (a.0, a.1, a.2, a.3).cmp(&(b.0, b.1, b.2, b.3)));
    rows.dedup_by(|a, b| (a.0, a.1, a.2, a.3) == (b.0, b.1, b.2, b.3));
    rows
}

/// Full report text (slow: reads every device once, so call it off the UI thread).
pub fn build(devices: &[crate::model::DeviceStatus]) -> String {
    let mut out = format!("PeriBattery {} — relatório de dispositivos ({})\r\n\r\n", env!("CARGO_PKG_VERSION"), crate::diag::now_hms());
    out.push_str("O que o PeriBattery está lendo:\r\n");
    if devices.is_empty() {
        out.push_str("  (nada)\r\n");
    }
    for d in devices {
        let line = crate::ui::widget::device_line(d).replace('\r', "").replace('\n', " — ");
        out.push_str(&format!("  {:?}: {line}\r\n", d.kind));
    }
    out.push_str("\r\nDispositivos HID conectados (fabricante, produto, página:uso):\r\n");
    let rows = unique(
        crate::hid::enumerate_where(|_| true)
            .iter()
            .map(|i| (i.vid, i.pid, i.usage_page, i.usage, crate::hid::product_name(i)))
            .collect(),
    );
    for (vid, pid, page, usage, name) in &rows {
        out.push_str(&format!("  {}\r\n", line(*vid, *pid, *page, *usage, name.as_deref())));
    }
    out.push_str("\r\nCole este texto numa issue do GitHub com o nome do seu mouse, teclado, fone e controle.\r\n");
    out
}

/// Writes the report next to the installed exe and opens it in Notepad.
pub fn write_and_open(devices: &[crate::model::DeviceStatus]) {
    let Some(base) = std::env::var_os("LOCALAPPDATA") else { return };
    let dir = std::path::PathBuf::from(base).join(crate::settings::APP);
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("relatorio.txt");
    // UTF-8 BOM so Notepad shows the accents right on every Windows build.
    if std::fs::write(&path, [&[0xEF, 0xBB, 0xBF][..], build(devices).as_bytes()].concat()).is_ok() {
        let _ = std::process::Command::new("notepad.exe").arg(&path).spawn();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vendor_names() {
        assert_eq!(vendor(0x046D), "Logitech");
        assert_eq!(vendor(0x1532), "Razer");
        assert_eq!(vendor(0x1234), "?");
    }

    #[test]
    fn line_marks_supported_vendors() {
        let l = line(0x046D, 0xC547, 0xFF00, 0x0002, Some("USB Receiver"));
        assert_eq!(l, "046d:c547  Logitech     ff00:0002  USB Receiver  [suportado]");
        let l = line(0x1532, 0x00B7, 0x0001, 0x0002, None);
        assert_eq!(l, "1532:00b7  Razer        0001:0002  (sem nome)");
    }

    #[test]
    fn rows_are_deduplicated_and_sorted() {
        let rows = vec![(0x1532, 0x00B7, 1, 2, None), (0x046D, 0xC547, 1, 2, None), (0x1532, 0x00B7, 1, 2, None)];
        let out = unique(rows);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].0, 0x046D);
    }
}
