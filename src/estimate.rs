//! Charging detection from voltage trend, and time-to-full / time-to-empty estimates.
//! Pure logic: the poller feeds readings, the UI formats the result.

use std::collections::VecDeque;

const MIN_SPACING_S: u64 = 300;
const MAX_SAMPLES: usize = 288; // 24 h at 5 min
const MIN_SPAN_S: u64 = 1200;
const MIN_DELTA_PCT: f32 = 3.0;

/// Charging guess from how a cell's voltage moves (devices that only report mV).
#[derive(Default, Clone, Debug)]
pub struct ChargeTrend {
    has: bool,
    charging: bool,
    peak: u16,
    trough: u16,
    pct_peak: u8,
    pct_trough: u8,
}

impl ChargeTrend {
    #[cfg(test)]
    pub fn update(&mut self, mv: u16) -> bool {
        self.update_with_pct(mv, None)
    }

    /// Voltage trend, plus the device's own percent when it reports one: on the ZOWIE dock the
    /// mouse's % rises well before the mV does. +50 mV or +3% ⇒ charging; −20 mV and −3% ⇒ not (just −20 mV without a %).
    pub fn update_with_pct(&mut self, mv: u16, pct: Option<u8>) -> bool {
        let p = pct.unwrap_or(0);
        if !self.has {
            self.has = true;
            self.charging = mv >= 4250; // a resting Li-ion cell never sits above ~4.2 V
            self.peak = mv;
            self.trough = mv;
            self.pct_peak = p;
            self.pct_trough = p;
        } else if self.charging {
            self.peak = self.peak.max(mv);
            self.pct_peak = self.pct_peak.max(p);
            // With a reported %, both must drop (single readings wobble while charging).
            let mv_dropped = mv + 20 <= self.peak;
            let stopped = if pct.is_some() { mv_dropped && p + 3 <= self.pct_peak } else { mv_dropped };
            if stopped {
                self.charging = false;
                self.trough = mv;
                self.pct_trough = p;
            }
        } else {
            self.trough = self.trough.min(mv);
            self.pct_trough = self.pct_trough.min(p);
            if mv >= self.trough + 50 || (pct.is_some() && p >= self.pct_trough + 3) {
                self.charging = true;
                self.peak = mv;
                self.pct_peak = p;
            }
        }
        self.charging
    }
}

/// (seconds, percent) samples of one charge or discharge run.
#[derive(Default, Clone, Debug)]
pub struct History {
    samples: VecDeque<(u64, f32)>,
    charging: bool,
}

impl History {
    /// Adds a reading; a change of direction starts a new run.
    pub fn push(&mut self, t: u64, pct: u8, charging: bool) {
        if charging != self.charging {
            self.samples.clear();
            self.charging = charging;
        }
        if let Some(&(last, _)) = self.samples.back() {
            if t < last + MIN_SPACING_S {
                return;
            }
        }
        if self.samples.len() == MAX_SAMPLES {
            self.samples.pop_front();
        }
        self.samples.push_back((t, pct as f32));
    }

    pub fn clear(&mut self) {
        self.samples.clear();
    }

    /// Least-squares slope in %/hour, when the run is long and moved enough.
    pub fn rate_per_hour(&self) -> Option<f32> {
        let (first, last) = (self.samples.front()?.0, self.samples.back()?.0);
        let span = last.saturating_sub(first);
        if span < MIN_SPAN_S || self.samples.len() < 3 {
            return None;
        }
        let n = self.samples.len() as f64;
        let mx = self.samples.iter().map(|s| (s.0 - first) as f64).sum::<f64>() / n;
        let my = self.samples.iter().map(|s| s.1 as f64).sum::<f64>() / n;
        let (mut sxy, mut sxx) = (0.0, 0.0);
        for &(t, p) in &self.samples {
            let dx = (t - first) as f64 - mx;
            sxy += dx * (p as f64 - my);
            sxx += dx * dx;
        }
        if sxx == 0.0 {
            return None;
        }
        let per_s = sxy / sxx;
        if (per_s * span as f64).abs() < MIN_DELTA_PCT as f64 {
            return None;
        }
        Some((per_s * 3600.0) as f32)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Source {
    Measured,
    Learned,
    Factory,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Estimate {
    /// Minutes until 100%.
    Full { minutes: u32, source: Source },
    /// Minutes until 0%.
    Empty { minutes: u32, source: Source },
    /// Charging, not enough data yet.
    Calculating,
}

/// Factory battery life in hours, by device key / name.
pub fn factory_hours(key: &str, name: &str) -> Option<f32> {
    let n = name.to_ascii_uppercase();
    match key {
        "zowie" => Some(70.0),
        "g435" => Some(18.0),
        "ds" => Some(6.0),
        k if k.starts_with("logi:") && n.contains("G915") => Some(40.0),
        _ => None,
    }
}

/// Minutes to drain `pct` at `rate` %/h (rate > 0).
pub fn minutes_to_empty(pct: u8, rate: f32) -> u32 {
    if rate <= 0.0 {
        return 0;
    }
    (pct as f32 / rate * 60.0).round() as u32
}

/// Minutes to fill from `pct` at `rate` %/h; above 80% charging runs at half speed.
pub fn minutes_to_full(pct: u8, rate: f32) -> u32 {
    if rate <= 0.0 {
        return 0;
    }
    let p = pct.min(100) as f32;
    let fast = (80.0 - p).max(0.0) / rate;
    let slow = (100.0 - p.max(80.0)) / (rate / 2.0);
    ((fast + slow) * 60.0).round() as u32
}

/// "~5 h 20 min" (rounded to 10 min above an hour), "~35 min".
pub fn format_minutes(m: u32) -> String {
    if m < 60 {
        return format!("~{m} min");
    }
    if m >= 600 {
        return format!("~{} h", (m + 30) / 60);
    }
    let r = (m + 5) / 10 * 10;
    match r % 60 {
        0 => format!("~{} h", r / 60),
        min => format!("~{} h {min} min", r / 60),
    }
}

/// Learned rates kept between runs: 50/50 moving average.
pub fn blend(old: Option<f32>, new: f32) -> f32 {
    match old {
        Some(o) => (o + new) / 2.0,
        None => new,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trend_detects_charge_and_unplug() {
        let mut t = ChargeTrend::default();
        assert!(!t.update(3700));
        assert!(!t.update(3690)); // trough 3690
        assert!(!t.update(3720)); // +30: noise
        assert!(t.update(3745)); // +55 over trough: charging
        assert!(t.update(3900)); // peak 3900
        assert!(t.update(3885)); // -15: noise
        assert!(!t.update(3878)); // -22 under peak: unplugged
    }

    #[test]
    fn trend_first_reading_above_full_cell_is_charging() {
        let mut t = ChargeTrend::default();
        assert!(t.update(4260));
    }

    #[test]
    fn history_rate_needs_span_and_delta() {
        let mut h = History::default();
        h.push(0, 50, false);
        h.push(600, 49, false);
        assert_eq!(h.rate_per_hour(), None); // too short
        let mut h = History::default();
        for i in 0..7u64 {
            h.push(i * 600, (60 - 2 * i) as u8, false); // -2% per 10 min = -12 %/h
        }
        let r = h.rate_per_hour().expect("rate");
        assert!((r + 12.0).abs() < 0.01, "{r}");
    }

    #[test]
    fn history_respects_spacing_and_resets_on_direction_change() {
        let mut h = History::default();
        h.push(0, 50, false);
        h.push(100, 49, false); // < 300 s: ignored
        assert_eq!(h.samples.len(), 1);
        h.push(400, 49, true); // started charging: new run
        assert_eq!(h.samples.len(), 1);
        assert!(h.charging);
    }

    #[test]
    fn history_caps_at_24h() {
        let mut h = History::default();
        for i in 0..400u64 {
            h.push(i * 300, 50, false);
        }
        assert_eq!(h.samples.len(), MAX_SAMPLES);
    }

    #[test]
    fn etas() {
        assert_eq!(minutes_to_empty(30, 6.0), 300);
        assert_eq!(minutes_to_full(60, 40.0), 30 + 60); // 20% at 40 %/h + 20% at 20 %/h
        assert_eq!(minutes_to_full(90, 40.0), 30);
    }

    #[test]
    fn factory_lives() {
        assert_eq!(factory_hours("zowie", "ZOWIE EC2-CW"), Some(70.0));
        assert_eq!(factory_hours("logi:c547:1", "G915 WIRELESS RGB MECHANICAL GAMING KEYBOARD"), Some(40.0));
        assert_eq!(factory_hours("g435", "Logitech G435"), Some(18.0));
        assert_eq!(factory_hours("ds", "DualSense"), Some(6.0));
        assert_eq!(factory_hours("pad:0", "8BitDo"), None);
    }

    #[test]
    fn formatting() {
        assert_eq!(format_minutes(35), "~35 min");
        assert_eq!(format_minutes(323), "~5 h 20 min");
        assert_eq!(format_minutes(120), "~2 h");
        assert_eq!(format_minutes(2165), "~36 h");
    }

    #[test]
    fn blend_moving_average() {
        assert_eq!(blend(None, 6.0), 6.0);
        assert_eq!(blend(Some(4.0), 6.0), 5.0);
    }
}

/// Per-device state kept by the poller between reads.
#[derive(Default, Clone, Debug)]
pub struct Tracker {
    trend: ChargeTrend,
    history: History,
    last_saved: Option<u64>,
}

/// Rates learned in earlier runs (%/h, both positive).
#[derive(Default, Clone, Copy, Debug)]
pub struct Learned {
    pub discharge: Option<f32>,
    pub charge: Option<f32>,
}

/// Devices that only report voltage: charging comes from the voltage trend.
fn uses_trend(key: &str) -> bool {
    key == "zowie" || key == "g435"
}

/// Fills `d.charging` (voltage trend where needed) and `d.estimate`. Returns
/// `Some((charging, rate %/h))` when a freshly measured rate should be saved.
pub fn annotate(d: &mut crate::model::DeviceStatus, tr: &mut Tracker, t: u64, learned: &Learned) -> Option<(bool, f32)> {
    const SAVE_EVERY_S: u64 = 1800;
    if !d.connected {
        tr.history.clear();
        d.estimate = None;
        return None;
    }
    if uses_trend(&d.key) {
        if let Some(mv) = d.millivolts {
            // ZOWIE reports its own %; the G435 % is derived from mV, so only mV counts there.
            let own_pct = if d.key == "zowie" { d.percent } else { None };
            let rising = tr.trend.update_with_pct(mv, own_pct);
            d.charging = d.charging || rising;
        }
    }
    let Some(pct) = d.percent else {
        d.estimate = None;
        return None;
    };
    tr.history.push(t, pct, d.charging);
    // A slope pointing the wrong way is noise, not a measurement.
    let measured = tr
        .history
        .rate_per_hour()
        .filter(|r| if d.charging { *r > 0.0 } else { *r < 0.0 })
        .map(f32::abs);
    let mut save = None;
    if let Some(r) = measured {
        if tr.last_saved.map_or(true, |s| t >= s + SAVE_EVERY_S) {
            tr.last_saved = Some(t);
            save = Some((d.charging, r));
        }
    }
    let pick = |m: Option<f32>, l: Option<f32>| m.map(|r| (r, Source::Measured)).or(l.map(|r| (r, Source::Learned)));
    d.estimate = if d.charging {
        match pick(measured, learned.charge) {
            Some((r, source)) => Some(Estimate::Full { minutes: coarse(minutes_to_full(pct, r)), source }),
            None => Some(Estimate::Calculating),
        }
    } else {
        pick(measured, learned.discharge)
            .or_else(|| factory_hours(&d.key, &d.name).map(|h| (100.0 / h, Source::Factory)))
            .map(|(r, source)| Estimate::Empty { minutes: coarse(minutes_to_empty(pct, r)), source })
    };
    save
}

#[cfg(test)]
mod annotate_tests {
    use super::*;
    use crate::model::{DeviceKind, DeviceStatus};

    fn dev(key: &str, name: &str, pct: u8, mv: Option<u16>, charging: bool) -> DeviceStatus {
        DeviceStatus { key: key.into(), kind: DeviceKind::Mouse, name: name.into(), percent: Some(pct), millivolts: mv, charging, connected: true, estimate: None }
    }

    #[test]
    fn factory_estimate_when_no_data() {
        let mut d = dev("zowie", "ZOWIE EC2-CW", 70, Some(3900), false);
        let mut tr = Tracker::default();
        annotate(&mut d, &mut tr, 0, &Learned::default());
        // 70% of a 70 h battery = 49 h
        assert_eq!(d.estimate, Some(Estimate::Empty { minutes: 2940, source: Source::Factory }));
    }

    #[test]
    fn learned_beats_factory() {
        let mut d = dev("zowie", "ZOWIE EC2-CW", 50, Some(3850), false);
        let learned = Learned { discharge: Some(10.0), charge: None };
        annotate(&mut d, &mut Tracker::default(), 0, &learned);
        assert_eq!(d.estimate, Some(Estimate::Empty { minutes: 300, source: Source::Learned }));
    }

    #[test]
    fn voltage_rise_marks_g435_charging_and_calculating() {
        let mut tr = Tracker::default();
        let mut d = dev("g435", "Logitech G435", 30, Some(3675), false);
        annotate(&mut d, &mut tr, 0, &Learned::default());
        assert!(!d.charging);
        let mut d = dev("g435", "Logitech G435", 33, Some(3740), false);
        annotate(&mut d, &mut tr, 120, &Learned::default());
        assert!(d.charging);
        assert_eq!(d.estimate, Some(Estimate::Calculating));
    }

    #[test]
    fn measured_charge_rate_is_used_and_reported_for_saving() {
        let mut tr = Tracker::default();
        let mut saved = None;
        for i in 0..5u64 {
            // DualSense reports charging itself: +10% every 10 min = 60 %/h
            let mut d = dev("ds", "DualSense", (20 + 10 * i) as u8, None, true);
            if let Some(s) = annotate(&mut d, &mut tr, i * 600, &Learned::default()) {
                saved = Some(s);
            }
            if i == 4 {
                // 60% → 80% at 60 %/h = 20 min, 80% → 100% at 30 %/h = 40 min
                assert_eq!(d.estimate, Some(Estimate::Full { minutes: 60, source: Source::Measured }));
            }
        }
        let (charging, rate) = saved.expect("rate to save");
        assert!(charging && (rate - 60.0).abs() < 0.1, "{rate}");
    }

    #[test]
    fn disconnected_has_no_estimate() {
        let mut d = dev("g435", "Logitech G435", 30, None, false);
        d.connected = false;
        annotate(&mut d, &mut Tracker::default(), 0, &Learned::default());
        assert_eq!(d.estimate, None);
    }
}

#[cfg(test)]
mod coarse_tests {
    use super::*;

    #[test]
    fn coarse_rounds_like_the_label() {
        assert_eq!(coarse(45), 45);
        assert_eq!(coarse(65), 70);
        assert_eq!(coarse(661), 660);
        assert_eq!(coarse(2165), 2160);
    }
}

/// Rounds minutes to what the tooltip shows, so tiny changes don't trigger repaints.
pub fn coarse(m: u32) -> u32 {
    match m {
        0..=59 => m,
        60..=599 => (m + 5) / 10 * 10,
        _ => (m + 30) / 60 * 60,
    }
}

#[cfg(test)]
mod pct_trend_tests {
    use super::*;

    #[test]
    fn mouse_reported_percent_rise_marks_charging() {
        // Real ZOWIE EC2-CW on its dock: 3713 mV 10% → 3738 mV 15% (+25 mV only).
        let mut t = ChargeTrend::default();
        assert!(!t.update_with_pct(3713, Some(10)));
        assert!(t.update_with_pct(3738, Some(15)));
        assert!(t.update_with_pct(3900, Some(60)));
        assert!(t.update_with_pct(3899, Some(59))); // 1% wobble: still charging
        assert!(t.update_with_pct(3895, Some(57))); // -3% but mV steady: a wobble, keep ⚡
        assert!(t.update_with_pct(3878, Some(59))); // -22 mV but % steady: keep ⚡
        assert!(!t.update_with_pct(3875, Some(56))); // both dropped: unplugged
    }

    #[test]
    fn percent_jitter_alone_is_not_charging() {
        let mut t = ChargeTrend::default();
        assert!(!t.update_with_pct(3700, Some(30)));
        assert!(!t.update_with_pct(3705, Some(32))); // +2%: noise
    }
}
