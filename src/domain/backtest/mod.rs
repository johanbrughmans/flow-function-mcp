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

pub mod multi_anchor_fib_backtest;
pub mod order_blocks_backtest;
pub mod order_flow_backtest;
pub mod structure_backtest;
