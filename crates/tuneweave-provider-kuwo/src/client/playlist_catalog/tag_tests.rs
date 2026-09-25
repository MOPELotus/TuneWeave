use super::*;
use crate::client::catalog::tests::{home, home_with, json_response, requests, response, setup};
use tuneweave_core::{
    Capability, MusicProvider, PageRequest, PlaylistCatalogTaxonomyRequest, PlaylistPlayableItem,
};

// Narrow synthetic field fixtures. These are not captured platform responses.
fn taxonomy_body() -> serde_json::Value {
    json!({"code":200,"data":[
        {"id":"5","name":"Group &amp; One","type":"list","mdigest":"5","data":[
            {"id":"2189","name":"Tag &amp; First","digest":"10000","extend":"|HOT","isnew":"0","opaque":"never-export"},
            {"id":77,"name":"Another dynamic tag","digest":10000}
        ]},
        {"id":"700","name":"Empty dynamic group","type":"list","mdigest":5,"data":[]},
        {"id":"999","name":"Trailing navigation","type":"list","mdigest":5,"data":[
            {"id":"9999","name":"Not a displayed tag","digest":10000}
        ]}
    ],"opaque":"never-export"})
}
fn request(limit: u32, offset: u32) -> PlaylistCatalogRequest {
    let mut r = PlaylistCatalogRequest::new(PlaylistCatalogKind::Tag, limit, offset);
    r.tag_id = Some("2189".into());
    r
}
fn taxonomy_request() -> PlaylistCatalogTaxonomyRequest {
    PlaylistCatalogTaxonomyRequest { account: None }
}
fn parse_taxonomy(v: &serde_json::Value) -> Result<tuneweave_core::PlaylistCatalogTaxonomy> {
    taxonomy::parse(&serde_json::to_vec(v).unwrap())
}
fn wire(raw: &str, page: Option<u32>, cookie: &str) {
    let url = Url::parse(&format!(
        "https://www.kuwo.cn{}",
        raw.split_whitespace().nth(1).unwrap()
    ))
    .unwrap();
    let query = url.query_pairs().collect::<BTreeMap<_, _>>();
    assert_eq!(query["loginUid"], "0");
    assert_eq!(query["loginSid"], "0");
    assert_eq!(query["httpsStatus"], "1");
    assert_eq!(query["plat"], "web_www");
    assert_eq!(query["from"], "");
    assert_eq!(query["reqId"].len(), 36);
    assert!(!query.contains_key("order"));
    if let Some(page) = page {
        assert_eq!(url.path(), "/api/www/classify/playlist/getTagPlayList");
        assert_eq!(query.len(), 9);
        assert_eq!(query["id"], "2189");
        assert_eq!(query["pn"], page.to_string());
        assert_eq!(query["rn"], "20");
    } else {
        assert_eq!(url.path(), "/api/www/playlist/getTagList");
        assert_eq!(query.len(), 6);
        assert!(!query.contains_key("id") && !query.contains_key("pn"));
    }
    assert!(raw.contains("referer: https://www.kuwo.cn/playlists\r\n"));
    assert!(raw.contains(&format!("cookie: {WEB_SESSION_COOKIE}={cookie}\r\n")));
    let secret = raw
        .lines()
        .find_map(|line| line.strip_prefix("secret: "))
        .unwrap();
    let nonce = u64::from_str_radix(&secret[secret.len() - 8..], 16).unwrap();
    assert_eq!(secret, web_secret_for_nonce(cookie, nonce).unwrap());
    for private in ["authorization:", "usersid", "never-export"] {
        assert!(!raw.contains(private));
    }
}

#[tokio::test]
async fn playlist_catalogue_tag_taxonomy_is_dynamic_ordered_and_preserves_visible_empty_groups() {
    for sdk in [false, true] {
        let mut f = setup(vec![home(), json_response(&taxonomy_body())]).await;
        assert!(f.provider.supports(Capability::PlaylistCatalog));
        let result = if sdk {
            f.client
                .playlist_catalog_taxonomy(&taxonomy_request())
                .await
        } else {
            f.provider
                .playlist_catalog_taxonomy(&taxonomy_request())
                .await
        }
        .unwrap();
        assert_eq!(result.platform, Platform::Kuwo);
        assert_eq!(
            result
                .groups
                .iter()
                .map(|g| g.id.as_str())
                .collect::<Vec<_>>(),
            ["5", "700"]
        );
        assert_eq!(result.groups[0].name, "Group & One");
        assert_eq!(
            result.groups[0]
                .tags
                .iter()
                .map(|t| t.id.as_str())
                .collect::<Vec<_>>(),
            ["2189", "77"]
        );
        assert_eq!(result.groups[0].tags[0].name, "Tag & First");
        assert!(result.groups[1].tags.is_empty());
        assert_eq!(result.extensions["omitted_trailing_groups"], 1);
        let text = serde_json::to_string(&result).unwrap();
        for omitted in [
            "never-export",
            "|HOT",
            "9999",
            "Trailing navigation",
            "snapshot_id",
        ] {
            assert!(!text.contains(omitted));
        }
        wire(
            &requests(&mut f, 2).await[1],
            None,
            "anonymousCatalogueCookie123456",
        );
    }
}

#[test]
fn playlist_catalogue_tag_taxonomy_validates_full_shape_ids_kinds_and_bounds() {
    let empty = parse_taxonomy(&json!({"code":200,"data":[]})).unwrap();
    assert!(empty.groups.is_empty());
    assert_eq!(empty.extensions["omitted_trailing_groups"], 0);
    let mut invalids = vec![
        json!({"code":200}),
        json!({"code":200,"data":null}),
        json!({"code":500,"data":[]}),
    ];
    for (path, value) in [
        ("/data/0/id", json!("01")),
        ("/data/0/name", json!("")),
        ("/data/0/mdigest", json!(8)),
        ("/data/0/type", json!("radio")),
        ("/data/0/data", json!(null)),
        ("/data/0/data/0/digest", json!(8)),
        ("/data/0/data/0/name", json!("")),
        ("/data/0/data/0/id", json!(0)),
        ("/data/0/data/1/id", json!("2189")),
        ("/data/1/id", json!("5")),
    ] {
        let mut v = taxonomy_body();
        *v.pointer_mut(path).unwrap() = value;
        invalids.push(v);
    }
    let mut missing = taxonomy_body();
    missing["data"][0].as_object_mut().unwrap().remove("data");
    invalids.push(missing);
    let mut cross = taxonomy_body();
    cross["data"][1]["data"] = json!([{"id":"2189","name":"Conflicting tag","digest":10000}]);
    invalids.push(cross);
    let many = taxonomy_body()["data"][0].clone();
    invalids.push(json!({"code":200,"data":vec![many;33]}));
    let mut too_many = taxonomy_body();
    too_many["data"][0]["data"] = json!(
        (1..=1025)
            .map(|id| json!({"id":id,"name":"Tag","digest":10000}))
            .collect::<Vec<_>>()
    );
    invalids.push(too_many);
    for value in invalids {
        assert!(parse_taxonomy(&value).is_err());
    }
}

#[tokio::test]
async fn playlist_catalogue_tag_sdk_and_provider_use_fresh_taxonomy_then_six_signed_pages() {
    for sdk in [false, true] {
        let mut replies = vec![home(), json_response(&taxonomy_body())];
        replies.extend((1..=6).map(|page| json_response(&tests::body(page, 150))));
        let mut f = setup(replies).await;
        let result = if sdk {
            f.client.playlist_catalog(&request(100, 19)).await
        } else {
            f.provider.playlist_catalog(&request(100, 19)).await
        }
        .unwrap();
        assert_eq!(
            result
                .items
                .iter()
                .map(|p| p.id.clone())
                .collect::<Vec<_>>(),
            (20..120).map(|id| id.to_string()).collect::<Vec<_>>()
        );
        assert_eq!(result.pagination.next_offset, Some(119));
        assert_eq!(result.pagination.total, Some(150));
        assert_eq!(result.pagination.extensions["catalog"], "tag");
        assert_eq!(result.pagination.extensions["tag_id"], "2189");
        assert_eq!(result.pagination.extensions["upstream_pages_fetched"], 6);
        assert!(!result.pagination.extensions.contains_key("upstream_order"));
        assert!(
            result
                .items
                .iter()
                .all(|p| p.tags.is_empty() && p.subscribed.is_none())
        );
        let calls = requests(&mut f, 8).await;
        wire(&calls[1], None, "anonymousCatalogueCookie123456");
        for (i, call) in calls[2..].iter().enumerate() {
            wire(call, Some(i as u32 + 1), "anonymousCatalogueCookie123456");
        }
    }
}

#[tokio::test]
async fn playlist_catalogue_tag_tails_and_out_of_range_preserve_upstream_total() {
    for (offset, total, count) in [(10489, 10490, 1), (10500, 10490, 0), (0, 0, 0)] {
        let page = offset / 20 + 1;
        let mut f = setup(vec![
            home(),
            json_response(&taxonomy_body()),
            json_response(&tests::body(page, total)),
        ])
        .await;
        let result = f
            .client
            .playlist_catalog(&request(100, offset))
            .await
            .unwrap();
        assert_eq!(result.items.len(), count);
        assert_eq!(result.pagination.total, Some(total));
        assert!(!result.pagination.has_more && result.pagination.next_offset.is_none());
        wire(
            &requests(&mut f, 3).await[2],
            Some(page),
            "anonymousCatalogueCookie123456",
        );
    }
}

#[tokio::test]
async fn playlist_catalogue_tag_is_revalidated_on_every_call_and_removed_tags_do_not_fetch_pages() {
    let mut changed = taxonomy_body();
    changed["data"][0]["data"] = json!([]);
    let mut f = setup(vec![
        home(),
        json_response(&taxonomy_body()),
        json_response(&tests::body(1, 1)),
        json_response(&changed),
    ])
    .await;
    assert_eq!(
        f.provider
            .playlist_catalog(&request(1, 0))
            .await
            .unwrap()
            .items
            .len(),
        1
    );
    assert_eq!(
        f.provider
            .playlist_catalog(&request(1, 0))
            .await
            .unwrap_err()
            .code,
        ErrorCode::InvalidRequest
    );
    let calls = requests(&mut f, 4).await;
    wire(&calls[1], None, "anonymousCatalogueCookie123456");
    wire(&calls[3], None, "anonymousCatalogueCookie123456");
    for id in ["9999", "5", "99999"] {
        let mut f = setup(vec![home(), json_response(&taxonomy_body())]).await;
        let mut r = request(1, 0);
        r.tag_id = Some(id.into());
        assert_eq!(
            f.client.playlist_catalog(&r).await.unwrap_err().code,
            ErrorCode::InvalidRequest
        );
        requests(&mut f, 2).await;
    }
}

#[tokio::test]
async fn playlist_catalogue_tag_input_matrix_and_credentials_fail_before_io() {
    let mut f = setup(vec![]).await;
    let mut invalids = Vec::new();
    for kind in [PlaylistCatalogKind::Latest, PlaylistCatalogKind::Hot] {
        let mut r = request(1, 0);
        r.catalog = kind;
        invalids.push(r);
    }
    for id in [
        None,
        Some(""),
        Some("0"),
        Some("01"),
        Some(" 2189"),
        Some("2189&pn=2"),
    ] {
        let mut r = request(1, 0);
        r.tag_id = id.map(str::to_owned);
        invalids.push(r);
    }
    for account in ["", "default", "named"] {
        let mut r = request(1, 0);
        r.account = Some(account.into());
        invalids.push(r);
        let r = PlaylistCatalogTaxonomyRequest {
            account: Some(account.into()),
        };
        assert_eq!(
            f.client
                .playlist_catalog_taxonomy(&r)
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
        assert_eq!(
            f.provider
                .playlist_catalog_taxonomy(&r)
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
    }
    for r in invalids {
        assert_eq!(
            f.client.playlist_catalog(&r).await.unwrap_err().code,
            ErrorCode::InvalidRequest
        );
        assert_eq!(
            f.provider.playlist_catalog(&r).await.unwrap_err().code,
            ErrorCode::InvalidRequest
        );
    }
    let credential = crate::client::native::tests::credential_fixture("42", "private-tag-session")
        .caller()
        .unwrap();
    let caller = f.provider.with_caller_credential(&credential).unwrap();
    assert_eq!(
        caller
            .playlist_catalog_taxonomy(&taxonomy_request())
            .await
            .unwrap_err()
            .code,
        ErrorCode::CapabilityNotSupported
    );
    assert_eq!(
        caller
            .playlist_catalog(&request(1, 0))
            .await
            .unwrap_err()
            .code,
        ErrorCode::CapabilityNotSupported
    );
    assert!(caller.take_response_credential().unwrap().is_none());
    assert!(f.seen.try_recv().is_err());
}

#[tokio::test]
async fn playlist_catalogue_tag_taxonomy_and_pages_refresh_once_without_losing_selection() {
    let reject = json_response(&json!({"success":false,"message":"The request is illegal!"}));
    for taxonomy_rejection in [false, true] {
        let mut replies = vec![home()];
        if taxonomy_rejection {
            replies.extend([reject.clone(), home_with("refreshedAnonymousCatalogue456")]);
        }
        replies.push(json_response(&taxonomy_body()));
        if !taxonomy_rejection {
            replies.extend([reject.clone(), home_with("refreshedAnonymousCatalogue456")]);
        }
        replies.push(json_response(&tests::body(1, 1)));
        let mut f = setup(replies).await;
        assert_eq!(
            f.client
                .playlist_catalog(&request(1, 0))
                .await
                .unwrap()
                .items
                .len(),
            1
        );
        let calls = requests(&mut f, 5).await;
        wire(&calls[4], Some(1), "refreshedAnonymousCatalogue456");
    }
    let mut f = setup(vec![
        home(),
        reject.clone(),
        home_with("refreshedAnonymousCatalogue456"),
        reject,
    ])
    .await;
    assert!(
        f.client
            .playlist_catalog_taxonomy(&taxonomy_request())
            .await
            .is_err()
    );
    requests(&mut f, 4).await;
}

#[tokio::test]
async fn playlist_catalogue_tag_page_integrity_errors_never_deliver_a_partial_window() {
    for mode in 0..4 {
        let mut second = tests::body(2, 40);
        match mode {
            0 => second["data"]["total"] = json!(41),
            1 => second["data"]["data"][19]["id"] = json!("1"),
            2 => second["data"]["rn"] = json!(30),
            _ => second["data"]["data"][19]["digest"] = json!(13),
        }
        let mut f = setup(vec![
            home(),
            json_response(&taxonomy_body()),
            json_response(&tests::body(1, 40)),
            json_response(&second),
        ])
        .await;
        assert_eq!(
            f.client
                .playlist_catalog(&request(2, 19))
                .await
                .unwrap_err()
                .code,
            ErrorCode::UpstreamError
        );
        requests(&mut f, 4).await;
    }
}

#[tokio::test]
async fn playlist_catalogue_tag_taxonomy_errors_mime_redirect_and_size_are_bounded() {
    for reply in [
        json_response(&json!({"code":500,"data":null,"message":"secret-never-export"})),
        response(200, "text/html", "", b"{}"),
        response(
            200,
            "application/json",
            "",
            &vec![b' '; 2 * 1024 * 1024 + 1],
        ),
        response(
            302,
            "application/json",
            "Location: https://example.test/private\r\n",
            b"{}",
        ),
    ] {
        let mut f = setup(vec![home(), reply]).await;
        let error = f.client.playlist_catalog(&request(1, 0)).await.unwrap_err();
        assert!(!format!("{error:?}").contains("secret-never-export"));
        requests(&mut f, 2).await;
    }
}

#[tokio::test]
async fn playlist_catalogue_tag_reference_uses_existing_detail_and_uni_source_identity() {
    let detail = json!({"code":200,"data":{"id":"1","name":"Synthetic playlist","isOfficial":0,"total":3,
        "musicList":[{"musicrid":"MUSIC_11","rid":11,"name":"First","artist":"Singer","duration":12,"online":1},
            {"musicrid":"MUSIC_22","rid":22,"name":"Second","artist":"Singer","duration":13,"online":1},
            {"musicrid":"MUSIC_11","rid":11,"name":"First","artist":"Singer","duration":12,"online":1}]}});
    let mut metadata = detail.clone();
    metadata["data"]["musicList"]
        .as_array_mut()
        .unwrap()
        .truncate(1);
    let mut f = setup(vec![
        home(),
        json_response(&taxonomy_body()),
        json_response(&tests::body(1, 1)),
        json_response(&metadata),
        json_response(&detail),
    ])
    .await;
    let catalogue = f.provider.playlist_catalog(&request(1, 0)).await.unwrap();
    let reference = &catalogue.items[0].resource_ref;
    assert_eq!(
        f.provider
            .playlist_source(reference.id(), "playlist", None)
            .await
            .unwrap()
            .resource_ref,
        *reference
    );
    let page = f
        .provider
        .playlist_source_items(reference.id(), "playlist", &PageRequest::new(20, 0))
        .await
        .unwrap();
    assert_eq!(
        page.items
            .iter()
            .map(|item| match item {
                PlaylistPlayableItem::Track(track) => track.id.as_str(),
                _ => panic!("wrong type"),
            })
            .collect::<Vec<_>>(),
        ["11", "22", "11"]
    );
    let calls = requests(&mut f, 5).await;
    for raw in &calls[3..] {
        assert!(raw.contains("/api/www/playlist/playListInfo?pid=1&"));
    }
}
