use super::*;

fn entry(id: &str, name: &str, channel_id: &str, priority: &str) -> serde_json::Value {
    json!({
        "id":id,
        "artistName":name,
        "headPic":"https://star.kuwo.cn/star/starheads/180/46/78/2274127599.jpg",
        "channelId":channel_id,
        "channelName":"Station",
        "priority":priority,
        "status":"1"
    })
}

fn parse(value: &serde_json::Value) -> Result<Vec<Artist>> {
    super::parse(&serde_json::to_vec(value).unwrap())
}

#[test]
fn catalogue_preserves_anchor_order_and_separate_artist_and_radio_identities() {
    let value = json!({"code":200,"data":{"banner":{},"list":[
        entry("3313276", "中国有声阅读", "0", "100"),
        entry("3003400", "大飞（主播）", "664", "100")
    ]}});
    let artists = parse(&value).unwrap();
    assert_eq!(artists.len(), 2);
    assert_eq!(artists[0].resource_ref.to_string(), "kuwo:3313276");
    assert_eq!(artists[0].extensions["channel_id"], "0");
    assert!(!artists[0].extensions.contains_key("radio_channel_ref"));
    assert_eq!(artists[1].resource_ref.to_string(), "kuwo:3003400");
    assert_eq!(artists[1].extensions["radio_channel_ref"], "kuwo:fm:664");
    assert_eq!(artists[1].extensions["source_priority"], "100");
    assert_eq!(artists[1].extensions["source_status"], "1");
    assert!(artists.iter().all(|artist| {
        artist.track_count.is_none()
            && artist.album_count.is_none()
            && artist.mv_count.is_none()
            && artist.video_count.is_none()
            && artist
                .avatar_url
                .as_deref()
                .is_some_and(|url| url.starts_with("https://star.kuwo.cn/"))
    }));
    assert_eq!(
        artists[0].extensions["catalog_scope"],
        "baicheng_sound_anchors"
    );
}

#[test]
fn catalogue_accepts_an_explicit_empty_list_but_rejects_missing_or_ambiguous_rows() {
    assert!(
        parse(&json!({"code":200,"data":{"list":[]}}))
            .unwrap()
            .is_empty()
    );
    for value in [
        json!({}),
        json!({"code":200,"data":{}}),
        json!({"code":200,"data":{"list":null}}),
        json!({"code":200,"data":{"list":[entry("3003400", "主播", "664", "1"), entry("3003400", "重复", "664", "2")]}}),
        json!({"code":200,"data":{"list":[entry("03003400", "主播", "664", "1")]}}),
        json!({"code":200,"data":{"list":[entry("3003400", " ", "664", "1")]}}),
        json!({"code":200,"data":{"list":[entry("3003400", "主播", "0664", "1")]}}),
        json!({"code":503,"data":{"list":[]}}),
    ] {
        assert!(parse(&value).is_err(), "{value}");
    }

    let mut untrusted_image =
        json!({"code":200,"data":{"list":[entry("3003400", "主播", "0", "1")]}});
    untrusted_image["data"]["list"][0]["headPic"] = json!("https://example.test/avatar.jpg");
    let artists = parse(&untrusted_image).unwrap();
    assert!(artists[0].avatar_url.is_none());

    let mut oversized = Vec::new();
    for number in 1..=super::MAX_ANCHORS + 1 {
        oversized.push(entry(&number.to_string(), "主播", "0", "1"));
    }
    assert!(parse(&json!({"code":200,"data":{"list":oversized}})).is_err());
}
