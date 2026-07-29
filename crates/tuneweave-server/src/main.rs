use std::{
    env,
    error::Error,
    io::{Error as IoError, ErrorKind},
    net::{Ipv4Addr, SocketAddr},
    path::PathBuf,
    sync::Arc,
};

use tokio::net::TcpListener;
use tracing::{error, info};
use tuneweave_core::{
    AccountCredentialStore, DirectoryUniPlaylistStore, FileAccountCredentialStore, Platform,
    ProviderRegistry, UniPlaylistStore,
};
use tuneweave_provider_bilibili::{BilibiliConfig, BilibiliProvider};
use tuneweave_provider_kugou::{KugouConfig, KugouProvider};
use tuneweave_provider_kuwo::{KuwoConfig, KuwoProvider};
use tuneweave_provider_migu::{MiguConfig, MiguProvider};
use tuneweave_provider_netease::{NeteaseConfig, NeteaseProvider};
use tuneweave_provider_qq::{QqConfig, QqProvider};
use tuneweave_provider_soda::{SodaConfig, SodaProvider};
use tuneweave_server::{
    AppState, build_router,
    logging::{LoggingConfig, init_logging},
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    let bind = env::var("TUNEWEAVE_BIND").unwrap_or_else(|_| "127.0.0.1:7832".to_owned());
    let address: SocketAddr = bind.parse()?;
    let data_dir = env::var_os("TUNEWEAVE_DATA_DIR")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(".local").join("data"));
    let logging_config = LoggingConfig::from_env(&data_dir)?;
    let logging = init_logging(&logging_config)?;
    for warning in &logging.retention_warnings {
        error!(
            error_kind = ?warning.error_kind,
            "failed to remove an expired log file"
        );
    }
    let credential_store: Arc<dyn AccountCredentialStore> = Arc::new(
        FileAccountCredentialStore::open(data_dir.join("accounts")).inspect_err(|error| {
            error!(
                stage = "credential_store_open",
                error_code = error.code.as_str(),
                "account credential store failed startup validation"
            );
        })?,
    );
    let uni_playlist_store: Arc<dyn UniPlaylistStore> = Arc::new(
        DirectoryUniPlaylistStore::open(data_dir.join("uni-playlists")).inspect_err(|error| {
            error!(
                stage = "uni_playlist_store_open",
                error_code = error.code.as_str(),
                "Uni Playlist store failed startup validation"
            );
        })?,
    );
    let netease_cookie = nonempty_env("TUNEWEAVE_NETEASE_COOKIE");
    let netease_proxy = nonempty_env("TUNEWEAVE_NETEASE_PROXY");
    let qq_proxy = nonempty_env("TUNEWEAVE_QQ_PROXY");
    let bilibili_proxy = nonempty_env("TUNEWEAVE_BILIBILI_PROXY");
    let kugou_proxy = nonempty_env("TUNEWEAVE_KUGOU_PROXY");
    let migu_proxy = nonempty_env("TUNEWEAVE_MIGU_PROXY");
    let kuwo_proxy = nonempty_env("TUNEWEAVE_KUWO_PROXY");
    let soda_proxy = nonempty_env("TUNEWEAVE_SODA_PROXY");
    let mut registry = ProviderRegistry::new();
    let netease_config = NeteaseConfig {
        cookie: netease_cookie.clone(),
        proxy_url: netease_proxy.clone(),
        real_ip: env::var("TUNEWEAVE_NETEASE_REAL_IP")
            .ok()
            .filter(|ip| !ip.trim().is_empty())
            .map(|ip| ip.trim().parse::<Ipv4Addr>())
            .transpose()?,
        random_cn_ip: env_bool("TUNEWEAVE_NETEASE_RANDOM_CN_IP")?,
        credential_store: Some(credential_store.clone()),
        ..NeteaseConfig::default()
    };
    registry.register(NeteaseProvider::new(netease_config)?)?;
    registry.register(QqProvider::new(QqConfig {
        proxy_url: qq_proxy.clone(),
        device_path: Some(data_dir.join("qq-device.json")),
        credential_store: Some(credential_store.clone()),
    })?)?;
    registry.register(BilibiliProvider::new(BilibiliConfig {
        proxy_url: bilibili_proxy.clone(),
        credential_store: Some(credential_store),
    })?)?;
    registry.register(KugouProvider::new(KugouConfig {
        proxy_url: kugou_proxy.clone(),
        device_path: Some(data_dir.join("kugou-device.json")),
    })?)?;
    registry.register(MiguProvider::new(MiguConfig {
        proxy_url: migu_proxy.clone(),
    })?)?;
    registry.register(KuwoProvider::new(KuwoConfig {
        proxy_url: kuwo_proxy.clone(),
    })?)?;
    registry.register(SodaProvider::new(SodaConfig {
        proxy_url: soda_proxy.clone(),
    })?)?;
    let state =
        AppState::new(registry, Platform::Netease).with_uni_playlist_store(uni_playlist_store);
    let app = build_router(state);
    let listener = TcpListener::bind(address).await?;

    info!(
        version = env!("CARGO_PKG_VERSION"),
        address = %listener.local_addr()?,
        data_dir = ?data_dir,
        log_format = logging_config.format.as_str(),
        log_filter_source = logging_config.filter_source.as_str(),
        log_to_stderr = logging_config.to_stderr,
        log_to_file = logging.file_output_active(),
        log_dir = ?logging_config.directory,
        log_max_files = logging_config.max_files,
        log_max_file_bytes = logging_config.max_file_bytes,
        log_max_total_bytes = logging_config.max_total_bytes,
        enabled_platforms = 7,
        default_platform = "netease",
        credential_store_open = true,
        uni_playlist_store_open = true,
        server_account_platforms = "netease,qq,bilibili",
        caller_credential_platforms = "netease,qq,bilibili",
        netease_bootstrap_cookie = netease_cookie.is_some(),
        netease_proxy = netease_proxy.is_some(),
        qq_proxy = qq_proxy.is_some(),
        bilibili_proxy = bilibili_proxy.is_some(),
        kugou_proxy = kugou_proxy.is_some(),
        migu_proxy = migu_proxy.is_some(),
        kuwo_proxy = kuwo_proxy.is_some(),
        soda_proxy = soda_proxy.is_some(),
        "TuneWeave startup completed"
    );
    let serve_result = axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await;
    if let Err(error) = &serve_result {
        error!(%error, "TuneWeave server exited with an error");
    }
    let dropped_lines = logging.dropped_lines();
    let file_write_errors = logging.file_write_errors();
    if dropped_lines > 0 || file_write_errors > 0 {
        error!(
            dropped_lines,
            file_write_errors, "non-blocking file logger encountered errors"
        );
    }
    info!(
        dropped_lines,
        file_write_errors, "TuneWeave shutdown completed"
    );
    drop(logging);
    serve_result?;
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

fn nonempty_env(name: &str) -> Option<String> {
    env::var(name)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

async fn shutdown_signal() {
    if let Err(error) = tokio::signal::ctrl_c().await {
        tracing::warn!(%error, "failed to install Ctrl+C handler");
    }
}
