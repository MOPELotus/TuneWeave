use std::{
    collections::BTreeMap,
    env, fs,
    io::{self, ErrorKind},
    path::{Component, Path, PathBuf},
    time::{Duration, SystemTime},
};

use tracing_appender::{
    non_blocking::{ErrorCounter, NonBlocking, NonBlockingBuilder, WorkerGuard},
    rolling::{RollingFileAppender, Rotation},
};
use tracing_subscriber::{
    EnvFilter,
    fmt::{MakeWriter, writer::MakeWriterExt},
};

const DEFAULT_LOG_FILE: &str = "tuneweave.log";
const DEFAULT_RETENTION_DAYS: u32 = 14;
const DEFAULT_MAX_FILES: usize = 30;
const MAX_RETENTION_DAYS: u32 = 3_650;
const MAX_LOG_FILES: usize = 10_000;
const LOG_ENVIRONMENT_VARIABLES: [&str; 9] = [
    "TUNEWEAVE_LOG_LEVEL",
    "RUST_LOG",
    "TUNEWEAVE_LOG_FORMAT",
    "TUNEWEAVE_LOG_DIR",
    "TUNEWEAVE_LOG_FILE",
    "TUNEWEAVE_LOG_RETENTION_DAYS",
    "TUNEWEAVE_LOG_MAX_FILES",
    "TUNEWEAVE_LOG_TO_STDERR",
    "TUNEWEAVE_LOG_TO_FILE",
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LogFormat {
    Human,
    Json,
}

impl LogFormat {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Human => "human",
            Self::Json => "json",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LogFilterSource {
    TuneWeave,
    RustLog,
    Default,
}

impl LogFilterSource {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::TuneWeave => "tuneweave_log_level",
            Self::RustLog => "rust_log",
            Self::Default => "default",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LoggingConfig {
    filter: String,
    data_directory: PathBuf,
    pub filter_source: LogFilterSource,
    pub format: LogFormat,
    pub directory: PathBuf,
    pub file_name: String,
    pub retention_days: u32,
    pub max_files: usize,
    pub to_stderr: bool,
    pub to_file: bool,
}

impl LoggingConfig {
    pub fn from_env(data_dir: &Path) -> io::Result<Self> {
        let mut values = BTreeMap::new();
        for name in LOG_ENVIRONMENT_VARIABLES {
            match env::var(name) {
                Ok(value) => {
                    values.insert(name.to_owned(), value);
                }
                Err(env::VarError::NotPresent) => {}
                Err(env::VarError::NotUnicode(_)) => {
                    return Err(invalid_config(format!(
                        "{name} must contain valid UTF-8 text"
                    )));
                }
            }
        }
        Self::from_values(data_dir, &values)
    }

    fn from_values(data_dir: &Path, values: &BTreeMap<String, String>) -> io::Result<Self> {
        let explicit_level = values.get("TUNEWEAVE_LOG_LEVEL");
        let rust_log = values.get("RUST_LOG");
        let (filter, filter_source) = if let Some(level) = explicit_level {
            let level = level.trim().to_ascii_lowercase();
            if !matches!(
                level.as_str(),
                "trace" | "debug" | "info" | "warn" | "error" | "off"
            ) {
                return Err(invalid_config(
                    "TUNEWEAVE_LOG_LEVEL must be trace, debug, info, warn, error, or off",
                ));
            }
            (format!("tuneweave={level}"), LogFilterSource::TuneWeave)
        } else if let Some(filter) = rust_log.filter(|value| !value.trim().is_empty()) {
            EnvFilter::try_new(filter).map_err(|_| {
                invalid_config("RUST_LOG must be a valid tracing filter expression")
            })?;
            (filter.to_owned(), LogFilterSource::RustLog)
        } else {
            ("tuneweave=info".to_owned(), LogFilterSource::Default)
        };

        let format = match values
            .get("TUNEWEAVE_LOG_FORMAT")
            .map(|value| value.trim().to_ascii_lowercase())
            .as_deref()
        {
            None | Some("") | Some("human") | Some("text") => LogFormat::Human,
            Some("json") => LogFormat::Json,
            Some(_) => {
                return Err(invalid_config(
                    "TUNEWEAVE_LOG_FORMAT must be human, text, or json",
                ));
            }
        };
        let to_stderr = parse_bool(values, "TUNEWEAVE_LOG_TO_STDERR", true)?;
        let to_file = parse_bool(values, "TUNEWEAVE_LOG_TO_FILE", true)?;
        if !to_stderr && !to_file {
            return Err(invalid_config(
                "at least one logging output must remain enabled",
            ));
        }
        let file_name = values
            .get("TUNEWEAVE_LOG_FILE")
            .map_or(DEFAULT_LOG_FILE, String::as_str)
            .trim()
            .to_owned();
        validate_file_name(&file_name)?;
        let retention_days = parse_bounded_u32(
            values,
            "TUNEWEAVE_LOG_RETENTION_DAYS",
            DEFAULT_RETENTION_DAYS,
            1,
            MAX_RETENTION_DAYS,
        )?;
        let max_files = usize::try_from(parse_bounded_u32(
            values,
            "TUNEWEAVE_LOG_MAX_FILES",
            u32::try_from(DEFAULT_MAX_FILES).unwrap_or(u32::MAX),
            1,
            u32::try_from(MAX_LOG_FILES).unwrap_or(u32::MAX),
        )?)
        .map_err(|_| invalid_config("TUNEWEAVE_LOG_MAX_FILES is too large"))?;
        let directory = values
            .get("TUNEWEAVE_LOG_DIR")
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| data_dir.join("logs"));
        if to_file {
            validate_directory_separation(data_dir, &directory)?;
        }

        Ok(Self {
            filter,
            data_directory: data_dir.to_path_buf(),
            filter_source,
            format,
            directory,
            file_name,
            retention_days,
            max_files,
            to_stderr,
            to_file,
        })
    }

    fn env_filter(&self) -> io::Result<EnvFilter> {
        EnvFilter::try_new(&self.filter)
            .map_err(|_| invalid_config("configured log filter is invalid"))
    }
}

#[derive(Debug)]
pub struct RetentionWarning {
    pub file_name: String,
    pub error_kind: ErrorKind,
}

pub struct LoggingHandle {
    file_guard: Option<WorkerGuard>,
    error_counter: Option<ErrorCounter>,
    pub retention_warnings: Vec<RetentionWarning>,
}

impl LoggingHandle {
    #[must_use]
    pub fn dropped_lines(&self) -> usize {
        self.error_counter
            .as_ref()
            .map_or(0, ErrorCounter::dropped_lines)
    }

    #[must_use]
    pub const fn file_output_active(&self) -> bool {
        self.file_guard.is_some()
    }
}

pub fn init_logging(
    config: &LoggingConfig,
) -> Result<LoggingHandle, Box<dyn std::error::Error + Send + Sync>> {
    let file_output = config
        .to_file
        .then(|| build_file_output(config))
        .transpose()?;
    let (file_writer, file_guard, error_counter, retention_warnings) = match file_output {
        Some(output) => (
            Some(output.writer),
            Some(output.guard),
            Some(output.error_counter),
            output.retention_warnings,
        ),
        None => (None, None, None, Vec::new()),
    };

    match (config.to_stderr, file_writer) {
        (true, Some(writer)) => install_subscriber(config, std::io::stderr.and(writer))?,
        (true, None) => install_subscriber(config, std::io::stderr)?,
        (false, Some(writer)) => install_subscriber(config, writer)?,
        (false, None) => unreachable!("logging configuration requires at least one output"),
    }

    Ok(LoggingHandle {
        file_guard,
        error_counter,
        retention_warnings,
    })
}

fn install_subscriber<W>(
    config: &LoggingConfig,
    writer: W,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>>
where
    W: for<'writer> MakeWriter<'writer> + Send + Sync + 'static,
{
    let builder = tracing_subscriber::fmt()
        .with_env_filter(config.env_filter()?)
        .with_writer(writer)
        .with_ansi(false);
    match config.format {
        LogFormat::Human => builder.try_init()?,
        LogFormat::Json => builder.json().try_init()?,
    }
    Ok(())
}

struct FileOutput {
    writer: NonBlocking,
    guard: WorkerGuard,
    error_counter: ErrorCounter,
    retention_warnings: Vec<RetentionWarning>,
}

fn build_file_output(config: &LoggingConfig) -> io::Result<FileOutput> {
    fs::create_dir_all(&config.data_directory)?;
    fs::create_dir_all(&config.directory)?;
    validate_canonical_directory_separation(&config.data_directory, &config.directory)?;
    let retention_warnings = prune_expired_files(config)?;
    let appender = RollingFileAppender::builder()
        .rotation(Rotation::DAILY)
        .filename_prefix(&config.file_name)
        .max_log_files(config.max_files)
        .build(&config.directory)
        .map_err(|error| io::Error::other(format!("failed to open rolling log file: {error}")))?;
    let (writer, guard) = NonBlockingBuilder::default()
        .lossy(true)
        .thread_name("tuneweave-log-writer")
        .finish(appender);
    let error_counter = writer.error_counter();
    Ok(FileOutput {
        writer,
        guard,
        error_counter,
        retention_warnings,
    })
}

fn prune_expired_files(config: &LoggingConfig) -> io::Result<Vec<RetentionWarning>> {
    let cutoff = SystemTime::now()
        .checked_sub(Duration::from_secs(
            u64::from(config.retention_days).saturating_mul(86_400),
        ))
        .ok_or_else(|| io::Error::other("log retention cutoff overflowed"))?;
    let prefix = format!("{}.", config.file_name);
    let mut warnings = Vec::new();
    for entry in fs::read_dir(&config.directory)? {
        let entry = entry?;
        let file_name = entry.file_name().to_string_lossy().into_owned();
        if !file_name.starts_with(&prefix) {
            continue;
        }
        let metadata = fs::symlink_metadata(entry.path())?;
        if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
            continue;
        }
        let expired = metadata.modified().is_ok_and(|modified| modified < cutoff);
        if expired && let Err(error) = fs::remove_file(entry.path()) {
            warnings.push(RetentionWarning {
                file_name,
                error_kind: error.kind(),
            });
        }
    }
    Ok(warnings)
}

fn validate_file_name(value: &str) -> io::Result<()> {
    let stem = value
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || stem
            .strip_prefix("COM")
            .or_else(|| stem.strip_prefix("LPT"))
            .is_some_and(|suffix| suffix.len() == 1 && matches!(suffix.as_bytes()[0], b'1'..=b'9'));
    if value.is_empty()
        || value.len() > 128
        || value.ends_with('.')
        || reserved
        || !value.bytes().enumerate().all(|(index, byte)| {
            byte.is_ascii_alphanumeric() || (index > 0 && b"._-".contains(&byte))
        })
    {
        return Err(invalid_config(
            "TUNEWEAVE_LOG_FILE must be a 1 to 128 byte portable file name",
        ));
    }
    Ok(())
}

fn validate_directory_separation(data_dir: &Path, log_dir: &Path) -> io::Result<()> {
    let data_dir = lexical_absolute(data_dir)?;
    let log_dir = lexical_absolute(log_dir)?;
    if log_dir == data_dir
        || data_dir.starts_with(&log_dir)
        || log_dir.starts_with(data_dir.join("accounts"))
        || log_dir.starts_with(data_dir.join("uni-playlists"))
    {
        return Err(invalid_config(
            "TUNEWEAVE_LOG_DIR must be separate from credentials and Uni Playlist data",
        ));
    }
    Ok(())
}

fn validate_canonical_directory_separation(data_dir: &Path, log_dir: &Path) -> io::Result<()> {
    let canonical_log_dir = fs::canonicalize(log_dir)?;
    let canonical_data_dir = fs::canonicalize(data_dir)?;
    if canonical_log_dir == canonical_data_dir
        || canonical_data_dir.starts_with(&canonical_log_dir)
        || canonical_log_dir.starts_with(canonical_data_dir.join("accounts"))
        || canonical_log_dir.starts_with(canonical_data_dir.join("uni-playlists"))
    {
        return Err(invalid_config(
            "log directory resolved into a protected TuneWeave data directory",
        ));
    }
    Ok(())
}

fn lexical_absolute(path: &Path) -> io::Result<PathBuf> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        env::current_dir()?.join(path)
    };
    let mut normalized = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::Prefix(_) | Component::RootDir | Component::Normal(_) => {
                normalized.push(component.as_os_str());
            }
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
        }
    }
    Ok(normalized)
}

fn parse_bool(values: &BTreeMap<String, String>, name: &str, default: bool) -> io::Result<bool> {
    let Some(value) = values.get(name) else {
        return Ok(default);
    };
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" => Ok(false),
        _ => Err(invalid_config(format!(
            "{name} must be true/false, yes/no, on/off, or 1/0"
        ))),
    }
}

fn parse_bounded_u32(
    values: &BTreeMap<String, String>,
    name: &str,
    default: u32,
    minimum: u32,
    maximum: u32,
) -> io::Result<u32> {
    let Some(value) = values.get(name) else {
        return Ok(default);
    };
    let parsed = value
        .trim()
        .parse::<u32>()
        .ok()
        .filter(|value| (*value >= minimum) && (*value <= maximum))
        .ok_or_else(|| {
            invalid_config(format!(
                "{name} must be an integer between {minimum} and {maximum}"
            ))
        })?;
    Ok(parsed)
}

fn invalid_config(message: impl Into<String>) -> io::Error {
    io::Error::new(ErrorKind::InvalidInput, message.into())
}

#[cfg(test)]
mod tests {
    use std::{
        sync::atomic::{AtomicU64, Ordering},
        time::{SystemTime, UNIX_EPOCH},
    };

    use tracing::info;

    use super::*;

    static TEST_SEQUENCE: AtomicU64 = AtomicU64::new(1);

    #[test]
    fn defaults_enable_human_console_and_daily_file_output() {
        let config = LoggingConfig::from_values(Path::new("private-data"), &BTreeMap::new())
            .expect("default logging config");
        assert_eq!(config.filter, "tuneweave=info");
        assert_eq!(config.filter_source, LogFilterSource::Default);
        assert_eq!(config.format, LogFormat::Human);
        assert!(config.to_stderr);
        assert!(config.to_file);
        assert_eq!(config.directory, Path::new("private-data/logs"));
        assert_eq!(config.file_name, DEFAULT_LOG_FILE);
        assert_eq!(config.retention_days, DEFAULT_RETENTION_DAYS);
        assert_eq!(config.max_files, DEFAULT_MAX_FILES);
    }

    #[test]
    fn tuneweave_level_overrides_rust_log_and_json_targets_are_explicit() {
        let values = values(&[
            ("TUNEWEAVE_LOG_LEVEL", "debug"),
            ("RUST_LOG", "tuneweave=error,hyper=trace"),
            ("TUNEWEAVE_LOG_FORMAT", "json"),
            ("TUNEWEAVE_LOG_TO_STDERR", "no"),
            ("TUNEWEAVE_LOG_TO_FILE", "yes"),
            ("TUNEWEAVE_LOG_FILE", "api.jsonl"),
            ("TUNEWEAVE_LOG_RETENTION_DAYS", "7"),
            ("TUNEWEAVE_LOG_MAX_FILES", "9"),
        ]);
        let config = LoggingConfig::from_values(Path::new("private-data"), &values)
            .expect("explicit logging config");
        assert_eq!(config.filter, "tuneweave=debug");
        assert_eq!(config.filter_source, LogFilterSource::TuneWeave);
        assert_eq!(config.format, LogFormat::Json);
        assert!(!config.to_stderr);
        assert!(config.to_file);
        assert_eq!(config.file_name, "api.jsonl");
        assert_eq!(config.retention_days, 7);
        assert_eq!(config.max_files, 9);
    }

    #[test]
    fn invalid_filters_outputs_names_bounds_and_data_collisions_are_rejected() {
        for (name, value) in [
            ("TUNEWEAVE_LOG_LEVEL", "verbose"),
            ("RUST_LOG", "[invalid"),
            ("TUNEWEAVE_LOG_FORMAT", "xml"),
            ("TUNEWEAVE_LOG_TO_FILE", "sometimes"),
            ("TUNEWEAVE_LOG_FILE", "../secret.log"),
            ("TUNEWEAVE_LOG_FILE", "CON"),
            ("TUNEWEAVE_LOG_FILE", "COM1.jsonl"),
            ("TUNEWEAVE_LOG_FILE", "file."),
            ("TUNEWEAVE_LOG_RETENTION_DAYS", "0"),
            ("TUNEWEAVE_LOG_MAX_FILES", "10001"),
        ] {
            let values = values(&[(name, value)]);
            assert!(
                LoggingConfig::from_values(Path::new("private-data"), &values).is_err(),
                "{name}={value} must fail"
            );
        }

        let disabled = values(&[
            ("TUNEWEAVE_LOG_TO_STDERR", "false"),
            ("TUNEWEAVE_LOG_TO_FILE", "false"),
        ]);
        assert!(LoggingConfig::from_values(Path::new("private-data"), &disabled).is_err());

        for directory in [
            "private-data",
            "private-data/accounts",
            "private-data/accounts/logs",
            "private-data/uni-playlists/logs",
            "private-data/other/../accounts/logs",
        ] {
            let values = values(&[("TUNEWEAVE_LOG_DIR", directory)]);
            assert!(
                LoggingConfig::from_values(Path::new("private-data"), &values).is_err(),
                "{directory} must fail"
            );
        }

        let ancestor = values(&[("TUNEWEAVE_LOG_DIR", "private-data")]);
        assert!(
            LoggingConfig::from_values(Path::new("private-data/data"), &ancestor).is_err(),
            "a log directory containing the data directory must fail"
        );
    }

    #[test]
    fn non_blocking_daily_writer_flushes_and_keeps_unrelated_files_separate() {
        let root = temporary_directory("writer");
        let directory = root.join("logs");
        fs::create_dir_all(&directory).expect("create test log directory");
        fs::write(directory.join("unrelated.txt"), b"keep").expect("write unrelated file");
        let values = values(&[
            (
                "TUNEWEAVE_LOG_DIR",
                directory.to_str().expect("UTF-8 test directory"),
            ),
            ("TUNEWEAVE_LOG_TO_STDERR", "false"),
            ("TUNEWEAVE_LOG_FILE", "test.log"),
        ]);
        let config =
            LoggingConfig::from_values(&root.join("data"), &values).expect("test logging config");
        let output = build_file_output(&config).expect("build file output");
        let FileOutput {
            writer,
            guard,
            error_counter,
            retention_warnings,
        } = output;
        assert!(retention_warnings.is_empty());
        let subscriber = tracing_subscriber::fmt()
            .without_time()
            .with_ansi(false)
            .with_writer(writer)
            .finish();
        tracing::subscriber::with_default(subscriber, || {
            info!(operation = "logging_test", "safe file event");
        });
        drop(guard);
        assert_eq!(error_counter.dropped_lines(), 0);
        let logs = fs::read_dir(&directory)
            .expect("read test log directory")
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().starts_with("test.log."))
            .collect::<Vec<_>>();
        assert_eq!(logs.len(), 1);
        let content = fs::read_to_string(logs[0].path()).expect("read flushed test log");
        assert!(content.contains("safe file event"));
        assert!(content.contains("logging_test"));
        assert_eq!(
            fs::read(directory.join("unrelated.txt")).expect("read unrelated file"),
            b"keep"
        );
        fs::remove_dir_all(root).expect("remove test directory");
    }

    fn values(entries: &[(&str, &str)]) -> BTreeMap<String, String> {
        entries
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect()
    }

    fn temporary_directory(label: &str) -> PathBuf {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock")
            .as_nanos();
        let sequence = TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        env::temp_dir().join(format!(
            "tuneweave-logging-{label}-{}-{timestamp}-{sequence}",
            std::process::id()
        ))
    }
}
