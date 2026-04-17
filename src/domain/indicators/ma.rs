/// Moving Average Cross — SMA or EMA, fast/slow crossover detection.
///
/// Seed requirement: `slow` candles consumed before first output.
/// Caller fetches `last_n + slow` raw candles.

use crate::domain::candle::OhlcvCandle;

pub const DEFAULT_FAST: usize = 9;
pub const DEFAULT_SLOW: usize = 21;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MaType { Sma, Ema }

#[derive(Debug)]
pub struct MaTypeParseError;

impl std::fmt::Display for MaTypeParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "unknown ma_type — expected sma | ema")
    }
}
impl std::error::Error for MaTypeParseError {}

impl std::str::FromStr for MaType {
    type Err = MaTypeParseError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "sma" => Ok(Self::Sma),
            "ema" => Ok(Self::Ema),
            _     => Err(MaTypeParseError),
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MaCrossPoint {
    pub ts:      String,
    pub fast_ma: f64,
    pub slow_ma: f64,
    /// "bullish" when fast crosses above slow; "bearish" when fast crosses below. Absent otherwise.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cross:   Option<String>,
}

pub fn compute_ma_cross(
    raw:     &[OhlcvCandle],
    fast:    usize,
    slow:    usize,
    ma_type: MaType,
) -> Vec<MaCrossPoint> {
    if fast >= slow || raw.len() < slow { return vec![]; }

    let closes: Vec<f64> = raw.iter().map(|c| c.close).collect();

    // Compute MA sequences aligned to the same indices.
    let fast_vals = ma_series(&closes, fast, ma_type);
    let slow_vals = ma_series(&closes, slow, ma_type);

    // slow_vals starts at index (slow-1) in `closes`; fast_vals has more values.
    // Align: slow_vals[0] corresponds to closes[slow-1].
    //        fast_vals alignment depends on start.
    let fast_start = slow - fast; // how many extra fast_vals there are before slow_vals align
    let len = slow_vals.len();

    let mut result = Vec::with_capacity(len);

    for i in 0..len {
        let fi = i + fast_start;
        let f_val = fast_vals[fi];
        let s_val = slow_vals[i];
        let raw_i = i + slow - 1;

        let cross = if i > 0 {
            let prev_f = fast_vals[fi - 1];
            let prev_s = slow_vals[i - 1];
            if prev_f < prev_s && f_val >= s_val {
                Some("bullish".to_string())
            } else if prev_f > prev_s && f_val <= s_val {
                Some("bearish".to_string())
            } else {
                None
            }
        } else {
            None
        };

        result.push(MaCrossPoint { ts: raw[raw_i].ts.clone(), fast_ma: f_val, slow_ma: s_val, cross });
    }

    result
}

fn ma_series(closes: &[f64], period: usize, ma_type: MaType) -> Vec<f64> {
    match ma_type {
        MaType::Sma => sma_series(closes, period),
        MaType::Ema => ema_series(closes, period),
    }
}

fn sma_series(closes: &[f64], period: usize) -> Vec<f64> {
    if closes.len() < period { return vec![]; }
    let mut result = Vec::with_capacity(closes.len() - period + 1);
    let mut window_sum: f64 = closes[..period].iter().sum();
    result.push(window_sum / period as f64);
    for i in period..closes.len() {
        window_sum += closes[i] - closes[i - period];
        result.push(window_sum / period as f64);
    }
    result
}

fn ema_series(closes: &[f64], period: usize) -> Vec<f64> {
    if closes.len() < period { return vec![]; }
    let alpha = 2.0 / (period as f64 + 1.0);
    let seed: f64 = closes[..period].iter().sum::<f64>() / period as f64;
    let mut result = Vec::with_capacity(closes.len() - period + 1);
    let mut ema = seed;
    result.push(ema);
    for &c in &closes[period..] {
        ema = c * alpha + ema * (1.0 - alpha);
        result.push(ema);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(closes: &[f64]) -> Vec<OhlcvCandle> {
        closes.iter().enumerate().map(|(i, &c)| OhlcvCandle {
            ts: i.to_string(), open: c, high: c, low: c, close: c, volume: 100.0,
            mb_vol: None, ms_vol: None, lb_vol: None, ls_vol: None,
            mb_count: None, ms_count: None, lb_count: None, ls_count: None,
        }).collect()
    }

    #[test]
    fn insufficient_data_returns_empty() {
        assert!(compute_ma_cross(&raw(&[1.0; 10]), 9, 21, MaType::Sma).is_empty());
    }

    #[test]
    fn fast_gte_slow_returns_empty() {
        assert!(compute_ma_cross(&raw(&[1.0; 50]), 21, 9, MaType::Sma).is_empty());
    }

    #[test]
    fn cross_detected_bullish() {
        // Downtrend then uptrend
        let mut closes: Vec<f64> = (0..21).map(|i| 100.0 - i as f64).collect();
        closes.extend((0..10).map(|i| 80.0 + i as f64 * 3.0));
        let pts = compute_ma_cross(&raw(&closes), 5, 10, MaType::Sma);
        assert!(pts.iter().any(|p| p.cross.as_deref() == Some("bullish")));
    }

    #[test]
    fn sma_constant_series_equals_constant() {
        let pts = compute_ma_cross(&raw(&[5.0_f64; 30]), 9, 21, MaType::Sma);
        assert!(pts.iter().all(|p| (p.fast_ma - 5.0).abs() < 1e-9));
    }
}
