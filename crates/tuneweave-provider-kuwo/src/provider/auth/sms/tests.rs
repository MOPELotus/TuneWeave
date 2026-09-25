use super::*;
use crate::client::{
    catalog::tests::response,
    native::tests::{credential_fixture, encrypted, requests},
};
use crate::provider::auth::tests::{
    Store, device, login, received, replies, seed, setup, stored, valid,
};
use std::sync::atomic::Ordering;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::Notify,
};
use tuneweave_core::{AuthChallengeProgress, ChallengeMethod};
use url::Url;

const PHONE: &str = "13800000000";
const CODE: &str = "24680";
const SID: &str = "native-sms-session";
fn request(account: &str) -> AuthChallengeRequest {
    AuthChallengeRequest {
        account: account.into(),
        principal: PHONE.into(),
        method: ChallengeMethod::Sms,
        backend: AuthChallengeBackend::Standard,
        country_code: Some("86".into()),
        allow_account_creation: true,
        accept_platform_policies: false,
    }
}
fn sent() -> Vec<u8> {
    encrypted(&json!({"status":200,"tm":"sms-server-receipt"}))
}
fn failed() -> Vec<u8> {
    response(503, "application/json", "", b"private-phone-or-code")
}
fn redacted(error: &TuneWeaveError) {
    let text = format!("{error:?} {error}");
    for secret in [
        PHONE,
        CODE,
        SID,
        "sms-server-receipt",
        "private-phone-or-code",
    ] {
        assert!(!text.contains(secret));
    }
}
fn expire_cooldown(provider: &KuwoProvider) {
    for until in provider
        .auth_registry
        .lock()
        .unwrap()
        .sms_cooldowns
        .values_mut()
    {
        *until = Instant::now() - Duration::from_secs(1);
    }
}

fn web_response(status: &str, mime: &str, headers: &str, body: &[u8]) -> Vec<u8> {
    let mut wire = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {mime}\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )
    .into_bytes();
    wire.extend_from_slice(body);
    wire
}

fn web_json(body: &str, headers: &str) -> Vec<u8> {
    web_response("200 OK", "application/json", headers, body.as_bytes())
}

async fn web_server(frames: Vec<Vec<u8>>) -> (Url, tokio::task::JoinHandle<Vec<String>>) {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let origin = Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
    let task = tokio::spawn(async move {
        let mut requests = Vec::new();
        for frame in frames {
            let (mut socket, _) = tokio::time::timeout(Duration::from_secs(5), listener.accept())
                .await
                .unwrap()
                .unwrap();
            let mut bytes = Vec::new();
            let mut buffer = [0; 2048];
            loop {
                let count = socket.read(&mut buffer).await.unwrap();
                assert!(count > 0);
                bytes.extend_from_slice(&buffer[..count]);
                assert!(bytes.len() <= 32 * 1024);
                if let Some(end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
                    let headers = std::str::from_utf8(&bytes[..end]).unwrap();
                    let body_len = headers
                        .lines()
                        .find_map(|line| {
                            let (name, value) = line.split_once(':')?;
                            name.eq_ignore_ascii_case("content-length")
                                .then(|| value.trim().parse::<usize>().unwrap())
                        })
                        .unwrap_or(0);
                    let total = end + 4 + body_len;
                    while bytes.len() < total {
                        let count = socket.read(&mut buffer).await.unwrap();
                        assert!(count > 0);
                        bytes.extend_from_slice(&buffer[..count]);
                    }
                    break;
                }
            }
            requests.push(String::from_utf8(bytes).unwrap());
            socket.write_all(&frame).await.unwrap();
        }
        requests
    });
    (origin, task)
}

#[tokio::test]
async fn sms_honors_all_three_modes_and_only_returns_conditionally_committed_credentials() {
    for existing in [false, true] {
        for mode in [
            CredentialMode::Server,
            CredentialMode::Client,
            CredentialMode::Both,
        ] {
            let account = if mode == CredentialMode::Client {
                "default"
            } else {
                "personal"
            };
            let store = Arc::new(Store::default());
            let other = seed(&store, "other", "7", "other-session")
                .stored("other")
                .unwrap();
            if existing {
                seed(&store, account, "8", "prior-session");
            }
            let prior = stored(&store, account);
            if mode == CredentialMode::Client {
                store.forbid_reads.store(true, Ordering::SeqCst);
            }
            let mut f = setup(
                replies(vec![device(), sent(), login(42, SID), valid()]),
                store,
            )
            .await;
            let c = f
                .provider
                .begin_auth_challenge(&request(account), mode)
                .await
                .unwrap();
            assert_eq!(c.request(), &request(account));
            assert_eq!(c.credential_mode(), mode);
            assert_eq!(
                f.provider.auth_challenge_status(&c).await.unwrap(),
                AuthChallengeStatus::Waiting
            );
            assert_eq!(stored(&f.store, account), prior);
            let result = f
                .provider
                .advance_auth_challenge(
                    &c,
                    &tuneweave_core::AuthChallengeAction::SubmitCode { code: CODE.into() },
                )
                .await
                .unwrap();
            let AuthChallengeProgress::Confirmed(result) = result else {
                panic!("login must complete after identity validation")
            };
            assert!(result.profile.authenticated);
            assert_eq!(result.profile.account, account);
            assert_eq!(result.profile.user_id.as_deref(), Some("42"));
            assert_eq!(result.credential.is_some(), mode.returns_to_caller());
            if mode.persists_on_server() {
                assert_ne!(stored(&f.store, account), prior);
                if let Some(caller) = result.credential {
                    assert_eq!(caller.secret(), stored(&f.store, account).unwrap().secret());
                }
            } else {
                assert_eq!(stored(&f.store, account), prior);
                assert_eq!(f.store.reads.load(Ordering::SeqCst), 0);
            }
            assert_eq!(stored(&f.store, "other"), Some(other));
            assert!(f.provider.auth_registry.lock().unwrap().attempts.is_empty());
            assert_eq!(
                f.provider
                    .complete_auth_challenge(&c, CODE)
                    .await
                    .unwrap_err()
                    .code,
                ErrorCode::ResourceNotFound
            );
            requests(&mut f.network, 4).await;
        }
    }
}

#[tokio::test]
async fn web_middle_sms_commits_only_after_web_and_native_identity_checks() {
    const COOKIE: &str = "providerWebSmsCookieA123456789";
    let www_home = web_response(
        "200 OK",
        "text/html",
        &format!(
            "Set-Cookie: {}={COOKIE}; Path=/\r\n",
            crate::client::WEB_SESSION_COOKIE
        ),
        b"<html>anonymous www session</html>",
    );
    let h5_page = web_response("200 OK", "text/html", "", b"<html>official web sms</html>");
    let sent = web_json(
        r#"{"meta":{"code":200},"data":{"status":200,"tm":"1700000000123"}}"#,
        "",
    );
    let logged_in = web_json(
        r#"{"meta":{"code":200},"data":{"result":"succ","uid":"42","sid":"web-sms-session"}}"#,
        "",
    );
    let checked = web_json(r#"{"status":200}"#, "");
    let (origin, web_requests) =
        web_server(vec![www_home, h5_page, sent, logged_in, checked]).await;
    let mut fixture = setup(replies(vec![device(), valid()]), Arc::default()).await;
    fixture.provider.client.set_login_test_origin(origin);
    let mut web = request("default");
    web.backend = AuthChallengeBackend::Middle;
    web.accept_platform_policies = true;
    let challenge = fixture
        .provider
        .begin_auth_challenge(&web, CredentialMode::Client)
        .await
        .unwrap();
    assert_eq!(
        fixture
            .provider
            .auth_challenge_status(&challenge)
            .await
            .unwrap(),
        AuthChallengeStatus::Waiting
    );
    assert_eq!(
        fixture
            .provider
            .complete_auth_challenge(&challenge, "1234")
            .await
            .unwrap_err()
            .code,
        ErrorCode::InvalidRequest
    );
    let result = fixture
        .provider
        .complete_auth_challenge(&challenge, CODE)
        .await
        .unwrap();
    assert_eq!(result.profile.user_id.as_deref(), Some("42"));
    assert!(result.profile.authenticated);
    assert!(result.credential.is_some());
    assert!(
        fixture
            .provider
            .auth_registry
            .lock()
            .unwrap()
            .attempts
            .is_empty()
    );
    let web_seen = web_requests.await.unwrap();
    assert_eq!(web_seen.len(), 5);
    assert!(web_seen[0].starts_with("GET / HTTP"));
    assert!(web_seen[1].starts_with("GET /vip/added/webView/kwOutLogin/index.html HTTP"));
    assert!(web_seen[2].starts_with("GET /vip/manage/hanger?"));
    assert!(web_seen[3].starts_with("GET /vip/manage/hanger?"));
    assert!(web_seen[4].starts_with("POST /api/user/checkLogin HTTP"));
    assert!(web_seen[2].find("\r\nCookie:").is_none());
    assert!(web_seen[3].find("\r\nCookie:").is_none());
    requests(&mut fixture.network, 2).await;
}

#[tokio::test]
async fn forged_or_modified_receipts_cannot_consume_or_redirect_the_original_transaction() {
    let mut f = setup(
        replies(vec![device(), sent(), login(42, SID), valid()]),
        Arc::default(),
    )
    .await;
    let c = f
        .provider
        .begin_auth_challenge(&request("personal"), CredentialMode::Both)
        .await
        .unwrap();
    let mut bad = Vec::new();
    for index in 0..6 {
        let mut input = c.request().clone();
        match index {
            0 => input.account = "other".into(),
            1 => input.principal = "13900000000".into(),
            2 => input.allow_account_creation = false,
            3 => input.country_code = Some("+86".into()),
            4 => input.backend = AuthChallengeBackend::Middle,
            _ => input.country_code = None,
        }
        bad.push(
            ProviderAuthChallenge::stateful(
                Platform::Kuwo,
                input,
                CredentialMode::Both,
                c.provider_transaction_id().unwrap().into(),
            )
            .unwrap(),
        );
    }
    bad.push(
        ProviderAuthChallenge::stateful(
            Platform::Kuwo,
            c.request().clone(),
            CredentialMode::Server,
            c.provider_transaction_id().unwrap().into(),
        )
        .unwrap(),
    );
    bad.push(
        ProviderAuthChallenge::stateful(
            Platform::Kuwo,
            c.request().clone(),
            CredentialMode::Both,
            "unknown-handle".into(),
        )
        .unwrap(),
    );
    bad.push(
        ProviderAuthChallenge::stateful(
            Platform::Migu,
            c.request().clone(),
            CredentialMode::Both,
            c.provider_transaction_id().unwrap().into(),
        )
        .unwrap(),
    );
    bad.push(ProviderAuthChallenge::stateless(
        Platform::Kuwo,
        c.request().clone(),
        CredentialMode::Both,
    ));
    for forged in bad {
        redacted(
            &f.provider
                .complete_auth_challenge(&forged, CODE)
                .await
                .unwrap_err(),
        );
        assert!(f.provider.auth_challenge_status(&forged).await.is_err());
    }
    let independent = KuwoProvider::from_client(f.provider.client.clone());
    assert_eq!(
        independent
            .complete_auth_challenge(&c, CODE)
            .await
            .unwrap_err()
            .code,
        ErrorCode::ResourceNotFound
    );
    assert_eq!(
        f.provider.auth_challenge_status(&c).await.unwrap(),
        AuthChallengeStatus::Waiting
    );
    f.provider.complete_auth_challenge(&c, CODE).await.unwrap();
    requests(&mut f.network, 4).await;
}

#[tokio::test]
async fn cooldown_covers_aliases_and_modes_and_explicit_resend_retires_the_old_phone_receipt() {
    let mut f = setup(
        replies(vec![device(), sent(), sent(), login(42, SID), valid()]),
        Arc::default(),
    )
    .await;
    let old = f
        .provider
        .begin_auth_challenge(&request("first"), CredentialMode::Server)
        .await
        .unwrap();
    for (account, mode) in [
        ("other", CredentialMode::Both),
        ("default", CredentialMode::Client),
    ] {
        let error = f
            .provider
            .begin_auth_challenge(&request(account), mode)
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::RateLimited);
        assert!((1..=60).contains(&error.details["retry_after_secs"].as_u64().unwrap()));
        redacted(&error);
    }
    expire_cooldown(&f.provider);
    let new = f
        .provider
        .begin_auth_challenge(&request("other"), CredentialMode::Both)
        .await
        .unwrap();
    assert_ne!(old.provider_transaction_id(), new.provider_transaction_id());
    assert!(
        f.provider
            .complete_auth_challenge(&old, CODE)
            .await
            .is_err()
    );
    f.provider
        .complete_auth_challenge(&new, CODE)
        .await
        .unwrap();
    assert!(stored(&f.store, "first").is_none());
    assert!(stored(&f.store, "other").is_some());
    requests(&mut f.network, 5).await;
}

#[tokio::test]
async fn every_network_boundary_rejects_late_success_or_error_after_replacement_logout_or_expiry() {
    for boundary in 0..4 {
        for action in 0..3 {
            for late_error in [false, true] {
                let gate = Arc::new(Notify::new());
                let mut bodies = vec![device(), sent(), login(42, SID), valid()];
                if late_error {
                    bodies[boundary] = failed();
                }
                let responses = bodies
                    .into_iter()
                    .enumerate()
                    .take(boundary + 1)
                    .map(|(i, b)| (b, (i == boundary).then(|| gate.clone())))
                    .collect();
                let store = Arc::new(Store::default());
                seed(&store, "personal", "7", "original-session");
                let mut f = setup(responses, store).await;
                let p = f.provider.clone();
                let task = tokio::spawn(async move {
                    let c = p
                        .begin_auth_challenge(&request("personal"), CredentialMode::Both)
                        .await?;
                    p.complete_auth_challenge(&c, CODE).await
                });
                for _ in 0..=boundary {
                    received(&mut f).await;
                }
                match action {
                    0 => {
                        seed(&f.store, "personal", "9", "new-login-session");
                    }
                    1 => {
                        f.provider.logout("personal").await.unwrap();
                    }
                    _ => {
                        for a in f
                            .provider
                            .auth_registry
                            .lock()
                            .unwrap()
                            .attempts
                            .values_mut()
                        {
                            a.deadline = Instant::now() - Duration::from_secs(1);
                        }
                    }
                }
                let expected = stored(&f.store, "personal");
                gate.notify_one();
                let error = task.await.unwrap().unwrap_err();
                assert_eq!(
                    error.code,
                    ErrorCode::Conflict,
                    "boundary{boundary} action{action} error{late_error}"
                );
                redacted(&error);
                assert_eq!(stored(&f.store, "personal"), expected);
                assert!(f.provider.auth_registry.lock().unwrap().attempts.is_empty());
                requests(&mut f.network, 0).await;
            }
        }
    }
}

#[tokio::test]
async fn cancellation_releases_each_boundary_but_keeps_uncertain_delivery_cooldowns() {
    for boundary in 0..4 {
        let gate = Arc::new(Notify::new());
        let responses = [device(), sent(), login(42, SID), valid()]
            .into_iter()
            .enumerate()
            .take(boundary + 1)
            .map(|(i, b)| (b, (i == boundary).then(|| gate.clone())))
            .collect();
        let store = Arc::new(Store::default());
        let original = seed(&store, "personal", "7", "old-session")
            .stored("personal")
            .unwrap();
        let mut f = setup(responses, store).await;
        let p = f.provider.clone();
        let task = tokio::spawn(async move {
            let c = p
                .begin_auth_challenge(&request("personal"), CredentialMode::Both)
                .await?;
            p.complete_auth_challenge(&c, CODE).await
        });
        for _ in 0..=boundary {
            received(&mut f).await;
        }
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        assert!(f.provider.auth_registry.lock().unwrap().attempts.is_empty());
        assert_eq!(stored(&f.store, "personal"), Some(original));
        if boundary > 0 {
            assert_eq!(
                f.provider
                    .begin_auth_challenge(&request("default"), CredentialMode::Client)
                    .await
                    .unwrap_err()
                    .code,
                ErrorCode::RateLimited
            );
        } else {
            assert!(
                f.provider
                    .auth_registry
                    .lock()
                    .unwrap()
                    .sms_cooldowns
                    .is_empty()
            );
        }
        gate.notify_one();
        requests(&mut f.network, 0).await;
    }
}

#[tokio::test]
async fn invalid_requests_caller_scope_and_shared_capacity_fail_before_device_or_sms_io() {
    let mut f = setup(vec![], Arc::default()).await;
    let mut cases = Vec::new();
    let mut c = request("personal");
    c.allow_account_creation = false;
    cases.push((c, CredentialMode::Both));
    for phone in ["", "+8613800000000", "12800000000", "13800000000\n"] {
        let mut c = request("personal");
        c.principal = phone.into();
        cases.push((c, CredentialMode::Both));
    }
    let mut c = request("personal");
    c.backend = AuthChallengeBackend::Middle;
    cases.push((c, CredentialMode::Both));
    let mut c = request("personal");
    c.accept_platform_policies = true;
    cases.push((c, CredentialMode::Both));
    let mut c = request("personal");
    c.backend = AuthChallengeBackend::Middle;
    c.accept_platform_policies = true;
    c.allow_account_creation = false;
    cases.push((c, CredentialMode::Both));
    let mut c = request("personal");
    c.country_code = Some("1".into());
    cases.push((c, CredentialMode::Both));
    cases.push((request(" named"), CredentialMode::Server));
    cases.push((request("named"), CredentialMode::Client));
    for (c, mode) in cases {
        assert_eq!(
            f.provider
                .begin_auth_challenge(&c, mode)
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
    }
    let mut accepted_web = request("personal");
    accepted_web.backend = AuthChallengeBackend::Middle;
    accepted_web.accept_platform_policies = true;
    assert!(super::validate_web_sms_request(&accepted_web).is_ok());
    let caller = f
        .provider
        .with_caller_credential(&credential_fixture("7", "prior-session").caller().unwrap())
        .unwrap();
    assert_eq!(
        caller
            .begin_auth_challenge(&request("default"), CredentialMode::Client)
            .await
            .unwrap_err()
            .code,
        ErrorCode::InvalidRequest
    );
    let mut no_store = f.provider.clone();
    no_store.credential_store = None;
    assert_eq!(
        no_store
            .begin_auth_challenge(&request("personal"), CredentialMode::Both)
            .await
            .unwrap_err()
            .code,
        ErrorCode::InternalError
    );
    let mut leases = Vec::new();
    for _ in 0..CAPACITY {
        leases.push(
            f.provider
                .reserve("default", CredentialMode::Client)
                .unwrap()
                .0,
        );
    }
    assert_eq!(
        f.provider
            .begin_auth_challenge(&request("default"), CredentialMode::Client)
            .await
            .unwrap_err()
            .code,
        ErrorCode::RateLimited
    );
    drop(leases);
    {
        let mut registry = f.provider.auth_registry.lock().unwrap();
        for i in 0..COOLDOWN_CAPACITY {
            registry
                .sms_cooldowns
                .insert(format!("recent-{i}"), Instant::now() + COOLDOWN);
        }
    }
    assert_eq!(
        f.provider
            .begin_auth_challenge(&request("default"), CredentialMode::Client)
            .await
            .unwrap_err()
            .code,
        ErrorCode::RateLimited
    );
    assert!(f.provider.auth_registry.lock().unwrap().attempts.is_empty());
    requests(&mut f.network, 0).await;
}

#[tokio::test]
async fn failed_send_and_failed_conditional_commit_do_not_export_credentials_or_erase_prior_alias()
{
    let mut f = setup(replies(vec![device(), failed()]), Arc::default()).await;
    assert!(
        f.provider
            .begin_auth_challenge(&request("personal"), CredentialMode::Both)
            .await
            .is_err()
    );
    assert_eq!(
        f.provider
            .begin_auth_challenge(&request("other"), CredentialMode::Server)
            .await
            .unwrap_err()
            .code,
        ErrorCode::RateLimited
    );
    assert!(f.provider.auth_registry.lock().unwrap().attempts.is_empty());
    requests(&mut f.network, 2).await;
    for existing in [false, true] {
        for storage_error in [false, true] {
            let store = Arc::new(Store::default());
            if existing {
                seed(&store, "personal", "7", "old-session");
            }
            let old = stored(&store, "personal");
            let mut f = setup(
                replies(vec![device(), sent(), login(42, SID), valid()]),
                store,
            )
            .await;
            let c = f
                .provider
                .begin_auth_challenge(&request("personal"), CredentialMode::Both)
                .await
                .unwrap();
            if storage_error {
                f.store.fail_write.store(true, Ordering::SeqCst);
            } else {
                f.store.reject_write.store(true, Ordering::SeqCst);
            }
            let error = f
                .provider
                .complete_auth_challenge(&c, CODE)
                .await
                .unwrap_err();
            redacted(&error);
            assert_eq!(stored(&f.store, "personal"), old);
            assert!(f.provider.auth_registry.lock().unwrap().attempts.is_empty());
            requests(&mut f.network, 4).await;
        }
    }
}

#[tokio::test]
async fn only_one_verification_can_claim_a_receipt_and_status_never_reports_a_busy_transaction_as_waiting()
 {
    let gate = Arc::new(Notify::new());
    let mut f = setup(
        vec![
            (device(), None),
            (sent(), None),
            (login(42, SID), Some(gate.clone())),
            (valid(), None),
        ],
        Arc::default(),
    )
    .await;
    let c = f
        .provider
        .begin_auth_challenge(&request("personal"), CredentialMode::Both)
        .await
        .unwrap();
    assert_eq!(
        f.provider
            .complete_auth_challenge(&c, "1234")
            .await
            .unwrap_err()
            .code,
        ErrorCode::InvalidRequest
    );
    let p = f.provider.clone();
    let c2 = c.clone();
    let task = tokio::spawn(async move { p.complete_auth_challenge(&c2, CODE).await });
    for _ in 0..3 {
        received(&mut f).await;
    }
    assert_eq!(
        f.provider.auth_challenge_status(&c).await.unwrap_err().code,
        ErrorCode::Conflict
    );
    assert_eq!(
        f.provider
            .complete_auth_challenge(&c, CODE)
            .await
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
    expire_cooldown(&f.provider);
    assert_eq!(
        f.provider
            .begin_auth_challenge(&request("other"), CredentialMode::Both)
            .await
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
    gate.notify_one();
    task.await.unwrap().unwrap();
    assert_eq!(
        f.provider.auth_challenge_status(&c).await.unwrap_err().code,
        ErrorCode::ResourceNotFound
    );
    requests(&mut f.network, 1).await;
}

#[tokio::test]
async fn two_phone_transactions_complete_in_reverse_order_and_share_no_account_or_receipt_state() {
    let mut f = setup(
        replies(vec![
            device(),
            sent(),
            sent(),
            login(43, "second-session"),
            valid(),
            login(42, SID),
            valid(),
        ]),
        Arc::default(),
    )
    .await;
    let first = f
        .provider
        .begin_auth_challenge(&request("first"), CredentialMode::Both)
        .await
        .unwrap();
    let mut second = request("second");
    second.principal = "13900000000".into();
    let second = f
        .provider
        .begin_auth_challenge(&second, CredentialMode::Server)
        .await
        .unwrap();
    let second_result = f
        .provider
        .complete_auth_challenge(&second, "13579")
        .await
        .unwrap();
    let first_result = f
        .provider
        .complete_auth_challenge(&first, CODE)
        .await
        .unwrap();
    assert_eq!(second_result.profile.user_id.as_deref(), Some("43"));
    assert!(second_result.credential.is_none());
    assert_eq!(first_result.profile.user_id.as_deref(), Some("42"));
    assert!(first_result.credential.is_some());
    assert_ne!(
        stored(&f.store, "first").unwrap().secret(),
        stored(&f.store, "second").unwrap().secret()
    );
    requests(&mut f.network, 7).await;
}

#[tokio::test]
async fn sms_and_password_commits_cancel_other_persisted_attempts_for_the_same_alias() {
    let mut f = setup(
        replies(vec![device(), sent(), login(42, SID), valid()]),
        Arc::default(),
    )
    .await;
    let c = f
        .provider
        .begin_auth_challenge(&request("personal"), CredentialMode::Both)
        .await
        .unwrap();
    let (lease, previous) = f
        .provider
        .reserve("personal", CredentialMode::Server)
        .unwrap();
    f.provider.complete_auth_challenge(&c, CODE).await.unwrap();
    assert!(
        f.provider
            .check_attempt(
                &mut f.provider.auth_registry.lock().unwrap(),
                &lease,
                previous.as_ref()
            )
            .is_err()
    );
    drop(lease);
    requests(&mut f.network, 4).await;
    let mut f = setup(
        replies(vec![
            device(),
            sent(),
            login(43, "password-session"),
            valid(),
        ]),
        Arc::default(),
    )
    .await;
    let c = f
        .provider
        .begin_auth_challenge(&request("personal"), CredentialMode::Both)
        .await
        .unwrap();
    let result = f
        .provider
        .password_login_with_mode(
            &crate::provider::auth::tests::request("personal"),
            CredentialMode::Both,
        )
        .await
        .unwrap();
    assert_eq!(result.profile.user_id.as_deref(), Some("43"));
    assert_eq!(
        f.provider
            .complete_auth_challenge(&c, CODE)
            .await
            .unwrap_err()
            .code,
        ErrorCode::ResourceNotFound
    );
    requests(&mut f.network, 4).await;
}

#[tokio::test]
async fn independent_provider_instances_cannot_both_commit_the_same_sms_alias() {
    for existing in [false, true] {
        let store = Arc::new(Store::default());
        if existing {
            seed(&store, "personal", "7", "prior-session");
        }
        let first = setup(
            replies(vec![device(), sent(), login(42, SID), valid()]),
            store.clone(),
        )
        .await;
        let second = setup(
            replies(vec![device(), sent(), login(43, "other-session"), valid()]),
            store.clone(),
        )
        .await;
        let a = first
            .provider
            .begin_auth_challenge(&request("personal"), CredentialMode::Both)
            .await
            .unwrap();
        let b = second
            .provider
            .begin_auth_challenge(&request("personal"), CredentialMode::Both)
            .await
            .unwrap();
        let (a, b) = tokio::join!(
            first.provider.complete_auth_challenge(&a, CODE),
            second.provider.complete_auth_challenge(&b, CODE)
        );
        let (winner, loser) = match (a, b) {
            (Ok(w), Err(l)) | (Err(l), Ok(w)) => (w, l),
            _ => panic!("exactly one conditional commit must succeed"),
        };
        assert_eq!(loser.code, ErrorCode::Conflict);
        assert_eq!(
            winner.credential.unwrap().secret(),
            stored(&store, "personal").unwrap().secret()
        );
        // A losing source check may stop before its remaining prepared replies.
        first.network.server.abort();
        second.network.server.abort();
        assert!(
            first
                .provider
                .auth_registry
                .lock()
                .unwrap()
                .attempts
                .is_empty()
        );
        assert!(
            second
                .provider
                .auth_registry
                .lock()
                .unwrap()
                .attempts
                .is_empty()
        );
    }
}

#[tokio::test]
async fn absent_alias_logout_and_idle_source_changes_retire_pending_sms_without_network() {
    for action in 0..3 {
        let mut f = setup(replies(vec![device(), sent()]), Arc::default()).await;
        let c = f
            .provider
            .begin_auth_challenge(&request("personal"), CredentialMode::Both)
            .await
            .unwrap();
        match action {
            0 => {
                assert!(!f.provider.logout("personal").await.unwrap());
            }
            1 => {
                seed(&f.store, "personal", "9", "new-session");
            }
            _ => {
                for a in f
                    .provider
                    .auth_registry
                    .lock()
                    .unwrap()
                    .attempts
                    .values_mut()
                {
                    a.deadline = Instant::now() - Duration::from_secs(1);
                }
            }
        }
        assert!(f.provider.auth_challenge_status(&c).await.is_err());
        assert!(f.provider.complete_auth_challenge(&c, CODE).await.is_err());
        assert!(f.provider.auth_registry.lock().unwrap().attempts.is_empty());
        requests(&mut f.network, 2).await;
    }
}

#[tokio::test]
async fn preparing_and_waiting_sms_count_toward_capacity_and_preparing_phone_cannot_reenter() {
    let gate = Arc::new(Notify::new());
    let mut f = setup(
        vec![(device(), Some(gate.clone())), (sent(), None)],
        Arc::default(),
    )
    .await;
    let p = f.provider.clone();
    let task = tokio::spawn(async move {
        p.begin_auth_challenge(&request("personal"), CredentialMode::Both)
            .await
    });
    received(&mut f).await;
    assert!(
        f.provider
            .auth_registry
            .lock()
            .unwrap()
            .sms_cooldowns
            .is_empty()
    );
    assert_eq!(
        f.provider
            .begin_auth_challenge(&request("other"), CredentialMode::Both)
            .await
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
    let mut other = request("other");
    other.principal = "13900000000".into();
    let mut leases = Vec::new();
    for _ in 1..CAPACITY {
        leases.push(
            f.provider
                .reserve("default", CredentialMode::Client)
                .unwrap()
                .0,
        );
    }
    assert_eq!(
        f.provider.auth_registry.lock().unwrap().attempts.len(),
        CAPACITY
    );
    assert_eq!(
        f.provider
            .begin_auth_challenge(&other, CredentialMode::Both)
            .await
            .unwrap_err()
            .code,
        ErrorCode::RateLimited
    );
    drop(leases);
    gate.notify_one();
    let challenge = task.await.unwrap().unwrap();
    assert_eq!(
        f.provider.auth_challenge_status(&challenge).await.unwrap(),
        AuthChallengeStatus::Waiting
    );
    let mut leases = Vec::new();
    for _ in 1..CAPACITY {
        leases.push(
            f.provider
                .reserve("default", CredentialMode::Client)
                .unwrap()
                .0,
        );
    }
    assert_eq!(
        f.provider
            .begin_auth_challenge(&other, CredentialMode::Both)
            .await
            .unwrap_err()
            .code,
        ErrorCode::RateLimited
    );
    drop(leases);
    assert!(!f.provider.logout("personal").await.unwrap());
    assert!(f.provider.auth_registry.lock().unwrap().attempts.is_empty());
    requests(&mut f.network, 1).await;
}

#[tokio::test]
async fn timeouts_at_all_network_boundaries_consume_attempts_without_changing_the_stored_session() {
    for boundary in 0..4 {
        let gate = Arc::new(Notify::new());
        let responses = [device(), sent(), login(42, SID), valid()]
            .into_iter()
            .enumerate()
            .take(boundary + 1)
            .map(|(i, body)| (body, (i == boundary).then(|| gate.clone())))
            .collect();
        let store = Arc::new(Store::default());
        let old = seed(&store, "personal", "7", "old-session")
            .stored("personal")
            .unwrap();
        let mut f = setup(responses, store).await;
        let p = f.provider.clone();
        let task = tokio::spawn(async move {
            let challenge = p
                .begin_auth_challenge(&request("personal"), CredentialMode::Both)
                .await?;
            p.complete_auth_challenge(&challenge, CODE).await
        });
        for _ in 0..=boundary {
            received(&mut f).await;
        }
        let error = tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::UpstreamTimeout);
        assert!(!error.retryable);
        redacted(&error);
        assert_eq!(stored(&f.store, "personal"), Some(old));
        assert!(f.provider.auth_registry.lock().unwrap().attempts.is_empty());
        gate.notify_one();
        requests(&mut f.network, 0).await;
    }
}
