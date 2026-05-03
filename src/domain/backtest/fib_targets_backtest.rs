/// Fibonacci Take-Profit Targets — indicator-level target hit calibration (ADR-017, Story #52 / #40).
///
/// Indicator claim: Fibonacci confluence resistance clusters projected above an entry price
/// represent statistically meaningful price targets. Higher-strength clusters (more anchors
/// converging) should be hit more often than lower-strength ones within a forward window.
///
/// Measurement: walk-forward over candles. For each candle t (with ≥ window_size history):
///   entry_price = close_t
///   compute_fib_targets(candles[t-window+1..=t], entry_price, profile) → resistance targets
///   forward window = candles[t+1..t+1+lookahead_bars]
///   hit = any future candle where high >= target.price
///
/// Strength field is usize (integer anchor count, range typically 1–5).
/// Bucketed as: strength_1 (==1), strength_2 (==2), strength_3plus (>=3).
///
/// Calibration gate: n ≥ 30 observations per bucket (ADR-017).
/// Monotonicity check: hit_rate(3+) >= hit_rate(2) >= hit_rate(1).
///
/// Causal-safe: targets computed from candles[..=t] only; no future data in signal.

use chrono::Utc;

use crate::domain::{
    candle::OhlcvCandle,
    smc::{fib_targets::compute_fib_targets, fib_profile::FibProfile},
};

// ── Output types ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize)]
pub struct FibTargetBucketMetrics {
    pub bucket:     String,   // "strength_1" | "strength_2" | "strength_3plus"
    pub n_targets:  usize,    // total target observations in this bucket
    pub n_hit:      usize,    // targets where price reached the level
    pub hit_rate:   f64,      // n_hit / n_targets
    pub calibrated: bool,     // n_targets >= 30
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct FibTargetsBacktestResponse {
    pub pair:             String,
    pub tf:               String,
    pub candles_analyzed: usize,
    pub total_targets:    usize,    // all target observations across all buckets
    pub bucket_results:   Vec<FibTargetBucketMetrics>,
    pub monotonic:        bool,     // hit_rate increases 1 → 2 → 3+, requires all 3 calibrated
    pub window_size:      usize,
    pub lookahead_bars:   usize,
    pub profile:          String,
    pub computed_at:      String,
}

// ── Internal observation ──────────────────────────────────────────────────────

struct TargetObservation {
    strength: usize,
    hit:      bool,
}

// ── Public entry point ────────────────────────────────────────────────────────

pub fn backtest_fib_targets(
    candles:        &[OhlcvCandle],
    window_size:    usize,
    lookahead_bars: usize,
    profile:        &FibProfile,
    tf_str:         &str,
    pair_str:       &str,
) -> FibTargetsBacktestResponse {
    let empty = || FibTargetsBacktestResponse {
        pair:             pair_str.to_string(),
        tf:               tf_str.to_string(),
        candles_analyzed: 0,
        total_targets:    0,
        bucket_results:   vec![],
        monotonic:        false,
        window_size,
        lookahead_bars,
        profile:          profile.name.clone(),
        computed_at:      Utc::now().to_rfc3339(),
    };

    if candles.len() <= window_size + lookahead_bars {
        return empty();
    }

    let mut observations: Vec<TargetObservation> = Vec::new();
    let end = candles.len().saturating_sub(lookahead_bars);

    for t in window_size..end {
        let window = &candles[t.saturating_sub(window_size)..=t];
        let entry  = window.last().unwrap().close;
        if entry <= 0.0 { continue; }
        let Ok(result) = compute_fib_targets(window, entry, profile) else { continue; };
        let future = &candles[t + 1..t + 1 + lookahead_bars];
        for target in &result.targets {
            let hit = future.iter().any(|c| c.high >= target.price);
            observations.push(TargetObservation { strength: target.strength, hit });
        }
    }

    let candles_analyzed = end.saturating_sub(window_size);
    let total_targets    = observations.len();
    let bucket_results   = aggregate(&observations);
    let monotonic        = check_monotonic(&bucket_results);

    FibTargetsBacktestResponse {
        pair: pair_str.to_string(),
        tf:   tf_str.to_string(),
        candles_analyzed,
        total_targets,
        bucket_results,
        monotonic,
        window_size,
        lookahead_bars,
        profile:    profile.name.clone(),
        computed_at: Utc::now().to_rfc3339(),
    }
}

// ── Bucketing ─────────────────────────────────────────────────────────────────

fn bucket_name(s: usize) -> &'static str {
    match s {
        1 => "strength_1",
        2 => "strength_2",
        _ => "strength_3plus",
    }
}

// ── Aggregation ───────────────────────────────────────────────────────────────

fn aggregate(obs: &[TargetObservation]) -> Vec<FibTargetBucketMetrics> {
    let bucket_order = ["strength_1", "strength_2", "strength_3plus"];
    let mut results  = Vec::new();

    for &bucket in &bucket_order {
        let filtered: Vec<_> = obs.iter()
            .filter(|o| bucket_name(o.strength) == bucket)
            .collect();
        if filtered.is_empty() { continue; }

        let n_targets  = filtered.len();
        let n_hit      = filtered.iter().filter(|o| o.hit).count();
        let hit_rate   = round4(n_hit as f64 / n_targets as f64);
        let calibrated = n_targets >= 30;

        results.push(FibTargetBucketMetrics {
            bucket: bucket.to_string(),
            n_targets,
            n_hit,
            hit_rate,
            calibrated,
        });
    }
    results
}

// ── Monotonicity check ────────────────────────────────────────────────────────

fn check_monotonic(buckets: &[FibTargetBucketMetrics]) -> bool {
    let order = ["strength_1", "strength_2", "strength_3plus"];
    let series: Vec<_> = order.iter()
        .filter_map(|&name| buckets.iter().find(|b| b.bucket == name))
        .collect();

    // All 3 buckets must be present AND calibrated
    if series.len() < 3 || series.iter().any(|b| !b.calibrated) {
        return false;
    }

    // hit_rate(1) <= hit_rate(2) <= hit_rate(3+)
    series.windows(2).all(|w| w[1].hit_rate >= w[0].hit_rate)
}

// ── Helpers ───────────────────────────────────────────────────────────────────

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

    fn flat_candles(n: usize, price: f64) -> Vec<OhlcvCandle> {
        (0..n).map(|i| c(&i.to_string(), price, price + 1.0, price - 1.0, price)).collect()
    }

    #[test]
    fn too_few_candles_returns_empty() {
        // window_size=200, lookahead=10 — need > 210 candles
        let candles = flat_candles(50, 100.0);
        let r = backtest_fib_targets(&candles, 200, 10, &FibProfile::mature(), "1d", "BTCEUR");
        assert_eq!(r.total_targets, 0);
        assert_eq!(r.candles_analyzed, 0);
        assert!(r.bucket_results.is_empty());
        assert!(!r.monotonic);
    }

    #[test]
    fn correct_strength_bucketing() {
        assert_eq!(bucket_name(1), "strength_1");
        assert_eq!(bucket_name(2), "strength_2");
        assert_eq!(bucket_name(3), "strength_3plus");
        assert_eq!(bucket_name(4), "strength_3plus");
        assert_eq!(bucket_name(5), "strength_3plus");
        assert_eq!(bucket_name(10), "strength_3plus");
    }

    #[test]
    fn hit_detected_when_future_high_reaches_target() {
        let obs = vec![
            TargetObservation { strength: 1, hit: true },
        ];
        let buckets = aggregate(&obs);
        let b = buckets.iter().find(|b| b.bucket == "strength_1").unwrap();
        assert_eq!(b.n_hit, 1);
        assert!((b.hit_rate - 1.0).abs() < 1e-9);
    }

    #[test]
    fn miss_when_future_never_reaches_target() {
        let obs = vec![
            TargetObservation { strength: 2, hit: false },
            TargetObservation { strength: 2, hit: false },
        ];
        let buckets = aggregate(&obs);
        let b = buckets.iter().find(|b| b.bucket == "strength_2").unwrap();
        assert_eq!(b.n_hit, 0);
        assert!((b.hit_rate - 0.0).abs() < 1e-9);
    }

    #[test]
    fn calibrated_flag_requires_n_30() {
        // 29 observations → not calibrated
        let obs: Vec<_> = (0..29).map(|_| TargetObservation { strength: 1, hit: true }).collect();
        let buckets = aggregate(&obs);
        let b = buckets.iter().find(|b| b.bucket == "strength_1").unwrap();
        assert!(!b.calibrated);

        // 30 observations → calibrated
        let obs: Vec<_> = (0..30).map(|_| TargetObservation { strength: 1, hit: true }).collect();
        let buckets = aggregate(&obs);
        let b = buckets.iter().find(|b| b.bucket == "strength_1").unwrap();
        assert!(b.calibrated);
    }

    #[test]
    fn monotonic_false_when_not_all_three_buckets_present() {
        let buckets = vec![
            FibTargetBucketMetrics { bucket: "strength_1".into(), n_targets: 40, n_hit: 10, hit_rate: 0.25, calibrated: true },
            FibTargetBucketMetrics { bucket: "strength_2".into(), n_targets: 40, n_hit: 20, hit_rate: 0.50, calibrated: true },
            // strength_3plus missing
        ];
        assert!(!check_monotonic(&buckets));
    }

    #[test]
    fn monotonic_false_when_any_bucket_uncalibrated() {
        let buckets = vec![
            FibTargetBucketMetrics { bucket: "strength_1".into(),    n_targets: 40, n_hit: 10, hit_rate: 0.25, calibrated: true },
            FibTargetBucketMetrics { bucket: "strength_2".into(),    n_targets: 40, n_hit: 20, hit_rate: 0.50, calibrated: true },
            FibTargetBucketMetrics { bucket: "strength_3plus".into(), n_targets: 5,  n_hit: 4,  hit_rate: 0.80, calibrated: false },
        ];
        assert!(!check_monotonic(&buckets));
    }

    #[test]
    fn monotonic_true_when_rates_increasing_and_all_calibrated() {
        let buckets = vec![
            FibTargetBucketMetrics { bucket: "strength_1".into(),    n_targets: 40, n_hit: 10, hit_rate: 0.25, calibrated: true },
            FibTargetBucketMetrics { bucket: "strength_2".into(),    n_targets: 40, n_hit: 20, hit_rate: 0.50, calibrated: true },
            FibTargetBucketMetrics { bucket: "strength_3plus".into(), n_targets: 40, n_hit: 32, hit_rate: 0.80, calibrated: true },
        ];
        assert!(check_monotonic(&buckets));
    }

    #[test]
    fn monotonic_false_when_rates_not_increasing() {
        let buckets = vec![
            FibTargetBucketMetrics { bucket: "strength_1".into(),    n_targets: 40, n_hit: 20, hit_rate: 0.50, calibrated: true },
            FibTargetBucketMetrics { bucket: "strength_2".into(),    n_targets: 40, n_hit: 10, hit_rate: 0.25, calibrated: true }, // dips
            FibTargetBucketMetrics { bucket: "strength_3plus".into(), n_targets: 40, n_hit: 32, hit_rate: 0.80, calibrated: true },
        ];
        assert!(!check_monotonic(&buckets));
    }
}
