/// Fibonacci Time Zones — profile-gated temporal projection tool (ADR-002).
///
/// Profile gate: mature → Err; developing (max_bars=55) and nascent (max_bars=34) → Ok.
/// Anchor: candle with highest (high−low)/ATR₁₄ in the window — deterministic tie-break
///         (last occurrence when equal, favouring the most recent significant impulse).
/// Projects the Fibonacci sequence [1,1,2,3,5,8,13,21,34,55] as bar offsets from anchor.
/// Zones where fib_n > profile.time_zone_max_bars are omitted.

use crate::domain::{
    candle::OhlcvCandle,
    indicators::atr::compute_atr,
    smc::fib_profile::FibProfile,
};

const ATR_PERIOD: usize = 14;
const FIB_SEQUENCE: &[u32] = &[1, 1, 2, 3, 5, 8, 13, 21, 34, 55];

// ── Output types ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize)]
pub struct FibTimeZone {
    /// Fibonacci number for this zone (1, 1, 2, 3, 5, 8, …).
    pub fib_n:     u32,
    /// Timestamp of the projected candle; None when beyond the available window.
    pub ts:        Option<String>,
    /// true when the bar exists in the supplied candle data.
    pub in_window: bool,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct FibTimeZonesResult {
    /// Timestamp of the anchor (highest range/ATR₁₄ candle).
    pub anchor_ts:    String,
    /// (high−low)/ATR₁₄ ratio at the anchor.
    pub anchor_ratio: f64,
    pub profile:      String,
    /// true for nascent profile — lower signal confidence.
    pub exploratory:  bool,
    pub zones:        Vec<FibTimeZone>,
}

// ── Public entry point ────────────────────────────────────────────────────────

pub fn compute_fib_time_zones(
    raw:     &[OhlcvCandle],
    profile: &FibProfile,
) -> Result<FibTimeZonesResult, String> {
    if !profile.time_zone_enabled {
        return Err(format!(
            "fib_time_zones not available for profile=\"{}\". \
             Use \"developing\" or \"nascent\" to enable temporal projections.",
            profile.name
        ));
    }

    let min_required = ATR_PERIOD + 2;
    if raw.len() < min_required {
        return Err(format!(
            "not enough candles: need at least {} for ATR{}+anchor; got {}",
            min_required, ATR_PERIOD, raw.len()
        ));
    }

    let atr_pts = compute_atr(raw, ATR_PERIOD);

    // Find anchor: max (high−low)/ATR₁₄.
    // atr_pts[i] corresponds to raw[ATR_PERIOD + i] (from compute_atr contract).
    let (anchor_atr_idx, anchor_ratio) = atr_pts
        .iter()
        .enumerate()
        .map(|(i, pt)| {
            let raw_idx = ATR_PERIOD + i;
            let range = raw[raw_idx].high - raw[raw_idx].low;
            let ratio = if pt.atr > 1e-12 { range / pt.atr } else { 0.0_f64 };
            (i, ratio)
        })
        .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
        .ok_or_else(|| "no ATR points computed".to_string())?;

    let anchor_raw_idx = ATR_PERIOD + anchor_atr_idx;
    let anchor_ts      = raw[anchor_raw_idx].ts.clone();
    let max_bars       = profile.time_zone_max_bars as usize;

    let zones = FIB_SEQUENCE
        .iter()
        .filter(|&&n| n as usize <= max_bars)
        .map(|&fib_n| {
            let target_idx = anchor_raw_idx + fib_n as usize;
            let in_window  = target_idx < raw.len();
            FibTimeZone {
                fib_n,
                ts:        if in_window { Some(raw[target_idx].ts.clone()) } else { None },
                in_window,
            }
        })
        .collect();

    Ok(FibTimeZonesResult {
        anchor_ts,
        anchor_ratio,
        profile:     profile.name.clone(),
        exploratory: profile.exploratory,
        zones,
    })
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::candle::OhlcvCandle;

    fn make_candles(n: usize, range: f64) -> Vec<OhlcvCandle> {
        (0..n).map(|i| OhlcvCandle {
            ts: i.to_string(),
            open: 100.0, high: 100.0 + range, low: 100.0 - range, close: 100.0,
            volume: 1.0,
            mb_vol: None, ms_vol: None, lb_vol: None, ls_vol: None,
            mb_count: None, ms_count: None, lb_count: None, ls_count: None,
        }).collect()
    }

    #[test]
    fn mature_profile_returns_err() {
        let raw = make_candles(50, 1.0);
        let err = compute_fib_time_zones(&raw, &FibProfile::mature()).unwrap_err();
        assert!(err.contains("mature"), "error should name the blocked profile");
        assert!(
            err.contains("developing") || err.contains("nascent"),
            "error should suggest valid alternatives"
        );
    }

    #[test]
    fn developing_profile_returns_zones() {
        let raw = make_candles(50, 1.0);
        let res = compute_fib_time_zones(&raw, &FibProfile::developing()).unwrap();
        assert!(!res.zones.is_empty());
        assert_eq!(res.profile, "developing");
        assert!(!res.exploratory);
    }

    #[test]
    fn nascent_profile_is_exploratory_and_returns_zones() {
        let raw = make_candles(50, 1.0);
        let res = compute_fib_time_zones(&raw, &FibProfile::nascent()).unwrap();
        assert!(!res.zones.is_empty());
        assert_eq!(res.profile, "nascent");
        assert!(res.exploratory);
    }

    #[test]
    fn anchor_selected_by_highest_range_atr_ratio() {
        let mut raw = make_candles(60, 1.0);
        raw[30].high = 200.0;
        raw[30].low  = 50.0;
        raw[30].ts   = "impulse".to_string();
        let res = compute_fib_time_zones(&raw, &FibProfile::developing()).unwrap();
        assert_eq!(res.anchor_ts, "impulse");
        assert!(res.anchor_ratio > 5.0, "impulse candle should have high ratio");
    }

    #[test]
    fn zones_capped_by_profile_max_bars() {
        let raw = make_candles(100, 1.0);
        let res = compute_fib_time_zones(&raw, &FibProfile::nascent()).unwrap();
        for z in &res.zones {
            assert!(z.fib_n <= 34, "nascent max_bars=34; got fib_n={}", z.fib_n);
        }
    }

    #[test]
    fn future_zones_have_ts_none_and_in_window_false() {
        let raw = make_candles(20, 1.0);
        let res = compute_fib_time_zones(&raw, &FibProfile::developing()).unwrap();
        let future: Vec<_> = res.zones.iter().filter(|z| !z.in_window).collect();
        assert!(!future.is_empty(), "20 candles should produce future zones");
        for z in future {
            assert!(z.ts.is_none(), "future zone must have ts=None");
        }
    }
}
