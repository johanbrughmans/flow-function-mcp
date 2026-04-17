/// CompositeAdapter — single struct implementing both MarketDataPort and OnChainPort.
///   MarketDataPort → PctsAdapter (OHLCV)
///   OnChainPort    → SqliteAdapter (governance, orderbook, cosmos, transfers, wallets)
///
/// All rusqlite calls are wrapped in spawn_blocking (rusqlite is sync).

use anyhow::Result;
use async_trait::async_trait;

use crate::{
    adapters::{pcts::PctsAdapter, sqlite::SqliteAdapter},
    domain::{
        candle::OhlcvCandle,
        onchain::{
            governance::GovernanceSnapshot,
            orderbook::OrderBookSnapshot,
            staking::{CosmosStakeEvent, StakeMsgType},
            wallet::{TransferEvent, WalletClassification},
        },
        pair::Pair,
        timeframe::Timeframe,
        window::Window,
    },
    ports::{market_data::MarketDataPort, onchain::OnChainPort},
};

#[derive(Clone)]
pub struct CompositeAdapter {
    pcts:   PctsAdapter,
    sqlite: SqliteAdapter,
}

impl CompositeAdapter {
    pub fn new(pcts: PctsAdapter, sqlite: SqliteAdapter) -> Self { Self { pcts, sqlite } }
}

#[async_trait]
impl MarketDataPort for CompositeAdapter {
    async fn ohlcv(&self, pair: &Pair, tf: Timeframe, window: &Window) -> Result<Vec<OhlcvCandle>> {
        self.pcts.ohlcv(pair, tf, window).await
    }
}

#[async_trait]
impl OnChainPort for CompositeAdapter {
    async fn governance(&self, pair: Option<&Pair>) -> Result<Vec<GovernanceSnapshot>> {
        let sqlite = self.sqlite.clone();
        let pair   = pair.cloned();
        tokio::task::spawn_blocking(move || sqlite.governance(pair.as_ref()))
            .await
            .map_err(|e| anyhow::anyhow!("spawn_blocking error: {e}"))?
    }

    async fn orderbook(&self, pair: &Pair, last_n: Option<u32>) -> Result<Vec<OrderBookSnapshot>> {
        let sqlite = self.sqlite.clone();
        let pair   = pair.clone();
        tokio::task::spawn_blocking(move || sqlite.orderbook(&pair, last_n))
            .await
            .map_err(|e| anyhow::anyhow!("spawn_blocking error: {e}"))?
    }

    async fn cosmos_stake_events(&self, last_n: Option<u32>, msg_type: Option<StakeMsgType>) -> Result<Vec<CosmosStakeEvent>> {
        let sqlite = self.sqlite.clone();
        tokio::task::spawn_blocking(move || sqlite.cosmos_stake_events(last_n, msg_type))
            .await
            .map_err(|e| anyhow::anyhow!("spawn_blocking error: {e}"))?
    }

    async fn transfers(&self, token: &str, last_n: Option<u32>) -> Result<Vec<TransferEvent>> {
        let sqlite = self.sqlite.clone();
        let token  = token.to_string();
        tokio::task::spawn_blocking(move || sqlite.transfers(&token, last_n))
            .await
            .map_err(|e| anyhow::anyhow!("spawn_blocking error: {e}"))?
    }

    async fn wallets(&self, address: Option<&str>) -> Result<Vec<WalletClassification>> {
        let sqlite  = self.sqlite.clone();
        let address = address.map(str::to_string);
        tokio::task::spawn_blocking(move || sqlite.wallets(address.as_deref()))
            .await
            .map_err(|e| anyhow::anyhow!("spawn_blocking error: {e}"))?
    }
}
