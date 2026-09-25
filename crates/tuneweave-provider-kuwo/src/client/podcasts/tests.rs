use super::*;
use crate::client::catalog::tests::{json_response, requests, setup};
use tuneweave_core::{
    Capability, MusicProvider, PodcastCatalog, PodcastListRequest, PodcastTaxonomyKind,
    PodcastTaxonomyRequest,
};

fn body() -> serde_json::Value {
    json!({"albumid":"9240675","isstar":"1","content_type":"0",
        "name":"主播节目集","desc":"节目简介\n第二行", "mcnum":"457",
        "artist":"主播", "artid":"3194753", "art_uid":"do-not-export",
        "big_pic":"http://img4.kuwo.cn/star/albumcover/1000/70/89/71164012.jpg",
        "artpic":"https://img1.kuwo.cn/star/starheads/120/74/21/1879470010.jpg",
        "coll_num":3993,"pnum":"79005","payPolicy":0,
        "opaque":{"sid":"do-not-export"},"richtext":[{"url":"https://foreign.invalid"}]})
}

#[tokio::test]
async fn anchor_detail_provider_preserves_show_identity_and_unknown_entitlements() {
    let mut f = setup(vec![json_response(&body())]).await;
    let result = f.provider.podcast("anchor:9240675", None).await.unwrap();
    assert_eq!(result.resource_ref.to_string(), "kuwo:anchor:9240675");
    assert_eq!(result.description, "节目简介\n第二行");
    assert_eq!(result.episode_count, Some(457));
    assert_eq!(result.subscriber_count, Some(3993));
    assert_eq!(result.play_count, Some(79005));
    assert_eq!(
        result.creator.unwrap().resource_ref.unwrap().id(),
        "3194753"
    );
    assert_eq!(result.paid, None);
    assert_eq!(result.purchased, None);
    assert_eq!(result.subscribed, None);
    let serialized = serde_json::to_string(&result.extensions).unwrap();
    assert!(!serialized.contains("do-not-export"));
    assert!(!serialized.contains("foreign.invalid"));
    let wire = requests(&mut f, 1).await.remove(0);
    assert!(wire.starts_with("GET /basedata.s?type=get_album_info&id=9240675&szb=1&aapiver=1 "));
    for secret in ["cookie:", "secret:", "authorization:", "loginsid", "appuid"] {
        assert!(!wire.to_lowercase().contains(secret));
    }
}

#[tokio::test]
async fn anchor_detail_invalid_ids_and_accounts_never_reach_upstream() {
    let mut f = setup(vec![]).await;
    for id in [
        "9240675",
        "fm:9240675",
        "anchor:0",
        "anchor:01",
        "anchor:+1",
        "anchor:1/2",
        "anchor:18446744073709551616",
    ] {
        assert_eq!(
            f.provider.podcast(id, None).await.unwrap_err().code,
            ErrorCode::InvalidRequest
        );
    }
    assert_eq!(
        f.provider
            .podcast("anchor:9240675", Some("default"))
            .await
            .unwrap_err()
            .code,
        ErrorCode::InvalidRequest
    );
    let credential =
        crate::client::native::tests::credential_fixture("42", "private-anchor-session")
            .caller()
            .unwrap();
    let caller = f.provider.with_caller_credential(&credential).unwrap();
    assert_eq!(
        caller
            .podcast("anchor:9240675", None)
            .await
            .unwrap_err()
            .code,
        ErrorCode::CapabilityNotSupported
    );
    assert!(caller.take_response_credential().unwrap().is_none());
    requests(&mut f, 0).await;
}

#[tokio::test]
async fn anchor_detail_invalid_responses_do_not_retry_or_refresh_web_sessions() {
    use crate::client::catalog::tests::response;
    for reply in [
        response(503, "application/json", "", b"{}"),
        response(
            302,
            "application/json",
            "Location: https://foreign.invalid/\r\n",
            b"{}",
        ),
        response(200, "text/html", "", b"<html>not metadata</html>"),
        response(200, "application/json", "", b"not json"),
        json_response(&json!({"code":200,"data":null})),
    ] {
        let mut f = setup(vec![reply]).await;
        assert!(f.provider.podcast("anchor:9240675", None).await.is_err());
        requests(&mut f, 1).await;
    }
}

#[test]
fn anchor_detail_rejects_other_albums_mini_apps_and_malformed_metadata() {
    for (key, value) in [
        ("albumid", json!("9240676")),
        ("albumid", json!("09240675")),
        ("isstar", json!(0)),
        ("content_type", json!(1)),
        ("name", json!(" ")),
        ("name", json!("bad\u{0000}")),
        ("mcnum", json!("-1")),
        ("mcnum", json!("01")),
        ("pnum", json!(1.5)),
        ("big_pic", json!("https://foreign.invalid/star/x")),
        ("big_pic", json!("https://img1.kuwo.cn/star/x?sid=private")),
        ("big_pic", json!("https://user@img1.kuwo.cn/star/x")),
    ] {
        let mut data = body();
        data[key] = value;
        assert!(
            parse(&serde_json::to_vec(&data).unwrap(), "9240675").is_err(),
            "{key}"
        );
    }
    assert!(parse(br#"{"code":200,"data":null}"#, "9240675").is_err());
    let mut data = body();
    data.as_object_mut().unwrap().remove("mcnum");
    assert_eq!(
        parse(&serde_json::to_vec(&data).unwrap(), "9240675")
            .unwrap()
            .episode_count,
        None
    );
}

#[tokio::test]
#[ignore = "real anonymous official metadata only; no account or media requests"]
async fn live_anchor_detail_reads_verified_native_album_metadata() {
    let client = KuwoClient::new(&KuwoConfig::default()).unwrap();
    let result = client.podcast("anchor:9240675", None).await.unwrap();
    assert_eq!(result.resource_ref.to_string(), "kuwo:anchor:9240675");
    assert!(!result.name.is_empty());
    assert!(result.episode_count.is_some_and(|n| n > 0));
    assert_eq!(result.purchased, None);
}

#[tokio::test]
#[ignore = "anonymous official catalogue contract probe; no account or business writes"]
async fn live_anchor_catalogue_uses_complete_anonymous_native_producer() {
    use flate2::read::ZlibDecoder;
    use sha1::{Digest, Sha1};
    use std::io::Read;

    let client = KuwoClient::new(&KuwoConfig::default()).unwrap();
    let device = KuwoNativeDeviceStore::default()
        .initialize(&client)
        .await
        .unwrap();
    let app_uid = device.app_uid();
    let query = format!(
        "user={}&android_id={}&prod=kwplayer_ar_12.2.2.0&corp=kuwo&newver=3&vipver=12.2.2.0&source=kwplayer_ar_12.2.2.0_newpcguanwangmobile.apk&p2p=1&q36={}&approval=false&loginUid=0&loginSid=0&appuid={app_uid}&allpay=0&notrace=0&oaid=&vipMode=0&type=tiaopin&uid={app_uid}&loginUid=0&apiv=2&net_type=UNKNOWN",
        device.device_user(),
        device.android_id(),
        "f2ce3c2ef68ddfd1b2bea7ed00001f314716",
    );
    let sealed = crate::client::native::seal_catalog_query(query.as_bytes()).unwrap();
    let target = format!("https://mobi.kuwo.cn/mobi.s?f=kuwo&q={sealed}");
    let response = client
        .http
        .get(target)
        .header(reqwest::header::ACCEPT, "application/octet-stream")
        .send()
        .await
        .unwrap();
    let status = response.status();
    assert_eq!(status.as_u16(), 200);
    let mime = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("missing")
        .to_owned();
    let mut response = response;
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.unwrap() {
        assert!(bytes.len().saturating_add(chunk.len()) <= 4 * 1024 * 1024);
        bytes.extend_from_slice(&chunk);
    }
    let response_hash = Sha1::digest(&bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let prefix_hex = bytes
        .iter()
        .take(64)
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    eprintln!(
        "anchor catalogue response: status={} mime={} bytes={} sha1={} prefix_hex={}",
        status.as_u16(),
        mime,
        bytes.len(),
        response_hash,
        prefix_hex
    );

    let frame_header_end = bytes
        .windows(2)
        .take(64)
        .position(|window| window == b"\r\n");
    if let Some(header_end) = frame_header_end.filter(|header_end| {
        let signature_len = header_end.saturating_sub(4);
        bytes.starts_with(b"sig=")
            && (1..=32).contains(&signature_len)
            && bytes[4..*header_end].iter().all(u8::is_ascii_digit)
    }) {
        let payload_offset = header_end + 2;
        assert!(bytes.len() >= payload_offset + 8);
        let compressed = u32::from_le_bytes(
            bytes[payload_offset..payload_offset + 4]
                .try_into()
                .unwrap(),
        ) as usize;
        let expanded = u32::from_le_bytes(
            bytes[payload_offset + 4..payload_offset + 8]
                .try_into()
                .unwrap(),
        ) as usize;
        assert!(compressed > 0 && expanded <= 256 * 1024);
        let end = payload_offset
            .checked_add(8)
            .and_then(|offset| offset.checked_add(compressed))
            .unwrap();
        assert_eq!(bytes.len(), end, "unexpected bytes outside the framed body");
        let mut decoder = ZlibDecoder::new(&bytes[payload_offset + 8..end]);
        let mut plain = Vec::with_capacity(expanded);
        decoder
            .by_ref()
            .take((expanded + 1) as u64)
            .read_to_end(&mut plain)
            .unwrap();
        assert_eq!(plain.len(), expanded);
        assert_eq!(decoder.total_in(), compressed as u64);
        assert!(plain.starts_with(b"<?xml version="));
        assert!(
            plain
                .windows(b"</root>".len())
                .any(|window| window == b"</root>")
        );
        if let Ok(path) = std::env::var("TUNEWEAVE_KUWO_ANCHOR_XML_DUMP") {
            std::fs::write(path, &plain).unwrap();
        }
        let section_count = plain
            .windows(b"<section".len())
            .filter(|window| *window == b"<section")
            .count();
        assert!(section_count > 0);
        let plain_hash = Sha1::digest(&plain)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        eprintln!(
            "anchor catalogue decoded: bytes={} sha1={} sections={}",
            plain.len(),
            plain_hash,
            section_count
        );
    } else {
        panic!(
            "unrecognized anchor catalogue response frame: status={} bytes={} sha1={}",
            status.as_u16(),
            bytes.len(),
            response_hash
        );
    }
}

#[tokio::test]
async fn native_category_page_maps_all_types_and_advertises_bounded_capabilities() {
    let body = json!({
        "code": 200,
        "data": {"list": [
            {"id":31,"name":"悬疑推理","type":"cat","icon":"https://kwimg1.kuwo.cn/star/upload/1/icon.png","desc":"推理"},
            {"id":8,"name":"畅销小说","type":"link","icon":"https://img4.kuwo.cn/star/upload/2/icon.png","linkUrl":"https://kweex.kuwo.cn/500005/web/KwCategoryPage.html?categoryId=2&from=diantai-more","desc":"热门"},
            {"id":21,"name":"音频直播","type":"xshow_index","icon":"https://img1.kuwo.cn/star/upload/3/icon.png","desc":"直播"}
        ]}
    });
    let mut fixture = setup(vec![json_response(&body)]).await;
    let taxonomy = fixture
        .provider
        .podcast_categories(&PodcastTaxonomyRequest::new(PodcastTaxonomyKind::All))
        .await
        .unwrap();
    assert_eq!(taxonomy.categories.len(), 3);
    assert_eq!(taxonomy.categories[0].id, "31");
    assert_eq!(taxonomy.categories[0].name, "悬疑推理");
    assert_eq!(
        taxonomy.categories[0].icon_url.as_deref(),
        Some("https://kwimg1.kuwo.cn/star/upload/1/icon.png")
    );
    assert_eq!(taxonomy.categories[0].extensions["native_type"], "cat");
    assert_eq!(
        taxonomy.categories[1].extensions["official_link"],
        "https://kweex.kuwo.cn/500005/web/KwCategoryPage.html?categoryId=2&from=diantai-more"
    );
    assert_eq!(taxonomy.extensions["backend"], "kuwo_fm_category_all");
    let capabilities = fixture.provider.capabilities();
    assert!(capabilities.contains(&Capability::PodcastList));
    assert!(capabilities.contains(&Capability::PodcastCategories));
    assert!(capabilities.contains(&Capability::PodcastCategoryRecommendations));
    let wire = requests(&mut fixture, 1).await.remove(0);
    assert!(wire.starts_with("GET /api/fm/category/all HTTP/1.1"));
    assert!(wire.to_lowercase().contains("accept: application/json"));
}

#[tokio::test]
async fn category_taxonomy_rejects_account_and_unproven_non_hot_modes_without_requests() {
    let mut fixture = setup(vec![]).await;
    let non_hot = fixture
        .provider
        .podcast_categories(&PodcastTaxonomyRequest::new(PodcastTaxonomyKind::NonHot))
        .await
        .unwrap_err();
    assert_eq!(non_hot.code, ErrorCode::CapabilityNotSupported);

    let mut selected = PodcastTaxonomyRequest::new(PodcastTaxonomyKind::All);
    selected.account = Some("listener-1".to_owned());
    let account = fixture
        .provider
        .podcast_categories(&selected)
        .await
        .unwrap_err();
    assert_eq!(account.code, ErrorCode::InvalidRequest);
    requests(&mut fixture, 0).await;
}

#[tokio::test]
async fn category_recommendations_are_hot_first_page_and_preserve_anchor_identity() {
    let categories = json!({
        "code":200,
        "data":{"list":[
            {"id":31,"name":"悬疑推理","type":"cat","icon":"https://kwimg1.kuwo.cn/star/upload/1/icon.png","desc":"推理"},
            {"id":8,"name":"畅销小说","type":"link","linkUrl":"https://kweex.kuwo.cn/500005/web/KwCategoryPage.html?categoryId=2&from=diantai-more"},
            {"id":21,"name":"音频直播","type":"xshow_index"}
        ]}
    });
    let album = json!({
        "id":"22879922","type":"album","isstar":1,"name":"镇龙棺",
        "desc":"第552集 人间值得（完结）",
        "pic":"https://img3.kuwo.cn/star/albumcover/300/84/30/3973856533.jpg",
        "artist":"漫步文学","artistId":"8280958","musicCount":552,
        "collectCount":11766,"playCount":7829904
    });
    let mut albums = Vec::new();
    for index in 0..9_u64 {
        let mut item = album.clone();
        if index > 0 {
            item["id"] = json!(22879922 + index);
            item["name"] = json!(format!("节目集 {index}"));
        }
        albums.push(item);
    }
    let page = json!({
        "code":200,
        "data":{"total":63,"pn":1,"rn":10,"list":albums}
    });
    let mut fixture = setup(vec![json_response(&categories), json_response(&page)]).await;
    let result = fixture
        .provider
        .podcast_category_recommendations(None)
        .await
        .unwrap();
    assert_eq!(result.sections.len(), 1);
    let section = &result.sections[0];
    assert_eq!(section.category.id, "31");
    assert_eq!(section.podcasts.len(), 9);
    let podcast = &section.podcasts[0];
    assert_eq!(podcast.resource_ref.to_string(), "kuwo:anchor:22879922");
    assert_eq!(podcast.episode_count, Some(552));
    assert_eq!(podcast.subscriber_count, Some(11766));
    assert_eq!(podcast.play_count, Some(7829904));
    assert_eq!(
        podcast
            .creator
            .as_ref()
            .unwrap()
            .resource_ref
            .as_ref()
            .unwrap()
            .id(),
        "8280958"
    );
    assert_eq!(podcast.description, "");
    assert_eq!(
        podcast.extensions["latest_episode_description"],
        "第552集 人间值得（完结）"
    );
    assert_eq!(section.extensions["total"], 63);
    assert_eq!(section.extensions["complete"], false);
    assert_eq!(result.extensions["first_page_only"], true);
    let wires = requests(&mut fixture, 2).await;
    assert!(wires[0].starts_with("GET /api/fm/category/all HTTP/1.1"));
    assert!(
        wires[1]
            .starts_with("GET /api/fm/category/list?parentId=cat.31&pn=1&rn=10&sort=1 HTTP/1.1")
    );
    assert!(
        !wires
            .iter()
            .any(|wire| wire.to_lowercase().contains("cookie:"))
    );
}

#[tokio::test]
async fn category_recommendations_reject_account_and_malformed_pages_without_partial_results() {
    let mut fixture = setup(vec![]).await;
    assert_eq!(
        fixture
            .provider
            .podcast_category_recommendations(Some("listener-1"))
            .await
            .unwrap_err()
            .code,
        ErrorCode::InvalidRequest
    );
    requests(&mut fixture, 0).await;

    for bad_page in [
        json!({"code":200,"data":{"total":63,"pn":2,"rn":10,"list":[]}}),
        json!({"code":200,"data":{"total":1,"pn":1,"rn":10,"list":[]}}),
        json!({"code":200,"data":{"total":1,"pn":1,"rn":10,"list":[{"id":22879922,"type":"music","isstar":1,"name":"not an album"}]}}),
    ] {
        let categories =
            json!({"code":200,"data":{"list":[{"id":31,"name":"悬疑推理","type":"cat"}]}});
        let mut fixture = setup(vec![json_response(&categories), json_response(&bad_page)]).await;
        assert!(
            fixture
                .provider
                .podcast_category_recommendations(None)
                .await
                .is_err()
        );
        requests(&mut fixture, 2).await;
    }
}

#[tokio::test]
async fn category_hot_catalog_translates_offset_to_validated_native_page() {
    let categories = json!({
        "code":200,
        "data":{"list":[{"id":31,"name":"悬疑推理","type":"cat"}]}
    });
    let page = json!({
        "code":200,
        "data":{"total":63,"pn":2,"rn":10,"list":[
            {"id":"23149313","type":"album","isstar":1,"name":"另一节目集","artist":"主播","artistId":"8280958","musicCount":45,"collectCount":6,"playCount":100}
        ]}
    });
    let mut request = PodcastListRequest::new(PodcastCatalog::CategoryHot, 10, 10);
    request.category_id = Some("31".to_owned());
    let mut fixture = setup(vec![json_response(&categories), json_response(&page)]).await;
    let result = fixture.provider.podcasts(&request).await.unwrap();
    assert_eq!(result.items.len(), 1);
    assert_eq!(
        result.items[0].resource_ref.to_string(),
        "kuwo:anchor:23149313"
    );
    assert_eq!(result.pagination.total, Some(63));
    assert_eq!(result.pagination.offset, 10);
    assert_eq!(result.pagination.next_offset, Some(20));
    assert!(result.pagination.has_more);
    assert_eq!(result.pagination.extensions["returned_count"], 1);
    let wires = requests(&mut fixture, 2).await;
    assert!(wires[0].starts_with("GET /api/fm/category/all HTTP/1.1"));
    assert!(
        wires[1]
            .starts_with("GET /api/fm/category/list?parentId=cat.31&pn=2&rn=10&sort=1 HTTP/1.1")
    );
}

#[tokio::test]
async fn category_newest_catalog_uses_native_recently_updated_sort() {
    let categories = json!({
        "code":200,
        "data":{"list":[{"id":31,"name":"悬疑推理","type":"cat"}]}
    });
    let page = json!({
        "code":200,
        "data":{"total":25,"pn":2,"rn":10,"list":[
            {"id":"23149313","type":"album","isstar":1,"name":"最近更新节目集","artist":"主播","artistId":"8280958","musicCount":45}
        ]}
    });
    let mut request = PodcastListRequest::new(PodcastCatalog::CategoryNewest, 10, 10);
    request.category_id = Some("31".to_owned());
    let mut fixture = setup(vec![json_response(&categories), json_response(&page)]).await;
    let result = fixture.provider.podcasts(&request).await.unwrap();
    assert_eq!(result.items.len(), 1);
    assert_eq!(
        result.items[0].resource_ref.to_string(),
        "kuwo:anchor:23149313"
    );
    assert_eq!(result.pagination.total, Some(25));
    assert_eq!(result.pagination.next_offset, Some(20));
    assert_eq!(result.pagination.extensions["catalog"], "category_newest");
    let wires = requests(&mut fixture, 2).await;
    assert!(
        wires[1]
            .starts_with("GET /api/fm/category/list?parentId=cat.31&pn=2&rn=10&sort=2 HTTP/1.1")
    );
}

#[tokio::test]
async fn category_catalog_keeps_successful_empty_middle_pages_navigable() {
    let categories = json!({
        "code":200,
        "data":{"list":[{"id":15,"name":"有声节目","type":"cat"}]}
    });
    let empty_middle_page = json!({
        "code":200,
        "msg":"success",
        "data":{"total":25,"pn":22,"rn":1,"list":[]}
    });
    let following_page = json!({
        "code":200,
        "data":{"total":25,"pn":23,"rn":1,"list":[
            {"id":"41616034","type":"album","isstar":1,"name":"下一页节目集"}
        ]}
    });
    let mut request = PodcastListRequest::new(PodcastCatalog::CategoryHot, 1, 21);
    request.category_id = Some("15".to_owned());
    let mut fixture = setup(vec![
        json_response(&categories),
        json_response(&empty_middle_page),
        json_response(&categories),
        json_response(&following_page),
    ])
    .await;

    let empty = fixture.provider.podcasts(&request).await.unwrap();
    assert!(empty.items.is_empty());
    assert_eq!(empty.pagination.total, Some(25));
    assert_eq!(empty.pagination.extensions["returned_count"], 0);
    assert!(empty.pagination.has_more);
    assert_eq!(empty.pagination.next_offset, Some(22));

    request.offset = empty.pagination.next_offset.unwrap();
    let next = fixture.provider.podcasts(&request).await.unwrap();
    assert_eq!(next.items.len(), 1);
    assert_eq!(
        next.items[0].resource_ref.to_string(),
        "kuwo:anchor:41616034"
    );
    assert_eq!(next.pagination.offset, 22);
    assert_eq!(next.pagination.next_offset, Some(23));

    let wires = requests(&mut fixture, 4).await;
    assert!(
        wires[1]
            .starts_with("GET /api/fm/category/list?parentId=cat.15&pn=22&rn=1&sort=1 HTTP/1.1")
    );
    assert!(
        wires[3]
            .starts_with("GET /api/fm/category/list?parentId=cat.15&pn=23&rn=1&sort=1 HTTP/1.1")
    );
}

#[tokio::test]
async fn category_hot_catalog_rejects_unaligned_or_unsupported_requests_before_network() {
    let mut fixture = setup(vec![]).await;
    let mut request = PodcastListRequest::new(PodcastCatalog::CategoryHot, 10, 1);
    request.category_id = Some("31".to_owned());
    assert_eq!(
        fixture.provider.podcasts(&request).await.unwrap_err().code,
        ErrorCode::InvalidRequest
    );

    let mut request = PodcastListRequest::new(PodcastCatalog::CategoryHot, 0, 0);
    request.category_id = Some("31".to_owned());
    assert_eq!(
        fixture.provider.podcasts(&request).await.unwrap_err().code,
        ErrorCode::InvalidRequest
    );

    let mut request = PodcastListRequest::new(PodcastCatalog::CategoryHot, 10, 0);
    request.category_id = Some("031".to_owned());
    assert_eq!(
        fixture.provider.podcasts(&request).await.unwrap_err().code,
        ErrorCode::InvalidRequest
    );

    requests(&mut fixture, 0).await;
}

#[tokio::test]
async fn category_hot_catalog_does_not_treat_navigation_links_as_categories() {
    let categories = json!({
        "code":200,
        "data":{"list":[{"id":8,"name":"畅销小说","type":"link","linkUrl":"https://kweex.kuwo.cn/500005/web/KwCategoryPage.html?categoryId=2&from=diantai-more"}]}
    });
    let mut request = PodcastListRequest::new(PodcastCatalog::CategoryHot, 10, 0);
    request.category_id = Some("8".to_owned());
    let mut fixture = setup(vec![json_response(&categories)]).await;
    assert_eq!(
        fixture.provider.podcasts(&request).await.unwrap_err().code,
        ErrorCode::ResourceNotFound
    );
    requests(&mut fixture, 1).await;
}

#[tokio::test]
#[ignore = "anonymous official category APIs only; no account or media requests"]
async fn live_anchor_category_taxonomy_and_hot_page_are_readable() {
    let client = KuwoClient::new(&KuwoConfig::default()).unwrap();
    let taxonomy = client
        .podcast_categories(&PodcastTaxonomyRequest::new(PodcastTaxonomyKind::All))
        .await
        .unwrap();
    let category = taxonomy
        .categories
        .iter()
        .find(|category| {
            category
                .extensions
                .get("native_type")
                .and_then(serde_json::Value::as_str)
                == Some("cat")
        })
        .unwrap();
    let mut request = PodcastListRequest::new(PodcastCatalog::CategoryHot, 10, 0);
    request.category_id = Some(category.id.clone());
    let page = client.podcasts(&request).await.unwrap();
    assert!(page.pagination.total.is_some_and(|total| total > 0));
    assert!(!page.items.is_empty());
    assert!(page.items.len() <= 10);
    assert_eq!(page.items[0].resource_ref.platform(), Platform::Kuwo);
    assert!(page.items[0].resource_ref.id().starts_with("anchor:"));

    let mut newest = PodcastListRequest::new(PodcastCatalog::CategoryNewest, 10, 0);
    newest.category_id = Some(category.id.clone());
    let page = client.podcasts(&newest).await.unwrap();
    assert_eq!(page.pagination.extensions["catalog"], "category_newest");
    assert!(page.pagination.total.is_some_and(|total| total > 0));
    assert!(!page.items.is_empty());
    assert!(page.items.len() <= 10);
}
