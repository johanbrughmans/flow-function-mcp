/// Order book pressure computation.
///
/// Consumes raw OrderBookSnapshot records (from OMV SQLite / Kraken WS collector)
/// and computes directional pressure ratios per snapshot.
///
/// bid_ask_ratio_10/25/50: bid volume / ask volume at 10%/25%/full-depth tiers.
/// dominant_side: "bid" when ratio_25 > 1.1, "ask" when < 0.9, else "neutral".
/// All ratio fields are Option<f64> — None when denominator is zero.

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct OrderBookSnapshot {
    pub ts:                      String,
    pub mid_price:               f64,
    pub bid1:                    f64,
    pub ask1:                    f64,
    pub spread_bps:              f64,
    pub bid_vol_10:              f64,
    pub ask_vol_10:              f64,
    pub bid_vol_25:              f64,
    pub ask_vol_25:              f64,
    pub bid_vol_50:              f64,
    pub ask_vol_50:              f64,
    pub bid_depth:               f64,
    pub ask_depth:               f64,
    pub depth_levels:            i64,
    pub bid_vwap_25:             f64,
    pub ask_vwap_25:             f64,
    pub bid_vwap_100:            f64,
    pub ask_vwap_100:            f64,
    pub bid_price_range_100:     f64,
    pub ask_price_range_100:     f64,
    pub effective_spread_25_bps: f64,
    pub bid_level_count:         i64,
    pub ask_level_count:         i64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct OrderbookPressurePoint {
    pub ts:                String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bid_ask_ratio_10:  Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bid_ask_ratio_25:  Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bid_ask_ratio_50:  Option<f64>,
    pub dominant_side:     String,
    pub spread_bps:        f64,
}

pub fn compute_orderbook_pressure(snapshots: &[OrderBookSnapshot]) -> Vec<OrderbookPressurePoint> {
    snapshots.iter().map(|s| {
        let r10 = safe_div(s.bid_vol_10, s.ask_vol_10);
        let r25 = safe_div(s.bid_vol_25, s.ask_vol_25);
        let r50 = safe_div(s.bid_depth,  s.ask_depth);

        let dominant_side = match r25 {
            Some(r) if r > 1.1 => "bid",
            Some(r) if r < 0.9 => "ask",
            _                  => "neutral",
        }.to_string();

        OrderbookPressurePoint {
            ts: s.ts.clone(),
            bid_ask_ratio_10: r10,
            bid_ask_ratio_25: r25,
            bid_ask_ratio_50: r50,
            dominant_side,
            spread_bps: s.spread_bps,
        }
    }).collect()
}

fn safe_div(num: f64, den: f64) -> Option<f64> {
    if den > f64::EPSILON { Some(num / den) } else { None }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap(bid_25: f64, ask_25: f64) -> OrderBookSnapshot {
        OrderBookSnapshot {
            ts: "2026-01-01 00:00:00".into(),
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
            bid_level_count: 100, ask_level_count: 100,
        }
    }

    #[test]
    fn bid_dominant_when_ratio_high() {
        let pts = compute_orderbook_pressure(&[snap(1200.0, 800.0)]);
        assert_eq!(pts[0].dominant_side, "bid");
    }

    #[test]
    fn ask_dominant_when_ratio_low() {
        let pts = compute_orderbook_pressure(&[snap(800.0, 1200.0)]);
        assert_eq!(pts[0].dominant_side, "ask");
    }

    #[test]
    fn neutral_when_balanced() {
        let pts = compute_orderbook_pressure(&[snap(1000.0, 1000.0)]);
        assert_eq!(pts[0].dominant_side, "neutral");
    }
}
