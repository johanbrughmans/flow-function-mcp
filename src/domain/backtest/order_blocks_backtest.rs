/// Order Blocks — indicator-level retest-respect backtest (ADR-017, Story #40).
///
/// Claim: an OB is a zone where institutional orders are likely clustered; if price
/// returns to the zone within `lookahead_bars`, a bullish OB should hold as support
/// (no close below bottom) and a bearish OB should hold as resistance (no close above
/// top).
///
/// Measurement per OB:
///   1. return_rate  = % of OBs where price entered the zone within lookahead
///   2. respect_rate = % of returns where the OB was NOT broken (given a return)
///
/// return_rate is market-activity dependent (active markets revisit more zones); the
/// calibration signal is `respect_rate`.
///
/// Causal-safe: `compute_order_blocks` confirms an OB at candle i+1 based on
/// candles[i] and candles[i+1]; no future data is used for detection itself. The
/// existing `broken` field IS look-ahead and is ignored here. We re-validate with a
/// bounded future window.

use std::collections::HashMap;

use chrono::Utc;

use crate::domain::{
    candle::OhlcvCandle,
    smc::order_blocks::compute_order_blocks,
    types::Direction,
};

const DEFAULT_RESPECT_GATE: f64 = 0.55;

// ── Output types ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize)]
pub struct OrderBlockBucketMetrics {
    pub direction:          String,    // "bullish" | "bearish"
    pub n_blocks:           usize,
    pub n_returned:         usize,
    pub n_respected:        usize,
    pub return_rate:        f64,
    pub respect_rate:       f64,       // n_respected / n_returned
    pub avg_bars_to_return: Option<f64>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct OrderBlocksBacktestResponse {
    pub pair:             String,
    pub tf:               String,
    pub total_blocks:     usize,
    pub buckets:          Vec<OrderBlockBucketMetrics>,
    pub bullish_better_than_random: bool,
    pub bearish_better_than_random: bool,
    pub candles_analyzed: usize,
    pub lookahead_bars:   usize,
    pub respect_gate:     f64,
    pub computed_at:      String,
}

// ── Internal observation ──────────────────────────────────────────────────────

struct BlockObservation {
    direction:      Direction,
    returned:       bool,
    respected:      bool,
    bars_to_return: Option<usize>,
}

// ── Public entry point ────────────────────────────────────────────────────────

pub fn backtest_order_blocks(
    candles:        &[OhlcvCandle],
    lookahead_bars: usize,
    tf_str:         &str,
    pair_str:       &str,
) -> OrderBlocksBacktestResponse {
    let empty = || OrderBlocksBacktestResponse {
        pair:             pair_str.to_string(),
        tf:               tf_str.to_string(),
        total_blocks:     0,
        buckets:          vec![],
        bullish_better_than_random: false,
        bearish_better_than_random: false,
        candles_analyzed: 0,
        lookahead_bars,
        respect_gate:     DEFAULT_RESPECT_GATE,
        computed_at:      Utc::now().to_rfc3339(),
    };

    if candles.len() <= lookahead_bars + 2 {
        return empty();
    }

    let blocks = compute_order_blocks(candles);
    let ts_idx: HashMap<String, usize> = candles.iter()
        .enumerate()
        .map(|(i, c)| (c.ts.clone(), i))
        .collect();

    let mut observations: Vec<BlockObservation> = Vec::new();

    for ob in &blocks {
        let Some(&i) = ts_idx.get(&ob.ts) else { continue };
        let future_start = i + 2;
        if future_start + lookahead_bars - 1 >= candles.len() { continue; }
        let future = &candles[future_start..future_start + lookahead_bars];

        observations.push(validate_block(ob, future));
    }

    let buckets = aggregate(&observations);
    let (bu, be) = gates(&buckets);

    OrderBlocksBacktestResponse {
        pair:             pair_str.to_string(),
        tf:               tf_str.to_string(),
        total_blocks:     observations.len(),
        buckets,
        bullish_better_than_random: bu,
        bearish_better_than_random: be,
        candles_analyzed: candles.len().saturating_sub(lookahead_bars + 2),
        lookahead_bars,
        respect_gate:     DEFAULT_RESPECT_GATE,
        computed_at:      Utc::now().to_rfc3339(),
    }
}

// ── Validation ────────────────────────────────────────────────────────────────

fn validate_block(
    ob:     &crate::domain::smc::order_blocks::OrderBlock,
    future: &[OhlcvCandle],
) -> BlockObservation {
    let mut returned       = false;
    let mut bars_to_return = None;
    let mut first_return   = 0usize;

    for (i, c) in future.iter().enumerate() {
        let touched = c.low <= ob.top && c.high >= ob.bottom;
        if touched {
            returned       = true;
            bars_to_return = Some(i + 1);
            first_return   = i;
            break;
        }
    }

    let respected = if returned {
        let remaining = &future[first_return..];
        match ob.direction {
            Direction::Bullish => !remaining.iter().any(|c| c.close < ob.bottom),
            Direction::Bearish => !remaining.iter().any(|c| c.close > ob.top),
        }
    } else {
        false
    };

    BlockObservation {
        direction: ob.direction,
        returned,
        respected,
        bars_to_return,
    }
}

// ── Aggregation ───────────────────────────────────────────────────────────────

fn aggregate(obs: &[BlockObservation]) -> Vec<OrderBlockBucketMetrics> {
    let mut buckets = Vec::new();
    for dir in [Direction::Bullish, Direction::Bearish] {
        let filtered: Vec<_> = obs.iter().filter(|o| o.direction == dir).collect();
        if filtered.is_empty() { continue; }

        let n_blocks    = filtered.len();
        let n_returned  = filtered.iter().filter(|o| o.returned).count();
        let n_respected = filtered.iter().filter(|o| o.respected).count();
        let return_rate  = n_returned  as f64 / n_blocks    as f64;
        let respect_rate = if n_returned > 0 { n_respected as f64 / n_returned as f64 } else { 0.0 };

        let bars: Vec<f64> = filtered.iter().filter_map(|o| o.bars_to_return.map(|b| b as f64)).collect();
        let avg_bars_to_return = if bars.is_empty() { None } else { Some(round2(bars.iter().sum::<f64>() / bars.len() as f64)) };

        buckets.push(OrderBlockBucketMetrics {
            direction:          direction_str(dir).to_string(),
            n_blocks,
            n_returned,
            n_respected,
            return_rate:        round4(return_rate),
            respect_rate:       round4(respect_rate),
            avg_bars_to_return,
        });
    }
    buckets
}

fn gates(buckets: &[OrderBlockBucketMetrics]) -> (bool, bool) {
    let gate = |dir: &str| -> bool {
        buckets.iter()
            .find(|b| b.direction == dir)
            .map(|b| b.n_returned >= 30 && b.respect_rate >= DEFAULT_RESPECT_GATE)
            .unwrap_or(false)
    };
    (gate("bullish"), gate("bearish"))
}

fn direction_str(d: Direction) -> &'static str {
    match d {
        Direction::Bullish => "bullish",
        Direction::Bearish => "bearish",
    }
}

fn round2(x: f64) -> f64 { (x * 100.0).round() / 100.0 }
fn round4(x: f64) -> f64 { (x * 10_000.0).round() / 10_000.0 }

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::smc::order_blocks::OrderBlock;

    fn c(ts: &str, open: f64, high: f64, low: f64, close: f64) -> OhlcvCandle {
        OhlcvCandle {
            ts: ts.to_string(), open, high, low, close, volume: 100.0,
            mb_vol: None, ms_vol: None, lb_vol: None, ls_vol: None,
            mb_count: None, ms_count: None, lb_count: None, ls_count: None,
        }
    }

    #[test]
    fn too_few_candles_returns_empty() {
        let candles: Vec<_> = (0..5).map(|i| c(&i.to_string(), 100.0, 101.0, 99.0, 100.0)).collect();
        let r = backtest_order_blocks(&candles, 10, "1d", "BTCEUR");
        assert_eq!(r.total_blocks, 0);
    }

    #[test]
    fn bullish_ob_respected_when_price_returns_and_holds() {
        let ob = OrderBlock {
            ts: "0".into(), direction: Direction::Bullish,
            top: 100.0, bottom: 98.0, broken: false,
        };
        let future = vec![
            c("1", 105.0, 106.0, 104.0, 105.5),
            c("2", 105.5, 106.0, 99.5, 100.2),  // low 99.5 ≤ top 100 → return
            c("3", 100.2, 103.0, 99.0, 102.5),  // close 102.5 > bottom 98 → held
            c("4", 102.5, 105.0, 101.0, 104.0),
        ];
        let obs = validate_block(&ob, &future);
        assert!(obs.returned);
        assert!(obs.respected);
        assert_eq!(obs.bars_to_return, Some(2));
    }

    #[test]
    fn bullish_ob_broken_when_price_closes_below_bottom() {
        let ob = OrderBlock {
            ts: "0".into(), direction: Direction::Bullish,
            top: 100.0, bottom: 98.0, broken: false,
        };
        let future = vec![
            c("1", 105.0, 106.0, 99.5, 100.2),  // returns
            c("2", 100.2, 101.0, 95.0, 95.5),   // close 95.5 < bottom 98 → broken
            c("3", 95.5, 96.0, 94.0, 94.5),
        ];
        let obs = validate_block(&ob, &future);
        assert!(obs.returned);
        assert!(!obs.respected);
    }

    #[test]
    fn bearish_ob_respected_when_price_returns_and_holds_below_top() {
        let ob = OrderBlock {
            ts: "0".into(), direction: Direction::Bearish,
            top: 102.0, bottom: 100.0, broken: false,
        };
        let future = vec![
            c("1", 98.0, 99.0, 97.0, 98.5),
            c("2", 98.5, 100.5, 98.0, 99.5),  // high 100.5 ≥ bottom 100 → return
            c("3", 99.5, 101.0, 98.0, 98.5),  // close 98.5 < top 102 → held
        ];
        let obs = validate_block(&ob, &future);
        assert!(obs.returned);
        assert!(obs.respected);
    }

    #[test]
    fn ob_not_returned_when_price_never_touches_zone() {
        let ob = OrderBlock {
            ts: "0".into(), direction: Direction::Bullish,
            top: 100.0, bottom: 98.0, broken: false,
        };
        let future = vec![
            c("1", 110.0, 111.0, 109.0, 110.5),
            c("2", 110.5, 112.0, 108.0, 111.0),
            c("3", 111.0, 113.0, 110.0, 112.5),
        ];
        let obs = validate_block(&ob, &future);
        assert!(!obs.returned);
        assert!(!obs.respected);
    }

    #[test]
    fn gate_requires_n_returned_at_least_30_and_respect_at_least_055() {
        let buckets = vec![
            OrderBlockBucketMetrics {
                direction: "bullish".into(), n_blocks: 50, n_returned: 40, n_respected: 24,
                return_rate: 0.8, respect_rate: 0.6, avg_bars_to_return: None,
            },
            OrderBlockBucketMetrics {
                direction: "bearish".into(), n_blocks: 40, n_returned: 10, n_respected: 8,  // respect high but n<30
                return_rate: 0.25, respect_rate: 0.8, avg_bars_to_return: None,
            },
        ];
        let (bu, be) = gates(&buckets);
        assert!(bu);
        assert!(!be);
    }

    #[test]
    fn aggregate_splits_by_direction() {
        let obs = vec![
            BlockObservation { direction: Direction::Bullish, returned: true, respected: true, bars_to_return: Some(2) },
            BlockObservation { direction: Direction::Bullish, returned: true, respected: false, bars_to_return: Some(3) },
            BlockObservation { direction: Direction::Bearish, returned: true, respected: true, bars_to_return: Some(1) },
        ];
        let buckets = aggregate(&obs);
        let bu = buckets.iter().find(|b| b.direction == "bullish").unwrap();
        assert_eq!(bu.n_blocks, 2);
        assert_eq!(bu.n_respected, 1);
        assert!((bu.respect_rate - 0.5).abs() < 1e-9);
    }
}
