use super::*;
use crate::login::crypto::ExchangeCipher;
use crate::provider::session::tests::{Store, credential, paused, raw, read, reply, server};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use serde_json::Value;

pub(super) const SEED: &str = "0123456789ABCDEF0123456789ABCDEF";

pub(super) fn input(account: &str) -> PasswordLoginRequest {
    serde_json::from_value(json!({"backend":"native","account":account,
        "principal_type":"phone","principal":"13800138000","password":"first-password"}))
    .unwrap()
}
pub(super) fn rejected(code: i64) -> String {
    raw(json!({"status":0,"error_code":code}))
}
pub(super) fn image(key: &str) -> String {
    reply(json!({"verifykey":key,"verifycode":BASE64.encode(b"\x89PNG\r\n\x1a\nsynthetic-image")}))
}
pub(super) fn success() -> String {
    reply(
        json!({"userid":"111","secu_params":ExchangeCipher::for_test(SEED)
        .encrypt(br#"{"token":"synthetic-native-token"}"#).unwrap()}),
    )
}
pub(super) fn pending(progress: PasswordLoginProgress, attempts: u8) -> ProviderPasswordChallenge {
    let PasswordLoginProgress::Pending {
        challenge,
        verification,
    } = progress
    else {
        panic!("expected pending image")
    };
    let PasswordVerification::Image { image } = verification else {
        panic!("expected image")
    };
    assert_eq!(image.remaining_attempts, attempts);
    assert_eq!(image.answer_kind, AuthImageAnswerKind::Alphanumeric);
    assert!(image.image_data_url.starts_with("data:image/png;base64,"));
    assert!(image.refresh_after_secs <= 2);
    for secret in ["13800138000", "first-password", "private-key"] {
        assert!(!format!("{challenge:?}").contains(secret));
    }
    challenge
}
pub(super) fn answer(value: &str, password: &str) -> PasswordChallengeAction {
    PasswordChallengeAction::SubmitImage {
        answer: value.into(),
        password: password.into(),
    }
}
fn allow_refresh(provider: &KugouProvider, receipt: &ProviderPasswordChallenge) {
    let mut registry = provider.qr_transactions.lock().unwrap();
    registry
        .passwords
        .get_mut(receipt.provider_transaction_id())
        .unwrap()
        .native
        .as_mut()
        .unwrap()
        .context
        .as_mut()
        .unwrap()
        .next_image_at = Instant::now();
}
pub(super) fn request_parts(request: &str) -> (std::collections::BTreeMap<String, String>, &str) {
    let (headers, body) = request.split_once("\r\n\r\n").unwrap();
    let target = headers
        .lines()
        .next()
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap();
    let url = url::Url::parse(&format!("http://localhost{target}")).unwrap();
    (url.query_pairs().into_owned().collect(), body)
}

#[tokio::test]
async fn native_image_retry_refresh_and_confirmation_keep_one_device_and_all_ownership_modes() {
    for mode in [
        CredentialMode::Server,
        CredentialMode::Client,
        CredentialMode::Both,
    ] {
        let mut f = server(vec![
            rejected(30709).into(),
            image("private-key-1").into(),
            rejected(20020).into(),
            image("private-key-2").into(),
            image("private-key-3").into(),
            success().into(),
            reply(json!({"userid":"111"})).into(),
        ])
        .await;
        f.provider.client.password_test_seed = Some(SEED.into());
        let store = Arc::new(Store::default());
        let account = if mode == CredentialMode::Client {
            "default"
        } else {
            "A"
        };
        let old = credential("222", "old-native-token");
        store.put(&old.stored(account).unwrap()).unwrap();
        f.provider.credential_store = Some(store.clone());
        let receipt = pending(
            f.provider
                .begin_password_login(&input(account), mode)
                .await
                .unwrap(),
            5,
        );
        assert_eq!(read(&store, account), old);
        assert_eq!(receipt.identity().backend, PasswordLoginBackend::Native);
        let again = pending(
            f.provider
                .advance_password_login(&receipt, &answer("Wrong1", "second-password"))
                .await
                .unwrap(),
            4,
        );
        assert_eq!(again, receipt);
        let error = f
            .provider
            .advance_password_login(&receipt, &PasswordChallengeAction::RefreshImage)
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::RateLimited);
        assert!(!error.auth_challenge_consumed());
        allow_refresh(&f.provider, &receipt);
        let refreshed = pending(
            f.provider
                .advance_password_login(&receipt, &PasswordChallengeAction::RefreshImage)
                .await
                .unwrap(),
            4,
        );
        assert_eq!(refreshed, receipt);
        assert_eq!(read(&store, account), old);
        let PasswordLoginProgress::Confirmed(result) = f
            .provider
            .advance_password_login(&receipt, &answer("A7b9", "final-password"))
            .await
            .unwrap()
        else {
            panic!("expected confirmation")
        };
        assert_eq!(result.profile.account, account);
        assert_eq!(result.profile.user_id.as_deref(), Some("111"));
        assert_eq!(result.credential.is_some(), mode.returns_to_caller());
        if mode.persists_on_server() {
            assert!(!read(&store, account).same_login(&old));
            if mode == CredentialMode::Both {
                assert_eq!(
                    read(&store, account).caller().unwrap(),
                    result.credential.unwrap()
                );
            }
        } else {
            assert_eq!(read(&store, account), old);
        }
        assert!(
            f.provider
                .qr_transactions
                .lock()
                .unwrap()
                .passwords
                .is_empty()
        );
        assert_eq!(
            f.provider
                .advance_password_login(&receipt, &answer("A7b9", "final-password"))
                .await
                .unwrap_err()
                .code,
            ErrorCode::ResourceNotFound
        );
        let requests = f.requests.await.unwrap();
        assert_eq!(requests.len(), 7);
        let first_mid = request_parts(&requests[0]).0["mid"].clone();
        for (i, password, code, key) in [
            (0, "first-password", None, None),
            (2, "second-password", Some("Wrong1"), Some("private-key-1")),
            (5, "final-password", Some("A7b9"), Some("private-key-3")),
        ] {
            assert!(requests[i].starts_with("POST /login.user/v9/login_by_pwd?"));
            let (mut query, body) = request_parts(&requests[i]);
            assert_eq!(query["clientver"], "20809");
            assert_eq!(query["mid"], first_mid);
            let signature = query.remove("signature").unwrap();
            let borrowed = query.iter().map(|(k, v)| (k.as_str(), v.clone())).collect();
            assert_eq!(
                signature,
                crate::signing::android_signature(&borrowed, body.as_bytes())
            );
            let body: Value = serde_json::from_str(body).unwrap();
            assert_eq!(body.get("verifykey").and_then(Value::as_str), key);
            let decrypted: Value = serde_json::from_slice(
                &ExchangeCipher::for_test(SEED)
                    .decrypt(body["params"].as_str().unwrap())
                    .unwrap(),
            )
            .unwrap();
            assert_eq!(decrypted["pwd"], password);
            assert_eq!(decrypted.get("verifycode").and_then(Value::as_str), code);
            assert!(!requests[i].contains(password));
        }
        for i in [1, 3, 4] {
            assert!(requests[i].starts_with("GET /v2/get_img_code_ex?"));
            let (query, body) = request_parts(&requests[i]);
            assert_eq!(query["clientver"], "20809");
            assert_eq!(query["codetype"], "0");
            assert_eq!(query["type"], "LoginCheckCode");
            assert!(body.is_empty());
            assert!(!requests[i].to_lowercase().contains("cookie:"));
        }
        assert!(requests[6].starts_with("POST /usercenter/v3/get_my_info?"));
    }
}

#[tokio::test]
async fn native_image_binding_and_invalid_actions_cannot_change_or_consume_the_login() {
    let mut f = server(vec![
        rejected(30709).into(),
        image("private-key").into(),
        success().into(),
        reply(json!({"userid":"111"})).into(),
    ])
    .await;
    f.provider.client.password_test_seed = Some(SEED.into());
    let receipt = pending(
        f.provider
            .begin_password_login(&input("default"), CredentialMode::Client)
            .await
            .unwrap(),
        5,
    );
    for action in [
        answer("", "password"),
        answer("A\n", "password"),
        answer("A7b9", " padded"),
        PasswordChallengeAction::SubmitSms {
            code: "1234".into(),
        },
        PasswordChallengeAction::ResendSms,
    ] {
        let error = f
            .provider
            .advance_password_login(&receipt, &action)
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::InvalidRequest);
        assert!(!error.auth_challenge_consumed());
    }
    for change in 0..4 {
        let mut identity = receipt.identity().clone();
        match change {
            0 => identity.backend = PasswordLoginBackend::Web,
            1 => identity.account = "B".into(),
            2 => identity.principal = "13900139000".into(),
            _ => (),
        }
        let forged = ProviderPasswordChallenge::new(
            if change == 3 {
                Platform::Migu
            } else {
                Platform::Kugou
            },
            identity,
            CredentialMode::Client,
            receipt.provider_transaction_id().into(),
        )
        .unwrap();
        assert_eq!(
            f.provider
                .advance_password_login(&forged, &answer("A7b9", "password"))
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
    }
    assert!(matches!(
        f.provider
            .advance_password_login(&receipt, &answer("A7b9", "password"))
            .await
            .unwrap(),
        PasswordLoginProgress::Confirmed(_)
    ));
    assert_eq!(f.requests.await.unwrap().len(), 4);
}

#[tokio::test]
async fn native_image_only_runs_for_verification_codes_and_rejects_missing_material() {
    for code in [30702, 30703, 20021] {
        let mut frames = vec![rejected(code).into()];
        if code == 20021 {
            frames.push(reply(json!({"verifykey":"private-key"})).into());
        }
        let f = server(frames).await;
        let error = f
            .provider
            .begin_password_login(&input("default"), CredentialMode::Client)
            .await
            .unwrap_err();
        assert_eq!(
            error.code,
            match code {
                30702 | 30703 => ErrorCode::AuthenticationRequired,
                _ => ErrorCode::UpstreamError,
            }
        );
        assert!(!format!("{error:?}").contains("private-"));
        assert!(
            f.provider
                .qr_transactions
                .lock()
                .unwrap()
                .passwords
                .is_empty()
        );
        let requests = f.requests.await.unwrap();
        assert_eq!(
            requests.len(),
            if matches!(code, 30702 | 30703) { 1 } else { 2 }
        );
    }
}

#[tokio::test]
async fn native_image_pending_logout_or_replacement_prevents_any_resume_request() {
    for replacement in [false, true] {
        let mut frames = vec![rejected(30709).into(), image("private-key").into()];
        if replacement {
            frames.push(reply(json!({"userid":"333"})).replace("Content-Type:",
            "Set-Cookie: KuGoo=KugooID=333&t=new-web-token&a_id=1014; Domain=.kugou.com; Path=/\r\nContent-Type:").into());
        }
        let mut f = server(frames).await;
        let store = Arc::new(Store::default());
        store
            .put(&credential("222", "old-native-token").stored("A").unwrap())
            .unwrap();
        f.provider.credential_store = Some(store.clone());
        let receipt = pending(
            f.provider
                .begin_password_login(&input("A"), CredentialMode::Both)
                .await
                .unwrap(),
            5,
        );
        if replacement {
            let mut request = input("A");
            request.backend = PasswordLoginBackend::Web;
            f.provider.password_login(&request).await.unwrap();
        } else {
            f.provider
                .logout_with_ownership("A", None, CredentialMode::Server)
                .await
                .unwrap();
        }
        let saved = store.values.lock().unwrap().clone();
        assert_eq!(
            f.provider
                .advance_password_login(&receipt, &answer("A7b9", "password"))
                .await
                .unwrap_err()
                .code,
            ErrorCode::ResourceNotFound
        );
        assert_eq!(*store.values.lock().unwrap(), saved);
        assert_eq!(
            f.requests.await.unwrap().len(),
            if replacement { 3 } else { 2 }
        );
    }
}

#[tokio::test]
async fn native_image_expiry_and_cancelled_fetch_release_the_shared_transaction() {
    let f = server(vec![rejected(30709).into(), image("private-key").into()]).await;
    let receipt = pending(
        f.provider
            .begin_password_login(&input("default"), CredentialMode::Client)
            .await
            .unwrap(),
        5,
    );
    f.provider
        .qr_transactions
        .lock()
        .unwrap()
        .passwords
        .get_mut(receipt.provider_transaction_id())
        .unwrap()
        .deadline = Instant::now();
    assert_eq!(
        f.provider
            .advance_password_login(&receipt, &answer("A7b9", "password"))
            .await
            .unwrap_err()
            .code,
        ErrorCode::ResourceNotFound
    );
    assert!(
        f.provider
            .qr_transactions
            .lock()
            .unwrap()
            .passwords
            .is_empty()
    );
    assert_eq!(f.requests.await.unwrap().len(), 2);
    let (frame, resume) = paused(image("private-key"));
    let mut f = server(vec![rejected(30709).into(), frame]).await;
    let worker = f.provider.clone();
    let task = tokio::spawn(async move {
        worker
            .begin_password_login(&input("default"), CredentialMode::Client)
            .await
    });
    f.seen.recv().await.unwrap();
    f.seen.recv().await.unwrap();
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert!(
        f.provider
            .qr_transactions
            .lock()
            .unwrap()
            .passwords
            .is_empty()
    );
    resume.send(()).unwrap();
    assert_eq!(f.requests.await.unwrap().len(), 2);
}

#[tokio::test]
async fn native_image_attempt_limit_consumes_the_receipt_without_an_extra_image_or_password_retry()
{
    let mut frames = vec![rejected(30709).into(), image("private-key").into()];
    for _ in 0..4 {
        frames.extend([rejected(20020).into(), image("private-key").into()]);
    }
    frames.push(rejected(20020).into());
    let f = server(frames).await;
    let receipt = pending(
        f.provider
            .begin_password_login(&input("default"), CredentialMode::Client)
            .await
            .unwrap(),
        5,
    );
    for remaining in (1..5).rev() {
        assert_eq!(
            pending(
                f.provider
                    .advance_password_login(&receipt, &answer("Wrong", "password"))
                    .await
                    .unwrap(),
                remaining
            ),
            receipt
        );
    }
    let error = f
        .provider
        .advance_password_login(&receipt, &answer("Wrong", "password"))
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::RateLimited);
    assert!(error.auth_challenge_consumed());
    assert!(
        f.provider
            .qr_transactions
            .lock()
            .unwrap()
            .passwords
            .is_empty()
    );
    assert_eq!(f.requests.await.unwrap().len(), 11);
}

const SCRIPT_TARGET: &str = "https://h5.kugou.com/synthetic-script.js?private-query";

fn script(key: Option<&str>) -> String {
    browser_challenge(key, SCRIPT_TARGET)
}

fn browser_challenge(key: Option<&str>, target: &str) -> String {
    // Browser material has priority over an image; it must never be silently downgraded.
    reply(json!({"verifykey":key,"serpath":target,
        "verifycode":BASE64.encode(b"\x89PNG\r\n\x1a\nsynthetic-image")}))
}
fn browser_pending(
    progress: PasswordLoginProgress,
    attempts: u8,
) -> (ProviderPasswordChallenge, String) {
    browser_pending_target(progress, attempts, SCRIPT_TARGET)
}

fn browser_pending_target(
    progress: PasswordLoginProgress,
    attempts: u8,
    target: &str,
) -> (ProviderPasswordChallenge, String) {
    let PasswordLoginProgress::Pending {
        challenge,
        verification,
    } = progress
    else {
        panic!("expected pending browser")
    };
    assert!(!format!("{verification:?}").contains("private-query"));
    let PasswordVerification::Browser {
        protocol,
        verification_id,
        url,
        device_id,
        client_version,
        remaining_attempts,
    } = verification
    else {
        panic!("expected browser")
    };
    assert_eq!(protocol, PasswordBrowserProtocol::KugouNativeBridge);
    assert_eq!(client_version, CLIENT_VERSION);
    assert!(!device_id.is_empty());
    assert!(device_id.bytes().all(|b| b.is_ascii_digit()));
    assert!(!format!("{challenge:?}").contains(&device_id));
    assert_eq!(remaining_attempts, attempts);
    assert_eq!(verification_id.len(), 64);
    let url = url::Url::parse(&url).unwrap();
    assert_eq!(url.origin().ascii_serialization(), "https://h5.kugou.com");
    assert_eq!(url.path(), "/apps/h5Verify/verify.html");
    let parameters: Vec<_> = url.query_pairs().collect();
    assert_eq!(parameters, vec![("thisurl".into(), target.into())]);
    assert!(!url.query().unwrap().contains('+'));
    (challenge, verification_id)
}
fn browser_answer(id: &str, response: &str) -> PasswordChallengeAction {
    PasswordChallengeAction::SubmitBrowser {
        verification_id: id.into(),
        response: response.into(),
        password: "browser-password".into(),
    }
}
const CALLBACK: &str = r#"{"close":0,"ticket":"opaque%2F+ticket"}"#;

#[tokio::test]
async fn native_browser_retry_and_image_transition_bind_the_ticket_and_keep_ownership() {
    for mode in [
        CredentialMode::Server,
        CredentialMode::Client,
        CredentialMode::Both,
    ] {
        let mut f = server(vec![
            rejected(30791).into(),
            script(Some("private-key-1")).into(),
            rejected(30791).into(),
            script(None).into(),
            rejected(20020).into(),
            image("private-image-key").into(),
            success().into(),
            reply(json!({"userid":"111"})).into(),
        ])
        .await;
        f.provider.client.password_test_seed = Some(SEED.into());
        let store = Arc::new(Store::default());
        let account = if mode == CredentialMode::Client {
            "default"
        } else {
            "A"
        };
        let old = credential("222", "old-native-token");
        store.put(&old.stored(account).unwrap()).unwrap();
        f.provider.credential_store = Some(store.clone());
        let (receipt, first_id) = browser_pending(
            f.provider
                .begin_password_login(&input(account), mode)
                .await
                .unwrap(),
            5,
        );
        let deadline = f.provider.qr_transactions.lock().unwrap().passwords
            [receipt.provider_transaction_id()]
        .deadline;
        for action in [
            answer("A7b9", "password"),
            PasswordChallengeAction::RefreshImage,
            browser_answer("wrong-id", CALLBACK),
            browser_answer(&first_id, r#"{"close":1,"ticket":"cancelled"}"#),
            browser_answer(
                &first_id,
                r#"{"close":0,"state":"ready","ticket":"not-proof"}"#,
            ),
            browser_answer(
                &first_id,
                r#"{"status":1,"error_code":0,"verify_data":"web-sms-proof"}"#,
            ),
        ] {
            let error = f
                .provider
                .advance_password_login(&receipt, &action)
                .await
                .unwrap_err();
            assert_eq!(error.code, ErrorCode::InvalidRequest);
            assert!(!error.auth_challenge_consumed());
        }
        let (again, second_id) = browser_pending(
            f.provider
                .advance_password_login(&receipt, &browser_answer(&first_id, CALLBACK))
                .await
                .unwrap(),
            4,
        );
        assert_eq!(again, receipt);
        assert_ne!(first_id, second_id);
        assert_eq!(read(&store, account), old);
        assert_eq!(
            f.provider
                .advance_password_login(&receipt, &browser_answer(&first_id, CALLBACK))
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
        let same = pending(
            f.provider
                .advance_password_login(&receipt, &browser_answer(&second_id, CALLBACK))
                .await
                .unwrap(),
            3,
        );
        assert_eq!(same, receipt);
        assert_eq!(
            f.provider.qr_transactions.lock().unwrap().passwords[receipt.provider_transaction_id()]
                .deadline,
            deadline
        );
        assert_eq!(
            f.provider
                .advance_password_login(&receipt, &browser_answer(&second_id, CALLBACK))
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
        let PasswordLoginProgress::Confirmed(result) = f
            .provider
            .advance_password_login(&receipt, &answer("A7b9", "final-password"))
            .await
            .unwrap()
        else {
            panic!("expected confirmed")
        };
        assert_eq!(result.profile.account, account);
        assert_eq!(result.credential.is_some(), mode.returns_to_caller());
        assert_eq!(
            read(&store, account).same_login(&old),
            !mode.persists_on_server()
        );
        assert_eq!(
            f.provider
                .advance_password_login(&receipt, &browser_answer(&second_id, CALLBACK))
                .await
                .unwrap_err()
                .code,
            ErrorCode::ResourceNotFound
        );
        let requests = f.requests.await.unwrap();
        assert_eq!(requests.len(), 8);
        let mid = request_parts(&requests[0]).0["mid"].clone();
        for (index, expected_key, expected_code, expected_password) in [
            (2, "private-key-1", "opaque%2F+ticket", "browser-password"),
            (4, "", "opaque%2F+ticket", "browser-password"),
            (6, "private-image-key", "A7b9", "final-password"),
        ] {
            let (mut query, body) = request_parts(&requests[index]);
            assert_eq!(query["mid"], mid);
            let signature = query.remove("signature").unwrap();
            assert_eq!(
                signature,
                crate::signing::android_signature(
                    &query.iter().map(|(k, v)| (k.as_str(), v.clone())).collect(),
                    body.as_bytes()
                )
            );
            let body: Value = serde_json::from_str(body).unwrap();
            assert_eq!(body["verifykey"], expected_key);
            let secret: Value = serde_json::from_slice(
                &ExchangeCipher::for_test(SEED)
                    .decrypt(body["params"].as_str().unwrap())
                    .unwrap(),
            )
            .unwrap();
            assert_eq!(secret["verifycode"], expected_code);
            assert_eq!(secret["pwd"], expected_password);
            for value in [
                expected_code,
                expected_password,
                "unused-random",
                "verify_data",
                "private-query",
            ] {
                assert!(!requests[index].contains(value));
            }
        }
        for index in [1, 3] {
            assert_eq!(request_parts(&requests[index]).0["codetype"], "3");
        }
        assert_eq!(request_parts(&requests[5]).0["codetype"], "0");
    }
}

#[tokio::test]
async fn native_image_refresh_can_require_browser_then_confirm_directly() {
    let mut f = server(vec![
        rejected(30709).into(),
        image("private-key").into(),
        script(Some("browser-key")).into(),
        success().into(),
        reply(json!({"userid":"111"})).into(),
    ])
    .await;
    f.provider.client.password_test_seed = Some(SEED.into());
    let receipt = pending(
        f.provider
            .begin_password_login(&input("default"), CredentialMode::Client)
            .await
            .unwrap(),
        5,
    );
    allow_refresh(&f.provider, &receipt);
    let (same, id) = browser_pending(
        f.provider
            .advance_password_login(&receipt, &PasswordChallengeAction::RefreshImage)
            .await
            .unwrap(),
        5,
    );
    assert_eq!(same, receipt);
    assert!(matches!(
        f.provider
            .advance_password_login(&receipt, &browser_answer(&id, CALLBACK))
            .await
            .unwrap(),
        PasswordLoginProgress::Confirmed(_)
    ));
    assert_eq!(f.requests.await.unwrap().len(), 5);
}

#[test]
fn native_browser_callback_matches_native_transport_without_web_sms_decoding() {
    assert_eq!(browser::ticket(CALLBACK).unwrap(), "opaque%2F+ticket");
    for response in [
        r#"{"close":0,"ticket":""}"#,
        r#"{"close":0,"ticket":"a","ticket":"b"}"#,
        r#"{"close":0,"ticket":"a","info":null}"#,
        r#"{"close":0,"ticket":"   "}"#,
        r#"{"close":"0","ticket":"a"}"#,
        r#"{"ret":0,"ticket":"a"}"#,
        r#"{"close":1,"ticket":"a"}"#,
        r#"{"close":0,"ticket":"a","verify_data":"web-sms"}"#,
        r#"{"close":0,"ticket":"a\nb"}"#,
    ] {
        assert_eq!(
            browser::ticket(response).unwrap_err().code,
            ErrorCode::InvalidRequest
        );
    }
    for target in [
        "http://h5.kugou.com/x",
        "https://secret@h5.kugou.com/x",
        "https://h5.kugou.com/x#secret",
        "https://h5.kugou.com/KGCodeGT.js",
        "javascript:alert(1)",
        "KGCodeTX|",
        "KGCodeTX|123|secret",
        "KGCodeGT|{}",
        r#"KGCodeGT|{"gt":"id","challenge":"challenge","success":2}"#,
        r#"KGCodeGT|{"gt":"id","challenge":"challenge|secret","success":1}"#,
    ] {
        let error = browser::page_url(target).unwrap_err();
        assert_eq!(error.code, ErrorCode::UpstreamError);
        assert!(!format!("{error:?}").contains("secret"));
    }
}

#[tokio::test]
async fn native_modern_browser_proof_is_forwarded_unchanged_in_encrypted_password_request() {
    for (target, proof) in [
        (
            "KGCodeTX|1234567890",
            r#"KGCodeTX|{"ticket":"opaque%2F+ticket","randstr":"private-random","txappid":"1234567890"}"#,
        ),
        (
            r#"KGCodeGT|{ "gt": "synthetic-gt", "challenge": "private-challenge", "success": 1, "new_captcha": 1 }"#,
            r#"KGCodeGT|{"geetest_challenge":"opaque%2F+challenge","geetest_validate":"private-validate","geetest_seccode":"private-seccode"}"#,
        ),
    ] {
        let mut f = server(vec![
            rejected(30791).into(),
            browser_challenge(None, target).into(),
            success().into(),
            reply(json!({"userid":"111"})).into(),
        ])
        .await;
        f.provider.client.password_test_seed = Some(SEED.into());
        let progress = f
            .provider
            .begin_password_login(&input("default"), CredentialMode::Client)
            .await
            .unwrap();
        let PasswordLoginProgress::Pending {
            verification: PasswordVerification::Browser { device_id, .. },
            ..
        } = &progress
        else {
            panic!("expected browser");
        };
        let device_id = device_id.clone();
        let (receipt, id) = browser_pending_target(progress, 5, target);
        let response = json!({"close":0,"ticket":proof}).to_string();
        assert!(matches!(
            f.provider
                .advance_password_login(&receipt, &browser_answer(&id, &response))
                .await
                .unwrap(),
            PasswordLoginProgress::Confirmed(_)
        ));
        let requests = f.requests.await.unwrap();
        assert_eq!(requests.len(), 4);
        let (query, body) = request_parts(&requests[2]);
        assert_eq!(query["mid"], device_id);
        assert_eq!(request_parts(&requests[0]).0["mid"], device_id);
        let body: Value = serde_json::from_str(body).unwrap();
        assert_eq!(body["verifykey"], "");
        let secret: Value = serde_json::from_slice(
            &ExchangeCipher::for_test(SEED)
                .decrypt(body["params"].as_str().unwrap())
                .unwrap(),
        )
        .unwrap();
        assert_eq!(secret["verifycode"], proof);
        assert_eq!(secret["pwd"], "browser-password");
        for secret in [
            proof,
            "private-random",
            "private-validate",
            "browser-password",
        ] {
            assert!(!requests[2].contains(secret));
        }
    }
}
