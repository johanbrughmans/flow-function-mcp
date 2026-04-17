/// MarketDataPort — outbound port for OHLCV data.
/// Implemented by PctsAdapter (primary) via CompositeAdapter.

use anyhow::Result;
use async_trait::async_trait;

use crate::domain::{candle::OhlcvCandle, pair::Pair, timeframe::Timeframe, window::Window};

#[async_trait]
pub trait MarketDataPort: Send + Sync + 'static {
    async fn ohlcv(&self, pair: &Pair, tf: Timeframe, window: &Window) -> Result<Vec<OhlcvCandle>>;
}
