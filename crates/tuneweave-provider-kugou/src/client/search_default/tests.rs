use super::*;

pub(crate) fn payload() -> Value {
    json!({"status":1,"error_code":0,"data":{"timestamp":1790166855,"ads":[
        {"main_title":" First & <Song> ","sub_title":"New release","id":999,
         "jump_url":"https://unused.invalid/","token":"do-not-export"},
        {"main_title":"Later","sub_title":"Not initially visible"},
        {"main_title":" First & <Song> ","sub_title":"Duplicate"}
    ]}})
}

#[test]
fn search_default_signs_exact_anonymous_body_and_rounds_seconds() {
    let mid = "51ffe03a61ac1743bc98078c6b7106be";
    let query = parameters(mid, Duration::from_millis(1_790_166_854_959));
    assert_eq!(query["signature"], "a63db202c4267a8eac4b4ea9c906209b");
    assert_eq!(query["uuid"], "1790166854959");
    assert_eq!(query["clienttime"], "1790166855");
    assert_eq!(query["mid"], mid);
    assert_eq!(query.len(), 8);
    for (millis, expected) in [(1_499, "1"), (1_500, "2"), (1_999, "2")] {
        assert_eq!(
            parameters(mid, Duration::from_millis(millis))["clienttime"],
            expected
        );
    }
}

#[test]
fn search_default_preserves_initial_title_without_promoting_ads_to_resources() {
    let result = parse(payload().to_string().as_bytes()).unwrap();
    assert_eq!(result.keyword, " First & <Song> ");
    assert_eq!(result.display_text, result.keyword);
    assert_eq!(result.kind, Some(SearchKind::Track));
    assert!(result.image_url.is_none());
    assert_eq!(result.extensions["subtitle"], "New release");
    assert_eq!(result.extensions["rotation_total"], 3);
    let serialized = serde_json::to_string(&result).unwrap();
    for ignored in [
        "999",
        "unused.invalid",
        "do-not-export",
        "Later",
        "Duplicate",
    ] {
        assert!(!serialized.contains(ignored));
    }
    let no_subtitle = json!({"status":1,"error_code":0,"data":{"ads":[{"main_title":"Q"}]}});
    assert!(
        !parse(no_subtitle.to_string().as_bytes())
            .unwrap()
            .extensions
            .contains_key("subtitle")
    );
}

#[test]
fn search_default_empty_success_is_not_found_and_business_failure_is_not_empty() {
    let empty = br#"{"status":1,"error_code":0,"data":{"timestamp":1790166855,"ads":[]}}"#;
    assert_eq!(parse(empty).unwrap_err().code, ErrorCode::ResourceNotFound);
    for value in [
        json!({"status":0,"error_code":20001,"msg":"private-upstream-message"}),
        json!({"status":1,"error_code":20001,"data":{"ads":[]}}),
    ] {
        let error = parse(value.to_string().as_bytes()).unwrap_err();
        assert_eq!(error.code, ErrorCode::UpstreamError);
        assert!(!format!("{error:?}").contains("private-upstream-message"));
    }
}

#[test]
fn search_default_rejects_malformed_titles_without_selecting_a_later_ad() {
    for bad in [
        json!(""),
        json!(" "),
        json!("a\nb"),
        json!("a".repeat(1025)),
        json!(42),
        Value::Null,
    ] {
        let mut value = payload();
        value["data"]["ads"][0]["main_title"] = bad;
        assert_eq!(
            parse(value.to_string().as_bytes()).unwrap_err().code,
            ErrorCode::UpstreamError
        );
    }
    for bad in [json!("a\nb"), json!("a".repeat(1025)), json!(42)] {
        let mut value = payload();
        value["data"]["ads"][0]["sub_title"] = bad;
        assert_eq!(
            parse(value.to_string().as_bytes()).unwrap_err().code,
            ErrorCode::UpstreamError
        );
    }
    for bad in [
        r#"{"status":1,"error_code":0}"#.to_owned(),
        r#"{"status":1,"error_code":0,"data":{"ads":null}}"#.to_owned(),
        r#"{"status":1,"status":1,"error_code":0,"data":{"ads":[]}}"#.to_owned(),
        format!("{}{{}}", payload()),
        " ".repeat(RESPONSE_LIMIT + 1),
    ] {
        assert_eq!(
            parse(bad.as_bytes()).unwrap_err().code,
            ErrorCode::UpstreamError
        );
    }
    assert_eq!(parse(&[0xff]).unwrap_err().code, ErrorCode::UpstreamError);
}
