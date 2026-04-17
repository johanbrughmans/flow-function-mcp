/// Donchian Channels — rolling highest high / lowest low over `period` candles.
///
/// upper = max(high[i-period+1..=i])
/// lower = min(low[i-period+1..=i])
/// mid   = (upper + lower) / 2
/// width = upper − lower
///
/// Seed requirement: period candles. Caller fetches last_n + period raw candles.

use crate::domain::candle::OhlcvCandle;

pub const DEFAULT_PERIOD: usize = 20;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DonchianPoint {
    pub ts:    String,
    pub upper: f64,
    pub mid:   f64,
    pub lower: f64,
    pub width: f64,
}

pub fn compute_donchian(raw: &[OhlcvCandle], period: usize) -> Vec<DonchianPoint> {
    if raw.len() < period { return vec![]; }

    let mut result = Vec::with_capacity(raw.len() - period + 1);

    for i in (period - 1)..raw.len() {
        let window = &raw[(i + 1 - period)..=i];
        let upper  = window.iter().map(|c| c.high).fold(f64::NEG_INFINITY, f64::max);
        let lower  = window.iter().map(|c| c.low ).fold(f64::INFINITY,     f64::min);
        let mid    = (upper + lower) / 2.0;
        let width  = upper - lower;
        result.push(DonchianPoint { ts: raw[i].ts.clone(), upper, mid, lower, width });
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(highs: &[f64], lows: &[f64]) -> Vec<OhlcvCandle> {
        highs.iter().zip(lows).enumerate().map(|(i, (&h, &l))| OhlcvCandle {
            ts: i.to_string(), open: (h + l) / 2.0, high: h, low: l,
            close: (h + l) / 2.0, volume: 100.0,
            mb_vol: None, ms_vol: None, lb_vol: None, ls_vol: None,
            mb_count: None, ms_count: None, lb_count: None, ls_count: None,
        }).collect()
    }

    #[test]
    fn upper_is_max_high() {
        let highs: Vec<f64> = (1..=25).map(|i| i as f64).collect();
        let lows:  Vec<f64> = highs.iter().map(|h| h - 0.5).collect();
        let pts = compute_donchian(&raw(&highs, &lows), 20);
        assert!((pts[0].upper - 20.0).abs() < 1e-9);
    }

    #[test]
    fn width_nonneg() {
        let highs = vec![1.0, 2.0, 1.5, 2.5, 1.8, 2.2, 2.0, 1.9, 2.3, 2.1,
                         1.7, 2.4, 2.2, 1.6, 2.0, 1.8, 2.5, 1.9, 2.1, 2.0, 2.3];
        let lows: Vec<f64> = highs.iter().map(|h| h - 0.3).collect();
        for pt in compute_donchian(&raw(&highs, &lows), 20) {
            assert!(pt.width >= 0.0);
        }
    }
}
