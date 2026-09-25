use super::super::session::tests::{Store, credential, raw, server};
use super::*;
use serde_json::{Value, json};
use std::collections::BTreeMap;

fn query(kind: SearchKind, limit: u32, offset: u32) -> SearchQuery {
    let mut q = SearchQuery::tracks(" Test & 中文 ", limit, offset);
    q.kind = kind;
    q
}
fn response(kind: SearchKind, page: u32, total: u64) -> Value {
    let start = u64::from(page - 1) * 20;
    let items:Vec<_>=(start..total.min(start+20)).map(|n| match kind {
        SearchKind::Album=>json!({"albumid":n+1,"albumname":format!("Album {n}")}),
        SearchKind::Artist=>json!({"AuthorId":n+1,"AuthorName":format!("Artist {n}")}),
        SearchKind::Playlist=>json!({"gid":format!("collection_3_222_{}_0",n+1),"specialname":format!("Playlist {n}")}),
        _=>unreachable!(),
    }).collect();
    json!({"status":1,"error_code":0,"data":{"page":page,"pagesize":20,"from":start,"size":20,"total":total,"lists":items}})
}

#[tokio::test]
async fn catalogue_search_crosses_pages_for_all_three_kinds_with_only_anonymous_signed_parameters()
{
    for (kind, path) in [
        (SearchKind::Album, "album"),
        (SearchKind::Artist, "author"),
        (SearchKind::Playlist, "special"),
    ] {
        let mut f = server(vec![
            raw(response(kind, 1, 28)).into(),
            raw(response(kind, 2, 28)).into(),
        ])
        .await;
        let store = Arc::new(Store::default());
        let old = credential("999", "private-account-secret");
        store.put(&old.stored("default").unwrap()).unwrap();
        f.provider.credential_store = Some(store.clone());
        let p = f
            .provider
            .search_catalog(&query(kind, 8, 17))
            .await
            .unwrap();
        assert_eq!(p.items.len(), 8);
        assert_eq!(p.pagination.total, Some(28));
        assert_eq!(p.pagination.next_offset, Some(25));
        assert!(p.pagination.has_more);
        let expected = if kind == SearchKind::Playlist {
            "collection_3_222_18_0"
        } else {
            "18"
        };
        assert_eq!(item_id(&p.items[0]), expected);
        let requests = f.requests.await.unwrap();
        assert_eq!(requests.len(), 2);
        let mut first_mid = None;
        for (i, r) in requests.iter().enumerate() {
            assert!(r.starts_with(&format!("GET /v1/search/{path}?")));
            let uri = r.lines().next().unwrap().split_whitespace().nth(1).unwrap();
            let url = url::Url::parse(&format!("http://localhost{uri}")).unwrap();
            let mut params: BTreeMap<_, _> = url.query_pairs().into_owned().collect();
            let signature = params.remove("signature").unwrap();
            let borrowed = params
                .iter()
                .map(|(k, v)| (k.as_str(), v.clone()))
                .collect();
            assert_eq!(signature, crate::signing::android_signature(&borrowed, b""));
            assert_eq!(params.len(), 13);
            assert_eq!(params["keyword"], "Test & 中文");
            assert_eq!(params["page"], (i + 1).to_string());
            assert_eq!(params["pagesize"], "20");
            assert_eq!(params["appid"], "1005");
            assert_eq!(params["clientver"], "20489");
            assert_eq!(params["userid"], "0");
            assert_eq!(params["token"], "");
            assert_eq!(params["dfid"], "-");
            assert_eq!(params["platform"], "AndroidFilter");
            assert_eq!(params["iscorrection"], "1");
            assert_eq!(params["uuid"].len(), 36);
            if let Some(first) = &first_mid {
                assert_eq!(first, &params["mid"]);
            } else {
                first_mid = Some(params["mid"].clone());
            }
            assert!(
                r.to_lowercase()
                    .contains("x-router: complexsearch.kugou.com")
            );
            assert!(!r.to_lowercase().contains("cookie:"));
            assert!(!r.contains("private-account-secret"));
            assert!(!r.to_lowercase().contains("authorization:"));
            assert!(r.ends_with("\r\n\r\n"));
        }
        assert_eq!(
            store.values.lock().unwrap().get("default").unwrap(),
            &old.stored("default").unwrap()
        );
    }
}

#[tokio::test]
async fn catalogue_search_obeys_six_page_budget_and_preserves_final_and_empty_page_metadata() {
    let kind = SearchKind::Album;
    let f = server(
        (1..=6)
            .map(|p| raw(response(kind, p, 125)).into())
            .collect(),
    )
    .await;
    let p = f
        .provider
        .search_catalog(&query(kind, 100, 19))
        .await
        .unwrap();
    assert_eq!(p.items.len(), 100);
    assert_eq!(item_id(&p.items[0]), "20");
    assert_eq!(item_id(p.items.last().unwrap()), "119");
    assert_eq!(p.pagination.next_offset, Some(119));
    assert_eq!(f.requests.await.unwrap().len(), 6);
    for offset in [21, 28, 40] {
        let f = server(vec![raw(response(kind, offset / 20 + 1, 28)).into()]).await;
        let p = f
            .provider
            .search_catalog(&query(kind, 10, offset))
            .await
            .unwrap();
        assert_eq!(p.items.len(), 28_usize.saturating_sub(offset as usize));
        assert!(!p.pagination.has_more);
        assert_eq!(p.pagination.next_offset, None);
        assert_eq!(p.pagination.total, Some(28));
        assert_eq!(f.requests.await.unwrap().len(), 1);
    }
}

#[tokio::test]
async fn catalogue_paging_rejects_total_changes_and_repeated_identities_without_partial_results() {
    for changed_total in [false, true] {
        let first = response(SearchKind::Album, 1, 40);
        let mut second = response(SearchKind::Album, 2, 40);
        if changed_total {
            second["data"]["total"] = json!(41);
        } else {
            second["data"]["lists"][0]["albumid"] = json!(1);
        }
        let f = server(vec![raw(first).into(), raw(second).into()]).await;
        let e = f
            .provider
            .search_catalog(&query(SearchKind::Album, 5, 18))
            .await
            .unwrap_err();
        assert_eq!(e.code, ErrorCode::UpstreamError);
        assert_eq!(f.requests.await.unwrap().len(), 2);
    }
}

#[tokio::test]
async fn catalogue_transport_rejects_redirects_challenges_mime_limits_and_business_errors_without_retries()
 {
    let good = raw(response(SearchKind::Album, 1, 1));
    for response in [
        "HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:1/private\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_owned(),
        good.replace("Content-Type:","SSA-CODE: private-challenge\r\nContent-Type:"),
        good.replace("application/json","text/html"),
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 1048577\r\nConnection: close\r\n\r\n".to_owned(),
        raw(json!({"status":0,"error_code":20006,"data":"private-error"})),
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n".to_owned()+&"x".repeat(1048577),
    ] {
        let f=server(vec![response.into()]).await;
        let e=f.provider.search_catalog(&query(SearchKind::Album,1,0)).await.unwrap_err();
        assert!(!format!("{e:?}").contains("private-"));assert_eq!(f.requests.await.unwrap().len(),1);
    }
    let f = server(vec![
        good.replace("application/json", "text/plain; charset=utf-8")
            .into(),
    ])
    .await;
    assert_eq!(
        f.provider
            .search_catalog(&query(SearchKind::Album, 1, 0))
            .await
            .unwrap()
            .items
            .len(),
        1
    );
    assert_eq!(f.requests.await.unwrap().len(), 1);
}

#[tokio::test]
#[ignore = "uses live official anonymous catalogue search"]
async fn live_public_catalogue_search_supports_three_types_and_non_aligned_pagination() {
    let provider = KugouProvider::new(KugouConfig::default()).unwrap();
    for kind in [SearchKind::Album, SearchKind::Artist, SearchKind::Playlist] {
        let mut q = query(kind, 5, 18);
        q.query = "周杰伦".into();
        let page = provider.search_catalog(&q).await.unwrap();
        assert!(!page.items.is_empty());
        assert_eq!(page.items.len(), 5);
        assert_eq!(page.pagination.offset, 18);
        assert_eq!(page.pagination.next_offset, Some(23));
        assert!(page.items.iter().all(|item| !item_id(item).is_empty()));
    }
}

#[tokio::test]
async fn catalogue_rejects_account_sources_and_unsupported_options_before_network() {
    let f = server(vec![]).await;
    for kind in [
        SearchKind::Album,
        SearchKind::Artist,
        SearchKind::Playlist,
        SearchKind::Mv,
    ] {
        let base = query(kind, 5, 0);
        for i in 0..10 {
            let mut q = base.clone();
            match i {
                0 => q.account = Some("named".into()),
                1 => q.variant = SearchVariant::Legacy,
                2 => q.highlight = true,
                3 => q.search_id = Some("cursor".into()),
                4 => q.query = " ".into(),
                5 => q.query = "x".repeat(513),
                6 => q.query = "bad\nquery".into(),
                7 => q.limit = 0,
                8 => q.limit = 101,
                9 => q.offset = u32::MAX,
                _ => unreachable!(),
            }
            assert_eq!(
                f.provider.search_catalog(&q).await.unwrap_err().code,
                ErrorCode::InvalidRequest
            );
        }
        let scoped = f
            .provider
            .caller_scope(&credential("111", "caller-secret").caller().unwrap())
            .unwrap();
        assert_eq!(
            scoped.search_catalog(&base).await.unwrap_err().code,
            ErrorCode::InvalidRequest
        );
    }
    for kind in [SearchKind::Mixed, SearchKind::Video, SearchKind::User] {
        assert_eq!(
            f.provider
                .search_catalog(&query(kind, 5, 0))
                .await
                .unwrap_err()
                .code,
            ErrorCode::CapabilityNotSupported
        );
    }
    assert!(f.requests.await.unwrap().is_empty());
}
