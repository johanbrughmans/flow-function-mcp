/// CandleSource — shared optional input for all OHLCV-based MCP tools.
///
/// Convention: every new OHLCV tool input struct MUST include:
///   #[serde(flatten)]
///   source: CandleSource,
///
/// Excluded tools (by design — candle_source has no meaning there):
///   ha_pattern  — already computes HA internally
///   order_flow  — MB/MS/LB/LS trade-flow columns drive the metric;
///                 HA OHLC smoothing adds no signal value here

use schemars::JsonSchema;
use serde::Deserialize;

use crate::domain::{candle::OhlcvCandle, ha::ohlcv_to_ha};

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CandleSource {
    #[schemars(
        description = "Candle data source: \"ohlcv\" (default, raw PCTS data) | \"ha\" (Heikin Ashi smoothed). \
                       HA reduces noise for pattern detection; OHLCV preserves raw price action."
    )]
    #[serde(default = "default_candle_source")]
    pub candle_source: String,
}

fn default_candle_source() -> String { "ohlcv".to_string() }

/// Apply the candle source transform.  "ohlcv" → raw unchanged; "ha" → HA-converted.
pub fn apply_candle_source(
    raw:    Vec<OhlcvCandle>,
    source: &str,
) -> Result<Vec<OhlcvCandle>, String> {
    match source {
        "ohlcv" => Ok(raw),
        "ha"    => Ok(ohlcv_to_ha(&raw)),
        other   => Err(format!("unknown candle_source \"{other}\": expected \"ohlcv\" or \"ha\"")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::candle::OhlcvCandle;

    fn bare(o: f64, h: f64, l: f64, c: f64) -> OhlcvCandle {
        OhlcvCandle {
            ts: "t".into(), open: o, high: h, low: l, close: c, volume: 1.0,
            mb_vol: None, ms_vol: None, lb_vol: None, ls_vol: None,
            mb_count: None, ms_count: None, lb_count: None, ls_count: None,
        }
    }

    #[test]
    fn ohlcv_source_returns_raw_unchanged() {
        let raw = vec![bare(1.0, 2.0, 0.5, 1.5)];
        let out = apply_candle_source(raw.clone(), "ohlcv").unwrap();
        assert_eq!(out[0].open,  raw[0].open);
        assert_eq!(out[0].close, raw[0].close);
    }

    #[test]
    fn ha_source_modifies_ohlc() {
        let raw = vec![bare(1.0, 2.0, 0.5, 1.5)];
        let out = apply_candle_source(raw, "ha").unwrap();
        // ha_close = (1+2+0.5+1.5)/4 = 1.25
        assert!((out[0].close - 1.25).abs() < 1e-9);
    }

    #[test]
    fn unknown_source_returns_err() {
        let raw = vec![bare(1.0, 2.0, 0.5, 1.5)];
        let err = apply_candle_source(raw, "tick").unwrap_err();
        assert!(err.contains("tick"));
    }

    #[test]
    fn default_candle_source_is_ohlcv() {
        let cs: CandleSource = serde_json::from_str("{}").unwrap();
        assert_eq!(cs.candle_source, "ohlcv");
    }

    #[test]
    fn explicit_ha_source_deserialises() {
        let cs: CandleSource = serde_json::from_str(r#"{"candle_source":"ha"}"#).unwrap();
        assert_eq!(cs.candle_source, "ha");
    }
}
