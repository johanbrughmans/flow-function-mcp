/// Indicator-level backtest validation (ADR-017).
///
/// Walk-forward evaluation of indicators in isolation, before strategy composition.
/// Each module validates one indicator's predictions against future price action
/// using only data available at prediction time.
///
/// Current modules:
///   - multi_anchor_fib_backtest: validates score-bucket calibration for
///     multi-anchor Fibonacci confluence (Stories #39 + #43).
///   - structure_backtest: validates BOS/CHoCH follow-through (Story #40).
///   - order_flow_backtest: validates net_aggression forward-return bucketing (Story #40).
///   - order_blocks_backtest: validates OB retest-respect rate (Story #40).
///   - orderbook_pressure_backtest: validates OBI daily forward-return bucketing (Story #51 / #40).
///   - fib_targets_backtest: validates Fibonacci TP target hit rates by anchor strength (Story #52 / #40).
///   - harmonic_patterns_backtest: validates XABCD quality-bucket calibration for reversal follow-through (Story #53 / #40).
///   - fib_time_zones_backtest: validates temporal acceleration claim for Fibonacci Time Zones (Story #54 / #40).

pub mod fib_targets_backtest;
pub mod fib_time_zones_backtest;
pub mod harmonic_patterns_backtest;
pub mod multi_anchor_fib_backtest;
pub mod order_blocks_backtest;
pub mod order_flow_backtest;
pub mod orderbook_pressure_backtest;
pub mod structure_backtest;
