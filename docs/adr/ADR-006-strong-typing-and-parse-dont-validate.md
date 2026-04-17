# ADR-006: Strong typing and parse-don't-validate

**Status:** Accepted
**Date:** 2026-04-17

## Context

MCP tool inputs arrive as raw strings. Without a systematic approach, invalid values propagate into computation and produce silent garbage output.

## Decision

### Parse-don't-validate — convert at MCP boundary, never inside domain

```rust
// MCP boundary — rejects invalid input before entering domain
fn parse_pair(s: &str) -> Result<Pair, String>       { Pair::parse(s).map_err(|e| e.to_string()) }
fn parse_tf(s: &str)   -> Result<Timeframe, String>  { s.parse().map_err(|e| e.to_string()) }
fn parse_dir(s: &str)  -> Result<Direction, String>  { s.parse().map_err(|e| e.to_string()) }
```

### Strong types

| Type | Guarantee |
|------|-----------|
| `Pair(String)` | Uppercase, non-empty. `Pair::parse("")` -> Err |
| `Timeframe` | Enum — only known timeframes compile |
| `Direction` | Enum — `Bullish \| Bearish` only |
| `StructureType` | Enum — `Bos \| Choch` only |
| `Period(NonZeroU32)` | Cannot be zero — eliminates period=0 divide-by-zero bugs |

### CQRS

All 16 tools are QUERY operations. Zero COMMAND operations exist in this server. The `MarketDataPort` and `OnChainPort` traits expose read-only methods only.

### SoC — domain purity

`domain/` modules have zero imports from `adapters/` or `ports/`. The dependency arrow points inward only: adapters depend on ports depend on domain types.

## Consequence

Invalid inputs are rejected with clear error messages at the MCP tool boundary. Domain functions receive only valid, strongly-typed data. Unit tests for domain functions do not require any adapter setup.
