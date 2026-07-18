use std::{
    env,
    error::Error,
    io::{Error as IoError, ErrorKind},
    net::{Ipv4Addr, SocketAddr},
    path::PathBuf,
    sync::Arc,
};

use tokio::net::TcpListener;
use tracing::info;
use tracing_subscriber::EnvFilter;
use tuneweave_core::{
    AccountCredentialStore, FileAccountCredentialStore, Platform, ProviderRegistry,
};
use tuneweave_provider_netease::{NeteaseConfig, NeteaseProvider};
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
    let data_dir = env::var_os("TUNEWEAVE_DATA_DIR")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(".local").join("data"));
    let credential_store: Arc<dyn AccountCredentialStore> =
        Arc::new(FileAccountCredentialStore::new(data_dir.join("accounts")));
    let mut registry = ProviderRegistry::new();
    let netease_config = NeteaseConfig {
        cookie: env::var("TUNEWEAVE_NETEASE_COOKIE")
            .ok()
            .filter(|cookie| !cookie.trim().is_empty()),
        proxy_url: env::var("TUNEWEAVE_NETEASE_PROXY")
            .ok()
            .filter(|proxy| !proxy.trim().is_empty()),
        real_ip: env::var("TUNEWEAVE_NETEASE_REAL_IP")
            .ok()
            .filter(|ip| !ip.trim().is_empty())
            .map(|ip| ip.trim().parse::<Ipv4Addr>())
            .transpose()?,
        random_cn_ip: env_bool("TUNEWEAVE_NETEASE_RANDOM_CN_IP")?,
        credential_store: Some(credential_store),
        ..NeteaseConfig::default()
    };
    registry.register(NeteaseProvider::new(netease_config)?)?;
    let state = AppState::new(registry, Platform::Netease);
    let app = build_router(state);
    let listener = TcpListener::bind(address).await?;

    info!(address = %listener.local_addr()?, "TuneWeave is listening");
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

fn env_bool(name: &str) -> Result<bool, IoError> {
    let Some(value) = env::var(name).ok() else {
        return Ok(false);
    };
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" | "" => Ok(false),
        _ => Err(IoError::new(
            ErrorKind::InvalidInput,
            format!("{name} must be true/false, yes/no, on/off, or 1/0"),
        )),
    }
}

async fn shutdown_signal() {
    if let Err(error) = tokio::signal::ctrl_c().await {
        tracing::warn!(%error, "failed to install Ctrl+C handler");
    }
}
