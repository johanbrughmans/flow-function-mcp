/// Order Book Imbalance (OBI) — daily calibration backtest (ADR-017, Story #51 / #40).
///
/// Indicator claim: bid_vol_25 / ask_vol_25 reflects real-time order book pressure.
/// High ratio (>1.3) = more buy-side depth than sell-side at 25-level depth window.
///
/// Measurement: aggregate OBI from last 1-min snapshot per UTC calendar day.
/// Forward window: next-day OHLCV return = (close_t+1 − close_t) / close_t × 100.
///
/// 5 OBI ratio buckets:
///   strong_bid   > 1.3
///   mild_bid     1.1–1.3
///   neutral      0.9–1.1
///   mild_ask     0.7–0.9
///   strong_ask   < 0.7
///
/// Quality gate: skip snapshot if bid_level_count == 0 AND ts < "2026-05-03 06:36:00"
/// (pre-fix rows where the stale binary produced zero level counts).
///
/// Calibration gate: n ≥ 30 per bucket before trusting monotonicity (ADR-017).
///
/// Causal-safe: OBI classification uses only snapshot at date t; forward return
/// uses only OHLCV data from date t+1 onward.

use chrono::Utc;
use std::collections::HashMap;

use crate::domain::{candle::OhlcvCandle, onchain::orderbook::OrderBookSnapshot};

const MIN_BUCKET_N: usize = 30;

// ── Bucket definitions ────────────────────────────────────────────────────────
//
// (name, lo_exclusive, hi_inclusive) for bid_vol_25 / ask_vol_25

const BUCKETS: &[(&str, f64, f64)] = &[
    ("strong_ask",  f64::NEG_INFINITY, 0.7),
    ("mild_ask",    0.7,               0.9),
    ("neutral",     0.9,               1.1),
    ("mild_bid",    1.1,               1.3),
    ("strong_bid",  1.3,               f64::INFINITY),
];

fn bucket_of(ratio: f64) -> Option<&'static str> {
    for &(name, lo, hi) in BUCKETS {
        if ratio > lo && ratio <= hi { return Some(name); }
    }
    None
}

// ── Output types ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize)]
pub struct ObiBucketMetrics {
    pub bucket:               String,  // "strong_bid" | "mild_bid" | "neutral" | "mild_ask" | "strong_ask"
    pub n:                    usize,
    pub mean_return_pct:      f64,
    pub std_dev_pct:          f64,
    pub positive_return_rate: f64,
    pub calibrated:           bool,    // n >= 30
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct OrderbookPressureBacktestResponse {
    pub pair:                  String,
    pub days_analyzed:         usize,
    pub total_observations:    usize,
    pub bucket_results:        Vec<ObiBucketMetrics>,
    pub monotonic:             bool,   // strong_bid > mild_bid > neutral > mild_ask > strong_ask (mean_return)
    pub min_n_for_calibration: usize,  // always 30
    pub quality_gate_applied:  bool,
    pub rows_excluded_by_gate: usize,
    pub computed_at:           String,
}

// ── Internal observation ──────────────────────────────────────────────────────

struct ObiObservation {
    bucket:             &'static str,
    forward_return_pct: f64,
}

// ── Public entry point ────────────────────────────────────────────────────────

pub fn backtest_orderbook_pressure(
    snapshots: &[OrderBookSnapshot],
    candles:   &[OhlcvCandle],
    pair_str:  &str,
) -> OrderbookPressureBacktestResponse {
    let empty = || OrderbookPressureBacktestResponse {
        pair:                  pair_str.to_string(),
        days_analyzed:         0,
        total_observations:    0,
        bucket_results:        vec![],
        monotonic:             false,
        min_n_for_calibration: MIN_BUCKET_N,
        quality_gate_applied:  true,
        rows_excluded_by_gate: 0,
        computed_at:           Utc::now().to_rfc3339(),
    };

    if snapshots.is_empty() || candles.is_empty() {
        return empty();
    }

    // Build date → close_price map from OHLCV candles.
    // ts format for 1d candles: "2026-04-15 00:00:00" — first 10 chars is the date.
    let close_by_date: HashMap<String, f64> = candles
        .iter()
        .map(|c| (c.ts[..10.min(c.ts.len())].to_string(), c.close))
        .collect();

    // Quality gate: exclude if bid_level_count == 0 AND ts < "2026-05-03 06:36:00"
    const GATE_TS: &str = "2026-05-03 06:36:00";
    let mut rows_excluded: usize = 0;
    let valid_snapshots: Vec<&OrderBookSnapshot> = snapshots
        .iter()
        .filter(|s| {
            if s.bid_level_count == 0 && s.ts.as_str() < GATE_TS {
                rows_excluded += 1;
                false
            } else {
                true
            }
        })
        .collect();

    // Group by UTC date (first 10 chars of ts), keep last snapshot per date (max ts).
    let mut by_date: HashMap<String, &OrderBookSnapshot> = HashMap::new();
    for snap in &valid_snapshots {
        let date = snap.ts[..10.min(snap.ts.len())].to_string();
        let entry = by_date.entry(date).or_insert(snap);
        if snap.ts > entry.ts {
            *entry = snap;
        }
    }

    let days_analyzed = by_date.len();

    // Build observations: for each daily OBI point, look up close at t and t+1.
    let mut observations: Vec<ObiObservation> = Vec::new();

    for (date_t, snap) in &by_date {
        let close_t = match close_by_date.get(date_t) {
            Some(&v) if v > 0.0 => v,
            _ => continue,
        };

        // Find the next calendar date that has an OHLCV entry.
        let close_t1 = find_next_close(date_t, &close_by_date);
        let close_t1 = match close_t1 {
            Some(v) => v,
            None    => continue,
        };

        let ratio = if snap.ask_vol_25 > f64::EPSILON {
            snap.bid_vol_25 / snap.ask_vol_25
        } else {
            continue;
        };

        let bucket = match bucket_of(ratio) {
            Some(b) => b,
            None    => continue,
        };

        let forward_return_pct = (close_t1 - close_t) / close_t * 100.0;
        observations.push(ObiObservation { bucket, forward_return_pct });
    }

    let bucket_results = aggregate_buckets(&observations);
    let monotonic      = check_monotonic(&bucket_results);

    OrderbookPressureBacktestResponse {
        pair:                  pair_str.to_string(),
        days_analyzed,
        total_observations:    observations.len(),
        bucket_results,
        monotonic,
        min_n_for_calibration: MIN_BUCKET_N,
        quality_gate_applied:  true,
        rows_excluded_by_gate: rows_excluded,
        computed_at:           Utc::now().to_rfc3339(),
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Find the closest next trading day that has a candle in close_by_date.
/// Tries up to 7 days forward (weekends / gaps).
fn find_next_close(date_t: &str, close_by_date: &HashMap<String, f64>) -> Option<f64> {
    use std::str::FromStr;

    let d = chrono::NaiveDate::from_str(date_t).ok()?;
    for offset in 1..=7i64 {
        let next = d + chrono::Duration::days(offset);
        let key  = next.format("%Y-%m-%d").to_string();
        if let Some(&v) = close_by_date.get(&key) {
            return Some(v);
        }
    }
    None
}

fn aggregate_buckets(obs: &[ObiObservation]) -> Vec<ObiBucketMetrics> {
    let mut results = Vec::new();
    for &(name, _, _) in BUCKETS {
        let returns: Vec<f64> = obs
            .iter()
            .filter(|o| o.bucket == name)
            .map(|o| o.forward_return_pct)
            .collect();

        if returns.is_empty() { continue; }

        let n = returns.len();
        let mean = returns.iter().sum::<f64>() / n as f64;
        let variance = returns.iter().map(|r| (r - mean).powi(2)).sum::<f64>() / n as f64;
        let std_dev  = variance.sqrt();
        let n_positive = returns.iter().filter(|&&r| r > 0.0).count();
        let positive_return_rate = n_positive as f64 / n as f64;

        results.push(ObiBucketMetrics {
            bucket:               name.to_string(),
            n,
            mean_return_pct:      round4(mean),
            std_dev_pct:          round4(std_dev),
            positive_return_rate: round4(positive_return_rate),
            calibrated:           n >= MIN_BUCKET_N,
        });
    }
    results
}

/// Check monotonicity: mean_return_pct increases from strong_ask → mild_ask → neutral → mild_bid → strong_bid.
/// Only check if ALL 5 buckets are present and ALL are calibrated.
fn check_monotonic(results: &[ObiBucketMetrics]) -> bool {
    let order = ["strong_ask", "mild_ask", "neutral", "mild_bid", "strong_bid"];
    let mut series: Vec<f64> = Vec::new();

    for name in order {
        match results.iter().find(|b| b.bucket == name) {
            Some(b) if b.calibrated => series.push(b.mean_return_pct),
            _ => return false,
        }
    }

    if series.len() < 5 { return false; }
    series.windows(2).all(|w| w[1] >= w[0])
}

fn round4(x: f64) -> f64 { (x * 10_000.0).round() / 10_000.0 }

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn snap(ts: &str, bid_25: f64, ask_25: f64, bid_level_count: i64) -> OrderBookSnapshot {
        OrderBookSnapshot {
            ts: ts.to_string(),
            mid_price: 1.0, bid1: 0.999, ask1: 1.001, spread_bps: 2.0,
            bid_vol_10: bid_25 * 0.4, ask_vol_10: ask_25 * 0.4,
            bid_vol_25: bid_25, ask_vol_25: ask_25,
            bid_vol_50: bid_25 * 2.0, ask_vol_50: ask_25 * 2.0,
            bid_depth: bid_25 * 4.0, ask_depth: ask_25 * 4.0,
            depth_levels: 100,
            bid_vwap_25: 0.999, ask_vwap_25: 1.001,
            bid_vwap_100: 0.998, ask_vwap_100: 1.002,
            bid_price_range_100: 0.005, ask_price_range_100: 0.005,
            effective_spread_25_bps: 3.0,
            bid_level_count,
            ask_level_count: 100,
        }
    }

    fn candle(ts: &str, close: f64) -> OhlcvCandle {
        OhlcvCandle {
            ts: ts.to_string(),
            open: close, high: close, low: close, close,
            volume: 1000.0,
            mb_vol: None, ms_vol: None, lb_vol: None, ls_vol: None,
            mb_count: None, ms_count: None, lb_count: None, ls_count: None,
        }
    }

    #[test]
    fn too_few_data_returns_empty() {
        let r = backtest_orderbook_pressure(&[], &[], "BTCEUR");
        assert_eq!(r.days_analyzed, 0);
        assert_eq!(r.total_observations, 0);
        assert!(!r.monotonic);
    }

    #[test]
    fn quality_gate_excludes_stale_rows() {
        let snapshots = vec![
            // Should be excluded: bid_level_count=0 AND ts < gate
            snap("2026-04-01 12:00:00", 1000.0, 800.0, 0),
            // Should be included: bid_level_count > 0
            snap("2026-04-02 12:00:00", 1000.0, 800.0, 50),
            // Should be included: ts >= gate even if bid_level_count=0
            snap("2026-05-03 07:00:00", 1000.0, 800.0, 0),
        ];
        let candles = vec![
            candle("2026-04-01", 100.0),
            candle("2026-04-02", 102.0),
            candle("2026-04-03", 104.0),
            candle("2026-05-03", 110.0),
            candle("2026-05-04", 112.0),
        ];
        let r = backtest_orderbook_pressure(&snapshots, &candles, "BTCEUR");
        assert_eq!(r.rows_excluded_by_gate, 1);
        assert!(r.quality_gate_applied);
        // 2026-04-02 and 2026-05-03 both have forward returns
        assert_eq!(r.total_observations, 2);
    }

    #[test]
    fn correct_obi_bucketing() {
        // Bucket rule: ratio > lo && ratio <= hi
        // strong_ask: (NEG_INF, 0.7]  — ratio <= 0.7
        // mild_ask:   (0.7,     0.9]  — ratio > 0.7 && ratio <= 0.9
        // neutral:    (0.9,     1.1]  — ratio > 0.9 && ratio <= 1.1
        // mild_bid:   (1.1,     1.3]  — ratio > 1.1 && ratio <= 1.3
        // strong_bid: (1.3,     INF)  — ratio > 1.3
        assert_eq!(bucket_of(0.5),  Some("strong_ask")); // well below 0.7
        assert_eq!(bucket_of(0.7),  Some("strong_ask")); // = 0.7 → hi of strong_ask (inclusive)
        assert_eq!(bucket_of(0.71), Some("mild_ask"));   // just above 0.7
        assert_eq!(bucket_of(0.8),  Some("mild_ask"));   // mid mild_ask
        assert_eq!(bucket_of(0.9),  Some("mild_ask"));   // = 0.9 → hi of mild_ask (inclusive)
        assert_eq!(bucket_of(0.91), Some("neutral"));    // just above 0.9
        assert_eq!(bucket_of(1.0),  Some("neutral"));    // mid neutral
        assert_eq!(bucket_of(1.1),  Some("neutral"));    // = 1.1 → hi of neutral (inclusive)
        assert_eq!(bucket_of(1.11), Some("mild_bid"));   // just above 1.1
        assert_eq!(bucket_of(1.2),  Some("mild_bid"));   // mid mild_bid
        assert_eq!(bucket_of(1.3),  Some("mild_bid"));   // = 1.3 → hi of mild_bid (inclusive)
        assert_eq!(bucket_of(1.31), Some("strong_bid")); // just above 1.3
        assert_eq!(bucket_of(1.5),  Some("strong_bid")); // well above 1.3
    }

    #[test]
    fn forward_return_computed_correctly() {
        let snapshots = vec![
            // ratio = 1.4 → strong_bid
            snap("2026-04-01 23:59:00", 1400.0, 1000.0, 100),
        ];
        let candles = vec![
            candle("2026-04-01", 100.0),
            candle("2026-04-02", 110.0), // +10%
        ];
        let r = backtest_orderbook_pressure(&snapshots, &candles, "ENJEUR");
        assert_eq!(r.total_observations, 1);
        let strong_bid = r.bucket_results.iter().find(|b| b.bucket == "strong_bid");
        assert!(strong_bid.is_some(), "strong_bid bucket missing");
        let b = strong_bid.unwrap();
        assert!((b.mean_return_pct - 10.0).abs() < 0.01, "expected ~10% return, got {}", b.mean_return_pct);
    }

    #[test]
    fn calibrated_flag_requires_n_30() {
        // Build exactly 29 observations in strong_bid bucket — should not be calibrated.
        // Use March 1–29 for snapshots, March 1–30 for candles (provides forward return
        // for each snapshot day).
        let snapshots: Vec<_> = (0..29usize).map(|i| {
            let day = format!("2026-03-{:02}", i + 1);
            // ratio = 1400/1000 = 1.4 → strong_bid
            snap(&format!("{} 23:00:00", day), 1400.0, 1000.0, 100)
        }).collect();
        let candles: Vec<_> = (0..30usize).map(|i| {
            let day = format!("2026-03-{:02}", i + 1);
            candle(&day, 100.0 + i as f64)
        }).collect();

        let r = backtest_orderbook_pressure(&snapshots, &candles, "BTCEUR");
        assert_eq!(r.total_observations, 29);
        let b = r.bucket_results.iter().find(|b| b.bucket == "strong_bid");
        assert!(b.is_some());
        assert!(!b.unwrap().calibrated, "n=29 should NOT be calibrated");

        // Add one more snapshot + candle to reach n=30.
        let mut snapshots30 = snapshots.clone();
        snapshots30.push(snap("2026-03-30 23:00:00", 1400.0, 1000.0, 100));
        let mut candles31 = candles.clone();
        candles31.push(candle("2026-03-31", 130.0));

        let r30 = backtest_orderbook_pressure(&snapshots30, &candles31, "BTCEUR");
        assert_eq!(r30.total_observations, 30);
        let b30 = r30.bucket_results.iter().find(|b| b.bucket == "strong_bid");
        assert!(b30.is_some());
        assert!(b30.unwrap().calibrated, "n=30 should be calibrated");
    }
}
