use super::*;
use std::{
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

const WORKER_DIRECTORY: &str = "TUNEWEAVE_STORE_TEST_WORKER_DIRECTORY";
const WORKER_MODE: &str = "TUNEWEAVE_STORE_TEST_WORKER_MODE";
const WORKER_ID: &str = "TUNEWEAVE_STORE_TEST_WORKER_ID";

struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "tuneweave-store-process-{}-{}",
            process::id(),
            CREDENTIAL_FILE_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    fn store(&self) -> FileAccountCredentialStore {
        FileAccountCredentialStore::new(self.0.join("store"))
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct Worker {
    child: Child,
    directory: PathBuf,
    id: String,
}
impl Worker {
    fn spawn(directory: &Directory, mode: &str, id: impl ToString) -> Self {
        let id = id.to_string();
        let child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "credential_store::process_tests::store_worker",
                "--nocapture",
            ])
            .env(WORKER_DIRECTORY, &directory.0)
            .env(WORKER_MODE, mode)
            .env(WORKER_ID, &id)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        Self {
            child,
            directory: directory.0.clone(),
            id,
        }
    }
    fn path(&self, name: &str) -> PathBuf {
        self.directory.join(format!("{}-{name}", self.id))
    }
    fn wait_marker(&mut self, name: &str) {
        let deadline = Instant::now() + Duration::from_secs(15);
        while !self.path(name).exists() {
            assert!(
                self.child.try_wait().unwrap().is_none(),
                "worker exited before {name}"
            );
            assert!(Instant::now() < deadline, "worker did not reach {name}");
            thread::sleep(Duration::from_millis(2));
        }
    }
    fn start(&self) {
        fs::write(self.path("start"), []).unwrap();
    }
    fn finish(&mut self) -> String {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                assert!(status.success(), "worker failed: {status}");
                return fs::read_to_string(self.path("result")).unwrap();
            }
            assert!(Instant::now() < deadline, "worker did not exit");
            thread::sleep(Duration::from_millis(2));
        }
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn credential(account: &str, secret: &str) -> StoredAccountCredential {
    StoredAccountCredential::new(Platform::Migu, account, "synthetic-session", secret).unwrap()
}
fn wait_file(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(15);
    while !path.exists() {
        assert!(
            Instant::now() < deadline,
            "parent did not release test worker"
        );
        thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn store_worker() {
    let Some(directory) = std::env::var_os(WORKER_DIRECTORY) else {
        return;
    };
    let directory = PathBuf::from(directory);
    let mode = std::env::var(WORKER_MODE).unwrap();
    let id = std::env::var(WORKER_ID).unwrap();
    let marker = |name: &str| directory.join(format!("{id}-{name}"));
    let store = FileAccountCredentialStore::new(directory.join("store"));
    fs::write(marker("ready"), []).unwrap();
    wait_file(&marker("start"));
    let mode = if let Some(mode) = mode.strip_prefix("blocked-") {
        let file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(store.root.join(LOCK_FILE))
            .unwrap();
        assert!(matches!(
            fs4::FileExt::try_lock(&file),
            Err(fs4::TryLockError::WouldBlock)
        ));
        fs::write(marker("blocked"), []).unwrap();
        mode
    } else {
        &mode
    };
    let result = match mode {
        "insert" => store
            .insert_if_absent(&credential("a", &id))
            .unwrap()
            .to_string(),
        "cas" => store
            .compare_exchange(&credential("a", "initial"), Some(&credential("a", &id)))
            .unwrap()
            .to_string(),
        "cas-remove" => store
            .compare_exchange(&credential("a", "initial"), None)
            .unwrap()
            .to_string(),
        "put" => {
            store.put(&credential("a", &id)).unwrap();
            "put".into()
        }
        "remove" => store.remove(Platform::Migu, "a").unwrap().to_string(),
        "load" => serde_json::to_string(&store.load_platform(Platform::Migu).unwrap()).unwrap(),
        "hold" => {
            let _guard = store.lock().unwrap();
            fs::write(marker("held"), []).unwrap();
            wait_file(&marker("release"));
            "released".into()
        }
        _ => panic!("unknown worker mode"),
    };
    fs::write(marker("result"), result).unwrap();
}

#[test]
fn first_writes_have_one_winner_across_processes_and_preserve_other_accounts() {
    let directory = Directory::new();
    let store = directory.store();
    let other = credential("b", "untouched");
    store.put(&other).unwrap();
    let mut workers: Vec<_> = (0..8)
        .map(|id| Worker::spawn(&directory, "insert", id))
        .collect();
    for worker in &mut workers {
        worker.wait_marker("ready");
    }
    for worker in &workers {
        worker.start();
    }
    let winners: Vec<_> = workers
        .iter_mut()
        .filter_map(|worker| (worker.finish() == "true").then(|| worker.id.clone()))
        .collect();
    assert_eq!(winners.len(), 1);
    assert_eq!(
        store.load_platform(Platform::Migu).unwrap(),
        vec![credential("a", &winners[0]), other]
    );
    assert!(!store.insert_if_absent(&credential("a", "late")).unwrap());
    let same_alias_other_platform =
        StoredAccountCredential::new(Platform::Soda, "a", "synthetic-session", "other-platform")
            .unwrap();
    assert!(store.insert_if_absent(&same_alias_other_platform).unwrap());
    assert_eq!(
        store.load_platform(Platform::Soda).unwrap(),
        vec![same_alias_other_platform]
    );
}

#[test]
fn refresh_and_logout_have_one_winner_across_processes_without_revival() {
    for mixed_logout in [false, true] {
        let directory = Directory::new();
        let store = directory.store();
        store.put(&credential("a", "initial")).unwrap();
        let mut workers: Vec<_> = (0..8)
            .map(|id| {
                Worker::spawn(
                    &directory,
                    if mixed_logout && id % 2 == 0 {
                        "cas-remove"
                    } else {
                        "cas"
                    },
                    id,
                )
            })
            .collect();
        for worker in &mut workers {
            worker.wait_marker("ready");
        }
        for worker in &workers {
            worker.start();
        }
        let winners: Vec<_> = workers
            .iter_mut()
            .filter_map(|worker| (worker.finish() == "true").then(|| worker.id.clone()))
            .collect();
        assert_eq!(winners.len(), 1);
        let winner: usize = winners[0].parse().unwrap();
        let expected = if mixed_logout && winner % 2 == 0 {
            vec![]
        } else {
            vec![credential("a", &winners[0])]
        };
        assert_eq!(store.load_platform(Platform::Migu).unwrap(), expected);
        assert!(
            !store
                .compare_exchange(&credential("a", "initial"), Some(&credential("a", "late")))
                .unwrap()
        );
    }
}

#[test]
fn all_store_operations_wait_for_the_same_process_lock() {
    for mode in ["insert", "cas", "cas-remove", "put", "remove", "load"] {
        let directory = Directory::new();
        let store = directory.store();
        store.put(&credential("a", "initial")).unwrap();
        let mut worker = Worker::spawn(&directory, &format!("blocked-{mode}"), "worker");
        worker.wait_marker("ready");
        let guard = store.lock().unwrap();
        worker.start();
        worker.wait_marker("blocked");
        // The child has confirmed this exact OS lock is held before calling the operation.
        let deadline = Instant::now() + Duration::from_millis(100);
        while Instant::now() < deadline {
            assert!(
                worker.child.try_wait().unwrap().is_none(),
                "{mode} ignored the store lock"
            );
            assert!(
                !worker.path("result").exists(),
                "{mode} completed under another process's lock"
            );
            thread::sleep(Duration::from_millis(2));
        }
        drop(guard);
        let result = worker.finish();
        match mode {
            "insert" => assert_eq!(result, "false"),
            "cas" | "cas-remove" | "remove" => assert_eq!(result, "true"),
            "put" => assert_eq!(result, "put"),
            "load" => assert_eq!(
                serde_json::from_str::<Vec<StoredAccountCredential>>(&result).unwrap(),
                vec![credential("a", "initial")]
            ),
            _ => unreachable!(),
        }
    }
}

#[test]
fn process_exit_releases_lock_without_deleting_it_or_losing_credentials() {
    let directory = Directory::new();
    let store = directory.store();
    store.put(&credential("a", "initial")).unwrap();
    let mut holder = Worker::spawn(&directory, "hold", "holder");
    holder.wait_marker("ready");
    holder.start();
    holder.wait_marker("held");
    let mut next = Worker::spawn(&directory, "blocked-cas", "successor");
    next.wait_marker("ready");
    next.start();
    next.wait_marker("blocked");
    holder.child.kill().unwrap();
    holder.child.wait().unwrap();
    assert_eq!(next.finish(), "true");
    assert_eq!(
        store.load_platform(Platform::Migu).unwrap(),
        vec![credential("a", "successor")]
    );
    assert_eq!(fs::metadata(store.root.join(LOCK_FILE)).unwrap().len(), 0);
    assert!(store.remove(Platform::Migu, "a").unwrap());
    assert!(store.root.join(LOCK_FILE).is_file());
}

#[test]
fn clock_rollback_and_abandoned_temporary_files_cannot_restore_old_credentials() {
    let directory = Directory::new();
    let store = directory.store();
    store.put(&credential("a", "initial")).unwrap();
    let account = store.account_dir(Platform::Migu, "a");
    let old = fs::read_dir(&account)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let future = format!("{:039}-0000000001-0000000000000001", 10_u128.pow(30));
    let future_path = account.join(format!("{future}.json"));
    fs::rename(&old, &future_path).unwrap();
    let old_bytes = fs::read(&future_path).unwrap();
    fs::write(account.join(format!("{future}.tmp")), b"incomplete").unwrap();
    store.put(&credential("a", "newer")).unwrap();
    let published = fs::read_dir(&account)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect::<Vec<_>>();
    assert_eq!(published.len(), 1);
    assert!(published[0].file_name() > future_path.file_name());
    // Model termination after publish but before cleanup: an old generation remains.
    fs::write(&future_path, old_bytes).unwrap();
    assert_eq!(
        directory.store().load_platform(Platform::Migu).unwrap(),
        vec![credential("a", "newer")]
    );
    assert!(
        store
            .compare_exchange(&credential("a", "newer"), Some(&credential("a", "latest")))
            .unwrap()
    );
    assert_eq!(fs::read_dir(&account).unwrap().count(), 1);
    assert_eq!(
        directory.store().load_platform(Platform::Migu).unwrap(),
        vec![credential("a", "latest")]
    );
}

#[test]
fn invalid_lock_files_fail_without_rewriting_the_target() {
    for non_regular in [false, true] {
        let directory = Directory::new();
        let store = directory.store();
        fs::create_dir_all(&store.root).unwrap();
        let lock = store.root.join(LOCK_FILE);
        if non_regular {
            fs::create_dir(&lock).unwrap();
        } else {
            fs::write(&lock, b"unrelated contents").unwrap();
        }
        assert_eq!(
            store
                .put(&credential("a", "must-not-appear"))
                .unwrap_err()
                .code,
            ErrorCode::InternalError
        );
        assert!(!store.platform_dir(Platform::Migu).exists());
        if !non_regular {
            assert_eq!(fs::read(lock).unwrap(), b"unrelated contents");
        }
    }
}

#[cfg(unix)]
#[test]
fn lock_rejects_symlinks_and_hardlinks_and_keeps_private_stable_permissions() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
    for hard_link in [false, true] {
        let directory = Directory::new();
        let store = directory.store();
        fs::create_dir_all(&store.root).unwrap();
        let target = directory.0.join("target");
        fs::write(&target, []).unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(0o644)).unwrap();
        let lock = store.root.join(LOCK_FILE);
        if hard_link {
            fs::hard_link(&target, &lock).unwrap();
        } else {
            symlink(&target, &lock).unwrap();
        }
        assert!(store.load_platform(Platform::Migu).is_err());
        assert_eq!(
            fs::metadata(&target).unwrap().permissions().mode() & 0o777,
            0o644
        );
    }
    let directory = Directory::new();
    let store = directory.store();
    store.put(&credential("a", "initial")).unwrap();
    let path = store.root.join(LOCK_FILE);
    let inode = fs::metadata(&path).unwrap().ino();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o666)).unwrap();
    store.remove(Platform::Migu, "a").unwrap();
    assert!(store.insert_if_absent(&credential("a", "next")).unwrap());
    let metadata = fs::metadata(path).unwrap();
    assert_eq!(metadata.ino(), inode);
    assert_eq!(metadata.permissions().mode() & 0o777, 0o600);
    assert_eq!(
        fs::metadata(&store.root).unwrap().permissions().mode() & 0o777,
        0o700
    );
}
