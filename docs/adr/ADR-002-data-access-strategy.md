# ADR-002: Data access — direct adapters, no MCP-over-MCP

**Status:** Accepted
**Date:** 2026-04-17

## Context

flow-function-mcp needs OHLCV and SQLite data. Two options:
1. Call `flow-data-mcp` MCP tools over HTTP
2. Access PCTS SQL Server and OMV SQLite directly

## Decision

Access data sources directly (Option 2). Do not depend on flow-data-mcp at runtime.

## Rationale

- MCP-over-MCP creates availability coupling: if flow-data-mcp is down, flow-function-mcp fails
- Adds network + serialisation latency on every indicator request
- Makes integration testing harder (two servers must run)
- ~300 lines of adapter code is acceptable duplication

## Consequence

Both servers access the same PCTS and SQLite sources independently. Schema changes must be updated in both. This is an accepted trade-off.
