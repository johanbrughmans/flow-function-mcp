/// ATR — Average True Range (Wilder's smoothing).
///
/// True Range = max(High−Low, |High−PrevClose|, |Low−PrevClose|)
/// ATR[0]     = SMA(TR, period)
/// ATR[i]     = (ATR[i-1] × (period-1) + TR[i]) / period
///
/// Seed requirement: period + 1 candles consumed before first output.

use crate::domain::candle::OhlcvCandle;

pub const DEFAULT_PERIOD: usize = 14;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AtrPoint {
    pub ts:  String,
    pub atr: f64,
}

pub fn compute_atr(raw: &[OhlcvCandle], period: usize) -> Vec<AtrPoint> {
    if raw.len() <= period { return vec![]; }

    let trs: Vec<f64> = (1..raw.len())
        .map(|i| {
            let h = raw[i].high;
            let l = raw[i].low;
            let pc = raw[i - 1].close;
            (h - l).max((h - pc).abs()).max((l - pc).abs())
        })
        .collect();

    if trs.len() < period { return vec![]; }

    // Seed with SMA
    let mut atr: f64 = trs[..period].iter().sum::<f64>() / period as f64;
    let mut result = Vec::with_capacity(trs.len() - period + 1);
    result.push(AtrPoint { ts: raw[period].ts.clone(), atr });

    let f = period as f64;
    for i in period..trs.len() {
        atr = (atr * (f - 1.0) + trs[i]) / f;
        result.push(AtrPoint { ts: raw[i + 1].ts.clone(), atr });
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candles(data: &[(f64, f64, f64)]) -> Vec<OhlcvCandle> {
        data.iter().enumerate().map(|(i, &(h, l, c))| OhlcvCandle {
            ts: i.to_string(), open: c, high: h, low: l, close: c, volume: 100.0,
            mb_vol: None, ms_vol: None, lb_vol: None, ls_vol: None,
            mb_count: None, ms_count: None, lb_count: None, ls_count: None,
        }).collect()
    }

    #[test]
    fn returns_empty_when_not_enough_data() {
        let raw = candles(&[(2.0, 0.5, 1.0); 14]);
        assert!(compute_atr(&raw, 14).is_empty());
    }

    #[test]
    fn atr_positive_for_volatile_candles() {
        let raw = candles(&[(2.0, 0.5, 1.0); 20]);
        let pts = compute_atr(&raw, 14);
        assert!(pts.iter().all(|p| p.atr > 0.0));
    }

    #[test]
    fn atr_zero_for_doji_candles() {
        let raw = candles(&[(1.0, 1.0, 1.0); 20]);
        let pts = compute_atr(&raw, 14);
        assert!(pts.iter().all(|p| p.atr.abs() < 1e-9));
    }
}
