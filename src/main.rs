/// flow-function-mcp — Function Layer MCP Server (TGF Layer 3)
///
/// Stateless signal computation: indicators, SMC, order flow, on-chain analysis.
/// All 16 tools are QUERY — no write operations.
///
/// Hexagonal architecture:
///   Inbound  adapter: MCP HTTP (FlowFunctionServer)
///   Domain:           indicators, SMC, HA patterns, order flow, on-chain functions
///   Outbound adapters: PCTS SQL Server (OHLCV + trade-flow), OMV SQLite (governance, orderbook, etc.)
///
/// .env (at /opt/flow-function-mcp/.env on OMV):
///   FLOW_FUNCTION_PORT=3467
///   FLOW_DATA_DB=/opt/enj-flow/data/token-flow.db
///   PCTS_HOST=192.168.0.137
///   PCTS_USER=<user>
///   PCTS_PASS=<pass>

mod adapters;
mod domain;
mod ports;

use std::sync::Arc;
use anyhow::{Context, Result};
use axum::Router;
use rmcp::transport::{
    StreamableHttpServerConfig,
    streamable_http_server::{session::never::NeverSessionManager, tower::StreamableHttpService},
};

use adapters::{
    composite::CompositeAdapter,
    mcp::FlowFunctionServer,
    pcts::PctsAdapter,
    sqlite::SqliteAdapter,
};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("flow_function_mcp=info,rmcp=warn")),
        )
        .init();

    let _ = dotenvy::dotenv();

    let port = std::env::var("FLOW_FUNCTION_PORT").unwrap_or_else(|_| "3467".to_string());
    let db   = std::env::var("FLOW_DATA_DB")
        .unwrap_or_else(|_| "/opt/enj-flow/data/token-flow.db".to_string());

    let pcts   = PctsAdapter::from_env()
        .context("PCTS adapter init — set PCTS_USER and PCTS_PASS in .env")?;
    let sqlite = SqliteAdapter::new(&db);

    let adapter = Arc::new(CompositeAdapter::new(pcts, sqlite));
    let bind    = format!("0.0.0.0:{port}");

    tracing::info!("flow-function-mcp starting on http://{}/mcp", bind);
    tracing::info!("SQLite: {}", db);

    let adapter_clone   = adapter.clone();
    let session_manager = Arc::new(NeverSessionManager::default());

    let service: StreamableHttpService<FlowFunctionServer, NeverSessionManager> =
        StreamableHttpService::new(
            move || Ok::<_, std::io::Error>(FlowFunctionServer::new(adapter_clone.clone())),
            session_manager,
            StreamableHttpServerConfig::default()
                .with_stateful_mode(false)
                .with_json_response(true)
                .disable_allowed_hosts(),
        );

    let router = Router::new()
        .nest_service("/mcp", service)
        .route("/health", axum::routing::get(|| async { "flow-function-mcp ok" }));

    let listener = tokio::net::TcpListener::bind(&bind)
        .await
        .with_context(|| format!("bind failed at {bind}"))?;

    tracing::info!("Listening. MCP endpoint: http://192.168.0.126:{port}/mcp");
    axum::serve(listener, router).await.context("server error")?;
    Ok(())
}
