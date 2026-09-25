use super::*;

fn singer(id: u64, name: &str) -> Value {
    json!({"singerid":id,"singername":name,"fanscount":"12",
        "imgurl":"http://singerimg.kugou.com/{size}/artist.jpg"})
}

pub(crate) fn catalogue() -> Value {
    json!({"status":1,"errcode":0,"data":{
        "timestamp":123,
        "enu_list":{
            "types":[{"key":0,"value":"全部","musician":"0"},
                {"key":1,"value":"华语","musician":"0"},
                {"key":6,"value":"韩国","musician":"0"},
                {"key":0,"value":"音乐人","musician":"3"},
                {"key":7,"value":"粤语","musician":"0"}],
            "sextypes":[{"key":0,"value":"全部"},{"key":2,"value":"女"}]},
        "info":[{"title":"热门","singer":[singer(42,"Alpha")]},
            {"title":"A","singer":[singer(42,"Alpha"),singer(43,"Another")]},
            {"title":"B","singer":[]},
            {"title":"#","singer":[singer(44,"12")]}]}})
}

#[test]
fn artist_catalog_preserves_hot_initial_order_identity_and_absent_counts() {
    let parsed = parse_catalogue(catalogue().to_string().as_bytes(), &Default::default()).unwrap();
    assert_eq!(parsed.featured_artists[0].id, "42");
    assert_eq!(
        parsed
            .artists
            .iter()
            .map(|a| a.id.as_str())
            .collect::<Vec<_>>(),
        ["42", "43", "44"]
    );
    assert_eq!(parsed.artists[0].resource_ref.id(), "42");
    assert_eq!(parsed.artists[0].track_count, None);
    assert_eq!(parsed.artists[0].album_count, None);
    assert_eq!(parsed.artists[0].mv_count, None);
    assert_eq!(parsed.artists[0].extensions["directory_group"], "A");
    assert_eq!(parsed.artists[0].extensions["fans_count"], 12);
    assert!(
        parsed.artists[0]
            .avatar_url
            .as_ref()
            .unwrap()
            .starts_with("https://")
    );
    assert_eq!(
        parsed
            .filters
            .initials
            .iter()
            .map(|f| f.id.as_str())
            .collect::<Vec<_>>(),
        ["A", "B", "#"]
    );
    assert_eq!(
        parsed
            .filters
            .areas
            .iter()
            .map(|f| f.id.as_str())
            .collect::<Vec<_>>(),
        ["all", "chinese", "korean"]
    );
    assert!(parsed.filters.genres.is_empty());
    assert!(!parsed.extensions.contains_key("complete_snapshot"));
    assert_eq!(
        parsed.extensions["catalog_scope"],
        "upstream_grouped_directory"
    );
}

#[test]
fn artist_catalog_rejects_duplicate_groups_ids_and_contradictory_cross_section_names() {
    for case in 0..7 {
        let mut value = catalogue();
        match case {
            0 => value["data"]["info"][2]["title"] = json!("A"),
            1 => value["data"]["info"][1]["singer"][1] = singer(42, "Alpha"),
            2 => value["data"]["info"][1]["singer"][0]["singername"] = json!("Different"),
            3 => value["data"]["info"][1]["singer"][0]["singerid"] = json!("042"),
            4 => value["data"]["info"][1]["singer"][0]["singerid"] = json!(0),
            5 => value["data"]["info"][1]["title"] = json!("unexpected"),
            6 => {
                value["data"]["info"][0]["singer"] =
                    json!([singer(42, "Alpha"), singer(42, "Alpha")])
            }
            _ => unreachable!(),
        }
        assert!(
            parse_catalogue(value.to_string().as_bytes(), &Default::default()).is_err(),
            "case {case}"
        );
    }
}

#[test]
fn artist_catalog_requires_valid_business_envelope_and_selected_filter_availability() {
    for case in 0..5 {
        let mut value = catalogue();
        match case {
            0 => value["status"] = json!(0),
            1 => value["errcode"] = json!(20006),
            2 => value["data"]["enu_list"]["types"][0]["key"] = json!(9),
            3 => value["data"]["enu_list"]["sextypes"][0]["key"] = json!(9),
            4 => {
                value["data"]["enu_list"]["types"][1] =
                    value["data"]["enu_list"]["types"][0].clone()
            }
            _ => unreachable!(),
        }
        assert!(parse_catalogue(value.to_string().as_bytes(), &Default::default()).is_err());
    }
    let mut value = catalogue();
    value["data"]["info"] = json!([]);
    let parsed = parse_catalogue(value.to_string().as_bytes(), &Default::default()).unwrap();
    assert!(parsed.artists.is_empty() && parsed.featured_artists.is_empty());
}

#[test]
fn artist_catalog_bounds_hot_section_and_rejects_unsupported_inputs() {
    let mut value = catalogue();
    value["data"]["info"][0]["singer"] =
        json!((1..=201).map(|id| singer(id, "Artist")).collect::<Vec<_>>());
    assert!(parse_catalogue(value.to_string().as_bytes(), &Default::default()).is_err());
    for request in [
        ArtistCatalogRequest {
            account: Some("default".into()),
            ..Default::default()
        },
        ArtistCatalogRequest {
            area: ArtistArea::HongKongTaiwan,
            ..Default::default()
        },
        ArtistCatalogRequest {
            area: ArtistArea::JapaneseKorean,
            ..Default::default()
        },
        ArtistCatalogRequest {
            genre: ArtistGenre::Pop,
            ..Default::default()
        },
    ] {
        assert_eq!(
            selection(&request).unwrap_err().code,
            ErrorCode::InvalidRequest
        );
    }
}
