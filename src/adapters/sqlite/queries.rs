/// SQLite adapter — reads from OMV token-flow.db (read-only).
/// Provides: governance, orderbook, cosmos_stake_events, transfers, wallets.
/// All methods are synchronous (rusqlite); callers wrap in spawn_blocking.

use anyhow::{Context, Result};
use rusqlite::{Connection, OpenFlags};

use crate::domain::{
    onchain::{
        governance::{GovernanceSnapshot, GovernanceState},
        orderbook::OrderBookSnapshot,
        staking::{CosmosStakeEvent, StakeMsgType},
        wallet::{TransferEvent, WalletClass, WalletClassification},
    },
    candle::HaColor,
    pair::Pair,
};

#[derive(Clone)]
pub struct SqliteAdapter {
    path: String,
}

impl SqliteAdapter {
    pub fn new(db_path: &str) -> Self { Self { path: db_path.to_string() } }

    fn open(&self) -> Result<Connection> {
        Connection::open_with_flags(
            &self.path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        ).with_context(|| format!("cannot open SQLite at {}", self.path))
    }

    // ── Governance ──────────────────────────────────────────────────────────────

    pub fn governance(&self, pair: Option<&Pair>) -> Result<Vec<GovernanceSnapshot>> {
        let conn = self.open()?;

        let (sql, params_box): (String, Vec<Box<dyn rusqlite::ToSql>>) = if let Some(p) = pair {
            (
                "SELECT c.symbol, COALESCE(s.state,'watch'), s.ha_color,
                        s.ha_has_lower_wick, a.pct_from_ath AS depression_pct, c.entry_levels,
                        s.last_assessed_at
                 FROM asset_governance_config c
                 LEFT JOIN asset_governance_state s ON s.symbol = c.symbol
                 LEFT JOIN asset_ath a ON a.symbol = c.symbol
                 WHERE c.active=1 AND c.symbol=?
                 ORDER BY c.symbol".to_string(),
                vec![Box::new(p.as_str().to_string())],
            )
        } else {
            (
                "SELECT c.symbol, COALESCE(s.state,'watch'), s.ha_color,
                        s.ha_has_lower_wick, a.pct_from_ath AS depression_pct, c.entry_levels,
                        s.last_assessed_at
                 FROM asset_governance_config c
                 LEFT JOIN asset_governance_state s ON s.symbol = c.symbol
                 LEFT JOIN asset_ath a ON a.symbol = c.symbol
                 WHERE c.active=1
                 ORDER BY c.symbol".to_string(),
                vec![],
            )
        };

        let mut stmt = conn.prepare(&sql)?;
        let params: Vec<&dyn rusqlite::ToSql> = params_box.iter().map(|b| b.as_ref()).collect();

        let rows: Vec<GovernanceSnapshot> = stmt
            .query_map(params.as_slice(), |row| {
                let symbol:           String         = row.get(0)?;
                let state_str:        String         = row.get(1)?;
                let ha_color_str:     Option<String> = row.get(2)?;
                let lower_wick:       Option<i64>    = row.get(3)?;
                let depression:       Option<f64>    = row.get(4)?;
                let entry_levels_str: Option<String> = row.get(5)?;
                let assessed:         Option<String> = row.get(6)?;
                Ok((symbol, state_str, ha_color_str, lower_wick, depression, entry_levels_str, assessed))
            })?
            .filter_map(|r| r.ok())
            .map(|(symbol, state_str, ha_color_str, lower_wick, depression, entry_levels_str, assessed)| {
                let state = state_str.parse::<GovernanceState>().unwrap_or(GovernanceState::Watch);
                let ha_color = ha_color_str.as_deref().and_then(parse_ha_color);
                let entry_levels = entry_levels_str
                    .as_deref()
                    .and_then(|s| serde_json::from_str::<Vec<f64>>(s).ok())
                    .unwrap_or_default();
                GovernanceSnapshot {
                    pair: symbol, state, ha_color,
                    has_lower_wick: lower_wick.map(|v| v != 0),
                    depression_pct: depression, entry_levels, last_assessed_at: assessed,
                }
            })
            .collect();

        Ok(rows)
    }

    // ── Order book ──────────────────────────────────────────────────────────────

    pub fn orderbook(&self, pair: &Pair, last_n: Option<u32>) -> Result<Vec<OrderBookSnapshot>> {
        let conn  = self.open()?;
        let limit = last_n.unwrap_or(60).min(500);

        let mut stmt = conn.prepare(
            "SELECT ts, mid_price, bid1, ask1, spread_bps,
                    bid_vol_10, ask_vol_10, bid_vol_25, ask_vol_25,
                    bid_vol_50, ask_vol_50, bid_depth, ask_depth, depth_levels,
                    bid_vwap_25, ask_vwap_25, bid_vwap_100, ask_vwap_100,
                    bid_price_range_100, ask_price_range_100,
                    effective_spread_25_bps, bid_level_count, ask_level_count
             FROM kraken_orderbook WHERE db_symbol=? ORDER BY ts DESC LIMIT ?",
        )?;

        let mut rows: Vec<OrderBookSnapshot> = stmt
            .query_map(rusqlite::params![pair.as_str(), i64::from(limit)], row_to_orderbook)?
            .filter_map(|r| r.ok())
            .collect();
        rows.reverse();
        Ok(rows)
    }

    // ── Cosmos ──────────────────────────────────────────────────────────────────

    pub fn cosmos_stake_events(&self, last_n: Option<u32>, msg_type: Option<StakeMsgType>) -> Result<Vec<CosmosStakeEvent>> {
        let conn  = self.open()?;
        let limit = last_n.unwrap_or(100).min(1000);

        let (sql, params): (String, Vec<Box<dyn rusqlite::ToSql>>) = if let Some(mt) = msg_type {
            (
                format!(
                    "SELECT tx_hash, msg_index, block_height, timestamp, msg_type, \
                     delegator, validator, validator_dst, amount_atom \
                     FROM cosmos_stake_events WHERE msg_type=? ORDER BY timestamp DESC LIMIT {limit}"
                ),
                vec![Box::new(mt.as_db_str().to_string())],
            )
        } else {
            (
                format!(
                    "SELECT tx_hash, msg_index, block_height, timestamp, msg_type, \
                     delegator, validator, validator_dst, amount_atom \
                     FROM cosmos_stake_events ORDER BY timestamp DESC LIMIT {limit}"
                ),
                vec![],
            )
        };

        let mut stmt = conn.prepare(&sql)?;
        let p_ref: Vec<&dyn rusqlite::ToSql> = params.iter().map(|b| b.as_ref()).collect();
        let mut rows: Vec<CosmosStakeEvent> = stmt
            .query_map(p_ref.as_slice(), row_to_cosmos_stake_event)?
            .filter_map(|r| r.ok())
            .collect();
        rows.reverse();
        Ok(rows)
    }

    // ── Transfers ───────────────────────────────────────────────────────────────

    pub fn transfers(&self, token: &str, last_n: Option<u32>) -> Result<Vec<TransferEvent>> {
        let conn  = self.open()?;
        let limit = last_n.unwrap_or(100).min(1000);

        let mut stmt = conn.prepare(&format!(
            "SELECT te.tx_hash, te.log_index, te.block_number, te.timestamp, \
                    t.symbol AS token, te.from_addr, te.to_addr, te.amount_raw, te.amount_norm \
             FROM transfer_events te \
             JOIN tokens t ON t.id = te.token_id \
             WHERE t.symbol=? \
             ORDER BY te.timestamp DESC LIMIT {limit}"
        ))?;

        let mut rows: Vec<TransferEvent> = stmt
            .query_map([token], row_to_transfer_event)?
            .filter_map(|r| r.ok())
            .collect();
        rows.reverse();
        Ok(rows)
    }

    // ── Wallets ─────────────────────────────────────────────────────────────────

    pub fn wallets(&self, address: Option<&str>) -> Result<Vec<WalletClassification>> {
        let conn = self.open()?;

        let results: Vec<WalletClassification> = if let Some(addr) = address {
            let mut stmt = conn.prepare(
                "SELECT address, class, confidence, source, classified_at \
                 FROM wallet_classifications WHERE address=? ORDER BY classified_at DESC",
            )?;
            let rows: Vec<WalletClassification> = stmt
                .query_map([addr], row_to_wallet_classification)?
                .filter_map(|r| r.ok())
                .collect();
            rows
        } else {
            let mut stmt = conn.prepare(
                "SELECT address, class, confidence, source, classified_at \
                 FROM wallet_classifications ORDER BY classified_at DESC",
            )?;
            let rows: Vec<WalletClassification> = stmt
                .query_map([], row_to_wallet_classification)?
                .filter_map(|r| r.ok())
                .collect();
            rows
        };

        Ok(results)
    }
}

// ── Row helpers ────────────────────────────────────────────────────────────────

fn row_to_orderbook(row: &rusqlite::Row<'_>) -> rusqlite::Result<OrderBookSnapshot> {
    Ok(OrderBookSnapshot {
        ts:                      row.get(0)?,
        mid_price:               row.get(1)?,
        bid1:                    row.get(2)?,
        ask1:                    row.get(3)?,
        spread_bps:              row.get(4)?,
        bid_vol_10:              row.get(5)?,
        ask_vol_10:              row.get(6)?,
        bid_vol_25:              row.get(7)?,
        ask_vol_25:              row.get(8)?,
        bid_vol_50:              row.get(9)?,
        ask_vol_50:              row.get(10)?,
        bid_depth:               row.get(11)?,
        ask_depth:               row.get(12)?,
        depth_levels:            row.get(13)?,
        bid_vwap_25:             row.get(14)?,
        ask_vwap_25:             row.get(15)?,
        bid_vwap_100:            row.get(16)?,
        ask_vwap_100:            row.get(17)?,
        bid_price_range_100:     row.get(18)?,
        ask_price_range_100:     row.get(19)?,
        effective_spread_25_bps: row.get(20)?,
        bid_level_count:         row.get(21)?,
        ask_level_count:         row.get(22)?,
    })
}

fn row_to_cosmos_stake_event(row: &rusqlite::Row<'_>) -> rusqlite::Result<CosmosStakeEvent> {
    let msg_type_str: String = row.get(4)?;
    let msg_type = msg_type_str.parse::<StakeMsgType>()
        .map_err(|e| rusqlite::Error::FromSqlConversionFailure(
            4, rusqlite::types::Type::Text, Box::new(e),
        ))?;
    Ok(CosmosStakeEvent {
        tx_hash:       row.get(0)?,
        msg_index:     row.get(1)?,
        block_height:  row.get(2)?,
        timestamp:     row.get(3)?,
        msg_type,
        delegator:     row.get(5)?,
        validator:     row.get(6)?,
        validator_dst: row.get(7)?,
        amount_atom:   row.get(8)?,
    })
}

fn row_to_transfer_event(row: &rusqlite::Row<'_>) -> rusqlite::Result<TransferEvent> {
    Ok(TransferEvent {
        tx_hash:      row.get(0)?,
        log_index:    row.get(1)?,
        block_number: row.get(2)?,
        timestamp:    row.get(3)?,
        token:        row.get(4)?,
        from_addr:    row.get(5)?,
        to_addr:      row.get(6)?,
        amount_raw:   row.get(7)?,
        amount_norm:  row.get(8)?,
    })
}

fn row_to_wallet_classification(row: &rusqlite::Row<'_>) -> rusqlite::Result<WalletClassification> {
    let class_str: String = row.get(1)?;
    let class = class_str.parse::<WalletClass>().unwrap_or(WalletClass::Unknown);
    Ok(WalletClassification {
        address:       row.get(0)?,
        class,
        confidence:    row.get(2)?,
        source:        row.get(3)?,
        classified_at: row.get(4)?,
    })
}

fn parse_ha_color(s: &str) -> Option<HaColor> {
    match s {
        "blue"  => Some(HaColor::Blue),
        "green" => Some(HaColor::Green),
        "red"   => Some(HaColor::Red),
        "gray"  => Some(HaColor::Gray),
        _       => None,
    }
}
