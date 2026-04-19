/// FlowFunctionServer — MCP inbound adapter (Function Layer, TGF Layer 3).
///
/// 18 QUERY tools (read-only):
///   OHLCV indicators : rsi, ma_cross, atr, bollinger, donchian, volatility
///   SMC              : fvg, order_blocks, structure, liquidity, fib_confluence, fib_targets
///   Price action+flow: ha_pattern, order_flow
///   Non-OHLCV        : governance_signal, orderbook_pressure, staking_flow, wallet_flow
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
    adapters::composite::CompositeAdapter,
    domain::{
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
            fib_confluence::compute_fib_confluence,
            fib_profile::FibProfile,
            fib_targets::compute_fib_targets,
            fvg::compute_fvg,
            liquidity::compute_liquidity,
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
}

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

#[derive(Debug, Deserialize, JsonSchema)]
struct FibConfluenceInput {
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

fn default_last_n()     -> u32 { 60 }
fn default_fib_last_n() -> u32 { 200 }

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
                       Default period=14. QUERY — read-only."
    )]
    async fn rsi(&self, Parameters(req): Parameters<OhlcvIndicatorInput>) -> Result<String, String> {
        let pair   = parse_pair(&req.pair)?;
        let tf     = parse_tf(&req.tf)?;
        let period = period_usize(req.period, RSI_DEFAULT)?;
        let seed   = (period + 1) as u32;
        let raw    = self.fetch_ohlcv(&pair, tf, req.last_n, seed).await?;
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
                       Default fast=9, slow=21, ma_type=sma. QUERY — read-only."
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
        let pts  = compute_ma_cross(&raw, fast, slow, ma_type);
        let out: Vec<_> = pts.into_iter().rev().take(req.last_n as usize).rev().collect();
        serde_json::to_string(&out).map_err(|e| e.to_string())
    }

    // ── ATR ────────────────────────────────────────────────────────────────────

    #[tool(
        name = "atr",
        description = "Average True Range (Wilder's smoothing). \
                       Returns [{ts, atr}] ascending. \
                       Default period=14. QUERY — read-only."
    )]
    async fn atr(&self, Parameters(req): Parameters<OhlcvIndicatorInput>) -> Result<String, String> {
        let pair   = parse_pair(&req.pair)?;
        let tf     = parse_tf(&req.tf)?;
        let period = period_usize(req.period, ATR_DEFAULT)?;
        let seed   = (period + 1) as u32;
        let raw    = self.fetch_ohlcv(&pair, tf, req.last_n, seed).await?;
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
                       Default period=20, n_std=2.0. QUERY — read-only."
    )]
    async fn bollinger(&self, Parameters(req): Parameters<BollingerInput>) -> Result<String, String> {
        let pair   = parse_pair(&req.pair)?;
        let tf     = parse_tf(&req.tf)?;
        let period = period_usize(req.period, BOLL_DEFAULT)?;
        let n_std  = req.n_std.unwrap_or(DEFAULT_N_STD);
        let seed   = period as u32;
        let raw    = self.fetch_ohlcv(&pair, tf, req.last_n, seed).await?;
        let pts    = compute_bollinger(&raw, period, n_std);
        let out: Vec<_> = pts.into_iter().rev().take(req.last_n as usize).rev().collect();
        serde_json::to_string(&out).map_err(|e| e.to_string())
    }

    // ── Donchian Channels ──────────────────────────────────────────────────────

    #[tool(
        name = "donchian",
        description = "Donchian Channels — rolling highest high / lowest low. \
                       Returns [{ts, upper, mid, lower, width}] ascending. \
                       Default period=20. QUERY — read-only."
    )]
    async fn donchian(&self, Parameters(req): Parameters<OhlcvIndicatorInput>) -> Result<String, String> {
        let pair   = parse_pair(&req.pair)?;
        let tf     = parse_tf(&req.tf)?;
        let period = period_usize(req.period, DONCH_DEFAULT)?;
        let seed   = period as u32;
        let raw    = self.fetch_ohlcv(&pair, tf, req.last_n, seed).await?;
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
                       Default period=20 (sample std dev, n-1 denominator). QUERY — read-only."
    )]
    async fn volatility(&self, Parameters(req): Parameters<OhlcvIndicatorInput>) -> Result<String, String> {
        let pair   = parse_pair(&req.pair)?;
        let tf     = parse_tf(&req.tf)?;
        let period = period_usize(req.period, HV_DEFAULT)?;
        let seed   = (period + 1) as u32;
        let raw    = self.fetch_ohlcv(&pair, tf.clone(), req.last_n, seed).await?;
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
                       QUERY — read-only."
    )]
    async fn fvg(&self, Parameters(req): Parameters<SmcInput>) -> Result<String, String> {
        let pair  = parse_pair(&req.pair)?;
        let tf    = parse_tf(&req.tf)?;
        let raw   = self.fetch_ohlcv(&pair, tf, req.last_n, 50).await?;
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
                       QUERY — read-only."
    )]
    async fn order_blocks(&self, Parameters(req): Parameters<SmcInput>) -> Result<String, String> {
        let pair   = parse_pair(&req.pair)?;
        let tf     = parse_tf(&req.tf)?;
        let raw    = self.fetch_ohlcv(&pair, tf, req.last_n, 50).await?;
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
                       QUERY — read-only."
    )]
    async fn structure(&self, Parameters(req): Parameters<SmcInput>) -> Result<String, String> {
        let pair   = parse_pair(&req.pair)?;
        let tf     = parse_tf(&req.tf)?;
        let raw    = self.fetch_ohlcv(&pair, tf, req.last_n, 50).await?;
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
                       QUERY — read-only."
    )]
    async fn liquidity(&self, Parameters(req): Parameters<SmcInput>) -> Result<String, String> {
        let pair   = parse_pair(&req.pair)?;
        let tf     = parse_tf(&req.tf)?;
        let raw    = self.fetch_ohlcv(&pair, tf, req.last_n, 50).await?;
        let levels = compute_liquidity(&raw);
        serde_json::to_string(&levels).map_err(|e| e.to_string())
    }

    // ── Fibonacci Confluence ───────────────────────────────────────────────────

    #[tool(
        name = "fib_confluence",
        description = "DiNapoli Fibonacci Confluence zones from PCTS OHLCV. \
                       Detects swing pivots (3-bar high/low), then computes: \
                       Retracements 38.2/50.0/61.8% from each A→B leg (DiNapoli primary set); \
                       Expansions COP=61.8%, OP=100%, XOP=161.8% from ABC patterns (DiNapoli 1998). \
                       Clusters levels within tolerance (profile-controlled). \
                       Returns [{price, strength, direction, levels, atr_compressed, distance_pct}] nearest-first. \
                       direction: \"support\" (below close) | \"resistance\" (above close). \
                       profile: \"nascent\" (0.8% tol, min 2) | \"developing\" (0.5%, min 2) | \"mature\" (0.3%, min 3, default). \
                       Default last_n=200 candles. QUERY — read-only."
    )]
    async fn fib_confluence(&self, Parameters(req): Parameters<FibConfluenceInput>) -> Result<String, String> {
        let pair    = parse_pair(&req.pair)?;
        let tf      = parse_tf(&req.tf)?;
        let profile = parse_profile(req.profile)?;
        let n       = req.last_n.max(50);
        let raw     = self.fetch_ohlcv(&pair, tf, n, 50).await?;
        let zones   = compute_fib_confluence(&raw, &profile);
        serde_json::to_string(&zones).map_err(|e| e.to_string())
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
                       exploratory=true when profile=nascent — lower signal confidence. \
                       Default last_n=200. QUERY — read-only."
    )]
    async fn fib_targets(&self, Parameters(req): Parameters<FibTargetsInput>) -> Result<String, String> {
        let pair    = parse_pair(&req.pair)?;
        let tf      = parse_tf(&req.tf)?;
        let profile = parse_profile(req.profile)?;
        let n       = req.last_n.max(50);
        let raw     = self.fetch_ohlcv(&pair, tf, n, 50).await?;
        let result  = compute_fib_targets(&raw, req.entry_price, &profile)?;
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

// ── OHLCV fetch helper (with seed) ────────────────────────────────────────────

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
             OHLCV Indicators (from PCTS SQL Server):\n\
               rsi {pair,tf,last_n,period?} | ma_cross {pair,tf,last_n,fast?,slow?,ma_type?} | \
               atr {pair,tf,last_n,period?} | bollinger {pair,tf,last_n,period?,n_std?} | \
               donchian {pair,tf,last_n,period?} | volatility {pair,tf,last_n,period?}\n\
             SMC (from PCTS SQL Server):\n\
               fvg {pair,tf,last_n} | order_blocks {pair,tf,last_n} | \
               structure {pair,tf,last_n} | liquidity {pair,tf,last_n} | \
               fib_confluence {pair,tf,last_n?,profile?} | fib_targets {pair,tf,last_n?,entry_price,profile?}\n\
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
