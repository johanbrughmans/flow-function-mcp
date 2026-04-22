/// Multi-Anchor Fibonacci Confluence — Story #37.
///
/// Scores fib levels across five independent reference frames:
///   P1 — current structure range (last opposing BOS/CHoCH levels from `structure`)
///   P2 — swing pivot range (last swing high + swing low from `pivots`)
///   P3 — session range (highest high / lowest low of the candle window)
///   P4 — previous day high/low (passed in from caller)
///   P5 — previous week high/low (passed in from caller)
///
/// Score = count of anchors whose fib level for a given (direction, ratio)
/// falls within ATR-based tolerance of the reference. Max score = 5.
///
/// P1 fallback: if no BOS/CHoCH found, use last `fallback_weeks` candles range.
/// Tolerance: ATR(14) × profile multiplier (mature=0.20, developing=0.25, nascent=0.35).

use chrono::Utc;

use crate::domain::{
    candle::OhlcvCandle,
    indicators::atr::compute_atr,
    smc::{
        fib_profile::FibProfile,
        pivots::{detect_pivots, PivotKind},
        structure::compute_structure,
    },
    types::Direction,
};

const ATR_PERIOD: usize = 14;

const RATIOS: &[(f64, &str)] = &[
    (0.236, "0.236"),
    (0.382, "0.382"),
    (0.500, "0.500"),
    (0.618, "0.618"),
    (0.786, "0.786"),
];

// ── Output types ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize)]
pub struct FibZone {
    pub ratio:     f64,
    pub direction: String,     // "up" | "down"
    pub level:     f64,        // zone center price
    pub zone_low:  f64,        // center − tolerance
    pub zone_high: f64,        // center + tolerance
    pub score:     u8,         // 1–5
    pub anchors:   Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct MultiAnchorFibResponse {
    pub pair:        String,
    pub tf:          String,
    pub zones:       Vec<FibZone>,
    pub p1_source:   String,   // "structure" | "fallback_Nw"
    pub computed_at: String,
}

// ── Public entry point ────────────────────────────────────────────────────────

pub fn compute_multi_anchor_fib(
    candles:        &[OhlcvCandle],
    pdh:            Option<f64>,
    pdl:            Option<f64>,
    pwh:            Option<f64>,
    pwl:            Option<f64>,
    profile:        &FibProfile,
    min_score:      u8,
    fallback_weeks: u32,
    tf_str:         &str,
    pair_str:       &str,
) -> MultiAnchorFibResponse {
    let empty = |p1_source: &str| MultiAnchorFibResponse {
        pair:        pair_str.to_string(),
        tf:          tf_str.to_string(),
        zones:       vec![],
        p1_source:   p1_source.to_string(),
        computed_at: Utc::now().to_rfc3339(),
    };

    if candles.is_empty() {
        return empty("none");
    }

    let tol           = compute_tolerance(candles, profile);
    let current_close = candles.last().unwrap().close;

    let (p1_high, p1_low, p1_source) = derive_p1(candles, fallback_weeks, tf_str);
    let p2 = derive_p2(candles);
    let p3 = derive_p3(candles);

    let mut zones: Vec<FibZone> = Vec::new();

    for &(ratio, _) in RATIOS {
        for dir in ["up", "down"] {
            let mut anchor_levels: Vec<(&str, f64)> = Vec::new();

            let fib_level = |lo: f64, hi: f64| -> f64 {
                if dir == "up" { lo + (hi - lo) * ratio } else { hi - (hi - lo) * ratio }
            };

            if p1_high > p1_low {
                anchor_levels.push(("p1_structure", fib_level(p1_low, p1_high)));
            }
            if let Some((h, l)) = p2 {
                anchor_levels.push(("p2_swing", fib_level(l, h)));
            }
            if let Some((h, l)) = p3 {
                anchor_levels.push(("p3_session", fib_level(l, h)));
            }
            if let (Some(dh), Some(dl)) = (pdh, pdl) {
                if dh > dl { anchor_levels.push(("p4_daily", fib_level(dl, dh))); }
            }
            if let (Some(wh), Some(wl)) = (pwh, pwl) {
                if wh > wl { anchor_levels.push(("p5_weekly", fib_level(wl, wh))); }
            }

            if let Some((score, center, anchors)) = score_confluence(&anchor_levels, tol) {
                if score >= min_score {
                    zones.push(FibZone {
                        ratio,
                        direction:  dir.to_string(),
                        level:      round5(center),
                        zone_low:   round5((center - tol).max(0.0)),
                        zone_high:  round5(center + tol),
                        score,
                        anchors,
                    });
                }
            }
        }
    }

    zones.sort_by(|a, b| {
        let da = (a.level - current_close).abs();
        let db = (b.level - current_close).abs();
        da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
    });

    MultiAnchorFibResponse {
        pair:        pair_str.to_string(),
        tf:          tf_str.to_string(),
        zones,
        p1_source,
        computed_at: Utc::now().to_rfc3339(),
    }
}

// ── Anchor derivation ─────────────────────────────────────────────────────────

fn derive_p1(candles: &[OhlcvCandle], fallback_weeks: u32, tf_str: &str) -> (f64, f64, String) {
    let events        = compute_structure(candles);
    let last_bullish  = events.iter().rev().find(|e| e.direction == Direction::Bullish).map(|e| e.level);
    let last_bearish  = events.iter().rev().find(|e| e.direction == Direction::Bearish).map(|e| e.level);

    if let (Some(bl), Some(brl)) = (last_bullish, last_bearish) {
        let hi = bl.max(brl);
        let lo = bl.min(brl);
        if lo > 0.0 && (hi - lo) / lo > 0.001 {
            return (hi, lo, "structure".to_string());
        }
    }

    let n     = (weeks_to_candles(fallback_weeks, tf_str) as usize).max(2);
    let start = candles.len().saturating_sub(n);
    let slice = &candles[start..];
    let hi    = slice.iter().map(|c| c.high).fold(f64::NEG_INFINITY, f64::max);
    let lo    = slice.iter().map(|c| c.low).fold(f64::INFINITY, f64::min);
    (hi, lo, format!("fallback_{}w", fallback_weeks))
}

fn derive_p2(candles: &[OhlcvCandle]) -> Option<(f64, f64)> {
    let pivots    = detect_pivots(candles);
    let last_high = pivots.iter().rev().find(|p| p.kind == PivotKind::High).map(|p| p.price);
    let last_low  = pivots.iter().rev().find(|p| p.kind == PivotKind::Low).map(|p| p.price);
    match (last_high, last_low) {
        (Some(h), Some(l)) if h > l => Some((h, l)),
        _ => None,
    }
}

fn derive_p3(candles: &[OhlcvCandle]) -> Option<(f64, f64)> {
    if candles.is_empty() { return None; }
    let hi = candles.iter().map(|c| c.high).fold(f64::NEG_INFINITY, f64::max);
    let lo = candles.iter().map(|c| c.low).fold(f64::INFINITY, f64::min);
    if hi > lo { Some((hi, lo)) } else { None }
}

// ── Confluence scoring ────────────────────────────────────────────────────────

fn score_confluence(
    anchor_levels: &[(&str, f64)],
    tol:           f64,
) -> Option<(u8, f64, Vec<String>)> {
    if anchor_levels.is_empty() { return None; }

    let mut best_score   = 0u8;
    let mut best_center  = anchor_levels[0].1;
    let mut best_anchors: Vec<String> = vec![];

    for &(_, reference) in anchor_levels {
        let matching: Vec<_> = anchor_levels.iter()
            .filter(|&&(_, l)| (l - reference).abs() <= tol)
            .collect();
        let score = matching.len() as u8;
        if score > best_score {
            best_score  = score;
            best_center = matching.iter().map(|&&(_, l)| l).sum::<f64>() / matching.len() as f64;
            best_anchors = matching.iter().map(|&&(name, _)| name.to_string()).collect();
        }
    }

    if best_score > 0 { Some((best_score, best_center, best_anchors)) } else { None }
}

// ── Tolerance ─────────────────────────────────────────────────────────────────

fn compute_tolerance(candles: &[OhlcvCandle], profile: &FibProfile) -> f64 {
    let atr_mult = match profile.name.as_str() {
        "mature"     => 0.20_f64,
        "developing" => 0.25,
        _            => 0.35,
    };
    let atr_pts = compute_atr(candles, ATR_PERIOD);
    if let Some(last) = atr_pts.last() {
        if last.atr > 0.0 { return last.atr * atr_mult; }
    }
    candles.last().map_or(0.0, |c| c.close * profile.cluster_tolerance)
}

fn weeks_to_candles(weeks: u32, tf: &str) -> u32 {
    let per_week: u32 = match tf {
        "1h" => 7 * 24,
        "4h" => 7 * 6,
        "1d" => 7,
        "1w" => 1,
        _    => 7,
    };
    weeks * per_week
}

fn round5(x: f64) -> f64 { (x * 100_000.0).round() / 100_000.0 }

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn c(ts: &str, open: f64, high: f64, low: f64, close: f64) -> OhlcvCandle {
        OhlcvCandle {
            ts: ts.to_string(), open, high, low, close, volume: 100.0,
            mb_vol: None, ms_vol: None, lb_vol: None, ls_vol: None,
            mb_count: None, ms_count: None, lb_count: None, ls_count: None,
        }
    }

    #[test]
    fn empty_candles_returns_empty_response() {
        let r = compute_multi_anchor_fib(&[], None, None, None, None, &FibProfile::mature(), 2, 6, "1d", "BTCEUR");
        assert!(r.zones.is_empty());
        assert_eq!(r.p1_source, "none");
    }

    #[test]
    fn score_confluence_single_anchor_is_score_one() {
        let anchors = vec![("p1_structure", 100.0_f64)];
        let (score, center, names) = score_confluence(&anchors, 1.0).unwrap();
        assert_eq!(score, 1);
        assert!((center - 100.0).abs() < 1e-9);
        assert_eq!(names, vec!["p1_structure"]);
    }

    #[test]
    fn score_confluence_two_within_tol_is_score_two() {
        let anchors = vec![("p1_structure", 100.0_f64), ("p2_swing", 100.5)];
        let (score, _, _) = score_confluence(&anchors, 1.0).unwrap();
        assert_eq!(score, 2);
    }

    #[test]
    fn score_confluence_two_outside_tol_is_score_one() {
        let anchors = vec![("p1_structure", 100.0_f64), ("p2_swing", 102.0)];
        let (score, _, _) = score_confluence(&anchors, 1.0).unwrap();
        assert_eq!(score, 1);
    }

    #[test]
    fn min_score_filters_low_scoring_zones() {
        let candles: Vec<_> = (0..30)
            .map(|i| c(&i.to_string(), 95.0 + i as f64, 100.0 + i as f64, 90.0, 98.0 + i as f64))
            .collect();
        let r = compute_multi_anchor_fib(&candles, None, None, None, None, &FibProfile::mature(), 5, 6, "1d", "BTCEUR");
        assert!(r.zones.iter().all(|z| z.score >= 5));
    }

    #[test]
    fn p1_fallback_used_when_no_structure_events() {
        let candles: Vec<_> = (0..10).map(|i| c(&i.to_string(), 1.0, 1.0, 1.0, 1.0)).collect();
        let r = compute_multi_anchor_fib(&candles, None, None, None, None, &FibProfile::mature(), 2, 6, "1d", "BTCEUR");
        assert!(r.p1_source.starts_with("fallback_"));
    }

    #[test]
    fn zones_sorted_by_distance_from_close() {
        let candles: Vec<_> = (0..50)
            .map(|i| c(&i.to_string(), 95.0 + i as f64 * 0.5, 100.0 + i as f64 * 0.5, 90.0 + i as f64 * 0.5, 98.0 + i as f64 * 0.5))
            .collect();
        let r = compute_multi_anchor_fib(&candles, None, None, None, None, &FibProfile::nascent(), 1, 6, "1d", "BTCEUR");
        let close = candles.last().unwrap().close;
        for w in r.zones.windows(2) {
            let da = (w[0].level - close).abs();
            let db = (w[1].level - close).abs();
            assert!(da <= db, "zones not sorted: {:.2} > {:.2}", da, db);
        }
    }

    #[test]
    fn weeks_to_candles_1d_is_7_per_week() {
        assert_eq!(weeks_to_candles(6, "1d"), 42);
    }

    #[test]
    fn weeks_to_candles_1h_is_168_per_week() {
        assert_eq!(weeks_to_candles(1, "1h"), 168);
    }
}
