use super::super::session::tests::{Store, credential, raw, server};
use super::*;
use crate::client::artists::tests::{albums, detail, tracks};
use serde_json::Value;
use std::collections::BTreeMap;
use tuneweave_core::{ArtistArea, ArtistCatalogRequest, ArtistCategory, ArtistGenre};

#[tokio::test]
async fn artist_top_tracks_returns_a_fixed_projection_after_reading_the_complete_hot_catalogue() {
    let mut first = tracks(1, 105, 1);
    // Preserve official order and distinct album versions of the same audio group.
    first["data"]["songs"].as_array_mut().unwrap().swap(0, 9);
    let mut f = server(vec![
        raw(detail(105, 31)).into(),
        raw(first).into(),
        raw(tracks(2, 105, 1)).into(),
    ])
    .await;
    let store = Arc::new(Store::default());
    let original = credential("999", "account-secret")
        .stored("default")
        .unwrap();
    store.put(&original).unwrap();
    f.provider.credential_store = Some(store.clone());
    let result = f.provider.artist_top_tracks("42", None).await.unwrap();
    assert_eq!(result.items.len(), 10);
    assert_eq!(result.items[0].id, "1009");
    assert_eq!(result.items[9].id, "1000");
    assert_eq!(result.items[0].resource_ref.id(), "1009");
    assert!(
        result
            .items
            .iter()
            .all(|track| track.extensions["audio_group_id"] == "77")
    );
    assert_eq!(result.pagination.limit, 10);
    assert_eq!(result.pagination.offset, 0);
    assert_eq!(result.pagination.total, Some(10));
    assert!(!result.pagination.has_more);
    assert_eq!(result.pagination.next_offset, None);
    assert_eq!(
        result.pagination.extensions["result_scope"],
        "hot_catalogue_first_10"
    );
    assert_eq!(result.pagination.extensions["source_catalogue_total"], 105);
    assert_eq!(result.pagination.extensions["upstream_pages_fetched"], 2);
    assert_eq!(result.pagination.extensions["order"], "hot");
    assert_eq!(result.pagination.extensions["artist_id"], "42");
    let requests = f.requests.await.unwrap();
    assert_eq!(requests.len(), 3);
    for (index, wire) in requests.iter().enumerate() {
        let (url, q, body) = query(wire);
        assert!(!wire.contains("account-secret"));
        if index == 0 {
            assert_eq!(url.path(), "/kmr/v3/author");
            assert_eq!(
                serde_json::from_str::<Value>(body).unwrap(),
                json!({"author_id":42})
            );
        } else {
            assert_eq!(url.path(), "/openapi/kmr/v2/audio_group/author");
            assert_eq!(q["author_id"], "42");
            assert_eq!(q["sort"], "1");
            assert_eq!(q["pagesize"], "100");
            assert_eq!(q["page"], index.to_string());
            assert!(body.is_empty());
        }
    }
    assert_eq!(store.values.lock().unwrap().get("default"), Some(&original));
}

#[tokio::test]
async fn artist_top_tracks_empty_short_and_exact_snapshots_have_no_continuation() {
    for total in [0, 3, 10] {
        let f = server(vec![
            raw(detail(total, 0)).into(),
            raw(tracks(1, total, 1)).into(),
        ])
        .await;
        let result = f.provider.artist_top_tracks("42", None).await.unwrap();
        assert_eq!(result.items.len() as u64, total);
        assert_eq!(result.pagination.total, Some(total));
        assert_eq!(result.pagination.limit, 10);
        assert_eq!(result.pagination.offset, 0);
        assert!(!result.pagination.has_more);
        assert_eq!(result.pagination.next_offset, None);
        assert_eq!(
            result.pagination.extensions["source_catalogue_total"],
            total
        );
        assert_eq!(f.requests.await.unwrap().len(), 2);
    }
}

#[tokio::test]
async fn artist_top_tracks_rejects_invalid_later_pages_instead_of_returning_a_partial_top_ten() {
    for case in ["count", "sort", "duplicate", "foreign_artist", "business"] {
        let mut last = tracks(2, 105, 1);
        match case {
            "count" => last = tracks(2, 106, 1),
            "sort" => last["data"]["input_param"]["sort"] = json!(2),
            "duplicate" => last["data"]["songs"][0]["album_audio_id"] = json!(1000),
            "foreign_artist" => {
                last["data"]["songs"][0]["authors"] = json!([
                    {"base":{"author_id":43,"author_name":"Other artist"}}
                ])
            }
            "business" => last = json!({"status":0,"error_code":20001,"data":"private-message"}),
            _ => unreachable!(),
        }
        let f = server(vec![
            raw(detail(105, 0)).into(),
            raw(tracks(1, 105, 1)).into(),
            raw(last).into(),
        ])
        .await;
        let error = f.provider.artist_top_tracks("42", None).await.unwrap_err();
        assert_eq!(error.code, ErrorCode::UpstreamError, "{case}");
        assert!(!format!("{error:?}").contains("private-message"));
        assert_eq!(f.requests.await.unwrap().len(), 3);
    }
}

#[tokio::test]
async fn artist_top_tracks_rejects_noncanonical_ids_and_account_sources_before_network() {
    let f = server(vec![]).await;
    for id in ["0", "042", "artist:42", "-1", "", "18446744073709551616"] {
        assert_eq!(
            f.provider
                .artist_top_tracks(id, None)
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
    }
    for account in ["", "default", "Other"] {
        assert_eq!(
            f.provider
                .artist_top_tracks("42", Some(account))
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
    }
    let scoped = f
        .provider
        .caller_scope(&credential("111", "caller-secret").caller().unwrap())
        .unwrap();
    assert_eq!(
        scoped.artist_top_tracks("42", None).await.unwrap_err().code,
        ErrorCode::InvalidRequest
    );
    assert!(f.requests.await.unwrap().is_empty());
}

#[tokio::test]
async fn artist_stats_preserve_public_fans_mv_counts_and_unknown_account_state() {
    let mut f = server(vec![raw(detail(100, 20)).into()]).await;
    let store = Arc::new(Store::default());
    let original = credential("999", "account-secret")
        .stored("default")
        .unwrap();
    store.put(&original).unwrap();
    f.provider.credential_store = Some(store.clone());
    let stats = f.provider.artist_stats("42", None).await.unwrap();
    assert_eq!(stats.artist_ref.id(), "42");
    assert_eq!(stats.follower_count, Some(99));
    assert_eq!(stats.followed, None);
    assert_eq!(stats.online_concert_count, None);
    assert_eq!(stats.video_counts.len(), 1);
    assert_eq!(stats.video_counts[0].category.as_deref(), Some("mv"));
    assert_eq!(stats.video_counts[0].count, 4);
    assert_eq!(
        stats.video_counts[0].extensions["catalog_scope"],
        "official_artist_mv_catalogue"
    );
    let requests = f.requests.await.unwrap();
    assert_eq!(requests.len(), 1);
    let (url, _, body) = query(&requests[0]);
    assert!(requests[0].starts_with("POST "));
    assert_eq!(url.path(), "/kmr/v3/author");
    assert_eq!(
        serde_json::from_str::<Value>(body).unwrap(),
        json!({"author_id":42})
    );
    assert!(!requests[0].contains("account-secret"));
    assert_eq!(store.values.lock().unwrap().get("default"), Some(&original));
}

#[tokio::test]
async fn artist_stats_distinguish_missing_zero_and_canonical_string_counts() {
    for (counts, expected) in [
        (None, None),
        (Some(json!(0)), Some(0)),
        (Some(json!("123")), Some(123)),
    ] {
        let mut value = detail(0, 0);
        let data = value["data"].as_object_mut().unwrap();
        if let Some(count) = counts {
            data.insert("fansnums".into(), count.clone());
            data.insert("mv_count".into(), count);
        } else {
            data.remove("fansnums");
            data.remove("mv_count");
        }
        // This unrelated field must never turn an anonymous statistic into follow state.
        data.insert("user_status".into(), json!(1));
        let f = server(vec![raw(value).into()]).await;
        let stats = f.provider.artist_stats("42", None).await.unwrap();
        assert_eq!(stats.follower_count, expected);
        assert_eq!(stats.video_counts.first().map(|v| v.count), expected);
        assert_eq!(stats.followed, None);
        assert_eq!(stats.online_concert_count, None);
        assert_eq!(f.requests.await.unwrap().len(), 1);
    }
}

#[tokio::test]
async fn artist_stats_reject_wrong_identity_malformed_counts_and_upstream_rejection() {
    for case in 0..4 {
        let mut value = detail(1, 1);
        match case {
            0 => value["data"]["author_id"] = json!(43),
            1 => value["data"]["fansnums"] = json!(-1),
            2 => value["data"]["mv_count"] = json!("04"),
            3 => value = json!({"status":0,"error_code":20006,"data":"private-message"}),
            _ => unreachable!(),
        }
        let f = server(vec![raw(value).into()]).await;
        let error = f.provider.artist_stats("42", None).await.unwrap_err();
        assert_eq!(error.code, ErrorCode::UpstreamError);
        assert!(!format!("{error:?}").contains("private-message"));
        assert_eq!(f.requests.await.unwrap().len(), 1);
    }
}

#[tokio::test]
async fn artist_stats_reject_noncanonical_ids_and_account_sources_before_network() {
    let f = server(vec![]).await;
    for id in ["0", "042", "artist:42", "-1", ""] {
        assert_eq!(
            f.provider.artist_stats(id, None).await.unwrap_err().code,
            ErrorCode::InvalidRequest
        );
    }
    assert_eq!(
        f.provider
            .artist_stats("42", Some("default"))
            .await
            .unwrap_err()
            .code,
        ErrorCode::InvalidRequest
    );
    let scoped = f
        .provider
        .caller_scope(&credential("111", "caller-secret").caller().unwrap())
        .unwrap();
    assert_eq!(
        scoped.artist_stats("42", None).await.unwrap_err().code,
        ErrorCode::InvalidRequest
    );
    assert!(f.requests.await.unwrap().is_empty());
}

#[tokio::test]
#[ignore = "requires anonymous official KuGou public artist metadata"]
async fn live_artist_stats_reads_public_fans_and_official_mv_count() {
    let provider = KugouProvider::new(KugouConfig::default()).unwrap();
    let stats = provider.artist_stats("3520", None).await.unwrap();
    assert_eq!(stats.artist_ref.id(), "3520");
    assert!(stats.follower_count.unwrap() > 0);
    assert_eq!(stats.video_counts[0].category.as_deref(), Some("mv"));
    assert!(stats.video_counts[0].count > 0);
    assert_eq!(stats.followed, None);
    assert_eq!(stats.online_concert_count, None);
}

#[tokio::test]
async fn artist_list_paginates_only_initial_groups_and_keeps_canonical_artist_ids() {
    use crate::client::artist_catalog::tests::catalogue;
    let f = server(vec![raw(catalogue()).into(), raw(catalogue()).into()]).await;
    let first = f
        .provider
        .artists(&tuneweave_core::ArtistListRequest::new(1, 0))
        .await
        .unwrap();
    assert_eq!(first.items[0].id, "42");
    assert_eq!(first.items[0].resource_ref.id(), "42");
    assert_eq!(first.pagination.total, Some(3));
    assert!(first.pagination.has_more);
    assert_eq!(first.pagination.next_offset, Some(1));
    let second = f
        .provider
        .artists(&tuneweave_core::ArtistListRequest::new(2, 1))
        .await
        .unwrap();
    assert_eq!(
        second
            .items
            .iter()
            .map(|a| a.id.as_str())
            .collect::<Vec<_>>(),
        ["43", "44"]
    );
    assert_eq!(second.pagination.total, Some(3));
    assert!(!second.pagination.has_more);
    assert_eq!(second.pagination.next_offset, None);
    assert_eq!(
        second.pagination.extensions["catalog_scope"],
        "upstream_grouped_directory"
    );
    assert_eq!(
        second.pagination.extensions["pagination_source"],
        "local_directory_slice"
    );
    assert_eq!(
        second.pagination.extensions["total_scope"],
        "selected_directory_groups"
    );
    let requests = f.requests.await.unwrap();
    assert_eq!(requests.len(), 2);
    for request in requests {
        let (url, q, body) = query(&request);
        assert_eq!(url.path(), "/ocean/v6/singer/list");
        assert_eq!(q["showtype"], "2");
        assert_eq!(q["hotsize"], "200");
        assert!(!q.contains_key("page") && !q.contains_key("pagesize"));
        assert_eq!(body, "");
    }
}

#[tokio::test]
async fn artist_list_filters_before_pagination_and_reports_empty_or_exhausted_groups() {
    use crate::client::artist_catalog::tests::catalogue;
    for (initial, offset, expected, total) in [
        ("A", 1, vec!["43"], 2),
        ("#", 0, vec!["44"], 1),
        ("B", 0, vec![], 0),
        ("Z", 0, vec![], 0),
        ("A", 99, vec![], 2),
    ] {
        let f = server(vec![raw(catalogue()).into()]).await;
        let page = f
            .provider
            .artists(&tuneweave_core::ArtistListRequest {
                initial: Some(initial.into()),
                ..tuneweave_core::ArtistListRequest::new(5, offset)
            })
            .await
            .unwrap();
        assert_eq!(
            page.items.iter().map(|a| a.id.as_str()).collect::<Vec<_>>(),
            expected
        );
        assert_eq!(page.pagination.total, Some(total));
        assert!(!page.pagination.has_more);
        assert_eq!(page.pagination.next_offset, None);
        assert_eq!(page.pagination.extensions["initial"], initial);
        assert_eq!(f.requests.await.unwrap().len(), 1);
    }
}

#[tokio::test]
async fn artist_list_rejects_invalid_pagination_initials_and_account_sources_before_network() {
    use tuneweave_core::ArtistListRequest;
    let f = server(vec![]).await;
    let mut requests = vec![
        ArtistListRequest::new(0, 0),
        ArtistListRequest::new(101, 0),
        ArtistListRequest::new(1, u32::MAX),
    ];
    for initial in ["", "a", " A", "AB", "热", "1"] {
        requests.push(ArtistListRequest {
            initial: Some(initial.into()),
            ..ArtistListRequest::new(10, 0)
        });
    }
    requests.push(ArtistListRequest {
        account: Some("default".into()),
        ..ArtistListRequest::new(10, 0)
    });
    requests.push(ArtistListRequest {
        area: ArtistArea::HongKongTaiwan,
        ..ArtistListRequest::new(10, 0)
    });
    requests.push(ArtistListRequest {
        genre: ArtistGenre::Pop,
        ..ArtistListRequest::new(10, 0)
    });
    for request in requests {
        assert_eq!(
            f.provider.artists(&request).await.unwrap_err().code,
            ErrorCode::InvalidRequest
        );
    }
    let scoped = f
        .provider
        .caller_scope(&credential("111", "caller-secret").caller().unwrap())
        .unwrap();
    assert_eq!(
        scoped
            .artists(&ArtistListRequest::new(10, 0))
            .await
            .unwrap_err()
            .code,
        ErrorCode::InvalidRequest
    );
    assert!(f.requests.await.unwrap().is_empty());
}

#[tokio::test]
async fn artist_list_preserves_directory_failure_instead_of_returning_an_empty_page() {
    let f = server(vec![
        raw(json!({"status":0,"errcode":20006,"data":{},"error":"private-upstream"})).into(),
    ])
    .await;
    let error = f
        .provider
        .artists(&tuneweave_core::ArtistListRequest::new(10, 0))
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::UpstreamError);
    assert!(!format!("{error:?}").contains("private-upstream"));
    assert_eq!(f.requests.await.unwrap().len(), 1);
}

#[tokio::test]
#[ignore = "requires anonymous official KuGou grouped artist directory"]
async fn live_artist_list_filters_anonymous_directory_without_requesting_account_data() {
    let provider = KugouProvider::new(KugouConfig::default()).unwrap();
    let result = provider
        .artists(&tuneweave_core::ArtistListRequest {
            initial: Some("A".into()),
            ..tuneweave_core::ArtistListRequest::new(3, 1)
        })
        .await
        .unwrap();
    assert_eq!(result.items.len(), 3);
    assert!(
        result
            .items
            .iter()
            .all(|a| a.extensions["directory_group"] == "A")
    );
    assert!(result.pagination.total.unwrap() >= 4);
    assert_eq!(
        result.pagination.extensions["pagination_source"],
        "local_directory_slice"
    );
}

#[tokio::test]
async fn artist_catalog_uses_official_signed_anonymous_filters_and_preserves_stored_account() {
    use crate::client::artist_catalog::tests::catalogue;
    let mut f = server(vec![
        raw(catalogue())
            .replace("application/json", "text/html")
            .into(),
    ])
    .await;
    let store = Arc::new(Store::default());
    let original = credential("999", "account-secret")
        .stored("default")
        .unwrap();
    store.put(&original).unwrap();
    f.provider.credential_store = Some(store.clone());
    let request = ArtistCatalogRequest {
        area: ArtistArea::Korean,
        category: ArtistCategory::Female,
        ..Default::default()
    };
    let result = f.provider.artist_catalog(&request).await.unwrap();
    assert_eq!(result.area, ArtistArea::Korean);
    assert_eq!(result.category, ArtistCategory::Female);
    assert_eq!(result.featured_artists[0].id, "42");
    let requests = f.requests.await.unwrap();
    assert_eq!(requests.len(), 1);
    let (url, q, body) = query(&requests[0]);
    assert!(requests[0].starts_with("GET "));
    assert_eq!(url.path(), "/ocean/v6/singer/list");
    assert_eq!(body, "");
    for (key, expected) in [
        ("type", "6"),
        ("sextype", "2"),
        ("musician", "0"),
        ("showtype", "2"),
        ("hotsize", "200"),
        ("with_discuss", "0"),
        ("is_thumb", "1"),
    ] {
        assert_eq!(q[key], expected);
    }
    assert!(!q.contains_key("page") && !q.contains_key("pagesize"));
    assert!(!requests[0].contains("account-secret"));
    assert_eq!(store.values.lock().unwrap().get("default"), Some(&original));
}

#[tokio::test]
async fn artist_catalog_rejects_account_and_unmapped_filters_before_network() {
    let f = server(vec![]).await;
    for request in [
        ArtistCatalogRequest {
            account: Some("default".into()),
            ..Default::default()
        },
        ArtistCatalogRequest {
            area: ArtistArea::HongKongTaiwan,
            ..Default::default()
        },
        ArtistCatalogRequest {
            genre: ArtistGenre::Pop,
            ..Default::default()
        },
    ] {
        assert_eq!(
            f.provider.artist_catalog(&request).await.unwrap_err().code,
            ErrorCode::InvalidRequest
        );
    }
    let caller = credential("111", "caller-secret").caller().unwrap();
    let scoped = f.provider.caller_scope(&caller).unwrap();
    assert_eq!(
        scoped
            .artist_catalog(&Default::default())
            .await
            .unwrap_err()
            .code,
        ErrorCode::InvalidRequest
    );
    assert!(f.requests.await.unwrap().is_empty());
}

#[tokio::test]
#[ignore = "requires anonymous official KuGou directory and artist metadata"]
async fn live_artist_catalog_identity_resolves_to_existing_artist_detail() {
    let provider = KugouProvider::new(KugouConfig::default()).unwrap();
    let directory = provider
        .artist_catalog(&ArtistCatalogRequest::default())
        .await
        .unwrap();
    let first = directory.featured_artists.first().unwrap();
    let detail = provider.artist(&first.id, None).await.unwrap();
    assert_eq!(detail.id, first.id);
    assert_eq!(detail.name, first.name);
    assert!(!directory.artists.is_empty());
}

fn request(offset: u32, limit: u32, order: ArtistTrackOrder) -> ArtistTrackListRequest {
    ArtistTrackListRequest {
        offset,
        limit,
        order,
        account: None,
    }
}
fn query(r: &str) -> (url::Url, BTreeMap<String, String>, &str) {
    let (head, body) = r.split_once("\r\n\r\n").unwrap();
    let target = head
        .lines()
        .next()
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap();
    let url = url::Url::parse(&format!("http://localhost{target}")).unwrap();
    let mut q: BTreeMap<_, _> = url.query_pairs().into_owned().collect();
    let signature = q.remove("signature").unwrap();
    let borrowed = q.iter().map(|(k, v)| (k.as_str(), v.clone())).collect();
    assert_eq!(
        signature,
        crate::signing::android_signature(&borrowed, body.as_bytes())
    );
    assert_eq!(q["userid"], "0");
    assert_eq!(q["token"], "");
    assert_eq!(q["appid"], "1005");
    assert!(!head.to_lowercase().contains("cookie:"));
    assert!(!head.to_lowercase().contains("authorization:"));
    assert!(head.to_lowercase().contains("kg-tid: 36"));
    (url, q, body)
}

#[tokio::test]
async fn artists_preserve_complete_track_versions_and_signed_sort_order_without_using_accounts() {
    for (order, sort) in [
        (ArtistTrackOrder::Hot, 1),
        (ArtistTrackOrder::Time, 2),
        (ArtistTrackOrder::PlatformDefault, 2),
    ] {
        let mut f = server(vec![
            raw(detail(105, 31)).into(),
            raw(tracks(1, 105, sort)).into(),
            raw(tracks(2, 105, sort)).into(),
        ])
        .await;
        let store = Arc::new(Store::default());
        let old = credential("999", "account-secret");
        store.put(&old.stored("default").unwrap()).unwrap();
        f.provider.credential_store = Some(store.clone());
        let p = f
            .provider
            .artist_tracks("42", &request(98, 5, order))
            .await
            .unwrap();
        assert_eq!(
            p.items.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(),
            ["1098", "1099", "1100", "1101", "1102"]
        );
        assert_eq!(p.pagination.total, Some(105));
        assert_eq!(p.pagination.next_offset, Some(103));
        assert_eq!(p.items[0].extensions["artist_position"], 98);
        assert_eq!(p.pagination.extensions["upstream_pages_fetched"], 2);
        let requests = f.requests.await.unwrap();
        assert_eq!(requests.len(), 3);
        let mut identity = None;
        for (i, r) in requests.iter().enumerate() {
            let (url, q, body) = query(r);
            let current = (q["mid"].clone(), q["uuid"].clone());
            if let Some(old) = &identity {
                assert_eq!(old, &current);
            } else {
                identity = Some(current);
            }
            assert!(!r.contains("account-secret"));
            if i == 0 {
                assert!(r.starts_with("POST "));
                assert_eq!(url.path(), "/kmr/v3/author");
                assert_eq!(
                    serde_json::from_str::<Value>(body).unwrap(),
                    json!({"author_id":42})
                );
            } else {
                assert!(r.starts_with("GET "));
                assert_eq!(url.path(), "/openapi/kmr/v2/audio_group/author");
                assert_eq!(body, "");
                assert_eq!(q["sort"], sort.to_string());
                assert_eq!(q["page"], i.to_string());
                assert_eq!(q["pagesize"], "100");
                assert_eq!(q["author_id"], "42");
                assert_eq!(q["replace_api_version"], "1");
            }
        }
        assert_eq!(
            store.values.lock().unwrap().get("default").unwrap(),
            &old.stored("default").unwrap()
        );
    }
}

#[tokio::test]
async fn artists_fetch_every_album_page_and_use_the_original_album_reference() {
    let f = server(vec![
        raw(detail(105, 31)).into(),
        raw(albums(1, 31)).into(),
        raw(albums(2, 31)).into(),
    ])
    .await;
    let p = f
        .provider
        .artist_albums(
            "42",
            &PageRequest {
                offset: 29,
                limit: 5,
                account: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(
        p.items.iter().map(|a| a.id.as_str()).collect::<Vec<_>>(),
        ["929", "930"]
    );
    assert_eq!(p.pagination.total, Some(31));
    assert!(!p.pagination.has_more);
    assert_eq!(p.pagination.extensions["order"], "time");
    let requests = f.requests.await.unwrap();
    assert_eq!(requests.len(), 3);
    for (i, r) in requests.iter().enumerate() {
        let (url, _, body) = query(r);
        if i > 0 {
            assert!(r.starts_with("POST "));
            assert_eq!(url.path(), "/kmr/v1/author/albums");
            assert_eq!(
                serde_json::from_str::<Value>(body).unwrap(),
                json!({"author_id":42,"pagesize":30,"page":i,"sort":1,"category":1,"area_code":"all"})
            );
        }
    }
}

#[tokio::test]
async fn artists_discard_partial_catalogues_when_later_pages_or_profile_counts_conflict() {
    for case in 0..7 {
        let mut second = tracks(2, 200, 1);
        match case {
            0 => {
                second["data"]["total"] = json!(201);
                second["extra"]["page_total"] = json!(201);
            }
            1 => second = tracks(1, 200, 1),
            2 => second["data"]["songs"][0]["authors"][1]["base"]["author_id"] = json!(43),
            3 => second["data"]["songs"] = json!([]),
            4 => second["data"]["input_param"]["sort"] = json!(2),
            5 => second = json!({"status":0,"error_code":20006,"errmsg":"private-error"}),
            6 => second["data"]["songs"][0]["album_audio_id"] = json!(1000),
            _ => unreachable!(),
        }
        let f = server(vec![
            raw(detail(200, 31)).into(),
            raw(tracks(1, 200, 1)).into(),
            raw(second).into(),
        ])
        .await;
        let e = f
            .provider
            .artist_tracks("42", &request(0, 5, ArtistTrackOrder::Hot))
            .await
            .unwrap_err();
        assert_eq!(e.code, ErrorCode::UpstreamError);
        assert!(!format!("{e:?}").contains("private-error"));
        assert_eq!(f.requests.await.unwrap().len(), 3);
    }
    for case in 0..3 {
        let mut second = albums(2, 60);
        match case {
            0 => second = albums(1, 60),
            1 => {
                second["total"] = json!(61);
                second["extra"]["page_total"] = json!(61);
            }
            2 => second["data"][0]["authors"][0]["author_id"] = json!(43),
            _ => unreachable!(),
        }
        let f = server(vec![
            raw(detail(1, 60)).into(),
            raw(albums(1, 60)).into(),
            raw(second).into(),
        ])
        .await;
        assert!(
            f.provider
                .artist_albums(
                    "42",
                    &PageRequest {
                        offset: 0,
                        limit: 5,
                        account: None
                    }
                )
                .await
                .is_err()
        );
        assert_eq!(f.requests.await.unwrap().len(), 3);
    }
    let f = server(vec![
        raw(detail(106, 31)).into(),
        raw(tracks(1, 105, 1)).into(),
    ])
    .await;
    assert!(
        f.provider
            .artist_tracks("42", &request(0, 5, ArtistTrackOrder::Hot))
            .await
            .is_err()
    );
    assert_eq!(f.requests.await.unwrap().len(), 2);
}

#[tokio::test]
async fn artists_handle_empty_out_of_range_metadata_and_complete_overview() {
    for (total, offset) in [(0, 0), (2, 50)] {
        let f = server(vec![
            raw(detail(total, 0)).into(),
            raw(tracks(1, total, 1)).into(),
        ])
        .await;
        let p = f
            .provider
            .artist_tracks("42", &request(offset, 10, ArtistTrackOrder::Hot))
            .await
            .unwrap();
        assert!(p.items.is_empty());
        assert_eq!(p.pagination.total, Some(total));
        assert!(!p.pagination.has_more);
        f.requests.await.unwrap();
    }
    let f = server(vec![
        raw(detail(105, 31)).into(),
        raw(tracks(1, 105, 1)).into(),
        raw(tracks(2, 105, 1)).into(),
    ])
    .await;
    let o = f.provider.artist_overview("42", None).await.unwrap();
    assert_eq!(o.featured_tracks.len(), 10);
    assert!(o.has_more_tracks);
    assert_eq!(o.artist.id, "42");
    assert_eq!(f.requests.await.unwrap().len(), 3);
    let f = server(vec![raw(detail(0, 0)).into(), raw(albums(1, 0)).into()]).await;
    let p = f
        .provider
        .artist_albums(
            "42",
            &PageRequest {
                offset: 0,
                limit: 5,
                account: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(p.pagination.total, Some(0));
    assert!(p.items.is_empty());
    f.requests.await.unwrap();
    let f = server(vec![raw(detail(1, 1)).into()]).await;
    assert_eq!(f.provider.artist("42", None).await.unwrap().id, "42");
    assert_eq!(f.requests.await.unwrap().len(), 1);
}

#[tokio::test]
async fn artists_reject_invalid_ids_pagination_accounts_and_caller_sources_before_network() {
    let f = server(vec![]).await;
    for id in [
        "",
        "0",
        "042",
        "+42",
        " 42",
        "-1",
        "18446744073709551616",
        "https://untrusted.invalid",
    ] {
        assert_eq!(
            f.provider.artist(id, None).await.unwrap_err().code,
            ErrorCode::InvalidRequest
        );
        assert!(
            f.provider
                .artist_tracks(id, &request(0, 5, ArtistTrackOrder::Hot))
                .await
                .is_err()
        );
        assert!(
            f.provider
                .artist_albums(
                    id,
                    &PageRequest {
                        offset: 0,
                        limit: 5,
                        account: None
                    }
                )
                .await
                .is_err()
        );
    }
    for (offset, limit) in [(0, 0), (0, 101), (u32::MAX, 1)] {
        assert!(
            f.provider
                .artist_tracks("42", &request(offset, limit, ArtistTrackOrder::Hot))
                .await
                .is_err()
        );
        assert!(
            f.provider
                .artist_albums(
                    "42",
                    &PageRequest {
                        offset,
                        limit,
                        account: None
                    }
                )
                .await
                .is_err()
        );
    }
    assert!(f.provider.artist("42", Some("other")).await.is_err());
    assert!(
        f.provider
            .artist_overview("42", Some("other"))
            .await
            .is_err()
    );
    let mut r = request(0, 5, ArtistTrackOrder::Hot);
    r.account = Some("other".into());
    assert!(f.provider.artist_tracks("42", &r).await.is_err());
    assert!(
        f.provider
            .artist_albums(
                "42",
                &PageRequest {
                    offset: 0,
                    limit: 5,
                    account: Some("other".into())
                }
            )
            .await
            .is_err()
    );
    let scoped = f
        .provider
        .caller_scope(&credential("999", "caller-secret").caller().unwrap())
        .unwrap();
    assert!(scoped.artist("42", None).await.is_err());
    assert!(scoped.artist_overview("42", None).await.is_err());
    assert!(
        scoped
            .artist_tracks("42", &request(0, 5, ArtistTrackOrder::Hot))
            .await
            .is_err()
    );
    assert!(
        scoped
            .artist_albums(
                "42",
                &PageRequest {
                    offset: 0,
                    limit: 5,
                    account: None
                }
            )
            .await
            .is_err()
    );
    assert!(f.requests.await.unwrap().is_empty());
}

#[tokio::test]
async fn artists_do_not_follow_redirects_retry_or_return_partial_tracks_on_transport_failure() {
    let good = raw(tracks(1, 1, 1));
    for response in [
        "HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:1/private\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_owned(),
        good.replace("Content-Type:","SSA-CODE: private-challenge\r\nContent-Type:"),
        good.replace("application/json","text/html"),
        "HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_owned(),
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 1048577\r\nConnection: close\r\n\r\n".to_owned(),
    ] {
        let f=server(vec![raw(detail(1,1)).into(),response.into()]).await;
        let e=f.provider.artist_tracks("42",&request(0,5,ArtistTrackOrder::Hot)).await.unwrap_err();
        assert!(!format!("{e:?}").contains("private-"));assert_eq!(f.requests.await.unwrap().len(),2);
    }
}

#[tokio::test]
#[ignore = "uses official anonymous artist metadata, full track and album catalogues"]
async fn live_public_artist_catalogues_preserve_versions_collaborators_and_detail_identity() {
    let p = KugouProvider::new(KugouConfig::default()).unwrap();
    let a = p.artist("3520", None).await.unwrap();
    assert_eq!(a.id, "3520");
    assert!(!a.biography_sections.is_empty());
    let tracks = p
        .artist_tracks("3520", &request(0, 100, ArtistTrackOrder::Hot))
        .await
        .unwrap();
    assert_eq!(tracks.pagination.total, a.track_count);
    assert!(tracks.pagination.total.unwrap() > 100);
    assert!(tracks.items.iter().any(|t| t.artists.len() > 1));
    for t in &tracks.items {
        assert!(
            t.artists
                .iter()
                .any(|a| a.resource_ref.as_ref().is_some_and(|r| r.id() == "3520"))
        );
        assert_eq!(t.playable, None);
    }
    let first = &tracks.items[0];
    let detail = p.track(&first.id, None).await.unwrap();
    assert_eq!(first.resource_ref, detail.resource_ref);
    assert_eq!(first.name, detail.name);
    let albums = p
        .artist_albums(
            "3520",
            &PageRequest {
                offset: 0,
                limit: 100,
                account: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(albums.pagination.total, a.album_count);
    assert_eq!(albums.items.len() as u64, albums.pagination.total.unwrap());
    let first = &albums.items[0];
    let detail = p.album(&first.id, None).await.unwrap();
    assert_eq!(first.resource_ref, detail.resource_ref);
    assert_eq!(first.name, detail.name);
}
