use super::*;

// Synthetic metadata follows the archived official producer/DTO and the two
// credential-free directory responses. No account or captured media is included.
pub(crate) fn envelope(contents: Vec<Value>) -> Value {
    json!({"code":"000000","data":{
        "header":{"title":"Directory","dataVersion":"1234","nextPageNo":1,"nextPageNo2":1,"update":false},
        "contents":contents}})
}

pub(crate) fn tabs() -> Value {
    envelope([("华语", "huayu"), ("欧美", "oumei"), ("日韩", "rihan")]
        .into_iter()
        .flat_map(|(name, key)| [
            json!({"view":"ZJ-Title","contents":[{"view":"ZJ-Title","txt":name,"txt2":key}]}),
            json!({"view":"ZJ-SubTab-Scroll","contents":[
                {"view":"ZJ-Tab-Item","txt":"男","action":"nan"},
                {"view":"ZJ-Tab-Item","txt":"女","action":"nv"},
                {"view":"ZJ-Tab-Item","txt":"组合","action":"group"}]}),
        ]).collect())
}

pub(crate) fn singer(id: &str, initial: &str) -> Value {
    json!({"view":"ZJ-Singer-Item","resType":"2002","resId":id,
        "txt":format!("Artist {id}"),"txt2":id,"txt3":initial,"txt4":"123",
        "action":format!("mgmusic://singer-info?id={id}"),
        "img":format!("https://d.musicapp.migu.cn/data/oss/resource/artist-{id}.webp")})
}

pub(crate) fn directory() -> Value {
    envelope(vec![
        singer("10", "热"),
        singer("20", "热"),
        singer("10", "A"),
        singer("30", "Z"),
    ])
}

fn data(value: Value) -> Data {
    decode(&serde_json::to_vec(&value).unwrap()).unwrap()
}

fn selection() -> Selection {
    Selection::new(ArtistArea::Chinese, ArtistCategory::Male, ArtistGenre::All).unwrap()
}

fn parse(value: Value) -> Result<ArtistCatalog> {
    let selection = selection();
    let filters = taxonomy(&data(tabs()), &selection)?;
    catalog(
        decode(&serde_json::to_vec(&value).unwrap())?,
        selection,
        filters,
    )
}

#[test]
fn artist_directory_exact_taxonomy_maps_only_observed_semantic_subset() {
    let mut v = tabs();
    v["data"]["contents"].as_array_mut().unwrap().rotate_left(2);
    for area in [
        ArtistArea::Chinese,
        ArtistArea::Western,
        ArtistArea::JapaneseKorean,
    ] {
        for category in [
            ArtistCategory::Male,
            ArtistCategory::Female,
            ArtistCategory::Group,
        ] {
            let selected = Selection::new(area, category, ArtistGenre::All).unwrap();
            let filters = taxonomy(&data(v.clone()), &selected).unwrap();
            assert_eq!(
                filters
                    .areas
                    .iter()
                    .map(|v| v.id.as_str())
                    .collect::<Vec<_>>(),
                ["western", "japanese_korean", "chinese"]
            );
            assert_eq!(filters.areas[1].name, "日韩");
            assert_eq!(filters.areas[1].extensions["source_key"], "rihan");
            assert_eq!(
                filters
                    .categories
                    .iter()
                    .map(|v| v.id.as_str())
                    .collect::<Vec<_>>(),
                ["male", "female", "group"]
            );
            assert_eq!(filters.genres[0].extensions["upstream_filter_sent"], false);
            assert_eq!(filters.extensions["source_data_version"], "1234");
        }
    }
}

#[test]
fn artist_directory_retains_featured_and_complete_view_without_fabricating_counts() {
    let mut v = directory();
    v["data"]["contents"][2]["txt4"] = json!("125");
    let result = parse(v).unwrap();
    assert_eq!(
        result
            .featured_artists
            .iter()
            .map(|v| v.id.as_str())
            .collect::<Vec<_>>(),
        ["10", "20"]
    );
    assert_eq!(
        result
            .artists
            .iter()
            .map(|v| v.id.as_str())
            .collect::<Vec<_>>(),
        ["10", "30"]
    );
    assert_eq!(
        result.featured_artists[0].resource_ref,
        result.artists[0].resource_ref
    );
    assert_eq!(result.artists[0].resource_ref.to_string(), "migu:10");
    assert_eq!(result.featured_artists[0].extensions["follower_count"], 123);
    assert_eq!(result.artists[0].extensions["follower_count"], 125);
    assert_eq!(result.extensions["upstream_raw_count"], 4);
    assert_eq!(result.extensions["upstream_unique_artist_count"], 3);
    assert_eq!(
        result
            .filters
            .initials
            .iter()
            .map(|v| v.id.as_str())
            .collect::<Vec<_>>(),
        ["A", "Z"]
    );
    assert!(result.artists.iter().all(|v| v.album_count.is_none()
        && v.track_count.is_none()
        && v.mv_count.is_none()
        && v.description.is_empty()));
}

#[test]
fn artist_directory_changed_or_missing_taxonomy_never_guesses_request_keys() {
    for mutation in 0..10 {
        let mut v = tabs();
        match mutation {
            0 => v["data"]["contents"][0]["contents"][0]["txt"] = json!("日韩"),
            1 => v["data"]["contents"][0]["contents"][0]["txt2"] = json!("huayu&uid=secret"),
            2 => v["data"]["contents"][1]["contents"][0]["action"] = json!("nv"),
            3 => v["data"]["contents"][1]["contents"][0]["txt"] = json!("女"),
            4 => {
                v["data"]["contents"].as_array_mut().unwrap().drain(..2);
            }
            5 => {
                v["data"]["contents"][1]["contents"]
                    .as_array_mut()
                    .unwrap()
                    .remove(0);
            }
            6 => {
                v["data"]["contents"][1]["contents"][1] =
                    v["data"]["contents"][1]["contents"][0].clone()
            }
            7 => v["data"]["contents"][0]["contents"] = json!([]),
            8 => v["data"]["contents"][1]["view"] = json!("Other"),
            _ => {
                let duplicate = v["data"]["contents"].as_array().unwrap()[..2].to_vec();
                v["data"]["contents"]
                    .as_array_mut()
                    .unwrap()
                    .extend(duplicate);
            }
        }
        assert!(
            taxonomy(&data(v), &selection()).is_err(),
            "mutation {mutation}"
        );
    }
}

#[test]
fn artist_directory_artist_identity_and_action_are_checked_for_every_row() {
    for (field, bad) in [
        ("view", "ZJ-User-Item"),
        ("resType", "1"),
        ("resId", "31"),
        ("txt2", "030"),
        ("txt", ""),
        ("txt3", "other"),
        ("txt4", "123人"),
        ("action", "mgmusic://singer-info?id=31"),
        ("action", "mgmusic://user-info?id=30"),
        ("action", "mgmusic://singer-info?id=30&id=30"),
        ("action", "https://outside.invalid/?secret=credential"),
        ("img", "https://outside.invalid/data/oss/resource/30.webp"),
        ("img", "http://d.musicapp.migu.cn/data/oss/resource/30.webp"),
        (
            "img",
            "https://d.musicapp.migu.cn/data/oss/resource/30.webp?secret=credential",
        ),
        (
            "img",
            "https://d.musicapp.migu.cn/data/oss/resource/%2e%2e/30.webp",
        ),
    ] {
        let mut v = directory();
        v["data"]["contents"][3][field] = json!(bad);
        let error = parse(v).unwrap_err();
        assert_eq!(error.code, ErrorCode::UpstreamError, "{field}={bad}");
        assert!(!error.message.contains("credential"));
    }
}

#[test]
fn artist_directory_duplicates_and_cross_section_identity_conflicts_do_not_return_partial_view() {
    for mutation in 0..5 {
        let mut v = directory();
        match mutation {
            0 => v["data"]["contents"][1] = v["data"]["contents"][0].clone(),
            1 => v["data"]["contents"][3] = v["data"]["contents"][2].clone(),
            2 => v["data"]["contents"][2]["txt"] = json!("Different Artist"),
            3 => {
                v["data"]["contents"][2]["img"] =
                    json!("https://d.musicapp.migu.cn/data/oss/resource/other.webp")
            }
            _ => v["data"]["contents"][3]["txt3"] = json!("热"),
        }
        assert!(parse(v).is_err(), "mutation {mutation}");
    }
    assert!(
        parse(envelope(
            (1..=MAX_ARTISTS + 1)
                .map(|i| singer(&i.to_string(), "A"))
                .collect()
        ))
        .is_err()
    );
}

#[test]
fn artist_directory_rejects_failed_ack_missing_data_and_unproved_continuation() {
    for (field, value) in [
        ("nextPageNo", json!(2)),
        ("nextPageNo2", json!(0)),
        ("nextPageUrl", json!("https://outside.invalid/next")),
        ("hasNextPage", json!(true)),
        ("hasNext", json!(true)),
        ("update", json!(true)),
        ("dataVersion", json!("")),
    ] {
        let mut v = directory();
        v["data"]["header"][field] = value;
        assert!(parse(v).is_err(), "{field}");
    }
    for v in [
        json!({"code":"bad","info":"sensitive"}),
        json!({"code":"000000","data":null}),
        json!({"code":0,"data":{}}),
    ] {
        assert!(parse(v).is_err());
    }
    let empty = parse(envelope(vec![])).unwrap();
    assert!(empty.artists.is_empty() && empty.featured_artists.is_empty());
    assert_eq!(empty.extensions["upstream_raw_count"], 0);
}

#[test]
fn artist_directory_optional_fields_do_not_become_invented_profile_values() {
    let mut v = envelope(vec![singer("10", "#")]);
    v["data"]["contents"][0]
        .as_object_mut()
        .unwrap()
        .remove("img");
    v["data"]["contents"][0]
        .as_object_mut()
        .unwrap()
        .remove("txt4");
    let result = parse(v).unwrap();
    assert!(result.artists[0].avatar_url.is_none());
    assert!(!result.artists[0].extensions.contains_key("follower_count"));
    assert_eq!(result.filters.initials[0].id, "#");
}

#[test]
fn artist_directory_avatar_locations_keep_only_observed_https_metadata_forms() {
    for path in [
        "/data/oss/resource/00/ab/cd/avatar.webp".to_owned(),
        "/data/resource-service/file-down/00/ab/cd/ef".to_owned(),
        format!(
            "/prod/file-service/file-down/{}/{}/{}",
            "a".repeat(32),
            "b".repeat(32),
            "c".repeat(32)
        ),
    ] {
        let mut v = envelope(vec![singer("10", "A")]);
        let url = format!("https://d.musicapp.migu.cn{path}");
        v["data"]["contents"][0]["img"] = json!(url);
        assert_eq!(
            parse(v).unwrap().artists[0].avatar_url.as_deref(),
            Some(url.as_str())
        );
    }
    for path in [
        "/data/resource-service/file-down/00/ab/cd",
        "/data/resource-service/file-down/00/ab/cd/ef/gh",
        "/prod/file-service/file-down/a/b/c",
        "/data/oss/resource/",
    ] {
        let mut v = envelope(vec![singer("10", "A")]);
        v["data"]["contents"][0]["img"] = json!(format!("https://d.musicapp.migu.cn{path}"));
        assert!(parse(v).is_err());
    }
}
