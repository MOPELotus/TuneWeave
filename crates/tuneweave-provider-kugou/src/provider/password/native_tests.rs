use super::*;
use crate::login::crypto::ExchangeCipher;
use crate::provider::session::tests::{Store, credential, paused, raw, read, reply, server};
use tuneweave_core::PasswordLoginProgress;

const SEED: &str = "0123456789ABCDEF0123456789ABCDEF";
const TOKEN: &str = "synthetic-native-password-token";

fn input(account: &str) -> PasswordLoginRequest {
    serde_json::from_value(json!({
        "backend":"native", "account":account, "principal_type":"phone",
        "principal":"13800138000", "password":"synthetic-password"
    }))
    .unwrap()
}

fn success() -> String {
    let cipher = ExchangeCipher::for_test(SEED);
    reply(json!({"userid":"111", "secu_params":cipher.encrypt(
        &serde_json::to_vec(&json!({"token":TOKEN})).unwrap()
    ).unwrap()}))
}

#[tokio::test]
async fn native_password_creates_or_replaces_web_accounts_with_all_ownership_modes() {
    for existing in [false, true] {
        for mode in [
            CredentialMode::Server,
            CredentialMode::Client,
            CredentialMode::Both,
        ] {
            let account = if mode == CredentialMode::Client {
                "default"
            } else {
                "A"
            };
            let mut frames = vec![];
            if existing {
                frames.push(reply(json!({"userid":"222"})).replace("Content-Type:",
                    "Set-Cookie: KuGoo=KugooID=222&t=old-web-token&a_id=1014; Domain=.kugou.com; Path=/\r\nContent-Type:").into());
            }
            frames.extend([
                success().into(),
                reply(json!({"userid":"111", "nickname":"Listener"})).into(),
            ]);
            let mut f = server(frames).await;
            let store = Arc::new(Store::default());
            f.provider.credential_store = Some(store.clone());
            f.provider.client.password_test_seed = Some(SEED.into());
            if existing {
                let mut web = input(account);
                web.backend = PasswordLoginBackend::Web;
                f.provider.password_login(&web).await.unwrap();
            }
            let previous = store.values.lock().unwrap().clone();
            let request = input(account);
            let PasswordLoginProgress::Confirmed(result) = f
                .provider
                .begin_password_login(&request, mode)
                .await
                .unwrap()
            else {
                panic!("expected confirmed native password login");
            };
            assert_eq!(result.profile.account, account);
            assert_eq!(result.profile.user_id.as_deref(), Some("111"));
            assert_eq!(result.credential.is_some(), mode.returns_to_caller());
            if let Some(caller) = &result.credential {
                assert_eq!(caller.kind, "kugou_native_v1");
                assert!(!caller.secret().contains(&request.password));
                assert!(!caller.secret().contains(&request.principal));
            }
            if mode.persists_on_server() {
                let saved = read(&store, account);
                let KugouCredential::Native(native) = &saved else {
                    panic!("expected native session")
                };
                assert_eq!(native.session.token, TOKEN);
                if mode == CredentialMode::Both {
                    assert_eq!(saved.caller().unwrap(), result.credential.unwrap());
                }
            } else {
                assert_eq!(*store.values.lock().unwrap(), previous);
            }
            assert!(
                f.provider
                    .qr_transactions
                    .lock()
                    .unwrap()
                    .passwords
                    .is_empty()
            );
            let requests = f.requests.await.unwrap();
            let offset = usize::from(existing);
            assert_eq!(requests.len(), offset + 2);
            assert!(requests[offset].starts_with("POST /login.user/v9/login_by_pwd?"));
            assert!(requests[offset + 1].starts_with("POST /usercenter/v3/get_my_info?"));
            let native = &requests[offset];
            assert!(!native.to_lowercase().contains("cookie:"));
            assert!(!native.contains("old-web-token"));
            assert!(!native.contains(&request.principal));
            assert!(!native.contains(&request.password));
            let body: serde_json::Value =
                serde_json::from_str(native.split_once("\r\n\r\n").unwrap().1).unwrap();
            let decrypted: serde_json::Value = serde_json::from_slice(
                &ExchangeCipher::for_test(SEED)
                    .decrypt(body["params"].as_str().unwrap())
                    .unwrap(),
            )
            .unwrap();
            assert_eq!(decrypted["pwd"], request.password);
            assert_eq!(decrypted["username"], request.principal);
        }
    }
}

#[tokio::test]
async fn native_password_rejection_and_profile_mismatch_preserve_the_stored_account() {
    for profile_mismatch in [false, true] {
        let frames = if profile_mismatch {
            vec![success().into(), reply(json!({"userid":"999"})).into()]
        } else {
            vec![raw(json!({"status":0,"error_code":30767})).into()]
        };
        let mut f = server(frames).await;
        let store = Arc::new(Store::default());
        let old = credential("222", "old-native");
        store.put(&old.stored("A").unwrap()).unwrap();
        f.provider.credential_store = Some(store.clone());
        f.provider.client.password_test_seed = Some(SEED.into());
        let mut error = f
            .provider
            .password_login_with_mode(&input("A"), CredentialMode::Both)
            .await
            .unwrap_err();
        assert!(error.take_caller_credential_update().is_none());
        assert_eq!(read(&store, "A"), old);
        assert_eq!(
            f.requests.await.unwrap().len(),
            if profile_mismatch { 2 } else { 1 }
        );
    }
}

#[tokio::test]
async fn native_password_logout_stops_profile_verification_or_discards_its_late_reply() {
    for verifying in [false, true] {
        let (frame, resume) = paused(if verifying {
            reply(json!({"userid":"111"}))
        } else {
            success()
        });
        let frames = if verifying {
            vec![success().into(), frame]
        } else {
            vec![frame]
        };
        let mut f = server(frames).await;
        let store = Arc::new(Store::default());
        store
            .put(&credential("222", "old-native").stored("A").unwrap())
            .unwrap();
        f.provider.credential_store = Some(store.clone());
        f.provider.client.password_test_seed = Some(SEED.into());
        let worker = f.provider.clone();
        let task = tokio::spawn(async move {
            worker
                .password_login_with_mode(&input("A"), CredentialMode::Both)
                .await
        });
        f.seen.recv().await.unwrap();
        if verifying {
            f.seen.recv().await.unwrap();
        }
        f.provider
            .logout_with_ownership("A", None, CredentialMode::Server)
            .await
            .unwrap();
        resume.send(()).unwrap();
        assert_eq!(task.await.unwrap().unwrap_err().code, ErrorCode::Conflict);
        assert!(store.values.lock().unwrap().is_empty());
        assert!(
            f.provider
                .qr_transactions
                .lock()
                .unwrap()
                .passwords
                .is_empty()
        );
        assert_eq!(
            f.requests.await.unwrap().len(),
            if verifying { 2 } else { 1 }
        );
    }
}
