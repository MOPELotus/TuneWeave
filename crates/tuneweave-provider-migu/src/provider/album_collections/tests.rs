use super::*;
use crate::client::album_collections::tests::entry;
use crate::provider::{
    favorites::tests::server,
    session::tests::{Store, gated, read, stored},
};
use std::time::Duration;

const KINDS: [AlbumKind; 2] = [AlbumKind::Ordinary, AlbumKind::Digital];
const TITLE: &str = "专辑 A & B # / 100%";
#[derive(Clone, Copy)]
enum Action {
    Read,
    Write(bool),
}
struct Flow {
    values: Vec<serde_json::Value>,
    labels: Vec<&'static str>,
}
impl Flow {
    fn new() -> Self {
        Self {
            values: Vec::new(),
            labels: Vec::new(),
        }
    }
    fn push(&mut self, label: &'static str, value: serde_json::Value) {
        self.labels.push(label);
        self.values.push(value);
    }
    fn profile(&mut self) {
        self.push(
            "profile",
            json!({"code":"000000","data":{"userId":"111","nickName":"Account"}}),
        );
    }
    fn pair(&mut self, label: &'static str, value: serde_json::Value) {
        self.push(label, value);
        self.profile();
    }
    fn library(&mut self, items: &[serde_json::Value]) {
        for chunk in items.chunks(10) {
            self.pair(
                "library",
                json!({"code":"000000","collections":chunk,"totalCount":items.len()}),
            );
        }
        if items.is_empty() {
            self.pair(
                "library",
                json!({"code":"000000","collections":[],"totalCount":0}),
            );
        }
    }
    fn writing(&mut self, kind: AlbumKind, id: &str, subscribed: bool, retained: Option<&str>) {
        if subscribed {
            let mut value = entry(kind, id);
            value["title"] = json!(TITLE);
            self.pair("metadata", json!({"code":"000000","data":value}));
        }
        self.pair("write", json!({"code":"000000"}));
        let mut items = Vec::new();
        if subscribed {
            items.push(entry(kind, id));
        }
        items.push(entry(other(kind), id));
        if let Some(id) = retained {
            items.push(entry(kind, id));
        }
        items.extend(mixed());
        self.library(&items);
        self.pair("state",json!([{"resourceType":kind.resource_type(),"resourceId":id,"opType":"03","isOP":if subscribed{"00"}else{"01"},"userId":"111"}]));
    }
    fn at(&self, label: &str) -> usize {
        self.labels.iter().position(|s| *s == label).unwrap()
    }
    fn truncate(&mut self, len: usize) {
        self.values.truncate(len);
        self.labels.truncate(len);
    }
    fn wire(&self) -> Vec<String> {
        self.values.iter().enumerate().map(|(i,v)|{let body=v.to_string();format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\npacmtoken: p{i}\r\nConnection: close\r\n\r\n{body}",body.len())}).collect()
    }
}
fn other(kind: AlbumKind) -> AlbumKind {
    if kind == AlbumKind::Ordinary {
        AlbumKind::Digital
    } else {
        AlbumKind::Ordinary
    }
}
fn mixed() -> Vec<serde_json::Value> {
    (0..23)
        .map(|i| {
            entry(
                if i % 2 == 0 {
                    AlbumKind::Ordinary
                } else {
                    AlbumKind::Digital
                },
                &(i / 2 + 1).to_string(),
            )
        })
        .collect()
}
fn frames(kind: AlbumKind, action: Action) -> Flow {
    let mut f = Flow::new();
    f.profile();
    match action {
        Action::Read => f.library(&mixed()),
        Action::Write(subscribed) => f.writing(kind, "77", subscribed, None),
    };
    f
}
fn setup(p: &mut MiguProvider) -> (Arc<Store>, MiguCredential, MiguCredential) {
    let store = Arc::new(Store::default());
    let a = MiguCredential::verified("111".into(), "initial".into()).unwrap();
    let b = MiguCredential::verified("222".into(), "other".into()).unwrap();
    store.put(&stored("A", &a)).unwrap();
    store.put(&stored("B", &b)).unwrap();
    p.credential_store = Some(store.clone());
    (store, a, b)
}
async fn reading(
    p: &MiguProvider,
    kind: AlbumKind,
    user: bool,
    r: &PageRequest,
) -> Result<serde_json::Value> {
    match (kind, user) {
        (AlbumKind::Ordinary, false) => p.account_albums(r).await.map(|p| json!(p)),
        (AlbumKind::Ordinary, true) => p.user_favorite_albums("111", r).await.map(|p| json!(p)),
        (AlbumKind::Digital, false) => p.account_digital_albums(r).await.map(|p| json!(p)),
        (AlbumKind::Digital, true) => p
            .user_favorite_digital_albums("111", r)
            .await
            .map(|p| json!(p)),
    }
}
async fn run(
    p: &MiguProvider,
    kind: AlbumKind,
    action: Action,
    account: Option<&str>,
) -> Result<serde_json::Value> {
    match action {
        Action::Read => {
            reading(
                p,
                kind,
                false,
                &PageRequest {
                    limit: 3,
                    offset: 2,
                    account: account.map(str::to_owned),
                },
            )
            .await
        }
        Action::Write(subscribed) => match kind {
            AlbumKind::Ordinary => p
                .set_album_subscription("77", subscribed, account)
                .await
                .map(|v| json!(v)),
            AlbumKind::Digital => p
                .set_digital_album_subscription("77", subscribed, account)
                .await
                .map(|v| json!(v)),
        },
    }
}
fn is_write(request: &str) -> bool {
    request.starts_with("GET /pc/v1.0/user/add_collection.do?")
        || request.starts_with("GET /pc/v1.0/user/del_collection.do?")
}
fn assert_rotations(requests: &[String]) {
    for (i, r) in requests.iter().enumerate() {
        let token = if i == 0 {
            "initial".into()
        } else {
            format!("p{}", i - 1)
        };
        assert!(r.contains(&format!("pacmtoken: {token}\r\n")), "{i}");
        assert!(!r.contains("cookie:"));
        assert!(!r.contains("other"));
    }
}

#[tokio::test]
async fn typed_album_libraries_read_all_mixed_pages_before_filtering_and_slicing() {
    for kind in KINDS {
        for caller in [false, true] {
            for user in [false, true] {
                for (limit, offset) in [(3, 2), (4, 8), (2, 99)] {
                    let f = frames(kind, Action::Read);
                    let (mut p, requests) = server(f.wire()).await;
                    let (store, a, b) = setup(&mut p);
                    let alias = if caller {
                        p = p.caller_scope(&a.caller().unwrap()).unwrap();
                        "default"
                    } else {
                        "A"
                    };
                    let result = reading(
                        &p,
                        kind,
                        user,
                        &PageRequest {
                            limit,
                            offset,
                            account: Some(alias.into()),
                        },
                    )
                    .await
                    .unwrap();
                    let total = if kind == AlbumKind::Ordinary { 12 } else { 11 };
                    assert_eq!(result["pagination"]["total"], total);
                    assert_eq!(
                        result["pagination"]["extensions"]["upstream_raw_collection_count"],
                        23
                    );
                    let ids = result["items"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|item| {
                            assert_eq!(item["extensions"]["resource_type"], kind.resource_type());
                            item["id"].as_str().unwrap().to_owned()
                        })
                        .collect::<Vec<_>>();
                    assert_eq!(
                        ids,
                        (1..=total)
                            .skip(offset as usize)
                            .take(limit as usize)
                            .map(|id| id.to_string())
                            .collect::<Vec<_>>()
                    );
                    let update = p.take_response_credential().unwrap();
                    if caller {
                        assert_eq!(
                            MiguCredential::parse_caller(&update.unwrap())
                                .unwrap()
                                .token(),
                            "p6"
                        );
                        assert_eq!(read(&store, "A"), a);
                    } else {
                        assert!(update.is_none());
                        assert_eq!(read(&store, "A").token(), "p6");
                    }
                    assert_eq!(read(&store, "B"), b);
                    let requests = requests.await.unwrap();
                    assert_eq!(requests.len(), 7);
                    assert_rotations(&requests);
                    for (i, r) in requests
                        .iter()
                        .filter(|r| r.starts_with("GET /pc/v1.0/user/collections.do?"))
                        .enumerate()
                    {
                        assert!(r.starts_with(&format!("GET /pc/v1.0/user/collections.do?pageNo={}&pageSize=10&type=1&oPType=03&resourceType=2003%7C5 ",i+1)));
                        assert!(r.contains("channel: 014X031\r\n"));
                    }
                }
            }
        }
    }
}

#[tokio::test]
async fn collection_writes_use_exact_resource_types_and_two_independent_confirmations() {
    for kind in KINDS {
        for subscribed in [false, true] {
            for caller in [false, true] {
                let f = frames(kind, Action::Write(subscribed));
                let (mut p, requests) = server(f.wire()).await;
                let (store, a, b) = setup(&mut p);
                let alias = if caller {
                    p = p.caller_scope(&a.caller().unwrap()).unwrap();
                    "default"
                } else {
                    "A"
                };
                let result = run(&p, kind, Action::Write(subscribed), Some(alias))
                    .await
                    .unwrap();
                assert_eq!(result["resource_ref"], "migu:77");
                assert_eq!(result["subscribed"], subscribed);
                assert_eq!(result["extensions"]["resource_type"], kind.resource_type());
                let update = p.take_response_credential().unwrap();
                if caller {
                    assert_eq!(
                        MiguCredential::parse_caller(&update.unwrap())
                            .unwrap()
                            .token(),
                        format!("p{}", f.values.len() - 1)
                    );
                    assert_eq!(read(&store, "A"), a);
                } else {
                    assert!(update.is_none());
                    assert_eq!(
                        read(&store, "A").token(),
                        format!("p{}", f.values.len() - 1)
                    );
                }
                assert_eq!(read(&store, "B"), b);
                let requests = requests.await.unwrap();
                assert_rotations(&requests);
                assert_eq!(requests.len(), f.values.len());
                let writes: Vec<_> = requests.iter().filter(|r| is_write(r)).collect();
                assert_eq!(writes.len(), 1);
                let url = url::Url::parse(&format!(
                    "https://app.c.nf.migu.cn{}",
                    writes[0].split_whitespace().nth(1).unwrap()
                ))
                .unwrap();
                let params: std::collections::BTreeMap<_, _> = url
                    .query_pairs()
                    .map(|(k, v)| (k.into_owned(), v.into_owned()))
                    .collect();
                let expected = if subscribed {
                    vec![
                        ("outOPType", "03"),
                        ("outResourceName", TITLE),
                        ("outResourceId", "77"),
                        ("outResourceType", kind.resource_type()),
                    ]
                } else {
                    vec![
                        ("oPType", "03"),
                        ("resourceId", "77"),
                        ("resourceType", kind.resource_type()),
                    ]
                };
                assert_eq!(
                    params,
                    expected
                        .into_iter()
                        .map(|(k, v)| (k.into(), v.into()))
                        .collect()
                );
                assert!(requests[requests.len() - 2].starts_with(&format!(
                    "GET /pc/query-ops/{}?opType=03&resourceId=77 ",
                    kind.resource_type()
                )));
                if subscribed {
                    let path = if kind == AlbumKind::Ordinary {
                        "/resource/album/v2.0?albumId=77"
                    } else {
                        "/pc/resource/dalbum/v2.0?dAlbumId=77"
                    };
                    assert!(requests[1].starts_with(&format!("GET {path} ")));
                } else {
                    assert!(
                        !requests
                            .iter()
                            .any(|r| r.starts_with("GET /resource/album/")
                                || r.starts_with("GET /pc/resource/dalbum/"))
                    );
                }
            }
        }
    }
}

#[tokio::test]
async fn malformed_later_pages_and_unknown_collection_state_never_become_partial_success() {
    for kind in KINDS {
        for action in [Action::Read, Action::Write(true), Action::Write(false)] {
            for variant in 0..5 {
                let mut f = frames(kind, action);
                let last_library = f.labels.iter().rposition(|v| *v == "library").unwrap();
                let count = if variant < 3 || matches!(action, Action::Read) {
                    let i = last_library;
                    match variant {
                        0 => f.values[i]["collections"] = json!(null),
                        1 => f.values[i]["collections"][0] = entry(kind, "1"),
                        2 => f.values[i]["totalCount"] = json!(600),
                        3 => f.values[i]["collections"][0]["resourceType"] = json!("2021"),
                        _ => f.values[i]["userId"] = json!("222"),
                    };
                    i + if variant == 4 { 1 } else { 2 }
                } else {
                    let i = f.at("state");
                    f.values[i] = if variant == 3 {
                        json!([{"isOP":"02"}])
                    } else {
                        json!([{"isOP":"00","userId":"222"}])
                    };
                    i + if variant == 4 { 1 } else { 2 }
                };
                f.truncate(count);
                let (mut p, requests) = server(f.wire()).await;
                let (store, a, b) = setup(&mut p);
                let p = p.caller_scope(&a.caller().unwrap()).unwrap();
                let mut e = run(&p, kind, action, None).await.unwrap_err();
                let auth = variant == 4;
                assert_eq!(
                    e.code,
                    if auth {
                        ErrorCode::AuthenticationRequired
                    } else {
                        ErrorCode::UpstreamError
                    }
                );
                assert_eq!(
                    e.details.get("write_outcome").is_some(),
                    matches!(action, Action::Write(_))
                );
                let update = e.take_caller_credential_update();
                assert_eq!(update.is_none(), auth);
                if let Some(update) = update {
                    assert_eq!(
                        MiguCredential::parse_caller(&update).unwrap().token(),
                        format!("p{}", count - 1)
                    );
                }
                assert_eq!(read(&store, "A"), a);
                assert_eq!(read(&store, "B"), b);
                assert_eq!(
                    requests
                        .await
                        .unwrap()
                        .iter()
                        .filter(|r| is_write(r))
                        .count(),
                    usize::from(matches!(action, Action::Write(_)))
                );
            }
        }
    }
}

#[tokio::test]
async fn album_state_contradictions_wrong_resource_type_and_success_envelopes_are_not_confirmation()
{
    for kind in KINDS {
        for subscribed in [false, true] {
            for variant in 0..6 {
                let mut f = frames(kind, Action::Write(subscribed));
                let i = f.at("state");
                f.values[i] = match variant {
                    0 => json!([]),
                    1 => json!([{"isOP":if subscribed{"01"}else{"00"}}]),
                    2 => json!([{"isOP":"00","resourceType":other(kind).resource_type()}]),
                    3 => json!({"code":"000000","data":[{"isOP":"00"}]}),
                    4 => json!([{"isOP":"00","resourceId":"88"}]),
                    _ => json!({"code":"290001"}),
                };
                let auth = variant == 5;
                let count = i + if auth || variant == 3 { 1 } else { 2 };
                f.truncate(count);
                let (mut p, requests) = server(f.wire()).await;
                let (store, _, b) = setup(&mut p);
                let e = run(&p, kind, Action::Write(subscribed), Some("A"))
                    .await
                    .unwrap_err();
                assert_eq!(
                    e.code,
                    if auth {
                        ErrorCode::AuthenticationRequired
                    } else {
                        ErrorCode::UpstreamError
                    }
                );
                assert_eq!(e.details["write_outcome"], "unconfirmed");
                assert!(!e.retryable);
                assert!(p.take_response_credential().unwrap().is_none());
                if auth {
                    assert!(
                        !store
                            .load_platform(Platform::Migu)
                            .unwrap()
                            .iter()
                            .any(|v| v.account == "A")
                    );
                } else {
                    assert_eq!(
                        read(&store, "A").token(),
                        format!("p{}", if variant == 3 { i - 1 } else { count - 1 })
                    );
                }
                assert_eq!(read(&store, "B"), b);
                assert_eq!(
                    requests
                        .await
                        .unwrap()
                        .iter()
                        .filter(|r| is_write(r))
                        .count(),
                    1
                );
            }
        }
    }
}

#[tokio::test]
async fn album_library_limits_missing_sources_and_foreign_user_requests_fail_before_network() {
    let (mut p, requests) = server(vec![]).await;
    let (_, a, _) = setup(&mut p);
    for kind in KINDS {
        for (limit, offset) in [(0, 0), (101, 0), (2, u32::MAX)] {
            let r = PageRequest {
                limit,
                offset,
                account: Some("A".into()),
            };
            assert_eq!(
                reading(&p, kind, false, &r).await.unwrap_err().code,
                ErrorCode::InvalidRequest
            );
        }
        for ids in [
            vec![],
            vec!["77".to_owned(); 2],
            vec!["077".into()],
            vec!["bad/id".into()],
            vec!["7".repeat(65)],
            (1..=101).map(|i| i.to_string()).collect(),
        ] {
            assert_eq!(
                p.change_album_collections(kind, &ids, true, Some("A"))
                    .await
                    .unwrap_err()
                    .code,
                ErrorCode::InvalidRequest
            );
        }
        assert_eq!(
            p.collected_album_page(
                kind,
                Some("222"),
                &PageRequest {
                    limit: 1,
                    offset: 0,
                    account: Some("A".into())
                }
            )
            .await
            .err()
            .unwrap()
            .code,
            ErrorCode::PermissionDenied
        );
        assert_eq!(
            run(&p, kind, Action::Read, Some("missing"))
                .await
                .unwrap_err()
                .code,
            ErrorCode::AuthenticationRequired
        );
    }
    let p = p.caller_scope(&a.caller().unwrap()).unwrap();
    assert_eq!(
        run(&p, AlbumKind::Digital, Action::Read, Some("A"))
            .await
            .unwrap_err()
            .code,
        ErrorCode::InvalidRequest
    );
    assert!(requests.await.unwrap().is_empty());
}

#[tokio::test]
async fn mixed_collection_short_end_and_full_page_budget_do_not_skip_filtered_items() {
    for total in [0usize, 20] {
        let mut f = Flow::new();
        f.profile();
        let items = mixed().into_iter().take(total).collect::<Vec<_>>();
        f.library(&items);
        // With no total, a full final page needs an additional empty page.
        for value in &mut f.values {
            if value.get("collections").is_some() {
                value.as_object_mut().unwrap().remove("totalCount");
            }
        }
        if total == 20 {
            f.pair("library", json!({"code":"000000","collections":[]}));
        }
        let (mut p, requests) = server(f.wire()).await;
        setup(&mut p);
        let result = run(&p, AlbumKind::Ordinary, Action::Read, Some("A"))
            .await
            .unwrap();
        assert_eq!(result["pagination"]["total"], total / 2);
        assert_eq!(requests.await.unwrap().len(), f.values.len());
    }
    let mut f = Flow::new();
    f.profile();
    for page in 0..64 {
        let items: Vec<_> = (0..10)
            .map(|i| entry(AlbumKind::Digital, &(page * 10 + i + 1).to_string()))
            .collect();
        f.pair("library", json!({"code":"000000","collections":items}));
    }
    let (mut p, requests) = server(f.wire()).await;
    setup(&mut p);
    assert_eq!(
        run(&p, AlbumKind::Ordinary, Action::Read, Some("A"))
            .await
            .unwrap_err()
            .code,
        ErrorCode::UpstreamError
    );
    assert_eq!(requests.await.unwrap().len(), 129);
}

#[tokio::test]
async fn album_collection_batches_keep_one_source_and_report_confirmed_and_unattempted_items() {
    for kind in KINDS {
        for fail_before_dispatch in [false, true] {
            let mut f = frames(kind, Action::Write(true));
            if fail_before_dispatch {
                f.pair(
                    "metadata",
                    json!({"code":"000000","data":entry(other(kind),"78")}),
                );
            } else {
                f.pair("metadata", json!({"code":"000000","data":entry(kind,"78")}));
                f.push("write", json!({"code":"299999"}));
            }
            let (mut p, requests) = server(f.wire()).await;
            let (store, a, b) = setup(&mut p);
            let p = p.caller_scope(&a.caller().unwrap()).unwrap();
            let mut e = p
                .change_album_collections(
                    kind,
                    &["77".into(), "78".into(), "79".into()],
                    true,
                    None,
                )
                .await
                .unwrap_err();
            assert_eq!(e.details["atomic"], false);
            assert_eq!(e.details["completed_refs"], json!(["migu:77"]));
            assert_eq!(e.details["failed_ref"], "migu:78");
            assert_eq!(e.details["remaining_refs"], json!(["migu:79"]));
            assert_eq!(e.details["failed_write_dispatched"], !fail_before_dispatch);
            assert_eq!(e.details["write_outcome"], "unconfirmed");
            assert!(!e.retryable);
            let last = f.labels.iter().rposition(|v| *v == "profile").unwrap();
            assert_eq!(
                MiguCredential::parse_caller(&e.take_caller_credential_update().unwrap())
                    .unwrap()
                    .token(),
                format!("p{last}")
            );
            assert_eq!(read(&store, "A"), a);
            assert_eq!(read(&store, "B"), b);
            let requests = requests.await.unwrap();
            assert_rotations(&requests);
            assert_eq!(
                requests.iter().filter(|r| is_write(r)).count(),
                if fail_before_dispatch { 1 } else { 2 }
            );
        }
    }
    for kind in KINDS {
        let mut f = frames(kind, Action::Write(true));
        f.writing(kind, "78", true, Some("77"));
        let (mut p, requests) = server(f.wire()).await;
        setup(&mut p);
        let results = match kind {
            AlbumKind::Ordinary => {
                p.set_album_subscriptions(&["77".into(), "78".into()], true, Some("A"))
                    .await
            }
            AlbumKind::Digital => {
                p.set_digital_album_subscriptions(&["77".into(), "78".into()], true, Some("A"))
                    .await
            }
        }
        .unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[1].resource_ref.id(), "78");
        let requests = requests.await.unwrap();
        assert_rotations(&requests);
        assert_eq!(requests.iter().filter(|r| is_write(r)).count(), 2);
    }
}

#[tokio::test]
async fn every_album_collection_read_or_write_boundary_rejects_late_source_replacement() {
    for kind in KINDS {
        for action in [Action::Read, Action::Write(true), Action::Write(false)] {
            let f = frames(kind, action);
            let replies = f.wire();
            for caller in [false, true] {
                for stage in 1..=replies.len() {
                    let (mut p, seen, release, server) = gated(replies[..stage].to_vec()).await;
                    let (store, a, b) = setup(&mut p);
                    let alias = if caller {
                        p = p.caller_scope(&a.caller().unwrap()).unwrap();
                        "default"
                    } else {
                        "A"
                    };
                    let p = Arc::new(p);
                    let worker = p.clone();
                    let task =
                        tokio::spawn(async move { run(&worker, kind, action, Some(alias)).await });
                    tokio::time::timeout(Duration::from_secs(5), seen)
                        .await
                        .unwrap()
                        .unwrap();
                    let newer = MiguCredential::verified("111".into(), "new-login".into()).unwrap();
                    if caller {
                        *p.caller_credential.as_ref().unwrap().lock().unwrap() = newer.clone();
                    } else if stage % 2 == 0 {
                        store.remove(Platform::Migu, "A").unwrap();
                    } else {
                        store.put(&stored("A", &newer)).unwrap();
                    }
                    release.send(()).unwrap();
                    let mut e = tokio::time::timeout(Duration::from_secs(5), task)
                        .await
                        .unwrap()
                        .unwrap()
                        .unwrap_err();
                    assert_eq!(e.code, ErrorCode::Conflict);
                    assert!(e.take_caller_credential_update().is_none());
                    assert!(p.take_response_credential().unwrap().is_none());
                    assert_eq!(
                        e.details.get("write_outcome").is_some(),
                        f.labels[..stage].contains(&"write")
                    );
                    if caller {
                        assert_eq!(read(&store, "A"), a);
                    } else if stage % 2 == 1 {
                        assert_eq!(read(&store, "A"), newer);
                    } else {
                        assert!(
                            !store
                                .load_platform(Platform::Migu)
                                .unwrap()
                                .iter()
                                .any(|v| v.account == "A")
                        );
                    }
                    assert_eq!(read(&store, "B"), b);
                    server.await.unwrap();
                }
            }
        }
    }
}

#[tokio::test]
async fn album_collection_timeouts_export_only_last_profile_verified_tokens() {
    for kind in KINDS {
        for action in [Action::Read, Action::Write(true), Action::Write(false)] {
            let f = frames(kind, action);
            let replies = f.wire();
            for stage in 1..=replies.len() {
                let (mut p, seen, release, server) = gated(replies[..stage].to_vec()).await;
                let (_, a, _) = setup(&mut p);
                p.client = p
                    .client
                    .with_session_test_timeout(Duration::from_millis(200));
                let p = Arc::new(p.caller_scope(&a.caller().unwrap()).unwrap());
                let worker = p.clone();
                let task = tokio::spawn(async move { run(&worker, kind, action, None).await });
                tokio::time::timeout(Duration::from_secs(5), seen)
                    .await
                    .unwrap()
                    .unwrap();
                let mut e = tokio::time::timeout(Duration::from_secs(5), task)
                    .await
                    .unwrap()
                    .unwrap()
                    .unwrap_err();
                assert_eq!(e.code, ErrorCode::UpstreamTimeout);
                let write = f.labels[..stage].contains(&"write");
                assert_eq!(e.details.get("write_outcome").is_some(), write);
                if write {
                    assert!(!e.retryable);
                }
                let expected = f.labels[..stage - 1]
                    .iter()
                    .rposition(|label| *label == "profile")
                    .map(|i| format!("p{i}"));
                assert_eq!(
                    e.take_caller_credential_update()
                        .map(|v| MiguCredential::parse_caller(&v).unwrap().token().to_owned()),
                    expected
                );
                assert_eq!(
                    p.take_response_credential()
                        .unwrap()
                        .map(|v| MiguCredential::parse_caller(&v).unwrap().token().to_owned()),
                    expected
                );
                server.abort();
                assert!(server.await.unwrap_err().is_cancelled());
                drop(release);
            }
        }
    }
}
