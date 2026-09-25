use super::*;

pub(crate) fn payload() -> Value {
    json!({
        "status":1,"error_code":0,"ErrorCode":0,
        "data":[
            {"LableName":"","RecordCount":3,"RecordDatas":[
                {"HintInfo":"Same","IsRadio":1,"jump":{"url":"https://unused.invalid/"}},
                {"HintInfo":" Same ","id":999,"hash":"not-a-track-id"},
                {"HintInfo":"A & <B>","token":"do-not-export"}
            ]},
            {"LableName":"MV","RecordCount":1,"RecordDatas":[{"HintInfo":"Same"}]},
            {"LableName":"专辑","RecordCount":1,"RecordDatas":[{"HintInfo":"Ignored album"}]}
        ],
        "SingerShortcut":{"id":42,"name":"Not a keyword"},
        "Shortcuts":[{"token":"do-not-export"}]
    })
}

pub(crate) fn jsonp(value: &Value) -> String {
    format!("{CALLBACK}({value})")
}

#[test]
fn search_suggestions_preserve_group_order_duplicates_and_literal_hints_without_resources() {
    let result = parse(jsonp(&payload()).as_bytes(), "Query").unwrap();
    assert_eq!(result.query, "Query");
    assert_eq!(result.client, SearchSuggestionClient::Web);
    assert_eq!(
        result
            .suggestions
            .iter()
            .map(|v| (v.keyword.as_str(), v.kind))
            .collect::<Vec<_>>(),
        [
            ("Same", Some(SearchKind::Track)),
            ("Same", Some(SearchKind::Track)),
            ("A & <B>", Some(SearchKind::Track)),
            ("Same", Some(SearchKind::Mv)),
        ]
    );
    assert_eq!(
        result.suggestions[1].display_text.as_deref(),
        Some(" Same ")
    );
    assert!(result.suggestions.iter().all(|v| v.resource.is_none()));
    assert!(result.suggestions.iter().all(|v| v.icon_url.is_none()));
    assert!(result.recommendations.is_empty());
    let serialized = serde_json::to_string(&result).unwrap();
    for ignored in [
        "do-not-export",
        "Not a keyword",
        "Ignored album",
        "unused.invalid",
    ] {
        assert!(!serialized.contains(ignored));
    }
}

#[test]
fn search_suggestions_jsonp_accepts_only_the_fixed_callback_and_one_json_value() {
    let valid = jsonp(&payload());
    assert!(parse(format!(" \n{valid}; \n").as_bytes(), "Q").is_ok());
    for text in [
        payload().to_string(),
        valid.replacen(CALLBACK, "otherCallback", 1),
        format!("global.{valid}"),
        format!("{valid};doSomething()"),
        format!("{valid};{valid}"),
        format!("{CALLBACK}({}, {{}})", payload()),
        valid.replace("\"status\":1", "\"status\":1,\"status\":1"),
        format!("{valid};;"),
    ] {
        assert!(parse(text.as_bytes(), "Q").is_err(), "{text}");
    }
    assert!(parse(&[0xff], "Q").is_err());
    assert!(parse(&vec![b' '; RESPONSE_LIMIT + 1], "Q").is_err());
}

#[test]
fn search_suggestions_reject_business_errors_bad_groups_and_malformed_hints() {
    for case in 0..13 {
        let mut value = payload();
        match case {
            0 => value = json!({"status":0,"error_code":20001,"data":"private-upstream-message"}),
            1 => value["ErrorCode"] = json!(2),
            2 => {
                value["data"].as_array_mut().unwrap().pop();
            }
            3 => value["data"][1]["LableName"] = json!("专辑"),
            4 => value["data"][0]["RecordCount"] = json!(4),
            5 => value["data"][1]["RecordCount"] = json!(-1),
            6 => {
                value["data"][0]["RecordDatas"] = json!(vec![json!({"HintInfo":"Q"}); 6]);
                value["data"][0]["RecordCount"] = json!(6);
            }
            7 => value["data"][0]["RecordDatas"][0] = json!({}),
            8 => value["data"][0]["RecordDatas"][0]["HintInfo"] = json!(" "),
            9 => value["data"][0]["RecordDatas"][0]["HintInfo"] = json!("a\nb"),
            10 => value["data"][0]["RecordDatas"][0]["HintInfo"] = json!("a".repeat(1025)),
            11 => value["data"][0]["RecordDatas"][0]["HintInfo"] = Value::Null,
            12 => value["data"][0]["RecordCount"] = json!("3"),
            _ => unreachable!(),
        }
        let error = parse(jsonp(&value).as_bytes(), "Q").unwrap_err();
        assert_eq!(error.code, ErrorCode::UpstreamError, "case {case}");
        assert!(!format!("{error:?}").contains("private-upstream-message"));
    }
}

#[test]
fn search_suggestions_empty_groups_are_success_without_fabricated_recommendations() {
    let mut value = payload();
    for group in value["data"].as_array_mut().unwrap() {
        group["RecordDatas"] = json!([]);
        group["RecordCount"] = json!(0);
    }
    let result = parse(jsonp(&value).as_bytes(), "No match").unwrap();
    assert!(result.suggestions.is_empty());
    assert!(result.recommendations.is_empty());
}
