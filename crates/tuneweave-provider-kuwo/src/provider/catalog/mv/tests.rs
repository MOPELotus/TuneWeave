use super::*;
use crate::client::{
    catalog::tests::{
        body, home, home_with, json_response, requests, response, setup, setup_with_gates,
    },
    native::tests as native_fixture,
};
use std::time::Duration;
use tokio::sync::Notify;
use tuneweave_core::{
    AccountCredentialStore, ErrorCode, StoredAccountCredential, VideoDetailRequest,
};

const COOKIE: &str = "anonymousCatalogueCookie123456";
fn query(limit: u32, offset: u32) -> SearchQuery {
    let mut q = SearchQuery::tracks(" 周杰伦 & + / ? ", limit, offset);
    q.kind = SearchKind::Mv;
    q
}
fn page(number: u32, total: u64) -> serde_json::Value {
    body(CatalogKind::Mv, number, total)
}
fn ids(items: &[SearchItem]) -> Vec<String> {
    items
        .iter()
        .map(|item| {
            let SearchItem::Video(v) = item else {
                panic!("wrong kind")
            };
            v.id.clone()
        })
        .collect()
}

#[tokio::test]
async fn mv_search_preserves_music_ids_offline_positions_credits_and_unknowns_without_permissions()
{
    let mut b = page(1, 3);
    let rows = b["data"]["mvlist"].as_array_mut().unwrap();
    rows[0]["artist"] = json!("Primary&amp;Name&Guest");
    rows[0]["artistid"] = json!(77);
    rows[0]["vid"] = json!(8132306);
    rows[0]["opaque"] = json!({"sid":"never-export","url":"https://foreign.invalid"});
    rows[1]["online"] = json!(0);
    rows[1]["artistid"] = json!(99);
    for field in ["online", "duration", "mvPlayCnt", "pic"] {
        rows[2].as_object_mut().unwrap().remove(field);
    }
    let mut f = setup(vec![home(), json_response(&b)]).await;
    let result = f.provider.search_catalog(&query(3, 0)).await.unwrap();
    assert_eq!(ids(&result.items), ["1", "2", "3"]);
    assert_eq!(result.pagination.total, Some(3));
    assert_eq!(
        result.pagination.extensions["pagination_scope"],
        "upstream_catalogue_positions"
    );
    let SearchItem::Video(v) = &result.items[0] else {
        panic!()
    };
    assert_eq!(v.resource_ref.to_string(), "kuwo:1");
    assert_eq!(v.extensions["source_track_id"], "1");
    assert_eq!(v.extensions["kind"], "mv");
    assert_eq!(v.title, "Video 1 & Live");
    assert_eq!(v.duration_ms, Some(269000));
    assert_eq!(v.play_count, Some(1234));
    assert_eq!(v.creators[0].name, "Primary&Name");
    assert_eq!(v.creators[0].resource_ref.as_ref().unwrap().id(), "77");
    assert!(v.creators[1].resource_ref.is_none());
    assert!(v.subscribed.is_none() && v.published_at.is_none());
    let SearchItem::Video(v) = &result.items[1] else {
        panic!()
    };
    assert_eq!(v.extensions["online"], 0);
    let SearchItem::Video(v) = &result.items[2] else {
        panic!()
    };
    assert!(!v.extensions.contains_key("online"));
    assert!(v.duration_ms.is_none() && v.play_count.is_none() && v.cover_url.is_none());
    let encoded = serde_json::to_string(&result).unwrap();
    for forbidden in [
        "never-export",
        "foreign.invalid",
        "8132306",
        "playable",
        "source_artist_id",
    ] {
        assert!(!encoded.contains(forbidden));
    }
    requests(&mut f, 2).await;
}

#[tokio::test]
async fn mv_search_maximum_windows_seed_total_once_and_sign_exact_pages() {
    for (offset, pages) in [
        (19, vec![1, 2, 3, 4, 5, 6]),
        (59, vec![1, 3, 4, 5, 6, 7, 8]),
    ] {
        let mut rows = vec![home()];
        rows.extend(pages.iter().map(|pn| json_response(&page(*pn, 162))));
        let mut f = setup(rows).await;
        let result = f
            .provider
            .search_catalog(&query(100, offset))
            .await
            .unwrap();
        assert_eq!(
            ids(&result.items),
            (offset + 1..offset + 101)
                .map(|id| id.to_string())
                .collect::<Vec<_>>()
        );
        assert_eq!(result.pagination.total, Some(162));
        assert_eq!(result.pagination.next_offset, Some(offset + 100));
        assert_eq!(
            result.pagination.extensions["upstream_pages_fetched"],
            pages.len()
        );
        let seen = requests(&mut f, 1 + pages.len()).await;
        assert!(!seen[0].to_lowercase().contains("cookie:"));
        let mut request_ids = BTreeSet::new();
        for (wire, pn) in seen[1..].iter().zip(pages) {
            let url = url::Url::parse(&format!(
                "https://www.kuwo.cn{}",
                wire.split_whitespace().nth(1).unwrap()
            ))
            .unwrap();
            assert_eq!(url.path(), "/api/www/search/searchMvBykeyWord");
            let params = url
                .query_pairs()
                .collect::<std::collections::BTreeMap<_, _>>();
            assert_eq!(params.len(), 7);
            assert_eq!(params["key"], "周杰伦 & + / ?");
            assert_eq!(params["pn"], pn.to_string());
            assert_eq!(params["rn"], "20");
            assert_eq!(params["plat"], "web_www");
            assert_eq!(params["httpsStatus"], "1");
            assert_eq!(params["from"], "");
            assert!(request_ids.insert(params["reqId"].to_string()));
            let lower = wire.to_lowercase();
            assert!(wire.contains(COOKIE));
            assert!(lower.contains("secret:"));
            assert!(lower.contains("referer: https://www.kuwo.cn/search/list"));
            assert!(!lower.contains("sid=") && !lower.contains("authorization:"));
        }
    }
}

#[tokio::test]
async fn mv_search_empty_end_and_out_of_range_keep_seed_total_and_skip_unnecessary_requests() {
    for (total, offset, expected, pages) in [
        (23, 19, 4, vec![1, 2]),
        (561, 560, 1, vec![1, 29]),
        (561, 580, 0, vec![1]),
        (0, 0, 0, vec![1]),
        (23, u32::MAX - 20, 0, vec![1]),
    ] {
        let mut rows = vec![home()];
        rows.extend(pages.iter().map(|pn| json_response(&page(*pn, total))));
        let mut f = setup(rows).await;
        let result = f.provider.search_catalog(&query(20, offset)).await.unwrap();
        assert_eq!(result.items.len(), expected);
        assert_eq!(result.pagination.total, Some(total));
        assert!(!result.pagination.has_more);
        assert_eq!(result.pagination.next_offset, None);
        requests(&mut f, 1 + pages.len()).await;
    }
}

#[tokio::test]
async fn mv_search_drift_duplicates_and_malformed_pages_never_return_partial_results() {
    for fault in [
        "total",
        "repeated-page",
        "duplicate",
        "short",
        "missing-total",
        "bad-kind",
        "bad-json",
    ] {
        let first = page(1, 45);
        let mut second = page(2, 45);
        match fault {
            "total" => second["data"]["total"] = json!(46),
            "repeated-page" => second["data"]["mvlist"] = first["data"]["mvlist"].clone(),
            "duplicate" => second["data"]["mvlist"][1] = second["data"]["mvlist"][0].clone(),
            "short" => {
                second["data"]["mvlist"].as_array_mut().unwrap().pop();
            }
            "missing-total" => {
                second["data"].as_object_mut().unwrap().remove("total");
            }
            "bad-kind" => {
                second["data"]["albumList"] = second["data"]["mvlist"].take();
            }
            _ => {}
        }
        let reply = if fault == "bad-json" {
            response(200, "application/json", "", b"{")
        } else {
            json_response(&second)
        };
        let mut f = setup(vec![home(), json_response(&first), reply]).await;
        assert_eq!(
            f.provider
                .search_catalog(&query(20, 19))
                .await
                .unwrap_err()
                .code,
            ErrorCode::UpstreamError,
            "{fault}"
        );
        requests(&mut f, 3).await;
    }
    // Also validate duplicate identities outside the slice the caller requested.
    let mut first = page(1, 3);
    first["data"]["mvlist"][2] = first["data"]["mvlist"][1].clone();
    let mut f = setup(vec![home(), json_response(&first)]).await;
    assert!(f.provider.search_catalog(&query(1, 0)).await.is_err());
    requests(&mut f, 2).await;
}

#[tokio::test]
async fn mv_search_metadata_boundaries_reject_bad_identity_but_do_not_invent_cover_or_missing_values()
 {
    for (key, value) in [
        ("id", json!("01")),
        ("id", json!(0)),
        ("artistid", json!(0)),
        ("artist", json!("")),
        ("name", json!("\u{0000}")),
        ("name", json!("a".repeat(513))),
        ("online", json!(2)),
        ("duration", json!(u64::MAX)),
        ("mvPlayCnt", json!(-1)),
    ] {
        let mut b = page(1, 1);
        b["data"]["mvlist"][0][key] = value;
        let mut f = setup(vec![home(), json_response(&b)]).await;
        assert_eq!(
            f.provider
                .search_catalog(&query(1, 0))
                .await
                .unwrap_err()
                .code,
            ErrorCode::UpstreamError,
            "{key}"
        );
        requests(&mut f, 2).await;
    }
    for image in [
        "http://img1.kuwo.cn/wmvpic/a.jpg",
        "https://img1.kuwo.cn.evil.test/wmvpic/a.jpg",
        "https://u@img1.kuwo.cn/wmvpic/a.jpg",
        "https://img1.kuwo.cn/wmvpic/a.jpg?token=secret",
    ] {
        let mut b = page(1, 1);
        b["data"]["mvlist"][0]["pic"] = json!(image);
        let mut f = setup(vec![home(), json_response(&b)]).await;
        let result = f.provider.search_catalog(&query(1, 0)).await.unwrap();
        let SearchItem::Video(v) = &result.items[0] else {
            panic!()
        };
        assert!(v.cover_url.is_none());
        requests(&mut f, 2).await;
    }
}

#[tokio::test]
async fn mv_search_signature_refresh_is_bounded_and_transport_failures_are_terminal() {
    for failed in [
        response(403, "text/plain", "", b"denied"),
        json_response(&json!({"success":false,"message":"The request is illegal!"})),
    ] {
        let mut f = setup(vec![
            home(),
            failed.clone(),
            home_with("refreshedMvAnonymousCookie1234"),
            json_response(&page(1, 1)),
        ])
        .await;
        assert_eq!(
            f.provider
                .search_catalog(&query(1, 0))
                .await
                .unwrap()
                .items
                .len(),
            1
        );
        let seen = requests(&mut f, 4).await;
        assert!(seen[3].contains("refreshedMvAnonymousCookie1234"));
        assert!(!seen[3].contains(COOKIE));
        let mut f = setup(vec![home(), failed.clone(), home(), failed]).await;
        assert!(f.provider.search_catalog(&query(1, 0)).await.is_err());
        requests(&mut f, 4).await;
    }
    for failed in [
        response(429, "application/json", "", b"{}"),
        response(500, "application/json", "", b"{}"),
        response(
            302,
            "text/html",
            "Location: https://foreign.invalid/\r\n",
            b"",
        ),
        response(200, "text/html", "", b"{}"),
        response(
            200,
            "application/json",
            "",
            &vec![b' '; 2 * 1024 * 1024 + 1],
        ),
        json_response(&json!({"code":-1,"msg":"private-message","data":null})),
    ] {
        let mut f = setup(vec![home(), failed]).await;
        let e = f.provider.search_catalog(&query(1, 0)).await.unwrap_err();
        assert!(!format!("{e:?}").contains("private-message"));
        requests(&mut f, 2).await;
    }
}

struct NoAccountAccess;
impl AccountCredentialStore for NoAccountAccess {
    fn load_platform(&self, _: Platform) -> Result<Vec<StoredAccountCredential>> {
        panic!("public search must not load saved accounts")
    }
    fn put(&self, _: &StoredAccountCredential) -> Result<()> {
        panic!("public search must not write credentials")
    }
    fn remove(&self, _: Platform, _: &str) -> Result<bool> {
        panic!("public search must not remove credentials")
    }
}
#[tokio::test]
async fn mv_search_public_scope_never_reads_saved_accounts_and_rejects_caller_options_before_network()
 {
    let mut f = setup(vec![home(), json_response(&page(1, 1))]).await;
    f.provider.credential_store = Some(Arc::new(NoAccountAccess));
    let valid = query(1, 0);
    let mut cases = vec![];
    let mut q = valid.clone();
    q.account = Some("default".into());
    cases.push(q);
    let mut q = valid.clone();
    q.variant = SearchVariant::Cloud;
    cases.push(q);
    let mut q = valid.clone();
    q.highlight = true;
    cases.push(q);
    let mut q = valid.clone();
    q.search_id = Some("other".into());
    cases.push(q);
    let mut q = valid.clone();
    q.selectors.push(tuneweave_core::SearchSelector {
        id: 1,
        name: "unproven".into(),
        selector_type: 1,
        extensions: Extensions::new(),
    });
    cases.push(q);
    let mut q = valid.clone();
    q.video_filters = Some(Default::default());
    cases.push(q);
    let mut q = valid.clone();
    q.offset = u32::MAX;
    cases.push(q);
    for limit in [0, 101] {
        let mut q = valid.clone();
        q.limit = limit;
        cases.push(q);
    }
    for text in ["".into(), "a".repeat(513), "a\0b".into()] {
        let mut q = valid.clone();
        q.query = text;
        cases.push(q);
    }
    for q in cases {
        assert_eq!(
            f.provider.search_catalog(&q).await.unwrap_err().code,
            ErrorCode::InvalidRequest
        );
    }
    let credential = native_fixture::credential_fixture("42", "private-search-session")
        .caller()
        .unwrap();
    let caller = f.provider.with_caller_credential(&credential).unwrap();
    assert!(caller.search_catalog(&valid).await.is_err());
    assert!(caller.take_response_credential().unwrap().is_none());
    assert_eq!(
        f.provider.search_catalog(&valid).await.unwrap().items.len(),
        1
    );
    let seen = requests(&mut f, 2).await;
    assert!(seen.iter().all(|v| !v.contains("private-search-session")));
}

#[tokio::test]
async fn mv_search_total_deadline_and_cancellation_cover_bootstrap_and_each_page() {
    for boundary in 0..3 {
        for cancel in [false, true] {
            let gate = Arc::new(Notify::new());
            let mut replies = vec![
                home(),
                json_response(&page(1, 23)),
                json_response(&page(2, 23)),
            ];
            replies.truncate(boundary + 1);
            let mut f = setup_with_gates(
                replies
                    .into_iter()
                    .enumerate()
                    .map(|(i, b)| (b, (i == boundary).then(|| gate.clone())))
                    .collect(),
            )
            .await;
            native_fixture::set_request_timeout(&mut f.provider.client, Duration::from_secs(60));
            let provider = f.provider.clone();
            let task = tokio::spawn(async move { provider.search_catalog(&query(20, 19)).await });
            for _ in 0..=boundary {
                tokio::time::timeout(Duration::from_secs(5), f.seen.recv())
                    .await
                    .unwrap()
                    .unwrap();
            }
            if cancel {
                task.abort();
                assert!(task.await.unwrap_err().is_cancelled());
            } else {
                tokio::time::pause();
                let result = task.await;
                tokio::time::resume();
                let error = result.unwrap().unwrap_err();
                assert_eq!(error.code, ErrorCode::UpstreamTimeout);
                assert!(error.message.contains("total time budget"));
            }
            gate.notify_one();
            (&mut f.server).await.unwrap();
            assert!(f.seen.try_recv().is_err());
            assert!(f.provider.take_response_credential().unwrap().is_none());
        }
    }
}

#[tokio::test]
#[ignore = "Official anonymous MV search and metadata only; no accounts or media"]
async fn live_mv_search_cross_page_empty_and_details_use_the_same_music_identity() {
    let provider = KuwoProvider::new(KuwoConfig::default()).unwrap();
    let mut q = query(4, 18);
    q.query = "周杰伦".into();
    let result = provider.search_catalog(&q).await.unwrap();
    assert_eq!(result.items.len(), 4);
    assert_eq!(result.pagination.extensions["upstream_pages_fetched"], 2);
    let SearchItem::Video(v) = &result.items[0] else {
        panic!()
    };
    let detail = provider
        .video(
            &v.id,
            &VideoDetailRequest::new(tuneweave_core::VideoResourceKind::Mv),
        )
        .await
        .unwrap();
    assert_eq!(detail.video.resource_ref, v.resource_ref);
    q.offset = u32::try_from(result.pagination.total.unwrap() + 20).unwrap();
    let beyond = provider.search_catalog(&q).await.unwrap();
    assert!(beyond.items.is_empty());
    assert!(beyond.pagination.total.unwrap() > 0);
    assert_eq!(beyond.pagination.extensions["upstream_pages_fetched"], 1);
    q.query = "zzzzTuneWeaveMvNoMatch928415".into();
    q.offset = 0;
    let empty = provider.search_catalog(&q).await.unwrap();
    assert_eq!(empty.pagination.total, Some(0));
    assert!(empty.items.is_empty());
}
