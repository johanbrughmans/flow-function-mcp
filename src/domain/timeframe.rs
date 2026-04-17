/// Timeframe — aggregation level string (e.g. "1m", "1h", "4h", "1d", "1w").
/// Accepts any non-empty string; behaviour is derived from label suffix.

#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct Timeframe(String);

impl Timeframe {
    pub fn label(&self) -> &str { &self.0 }

    pub fn is_intraday(&self) -> bool {
        self.0.ends_with('m') || self.0.ends_with('h')
    }

    pub fn ts_format(&self) -> &'static str {
        if self.is_intraday() { "%Y-%m-%d %H:%M:%S" } else { "%Y-%m-%d" }
    }
}

#[derive(Debug)]
pub struct TimeframeParseError;

impl std::fmt::Display for TimeframeParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "timeframe must be a non-empty string such as '1m', '1h', '4h', '1d', '1w'")
    }
}
impl std::error::Error for TimeframeParseError {}

impl std::str::FromStr for Timeframe {
    type Err = TimeframeParseError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim().to_lowercase();
        if s.is_empty() { return Err(TimeframeParseError); }
        Ok(Self(s))
    }
}

impl std::fmt::Display for Timeframe {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn parse_1h()       { assert_eq!("1h".parse::<Timeframe>().unwrap().label(), "1h"); }
    #[test] fn parse_empty()    { assert!("".parse::<Timeframe>().is_err()); }
    #[test] fn intraday_4h()    { assert!("4h".parse::<Timeframe>().unwrap().is_intraday()); }
    #[test] fn not_intraday_1d(){ assert!(!"1d".parse::<Timeframe>().unwrap().is_intraday()); }
}
