/// Bollinger Bands — SMA ± n_std standard deviations.
///
/// middle = SMA(close, period)
/// upper  = middle + n_std × σ
/// lower  = middle − n_std × σ
/// width  = (upper − lower) / middle
/// pct_b  = (close − lower) / (upper − lower)  [can be outside 0–1]
///
/// Seed requirement: period candles. Caller fetches last_n + period raw candles.

use crate::domain::candle::OhlcvCandle;

pub const DEFAULT_PERIOD: usize = 20;
pub const DEFAULT_N_STD:  f64   = 2.0;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BollingerPoint {
    pub ts:     String,
    pub middle: f64,
    pub upper:  f64,
    pub lower:  f64,
    pub width:  f64,
    pub pct_b:  f64,
}

pub fn compute_bollinger(raw: &[OhlcvCandle], period: usize, n_std: f64) -> Vec<BollingerPoint> {
    if raw.len() < period { return vec![]; }

    let closes: Vec<f64> = raw.iter().map(|c| c.close).collect();
    let mut result = Vec::with_capacity(closes.len() - period + 1);

    for i in (period - 1)..closes.len() {
        let window = &closes[(i + 1 - period)..=i];
        let mean   = window.iter().sum::<f64>() / period as f64;
        let var    = window.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / period as f64;
        let std    = var.sqrt();

        let upper = mean + n_std * std;
        let lower = mean - n_std * std;
        let width = if mean.abs() > f64::EPSILON { (upper - lower) / mean } else { 0.0 };
        let range = upper - lower;
        let pct_b = if range > f64::EPSILON { (closes[i] - lower) / range } else { 0.5 };

        result.push(BollingerPoint { ts: raw[i].ts.clone(), middle: mean, upper, lower, width, pct_b });
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(closes: &[f64]) -> Vec<OhlcvCandle> {
        closes.iter().enumerate().map(|(i, &c)| OhlcvCandle {
            ts: i.to_string(), open: c, high: c + 0.1, low: c - 0.1, close: c, volume: 100.0,
            mb_vol: None, ms_vol: None, lb_vol: None, ls_vol: None,
            mb_count: None, ms_count: None, lb_count: None, ls_count: None,
        }).collect()
    }

    #[test]
    fn upper_gte_middle_gte_lower() {
        let closes: Vec<f64> = (0..30).map(|i| 100.0 + (i as f64).sin() * 5.0).collect();
        for pt in compute_bollinger(&raw(&closes), 20, 2.0) {
            assert!(pt.upper >= pt.middle);
            assert!(pt.middle >= pt.lower);
        }
    }

    #[test]
    fn constant_series_has_zero_width() {
        let pts = compute_bollinger(&raw(&[10.0_f64; 30]), 20, 2.0);
        assert!(pts.iter().all(|p| p.width.abs() < 1e-9));
    }

    #[test]
    fn output_count() {
        let pts = compute_bollinger(&raw(&[1.0_f64; 30]), 20, 2.0);
        assert_eq!(pts.len(), 30 - 20 + 1);
    }
}
