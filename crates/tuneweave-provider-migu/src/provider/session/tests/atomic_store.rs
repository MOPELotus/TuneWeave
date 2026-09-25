use super::*;
use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};
use tuneweave_core::FileAccountCredentialStore;

static SEQUENCE: AtomicU64 = AtomicU64::new(0);
struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!(
            "tuneweave-migu-atomic-{}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        )))
    }
    fn store(&self) -> Arc<FileAccountCredentialStore> {
        Arc::new(FileAccountCredentialStore::new(&self.0))
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[tokio::test]
async fn independent_logins_commit_one_result_for_both_empty_and_existing_aliases() {
    for mode in [CredentialMode::Server, CredentialMode::Both] {
        for existing in [false, true] {
            let directory = Directory::new();
            let store = directory.store();
            let other = stored(
                "B",
                &MiguCredential::verified("999".into(), "other".into()).unwrap(),
            );
            store.put(&other).unwrap();
            if existing {
                store
                    .put(&stored(
                        "A",
                        &MiguCredential::verified("333".into(), "old".into()).unwrap(),
                    ))
                    .unwrap();
            }
            let mut tasks = Vec::new();
            let mut gates = Vec::new();
            for (uid, token) in [("111", "first"), ("222", "second")] {
                let (mut provider, seen, release, server) =
                    gated(vec![check(token), profile(uid, "")]).await;
                // Each provider has independent auth/transaction locks and a separately
                // opened FileStore. Only the shared store's conditional write coordinates them.
                provider.credential_store = Some(directory.store());
                tasks.push(tokio::spawn(async move {
                    provider.import_credential(&request("A", token), mode).await
                }));
                gates.push((seen, release, server));
            }
            let mut releases = Vec::new();
            let mut servers = Vec::new();
            for (seen, release, server) in gates {
                tokio::time::timeout(Duration::from_secs(10), seen)
                    .await
                    .unwrap()
                    .unwrap();
                releases.push(release);
                servers.push(server);
            }
            // Both captured the same original absence/generation and reached final profile.
            for release in releases {
                release.send(()).unwrap();
            }
            let mut successes = Vec::new();
            let mut failures = Vec::new();
            for task in tasks {
                match tokio::time::timeout(Duration::from_secs(10), task)
                    .await
                    .unwrap()
                    .unwrap()
                {
                    Ok(value) => successes.push(value),
                    Err(error) => failures.push(error),
                }
            }
            for server in servers {
                server.await.unwrap();
            }
            assert_eq!(successes.len(), 1);
            assert_eq!(failures.len(), 1);
            assert_eq!(failures[0].code, ErrorCode::Conflict);
            assert!(failures[0].take_caller_credential_update().is_none());
            let current = store.load_platform(Platform::Migu).unwrap();
            assert_eq!(current.len(), 2);
            assert_eq!(current[1], other);
            let accepted = MiguCredential::parse(current[0].secret()).unwrap();
            assert_eq!(
                successes[0].profile.user_id.as_deref(),
                Some(accepted.user_id())
            );
            assert_eq!(successes[0].credential.is_some(), mode.returns_to_caller());
            if let Some(caller) = &successes[0].credential {
                assert_eq!(MiguCredential::parse_caller(caller).unwrap(), accepted);
            }
        }
    }
}

struct NoConditionalWrites;
impl AccountCredentialStore for NoConditionalWrites {
    fn load_platform(&self, _: Platform) -> Result<Vec<StoredAccountCredential>> {
        Ok(vec![])
    }
    fn put(&self, _: &StoredAccountCredential) -> Result<()> {
        panic!("login must not fall back to unconditional writes")
    }
    fn remove(&self, _: Platform, _: &str) -> Result<bool> {
        panic!("login must not remove a credential")
    }
}

#[tokio::test]
async fn missing_atomic_storage_fails_explicitly_but_client_login_needs_no_writes() {
    for mode in [
        CredentialMode::Server,
        CredentialMode::Both,
        CredentialMode::Client,
    ] {
        let (mut provider, requests) = server(vec![check("checked"), profile("111", "")]).await;
        provider.credential_store = Some(Arc::new(NoConditionalWrites));
        let result = provider
            .import_credential(&request("default", "imported"), mode)
            .await;
        if mode.persists_on_server() {
            let mut error = result.unwrap_err();
            assert_eq!(error.code, ErrorCode::CapabilityNotSupported);
            assert!(error.take_caller_credential_update().is_none());
        } else {
            let result = result.unwrap();
            assert!(result.profile.authenticated);
            assert_eq!(
                MiguCredential::parse_caller(&result.credential.unwrap())
                    .unwrap()
                    .token(),
                "checked"
            );
        }
        assert_eq!(requests.await.unwrap().len(), 2);
    }
}
