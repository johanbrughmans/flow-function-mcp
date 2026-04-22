/// Multi-anchor Fibonacci confluence — indicator-level walk-forward backtest (ADR-017, Stories #39 + #43).
///
/// Three validation tracks in a single walk-forward pass:
///
///   LEGACY (v1, kept for audit): naive respect = price did not close beyond zone.
///     Regime-sensitive; retained as `legacy_respect`.
///
///   TRACK A — Author-faithful reaction validation.
///     Tests the narrow claim: "multi-anchor agreement marks locations where price
///     reacts measurably". Reaction = volatility spike, volume spike, or wick
///     prominence after first touch. Uses OHLCV-intrinsic data only.
///     Gate: `monotonic_reaction` — does reaction_rate rise with score?
///
///   TRACK B — Contextual respect test.
///     Respected is conditioned on arrival direction (from_above / from_below) and
///     trend regime at observation time (bullish / bearish / neutral, from
///     compute_structure). Reports respect_rate per (score × arrival × trend)
///     quadrant with n ≥ 30 statistical floor.
///     Gate: `any_calibrated_bucket` — does any (arrival, trend) quadrant show
///     monotonic respect across scores?
///
/// Lookahead default tightened from 20 → 10 bars.

use chrono::Utc;

use crate::domain::{
    candle::OhlcvCandle,
    indicators::atr::compute_atr,
    smc::{
        fib_profile::FibProfile,
        multi_anchor_fib::{compute_multi_anchor_fib, FibZone},
        structure::compute_structure,
    },
    types::Direction,
};

const MIN_BUCKET_N: usize = 30;
const ATR_PERIOD: usize = 14;
const REACTION_WINDOW: usize = 5;
const BASELINE_WINDOW: usize = 20;
const VOL_Z_THRESHOLD: f64 = 1.0;
const VOLUME_Z_THRESHOLD: f64 = 1.0;
const WICK_PROMINENCE_THRESHOLD: f64 = 2.0;

// ── Output types ──────────────────────────────────────────────────────────────

/// Legacy v1 metric — retained for regression comparison (ADR-017 audit trail).
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
pub struct ReactionBucket {
    pub score:                     u8,
    pub n_zones:                   usize,
    pub n_reacted:                 usize,
    pub reaction_rate:             f64,
    pub median_volatility_spike_z: Option<f64>,
    pub median_volume_spike_z:     Option<f64>,
    pub median_wick_prominence:    Option<f64>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ContextualBucket {
    pub score:        u8,
    pub arrival:      String,  // "from_above" | "from_below"
    pub trend:        String,  // "bullish" | "bearish" | "neutral"
    pub n_zones:      usize,
    pub n_touched:    usize,
    pub n_respected:  usize,
    pub respect_rate: f64,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct FibConfluenceBacktestResponse {
    pub pair:              String,
    pub tf:                String,
    pub candles_analyzed:  usize,
    pub total_zones:       usize,
    pub window_size:       usize,
    pub lookahead_bars:    usize,
    pub min_score:         u8,
    pub profile:           String,

    pub legacy_respect:            Vec<ScoreBucketMetrics>,
    pub legacy_monotonic_respect:  bool,

    pub track_a_reaction:   Vec<ReactionBucket>,
    pub monotonic_reaction: bool,

    pub track_b_contextual:    Vec<ContextualBucket>,
    pub any_calibrated_bucket: bool,

    pub computed_at: String,
}

// ── Internal observation ──────────────────────────────────────────────────────

struct ZoneObservation {
    score:               u8,
    direction:           String,

    touched:             bool,
    legacy_respected:    bool,
    bars_to_touch:       Option<usize>,
    penetration_pct:     Option<f64>,

    volatility_spike_z:  Option<f64>,
    volume_spike_z:      Option<f64>,
    wick_prominence:     Option<f64>,
    reacted:             Option<bool>,

    arrival:             Option<String>,
    trend:               String,
    contextual_respected: Option<bool>,
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
        pair:                     pair_str.to_string(),
        tf:                       tf_str.to_string(),
        candles_analyzed:         0,
        total_zones:              0,
        window_size,
        lookahead_bars,
        min_score,
        profile:                  profile.name.clone(),
        legacy_respect:           vec![],
        legacy_monotonic_respect: false,
        track_a_reaction:         vec![],
        monotonic_reaction:       false,
        track_b_contextual:       vec![],
        any_calibrated_bucket:    false,
        computed_at:              Utc::now().to_rfc3339(),
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

        let trend = derive_trend_regime(history);
        let baseline = baseline_stats(history);

        let (pdh, pdl) = prev_period_hl(candles, t, p4_bars(tf_str));
        let (pwh, pwl) = prev_period_hl(candles, t, p5_bars(tf_str));

        let resp = compute_multi_anchor_fib(
            history, pdh, pdl, pwh, pwl,
            profile, min_score, fallback_weeks, tf_str, pair_str,
        );

        for zone in resp.zones {
            observations.push(validate_zone(&zone, future, close_t, &baseline, &trend));
        }
    }

    let legacy_respect = aggregate_legacy(&observations);
    let legacy_monotonic_respect = check_legacy_monotonic(&legacy_respect);

    let track_a_reaction = aggregate_reaction(&observations);
    let monotonic_reaction = check_reaction_monotonic(&track_a_reaction);

    let track_b_contextual = aggregate_contextual(&observations);
    let any_calibrated_bucket = check_any_calibrated_bucket(&track_b_contextual);

    FibConfluenceBacktestResponse {
        pair:                     pair_str.to_string(),
        tf:                       tf_str.to_string(),
        candles_analyzed:         end.saturating_sub(start),
        total_zones:              observations.len(),
        window_size,
        lookahead_bars,
        min_score,
        profile:                  profile.name.clone(),
        legacy_respect,
        legacy_monotonic_respect,
        track_a_reaction,
        monotonic_reaction,
        track_b_contextual,
        any_calibrated_bucket,
        computed_at:              Utc::now().to_rfc3339(),
    }
}

// ── Baseline stats (ATR + volume mean/std from history) ───────────────────────

struct BaselineStats {
    atr_mean:    f64,
    atr_std:     f64,
    vol_mean:    f64,
    vol_std:     f64,
}

fn baseline_stats(history: &[OhlcvCandle]) -> BaselineStats {
    let atr_pts = compute_atr(history, ATR_PERIOD);
    let atr_tail: Vec<f64> = atr_pts.iter()
        .rev()
        .take(BASELINE_WINDOW)
        .map(|p| p.atr)
        .filter(|a| *a > 0.0)
        .collect();
    let (atr_mean, atr_std) = mean_std(&atr_tail);

    let vol_tail: Vec<f64> = history.iter()
        .rev()
        .take(BASELINE_WINDOW)
        .map(|c| c.volume)
        .filter(|v| *v > 0.0)
        .collect();
    let (vol_mean, vol_std) = mean_std(&vol_tail);

    BaselineStats { atr_mean, atr_std, vol_mean, vol_std }
}

fn mean_std(xs: &[f64]) -> (f64, f64) {
    if xs.is_empty() { return (0.0, 0.0); }
    let n = xs.len() as f64;
    let mean = xs.iter().sum::<f64>() / n;
    let var = xs.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / n;
    (mean, var.sqrt())
}

// ── Trend regime ──────────────────────────────────────────────────────────────

fn derive_trend_regime(history: &[OhlcvCandle]) -> String {
    let events = compute_structure(history);
    match events.last() {
        Some(e) if e.direction == Direction::Bullish => "bullish".to_string(),
        Some(e) if e.direction == Direction::Bearish => "bearish".to_string(),
        _ => "neutral".to_string(),
    }
}

// ── Zone validation (all three tracks in one pass) ────────────────────────────

fn validate_zone(
    zone:     &FibZone,
    future:   &[OhlcvCandle],
    close_t:  f64,
    baseline: &BaselineStats,
    trend:    &str,
) -> ZoneObservation {
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

    let mut legacy_respected = false;
    let mut penetration_pct  = None;

    let mut volatility_spike_z = None;
    let mut volume_spike_z     = None;
    let mut wick_prominence    = None;
    let mut reacted            = None;

    let mut arrival              = None;
    let mut contextual_respected = None;

    if touched {
        let zone_above_close = zone.level > close_t;
        let zone_below_close = zone.level < close_t;
        let remaining        = &future[first_touch_idx..];
        let break_threshold  = 0.005_f64;

        let broke_legacy = if zone_above_close {
            remaining.iter().any(|c| c.close > zone.zone_high * (1.0 + break_threshold))
        } else {
            remaining.iter().any(|c| c.close < zone.zone_low * (1.0 - break_threshold))
        };
        legacy_respected = !broke_legacy;

        let max_pen = if zone_above_close {
            remaining.iter().map(|c| c.high - zone.zone_high).fold(f64::NEG_INFINITY, f64::max)
        } else {
            remaining.iter().map(|c| zone.zone_low - c.low).fold(f64::NEG_INFINITY, f64::max)
        };
        if max_pen > 0.0 && zone.level > 0.0 {
            penetration_pct = Some(max_pen / zone.level * 100.0);
        }

        let first_touch_candle = &future[first_touch_idx];
        wick_prominence = Some(compute_wick_prominence(first_touch_candle));

        let post = &future[first_touch_idx..future.len().min(first_touch_idx + REACTION_WINDOW)];
        if !post.is_empty() {
            let post_ranges: Vec<f64> = post.iter().map(|c| c.high - c.low).collect();
            let post_atr = post_ranges.iter().sum::<f64>() / post_ranges.len() as f64;
            if baseline.atr_std > 0.0 {
                volatility_spike_z = Some((post_atr - baseline.atr_mean) / baseline.atr_std);
            }

            let post_vol: f64 = post.iter().map(|c| c.volume).sum::<f64>() / post.len() as f64;
            if baseline.vol_std > 0.0 {
                volume_spike_z = Some((post_vol - baseline.vol_mean) / baseline.vol_std);
            }
        }

        let vol_hit  = volatility_spike_z.map_or(false, |z| z > VOL_Z_THRESHOLD);
        let vl_hit   = volume_spike_z.map_or(false, |z| z > VOLUME_Z_THRESHOLD);
        let wick_hit = wick_prominence.map_or(false, |w| w > WICK_PROMINENCE_THRESHOLD);
        reacted = Some(vol_hit || vl_hit || wick_hit);

        if zone_above_close {
            arrival = Some("from_below".to_string());
            let reversed_down = remaining.iter()
                .skip(1)
                .any(|c| c.close < zone.zone_low);
            contextual_respected = Some(reversed_down && !broke_legacy);
        } else if zone_below_close {
            arrival = Some("from_above".to_string());
            let reversed_up = remaining.iter()
                .skip(1)
                .any(|c| c.close > zone.zone_high);
            contextual_respected = Some(reversed_up && !broke_legacy);
        }
    }

    ZoneObservation {
        score:                zone.score,
        direction:            zone.direction.clone(),
        touched,
        legacy_respected,
        bars_to_touch,
        penetration_pct,
        volatility_spike_z,
        volume_spike_z,
        wick_prominence,
        reacted,
        arrival,
        trend:                trend.to_string(),
        contextual_respected,
    }
}

fn compute_wick_prominence(c: &OhlcvCandle) -> f64 {
    let body = (c.close - c.open).abs();
    let upper_wick = c.high - c.close.max(c.open);
    let lower_wick = c.close.min(c.open) - c.low;
    let max_wick = upper_wick.max(lower_wick);
    if body > f64::EPSILON { max_wick / body } else { max_wick / (c.close * 0.001).max(f64::EPSILON) }
}

// ── Aggregation: legacy (v1) ──────────────────────────────────────────────────

fn aggregate_legacy(obs: &[ZoneObservation]) -> Vec<ScoreBucketMetrics> {
    let mut buckets: Vec<ScoreBucketMetrics> = Vec::new();
    for score in 1u8..=5 {
        for dir in ["up", "down", "all"] {
            let filtered: Vec<_> = obs.iter()
                .filter(|o| o.score == score && (dir == "all" || o.direction == dir))
                .collect();
            if filtered.is_empty() { continue; }

            let n_zones     = filtered.len();
            let n_touched   = filtered.iter().filter(|o| o.touched).count();
            let n_respected = filtered.iter().filter(|o| o.legacy_respected).count();
            let touch_rate   = n_touched as f64 / n_zones as f64;
            let respect_rate = if n_touched > 0 { n_respected as f64 / n_touched as f64 } else { 0.0 };

            let bars: Vec<f64> = filtered.iter().filter_map(|o| o.bars_to_touch.map(|b| b as f64)).collect();
            let avg_bars_to_touch = if bars.is_empty() { None } else { Some(round2(bars.iter().sum::<f64>() / bars.len() as f64)) };

            let pens: Vec<f64> = filtered.iter().filter_map(|o| o.penetration_pct).collect();
            let avg_penetration_pct = if pens.is_empty() { None } else { Some(round4(pens.iter().sum::<f64>() / pens.len() as f64)) };

            buckets.push(ScoreBucketMetrics {
                score,
                direction:           dir.to_string(),
                n_zones,
                n_touched,
                n_respected,
                touch_rate:          round4(touch_rate),
                respect_rate:        round4(respect_rate),
                avg_bars_to_touch,
                avg_penetration_pct,
            });
        }
    }
    buckets
}

fn check_legacy_monotonic(buckets: &[ScoreBucketMetrics]) -> bool {
    let mut by_score: Vec<(u8, f64)> = buckets.iter()
        .filter(|b| b.direction == "all" && b.n_touched >= MIN_BUCKET_N)
        .map(|b| (b.score, b.respect_rate))
        .collect();
    by_score.sort_by_key(|(s, _)| *s);
    if by_score.len() < 2 { return false; }
    by_score.windows(2).all(|w| w[1].1 >= w[0].1)
}

// ── Aggregation: Track A (reaction) ───────────────────────────────────────────

fn aggregate_reaction(obs: &[ZoneObservation]) -> Vec<ReactionBucket> {
    let mut buckets: Vec<ReactionBucket> = Vec::new();
    for score in 1u8..=5 {
        let filtered: Vec<_> = obs.iter()
            .filter(|o| o.score == score && o.touched)
            .collect();
        if filtered.is_empty() { continue; }

        let n_zones   = filtered.len();
        let n_reacted = filtered.iter().filter(|o| o.reacted == Some(true)).count();
        let reaction_rate = n_reacted as f64 / n_zones as f64;

        let vol_zs: Vec<f64> = filtered.iter().filter_map(|o| o.volatility_spike_z).collect();
        let vl_zs:  Vec<f64> = filtered.iter().filter_map(|o| o.volume_spike_z).collect();
        let wicks:  Vec<f64> = filtered.iter().filter_map(|o| o.wick_prominence).collect();

        buckets.push(ReactionBucket {
            score,
            n_zones,
            n_reacted,
            reaction_rate:             round4(reaction_rate),
            median_volatility_spike_z: median(&vol_zs).map(round4),
            median_volume_spike_z:     median(&vl_zs).map(round4),
            median_wick_prominence:    median(&wicks).map(round4),
        });
    }
    buckets
}

fn check_reaction_monotonic(buckets: &[ReactionBucket]) -> bool {
    let mut by_score: Vec<(u8, f64)> = buckets.iter()
        .filter(|b| b.n_zones >= MIN_BUCKET_N)
        .map(|b| (b.score, b.reaction_rate))
        .collect();
    by_score.sort_by_key(|(s, _)| *s);
    if by_score.len() < 2 { return false; }
    by_score.windows(2).all(|w| w[1].1 >= w[0].1)
}

// ── Aggregation: Track B (contextual) ─────────────────────────────────────────

fn aggregate_contextual(obs: &[ZoneObservation]) -> Vec<ContextualBucket> {
    let mut buckets: Vec<ContextualBucket> = Vec::new();
    for score in 1u8..=5 {
        for arrival in ["from_above", "from_below"] {
            for trend in ["bullish", "bearish", "neutral"] {
                let filtered: Vec<_> = obs.iter()
                    .filter(|o| o.score == score
                        && o.arrival.as_deref() == Some(arrival)
                        && o.trend == trend)
                    .collect();
                if filtered.is_empty() { continue; }

                let n_zones     = filtered.len();
                let n_touched   = filtered.iter().filter(|o| o.touched).count();
                let n_respected = filtered.iter().filter(|o| o.contextual_respected == Some(true)).count();
                let respect_rate = if n_touched > 0 { n_respected as f64 / n_touched as f64 } else { 0.0 };

                buckets.push(ContextualBucket {
                    score,
                    arrival:      arrival.to_string(),
                    trend:        trend.to_string(),
                    n_zones,
                    n_touched,
                    n_respected,
                    respect_rate: round4(respect_rate),
                });
            }
        }
    }
    buckets
}

fn check_any_calibrated_bucket(buckets: &[ContextualBucket]) -> bool {
    for arrival in ["from_above", "from_below"] {
        for trend in ["bullish", "bearish", "neutral"] {
            let mut by_score: Vec<(u8, f64)> = buckets.iter()
                .filter(|b| b.arrival == arrival && b.trend == trend && b.n_touched >= MIN_BUCKET_N)
                .map(|b| (b.score, b.respect_rate))
                .collect();
            by_score.sort_by_key(|(s, _)| *s);
            if by_score.len() >= 2 && by_score.windows(2).all(|w| w[1].1 >= w[0].1) {
                return true;
            }
        }
    }
    false
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn median(xs: &[f64]) -> Option<f64> {
    if xs.is_empty() { return None; }
    let mut v: Vec<f64> = xs.iter().copied().filter(|x| x.is_finite()).collect();
    if v.is_empty() { return None; }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let n = v.len();
    Some(if n % 2 == 0 { (v[n/2 - 1] + v[n/2]) / 2.0 } else { v[n/2] })
}

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

    fn c(ts: &str, open: f64, high: f64, low: f64, close: f64, volume: f64) -> OhlcvCandle {
        OhlcvCandle {
            ts: ts.to_string(), open, high, low, close, volume,
            mb_vol: None, ms_vol: None, lb_vol: None, ls_vol: None,
            mb_count: None, ms_count: None, lb_count: None, ls_count: None,
        }
    }

    fn sine_history(n: usize, base: f64, amplitude: f64) -> Vec<OhlcvCandle> {
        (0..n).map(|i| {
            let phase = (i as f64) * 0.12;
            let mid = base + (phase.sin()) * amplitude;
            c(&format!("{}", i), mid, mid + 2.0, mid - 2.0, mid + 0.2, 100.0)
        }).collect()
    }

    #[test]
    fn too_few_candles_returns_empty_response() {
        let candles = sine_history(50, 100.0, 5.0);
        let r = backtest_multi_anchor_fib(
            &candles, &FibProfile::mature(), 2, 200, 10, 6, "1d", "BTCEUR",
        );
        assert_eq!(r.total_zones, 0);
        assert_eq!(r.candles_analyzed, 0);
        assert!(!r.legacy_monotonic_respect);
        assert!(!r.monotonic_reaction);
        assert!(!r.any_calibrated_bucket);
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
    fn all_three_tracks_populated() {
        let candles = sine_history(400, 100.0, 8.0);
        let r = backtest_multi_anchor_fib(
            &candles, &FibProfile::nascent(), 1, 100, 10, 6, "1d", "BTCEUR",
        );
        assert!(!r.legacy_respect.is_empty());
        assert!(!r.track_a_reaction.is_empty());
        assert!(!r.track_b_contextual.is_empty());
    }

    #[test]
    fn reaction_bucket_invariants() {
        let candles = sine_history(400, 100.0, 8.0);
        let r = backtest_multi_anchor_fib(
            &candles, &FibProfile::nascent(), 1, 100, 10, 6, "1d", "BTCEUR",
        );
        for b in &r.track_a_reaction {
            assert!(b.n_reacted <= b.n_zones);
            assert!(b.reaction_rate >= 0.0 && b.reaction_rate <= 1.0);
        }
    }

    #[test]
    fn contextual_bucket_invariants() {
        let candles = sine_history(400, 100.0, 8.0);
        let r = backtest_multi_anchor_fib(
            &candles, &FibProfile::nascent(), 1, 100, 10, 6, "1d", "BTCEUR",
        );
        for b in &r.track_b_contextual {
            assert!(b.n_touched <= b.n_zones);
            assert!(b.n_respected <= b.n_touched);
            assert!(b.respect_rate >= 0.0 && b.respect_rate <= 1.0);
            assert!(["from_above", "from_below"].contains(&b.arrival.as_str()));
            assert!(["bullish", "bearish", "neutral"].contains(&b.trend.as_str()));
        }
    }

    #[test]
    fn wick_prominence_large_when_long_wick_small_body() {
        let c1 = c("1", 100.0, 108.0, 99.9, 100.1, 50.0);  // tiny body, long upper wick
        let w = compute_wick_prominence(&c1);
        assert!(w > 10.0);
    }

    #[test]
    fn wick_prominence_small_when_long_body_no_wick() {
        let c1 = c("1", 100.0, 105.1, 99.9, 105.0, 50.0);  // body 5.0, wicks ~0.1
        let w = compute_wick_prominence(&c1);
        assert!(w < 0.1);
    }

    #[test]
    fn derive_trend_neutral_on_flat_history() {
        let flat: Vec<_> = (0..50).map(|i| c(&i.to_string(), 100.0, 100.5, 99.5, 100.0, 100.0)).collect();
        assert_eq!(derive_trend_regime(&flat), "neutral");
    }

    #[test]
    fn mean_std_empty_is_zero() {
        let (m, s) = mean_std(&[]);
        assert_eq!(m, 0.0);
        assert_eq!(s, 0.0);
    }

    #[test]
    fn median_odd_and_even() {
        assert_eq!(median(&[1.0, 2.0, 3.0]), Some(2.0));
        assert_eq!(median(&[1.0, 2.0, 3.0, 4.0]), Some(2.5));
        assert_eq!(median(&[]), None);
    }

    #[test]
    fn prev_period_hl_respects_bounds() {
        let candles: Vec<_> = (0..10).map(|i| c(&i.to_string(), 100.0 + i as f64, 105.0 + i as f64, 95.0 + i as f64, 101.0 + i as f64, 100.0)).collect();
        let (h, l) = prev_period_hl(&candles, 5, 3);
        assert_eq!(h, Some(109.0));
        assert_eq!(l, Some(97.0));
    }

    #[test]
    fn arrival_classified_as_from_above_when_close_below_zone() {
        let zone = FibZone {
            ratio: 0.5, direction: "up".into(), level: 100.0,
            zone_low: 99.0, zone_high: 101.0, score: 3, anchors: vec![],
        };
        let baseline = BaselineStats { atr_mean: 1.0, atr_std: 0.1, vol_mean: 100.0, vol_std: 10.0 };
        let future = vec![
            c("0", 95.0, 100.0, 94.0, 99.5, 100.0),  // first touch — from below
            c("1", 99.5, 101.5, 98.0, 100.5, 100.0),
        ];
        let obs = validate_zone(&zone, &future, 94.0, &baseline, "bullish");
        assert!(obs.touched);
        assert_eq!(obs.arrival.as_deref(), Some("from_below"));
    }

    #[test]
    fn arrival_classified_as_from_below_when_close_above_zone() {
        let zone = FibZone {
            ratio: 0.5, direction: "down".into(), level: 100.0,
            zone_low: 99.0, zone_high: 101.0, score: 3, anchors: vec![],
        };
        let baseline = BaselineStats { atr_mean: 1.0, atr_std: 0.1, vol_mean: 100.0, vol_std: 10.0 };
        let future = vec![
            c("0", 105.0, 106.0, 100.5, 101.5, 100.0),
            c("1", 101.5, 102.5, 99.5, 100.5, 100.0),
        ];
        let obs = validate_zone(&zone, &future, 106.0, &baseline, "bearish");
        assert!(obs.touched);
        assert_eq!(obs.arrival.as_deref(), Some("from_above"));
    }

    #[test]
    fn contextual_respected_when_approach_from_above_and_price_reverses_up() {
        let zone = FibZone {
            ratio: 0.5, direction: "up".into(), level: 100.0,
            zone_low: 99.0, zone_high: 101.0, score: 3, anchors: vec![],
        };
        let baseline = BaselineStats { atr_mean: 1.0, atr_std: 0.1, vol_mean: 100.0, vol_std: 10.0 };
        let future = vec![
            c("0", 103.0, 104.0, 99.5, 100.5, 100.0),  // drops into zone
            c("1", 100.5, 103.5, 99.8, 103.0, 100.0),  // closes above zone_high → reversed up
            c("2", 103.0, 105.0, 102.5, 104.5, 100.0),
        ];
        let obs = validate_zone(&zone, &future, 103.0, &baseline, "bullish");
        assert!(obs.touched);
        assert_eq!(obs.arrival.as_deref(), Some("from_above"));
        assert_eq!(obs.contextual_respected, Some(true));
    }

    #[test]
    fn reaction_rate_true_on_volume_spike() {
        let zone = FibZone {
            ratio: 0.5, direction: "up".into(), level: 100.0,
            zone_low: 99.0, zone_high: 101.0, score: 3, anchors: vec![],
        };
        let baseline = BaselineStats { atr_mean: 1.0, atr_std: 0.1, vol_mean: 100.0, vol_std: 10.0 };
        let future = vec![
            c("0", 100.5, 101.0, 99.5, 100.2, 200.0),  // touch with 10σ volume spike
            c("1", 100.2, 101.5, 99.8, 100.8, 200.0),
        ];
        let obs = validate_zone(&zone, &future, 98.0, &baseline, "bullish");
        assert!(obs.touched);
        assert_eq!(obs.reacted, Some(true));
    }
}
