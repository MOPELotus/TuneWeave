use super::*;
use crate::client::catalog::tests::{home, home_with, json_response, requests, response, setup};
use tuneweave_core::{Capability, MusicProvider, PageRequest, PlaylistPlayableItem};

fn row(id: u64) -> serde_json::Value {
    json!({"id":id.to_string(),"name":format!("Playlist {id} &amp; Co"),"digest":"8","radio_id":"",
        "uid":"42","uname":"Listener &amp; Co","total":"3","listencnt":"430","favorcnt":"0",
        "img":"https://img1.kuwo.cn/star/userpl2015/a.jpg","desc":"Line one\nLine two","info":"Other",
        "lossless_mark":"1","opaque":{"sid":"never-export"}})
}
pub(super) fn body(page: u32, total: u64) -> serde_json::Value {
    let start = u64::from(page - 1) * 20;
    let rows = (start..(start + 20).min(total))
        .map(|id| row(id + 1))
        .collect::<Vec<_>>();
    json!({"code":200,"data":{"total":total,"pn":page,"rn":20,"data":rows}})
}
fn request(limit: u32, offset: u32) -> PlaylistCatalogRequest {
    PlaylistCatalogRequest::new(PlaylistCatalogKind::Latest, limit, offset)
}
fn wire(raw: &str, page: u32, order: &str, cookie: &str) -> String {
    let url = Url::parse(&format!(
        "https://www.kuwo.cn{}",
        raw.split_whitespace().nth(1).unwrap()
    ))
    .unwrap();
    assert_eq!(url.path(), "/api/www/classify/playlist/getRcmPlayList");
    let query = url.query_pairs().collect::<BTreeMap<_, _>>();
    assert_eq!(query.len(), 9);
    for (key, value) in [
        ("rn", "20"),
        ("loginUid", "0"),
        ("loginSid", "0"),
        ("order", order),
        ("httpsStatus", "1"),
        ("plat", "web_www"),
        ("from", ""),
    ] {
        assert_eq!(query[key], value);
    }
    assert_eq!(query["pn"], page.to_string());
    assert_eq!(query["reqId"].len(), 36);
    assert!(raw.contains("referer: https://www.kuwo.cn/playlists\r\n"));
    assert!(raw.contains(&format!("cookie: {WEB_SESSION_COOKIE}={cookie}\r\n")));
    let secret = raw
        .lines()
        .find_map(|line| line.strip_prefix("secret: "))
        .unwrap();
    let nonce = u64::from_str_radix(&secret[secret.len() - 8..], 16).unwrap();
    assert_eq!(secret, web_secret_for_nonce(cookie, nonce).unwrap());
    assert!(!raw.contains("authorization:") && !raw.contains("usersid"));
    query["reqId"].to_string()
}

#[tokio::test]
async fn playlist_catalogue_sdk_provider_and_orders_use_anonymous_signed_contract() {
    for sdk in [false, true] {
        for (catalog, order) in [
            (PlaylistCatalogKind::Latest, "new"),
            (PlaylistCatalogKind::Hot, "hot"),
        ] {
            let mut f = setup(vec![home(), json_response(&body(1, 1))]).await;
            assert!(f.provider.supports(Capability::PlaylistCatalog));
            let request = PlaylistCatalogRequest::new(catalog, 20, 0);
            let result = if sdk {
                f.client.playlist_catalog(&request).await
            } else {
                f.provider.playlist_catalog(&request).await
            }
            .unwrap();
            let item = &result.items[0];
            assert_eq!(item.resource_ref.to_string(), "kuwo:1");
            assert_eq!(item.name, "Playlist 1 & Co");
            assert_eq!(item.description, "Line one\nLine two");
            assert_eq!(item.creator.as_ref().unwrap().name, "Listener & Co");
            assert!(item.creator.as_ref().unwrap().resource_ref.is_none());
            assert_eq!(item.extensions["creator_uid"], "42");
            assert_eq!(item.extensions["favorite_count"], 0);
            assert_eq!(item.track_count, Some(3));
            assert!(
                item.subscribed.is_none() && item.created_at.is_none() && item.updated_at.is_none()
            );
            assert_eq!(result.pagination.total, Some(1));
            assert!(!result.pagination.has_more);
            let encoded = serde_json::to_string(item).unwrap();
            for absent in [
                "never-export",
                "lossless_mark",
                "actual_quality",
                "official",
            ] {
                assert!(!encoded.contains(absent));
            }
            wire(
                &requests(&mut f, 2).await[1],
                1,
                order,
                "anonymousCatalogueCookie123456",
            );
        }
    }
}

#[tokio::test]
async fn playlist_catalogue_arbitrary_maximum_window_uses_six_pages_in_order() {
    let mut replies = vec![home()];
    replies.extend((1..=6).map(|page| json_response(&body(page, 150))));
    let mut f = setup(replies).await;
    let result = f
        .provider
        .playlist_catalog(&request(100, 19))
        .await
        .unwrap();
    assert_eq!(
        result
            .items
            .iter()
            .map(|item| item.id.clone())
            .collect::<Vec<_>>(),
        (20..120).map(|id| id.to_string()).collect::<Vec<_>>()
    );
    assert_eq!(result.pagination.total, Some(150));
    assert_eq!(result.pagination.next_offset, Some(119));
    assert_eq!(result.pagination.extensions["upstream_pages_fetched"], 6);
    let wires = requests(&mut f, 7).await;
    let ids = wires[1..]
        .iter()
        .enumerate()
        .map(|(i, raw)| wire(raw, i as u32 + 1, "new", "anonymousCatalogueCookie123456"))
        .collect::<BTreeSet<_>>();
    assert_eq!(ids.len(), 6);
}

#[tokio::test]
async fn playlist_catalogue_tails_far_offsets_and_explicit_empty_preserve_totals() {
    for (offset, total, count) in [
        (0, 0, 0),
        (1749, 1754, 5),
        (1760, 1754, 0),
        (1_999_980, 9080, 0),
    ] {
        let page = offset / 20 + 1;
        let mut f = setup(vec![home(), json_response(&body(page, total))]).await;
        let result = f
            .client
            .playlist_catalog(&request(100, offset))
            .await
            .unwrap();
        assert_eq!(result.items.len(), count);
        assert_eq!(result.pagination.total, Some(total));
        assert!(!result.pagination.has_more);
        assert!(result.pagination.next_offset.is_none());
        wire(
            &requests(&mut f, 2).await[1],
            page,
            "new",
            "anonymousCatalogueCookie123456",
        );
    }
}

#[test]
fn playlist_catalogue_missing_counts_kind_identity_and_arrays_do_not_become_success() {
    let good = body(1, 1);
    let mut bad = vec![
        json!({"code":200}),
        json!({"code":200,"data":null}),
        json!({"code":500,"data":good["data"]}),
    ];
    for field in ["total", "pn", "rn", "data"] {
        let mut value = good.clone();
        value["data"].as_object_mut().unwrap().remove(field);
        bad.push(value);
    }
    for (path, value) in [
        ("/data/pn", json!(0)),
        ("/data/rn", json!(30)),
        ("/data/total", json!(2)),
        ("/data/data", json!([])),
        ("/data/data/0/id", json!("01")),
        ("/data/data/0/id", json!(0)),
        ("/data/data/0/digest", json!(13)),
        ("/data/data/0/radio_id", json!("7")),
        ("/data/data/0/name", json!("")),
        ("/data/data/0/total", json!("unknown")),
    ] {
        let mut v = good.clone();
        *v.pointer_mut(path).unwrap() = value;
        bad.push(v);
    }
    // Invalid metadata even outside the requested window must not be silently filtered.
    let mut tail = body(1, 20);
    tail["data"]["data"][19]["id"] = json!("bad");
    bad.push(tail);
    for value in bad {
        assert!(
            parse(&serde_json::to_vec(&value).unwrap(), 1).is_err(),
            "{value}"
        );
    }
}

#[test]
fn playlist_catalogue_unknown_metadata_stays_unknown_and_untrusted_images_are_omitted() {
    let mut value = body(1, 1);
    for key in [
        "uid",
        "uname",
        "total",
        "listencnt",
        "favorcnt",
        "desc",
        "info",
    ] {
        value["data"]["data"][0]
            .as_object_mut()
            .unwrap()
            .remove(key);
    }
    value["data"]["data"][0]["img"] = json!("https://example.test/image.jpg");
    let item = parse(&serde_json::to_vec(&value).unwrap(), 1)
        .unwrap()
        .items
        .remove(0);
    assert!(item.creator.is_none() && item.track_count.is_none() && item.cover_url.is_none());
    assert!(
        !item.extensions.contains_key("creator_uid")
            && !item.extensions.contains_key("listen_count")
    );
    assert!(item.description.is_empty());
}

#[tokio::test]
async fn playlist_catalogue_cross_page_drift_or_duplicates_abort_the_whole_window() {
    for mode in 0..3 {
        let mut second = body(2, 40);
        if mode == 0 {
            second["data"]["total"] = json!(41);
        } else {
            second["data"]["data"][if mode == 1 { 0 } else { 19 }]["id"] = json!("1");
        }
        let mut f = setup(vec![
            home(),
            json_response(&body(1, 40)),
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
        requests(&mut f, 3).await;
    }
    let mut value = body(1, 20);
    value["data"]["data"][19]["id"] = json!("1");
    let mut f = setup(vec![home(), json_response(&value)]).await;
    assert!(f.client.playlist_catalog(&request(1, 0)).await.is_err());
    requests(&mut f, 2).await;
}

#[tokio::test]
async fn playlist_catalogue_session_rejection_refreshes_only_once_per_page() {
    for status_rejection in [false, true] {
        for terminal in [false, true] {
            let reject = if status_rejection {
                response(403, "application/json", "", b"{}")
            } else {
                json_response(&json!({"success":false,"message":"The request is illegal!"}))
            };
            let last = if terminal {
                reject.clone()
            } else {
                json_response(&body(1, 1))
            };
            let mut f = setup(vec![
                home(),
                reject,
                home_with("refreshedAnonymousCatalogue456"),
                last,
            ])
            .await;
            let result = f.client.playlist_catalog(&request(1, 0)).await;
            assert_eq!(result.is_err(), terminal);
            let calls = requests(&mut f, 4).await;
            wire(&calls[1], 1, "new", "anonymousCatalogueCookie123456");
            wire(&calls[3], 1, "new", "refreshedAnonymousCatalogue456");
        }
    }
}

#[tokio::test]
async fn playlist_catalogue_wrong_mime_oversize_redirect_and_business_errors_are_bounded() {
    for reply in [
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
        json_response(&json!({"code":500,"data":null,"message":"secret-never-export"})),
    ] {
        let mut f = setup(vec![home(), reply]).await;
        let error = f.client.playlist_catalog(&request(1, 0)).await.unwrap_err();
        assert!(!format!("{error:?}").contains("secret-never-export"));
        requests(&mut f, 2).await;
    }
}

#[tokio::test]
async fn playlist_catalogue_invalid_input_and_account_aliases_reject_before_io() {
    let mut f = setup(vec![]).await;
    let mut invalids = vec![request(0, 0), request(101, 0), request(1, u32::MAX)];
    for account in ["", "default", "named"] {
        let mut r = request(1, 0);
        r.account = Some(account.into());
        invalids.push(r);
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
    assert!(f.seen.try_recv().is_err());
}

#[tokio::test]
async fn playlist_catalogue_caller_scope_rejects_without_io_or_credential_rotation() {
    let mut f = setup(vec![]).await;
    let credential =
        crate::client::native::tests::credential_fixture("42", "catalogue-private-session")
            .caller()
            .unwrap();
    let caller = f.provider.with_caller_credential(&credential).unwrap();
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
async fn playlist_catalogue_reference_reads_existing_detail_and_uni_source_with_duplicates() {
    let detail = json!({"code":200,"data":{"id":"1","name":"Playlist 1 & Co","isOfficial":0,"total":3,
    "musicList":[
        {"musicrid":"MUSIC_11","rid":11,"name":"First","artist":"Singer","duration":12,"online":1},
        {"musicrid":"MUSIC_22","rid":22,"name":"Second","artist":"Singer","duration":13,"online":1},
        {"musicrid":"MUSIC_11","rid":11,"name":"First","artist":"Singer","duration":12,"online":1}
    ]}});
    let mut metadata = detail.clone();
    metadata["data"]["musicList"]
        .as_array_mut()
        .unwrap()
        .truncate(1);
    let mut f = setup(vec![
        home(),
        json_response(&body(1, 1)),
        json_response(&metadata),
        json_response(&detail),
    ])
    .await;
    let catalogue = f.provider.playlist_catalog(&request(1, 0)).await.unwrap();
    let reference = &catalogue.items[0].resource_ref;
    let source = f
        .provider
        .playlist_source(reference.id(), "playlist", None)
        .await
        .unwrap();
    assert_eq!(source.resource_ref, *reference);
    assert_eq!(source.track_count, Some(3));
    let rows = f
        .provider
        .playlist_source_items(reference.id(), "playlist", &PageRequest::new(20, 0))
        .await
        .unwrap();
    let ids = rows
        .items
        .iter()
        .map(|item| match item {
            PlaylistPlayableItem::Track(track) => track.id.as_str(),
            _ => panic!("wrong kind"),
        })
        .collect::<Vec<_>>();
    assert_eq!(ids, vec!["11", "22", "11"]);
    let calls = requests(&mut f, 4).await;
    for call in &calls[2..] {
        assert!(call.contains("/api/www/playlist/playListInfo?pid=1&"));
    }
}
