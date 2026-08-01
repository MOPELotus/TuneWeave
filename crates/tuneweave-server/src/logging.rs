use std::{
    borrow::Cow,
    collections::BTreeMap,
    env, fmt, fs,
    io::{self, ErrorKind, IsTerminal, Write},
    path::{Component, Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
        mpsc::{self, Receiver, Sender},
    },
    thread::{self, JoinHandle},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use tracing::{Event, Level, Subscriber};
use tracing_appender::non_blocking::{ErrorCounter, NonBlocking, NonBlockingBuilder, WorkerGuard};
use tracing_subscriber::{
    EnvFilter, Registry,
    fmt::{
        FmtContext,
        format::{FormatEvent, FormatFields, Writer},
        time::{ChronoLocal, FormatTime},
        writer::BoxMakeWriter,
    },
    layer::{Layer, SubscriberExt},
    registry::LookupSpan,
    util::SubscriberInitExt,
};

#[cfg(test)]
use tracing_subscriber::fmt::MakeWriter;

const DEFAULT_LOG_FILE: &str = "tuneweave.log";
const DEFAULT_RETENTION_DAYS: u32 = 14;
const DEFAULT_MAX_FILES: usize = 30;
const DEFAULT_MAX_FILE_BYTES: u64 = 16 * 1024 * 1024;
const DEFAULT_MAX_TOTAL_BYTES: u64 = 256 * 1024 * 1024;
const MAX_RETENTION_DAYS: u32 = 3_650;
const MAX_LOG_FILES: usize = 10_000;
const MIN_MAX_FILE_BYTES: u64 = 64 * 1024;
const MAX_MAX_FILE_BYTES: u64 = 4 * 1024 * 1024 * 1024;
const MAX_MAX_TOTAL_BYTES: u64 = 1024 * 1024 * 1024 * 1024;
const LOG_HEALTH_INTERVAL: Duration = Duration::from_secs(5);
const LOG_ENVIRONMENT_VARIABLES: [&str; 11] = [
    "TUNEWEAVE_LOG_LEVEL",
    "RUST_LOG",
    "TUNEWEAVE_LOG_FORMAT",
    "TUNEWEAVE_LOG_DIR",
    "TUNEWEAVE_LOG_FILE",
    "TUNEWEAVE_LOG_RETENTION_DAYS",
    "TUNEWEAVE_LOG_MAX_FILES",
    "TUNEWEAVE_LOG_MAX_FILE_BYTES",
    "TUNEWEAVE_LOG_MAX_TOTAL_BYTES",
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
    pub max_file_bytes: u64,
    pub max_total_bytes: u64,
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
        let max_file_bytes = parse_bounded_u64(
            values,
            "TUNEWEAVE_LOG_MAX_FILE_BYTES",
            DEFAULT_MAX_FILE_BYTES,
            MIN_MAX_FILE_BYTES,
            MAX_MAX_FILE_BYTES,
        )?;
        let max_total_bytes = parse_bounded_u64(
            values,
            "TUNEWEAVE_LOG_MAX_TOTAL_BYTES",
            DEFAULT_MAX_TOTAL_BYTES,
            MIN_MAX_FILE_BYTES,
            MAX_MAX_TOTAL_BYTES,
        )?;
        if max_total_bytes < max_file_bytes {
            return Err(invalid_config(
                "TUNEWEAVE_LOG_MAX_TOTAL_BYTES must be at least TUNEWEAVE_LOG_MAX_FILE_BYTES",
            ));
        }
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
            max_file_bytes,
            max_total_bytes,
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
    _health_monitor: Option<FileLogHealthMonitor>,
    error_counter: Option<ErrorCounter>,
    write_error_counter: Option<FileWriteErrorCounter>,
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
    pub fn file_write_errors(&self) -> usize {
        self.write_error_counter
            .as_ref()
            .map_or(0, FileWriteErrorCounter::total)
    }

    #[must_use]
    pub const fn file_output_active(&self) -> bool {
        self.file_guard.is_some()
    }
}

impl Drop for LoggingHandle {
    fn drop(&mut self) {
        drop(self.file_guard.take());
        drop(self._health_monitor.take());
    }
}

pub fn init_logging(
    config: &LoggingConfig,
) -> Result<LoggingHandle, Box<dyn std::error::Error + Send + Sync>> {
    let file_output = config
        .to_file
        .then(|| build_file_output(config))
        .transpose()?;
    let (file_writer, file_guard, error_counter, write_error_counter, retention_warnings) =
        match file_output {
            Some(output) => (
                Some(output.writer),
                Some(output.guard),
                Some(output.error_counter),
                Some(output.write_error_counter),
                output.retention_warnings,
            ),
            None => (None, None, None, None, Vec::new()),
        };

    install_subscriber(config, file_writer)?;

    Ok(LoggingHandle {
        _health_monitor: error_counter
            .as_ref()
            .zip(write_error_counter.as_ref())
            .map(|(queue_counter, write_counter)| {
                FileLogHealthMonitor::start(queue_counter.clone(), write_counter.clone())
            })
            .transpose()?,
        file_guard,
        error_counter,
        write_error_counter,
        retention_warnings,
    })
}

#[cfg(test)]
pub(crate) fn init_test_logging() {
    static TEST_LOGGING: std::sync::Once = std::sync::Once::new();

    TEST_LOGGING.call_once(|| {
        let values = BTreeMap::from([
            ("TUNEWEAVE_LOG_LEVEL".to_owned(), "debug".to_owned()),
            ("TUNEWEAVE_LOG_TO_FILE".to_owned(), "false".to_owned()),
        ]);
        let config = LoggingConfig::from_values(Path::new(".local/test-logging"), &values)
            .expect("test logging configuration must remain valid");
        install_test_subscriber(&config, tracing_subscriber::fmt::TestWriter::new())
            .expect("test logging subscriber must initialize once");
    });
}

struct FileLogHealthMonitor {
    stop: Option<Sender<()>>,
    thread: Option<JoinHandle<()>>,
    #[cfg(test)]
    reported_total: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}

impl FileLogHealthMonitor {
    fn start(
        queue_error_counter: ErrorCounter,
        write_error_counter: FileWriteErrorCounter,
    ) -> io::Result<Self> {
        Self::start_with_interval(
            queue_error_counter,
            write_error_counter,
            LOG_HEALTH_INTERVAL,
        )
    }

    fn start_with_interval(
        queue_error_counter: ErrorCounter,
        write_error_counter: FileWriteErrorCounter,
        interval: Duration,
    ) -> io::Result<Self> {
        let (stop, receiver) = mpsc::channel();
        #[cfg(test)]
        let reported_total = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        #[cfg(test)]
        let thread_reported_total = reported_total.clone();
        let thread = thread::Builder::new()
            .name("tuneweave-log-health".to_owned())
            .spawn(move || {
                monitor_file_log_health(
                    &receiver,
                    &queue_error_counter,
                    &write_error_counter,
                    interval,
                    #[cfg(test)]
                    &thread_reported_total,
                );
            })?;
        Ok(Self {
            stop: Some(stop),
            thread: Some(thread),
            #[cfg(test)]
            reported_total,
        })
    }

    #[cfg(test)]
    fn reported_total(&self) -> usize {
        self.reported_total
            .load(std::sync::atomic::Ordering::Acquire)
    }
}

impl Drop for FileLogHealthMonitor {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn monitor_file_log_health(
    receiver: &Receiver<()>,
    queue_error_counter: &ErrorCounter,
    write_error_counter: &FileWriteErrorCounter,
    interval: Duration,
    #[cfg(test)] reported_total: &std::sync::atomic::AtomicUsize,
) {
    let mut previous_queue_errors = 0;
    let mut previous_write_errors = 0;
    loop {
        let stopping = match receiver.recv_timeout(interval) {
            Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => true,
            Err(mpsc::RecvTimeoutError::Timeout) => false,
        };
        let queue_errors = queue_error_counter.dropped_lines();
        let write_errors = write_error_counter.total();
        let queue_delta = queue_errors.saturating_sub(previous_queue_errors);
        let write_delta = write_errors.saturating_sub(previous_write_errors);
        if queue_delta > 0 || write_delta > 0 {
            previous_queue_errors = queue_errors;
            previous_write_errors = write_errors;
            #[cfg(test)]
            reported_total.store(
                queue_errors.saturating_add(write_errors),
                std::sync::atomic::Ordering::Release,
            );
            eprintln!(
                "ERROR tuneweave: non-blocking file logger encountered errors \
                 dropped_lines_delta={queue_delta} dropped_lines_total={queue_errors} \
                 file_write_errors_delta={write_delta} file_write_errors_total={write_errors}"
            );
        }
        if stopping {
            break;
        }
    }
}

#[derive(Clone, Debug, Default)]
struct FileWriteErrorCounter(Arc<AtomicUsize>);

impl FileWriteErrorCounter {
    fn record(&self) {
        let _ = self
            .0
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
                Some(current.saturating_add(1))
            });
    }

    fn total(&self) -> usize {
        self.0.load(Ordering::Acquire)
    }
}

struct MonitoredWriter<W> {
    inner: W,
    error_counter: FileWriteErrorCounter,
}

impl<W> MonitoredWriter<W> {
    fn new(inner: W, error_counter: FileWriteErrorCounter) -> Self {
        Self {
            inner,
            error_counter,
        }
    }
}

impl<W: io::Write> io::Write for MonitoredWriter<W> {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.inner.write(buffer).inspect_err(|_| {
            self.error_counter.record();
        })
    }

    fn write_all(&mut self, buffer: &[u8]) -> io::Result<()> {
        self.inner.write_all(buffer).inspect_err(|_| {
            self.error_counter.record();
        })
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush().inspect_err(|_| {
            self.error_counter.record();
        })
    }
}

#[derive(Clone, Debug)]
struct HumanEventFormatter {
    timer: ChronoLocal,
}

impl Default for HumanEventFormatter {
    fn default() -> Self {
        Self {
            timer: ChronoLocal::new("%Y-%m-%d %H:%M:%S".to_owned()),
        }
    }
}

impl<S, N> FormatEvent<S, N> for HumanEventFormatter
where
    S: Subscriber + for<'lookup> LookupSpan<'lookup>,
    N: for<'writer> FormatFields<'writer> + 'static,
{
    fn format_event(
        &self,
        context: &FmtContext<'_, S, N>,
        mut writer: Writer<'_>,
        event: &Event<'_>,
    ) -> fmt::Result {
        let metadata = event.metadata();
        let ansi = writer.has_ansi_escapes();
        if ansi {
            write!(
                writer,
                "\u{1b}[36m[{}]\u{1b}[0m{}[{}]\u{1b}[0m\u{1b}[90m[",
                human_target(metadata.target()),
                level_color(metadata.level()),
                metadata.level(),
            )?;
        } else {
            write!(
                writer,
                "[{}][{}][",
                human_target(metadata.target()),
                metadata.level()
            )?;
        }
        self.timer.format_time(&mut writer)?;
        if ansi {
            writer.write_str("]\u{1b}[0m ")?;
        } else {
            writer.write_str("] ")?;
        }
        context
            .field_format()
            .format_fields(writer.by_ref(), event)?;
        writeln!(writer)
    }
}

fn level_color(level: &Level) -> &'static str {
    match *level {
        Level::TRACE => "\u{1b}[35m",
        Level::DEBUG => "\u{1b}[34m",
        Level::INFO => "\u{1b}[32m",
        Level::WARN => "\u{1b}[33m",
        Level::ERROR => "\u{1b}[31m",
    }
}

fn human_target(target: &str) -> Cow<'_, str> {
    if target == "tuneweave" {
        return Cow::Borrowed("TuneWeave");
    }
    let Some(target) = target.strip_prefix("tuneweave_") else {
        return Cow::Borrowed(target);
    };
    let component = target
        .split_once("::")
        .map_or(target, |(component, _)| component);
    let component = component.replace('_', "-");
    Cow::Owned(format!("TuneWeave/{component}"))
}

fn install_subscriber(
    config: &LoggingConfig,
    file_writer: Option<NonBlocking>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    type OutputLayer = Box<dyn Layer<Registry> + Send + Sync + 'static>;

    let mut output_layers = Vec::<OutputLayer>::with_capacity(2);
    match config.format {
        LogFormat::Human => {
            if config.to_stderr {
                output_layers.push(Box::new(
                    tracing_subscriber::fmt::layer()
                        .event_format(HumanEventFormatter::default())
                        .with_writer(BoxMakeWriter::new(std::io::stderr))
                        .with_ansi(std::io::stderr().is_terminal()),
                ));
            }
            if let Some(writer) = file_writer {
                output_layers.push(Box::new(
                    tracing_subscriber::fmt::layer()
                        .event_format(HumanEventFormatter::default())
                        .with_writer(BoxMakeWriter::new(writer))
                        .with_ansi(false),
                ));
            }
        }
        LogFormat::Json => {
            if config.to_stderr {
                output_layers.push(Box::new(
                    tracing_subscriber::fmt::layer()
                        .json()
                        .with_writer(BoxMakeWriter::new(std::io::stderr))
                        .with_ansi(false),
                ));
            }
            if let Some(writer) = file_writer {
                output_layers.push(Box::new(
                    tracing_subscriber::fmt::layer()
                        .json()
                        .with_writer(BoxMakeWriter::new(writer))
                        .with_ansi(false),
                ));
            }
        }
    }
    debug_assert!(!output_layers.is_empty());
    tracing_subscriber::registry()
        .with(output_layers)
        .with(config.env_filter()?)
        .try_init()?;
    Ok(())
}

#[cfg(test)]
fn install_test_subscriber<W>(
    config: &LoggingConfig,
    writer: W,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>>
where
    W: for<'writer> MakeWriter<'writer> + Send + Sync + 'static,
{
    match config.format {
        LogFormat::Human => tracing_subscriber::registry()
            .with(
                tracing_subscriber::fmt::layer()
                    .with_writer(writer)
                    .event_format(HumanEventFormatter::default())
                    .with_ansi(false)
                    .with_filter(config.env_filter()?),
            )
            .try_init()?,
        LogFormat::Json => tracing_subscriber::fmt()
            .with_env_filter(config.env_filter()?)
            .with_writer(writer)
            .json()
            .with_ansi(false)
            .try_init()?,
    }
    Ok(())
}

struct FileOutput {
    writer: NonBlocking,
    guard: WorkerGuard,
    error_counter: ErrorCounter,
    write_error_counter: FileWriteErrorCounter,
    retention_warnings: Vec<RetentionWarning>,
}

fn build_file_output(config: &LoggingConfig) -> io::Result<FileOutput> {
    fs::create_dir_all(&config.data_directory)?;
    fs::create_dir_all(&config.directory)?;
    validate_canonical_directory_separation(&config.data_directory, &config.directory)?;
    let retention_warnings = prune_expired_files(config)?;
    let appender = BoundedRollingFile::new(config)?;
    let write_error_counter = FileWriteErrorCounter::default();
    let (writer, guard) = NonBlockingBuilder::default()
        .lossy(true)
        .thread_name("tuneweave-log-writer")
        .finish(MonitoredWriter::new(appender, write_error_counter.clone()));
    let error_counter = writer.error_counter();
    Ok(FileOutput {
        writer,
        guard,
        error_counter,
        write_error_counter,
        retention_warnings,
    })
}

struct BoundedRollingFile {
    directory: PathBuf,
    file_name: String,
    max_files: usize,
    max_file_bytes: u64,
    max_total_bytes: u64,
    current: Option<fs::File>,
    current_size: u64,
}

impl BoundedRollingFile {
    fn new(config: &LoggingConfig) -> io::Result<Self> {
        let mut writer = Self {
            directory: config.directory.clone(),
            file_name: config.file_name.clone(),
            max_files: config.max_files,
            max_file_bytes: config.max_file_bytes,
            max_total_bytes: config.max_total_bytes,
            current: None,
            current_size: 0,
        };
        writer.open_new_file()?;
        Ok(writer)
    }

    fn prepare_write(&mut self, length: usize) -> io::Result<()> {
        let length = u64::try_from(length)
            .map_err(|_| invalid_config("log event length is not supported on this platform"))?;
        if length > self.max_file_bytes {
            return Err(io::Error::new(
                ErrorKind::InvalidData,
                "one log event exceeds TUNEWEAVE_LOG_MAX_FILE_BYTES",
            ));
        }
        let next_size = self
            .current_size
            .checked_add(length)
            .ok_or_else(|| io::Error::other("log file size overflowed"))?;
        if self.current.is_none() || next_size > self.max_file_bytes {
            self.open_new_file()?;
        }
        Ok(())
    }

    fn open_new_file(&mut self) -> io::Result<()> {
        if let Some(mut current) = self.current.take() {
            current.flush()?;
        }
        self.current_size = 0;
        prune_for_new_log_file(
            &self.directory,
            &self.file_name,
            self.max_files,
            self.max_file_bytes,
            self.max_total_bytes,
        )?;
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| io::Error::other("system clock is before the Unix epoch"))?
            .as_millis();
        for sequence in 0_u16..10_000 {
            let name = format!(
                "{}.{timestamp}.{}.{sequence:04}",
                self.file_name,
                std::process::id()
            );
            let path = self.directory.join(name);
            match fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(path)
            {
                Ok(file) => {
                    self.current = Some(file);
                    return Ok(());
                }
                Err(error) if error.kind() == ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error),
            }
        }
        Err(io::Error::new(
            ErrorKind::AlreadyExists,
            "could not allocate a unique rolling log file name",
        ))
    }

    fn refresh_current_size(&mut self) {
        if let Some(current) = &self.current
            && let Ok(metadata) = current.metadata()
        {
            self.current_size = metadata.len();
        }
    }
}

impl Write for BoundedRollingFile {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.prepare_write(buffer.len())?;
        let result = self
            .current
            .as_mut()
            .ok_or_else(|| io::Error::other("rolling log file is not open"))?
            .write(buffer);
        match result {
            Ok(written) => {
                self.current_size = self
                    .current_size
                    .saturating_add(u64::try_from(written).unwrap_or(u64::MAX));
                Ok(written)
            }
            Err(error) => {
                self.refresh_current_size();
                Err(error)
            }
        }
    }

    fn write_all(&mut self, buffer: &[u8]) -> io::Result<()> {
        self.prepare_write(buffer.len())?;
        let result = self
            .current
            .as_mut()
            .ok_or_else(|| io::Error::other("rolling log file is not open"))?
            .write_all(buffer);
        match result {
            Ok(()) => {
                self.current_size = self
                    .current_size
                    .saturating_add(u64::try_from(buffer.len()).unwrap_or(u64::MAX));
                Ok(())
            }
            Err(error) => {
                self.refresh_current_size();
                Err(error)
            }
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        self.current.as_mut().map_or(Ok(()), Write::flush)
    }
}

struct ManagedLogFile {
    path: PathBuf,
    file_name: String,
    size: u64,
    modified: SystemTime,
}

fn prune_for_new_log_file(
    directory: &Path,
    file_name: &str,
    max_files: usize,
    max_file_bytes: u64,
    max_total_bytes: u64,
) -> io::Result<()> {
    let mut files = managed_log_files(directory, file_name)?;
    let mut total_bytes = files
        .iter()
        .fold(0_u64, |total, file| total.saturating_add(file.size));
    let reserved_total = max_total_bytes.saturating_sub(max_file_bytes);
    while files.len() >= max_files || total_bytes > reserved_total {
        let oldest = files.remove(0);
        fs::remove_file(&oldest.path)?;
        total_bytes = total_bytes.saturating_sub(oldest.size);
    }
    Ok(())
}

fn managed_log_files(directory: &Path, file_name: &str) -> io::Result<Vec<ManagedLogFile>> {
    let prefix = format!("{file_name}.");
    let mut files = Vec::new();
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if !name.starts_with(&prefix) {
            continue;
        }
        let metadata = fs::symlink_metadata(entry.path())?;
        if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
            continue;
        }
        files.push(ManagedLogFile {
            path: entry.path(),
            file_name: name,
            size: metadata.len(),
            modified: metadata.modified().unwrap_or(UNIX_EPOCH),
        });
    }
    files.sort_by(|left, right| {
        left.modified
            .cmp(&right.modified)
            .then_with(|| left.file_name.cmp(&right.file_name))
    });
    Ok(files)
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

fn parse_bounded_u64(
    values: &BTreeMap<String, String>,
    name: &str,
    default: u64,
    minimum: u64,
    maximum: u64,
) -> io::Result<u64> {
    let Some(value) = values.get(name) else {
        return Ok(default);
    };
    value
        .trim()
        .parse::<u64>()
        .ok()
        .filter(|value| (*value >= minimum) && (*value <= maximum))
        .ok_or_else(|| {
            invalid_config(format!(
                "{name} must be an integer between {minimum} and {maximum}"
            ))
        })
}

fn invalid_config(message: impl Into<String>) -> io::Error {
    io::Error::new(ErrorKind::InvalidInput, message.into())
}

#[cfg(test)]
mod tests {
    use std::{
        io::Write,
        sync::{
            Arc, Mutex,
            atomic::{AtomicU64, Ordering},
        },
        time::{SystemTime, UNIX_EPOCH},
    };

    use tracing::info;

    use super::*;

    static TEST_SEQUENCE: AtomicU64 = AtomicU64::new(1);

    struct CaptureWriter(Arc<Mutex<Vec<u8>>>);

    impl Write for CaptureWriter {
        fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
            self.0.lock().expect("capture writer lock").write(buffer)
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn human_formatter_uses_backend_style_local_timestamp_and_clean_target() {
        let output = Arc::new(Mutex::new(Vec::new()));
        let writer_output = output.clone();
        let subscriber = tracing_subscriber::registry().with(
            tracing_subscriber::fmt::layer()
                .with_writer(move || CaptureWriter(writer_output.clone()))
                .event_format(HumanEventFormatter::default())
                .with_ansi(false),
        );
        tracing::subscriber::with_default(subscriber, || {
            info!(target: "tuneweave", answer = 42, "ready");
        });
        let output = String::from_utf8(output.lock().expect("captured output lock").clone())
            .expect("UTF-8 human log output");
        let line = output.trim_end();
        let timestamp = line
            .strip_prefix("[TuneWeave][INFO][")
            .and_then(|line| line.split_once("] ready answer=42"))
            .map(|(timestamp, _)| timestamp)
            .expect("backend-style log line");
        let (date, time) = timestamp.split_once(' ').expect("date and time sections");
        assert_eq!(date.len(), 10);
        assert_eq!(time.len(), 8);
        assert_eq!(date.as_bytes()[4], b'-');
        assert_eq!(date.as_bytes()[7], b'-');
        assert_eq!(time.as_bytes()[2], b':');
        assert_eq!(time.as_bytes()[5], b':');
        assert!(!output.contains("\u{1b}["));
    }

    #[test]
    fn human_formatter_colors_levels_timestamps_and_targets_when_enabled() {
        let output = Arc::new(Mutex::new(Vec::new()));
        let writer_output = output.clone();
        let subscriber = tracing_subscriber::registry().with(
            tracing_subscriber::fmt::layer()
                .with_writer(move || CaptureWriter(writer_output.clone()))
                .event_format(HumanEventFormatter::default())
                .with_ansi(true),
        );
        tracing::subscriber::with_default(subscriber, || {
            tracing::warn!(target: "tuneweave_server::http", "attention");
        });
        let output = String::from_utf8(output.lock().expect("captured output lock").clone())
            .expect("UTF-8 colored log output");
        assert!(output.contains("\u{1b}[36m[TuneWeave/server]\u{1b}[0m"));
        assert!(output.contains("\u{1b}[33m[WARN]\u{1b}[0m"));
        assert!(output.contains("\u{1b}[90m["));
        assert!(output.contains("\u{1b}[0m attention"));
    }

    #[test]
    fn human_output_filter_keeps_matching_tuneweave_events() {
        let output = Arc::new(Mutex::new(Vec::new()));
        let writer_output = output.clone();
        let subscriber = tracing_subscriber::registry().with(
            tracing_subscriber::fmt::layer()
                .with_writer(move || CaptureWriter(writer_output.clone()))
                .event_format(HumanEventFormatter::default())
                .with_ansi(false)
                .with_filter(EnvFilter::new("tuneweave=info")),
        );
        tracing::subscriber::with_default(subscriber, || {
            info!(target: "tuneweave", "visible");
            info!(target: "unrelated", "hidden");
        });
        let output = String::from_utf8(output.lock().expect("captured output lock").clone())
            .expect("UTF-8 filtered log output");
        assert!(output.contains("[TuneWeave][INFO]["));
        assert!(output.contains("] visible"));
        assert!(!output.contains("hidden"));
    }

    #[test]
    fn test_logging_initialization_is_idempotent() {
        init_test_logging();
        init_test_logging();
    }

    #[test]
    fn defaults_enable_human_console_and_bounded_file_output() {
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
        assert_eq!(config.max_file_bytes, DEFAULT_MAX_FILE_BYTES);
        assert_eq!(config.max_total_bytes, DEFAULT_MAX_TOTAL_BYTES);
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
            ("TUNEWEAVE_LOG_MAX_FILE_BYTES", "1048576"),
            ("TUNEWEAVE_LOG_MAX_TOTAL_BYTES", "4194304"),
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
        assert_eq!(config.max_file_bytes, 1_048_576);
        assert_eq!(config.max_total_bytes, 4_194_304);
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
            ("TUNEWEAVE_LOG_MAX_FILE_BYTES", "65535"),
            ("TUNEWEAVE_LOG_MAX_TOTAL_BYTES", "1099511627777"),
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

        let inverted_sizes = values(&[
            ("TUNEWEAVE_LOG_MAX_FILE_BYTES", "4194304"),
            ("TUNEWEAVE_LOG_MAX_TOTAL_BYTES", "1048576"),
        ]);
        assert!(LoggingConfig::from_values(Path::new("private-data"), &inverted_sizes).is_err());

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
    fn non_blocking_bounded_writer_flushes_and_keeps_unrelated_files_separate() {
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
            write_error_counter,
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
        assert_eq!(write_error_counter.total(), 0);
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

    #[test]
    fn bounded_writer_rolls_by_size_and_prunes_only_its_oldest_files() {
        let root = temporary_directory("bounded");
        let directory = root.join("logs");
        fs::create_dir_all(&directory).expect("create bounded log directory");
        fs::write(directory.join("unrelated.txt"), b"keep").expect("write unrelated file");
        let values = values(&[
            (
                "TUNEWEAVE_LOG_DIR",
                directory.to_str().expect("UTF-8 test directory"),
            ),
            ("TUNEWEAVE_LOG_TO_STDERR", "false"),
            ("TUNEWEAVE_LOG_FILE", "bounded.log"),
        ]);
        let mut config =
            LoggingConfig::from_values(&root.join("data"), &values).expect("logging config");
        config.max_files = 2;
        config.max_file_bytes = 16;
        config.max_total_bytes = 32;

        let mut writer = BoundedRollingFile::new(&config).expect("open bounded writer");
        writer.write_all(b"first-record").expect("write first file");
        writer
            .write_all(b"second-record")
            .expect("roll to second file");
        writer
            .write_all(b"third-record")
            .expect("prune and roll to third file");
        let oversized = writer
            .write_all(b"one-record-is-larger-than-the-limit")
            .expect_err("oversized event must fail");
        assert_eq!(oversized.kind(), ErrorKind::InvalidData);
        writer.flush().expect("flush bounded writer");
        drop(writer);

        let logs = managed_log_files(&directory, "bounded.log").expect("list bounded logs");
        assert_eq!(logs.len(), 2);
        assert!(logs.iter().map(|file| file.size).sum::<u64>() <= 32);
        let contents = logs
            .iter()
            .map(|file| fs::read_to_string(&file.path).expect("read bounded log"))
            .collect::<String>();
        assert!(!contents.contains("first-record"));
        assert!(contents.contains("second-record"));
        assert!(contents.contains("third-record"));
        assert_eq!(
            fs::read(directory.join("unrelated.txt")).expect("read unrelated file"),
            b"keep"
        );
        fs::remove_dir_all(root).expect("remove bounded test directory");
    }

    #[test]
    fn file_health_monitor_observes_background_writer_failures() {
        struct FailingWriter;

        impl Write for FailingWriter {
            fn write(&mut self, _buffer: &[u8]) -> io::Result<usize> {
                Err(io::Error::other("intentional test failure"))
            }

            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }

        let write_error_counter = FileWriteErrorCounter::default();
        let monitored_writer = MonitoredWriter::new(FailingWriter, write_error_counter.clone());
        let (mut writer, guard) = NonBlockingBuilder::default()
            .lossy(true)
            .finish(monitored_writer);
        let queue_error_counter = writer.error_counter();
        let monitor = FileLogHealthMonitor::start_with_interval(
            queue_error_counter.clone(),
            write_error_counter.clone(),
            Duration::from_secs(60),
        )
        .expect("start file log health monitor");
        writer
            .write_all(b"background writer failure\n")
            .expect("enqueue test event");
        drop(guard);
        assert_eq!(queue_error_counter.dropped_lines(), 0);
        assert_eq!(write_error_counter.total(), 1);
        assert_eq!(monitor.reported_total(), 0);
        let reported_total = monitor.reported_total.clone();
        drop(monitor);
        assert_eq!(reported_total.load(std::sync::atomic::Ordering::Acquire), 1);
        drop(writer);
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
