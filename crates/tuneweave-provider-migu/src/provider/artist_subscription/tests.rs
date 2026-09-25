use super::*;
use crate::client::artists::tests::info;
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

fn directory(ids: &[&str]) -> String {
    let entries: Vec<_> = ids
        .iter()
        .map(|id| {
            json!({"user":{
                "userId":id,"userType":"01","nickName":format!("Singer {id}"),
                "smallIcon":"https://d.musicapp.migu.cn/singer.jpg",
                "msisdn":"discard-phone","usessionId":"discard-session"
            }})
        })
        .collect();
    reply(json!({"code":"000000","followsFromUser":entries}), None)
}

fn stable(ids: &[&str], prefix: &str) -> Vec<String> {
    vec![
        directory(ids),
        directory(&[]),
        profile("111", &format!("pacmtoken: {prefix}-middle\r\n")),
        directory(ids),
        directory(&[]),
        profile("111", &format!("pacmtoken: {prefix}-final\r\n")),
    ]
}

struct Flow {
    frames: Vec<String>,
    write: Option<usize>,
    after: Option<usize>,
}

fn flow(subscribed: bool, already: bool) -> Flow {
    let mut frames = vec![
        profile("111", "pacmtoken: profile-pacm\r\n"),
        reply(
            json!({"code":"000000","data":"native-token-fixture"}),
            Some("exchange-pacm"),
        ),
        profile("111", "pacmtoken: verified-pacm\r\n"),
        encrypted(json!({"code":"000000","data":{"userInfoItem":{"userId":"111"}}})),
    ];
    frames.extend(stable(
        if already {
            &["103", "101", "102"]
        } else {
            &["101", "102"]
        },
        "before",
    ));
    let mut write = None;
    let mut after = None;
    if subscribed != already {
        if subscribed {
            frames.push(reply(
                json!({"code":"000000","data":info("103",None,None)}),
                None,
            ));
        }
        write = Some(frames.len());
        frames.push(reply(
            json!({"code":"000000","follow":if subscribed {"1"} else {"0"}}),
            None,
        ));
        after = Some(frames.len());
        frames.extend(stable(
            if subscribed {
                &["103", "101", "102"]
            } else {
                &["101", "102"]
            },
            "after",
        ));
    }
    Flow {
        frames,
        write,
        after,
    }
}

#[tokio::test]
async fn artist_subscription_changes_only_the_selected_account_once_and_confirms_complete_readback()
{
    for selection in ["default", "named", "caller"] {
        for subscribed in [true, false] {
            let f = flow(subscribed, !subscribed);
            let write = f.write.unwrap();
            let after = f.after.unwrap();
            let (mut provider, requests) = server(f.frames).await;
            let (store, original, alias) = setup(&mut provider, selection);
            let result = provider
                .set_artist_subscription(
                    "103",
                    subscribed,
                    if selection == "default" {
                        None
                    } else {
                        Some(alias)
                    },
                )
                .await
                .unwrap();
            assert_eq!(
                result.resource_ref,
                ResourceRef::new(Platform::Migu, "103").unwrap()
            );
            assert_eq!(result.subscribed, subscribed);
            assert_eq!(result.extensions["write_performed"], true);
            assert_eq!(result.extensions["source_user_id"], "111");
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
                    original.rotate("after-final".into()).unwrap()
                );
                assert!(!update.secret().contains("native-token-fixture"));
            } else {
                assert_eq!(
                    read(&store, alias),
                    original.rotate("after-final".into()).unwrap()
                );
            }
            assert!(provider.take_response_credential().unwrap().is_none());
            let wire = requests.await.unwrap();
            assert_eq!(wire.len(), after + 6);
            assert_eq!(
                wire.iter()
                    .filter(|r| r.contains("/follow.do?") || r.contains("/unfollow.do?"))
                    .count(),
                1
            );
            assert!(wire[write].starts_with(&format!(
                "GET /MIGUM2.0/v1.0/user/{}.do?followId=103&type=1&userId=111 HTTP/1.1\r\n",
                if subscribed { "follow" } else { "unfollow" }
            )));
            for header in [
                "token: native-token-fixture\r\n",
                "ce: ",
                "signversion: V005\r\n",
                "sign: ",
                "appid: music\r\n",
            ] {
                assert!(wire[write].contains(header));
            }
            for absent in ["pacmtoken:", "cookie:", "requestenc:", "responseenc:"] {
                assert!(!wire[write].contains(absent));
            }
            for at in [4, 7, after, after + 3] {
                assert!(wire[at].starts_with("GET /MIGUM2.0/v1.0/user/followingSingers.do?pageNo=1&pageSize=20&userId=111&userType=1 "));
                assert!(
                    wire[at + 1]
                        .starts_with("GET /MIGUM2.0/v1.0/user/followingSingers.do?pageNo=2&")
                );
            }
            if subscribed {
                assert!(wire[write - 1].starts_with("GET /pc/bmw/singer/info/v1.1?singerId=103 "));
                for secret in ["pacmtoken:", "token:", "cookie:"] {
                    assert!(!wire[write - 1].contains(secret));
                }
            } else {
                assert!(wire.iter().all(|r| !r.contains("/pc/bmw/singer/info/")));
            }
        }
    }
}

#[tokio::test]
async fn artist_subscription_noop_still_checks_stable_identity_without_writing_or_public_lookup() {
    for subscribed in [true, false] {
        let f = flow(subscribed, subscribed);
        assert!(f.write.is_none());
        let (mut provider, requests) = server(f.frames).await;
        let (store, _, alias) = setup(&mut provider, "named");
        let result = provider
            .set_artist_subscription("103", subscribed, Some(alias))
            .await
            .unwrap();
        assert_eq!(result.subscribed, subscribed);
        assert_eq!(result.extensions["write_performed"], false);
        assert_eq!(read(&store, alias).token(), "before-final");
        let wire = requests.await.unwrap();
        assert_eq!(wire.len(), 10);
        assert!(wire.iter().all(|r| !r.contains("/follow.do?")
            && !r.contains("/unfollow.do?")
            && !r.contains("/pc/bmw/singer/info/")));
    }
}

#[tokio::test]
async fn artist_subscription_ack_requires_explicit_supported_state_and_never_retries() {
    for subscribed in [true, false] {
        for ack in [
            json!({"code":"000000"}),
            json!({"code":"000000","follow":null}),
            json!({"code":"000000","follow":""}),
            json!({"code":"000000","follow":1}),
            json!({"code":"000000","follow":"3"}),
            json!({"code":"000000","follow":if subscribed {"0"} else {"1"}}),
            json!({"code":"999999","follow":"0","info":"native-token-fixture"}),
        ] {
            let mut f = flow(subscribed, !subscribed);
            let write = f.write.unwrap();
            f.frames[write] = reply(ack, None);
            f.frames.truncate(write + 1);
            let (mut provider, requests) = server(f.frames).await;
            let (store, original, alias) = setup(&mut provider, "caller");
            let mut failure = provider
                .set_artist_subscription("103", subscribed, Some(alias))
                .await
                .unwrap_err();
            assert_eq!(failure.code, ErrorCode::UpstreamError);
            assert_eq!(failure.details["write_outcome"], "unconfirmed");
            assert_eq!(failure.details["retry_safe"], false);
            assert!(!failure.retryable);
            assert!(!failure.message.contains("native-token-fixture"));
            assert!(!failure.details.to_string().contains("native-token-fixture"));
            let update = provider.take_response_credential().unwrap().unwrap();
            let expected = original.rotate("before-final".into()).unwrap();
            assert_eq!(MiguCredential::parse_caller(&update).unwrap(), expected);
            assert_eq!(
                MiguCredential::parse_caller(&failure.take_caller_credential_update().unwrap())
                    .unwrap(),
                expected
            );
            assert!(provider.take_response_credential().unwrap().is_none());
            assert!(failure.take_caller_credential_update().is_none());
            assert_eq!(read(&store, alias), original);
            assert_eq!(requests.await.unwrap().len(), write + 1);
        }
    }
    // Both current nonzero states are explicitly used by the native UI.
    let mut f = flow(true, false);
    f.frames[f.write.unwrap()] = reply(json!({"code":"000000","follow":"2"}), None);
    let count = f.frames.len();
    let (mut provider, requests) = server(f.frames).await;
    let (_, _, alias) = setup(&mut provider, "named");
    assert!(
        provider
            .set_artist_subscription("103", true, Some(alias))
            .await
            .unwrap()
            .subscribed
    );
    assert_eq!(requests.await.unwrap().len(), count);
}

#[tokio::test]
async fn artist_subscription_missing_state_or_collateral_directory_change_is_unconfirmed() {
    for (subscribed, ids) in [
        (true, vec!["101", "102"]),
        (false, vec!["103", "101", "102"]),
        (true, vec!["103", "102", "101"]),
        (true, vec!["103", "101", "104"]),
        (false, vec!["101"]),
    ] {
        let mut f = flow(subscribed, !subscribed);
        let after = f.after.unwrap();
        f.frames.truncate(after);
        f.frames.extend(stable(&ids, "after"));
        let expected = f.frames.len();
        let (mut provider, requests) = server(f.frames).await;
        let (_, _, alias) = setup(&mut provider, "named");
        let failure = provider
            .set_artist_subscription("103", subscribed, Some(alias))
            .await
            .unwrap_err();
        assert_eq!(failure.code, ErrorCode::UpstreamError);
        assert_eq!(failure.details["write_outcome"], "unconfirmed");
        assert!(!failure.retryable);
        assert_eq!(requests.await.unwrap().len(), expected);
    }
}

#[tokio::test]
async fn artist_subscription_preflight_failures_do_not_write_and_postwrite_errors_stay_uncertain() {
    for (at,response,code,last) in [
        (3,encrypted(json!({"code":"000000","data":{"userInfoItem":{"userId":"222"}}})),ErrorCode::PermissionDenied,3),
        (7,directory(&["101","104"]),ErrorCode::Conflict,8),
        (10,reply(json!({"code":"000000","data":info("999",None,None)}),None),ErrorCode::UpstreamError,10),
        (11,"HTTP/1.1 429 Too Many Requests\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into(),ErrorCode::RateLimited,11),
        (11,"HTTP/1.1 302 Found\r\nLocation: https://example.invalid/leak\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into(),ErrorCode::UpstreamError,11),
        (12,reply(json!({"code":"000001","followsFromUser":[]}),None),ErrorCode::UpstreamError,12),
        (15,directory(&["103","101","104"]),ErrorCode::Conflict,16),
        (17,profile("222",""),ErrorCode::AuthenticationRequired,17),
    ] {
        let mut f = flow(true,false); f.frames[at] = response; f.frames.truncate(last+1);
        let (mut provider, requests) = server(f.frames).await; let (store,original,alias) = setup(&mut provider,"caller");
        let mut failure = provider.set_artist_subscription("103",true,Some(alias)).await.unwrap_err();
        assert_eq!(failure.code,code,"at {at}");
        if at>=11 { assert_eq!(failure.details["write_outcome"],"unconfirmed"); assert!(!failure.retryable); }
        else { assert!(failure.details.get("write_outcome").is_none()); }
        if matches!(code,ErrorCode::Conflict|ErrorCode::AuthenticationRequired) {
            assert!(provider.take_response_credential().unwrap().is_none()); assert!(failure.take_caller_credential_update().is_none());
        }
        assert_eq!(read(&store,alias),original);
        assert_eq!(requests.await.unwrap().len(),last+1);
    }
}

#[tokio::test]
async fn artist_subscription_invalid_targets_and_missing_accounts_fail_before_network() {
    for (id, account, code) in [
        ("../103", "personal", ErrorCode::InvalidRequest),
        ("00103", "personal", ErrorCode::InvalidRequest),
        ("103", "missing", ErrorCode::AuthenticationRequired),
    ] {
        let (mut provider, requests) = server(vec![]).await;
        setup(&mut provider, "named");
        assert_eq!(
            provider
                .set_artist_subscription(id, true, Some(account))
                .await
                .unwrap_err()
                .code,
            code
        );
        assert!(requests.await.unwrap().is_empty());
    }
    let (provider, requests) = server(vec![]).await;
    assert_eq!(
        provider
            .set_artist_subscription("103", true, None)
            .await
            .unwrap_err()
            .code,
        ErrorCode::AuthenticationRequired
    );
    assert!(requests.await.unwrap().is_empty());
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
        for (i, response) in flow(true, false)
            .frames
            .into_iter()
            .take(at + 1)
            .enumerate()
        {
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
async fn artist_subscription_replacement_login_at_each_boundary_preserves_the_new_generation() {
    for at in 0..18 {
        let mut gate = gated_server(at).await;
        let (store, _, alias) = setup(&mut gate.provider, "named");
        let provider = gate.provider.clone();
        let task = tokio::spawn(async move {
            provider
                .set_artist_subscription("103", true, Some(alias))
                .await
        });
        tokio::time::timeout(Duration::from_secs(5), gate.seen)
            .await
            .unwrap()
            .unwrap();
        let replacement =
            MiguCredential::verified("111".into(), "replacement-login".into()).unwrap();
        store.put(&stored(alias, &replacement)).unwrap();
        gate.release.send(()).unwrap();
        let failure = task.await.unwrap().unwrap_err();
        assert_eq!(failure.code, ErrorCode::Conflict, "at {at}");
        if at >= 11 {
            assert_eq!(failure.details["write_outcome"], "unconfirmed");
            assert!(!failure.retryable);
        } else {
            assert!(failure.details.get("write_outcome").is_none());
        }
        assert_eq!(read(&store, alias), replacement);
        gate.task.await.unwrap();
    }
}

#[tokio::test]
async fn artist_subscription_cancellation_never_exposes_undelivered_caller_rotations() {
    for at in 0..18 {
        let mut gate = gated_server(at).await;
        let (store, original, alias) = setup(&mut gate.provider, "caller");
        let provider = gate.provider.clone();
        let task = tokio::spawn(async move {
            provider
                .set_artist_subscription("103", true, Some(alias))
                .await
        });
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
