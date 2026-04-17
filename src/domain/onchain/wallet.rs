/// Wallet flow computation.
///
/// Aggregates ERC-20 transfers into exchange inflow/outflow by identifying
/// exchange-classified wallet addresses.
///
/// exchange_inflow:  sum(amount_norm) where to_addr is classified as Exchange
/// exchange_outflow: sum(amount_norm) where from_addr is classified as Exchange
/// net_flow = outflow - inflow  (positive = tokens LEAVING exchanges = bullish)
///
/// Transfers are grouped by calendar day (UTC from unix timestamp).

use chrono::DateTime;
use std::collections::{BTreeMap, HashSet};

// ── Raw data types ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WalletClass {
    Exchange,
    Dex,
    Relay,
    Whale,
    Foundation,
    Unknown,
}

impl std::str::FromStr for WalletClass {
    type Err = std::convert::Infallible;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s.to_lowercase().as_str() {
            "exchange"   => Self::Exchange,
            "dex"        => Self::Dex,
            "relay"      => Self::Relay,
            "whale"      => Self::Whale,
            "foundation" => Self::Foundation,
            _            => Self::Unknown,
        })
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct WalletClassification {
    pub address:       String,
    pub class:         WalletClass,
    pub confidence:    f64,
    pub source:        String,
    pub classified_at: i64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TransferEvent {
    pub tx_hash:      String,
    pub log_index:    i64,
    pub block_number: i64,
    pub timestamp:    i64,
    pub token:        String,
    pub from_addr:    String,
    pub to_addr:      String,
    pub amount_raw:   String,
    pub amount_norm:  f64,
}

// ── Computed output ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct WalletFlowPeriod {
    pub period:           String,   // "YYYY-MM-DD"
    pub exchange_inflow:  f64,      // tokens arriving at exchanges
    pub exchange_outflow: f64,      // tokens leaving exchanges
    pub net_flow:         f64,      // outflow - inflow (positive = bullish)
    pub flow_direction:   String,   // "outflow" | "inflow" | "neutral"
    pub transfer_count:   u32,
}

pub fn compute_wallet_flow(
    transfers: &[TransferEvent],
    wallets:   &[WalletClassification],
) -> Vec<WalletFlowPeriod> {
    let exchange_addrs: HashSet<&str> = wallets.iter()
        .filter(|w| w.class == WalletClass::Exchange)
        .map(|w| w.address.as_str())
        .collect();

    // (inflow, outflow, count) per day
    let mut by_day: BTreeMap<String, (f64, f64, u32)> = BTreeMap::new();

    for t in transfers {
        let day = DateTime::from_timestamp(t.timestamp, 0)
            .unwrap_or_else(|| DateTime::UNIX_EPOCH)
            .format("%Y-%m-%d")
            .to_string();

        let entry = by_day.entry(day).or_insert((0.0, 0.0, 0));
        entry.2 += 1;

        let to_exchange   = exchange_addrs.contains(t.to_addr.as_str());
        let from_exchange = exchange_addrs.contains(t.from_addr.as_str());

        if to_exchange && !from_exchange {
            entry.0 += t.amount_norm;  // inflow
        } else if from_exchange && !to_exchange {
            entry.1 += t.amount_norm;  // outflow
        }
        // exchange-to-exchange transfers: not counted in either direction
    }

    by_day.into_iter().map(|(period, (inflow, outflow, count))| {
        let net = outflow - inflow;
        let flow_direction = if net > 0.01 { "outflow" }
                             else if net < -0.01 { "inflow" }
                             else { "neutral" }.to_string();
        WalletFlowPeriod { period, exchange_inflow: inflow, exchange_outflow: outflow,
                           net_flow: net, flow_direction, transfer_count: count }
    }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wallet(addr: &str, class: WalletClass) -> WalletClassification {
        WalletClassification { address: addr.into(), class, confidence: 1.0,
                               source: "test".into(), classified_at: 0 }
    }

    fn transfer(ts: i64, from: &str, to: &str, amount: f64) -> TransferEvent {
        TransferEvent {
            tx_hash: "0x".into(), log_index: 0, block_number: 1,
            timestamp: ts, token: "ENJ".into(),
            from_addr: from.into(), to_addr: to.into(),
            amount_raw: "0".into(), amount_norm: amount,
        }
    }

    #[test]
    fn inflow_to_exchange() {
        let ts  = 1_700_000_000_i64;
        let ws  = vec![wallet("0xexchange", WalletClass::Exchange)];
        let txs = vec![transfer(ts, "0xuser", "0xexchange", 100.0)];
        let flow = compute_wallet_flow(&txs, &ws);
        assert!((flow[0].exchange_inflow - 100.0).abs() < 1e-9);
        assert_eq!(flow[0].flow_direction, "inflow");
    }

    #[test]
    fn outflow_from_exchange_is_bullish() {
        let ts  = 1_700_000_000_i64;
        let ws  = vec![wallet("0xexchange", WalletClass::Exchange)];
        let txs = vec![transfer(ts, "0xexchange", "0xuser", 200.0)];
        let flow = compute_wallet_flow(&txs, &ws);
        assert!(flow[0].net_flow > 0.0, "positive net_flow = bullish");
        assert_eq!(flow[0].flow_direction, "outflow");
    }

    #[test]
    fn exchange_to_exchange_not_counted() {
        let ts  = 1_700_000_000_i64;
        let ws  = vec![wallet("0xex1", WalletClass::Exchange),
                       wallet("0xex2", WalletClass::Exchange)];
        let txs = vec![transfer(ts, "0xex1", "0xex2", 500.0)];
        let flow = compute_wallet_flow(&txs, &ws);
        assert!((flow[0].exchange_inflow).abs() < 1e-9);
        assert!((flow[0].exchange_outflow).abs() < 1e-9);
    }
}
