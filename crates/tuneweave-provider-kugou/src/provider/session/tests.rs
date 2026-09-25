use super::*;
use crate::{KugouLoginClient, credential::NativeSession, device::KugouDevice};
use std::{
    collections::BTreeMap,
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::{mpsc, oneshot},
};

#[derive(Default)]
pub(in crate::provider) struct Store {
    pub values: Mutex<BTreeMap<String, StoredAccountCredential>>,
    pub writes: AtomicUsize,
    pub fail: AtomicBool,
}
impl AccountCredentialStore for Store {
    fn load_platform(&self, platform: Platform) -> Result<Vec<StoredAccountCredential>> {
        Ok(self
            .values
            .lock()
            .unwrap()
            .values()
            .filter(|v| v.platform == platform)
            .cloned()
            .collect())
    }
    fn put(&self, value: &StoredAccountCredential) -> Result<()> {
        self.values
            .lock()
            .unwrap()
            .insert(value.account.clone(), value.clone());
        Ok(())
    }
    fn remove(&self, _: Platform, account: &str) -> Result<bool> {
        Ok(self.values.lock().unwrap().remove(account).is_some())
    }
    fn insert_if_absent(&self, value: &StoredAccountCredential) -> Result<bool> {
        if self.fail.load(Ordering::SeqCst) {
            return Err(state_error());
        }
        let mut values = self.values.lock().unwrap();
        if values.contains_key(&value.account) {
            return Ok(false);
        }
        values.insert(value.account.clone(), value.clone());
        self.writes.fetch_add(1, Ordering::SeqCst);
        Ok(true)
    }
    fn compare_exchange(
        &self,
        old: &StoredAccountCredential,
        next: Option<&StoredAccountCredential>,
    ) -> Result<bool> {
        if self.fail.load(Ordering::SeqCst) {
            return Err(state_error());
        }
        let mut values = self.values.lock().unwrap();
        if values.get(&old.account) != Some(old) {
            return Ok(false);
        }
        match next {
            Some(v) => {
                values.insert(old.account.clone(), v.clone());
            }
            None => {
                values.remove(&old.account);
            }
        }
        self.writes.fetch_add(1, Ordering::SeqCst);
        Ok(true)
    }
}
pub(in crate::provider) fn credential(uid: &str, token: &str) -> KugouCredential {
    KugouCredential::verified(NativeSession {
        client: KugouLoginClient::Standard,
        device: KugouDevice::default().identity(),
        user_id: uid.to_owned(),
        token: token.to_owned(),
        vip_token: None,
        t1: None,
    })
    .unwrap()
}
pub(in crate::provider) fn read(store: &Store, account: &str) -> KugouCredential {
    KugouCredential::parse_stored(store.values.lock().unwrap().get(account).unwrap()).unwrap()
}
pub(in crate::provider) fn reply(data: serde_json::Value) -> String {
    raw(json!({"status":1,"error_code":0,"data":data}))
}
pub(in crate::provider) fn raw(body: serde_json::Value) -> String {
    let body = body.to_string();
    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}
pub(in crate::provider) fn exchange(uid: &str, token: &str) -> String {
    reply(json!({"userid":uid,"token":token}))
}
pub(in crate::provider) fn profile(uid: &str) -> String {
    reply(json!({"userid":uid,"nickname":"Listener","pic":"https://imge.kugou.com/avatar.jpg"}))
}
pub(in crate::provider) struct Frame {
    body: String,
    gate: Option<oneshot::Receiver<()>>,
}
impl From<String> for Frame {
    fn from(body: String) -> Self {
        Self { body, gate: None }
    }
}
pub(in crate::provider) fn paused(body: String) -> (Frame, oneshot::Sender<()>) {
    let (tx, rx) = oneshot::channel();
    (
        Frame {
            body,
            gate: Some(rx),
        },
        tx,
    )
}
pub(in crate::provider) struct Fixture {
    pub provider: KugouProvider,
    pub requests: tokio::task::JoinHandle<Vec<String>>,
    pub seen: mpsc::UnboundedReceiver<String>,
}
pub(in crate::provider) async fn server(frames: Vec<Frame>) -> Fixture {
    let fixture = binary_server(frames).await;
    Fixture {
        provider: fixture.provider,
        requests: tokio::spawn(async move {
            fixture
                .requests
                .await
                .unwrap()
                .into_iter()
                .map(|r| String::from_utf8(r).unwrap())
                .collect()
        }),
        seen: fixture.seen,
    }
}
pub(in crate::provider) struct BinaryFixture {
    pub provider: KugouProvider,
    pub requests: tokio::task::JoinHandle<Vec<Vec<u8>>>,
    pub seen: mpsc::UnboundedReceiver<String>,
}
pub(in crate::provider) async fn binary_server(frames: Vec<Frame>) -> BinaryFixture {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let origin = url::Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
    let (tx, seen) = mpsc::unbounded_channel();
    let task = tokio::spawn(async move {
        let mut requests = vec![];
        for frame in frames {
            let (mut socket, _) = tokio::time::timeout(Duration::from_secs(5), listener.accept())
                .await
                .unwrap()
                .unwrap();
            let mut request = vec![];
            loop {
                let mut buffer = [0; 4096];
                let n = tokio::time::timeout(Duration::from_secs(5), socket.read(&mut buffer))
                    .await
                    .unwrap()
                    .unwrap();
                assert!(n > 0);
                request.extend_from_slice(&buffer[..n]);
                assert!(request.len() < 262144);
                if let Some(end) = request.windows(4).position(|v| v == b"\r\n\r\n") {
                    let head = std::str::from_utf8(&request[..end]).unwrap();
                    let length = head
                        .lines()
                        .find_map(|line| {
                            let (k, v) = line.split_once(':')?;
                            k.eq_ignore_ascii_case("content-length")
                                .then(|| v.trim().parse::<usize>().unwrap())
                        })
                        .unwrap_or(0);
                    if request.len() >= end + 4 + length {
                        break;
                    }
                }
            }
            let response = crate::account::response_bytes_for_request(&request, &frame.body);
            let _ = tx.send(String::from_utf8_lossy(&request).into_owned());
            requests.push(request);
            if let Some(gate) = frame.gate {
                let _ = gate.await;
            }
            let _ = socket.write_all(&response).await;
            let _ = socket.shutdown().await;
        }
        requests
    });
    let mut client = KugouClient::new(&KugouConfig::default()).unwrap();
    client.http = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(3))
        .build()
        .unwrap();
    client.login_test_origin = Some(origin);
    BinaryFixture {
        provider: KugouProvider::from_client(client),
        requests: task,
        seen,
    }
}

#[test]
fn provider_configuration_reopens_native_credentials_from_the_private_file_store() {
    use tuneweave_core::FileAccountCredentialStore;
    let root = std::env::temp_dir().join(format!(
        "tuneweave-kugou-accounts-{}",
        rand::random::<u64>()
    ));
    let store = Arc::new(FileAccountCredentialStore::open(&root).unwrap());
    let value = credential("111", "synthetic-persisted-token");
    store.put(&value.stored("A").unwrap()).unwrap();
    let config = KugouConfig {
        credential_store: Some(store.clone()),
        ..Default::default()
    };
    assert!(!format!("{config:?}").contains("synthetic-persisted-token"));
    let provider = KugouProvider::new(config).unwrap();
    assert_eq!(provider.selected("A").unwrap().unwrap().0, value);
    drop(provider);
    drop(store);
    let reopened = Arc::new(FileAccountCredentialStore::open(&root).unwrap());
    let provider = KugouProvider::new(KugouConfig {
        credential_store: Some(reopened),
        ..Default::default()
    })
    .unwrap();
    assert_eq!(provider.selected("A").unwrap().unwrap().0, value);
    assert!(provider.selected("other").unwrap().is_none());
    drop(provider);
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn account_reads_use_exact_alias_and_caller_scope_never_falls_back_to_server() {
    let mut f = server(vec![
        exchange("111", "next-a").into(),
        profile("111").into(),
        exchange("222", "next-caller").into(),
        profile("222").into(),
    ])
    .await;
    let store = Arc::new(Store::default());
    let a = credential("111", "server-a");
    let b = credential("333", "server-b");
    store.put(&a.stored("A").unwrap()).unwrap();
    store.put(&b.stored("B").unwrap()).unwrap();
    f.provider.credential_store = Some(store.clone());
    assert!(
        !f.provider
            .session_profile("missing")
            .await
            .unwrap()
            .authenticated
    );
    let first = f.provider.session_profile("A").await.unwrap();
    assert_eq!(first.user_id.as_deref(), Some("111"));
    assert_eq!(first.account, "A");
    assert_eq!(read(&store, "A").native().session.token, "next-a");
    assert_eq!(read(&store, "B"), b);
    assert!(f.provider.take_response_credential().unwrap().is_none());
    let caller = credential("222", "caller-only").caller().unwrap();
    let scoped = f.provider.caller_scope(&caller).unwrap();
    assert!(scoped.selected("A").is_err());
    assert!(scoped.credential_store.is_none());
    let second = scoped.session_profile("default").await.unwrap();
    assert_eq!(second.user_id.as_deref(), Some("222"));
    let update = scoped.take_response_credential().unwrap().unwrap();
    assert_eq!(
        KugouCredential::parse_caller(&update)
            .unwrap()
            .native()
            .session
            .token,
        "next-caller"
    );
    assert!(scoped.take_response_credential().unwrap().is_none());
    assert_eq!(store.values.lock().unwrap().len(), 2);
    let requests = f.requests.await.unwrap();
    assert!(requests[0].contains("token=server-a"));
    assert!(requests[2].contains("token=caller-only"));
    assert!(!requests[2].contains("server-a"));
    assert!(requests.iter().all(|r| !r.contains("server-b")));
}

#[tokio::test]
async fn ownership_refresh_keeps_generation_and_both_uses_latest_server_token() {
    for mode in [
        CredentialMode::Server,
        CredentialMode::Client,
        CredentialMode::Both,
    ] {
        let mut f = server(vec![
            exchange("111", "refreshed").into(),
            profile("111").into(),
        ])
        .await;
        let store = Arc::new(Store::default());
        let old = credential("111", "old");
        let mut session = old.native().session.clone();
        session.token = "server-current".to_owned();
        let current = old.rotate(session).unwrap();
        let account = if mode == CredentialMode::Client {
            "default"
        } else {
            "A"
        };
        store.put(&current.stored(account).unwrap()).unwrap();
        f.provider.credential_store = Some(store.clone());
        let caller = old.caller().unwrap();
        let source = mode.returns_to_caller().then_some(&caller);
        let result = f
            .provider
            .refresh_session_with_ownership(account, source, mode)
            .await
            .unwrap();
        assert_eq!(result.credential.is_some(), mode.returns_to_caller());
        assert_eq!(result.profile.account, account);
        if let Some(value) = result.credential {
            let next = KugouCredential::parse_caller(&value).unwrap();
            assert!(old.same_login(&next));
            assert_eq!(next.native().session.token, "refreshed");
            if mode == CredentialMode::Both {
                assert_eq!(next, read(&store, account));
            }
        }
        assert_eq!(
            read(&store, account).native().session.token,
            if mode == CredentialMode::Client {
                "server-current"
            } else {
                "refreshed"
            }
        );
        assert!(
            f.requests.await.unwrap()[0].contains(if mode == CredentialMode::Client {
                "token=old"
            } else {
                "token=server-current"
            })
        );
    }
}

#[tokio::test]
async fn verified_rotation_is_preserved_on_profile_failure_only_for_the_selected_owner() {
    for mode in [
        CredentialMode::Server,
        CredentialMode::Client,
        CredentialMode::Both,
    ] {
        for code in [20010, 20017] {
            let mut f = server(vec![
                exchange("111", "rotated").into(),
                raw(json!({"status":0,"error_code":code,"data":null})).into(),
            ])
            .await;
            let store = Arc::new(Store::default());
            let source = credential("111", "initial");
            let caller = source.caller().unwrap();
            let account = if mode == CredentialMode::Client {
                "default"
            } else {
                "A"
            };
            store.put(&source.stored(account).unwrap()).unwrap();
            f.provider.credential_store = Some(store.clone());
            let mut failure = f
                .provider
                .refresh_session_with_ownership(
                    account,
                    mode.returns_to_caller().then_some(&caller),
                    mode,
                )
                .await
                .unwrap_err();
            assert_eq!(
                failure.take_caller_credential_update().is_some(),
                code == 20010 && mode.returns_to_caller()
            );
            if mode.persists_on_server() {
                if code == 20017 {
                    assert!(store.values.lock().unwrap().is_empty());
                } else {
                    assert_eq!(read(&store, account).native().session.token, "rotated");
                }
            } else {
                assert_eq!(read(&store, account), source);
            }
            assert_eq!(f.requests.await.unwrap().len(), 2);
        }
    }
}

#[tokio::test]
async fn logout_and_relogin_defeat_late_profile_success_and_failure() {
    for replace in [false, true] {
        for success in [false, true] {
            let response = if success {
                profile("111")
            } else {
                raw(json!({"status":0,"error_code":20017,"data":null}))
            };
            let (last, resume) = paused(response);
            let mut f = server(vec![exchange("111", "rotated").into(), last]).await;
            let store = Arc::new(Store::default());
            let old = credential("111", "initial");
            let caller = old.caller().unwrap();
            store.put(&old.stored("A").unwrap()).unwrap();
            f.provider.credential_store = Some(store.clone());
            let provider = f.provider.clone();
            let read = tokio::spawn(async move {
                provider
                    .refresh_session_with_ownership("A", Some(&caller), CredentialMode::Both)
                    .await
            });
            f.seen.recv().await.unwrap();
            f.seen.recv().await.unwrap();
            let new = credential("222", "new-login");
            if replace {
                store.put(&new.stored("A").unwrap()).unwrap();
            } else {
                assert!(f.provider.logout("A").await.unwrap());
            }
            resume.send(()).unwrap();
            let mut error = read.await.unwrap().unwrap_err();
            assert_eq!(error.code, ErrorCode::Conflict);
            assert!(error.take_caller_credential_update().is_none());
            if replace {
                assert_eq!(super::tests::read(&store, "A"), new);
            } else {
                assert!(store.values.lock().unwrap().is_empty());
            }
            f.requests.await.unwrap();
        }
    }
}

#[tokio::test]
async fn store_failure_stops_before_profile_and_never_exports_unpersisted_rotation() {
    let mut f = server(vec![exchange("111", "next").into()]).await;
    let store = Arc::new(Store::default());
    let source = credential("111", "old");
    let caller = source.caller().unwrap();
    store.put(&source.stored("A").unwrap()).unwrap();
    store.fail.store(true, Ordering::SeqCst);
    f.provider.credential_store = Some(store.clone());
    let mut error = f
        .provider
        .refresh_session_with_ownership("A", Some(&caller), CredentialMode::Both)
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::InternalError);
    assert!(error.take_caller_credential_update().is_none());
    assert_eq!(read(&store, "A"), source);
    assert_eq!(f.requests.await.unwrap().len(), 1);
}

#[tokio::test]
async fn local_logout_is_idempotent_and_cannot_remove_a_different_generation() {
    let mut f = server(vec![]).await;
    let store = Arc::new(Store::default());
    let a = credential("111", "a");
    let b = credential("111", "b");
    let caller = a.caller().unwrap();
    store.put(&b.stored("A").unwrap()).unwrap();
    f.provider.credential_store = Some(store.clone());
    assert_eq!(
        f.provider
            .logout_with_ownership("A", Some(&caller), CredentialMode::Both)
            .await
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
    assert_eq!(read(&store, "A"), b);
    let client = f
        .provider
        .logout_with_ownership("default", Some(&caller), CredentialMode::Client)
        .await
        .unwrap();
    assert!(!client.removed && client.caller_credential_discard_required);
    assert_eq!(read(&store, "A"), b);
    let matching = b.caller().unwrap();
    assert!(
        f.provider
            .logout_with_ownership("A", Some(&matching), CredentialMode::Both)
            .await
            .unwrap()
            .removed
    );
    let repeat = f
        .provider
        .logout_with_ownership("A", Some(&matching), CredentialMode::Both)
        .await
        .unwrap();
    assert!(!repeat.removed && repeat.caller_credential_discard_required);
    assert!(!f.provider.logout("A").await.unwrap());
    assert!(f.requests.await.unwrap().is_empty());
}

#[tokio::test]
async fn caller_views_cannot_silently_use_unsupported_public_or_account_mutation_methods() {
    let f = server(vec![]).await;
    let c = credential("111", "synthetic").caller().unwrap();
    let scoped = f.provider.caller_scope(&c).unwrap();
    assert!(scoped.lyrics("123", None).await.is_err());
    assert!(scoped.playlist("123", None).await.is_err());
    assert!(
        scoped
            .refresh_session_with_ownership("default", Some(&c), CredentialMode::Client)
            .await
            .is_err()
    );
    assert!(
        scoped
            .start_qr_login_with_mode(None, CredentialMode::Client)
            .await
            .is_err()
    );
    assert!(scoped.logout("default").await.is_err());
    assert!(f.requests.await.unwrap().is_empty());
}

#[tokio::test]
async fn authenticated_user_profile_is_self_only_and_keeps_unknown_fields_unknown() {
    use tuneweave_core::UserProfileBackend;
    let f = server(vec![exchange("111", "next").into(), profile("111").into()]).await;
    let c = credential("111", "caller").caller().unwrap();
    let scoped = f.provider.caller_scope(&c).unwrap();
    assert!(
        scoped
            .user_profile("222", UserProfileBackend::Modern, None)
            .await
            .is_err()
    );
    assert!(
        scoped
            .user_profile("111", UserProfileBackend::Legacy, None)
            .await
            .is_err()
    );
    let profile = scoped
        .user_profile("111", UserProfileBackend::Modern, None)
        .await
        .unwrap();
    assert_eq!(profile.user.id, "111");
    assert_eq!(profile.user.name, "Listener");
    assert!(profile.level.is_none() && profile.extensions.is_empty());
    assert!(scoped.take_response_credential().unwrap().is_some());
    assert_eq!(f.requests.await.unwrap().len(), 2);
}
