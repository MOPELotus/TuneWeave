use super::*;
use crate::client::catalog::tests::{home, home_with, json_response, requests, response, setup};
use std::collections::BTreeMap;
use tuneweave_core::{Capability, MusicProvider, MusicVideoOrder, VideoDetailRequest};

fn row(id: u64, online: u8) -> serde_json::Value {
    json!({"id":id,"name":format!("Video {id} &amp; Live"),"artist":"Singer","artistid":336,
        "duration":269,"mvPlayCnt":1234,"online":online,
        "pic":"https://img1.kuwo.cn/wmvpic/324/a.jpg","opaque":"never-export"})
}

fn body(total: u64, rows: Vec<serde_json::Value>) -> serde_json::Value {
    json!({"code":200,"data":{"total":total,"mvlist":rows}})
}

fn request(limit: u32, offset: u32) -> MusicVideoListRequest {
    let mut request = MusicVideoListRequest::new(MusicVideoCatalog::Group, limit, offset);
    request.group_id = Some(GROUPS[0].0.into());
    request
}

fn wire(raw: &str, id: &str, page: u32, cookie: &str) -> String {
    let url = Url::parse(&format!(
        "https://www.kuwo.cn{}",
        raw.split_whitespace().nth(1).unwrap()
    ))
    .unwrap();
    assert_eq!(url.path(), "/api/www/music/mvList");
    let query = url.query_pairs().collect::<BTreeMap<_, _>>();
    assert_eq!(query.len(), 7);
    assert_eq!(query["pid"], id);
    assert_eq!(query["pn"], page.to_string());
    assert_eq!(query["rn"], "20");
    assert_eq!(query["httpsStatus"], "1");
    assert_eq!(query["plat"], "web_www");
    assert_eq!(query["from"], "");
    assert_eq!(query["reqId"].len(), 36);
    assert!(raw.contains("referer: https://www.kuwo.cn/mvs\r\n"));
    assert!(raw.contains(&format!("cookie: {WEB_SESSION_COOKIE}={cookie}\r\n")));
    let secret = raw
        .lines()
        .find_map(|line| line.strip_prefix("secret: "))
        .unwrap();
    let nonce = u64::from_str_radix(&secret[secret.len() - 8..], 16).unwrap();
    assert_eq!(secret, web_secret_for_nonce(cookie, nonce).unwrap());
    for private in ["authorization:", "loginUid", "loginSid", "usersid", "uname"] {
        assert!(!raw.contains(private));
    }
    query["reqId"].to_string()
}

#[tokio::test]
async fn mv_catalogue_taxonomy_sdk_and_provider_preserve_all_nine_official_groups() {
    for sdk in [false, true] {
        let mut f = setup(vec![]).await;
        let request = VideoTaxonomyRequest::new(VideoTaxonomyKind::Groups, 20, 0);
        let result = if sdk {
            f.client.video_taxonomy(&request).await
        } else {
            f.provider.video_taxonomy(&request).await
        }
        .unwrap();
        assert!(
            f.provider
                .capabilities()
                .contains(&Capability::VideoCatalog)
        );
        assert!(
            f.provider
                .capabilities()
                .contains(&Capability::VideoTaxonomy)
        );
        assert_eq!(
            result
                .items
                .iter()
                .map(|item| (item.id.as_str(), item.name.as_str()))
                .collect::<Vec<_>>(),
            GROUPS
        );
        assert!(
            result
                .items
                .iter()
                .all(|item| item.related_video_type.as_deref() == Some("mv")
                    && item.selected.is_none()
                    && item.url.is_none())
        );
        assert_eq!(result.pagination.total, Some(9));
        assert!(!result.pagination.has_more);
        assert_eq!(
            result.pagination.extensions["source"],
            "fixed_official_web_tags"
        );
        assert!(f.seen.try_recv().is_err());
    }
}

#[tokio::test]
async fn mv_catalogue_taxonomy_pagination_is_local_and_exhausts_exactly() {
    let f = setup(vec![]).await;
    for (limit, offset, count, next) in [
        (3, 3, 3, Some(6)),
        (4, 8, 1, None),
        (20, 9, 0, None),
        (20, 1000, 0, None),
    ] {
        let result = f
            .client
            .video_taxonomy(&VideoTaxonomyRequest::new(
                VideoTaxonomyKind::Groups,
                limit,
                offset,
            ))
            .await
            .unwrap();
        assert_eq!(result.items.len(), count);
        assert_eq!(result.pagination.total, Some(9));
        assert_eq!(result.pagination.next_offset, next);
    }
}

#[tokio::test]
async fn mv_catalogue_sdk_and_provider_keep_music_identity_offline_rows_and_detail_link() {
    for sdk in [false, true] {
        let detail = json!({"code":200,"data":{"rid":215252,"musicrid":"MUSIC_215252","name":"Video",
            "artist":"Singer","artistid":336,"hasmv":1,"content_type":0,"online":1,
            "mvpayinfo":{"vid":9988,"play":0,"down":1}}});
        let mut f = setup(vec![
            home(),
            json_response(&body(2, vec![row(215252, 1), row(42, 0)])),
            json_response(&detail),
        ])
        .await;
        let list = if sdk {
            f.client.music_videos(&request(20, 0)).await
        } else {
            f.provider.music_videos(&request(20, 0)).await
        }
        .unwrap();
        assert_eq!(list.items.len(), 2);
        assert_eq!(list.items[0].resource_ref.to_string(), "kuwo:215252");
        assert_eq!(list.items[0].title, "Video 215252 & Live");
        assert_eq!(list.items[0].duration_ms, Some(269000));
        assert_eq!(
            list.items[0].creators[0]
                .resource_ref
                .as_ref()
                .unwrap()
                .to_string(),
            "kuwo:336"
        );
        assert_eq!(list.items[1].extensions["online"], 0);
        assert_eq!(list.items[0].extensions["source_track_id"], "215252");
        assert_eq!(list.items[0].extensions["source_group_id"], GROUPS[0].0);
        assert!(
            list.items
                .iter()
                .all(|item| item.subscribed.is_none() && item.published_at.is_none())
        );
        assert!(
            !serde_json::to_string(&list)
                .unwrap()
                .contains("never-export")
        );
        assert!(!list.items[0].extensions.contains_key("playable"));
        let detail = f
            .provider
            .video(
                &list.items[0].id,
                &VideoDetailRequest::new(VideoResourceKind::Mv),
            )
            .await
            .unwrap();
        assert_eq!(detail.video.resource_ref, list.items[0].resource_ref);
        assert_eq!(detail.video.extensions["mv_pay_info"]["vid"], "9988");
        let calls = requests(&mut f, 3).await;
        wire(&calls[1], GROUPS[0].0, 1, "anonymousCatalogueCookie123456");
        assert!(calls[2].contains("/api/www/music/musicInfo?mid=215252&"));
        assert!(!calls[2].contains("mid=9988"));
    }
}

#[tokio::test]
async fn mv_catalogue_sparse_and_empty_pages_advance_source_slots_without_losing_later_rows() {
    let mut f = setup(vec![
        home(),
        json_response(&body(100, vec![row(1, 1), row(2, 0)])),
        json_response(&body(100, vec![])),
        json_response(&body(100, vec![row(41, 1)])),
    ])
    .await;
    let result = f.provider.music_videos(&request(60, 0)).await.unwrap();
    assert_eq!(
        result
            .items
            .iter()
            .map(|item| item.id.as_str())
            .collect::<Vec<_>>(),
        ["1", "2", "41"]
    );
    assert_eq!(result.pagination.total, Some(100));
    assert_eq!(result.pagination.next_offset, Some(60));
    assert_eq!(
        result.pagination.extensions["total_scope"],
        "upstream_catalogue_slots"
    );
    assert_eq!(
        result.pagination.extensions["offset_scope"],
        "upstream_catalogue_slots"
    );
    assert_eq!(result.pagination.extensions["upstream_pages_fetched"], 3);
    let mut ids = BTreeSet::new();
    for (index, raw) in requests(&mut f, 4).await[1..].iter().enumerate() {
        assert!(ids.insert(wire(
            raw,
            GROUPS[0].0,
            index as u32 + 1,
            "anonymousCatalogueCookie123456"
        )));
    }
}

#[tokio::test]
async fn mv_catalogue_one_empty_page_still_provides_a_progressing_next_offset() {
    let mut f = setup(vec![home(), json_response(&body(100, vec![]))]).await;
    let result = f.client.music_videos(&request(20, 20)).await.unwrap();
    assert!(result.items.is_empty());
    assert!(result.pagination.has_more);
    assert_eq!(result.pagination.next_offset, Some(40));
    wire(
        &requests(&mut f, 2).await[1],
        GROUPS[0].0,
        2,
        "anonymousCatalogueCookie123456",
    );
}

#[tokio::test]
async fn mv_catalogue_maximum_window_fetches_only_five_physical_pages() {
    let mut replies = vec![home()];
    for page in 0..5 {
        replies.push(json_response(&body(
            200,
            (page * 20 + 1..=page * 20 + 20)
                .map(|id| row(id, 1))
                .collect(),
        )));
    }
    let mut f = setup(replies).await;
    let result = f.provider.music_videos(&request(100, 0)).await.unwrap();
    assert_eq!(result.items.len(), 100);
    assert_eq!(result.pagination.next_offset, Some(100));
    assert_eq!(result.pagination.extensions["upstream_pages_fetched"], 5);
    requests(&mut f, 6).await;
}

#[tokio::test]
async fn mv_catalogue_tail_out_of_range_and_empty_catalogue_preserve_total() {
    for (offset, total, rows) in [
        (20, 26, vec![row(21, 1), row(26, 1)]),
        (40, 26, vec![]),
        (0, 0, vec![]),
    ] {
        let count = rows.len();
        let mut f = setup(vec![home(), json_response(&body(total, rows))]).await;
        let result = f.client.music_videos(&request(100, offset)).await.unwrap();
        assert_eq!(result.items.len(), count);
        assert_eq!(result.pagination.total, Some(total));
        assert!(!result.pagination.has_more);
        assert_eq!(result.pagination.next_offset, None);
        requests(&mut f, 2).await;
    }
}

#[test]
fn mv_catalogue_required_arrays_and_physical_page_bounds_do_not_invent_empty_successes() {
    for value in [
        json!({"code":200}),
        json!({"code":200,"data":null}),
        json!({"code":200,"data":{}}),
        json!({"code":200,"data":{"total":0}}),
        json!({"code":200,"data":{"total":0,"mvlist":null}}),
        json!({"code":200,"data":{"mvlist":[]}}),
        body(100, (1..=21).map(|id| row(id, 1)).collect()),
        body(0, vec![row(1, 1)]),
        body(1, vec![row(1, 1), row(2, 1)]),
    ] {
        assert!(parse_group(&serde_json::to_vec(&value).unwrap(), GROUPS[0].0, 1).is_err());
    }
    assert!(
        parse_group(
            &serde_json::to_vec(&body(26, vec![row(27, 1)])).unwrap(),
            GROUPS[0].0,
            3
        )
        .is_err()
    );
    assert!(
        parse_group(
            &serde_json::to_vec(&body(100, vec![])).unwrap(),
            GROUPS[0].0,
            1
        )
        .is_ok()
    );
}

#[test]
fn mv_catalogue_unknown_metadata_stays_unknown_and_invalid_identity_is_rejected() {
    let minimal = json!({"id":42,"name":"Minimal MV","artist":"Singer","artistid":"336"});
    let parsed = parse_group(
        &serde_json::to_vec(&body(1, vec![minimal])).unwrap(),
        GROUPS[0].0,
        1,
    )
    .unwrap();
    let video = &parsed.items[0];
    assert!(video.duration_ms.is_none() && video.cover_url.is_none() && video.play_count.is_none());
    assert!(!video.extensions.contains_key("online"));
    for (key, bad) in [
        ("id", json!(0)),
        ("id", json!("01")),
        ("artistid", json!(0)),
        ("online", json!(2)),
        ("duration", json!(u64::MAX)),
        ("name", json!("")),
    ] {
        let mut item = row(1, 1);
        item[key] = bad;
        assert!(
            parse_group(
                &serde_json::to_vec(&body(1, vec![item])).unwrap(),
                GROUPS[0].0,
                1
            )
            .is_err()
        );
    }
}

#[tokio::test]
async fn mv_catalogue_duplicate_rows_and_changed_totals_fail_without_partial_results() {
    for bodies in [
        vec![body(2, vec![row(1, 1), row(1, 1)])],
        vec![body(60, vec![row(1, 1)]), body(60, vec![row(1, 1)])],
        vec![body(60, vec![row(1, 1)]), body(61, vec![row(21, 1)])],
    ] {
        let count = bodies.len() + 1;
        let mut replies = vec![home()];
        replies.extend(bodies.iter().map(json_response));
        let mut f = setup(replies).await;
        assert!(f.provider.music_videos(&request(40, 0)).await.is_err());
        requests(&mut f, count).await;
    }
}

#[tokio::test]
async fn mv_catalogue_http_mime_json_and_business_errors_remain_errors() {
    for bad in [
        response(500, "application/json", "", b"{}"),
        response(200, "text/html", "", b"{}"),
        response(200, "application/json", "", b"invalid"),
        json_response(&json!({"code":500,"msg":"never-export","data":null})),
    ] {
        let mut f = setup(vec![home(), bad]).await;
        let error = f.client.music_videos(&request(20, 0)).await.unwrap_err();
        assert!(!format!("{error:?}").contains("never-export"));
        requests(&mut f, 2).await;
    }
}

#[tokio::test]
async fn mv_catalogue_session_refresh_is_bounded_and_signs_with_the_new_anonymous_cookie() {
    for success in [false, true] {
        let rejected = response(403, "application/json", "", b"{}");
        let final_reply = if success {
            json_response(&body(0, vec![]))
        } else {
            rejected.clone()
        };
        let mut f = setup(vec![
            home(),
            rejected,
            home_with("replacementCatalogueCookie123456"),
            final_reply,
        ])
        .await;
        let result = f.client.music_videos(&request(20, 0)).await;
        assert_eq!(result.is_ok(), success);
        let calls = requests(&mut f, 4).await;
        wire(&calls[1], GROUPS[0].0, 1, "anonymousCatalogueCookie123456");
        wire(
            &calls[3],
            GROUPS[0].0,
            1,
            "replacementCatalogueCookie123456",
        );
    }
}

#[tokio::test]
async fn mv_catalogue_all_official_group_ids_are_bound_to_their_own_request() {
    for (id, _) in GROUPS {
        let mut f = setup(vec![home(), json_response(&body(0, vec![]))]).await;
        let mut r = request(20, 0);
        r.group_id = Some(id.into());
        r.area = Some(MusicVideoArea::All);
        r.video_type = Some(MusicVideoType::Mv);
        let result = f.client.music_videos(&r).await.unwrap();
        assert_eq!(result.pagination.extensions["group_id"], id);
        wire(
            &requests(&mut f, 2).await[1],
            id,
            1,
            "anonymousCatalogueCookie123456",
        );
    }
}

#[tokio::test]
async fn mv_catalogue_unsupported_filters_accounts_and_windows_fail_before_network() {
    let mut f = setup(vec![]).await;
    let mut invalid = Vec::new();
    for catalog in [
        MusicVideoCatalog::All,
        MusicVideoCatalog::Latest,
        MusicVideoCatalog::Exclusive,
        MusicVideoCatalog::TimelineAll,
    ] {
        let mut r = request(20, 0);
        r.catalog = catalog;
        invalid.push(r);
    }
    for id in [
        None,
        Some(""),
        Some("236682870"),
        Some("0236682871"),
        Some("../236682871"),
    ] {
        let mut r = request(20, 0);
        r.group_id = id.map(str::to_owned);
        invalid.push(r);
    }
    for (limit, offset) in [
        (0, 0),
        (1, 0),
        (21, 0),
        (120, 0),
        (20, 1),
        (20, u32::MAX - 15),
    ] {
        invalid.push(request(limit, offset));
    }
    for account in ["", "default", "selected"] {
        let mut r = request(20, 0);
        r.account = Some(account.into());
        invalid.push(r);
    }
    let mut r = request(20, 0);
    r.area = Some(MusicVideoArea::Japan);
    invalid.push(r);
    let mut r = request(20, 0);
    r.video_type = Some(MusicVideoType::All);
    invalid.push(r);
    let mut r = request(20, 0);
    r.order = Some(MusicVideoOrder::New);
    invalid.push(r);
    for r in invalid {
        assert_eq!(
            f.client.music_videos(&r).await.unwrap_err().code,
            ErrorCode::InvalidRequest
        );
        assert_eq!(
            f.provider.music_videos(&r).await.unwrap_err().code,
            ErrorCode::InvalidRequest
        );
    }
    for (kind, limit, offset, account) in [
        (VideoTaxonomyKind::Categories, 20, 0, None),
        (VideoTaxonomyKind::Groups, 0, 0, None),
        (VideoTaxonomyKind::Groups, 101, 0, None),
        (VideoTaxonomyKind::Groups, 20, u32::MAX, None),
        (VideoTaxonomyKind::Groups, 20, 0, Some("default")),
    ] {
        let mut r = VideoTaxonomyRequest::new(kind, limit, offset);
        r.account = account.map(str::to_owned);
        assert_eq!(
            f.client.video_taxonomy(&r).await.unwrap_err().code,
            ErrorCode::InvalidRequest
        );
        assert_eq!(
            f.provider.video_taxonomy(&r).await.unwrap_err().code,
            ErrorCode::InvalidRequest
        );
    }
    assert!(f.seen.try_recv().is_err());
}

#[tokio::test]
async fn mv_catalogue_caller_scope_is_rejected_for_both_entries_without_rotation() {
    let mut f = setup(vec![]).await;
    let credential =
        crate::client::native::tests::credential_fixture("42", "private-mv-catalogue-session")
            .caller()
            .unwrap();
    let caller = f.provider.with_caller_credential(&credential).unwrap();
    assert_eq!(
        caller.music_videos(&request(20, 0)).await.unwrap_err().code,
        ErrorCode::CapabilityNotSupported
    );
    assert_eq!(
        caller
            .video_taxonomy(&VideoTaxonomyRequest::new(VideoTaxonomyKind::Groups, 20, 0))
            .await
            .unwrap_err()
            .code,
        ErrorCode::CapabilityNotSupported
    );
    assert!(caller.take_response_credential().unwrap().is_none());
    assert!(f.seen.try_recv().is_err());
}
