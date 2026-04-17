/// Historical Volatility — annualised close-to-close log-return standard deviation.
///
/// log_return[i] = ln(close[i] / close[i-1])
/// hv           = std_dev(log_returns over period) × √annualisation_factor × 100  (as %)
///
/// Annualisation factor derived from timeframe label:
///   1m → 252×24×60, 5m → 252×24×12, 15m → 252×24×4,
///   1h → 252×24, 4h → 252×6, 1d → 252, 1w → 52, 1M → 12
///
/// Uses sample std dev (n-1 denominator).
/// Seed requirement: period + 1 candles. Caller fetches last_n + period + 1 raw candles.

use crate::domain::{candle::OhlcvCandle, timeframe::Timeframe};

pub const DEFAULT_PERIOD: usize = 20;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct HvPoint {
    pub ts: String,
    pub hv: f64,  // annualised HV in percent
}

pub fn annualization_factor(tf: &Timeframe) -> f64 {
    let label = tf.label();
    if let Some(rest) = label.strip_suffix('m') {
        let mins: f64 = rest.parse().unwrap_or(1.0);
        252.0 * 24.0 * 60.0 / mins
    } else if let Some(rest) = label.strip_suffix('h') {
        let hours: f64 = rest.parse().unwrap_or(1.0);
        252.0 * 24.0 / hours
    } else if label.ends_with('d') {
        252.0
    } else if label.ends_with('w') {
        52.0
    } else {
        12.0 // monthly
    }
}

pub fn compute_hv(raw: &[OhlcvCandle], period: usize, tf: &Timeframe) -> Vec<HvPoint> {
    if raw.len() <= period { return vec![]; }

    let factor = annualization_factor(tf).sqrt();

    let log_returns: Vec<f64> = raw.windows(2)
        .map(|w| {
            let r = w[1].close / w[0].close;
            if r > 0.0 { r.ln() } else { 0.0 }
        })
        .collect();

    if log_returns.len() < period { return vec![]; }

    let mut result = Vec::with_capacity(log_returns.len() - period + 1);

    for i in (period - 1)..log_returns.len() {
        let window = &log_returns[(i + 1 - period)..=i];
        let mean   = window.iter().sum::<f64>() / period as f64;
        let var    = if period > 1 {
            window.iter().map(|r| (r - mean).powi(2)).sum::<f64>() / (period - 1) as f64
        } else {
            0.0
        };
        let hv = var.sqrt() * factor * 100.0;
        result.push(HvPoint { ts: raw[i + 1].ts.clone(), hv });
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tf(label: &str) -> Timeframe { label.parse().unwrap() }

    fn raw(closes: &[f64]) -> Vec<OhlcvCandle> {
        closes.iter().enumerate().map(|(i, &c)| OhlcvCandle {
            ts: i.to_string(), open: c, high: c + 0.1, low: c - 0.1, close: c, volume: 100.0,
            mb_vol: None, ms_vol: None, lb_vol: None, ls_vol: None,
            mb_count: None, ms_count: None, lb_count: None, ls_count: None,
        }).collect()
    }

    #[test]
    fn annualization_daily()  { assert!((annualization_factor(&tf("1d")) - 252.0).abs() < 1e-9); }
    #[test]
    fn annualization_weekly() { assert!((annualization_factor(&tf("1w")) - 52.0).abs() < 1e-9); }
    #[test]
    fn annualization_1h()     { assert!((annualization_factor(&tf("1h")) - 252.0 * 24.0).abs() < 1e-9); }
    #[test]
    fn annualization_4h()     { assert!((annualization_factor(&tf("4h")) - 252.0 * 6.0).abs() < 1e-9); }

    #[test]
    fn constant_series_hv_is_zero() {
        let pts = compute_hv(&raw(&[1.0_f64; 25]), 20, &tf("1d"));
        assert!(pts.iter().all(|p| p.hv.abs() < 1e-9));
    }

    #[test]
    fn volatile_series_hv_positive() {
        use std::f64::consts::PI;
        let closes: Vec<f64> = (0..30).map(|i| 100.0 + (i as f64 * PI / 3.0).sin() * 5.0).collect();
        let pts = compute_hv(&raw(&closes), 20, &tf("1d"));
        assert!(pts.iter().any(|p| p.hv > 0.0));
    }
}
