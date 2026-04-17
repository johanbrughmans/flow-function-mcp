/// Order Block detection.
///
/// Bullish OB: last bearish candle before a strong bullish impulse.
///   Condition: candle[i].close < candle[i].open (bearish body)
///             AND candle[i+1].close > candle[i].high (next closes above OB top)
///   Zone: [candle[i].low, candle[i].high]
///   broken: any subsequent candle closes below bottom.
///
/// Bearish OB: last bullish candle before a strong bearish impulse.
///   Condition: candle[i].close > candle[i].open (bullish body)
///             AND candle[i+1].close < candle[i].low (next closes below OB bottom)
///   Zone: [candle[i].low, candle[i].high]
///   broken: any subsequent candle closes above top.

use crate::domain::{candle::OhlcvCandle, types::Direction};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct OrderBlock {
    pub ts:        String,
    pub direction: Direction,
    pub top:       f64,
    pub bottom:    f64,
    pub broken:    bool,
}

pub fn compute_order_blocks(raw: &[OhlcvCandle]) -> Vec<OrderBlock> {
    let mut blocks = Vec::new();

    for i in 0..raw.len().saturating_sub(1) {
        let curr = &raw[i];
        let next = &raw[i + 1];

        let is_bearish_candle = curr.close < curr.open;
        let is_bullish_candle = curr.close > curr.open;

        if is_bearish_candle && next.close > curr.high {
            // Bullish OB: bearish candle before strong bullish move
            let top    = curr.high;
            let bottom = curr.low;
            let broken = raw.get(i + 1..)
                .map_or(false, |rest| rest.iter().any(|c| c.close < bottom));
            blocks.push(OrderBlock { ts: curr.ts.clone(), direction: Direction::Bullish, top, bottom, broken });
        } else if is_bullish_candle && next.close < curr.low {
            // Bearish OB: bullish candle before strong bearish move
            let top    = curr.high;
            let bottom = curr.low;
            let broken = raw.get(i + 1..)
                .map_or(false, |rest| rest.iter().any(|c| c.close > top));
            blocks.push(OrderBlock { ts: curr.ts.clone(), direction: Direction::Bearish, top, bottom, broken });
        }
    }

    blocks
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(ts: &str, open: f64, high: f64, low: f64, close: f64) -> OhlcvCandle {
        OhlcvCandle {
            ts: ts.to_string(), open, high, low, close, volume: 100.0,
            mb_vol: None, ms_vol: None, lb_vol: None, ls_vol: None,
            mb_count: None, ms_count: None, lb_count: None, ls_count: None,
        }
    }

    #[test]
    fn bullish_ob_detected() {
        let raw = vec![
            c("0", 1.5, 1.6, 1.0, 1.1),  // bearish: close(1.1) < open(1.5)
            c("1", 1.1, 2.0, 1.0, 1.9),  // close(1.9) > prev high(1.6) → bullish OB
        ];
        let blocks = compute_order_blocks(&raw);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].direction, Direction::Bullish);
    }

    #[test]
    fn bearish_ob_detected() {
        let raw = vec![
            c("0", 1.0, 2.0, 1.0, 1.8),  // bullish: close(1.8) > open(1.0)
            c("1", 1.8, 1.9, 0.5, 0.6),  // close(0.6) < prev low(1.0) → bearish OB
        ];
        let blocks = compute_order_blocks(&raw);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].direction, Direction::Bearish);
    }

    #[test]
    fn broken_when_price_closes_through() {
        let raw = vec![
            c("0", 1.5, 1.6, 1.0, 1.1),
            c("1", 1.1, 2.0, 1.0, 1.9),
            c("2", 1.9, 2.1, 0.8, 0.85),  // close(0.85) < bottom(1.0) → broken
        ];
        let blocks = compute_order_blocks(&raw);
        assert!(blocks[0].broken);
    }
}
