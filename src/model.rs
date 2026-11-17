//! Device model shared by the poller and the UI.

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DeviceKind {
    Mouse,
    Keyboard,
    Headset,
    Gamepad,
}

#[derive(Clone, PartialEq, Debug)]
pub struct DeviceStatus {
    pub key: String,
    pub kind: DeviceKind,
    pub name: String,
    pub percent: Option<u8>,
    pub millivolts: Option<u16>,
    pub charging: bool,
    pub connected: bool,
}

/// Piecewise-linear voltage → percent. `volts` is descending and paired with `pcts`.
/// Values above the first point clamp to its percent, below the last to the last one.
pub fn interpolate(mv: u16, volts: &[u16], pcts: &[u8]) -> u8 {
    let n = volts.len().min(pcts.len());
    if n == 0 {
        return 0;
    }
    if mv >= volts[0] {
        return pcts[0].min(100);
    }
    for i in 1..n {
        if mv >= volts[i] {
            let (v_hi, v_lo) = (volts[i - 1] as u32, volts[i] as u32);
            let (p_hi, p_lo) = (pcts[i - 1] as u32, pcts[i] as u32);
            let span = v_hi - v_lo;
            if span == 0 {
                return p_lo.min(100) as u8;
            }
            let num = (p_hi.saturating_sub(p_lo)) * (mv as u32 - v_lo);
            return (p_lo + (num * 2 + span) / (span * 2)).min(100) as u8; // round half up
        }
    }
    pcts[n - 1].min(100)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interp_endpoints() {
        let v = [4200, 4000, 3500];
        let p = [100, 75, 0];
        assert_eq!(interpolate(4300, &v, &p), 100);
        assert_eq!(interpolate(3400, &v, &p), 0);
        assert_eq!(interpolate(4000, &v, &p), 75);
    }

    #[test]
    fn interp_midpoint() {
        assert_eq!(interpolate(4100, &[4200, 4000], &[100, 75]), 88); // 87.5 rounds half up
    }
}
