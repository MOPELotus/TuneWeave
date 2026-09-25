use super::super::session::tests::{Store, credential, raw, server};
use super::*;
use crate::client::charts::tests::{catalogue, info, tracks};
use serde_json::Value;
use std::collections::BTreeMap;
use tuneweave_core::ChartCatalogView;

mod history;

fn html(v: Value) -> String {
    raw(v).replace("application/json", "text/html; charset=utf-8")
}
fn signed(r: &str) -> (url::Url, BTreeMap<String, String>, &str) {
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
    assert!(head.to_lowercase().contains("kg-tid: 369"));
    (url, q, body)
}

#[tokio::test]
async fn charts_keep_all_views_and_legacy_json_mime_without_reading_stored_accounts() {
    for view in [
        ChartCatalogView::Overview,
        ChartCatalogView::Summary,
        ChartCatalogView::Modern,
    ] {
        let mut f = server(vec![html(catalogue()).into()]).await;
        let store = Arc::new(Store::default());
        let old = credential("999", "account-secret");
        store.put(&old.stored("default").unwrap()).unwrap();
        f.provider.credential_store = Some(store.clone());
        let c = f
            .provider
            .chart_catalog(&ChartCatalogRequest::new(view))
            .await
            .unwrap();
        assert_eq!(c.view, view);
        assert_eq!(c.groups[0].charts.len(), 2);
        assert_eq!(c.groups[0].charts[0].extensions["rank_cid"], 0);
        assert_eq!(c.groups[0].charts[0].playable, None);
        let requests = f.requests.await.unwrap();
        assert_eq!(requests.len(), 1);
        let (url, q, body) = signed(&requests[0]);
        assert_eq!(url.path(), "/ocean/v6/rank/list");
        assert_eq!(q["plat"], "2");
        assert_eq!(q["withsong"], "1");
        assert_eq!(q["parentid"], "0");
        assert_eq!(body, "");
        assert!(!requests[0].contains("account-secret"));
        assert_eq!(
            store.values.lock().unwrap().get("default").unwrap(),
            &old.stored("default").unwrap()
        );
    }
}

#[tokio::test]
async fn charts_resolve_the_actual_period_then_fetch_all_pages_before_slicing_with_optional_tags() {
    for include_tags in [true, false] {
        let f = server(vec![
            html(info(105)).into(),
            raw(tracks(1, 105)).into(),
            raw(tracks(2, 105)).into(),
        ])
        .await;
        let mut r = ChartTrackListRequest::new(5, 98);
        r.include_tags = include_tags;
        let p = f.provider.chart_tracks("chart:42", &r).await.unwrap();
        assert_eq!(
            p.items.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(),
            ["1098", "1099", "1100", "1101", "1102"]
        );
        assert_eq!(p.pagination.total, Some(105));
        assert_eq!(p.pagination.next_offset, Some(103));
        assert_eq!(p.pagination.extensions["rank_cid"], "7");
        assert_eq!(p.items[0].extensions["chart_rank"], 99);
        assert_eq!(p.items[0].extensions["chart_position"], 98);
        assert_eq!(
            p.pagination.extensions.contains_key("chart_tags"),
            include_tags
        );
        assert_eq!(
            p.items[0].extensions.contains_key("chart_remarks"),
            include_tags
        );
        let requests = f.requests.await.unwrap();
        assert_eq!(requests.len(), 3);
        let mut identity = None;
        for (i, r) in requests.iter().enumerate() {
            let (url, q, body) = signed(r);
            let current = (q["mid"].clone(), q["uuid"].clone());
            if let Some(old) = &identity {
                assert_eq!(old, &current);
            } else {
                identity = Some(current);
            }
            if i == 0 {
                assert!(r.starts_with("GET "));
                assert_eq!(url.path(), "/ocean/v6/rank/info");
                assert_eq!(q["rankid"], "42");
                assert_eq!(q["rank_cid"], "0");
                assert_eq!(body, "");
            } else {
                assert!(r.starts_with("POST "));
                assert_eq!(url.path(), "/openapi/kmr/v2/rank/audio");
                assert_eq!(
                    serde_json::from_str::<Value>(body).unwrap(),
                    json!({"show_portrait_mv":1,"show_type_total":1,"filter_original_remarks":1,"area_code":1,"pagesize":100,"rank_cid":7,"type":1,"page":i,"rank_id":42})
                );
            }
        }
    }
}

#[tokio::test]
async fn charts_discard_partial_data_on_period_chart_rank_identity_or_total_conflicts() {
    for case in 0..8 {
        let mut second = tracks(2, 200);
        match case {
            0 => second["data"]["songlist"][0]["rank_cid"] = json!(8),
            1 => second["data"]["songlist"][0]["business"]["rank_id"] = json!("8"),
            2 => second["data"]["songlist"][0]["business"]["parent_id"] = json!("43"),
            3 => second["data"]["songlist"][0]["business"]["original_index"] = json!(1),
            4 => {
                second["total"] = json!(201);
                second["data"]["total"] = json!(201);
                second["extra"]["resp"]["all_total"] = json!(201);
            }
            5 => second["data"]["songlist"][0]["album_audio_id"] = json!(1000),
            6 => second["data"]["songlist"] = json!([]),
            7 => second = json!({"status":0,"error_code":20006,"errmsg":"private-error"}),
            _ => unreachable!(),
        }
        let f = server(vec![
            html(info(200)).into(),
            raw(tracks(1, 200)).into(),
            raw(second).into(),
        ])
        .await;
        let e = f
            .provider
            .chart_tracks("42", &ChartTrackListRequest::new(5, 0))
            .await
            .unwrap_err();
        assert_eq!(e.code, ErrorCode::UpstreamError);
        assert!(!format!("{e:?}").contains("private-error"));
        assert_eq!(f.requests.await.unwrap().len(), 3);
    }
    let f = server(vec![html(info(106)).into(), raw(tracks(1, 105)).into()]).await;
    assert!(
        f.provider
            .chart_tracks("42", &ChartTrackListRequest::new(5, 0))
            .await
            .is_err()
    );
    assert_eq!(f.requests.await.unwrap().len(), 2);
    let f = server(vec![html(info(12801)).into()]).await;
    assert!(
        f.provider
            .chart_tracks("42", &ChartTrackListRequest::new(5, 0))
            .await
            .is_err()
    );
    assert_eq!(f.requests.await.unwrap().len(), 1);
}

#[tokio::test]
async fn charts_handle_empty_and_out_of_range_complete_periods_without_playlists() {
    for (total, offset) in [(0, 0), (2, 30)] {
        let f = server(vec![html(info(total)).into(), raw(tracks(1, total)).into()]).await;
        let p = f
            .provider
            .chart_tracks("42", &ChartTrackListRequest::new(5, offset))
            .await
            .unwrap();
        assert!(p.items.is_empty());
        assert_eq!(p.pagination.total, Some(total));
        assert!(!p.pagination.has_more);
        assert_eq!(p.pagination.next_offset, None);
        assert_eq!(f.requests.await.unwrap().len(), 2);
    }
    let f = server(vec![
        html(json!({"status":1,"errcode":0,"data":{"total":0,"info":[]}})).into(),
    ])
    .await;
    let c = f
        .provider
        .chart_catalog(&ChartCatalogRequest::new(ChartCatalogView::Summary))
        .await
        .unwrap();
    assert!(c.groups.iter().all(|g| g.charts.is_empty()));
    assert_eq!(f.requests.await.unwrap().len(), 1);
}

#[tokio::test]
async fn charts_reject_invalid_ids_accounts_caller_scopes_and_overflow_before_network() {
    let f = server(vec![]).await;
    for id in [
        "",
        "0",
        "chart:0",
        "042",
        "chart:042",
        "+42",
        "-1",
        "chart:chart:42",
        "18446744073709551616",
        "https://untrusted.invalid",
    ] {
        assert_eq!(
            f.provider
                .chart_tracks(id, &ChartTrackListRequest::new(5, 0))
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
    }
    for (limit, offset) in [(0, 0), (101, 0), (1, u32::MAX)] {
        assert!(
            f.provider
                .chart_tracks("42", &ChartTrackListRequest::new(limit, offset))
                .await
                .is_err()
        );
    }
    let mut r = ChartCatalogRequest::new(ChartCatalogView::Summary);
    r.account = Some("other".into());
    assert!(f.provider.chart_catalog(&r).await.is_err());
    let mut r = ChartTrackListRequest::new(5, 0);
    r.account = Some("other".into());
    assert!(f.provider.chart_tracks("42", &r).await.is_err());
    let scoped = f
        .provider
        .caller_scope(&credential("999", "caller-secret").caller().unwrap())
        .unwrap();
    assert!(
        scoped
            .chart_catalog(&ChartCatalogRequest::new(ChartCatalogView::Summary))
            .await
            .is_err()
    );
    assert!(
        scoped
            .chart_tracks("42", &ChartTrackListRequest::new(5, 0))
            .await
            .is_err()
    );
    assert!(f.requests.await.unwrap().is_empty());
}

#[tokio::test]
async fn charts_keep_html_mime_exception_local_and_never_retry_redirect_or_return_html_as_success()
{
    let good = html(catalogue());
    for response in [
        "HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:1/private\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_owned(),
        good.replace("Content-Type:","SSA-CODE: private-challenge\r\nContent-Type:"),
        "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: 16\r\nConnection: close\r\n\r\n<html>bad</html>".to_owned(),
        "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: 1048577\r\nConnection: close\r\n\r\n".to_owned(),
        "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nConnection: close\r\n\r\n".to_owned()+&"x".repeat(1048577),
        good.replace("text/html; charset=utf-8","image/jpeg"),
    ] {
        let f=server(vec![response.into()]).await;let e=f.provider.chart_catalog(&ChartCatalogRequest::new(ChartCatalogView::Summary)).await.unwrap_err();
        assert!(!format!("{e:?}").contains("private-"));assert_eq!(f.requests.await.unwrap().len(),1);
    }
    for response in [
        html(tracks(1, 1)),
        "HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            .to_owned(),
    ] {
        let f = server(vec![html(info(1)).into(), response.into()]).await;
        assert!(
            f.provider
                .chart_tracks("42", &ChartTrackListRequest::new(5, 0))
                .await
                .is_err()
        );
        assert_eq!(f.requests.await.unwrap().len(), 2);
    }
}

#[tokio::test]
#[ignore = "uses official anonymous chart catalogue and complete period tracks"]
async fn live_public_charts_preserve_complete_periods_and_do_not_infer_unavailability_from_display_flags()
 {
    let p = KugouProvider::new(KugouConfig::default()).unwrap();
    let c = p
        .chart_catalog(&ChartCatalogRequest::new(ChartCatalogView::Summary))
        .await
        .unwrap();
    let charts = c.groups.iter().flat_map(|g| &g.charts).collect::<Vec<_>>();
    assert!(!charts.is_empty());
    for id in ["8888", "90379"] {
        let chart = charts.iter().find(|c| c.id.as_deref() == Some(id)).unwrap();
        assert_eq!(
            chart.resource_ref.as_ref().unwrap().id(),
            format!("chart:{id}")
        );
        assert_eq!(chart.playable, None);
        let page = p
            .chart_tracks(&format!("chart:{id}"), &ChartTrackListRequest::new(5, 98))
            .await
            .unwrap();
        assert_eq!(page.items.len(), 5);
        assert!(page.pagination.total.unwrap() > 100);
        assert_eq!(page.items[0].extensions["chart_rank"], 99);
        assert_eq!(page.items[0].extensions["chart_position"], 98);
        let period = &page.pagination.extensions["rank_cid"];
        assert_ne!(period, "0");
        for t in &page.items {
            assert_eq!(&t.extensions["rank_cid"], period);
            assert_eq!(t.extensions["chart_id"], id);
            assert!(t.duration_ms.is_some());
            assert_eq!(t.playable, None);
        }
        let first = &page.items[0];
        let detail = p.track(&first.id, None).await.unwrap();
        assert_eq!(first.resource_ref, detail.resource_ref);
        assert_eq!(first.name, detail.name);
    }
}
