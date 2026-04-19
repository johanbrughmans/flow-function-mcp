/// FibProfile — maturity-aware parameter set for all Fibonacci tools.
///
/// Three canonical profiles aligned with asset lifecycle phase:
///   "nascent"    — early formation, wide tolerances, exploratory
///   "developing" — structure forming, intermediate parameters
///   "mature"     — established structure, strict Boroden/DiNapoli standard (default)
///
/// Parse-don't-validate: use `FibProfile::parse(s)` at every MCP boundary.
/// Unknown profile name → immediate Err, never silent fallback.

#[derive(Debug, Clone)]
pub struct FibProfile {
    pub name: String,

    // Clustering
    pub cluster_tolerance:        f64,
    pub min_cluster_size:         usize,

    // Pivot detection
    pub pivot_window:             usize,
    pub min_pivot_count:          usize,

    // ATR compression
    pub atr_compression_ratio:    f64,
    pub atr_compression_lookback: usize,

    // Harmonic patterns (ADR-001)
    pub harmonic_ratio_tolerance: f64,
    pub harmonic_patterns:        Vec<String>,

    // Fibonacci time zones (ADR-002)
    pub time_zone_enabled:        bool,
    pub time_zone_max_bars:       u32,

    // Observability
    pub exploratory:              bool,
}

impl FibProfile {
    pub fn mature() -> Self {
        Self {
            name:                     "mature".into(),
            cluster_tolerance:        0.003,
            min_cluster_size:         3,
            pivot_window:             1,
            min_pivot_count:          8,
            atr_compression_ratio:    0.75,
            atr_compression_lookback: 20,
            harmonic_ratio_tolerance: 0.03,
            harmonic_patterns:        vec!["Gartley".into(), "Bat".into(), "Butterfly".into(), "Crab".into()],
            time_zone_enabled:        false,
            time_zone_max_bars:       89,
            exploratory:              false,
        }
    }

    pub fn developing() -> Self {
        Self {
            name:                     "developing".into(),
            cluster_tolerance:        0.005,
            min_cluster_size:         2,
            pivot_window:             1,
            min_pivot_count:          6,
            atr_compression_ratio:    0.70,
            atr_compression_lookback: 15,
            harmonic_ratio_tolerance: 0.05,
            harmonic_patterns:        vec!["Gartley".into(), "Bat".into(), "Butterfly".into()],
            time_zone_enabled:        true,
            time_zone_max_bars:       55,
            exploratory:              false,
        }
    }

    pub fn nascent() -> Self {
        Self {
            name:                     "nascent".into(),
            cluster_tolerance:        0.008,
            min_cluster_size:         2,
            pivot_window:             1,
            min_pivot_count:          4,
            atr_compression_ratio:    0.60,
            atr_compression_lookback: 10,
            harmonic_ratio_tolerance: 0.07,
            harmonic_patterns:        vec!["Gartley".into(), "Bat".into()],
            time_zone_enabled:        true,
            time_zone_max_bars:       34,
            exploratory:              true,
        }
    }

    pub fn parse(s: &str) -> Result<Self, String> {
        match s {
            "mature"     => Ok(Self::mature()),
            "developing" => Ok(Self::developing()),
            "nascent"    => Ok(Self::nascent()),
            other        => Err(format!(
                "unknown FibProfile '{}'. Valid: nascent | developing | mature",
                other
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mature_has_boroden_standard_values() {
        let p = FibProfile::mature();
        assert!((p.cluster_tolerance - 0.003).abs() < f64::EPSILON);
        assert_eq!(p.min_cluster_size, 3);
        assert!(!p.exploratory);
        assert!(!p.time_zone_enabled);
    }

    #[test]
    fn nascent_is_exploratory_with_wide_tolerance() {
        let p = FibProfile::nascent();
        assert!((p.cluster_tolerance - 0.008).abs() < f64::EPSILON);
        assert_eq!(p.min_cluster_size, 2);
        assert!(p.exploratory);
        assert!(p.time_zone_enabled);
    }

    #[test]
    fn developing_is_intermediate() {
        let d = FibProfile::developing();
        let n = FibProfile::nascent();
        let m = FibProfile::mature();
        assert!(d.cluster_tolerance > m.cluster_tolerance);
        assert!(d.cluster_tolerance < n.cluster_tolerance);
        assert!(!d.exploratory);
    }

    #[test]
    fn parse_known_profiles() {
        assert!(FibProfile::parse("mature").is_ok());
        assert!(FibProfile::parse("developing").is_ok());
        assert!(FibProfile::parse("nascent").is_ok());
    }

    #[test]
    fn parse_unknown_returns_err_listing_valid_options() {
        let err = FibProfile::parse("expert").unwrap_err();
        assert!(err.contains("nascent"));
        assert!(err.contains("developing"));
        assert!(err.contains("mature"));
    }
}
