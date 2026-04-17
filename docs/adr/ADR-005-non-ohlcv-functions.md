# ADR-005: Non-OHLCV functions — data sources and contracts

**Status:** Accepted
**Date:** 2026-04-17

## Context

Four tools use data sources that are **not OHLCV from PCTS**. Their inputs, sources, and output shapes differ from the standard indicator pattern. Documenting explicitly to prevent misuse.

## Tools

### `governance_signal`
- **Source:** `asset_governance_state` JOIN `asset_ath` JOIN `asset_governance_config` (OMV SQLite)
- **Input:** `pair?: String` — omit for all pairs
- **Output:** `GovernanceSignal { pair, state, ha_color, depression_pct, entry_levels, ready_for_entry, signal_strength }`
- `ready_for_entry = state == "entry_ready"` — computed, not stored
- `signal_strength`: +0.4 (entry_ready) + 0.3 (blue HA) + 0.3 (depression <= -90%)

### `orderbook_pressure`
- **Source:** `kraken_orderbook` table (OMV SQLite) — 1-min Kraken WS v2 snapshots
- **Input:** `pair: String, last_n?: u32`
- **Output:** `OrderbookPressurePoint { ts, bid_ask_ratio_10, bid_ask_ratio_25, bid_ask_ratio_50, dominant_side, spread_bps }`
- `dominant_side = if bid_ask_ratio_25 > 1.0 { Bullish } else { Bearish }`

### `staking_flow`
- **Source:** `cosmos_stake_events` (OMV SQLite) — msg_type IN ('delegate', 'undelegate')
- **Input:** `last_n?: u32, period_type?: "day" | "week"`
- **Output:** `StakingFlowPeriod { period, delegated_atom, undelegated_atom, net_atom, flow_direction, event_count }`
- `net_atom = delegated - undelegated`; `flow_direction = if net_atom > 0.0 { Bullish } else { Bearish }`

### `wallet_flow`
- **Source:** `transfer_events` JOIN `wallet_classifications` (OMV SQLite) — class = 'exchange' only
- **Input:** `token: String, last_n?: u32`
- **Output:** `WalletFlowPeriod { period, exchange_inflow, exchange_outflow, net_flow, flow_direction, transfer_count }`
- `net_flow = outflow - inflow` — **positive is bullish** (tokens leaving exchanges)
- Excludes non-exchange-classified wallets

## Consequence

Agents must understand that `governance_signal`, `orderbook_pressure`, `staking_flow`, and `wallet_flow` do not accept OHLCV parameters. The MCP tool descriptions make this explicit.
