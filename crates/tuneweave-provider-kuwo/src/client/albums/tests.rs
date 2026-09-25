use super::*;
use crate::{
    KuwoProvider,
    client::catalog::tests::{json_response, requests, response, setup},
};
use tuneweave_core::{MusicProvider, PageRequest};

fn body(total: u64) -> serde_json::Value {
    let music: Vec<_> = (1..=total).map(|n| json!({
        "id":n.to_string(),"musicrid":n.to_string(),"albumId":1293,"name":format!("Track {n}"),
        "artist":"First&amp;Name&Second","artistid":"336","allartistid":"336&337",
        "album":"Album","duration":"342","track":(total-n+1).to_string(),
        "releasedate":"2003-07-31","online":"1","content_type":"0","ad_type":"",
        "web_albumpic_short":"120/a/cover.jpg","MINFO":"level:ff,bitrate:2000,format:flac;level:p,bitrate:320,format:mp3",
        "pay":"255","MVFLAG":"1","mvpayinfo":{"vid":"900001","play":"1"},"opaque":{"sid":"do-not-export"},"playurl":"https://example.test/not-a-grant"
    })).collect();
    json!({"id":"1293","albumid":"1293","name":"Album &amp; More","artist":"First&amp;Name&Second","artistid":"336",
        "songnum":total.to_string(),"musiclist":music,"info":"First paragraph\n第二段",
        "pic":"120/a/cover.jpg","pub":"2003-07-31","company":"Label","lang":"国语","content_type":"0","ad_type":"",
        "pay":"255","vip":"1","opaque":{"sid":"do-not-export"}})
}
fn parsed(value: &serde_json::Value) -> Result<AlbumDetail> {
    parse(&serde_json::to_vec(value).unwrap(), "1293")
}

#[test]
fn complete_album_preserves_order_long_metadata_and_actual_collaboration_ids() {
    let mut value = body(25);
    let biography = "完整专辑简介\n".repeat(1500);
    value["info"] = json!(biography);
    let result = parsed(&value).unwrap();
    assert_eq!(result.album.name, "Album & More");
    assert_eq!(result.album.track_count, Some(25));
    assert_eq!(result.album.description, biography.trim());
    assert_eq!(result.album.published_at.as_deref(), Some("2003-07-31"));
    assert_eq!(result.album.company.as_deref(), Some("Label"));
    assert_eq!(result.album.extensions["language"], "国语");
    assert_eq!(result.album.artists[0].name, "First&Name");
    assert!(result.album.artists[1].resource_ref.is_none());
    assert_eq!(
        result.album.cover_url.as_deref(),
        Some("https://img4.kuwo.cn/star/albumcover/120/a/cover.jpg")
    );
    assert_eq!(
        result
            .tracks
            .iter()
            .map(|v| v.id.clone())
            .collect::<Vec<_>>(),
        (1..=25).map(|n| n.to_string()).collect::<Vec<_>>()
    );
    let first = &result.tracks[0];
    assert_eq!(first.extensions["track_number"], "25");
    assert_eq!(first.duration_ms, Some(342000));
    assert_eq!(first.artists[1].resource_ref.as_ref().unwrap().id(), "337");
    assert_eq!(first.album.as_ref().unwrap().name, "Album & More");
    assert_eq!(
        first
            .album
            .as_ref()
            .unwrap()
            .resource_ref
            .as_ref()
            .unwrap()
            .id(),
        "1293"
    );
    assert_eq!(first.available_qualities, vec![Quality::Lossless]);
    assert!(first.playable.is_none());
    assert_eq!(first.mv_ref.as_ref().unwrap().to_string(), "kuwo:1");
    assert_eq!(first.extensions["mv_pay_info"]["vid"], "900001");
    let text = format!(
        "{}{}",
        serde_json::to_string(&result.album).unwrap(),
        serde_json::to_string(&result.tracks).unwrap()
    );
    assert!(!text.contains("do-not-export"));
    assert!(!text.contains("not-a-grant"));
    // A compilation can contain a different credited artist; the album identity binds it.
    value["musiclist"][24]["artist"] = json!("Guest");
    value["musiclist"][24]["artistid"] = json!("999");
    value["musiclist"][24]["allartistid"] = json!("999");
    assert!(parsed(&value).is_ok());
}

#[test]
fn absent_identity_is_not_a_valid_empty_album_and_counts_are_not_inferred() {
    assert!(parsed(&body(0)).unwrap().tracks.is_empty());
    for value in [
        json!({}),
        json!({"musiclist":[],"songnum":"0"}),
        json!({"code":200,"data":{}}),
        json!({"code":2001,"msg":"do-not-export"}),
    ] {
        let error = parsed(&value).err().unwrap();
        assert_eq!(error.code, ErrorCode::UpstreamError);
        assert!(!format!("{error:?}").contains("do-not-export"));
    }
    for (key, bad) in [
        ("id", json!("1294")),
        ("albumid", json!("01293")),
        ("songnum", json!("24")),
        ("songnum", json!(-1)),
        ("songnum", json!(1001)),
        ("content_type", json!(4)),
        ("ad_type", json!("ad")),
        ("name", json!(" ")),
        ("info", json!("a".repeat(65537))),
        ("pub", json!("2003-02-30")),
        ("code", json!(2001)),
    ] {
        let mut value = body(25);
        value[key] = bad;
        assert!(parsed(&value).is_err(), "{key}");
    }
    let mut value = body(1);
    for key in ["company", "lang", "pub", "pic"] {
        value.as_object_mut().unwrap().remove(key);
    }
    let unknown = parsed(&value).unwrap().album;
    assert!(
        unknown.company.is_none() && unknown.published_at.is_none() && unknown.cover_url.is_none()
    );
}

#[test]
fn all_tracks_are_checked_for_identity_duplicates_and_malformed_credits() {
    for (key, bad) in [
        ("id", json!("1")),
        ("musicrid", json!("MUSIC_25")),
        ("albumId", json!(1294)),
        ("duration", json!(u64::MAX)),
        ("duration", json!("3.5")),
        ("artistid", json!("0336")),
        ("allartistid", json!("337&336")),
        ("allartistid", json!("336&337&338")),
        ("content_type", json!(6)),
        ("ad_type", json!("advertisement")),
        ("name", json!("\u{0000}")),
        ("MINFO", json!("x".repeat(16385))),
    ] {
        let mut value = body(25);
        value["musiclist"][24][key] = bad;
        assert!(parsed(&value).is_err(), "{key}");
    }
    let mut repeated = body(25);
    repeated["musiclist"][24] = repeated["musiclist"][0].clone();
    assert!(parsed(&repeated).is_err());
    let mut partial = body(1);
    partial["musiclist"][0]
        .as_object_mut()
        .unwrap()
        .remove("allartistid");
    assert!(
        parsed(&partial).unwrap().tracks[0].artists[1]
            .resource_ref
            .is_none()
    );
}

#[test]
fn relative_covers_stay_inside_the_official_album_directory() {
    assert!(cover("120/path/picture.jpg").is_some());
    for path in [
        "",
        "/evil.test/a",
        "https://evil.test/a",
        "../a",
        "120/../a",
        "120/%2e%2e/a",
        "120/a?x=1",
        "120/a#x",
        "120/a\\b",
        "120//a",
    ] {
        assert!(cover(path).is_none(), "{path}");
    }
}

#[tokio::test]
async fn pages_slice_the_complete_album_with_one_unsigned_request() {
    for (total, offset, limit, count, more) in [
        (25, 19, 20, 6, false),
        (25, 0, 20, 20, true),
        (25, 50, 20, 0, false),
        (0, 0, 20, 0, false),
        (125, 19, 100, 100, true),
    ] {
        let bytes = serde_json::to_vec(&body(total)).unwrap();
        let mut fixture = setup(vec![response(
            200,
            "text/javascript; charset=utf-8",
            "",
            &bytes,
        )])
        .await;
        let page = fixture
            .provider
            .album_tracks("1293", &PageRequest::new(limit, offset))
            .await
            .unwrap();
        assert_eq!(page.items.len(), count);
        assert_eq!(page.pagination.total, Some(total));
        assert_eq!(page.pagination.has_more, more);
        assert_eq!(
            page.pagination.next_offset,
            more.then_some(offset + count as u32)
        );
        if count > 0 {
            assert_eq!(page.items[0].id, (offset + 1).to_string());
        }
        assert_eq!(page.pagination.extensions["complete_read"], true);
        let seen = requests(&mut fixture, 1).await;
        let wire = &seen[0];
        assert!(!wire.to_ascii_lowercase().contains("cookie:"));
        assert!(!wire.to_ascii_lowercase().contains("secret:"));
        let path = wire.split_whitespace().nth(1).unwrap();
        let url = Url::parse(&format!("https://searchlist.kuwo.cn{path}")).unwrap();
        assert_eq!(url.path(), "/r.s");
        let query = url.query_pairs().collect::<BTreeMap<_, _>>();
        assert_eq!(query.len(), 8);
        assert_eq!(query["stype"], "albuminfo");
        assert_eq!(query["albumid"], "1293");
        for key in [
            "show_copyright_off",
            "alflac",
            "vipver",
            "sortby",
            "newver",
            "mobi",
        ] {
            assert_eq!(query[key], "1");
        }
    }
}

#[tokio::test]
async fn invalid_input_stops_before_io_and_corruption_outside_window_is_rejected() {
    let mut fixture = setup(vec![]).await;
    for id in ["", "0", "01293", "-1", "1293&x=1", "18446744073709551616"] {
        assert!(fixture.provider.album(id, None).await.is_err());
        assert!(
            fixture
                .provider
                .album_tracks(id, &PageRequest::new(20, 0))
                .await
                .is_err()
        );
    }
    assert!(
        fixture
            .provider
            .album("1293", Some("private"))
            .await
            .is_err()
    );
    for request in [
        PageRequest::new(0, 0),
        PageRequest::new(101, 0),
        PageRequest::new(1, u32::MAX),
        PageRequest {
            account: Some("private".into()),
            ..PageRequest::new(20, 0)
        },
    ] {
        assert!(
            fixture
                .provider
                .album_tracks("1293", &request)
                .await
                .is_err()
        );
    }
    requests(&mut fixture, 0).await;
    let mut value = body(25);
    value["musiclist"][24]["albumId"] = json!(1294);
    let mut fixture = setup(vec![json_response(&value)]).await;
    assert!(
        fixture
            .provider
            .album_tracks("1293", &PageRequest::new(1, 0))
            .await
            .is_err()
    );
    requests(&mut fixture, 1).await;
}

#[tokio::test]
async fn errors_do_not_retry_or_evaluate_javascript() {
    for wire in [response(401,"application/json","",b"{}"),response(429,"application/json","Retry-After: 2\r\n",b"{}"),response(302,"application/json","Location: https://example.test/\r\n",b"{}"),response(200,"text/html","",b"{}"),response(200,"text/javascript","",b"callback({musiclist:[]});"),response(200,"application/json","",b"{}"),b"HTTP/1.1 200 OK\r\nContent-Type: text/javascript\r\nContent-Length: 2097153\r\nConnection: close\r\n\r\n".to_vec()] {
        let mut fixture=setup(vec![wire]).await;
        assert!(fixture.provider.album("1293",None).await.is_err());
        requests(&mut fixture,1).await;
    }
}

#[tokio::test]
#[ignore = "Current official public album metadata and complete tracks; no account or media"]
async fn live_album_detail_complete_tracks_and_collaboration() {
    let provider = KuwoProvider::new(KuwoConfig::default()).unwrap();
    let album = provider.album("1293", None).await.unwrap();
    assert_eq!(album.name, "叶惠美");
    assert_eq!(album.track_count, Some(11));
    let end = provider
        .album_tracks("12478", &PageRequest::new(20, 19))
        .await
        .unwrap();
    assert_eq!(end.pagination.total, Some(25));
    assert_eq!(end.items.len(), 6);
    assert!(!end.pagination.has_more);
    let beyond = provider
        .album_tracks("12478", &PageRequest::new(20, 50))
        .await
        .unwrap();
    assert_eq!(beyond.pagination.total, Some(25));
    assert!(beyond.items.is_empty());
    let collaboration = provider
        .album_tracks("63184186", &PageRequest::new(20, 0))
        .await
        .unwrap();
    assert_eq!(collaboration.items.len(), 1);
    assert_eq!(
        collaboration.items[0].artists[1]
            .resource_ref
            .as_ref()
            .unwrap()
            .id(),
        "336"
    );
    assert_eq!(
        provider.album("999999999999", None).await.unwrap_err().code,
        ErrorCode::UpstreamError
    );
}
