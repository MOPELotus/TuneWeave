use super::*;
use crate::client::{
    catalog::tests::{json_response, response},
    native::tests as fixture,
};
use serde_json::Value;
use tokio::sync::Notify;

const SID: &str = "private-library-session";
pub(crate) fn created() -> Value {
    json!({"errcode":0,"result":"ok","plist":[
        {"type":"GENERAL","id":"101","title":"第二个 + %20","pic":"http://img2.kuwo.cn/star/userpl/test.jpg",
         "info":"第一行\n第二行","ispub":false,"playnum":"0","musicnum":"4","turn":2,"token":"never-export"},
        {"type":"MYFAVORITE","id":90,"title":"not-an-ordinary-playlist"},
        {"type":"MOBI_DEFAULT","hidden":1}, {"type":"PC_DEFAULT"}, {"type":"RADIO"}, {"type":"ORDER"},
        {"type":"GENERAL","id":102,"title":"第一个","turn":"1"}
    ]})
}
pub(crate) fn saved(start: u32, count: usize) -> Value {
    json!({"result":"ok","data":(0..count).map(|n| json!({
        "id":100+start+n as u32,"name":format!("收藏 {n}"),"desc":"简介", "total":"3",
        "pic":"https://img4.kuwo.cn/star/userpl/collection.jpg", "token":"never-export"
    })).collect::<Vec<_>>()})
}
pub(crate) fn flow() -> Vec<Vec<u8>> {
    vec![
        json_response(&json!({"result":"ok"})),
        json_response(&created()),
        json_response(&saved(0, 20)),
        json_response(&saved(20, 1)),
    ]
}
fn credential() -> ProviderCredential {
    fixture::credential_fixture("42", SID).caller().unwrap()
}
fn input() -> KuwoNativeSessionInput {
    credential::NativeCredential::parse(&credential())
        .unwrap()
        .input()
        .unwrap()
}
fn parse(value: &Value, section: Section) -> Result<Vec<Playlist>> {
    dto::parse(&serde_json::to_vec(value).unwrap(), &input(), section)
}

#[test]
fn native_library_types_sorting_unknowns_and_privacy_preserve_directory_semantics() {
    let items = parse(&created(), Section::Created).unwrap();
    assert_eq!(
        items.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(),
        ["102", "101"]
    );
    assert_eq!(items[1].name, "第二个 + %20");
    assert_eq!(items[1].track_count, Some(4));
    assert_eq!(items[1].extensions["is_public"], false);
    assert_eq!(items[1].extensions["owner_id"], "42");
    assert!(items[0].track_count.is_none());
    assert!(items[0].extensions["is_public"].is_null());
    assert!(
        items
            .iter()
            .all(|p| p.creator.is_none() && p.subscribed.is_none())
    );
    let collected = parse(&saved(0, 1), Section::Saved).unwrap();
    assert_eq!(collected[0].subscribed, Some(true));
    assert_eq!(collected[0].track_count, Some(3));
    assert!(collected[0].creator.is_none());
    assert!(!collected[0].extensions.contains_key("owner_id"));
    assert_eq!(collected[0].extensions["library_owner_id"], "42");
    let encoded = serde_json::to_string(&[items, collected]).unwrap();
    for secret in [SID, "never-export", "not-an-ordinary-playlist", "token"] {
        assert!(!encoded.contains(secret));
    }
    let mut unknown = created();
    unknown["plist"][6].as_object_mut().unwrap().remove("turn");
    let items = parse(&unknown, Section::Created).unwrap();
    assert_eq!(items[0].id, "101");
    assert_eq!(items[0].extensions["ordering"], "upstream");
    let mut ties = created();
    ties["plist"][6]["turn"] = json!(2);
    assert_eq!(parse(&ties, Section::Created).unwrap()[0].id, "101");
}

#[test]
fn native_library_rejects_ambiguous_success_identity_fields_and_reflected_secrets() {
    for bad in [
        json!({}),
        json!({"plist":[]}),
        json!({"errcode":0}),
        json!({"errcode":603,"plist":[]}),
        json!({"errcode":0,"result":"fail","plist":[]}),
        json!({"errcode":0,"uid":43,"plist":[]}),
        json!({"errcode":0,"plist":[{"type":"UNKNOWN"}]}),
    ] {
        assert!(parse(&bad, Section::Created).is_err(), "{bad}");
    }
    for (field, value) in [
        ("id", json!(0)),
        ("id", json!("0101")),
        ("title", json!(SID)),
        ("title", json!("")),
        ("title", json!("\u{0}")),
        ("info", json!("x".repeat(16 * 1024 + 1))),
        ("musicnum", json!(-1)),
        ("musicnum", json!(1.5)),
        ("turn", json!(2147483648_u64)),
        ("ispub", json!(0)),
        ("pic", json!("https://img1.kuwo.cn.evil.test/x")),
        (
            "pic",
            json!("https://img1.kuwo.cn/x?sid=private-library-session"),
        ),
        ("pic", json!("https://user@img1.kuwo.cn/x")),
        ("pic", json!("//img1.kuwo.cn/x")),
    ] {
        let mut bad = created();
        bad["plist"][0][field] = value;
        assert!(parse(&bad, Section::Created).is_err(), "{field}");
    }
    let mut duplicate = created();
    duplicate["plist"][6]["id"] = json!(101);
    assert!(parse(&duplicate, Section::Created).is_err());
    for raw in [
        r#"{"errcode":0,"errcode":603,"plist":[]}"#,
        r#"{"errcode":0,"plist":[{"type":"GENERAL","id":1,"id":2,"title":"x"}]}"#,
    ] {
        assert!(dto::parse(raw.as_bytes(), &input(), Section::Created).is_err());
    }
    for bad in [
        json!({"data":[]}),
        json!({"result":"ok"}),
        json!({"result":"fail","data":[]}),
        json!({"result":"ok","errcode":6,"data":[]}),
        json!({"result":"ok","uid":43,"data":[]}),
        saved(0, 21),
    ] {
        assert!(parse(&bad, Section::Saved).is_err());
    }
    let empty = json!({"result":"ok","data":[],"total":0});
    assert!(parse(&empty, Section::Saved).unwrap().is_empty());
}

#[tokio::test]
async fn native_library_sdk_reads_all_pages_and_binds_exact_selected_account_and_installation() {
    let mut replies = flow();
    replies[1] = response(
        200,
        "application/json",
        "Set-Cookie: alien=never-send; Path=/\r\n",
        &serde_json::to_vec(&created()).unwrap(),
    );
    let mut f = fixture::setup(replies).await;
    let page = f
        .client
        .native_account_playlists(&credential(), &PageRequest::new(3, 1))
        .await
        .unwrap();
    assert_eq!(
        page.items.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(),
        ["101", "100", "101"]
    );
    assert_eq!(page.items[0].extensions["library_section"], "created");
    assert_eq!(page.items[2].extensions["library_section"], "collected");
    assert_eq!(page.pagination.total, Some(23));
    assert_eq!(page.pagination.next_offset, Some(4));
    assert_eq!(page.pagination.extensions["upstream_pages_fetched"], 3);
    let requests = fixture::requests(&mut f, 4).await;
    assert!(requests[0].starts_with("GET /u.s?"));
    let input = input();
    for (i, r) in requests.iter().enumerate().skip(1) {
        let target = r.split_whitespace().nth(1).unwrap();
        let url = Url::parse(&format!("https://fixture.test{target}")).unwrap();
        let pairs = url.query_pairs().collect::<Vec<_>>();
        let map = pairs.iter().cloned().collect::<BTreeMap<_, _>>();
        assert_eq!(map.len(), pairs.len());
        assert_eq!(map["user"], input.device_user());
        assert_eq!(map["uid"], "42");
        assert_eq!(map["source"], CLIENT_SOURCE);
        if i == 1 {
            assert_eq!(url.path(), OWNED_PATH);
            assert_eq!(map["op"], "pl3_getuserlists");
            assert_eq!(map["sid"], SID);
            assert_eq!(map["devid"], input.device_id());
            assert_eq!(map["imei"], input.device_user());
            assert!(map["ttime"].parse::<u64>().unwrap() > 0);
            assert!(!map.contains_key("pn"));
        } else {
            assert_eq!(url.path(), SAVED_PATH);
            assert_eq!(map["type"], "get_like_sl");
            assert_eq!(map["loginUid"], "42");
            assert_eq!(map["loginSid"], SID);
            assert_eq!(
                map["android_id"],
                input.context.as_ref().unwrap().android_id
            );
            assert_eq!(map["count"], "20");
            assert_eq!(map["start"], if i == 2 { "0" } else { "20" });
        }
        assert!(r.contains(&format!(
            "loginUid=42,loginSid={SID},appUid={}",
            input.device_id()
        )));
        assert!(!r.to_lowercase().contains("\r\ncookie:"));
        assert!(!r.contains("alien"));
        assert!(!r.contains("ucheck"));
        assert!(!r.contains("adddev"));
    }
}

#[tokio::test]
async fn native_library_sdk_empty_scopes_and_out_of_range_windows_remain_distinct() {
    for created_only in [false, true] {
        for offset in [0, 100] {
            let body = if created_only {
                json!({"errcode":0,"plist":[]})
            } else {
                json!({"result":"ok","data":[],"total":0})
            };
            let mut f = fixture::setup(vec![
                json_response(&json!({"result":"ok"})),
                json_response(&body),
            ])
            .await;
            let request = PageRequest::new(10, offset);
            let page = if created_only {
                f.client
                    .native_created_playlists(&credential(), &request)
                    .await
            } else {
                f.client
                    .native_collected_playlists(&credential(), &request)
                    .await
            }
            .unwrap();
            assert!(page.items.is_empty());
            assert_eq!(page.pagination.total, Some(0));
            assert!(!page.pagination.has_more);
            assert!(page.pagination.next_offset.is_none());
            fixture::requests(&mut f, 2).await;
        }
    }
    let mut f = fixture::setup(flow()).await;
    let page = f
        .client
        .native_account_playlists(&credential(), &PageRequest::new(10, 100))
        .await
        .unwrap();
    assert!(page.items.is_empty());
    assert_eq!(page.pagination.total, Some(23));
    fixture::requests(&mut f, 4).await;
}

#[tokio::test]
async fn native_library_errors_and_duplicate_late_pages_never_return_partial_results_or_retry() {
    for bad in [
        json_response(&saved(0, 1)),
        json_response(&json!({"result":"fail","data":[]})),
        response(403, "application/json", "", b"private-error"),
        response(
            302,
            "application/json",
            "Location: https://evil.test/\r\n",
            b"",
        ),
        response(429, "application/json", "Retry-After: 2\r\n", b""),
        response(200, "text/html", "", b"<html>error</html>"),
        response(200, "application/json", "", b"{broken"),
    ] {
        let mut replies = flow();
        replies[3] = bad;
        let mut f = fixture::setup(replies).await;
        let error = f
            .client
            .native_account_playlists(&credential(), &PageRequest::new(1, 0))
            .await
            .unwrap_err();
        assert!(!format!("{error:?}").contains("private-error"));
        fixture::requests(&mut f, 4).await;
    }
    let mut f = fixture::setup(vec![json_response(
        &json!({"result":"fail","reason":"error_user_invalid"}),
    )])
    .await;
    assert_eq!(
        f.client
            .native_account_playlists(&credential(), &PageRequest::new(10, 0))
            .await
            .unwrap_err()
            .code,
        ErrorCode::AuthenticationRequired
    );
    fixture::requests(&mut f, 1).await;
}

#[tokio::test]
async fn native_library_preflight_and_read_budgets_do_not_silently_truncate() {
    let f = fixture::setup(vec![]).await;
    for (limit, offset) in [(0, 0), (101, 0), (1, u32::MAX)] {
        assert_eq!(
            f.client
                .native_account_playlists(&credential(), &PageRequest::new(limit, offset))
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
    }
    let mut request = PageRequest::new(10, 0);
    request.account = Some("personal".into());
    assert_eq!(
        f.client
            .native_account_playlists(&credential(), &request)
            .await
            .unwrap_err()
            .code,
        ErrorCode::InvalidRequest
    );
    let mut replies = vec![json_response(&json!({"result":"ok"}))];
    for i in 0..MAX_PAGES {
        replies.push(json_response(&saved((i * PAGE_SIZE) as u32, PAGE_SIZE)));
    }
    let mut f = fixture::setup(replies).await;
    assert!(
        f.client
            .native_collected_playlists(&credential(), &PageRequest::new(1, 0))
            .await
            .is_err()
    );
    fixture::requests(&mut f, MAX_PAGES + 1).await;
    let oversized = response(200, "application/json", "", &vec![b' '; MAX_RESPONSE + 1]);
    let mut f = fixture::setup(vec![json_response(&json!({"result":"ok"})), oversized]).await;
    assert!(
        f.client
            .native_created_playlists(&credential(), &PageRequest::new(1, 0))
            .await
            .is_err()
    );
    fixture::requests(&mut f, 2).await;
}

#[tokio::test]
async fn native_library_sdk_cancellation_at_each_network_boundary_never_yields_a_directory() {
    for boundary in 0..4 {
        let gate = Arc::new(Notify::new());
        let mut f = fixture::setup_gated(
            flow()
                .into_iter()
                .enumerate()
                .map(|(i, b)| (b, (i == boundary).then(|| gate.clone())))
                .collect(),
        )
        .await;
        let client = f.client.clone();
        let task = tokio::spawn(async move {
            client
                .native_account_playlists(&credential(), &PageRequest::new(1, 0))
                .await
        });
        for _ in 0..=boundary {
            tokio::time::timeout(Duration::from_secs(3), f.seen.recv())
                .await
                .unwrap()
                .unwrap();
        }
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        assert!(f.seen.try_recv().is_err());
    }
}
