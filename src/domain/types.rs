/// Shared SMC / structural domain types.
/// All enums use parse-don't-validate via FromStr.

use std::num::NonZeroU32;

// ── Direction ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Direction { Bullish, Bearish }

#[derive(Debug)]
pub struct DirectionParseError(String);

impl std::fmt::Display for DirectionParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "unknown direction '{}' — expected bullish | bearish", self.0)
    }
}
impl std::error::Error for DirectionParseError {}

impl std::str::FromStr for Direction {
    type Err = DirectionParseError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "bullish" | "bull" | "long"  => Ok(Self::Bullish),
            "bearish" | "bear" | "short" => Ok(Self::Bearish),
            other => Err(DirectionParseError(other.to_string())),
        }
    }
}

// ── StructureType ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StructureType { Bos, Choch }

// ── Period ────────────────────────────────────────────────────────────────────

/// Newtype over NonZeroU32 — period=0 is rejected at construction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Period(NonZeroU32);

impl Period {
    pub fn new(n: u32) -> Result<Self, anyhow::Error> {
        NonZeroU32::new(n)
            .map(Self)
            .ok_or_else(|| anyhow::anyhow!("period must be > 0"))
    }

    pub fn get(self) -> usize { self.0.get() as usize }
}

impl Default for Period {
    fn default() -> Self {
        Self(NonZeroU32::new(14).expect("14 > 0"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn direction_bullish()  { assert_eq!("bullish".parse::<Direction>().unwrap(), Direction::Bullish); }
    #[test] fn direction_bear()     { assert_eq!("bear".parse::<Direction>().unwrap(), Direction::Bearish); }
    #[test] fn direction_unknown()  { assert!("sideways".parse::<Direction>().is_err()); }
    #[test] fn period_zero_fails()  { assert!(Period::new(0).is_err()); }
    #[test] fn period_nonzero_ok()  { assert_eq!(Period::new(14).unwrap().get(), 14); }
}
