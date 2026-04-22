/// Indicator-level backtest validation (ADR-017).
///
/// Walk-forward evaluation of indicators in isolation, before strategy composition.
/// Each module validates one indicator's predictions against future price action
/// using only data available at prediction time.
///
/// Current modules:
///   - multi_anchor_fib_backtest: validates score-bucket calibration for
///     multi-anchor Fibonacci confluence (Story #39).

pub mod multi_anchor_fib_backtest;
