use super::*;

pub(crate) fn modules(white: Value, black: Value) -> Value {
    json!({"resourceModuleQueryList":[{"resourceId":"112","resourceType":"2002","whiteList":white,"blackList":black}]})
}
pub(crate) fn row(id: &str) -> Value {
    json!({"view":"ZJ-Singer-Item","resId":id,"resType":"2002","txt2":id,"txt":format!("Artist {id}"),"action":format!("mgmusic://singer-info?id={id}"),"img":"https://d.musicapp.migu.cn/data/oss/resource/00/test.webp","txt4":"123","private":"unused"})
}
pub(crate) fn index() -> Value {
    json!({"header":{"nextPageNo":1,"nextPageNo2":1,"update":false},"contents":[
        {"view":"ZJ-Singer-Intro-Scroll","contents":[{"txt":"Biography","txt2":"private-not-returned"}]},
        {"view":"ZJ-Title","contents":[{"txt":"相似歌手"}]},
        {"view":"ZJ-Singer-Scroll","contents":[row("266"),row("99"),row("270")]}
    ]})
}

#[test]
fn similar_artists_module_policy_binds_identity_and_respects_whitelist_precedence() {
    for (white, black, allowed) in [
        (json!(null), json!(null), true),
        (json!([]), json!([]), true),
        (json!([]), json!(["similarSinger"]), false),
        (json!(["similarSinger"]), json!(["similarSinger"]), true),
        (json!([]), json!(["otherFunction"]), true),
    ] {
        assert_eq!(
            module_allowed(modules(white, black), "112").unwrap(),
            allowed
        );
    }
    for value in [
        json!({}),
        json!({"resourceModuleQueryList":[]}),
        json!({"resourceModuleQueryList":[{"resourceId":"999","resourceType":"2002"}]}),
        json!({"resourceModuleQueryList":[{"resourceId":"112","resourceType":"2"}]}),
        modules(json!(false), json!([])),
        modules(json!([]), json!(["a", "a"])),
        modules(json!([]), json!(["bad\nflag"])),
        modules(json!(vec!["x"; 129]), json!([])),
    ] {
        assert!(module_allowed(value, "112").is_err());
    }
}

#[test]
fn similar_artists_index_preserves_order_and_uses_only_verified_resource_fields() {
    let artists = catalogue(index(), "112").unwrap();
    assert_eq!(
        artists.iter().map(|a| a.id.as_str()).collect::<Vec<_>>(),
        ["266", "99", "270"]
    );
    assert!(artists.iter().all(|a| a.resource_ref.id() == a.id
        && a.track_count.is_none()
        && a.album_count.is_none()
        && a.mv_count.is_none()
        && a.identities.is_empty()));
    assert!(!serde_json::to_string(&artists).unwrap().contains("private"));
    let mut empty = index();
    empty["contents"][2]["contents"] = json!([]);
    assert!(catalogue(empty, "112").unwrap().is_empty());
}

#[test]
fn similar_artists_invalid_tail_or_changed_layout_never_becomes_partial_success() {
    for (key, value) in [
        ("resId", json!("266")),
        ("resId", json!("112")),
        ("txt2", json!("999")),
        ("resType", json!("1")),
        ("action", json!("mgmusic://singer-info?id=270&id=270")),
        ("action", json!("https://outside.invalid/270")),
        ("txt", json!("\nname")),
        (
            "img",
            json!("http://d.musicapp.migu.cn/data/oss/resource/a.webp"),
        ),
        ("img", json!("https://outside.invalid/a.webp")),
        (
            "img",
            json!("https://d.musicapp.migu.cn/data/oss/resource/a.webp?private=1"),
        ),
    ] {
        let mut body = index();
        body["contents"][2]["contents"][2][key] = value;
        assert!(catalogue(body, "112").is_err(), "{key}");
    }
    for field in [
        "nextPageUrl",
        "hasNext",
        "hasNextPage",
        "nextPageNo",
        "update",
    ] {
        let mut body = index();
        body["header"][field] = match field {
            "nextPageUrl" => json!("https://outside.invalid/next"),
            "nextPageNo" => json!(2),
            _ => json!(true),
        };
        assert!(catalogue(body, "112").is_err());
    }
    let mut body = index();
    body["contents"][2]["contents"] = json!(
        (1..=201)
            .map(|n| row(&(1000 + n).to_string()))
            .collect::<Vec<_>>()
    );
    assert!(catalogue(body, "112").is_err());
    let mut body = index();
    let node = body["contents"][2].clone();
    body["contents"].as_array_mut().unwrap().push(node);
    assert!(catalogue(body, "112").is_err());
    for contents in [
        json!([]),
        json!([{"view":"unknown"}]),
        json!([{"view":"ZJ-Singer-Scroll","contents":null}]),
    ] {
        assert!(catalogue(json!({"header":{},"contents":contents}), "112").is_err());
    }
}

#[test]
fn similar_artists_require_successful_object_envelopes() {
    for body in [
        json!({"code":"111111","info":"private","data":{}}),
        json!({"code":"000000","data":null}),
        json!({"code":0,"data":{}}),
        json!({"code":"000000"}),
    ] {
        let error = data(&serde_json::to_vec(&body).unwrap()).unwrap_err();
        assert_eq!(error.code, ErrorCode::UpstreamError);
        assert!(!error.message.contains("private"));
    }
}
