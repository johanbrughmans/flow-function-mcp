/// Heikin Ashi computation + extended pattern detection.
///
/// compute_ha_patterns is the Function-layer output:
///   same HA formula as Infrastructure, but enriched with consecutive_count,
///   reversal flag, and lower_wick_signal for the MCP tool response.

use crate::domain::candle::{HaCandle, HaColor, OhlcvCandle};

pub const SEED_LOOKBACK: usize = 10;

// ── Base HA computation (shared with indicators that need HA) ─────────────────

pub fn compute_ha(raw: &[OhlcvCandle], requested: usize) -> Vec<HaCandle> {
    if raw.is_empty() { return vec![]; }

    let mut prev_open:  f64 = 0.0;
    let mut prev_close: f64 = 0.0;
    let mut result: Vec<HaCandle> = Vec::with_capacity(raw.len());

    for (i, c) in raw.iter().enumerate() {
        let ha_close = (c.open + c.high + c.low + c.close) / 4.0;
        let ha_open  = if i == 0 { (c.open + c.close) / 2.0 }
                       else       { (prev_open + prev_close) / 2.0 };
        let ha_high  = c.high.max(ha_open).max(ha_close);
        let ha_low   = c.low.min(ha_open).min(ha_close);

        let color          = classify(ha_open, ha_close, prev_open, i);
        let has_lower_wick = ha_low  < ha_open.min(ha_close) - f64::EPSILON;
        let has_upper_wick = ha_high > ha_open.max(ha_close) + f64::EPSILON;

        prev_open  = ha_open;
        prev_close = ha_close;

        result.push(HaCandle { ts: c.ts.clone(), ha_open, ha_high, ha_low, ha_close,
                               color, has_lower_wick, has_upper_wick });
    }

    let skip = result.len().saturating_sub(requested);
    result.into_iter().skip(skip).collect()
}

fn classify(ha_open: f64, ha_close: f64, prev_ha_open: f64, index: usize) -> HaColor {
    let bullish = ha_close > ha_open;
    let prev = if index == 0 { ha_open } else { prev_ha_open };
    match (bullish, ha_open > prev) {
        (true,  true)  => HaColor::Blue,
        (true,  false) => HaColor::Green,
        (false, false) => HaColor::Red,
        (false, true)  => HaColor::Gray,
    }
}

// ── Extended pattern output ───────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct HaPattern {
    pub ts:                String,
    pub color:             HaColor,
    pub has_lower_wick:    bool,
    pub has_upper_wick:    bool,
    /// Number of consecutive candles (including this one) with the same color.
    pub consecutive_count: u32,
    /// True when this candle's color differs from the previous one (first = false).
    pub reversal:          bool,
    /// True for a bullish candle (Blue/Green) that has a lower wick — continuation signal.
    pub lower_wick_signal: bool,
}

pub fn compute_ha_patterns(raw: &[OhlcvCandle], requested: usize) -> Vec<HaPattern> {
    if raw.is_empty() { return vec![]; }

    // Compute full HA sequence so seed candles warm the open formula.
    let all_ha = compute_ha(raw, raw.len());
    let mut patterns: Vec<HaPattern> = Vec::with_capacity(all_ha.len());
    let mut run: u32 = 1;

    for (i, c) in all_ha.iter().enumerate() {
        if i > 0 {
            if c.color == all_ha[i - 1].color { run += 1; } else { run = 1; }
        }
        let reversal          = i > 0 && c.color != all_ha[i - 1].color;
        let is_bullish        = matches!(c.color, HaColor::Blue | HaColor::Green);
        let lower_wick_signal = is_bullish && c.has_lower_wick;

        patterns.push(HaPattern {
            ts:                c.ts.clone(),
            color:             c.color,
            has_lower_wick:    c.has_lower_wick,
            has_upper_wick:    c.has_upper_wick,
            consecutive_count: run,
            reversal,
            lower_wick_signal,
        });
    }

    let skip = patterns.len().saturating_sub(requested);
    patterns.into_iter().skip(skip).collect()
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
    fn empty_input_returns_empty() { assert!(compute_ha_patterns(&[], 10).is_empty()); }

    #[test]
    fn consecutive_count_increments_on_same_color() {
        let raw: Vec<_> = (0..5).map(|i| c(&i.to_string(), 1.0, 2.0, 0.5, 1.8)).collect();
        let result = compute_ha_patterns(&raw, 5);
        assert!(result.last().map_or(false, |p| p.consecutive_count > 1));
    }

    #[test]
    fn reversal_false_on_first_candle() {
        let raw = vec![c("0", 1.0, 2.0, 0.5, 1.5)];
        let result = compute_ha_patterns(&raw, 1);
        assert!(!result[0].reversal);
    }

    #[test]
    fn lower_wick_signal_requires_bullish_and_lower_wick() {
        let raw = vec![
            c("0", 1.0, 1.5, 1.0, 1.2),
            c("1", 1.2, 1.8, 0.5, 1.6),
        ];
        let result = compute_ha_patterns(&raw, 1);
        if result[0].lower_wick_signal {
            assert!(result[0].has_lower_wick);
            assert!(matches!(result[0].color, HaColor::Blue | HaColor::Green));
        }
    }
}
