/// Order-flow aggression — indicator-level forward-return backtest (ADR-017, Story #40).
///
/// Claim: the net_aggression signal from trade-flow volumes (MB–MS)/(MB+MS) at candle t
/// predicts the direction of price movement over the next `lookahead_bars`.
///
/// Buckets net_aggression into five fixed ranges (strong_bearish..strong_bullish) and
/// measures avg forward return per bucket. Calibration signal: avg_forward_return_pct
/// must be monotonically increasing from strong_bearish → strong_bullish.
///
/// Causal-safe: uses only the candle at t to classify; future window is used solely for
/// return measurement. No look-ahead in the signal itself.
///
/// Observations without trade-flow data (mb_vol/ms_vol = None) are skipped.

use chrono::Utc;

use crate::domain::candle::OhlcvCandle;

const MIN_BUCKET_N: usize = 30;

// ── Buckets ───────────────────────────────────────────────────────────────────

const BUCKETS: &[(f64, f64, &str)] = &[
    (f64::NEG_INFINITY, -0.3, "strong_bearish"),
    (-0.3,              -0.1, "mild_bearish"),
    (-0.1,               0.1, "neutral"),
    ( 0.1,               0.3, "mild_bullish"),
    ( 0.3,       f64::INFINITY, "strong_bullish"),
];

fn bucket_of(na: f64) -> Option<&'static str> {
    for &(lo, hi, name) in BUCKETS {
        if na > lo && na <= hi { return Some(name); }
    }
    None
}

// ── Output types ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize)]
pub struct AggressionBucketMetrics {
    pub bucket:                     String,
    pub n_events:                   usize,
    pub avg_forward_return_pct:     f64,
    pub median_forward_return_pct:  Option<f64>,
    pub positive_return_rate:       f64,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct OrderFlowBacktestResponse {
    pub pair:                     String,
    pub tf:                       String,
    pub total_observations:       usize,
    pub buckets:                  Vec<AggressionBucketMetrics>,
    pub monotonic_forward_return: bool,
    pub candles_analyzed:         usize,
    pub lookahead_bars:           usize,
    pub computed_at:              String,
}

// ── Internal observation ──────────────────────────────────────────────────────

struct FlowObservation {
    bucket:              &'static str,
    forward_return_pct:  f64,
}

// ── Public entry point ────────────────────────────────────────────────────────

pub fn backtest_order_flow(
    candles:        &[OhlcvCandle],
    lookahead_bars: usize,
    tf_str:         &str,
    pair_str:       &str,
) -> OrderFlowBacktestResponse {
    let empty = || OrderFlowBacktestResponse {
        pair:                     pair_str.to_string(),
        tf:                       tf_str.to_string(),
        total_observations:       0,
        buckets:                  vec![],
        monotonic_forward_return: false,
        candles_analyzed:         0,
        lookahead_bars,
        computed_at:              Utc::now().to_rfc3339(),
    };

    if candles.len() <= lookahead_bars {
        return empty();
    }

    let mut observations: Vec<FlowObservation> = Vec::new();
    let end = candles.len().saturating_sub(lookahead_bars);

    for t in 0..end {
        let c = &candles[t];
        let Some(na) = net_aggression(c) else { continue; };
        let Some(bucket) = bucket_of(na) else { continue; };

        let future_close = candles[t + lookahead_bars].close;
        if c.close <= 0.0 { continue; }
        let forward_return_pct = (future_close - c.close) / c.close * 100.0;

        observations.push(FlowObservation { bucket, forward_return_pct });
    }

    let buckets = aggregate(&observations);
    let monotonic_forward_return = check_monotonic(&buckets);

    OrderFlowBacktestResponse {
        pair:                     pair_str.to_string(),
        tf:                       tf_str.to_string(),
        total_observations:       observations.len(),
        buckets,
        monotonic_forward_return,
        candles_analyzed:         end,
        lookahead_bars,
        computed_at:              Utc::now().to_rfc3339(),
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn net_aggression(c: &OhlcvCandle) -> Option<f64> {
    match (c.mb_vol, c.ms_vol) {
        (Some(mb), Some(ms)) if mb + ms > f64::EPSILON => Some((mb - ms) / (mb + ms)),
        _ => None,
    }
}

fn aggregate(obs: &[FlowObservation]) -> Vec<AggressionBucketMetrics> {
    let mut buckets = Vec::new();
    for &(_, _, name) in BUCKETS {
        let filtered: Vec<f64> = obs.iter()
            .filter(|o| o.bucket == name)
            .map(|o| o.forward_return_pct)
            .collect();
        if filtered.is_empty() { continue; }

        let n_events = filtered.len();
        let avg = filtered.iter().sum::<f64>() / n_events as f64;
        let med = median(&filtered);
        let n_positive = filtered.iter().filter(|r| **r > 0.0).count();
        let positive_return_rate = n_positive as f64 / n_events as f64;

        buckets.push(AggressionBucketMetrics {
            bucket:                    name.to_string(),
            n_events,
            avg_forward_return_pct:    round4(avg),
            median_forward_return_pct: med.map(round4),
            positive_return_rate:      round4(positive_return_rate),
        });
    }
    buckets
}

fn check_monotonic(buckets: &[AggressionBucketMetrics]) -> bool {
    let order = ["strong_bearish", "mild_bearish", "neutral", "mild_bullish", "strong_bullish"];
    let mut series: Vec<f64> = Vec::new();
    for name in order {
        if let Some(b) = buckets.iter().find(|b| b.bucket == name) {
            if b.n_events < MIN_BUCKET_N { return false; }
            series.push(b.avg_forward_return_pct);
        }
    }
    if series.len() < 2 { return false; }
    series.windows(2).all(|w| w[1] >= w[0])
}

fn median(xs: &[f64]) -> Option<f64> {
    if xs.is_empty() { return None; }
    let mut v: Vec<f64> = xs.iter().copied().filter(|x| x.is_finite()).collect();
    if v.is_empty() { return None; }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let n = v.len();
    Some(if n % 2 == 0 { (v[n/2 - 1] + v[n/2]) / 2.0 } else { v[n/2] })
}

fn round4(x: f64) -> f64 { (x * 10_000.0).round() / 10_000.0 }

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn candle(ts: &str, close: f64, mb: Option<f64>, ms: Option<f64>) -> OhlcvCandle {
        OhlcvCandle {
            ts: ts.to_string(),
            open: close, high: close, low: close, close,
            volume: mb.unwrap_or(0.0) + ms.unwrap_or(0.0),
            mb_vol: mb, ms_vol: ms,
            lb_vol: None, ls_vol: None,
            mb_count: None, ms_count: None, lb_count: None, ls_count: None,
        }
    }

    #[test]
    fn bucket_boundaries() {
        assert_eq!(bucket_of(-0.5), Some("strong_bearish"));
        assert_eq!(bucket_of(-0.2), Some("mild_bearish"));
        assert_eq!(bucket_of(0.0),  Some("neutral"));
        assert_eq!(bucket_of(0.2),  Some("mild_bullish"));
        assert_eq!(bucket_of(0.5),  Some("strong_bullish"));
    }

    #[test]
    fn bucket_boundary_values() {
        assert_eq!(bucket_of(-0.3), Some("strong_bearish"));
        assert_eq!(bucket_of(-0.1), Some("mild_bearish"));
        assert_eq!(bucket_of(0.1),  Some("neutral"));
        assert_eq!(bucket_of(0.3),  Some("mild_bullish"));
    }

    #[test]
    fn too_few_candles_returns_empty() {
        let candles: Vec<_> = (0..5).map(|i| candle(&i.to_string(), 100.0, Some(50.0), Some(50.0))).collect();
        let r = backtest_order_flow(&candles, 10, "1d", "BTCEUR");
        assert_eq!(r.total_observations, 0);
        assert!(!r.monotonic_forward_return);
    }

    #[test]
    fn skips_candles_without_trade_flow_data() {
        let candles = vec![
            candle("0", 100.0, None, None),
            candle("1", 101.0, None, None),
            candle("2", 102.0, Some(50.0), Some(50.0)),
            candle("3", 103.0, Some(50.0), Some(50.0)),
        ];
        let r = backtest_order_flow(&candles, 1, "1d", "BTCEUR");
        assert_eq!(r.total_observations, 1);
    }

    #[test]
    fn forward_return_sign_matches_price_move() {
        let candles = vec![
            candle("0", 100.0, Some(70.0), Some(30.0)),  // strong bullish aggression — observation
            candle("1", 110.0, Some(50.0), Some(50.0)),  // +10% — used only as future close
        ];
        let r = backtest_order_flow(&candles, 1, "1d", "TEST");
        assert_eq!(r.total_observations, 1);
        let strong_bull = r.buckets.iter().find(|b| b.bucket == "strong_bullish");
        assert!(strong_bull.is_some());
        assert!(strong_bull.unwrap().avg_forward_return_pct > 9.0);
    }

    #[test]
    fn monotonic_check_requires_min_n_per_bucket() {
        let buckets = vec![
            AggressionBucketMetrics {
                bucket: "strong_bearish".into(), n_events: 10,
                avg_forward_return_pct: -2.0, median_forward_return_pct: None,
                positive_return_rate: 0.2,
            },
            AggressionBucketMetrics {
                bucket: "strong_bullish".into(), n_events: 50,
                avg_forward_return_pct: 2.0, median_forward_return_pct: None,
                positive_return_rate: 0.8,
            },
        ];
        assert!(!check_monotonic(&buckets));
    }

    #[test]
    fn monotonic_check_true_when_all_buckets_increasing_and_n_sufficient() {
        let make = |name: &str, avg: f64| AggressionBucketMetrics {
            bucket: name.into(), n_events: 40,
            avg_forward_return_pct: avg, median_forward_return_pct: None,
            positive_return_rate: 0.5,
        };
        let buckets = vec![
            make("strong_bearish", -2.0),
            make("mild_bearish",   -0.5),
            make("neutral",         0.0),
            make("mild_bullish",    0.5),
            make("strong_bullish",  2.0),
        ];
        assert!(check_monotonic(&buckets));
    }

    #[test]
    fn monotonic_check_false_when_middle_bucket_dips() {
        let make = |name: &str, avg: f64| AggressionBucketMetrics {
            bucket: name.into(), n_events: 40,
            avg_forward_return_pct: avg, median_forward_return_pct: None,
            positive_return_rate: 0.5,
        };
        let buckets = vec![
            make("strong_bearish", -2.0),
            make("mild_bearish",    0.5),  // non-monotonic
            make("neutral",         0.0),
            make("mild_bullish",    0.5),
            make("strong_bullish",  2.0),
        ];
        assert!(!check_monotonic(&buckets));
    }

    #[test]
    fn median_correct() {
        assert_eq!(median(&[1.0, 2.0, 3.0]), Some(2.0));
        assert_eq!(median(&[1.0, 2.0, 3.0, 4.0]), Some(2.5));
    }
}
