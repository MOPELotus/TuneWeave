use super::*;
use crate::credential::MiguCredential;
use crate::provider::account_media::tests::{reply, server, setup};
use crate::provider::session::tests::{profile, read, stored};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::oneshot;
use tuneweave_core::ResourceRef;

fn encrypted(value: serde_json::Value) -> String {
    let body = crate::client::native_http::encode(value.to_string().as_bytes());
    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

fn entry(id: &str) -> serde_json::Value {
    json!({"follow":1,"followNums":123,"msisdn":"discard-contact-phone", "user":{
        "userId":id,"userType":"01","nickName":format!("Singer {id}"),
        "smallIcon":"https://d.musicapp.migu.cn/small.jpg",
        "middleIcon":"https://d.musicapp.migu.cn/middle.jpg",
        "bigIcon":"https://d.musicapp.migu.cn/big.jpg",
        "icon":"https://d.musicapp.migu.cn/icon.jpg",
        "msisdn":"discard-phone","passId":"discard-pass-id",
        "accountName":"discard-account-name","usessionId":"discard-session"
    }})
}

fn page(entries: Vec<serde_json::Value>) -> String {
    // Deliberately inconsistent all-type count: the official singer presenter
    // does not consume this field or stop after a short nonempty page.
    reply(
        json!({"code":"000000","followsFromUser":entries,"totalCount":"900"}),
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
        encrypted(json!({"code":"000000","data":{"userInfoItem":{"userId":"111"}}})),
        page(vec![entry("101")]),
        page(vec![entry("102"), entry("103")]),
        page(vec![]),
        profile("111", "pacmtoken: middle-pacm\r\n"),
        page(vec![entry("101")]),
        page(vec![entry("102"), entry("103")]),
        page(vec![]),
        profile("111", "pacmtoken: final-pacm\r\n"),
    ]
}

fn request(alias: &str) -> PageRequest {
    PageRequest {
        limit: 1,
        offset: 1,
        account: Some(alias.into()),
    }
}

#[tokio::test]
async fn following_artists_reads_both_complete_directories_before_slicing_selected_account() {
    for selection in ["default", "named", "caller"] {
        let (mut provider, requests) = server(frames()).await;
        let (store, original, alias) = setup(&mut provider, selection);
        let mut req = request(alias);
        if selection == "default" {
            req.account = None;
        }
        let result = if selection == "named" {
            provider.user_following_artists("111", &req).await
        } else {
            provider.account_following_artists(&req).await
        }
        .unwrap();
        assert_eq!(result.items.len(), 1);
        let artist = &result.items[0];
        assert_eq!(artist.id, "102");
        assert_eq!(
            artist.resource_ref,
            ResourceRef::new(Platform::Migu, "102").unwrap()
        );
        assert_eq!(artist.name, "Singer 102");
        assert_eq!(
            artist.avatar_url.as_deref(),
            Some("https://d.musicapp.migu.cn/small.jpg")
        );
        assert!(artist.album_count.is_none() && artist.track_count.is_none());
        assert_eq!(result.pagination.total, Some(3));
        assert_eq!(result.pagination.next_offset, Some(2));
        assert!(result.pagination.has_more);
        assert_eq!(result.pagination.extensions["source_user_id"], "111");
        assert_eq!(
            result.pagination.extensions["consistency"],
            "two_complete_reads"
        );
        let output = serde_json::to_string(&result).unwrap();
        for absent in ["pacm", "native-token-fixture", "do-not-retain", "discard-"] {
            assert!(!output.contains(absent));
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
        } else {
            assert_eq!(
                read(&store, alias),
                original.rotate("final-pacm".into()).unwrap()
            );
        }
        assert!(provider.take_response_credential().unwrap().is_none());
        let wire = requests.await.unwrap();
        assert_eq!(wire.len(), 12);
        assert!(wire.iter().all(|request| request.starts_with("GET ")));
        for (at, number) in [(4, 1), (5, 2), (6, 3), (8, 1), (9, 2), (10, 3)] {
            assert!(wire[at].starts_with(&format!("GET /MIGUM2.0/v1.0/user/followingSingers.do?pageNo={number}&pageSize=20&userId=111&userType=1 HTTP/1.1\r\n")));
            for header in [
                "token: native-token-fixture\r\n",
                "signversion: V005\r\n",
                "sign: ",
                "ce: ",
                "appid: music\r\n",
                "os: Android\r\n",
            ] {
                assert!(wire[at].contains(header), "missing {header}");
            }
            for absent in [
                "pacmtoken:",
                "cookie:",
                "requestenc:",
                "responseenc:",
                "exchange-pacm",
                "do-not-retain-session",
            ] {
                assert!(!wire[at].contains(absent));
            }
        }
        assert!(wire[7].contains("pacmtoken: verified-pacm\r\n"));
        assert!(wire[11].contains("pacmtoken: middle-pacm\r\n"));
    }
}

#[tokio::test]
async fn following_artists_accepts_only_successful_empty_sentinels_and_safe_icons() {
    for terminal in [
        json!({"code":"000000","followsFromUser":[]}),
        json!({"code":"000000","followsFromUser":null}),
        json!({"code":"000000"}),
    ] {
        let all = frames();
        let responses = all[..4]
            .iter()
            .cloned()
            .chain([
                reply(terminal.clone(), None),
                all[7].clone(),
                reply(terminal, None),
                all[11].clone(),
            ])
            .collect();
        let (mut provider, requests) = server(responses).await;
        let (_, _, alias) = setup(&mut provider, "named");
        let result = provider
            .account_following_artists(&request(alias))
            .await
            .unwrap();
        assert!(result.items.is_empty());
        assert_eq!(result.pagination.total, Some(0));
        assert!(!result.pagination.has_more);
        assert!(result.pagination.next_offset.is_none());
        assert_eq!(requests.await.unwrap().len(), 8);
    }
    for (small, middle, expected) in [
        (
            "",
            "https://d.musicapp.migu.cn/middle.jpg",
            Some("https://d.musicapp.migu.cn/middle.jpg"),
        ),
        (
            "https://evil.invalid/a.jpg",
            "https://d.musicapp.migu.cn/middle.jpg",
            None,
        ),
        ("http://d.musicapp.migu.cn/a.jpg", "", None),
    ] {
        let mut item = entry("101");
        item["user"]["smallIcon"] = json!(small);
        item["user"]["middleIcon"] = json!(middle);
        let mut responses = frames();
        responses[4] = page(vec![item.clone()]);
        responses[8] = page(vec![item]);
        let (mut provider, requests) = server(responses).await;
        let (_, _, alias) = setup(&mut provider, "named");
        let mut req = request(alias);
        req.offset = 0;
        let result = provider.account_following_artists(&req).await.unwrap();
        assert_eq!(result.items[0].avatar_url.as_deref(), expected);
        assert_eq!(requests.await.unwrap().len(), 12);
    }
}

#[tokio::test]
async fn following_artists_rejects_invalid_or_ambiguous_directory_payloads_without_partial_pages() {
    let mut bad_items = Vec::new();
    for (key, value) in [
        ("userType", json!("00")),
        ("userType", json!(1)),
        ("userId", json!("001")),
        ("userId", json!("../101")),
        ("nickName", json!("")),
        ("nickName", json!("bad\u{0}name")),
        ("nickName", json!("x".repeat(2049))),
    ] {
        let mut item = entry("101");
        item["user"][key] = value;
        bad_items.push(json!({"code":"000000","followsFromUser":[item]}));
    }
    bad_items.extend([
        json!({"code":"bad-secret-info","info":"initial-pacm","followsFromUser":[]}),
        json!({"code":0,"followsFromUser":[]}),
        json!({"code":"000000","followsFromUser":[null]}),
        json!({"code":"000000","followsFromUser":[{"user":null}]}),
        json!({"code":"000000","followsFromUser":{}}),
        json!({"code":"000000","followsFromUser":(100..121).map(|i|entry(&i.to_string())).collect::<Vec<_>>()}),
    ]);
    for body in bad_items {
        let mut responses = frames();
        responses[4] = reply(body, None);
        responses.truncate(5);
        let (mut provider, requests) = server(responses).await;
        let (_, _, alias) = setup(&mut provider, "named");
        let failure = provider
            .account_following_artists(&request(alias))
            .await
            .unwrap_err();
        assert_eq!(failure.code, ErrorCode::UpstreamError);
        assert!(!failure.message.contains("initial-pacm"));
        assert!(!failure.details.to_string().contains("initial-pacm"));
        assert_eq!(requests.await.unwrap().len(), 5);
    }
}

#[tokio::test]
async fn following_artists_rejects_changes_duplicate_pages_wrong_identity_and_denials() {
    for (at, response, code, last) in [
        (3, encrypted(json!({"code":"000000","data":{"userInfoItem":{"userId":"222"}}})), ErrorCode::PermissionDenied, 3),
        (5, page(vec![entry("101")]), ErrorCode::Conflict, 5),
        (8, page(vec![entry("104")]), ErrorCode::Conflict, 10),
        (7, profile("222", ""), ErrorCode::AuthenticationRequired, 7),
        (11, profile("222", ""), ErrorCode::AuthenticationRequired, 11),
        (5, "HTTP/1.1 429 Too Many Requests\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into(), ErrorCode::RateLimited, 5),
        (5, "HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into(), ErrorCode::PermissionDenied, 5),
        (5, "HTTP/1.1 302 Found\r\nLocation: https://example.invalid/leak\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into(), ErrorCode::UpstreamError, 5),
    ] {
        let mut responses = frames(); responses[at] = response; responses.truncate(last + 1);
        let (mut provider, requests) = server(responses).await;
        let (store, original, alias) = setup(&mut provider, "caller");
        let mut failure = provider.account_following_artists(&request(alias)).await.unwrap_err();
        assert_eq!(failure.code, code, "at {at}");
        assert!(!failure.message.contains("native-token-fixture"));
        if matches!(code, ErrorCode::Conflict | ErrorCode::AuthenticationRequired) {
            assert!(provider.take_response_credential().unwrap().is_none());
            assert!(failure.take_caller_credential_update().is_none());
        } else {
            let update = provider.take_response_credential().unwrap().unwrap();
            let expected = original.rotate("verified-pacm".into()).unwrap();
            assert_eq!(MiguCredential::parse_caller(&update).unwrap(), expected);
            assert_eq!(MiguCredential::parse_caller(&failure.take_caller_credential_update().unwrap()).unwrap(), expected);
            assert!(!update.secret().contains("native-token-fixture"));
            assert!(provider.take_response_credential().unwrap().is_none());
            assert!(failure.take_caller_credential_update().is_none());
        }
        assert_eq!(read(&store, alias), original);
        assert_eq!(requests.await.unwrap().len(), last + 1);
    }
}

#[tokio::test]
async fn following_artists_rechecks_reflected_secrets_after_each_credential_rotation() {
    for (field, value, last) in [
        ("nickName", "initial-pacm", 4),
        ("nickName", "native-token-fixture", 4),
        ("nickName", "do-not-retain-session", 4),
        (
            "smallIcon",
            "https://d.musicapp.migu.cn/native%2Dtoken%2Dfixture.jpg",
            4,
        ),
        ("nickName", "middle-pacm", 7),
        ("nickName", "final-pacm", 11),
    ] {
        let mut item = entry("101");
        item["user"][field] = json!(value);
        let mut responses = frames();
        responses[4] = page(vec![item.clone()]);
        responses[8] = page(vec![item]);
        responses.truncate(last + 1);
        let (mut provider, requests) = server(responses).await;
        let (_, _, alias) = setup(&mut provider, "caller");
        let failure = provider
            .account_following_artists(&request(alias))
            .await
            .unwrap_err();
        assert_eq!(failure.code, ErrorCode::UpstreamError);
        assert!(!failure.message.contains(value));
        assert!(!failure.details.to_string().contains(value));
        let update = provider.take_response_credential().unwrap().unwrap();
        assert_eq!(
            MiguCredential::parse_caller(&update).unwrap().token(),
            match last {
                7 => "middle-pacm",
                11 => "final-pacm",
                _ => "verified-pacm",
            }
        );
        assert!(!update.secret().contains("native-token-fixture"));
        assert_eq!(requests.await.unwrap().len(), last + 1);
    }
}

#[tokio::test]
async fn following_artists_bounds_complete_reads_without_manufacturing_a_terminal_page() {
    let mut responses = frames()[..4].to_vec();
    for page_no in 1..=MAX_PAGES + 1 {
        responses.push(page(vec![entry(&(1000 + page_no).to_string())]));
    }
    let expected = responses.len();
    let (mut provider, requests) = server(responses).await;
    let (_, _, alias) = setup(&mut provider, "named");
    assert_eq!(
        provider
            .account_following_artists(&request(alias))
            .await
            .unwrap_err()
            .code,
        ErrorCode::UpstreamError
    );
    assert_eq!(requests.await.unwrap().len(), expected);
}

#[tokio::test]
async fn following_artists_rejects_foreign_missing_or_invalid_selection_before_network() {
    for (uid, limit, offset, account, code) in [
        (Some("222"), 1, 0, "personal", ErrorCode::PermissionDenied),
        (Some("../111"), 1, 0, "personal", ErrorCode::InvalidRequest),
        (None, 1, 0, "missing", ErrorCode::AuthenticationRequired),
        (None, 0, 0, "personal", ErrorCode::InvalidRequest),
        (None, 101, 0, "personal", ErrorCode::InvalidRequest),
        (None, 1, u32::MAX, "personal", ErrorCode::InvalidRequest),
    ] {
        let (mut provider, requests) = server(vec![]).await;
        setup(&mut provider, "named");
        let request = PageRequest {
            limit,
            offset,
            account: Some(account.into()),
        };
        let result = provider.read_following_artists(uid, &request).await;
        assert_eq!(result.unwrap_err().code, code);
        assert!(requests.await.unwrap().is_empty());
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
                    assert!(bytes.len() < 65536);
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
async fn following_artists_checks_original_login_generation_at_every_network_boundary() {
    for at in 0..12 {
        let mut gate = gated_server(at).await;
        let (store, _, alias) = setup(&mut gate.provider, "named");
        let provider = gate.provider.clone();
        let task =
            tokio::spawn(async move { provider.account_following_artists(&request(alias)).await });
        tokio::time::timeout(Duration::from_secs(5), gate.seen)
            .await
            .unwrap()
            .unwrap();
        let replacement =
            MiguCredential::verified("111".into(), "replacement-login".into()).unwrap();
        store.put(&stored(alias, &replacement)).unwrap();
        gate.release.send(()).unwrap();
        assert_eq!(
            task.await.unwrap().unwrap_err().code,
            ErrorCode::Conflict,
            "at {at}"
        );
        assert_eq!(read(&store, alias), replacement);
        gate.task.await.unwrap();
    }
}

#[tokio::test]
async fn following_artists_cancellation_does_not_leak_undelivered_caller_rotations() {
    for at in 0..12 {
        let mut gate = gated_server(at).await;
        let (store, original, alias) = setup(&mut gate.provider, "caller");
        let provider = gate.provider.clone();
        let task =
            tokio::spawn(async move { provider.account_following_artists(&request(alias)).await });
        tokio::time::timeout(Duration::from_secs(5), gate.seen)
            .await
            .unwrap()
            .unwrap();
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        assert!(gate.provider.take_response_credential().unwrap().is_none());
        assert_eq!(read(&store, alias), original);
        gate.task.abort();
        assert!(gate.task.await.unwrap_err().is_cancelled());
    }
}
