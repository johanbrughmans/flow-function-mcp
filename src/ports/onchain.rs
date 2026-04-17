/// OnChainPort — outbound port for OMV SQLite data (governance, orderbook, cosmos, transfers, wallets).
/// Implemented by SqliteAdapter via CompositeAdapter.

use anyhow::Result;
use async_trait::async_trait;

use crate::domain::{
    onchain::{
        governance::GovernanceSnapshot,
        orderbook::OrderBookSnapshot,
        staking::{CosmosStakeEvent, StakeMsgType},
        wallet::{TransferEvent, WalletClassification},
    },
    pair::Pair,
};

#[async_trait]
pub trait OnChainPort: Send + Sync + 'static {
    async fn governance(&self, pair: Option<&Pair>) -> Result<Vec<GovernanceSnapshot>>;
    async fn orderbook(&self, pair: &Pair, last_n: Option<u32>) -> Result<Vec<OrderBookSnapshot>>;
    async fn cosmos_stake_events(&self, last_n: Option<u32>, msg_type: Option<StakeMsgType>) -> Result<Vec<CosmosStakeEvent>>;
    async fn transfers(&self, token: &str, last_n: Option<u32>) -> Result<Vec<TransferEvent>>;
    async fn wallets(&self, address: Option<&str>) -> Result<Vec<WalletClassification>>;
}
