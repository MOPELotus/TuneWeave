use std::{env, error::Error, net::SocketAddr};

use tokio::net::TcpListener;
use tracing::info;
use tracing_subscriber::EnvFilter;
use tuneweave_core::{Platform, ProviderRegistry};
use tuneweave_server::{AppState, build_router};

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("tuneweave=info")),
        )
        .init();

    let bind = env::var("TUNEWEAVE_BIND").unwrap_or_else(|_| "127.0.0.1:7832".to_owned());
    let address: SocketAddr = bind.parse()?;
    let state = AppState::new(ProviderRegistry::new(), Platform::Netease);
    let app = build_router(state);
    let listener = TcpListener::bind(address).await?;

    info!(address = %listener.local_addr()?, "TuneWeave is listening");
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

async fn shutdown_signal() {
    if let Err(error) = tokio::signal::ctrl_c().await {
        tracing::warn!(%error, "failed to install Ctrl+C handler");
    }
}
