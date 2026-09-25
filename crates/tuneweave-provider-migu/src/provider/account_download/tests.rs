use super::super::account_media::tests::{reply, server, setup};
use super::super::session::tests::{gated, profile, read, stored};
use super::*;
use crate::credential::MiguCredential;
use tuneweave_core::ResourceRef;

fn validation(uid: &str) -> String {
    let data = json!({"code":"000000","data":{"userInfoItem":{"userId":uid}}}).to_string();
    let body = crate::client::native_http::encode(data.as_bytes());
    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}
fn download() -> String {
    reply(
        json!({"code":"000000","data":{"contentId":"123","copyrightId":"6005971HBUU","formatId":"020007","encryptionType":"0","size":"3450883","suffix":"mp3","url":"https://dlsdownfree.nf.migu.cn/wlansst/song?pars=download-fixture"}}),
        None,
    )
}
fn frames() -> Vec<String> {
    vec![
        profile("111", "pacmtoken: profile-pacm\r\n"),
        reply(
            json!({"code":"000000","data":"native-token-fixture"}),
            Some("exchange-pacm"),
        ),
        profile("111", "pacmtoken: verified-pacm\r\n"),
        validation("111"),
        reply(
            json!({"code":"000000","resource":[{"resourceType":"2","contentId":"123","copyrightId":"6005971HBUU","songId":"456","songName":"Song","singerId":"77","singer":"Artist","length":"03:32","rateFormats":[{"formatType":"PQ","format":"020007"}]}]}),
            None,
        ),
        download(),
        profile("111", "pacmtoken: final-pacm\r\n"),
    ]
}
fn track() -> Track {
    Track::new(
        ResourceRef::new(Platform::Migu, "123").unwrap(),
        "Caller stale title",
    )
}
fn request(alias: &str) -> StreamRequest {
    StreamRequest {
        account: Some(alias.into()),
        quality: Quality::Standard,
        ..Default::default()
    }
}

#[tokio::test]
async fn account_download_default_named_and_caller_exchange_validate_then_download() {
    for mode in ["default", "named", "caller"] {
        let (mut provider, wire) = server(frames()).await;
        let (store, original, alias) = setup(&mut provider, mode);
        let result = provider.download(&track(), &request(alias)).await.unwrap();
        assert!(result.available);
        assert_eq!(result.actual_quality, Quality::Standard);
        assert_eq!(
            result.extensions["backend"],
            "native_account_download_by_songid_v1"
        );
        assert!(result.headers.is_empty());
        let output = serde_json::to_string(&result).unwrap();
        for secret in [
            "native-token-fixture",
            "do-not-retain-session",
            "profile-pacm",
            "exchange-pacm",
            "verified-pacm",
            "final-pacm",
        ] {
            assert!(!output.contains(secret));
        }
        assert_eq!(read(&store, "other").token(), "unrelated-pacm");
        if mode == "caller" {
            assert_eq!(read(&store, alias), original);
            let update = provider.take_response_credential().unwrap().unwrap();
            assert_eq!(
                MiguCredential::parse_caller(&update).unwrap().token(),
                "final-pacm"
            );
            assert!(!update.secret().contains("native-token-fixture"));
            assert!(!update.secret().contains("do-not-retain-session"));
        } else {
            assert_eq!(read(&store, alias).token(), "final-pacm");
        }
        let wire = wire.await.unwrap();
        assert_eq!(wire.len(), 7);
        assert!(
            wire[1].starts_with("GET /user/h5/token/v1.0?uSessionId=do-not-retain-session&_t=")
        );
        assert!(wire[1].contains("pacmtoken: profile-pacm\r\n"));
        assert!(wire[3].starts_with("GET /user/token-validate/v2.0?loginType=1&sourceId=220024&token=native-token-fixture&tokenId=native-token-fixture "));
        assert!(wire[5].starts_with("GET /MIGUM2.0/strategy/download-url/by-songid/v1.0?contentId=123&formatType=PQ&songId=456 "));
        for at in [3, 5] {
            assert!(wire[at].contains("token: native-token-fixture\r\n"));
            assert!(wire[at].contains("signversion: V005\r\n"));
            assert!(wire[at].contains("sign: "));
            for header in [
                "appid: music\r\n",
                "os: Android\r\n",
                "pkgname: cmccwm.mobilemusic\r\n",
                "language: Chinese\r\n",
                "verify: verify\r\n",
            ] {
                assert!(wire[at].contains(header), "missing {header}");
            }
            for forbidden in [
                "pacmtoken",
                "requestenc",
                "responseenc",
                "\r\nuid:",
                "usessionid",
            ] {
                assert!(!wire[at].to_lowercase().contains(forbidden));
            }
        }
        assert!(
            wire.iter()
                .all(|request| !request.contains("/listen") && !request.contains("can-listen"))
        );
    }
}

#[tokio::test]
async fn failed_exchange_or_native_uid_never_reaches_download_or_playback() {
    for (index, replacement, expected) in [
        (
            0,
            reply(json!({"code":"000000","data":{"userId":"111"}}), None),
            ErrorCode::UpstreamError,
        ),
        (
            1,
            reply(
                json!({"code":"000000","data":{"token":"native-token-fixture"}}),
                None,
            ),
            ErrorCode::UpstreamError,
        ),
        (3, validation("222"), ErrorCode::PermissionDenied),
        (3, validation(""), ErrorCode::UpstreamError),
    ] {
        let mut responses = frames();
        responses[index] = replacement;
        // A successful PACM response is verified before rejecting its H5 data.
        responses.truncate(if index == 1 { 3 } else { index + 1 });
        let (mut provider, wire) = server(responses).await;
        let (_, _, alias) = setup(&mut provider, "named");
        let failure = provider
            .download(&track(), &request(alias))
            .await
            .unwrap_err();
        assert_eq!(failure.code, expected);
        assert!(!format!("{failure:?}").contains("native-token-fixture"));
        let wire = wire.await.unwrap();
        assert!(
            wire.iter()
                .all(|request| !request.contains("download-url") && !request.contains("/listen"))
        );
    }
}

#[tokio::test]
async fn account_download_final_uid_change_discards_resource() {
    let mut responses = frames();
    responses[6] = profile("222", "");
    let (mut provider, wire) = server(responses).await;
    let (_, _, alias) = setup(&mut provider, "caller");
    assert_eq!(
        provider
            .download(&track(), &request(alias))
            .await
            .unwrap_err()
            .code,
        ErrorCode::AuthenticationRequired
    );
    assert!(provider.take_response_credential().unwrap().is_none());
    assert_eq!(wire.await.unwrap().len(), 7);
}

#[tokio::test]
async fn account_download_rejects_new_login_generation_before_returning_authorization() {
    let (mut provider, seen, release, wire) = gated(frames()).await;
    let (store, _, alias) = setup(&mut provider, "named");
    let request = request(alias);
    let task = tokio::spawn(async move { provider.download(&track(), &request).await });
    tokio::time::timeout(Duration::from_secs(5), seen)
        .await
        .expect("download test did not reach its final profile request")
        .unwrap();
    let new_login = MiguCredential::verified("111".into(), "new-login-pacm".into()).unwrap();
    store.put(&stored(alias, &new_login)).unwrap();
    release.send(()).unwrap();
    assert_eq!(task.await.unwrap().unwrap_err().code, ErrorCode::Conflict);
    assert_eq!(read(&store, alias), new_login);
    wire.abort();
}

mod mg3d;
