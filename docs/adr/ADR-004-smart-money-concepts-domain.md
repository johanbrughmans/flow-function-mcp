# ADR-004: Smart Money Concepts domain model

**Status:** Accepted
**Date:** 2026-04-17

## Context

SMC (FVG, Order Blocks, Structure, Liquidity) are pattern-detection algorithms over OHLCV. Each produces a different output type. Need a clean domain model.

## Decision

### Types (parse-don't-validate)

```rust
enum Direction { Bullish, Bearish }  // FromStr rejects unknown
enum StructureType { Bos, Choch }    // serialises as "bos" / "choch"
```

### Output types

```rust
struct FvgZone         { ts, direction: Direction, top: f64, bottom: f64, filled: bool }
struct OrderBlock      { ts, direction: Direction, top: f64, bottom: f64, broken: bool }
struct StructureEvent  { ts, event_type: StructureType, level: f64, direction: Direction }
struct LiquidityLevel  { ts, price: f64, side: Direction, swept: bool }
```

### Stateless computation

All SMC functions are stateless — computed fresh on each request. No zone invalidation state is persisted. `filled` and `broken` are computed against the current candle series.

### Minimum candle requirements

- FVG: minimum 3 candles (needs candle[i-2])
- Order Blocks: minimum 5 candles (needs swing detection context)
- Structure: minimum 5 candles (needs swing high/low history)
- Liquidity: minimum 3 candles

## Consequence

SMC output is consistent and reproducible. Callers can filter on `filled: false` or `broken: false` to get active zones.
