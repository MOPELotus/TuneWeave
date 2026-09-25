use super::*;
use crate::client::{
    catalog::tests::{home, home_with, json_response, requests, response, setup, setup_with_gates},
    native::tests as native_fixture,
};
use std::time::Duration;
use tokio::sync::Notify;
use tuneweave_core::{
    AccountCredentialStore, ChartCatalogView, ErrorCode, StoredAccountCredential,
};

fn menu() -> serde_json::Value {
    json!({"code":200,"data":[{"name":"Official","list":[
        {"sourceid":"16","id":"489927","source":"2","name":"Hot &amp; New","intro":"Rules\nDaily","pic":"https://zimg.kuwo.cn/bang/7/4/1789599602.png","pub":"09月16日更新"}]},
        {"name":"Global","list":[{"sourceid":"93","id":"489929","source":"2","name":"Rising","pic":"https://img4.kuwo.cn/star/upload/10/10/1649672653402_.jpg"}]}]})
}
fn page(pn: u32, total: u32) -> serde_json::Value {
    let start = (pn - 1) * 20;
    let rows: Vec<_> = (start..total.min(start + 20)).map(|n| json!({
        "musicrid":format!("MUSIC_{}",n+1),"rid":n+1,"name":format!("Track {}",n+1),
        "artist":"Primary&amp;Name&Guest","artistid":77,"album":"Album","albumid":9,
        "duration":209,"online":1,"hasLossless":true,"content_type":"0","ad_type":"",
        "trend":"u0","rank_change":"3","pic":"https://img2.kuwo.cn/star/albumcover/500/a.jpg"
    })).collect();
    let mut value = json!({"code":200,"data":{"num":total.to_string(),"musicList":rows}});
    if start < total {
        value["data"]["pub"] = json!("2026-09-16");
    }
    value
}
fn req(limit: u32, offset: u32) -> ChartTrackListRequest {
    ChartTrackListRequest::new(limit, offset)
}
fn cat() -> ChartCatalogRequest {
    ChartCatalogRequest::new(ChartCatalogView::Summary)
}

#[tokio::test]
async fn chart_catalogue_preserves_groups_source_identity_views_and_unknown_metadata() {
    for view in [
        ChartCatalogView::Overview,
        ChartCatalogView::Summary,
        ChartCatalogView::Modern,
    ] {
        let mut f = setup(vec![home(), json_response(&menu())]).await;
        let result = f
            .provider
            .chart_catalog(&ChartCatalogRequest::new(view))
            .await
            .unwrap();
        assert_eq!(result.view, view);
        assert_eq!(
            result
                .groups
                .iter()
                .map(|g| g.name.as_str())
                .collect::<Vec<_>>(),
            ["Official", "Global"]
        );
        let chart = &result.groups[0].charts[0];
        assert_eq!(
            chart.resource_ref.as_ref().unwrap().to_string(),
            "kuwo:chart:16"
        );
        assert_eq!(chart.id.as_deref(), Some("16"));
        assert_eq!(chart.name, "Hot & New");
        assert_eq!(chart.extensions["display_id"], "489927");
        assert_eq!(chart.extensions["publication_label"], "09月16日更新");
        assert!(
            chart.updated_at_ms.is_none()
                && chart.update_frequency.is_none()
                && chart.track_count.is_none()
        );
        assert!(
            chart.playable.is_none() && chart.subscribed.is_none() && chart.previews.is_empty()
        );
        assert!(
            chart
                .cover_url
                .as_ref()
                .unwrap()
                .starts_with("https://zimg.kuwo.cn/bang/")
        );
        assert!(result.groups[1].charts[0].cover_url.is_some());
        let seen = requests(&mut f, 2).await;
        let url = url::Url::parse(&format!(
            "https://www.kuwo.cn{}",
            seen[1].split_whitespace().nth(1).unwrap()
        ))
        .unwrap();
        assert_eq!(url.path(), "/api/www/bang/bang/bangMenu");
        assert_eq!(url.query_pairs().count(), 4);
        assert!(!seen[0].to_lowercase().contains("cookie:"));
    }
}

#[tokio::test]
async fn chart_catalogue_rejects_ambiguous_identity_malformed_groups_and_untrusted_images() {
    let mut cases = vec![
        json!({}),
        json!({"code":200,"data":{}}),
        json!({"code":200,"data":null}),
    ];
    for (key, value) in [
        ("sourceid", json!("016")),
        ("source", json!(3)),
        ("id", json!(true)),
        ("name", json!(" ")),
        ("intro", json!("bad\u{0000}")),
    ] {
        let mut b = menu();
        b["data"][0]["list"][0][key] = value;
        cases.push(b);
    }
    let mut duplicate = menu();
    duplicate["data"][1]["list"][0]["sourceid"] = json!(16);
    cases.push(duplicate);
    let mut bad = menu();
    bad["data"][0]["name"] = json!(" ");
    cases.push(bad);
    for b in cases {
        let mut f = setup(vec![home(), json_response(&b)]).await;
        assert_eq!(
            f.provider.chart_catalog(&cat()).await.unwrap_err().code,
            ErrorCode::UpstreamError
        );
        requests(&mut f, 2).await;
    }
    for image in [
        "http://zimg.kuwo.cn/bang/a.png",
        "https://zimg.kuwo.cn.evil.test/bang/a.png",
        "https://u@zimg.kuwo.cn/bang/a.png",
        "https://zimg.kuwo.cn/bang/a.png?secret=x",
        "https://zimg.kuwo.cn/bang/%2e%2e/a.png",
        "https://zimg.kuwo.cn/bang/../a.png",
        "https://img4.kuwo.cn/other/a.jpg",
    ] {
        let mut b = menu();
        b["data"][0]["list"][0]["pic"] = json!(image);
        let mut f = setup(vec![home(), json_response(&b)]).await;
        assert!(
            f.provider.chart_catalog(&cat()).await.unwrap().groups[0].charts[0]
                .cover_url
                .is_none()
        );
        requests(&mut f, 2).await;
    }
}

#[tokio::test]
async fn chart_windows_seed_total_reuse_first_page_and_preserve_physical_ranks() {
    for (offset, pages) in [
        (19, vec![1, 2, 3, 4, 5, 6]),
        (59, vec![1, 3, 4, 5, 6, 7, 8]),
    ] {
        let mut replies = vec![home()];
        replies.extend(pages.iter().map(|p| json_response(&page(*p, 162))));
        let mut f = setup(replies).await;
        let result = f
            .provider
            .chart_tracks("chart:16", &req(100, offset))
            .await
            .unwrap();
        assert_eq!(
            result
                .items
                .iter()
                .map(|t| t.id.clone())
                .collect::<Vec<_>>(),
            (offset + 1..offset + 101)
                .map(|n| n.to_string())
                .collect::<Vec<_>>()
        );
        assert_eq!(result.pagination.total, Some(162));
        assert_eq!(result.pagination.next_offset, Some(offset + 100));
        assert_eq!(
            result.pagination.extensions["upstream_pages_fetched"],
            pages.len()
        );
        assert_eq!(
            result.pagination.extensions["publication_date"],
            "2026-09-16"
        );
        assert!(
            !result
                .pagination
                .extensions
                .contains_key("complete_snapshot")
        );
        for (i, t) in result.items.iter().enumerate() {
            assert_eq!(t.extensions["chart_rank"], offset + i as u32 + 1);
        }
        let seen = requests(&mut f, pages.len() + 1).await;
        let mut ids = BTreeSet::new();
        for (wire, pn) in seen[1..].iter().zip(pages) {
            let url = url::Url::parse(&format!(
                "https://www.kuwo.cn{}",
                wire.split_whitespace().nth(1).unwrap()
            ))
            .unwrap();
            assert_eq!(url.path(), "/api/www/bang/bang/musicList");
            let q = url
                .query_pairs()
                .collect::<std::collections::BTreeMap<_, _>>();
            assert_eq!(q.len(), 7);
            assert_eq!(q["bangId"], "16");
            assert_eq!(q["pn"], pn.to_string());
            assert_eq!(q["rn"], "20");
            assert_eq!(q["httpsStatus"], "1");
            assert_eq!(q["plat"], "web_www");
            assert_eq!(q["from"], "");
            assert!(ids.insert(q["reqId"].to_string()));
            assert!(wire.contains("anonymousCatalogueCookie123456"));
            assert!(wire.to_lowercase().contains("secret:"));
            assert!(
                wire.to_lowercase()
                    .contains("referer: https://www.kuwo.cn/ranklist")
            );
        }
    }
}

#[tokio::test]
async fn chart_end_empty_beyond_and_unknown_do_not_invent_a_new_total_or_date() {
    for (total, offset, wanted, pages) in [
        (23, 19, 4, vec![1, 2]),
        (300, 299, 1, vec![1, 15]),
        (300, 300, 0, vec![1]),
        (0, 0, 0, vec![1]),
        (300, u32::MAX - 20, 0, vec![1]),
    ] {
        let mut replies = vec![home()];
        replies.extend(pages.iter().map(|p| json_response(&page(*p, total))));
        let mut f = setup(replies).await;
        let result = f
            .provider
            .chart_tracks("16", &req(20, offset))
            .await
            .unwrap();
        assert_eq!(result.items.len(), wanted);
        assert_eq!(result.pagination.total, Some(u64::from(total)));
        assert!(!result.pagination.has_more);
        assert!(result.pagination.next_offset.is_none());
        if total == 0 {
            assert!(result.pagination.extensions["publication_date"].is_null());
        }
        requests(&mut f, pages.len() + 1).await;
    }
    let mut f = setup(vec![
        home(),
        json_response(&json!({"code":-1,"data":null,"msg":"private-error"})),
    ])
    .await;
    let e = f
        .provider
        .chart_tracks("999999999", &req(20, 0))
        .await
        .unwrap_err();
    assert_eq!(e.code, ErrorCode::ResourceNotFound);
    assert!(!format!("{e:?}").contains("private-error"));
    requests(&mut f, 2).await;
}

#[tokio::test]
async fn chart_tags_keep_raw_codes_without_fabricating_newness_previous_rank_or_permissions() {
    for tags in [true, false] {
        let mut b = page(1, 3);
        let rows = b["data"]["musicList"].as_array_mut().unwrap();
        rows[0]["opaque"] = json!({"sid":"not-public"});
        rows[1]["online"] = json!(0);
        rows[1]["isNew"] = json!(1);
        for k in ["online", "trend", "rank_change"] {
            rows[2].as_object_mut().unwrap().remove(k);
        }
        let mut f = setup(vec![home(), json_response(&b)]).await;
        let mut r = req(3, 0);
        r.include_tags = tags;
        let result = f.provider.chart_tracks("16", &r).await.unwrap();
        let t = &result.items[0];
        assert_eq!(t.resource_ref.to_string(), "kuwo:1");
        assert_eq!(t.duration_ms, Some(209000));
        assert!(t.playable.is_none());
        assert_eq!(t.artists[0].name, "Primary&Name");
        assert!(t.artists[1].resource_ref.is_none());
        assert_eq!(result.items[1].playable, Some(false));
        assert!(result.items[2].playable.is_none());
        assert_eq!(t.extensions.contains_key("chart_upstream_trend"), tags);
        assert_eq!(
            t.extensions.contains_key("chart_upstream_rank_change"),
            tags
        );
        assert!(!t.extensions.contains_key("chart_upstream_is_new"));
        assert_eq!(
            result.items[1]
                .extensions
                .contains_key("chart_upstream_is_new"),
            tags
        );
        let encoded = serde_json::to_string(&result).unwrap();
        for no in ["previous_rank", "not-public", "rank_cid"] {
            assert!(!encoded.contains(no));
        }
        requests(&mut f, 2).await;
    }
}

#[tokio::test]
async fn chart_malformed_metadata_short_pages_duplicates_and_publication_drift_fail_whole_window() {
    let mut cases = vec![];
    for (key, value) in [
        ("num", json!("0300")),
        ("num", json!(4294967296u64)),
        ("pub", json!("2026-02-29")),
        ("pub", json!(null)),
        ("musicList", json!([])),
    ] {
        let mut b = page(1, 23);
        b["data"][key] = value;
        cases.push(b);
    }
    for (key, value) in [
        ("rid", json!("01")),
        ("musicrid", json!("MUSIC_2")),
        ("name", json!("\u{0000}")),
        ("duration", json!(-1)),
        ("content_type", json!(1)),
        ("rank_change", json!(true)),
        ("isNew", json!("oops")),
        ("artistid", json!("077")),
    ] {
        let mut b = page(1, 23);
        b["data"]["musicList"][0][key] = value;
        cases.push(b);
    }
    let mut b = page(1, 23);
    b["data"]["musicList"][1] = b["data"]["musicList"][0].clone();
    cases.push(b);
    for b in cases {
        let mut f = setup(vec![home(), json_response(&b)]).await;
        assert!(f.provider.chart_tracks("16", &req(20, 19)).await.is_err());
        requests(&mut f, 2).await;
    }
    for change in 0..4 {
        let mut later = page(2, 23);
        match change {
            0 => later["data"]["pub"] = json!("2026-09-17"),
            1 => later["data"]["num"] = json!(24),
            2 => later["data"]["musicList"][0] = page(1, 23)["data"]["musicList"][0].clone(),
            _ => {
                later["data"]["musicList"].as_array_mut().unwrap().pop();
            }
        }
        let mut f = setup(vec![
            home(),
            json_response(&page(1, 23)),
            json_response(&later),
        ])
        .await;
        assert!(f.provider.chart_tracks("16", &req(20, 19)).await.is_err());
        requests(&mut f, 3).await;
    }
}

#[tokio::test]
async fn chart_transport_failures_do_not_retry_and_signature_rejection_refreshes_only_once() {
    for catalogue in [false, true] {
        let successful = if catalogue { menu() } else { page(1, 1) };
        for rejection in [
            response(403, "text/plain", "", b"denied"),
            json_response(&json!({"success":false,"message":"The request is illegal!"})),
        ] {
            let mut f = setup(vec![
                home(),
                rejection.clone(),
                home_with("newAnonymousChartsCookie1234"),
                json_response(&successful),
            ])
            .await;
            if catalogue {
                f.provider.chart_catalog(&cat()).await.unwrap();
            } else {
                f.provider.chart_tracks("16", &req(1, 0)).await.unwrap();
            }
            let seen = requests(&mut f, 4).await;
            assert!(seen[3].contains("newAnonymousChartsCookie1234"));
            assert!(!seen[3].contains("anonymousCatalogueCookie123456"));
            let mut f = setup(vec![home(), rejection.clone(), home(), rejection]).await;
            let error = if catalogue {
                f.provider.chart_catalog(&cat()).await.err()
            } else {
                f.provider.chart_tracks("16", &req(1, 0)).await.err()
            };
            assert!(error.is_some());
            requests(&mut f, 4).await;
        }
        for bad in [
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
        ] {
            let mut f = setup(vec![home(), bad]).await;
            let error = if catalogue {
                f.provider.chart_catalog(&cat()).await.err()
            } else {
                f.provider.chart_tracks("16", &req(1, 0)).await.err()
            };
            assert!(error.is_some());
            requests(&mut f, 2).await;
        }
    }
}

struct NoAccountAccess;
impl AccountCredentialStore for NoAccountAccess {
    fn load_platform(&self, _: Platform) -> Result<Vec<StoredAccountCredential>> {
        panic!("public charts read accounts")
    }
    fn put(&self, _: &StoredAccountCredential) -> Result<()> {
        panic!("public charts write accounts")
    }
    fn remove(&self, _: Platform, _: &str) -> Result<bool> {
        panic!("public charts remove accounts")
    }
}
#[tokio::test]
async fn chart_public_scope_and_invalid_inputs_fail_before_network_or_account_access() {
    let mut f = setup(vec![
        home(),
        json_response(&menu()),
        json_response(&page(1, 1)),
    ])
    .await;
    f.provider.credential_store = Some(Arc::new(NoAccountAccess));
    for id in [
        "0",
        "01",
        "+1",
        " 1",
        "chart:0",
        "chart:chart:1",
        "playlist:16",
        "18446744073709551616",
    ] {
        assert_eq!(
            f.provider
                .chart_tracks(id, &req(1, 0))
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
    }
    for (limit, offset) in [(0, 0), (101, 0), (1, u32::MAX)] {
        assert!(
            f.provider
                .chart_tracks("16", &req(limit, offset))
                .await
                .is_err()
        );
    }
    let mut c = cat();
    c.account = Some("default".into());
    assert!(f.provider.chart_catalog(&c).await.is_err());
    let mut r = req(1, 0);
    r.account = c.account;
    assert!(f.provider.chart_tracks("16", &r).await.is_err());
    let credential = native_fixture::credential_fixture("42", "private-charts-session")
        .caller()
        .unwrap();
    let caller = f.provider.with_caller_credential(&credential).unwrap();
    assert!(caller.chart_catalog(&cat()).await.is_err());
    assert!(caller.chart_tracks("16", &req(1, 0)).await.is_err());
    assert!(caller.take_response_credential().unwrap().is_none());
    f.provider.chart_catalog(&cat()).await.unwrap();
    f.provider.chart_tracks("16", &req(1, 0)).await.unwrap();
    let seen = requests(&mut f, 3).await;
    assert!(seen.iter().all(|s| !s.contains("private-charts-session")));
}

#[tokio::test]
async fn chart_total_deadline_and_cancellation_cover_bootstrap_menu_and_each_track_page() {
    for catalogue in [false, true] {
        for boundary in 0..if catalogue { 2 } else { 3 } {
            for cancel in [false, true] {
                let gate = Arc::new(Notify::new());
                let mut replies = if catalogue {
                    vec![home(), json_response(&menu())]
                } else {
                    vec![
                        home(),
                        json_response(&page(1, 23)),
                        json_response(&page(2, 23)),
                    ]
                };
                replies.truncate(boundary + 1);
                let mut f = setup_with_gates(
                    replies
                        .into_iter()
                        .enumerate()
                        .map(|(i, b)| (b, (i == boundary).then(|| gate.clone())))
                        .collect(),
                )
                .await;
                native_fixture::set_request_timeout(
                    &mut f.provider.client,
                    Duration::from_secs(60),
                );
                let provider = f.provider.clone();
                let task = tokio::spawn(async move {
                    if catalogue {
                        provider.chart_catalog(&cat()).await.map(|_| ())
                    } else {
                        provider.chart_tracks("16", &req(20, 19)).await.map(|_| ())
                    }
                });
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
                    let e = result.unwrap().unwrap_err();
                    assert_eq!(e.code, ErrorCode::UpstreamTimeout);
                    assert!(e.message.contains("total time budget"));
                }
                gate.notify_one();
                (&mut f.server).await.unwrap();
                assert!(f.seen.try_recv().is_err());
                assert!(f.provider.take_response_credential().unwrap().is_none());
            }
        }
    }
}

#[tokio::test]
#[ignore = "Official anonymous chart metadata only; no accounts or media"]
async fn live_charts_catalogue_cross_page_end_and_track_identity() {
    let provider = KuwoProvider::new(KuwoConfig::default()).unwrap();
    let catalogue = provider.chart_catalog(&cat()).await.unwrap();
    assert!(!catalogue.groups.is_empty());
    let chart = catalogue
        .groups
        .iter()
        .flat_map(|g| &g.charts)
        .find(|c| c.id.as_deref() == Some("16"))
        .unwrap();
    let result = provider
        .chart_tracks(chart.resource_ref.as_ref().unwrap().id(), &req(4, 18))
        .await
        .unwrap();
    assert_eq!(result.items.len(), 4);
    assert_eq!(result.items[0].extensions["chart_rank"], 19);
    assert_eq!(result.items[3].extensions["chart_rank"], 22);
    let detail = provider.track(&result.items[0].id, None).await.unwrap();
    assert_eq!(detail.resource_ref, result.items[0].resource_ref);
    let total = result.pagination.total.unwrap() as u32;
    let last = provider
        .chart_tracks("16", &req(20, total - 1))
        .await
        .unwrap();
    assert_eq!(last.items.len(), 1);
    assert!(!last.pagination.has_more);
    let beyond = provider.chart_tracks("16", &req(20, total)).await.unwrap();
    assert!(beyond.items.is_empty());
    assert_eq!(beyond.pagination.total, Some(u64::from(total)));
    assert_eq!(beyond.pagination.extensions["upstream_pages_fetched"], 1);
    assert_eq!(
        provider
            .chart_tracks("999999999", &req(20, 0))
            .await
            .unwrap_err()
            .code,
        ErrorCode::ResourceNotFound
    );
}
