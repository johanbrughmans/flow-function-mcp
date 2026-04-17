/// OhlcvCandle and HaCandle — core price data types.
/// Trade-flow fields (mb_vol/ms_vol/lb_vol/ls_vol/counts) are Optional;
/// None when PCTS row pre-dates aggregator population.

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct OhlcvCandle {
    pub ts:     String,
    pub open:   f64,
    pub high:   f64,
    pub low:    f64,
    pub close:  f64,
    pub volume: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mb_vol:   Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ms_vol:   Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lb_vol:   Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ls_vol:   Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mb_count: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ms_count: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lb_count: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ls_count: Option<i64>,
}

impl OhlcvCandle {
    #[cfg(test)]
    pub fn bare(ts: &str, open: f64, high: f64, low: f64, close: f64, volume: f64) -> Self {
        Self {
            ts: ts.to_string(), open, high, low, close, volume,
            mb_vol: None, ms_vol: None, lb_vol: None, ls_vol: None,
            mb_count: None, ms_count: None, lb_count: None, ls_count: None,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct HaCandle {
    pub ts:             String,
    pub ha_open:        f64,
    pub ha_high:        f64,
    pub ha_low:         f64,
    pub ha_close:       f64,
    pub color:          HaColor,
    pub has_lower_wick: bool,
    pub has_upper_wick: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HaColor {
    Blue,
    Green,
    Red,
    Gray,
}
