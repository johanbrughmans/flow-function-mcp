/// OrderFlow ratio computation from trade-flow OHLCV columns.
///
/// MB/MS/LB/LS raw volumes are Infrastructure (PCTS).
/// The ratios computed here are Function-layer outputs.
/// All ratio fields are Option<f64> — None when source data is absent or denominator is zero.

use crate::domain::candle::OhlcvCandle;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct OrderFlowPoint {
    pub ts:             String,
    /// Market-buy volume / market-sell volume. >1 = more aggressive buyers.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mb_ms_ratio:    Option<f64>,
    /// Limit-buy volume / limit-sell volume. >1 = more passive buyers.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lb_ls_ratio:    Option<f64>,
    /// (MB - MS) / (MB + MS). Range -1..+1. Positive = buyer aggression dominates.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub net_aggression: Option<f64>,
    /// Percentage of total volume that were market orders (MB+MS). 0–100.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub market_pct:     Option<f64>,
    /// Average market-buy order size.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avg_mb_size:    Option<f64>,
    /// Average market-sell order size.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avg_ms_size:    Option<f64>,
}

pub fn compute_order_flow(candles: &[OhlcvCandle]) -> Vec<OrderFlowPoint> {
    candles.iter().map(|c| {
        let mb = c.mb_vol;
        let ms = c.ms_vol;
        let lb = c.lb_vol;
        let ls = c.ls_vol;

        let mb_ms_ratio = div_opt(mb, ms);
        let lb_ls_ratio = div_opt(lb, ls);

        let net_aggression = match (mb, ms) {
            (Some(b), Some(s)) if b + s > f64::EPSILON => Some((b - s) / (b + s)),
            _ => None,
        };

        let market_pct = match (mb, ms) {
            (Some(b), Some(s)) if c.volume > f64::EPSILON => Some((b + s) / c.volume * 100.0),
            _ => None,
        };

        let avg_mb_size = match (mb, c.mb_count) {
            (Some(v), Some(n)) if n > 0 => Some(v / n as f64),
            _ => None,
        };

        let avg_ms_size = match (ms, c.ms_count) {
            (Some(v), Some(n)) if n > 0 => Some(v / n as f64),
            _ => None,
        };

        OrderFlowPoint {
            ts: c.ts.clone(),
            mb_ms_ratio,
            lb_ls_ratio,
            net_aggression,
            market_pct,
            avg_mb_size,
            avg_ms_size,
        }
    }).collect()
}

fn div_opt(num: Option<f64>, den: Option<f64>) -> Option<f64> {
    match (num, den) {
        (Some(n), Some(d)) if d > f64::EPSILON => Some(n / d),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candle_with_flow(ts: &str, mb: f64, ms: f64, lb: f64, ls: f64,
                        mb_c: i64, ms_c: i64) -> OhlcvCandle {
        OhlcvCandle {
            ts: ts.to_string(), open: 1.0, high: 1.1, low: 0.9, close: 1.05,
            volume: mb + ms + lb + ls,
            mb_vol: Some(mb), ms_vol: Some(ms),
            lb_vol: Some(lb), ls_vol: Some(ls),
            mb_count: Some(mb_c), ms_count: Some(ms_c),
            lb_count: None, ls_count: None,
        }
    }

    #[test]
    fn mb_ms_ratio_computed() {
        let c = candle_with_flow("t0", 600.0, 400.0, 300.0, 200.0, 60, 40);
        let pts = compute_order_flow(&[c]);
        assert!((pts[0].mb_ms_ratio.unwrap() - 1.5).abs() < 1e-9);
    }

    #[test]
    fn net_aggression_range() {
        let c = candle_with_flow("t0", 600.0, 400.0, 300.0, 200.0, 60, 40);
        let pts = compute_order_flow(&[c]);
        let na = pts[0].net_aggression.unwrap();
        assert!(na > -1.0 && na < 1.0);
    }

    #[test]
    fn none_when_no_flow_data() {
        let c = OhlcvCandle {
            ts: "t0".into(), open: 1.0, high: 1.1, low: 0.9, close: 1.05, volume: 1000.0,
            mb_vol: None, ms_vol: None, lb_vol: None, ls_vol: None,
            mb_count: None, ms_count: None, lb_count: None, ls_count: None,
        };
        let pts = compute_order_flow(&[c]);
        assert!(pts[0].mb_ms_ratio.is_none());
        assert!(pts[0].net_aggression.is_none());
    }
}
