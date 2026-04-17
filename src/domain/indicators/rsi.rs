/// RSI — Wilder's Relative Strength Index.
///
/// Seed requirement: period + 1 candles are consumed before the first output point.
/// The caller must fetch `last_n + period + 1` raw candles and pass all of them.
/// This function returns at most `raw.len() - period` points.

use crate::domain::candle::OhlcvCandle;

pub const DEFAULT_PERIOD: usize = 14;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RsiPoint {
    pub ts:  String,
    pub rsi: f64,
}

/// Compute RSI using Wilder's smoothing (same as TradingView default).
/// Returns an empty vec if `raw.len() <= period`.
pub fn compute_rsi(raw: &[OhlcvCandle], period: usize) -> Vec<RsiPoint> {
    if raw.len() <= period { return vec![]; }

    let closes: Vec<f64> = raw.iter().map(|c| c.close).collect();

    // Seed: SMA of gains/losses over the first `period` differences.
    let mut avg_gain = 0.0_f64;
    let mut avg_loss = 0.0_f64;
    for i in 1..=period {
        let diff = closes[i] - closes[i - 1];
        if diff > 0.0 { avg_gain += diff; } else { avg_loss -= diff; }
    }
    avg_gain /= period as f64;
    avg_loss /= period as f64;

    let mut result = Vec::with_capacity(raw.len() - period);
    result.push(RsiPoint { ts: raw[period].ts.clone(), rsi: wilder_rsi(avg_gain, avg_loss) });

    // Wilder's exponential smoothing.
    for i in (period + 1)..raw.len() {
        let diff = closes[i] - closes[i - 1];
        let gain = if diff > 0.0 { diff } else { 0.0 };
        let loss = if diff < 0.0 { -diff } else { 0.0 };
        let f = period as f64;
        avg_gain = (avg_gain * (f - 1.0) + gain) / f;
        avg_loss = (avg_loss * (f - 1.0) + loss) / f;
        result.push(RsiPoint { ts: raw[i].ts.clone(), rsi: wilder_rsi(avg_gain, avg_loss) });
    }

    result
}

#[inline]
fn wilder_rsi(avg_gain: f64, avg_loss: f64) -> f64 {
    if avg_loss < f64::EPSILON { 100.0 }
    else { 100.0 - 100.0 / (1.0 + avg_gain / avg_loss) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candles(closes: &[f64]) -> Vec<OhlcvCandle> {
        closes.iter().enumerate().map(|(i, &c)| OhlcvCandle {
            ts: i.to_string(), open: c, high: c + 0.1, low: c - 0.1, close: c, volume: 100.0,
            mb_vol: None, ms_vol: None, lb_vol: None, ls_vol: None,
            mb_count: None, ms_count: None, lb_count: None, ls_count: None,
        }).collect()
    }

    #[test]
    fn empty_when_not_enough_data() {
        assert!(compute_rsi(&candles(&[1.0; 14]), 14).is_empty());
    }

    #[test]
    fn output_count_correct() {
        let raw = candles(&[1.0; 30]);
        assert_eq!(compute_rsi(&raw, 14).len(), 30 - 14);
    }

    #[test]
    fn rsi_all_gains_is_100() {
        let raw = candles(&(0..20).map(|i| i as f64).collect::<Vec<_>>());
        let pts = compute_rsi(&raw, 14);
        assert!((pts.last().unwrap().rsi - 100.0).abs() < 1e-6);
    }

    #[test]
    fn rsi_all_losses_is_0() {
        let raw = candles(&(0..20).map(|i| 20.0 - i as f64).collect::<Vec<_>>());
        let pts = compute_rsi(&raw, 14);
        assert!(pts.last().unwrap().rsi < 1e-6);
    }

    #[test]
    fn rsi_bounded_0_to_100() {
        use std::f64::consts::PI;
        let closes: Vec<f64> = (0..50).map(|i| 100.0 + (i as f64 * PI / 5.0).sin() * 10.0).collect();
        let raw = candles(&closes);
        for pt in compute_rsi(&raw, 14) {
            assert!(pt.rsi >= 0.0 && pt.rsi <= 100.0);
        }
    }
}
