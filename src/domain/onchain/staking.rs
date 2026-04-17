/// Cosmos Hub staking event domain types + staking flow aggregation.
///
/// StakingFlowPeriod aggregates delegate/undelegate events into calendar periods.
/// Redelegate events count toward event_count but are excluded from net_atom
/// (they are internal reshuffling, not net inflow/outflow).
///
/// flow_direction: "inflow"  when net_atom > tolerance
///                 "outflow" when net_atom < -tolerance
///                 "neutral" otherwise

use chrono::{DateTime, Datelike};
use std::collections::BTreeMap;

// ── Raw data types ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StakeMsgType { Delegate, Undelegate, Redelegate }

#[derive(Debug)]
pub struct StakeMsgTypeParseError;

impl std::fmt::Display for StakeMsgTypeParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "unknown msg_type — expected delegate | undelegate | redelegate")
    }
}
impl std::error::Error for StakeMsgTypeParseError {}

impl std::str::FromStr for StakeMsgType {
    type Err = StakeMsgTypeParseError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "delegate"   => Ok(Self::Delegate),
            "undelegate" => Ok(Self::Undelegate),
            "redelegate" => Ok(Self::Redelegate),
            _            => Err(StakeMsgTypeParseError),
        }
    }
}

impl StakeMsgType {
    pub fn as_db_str(self) -> &'static str {
        match self {
            Self::Delegate   => "delegate",
            Self::Undelegate => "undelegate",
            Self::Redelegate => "redelegate",
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CosmosStakeEvent {
    pub tx_hash:       String,
    pub msg_index:     i64,
    pub block_height:  i64,
    pub timestamp:     i64,
    pub msg_type:      StakeMsgType,
    pub delegator:     String,
    pub validator:     String,
    pub validator_dst: Option<String>,
    pub amount_atom:   f64,
}

// ── Computed output ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PeriodType { Daily, Weekly, Monthly }

#[derive(Debug)]
pub struct PeriodTypeParseError;

impl std::fmt::Display for PeriodTypeParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "unknown period_type — expected daily | weekly | monthly")
    }
}
impl std::error::Error for PeriodTypeParseError {}

impl std::str::FromStr for PeriodType {
    type Err = PeriodTypeParseError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "daily"   => Ok(Self::Daily),
            "weekly"  => Ok(Self::Weekly),
            "monthly" => Ok(Self::Monthly),
            _         => Err(PeriodTypeParseError),
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct StakingFlowPeriod {
    pub period:           String,
    pub delegated_atom:   f64,
    pub undelegated_atom: f64,
    pub net_atom:         f64,
    pub flow_direction:   String,
    pub event_count:      u32,
}

pub fn compute_staking_flow(
    events:      &[CosmosStakeEvent],
    period_type: PeriodType,
) -> Vec<StakingFlowPeriod> {
    // BTreeMap gives sorted-by-period output automatically.
    let mut by_period: BTreeMap<String, (f64, f64, u32)> = BTreeMap::new();

    for e in events {
        let key = period_key(e.timestamp, period_type);
        let entry = by_period.entry(key).or_insert((0.0, 0.0, 0));
        match e.msg_type {
            StakeMsgType::Delegate   => { entry.0 += e.amount_atom; entry.2 += 1; }
            StakeMsgType::Undelegate => { entry.1 += e.amount_atom; entry.2 += 1; }
            StakeMsgType::Redelegate => { entry.2 += 1; }  // count only, excluded from net
        }
    }

    by_period.into_iter().map(|(period, (delegated, undelegated, count))| {
        let net = delegated - undelegated;
        let flow_direction = if net > 0.01 { "inflow" }
                             else if net < -0.01 { "outflow" }
                             else { "neutral" }.to_string();
        StakingFlowPeriod {
            period,
            delegated_atom:   delegated,
            undelegated_atom: undelegated,
            net_atom:         net,
            flow_direction,
            event_count:      count,
        }
    }).collect()
}

fn period_key(timestamp: i64, period_type: PeriodType) -> String {
    let dt = DateTime::from_timestamp(timestamp, 0)
        .unwrap_or_else(|| DateTime::UNIX_EPOCH);
    match period_type {
        PeriodType::Daily   => dt.format("%Y-%m-%d").to_string(),
        PeriodType::Weekly  => {
            let iso = dt.iso_week();
            format!("{}-W{:02}", iso.year(), iso.week())
        }
        PeriodType::Monthly => dt.format("%Y-%m").to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(ts: i64, msg_type: StakeMsgType, amount: f64) -> CosmosStakeEvent {
        CosmosStakeEvent {
            tx_hash: "0x".into(), msg_index: 0, block_height: 1,
            timestamp: ts, msg_type,
            delegator: "d".into(), validator: "v".into(),
            validator_dst: None, amount_atom: amount,
        }
    }

    #[test]
    fn net_atom_correct() {
        let ts = 1_700_000_000_i64; // 2023-11-14
        let events = vec![
            event(ts, StakeMsgType::Delegate,   1000.0),
            event(ts, StakeMsgType::Undelegate,  400.0),
        ];
        let flow = compute_staking_flow(&events, PeriodType::Daily);
        assert_eq!(flow.len(), 1);
        assert!((flow[0].net_atom - 600.0).abs() < 1e-6);
        assert_eq!(flow[0].flow_direction, "inflow");
    }

    #[test]
    fn redelegate_counted_but_not_in_net() {
        let ts = 1_700_000_000_i64;
        let events = vec![
            event(ts, StakeMsgType::Redelegate, 500.0),
        ];
        let flow = compute_staking_flow(&events, PeriodType::Daily);
        assert!((flow[0].net_atom).abs() < 1e-6);
        assert_eq!(flow[0].event_count, 1);
    }

    #[test]
    fn outflow_direction() {
        let ts = 1_700_000_000_i64;
        let events = vec![
            event(ts, StakeMsgType::Undelegate, 800.0),
            event(ts, StakeMsgType::Delegate,   200.0),
        ];
        let flow = compute_staking_flow(&events, PeriodType::Daily);
        assert_eq!(flow[0].flow_direction, "outflow");
    }
}
