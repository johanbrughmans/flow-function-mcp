/// Liquidity Level detection — equal highs (buy-side) and equal lows (sell-side).
///
/// Rationale: equal highs cluster stop-loss orders from short sellers above them (buy-side).
///            equal lows cluster stop-loss orders from long holders below them (sell-side).
///
/// Two candles within `tolerance` (fractional, default 0.001 = 0.1%) are considered equal.
/// For each candle[i], the nearest equal-priced candle within `search_window` ahead is used.
/// swept: any candle after the pair has its high > level price (buy-side)
///        or low < level price (sell-side).

use crate::domain::candle::OhlcvCandle;

const DEFAULT_TOLERANCE:     f64   = 0.001;
const DEFAULT_SEARCH_WINDOW: usize = 20;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LiquidityLevel {
    /// Timestamp of the second (confirming) candle.
    pub ts:     String,
    pub price:  f64,
    /// "buy_side" (equal highs, stops above) or "sell_side" (equal lows, stops below).
    pub side:   String,
    pub swept:  bool,
}

pub fn compute_liquidity(raw: &[OhlcvCandle]) -> Vec<LiquidityLevel> {
    compute_liquidity_with(raw, DEFAULT_TOLERANCE, DEFAULT_SEARCH_WINDOW)
}

pub fn compute_liquidity_with(
    raw:           &[OhlcvCandle],
    tolerance:     f64,
    search_window: usize,
) -> Vec<LiquidityLevel> {
    let mut levels = Vec::new();

    for i in 0..raw.len() {
        let end = (i + 1 + search_window).min(raw.len());

        // Equal highs — buy-side liquidity
        let h1 = raw[i].high;
        for j in (i + 1)..end {
            let h2 = raw[j].high;
            if h1 > 0.0 && (h1 - h2).abs() / h1 < tolerance {
                let price = (h1 + h2) / 2.0;
                let swept = raw.get(j..)
                    .map_or(false, |rest| rest.iter().any(|c| c.high > price + price * tolerance));
                levels.push(LiquidityLevel { ts: raw[j].ts.clone(), price, side: "buy_side".to_string(), swept });
                break;
            }
        }

        // Equal lows — sell-side liquidity
        let l1 = raw[i].low;
        for j in (i + 1)..end {
            let l2 = raw[j].low;
            if l1 > 0.0 && (l1 - l2).abs() / l1 < tolerance {
                let price = (l1 + l2) / 2.0;
                let swept = raw.get(j..)
                    .map_or(false, |rest| rest.iter().any(|c| c.low < price - price * tolerance));
                levels.push(LiquidityLevel { ts: raw[j].ts.clone(), price, side: "sell_side".to_string(), swept });
                break;
            }
        }
    }

    // Sort ascending by timestamp.
    levels.sort_by(|a, b| a.ts.cmp(&b.ts));
    levels
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(ts: &str, high: f64, low: f64) -> OhlcvCandle {
        OhlcvCandle {
            ts: ts.to_string(), open: low + (high - low) * 0.5,
            high, low, close: low + (high - low) * 0.4, volume: 100.0,
            mb_vol: None, ms_vol: None, lb_vol: None, ls_vol: None,
            mb_count: None, ms_count: None, lb_count: None, ls_count: None,
        }
    }

    #[test]
    fn equal_highs_detected() {
        let raw = vec![
            c("0", 1.500, 1.0),
            c("1", 1.501, 1.2),  // within 0.1% of 1.500
            c("2", 1.600, 1.3),
        ];
        let levels = compute_liquidity_with(&raw, 0.002, 5);
        assert!(levels.iter().any(|l| l.side == "buy_side"));
    }

    #[test]
    fn equal_lows_detected() {
        let raw = vec![
            c("0", 2.0, 1.000),
            c("1", 2.1, 1.001),  // within 0.1% of 1.000
            c("2", 2.2, 1.500),
        ];
        let levels = compute_liquidity_with(&raw, 0.002, 5);
        assert!(levels.iter().any(|l| l.side == "sell_side"));
    }

    #[test]
    fn swept_buy_side_when_price_exceeds() {
        let raw = vec![
            c("0", 1.500, 1.0),
            c("1", 1.501, 1.2),
            c("2", 1.600, 1.5),  // high=1.6 > level≈1.5 → swept
        ];
        let levels = compute_liquidity_with(&raw, 0.002, 5);
        let buy = levels.iter().find(|l| l.side == "buy_side");
        assert!(buy.map_or(false, |l| l.swept));
    }
}
