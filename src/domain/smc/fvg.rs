/// Fair Value Gap (FVG / Imbalance) detection.
///
/// Pattern: three consecutive candles A, B, C where
///   Bullish FVG: C.low > A.high  → imbalance zone [A.high, C.low]
///   Bearish FVG: C.high < A.low  → imbalance zone [C.high, A.low]
///
/// Timestamp assigned to candle B (the impulse).
/// `filled`: any candle after C overlaps the zone (low ≤ top AND high ≥ bottom).

use crate::domain::{candle::OhlcvCandle, types::Direction};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FvgZone {
    pub ts:        String,
    pub direction: Direction,
    pub top:       f64,
    pub bottom:    f64,
    pub filled:    bool,
}

pub fn compute_fvg(raw: &[OhlcvCandle]) -> Vec<FvgZone> {
    let mut zones = Vec::new();

    for i in 0..raw.len().saturating_sub(2) {
        let a = &raw[i];
        let b = &raw[i + 1];
        let c = &raw[i + 2];

        if c.low > a.high {
            let top    = c.low;
            let bottom = a.high;
            // Check candles AFTER C (i+3 onward) — C itself defines the gap boundary.
            let filled = raw.get(i + 3..)
                .map_or(false, |rest| rest.iter().any(|r| r.low <= top && r.high >= bottom));
            zones.push(FvgZone { ts: b.ts.clone(), direction: Direction::Bullish, top, bottom, filled });
        } else if c.high < a.low {
            let top    = a.low;
            let bottom = c.high;
            let filled = raw.get(i + 3..)
                .map_or(false, |rest| rest.iter().any(|r| r.low <= top && r.high >= bottom));
            zones.push(FvgZone { ts: b.ts.clone(), direction: Direction::Bearish, top, bottom, filled });
        }
    }

    zones
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
    fn bullish_fvg_detected() {
        // A.high=1.0, C.low=1.5 → gap [1.0, 1.5]
        let raw = vec![
            c("0", 0.8, 1.0, 0.7, 0.9),  // A
            c("1", 0.9, 2.0, 0.9, 1.8),  // B — impulse
            c("2", 1.6, 2.2, 1.5, 2.0),  // C — C.low(1.5) > A.high(1.0)
        ];
        let zones = compute_fvg(&raw);
        assert_eq!(zones.len(), 1);
        assert_eq!(zones[0].direction, Direction::Bullish);
        assert!((zones[0].bottom - 1.0).abs() < 1e-9);
        assert!((zones[0].top   - 1.5).abs() < 1e-9);
    }

    #[test]
    fn bearish_fvg_detected() {
        let raw = vec![
            c("0", 2.0, 2.2, 1.8, 2.1),  // A — A.low=1.8
            c("1", 2.1, 2.1, 0.8, 0.9),  // B — impulse
            c("2", 0.9, 1.5, 0.6, 0.8),  // C — C.high(1.5) < A.low(1.8)
        ];
        let zones = compute_fvg(&raw);
        assert_eq!(zones.len(), 1);
        assert_eq!(zones[0].direction, Direction::Bearish);
    }

    #[test]
    fn filled_when_price_returns() {
        let raw = vec![
            c("0", 0.8, 1.0, 0.7, 0.9),
            c("1", 0.9, 2.0, 0.9, 1.8),
            c("2", 1.6, 2.2, 1.5, 2.0),
            c("3", 2.0, 2.1, 1.2, 1.3),  // low=1.2 ≤ top=1.5 → fills the gap
        ];
        let zones = compute_fvg(&raw);
        assert!(zones[0].filled);
    }

    #[test]
    fn not_filled_when_price_stays_above() {
        let raw = vec![
            c("0", 0.8, 1.0, 0.7, 0.9),
            c("1", 0.9, 2.0, 0.9, 1.8),
            c("2", 1.6, 2.2, 1.5, 2.0),
            c("3", 2.0, 2.5, 1.8, 2.3),  // low=1.8 > top=1.5 → unfilled
        ];
        let zones = compute_fvg(&raw);
        assert!(!zones[0].filled);
    }
}
