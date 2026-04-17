/// Pair — validated, uppercase trading pair.
/// Parse-don't-validate: the only constructor is `Pair::parse`.

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash,
         serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct Pair(String);

#[derive(Debug)]
pub struct PairParseError;

impl std::fmt::Display for PairParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("pair must not be empty")
    }
}

impl std::error::Error for PairParseError {}

impl Pair {
    pub fn parse(s: &str) -> Result<Self, PairParseError> {
        let upper = s.trim().to_uppercase();
        if upper.is_empty() { return Err(PairParseError); }
        Ok(Self(upper))
    }

    pub fn as_str(&self) -> &str { &self.0 }
}

impl std::fmt::Display for Pair {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::str::FromStr for Pair {
    type Err = PairParseError;
    fn from_str(s: &str) -> Result<Self, Self::Err> { Self::parse(s) }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn uppercase()        { assert_eq!(Pair::parse("enjeur").unwrap().as_str(), "ENJEUR"); }
    #[test] fn trims_whitespace() { assert_eq!(Pair::parse("  btceur  ").unwrap().as_str(), "BTCEUR"); }
    #[test] fn empty_is_error()   { assert!(Pair::parse("").is_err()); }
    #[test] fn whitespace_only()  { assert!(Pair::parse("   ").is_err()); }
}
