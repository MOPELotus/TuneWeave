use super::*;
use crate::client::catalog::CatalogKind;
use crate::{
    KuwoProvider,
    client::catalog::tests::{
        body as catalogue, home, home_with, json_response, requests, response, setup,
    },
};
use tuneweave_core::{ArtistTrackListRequest, ArtistTrackOrder, MusicProvider, PageRequest};

fn info(tracks: u64, albums: u64) -> serde_json::Value {
    json!({"code":200,"data":{"id":336,"name":"Artist","aartist":"Stage&nbsp;Name","info":"Biography\n第二段","musicNum":tracks,"albumNum":albums,"mvNum":7,"artistFans":123,"country":"中国","gener":"男","birthday":"1979-01-18","pic":"https://star.kuwo.cn/star/starheads/120/artist.jpg","content_type":"0","upPcUrl":"kuwo://do-not-export","opaque":{"sid":"never-export"}}})
}
fn songs(page: u32, total: u64) -> serde_json::Value {
    let start = u64::from(page - 1) * 20;
    let list:Vec<_>=(start..total.min(start+20)).map(|n|json!({
        "rid":n+1,"musicrid":format!("MUSIC_{}",n+1),"name":format!("Track {}",n+1),
        "artist":"Artist","artistid":336,"album":"Album","albumid":"55","duration":269,"online":1,
        "releasedate":"2003-07-31","content_type":"0","pic120":"https://img4.kuwo.cn/star/albumcover/120/a.jpg",
        "hasLossless":true,"playurl":"https://example.test/not-a-play-grant","opaque":{"sid":"never-export"}
    })).collect();
    json!({"code":200,"data":{"total":total,"list":list}})
}
fn track_request(limit: u32, offset: u32) -> ArtistTrackListRequest {
    ArtistTrackListRequest {
        limit,
        offset,
        account: None,
        order: ArtistTrackOrder::PlatformDefault,
    }
}
fn detail(value: &serde_json::Value) -> Result<Artist> {
    parse_artist(&serde_json::to_vec(value).unwrap(), "336")
}
fn tracks(value: &serde_json::Value, page: u32) -> Result<ArtistPage<Track>> {
    parse_tracks(
        &serde_json::to_vec(value).unwrap(),
        &detail(&info(0, 0)).unwrap(),
        page,
    )
}
fn albums(value: &serde_json::Value, page: u32) -> Result<ArtistPage<Album>> {
    parse_albums(
        &serde_json::to_vec(value).unwrap(),
        &detail(&info(0, 0)).unwrap(),
        page,
    )
}

#[test]
fn detail_keeps_full_biography_aliases_counts_and_only_public_metadata() {
    let mut value = info(1710, 45);
    let biography = "完整简介\n".repeat(1500);
    value["data"]["info"] = json!(biography);
    let artist = detail(&value).unwrap();
    assert_eq!(artist.id, "336");
    assert_eq!(artist.aliases, vec!["Stage Name"]);
    assert_eq!(artist.description, biography.trim());
    assert_eq!(artist.track_count, Some(1710));
    assert_eq!(artist.album_count, Some(45));
    assert_eq!(artist.mv_count, Some(7));
    assert!(artist.video_count.is_none());
    assert_eq!(artist.extensions["gender"], "男");
    assert_eq!(artist.extensions["artist_fans"], 123);
    let encoded = serde_json::to_string(&artist).unwrap();
    assert!(!encoded.contains("never-export"));
    assert!(!encoded.contains("kuwo://"));
    for key in ["musicNum", "albumNum", "mvNum", "artistFans"] {
        value["data"].as_object_mut().unwrap().remove(key);
    }
    let unknown = detail(&value).unwrap();
    assert!(unknown.track_count.is_none());
    assert!(unknown.album_count.is_none());
    assert!(unknown.mv_count.is_none());
    let mut album = catalogue(CatalogKind::Album, 1, 1);
    album["data"]["albumList"][0]["albuminfo"] = json!(biography);
    assert_eq!(
        albums(&album, 1).unwrap().items[0].description,
        biography.trim()
    );
}

#[test]
fn malformed_detail_and_not_found_have_distinct_terminal_errors() {
    assert_eq!(
        detail(&json!({"code":-1,"data":null})).unwrap_err().code,
        ErrorCode::ResourceNotFound
    );
    for bad in [
        json!({}),
        json!({"code":200,"data":null}),
        json!({"code":200,"data":{}}),
        json!({"code":"200","data":{}}),
        json!({"code":2001,"data":{}}),
        json!({"code":-1,"data":{"id":336}}),
    ] {
        assert_eq!(detail(&bad).unwrap_err().code, ErrorCode::UpstreamError);
    }
    for (field, val) in [
        ("id", json!(337)),
        ("id", json!("0336")),
        ("albumNum", json!(-1)),
        ("musicNum", json!("2.5")),
        ("mvNum", json!(true)),
        ("info", json!("a".repeat(65537))),
        ("aartist", json!("\u{0000}")),
        ("name", json!(" ")),
        ("content_type", json!(4)),
    ] {
        let mut value = info(3, 1);
        value["data"][field] = val;
        assert!(detail(&value).is_err(), "{field}");
    }
    let error = detail(&json!({"code":-99,"msg":"never-export","data":null})).unwrap_err();
    assert!(!format!("{error:?}").contains("never-export"));
}

#[test]
fn track_and_album_identity_binding_preserves_metadata_and_rejects_foreign_artists() {
    let page = tracks(&songs(1, 1), 1).unwrap();
    let track = &page.items[0];
    assert_eq!(track.resource_ref.to_string(), "kuwo:1");
    assert_eq!(track.duration_ms, Some(269000));
    assert_eq!(track.extensions["release_date"], "2003-07-31");
    assert_eq!(track.extensions["source_artist_id"], "336");
    assert!(
        track
            .album
            .as_ref()
            .unwrap()
            .cover_url
            .as_ref()
            .unwrap()
            .starts_with("https://img4.kuwo.cn/")
    );
    assert!(track.playable.is_none());
    assert_eq!(track.available_qualities, vec![Quality::Lossless]);
    assert!(
        !serde_json::to_string(track)
            .unwrap()
            .contains("not-a-play-grant")
    );
    for (field, val) in [
        ("artistid", json!(337)),
        ("artistid", json!(null)),
        ("musicrid", json!("MUSIC_2")),
        ("rid", json!("01")),
        ("name", json!("\u{0000}")),
        ("content_type", json!(6)),
        ("ad_type", json!("advertisement")),
    ] {
        let mut value = songs(1, 1);
        value["data"]["list"][0][field] = val;
        assert!(tracks(&value, 1).is_err(), "{field}");
    }
    let mut collaboration = songs(1, 1);
    collaboration["data"]["list"][0]["artist"] = json!("Other&Artist");
    collaboration["data"]["list"][0]["artistid"] = json!("337&336");
    assert!(tracks(&collaboration, 1).is_ok());
    collaboration["data"]["list"][0]["artistid"] = json!(337);
    let partial = tracks(&collaboration, 1).unwrap();
    assert_eq!(
        partial.items[0].artists[0]
            .resource_ref
            .as_ref()
            .unwrap()
            .id(),
        "337"
    );
    assert_eq!(partial.items[0].artists[1].name, "Artist");
    assert!(partial.items[0].artists[1].resource_ref.is_none());
    assert_eq!(partial.items[0].extensions["source_artist_id"], "336");
    collaboration["data"]["list"][0]["artist"] = json!("Other&amp;Name&Artist");
    assert_eq!(
        tracks(&collaboration, 1).unwrap().items[0].artists[0].name,
        "Other&Name"
    );
    collaboration["data"]["list"][0]["artist"] = json!("Other&Unrelated");
    assert!(tracks(&collaboration, 1).is_err());
    let mut foreign = catalogue(CatalogKind::Album, 1, 1);
    foreign["data"]["albumList"][0]["artistid"] = json!(337);
    assert!(albums(&foreign, 1).is_err());
    foreign["data"]["albumList"][0]["artist"] = json!("Other&Artist");
    let collaboration = albums(&foreign, 1).unwrap();
    assert_eq!(
        collaboration.items[0].artists[0]
            .resource_ref
            .as_ref()
            .unwrap()
            .id(),
        "337"
    );
    assert_eq!(collaboration.items[0].artists[1].name, "Artist");
    assert!(collaboration.items[0].artists[1].resource_ref.is_none());
    assert_eq!(collaboration.items[0].extensions["source_artist_id"], "336");
    foreign["data"]["albumList"][0]["artist"] = json!("Other&Stage&nbsp;Name");
    assert!(albums(&foreign, 1).is_ok());
    foreign["data"]["albumList"][0]["artist"] = json!("Other&Unrelated");
    assert!(albums(&foreign, 1).is_err());
    let verified = detail(&info(0, 0)).unwrap();
    let sixteen = ["Artist"; 16].join("&");
    assert_eq!(
        artist_credits(&sixteen, "336", &verified).unwrap().len(),
        16
    );
    assert!(artist_credits(&format!("{sixteen}&Artist"), "336", &verified).is_err());
    assert!(artist_credits("Artist", &["336"; 17].join("&"), &verified).is_err());
    let mut missing = songs(1, 21);
    missing["data"]["list"].as_array_mut().unwrap().pop();
    assert!(tracks(&missing, 1).is_err());
    assert!(tracks(&songs(1, 0), 1).unwrap().items.is_empty());
}

#[tokio::test]
async fn full_windows_use_the_requested_artist_and_keep_wapi_unsigned() {
    for album in [false, true] {
        let mut responses = vec![home(), json_response(&info(162, 162))];
        for page in [1, 3, 4, 5, 6, 7, 8] {
            responses.push(json_response(&if album {
                catalogue(CatalogKind::Album, page, 162)
            } else {
                songs(page, 162)
            }));
        }
        let mut fixture = setup(responses).await;
        let (ids, pagination) = if album {
            let result = fixture
                .provider
                .artist_albums("336", &PageRequest::new(100, 59))
                .await
                .unwrap();
            (
                result.items.into_iter().map(|v| v.id).collect::<Vec<_>>(),
                result.pagination,
            )
        } else {
            let result = fixture
                .provider
                .artist_tracks("336", &track_request(100, 59))
                .await
                .unwrap();
            (
                result.items.into_iter().map(|v| v.id).collect::<Vec<_>>(),
                result.pagination,
            )
        };
        assert_eq!(ids, (60..=159).map(|n| n.to_string()).collect::<Vec<_>>());
        assert_eq!(pagination.total, Some(162));
        assert_eq!(pagination.next_offset, Some(159));
        assert_eq!(pagination.extensions["upstream_pages_fetched"], 7);
        assert_eq!(pagination.extensions["order"], "platform_default");
        let calls = requests(&mut fixture, 9).await;
        assert!(calls[1].starts_with("GET /api/www/artist/artist?"));
        assert!(!calls[1].contains("&pn="));
        for (call, page) in calls[2..].iter().zip([1, 3, 4, 5, 6, 7, 8]) {
            let url = Url::parse(&format!(
                "https://www.kuwo.cn{}",
                call.split_whitespace().nth(1).unwrap()
            ))
            .unwrap();
            let params = url.query_pairs().collect::<BTreeMap<_, _>>();
            assert_eq!(params["artistid"], "336");
            assert_eq!(params["pn"], page.to_string());
            assert_eq!(params["rn"], "20");
            assert_eq!(params.len(), 7);
            assert!(call.contains("referer: https://www.kuwo.cn/singer_detail/336"));
            assert_eq!(call.contains("secret: "), !album);
            assert_eq!(call.contains("cookie: "), !album);
            assert_eq!(
                url.path(),
                if album {
                    "/api/www/artist/artistAlbum"
                } else {
                    "/api/www/artist/artistMusic"
                }
            );
        }
        assert_eq!(
            UnsignedArtistEndpoint::Albums.target().0,
            "https://wapi.kuwo.cn/api/www/artist/artistAlbum"
        );
    }
}

#[tokio::test]
async fn last_empty_and_out_of_range_windows_preserve_the_first_page_total() {
    for album in [false, true] {
        for (total, offset, limit, count, pages) in [
            (45, 39, 20, 6, vec![1, 2, 3]),
            (45, 80, 20, 0, vec![1]),
            (0, 0, 20, 0, vec![1]),
            (7, 3, 10, 4, vec![1]),
        ] {
            let mut responses = vec![home(), json_response(&info(total, total))];
            responses.extend(pages.iter().map(|page| {
                json_response(&if album {
                    catalogue(CatalogKind::Album, *page, total)
                } else {
                    songs(*page, total)
                })
            }));
            let mut fixture = setup(responses).await;
            let (len, meta) = if album {
                let p = fixture
                    .provider
                    .artist_albums("336", &PageRequest::new(limit, offset))
                    .await
                    .unwrap();
                (p.items.len(), p.pagination)
            } else {
                let p = fixture
                    .provider
                    .artist_tracks("336", &track_request(limit, offset))
                    .await
                    .unwrap();
                (p.items.len(), p.pagination)
            };
            assert_eq!(len, count);
            assert_eq!(meta.total, Some(total));
            assert!(!meta.has_more);
            assert!(meta.next_offset.is_none());
            requests(&mut fixture, 2 + pages.len()).await;
        }
    }
}

#[tokio::test]
async fn overview_is_an_explicit_preview_with_verified_artist_counts() {
    for total in [0, 7, 25] {
        let mut fixture = setup(vec![
            home(),
            json_response(&info(total, 0)),
            json_response(&songs(1, total)),
        ])
        .await;
        let overview = fixture.provider.artist_overview("336", None).await.unwrap();
        assert_eq!(overview.artist.id, "336");
        assert_eq!(overview.featured_tracks.len(), total.min(10) as usize);
        assert_eq!(overview.has_more_tracks, total > 10);
        requests(&mut fixture, 3).await;
    }
    let mut unknown = info(0, 0);
    unknown["data"].as_object_mut().unwrap().remove("musicNum");
    let mut fixture = setup(vec![
        home(),
        json_response(&unknown),
        json_response(&songs(1, 25)),
    ])
    .await;
    let overview = fixture.provider.artist_overview("336", None).await.unwrap();
    assert!(overview.artist.track_count.is_none());
    assert!(overview.has_more_tracks);
    requests(&mut fixture, 3).await;
}

#[tokio::test]
async fn count_drift_duplicate_pages_and_foreign_items_never_return_partial_results() {
    for album in [false, true] {
        for mode in 0..5 {
            let mut metadata = info(23, 23);
            let mut first = if album {
                catalogue(CatalogKind::Album, 1, 23)
            } else {
                songs(1, 23)
            };
            let mut next = if album {
                catalogue(CatalogKind::Album, 2, 23)
            } else {
                songs(2, 23)
            };
            let key = if album { "albumList" } else { "list" };
            match mode {
                0 => {
                    metadata["data"][if album { "albumNum" } else { "musicNum" }] = json!(24);
                }
                1 => {
                    next = if album {
                        catalogue(CatalogKind::Album, 2, 24)
                    } else {
                        songs(2, 24)
                    };
                }
                2 => {
                    next["data"][key][0] = first["data"][key][0].clone();
                }
                3 => {
                    next["data"][key][0]["artistid"] = json!(337);
                }
                _ => {
                    first["data"][key][1] = first["data"][key][0].clone();
                }
            }
            let early = mode == 0 || mode == 4;
            let mut responses = vec![home(), json_response(&metadata), json_response(&first)];
            if !early {
                responses.push(json_response(&next));
            }
            let mut fixture = setup(responses).await;
            let failed = if album {
                fixture
                    .provider
                    .artist_albums("336", &PageRequest::new(100, 0))
                    .await
                    .is_err()
            } else {
                fixture
                    .provider
                    .artist_tracks("336", &track_request(100, 0))
                    .await
                    .is_err()
            };
            assert!(failed);
            requests(&mut fixture, if early { 3 } else { 4 }).await;
        }
    }
}

#[tokio::test]
async fn signed_artist_requests_refresh_once_but_unsigned_albums_never_retry() {
    let rejection = response(403, "text/plain", "", b"denied");
    let mut fixture = setup(vec![
        home(),
        rejection.clone(),
        home_with("newArtistAnonymousCookie12345"),
        json_response(&info(1, 1)),
        json_response(&songs(1, 1)),
    ])
    .await;
    assert_eq!(
        fixture
            .provider
            .artist_tracks("336", &track_request(1, 0))
            .await
            .unwrap()
            .items
            .len(),
        1
    );
    let calls = requests(&mut fixture, 5).await;
    assert!(calls[3].contains("newArtistAnonymousCookie12345"));
    assert!(calls[4].contains("newArtistAnonymousCookie12345"));
    let mut fixture = setup(vec![home(), rejection.clone(), home(), rejection]).await;
    assert!(fixture.provider.artist("336", None).await.is_err());
    requests(&mut fixture, 4).await;
    for response in [
        response(401, "application/json", "", b"{}"),
        response(429, "application/json", "", b"{}"),
        response(200, "text/html", "", b"{}"),
        response(302, "text/html", "Location: https://example.test/\r\n", b""),
        json_response(&json!({"code":-1,"data":null,"msg":"secret-message"})),
    ] {
        let mut fixture = setup(vec![home(), json_response(&info(1, 1)), response]).await;
        let error = fixture
            .provider
            .artist_albums("336", &PageRequest::new(1, 0))
            .await
            .unwrap_err();
        assert!(!format!("{error:?}").contains("secret-message"));
        requests(&mut fixture, 3).await;
    }
}

#[tokio::test]
async fn inputs_and_failed_artist_identity_stop_before_catalogue_requests() {
    let mut fixture = setup(vec![]).await;
    for id in ["", "0", "0336", "-1", "336&other", "18446744073709551616"] {
        assert!(fixture.provider.artist(id, None).await.is_err());
        assert!(fixture.provider.artist_overview(id, None).await.is_err());
        assert!(
            fixture
                .provider
                .artist_tracks(id, &track_request(1, 0))
                .await
                .is_err()
        );
        assert!(
            fixture
                .provider
                .artist_albums(id, &PageRequest::new(1, 0))
                .await
                .is_err()
        );
    }
    assert!(
        fixture
            .provider
            .artist("336", Some("private"))
            .await
            .is_err()
    );
    for r in [
        PageRequest::new(0, 0),
        PageRequest::new(101, 0),
        PageRequest::new(1, u32::MAX),
        PageRequest {
            account: Some("private".into()),
            ..PageRequest::new(1, 0)
        },
    ] {
        assert!(fixture.provider.artist_albums("336", &r).await.is_err());
        let r = ArtistTrackListRequest {
            limit: r.limit,
            offset: r.offset,
            account: r.account,
            order: ArtistTrackOrder::PlatformDefault,
        };
        assert!(fixture.provider.artist_tracks("336", &r).await.is_err());
    }
    for order in [ArtistTrackOrder::Hot, ArtistTrackOrder::Time] {
        let mut r = track_request(1, 0);
        r.order = order;
        assert!(fixture.provider.artist_tracks("336", &r).await.is_err());
    }
    let credential =
        tuneweave_core::ProviderCredential::new(Platform::Kuwo, "unknown", "secret", None).unwrap();
    assert!(
        fixture
            .provider
            .with_caller_credential(&credential)
            .is_err()
    );
    requests(&mut fixture, 0).await;
    for metadata in [
        json!({"code":-1,"data":null}),
        json!({"code":200,"data":{}}),
        {
            let mut data = info(1, 1);
            data["data"]["id"] = json!(337);
            data
        },
    ] {
        let mut fixture = setup(vec![home(), json_response(&metadata)]).await;
        assert!(
            fixture
                .provider
                .artist_albums("336", &PageRequest::new(1, 0))
                .await
                .is_err()
        );
        requests(&mut fixture, 2).await;
    }
}

#[tokio::test]
#[ignore = "Current official public artist metadata and catalogue; no account or media"]
async fn live_artist_detail_overview_and_catalogue_end_windows() {
    let provider = KuwoProvider::new(KuwoConfig::default()).unwrap();
    let artist = provider.artist("336", None).await.unwrap();
    assert_eq!(artist.name, "周杰伦");
    assert!(!artist.description.is_empty());
    let track_count = artist.track_count.unwrap();
    let album_count = artist.album_count.unwrap();
    assert!(track_count >= 15 && album_count >= 6);
    let tracks = provider
        .artist_tracks(
            "336",
            &track_request(20, u32::try_from(track_count - 15).unwrap()),
        )
        .await
        .unwrap();
    assert_eq!(tracks.items.len(), 15);
    assert_eq!(tracks.pagination.total, Some(track_count));
    assert!(!tracks.pagination.has_more);
    let albums = provider
        .artist_albums(
            "336",
            &PageRequest::new(20, u32::try_from(album_count - 6).unwrap()),
        )
        .await
        .unwrap();
    assert_eq!(albums.items.len(), 6);
    assert_eq!(albums.pagination.total, Some(album_count));
    assert!(!albums.pagination.has_more);
    let beyond = provider
        .artist_albums(
            "336",
            &PageRequest::new(20, u32::try_from(album_count + 20).unwrap()),
        )
        .await
        .unwrap();
    assert!(beyond.items.is_empty());
    assert_eq!(beyond.pagination.total, Some(album_count));
    let overview = provider.artist_overview("336", None).await.unwrap();
    assert_eq!(overview.featured_tracks.len(), 10);
    assert!(overview.has_more_tracks);
}
