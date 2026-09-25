use super::super::*;
use crate::client::{
    catalog::{
        CatalogKind,
        tests::{body, home, json_response, requests, response, setup, setup_with_gates},
    },
    native::tests as native_fixture,
};
use std::time::Duration;
use tokio::sync::Notify;
use tuneweave_core::{
    AccountCredentialStore, ErrorCode, RadioStationCursor, RadioStationListRequest,
    RadioTaxonomyRequest, StoredAccountCredential,
};

fn envelope(data: serde_json::Value) -> serde_json::Value {
    json!({"code":200,"curTime":1790129138586_u64,"data":data})
}
fn taxonomy() -> RadioTaxonomyRequest {
    RadioTaxonomyRequest { account: None }
}
fn menu() -> serde_json::Value {
    envelope(json!([
        {"categoryKey":3,"id":16,"name":"Category A","priority":99},
        {"categoryKey":95,"name":"收藏"}, {"categoryKey":96,"name":"百城声音"},
        {"categoryKey":97,"name":"最近"}, {"categoryKey":98,"name":"地区"},
        {"categoryKey":99,"name":"公告"}, {"categoryKey":"8","name":"Category B"}
    ]))
}
fn regions() -> serde_json::Value {
    envelope(json!([{"locationKey":7,"name":"北京"},{"locationKey":21,"name":"江苏"}]))
}
fn row(id: u64) -> serde_json::Value {
    json!({
        "channel_key":id.to_string(),"channel_name":format!("Broadcast {id}"),
        "channel_image_url":format!("https://image.kuwo.cn/mobile/fmRadio/{id}.jpg"),
        "category_key":"3","location_key":"7",
        "flow_url":format!("https://hls-pull-fm.kuwo.cn/kuwofm/stream_key_{id}.m3u8"),
        "hz":"102.5 1025","program_key":"1018422","program_name":"Current programme",
        "program_compere":"Presenter","listener_count":"8419",
        "unknown":{"sid":"private-unexported","endpoint":"https://evil.invalid"}
    })
}
fn list(count: u64) -> serde_json::Value {
    envelope(json!((1..=count).map(row).collect::<Vec<_>>()))
}
fn station_request(limit: u32, offset: u32) -> RadioStationListRequest {
    let mut request = RadioStationListRequest::new(limit);
    request.offset = offset;
    request
}
fn wire(wire: &str, path: &str) {
    let url = url::Url::parse(&format!(
        "https://wapi.kuwo.cn{}",
        wire.split_whitespace().nth(1).unwrap()
    ))
    .unwrap();
    assert_eq!(url.path(), path);
    if path.ends_with("/category_list") {
        assert_eq!(url.query(), Some("loginUid=0"));
    } else {
        assert!(url.query().is_none());
    }
    assert!(wire.starts_with("GET "));
    let lower = wire.to_ascii_lowercase();
    for header in ["cookie:", "secret:", "authorization:"] {
        assert!(!lower.contains(header));
    }
}

#[tokio::test]
async fn native_fm_taxonomy_separates_navigation_from_categories_and_preserves_regions() {
    for sdk in [false, true] {
        let mut f = setup(vec![json_response(&menu()), json_response(&regions())]).await;
        let result = if sdk {
            f.provider.client.radio_taxonomy(&taxonomy()).await
        } else {
            f.provider.radio_taxonomy(&taxonomy()).await
        }
        .unwrap();
        assert_eq!(
            result
                .categories
                .iter()
                .map(|x| (x.id.as_str(), x.name.as_str()))
                .collect::<Vec<_>>(),
            [("3", "Category A"), ("8", "Category B")]
        );
        assert_eq!(
            result
                .regions
                .iter()
                .map(|x| (x.id.as_str(), x.name.as_str()))
                .collect::<Vec<_>>(),
            [("7", "北京"), ("21", "江苏")]
        );
        let nav = result.extensions["native_navigation"].as_array().unwrap();
        assert_eq!(
            nav.iter()
                .map(|x| x["role"].as_str().unwrap())
                .collect::<Vec<_>>(),
            [
                "account_favorites",
                "artists",
                "local_recent",
                "regions",
                "hot_list"
            ]
        );
        assert_eq!(nav[4]["name"], "公告");
        assert_eq!(
            result.extensions["upstream_maintenance_notice"]["title"],
            "广播电台服务停止维护"
        );
        assert!(!result.extensions.contains_key("complete_snapshot"));
        let seen = requests(&mut f, 2).await;
        wire(&seen[0], "/api/fmradio/category_list");
        wire(&seen[1], "/api/fmradio/location_list");
    }
}

#[tokio::test]
async fn native_fm_current_notice_menu_and_explicit_empty_do_not_invent_fallback_categories() {
    for rows in [json!([]), json!([{"categoryKey":99,"name":"公告"}])] {
        let mut f = setup(vec![
            json_response(&envelope(rows.clone())),
            json_response(&envelope(json!([]))),
        ])
        .await;
        let result = f.provider.radio_taxonomy(&taxonomy()).await.unwrap();
        assert!(result.categories.is_empty());
        assert!(result.regions.is_empty());
        assert_eq!(
            result.extensions["native_navigation"]
                .as_array()
                .unwrap()
                .len(),
            rows.as_array().unwrap().len()
        );
        requests(&mut f, 2).await;
    }
}

#[tokio::test]
async fn native_fm_taxonomy_rejects_duplicate_and_invalid_identity_without_partial_output() {
    for region in [false, true] {
        for change in 0..6 {
            let mut categories = menu();
            let mut locations = regions();
            let data = if region {
                &mut locations["data"]
            } else {
                &mut categories["data"]
            };
            let field = if region { "locationKey" } else { "categoryKey" };
            match change {
                0 => data[1][field] = data[0][field].clone(),
                1 => data[0][field] = json!("01"),
                2 => data[0][field] = json!(0),
                3 => data[0][field] = json!(2147483648_u64),
                4 => data[0]["name"] = json!(" \t"),
                _ => {
                    *data = json!(
                        (1..=257)
                            .map(|i| {
                                let mut item = json!({"name":"X"});
                                item[field] = json!(i);
                                item
                            })
                            .collect::<Vec<_>>()
                    )
                }
            }
            let mut f = setup(vec![json_response(&categories), json_response(&locations)]).await;
            assert_eq!(
                f.provider
                    .radio_taxonomy(&taxonomy())
                    .await
                    .unwrap_err()
                    .code,
                ErrorCode::UpstreamError
            );
            requests(&mut f, 2).await;
        }
    }
}

#[tokio::test]
async fn native_fm_local_windows_use_one_unpaginated_response_for_each_filter() {
    for filter in 0..3 {
        for (limit, offset, wanted) in [
            (1, 0, 1),
            (20, 19, 20),
            (100, 3, 100),
            (20, 100, 3),
            (20, 103, 0),
            (1, u32::MAX - 1, 0),
        ] {
            let mut request = station_request(limit, offset);
            let path = match filter {
                0 => "/api/fmradio/hot_list",
                1 => {
                    request.category_id = Some("3".into());
                    "/api/fmradio/category_radio_list/3"
                }
                _ => {
                    request.region_id = Some("7".into());
                    "/api/fmradio/location_radio_list/7"
                }
            };
            let mut f = setup(vec![json_response(&list(103))]).await;
            let result = f.provider.radio_stations(&request).await.unwrap();
            assert_eq!(result.items.len(), wanted);
            assert_eq!(
                result
                    .items
                    .iter()
                    .map(|x| x.id.clone())
                    .collect::<Vec<_>>(),
                (u64::from(offset) + 1..u64::from(offset) + 1 + wanted as u64)
                    .map(|n| format!("fm:{n}"))
                    .collect::<Vec<_>>()
            );
            assert_eq!(result.pagination.total, Some(103));
            let more = u64::from(offset) + (wanted as u64) < 103;
            assert_eq!(result.pagination.has_more, more);
            assert_eq!(
                result.pagination.next_offset,
                more.then_some(offset + wanted as u32)
            );
            assert_eq!(
                result.pagination.extensions["pagination_scope"],
                "single_upstream_response"
            );
            assert!(
                !result
                    .pagination
                    .extensions
                    .contains_key("complete_snapshot")
            );
            wire(&requests(&mut f, 1).await[0], path);
        }
    }
}

#[tokio::test]
async fn native_fm_empty_array_is_empty_but_absent_or_null_data_is_an_error() {
    let mut f = setup(vec![json_response(&envelope(json!([])))]).await;
    let result = f
        .provider
        .client
        .radio_stations(&station_request(20, 10))
        .await
        .unwrap();
    assert!(result.items.is_empty());
    assert_eq!(result.pagination.total, Some(0));
    assert!(!result.pagination.has_more);
    requests(&mut f, 1).await;
    for value in [
        json!({"code":200,"msg":"success","curTime":1790129138586_u64}),
        envelope(json!(null)),
        envelope(json!({})),
        envelope(json!("")),
        json!({"code":200,"success":false,"data":[]}),
        json!({"code":-1,"data":[]}),
    ] {
        let mut f = setup(vec![json_response(&value)]).await;
        let mut request = station_request(20, 0);
        request.region_id = Some("3".into());
        assert_eq!(
            f.provider.radio_stations(&request).await.unwrap_err().code,
            ErrorCode::UpstreamError
        );
        requests(&mut f, 1).await;
    }
}

#[tokio::test]
async fn native_fm_whole_list_validation_rejects_duplicates_filter_mismatches_and_bad_unseen_rows()
{
    for change in 0..7 {
        let mut value = list(3);
        let mut request = station_request(1, 0);
        match change {
            0 => value["data"][2] = row(1),
            1 => {
                request.region_id = Some("7".into());
                value["data"][2]["location_key"] = json!("21");
            }
            2 => {
                request.category_id = Some("3".into());
                value["data"][2]["category_key"] = json!("8");
            }
            3 => {
                request.region_id = Some("7".into());
                value["data"][2]
                    .as_object_mut()
                    .unwrap()
                    .remove("location_key");
            }
            4 => value["data"][2]["channel_name"] = json!(""),
            5 => {
                value["data"][2]["flow_url"] =
                    json!("https://hls-pull-fm.kuwo.cn/kuwofm/stream_key_1.m3u8")
            }
            _ => value["data"] = json!(vec![json!({"channel_key":"1","channel_name":"A"}); 10001]),
        }
        let mut f = setup(vec![json_response(&value)]).await;
        assert_eq!(
            f.provider.radio_stations(&request).await.unwrap_err().code,
            ErrorCode::UpstreamError
        );
        requests(&mut f, 1).await;
    }
}

#[tokio::test]
async fn native_fm_detail_sdk_and_provider_keep_broadcast_identity_program_and_trusted_stream() {
    for sdk in [false, true] {
        for id in ["359", "fm:359"] {
            let mut f = setup(vec![json_response(&envelope(row(359)))]).await;
            let station = if sdk {
                f.provider.client.radio_station(id, None).await
            } else {
                f.provider.radio_station(id, None).await
            }
            .unwrap();
            assert_eq!(station.id, "fm:359");
            assert_eq!(station.resource_ref.to_string(), "kuwo:fm:359");
            assert_eq!(station.name, "Broadcast 359");
            assert_eq!(
                station.stream_url.as_deref(),
                Some("https://hls-pull-fm.kuwo.cn/kuwofm/stream_key_359.m3u8")
            );
            assert_eq!(
                station.cover_url.as_deref(),
                Some("https://image.kuwo.cn/mobile/fmRadio/359.jpg")
            );
            assert_eq!(
                station.current_program.as_deref(),
                Some("Current programme")
            );
            assert!(
                station.category.is_none()
                    && station.region.is_none()
                    && station.subscribed.is_none()
            );
            assert_eq!(station.extensions["category_id"], "3");
            assert_eq!(station.extensions["region_id"], "7");
            assert_eq!(station.extensions["frequency_label"], "102.5");
            assert_eq!(station.extensions["program_key"], "1018422");
            assert_eq!(station.extensions["reported_listener_count"], 8419);
            assert_eq!(
                station.extensions["stream_validation"],
                "official_url_metadata_only"
            );
            let serialized = serde_json::to_string(&station).unwrap();
            assert!(
                !serialized.contains("private-unexported") && !serialized.contains("evil.invalid")
            );
            wire(&requests(&mut f, 1).await[0], "/api/fmradio/radio_info/359");
        }
    }
}

#[tokio::test]
async fn native_fm_detail_keeps_missing_media_and_program_unknown_but_never_wrong_identity() {
    let minimal = envelope(json!({"channel_key":359,"channel_name":"Station"}));
    let mut f = setup(vec![json_response(&minimal)]).await;
    let station = f.provider.radio_station("fm:359", None).await.unwrap();
    assert!(
        station.stream_url.is_none()
            && station.cover_url.is_none()
            && station.current_program.is_none()
    );
    assert!(
        !station.extensions.contains_key("category_id")
            && !station.extensions.contains_key("program_key")
    );
    requests(&mut f, 1).await;
    for value in [
        envelope(row(360)),
        envelope(json!({})),
        envelope(json!([])),
        envelope(json!(null)),
        json!({"code":200}),
        json!({"code":404,"data":row(359)}),
    ] {
        let mut f = setup(vec![json_response(&value)]).await;
        assert_eq!(
            f.provider
                .radio_station("359", None)
                .await
                .unwrap_err()
                .code,
            ErrorCode::UpstreamError
        );
        requests(&mut f, 1).await;
    }
}

#[tokio::test]
async fn native_fm_stream_identity_and_url_shape_are_checked_without_fetching_the_stream() {
    for url in [
        "http://hls-pull-fm.kuwo.cn/kuwofm/stream_key_359.m3u8",
        "https://hls-pull-fm.kuwo.cn.evil.invalid/kuwofm/stream_key_359.m3u8",
        "https://x@hls-pull-fm.kuwo.cn/kuwofm/stream_key_359.m3u8",
        "https://hls-pull-fm.kuwo.cn:444/kuwofm/stream_key_359.m3u8",
        "https://hls-pull-fm.kuwo.cn/kuwofm/stream_key_359.m3u8?token=untrusted",
        "https://hls-pull-fm.kuwo.cn/kuwofm/stream_key_359.m3u8#fragment",
        "https://hls-pull-fm.kuwo.cn/kuwofm/stream_key_360.m3u8",
        "https://hls-pull-fm.kuwo.cn/kuwofm/stream_key_%33359.m3u8",
        "https://hls-pull-fm.kuwo.cn/kuwofm/../stream_key_359.m3u8",
        "file:///kuwofm/stream_key_359.m3u8",
    ] {
        let mut value = row(359);
        value["flow_url"] = json!(url);
        let mut f = setup(vec![json_response(&envelope(value))]).await;
        assert_eq!(
            f.provider
                .radio_station("359", None)
                .await
                .unwrap_err()
                .code,
            ErrorCode::UpstreamError
        );
        requests(&mut f, 1).await;
    }
    let mut value = row(359);
    value["channel_image_url"] = json!("https://image.kuwo.cn/mobile/fmRadio/360.jpg");
    let mut f = setup(vec![json_response(&envelope(value))]).await;
    assert!(
        f.provider
            .radio_station("359", None)
            .await
            .unwrap()
            .cover_url
            .is_none()
    );
    requests(&mut f, 1).await;
}

#[tokio::test]
async fn native_fm_malformed_station_fields_are_not_defaulted_or_returned_partially() {
    for (field, value) in [
        ("channel_key", json!("0359")),
        ("channel_key", json!(0)),
        ("channel_key", json!(true)),
        ("channel_name", json!("  ")),
        ("channel_name", json!("bad\u{0000}")),
        ("channel_name", json!("x".repeat(513))),
        ("program_name", json!("bad\nprogram")),
        ("category_key", json!("-1")),
        ("location_key", json!("2147483648")),
        ("listener_count", json!("18446744073709551616")),
        ("hz", json!("X".repeat(129))),
    ] {
        let mut item = row(359);
        item[field] = value;
        let mut f = setup(vec![json_response(&envelope(item))]).await;
        assert_eq!(
            f.provider
                .radio_station("359", None)
                .await
                .unwrap_err()
                .code,
            ErrorCode::UpstreamError
        );
        requests(&mut f, 1).await;
    }
}

struct NoAccountAccess;
impl AccountCredentialStore for NoAccountAccess {
    fn load_platform(&self, _: Platform) -> Result<Vec<StoredAccountCredential>> {
        panic!("public FM read accounts")
    }
    fn put(&self, _: &StoredAccountCredential) -> Result<()> {
        panic!("public FM wrote accounts")
    }
    fn remove(&self, _: Platform, _: &str) -> Result<bool> {
        panic!("public FM removed accounts")
    }
}

#[tokio::test]
async fn native_fm_invalid_inputs_and_account_scopes_fail_before_io() {
    let mut f = setup(vec![
        json_response(&menu()),
        json_response(&regions()),
        json_response(&list(1)),
        json_response(&envelope(row(359))),
    ])
    .await;
    f.provider.credential_store = Some(Arc::new(NoAccountAccess));
    for id in [
        "",
        "0",
        "01",
        "+1",
        " 1",
        "fm:0",
        "fm:fm:1",
        "Music_359",
        "track:359",
        "../359",
        "18446744073709551616",
    ] {
        assert_eq!(
            f.provider.radio_station(id, None).await.unwrap_err().code,
            ErrorCode::InvalidRequest
        );
        assert_eq!(
            f.provider
                .client
                .radio_station(id, None)
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
    }
    let mut invalids = vec![
        station_request(0, 0),
        station_request(101, 0),
        station_request(1, u32::MAX),
    ];
    let mut cursor = station_request(20, 0);
    cursor.cursor = Some(RadioStationCursor {
        id: "1".into(),
        score: 0,
    });
    invalids.push(cursor);
    let mut both = station_request(20, 0);
    both.category_id = Some("3".into());
    both.region_id = Some("7".into());
    invalids.push(both);
    for id in ["", "0", "01", "fm:7", "2147483648"] {
        let mut request = station_request(20, 0);
        request.region_id = Some(id.into());
        invalids.push(request);
    }
    for id in ["95", "96", "97", "98", "99"] {
        let mut request = station_request(20, 0);
        request.category_id = Some(id.into());
        invalids.push(request);
    }
    for request in invalids {
        assert_eq!(
            f.provider.radio_stations(&request).await.unwrap_err().code,
            ErrorCode::InvalidRequest
        );
        assert_eq!(
            f.provider
                .client
                .radio_stations(&request)
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
    }
    for account in ["", "default", "chosen"] {
        let t = RadioTaxonomyRequest {
            account: Some(account.into()),
        };
        let mut l = station_request(20, 0);
        l.account = Some(account.into());
        assert_eq!(
            f.provider.radio_taxonomy(&t).await.unwrap_err().code,
            ErrorCode::InvalidRequest
        );
        assert_eq!(
            f.provider.client.radio_taxonomy(&t).await.unwrap_err().code,
            ErrorCode::InvalidRequest
        );
        assert_eq!(
            f.provider.radio_stations(&l).await.unwrap_err().code,
            ErrorCode::InvalidRequest
        );
        assert_eq!(
            f.provider.client.radio_stations(&l).await.unwrap_err().code,
            ErrorCode::InvalidRequest
        );
        assert_eq!(
            f.provider
                .radio_station("359", Some(account))
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
        assert_eq!(
            f.provider
                .client
                .radio_station("359", Some(account))
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
    }
    let credential = native_fixture::credential_fixture("42", "private-fm-session")
        .caller()
        .unwrap();
    let caller = f.provider.with_caller_credential(&credential).unwrap();
    assert_eq!(
        caller.radio_taxonomy(&taxonomy()).await.unwrap_err().code,
        ErrorCode::CapabilityNotSupported
    );
    assert_eq!(
        caller
            .radio_stations(&station_request(20, 0))
            .await
            .unwrap_err()
            .code,
        ErrorCode::CapabilityNotSupported
    );
    assert_eq!(
        caller.radio_station("359", None).await.unwrap_err().code,
        ErrorCode::CapabilityNotSupported
    );
    assert!(caller.take_response_credential().unwrap().is_none());
    assert!(f.seen.try_recv().is_err());
    f.provider.radio_taxonomy(&taxonomy()).await.unwrap();
    f.provider
        .radio_stations(&station_request(20, 0))
        .await
        .unwrap();
    f.provider.radio_station("359", None).await.unwrap();
    let seen = requests(&mut f, 4).await;
    assert!(seen.iter().all(|wire| !wire.contains("private-fm-session")));
}

#[tokio::test]
async fn native_fm_keeps_signed_catalogue_cache_and_set_cookie_isolated() {
    let poisoned = response(
        200,
        "application/json",
        "Set-Cookie: Hm_Iuvt_cdb524f42f23cer9b268564v7y735ewrq2324=poison-fm-cookie; Path=/\r\n",
        &serde_json::to_vec(&list(1)).unwrap(),
    );
    let mut f = setup(vec![
        home(),
        json_response(&body(CatalogKind::Album, 1, 0)),
        poisoned,
        json_response(&envelope(row(359))),
        json_response(&body(CatalogKind::Album, 1, 0)),
    ])
    .await;
    f.provider
        .client
        .search_catalog_page(CatalogKind::Album, "A", 1)
        .await
        .unwrap();
    f.provider
        .radio_stations(&station_request(20, 0))
        .await
        .unwrap();
    f.provider.radio_station("359", None).await.unwrap();
    f.provider
        .client
        .search_catalog_page(CatalogKind::Album, "A", 1)
        .await
        .unwrap();
    let seen = requests(&mut f, 5).await;
    wire(&seen[2], "/api/fmradio/hot_list");
    wire(&seen[3], "/api/fmradio/radio_info/359");
    assert!(
        seen[1].contains("anonymousCatalogueCookie123456")
            && seen[4].contains("anonymousCatalogueCookie123456")
    );
    assert!(seen.iter().all(|wire| !wire.contains("poison-fm-cookie")));
}

#[tokio::test]
async fn native_fm_transport_and_size_failures_do_not_follow_redirects_or_return_empty() {
    let oversized = vec![b' '; 2 * 1024 * 1024 + 1];
    let mut streamed = format!(
        "HTTP/1.1 200 Fixture\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n{:x}\r\n",
        oversized.len()
    )
    .into_bytes();
    streamed.extend_from_slice(&oversized);
    streamed.extend_from_slice(b"\r\n0\r\n\r\n");
    for (reply, code) in [
        (
            response(403, "text/plain", "", b"private-body"),
            ErrorCode::UpstreamError,
        ),
        (
            response(429, "application/json", "", b"private-body"),
            ErrorCode::RateLimited,
        ),
        (
            response(503, "application/json", "", b"private-body"),
            ErrorCode::UpstreamError,
        ),
        (
            response(302, "text/html", "Location: https://evil.invalid/\r\n", b""),
            ErrorCode::UpstreamError,
        ),
        (
            response(200, "text/html", "", b"{}"),
            ErrorCode::UpstreamError,
        ),
        (
            response(200, "application/json", "", b"private-body"),
            ErrorCode::UpstreamError,
        ),
        (
            response(200, "application/json", "", &oversized),
            ErrorCode::UpstreamError,
        ),
        (streamed, ErrorCode::UpstreamError),
    ] {
        let mut f = setup(vec![reply]).await;
        let error = f
            .provider
            .radio_stations(&station_request(20, 0))
            .await
            .unwrap_err();
        assert_eq!(error.code, code);
        assert!(!format!("{error:?}").contains("private-body"));
        requests(&mut f, 1).await;
    }
}

#[tokio::test]
async fn native_fm_total_budget_and_cancellation_cover_every_taxonomy_and_station_boundary() {
    for sdk in [false, true] {
        for operation in 0..3 {
            for boundary in 0..if operation == 0 { 2 } else { 1 } {
                for cancel in [false, true] {
                    let gate = Arc::new(Notify::new());
                    let mut replies = match operation {
                        0 => vec![json_response(&menu()), json_response(&regions())],
                        1 => vec![json_response(&list(1))],
                        _ => vec![json_response(&envelope(row(359)))],
                    };
                    replies.truncate(boundary + 1);
                    let mut f = setup_with_gates(
                        replies
                            .into_iter()
                            .enumerate()
                            .map(|(i, reply)| (reply, (i == boundary).then(|| gate.clone())))
                            .collect(),
                    )
                    .await;
                    native_fixture::set_request_timeout(
                        &mut f.provider.client,
                        Duration::from_secs(60),
                    );
                    let provider = f.provider.clone();
                    let task = tokio::spawn(async move {
                        match (sdk, operation) {
                            (true, 0) => provider
                                .client
                                .radio_taxonomy(&taxonomy())
                                .await
                                .map(|_| ()),
                            (true, 1) => provider
                                .client
                                .radio_stations(&station_request(20, 0))
                                .await
                                .map(|_| ()),
                            (true, _) => {
                                provider.client.radio_station("359", None).await.map(|_| ())
                            }
                            (false, 0) => provider.radio_taxonomy(&taxonomy()).await.map(|_| ()),
                            (false, 1) => provider
                                .radio_stations(&station_request(20, 0))
                                .await
                                .map(|_| ()),
                            (false, _) => provider.radio_station("359", None).await.map(|_| ()),
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
                        let error = result.unwrap().unwrap_err();
                        assert_eq!(error.code, ErrorCode::UpstreamTimeout);
                        assert!(error.message.contains("total time budget"));
                    }
                    gate.notify_one();
                    (&mut f.server).await.unwrap();
                    assert!(f.seen.try_recv().is_err());
                }
            }
        }
    }
}
