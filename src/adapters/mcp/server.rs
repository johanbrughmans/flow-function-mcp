/// FlowFunctionServer — MCP inbound adapter (Function Layer, TGF Layer 3).
///
/// 20 QUERY tools (read-only):
///   OHLCV indicators : rsi, ma_cross, atr, bollinger, donchian, volatility
///   SMC              : fvg, order_blocks, structure, liquidity, fib_confluence, fib_targets, harmonic_patterns, fib_time_zones
///   Price action+flow: ha_pattern, order_flow
///   Non-OHLCV        : governance_signal, orderbook_pressure, staking_flow, wallet_flow
///
/// candle_source parameter ("ohlcv" | "ha", default "ohlcv") is available on all
/// OHLCV and SMC tools (13 total). Excluded: ha_pattern (HA internal), order_flow
/// (trade-flow volume columns drive the signal).
///
/// All OHLCV tools apply a seed lookback: fetch last_n + seed candles,
/// compute indicators, return the last last_n output points.

use std::sync::Arc;

use rmcp::{
    ServerHandler,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{Implementation, ServerCapabilities, ServerInfo},
    tool, tool_handler, tool_router,
};
use schemars::JsonSchema;
use serde::Deserialize;

use crate::{
    adapters::mcp::candle_source::{apply_candle_source, CandleSource},
    adapters::composite::CompositeAdapter,
    domain::{
        backtest::multi_anchor_fib_backtest::backtest_multi_anchor_fib,
        backtest::order_blocks_backtest::backtest_order_blocks,
        backtest::order_flow_backtest::backtest_order_flow,
        backtest::structure_backtest::backtest_structure,
        flow::compute_order_flow,
        ha::{compute_ha_patterns, SEED_LOOKBACK},
        indicators::{
            atr::{compute_atr, DEFAULT_PERIOD as ATR_DEFAULT},
            bollinger::{compute_bollinger, DEFAULT_N_STD, DEFAULT_PERIOD as BOLL_DEFAULT},
            donchian::{compute_donchian, DEFAULT_PERIOD as DONCH_DEFAULT},
            ma::{compute_ma_cross, MaType, DEFAULT_FAST, DEFAULT_SLOW},
            rsi::{compute_rsi, DEFAULT_PERIOD as RSI_DEFAULT},
            volatility::{compute_hv, DEFAULT_PERIOD as HV_DEFAULT},
        },
        onchain::{
            governance::compute_governance_signal,
            orderbook::compute_orderbook_pressure,
            staking::{compute_staking_flow, PeriodType},
            wallet::compute_wallet_flow,
        },
        pair::Pair,
        smc::{
            fib_profile::FibProfile,
            fib_targets::compute_fib_targets,
            fib_time_zones::compute_fib_time_zones,
            fvg::compute_fvg,
            harmonics::compute_harmonic_patterns,
            liquidity::compute_liquidity,
            multi_anchor_fib::compute_multi_anchor_fib,
            order_blocks::compute_order_blocks,
            structure::compute_structure,
        },
        timeframe::Timeframe,
        window::Window,
    },
    ports::{market_data::MarketDataPort, onchain::OnChainPort},
};

// ── Server struct ──────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct FlowFunctionServer {
    adapter:     Arc<CompositeAdapter>,
    tool_router: ToolRouter<Self>,
}

impl FlowFunctionServer {
    pub fn new(adapter: Arc<CompositeAdapter>) -> Self {
        Self { adapter, tool_router: Self::tool_router() }
    }
}

// ── Input schemas ──────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize, JsonSchema)]
struct OhlcvIndicatorInput {
    #[schemars(description = "Trading pair e.g. \"ENJEUR\", \"BTCEUR\", \"ATOMEUR\"")]
    pair:   String,
    #[schemars(description = "Timeframe: \"1h\", \"4h\", \"1d\", or \"1w\"")]
    tf:     String,
    #[schemars(description = "Number of output points to return")]
    #[serde(default = "default_last_n")]
    last_n: u32,
    #[schemars(description = "Indicator period (optional, uses default when omitted)")]
    #[serde(default)]
    period: Option<u32>,
    #[serde(flatten)]
    source: CandleSource,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct MaCrossInput {
    #[schemars(description = "Trading pair e.g. \"ENJEUR\"")]
    pair:    String,
    #[schemars(description = "Timeframe: \"1h\", \"4h\", \"1d\", or \"1w\"")]
    tf:      String,
    #[schemars(description = "Number of output points to return")]
    #[serde(default = "default_last_n")]
    last_n:  u32,
    #[schemars(description = "Fast MA period (default 9)")]
    #[serde(default)]
    fast:    Option<u32>,
    #[schemars(description = "Slow MA period (default 21)")]
    #[serde(default)]
    slow:    Option<u32>,
    #[schemars(description = "MA type: \"sma\" or \"ema\" (default \"sma\")")]
    #[serde(default)]
    ma_type: Option<String>,
    #[serde(flatten)]
    source:  CandleSource,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct BollingerInput {
    #[schemars(description = "Trading pair e.g. \"ENJEUR\"")]
    pair:   String,
    #[schemars(description = "Timeframe: \"1h\", \"4h\", \"1d\", or \"1w\"")]
    tf:     String,
    #[schemars(description = "Number of output points to return")]
    #[serde(default = "default_last_n")]
    last_n: u32,
    #[schemars(description = "Bollinger period (default 20)")]
    #[serde(default)]
    period: Option<u32>,
    #[schemars(description = "Number of standard deviations for band width (default 2.0)")]
    #[serde(default)]
    n_std:  Option<f64>,
    #[serde(flatten)]
    source: CandleSource,
}

/// Used by ha_pattern and order_flow — no candle_source (excluded by design).
#[derive(Debug, Deserialize, JsonSchema)]
struct SmcInput {
    #[schemars(description = "Trading pair e.g. \"ENJEUR\"")]
    pair:   String,
    #[schemars(description = "Timeframe: \"1h\", \"4h\", \"1d\", or \"1w\"")]
    tf:     String,
    #[schemars(description = "Number of candles to analyse (seed lookback applied automatically)")]
    #[serde(default = "default_last_n")]
    last_n: u32,
}

/// Used by fvg, order_blocks, structure, liquidity — supports candle_source.
#[derive(Debug, Deserialize, JsonSchema)]
struct SmcSourceInput {
    #[schemars(description = "Trading pair e.g. \"ENJEUR\"")]
    pair:   String,
    #[schemars(description = "Timeframe: \"1h\", \"4h\", \"1d\", or \"1w\"")]
    tf:     String,
    #[schemars(description = "Number of candles to analyse (seed lookback applied automatically)")]
    #[serde(default = "default_last_n")]
    last_n: u32,
    #[serde(flatten)]
    source: CandleSource,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct FibConfluenceInput {
    #[schemars(description = "Trading pair e.g. \"ENJEUR\"")]
    pair:      String,
    #[schemars(description = "Timeframe: \"1h\", \"4h\", \"1d\", or \"1w\"")]
    tf:        String,
    #[schemars(description = "Candles to scan (default 200, min 50). Defines the P3 session range.")]
    #[serde(default = "default_fib_last_n")]
    last_n:    u32,
    #[schemars(description = "Maturity profile: \"nascent\" | \"developing\" | \"mature\" (default \"mature\"). Controls ATR tolerance multiplier.")]
    #[serde(default)]
    profile:   Option<String>,
    #[schemars(description = "Minimum anchor score to include a zone (1–5, default 2). A score of 2 means at least 2 reference frames agree.")]
    #[serde(default)]
    min_score: Option<u8>,
    #[serde(flatten)]
    source:    CandleSource,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct FibConfluenceBacktestInput {
    #[schemars(description = "Trading pair e.g. \"ENJEUR\"")]
    pair:           String,
    #[schemars(description = "Timeframe: \"1h\", \"4h\", \"1d\", or \"1w\"")]
    tf:             String,
    #[schemars(description = "Total candle history to walk-forward over (default 1000, min window_size + lookahead + 50)")]
    #[serde(default = "default_backtest_last_n")]
    last_n:         u32,
    #[schemars(description = "Per-zone computation window in candles (default 200, min 50)")]
    #[serde(default = "default_backtest_window")]
    window_size:    u32,
    #[schemars(description = "Forward validation window in candles (default 20, min 1)")]
    #[serde(default = "default_backtest_lookahead")]
    lookahead_bars: u32,
    #[schemars(description = "Maturity profile: \"nascent\" | \"developing\" | \"mature\" (default \"mature\")")]
    #[serde(default)]
    profile:        Option<String>,
    #[schemars(description = "Minimum anchor score to include a zone (1–5, default 2)")]
    #[serde(default)]
    min_score:      Option<u8>,
    #[serde(flatten)]
    source:         CandleSource,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct OrderBlocksBacktestInput {
    #[schemars(description = "Trading pair e.g. \"ENJEUR\"")]
    pair:           String,
    #[schemars(description = "Timeframe: \"1h\", \"4h\", \"1d\", or \"1w\"")]
    tf:             String,
    #[schemars(description = "Total candle history to iterate over (default 1000)")]
    #[serde(default = "default_backtest_last_n")]
    last_n:         u32,
    #[schemars(description = "Forward validation window in candles (default 10)")]
    #[serde(default = "default_backtest_lookahead")]
    lookahead_bars: u32,
    #[serde(flatten)]
    source:         CandleSource,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct OrderFlowBacktestInput {
    #[schemars(description = "Trading pair e.g. \"ENJEUR\"")]
    pair:           String,
    #[schemars(description = "Timeframe: \"1h\", \"4h\", \"1d\", or \"1w\"")]
    tf:             String,
    #[schemars(description = "Total candle history to iterate over (default 1000)")]
    #[serde(default = "default_backtest_last_n")]
    last_n:         u32,
    #[schemars(description = "Forward-return horizon in candles (default 10)")]
    #[serde(default = "default_backtest_lookahead")]
    lookahead_bars: u32,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct StructureBacktestInput {
    #[schemars(description = "Trading pair e.g. \"ENJEUR\"")]
    pair:             String,
    #[schemars(description = "Timeframe: \"1h\", \"4h\", \"1d\", or \"1w\"")]
    tf:               String,
    #[schemars(description = "Total candle history to walk-forward over (default 1000)")]
    #[serde(default = "default_backtest_last_n")]
    last_n:           u32,
    #[schemars(description = "Minimum history before an event is validated (default 200)")]
    #[serde(default = "default_backtest_window")]
    window_size:      u32,
    #[schemars(description = "Forward validation window in candles (default 10)")]
    #[serde(default = "default_backtest_lookahead")]
    lookahead_bars:   u32,
    #[schemars(description = "Follow-through significance threshold as fraction of break level (default 0.005 = 0.5%)")]
    #[serde(default)]
    follow_threshold: Option<f64>,
    #[serde(flatten)]
    source:           CandleSource,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct FibTargetsInput {
    #[schemars(description = "Trading pair e.g. \"ENJEUR\"")]
    pair:        String,
    #[schemars(description = "Timeframe: \"1h\", \"4h\", \"1d\", or \"1w\"")]
    tf:          String,
    #[schemars(description = "Candles to scan for swing pivots (default 200, min 50)")]
    #[serde(default = "default_fib_last_n")]
    last_n:      u32,
    #[schemars(description = "Entry price paid — must be > 0")]
    entry_price: f64,
    #[schemars(description = "Maturity profile: \"nascent\" | \"developing\" | \"mature\" (default \"mature\")")]
    #[serde(default)]
    profile:     Option<String>,
    #[serde(flatten)]
    source:      CandleSource,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct HarmonicPatternsInput {
    #[schemars(description = "Trading pair e.g. \"ENJEUR\"")]
    pair:    String,
    #[schemars(description = "Timeframe: \"1h\", \"4h\", \"1d\", or \"1w\"")]
    tf:      String,
    #[schemars(description = "Candles to scan for swing pivots (default 200, min 50)")]
    #[serde(default = "default_fib_last_n")]
    last_n:  u32,
    #[schemars(description = "Maturity profile: \"nascent\" | \"developing\" | \"mature\" (default \"mature\")")]
    #[serde(default)]
    profile: Option<String>,
    #[serde(flatten)]
    source:  CandleSource,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct FibTimeZonesInput {
    #[schemars(description = "Trading pair e.g. \"ENJEUR\"")]
    pair:    String,
    #[schemars(description = "Timeframe: \"1h\", \"4h\", \"1d\", or \"1w\"")]
    tf:      String,
    #[schemars(description = "Candles to scan (default 200, min 50)")]
    #[serde(default = "default_fib_last_n")]
    last_n:  u32,
    #[schemars(description = "Maturity profile: \"nascent\" | \"developing\" (mature not supported — returns error)")]
    #[serde(default)]
    profile: Option<String>,
    #[serde(flatten)]
    source:  CandleSource,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct OptionalPairInput {
    #[schemars(description = "Trading pair (optional — omit for all configured pairs)")]
    #[serde(default)]
    pair: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct OrderbookPressureInput {
    #[schemars(description = "Trading pair e.g. \"ENJEUR\"")]
    pair:   String,
    #[schemars(description = "Number of snapshots to analyse (default 60, max 500)")]
    #[serde(default)]
    last_n: Option<u32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct StakingFlowInput {
    #[schemars(description = "Number of staking events to process (default 500, max 2000)")]
    #[serde(default)]
    last_n:      Option<u32>,
    #[schemars(description = "Aggregation period: \"daily\", \"weekly\", or \"monthly\" (default \"daily\")")]
    #[serde(default)]
    period_type: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct WalletFlowInput {
    #[schemars(description = "Token symbol e.g. \"ENJ\"")]
    token:  String,
    #[schemars(description = "Number of transfers to process (default 500, max 2000)")]
    #[serde(default)]
    last_n: Option<u32>,
}

fn default_last_n()              -> u32 { 60 }
fn default_fib_last_n()          -> u32 { 200 }
fn default_backtest_last_n()     -> u32 { 1000 }
fn default_backtest_window()     -> u32 { 200 }
fn default_backtest_lookahead()  -> u32 { 10 }

// ── Parse helpers ──────────────────────────────────────────────────────────────

fn parse_pair(s: &str) -> Result<Pair, String>      { Pair::parse(s).map_err(|e| e.to_string()) }
fn parse_tf(s: &str)   -> Result<Timeframe, String> { s.parse::<Timeframe>().map_err(|e| e.to_string()) }
fn parse_profile(opt: Option<String>) -> Result<FibProfile, String> {
    FibProfile::parse(opt.as_deref().unwrap_or("mature"))
}

fn period_usize(opt: Option<u32>, default: usize) -> Result<usize, String> {
    let n = opt.map_or(default, |v| v as usize);
    if n == 0 { Err("period must be > 0".to_string()) } else { Ok(n) }
}

// ── Tool implementations ───────────────────────────────────────────────────────

#[tool_router(router = tool_router)]
impl FlowFunctionServer {

    // ── RSI ────────────────────────────────────────────────────────────────────

    #[tool(
        name = "rsi",
        description = "Wilder's RSI computed from PCTS OHLCV. \
                       Seed lookback = period candles (fetched automatically). \
                       Returns [{ts, rsi}] ascending. rsi ∈ [0, 100]. \
                       Default period=14. candle_source: \"ohlcv\"(default) | \"ha\". QUERY — read-only."
    )]
    async fn rsi(&self, Parameters(req): Parameters<OhlcvIndicatorInput>) -> Result<String, String> {
        let pair   = parse_pair(&req.pair)?;
        let tf     = parse_tf(&req.tf)?;
        let period = period_usize(req.period, RSI_DEFAULT)?;
        let seed   = (period + 1) as u32;
        let raw    = self.fetch_ohlcv(&pair, tf, req.last_n, seed).await?;
        let raw    = apply_candle_source(raw, &req.source.candle_source)?;
        let pts    = compute_rsi(&raw, period);
        let out: Vec<_> = pts.into_iter().rev().take(req.last_n as usize).rev().collect();
        serde_json::to_string(&out).map_err(|e| e.to_string())
    }

    // ── MA Cross ───────────────────────────────────────────────────────────────

    #[tool(
        name = "ma_cross",
        description = "Moving average crossover (SMA or EMA). Fast/slow MA pair with cross detection. \
                       Returns [{ts, fast_ma, slow_ma, cross?}] ascending. \
                       cross: \"bullish\" | \"bearish\" (omitted when no cross). \
                       Default fast=9, slow=21, ma_type=sma. candle_source: \"ohlcv\"(default) | \"ha\". QUERY — read-only."
    )]
    async fn ma_cross(&self, Parameters(req): Parameters<MaCrossInput>) -> Result<String, String> {
        let pair    = parse_pair(&req.pair)?;
        let tf      = parse_tf(&req.tf)?;
        let fast    = period_usize(req.fast, DEFAULT_FAST)?;
        let slow    = period_usize(req.slow, DEFAULT_SLOW)?;
        let ma_type = req.ma_type.as_deref()
            .map(|s| s.parse::<MaType>().map_err(|e| e.to_string()))
            .transpose()?
            .unwrap_or(MaType::Sma);
        let seed = slow as u32;
        let raw  = self.fetch_ohlcv(&pair, tf, req.last_n, seed).await?;
        let raw  = apply_candle_source(raw, &req.source.candle_source)?;
        let pts  = compute_ma_cross(&raw, fast, slow, ma_type);
        let out: Vec<_> = pts.into_iter().rev().take(req.last_n as usize).rev().collect();
        serde_json::to_string(&out).map_err(|e| e.to_string())
    }

    // ── ATR ────────────────────────────────────────────────────────────────────

    #[tool(
        name = "atr",
        description = "Average True Range (Wilder's smoothing). \
                       Returns [{ts, atr}] ascending. \
                       Default period=14. candle_source: \"ohlcv\"(default) | \"ha\". QUERY — read-only."
    )]
    async fn atr(&self, Parameters(req): Parameters<OhlcvIndicatorInput>) -> Result<String, String> {
        let pair   = parse_pair(&req.pair)?;
        let tf     = parse_tf(&req.tf)?;
        let period = period_usize(req.period, ATR_DEFAULT)?;
        let seed   = (period + 1) as u32;
        let raw    = self.fetch_ohlcv(&pair, tf, req.last_n, seed).await?;
        let raw    = apply_candle_source(raw, &req.source.candle_source)?;
        let pts    = compute_atr(&raw, period);
        let out: Vec<_> = pts.into_iter().rev().take(req.last_n as usize).rev().collect();
        serde_json::to_string(&out).map_err(|e| e.to_string())
    }

    // ── Bollinger Bands ────────────────────────────────────────────────────────

    #[tool(
        name = "bollinger",
        description = "Bollinger Bands (SMA ± n_std × σ). \
                       Returns [{ts, middle, upper, lower, width, pct_b}] ascending. \
                       pct_b = (close−lower)/(upper−lower); can exceed [0,1]. \
                       Default period=20, n_std=2.0. candle_source: \"ohlcv\"(default) | \"ha\". QUERY — read-only."
    )]
    async fn bollinger(&self, Parameters(req): Parameters<BollingerInput>) -> Result<String, String> {
        let pair   = parse_pair(&req.pair)?;
        let tf     = parse_tf(&req.tf)?;
        let period = period_usize(req.period, BOLL_DEFAULT)?;
        let n_std  = req.n_std.unwrap_or(DEFAULT_N_STD);
        let seed   = period as u32;
        let raw    = self.fetch_ohlcv(&pair, tf, req.last_n, seed).await?;
        let raw    = apply_candle_source(raw, &req.source.candle_source)?;
        let pts    = compute_bollinger(&raw, period, n_std);
        let out: Vec<_> = pts.into_iter().rev().take(req.last_n as usize).rev().collect();
        serde_json::to_string(&out).map_err(|e| e.to_string())
    }

    // ── Donchian Channels ──────────────────────────────────────────────────────

    #[tool(
        name = "donchian",
        description = "Donchian Channels — rolling highest high / lowest low. \
                       Returns [{ts, upper, mid, lower, width}] ascending. \
                       Default period=20. candle_source: \"ohlcv\"(default) | \"ha\". QUERY — read-only."
    )]
    async fn donchian(&self, Parameters(req): Parameters<OhlcvIndicatorInput>) -> Result<String, String> {
        let pair   = parse_pair(&req.pair)?;
        let tf     = parse_tf(&req.tf)?;
        let period = period_usize(req.period, DONCH_DEFAULT)?;
        let seed   = period as u32;
        let raw    = self.fetch_ohlcv(&pair, tf, req.last_n, seed).await?;
        let raw    = apply_candle_source(raw, &req.source.candle_source)?;
        let pts    = compute_donchian(&raw, period);
        let out: Vec<_> = pts.into_iter().rev().take(req.last_n as usize).rev().collect();
        serde_json::to_string(&out).map_err(|e| e.to_string())
    }

    // ── Historical Volatility ──────────────────────────────────────────────────

    #[tool(
        name = "volatility",
        description = "Historical Volatility (HV) — annualised close-to-close log-return std dev. \
                       Returns [{ts, hv}] ascending. hv is in percent (e.g. 42.3 = 42.3% annualised). \
                       Annualisation factor derived automatically from the timeframe. \
                       Default period=20 (sample std dev, n-1 denominator). \
                       candle_source: \"ohlcv\"(default) | \"ha\". QUERY — read-only."
    )]
    async fn volatility(&self, Parameters(req): Parameters<OhlcvIndicatorInput>) -> Result<String, String> {
        let pair   = parse_pair(&req.pair)?;
        let tf     = parse_tf(&req.tf)?;
        let period = period_usize(req.period, HV_DEFAULT)?;
        let seed   = (period + 1) as u32;
        let raw    = self.fetch_ohlcv(&pair, tf.clone(), req.last_n, seed).await?;
        let raw    = apply_candle_source(raw, &req.source.candle_source)?;
        let pts    = compute_hv(&raw, period, &tf);
        let out: Vec<_> = pts.into_iter().rev().take(req.last_n as usize).rev().collect();
        serde_json::to_string(&out).map_err(|e| e.to_string())
    }

    // ── FVG ────────────────────────────────────────────────────────────────────

    #[tool(
        name = "fvg",
        description = "Fair Value Gap (imbalance) detection from PCTS OHLCV. \
                       3-candle pattern: bullish FVG when C.low > A.high, \
                       bearish FVG when C.high < A.low. \
                       Returns [{ts, direction, top, bottom, filled}] sorted by ts. \
                       filled: price has since traded back into the gap zone. \
                       candle_source: \"ohlcv\"(default) | \"ha\". QUERY — read-only."
    )]
    async fn fvg(&self, Parameters(req): Parameters<SmcSourceInput>) -> Result<String, String> {
        let pair  = parse_pair(&req.pair)?;
        let tf    = parse_tf(&req.tf)?;
        let raw   = self.fetch_ohlcv(&pair, tf, req.last_n, 50).await?;
        let raw   = apply_candle_source(raw, &req.source.candle_source)?;
        let zones = compute_fvg(&raw);
        serde_json::to_string(&zones).map_err(|e| e.to_string())
    }

    // ── Order Blocks ───────────────────────────────────────────────────────────

    #[tool(
        name = "order_blocks",
        description = "Order Block detection from PCTS OHLCV. \
                       Bullish OB: last bearish candle before a strong bullish impulse (close > OB high). \
                       Bearish OB: last bullish candle before a strong bearish impulse (close < OB low). \
                       Returns [{ts, direction, top, bottom, broken}] sorted by ts. \
                       broken: price has since closed beyond the opposite side of the OB. \
                       candle_source: \"ohlcv\"(default) | \"ha\". QUERY — read-only."
    )]
    async fn order_blocks(&self, Parameters(req): Parameters<SmcSourceInput>) -> Result<String, String> {
        let pair   = parse_pair(&req.pair)?;
        let tf     = parse_tf(&req.tf)?;
        let raw    = self.fetch_ohlcv(&pair, tf, req.last_n, 50).await?;
        let raw    = apply_candle_source(raw, &req.source.candle_source)?;
        let blocks = compute_order_blocks(&raw);
        serde_json::to_string(&blocks).map_err(|e| e.to_string())
    }

    // ── Structure ──────────────────────────────────────────────────────────────

    #[tool(
        name = "structure",
        description = "Market structure events (BOS / CHoCH) from PCTS OHLCV. \
                       BOS (Break of Structure): close breaks last swing high/low in direction of prior trend. \
                       CHoCH (Change of Character): close breaks last swing high/low AGAINST prior direction. \
                       Returns [{ts, event_type, level, direction}] sorted by ts. \
                       event_type: \"bos\" | \"choch\". direction: \"bullish\" | \"bearish\". \
                       candle_source: \"ohlcv\"(default) | \"ha\". QUERY — read-only."
    )]
    async fn structure(&self, Parameters(req): Parameters<SmcSourceInput>) -> Result<String, String> {
        let pair   = parse_pair(&req.pair)?;
        let tf     = parse_tf(&req.tf)?;
        let raw    = self.fetch_ohlcv(&pair, tf, req.last_n, 50).await?;
        let raw    = apply_candle_source(raw, &req.source.candle_source)?;
        let events = compute_structure(&raw);
        serde_json::to_string(&events).map_err(|e| e.to_string())
    }

    // ── Liquidity ──────────────────────────────────────────────────────────────

    #[tool(
        name = "liquidity",
        description = "Liquidity level detection from PCTS OHLCV. \
                       buy_side: equal highs within 0.1% — stop clusters above. \
                       sell_side: equal lows within 0.1% — stop clusters below. \
                       Returns [{ts, price, side, swept}] sorted by ts. \
                       swept: price has since traded through the level. \
                       candle_source: \"ohlcv\"(default) | \"ha\". QUERY — read-only."
    )]
    async fn liquidity(&self, Parameters(req): Parameters<SmcSourceInput>) -> Result<String, String> {
        let pair   = parse_pair(&req.pair)?;
        let tf     = parse_tf(&req.tf)?;
        let raw    = self.fetch_ohlcv(&pair, tf, req.last_n, 50).await?;
        let raw    = apply_candle_source(raw, &req.source.candle_source)?;
        let levels = compute_liquidity(&raw);
        serde_json::to_string(&levels).map_err(|e| e.to_string())
    }

    // ── Fibonacci Confluence ───────────────────────────────────────────────────

    #[tool(
        name = "fib_confluence",
        description = "Multi-anchor Fibonacci Confluence — scores fib levels across 5 independent reference frames (Story #37). \
                       P1: current structure range (last opposing BOS/CHoCH from market structure detection). \
                       P2: swing pivot range (last swing high + last swing low). \
                       P3: session range (highest high / lowest low of the candle window). \
                       P4: previous day high/low (auto-fetched at 1d). \
                       P5: previous week high/low (auto-fetched at 1w). \
                       score = count of anchors whose fib level falls within ATR tolerance. Max = 5. \
                       Returns {pair, tf, zones, p1_source, computed_at}. \
                       zones: [{ratio, direction, level, zone_low, zone_high, score, anchors}] nearest-first. \
                       direction: \"up\" (retracement from low) | \"down\" (retracement from high). \
                       p1_source: \"structure\" | \"fallback_Nw\" (N weeks used when no BOS/CHoCH found). \
                       profile controls ATR tolerance multiplier: \"mature\"=0.20×ATR | \"developing\"=0.25×ATR | \"nascent\"=0.35×ATR. \
                       min_score: minimum anchor count to include zone (default 2). \
                       candle_source: \"ohlcv\"(default) | \"ha\". Default last_n=200. QUERY — read-only."
    )]
    async fn fib_confluence(&self, Parameters(req): Parameters<FibConfluenceInput>) -> Result<String, String> {
        let pair     = parse_pair(&req.pair)?;
        let tf       = parse_tf(&req.tf)?;
        let tf_str   = tf.label().to_string();
        let profile  = parse_profile(req.profile)?;
        let n        = req.last_n.max(50);
        let min_sc   = req.min_score.unwrap_or(2).clamp(1, 5);

        let raw = self.fetch_ohlcv(&pair, tf, n, 50).await?;
        let raw = apply_candle_source(raw, &req.source.candle_source)?;

        let (pdh, pdl) = self.fetch_prev_level(&pair, "1d").await;
        let (pwh, pwl) = self.fetch_prev_level(&pair, "1w").await;

        let result = compute_multi_anchor_fib(
            &raw, pdh, pdl, pwh, pwl,
            &profile, min_sc, 6, &tf_str, &req.pair,
        );
        serde_json::to_string(&result).map_err(|e| e.to_string())
    }

    // ── Fibonacci Confluence Backtest (ADR-017, Story #39) ─────────────────────

    #[tool(
        name = "fib_confluence_backtest",
        description = "Indicator-level walk-forward backtest of multi-anchor Fibonacci confluence (ADR-017, Stories #39 + #43). \
                       Three validation tracks in a single walk-forward pass: \
                       LEGACY (v1 audit): naive respect = price did not close beyond zone. Regime-sensitive; kept for regression. \
                       TRACK A — author-faithful reaction: does high score correlate with measurable post-touch reaction \
                       (ATR spike, volume spike, wick prominence)? Gate: monotonic_reaction. \
                       TRACK B — contextual respect: respect_rate per (score × arrival_direction × trend_regime) bucket. \
                       arrival: \"from_above\" | \"from_below\" (where price was at observation time relative to zone). \
                       trend: \"bullish\" | \"bearish\" | \"neutral\" (last BOS/CHoCH direction at t). \
                       Gate: any_calibrated_bucket (does any quadrant show monotonic respect with n ≥ 30). \
                       Returns {pair, tf, total_zones, legacy_respect, track_a_reaction, track_b_contextual, \
                       legacy_monotonic_respect, monotonic_reaction, any_calibrated_bucket, ...}. \
                       Statistical floor: n ≥ 30 per bucket for monotonicity checks. \
                       P4/P5 approximation: previous-day/previous-week H/L derived from chart candles per TF \
                       (1h→24/168 bars, 4h→6/42, 1d→1/7). Default lookahead tightened to 10 bars. QUERY — read-only."
    )]
    async fn fib_confluence_backtest(&self, Parameters(req): Parameters<FibConfluenceBacktestInput>) -> Result<String, String> {
        let pair        = parse_pair(&req.pair)?;
        let tf          = parse_tf(&req.tf)?;
        let tf_str      = tf.label().to_string();
        let profile     = parse_profile(req.profile)?;
        let min_sc      = req.min_score.unwrap_or(2).clamp(1, 5);
        let window_size = req.window_size.max(50) as usize;
        let lookahead   = req.lookahead_bars.max(1) as usize;
        let min_last_n  = (window_size + lookahead + 50) as u32;
        let last_n      = req.last_n.max(min_last_n);

        let raw = self.fetch_ohlcv(&pair, tf, last_n, 0).await?;
        let raw = apply_candle_source(raw, &req.source.candle_source)?;

        let result = backtest_multi_anchor_fib(
            &raw, &profile, min_sc, window_size, lookahead, 6, &tf_str, &req.pair,
        );
        serde_json::to_string(&result).map_err(|e| e.to_string())
    }

    // ── Order Blocks Backtest (ADR-017, Story #40) ─────────────────────────────

    #[tool(
        name = "order_blocks_backtest",
        description = "Indicator-level retest-respect backtest of Order Blocks (ADR-017, Story #40). \
                       Tests the claim: \"when price returns to a bullish OB within lookahead_bars, \
                       it holds as support (no close below bottom); bearish OB holds as resistance \
                       (no close above top)\". \
                       Returns {pair, tf, total_blocks, buckets, bullish_better_than_random, \
                       bearish_better_than_random, ...}. \
                       buckets: [{direction, n_blocks, n_returned, n_respected, return_rate, \
                       respect_rate, avg_bars_to_return}] per direction (bullish/bearish). \
                       return_rate depends on market activity and is not a gate; the calibration \
                       signal is respect_rate. \
                       Gates: *_better_than_random = (n_returned ≥ 30 AND respect_rate ≥ 0.55). \
                       Causal-safe: OB detection confirms at i+1 from candles[i] and candles[i+1]; \
                       the existing `broken` field uses full-history look-ahead and is ignored here \
                       — the backtest re-validates with a bounded future window starting at i+2. \
                       QUERY — read-only."
    )]
    async fn order_blocks_backtest(&self, Parameters(req): Parameters<OrderBlocksBacktestInput>) -> Result<String, String> {
        let pair       = parse_pair(&req.pair)?;
        let tf         = parse_tf(&req.tf)?;
        let tf_str     = tf.label().to_string();
        let lookahead  = req.lookahead_bars.max(1) as usize;
        let min_last_n = (lookahead + 100) as u32;
        let last_n     = req.last_n.max(min_last_n);

        let raw = self.fetch_ohlcv(&pair, tf, last_n, 0).await?;
        let raw = apply_candle_source(raw, &req.source.candle_source)?;

        let result = backtest_order_blocks(&raw, lookahead, &tf_str, &req.pair);
        serde_json::to_string(&result).map_err(|e| e.to_string())
    }

    // ── Order Flow Backtest (ADR-017, Story #40) ───────────────────────────────

    #[tool(
        name = "order_flow_backtest",
        description = "Indicator-level forward-return backtest of order-flow net_aggression (ADR-017, Story #40). \
                       Tests the claim: \"net_aggression = (MB-MS)/(MB+MS) at candle t predicts direction of \
                       price movement over the next lookahead_bars candles\". \
                       Buckets net_aggression into 5 fixed ranges: strong_bearish (≤-0.3), mild_bearish (-0.3..-0.1], \
                       neutral (-0.1..0.1], mild_bullish (0.1..0.3], strong_bullish (>0.3). \
                       Returns {pair, tf, total_observations, buckets, monotonic_forward_return, ...}. \
                       buckets: [{bucket, n_events, avg_forward_return_pct, median_forward_return_pct, \
                       positive_return_rate}] one per range. \
                       Gate: monotonic_forward_return = true iff avg_forward_return_pct is monotonically \
                       non-decreasing from strong_bearish → strong_bullish AND every populated bucket has n ≥ 30. \
                       Candles without trade-flow data (mb_vol/ms_vol None) are skipped. \
                       Causal-safe: classification uses only candle[t]; future window used only for return. \
                       QUERY — read-only."
    )]
    async fn order_flow_backtest(&self, Parameters(req): Parameters<OrderFlowBacktestInput>) -> Result<String, String> {
        let pair       = parse_pair(&req.pair)?;
        let tf         = parse_tf(&req.tf)?;
        let tf_str     = tf.label().to_string();
        let lookahead  = req.lookahead_bars.max(1) as usize;
        let min_last_n = (lookahead + 100) as u32;
        let last_n     = req.last_n.max(min_last_n);

        let raw = self.fetch_ohlcv(&pair, tf, last_n, 0).await?;

        let result = backtest_order_flow(&raw, lookahead, &tf_str, &req.pair);
        serde_json::to_string(&result).map_err(|e| e.to_string())
    }

    // ── Structure Backtest (ADR-017, Story #40) ────────────────────────────────

    #[tool(
        name = "structure_backtest",
        description = "Indicator-level walk-forward backtest of market structure events (ADR-017, Story #40). \
                       Tests the claim: \"a BOS signals trend continuation; within lookahead_bars price makes a \
                       higher high (bullish) or lower low (bearish) beyond the break level by ≥ follow_threshold\". \
                       CHoCH events tested with the same directional expectation (reversal leg). \
                       Returns {pair, tf, total_events, buckets, *_better_than_random, ...}. \
                       buckets: [{event_type, direction, n_events, n_followed, follow_rate, avg_bars_to_follow, \
                       avg_follow_magnitude_pct}] per (event_type × direction) — 4 combinations total. \
                       Gates: bos_bullish / bos_bearish / choch_bullish / choch_bearish _better_than_random = \
                       (n_events ≥ 30 AND follow_rate ≥ 0.55). \
                       Causal-safe: compute_structure emits events at their natural timestamp; full-history \
                       computation is equivalent to per-candle walk-forward for the purposes of event detection. \
                       Events with insufficient history (idx < window_size) or future (idx + lookahead ≥ len) \
                       are skipped. QUERY — read-only."
    )]
    async fn structure_backtest(&self, Parameters(req): Parameters<StructureBacktestInput>) -> Result<String, String> {
        let pair         = parse_pair(&req.pair)?;
        let tf           = parse_tf(&req.tf)?;
        let tf_str       = tf.label().to_string();
        let window_size  = req.window_size.max(50) as usize;
        let lookahead    = req.lookahead_bars.max(1) as usize;
        let follow_thr   = req.follow_threshold.unwrap_or(0.005).max(0.0001);
        let min_last_n   = (window_size + lookahead + 50) as u32;
        let last_n       = req.last_n.max(min_last_n);

        let raw = self.fetch_ohlcv(&pair, tf, last_n, 0).await?;
        let raw = apply_candle_source(raw, &req.source.candle_source)?;

        let result = backtest_structure(
            &raw, window_size, lookahead, follow_thr, &tf_str, &req.pair,
        );
        serde_json::to_string(&result).map_err(|e| e.to_string())
    }

    // ── Fibonacci Targets ──────────────────────────────────────────────────────

    #[tool(
        name = "fib_targets",
        description = "DiNapoli take-profit targets from Fibonacci Confluence for a given entry price. \
                       Returns resistance clusters above current price as actionable TP levels \
                       plus the nearest support cluster below (stop-loss reference). \
                       Output: {current_price, entry_price, pnl_pct, targets, nearest_support, profile, exploratory}. \
                       targets: [{price, strength, distance_from_current_pct, distance_from_entry_pct}] ascending. \
                       distance_from_entry_pct negative when target is below entry (underwater position). \
                       nearest_support: strongest support cluster within 20% below current price. \
                       profile: \"nascent\" | \"developing\" | \"mature\" (default). \
                       candle_source: \"ohlcv\"(default) | \"ha\". \
                       exploratory=true when profile=nascent — lower signal confidence. \
                       Default last_n=200. QUERY — read-only."
    )]
    async fn fib_targets(&self, Parameters(req): Parameters<FibTargetsInput>) -> Result<String, String> {
        let pair    = parse_pair(&req.pair)?;
        let tf      = parse_tf(&req.tf)?;
        let profile = parse_profile(req.profile)?;
        let n       = req.last_n.max(50);
        let raw     = self.fetch_ohlcv(&pair, tf, n, 50).await?;
        let raw     = apply_candle_source(raw, &req.source.candle_source)?;
        let result  = compute_fib_targets(&raw, req.entry_price, &profile)?;
        serde_json::to_string(&result).map_err(|e| e.to_string())
    }

    // ── Harmonic Patterns ──────────────────────────────────────────────────────

    #[tool(
        name = "harmonic_patterns",
        description = "XABCD harmonic pattern detection from PCTS OHLCV (ADR-001). \
                       Detects completed Gartley, Bat, Butterfly, Crab patterns. \
                       Ratios per Carney/Pesavento methodology. \
                       Bullish: X(Low) A(High) B(Low) C(High) D(Low) — buy signal at D. \
                       Bearish: X(High) A(Low) B(High) C(Low) D(High) — sell signal at D. \
                       D completion ratios: Gartley=0.786, Bat=0.886, Butterfly=1.272, Crab=1.618. \
                       Returns [{ts_x, ts_a, ts_b, ts_c, ts_d, pattern, direction, d_price, xabcd_quality, exploratory}]. \
                       xabcd_quality 0.0–1.0: closeness to ideal ratios (D×60%, AB×40%). \
                       profile: \"nascent\" (Gartley+Bat, tol=7%) | \"developing\" (Gartley+Bat+Butterfly, tol=5%) \
                       | \"mature\" (all 4 patterns, tol=3%, default). \
                       candle_source: \"ohlcv\"(default) | \"ha\". \
                       Default last_n=200. QUERY — read-only."
    )]
    async fn harmonic_patterns(&self, Parameters(req): Parameters<HarmonicPatternsInput>) -> Result<String, String> {
        let pair    = parse_pair(&req.pair)?;
        let tf      = parse_tf(&req.tf)?;
        let profile = parse_profile(req.profile)?;
        let n       = req.last_n.max(50);
        let raw     = self.fetch_ohlcv(&pair, tf, n, 50).await?;
        let raw     = apply_candle_source(raw, &req.source.candle_source)?;
        let found   = compute_harmonic_patterns(&raw, &profile);
        serde_json::to_string(&found).map_err(|e| e.to_string())
    }

    // ── Fibonacci Time Zones ───────────────────────────────────────────────────

    #[tool(
        name = "fib_time_zones",
        description = "Fibonacci Time Zones — temporal projection from the highest-volatility impulse candle (ADR-002). \
                       Anchor: candle with highest (high−low)/ATR₁₄ ratio in the window. \
                       Projects Fibonacci sequence [1,1,2,3,5,8,13,21,34,55] bars forward from anchor. \
                       Returns {anchor_ts, anchor_ratio, profile, exploratory, zones}. \
                       zones: [{fib_n, ts, in_window}] — ts=null and in_window=false for future bars. \
                       Profile gate: \"mature\" not supported — returns error. \
                       \"developing\" (max_bars=55) | \"nascent\" (max_bars=34, exploratory=true). \
                       candle_source: \"ohlcv\"(default) | \"ha\". \
                       Default last_n=200. QUERY — read-only."
    )]
    async fn fib_time_zones(&self, Parameters(req): Parameters<FibTimeZonesInput>) -> Result<String, String> {
        let pair    = parse_pair(&req.pair)?;
        let tf      = parse_tf(&req.tf)?;
        let profile = parse_profile(req.profile)?;
        let n       = req.last_n.max(50);
        let raw     = self.fetch_ohlcv(&pair, tf, n, 50).await?;
        let raw     = apply_candle_source(raw, &req.source.candle_source)?;
        let result  = compute_fib_time_zones(&raw, &profile)?;
        serde_json::to_string(&result).map_err(|e| e.to_string())
    }

    // ── HA Pattern ─────────────────────────────────────────────────────────────

    #[tool(
        name = "ha_pattern",
        description = "Heikin Ashi pattern analysis from PCTS OHLCV. \
                       Seed lookback of 10 candles applied automatically. \
                       Returns [{ts, color, has_lower_wick, has_upper_wick, \
                       consecutive_count, reversal, lower_wick_signal}] ascending. \
                       color: blue|green|red|gray. \
                       consecutive_count: how many consecutive same-color candles ending here. \
                       reversal: true when color changed from previous candle. \
                       lower_wick_signal: bullish candle with lower wick (continuation signal). \
                       QUERY — read-only."
    )]
    async fn ha_pattern(&self, Parameters(req): Parameters<SmcInput>) -> Result<String, String> {
        let pair = parse_pair(&req.pair)?;
        let tf   = parse_tf(&req.tf)?;
        let seed = SEED_LOOKBACK as u32;
        let raw  = self.fetch_ohlcv(&pair, tf, req.last_n, seed).await?;
        let pts  = compute_ha_patterns(&raw, req.last_n as usize);
        serde_json::to_string(&pts).map_err(|e| e.to_string())
    }

    // ── Order Flow ─────────────────────────────────────────────────────────────

    #[tool(
        name = "order_flow",
        description = "Order flow ratios computed from PCTS trade-flow columns (MB/MS/LB/LS). \
                       Returns [{ts, mb_ms_ratio?, lb_ls_ratio?, net_aggression?, \
                       market_pct?, avg_mb_size?, avg_ms_size?}] ascending. \
                       All fields are optional — None when trade-flow data is absent or zero. \
                       net_aggression ∈ [-1, +1]: positive = buyer aggression dominates. \
                       market_pct: % of volume from market orders. \
                       QUERY — read-only."
    )]
    async fn order_flow(&self, Parameters(req): Parameters<SmcInput>) -> Result<String, String> {
        let pair = parse_pair(&req.pair)?;
        let tf   = parse_tf(&req.tf)?;
        let raw  = self.fetch_ohlcv(&pair, tf, req.last_n, 0).await?;
        let pts  = compute_order_flow(&raw);
        let out: Vec<_> = pts.into_iter().rev().take(req.last_n as usize).rev().collect();
        serde_json::to_string(&out).map_err(|e| e.to_string())
    }

    // ── Governance Signal ──────────────────────────────────────────────────────

    #[tool(
        name = "governance_signal",
        description = "Governance state enriched with computed signal strength. \
                       Data: asset_governance_state + asset_governance_config + asset_ath (OMV SQLite). \
                       Returns [{pair, state, ha_color, depression_pct, entry_levels, \
                       ready_for_entry, signal_strength}]. \
                       signal_strength 0.0–1.0: +0.4 entry_ready, +0.3 blue HA, +0.3 depression ≤ -90%. \
                       Omit pair for all configured pairs. QUERY — read-only."
    )]
    async fn governance_signal(&self, Parameters(req): Parameters<OptionalPairInput>) -> Result<String, String> {
        let pair    = req.pair.as_deref().map(parse_pair).transpose()?;
        let snaps   = self.adapter.governance(pair.as_ref()).await.map_err(|e| e.to_string())?;
        let signals: Vec<_> = snaps.iter().map(compute_governance_signal).collect();
        serde_json::to_string(&signals).map_err(|e| e.to_string())
    }

    // ── Orderbook Pressure ─────────────────────────────────────────────────────

    #[tool(
        name = "orderbook_pressure",
        description = "Order book directional pressure from OMV SQLite (Kraken WS, 1 snapshot/min). \
                       Returns [{ts, bid_ask_ratio_10?, bid_ask_ratio_25?, bid_ask_ratio_50?, \
                       dominant_side, spread_bps}] ascending. \
                       dominant_side: \"bid\" when ratio_25>1.1, \"ask\" when <0.9, else \"neutral\". \
                       Default last_n=60. QUERY — read-only."
    )]
    async fn orderbook_pressure(&self, Parameters(req): Parameters<OrderbookPressureInput>) -> Result<String, String> {
        let pair  = parse_pair(&req.pair)?;
        let n     = req.last_n.map(|n| n.min(500));
        let snaps = self.adapter.orderbook(&pair, n).await.map_err(|e| e.to_string())?;
        let pts   = compute_orderbook_pressure(&snaps);
        serde_json::to_string(&pts).map_err(|e| e.to_string())
    }

    // ── Staking Flow ───────────────────────────────────────────────────────────

    #[tool(
        name = "staking_flow",
        description = "Cosmos Hub staking flow aggregated by period from OMV SQLite. \
                       Delegate/undelegate amounts summed per period; redelegate counted but excluded from net. \
                       Returns [{period, delegated_atom, undelegated_atom, net_atom, \
                       flow_direction, event_count}] sorted by period. \
                       flow_direction: \"inflow\" | \"outflow\" | \"neutral\". \
                       period_type: \"daily\" (default) | \"weekly\" | \"monthly\". \
                       QUERY — read-only."
    )]
    async fn staking_flow(&self, Parameters(req): Parameters<StakingFlowInput>) -> Result<String, String> {
        let n           = req.last_n.map(|n| n.min(2000)).or(Some(500));
        let period_type = req.period_type.as_deref()
            .map(|s| s.parse::<PeriodType>().map_err(|e| e.to_string()))
            .transpose()?
            .unwrap_or(PeriodType::Daily);
        let events = self.adapter.cosmos_stake_events(n, None).await.map_err(|e| e.to_string())?;
        let flow   = compute_staking_flow(&events, period_type);
        serde_json::to_string(&flow).map_err(|e| e.to_string())
    }

    // ── Wallet Flow ────────────────────────────────────────────────────────────

    #[tool(
        name = "wallet_flow",
        description = "Exchange inflow/outflow for an ERC-20 token from OMV SQLite. \
                       Requires token symbol e.g. \"ENJ\". Default last_n=500. \
                       exchange_inflow: tokens moving TO exchanges (bearish). \
                       exchange_outflow: tokens moving FROM exchanges (bullish). \
                       net_flow = outflow − inflow (positive = bullish). \
                       Returns [{period, exchange_inflow, exchange_outflow, net_flow, \
                       flow_direction, transfer_count}] sorted by period (daily). \
                       QUERY — read-only."
    )]
    async fn wallet_flow(&self, Parameters(req): Parameters<WalletFlowInput>) -> Result<String, String> {
        let n         = req.last_n.map(|n| n.min(2000)).or(Some(500));
        let transfers = self.adapter.transfers(&req.token, n).await.map_err(|e| e.to_string())?;
        let wallets   = self.adapter.wallets(None).await.map_err(|e| e.to_string())?;
        let flow      = compute_wallet_flow(&transfers, &wallets);
        serde_json::to_string(&flow).map_err(|e| e.to_string())
    }
}

// ── OHLCV fetch helpers ───────────────────────────────────────────────────────

impl FlowFunctionServer {
    async fn fetch_ohlcv(
        &self,
        pair:   &Pair,
        tf:     Timeframe,
        last_n: u32,
        seed:   u32,
    ) -> Result<Vec<crate::domain::candle::OhlcvCandle>, String> {
        let window = Window::LastN(last_n.saturating_add(seed));
        self.adapter.ohlcv(pair, tf, &window).await.map_err(|e| e.to_string())
    }

    /// Fetch previous completed period high/low for a given timeframe.
    /// Fetches 3 candles and returns (high, low) of the second-to-last (index len-2).
    /// Returns (None, None) on error or insufficient data.
    async fn fetch_prev_level(&self, pair: &Pair, tf_str: &str) -> (Option<f64>, Option<f64>) {
        let tf = match tf_str.parse::<Timeframe>() {
            Ok(t) => t,
            Err(_) => return (None, None),
        };
        match self.fetch_ohlcv(pair, tf, 3, 0).await {
            Ok(candles) if candles.len() >= 2 => {
                let prev = &candles[candles.len() - 2];
                (Some(prev.high), Some(prev.low))
            }
            _ => (None, None),
        }
    }
}

// ── ServerHandler ──────────────────────────────────────────────────────────────

#[tool_handler(router = self.tool_router)]
impl ServerHandler for FlowFunctionServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(
            ServerCapabilities::builder().enable_tools().build(),
        )
        .with_server_info(Implementation::new("flow-function-mcp", env!("CARGO_PKG_VERSION")))
        .with_instructions(
            "Function Layer — Stateless signal computation (TGF Layer 3). All tools are QUERY (read-only).\n\
             candle_source: \"ohlcv\"(default, raw PCTS) | \"ha\"(Heikin Ashi smoothed) — available on all OHLCV and SMC tools.\n\
             OHLCV Indicators (from PCTS SQL Server):\n\
               rsi {pair,tf,last_n,period?,candle_source?} | ma_cross {pair,tf,last_n,fast?,slow?,ma_type?,candle_source?} | \
               atr {pair,tf,last_n,period?,candle_source?} | bollinger {pair,tf,last_n,period?,n_std?,candle_source?} | \
               donchian {pair,tf,last_n,period?,candle_source?} | volatility {pair,tf,last_n,period?,candle_source?}\n\
             SMC (from PCTS SQL Server):\n\
               fvg {pair,tf,last_n,candle_source?} | order_blocks {pair,tf,last_n,candle_source?} | \
               structure {pair,tf,last_n,candle_source?} | liquidity {pair,tf,last_n,candle_source?} | \
               fib_confluence {pair,tf,last_n?,profile?,min_score?,candle_source?} | \
               fib_confluence_backtest {pair,tf,last_n?,window_size?,lookahead_bars?,profile?,min_score?,candle_source?} | \
               structure_backtest {pair,tf,last_n?,window_size?,lookahead_bars?,follow_threshold?,candle_source?} | \
               order_flow_backtest {pair,tf,last_n?,lookahead_bars?} | \
               order_blocks_backtest {pair,tf,last_n?,lookahead_bars?,candle_source?} | \
               fib_targets {pair,tf,last_n?,entry_price,profile?,candle_source?} | \
               harmonic_patterns {pair,tf,last_n?,profile?,candle_source?} | fib_time_zones {pair,tf,last_n?,profile?,candle_source?}\n\
             Price Action + Flow (from PCTS SQL Server):\n\
               ha_pattern {pair,tf,last_n} | order_flow {pair,tf,last_n}\n\
             Non-OHLCV (from OMV SQLite):\n\
               governance_signal {pair?} | orderbook_pressure {pair,last_n?} | \
               staking_flow {last_n?,period_type?} | wallet_flow {token,last_n?}\n\
             FibProfile: nascent (0.8% tol, exploratory) | developing (0.5%) | mature (0.3% Boroden, default).\n\
             Pairs: ENJEUR, BTCEUR, ATOMEUR, ETHEUR."
        )
    }
}
