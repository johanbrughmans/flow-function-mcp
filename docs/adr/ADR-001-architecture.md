# ADR-001: Architecture — stateless hexagonal compute server

**Status:** Accepted
**Date:** 2026-04-17

## Context

Need a Function Layer (TGF Layer 3) that transforms raw market data into actionable signals and indicators. Sits between Infrastructure (`flow-data-mcp`) and Application layers.

## Decision

Separate stateless MCP server with hexagonal architecture:
- `domain/` — zero I/O; pure computation functions. No imports from adapters/ or ports/.
- `ports/` — async traits defining what data the domain needs
- `adapters/` — PCTS SQL Server + OMV SQLite (same sources as flow-data-mcp, independent access)
- `adapters/mcp/` — inbound adapter exposing 16 QUERY tools

No own storage. All state is derived from inputs per request.

## Structure

```
domain/           ← pure functions: indicators, SMC, HA, flow, onchain
ports/            ← MarketDataPort, OnChainPort (async traits)
adapters/
  pcts/           ← PCTS tiberius adapter (OHLCV)
  sqlite/         ← rusqlite adapter (orderbook, cosmos, transfers, wallets, governance)
  composite.rs    ← routing
  mcp/            ← FlowFunctionServer (16 tools, all QUERY)
main.rs           ← bootstrap
```

## Consequences

- Each server independently deployable — no runtime coupling between layers
- Small duplication of adapter boilerplate vs flow-data-mcp (acceptable)
- Domain functions are pure and trivially unit-testable without any DB
