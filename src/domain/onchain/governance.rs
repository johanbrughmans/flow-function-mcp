/// Governance domain types + signal computation.
///
/// GovernanceSnapshot: raw data from asset_governance_state + asset_governance_config + asset_ath.
/// GovernanceSignal:   enriched output — adds ready_for_entry flag and signal_strength score.
///
/// Signal scoring (max 1.0):
///   +0.4  state == entry_ready
///   +0.3  ha_color == blue
///   +0.3  depression_pct ≤ -90%

use crate::domain::candle::HaColor;

// ── Raw data type ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GovernanceSnapshot {
    pub pair:             String,
    pub state:            GovernanceState,
    pub ha_color:         Option<HaColor>,
    pub has_lower_wick:   Option<bool>,
    pub depression_pct:   Option<f64>,
    pub entry_levels:     Vec<f64>,
    pub last_assessed_at: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GovernanceState {
    Watch,
    EntryReady,
    Active,
    ExitSignaled,
    Exited,
}

#[derive(Debug)]
pub struct GovernanceStateParseError(String);

impl std::fmt::Display for GovernanceStateParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "unknown governance state '{}'", self.0)
    }
}
impl std::error::Error for GovernanceStateParseError {}

impl std::str::FromStr for GovernanceState {
    type Err = GovernanceStateParseError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "watch"         => Ok(Self::Watch),
            "entry_ready"   => Ok(Self::EntryReady),
            "active"        => Ok(Self::Active),
            "exit_signaled" => Ok(Self::ExitSignaled),
            "exited"        => Ok(Self::Exited),
            _               => Err(GovernanceStateParseError(s.to_string())),
        }
    }
}

// ── Computed output ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GovernanceSignal {
    pub pair:            String,
    pub state:           GovernanceState,
    pub ha_color:        Option<HaColor>,
    pub depression_pct:  Option<f64>,
    pub entry_levels:    Vec<f64>,
    pub ready_for_entry: bool,
    /// Composite score 0.0–1.0 based on state, HA color, depression depth.
    pub signal_strength: f64,
}

pub fn compute_governance_signal(snap: &GovernanceSnapshot) -> GovernanceSignal {
    let ready_for_entry = snap.state == GovernanceState::EntryReady;

    let mut strength = 0.0_f64;
    if ready_for_entry { strength += 0.4; }
    if snap.ha_color == Some(HaColor::Blue)         { strength += 0.3; }
    if snap.depression_pct.map_or(false, |d| d <= -90.0) { strength += 0.3; }

    GovernanceSignal {
        pair:            snap.pair.clone(),
        state:           snap.state,
        ha_color:        snap.ha_color,
        depression_pct:  snap.depression_pct,
        entry_levels:    snap.entry_levels.clone(),
        ready_for_entry,
        signal_strength: (strength * 100.0).round() / 100.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap(state: GovernanceState, ha_color: Option<HaColor>, depression: Option<f64>) -> GovernanceSnapshot {
        GovernanceSnapshot {
            pair: "ENJEUR".into(), state, ha_color, has_lower_wick: None,
            depression_pct: depression, entry_levels: vec![], last_assessed_at: None,
        }
    }

    #[test]
    fn max_strength_when_all_conditions_met() {
        let sig = compute_governance_signal(&snap(
            GovernanceState::EntryReady,
            Some(HaColor::Blue),
            Some(-92.0),
        ));
        assert!((sig.signal_strength - 1.0).abs() < 1e-9);
        assert!(sig.ready_for_entry);
    }

    #[test]
    fn zero_strength_when_no_conditions() {
        let sig = compute_governance_signal(&snap(GovernanceState::Watch, None, None));
        assert!(sig.signal_strength.abs() < 1e-9);
    }

    #[test]
    fn partial_score_entry_ready_only() {
        let sig = compute_governance_signal(&snap(GovernanceState::EntryReady, None, None));
        assert!((sig.signal_strength - 0.4).abs() < 1e-9);
    }
}
