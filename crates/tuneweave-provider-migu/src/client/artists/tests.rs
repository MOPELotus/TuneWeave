use super::*;

pub(crate) fn info(uid: &str, tracks: Option<u64>, albums: Option<u64>) -> Value {
    json!({"header":{},"contents":[{"view":"ZJ-SingerDetail-Scroll","contents":[{"view":"ZJ-SingerDetail-Item","action":format!("mgmusic://singer-detail-self-revealing-wall?id={uid}"),"txt":"Artist","txt4":"15","img":"https://d.musicapp.migu.cn/data/oss/cover.webp","img2":"https://d.musicapp.migu.cn/data/oss/avatar.webp"}]},{"view":"ZJ-Tab-Scroll","contents":[{"view":"ZJ-Tab-Item","action":"song","txt2":tracks.map(|n|n.to_string())},{"view":"ZJ-Tab-Item","action":"album","txt2":albums.map(|n|n.to_string())},{"view":"ZJ-Tab-Item","action":"mv","txt2":"12"}]}]})
}
pub(crate) fn bio() -> Value {
    json!({"header":{},"contents":[{"view":"ZJ-Singer-Intro-Scroll","contents":[{"view":"ZJ-Singer-Intro-Item","txt":"Biography","txt2":"First line\nSecond line"}]},{"view":"ZJ-Singer-Scroll","contents":[{"view":"ZJ-Singer-Item","txt":"Other artist"}]}]})
}
pub(crate) fn track(id: &str, uid: &str) -> Value {
    json!({"view":"ZJ-Singer-Song-Item","resType":"4001","resId":id,"action":format!("mgmusic://song-player?id={id}"),"songItem":{"resourceType":"2","contentId":id,"songId":format!("song{id}"),"songName":format!("Song{id}"),"singerList":[{"id":uid,"name":"Artist"}],"albumId":"99","album":"Actual album","copyrightId":format!("copyright{id}")}})
}
pub(crate) fn album(id: &str, digital: bool) -> Value {
    json!({"view":"ZJ-Album-Item","resId":id,"resType":if digital{"1"}else{"0"},"action":format!("mgmusic://{}?id={id}",if digital{"digital-album-info"}else{"album-info"}),"txt":format!("Album{id}"),"txt2":"Artist","txt3":"2026-03-19","img":"https://d.musicapp.migu.cn/data/oss/cover.webp"})
}
pub(crate) fn data(
    operation: ArtistOperation,
    uid: &str,
    page: u32,
    items: Vec<Value>,
    more: bool,
) -> Value {
    let contents = if operation == ArtistOperation::Songs {
        vec![
            json!({"view":"ZJ-Img-Scroll","contents":[{"txt":"Advertisement"}]}),
            json!({"view":"ZJ-Singer-Song-Scroll","contents":items}),
        ]
    } else {
        items
    };
    let mut header = json!({"dataVersion":format!("dynamic-{page}"),"nextPageNo":1});
    if more {
        header["nextPageUrl"] = json!(format!(
            "http://app.c.nf.migu.cn/MIGUM3.0/bmw/singer/{}/v1.0?singerId={uid}&pageNo={}{}",
            if operation == ArtistOperation::Songs {
                "song"
            } else {
                "album"
            },
            page + 1,
            if operation == ArtistOperation::Songs {
                "&type=1"
            } else {
                ""
            }
        ));
    }
    json!({"header":header,"contents":contents})
}
fn decode(value: Value) -> Data {
    serde_json::from_value(value).unwrap()
}

#[test]
fn artist_info_and_biography_reject_partial_pages_duplicate_sections_and_invalid_text() {
    let mut value = info("112", None, None);
    value["header"]["nextPageUrl"] = json!("https://app.c.nf.migu.cn/next");
    assert!(metadata(decode(value), "112").is_err());
    for variant in 0..5 {
        let mut value = bio();
        match variant {
            0 => value["header"]["nextPageUrl"] = json!("https://app.c.nf.migu.cn/next"),
            1 => {
                let duplicate = value["contents"][0].clone();
                value["contents"].as_array_mut().unwrap().push(duplicate);
            }
            2 => value["contents"][0]["contents"][0]["txt2"] = json!(" "),
            3 => value["contents"][0]["contents"][0]["txt2"] = json!("text\u{0000}"),
            _ => value["contents"][0]["contents"][0]["txt2"] = json!("x".repeat(131073)),
        }
        assert!(biography(decode(value)).is_err());
    }
}

#[test]
fn artist_identity_counts_and_biography_do_not_use_similar_artists_or_invent_mv_counts() {
    let mut artist = metadata(decode(info("112", Some(3), Some(4))), "112").unwrap();
    assert_eq!(artist.id, "112");
    assert_eq!(artist.track_count, Some(3));
    assert_eq!(artist.album_count, Some(4));
    assert_eq!(artist.mv_count, None);
    assert_eq!(artist.video_count, None);
    assert_eq!(artist.extensions["tab_counts"]["mv"], 12);
    assert_eq!(artist.extensions["fan_count"], 15);
    artist.biography_sections = biography(decode(bio())).unwrap();
    assert_eq!(artist.biography_sections.len(), 1);
    assert!(artist.biography_sections[0].text.contains('\n'));
    let artist = metadata(decode(info("112", None, None)), "112").unwrap();
    assert_eq!(artist.track_count, None);
    assert_eq!(artist.album_count, None);
    assert_eq!(
        metadata(decode(json!({"header":{},"contents":[]})), "112")
            .unwrap_err()
            .code,
        ErrorCode::ResourceNotFound
    );
    for change in 0..5 {
        let mut v = info("112", Some(3), Some(4));
        match change {
            0 => {
                v["contents"][0]["contents"][0]["action"] =
                    json!("mgmusic://singer-detail-self-revealing-wall?id=113")
            }
            1 => v["contents"][0]["contents"][0]["img2"] = json!("https://outside.invalid/avatar"),
            2 => v["contents"][1]["contents"][0]["txt2"] = json!("3 songs"),
            3 => {
                let duplicate = v["contents"][0].clone();
                v["contents"].as_array_mut().unwrap().push(duplicate);
            }
            _ => v["contents"][0]["contents"][0]["txt"] = json!(" "),
        };
        assert!(metadata(decode(v), "112").is_err());
    }
}

#[test]
fn artist_continuation_checks_exact_host_identity_page_and_query_without_following_urls() {
    for operation in [ArtistOperation::Songs, ArtistOperation::Albums] {
        let v = data(operation, "112", 1, vec![], true);
        let header = decode(v).header;
        assert!(continuation(&header, operation, "112", 1).unwrap());
        let url = header.next_page_url.unwrap();
        for bad in [
            url.replace("app.c.nf.migu.cn", "outside.invalid"),
            url.replace("pageNo=2", "pageNo=1"),
            url.replace("112", "113"),
            format!("{url}&singerId=112"),
            format!("{url}&extra=1"),
            format!("{url}#fragment"),
            url.replace("http://", "http://user@"),
            url.replace("http://", "ftp://"),
            url.replace("/MIGUM3.0/", "/wrong/"),
        ] {
            assert!(
                continuation(
                    &Header {
                        next_page_url: Some(bad)
                    },
                    operation,
                    "112",
                    1
                )
                .is_err()
            );
        }
    }
}

#[test]
fn artist_track_rows_verify_view_action_resource_and_each_artist_identity() {
    let page = songs(
        decode(data(
            ArtistOperation::Songs,
            "112",
            1,
            vec![track("77", "112")],
            false,
        )),
        "112",
        1,
    )
    .unwrap();
    assert_eq!(page.items[0].id, "77");
    assert_eq!(
        page.items[0].artists[0].resource_ref.as_ref().unwrap().id(),
        "112"
    );
    assert_eq!(
        page.items[0]
            .album
            .as_ref()
            .unwrap()
            .resource_ref
            .as_ref()
            .unwrap()
            .id(),
        "99"
    );
    assert!(!page.has_more);
    for mutation in 0..6 {
        let mut item = track("77", "112");
        match mutation {
            0 => item["resId"] = json!("78"),
            1 => item["resType"] = json!("2"),
            2 => item["songItem"]["resourceType"] = json!("5"),
            3 => item["songItem"]["singerList"][0]["id"] = json!("113"),
            4 => item["songItem"]["singerList"][0]["name"] = json!(""),
            _ => item["action"] = json!("mgmusic://song-player?id=77&token=secret"),
        };
        assert!(
            songs(
                decode(data(ArtistOperation::Songs, "112", 1, vec![item], false)),
                "112",
                1
            )
            .is_err()
        );
    }
    assert!(
        songs(
            decode(data(ArtistOperation::Songs, "112", 1, vec![], true)),
            "112",
            1
        )
        .is_err()
    );
    assert!(
        songs(
            decode(data(
                ArtistOperation::Songs,
                "112",
                1,
                vec![track("77", "112"); 51],
                false
            )),
            "112",
            1
        )
        .is_err()
    );
}

#[test]
fn artist_album_rows_preserve_digital_type_and_reject_fallbacks_or_wrong_actions() {
    let page = albums(
        decode(data(
            ArtistOperation::Albums,
            "112",
            1,
            vec![album("77", false), album("77", true)],
            false,
        )),
        "112",
        1,
    )
    .unwrap();
    assert_eq!(page.items[0].identity(), ("2003", "77"));
    assert_eq!(page.items[1].identity(), ("5", "77"));
    let ArtistAlbum::Digital(digital) = &page.items[1] else {
        panic!("expected digital")
    };
    assert_eq!(digital.purchased, None);
    assert_eq!(digital.price, None);
    assert_eq!(digital.track_count, None);
    assert_eq!(digital.artists[0].resource_ref, None);
    for mutation in 0..4 {
        let mut item = album("77", true);
        match mutation {
            0 => item["resType"] = json!("anything"),
            1 => item["action"] = json!("mgmusic://album-info?id=77"),
            2 => item["resId"] = json!("78"),
            _ => item["view"] = json!("ZJ-Song-Item"),
        };
        assert!(
            albums(
                decode(data(ArtistOperation::Albums, "112", 1, vec![item], false)),
                "112",
                1
            )
            .is_err()
        );
    }
    assert!(
        albums(
            decode(data(
                ArtistOperation::Albums,
                "112",
                1,
                vec![album("77", false); 11],
                false
            )),
            "112",
            1
        )
        .is_err()
    );
}
