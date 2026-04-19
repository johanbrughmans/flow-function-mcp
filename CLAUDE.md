# CLAUDE.md — flow-function-mcp

> **Inherits from:** `~/.claude/CLAUDE.md` (Global BIOS)
> **Project:** flow-function-mcp — Function Layer MCP Server

---

## Project Identity

| Attribute | Value |
|-----------|-------|
| Repo | `johanbrughmans/flow-function-mcp` |
| Layer | Function (TGF 5-layer: layer 3) |
| Role | Stateless signal computation — indicators, SMC, order flow, on-chain analysis |
| Language | Rust (edition 2021) |
| Architecture | Hexagonal + DDD + parse-don't-validate |
| Deploy target | OMV ARM64 (`192.168.0.126`) — systemd `flow-function-mcp.service` |
| Port | `3467` |
| MCP endpoint | `http://192.168.0.126:3467/mcp` |

---

## Architecture

```
domain/           ← zero I/O; pure computation functions
  indicators/     ← RSI, MA, ATR, Bollinger, Donchian, HV
  smc/            ← FVG, OrderBlocks, Structure, Liquidity
                    fib_confluence, fib_targets, harmonic_patterns
                    pivots (shared pivot detection), fib_profile
  ha.rs           ← HA computation + pattern detection + ohlcv_to_ha()
  flow.rs         ← OrderFlow ratios (MB/MS/LB/LS)
  onchain/        ← GovernanceSignal, OrderbookPressure, StakingFlow, WalletFlow
  types.rs        ← Direction, StructureType, Zone, Level, Period
  candle.rs       ← OhlcvCandle (with optional trade-flow fields)
  pair.rs         ← Pair newtype
  timeframe.rs    ← Timeframe enum
  window.rs       ← Window
ports/
  market_data.rs  ← MarketDataPort (fetch OHLCV from PCTS)
  onchain.rs      ← OnChainPort (fetch from OMV SQLite)
adapters/
  pcts/           ← PCTS SQL Server via tiberius (OHLCV)
  sqlite/         ← OMV SQLite via rusqlite (orderbook, cosmos, transfers, wallets, governance)
  composite.rs    ← routing
  mcp/
    candle_source.rs ← CandleSource struct + apply_candle_source (shared by all OHLCV tools)
    server.rs        ← FlowFunctionServer (19 tools)
main.rs           ← bootstrap: reads env, builds adapters, starts HTTP MCP
```

---

## Data Sources

| Source | Type | Data |
|--------|------|------|
| PCTS (`192.168.0.137:1433`) | SQL Server via tiberius | OHLCV + trade-flow (MB/MS/LB/LS) |
| OMV SQLite (`/opt/enj-flow/data/token-flow.db`) | rusqlite read-only | orderbook, cosmos_stake_events, transfer_events, wallet_classifications, governance |

---

## MCP Tools (all QUERY, read-only)

### OHLCV Indicators — support `candle_source?`
| Tool | Args | Returns |
|------|------|---------|
| `rsi` | `pair, tf, last_n, period?, candle_source?` | `[{ts, rsi}]` |
| `ma_cross` | `pair, tf, last_n, fast?, slow?, ma_type?, candle_source?` | `[{ts, fast_ma, slow_ma, cross?}]` |
| `atr` | `pair, tf, last_n, period?, candle_source?` | `[{ts, atr}]` |
| `bollinger` | `pair, tf, last_n, period?, n_std?, candle_source?` | `[{ts, middle, upper, lower, width, pct_b}]` |
| `donchian` | `pair, tf, last_n, period?, candle_source?` | `[{ts, upper, mid, lower, width}]` |
| `volatility` | `pair, tf, last_n, period?, candle_source?` | `[{ts, hv}]` (annualised %) |

### Smart Money Concepts — support `candle_source?`
| Tool | Args | Returns |
|------|------|---------|
| `fvg` | `pair, tf, last_n, candle_source?` | `[{ts, direction, top, bottom, filled}]` |
| `order_blocks` | `pair, tf, last_n, candle_source?` | `[{ts, direction, top, bottom, broken}]` |
| `structure` | `pair, tf, last_n, candle_source?` | `[{ts, event_type, level, direction}]` |
| `liquidity` | `pair, tf, last_n, candle_source?` | `[{ts, price, side, swept}]` |
| `fib_confluence` | `pair, tf, last_n?, profile?, candle_source?` | `[{price, strength, direction, levels, atr_compressed, distance_pct}]` |
| `fib_targets` | `pair, tf, last_n?, entry_price, profile?, candle_source?` | `{current_price, entry_price, pnl_pct, targets, nearest_support, profile, exploratory}` |
| `harmonic_patterns` | `pair, tf, last_n?, profile?, candle_source?` | `[{ts_x..ts_d, pattern, direction, d_price, xabcd_quality, exploratory}]` |

### Price Action + Flow — no `candle_source` (excluded by design)
| Tool | Args | Returns | Why excluded |
|------|------|---------|-------------|
| `ha_pattern` | `pair, tf, last_n` | `[{ts, color, has_lower_wick, has_upper_wick, consecutive_count, reversal, lower_wick_signal}]` | Already HA internally |
| `order_flow` | `pair, tf, last_n` | `[{ts, mb_ms_ratio?, lb_ls_ratio?, net_aggression?, market_pct?, avg_mb_size?, avg_ms_size?}]` | MB/MS/LB/LS columns drive the signal |

### Non-OHLCV (different data sources — see ADR-005)
| Tool | Args | Source | Returns |
|------|------|--------|---------|
| `governance_signal` | `pair?` | `asset_governance_state` + `asset_ath` | `[{pair, state, ha_color, depression_pct, entry_levels, ready_for_entry, signal_strength}]` |
| `orderbook_pressure` | `pair, last_n?` | `kraken_orderbook` | `[{ts, bid_ask_ratio_10/25/50, dominant_side, spread_bps}]` |
| `staking_flow` | `last_n?, period_type?` | `cosmos_stake_events` | `[{period, delegated_atom, undelegated_atom, net_atom, flow_direction, event_count}]` |
| `wallet_flow` | `token, last_n?` | `transfer_events` + `wallet_classifications` | `[{period, exchange_inflow, exchange_outflow, net_flow, flow_direction, transfer_count}]` |

### FibProfile (Fibonacci + Harmonic tool parameter)
| Profile | cluster_tol | harmonic_tol | harmonic_patterns | exploratory |
|---------|-------------|--------------|-------------------|-------------|
| `nascent` | 0.8% | 7% | Gartley, Bat | true |
| `developing` | 0.5% | 5% | Gartley, Bat, Butterfly | false |
| `mature` | 0.3% | 3% | Gartley, Bat, Butterfly, Crab | false |

---

## Governance

- CQRS: all 19 tools are QUERY — no write operations
- GitHub Issues: Epic #9 (Fibonacci Extended), Stories #10–#14
- Mutations require explicit mandaat per global CLAUDE.md

---

## Dev Conventions

- **Parse-don't-validate**: `Pair::parse()`, `Timeframe::from_str()`, `FibProfile::parse()` at every MCP boundary
- **Strong typing**: `Direction` enum, `StructureType` enum, `FibProfile` newtype-like struct
- **SoC**: `domain/` has zero imports from `adapters/` or `ports/`
- **Seed lookback**: fetch `last_n + seed` candles, compute, trim to `last_n` — all indicators aligned
- **`spawn_blocking`** for all rusqlite calls
- **Division-by-zero**: all ratio fields are `Option<f64>`, return `None` not panic
- **Harmonic pivots**: `pivots.rs` is the shared pivot detection module (pub(crate))
- **CandleSource convention**: every new OHLCV tool input struct MUST include:
  ```rust
  #[serde(flatten)]
  source: CandleSource,
  ```
  Then call `apply_candle_source(raw, &req.source.candle_source)?` after fetching OHLCV.
  Omit only for tools where HA OHLC has no semantic meaning (ha_pattern, order_flow).

---

## Env vars (`.env` on OMV at `/opt/flow-function-mcp/.env`)

```
FLOW_FUNCTION_PORT=3467
FLOW_DATA_DB=/opt/enj-flow/data/token-flow.db
PCTS_HOST=192.168.0.137
PCTS_USER=sql-admin-2
PCTS_PASS=DeWindWaaitHard01$
```

---

## Deploy

CI: GitHub Actions self-hosted runner `flow-function-mcp-arm64` on OMV.
Push to `main` → pull → `cargo build --release` → `systemctl restart flow-function-mcp`.

Health check: `curl http://192.168.0.126:3467/health`
