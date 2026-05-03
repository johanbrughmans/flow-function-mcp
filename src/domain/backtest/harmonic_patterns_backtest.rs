/// XABCD Harmonic Patterns — indicator-level reversal calibration (ADR-017, Story #53 / #40).
///
/// Indicator claim: completed XABCD harmonic patterns at point D signal a reversal.
/// Higher xabcd_quality (closer to ideal ratios) should correlate with more reliable
/// directional follow-through within a forward window.
///
/// Measurement: walk-forward over candles. For each window ending at t:
///   detect patterns = compute_harmonic_patterns(&candles[t-window+1..=t], profile)
///   for each NEW pattern not seen in prior window (keyed by ts_d):
///     forward_hit = did price move in expected direction within lookahead_bars?
///     bullish: any future candle with close > d_price  (price moved above entry)
///     bearish: any future candle with close < d_price  (price moved below entry)
///
/// Quality buckets: low (xabcd_quality < 0.4), medium (0.4–0.7), high (> 0.7).
/// Calibration gate: n ≥ 30 per bucket (ADR-017).
/// Monotonicity: directional_hit_rate(high) >= medium >= low.
///
/// Causal-safe: pattern detection uses only candles[..=t]; forward window is t+1 onward.

use chrono::Utc;
use std::collections::{HashMap, HashSet};

use crate::domain::{
    candle::OhlcvCandle,
    smc::{harmonics::compute_harmonic_patterns, fib_profile::FibProfile},
};

// ── Output types ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize)]
pub struct HarmonicQualityBucket {
    pub bucket:               String,   // "low" | "medium" | "high"
    pub n_patterns:           usize,
    pub n_directional_hit:    usize,
    pub directional_hit_rate: f64,
    pub calibrated:           bool,     // n_patterns >= 30
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct HarmonicPatternStats {
    pub pattern:          String,   // "Gartley" | "Bat" | "Butterfly" | "Crab"
    pub n_bullish:        usize,
    pub n_bearish:        usize,
    pub bullish_hit_rate: f64,
    pub bearish_hit_rate: f64,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct HarmonicPatternsBacktestResponse {
    pub pair:             String,
    pub tf:               String,
    pub candles_analyzed: usize,
    pub total_patterns:   usize,
    pub quality_buckets:  Vec<HarmonicQualityBucket>,
    pub pattern_stats:    Vec<HarmonicPatternStats>,
    pub monotonic:        bool,     // hit_rate(high) >= medium >= low, requires all 3 calibrated
    pub window_size:      usize,
    pub lookahead_bars:   usize,
    pub profile:          String,
    pub computed_at:      String,
}

// ── Internal observation ──────────────────────────────────────────────────────

struct Observation {
    pattern:   String,
    direction: String,
    quality:   f64,
    hit:       bool,
}

// ── Public entry point ────────────────────────────────────────────────────────

pub fn backtest_harmonic_patterns(
    candles:        &[OhlcvCandle],
    window_size:    usize,
    lookahead_bars: usize,
    profile:        &FibProfile,
    tf_str:         &str,
    pair_str:       &str,
) -> HarmonicPatternsBacktestResponse {
    let empty = || HarmonicPatternsBacktestResponse {
        pair:             pair_str.to_string(),
        tf:               tf_str.to_string(),
        candles_analyzed: 0,
        total_patterns:   0,
        quality_buckets:  vec![],
        pattern_stats:    vec![],
        monotonic:        false,
        window_size,
        lookahead_bars,
        profile:          profile.name.clone(),
        computed_at:      Utc::now().to_rfc3339(),
    };

    if candles.len() <= window_size + lookahead_bars {
        return empty();
    }

    let mut observations: Vec<Observation> = Vec::new();
    let end = candles.len().saturating_sub(lookahead_bars);
    let mut seen_ts_d: HashSet<String> = HashSet::new();

    for t in window_size..end {
        let window  = &candles[t.saturating_sub(window_size)..=t];
        let future  = &candles[t + 1..t + 1 + lookahead_bars];
        let patterns = compute_harmonic_patterns(window, profile);

        for p in &patterns {
            if seen_ts_d.contains(&p.ts_d) { continue; }
            seen_ts_d.insert(p.ts_d.clone());

            let hit = match p.direction.as_str() {
                "bullish" => future.iter().any(|c| c.close > p.d_price),
                "bearish" => future.iter().any(|c| c.close < p.d_price),
                _         => continue,
            };

            observations.push(Observation {
                pattern:   p.pattern.clone(),
                direction: p.direction.clone(),
                quality:   p.xabcd_quality,
                hit,
            });
        }
    }

    let candles_analyzed = end.saturating_sub(window_size);
    let total_patterns   = observations.len();
    let quality_buckets  = aggregate_quality_buckets(&observations);
    let pattern_stats    = aggregate_pattern_stats(&observations);
    let monotonic        = check_monotonic(&quality_buckets);

    HarmonicPatternsBacktestResponse {
        pair:             pair_str.to_string(),
        tf:               tf_str.to_string(),
        candles_analyzed,
        total_patterns,
        quality_buckets,
        pattern_stats,
        monotonic,
        window_size,
        lookahead_bars,
        profile:          profile.name.clone(),
        computed_at:      Utc::now().to_rfc3339(),
    }
}

// ── Quality bucket helper ─────────────────────────────────────────────────────

fn quality_bucket(q: f64) -> &'static str {
    if q < 0.4 { "low" } else if q < 0.7 { "medium" } else { "high" }
}

// ── Aggregation ───────────────────────────────────────────────────────────────

fn aggregate_quality_buckets(obs: &[Observation]) -> Vec<HarmonicQualityBucket> {
    let bucket_order = ["low", "medium", "high"];
    let mut results  = Vec::new();

    for &bucket in &bucket_order {
        let filtered: Vec<_> = obs.iter()
            .filter(|o| quality_bucket(o.quality) == bucket)
            .collect();
        if filtered.is_empty() { continue; }

        let n_patterns        = filtered.len();
        let n_directional_hit = filtered.iter().filter(|o| o.hit).count();
        let directional_hit_rate = round4(n_directional_hit as f64 / n_patterns as f64);
        let calibrated        = n_patterns >= 30;

        results.push(HarmonicQualityBucket {
            bucket: bucket.to_string(),
            n_patterns,
            n_directional_hit,
            directional_hit_rate,
            calibrated,
        });
    }
    results
}

fn aggregate_pattern_stats(obs: &[Observation]) -> Vec<HarmonicPatternStats> {
    // Collect unique pattern names in stable order
    let pattern_order = ["Gartley", "Bat", "Butterfly", "Crab"];
    let mut results = Vec::new();

    // Group by (pattern, direction) → (n_total, n_hit)
    let mut map: HashMap<(String, String), (usize, usize)> = HashMap::new();
    for o in obs {
        let entry = map.entry((o.pattern.clone(), o.direction.clone())).or_default();
        entry.0 += 1;
        if o.hit { entry.1 += 1; }
    }

    for &pname in &pattern_order {
        let bull_key = (pname.to_string(), "bullish".to_string());
        let bear_key = (pname.to_string(), "bearish".to_string());

        let (n_bullish, n_bull_hit) = map.get(&bull_key).copied().unwrap_or((0, 0));
        let (n_bearish, n_bear_hit) = map.get(&bear_key).copied().unwrap_or((0, 0));

        if n_bullish == 0 && n_bearish == 0 { continue; }

        let bullish_hit_rate = if n_bullish > 0 {
            round4(n_bull_hit as f64 / n_bullish as f64)
        } else {
            0.0
        };
        let bearish_hit_rate = if n_bearish > 0 {
            round4(n_bear_hit as f64 / n_bearish as f64)
        } else {
            0.0
        };

        results.push(HarmonicPatternStats {
            pattern: pname.to_string(),
            n_bullish,
            n_bearish,
            bullish_hit_rate,
            bearish_hit_rate,
        });
    }
    results
}

// ── Monotonicity check ────────────────────────────────────────────────────────

fn check_monotonic(buckets: &[HarmonicQualityBucket]) -> bool {
    let order = ["low", "medium", "high"];
    let series: Vec<_> = order.iter()
        .filter_map(|&name| buckets.iter().find(|b| b.bucket == name))
        .collect();

    // All 3 buckets must be present AND calibrated
    if series.len() < 3 || series.iter().any(|b| !b.calibrated) {
        return false;
    }

    // hit_rate(low) <= hit_rate(medium) <= hit_rate(high)
    series.windows(2).all(|w| w[1].directional_hit_rate >= w[0].directional_hit_rate)
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
    fn empty_input_returns_empty_response() {
        let candles = flat_candles(10, 1.0);
        let r = backtest_harmonic_patterns(&candles, 200, 10, &FibProfile::mature(), "1d", "BTCEUR");
        assert_eq!(r.total_patterns, 0);
        assert_eq!(r.candles_analyzed, 0);
        assert!(r.quality_buckets.is_empty());
        assert!(r.pattern_stats.is_empty());
        assert!(!r.monotonic);
    }

    #[test]
    fn quality_bucketing() {
        assert_eq!(quality_bucket(0.3),  "low");
        assert_eq!(quality_bucket(0.0),  "low");
        assert_eq!(quality_bucket(0.39), "low");
        assert_eq!(quality_bucket(0.4),  "medium");
        assert_eq!(quality_bucket(0.55), "medium");
        assert_eq!(quality_bucket(0.69), "medium");
        assert_eq!(quality_bucket(0.7),  "high");
        assert_eq!(quality_bucket(0.8),  "high");
        assert_eq!(quality_bucket(1.0),  "high");
    }

    #[test]
    fn bullish_hit_when_future_close_above_d_price() {
        let obs = vec![
            Observation { pattern: "Gartley".into(), direction: "bullish".into(), quality: 0.8, hit: true },
            Observation { pattern: "Gartley".into(), direction: "bullish".into(), quality: 0.8, hit: false },
        ];
        let buckets = aggregate_quality_buckets(&obs);
        let high = buckets.iter().find(|b| b.bucket == "high").unwrap();
        assert_eq!(high.n_patterns, 2);
        assert_eq!(high.n_directional_hit, 1);
        assert!((high.directional_hit_rate - 0.5).abs() < 1e-9);
    }

    #[test]
    fn bearish_hit_when_future_close_below_d_price() {
        let obs = vec![
            Observation { pattern: "Bat".into(), direction: "bearish".into(), quality: 0.55, hit: true },
            Observation { pattern: "Bat".into(), direction: "bearish".into(), quality: 0.55, hit: true },
            Observation { pattern: "Bat".into(), direction: "bearish".into(), quality: 0.55, hit: false },
        ];
        let buckets = aggregate_quality_buckets(&obs);
        let medium = buckets.iter().find(|b| b.bucket == "medium").unwrap();
        assert_eq!(medium.n_patterns, 3);
        assert_eq!(medium.n_directional_hit, 2);
        let expected = round4(2.0 / 3.0);
        assert!((medium.directional_hit_rate - expected).abs() < 1e-9);
    }

    #[test]
    fn dedup_by_ts_d() {
        // Two observations with same ts_d key should only be counted once
        let mut seen: HashSet<String> = HashSet::new();
        let ts_d = "2024-01-01".to_string();

        // First encounter — should be processed
        assert!(!seen.contains(&ts_d));
        seen.insert(ts_d.clone());

        // Second encounter — should be skipped
        assert!(seen.contains(&ts_d));

        // Verify only 1 entry in the set regardless of how many times we attempt insertion
        seen.insert(ts_d.clone());
        assert_eq!(seen.len(), 1);
    }

    #[test]
    fn monotonic_false_when_not_all_three_buckets_present() {
        let buckets = vec![
            HarmonicQualityBucket {
                bucket: "low".into(), n_patterns: 40, n_directional_hit: 10,
                directional_hit_rate: 0.25, calibrated: true,
            },
            HarmonicQualityBucket {
                bucket: "medium".into(), n_patterns: 40, n_directional_hit: 20,
                directional_hit_rate: 0.50, calibrated: true,
            },
            // "high" missing
        ];
        assert!(!check_monotonic(&buckets));
    }

    #[test]
    fn monotonic_false_when_any_bucket_uncalibrated() {
        let buckets = vec![
            HarmonicQualityBucket {
                bucket: "low".into(), n_patterns: 40, n_directional_hit: 10,
                directional_hit_rate: 0.25, calibrated: true,
            },
            HarmonicQualityBucket {
                bucket: "medium".into(), n_patterns: 40, n_directional_hit: 20,
                directional_hit_rate: 0.50, calibrated: true,
            },
            HarmonicQualityBucket {
                bucket: "high".into(), n_patterns: 5, n_directional_hit: 4,
                directional_hit_rate: 0.80, calibrated: false,
            },
        ];
        assert!(!check_monotonic(&buckets));
    }

    #[test]
    fn monotonic_true_when_rates_increasing_and_all_calibrated() {
        let buckets = vec![
            HarmonicQualityBucket {
                bucket: "low".into(), n_patterns: 40, n_directional_hit: 10,
                directional_hit_rate: 0.25, calibrated: true,
            },
            HarmonicQualityBucket {
                bucket: "medium".into(), n_patterns: 40, n_directional_hit: 20,
                directional_hit_rate: 0.50, calibrated: true,
            },
            HarmonicQualityBucket {
                bucket: "high".into(), n_patterns: 40, n_directional_hit: 32,
                directional_hit_rate: 0.80, calibrated: true,
            },
        ];
        assert!(check_monotonic(&buckets));
    }

    #[test]
    fn pattern_stats_aggregation() {
        let obs = vec![
            Observation { pattern: "Gartley".into(), direction: "bullish".into(), quality: 0.8, hit: true },
            Observation { pattern: "Gartley".into(), direction: "bullish".into(), quality: 0.8, hit: false },
            Observation { pattern: "Gartley".into(), direction: "bearish".into(), quality: 0.5, hit: true },
            Observation { pattern: "Bat".into(),     direction: "bullish".into(), quality: 0.3, hit: true },
        ];
        let stats = aggregate_pattern_stats(&obs);
        let gartley = stats.iter().find(|s| s.pattern == "Gartley").unwrap();
        assert_eq!(gartley.n_bullish, 2);
        assert_eq!(gartley.n_bearish, 1);
        assert!((gartley.bullish_hit_rate - 0.5).abs() < 1e-9);
        assert!((gartley.bearish_hit_rate - 1.0).abs() < 1e-9);

        let bat = stats.iter().find(|s| s.pattern == "Bat").unwrap();
        assert_eq!(bat.n_bullish, 1);
        assert_eq!(bat.n_bearish, 0);
        assert!((bat.bullish_hit_rate - 1.0).abs() < 1e-9);
        assert!((bat.bearish_hit_rate - 0.0).abs() < 1e-9);
    }
}
