use super::*;
use crate::client::catalog::tests::{json_response, response};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::{Notify, mpsc},
    task::JoinHandle,
};

const APP_UID: &str = "1234567890";
const KEY: [u8; 8] = *b"17894932";
struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        let path =
            std::env::temp_dir().join(format!("tuneweave-kuwo-device-{}", random_id().unwrap()));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn state(&self) -> PathBuf {
        self.0.join("device.json")
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
struct Reply {
    bytes: Vec<u8>,
    gate: Option<Arc<Notify>>,
}
fn accepted(id: &str) -> Reply {
    Reply {
        bytes: json_response(
            &json!({"code":200,"success":true,"data":{"appuid":id,"userType":3,"urlScheme":"https://ignored.invalid/"}}),
        ),
        gate: None,
    }
}
struct Fixture {
    client: Arc<KuwoClient>,
    seen: mpsc::UnboundedReceiver<String>,
    server: JoinHandle<()>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.server.abort();
    }
}
async fn setup(replies: Vec<Reply>) -> Fixture {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
    let (tx, seen) = mpsc::unbounded_channel();
    let server = tokio::spawn(async move {
        for reply in replies {
            let (mut stream, _) = tokio::time::timeout(Duration::from_secs(5), listener.accept())
                .await
                .unwrap()
                .unwrap();
            let mut request = Vec::new();
            while !request.ends_with(b"\r\n\r\n") {
                let mut buffer = [0; 1024];
                let n = stream.read(&mut buffer).await.unwrap();
                assert!(n > 0);
                request.extend_from_slice(&buffer[..n]);
                assert!(request.len() < 32 * 1024);
            }
            tx.send(String::from_utf8(request).unwrap()).unwrap();
            if let Some(gate) = reply.gate {
                gate.notified().await;
            }
            let _ = stream.write_all(&reply.bytes).await;
        }
    });
    let mut client = KuwoClient::test_client();
    client.http = Client::builder()
        .no_proxy()
        .redirect(Policy::none())
        .timeout(Duration::from_secs(3))
        .build()
        .unwrap();
    client.web_test_origin = Some(origin);
    client.native_test_response_key = Some(KEY);
    Fixture {
        client: Arc::new(client),
        seen,
        server,
    }
}
async fn next(fixture: &mut Fixture) -> String {
    tokio::time::timeout(Duration::from_secs(4), fixture.seen.recv())
        .await
        .unwrap()
        .unwrap()
}
fn decoded(request: &str) -> (String, String) {
    assert!(request.starts_with("GET "));
    let lower = request.to_ascii_lowercase();
    for forbidden in ["\r\ncookie:", "\r\nsecret:", "\r\nauthorization:"] {
        assert!(!lower.contains(forbidden));
    }
    let target = request.split_whitespace().nth(1).unwrap();
    let url = Url::parse(&format!("http://localhost{target}")).unwrap();
    assert_eq!(url.path(), transport::PATH);
    let query: BTreeMap<_, _> = url.query_pairs().into_owned().collect();
    assert_eq!(query.len(), 3);
    let cipher = BASE64_STANDARD.decode(&query["token"]).unwrap();
    let plain = String::from_utf8(
        cipher
            .iter()
            .zip(b"yeelion ".iter().cycle())
            .map(|(a, b)| a ^ b)
            .collect(),
    )
    .unwrap();
    assert_eq!(plain.len().to_string(), query["len"]);
    (plain, query["appuid"].clone())
}

#[test]
fn registration_requires_explicit_success_and_a_canonical_device_id() {
    for id in [json!(APP_UID), json!(1234567890u64)] {
        assert_eq!(
            transport::parse_fixture(
                &serde_json::to_vec(&json!({"code":200,"success":true,"data":{"appuid":id}}))
                    .unwrap()
            )
            .unwrap(),
            APP_UID
        );
    }
    for id in [
        json!(null),
        json!(0),
        json!("0"),
        json!("0123"),
        json!("-1"),
        json!(-1),
        json!(1.5),
        json!("18446744073709551616"),
        json!("id&uid=42"),
        json!({"id":1}),
    ] {
        assert!(
            transport::parse_fixture(
                &serde_json::to_vec(&json!({"code":200,"success":true,"data":{"appuid":id}}))
                    .unwrap()
            )
            .is_err()
        );
    }
    for raw in [
        r#"{"code":0,"success":true,"data":{"appuid":"123"}}"#,
        r#"{"code":200,"success":false,"data":{"appuid":"123"}}"#,
        r#"{"code":200,"data":{"appuid":"123"}}"#,
        r#"{"code":200,"success":true,"data":null}"#,
        r#"{"code":200,"success":true,"data":{"appuid":"123","appuid":"456"}}"#,
        r#"{"code":200,"code":500,"success":true,"data":{"appuid":"123"}}"#,
    ] {
        assert!(transport::parse_fixture(raw.as_bytes()).is_err());
    }
}

#[tokio::test]
async fn persistent_initialization_reuse_and_refresh_preserve_installation_context() {
    let directory = Directory::new();
    let path = directory.state();
    let store = KuwoNativeDeviceStore::new(Some(path.clone()));
    let mut fixture = setup(vec![accepted(APP_UID), accepted(APP_UID)]).await;
    let device = store.initialize(&fixture.client).await.unwrap();
    assert!(valid_id(device.device_user()) && valid_id(device.android_id()));
    assert_ne!(device.device_user(), device.android_id());
    assert_eq!(device.app_uid(), APP_UID);
    assert!(device.registered_at_ms() > 0);
    assert_eq!(store.initialize(&fixture.client).await.unwrap(), device);
    let reopened = KuwoNativeDeviceStore::new(Some(path.clone()));
    assert_eq!(reopened.initialize(&fixture.client).await.unwrap(), device);
    let refreshed = reopened.refresh(&fixture.client).await.unwrap();
    assert_eq!(refreshed.device_user(), device.device_user());
    assert_eq!(refreshed.android_id(), device.android_id());
    let (initial, outer) = decoded(&next(&mut fixture).await);
    assert_eq!(outer, "0");
    assert_eq!(
        initial,
        format!(
            "&new_user=1&mac={user}&hd={user}&android_id={android}&oaid=&q36=f2ce3c2ef68ddfd1b2bea7ed00001f314716&vmac=&ver=kwplayer_ar_12.2.2.0&src=kwplayer_ar_12.2.2.0_newpcguanwangmobile.apk&process=false&dev=TuneWeave SDK client",
            user = device.device_user(),
            android = device.android_id()
        )
    );
    let (refresh, outer) = decoded(&next(&mut fixture).await);
    assert_eq!(outer, APP_UID);
    assert_eq!(refresh, format!("&uid={APP_UID}{initial}"));
    (&mut fixture.server).await.unwrap();
    assert!(fixture.seen.try_recv().is_err());
    let serialized = fs::read_to_string(&path).unwrap();
    for absent in ["urlScheme", "userType", "cookie", "sid", "password"] {
        assert!(!serialized.contains(absent));
    }
    for secret in [
        device.device_user(),
        device.android_id(),
        device.app_uid(),
        path.to_str().unwrap(),
    ] {
        assert!(!format!("{device:?} {store:?}").contains(secret));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for file in [&path, &named_suffix(&path, ".lock").unwrap()] {
            assert_eq!(
                fs::metadata(file).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }
}

#[tokio::test]
async fn http_failure_keeps_the_previously_registered_device_and_does_not_retry() {
    let directory = Directory::new();
    let path = directory.state();
    let store = KuwoNativeDeviceStore::new(Some(path.clone()));
    let mut fixture = setup(vec![
        accepted(APP_UID),
        Reply {
            bytes: response(503, "application/json", "", b"{}"),
            gate: None,
        },
    ])
    .await;
    let device = store.initialize(&fixture.client).await.unwrap();
    let before = fs::read(&path).unwrap();
    assert_eq!(
        store.refresh(&fixture.client).await.unwrap_err().code,
        ErrorCode::UpstreamError
    );
    assert_eq!(fs::read(&path).unwrap(), before);
    assert_eq!(store.initialize(&fixture.client).await.unwrap(), device);
    (&mut fixture.server).await.unwrap();
    assert_eq!(fixture.seen.len(), 2);
}

#[tokio::test]
async fn rejected_first_registration_retries_with_the_same_saved_seed() {
    let directory = Directory::new();
    let path = directory.state();
    let store = KuwoNativeDeviceStore::new(Some(path.clone()));
    let mut fixture = setup(vec![
        Reply {
            bytes: json_response(&json!({"code":200,"success":true,"data":{"appuid":"0"}})),
            gate: None,
        },
        accepted(APP_UID),
    ])
    .await;
    assert!(store.initialize(&fixture.client).await.is_err());
    let seed = read_state(&path).unwrap().unwrap();
    assert!(seed.app_uid.is_none());
    let device = KuwoNativeDeviceStore::new(Some(path))
        .initialize(&fixture.client)
        .await
        .unwrap();
    assert_eq!(device.device_user(), seed.context.device_user);
    assert_eq!(
        decoded(&next(&mut fixture).await),
        decoded(&next(&mut fixture).await)
    );
}

#[tokio::test]
async fn cancelling_registration_retains_seed_and_releases_locks_in_both_storage_modes() {
    for persistent in [false, true] {
        let directory = Directory::new();
        let path = persistent.then(|| directory.state());
        let store = Arc::new(KuwoNativeDeviceStore::new(path.clone()));
        let gate = Arc::new(Notify::new());
        let mut delayed = accepted(APP_UID);
        delayed.gate = Some(gate.clone());
        let mut fixture = setup(vec![delayed, accepted(APP_UID)]).await;
        let task = {
            let store = store.clone();
            let client = fixture.client.clone();
            tokio::spawn(async move { store.initialize(&client).await })
        };
        let first = decoded(&next(&mut fixture).await);
        if let Some(path) = &path {
            assert!(read_state(path).unwrap().unwrap().app_uid.is_none());
        }
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        gate.notify_one();
        let device = store.initialize(&fixture.client).await.unwrap();
        assert!(first.0.contains(device.device_user()));
        assert_eq!(first, decoded(&next(&mut fixture).await));
    }
}

#[tokio::test]
async fn distinct_instances_sharing_a_path_make_one_initial_request() {
    let directory = Directory::new();
    let a = KuwoNativeDeviceStore::new(Some(directory.state()));
    let b = KuwoNativeDeviceStore::new(Some(directory.state()));
    let mut fixture = setup(vec![accepted(APP_UID)]).await;
    let (a, b) = tokio::join!(a.initialize(&fixture.client), b.initialize(&fixture.client));
    assert_eq!(a.unwrap(), b.unwrap());
    (&mut fixture.server).await.unwrap();
    assert_eq!(fixture.seen.len(), 1);
}

#[tokio::test]
async fn an_external_state_change_during_registration_is_not_overwritten() {
    let directory = Directory::new();
    let path = directory.state();
    let store = Arc::new(KuwoNativeDeviceStore::new(Some(path.clone())));
    let gate = Arc::new(Notify::new());
    let mut reply = accepted(APP_UID);
    reply.gate = Some(gate.clone());
    let mut fixture = setup(vec![reply]).await;
    let task = {
        let store = store.clone();
        let client = fixture.client.clone();
        tokio::spawn(async move { store.initialize(&client).await })
    };
    next(&mut fixture).await;
    let replacement = serde_json::to_vec(&State::generate().unwrap()).unwrap();
    fs::write(&path, &replacement).unwrap();
    gate.notify_one();
    assert_eq!(task.await.unwrap().unwrap_err().code, ErrorCode::Conflict);
    assert_eq!(fs::read(&path).unwrap(), replacement);
    assert!(fs::read_dir(&directory.0).unwrap().all(|entry| {
        !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .contains(".tmp-")
    }));
}

#[tokio::test]
async fn malformed_persistent_state_is_rejected_before_network_or_replacement() {
    let directory = Directory::new();
    let path = directory.state();
    let mut fixture = setup(vec![]).await;
    let seed = serde_json::to_value(State::generate().unwrap()).unwrap();
    let mut invalids = vec![b"not-json".to_vec(), vec![b' '; 8193]];
    for (field, value) in [
        ("version", json!(2)),
        ("unexpected", json!(true)),
        ("app_uid", json!(APP_UID)),
        ("registered_at_ms", json!(123)),
        (
            "context",
            json!({"device_user":"hardware-id","android_id":"hardware-id"}),
        ),
    ] {
        let mut invalid = seed.clone();
        invalid[field] = value;
        invalids.push(serde_json::to_vec(&invalid).unwrap());
    }
    for bytes in invalids {
        fs::write(&path, &bytes).unwrap();
        assert_eq!(
            KuwoNativeDeviceStore::new(Some(path.clone()))
                .initialize(&fixture.client)
                .await
                .unwrap_err()
                .code,
            ErrorCode::InternalError
        );
        assert_eq!(fs::read(&path).unwrap(), bytes);
    }
    (&mut fixture.server).await.unwrap();
    assert!(fixture.seen.try_recv().is_err());
}

#[cfg(unix)]
#[tokio::test]
async fn state_and_lock_symlinks_hardlinks_and_directories_are_rejected() {
    use std::os::unix::fs::symlink;
    let directory = Directory::new();
    let target = directory.0.join("original");
    for lock in [false, true] {
        for mode in ["symlink", "hardlink", "directory"] {
            let path = directory.0.join(format!("state-{lock}-{mode}.json"));
            let victim = if lock {
                named_suffix(&path, ".lock").unwrap()
            } else {
                path.clone()
            };
            let bytes = if lock {
                vec![]
            } else {
                serde_json::to_vec(&State::generate().unwrap()).unwrap()
            };
            fs::write(&target, &bytes).unwrap();
            match mode {
                "symlink" => symlink(&target, &victim).unwrap(),
                "hardlink" => fs::hard_link(&target, &victim).unwrap(),
                _ => fs::create_dir(&victim).unwrap(),
            }
            let fixture = setup(vec![]).await;
            assert_eq!(
                KuwoNativeDeviceStore::new(Some(path))
                    .initialize(&fixture.client)
                    .await
                    .unwrap_err()
                    .code,
                ErrorCode::InternalError
            );
            assert_eq!(fs::read(&target).unwrap(), bytes);
            if mode == "directory" {
                fs::remove_dir(victim).unwrap();
            } else {
                fs::remove_file(victim).unwrap();
            }
        }
    }
}

#[tokio::test]
async fn device_context_survives_session_exchange_and_reaches_independent_validation() {
    let mut fixture = setup(vec![
        accepted(APP_UID),
        Reply {
            bytes: response(
                200,
                "application/json",
                "",
                &codec::fixture_response(
                    br#"{"result":"succ","sid":"new-session","userInfo":{"uid":42}}"#,
                    &KEY,
                ),
            ),
            gate: None,
        },
        Reply {
            bytes: json_response(&json!({"result":"ok"})),
            gate: None,
        },
    ])
    .await;
    let device = KuwoNativeDeviceStore::default()
        .initialize(&fixture.client)
        .await
        .unwrap();
    let input = device.session_input("42", "old-session").unwrap();
    fn require_send(_: impl std::future::Future + Send) {}
    require_send(fixture.client.exchange_native_session(&input));
    require_send(fixture.client.validate_native_session(&input));
    assert_eq!(input.device_id(), device.app_uid());
    assert_eq!(input.device_user(), device.device_user());
    let exchanged = fixture
        .client
        .exchange_native_session(&input)
        .await
        .unwrap();
    assert!(input.context == exchanged.session().context);
    fixture
        .client
        .validate_native_session(exchanged.session())
        .await
        .unwrap();
    next(&mut fixture).await;
    let exchange = next(&mut fixture).await;
    assert!(!exchange.to_ascii_lowercase().contains("\r\ncookie:"));
    let validate = next(&mut fixture).await;
    let target = validate.split_whitespace().nth(1).unwrap();
    let url = Url::parse(&format!("http://localhost{target}")).unwrap();
    let query: BTreeMap<_, _> = url.query_pairs().into_owned().collect();
    assert_eq!(query["android_id"], device.android_id());
    assert_eq!(query["q36"], FALLBACK_Q36);
    assert_eq!(query["approval"], "false");
    assert_eq!(query["uid"], "42");
    assert_eq!(query["sid"], "new-session");
    assert_eq!(query["appuid"], device.app_uid());
    assert_eq!(query["user"], device.device_user());
    assert!(exchange_query(exchanged.session(), &KEY).contains("devType=SDK"));
}

const WORKER_PATH: &str = "TUNEWEAVE_KUWO_DEVICE_LOCK_WORKER";
struct Worker(std::process::Child);
impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
#[tokio::test]
async fn device_lock_worker() {
    let Some(path) = std::env::var_os(WORKER_PATH) else {
        return;
    };
    let path = PathBuf::from(path);
    let _guard = lock_file(&path, Duration::from_secs(2)).await.unwrap();
    fs::write(named_suffix(&path, ".ready").unwrap(), []).unwrap();
    let deadline = Instant::now() + Duration::from_secs(8);
    while !named_suffix(&path, ".release").unwrap().exists() {
        assert!(Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}
#[tokio::test]
async fn a_separate_process_holds_the_same_lock_and_waiting_is_bounded() {
    use std::process::{Command, Stdio};
    let directory = Directory::new();
    let path = directory.state();
    let mut worker = Worker(
        Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "client::native::device::tests::device_lock_worker",
            ])
            .env(WORKER_PATH, &path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap(),
    );
    let deadline = Instant::now() + Duration::from_secs(6);
    while !named_suffix(&path, ".ready").unwrap().exists() {
        assert!(worker.0.try_wait().unwrap().is_none());
        assert!(Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(
        lock_file(&path, Duration::from_millis(30))
            .await
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
    fs::write(named_suffix(&path, ".release").unwrap(), []).unwrap();
    let guard = lock_file(&path, Duration::from_secs(2)).await.unwrap();
    drop(guard);
    assert!(worker.0.wait().unwrap().success());
}

#[tokio::test]
#[ignore = "contacts official Kuwo anonymous device registration; no account or hardware identifiers"]
async fn live_anonymous_native_device_initialization_and_refresh() {
    let client = KuwoClient::new(&crate::KuwoConfig::default()).unwrap();
    let store = KuwoNativeDeviceStore::default();
    let device = store.initialize(&client).await.unwrap();
    assert_eq!(store.initialize(&client).await.unwrap(), device);
    let refreshed = store.refresh(&client).await.unwrap();
    assert_eq!(refreshed.app_uid(), device.app_uid());
    assert_eq!(refreshed.device_user(), device.device_user());
    assert_eq!(refreshed.android_id(), device.android_id());
}
