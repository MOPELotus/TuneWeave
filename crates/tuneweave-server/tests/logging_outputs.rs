use std::{
    fs,
    net::{SocketAddr, TcpListener, TcpStream},
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const STARTUP_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Clone, Copy)]
struct OutputMode {
    name: &'static str,
    stderr: bool,
    file: bool,
}

#[test]
fn binary_emits_human_logs_for_every_supported_output_mode() {
    for mode in [
        OutputMode {
            name: "console-only",
            stderr: true,
            file: false,
        },
        OutputMode {
            name: "file-only",
            stderr: false,
            file: true,
        },
        OutputMode {
            name: "dual-output",
            stderr: true,
            file: true,
        },
    ] {
        verify_output_mode(mode);
    }
}

fn verify_output_mode(mode: OutputMode) {
    let root = temporary_directory(mode.name);
    fs::create_dir_all(&root).expect("create logging smoke directory");
    let address = available_address();
    let mut child = Command::new(env!("CARGO_BIN_EXE_tuneweave"))
        .current_dir(&root)
        .env("TUNEWEAVE_BIND", address.to_string())
        .env("TUNEWEAVE_LOG_FORMAT", "human")
        .env("TUNEWEAVE_LOG_LEVEL", "info")
        .env(
            "TUNEWEAVE_LOG_TO_STDERR",
            if mode.stderr { "true" } else { "false" },
        )
        .env(
            "TUNEWEAVE_LOG_TO_FILE",
            if mode.file { "true" } else { "false" },
        )
        .env("TUNEWEAVE_DATA_DIR", "data")
        .env("TUNEWEAVE_LOG_DIR", "logs")
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start TuneWeave logging smoke process");

    let ready = wait_for_listener(&mut child, address);
    let file_output = if ready && mode.file {
        wait_for_file_output(&root.join("logs"))
    } else {
        String::new()
    };
    let output = stop_and_collect(child);

    assert!(
        ready,
        "{} process did not become ready: {}",
        mode.name,
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(root.join("data").is_dir(), "default data directory");
    assert_eq!(root.join("logs").is_dir(), mode.file);

    let stderr = String::from_utf8(output.stderr).expect("UTF-8 stderr logs");
    assert_eq!(stderr.contains("[TuneWeave][INFO]["), mode.stderr);
    assert_eq!(file_output.contains("[TuneWeave][INFO]["), mode.file);
    assert!(!file_output.contains("\u{1b}["));
    fs::remove_dir_all(root).expect("remove logging smoke directory");
}

fn available_address() -> SocketAddr {
    TcpListener::bind("127.0.0.1:0")
        .and_then(|listener| listener.local_addr())
        .expect("allocate local smoke-test address")
}

fn wait_for_listener(child: &mut std::process::Child, address: SocketAddr) -> bool {
    let deadline = Instant::now() + STARTUP_TIMEOUT;
    while Instant::now() < deadline {
        if child.try_wait().expect("inspect smoke process").is_some() {
            return false;
        }
        if TcpStream::connect_timeout(&address, Duration::from_millis(100)).is_ok() {
            thread::sleep(Duration::from_millis(200));
            return true;
        }
        thread::sleep(Duration::from_millis(50));
    }
    false
}

fn wait_for_file_output(directory: &Path) -> String {
    let deadline = Instant::now() + STARTUP_TIMEOUT;
    while Instant::now() < deadline {
        if let Ok(entries) = fs::read_dir(directory) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path
                    .file_name()
                    .is_some_and(|name| name.to_string_lossy().starts_with("tuneweave.log."))
                {
                    let content = fs::read_to_string(path).expect("read smoke log file");
                    if content.contains("[TuneWeave][INFO][") {
                        return content;
                    }
                }
            }
        }
        thread::sleep(Duration::from_millis(50));
    }
    String::new()
}

fn stop_and_collect(mut child: std::process::Child) -> Output {
    if child.try_wait().expect("inspect smoke process").is_none() {
        child.kill().expect("stop smoke process");
    }
    child
        .wait_with_output()
        .expect("collect smoke process output")
}

fn temporary_directory(name: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock after Unix epoch")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "tuneweave-logging-{name}-{}-{nonce}",
        std::process::id()
    ))
}
