use super::*;
use crate::credential::MiguCredential;
use crate::provider::account_media::tests::{reply, server, setup};
use crate::provider::session::tests::{profile, read, stored};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::oneshot;

fn encrypted(value: serde_json::Value) -> String {
    let body = crate::client::native_http::encode(value.to_string().as_bytes());
    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

fn h5_profile_without_native_session(uid: &str, pacm: &str) -> String {
    let body = json!({"code":"000000","data":{
        "userId":uid,"nickName":"H5 Listener",
        "smallIcon":"https://d.musicapp.migu.cn/avatar.jpg"
    }})
    .to_string();
    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\npacmtoken: {pacm}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

fn native_profile() -> serde_json::Value {
    json!({"code":"000000","userInfoItem":{
        "userId":"111","nickName":"Native Listener","signature":"听歌\n继续听歌",
        "birthday":"2000/02/29","middleIcon":"https://d.musicapp.migu.cn/middle.jpg",
        "smallIcon":"https://d.musicapp.migu.cn/small.jpg","bigIcon":"https://d.musicapp.migu.cn/big.jpg",
        "icon":"https://d.musicapp.migu.cn/icon.jpg","bgpic":"https://d.musicapp.migu.cn/background.jpg",
        "msisdn":"discard-native-phone","passId":"discard-pass-id","accountName":"discard-account-name",
        "usessionId":"discard-native-session","userLevelInfo":{"level":999},"unrelated":"discard-unknown"
    }})
}

fn frames() -> Vec<String> {
    vec![
        profile("111", "pacmtoken: profile-pacm\r\n"),
        reply(
            json!({"code":"000000","data":"native-token-fixture"}),
            Some("exchange-pacm"),
        ),
        profile("111", "pacmtoken: verified-pacm\r\n"),
        encrypted(json!({"code":"000000","data":{"userInfoItem":{"userId":"111"}}})),
        encrypted(native_profile()),
        profile("111", "pacmtoken: final-pacm\r\n"),
    ]
}

#[tokio::test]
async fn modern_profile_reads_native_display_fields_and_rotates_only_the_selected_identity() {
    for selection in ["default", "named", "caller"] {
        let (mut provider, requests) = server(frames()).await;
        let (store, original, alias) = setup(&mut provider, selection);
        let result = provider
            .user_profile("111", UserProfileBackend::Modern, Some(alias))
            .await
            .unwrap();
        assert_eq!(result.user.id, "111");
        assert_eq!(result.user.name, "Native Listener");
        assert_eq!(result.user.signature.as_deref(), Some("听歌\n继续听歌"));
        assert_eq!(result.birthday.as_deref(), Some("2000/02/29"));
        assert_eq!(
            result.user.avatar_url.as_deref(),
            Some("https://d.musicapp.migu.cn/middle.jpg")
        );
        assert_eq!(
            result.background_url.as_deref(),
            Some("https://d.musicapp.migu.cn/background.jpg")
        );
        assert!(
            result.level.is_none()
                && result.playlist_count.is_none()
                && result.public_listening_history.is_none()
        );
        let output = serde_json::to_string(&result).unwrap();
        for secret in ["pacm", "native-token-fixture", "do-not-retain", "discard-"] {
            assert!(!output.contains(secret));
        }
        assert_eq!(read(&store, "other").token(), "unrelated-pacm");
        if selection == "caller" {
            assert_eq!(read(&store, alias), original);
            let update = provider.take_response_credential().unwrap().unwrap();
            assert_eq!(
                MiguCredential::parse_caller(&update).unwrap(),
                original.rotate("final-pacm".into()).unwrap()
            );
            assert!(!update.secret().contains("native-token-fixture"));
            assert!(provider.take_response_credential().unwrap().is_none());
        } else {
            assert_eq!(
                read(&store, alias),
                original.rotate("final-pacm".into()).unwrap()
            );
            assert!(provider.take_response_credential().unwrap().is_none());
        }
        let wire = requests.await.unwrap();
        assert_eq!(wire.len(), 6);
        assert!(wire.iter().all(|request| request.starts_with("GET ")));
        assert!(wire[4].starts_with("GET /MIGUM3.0/user/user-info/v1.0?userId=111 HTTP/1.1\r\n"));
        for header in [
            "token: native-token-fixture\r\n",
            "signversion: V005\r\n",
            "sign: ",
            "appid: music\r\n",
            "os: Android\r\n",
        ] {
            assert!(wire[4].contains(header));
        }
        for absent in ["pacmtoken:", "cookie:", "requestenc:", "responseenc:"] {
            assert!(!wire[4].contains(absent));
        }
        assert!(wire[0].contains("pacmtoken: initial-pacm\r\n"));
        assert!(wire[5].contains("pacmtoken: verified-pacm\r\n"));
    }
}

#[tokio::test]
async fn modern_profile_falls_back_to_verified_h5_identity_when_native_session_is_absent() {
    let (mut provider, requests) = server(vec![h5_profile_without_native_session(
        "111",
        "profile-pacm",
    )])
    .await;
    let (store, original, alias) = setup(&mut provider, "named");
    let result = provider
        .user_profile("111", UserProfileBackend::Modern, Some(alias))
        .await
        .unwrap();
    assert_eq!(result.user.id, "111");
    assert_eq!(result.user.name, "H5 Listener");
    assert_eq!(
        result.user.avatar_url.as_deref(),
        Some("https://d.musicapp.migu.cn/avatar.jpg")
    );
    assert!(result.user.signature.is_none());
    assert!(result.birthday.is_none());
    assert!(result.background_url.is_none());
    assert_eq!(
        result.extensions.get("backend").and_then(|v| v.as_str()),
        Some("official_h5_user_info")
    );
    assert_eq!(
        read(&store, alias),
        original.rotate("profile-pacm".into()).unwrap()
    );
    let output = serde_json::to_string(&result).unwrap();
    for forbidden in ["pacm", "do-not-retain", "phone", "native-session"] {
        assert!(!output.contains(forbidden));
    }
    assert_eq!(requests.await.unwrap().len(), 1);
}

#[tokio::test]
async fn modern_profile_preserves_unknowns_and_official_icon_precedence() {
    for (item, expected) in [
        (json!({"userId":"111"}), None),
        (
            json!({"userId":"111","signature":null,"birthday":"","middleIcon":"","smallIcon":"https://d.musicapp.migu.cn/small.jpg","icon":"https://d.musicapp.migu.cn/icon.jpg"}),
            Some("https://d.musicapp.migu.cn/small.jpg"),
        ),
        (
            json!({"userId":"111","middleIcon":"","smallIcon":"","bigIcon":null,"icon":"https://d.musicapp.migu.cn/icon.jpg"}),
            Some("https://d.musicapp.migu.cn/icon.jpg"),
        ),
        (
            json!({"userId":"111","middleIcon":"https://example.invalid/image.jpg","smallIcon":"https://d.musicapp.migu.cn/small.jpg","bgpic":"http://d.musicapp.migu.cn/background.jpg"}),
            None,
        ),
        (
            json!({"userId":"111","middleIcon":"https://secret@d.musicapp.migu.cn/image.jpg","bgpic":"https://d.musicapp.migu.cn:8443/background.jpg"}),
            None,
        ),
    ] {
        let mut responses = frames();
        responses[4] = encrypted(json!({"code":"000000","userInfoItem":item}));
        let (mut provider, requests) = server(responses).await;
        let (_, _, alias) = setup(&mut provider, "named");
        let result = provider
            .user_profile("111", UserProfileBackend::Modern, Some(alias))
            .await
            .unwrap();
        assert_eq!(result.user.avatar_url.as_deref(), expected);
        assert!(
            result.user.signature.is_none()
                && result.birthday.is_none()
                && result.background_url.is_none()
        );
        assert!(
            result.level.is_none()
                && result.following_count.is_none()
                && result.follower_count.is_none()
        );
        assert_eq!(requests.await.unwrap().len(), 6);
    }
}

#[tokio::test]
async fn modern_profile_rejects_bad_native_or_final_identity_and_keeps_only_verified_rotations() {
    for (at, response, code) in [
        (
            3,
            encrypted(json!({"code":"000000","data":{"userInfoItem":{"userId":"222"}}})),
            ErrorCode::PermissionDenied,
        ),
        (
            4,
            encrypted(json!({"code":"000000","userInfoItem":{"userId":"222"}})),
            ErrorCode::PermissionDenied,
        ),
        (
            4,
            encrypted(json!({"code":"000000","data":{"userInfoItem":{"userId":"111"}}})),
            ErrorCode::UpstreamError,
        ),
        (
            4,
            encrypted(json!({"code":0,"userInfoItem":{"userId":"111"}})),
            ErrorCode::UpstreamError,
        ),
        (
            4,
            encrypted(json!({"code":"000000","userInfoItem":{"userId":"111","signature":{}}})),
            ErrorCode::UpstreamError,
        ),
        (
            4,
            encrypted(json!({"code":"000000","userInfoItem":{"userId":"111","birthday":20000229}})),
            ErrorCode::UpstreamError,
        ),
        (
            4,
            encrypted(
                json!({"code":"000000","userInfoItem":{"userId":"111","nickName":"bad\u{0}name"}}),
            ),
            ErrorCode::UpstreamError,
        ),
        (
            4,
            encrypted(
                json!({"code":"000000","userInfoItem":{"userId":"111","signature":"x".repeat(4097)}}),
            ),
            ErrorCode::UpstreamError,
        ),
        (
            4,
            "HTTP/1.1 429 Too Many Requests\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                .into(),
            ErrorCode::RateLimited,
        ),
        (5, profile("222", ""), ErrorCode::AuthenticationRequired),
    ] {
        let mut responses = frames();
        responses[at] = response;
        responses.truncate(at + 1);
        let (mut provider, requests) = server(responses).await;
        let (store, original, alias) = setup(&mut provider, "caller");
        let mut failure = provider
            .user_profile("111", UserProfileBackend::Modern, Some(alias))
            .await
            .unwrap_err();
        assert_eq!(failure.code, code, "at {at}");
        assert!(!failure.message.contains("native-token-fixture"));
        assert!(!failure.details.to_string().contains("native-token-fixture"));
        if code == ErrorCode::AuthenticationRequired {
            assert!(provider.take_response_credential().unwrap().is_none());
            assert!(failure.take_caller_credential_update().is_none());
        } else {
            let expected = original.rotate("verified-pacm".into()).unwrap();
            assert_eq!(
                MiguCredential::parse_caller(
                    &provider.take_response_credential().unwrap().unwrap()
                )
                .unwrap(),
                expected
            );
            assert_eq!(
                MiguCredential::parse_caller(&failure.take_caller_credential_update().unwrap())
                    .unwrap(),
                expected
            );
            assert!(provider.take_response_credential().unwrap().is_none());
            assert!(failure.take_caller_credential_update().is_none());
        }
        assert_eq!(read(&store, alias), original);
        assert_eq!(requests.await.unwrap().len(), at + 1);
    }
}

#[tokio::test]
async fn modern_profile_never_reflects_intermediate_or_final_authorization_material() {
    for (key, value, final_read) in [
        ("nickName", "initial-pacm", false),
        ("signature", "hello native-token-fixture", false),
        ("signature", "do-not-retain-session", false),
        ("birthday", "exchange-pacm", false),
        (
            "bgpic",
            "https://d.musicapp.migu.cn/image.jpg?x=native%2Dtoken%2Dfixture",
            false,
        ),
        (
            "middleIcon",
            "https://d.musicapp.migu.cn/native%2Dtoken%2Dfixture.jpg",
            false,
        ),
        ("signature", "final-pacm", true),
    ] {
        let mut value_json = native_profile();
        value_json["userInfoItem"][key] = json!(value);
        let mut responses = frames();
        responses[4] = encrypted(value_json);
        if !final_read {
            responses.truncate(5);
        }
        let (mut provider, requests) = server(responses).await;
        let (store, original, alias) = setup(&mut provider, "caller");
        let failure = provider
            .user_profile("111", UserProfileBackend::Modern, Some(alias))
            .await
            .unwrap_err();
        assert_eq!(failure.code, ErrorCode::UpstreamError);
        assert!(!failure.message.contains(value));
        assert!(!failure.details.to_string().contains(value));
        let update = provider.take_response_credential().unwrap().unwrap();
        assert_eq!(
            MiguCredential::parse_caller(&update).unwrap().token(),
            if final_read {
                "final-pacm"
            } else {
                "verified-pacm"
            }
        );
        assert!(!update.secret().contains("native-token-fixture"));
        assert_eq!(read(&store, alias), original);
        assert_eq!(
            requests.await.unwrap().len(),
            if final_read { 6 } else { 5 }
        );
    }
}

struct Gate {
    provider: MiguProvider,
    seen: oneshot::Receiver<()>,
    release: oneshot::Sender<()>,
    task: tokio::task::JoinHandle<()>,
}

async fn gated_server(at: usize) -> Gate {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let origin = url::Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
    let (seen_tx, seen) = oneshot::channel();
    let (release, release_rx) = oneshot::channel();
    let task = tokio::spawn(async move {
        let mut channels = Some((seen_tx, release_rx));
        for (i, response) in frames().into_iter().take(at + 1).enumerate() {
            tokio::time::timeout(Duration::from_secs(10), async {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut bytes = Vec::new();
                while !bytes.windows(4).any(|part| part == b"\r\n\r\n") {
                    let mut buffer = [0; 4096];
                    let n = socket.read(&mut buffer).await.unwrap();
                    assert!(n > 0);
                    bytes.extend_from_slice(&buffer[..n]);
                    assert!(bytes.len() < 65_536);
                }
                assert!(bytes.starts_with(b"GET "));
                if i == at {
                    let (seen_tx, release_rx) = channels.take().unwrap();
                    seen_tx.send(()).unwrap();
                    release_rx.await.unwrap();
                }
                socket.write_all(response.as_bytes()).await.unwrap();
                socket.shutdown().await.unwrap();
            })
            .await
            .unwrap();
        }
    });
    Gate {
        provider: MiguProvider::from_client(
            MiguClient::test_client().with_catalog_test_origin(origin),
        ),
        seen,
        release,
        task,
    }
}

#[tokio::test]
async fn modern_profile_checks_login_generation_at_every_network_boundary() {
    for at in 0..6 {
        let mut h = gated_server(at).await;
        let (store, _, alias) = setup(&mut h.provider, "named");
        let provider = h.provider.clone();
        let task = tokio::spawn(async move {
            provider
                .user_profile("111", UserProfileBackend::Modern, Some(alias))
                .await
        });
        tokio::time::timeout(Duration::from_secs(5), h.seen)
            .await
            .unwrap()
            .unwrap();
        let replacement =
            MiguCredential::verified("111".into(), "replacement-login".into()).unwrap();
        store.put(&stored(alias, &replacement)).unwrap();
        h.release.send(()).unwrap();
        assert_eq!(task.await.unwrap().unwrap_err().code, ErrorCode::Conflict);
        assert_eq!(read(&store, alias), replacement);
        h.task.await.unwrap();
    }
}

#[tokio::test]
async fn modern_profile_cancel_clears_undelivered_caller_rotations_at_every_boundary() {
    for at in 0..6 {
        let mut h = gated_server(at).await;
        let (store, original, alias) = setup(&mut h.provider, "caller");
        let provider = h.provider.clone();
        let task = tokio::spawn(async move {
            provider
                .user_profile("111", UserProfileBackend::Modern, Some(alias))
                .await
        });
        tokio::time::timeout(Duration::from_secs(5), h.seen)
            .await
            .unwrap()
            .unwrap();
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        assert!(h.provider.take_response_credential().unwrap().is_none());
        assert_eq!(read(&store, alias), original);
        h.task.abort();
    }
}

#[tokio::test]
async fn modern_profile_rejects_invalid_identity_missing_account_and_legacy_before_network() {
    for (uid, backend, account, code) in [
        (
            "222",
            UserProfileBackend::Modern,
            "personal",
            ErrorCode::PermissionDenied,
        ),
        (
            "../111",
            UserProfileBackend::Modern,
            "personal",
            ErrorCode::InvalidRequest,
        ),
        (
            "111",
            UserProfileBackend::Modern,
            "missing",
            ErrorCode::AuthenticationRequired,
        ),
        (
            "111",
            UserProfileBackend::Legacy,
            "personal",
            ErrorCode::CapabilityNotSupported,
        ),
    ] {
        let (mut provider, requests) = server(Vec::new()).await;
        setup(&mut provider, "named");
        let failure = provider
            .user_profile(uid, backend, Some(account))
            .await
            .unwrap_err();
        assert_eq!(failure.code, code);
        assert!(requests.await.unwrap().is_empty());
    }
}
