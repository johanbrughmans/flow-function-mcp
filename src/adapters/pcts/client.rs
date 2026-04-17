/// PCTS adapter — reads OHLCV + trade-flow columns from SQL Server.
/// Schema: one database per pair (`agg_{pair_lower}`), tables `agg_{tf}`.
/// OHLCV columns are DECIMAL — CAST to float; utc is datetime2 — CONVERT to varchar.

use anyhow::{Context, Result};
use tiberius::{AuthMethod, Client, Config};
use tokio::net::TcpStream;
use tokio_util::compat::TokioAsyncWriteCompatExt;

use crate::domain::{candle::OhlcvCandle, pair::Pair, timeframe::Timeframe, window::Window};

#[derive(Clone)]
pub struct PctsAdapter {
    host: String,
    user: String,
    pass: String,
}

impl PctsAdapter {
    pub fn from_env() -> Result<Self> {
        let host = std::env::var("PCTS_HOST").unwrap_or_else(|_| "PCTS".to_string());
        let user = std::env::var("PCTS_USER").context("PCTS_USER env var not set")?;
        let pass = std::env::var("PCTS_PASS").context("PCTS_PASS env var not set")?;
        Ok(Self { host, user, pass })
    }

    fn db_name(pair: &Pair) -> String {
        format!("agg_{}", pair.as_str().to_lowercase())
    }

    async fn connect(&self, db: &str) -> Result<Client<tokio_util::compat::Compat<TcpStream>>> {
        let mut config = Config::new();
        config.host(&self.host);
        config.port(1433);
        config.authentication(AuthMethod::sql_server(&self.user, &self.pass));
        config.database(db);
        config.trust_cert();

        let tcp = TcpStream::connect(config.get_addr())
            .await
            .with_context(|| format!("TCP connect to PCTS ({}) failed", config.get_addr()))?;
        tcp.set_nodelay(true)?;

        Client::connect(config, tcp.compat_write())
            .await
            .with_context(|| format!("TDS handshake failed for database {db}"))
    }

    pub async fn ohlcv(&self, pair: &Pair, tf: Timeframe, window: &Window) -> Result<Vec<OhlcvCandle>> {
        let db    = Self::db_name(pair);
        let table = format!("agg_{}", tf.label());
        let fmt   = tf.ts_format();

        let mut client = self.connect(&db).await?;

        let sql = match window {
            Window::LastN(n) => format!(
                "SELECT TOP ({n}) CONVERT(varchar(19), utc, 120) AS utc, \
                 CAST([open] AS float) AS [open], CAST(high AS float) AS high, \
                 CAST(low AS float) AS low, CAST([close] AS float) AS [close], \
                 CAST(volume AS float) AS volume, \
                 CAST(MBv AS float) AS mb_vol, CAST(MSv AS float) AS ms_vol, \
                 CAST(LBv AS float) AS lb_vol, CAST(LSv AS float) AS ls_vol, \
                 MBc AS mb_count, MSc AS ms_count, LBc AS lb_count, LSc AS ls_count \
                 FROM {table} WHERE volume > 0 ORDER BY utc DESC"
            ),
            Window::Range { from, to } => format!(
                "SELECT CONVERT(varchar(19), utc, 120) AS utc, \
                 CAST([open] AS float) AS [open], CAST(high AS float) AS high, \
                 CAST(low AS float) AS low, CAST([close] AS float) AS [close], \
                 CAST(volume AS float) AS volume, \
                 CAST(MBv AS float) AS mb_vol, CAST(MSv AS float) AS ms_vol, \
                 CAST(LBv AS float) AS lb_vol, CAST(LSv AS float) AS ls_vol, \
                 MBc AS mb_count, MSc AS ms_count, LBc AS lb_count, LSc AS ls_count \
                 FROM {table} \
                 WHERE volume > 0 AND utc >= '{from}' AND utc <= '{to}' \
                 ORDER BY utc ASC"
            ),
        };

        let rows = client.query(&sql[..], &[]).await?.into_first_result().await?;
        let mut candles: Vec<OhlcvCandle> = rows.iter()
            .filter_map(|row| row_to_candle(row, fmt).ok())
            .collect();

        if matches!(window, Window::LastN(_)) { candles.reverse(); }
        Ok(candles)
    }
}

fn row_to_candle(row: &tiberius::Row, ts_fmt: &str) -> Result<OhlcvCandle> {
    let utc_str: &str = row.get("utc").context("utc column missing")?;
    let utc = chrono::NaiveDateTime::parse_from_str(utc_str, "%Y-%m-%d %H:%M:%S")
        .with_context(|| format!("cannot parse utc '{utc_str}'"))?;
    let ts = utc.format(ts_fmt).to_string();

    Ok(OhlcvCandle {
        ts,
        open:     get_f64(row, "open")?,
        high:     get_f64(row, "high")?,
        low:      get_f64(row, "low")?,
        close:    get_f64(row, "close")?,
        volume:   get_f64(row, "volume")?,
        mb_vol:   row.get::<f64, _>("mb_vol"),
        ms_vol:   row.get::<f64, _>("ms_vol"),
        lb_vol:   row.get::<f64, _>("lb_vol"),
        ls_vol:   row.get::<f64, _>("ls_vol"),
        mb_count: row.get::<i64, _>("mb_count"),
        ms_count: row.get::<i64, _>("ms_count"),
        lb_count: row.get::<i64, _>("lb_count"),
        ls_count: row.get::<i64, _>("ls_count"),
    })
}

fn get_f64(row: &tiberius::Row, col: &str) -> Result<f64> {
    row.get(col).with_context(|| format!("column '{col}' missing or wrong type"))
}
