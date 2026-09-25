use super::*;
use crate::credential::MiguCredential;
use crate::provider::session::tests::{Store, gated, profile, read, stored};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

pub(in crate::provider) fn reply(value: serde_json::Value, token: Option<&str>) -> String {
    let body = value.to_string();
    let header = token
        .map(|v| format!("pacmtoken: {v}\r\n"))
        .unwrap_or_default();
    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n{header}Connection: close\r\n\r\n{body}",
        body.len()
    )
}
pub(in crate::provider) fn resource() -> String {
    reply(
        json!({"code":"000000","resource":[{"resourceType":"2","contentId":"123","copyrightId":"6005971HBUU","songId":"456","songName":"Song","singerId":"77","singer":"Artist","length":"03:32","rateFormats":[{"formatType":"PQ"}]}]}),
        None,
    )
}
fn rights(trial: bool) -> String {
    reply(
        json!({"code":"000000","data":{"canListenRespItemList":[{"contentId":"123","canListen":!trial,"limitLength":trial}]}}),
        Some("candidate-2"),
    )
}
fn play(trial: bool) -> String {
    let mut value = json!({"code":"000000","data":{"url":"https://freetyst.nf.migu.cn/public/product9th/product44/file.mp3?Tim=1&Key=synthetic&playSessionId=fixture","audioFormatType":"PQ","song":{"resourceType":"2","contentId":"123","copyrightId":"6005971HBUU","songId":"456","duration":212}}});
    if trial {
        value["data"]["auditionsStartTime"] = json!(65);
        value["data"]["auditionsLength"] = json!(60);
    }
    reply(value, Some("candidate-3"))
}
fn frames(trial: bool) -> Vec<String> {
    vec![
        profile("111", "pacmtoken: verified-1\r\n"),
        resource(),
        rights(trial),
        profile("111", "pacmtoken: verified-2\r\n"),
        play(trial),
        profile("111", "pacmtoken: verified-3\r\n"),
    ]
}
pub(in crate::provider) async fn server(
    responses: Vec<String>,
) -> (MiguProvider, tokio::task::JoinHandle<Vec<String>>) {
    server_bytes(responses.into_iter().map(String::into_bytes).collect()).await
}
async fn server_bytes(
    responses: Vec<Vec<u8>>,
) -> (MiguProvider, tokio::task::JoinHandle<Vec<String>>) {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let origin = url::Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
    let task = tokio::spawn(async move {
        let mut requests = Vec::new();
        for response in responses {
            let request = tokio::time::timeout(Duration::from_secs(5), async {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut bytes = Vec::new();
                loop {
                    let mut buffer = [0; 4096];
                    let n = socket.read(&mut buffer).await.unwrap();
                    assert!(n > 0);
                    bytes.extend_from_slice(&buffer[..n]);
                    assert!(bytes.len() < 65536);
                    if let Some(end) = bytes.windows(4).position(|v| v == b"\r\n\r\n") {
                        let headers = std::str::from_utf8(&bytes[..end]).unwrap();
                        let length = headers
                            .lines()
                            .find_map(|line| {
                                line.split_once(':')
                                    .filter(|(k, _)| k.eq_ignore_ascii_case("content-length"))
                                    .map(|(_, v)| v.trim().parse::<usize>().unwrap())
                            })
                            .unwrap_or(0);
                        if bytes.len() >= end + 4 + length {
                            break;
                        }
                    }
                }
                socket.write_all(&response).await.unwrap();
                socket.shutdown().await.unwrap();
                String::from_utf8(bytes).unwrap()
            })
            .await
            .unwrap();
            requests.push(request);
        }
        requests
    });
    (
        MiguProvider::from_client(MiguClient::test_client().with_catalog_test_origin(origin)),
        task,
    )
}
pub(in crate::provider) fn setup(
    provider: &mut MiguProvider,
    mode: &str,
) -> (Arc<Store>, MiguCredential, &'static str) {
    let store = Arc::new(Store::default());
    let credential = MiguCredential::verified("111".into(), "initial-pacm".into()).unwrap();
    for alias in ["default", "personal"] {
        store.put(&stored(alias, &credential)).unwrap();
    }
    store
        .put(&stored(
            "other",
            &MiguCredential::verified("222".into(), "unrelated-pacm".into()).unwrap(),
        ))
        .unwrap();
    provider.credential_store = Some(store.clone());
    if mode == "caller" {
        *provider = provider
            .caller_scope(&credential.caller().unwrap())
            .unwrap();
    }
    (
        store,
        credential,
        if mode == "named" {
            "personal"
        } else {
            "default"
        },
    )
}
fn track() -> Track {
    Track::new(
        ResourceRef::new(Platform::Migu, "123").unwrap(),
        "Stale caller title",
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
async fn account_media_default_named_and_caller_use_pc_authorization_and_verified_rotations() {
    for mode in ["default", "named", "caller"] {
        for trial in [false, true] {
            let (mut provider, requests) = server(frames(trial)).await;
            let (store, original, alias) = setup(&mut provider, mode);
            let stream = provider.stream(&track(), &request(alias)).await.unwrap();
            assert_eq!(stream.actual_quality, Quality::Standard);
            assert_eq!(stream.bitrate, Some(128000));
            assert_eq!(stream.trial.is_some(), trial);
            assert!(stream.headers.is_empty());
            assert_eq!(stream.resolved_track.to_string(), "migu:123");
            assert_eq!(read(&store, "other").token(), "unrelated-pacm");
            if mode == "caller" {
                assert_eq!(read(&store, alias), original);
                let update = provider.take_response_credential().unwrap().unwrap();
                assert_eq!(
                    MiguCredential::parse_caller(&update).unwrap().token(),
                    "verified-3"
                );
            } else {
                assert_eq!(read(&store, alias).token(), "verified-3");
                assert!(provider.take_response_credential().unwrap().is_none());
            }
            let requests = requests.await.unwrap();
            assert_eq!(requests.len(), 6);
            assert!(!requests[1].to_ascii_lowercase().contains("pacmtoken"));
            assert!(requests[2].starts_with("POST /strategy/pc/can-listen/v1.0 "));
            let body: serde_json::Value =
                serde_json::from_str(requests[2].split("\r\n\r\n").nth(1).unwrap()).unwrap();
            assert_eq!(body, json!({"contentIds":"123","curPlayContentId":""}));
            assert!(requests[4].starts_with("GET /strategy/pc/listen/v2.0?"));
            assert!(requests[4].contains("copyrightId=6005971HBUU"));
            assert!(requests[4].contains("toneFlag=PQ"));
            assert!(!requests[4].contains("lowerQualityContentId"));
            for (index, token) in [
                (0, "initial-pacm"),
                (2, "verified-1"),
                (3, "candidate-2"),
                (4, "verified-2"),
                (5, "candidate-3"),
            ] {
                assert!(
                    requests[index]
                        .to_ascii_lowercase()
                        .contains(&format!("pacmtoken: {token}\r\n")),
                    "{index}"
                );
            }
            assert!(requests[4].contains("signature: 1\r\n"));
            assert!(requests[4].contains("birth: h5page\r\n"));
        }
    }
}

#[tokio::test]
async fn account_media_availability_requires_actual_play_and_download_requires_authorization() {
    for trial in [false, true] {
        let (mut provider, requests) = server(frames(trial)).await;
        let (_, _, alias) = setup(&mut provider, "named");
        let result = provider
            .track_availability(
                "123",
                &TrackAvailabilityRequest {
                    account: Some(alias.into()),
                    bitrate: 128000,
                },
            )
            .await
            .unwrap();
        assert_eq!(result.playable, !trial);
        assert_eq!(result.actual_bitrate, Some(128000));
        assert!(
            !serde_json::to_string(&result)
                .unwrap()
                .contains("synthetic")
        );
        requests.await.unwrap();
    }
    let (mut provider, requests) = server(vec![resource()]).await;
    let (store, original, _) = setup(&mut provider, "default");
    let public = provider.track("123", None).await.unwrap();
    assert!(!public.extensions.contains_key("source_user_id"));
    assert_eq!(read(&store, "default"), original);
    assert!(!provider.requires_download_authorization(None));
    assert!(provider.requires_download_authorization(Some("default")));
    let caller = provider.caller_scope(&original.caller().unwrap()).unwrap();
    assert!(caller.requires_download_authorization(None));
    let wire = requests.await.unwrap();
    assert_eq!(wire.len(), 1);
    assert!(!wire[0].to_lowercase().contains("pacmtoken"));
}

#[tokio::test]
async fn account_media_auto_retries_only_explicit_format_denials_with_the_same_verified_account() {
    for authentication_failure in [false, true] {
        let mut responses = frames(false);
        responses[1] = resource().replace("\"PQ\"", "\"HQ\"");
        responses[4] = reply(
            json!({"code":"000000","data":{"cannotCode":"440013"}}),
            Some("candidate-3"),
        );
        if authentication_failure {
            responses[5] =
                "HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into();
        } else {
            responses.push(play(false));
            responses.push(profile("111", "pacmtoken: verified-4\r\n"));
        }
        let (mut provider, requests) = server(responses).await;
        let (_, _, alias) = setup(&mut provider, "named");
        let mut request = request(alias);
        request.quality = Quality::Auto;
        let result = provider.stream(&track(), &request).await;
        if authentication_failure {
            assert_eq!(result.unwrap_err().code, ErrorCode::PermissionDenied);
        } else {
            assert_eq!(result.unwrap().actual_quality, Quality::Standard);
        }
        let requests = requests.await.unwrap();
        assert!(requests[4].contains("toneFlag=HQ"));
        if !authentication_failure {
            assert_eq!(requests.len(), 8);
            assert!(requests[6].contains("toneFlag=PQ"));
            assert!(requests[6].contains("pacmtoken: verified-3"));
        }
    }
}

#[tokio::test]
async fn account_media_every_late_success_or_failure_is_bound_to_the_original_login() {
    for mode in ["default", "named", "caller"] {
        for boundary in 0..6 {
            for action in ["logout", "same-token-login", "switch-user"] {
                if mode == "caller" && action == "logout" {
                    continue;
                }
                for late_error in [false, true] {
                    let mut responses = frames(false);
                    responses.truncate(boundary + 1);
                    if late_error {
                        responses[boundary] = "HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into();
                    }
                    let (mut provider, seen, release, wire) = gated(responses).await;
                    let (store, _, alias) = setup(&mut provider, mode);
                    let provider = Arc::new(provider);
                    let operation = provider.clone();
                    let task =
                        tokio::spawn(
                            async move { operation.stream(&track(), &request(alias)).await },
                        );
                    tokio::time::timeout(Duration::from_secs(5), seen)
                        .await
                        .unwrap()
                        .unwrap();
                    let token = if mode == "caller" {
                        provider
                            .caller_credential
                            .as_ref()
                            .unwrap()
                            .lock()
                            .unwrap()
                            .token()
                            .to_owned()
                    } else {
                        read(&store, alias).token().to_owned()
                    };
                    let replacement = MiguCredential::verified(
                        if action == "switch-user" {
                            "333"
                        } else {
                            "111"
                        }
                        .into(),
                        token,
                    )
                    .unwrap();
                    if mode == "caller" {
                        *provider.caller_credential.as_ref().unwrap().lock().unwrap() =
                            replacement.clone();
                    } else if action == "logout" {
                        store.remove(Platform::Migu, alias).unwrap();
                    } else {
                        store.put(&stored(alias, &replacement)).unwrap();
                    }
                    release.send(()).unwrap();
                    let mut failure = tokio::time::timeout(Duration::from_secs(5), task)
                        .await
                        .unwrap()
                        .unwrap()
                        .unwrap_err();
                    assert_eq!(
                        failure.code,
                        ErrorCode::Conflict,
                        "{mode}/{boundary}/{action}/{late_error}"
                    );
                    assert!(failure.take_caller_credential_update().is_none());
                    assert!(provider.take_response_credential().unwrap().is_none());
                    assert_eq!(read(&store, "other").token(), "unrelated-pacm");
                    if mode != "caller" && action != "logout" {
                        assert_eq!(read(&store, alias), replacement);
                    }
                    wire.await.unwrap();
                }
            }
        }
    }
}

#[tokio::test]
async fn account_media_cancellation_and_timeout_at_every_boundary_export_only_verified_updates() {
    for cancel in [false, true] {
        for boundary in 0..6 {
            let mut responses = frames(false);
            responses.truncate(boundary + 1);
            let (mut provider, seen, _release, wire) = gated(responses).await;
            provider.client = provider
                .client
                .with_session_test_timeout(Duration::from_millis(200));
            let (store, original, alias) = setup(&mut provider, "caller");
            let provider = Arc::new(provider);
            let operation = provider.clone();
            let task =
                tokio::spawn(async move { operation.stream(&track(), &request(alias)).await });
            tokio::time::timeout(Duration::from_secs(5), seen)
                .await
                .unwrap()
                .unwrap();
            if cancel {
                task.abort();
                assert!(task.await.unwrap_err().is_cancelled());
                assert!(provider.take_response_credential().unwrap().is_none());
            } else {
                let mut failure = tokio::time::timeout(Duration::from_secs(5), task)
                    .await
                    .unwrap()
                    .unwrap()
                    .unwrap_err();
                assert_eq!(failure.code, ErrorCode::UpstreamTimeout);
                let update = failure.take_caller_credential_update();
                if boundary == 0 {
                    assert!(update.is_none());
                } else {
                    assert_eq!(
                        MiguCredential::parse_caller(&update.unwrap())
                            .unwrap()
                            .token(),
                        if boundary < 4 {
                            "verified-1"
                        } else {
                            "verified-2"
                        }
                    );
                }
            }
            assert_eq!(read(&store, alias), original);
            wire.abort();
        }
    }
}

fn search_page(id: &str, more: bool) -> String {
    reply(
        json!({"code":"000000","data":{"hasNext":more,"items":[{"song":{"resourceType":"2","contentId":id,"copyrightId":"6005971HBUU","songId":"456","songName":"Song","duration":212,"singerList":[{"id":"77","name":"Artist"}]}}]}}),
        None,
    )
}

#[tokio::test]
async fn account_media_candidates_and_details_keep_public_catalogue_and_selected_account_separate()
{
    for mode in ["default", "named", "caller"] {
        let responses = vec![
            profile("111", "pacmtoken: verified-1\r\n"),
            resource(),
            profile("111", "pacmtoken: verified-2\r\n"),
            search_page("123", true),
            search_page("456", false),
        ];
        let (mut provider, requests) = server(responses).await;
        let (store, original, alias) = setup(&mut provider, mode);
        let track = provider.track("123", Some(alias)).await.unwrap();
        assert_eq!(track.extensions["source_user_id"], "111");
        assert_eq!(track.extensions["catalogue_scope"], "public");
        let mut query = SearchQuery::tracks("Song Artist", 21, 0);
        query.account = Some(alias.into());
        let page = provider.search_catalog(&query).await.unwrap();
        assert_eq!(page.items.len(), 2);
        assert!(!page.pagination.has_more);
        assert_eq!(page.pagination.extensions["source_user_id"], "111");
        assert_eq!(page.pagination.extensions["catalogue_scope"], "public");
        if mode == "caller" {
            assert_eq!(read(&store, alias), original);
        }
        let requests = requests.await.unwrap();
        assert_eq!(requests.len(), 5);
        for index in [1, 3, 4] {
            assert!(!requests[index].to_lowercase().contains("pacmtoken"));
        }
        assert!(requests[3].contains("pageNo=1"));
        assert!(requests[4].contains("pageNo=2"));
    }
    // Logout after a public candidate page must stop subsequent pages as well.
    let (mut provider, seen, release, wire) =
        gated(vec![profile("111", ""), search_page("123", true)]).await;
    let (store, _, alias) = setup(&mut provider, "named");
    let mut query = SearchQuery::tracks("Song", 21, 0);
    query.account = Some(alias.into());
    let task = tokio::spawn(async move { provider.search(&query).await });
    seen.await.unwrap();
    store.remove(Platform::Migu, alias).unwrap();
    release.send(()).unwrap();
    assert_eq!(task.await.unwrap().unwrap_err().code, ErrorCode::Conflict);
    wire.await.unwrap();
}

#[tokio::test]
async fn account_media_real_resolver_uses_account_candidates_and_rejects_strict_mismatches() {
    use tuneweave_core::{ArtistSummary, ProviderRegistry, ResolveRequest, StreamResolver};
    for mismatch in [false, true] {
        let mut responses = vec![
            profile("111", "pacmtoken: verified-search\r\n"),
            search_page("123", false),
        ];
        if !mismatch {
            responses.extend(frames(false));
        }
        let (mut provider, requests) = server(responses).await;
        let (_, _, alias) = setup(&mut provider, "named");
        let mut registry = ProviderRegistry::new();
        registry.register(provider).unwrap();
        let resolver = StreamResolver::new(registry, vec![]);
        let mut origin = Track::new(
            ResourceRef::new(Platform::Netease, "789").unwrap(),
            if mismatch {
                "Completely different recording"
            } else {
                "Song"
            },
        );
        origin.duration_ms = Some(212000);
        origin.artists = vec![ArtistSummary {
            name: "Artist".into(),
            resource_ref: None,
        }];
        let request = ResolveRequest {
            quality: Quality::Standard,
            playback_platforms: vec![Platform::Migu],
            fallback: false,
            accounts: std::collections::BTreeMap::from([(Platform::Migu, alias.into())]),
            ..Default::default()
        };
        let result = resolver.resolve(&origin, &request).await;
        if mismatch {
            assert!(result.is_err());
        } else {
            let stream = result.unwrap();
            assert_eq!(stream.resolved_track.to_string(), "migu:123");
            assert_eq!(stream.origin_track, Some(origin.resource_ref));
        }
        let requests = requests.await.unwrap();
        assert_eq!(requests.len(), if mismatch { 2 } else { 8 });
    }
}

#[tokio::test]
async fn account_media_binary_pc_response_is_decoded_before_identity_verification() {
    let mut responses: Vec<Vec<u8>> = frames(false).into_iter().map(String::into_bytes).collect();
    let plain = play(false);
    let plain = plain.split("\r\n\r\n").nth(1).unwrap().as_bytes();
    let key = b"Jk8qzuePiJ1qE3mDYhLQ3T73DtDoAhLP";
    let mut encrypted = vec![0xab, 0xcd, 1, 19];
    encrypted.extend(
        plain
            .iter()
            .enumerate()
            .map(|(i, b)| b.wrapping_add(key[i % key.len()]).wrapping_sub(19)),
    );
    let mut response=format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nsignature: 1\r\npacmtoken: binary-candidate\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",encrypted.len()).into_bytes();
    response.extend(encrypted);
    responses[4] = response;
    let (mut provider, requests) = server_bytes(responses).await;
    let (_, _, alias) = setup(&mut provider, "named");
    assert_eq!(
        provider
            .stream(&track(), &request(alias))
            .await
            .unwrap()
            .actual_quality,
        Quality::Standard
    );
    let requests = requests.await.unwrap();
    assert!(requests[5].contains("pacmtoken: binary-candidate"));
}

#[tokio::test]
async fn account_media_invalid_authorizations_do_not_adopt_unverified_tokens_or_clear_other_accounts()
 {
    for mode in ["named", "caller"] {
        for kind in [
            "identity",
            "wrong-uid",
            "unauthorized",
            "forbidden",
            "business",
            "html",
            "oversized",
        ] {
            let mut responses = frames(false);
            let (expected, verified) = match kind {
                "identity" => {
                    responses[4] =
                        play(false).replace("\"contentId\":\"123\"", "\"contentId\":\"999\"");
                    (ErrorCode::UpstreamError, "verified-3")
                }
                "wrong-uid" => {
                    responses[5] = profile("222", "pacmtoken: untrusted-session\r\n");
                    (ErrorCode::AuthenticationRequired, "")
                }
                "unauthorized" => {
                    responses.truncate(5);
                    responses[4]="HTTP/1.1 401 Unauthorized\r\npacmtoken: untrusted-session\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into();
                    (ErrorCode::AuthenticationRequired, "")
                }
                "forbidden" => {
                    responses.truncate(5);
                    responses[4]="HTTP/1.1 403 Forbidden\r\npacmtoken: untrusted-session\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into();
                    (ErrorCode::PermissionDenied, "verified-2")
                }
                "business" => {
                    responses.truncate(5);
                    responses[4] = reply(
                        json!({"code":"290001","data":{}}),
                        Some("untrusted-session"),
                    );
                    (ErrorCode::UpstreamError, "verified-2")
                }
                "html" => {
                    responses.truncate(5);
                    responses[4]="HTTP/1.1 200 OK\r\nContent-Type: text/html\r\npacmtoken: untrusted-session\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into();
                    (ErrorCode::UpstreamError, "verified-2")
                }
                _ => {
                    responses.truncate(5);
                    responses[4]="HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 1048577\r\nConnection: close\r\n\r\n".into();
                    (ErrorCode::UpstreamError, "verified-2")
                }
            };
            let (mut provider, requests) = server(responses).await;
            let (store, original, alias) = setup(&mut provider, mode);
            let mut failure = provider
                .stream(&track(), &request(alias))
                .await
                .unwrap_err();
            assert_eq!(failure.code, expected, "{kind}");
            let update = failure.take_caller_credential_update();
            if mode == "caller" {
                assert_eq!(read(&store, alias), original);
                if verified.is_empty() {
                    assert!(update.is_none());
                    assert!(provider.take_response_credential().unwrap().is_none());
                } else {
                    assert_eq!(
                        MiguCredential::parse_caller(&update.unwrap())
                            .unwrap()
                            .token(),
                        verified
                    );
                }
            } else if verified.is_empty() {
                assert!(
                    !store
                        .load_platform(Platform::Migu)
                        .unwrap()
                        .iter()
                        .any(|v| v.account == alias)
                );
            } else {
                assert_eq!(read(&store, alias).token(), verified);
            }
            assert_eq!(read(&store, "other").token(), "unrelated-pacm");
            requests.await.unwrap();
        }
    }
}
