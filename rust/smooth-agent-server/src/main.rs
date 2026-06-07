//! Binary entry point for the smooth-agent WebSocket service.
//!
//! Reads configuration from the environment (see
//! [`smooth_agent_server::config`]) and serves the `/ws` endpoint until killed.

use anyhow::Result;
use smooth_agent_server::config::ServerConfig;

#[tokio::main]
async fn main() -> Result<()> {
    // Lightweight tracing — honors RUST_LOG, defaults to info for this crate.
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                tracing_subscriber::EnvFilter::new("info,smooth_agent_server=info")
            }),
        )
        .init();

    let config = ServerConfig::from_env();
    smooth_agent_server::server::run(config).await
}
