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
    AccountCredentialStore, DirectoryUniPlaylistStore, ErrorCode, FileAccountCredentialStore,
    MusicProvider, Platform, ProviderRegistry, TuneWeaveError, UniPlaylistStore,
};
use tuneweave_provider_bilibili::{BilibiliConfig, BilibiliProvider};
use tuneweave_provider_kugou::{KugouConfig, KugouProvider};
use tuneweave_provider_kuwo::{KuwoConfig, KuwoProvider};
use tuneweave_provider_migu::{MiguConfig, MiguProvider};
use tuneweave_provider_netease::{NeteaseConfig, NeteaseProvider};
use tuneweave_provider_qq::{QqConfig, QqProvider};
use tuneweave_provider_soda::{SodaConfig, SodaProvider};
use tuneweave_server::{
    AppState, ShutdownSnapshot, build_router,
    logging::{LoggingConfig, init_logging},
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    let bind = env::var("TUNEWEAVE_BIND").unwrap_or_else(|_| "127.0.0.1:7832".to_owned());
    let data_dir = env::var_os("TUNEWEAVE_DATA_DIR")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(".local").join("data"));
    let logging_config = LoggingConfig::from_env(&data_dir)?;
    let logging = init_logging(&logging_config)?;
    install_panic_observer();
    for warning in &logging.retention_warnings {
        error!(
            error_kind = ?warning.error_kind,
            "failed to remove an expired log file"
        );
    }
    let address: SocketAddr = bind.parse().inspect_err(|_| {
        log_server_configuration_failure(
            ServerLifecycleStage::BindConfiguration,
            "invalid_socket_address",
        );
    })?;
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
    let netease_real_ip = env::var("TUNEWEAVE_NETEASE_REAL_IP")
        .ok()
        .filter(|ip| !ip.trim().is_empty())
        .map(|ip| ip.trim().parse::<Ipv4Addr>())
        .transpose()
        .inspect_err(|_| {
            log_provider_configuration_failure(Platform::Netease, "real_ip");
        })?;
    let netease_random_cn_ip = env_bool("TUNEWEAVE_NETEASE_RANDOM_CN_IP").inspect_err(|_| {
        log_provider_configuration_failure(Platform::Netease, "random_cn_ip");
    })?;
    let netease_config = NeteaseConfig {
        cookie: netease_cookie.clone(),
        proxy_url: netease_proxy.clone(),
        real_ip: netease_real_ip,
        random_cn_ip: netease_random_cn_ip,
        credential_store: Some(credential_store.clone()),
        ..NeteaseConfig::default()
    };
    register_provider(
        &mut registry,
        Platform::Netease,
        NeteaseProvider::new(netease_config),
    )?;
    register_provider(
        &mut registry,
        Platform::Qq,
        QqProvider::new(QqConfig {
            proxy_url: qq_proxy.clone(),
            device_path: Some(data_dir.join("qq-device.json")),
            credential_store: Some(credential_store.clone()),
        }),
    )?;
    register_provider(
        &mut registry,
        Platform::Bilibili,
        BilibiliProvider::new(BilibiliConfig {
            proxy_url: bilibili_proxy.clone(),
            credential_store: Some(credential_store.clone()),
        }),
    )?;
    register_provider(
        &mut registry,
        Platform::Kugou,
        KugouProvider::new(KugouConfig {
            proxy_url: kugou_proxy.clone(),
            device_path: Some(data_dir.join("kugou-device.json")),
        }),
    )?;
    register_provider(
        &mut registry,
        Platform::Migu,
        MiguProvider::new(MiguConfig {
            proxy_url: migu_proxy.clone(),
        }),
    )?;
    register_provider(
        &mut registry,
        Platform::Kuwo,
        KuwoProvider::new(KuwoConfig {
            proxy_url: kuwo_proxy.clone(),
        }),
    )?;
    register_provider(
        &mut registry,
        Platform::Soda,
        SodaProvider::new(SodaConfig {
            proxy_url: soda_proxy.clone(),
            device_path: Some(data_dir.join("soda-device.json")),
            credential_store: Some(credential_store),
        }),
    )?;
    let state =
        AppState::new(registry, Platform::Netease).with_uni_playlist_store(uni_playlist_store);
    let app = build_router(state.clone());
    let listener = TcpListener::bind(address).await.inspect_err(|error| {
        log_server_io_failure(ServerLifecycleStage::ListenerBind, error);
    })?;
    let local_address = listener.local_addr().inspect_err(|error| {
        log_server_io_failure(ServerLifecycleStage::ListenerAddress, error);
    })?;

    info!(
        version = env!("CARGO_PKG_VERSION"),
        address = %local_address,
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
        server_account_platforms = "netease,qq,bilibili,soda",
        caller_credential_platforms = "netease,qq,bilibili,soda",
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
        .with_graceful_shutdown(shutdown_signal(state.clone()))
        .await;
    if let Err(error) = &serve_result {
        log_server_io_failure(ServerLifecycleStage::Serve, error);
    }
    let dropped_lines = logging.dropped_lines();
    let file_write_errors = logging.file_write_errors();
    let shutdown_snapshot = read_shutdown_snapshot(&state, "shutdown_completed");
    if dropped_lines > 0 || file_write_errors > 0 {
        error!(
            dropped_lines,
            file_write_errors, "non-blocking file logger encountered errors"
        );
    }
    info!(
        dropped_lines,
        file_write_errors,
        snapshot_available = shutdown_snapshot.is_some(),
        active_requests = shutdown_snapshot.map_or(0, |snapshot| snapshot.active_requests),
        auth_transactions = shutdown_snapshot.map_or(0, |snapshot| snapshot.auth_transactions),
        qr_auth_transactions =
            shutdown_snapshot.map_or(0, |snapshot| snapshot.qr_auth_transactions),
        sms_auth_transactions =
            shutdown_snapshot.map_or(0, |snapshot| snapshot.sms_auth_transactions),
        "TuneWeave shutdown completed"
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ServerLifecycleStage {
    BindConfiguration,
    ListenerBind,
    ListenerAddress,
    Serve,
}

impl ServerLifecycleStage {
    const fn as_str(self) -> &'static str {
        match self {
            Self::BindConfiguration => "bind_configuration",
            Self::ListenerBind => "listener_bind",
            Self::ListenerAddress => "listener_address",
            Self::Serve => "serve",
        }
    }
}

fn log_server_configuration_failure(stage: ServerLifecycleStage, error_kind: &'static str) {
    error!(
        event = "server_lifecycle_failure",
        stage = stage.as_str(),
        error_kind,
        "TuneWeave server failed lifecycle validation"
    );
}

fn log_server_io_failure(stage: ServerLifecycleStage, error: &IoError) {
    error!(
        event = "server_lifecycle_failure",
        stage = stage.as_str(),
        error_kind = ?error.kind(),
        "TuneWeave server failed lifecycle validation"
    );
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ProviderStartupStage {
    Configure,
    Initialize,
    Register,
}

impl ProviderStartupStage {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Configure => "configure",
            Self::Initialize => "initialize",
            Self::Register => "register",
        }
    }
}

fn log_provider_configuration_failure(platform: Platform, setting: &'static str) {
    error!(
        event = "provider_startup_failure",
        provider = platform.as_str(),
        stage = ProviderStartupStage::Configure.as_str(),
        setting,
        error_code = ErrorCode::InvalidRequest.as_str(),
        retryable = false,
        "TuneWeave provider failed startup validation"
    );
}

fn log_provider_startup_failure(
    platform: Platform,
    stage: ProviderStartupStage,
    error: &TuneWeaveError,
) {
    error!(
        event = "provider_startup_failure",
        provider = platform.as_str(),
        stage = stage.as_str(),
        error_code = error.code.as_str(),
        retryable = error.retryable,
        "TuneWeave provider failed startup validation"
    );
}

fn register_provider<P>(
    registry: &mut ProviderRegistry,
    platform: Platform,
    provider: tuneweave_core::Result<P>,
) -> tuneweave_core::Result<()>
where
    P: MusicProvider + 'static,
{
    let provider = provider.inspect_err(|error| {
        log_provider_startup_failure(platform, ProviderStartupStage::Initialize, error);
    })?;
    registry.register(provider).inspect_err(|error| {
        log_provider_startup_failure(platform, ProviderStartupStage::Register, error);
    })
}

fn install_panic_observer() {
    std::panic::set_hook(Box::new(|panic| {
        if let Some(location) = panic.location() {
            error!(
                event = "panic",
                source_file = location.file(),
                source_line = location.line(),
                source_column = location.column(),
                "TuneWeave task panicked"
            );
        } else {
            error!(event = "panic", "TuneWeave task panicked");
        }
    }));
}

fn read_shutdown_snapshot(state: &AppState, stage: &'static str) -> Option<ShutdownSnapshot> {
    match state.shutdown_snapshot() {
        Ok(snapshot) => Some(snapshot),
        Err(error) => {
            error!(
                stage,
                error_code = error.code.as_str(),
                "TuneWeave shutdown snapshot failed"
            );
            None
        }
    }
}

async fn shutdown_signal(state: AppState) {
    let reason = wait_for_shutdown_reason().await;
    let snapshot = read_shutdown_snapshot(&state, "shutdown_requested");
    info!(
        shutdown_reason = reason.as_str(),
        snapshot_available = snapshot.is_some(),
        active_requests = snapshot.map_or(0, |snapshot| snapshot.active_requests),
        auth_transactions = snapshot.map_or(0, |snapshot| snapshot.auth_transactions),
        qr_auth_transactions = snapshot.map_or(0, |snapshot| snapshot.qr_auth_transactions),
        sms_auth_transactions = snapshot.map_or(0, |snapshot| snapshot.sms_auth_transactions),
        "TuneWeave shutdown requested"
    );
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ShutdownReason {
    CtrlC,
    #[cfg(unix)]
    Terminate,
    SignalHandlerFailed,
}

impl ShutdownReason {
    const fn as_str(self) -> &'static str {
        match self {
            Self::CtrlC => "ctrl_c",
            #[cfg(unix)]
            Self::Terminate => "terminate",
            Self::SignalHandlerFailed => "signal_handler_failed",
        }
    }
}

async fn wait_for_ctrl_c() -> ShutdownReason {
    match tokio::signal::ctrl_c().await {
        Ok(()) => ShutdownReason::CtrlC,
        Err(error) => {
            tracing::warn!(
                error_kind = ?error.kind(),
                "failed to receive the Ctrl+C shutdown signal"
            );
            ShutdownReason::SignalHandlerFailed
        }
    }
}

#[cfg(not(unix))]
async fn wait_for_shutdown_reason() -> ShutdownReason {
    wait_for_ctrl_c().await
}

#[cfg(unix)]
async fn wait_for_shutdown_reason() -> ShutdownReason {
    use tokio::signal::unix::{SignalKind, signal};

    let mut terminate = match signal(SignalKind::terminate()) {
        Ok(signal) => signal,
        Err(error) => {
            tracing::warn!(
                error_kind = ?error.kind(),
                "failed to install the terminate shutdown signal"
            );
            return wait_for_ctrl_c().await;
        }
    };
    tokio::select! {
        reason = wait_for_ctrl_c() => reason,
        received = terminate.recv() => {
            if received.is_some() {
                ShutdownReason::Terminate
            } else {
                tracing::warn!("terminate shutdown signal stream ended unexpectedly");
                wait_for_ctrl_c().await
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ProviderStartupStage, ServerLifecycleStage, ShutdownReason};

    #[test]
    fn server_lifecycle_stages_use_stable_safe_names() {
        assert_eq!(
            ServerLifecycleStage::BindConfiguration.as_str(),
            "bind_configuration"
        );
        assert_eq!(ServerLifecycleStage::ListenerBind.as_str(), "listener_bind");
        assert_eq!(
            ServerLifecycleStage::ListenerAddress.as_str(),
            "listener_address"
        );
        assert_eq!(ServerLifecycleStage::Serve.as_str(), "serve");
    }

    #[test]
    fn provider_startup_stages_use_stable_safe_names() {
        assert_eq!(ProviderStartupStage::Configure.as_str(), "configure");
        assert_eq!(ProviderStartupStage::Initialize.as_str(), "initialize");
        assert_eq!(ProviderStartupStage::Register.as_str(), "register");
    }

    #[test]
    fn shutdown_reasons_use_stable_safe_names() {
        assert_eq!(ShutdownReason::CtrlC.as_str(), "ctrl_c");
        #[cfg(unix)]
        assert_eq!(ShutdownReason::Terminate.as_str(), "terminate");
        assert_eq!(
            ShutdownReason::SignalHandlerFailed.as_str(),
            "signal_handler_failed"
        );
    }
}
