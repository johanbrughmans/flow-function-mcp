# ADR-003: Indicator computation — seed lookback pattern

**Status:** Accepted
**Date:** 2026-04-17

## Context

All OHLCV indicators require a warm-up period. RSI(14) needs 15 candles before the first reliable value. An agent requests `last_n` candles of output — they should not be aware of seed mechanics.

## Decision

1. Fetch `last_n + seed_lookback` candles from PCTS
2. Compute indicator over the full series
3. Return only the last `last_n` values (seed candles discarded)

Output series is always aligned with ts: same length as `last_n`, ascending.

## Seed lookback per indicator

| Indicator | Seed |
|-----------|------|
| RSI | `period + 1` |
| MA cross | `slow_period` |
| ATR | `period + 1` |
| Bollinger | `period` |
| Donchian | `period` (window, no seed needed) |
| Historical Volatility | `period + 1` |
| HA pattern | `SEED_LOOKBACK = 10` (same as flow-data-mcp) |

## Consequence

All indicator output lengths equal the requested `last_n`. Callers need not specify warm-up.
