/// Multi-anchor Fibonacci confluence — indicator-level walk-forward backtest (ADR-017, Story #39).
///
/// For each historical candle t in [window_size, len − lookahead_bars):
///   1. Compute zones using candles[t+1−window_size..=t]  (data AVAILABLE at t)
///   2. For each zone with score ≥ min_score:
///      a. Examine candles[t+1..=t+lookahead_bars]       (for VALIDATION only — no look-ahead)
///      b. Record: touched, respected, bars_to_touch, penetration_pct
///   3. Aggregate observations per score bucket
///
/// Primary output signal: `monotonic_respect` — does respect_rate increase with score?
/// If false, the scoring model is miscalibrated and must not be used in a strategy.
///
/// P4/P5 approximation: at each historical point, previous-day and previous-week
/// ranges are derived from a fixed lookback on the chart candles themselves. This
/// avoids cross-TF fetches during walk-forward but makes the P4/P5 less precise
/// than the production fib_confluence path. Documented limitation.

use chrono::Utc;

use crate::domain::{
    candle::OhlcvCandle,
    smc::{fib_profile::FibProfile, multi_anchor_fib::{compute_multi_anchor_fib, FibZone}},
};

// ── Output types ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize)]
pub struct ScoreBucketMetrics {
    pub score:               u8,
    pub direction:           String,      // "up" | "down" | "all"
    pub n_zones:             usize,
    pub n_touched:           usize,
    pub n_respected:         usize,
    pub touch_rate:          f64,
    pub respect_rate:        f64,
    pub avg_bars_to_touch:   Option<f64>,
    pub avg_penetration_pct: Option<f64>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct FibConfluenceBacktestResponse {
    pub pair:              String,
    pub tf:                String,
    pub candles_analyzed:  usize,
    pub total_zones:       usize,
    pub buckets:           Vec<ScoreBucketMetrics>,
    pub monotonic_respect: bool,
    pub window_size:       usize,
    pub lookahead_bars:    usize,
    pub min_score:         u8,
    pub profile:           String,
    pub computed_at:       String,
}

// ── Internal observation ──────────────────────────────────────────────────────

struct ZoneObservation {
    score:           u8,
    direction:       String,
    touched:         bool,
    respected:       bool,
    bars_to_touch:   Option<usize>,
    penetration_pct: Option<f64>,
}

// ── Public entry point ────────────────────────────────────────────────────────

pub fn backtest_multi_anchor_fib(
    candles:        &[OhlcvCandle],
    profile:        &FibProfile,
    min_score:      u8,
    window_size:    usize,
    lookahead_bars: usize,
    fallback_weeks: u32,
    tf_str:         &str,
    pair_str:       &str,
) -> FibConfluenceBacktestResponse {
    let empty = || FibConfluenceBacktestResponse {
        pair:              pair_str.to_string(),
        tf:                tf_str.to_string(),
        candles_analyzed:  0,
        total_zones:       0,
        buckets:           vec![],
        monotonic_respect: false,
        window_size,
        lookahead_bars,
        min_score,
        profile:           profile.name.clone(),
        computed_at:       Utc::now().to_rfc3339(),
    };

    if candles.len() < window_size + lookahead_bars + 1 {
        return empty();
    }

    let mut observations: Vec<ZoneObservation> = Vec::new();
    let start = window_size.saturating_sub(1);
    let end   = candles.len().saturating_sub(lookahead_bars);

    for t in start..end {
        let history = &candles[t + 1 - window_size..=t];
        let future  = &candles[t + 1..=t + lookahead_bars];
        let close_t = candles[t].close;

        let (pdh, pdl) = prev_period_hl(candles, t, p4_bars(tf_str));
        let (pwh, pwl) = prev_period_hl(candles, t, p5_bars(tf_str));

        let resp = compute_multi_anchor_fib(
            history, pdh, pdl, pwh, pwl,
            profile, min_score, fallback_weeks, tf_str, pair_str,
        );

        for zone in resp.zones {
            observations.push(validate_zone(&zone, future, close_t));
        }
    }

    let buckets = aggregate(&observations);
    let monotonic_respect = check_monotonic(&buckets);
    let total_zones = observations.len();

    FibConfluenceBacktestResponse {
        pair:              pair_str.to_string(),
        tf:                tf_str.to_string(),
        candles_analyzed:  end.saturating_sub(start),
        total_zones,
        buckets,
        monotonic_respect,
        window_size,
        lookahead_bars,
        min_score,
        profile:           profile.name.clone(),
        computed_at:       Utc::now().to_rfc3339(),
    }
}

// ── Zone validation ───────────────────────────────────────────────────────────

fn validate_zone(zone: &FibZone, future: &[OhlcvCandle], close_t: f64) -> ZoneObservation {
    let mut touched         = false;
    let mut bars_to_touch   = None;
    let mut first_touch_idx = 0usize;

    for (i, c) in future.iter().enumerate() {
        if c.low <= zone.zone_high && c.high >= zone.zone_low {
            touched         = true;
            bars_to_touch   = Some(i + 1);
            first_touch_idx = i;
            break;
        }
    }

    let mut respected       = false;
    let mut penetration_pct = None;

    if touched {
        let zone_above_close = zone.level > close_t;
        let remaining        = &future[first_touch_idx..];
        let break_threshold  = 0.005_f64;

        let broke = if zone_above_close {
            remaining.iter().any(|c| c.close > zone.zone_high * (1.0 + break_threshold))
        } else {
            remaining.iter().any(|c| c.close < zone.zone_low * (1.0 - break_threshold))
        };
        respected = !broke;

        let max_pen = if zone_above_close {
            remaining.iter().map(|c| c.high - zone.zone_high).fold(f64::NEG_INFINITY, f64::max)
        } else {
            remaining.iter().map(|c| zone.zone_low - c.low).fold(f64::NEG_INFINITY, f64::max)
        };
        if max_pen > 0.0 && zone.level > 0.0 {
            penetration_pct = Some(max_pen / zone.level * 100.0);
        }
    }

    ZoneObservation {
        score:           zone.score,
        direction:       zone.direction.clone(),
        touched,
        respected,
        bars_to_touch,
        penetration_pct,
    }
}

// ── Aggregation ───────────────────────────────────────────────────────────────

fn aggregate(obs: &[ZoneObservation]) -> Vec<ScoreBucketMetrics> {
    let mut buckets: Vec<ScoreBucketMetrics> = Vec::new();
    for score in 1u8..=5 {
        for dir in ["up", "down", "all"] {
            let filtered: Vec<_> = obs.iter()
                .filter(|o| o.score == score && (dir == "all" || o.direction == dir))
                .collect();

            if filtered.is_empty() { continue; }

            let n_zones     = filtered.len();
            let n_touched   = filtered.iter().filter(|o| o.touched).count();
            let n_respected = filtered.iter().filter(|o| o.respected).count();

            let touch_rate   = n_touched as f64 / n_zones as f64;
            let respect_rate = if n_touched > 0 { n_respected as f64 / n_touched as f64 } else { 0.0 };

            let bars: Vec<f64> = filtered.iter().filter_map(|o| o.bars_to_touch.map(|b| b as f64)).collect();
            let avg_bars_to_touch = if bars.is_empty() { None } else { Some(bars.iter().sum::<f64>() / bars.len() as f64) };

            let pens: Vec<f64> = filtered.iter().filter_map(|o| o.penetration_pct).collect();
            let avg_penetration_pct = if pens.is_empty() { None } else { Some(pens.iter().sum::<f64>() / pens.len() as f64) };

            buckets.push(ScoreBucketMetrics {
                score,
                direction: dir.to_string(),
                n_zones,
                n_touched,
                n_respected,
                touch_rate:  round4(touch_rate),
                respect_rate: round4(respect_rate),
                avg_bars_to_touch: avg_bars_to_touch.map(round2),
                avg_penetration_pct: avg_penetration_pct.map(round4),
            });
        }
    }
    buckets
}

fn check_monotonic(buckets: &[ScoreBucketMetrics]) -> bool {
    let mut by_score: Vec<(u8, f64)> = buckets.iter()
        .filter(|b| b.direction == "all" && b.n_touched > 0)
        .map(|b| (b.score, b.respect_rate))
        .collect();
    by_score.sort_by_key(|(s, _)| *s);

    if by_score.len() < 2 { return false; }

    by_score.windows(2).all(|w| w[1].1 >= w[0].1)
}

// ── Timeframe-aware P4/P5 approximation ───────────────────────────────────────

fn p4_bars(tf: &str) -> usize {
    match tf {
        "1h" => 24,
        "4h" => 6,
        "1d" => 1,
        "1w" => 1,
        _    => 1,
    }
}

fn p5_bars(tf: &str) -> usize {
    match tf {
        "1h" => 168,
        "4h" => 42,
        "1d" => 7,
        "1w" => 1,
        _    => 7,
    }
}

fn prev_period_hl(candles: &[OhlcvCandle], t: usize, bars: usize) -> (Option<f64>, Option<f64>) {
    if bars == 0 || t == 0 || t < bars { return (None, None); }
    let slice = &candles[t - bars..t];
    if slice.is_empty() { return (None, None); }
    let h = slice.iter().map(|c| c.high).fold(f64::NEG_INFINITY, f64::max);
    let l = slice.iter().map(|c| c.low).fold(f64::INFINITY, f64::min);
    if h.is_finite() && l.is_finite() && h > l { (Some(h), Some(l)) } else { (None, None) }
}

fn round2(x: f64) -> f64 { (x * 100.0).round() / 100.0 }
fn round4(x: f64) -> f64 { (x * 10_000.0).round() / 10_000.0 }

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

    fn sine_history(n: usize, base: f64, amplitude: f64) -> Vec<OhlcvCandle> {
        (0..n).map(|i| {
            let phase = (i as f64) * 0.12;
            let mid = base + (phase.sin()) * amplitude;
            c(&format!("{}", i), mid, mid + 2.0, mid - 2.0, mid + 0.2)
        }).collect()
    }

    #[test]
    fn too_few_candles_returns_empty_response() {
        let candles = sine_history(50, 100.0, 5.0);
        let r = backtest_multi_anchor_fib(
            &candles, &FibProfile::mature(), 2, 200, 20, 6, "1d", "BTCEUR",
        );
        assert_eq!(r.total_zones, 0);
        assert_eq!(r.candles_analyzed, 0);
        assert!(!r.monotonic_respect);
    }

    #[test]
    fn walk_forward_does_not_look_ahead() {
        let candles = sine_history(500, 100.0, 10.0);
        let r = backtest_multi_anchor_fib(
            &candles, &FibProfile::nascent(), 1, 100, 10, 6, "1d", "BTCEUR",
        );
        assert_eq!(r.candles_analyzed, 500 - 10 - (100 - 1));
        assert!(r.total_zones > 0);
    }

    #[test]
    fn buckets_aggregate_by_score() {
        let candles = sine_history(300, 100.0, 8.0);
        let r = backtest_multi_anchor_fib(
            &candles, &FibProfile::nascent(), 1, 80, 15, 6, "1d", "BTCEUR",
        );
        for b in &r.buckets {
            assert!(b.n_touched <= b.n_zones);
            assert!(b.n_respected <= b.n_touched);
            assert!(b.touch_rate >= 0.0 && b.touch_rate <= 1.0);
            assert!(b.respect_rate >= 0.0 && b.respect_rate <= 1.0);
        }
    }

    #[test]
    fn validate_zone_untouched_when_future_far_above() {
        let zone = FibZone {
            ratio: 0.5, direction: "up".into(), level: 100.0,
            zone_low: 99.0, zone_high: 101.0, score: 3, anchors: vec![],
        };
        let future: Vec<_> = (0..10).map(|i| c(&i.to_string(), 200.0, 201.0, 199.0, 200.5)).collect();
        let obs = validate_zone(&zone, &future, 95.0);
        assert!(!obs.touched);
        assert!(!obs.respected);
        assert_eq!(obs.bars_to_touch, None);
    }

    #[test]
    fn validate_zone_touched_and_respected_when_price_bounces() {
        let zone = FibZone {
            ratio: 0.5, direction: "up".into(), level: 100.0,
            zone_low: 99.0, zone_high: 101.0, score: 3, anchors: vec![],
        };
        let future = vec![
            c("0", 102.0, 102.5, 100.5, 101.5),
            c("1", 101.5, 102.0, 100.0, 100.5),
            c("2", 100.5, 103.0, 100.0, 102.5),
            c("3", 102.5, 104.0, 102.0, 103.5),
        ];
        let obs = validate_zone(&zone, &future, 105.0);
        assert!(obs.touched);
        assert!(obs.respected);
        assert_eq!(obs.bars_to_touch, Some(1));
    }

    #[test]
    fn validate_zone_broken_when_price_closes_through() {
        let zone = FibZone {
            ratio: 0.5, direction: "down".into(), level: 100.0,
            zone_low: 99.0, zone_high: 101.0, score: 3, anchors: vec![],
        };
        let future = vec![
            c("0", 100.5, 101.0, 99.5, 100.2),
            c("1", 100.0, 100.5, 98.0, 98.5),
            c("2", 98.5, 98.8, 94.0, 94.5),
        ];
        let obs = validate_zone(&zone, &future, 102.0);
        assert!(obs.touched);
        assert!(!obs.respected);
    }

    #[test]
    fn prev_period_hl_respects_bounds() {
        let candles: Vec<_> = (0..10).map(|i| c(&i.to_string(), 100.0 + i as f64, 105.0 + i as f64, 95.0 + i as f64, 101.0 + i as f64)).collect();
        let (h, l) = prev_period_hl(&candles, 5, 3);
        assert_eq!(h, Some(109.0));
        assert_eq!(l, Some(97.0));
    }

    #[test]
    fn prev_period_hl_none_when_insufficient_history() {
        let candles: Vec<_> = (0..10).map(|i| c(&i.to_string(), 100.0, 101.0, 99.0, 100.0)).collect();
        let (h, l) = prev_period_hl(&candles, 2, 5);
        assert_eq!(h, None);
        assert_eq!(l, None);
    }

    #[test]
    fn check_monotonic_detects_monotonic_sequence() {
        let buckets = vec![
            ScoreBucketMetrics { score: 2, direction: "all".into(), n_zones: 10, n_touched: 10, n_respected: 5,
                touch_rate: 1.0, respect_rate: 0.5, avg_bars_to_touch: None, avg_penetration_pct: None },
            ScoreBucketMetrics { score: 3, direction: "all".into(), n_zones: 10, n_touched: 10, n_respected: 6,
                touch_rate: 1.0, respect_rate: 0.6, avg_bars_to_touch: None, avg_penetration_pct: None },
            ScoreBucketMetrics { score: 4, direction: "all".into(), n_zones: 10, n_touched: 10, n_respected: 8,
                touch_rate: 1.0, respect_rate: 0.8, avg_bars_to_touch: None, avg_penetration_pct: None },
        ];
        assert!(check_monotonic(&buckets));
    }

    #[test]
    fn check_monotonic_false_when_respect_rate_decreases() {
        let buckets = vec![
            ScoreBucketMetrics { score: 2, direction: "all".into(), n_zones: 10, n_touched: 10, n_respected: 8,
                touch_rate: 1.0, respect_rate: 0.8, avg_bars_to_touch: None, avg_penetration_pct: None },
            ScoreBucketMetrics { score: 3, direction: "all".into(), n_zones: 10, n_touched: 10, n_respected: 5,
                touch_rate: 1.0, respect_rate: 0.5, avg_bars_to_touch: None, avg_penetration_pct: None },
        ];
        assert!(!check_monotonic(&buckets));
    }
}
