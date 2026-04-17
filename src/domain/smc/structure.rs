/// Market Structure detection — BOS (Break of Structure) and CHoCH (Change of Character).
///
/// Algorithm:
///   1. Identify swing highs: candle[i].high > both neighbours.
///   2. Identify swing lows:  candle[i].low  < both neighbours.
///   3. When close breaks the last confirmed swing high → Bullish break.
///      When close breaks the last confirmed swing low  → Bearish break.
///   4. BOS  = break in the same direction as the prior break (trend continuation).
///      CHoCH = break against the prior break direction (potential reversal).
///
/// A minimum of 3 candles is required for swing detection.

use crate::domain::{candle::OhlcvCandle, types::{Direction, StructureType}};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct StructureEvent {
    pub ts:         String,
    pub event_type: StructureType,
    pub level:      f64,
    pub direction:  Direction,
}

pub fn compute_structure(raw: &[OhlcvCandle]) -> Vec<StructureEvent> {
    if raw.len() < 3 { return vec![]; }

    let mut events = Vec::new();
    let mut last_swing_high: Option<f64> = None;
    let mut last_swing_low:  Option<f64> = None;
    let mut last_break_dir:  Option<Direction> = None;
    // Track whether a swing high/low has been confirmed already for each index.
    let mut last_high_used = false;
    let mut last_low_used  = false;

    for i in 1..raw.len() - 1 {
        let prev = &raw[i - 1];
        let curr = &raw[i];
        let next = &raw[i + 1];

        // Register swing high: curr.high > both neighbours.
        if curr.high > prev.high && curr.high >= next.high {
            last_swing_high = Some(curr.high);
            last_high_used  = false;
        }
        // Register swing low: curr.low < both neighbours.
        if curr.low < prev.low && curr.low <= next.low {
            last_swing_low = Some(curr.low);
            last_low_used  = false;
        }

        // Check breaks against the NEXT candle (candle i+1 may break a level).
        let breaker = &raw[i + 1];

        if let (Some(sh), false) = (last_swing_high, last_high_used) {
            if breaker.close > sh {
                let event_type = match last_break_dir {
                    Some(Direction::Bullish) => StructureType::Bos,
                    _ => StructureType::Choch,
                };
                events.push(StructureEvent {
                    ts:         breaker.ts.clone(),
                    event_type,
                    level:      sh,
                    direction:  Direction::Bullish,
                });
                last_break_dir = Some(Direction::Bullish);
                last_high_used = true;
            }
        }

        if let (Some(sl), false) = (last_swing_low, last_low_used) {
            if breaker.close < sl {
                let event_type = match last_break_dir {
                    Some(Direction::Bearish) => StructureType::Bos,
                    _ => StructureType::Choch,
                };
                events.push(StructureEvent {
                    ts:         breaker.ts.clone(),
                    event_type,
                    level:      sl,
                    direction:  Direction::Bearish,
                });
                last_break_dir = Some(Direction::Bearish);
                last_low_used  = true;
            }
        }
    }

    events
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
    fn too_few_candles_returns_empty() {
        assert!(compute_structure(&[c("0", 1.0, 2.0, 0.5, 1.5)]).is_empty());
    }

    #[test]
    fn first_bullish_break_is_choch() {
        let raw = vec![
            c("0", 1.0, 1.0, 0.5, 0.8),
            c("1", 0.8, 1.5, 0.7, 1.4),  // swing high at 1.5
            c("2", 1.4, 1.4, 1.0, 1.1),
            c("3", 1.1, 2.0, 1.0, 1.9),  // breaks 1.5 → first break = CHoCH
        ];
        let events = compute_structure(&raw);
        assert!(events.iter().any(|e| e.direction == Direction::Bullish && e.event_type == StructureType::Choch));
    }
}
