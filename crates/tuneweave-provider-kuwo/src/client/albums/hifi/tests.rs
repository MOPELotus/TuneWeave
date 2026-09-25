use super::*;
use crate::KuwoProvider;
use crate::client::catalog::tests::{json_response, requests, response, setup};
use std::collections::BTreeMap;
use tuneweave_core::{Capability, MusicProvider};

fn request(limit: u32, offset: u32) -> AlbumListRequest {
    let mut request = AlbumListRequest::new(limit, offset);
    request.catalog = Some("hifi_latest".into());
    request
}

fn registration() -> Vec<u8> {
    json_response(&json!({"code":200,"success":true,"data":{"appuid":"1234567890"}}))
}

fn body(page: u32, total: u64) -> serde_json::Value {
    let start = u64::from(page - 1) * 18;
    let items = (start..total.min(start + 18))
        .map(|index| {
            json!({
                "id":index + 1, "name":format!("Album {} &amp; HiFi", index + 1),
                "artist":"First&amp;Name&Second", "artistId":336, "type":"album", "isStar":"0",
                "img":"http://img3.kuwo.cn/star/albumcover/300/s4s4/78/fixture.jpg",
                "hiresStatus":index % 2
            })
        })
        .collect::<Vec<_>>();
    json!({"code":200,"success":true,"data":{
        "total":total, "albumList":items,
        "tagList":[{"key":"sort","tags":[{"id":4001,"name":"最新"},{"id":4002,"name":"最热"}]}]
    }})
}

fn parse_fixture(value: &serde_json::Value) -> Result<HifiPage> {
    parse(&serde_json::to_vec(value).unwrap(), Sort::Latest, 1)
}

fn wire(raw: &str, sort: &str, page: u32) -> BTreeMap<String, String> {
    let url = Url::parse(&format!(
        "https://wapi.kuwo.cn{}",
        raw.split_whitespace().nth(1).unwrap()
    ))
    .unwrap();
    assert_eq!(url.path(), PATH);
    let pairs = url
        .query_pairs()
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect::<BTreeMap<_, _>>();
    assert_eq!(pairs.len(), 22);
    for (key, value) in [
        ("prod", "kwplayer_ar_12.2.2.0"),
        ("corp", "kuwo"),
        ("newver", "3"),
        ("vipver", "12.2.2.0"),
        ("source", "kwplayer_ar_12.2.2.0_newpcguanwangmobile.apk"),
        ("p2p", "1"),
        ("q36", "f2ce3c2ef68ddfd1b2bea7ed00001f314716"),
        ("approval", "false"),
        ("loginUid", "0"),
        ("loginSid", "0"),
        ("appuid", "1234567890"),
        ("uid", "1234567890"),
        ("allpay", "0"),
        ("notrace", "1"),
        ("oaid", ""),
        ("vipMode", "0"),
        ("plat", "ar"),
        ("rn", "18"),
    ] {
        assert_eq!(pairs[key], value, "{key}");
    }
    assert_eq!(pairs["sort"], sort);
    assert_eq!(pairs["pn"], page.to_string());
    assert_eq!(pairs["user"].len(), 32);
    assert_eq!(pairs["android_id"].len(), 32);
    assert_ne!(pairs["user"], pairs["android_id"]);
    let headers = raw.to_ascii_lowercase();
    for header in ["cookie:", "secret:", "authorization:"] {
        assert!(!headers.contains(header));
    }
    pairs
}

#[tokio::test]
async fn hifi_album_sdk_and_provider_use_official_sorts_and_anonymous_device() {
    for sdk in [false, true] {
        for (catalog, sort) in [("hifi_latest", "4001"), ("hifi_hot", "4002")] {
            let mut f = setup(vec![registration(), json_response(&body(1, 1))]).await;
            let mut request = request(20, 0);
            request.catalog = Some(catalog.into());
            let mut device = None;
            let result = if sdk {
                device = Some(
                    KuwoNativeDeviceStore::default()
                        .initialize(&f.client)
                        .await
                        .unwrap(),
                );
                f.client
                    .native_hifi_albums(&request, device.as_ref().unwrap())
                    .await
            } else {
                f.provider.albums(&request).await
            }
            .unwrap();
            assert!(f.provider.capabilities().contains(&Capability::AlbumList));
            assert_eq!(result.items[0].resource_ref.to_string(), "kuwo:1");
            assert_eq!(result.pagination.total, Some(1));
            assert!(!result.pagination.has_more);
            assert_eq!(result.pagination.next_offset, None);
            assert_eq!(result.pagination.extensions["catalog"], catalog);
            let seen = requests(&mut f, 2).await;
            let pairs = wire(&seen[1], sort, 1);
            if let Some(device) = device {
                assert_eq!(pairs["user"], device.device_user());
                assert_eq!(pairs["android_id"], device.android_id());
                assert_eq!(pairs["appuid"], device.app_uid());
            }
            let register = Url::parse(&format!(
                "https://wapi.kuwo.cn{}",
                seen[0].split_whitespace().nth(1).unwrap()
            ))
            .unwrap();
            let token = register
                .query_pairs()
                .find(|(k, _)| k == "token")
                .unwrap()
                .1
                .into_owned();
            let decoded = BASE64_STANDARD.decode(token).unwrap();
            let decoded = decoded
                .into_iter()
                .enumerate()
                .map(|(i, b)| b ^ b"yeelion "[i % 8])
                .collect::<Vec<_>>();
            let registered = url::form_urlencoded::parse(&decoded).collect::<BTreeMap<_, _>>();
            assert_eq!(registered["mac"], pairs["user"]);
            assert_eq!(registered["android_id"], pairs["android_id"]);
        }
    }
}

#[test]
fn hifi_album_metadata_preserves_album_artist_identity_without_inventing_rights() {
    let mut value = body(1, 1);
    value["data"]["albumList"][0]["secret"] = json!("not-exported");
    let page = parse_fixture(&value).unwrap();
    let album = &page.items[0];
    assert_eq!(album.id, "1");
    assert_eq!(album.name, "Album 1 & HiFi");
    assert_eq!(
        album.artists[0].resource_ref.as_ref().unwrap().to_string(),
        "kuwo:336"
    );
    assert_eq!(album.artists[0].name, "First&Name");
    assert_eq!(album.artists[1].name, "Second");
    assert!(album.artists[1].resource_ref.is_none());
    assert_eq!(
        album.cover_url.as_deref(),
        Some("https://img3.kuwo.cn/star/albumcover/300/s4s4/78/fixture.jpg")
    );
    assert!(album.published_at.is_none() && album.track_count.is_none() && album.kind.is_none());
    assert_eq!(album.extensions["catalogue_hires_status"], 0);
    assert_eq!(album.extensions["metadata_scope"], "catalogue_only");
    let serialized = serde_json::to_string(album).unwrap();
    for absent in [
        "not-exported",
        "quality",
        "rights",
        "playable",
        "complete_snapshot",
    ] {
        assert!(!serialized.contains(absent));
    }
    let detail = super::super::parse(&serde_json::to_vec(&json!({
        "id":"1","albumid":"1","name":"Album 1 &amp; HiFi",
        "artist":"First&amp;Name&Second","artistid":"336","songnum":"0","musiclist":[],"content_type":"0"
    })).unwrap(), "1").unwrap();
    assert_eq!(album.resource_ref, detail.album.resource_ref);
    assert_eq!(album.artists, detail.album.artists);
}

#[tokio::test]
async fn hifi_album_arbitrary_window_keeps_order_and_bounds_requested_pages() {
    for offset in [17, 179] {
        let pages = if offset == 17 {
            (1..=7).collect::<Vec<_>>()
        } else {
            std::iter::once(1).chain(10..=16).collect()
        };
        let mut responses = vec![registration()];
        responses.extend(pages.iter().map(|&p| json_response(&body(p, 400))));
        let mut f = setup(responses).await;
        let result = f.provider.albums(&request(100, offset)).await.unwrap();
        assert_eq!(
            result
                .items
                .iter()
                .map(|a| a.id.clone())
                .collect::<Vec<_>>(),
            (offset + 1..=offset + 100)
                .map(|id| id.to_string())
                .collect::<Vec<_>>()
        );
        assert_eq!(result.pagination.total, Some(400));
        assert_eq!(result.pagination.next_offset, Some(offset + 100));
        assert_eq!(
            result.pagination.extensions["upstream_pages_fetched"],
            pages.len()
        );
        let seen = requests(&mut f, pages.len() + 1).await;
        for (raw, page) in seen[1..].iter().zip(pages) {
            wire(raw, "4001", page);
        }
    }
}

#[tokio::test]
async fn hifi_album_tail_and_out_of_range_preserve_seed_total() {
    for offset in [35, 37, 999999] {
        let mut responses = vec![registration(), json_response(&body(1, 37))];
        if offset < 37 {
            responses.push(json_response(&body(2, 37)));
            responses.push(json_response(&body(3, 37)));
        }
        let mut f = setup(responses).await;
        let result = f.provider.albums(&request(100, offset)).await.unwrap();
        assert_eq!(result.items.len(), if offset < 37 { 2 } else { 0 });
        assert_eq!(result.pagination.total, Some(37));
        assert!(!result.pagination.has_more);
        assert_eq!(result.pagination.next_offset, None);
        requests(&mut f, if offset < 37 { 4 } else { 2 }).await;
    }
}

#[tokio::test]
async fn hifi_album_reuses_installation_context_without_account_session() {
    let mut f = setup(vec![
        registration(),
        json_response(&body(1, 2)),
        json_response(&body(1, 2)),
    ])
    .await;
    f.provider.albums(&request(1, 0)).await.unwrap();
    f.provider.albums(&request(1, 1)).await.unwrap();
    let seen = requests(&mut f, 3).await;
    assert_eq!(wire(&seen[1], "4001", 1), wire(&seen[2], "4001", 1));
}

#[tokio::test]
async fn hifi_album_explicit_empty_catalogue_has_complete_empty_page() {
    let mut f = setup(vec![registration(), json_response(&body(1, 0))]).await;
    let result = f.provider.albums(&request(20, 0)).await.unwrap();
    assert!(result.items.is_empty());
    assert_eq!(result.pagination.total, Some(0));
    assert!(!result.pagination.has_more);
    requests(&mut f, 2).await;
}

#[test]
fn hifi_album_missing_or_failed_envelope_is_not_empty_success() {
    for value in [
        json!({}),
        json!({"code":200,"success":true}),
        json!({"code":200,"success":true,"data":null}),
        json!({"code":200,"success":true,"data":{}}),
    ] {
        assert!(parse_fixture(&value).is_err());
    }
    for key in ["albumList", "tagList", "total"] {
        let mut value = body(1, 0);
        value["data"].as_object_mut().unwrap().remove(key);
        assert!(parse_fixture(&value).is_err(), "{key}");
    }
    for (key, bad) in [("code", json!(500)), ("success", json!(false))] {
        let mut value = body(1, 0);
        value[key] = bad;
        assert!(parse_fixture(&value).is_err());
    }
}

#[test]
fn hifi_album_requires_exact_album_identity_and_expected_sort_meaning() {
    for (key, bad) in [
        ("id", json!("0")),
        ("id", json!("01")),
        ("type", json!("radio")),
        ("isStar", json!(1)),
        ("content_type", json!(1)),
        ("hiresStatus", json!(2)),
        ("name", json!(" ")),
        ("artistId", json!(0)),
    ] {
        let mut value = body(1, 1);
        value["data"]["albumList"][0][key] = bad;
        assert!(parse_fixture(&value).is_err(), "{key}");
    }
    for tags in [
        json!([]),
        json!([{"key":"sort","tags":[]}]),
        json!([{"key":"sort","tags":[{"id":4001,"name":"随机"}]}]),
        json!([{"key":"sort","tags":[{"id":4001,"name":"最新"},{"id":4001,"name":"最新"}]}]),
        json!([{"key":"sort","tags":[]},{"key":"sort","tags":[]}]),
    ] {
        let mut value = body(1, 1);
        value["data"]["tagList"] = tags;
        assert!(parse_fixture(&value).is_err());
    }
}

#[test]
fn hifi_album_absent_optional_metadata_stays_unknown_and_images_are_trusted() {
    let mut value = body(1, 1);
    for key in ["artistId", "img", "hiresStatus"] {
        value["data"]["albumList"][0]
            .as_object_mut()
            .unwrap()
            .remove(key);
    }
    let page = parse_fixture(&value).unwrap();
    assert!(page.items[0].artists[0].resource_ref.is_none());
    assert_eq!(page.items[0].artists.len(), 1);
    assert!(page.items[0].cover_url.is_none());
    assert!(
        !page.items[0]
            .extensions
            .contains_key("catalogue_hires_status")
    );
    for img in [
        "https://example.test/cover.jpg",
        "http://img3.kuwo.cn.evil.test/star/albumcover/a.jpg",
        "https://user@img3.kuwo.cn/star/albumcover/a.jpg",
        "https://img3.kuwo.cn/star/albumcover/a.jpg?sid=private",
        "https://img3.kuwo.cn:8443/star/albumcover/a.jpg",
        "ftp://img3.kuwo.cn/star/albumcover/a.jpg",
    ] {
        value["data"]["albumList"][0]["img"] = json!(img);
        assert!(parse_fixture(&value).unwrap().items[0].cover_url.is_none());
    }
}

#[tokio::test]
async fn hifi_album_changed_total_or_repeated_identity_cannot_return_partial_window() {
    for duplicate in [false, true] {
        let mut second = body(2, if duplicate { 36 } else { 35 });
        if duplicate {
            second["data"]["albumList"][0]["id"] = json!(1);
        }
        let mut f = setup(vec![
            registration(),
            json_response(&body(1, 36)),
            json_response(&second),
        ])
        .await;
        assert!(f.provider.albums(&request(20, 0)).await.is_err());
        requests(&mut f, 3).await;
    }
    let mut value = body(1, 2);
    value["data"]["albumList"][1]["id"] = json!(1);
    assert!(parse_fixture(&value).is_err());
    let mut value = body(1, 18);
    value["data"]["albumList"].as_array_mut().unwrap().pop();
    assert!(parse_fixture(&value).is_err());
}

#[tokio::test]
async fn hifi_album_transport_failure_stops_without_partial_or_unauthenticated_fallback() {
    for bad in [
        response(500, "application/json", "", b"{}"),
        response(200, "text/html", "", b"<html/>"),
        response(200, "application/json", "", b"not json"),
    ] {
        let mut f = setup(vec![registration(), bad]).await;
        assert!(f.provider.albums(&request(20, 0)).await.is_err());
        requests(&mut f, 2).await;
    }
    let mut f = setup(vec![json_response(
        &json!({"code":200,"success":false,"data":{"appuid":"1234567890"}}),
    )])
    .await;
    assert!(f.provider.albums(&request(20, 0)).await.is_err());
    requests(&mut f, 1).await;
}

#[tokio::test]
async fn hifi_album_invalid_filters_and_account_fail_before_registration() {
    let mut invalid = vec![
        AlbumListRequest::new(20, 0),
        request(0, 0),
        request(101, 0),
        request(2, u32::MAX),
    ];
    for catalog in ["hifi", "all", "4001", ""] {
        let mut value = request(20, 0);
        value.catalog = Some(catalog.into());
        invalid.push(value);
    }
    for area in ["all", "china", ""] {
        let mut value = request(20, 0);
        value.area = Some(area.into());
        invalid.push(value);
    }
    let mut value = request(20, 0);
    value.account = Some("private-account".into());
    invalid.push(value);
    let mut f = setup(vec![]).await;
    for value in invalid {
        assert_eq!(
            f.provider.albums(&value).await.unwrap_err().code,
            ErrorCode::InvalidRequest
        );
    }
    assert!(f.seen.try_recv().is_err());
}

#[tokio::test]
async fn hifi_album_caller_scope_is_rejected_without_rotation_or_network() {
    let mut f = setup(vec![]).await;
    let credential = crate::client::native::tests::credential_fixture("42", "private-hifi-session")
        .caller()
        .unwrap();
    let caller = f.provider.with_caller_credential(&credential).unwrap();
    assert_eq!(
        caller.albums(&request(20, 0)).await.unwrap_err().code,
        ErrorCode::CapabilityNotSupported
    );
    assert!(caller.take_response_credential().unwrap().is_none());
    assert!(f.seen.try_recv().is_err());
}

#[tokio::test]
#[ignore = "Current official anonymous HiFi album catalog metadata only; no account or media"]
async fn live_hifi_album_catalogue_supports_both_official_sorts() {
    let provider = KuwoProvider::new(KuwoConfig::default()).unwrap();
    for catalog in ["hifi_latest", "hifi_hot"] {
        let mut request = request(5, 0);
        request.catalog = Some(catalog.into());
        let page = provider.albums(&request).await.unwrap();
        assert!(
            !page.items.is_empty(),
            "empty official catalogue: {catalog}"
        );
        assert_eq!(page.pagination.offset, 0);
        assert_eq!(page.pagination.limit, 5);
        assert_eq!(page.pagination.extensions["catalog"], catalog);
        let total = page.pagination.total.unwrap();
        assert!(total >= page.items.len() as u64);
        assert_eq!(page.pagination.has_more, total > page.items.len() as u64);
        assert!(page.items.iter().all(|album| {
            !album.id.is_empty()
                && !album.name.is_empty()
                && album.resource_ref.platform() == Platform::Kuwo
        }));
    }
}
