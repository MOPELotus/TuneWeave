use super::*;
use serde_json::Value;

pub(crate) fn reply() -> Value {
    json!({"code":"000000","data":{
        "singerList":[{"singerName":"Artist One","singerId":"not-an-identity"},
            {"songName":"Preferred Name","singerName":"Unused Name"}],
        "songList":[{"songName":"Song One","contentId":"unverified","highlightStr":["<script>private()</script>"]},
            {"songName":"","singerName":"Fallback Name"},{"songName":"Song One"}],
        "recommendations":[{"name":"unproved"}]}})
}

fn parse_value(value: Value) -> Result<SearchSuggestionList> {
    parse(&serde_json::to_vec(&value).unwrap(), "query")
}

#[test]
fn pc_search_suggestions_preserve_consumer_order_duplicates_and_name_precedence() {
    let result = parse_value(reply()).unwrap();
    assert_eq!(result.client, SearchSuggestionClient::Pc);
    assert_eq!(result.query, "query");
    assert_eq!(
        result
            .suggestions
            .iter()
            .map(|v| v.keyword.as_str())
            .collect::<Vec<_>>(),
        [
            "Artist One",
            "Preferred Name",
            "Song One",
            "Fallback Name",
            "Song One"
        ]
    );
    assert_eq!(result.suggestions[0].kind, Some(SearchKind::Artist));
    assert_eq!(result.suggestions[2].kind, Some(SearchKind::Track));
    assert!(
        result
            .suggestions
            .iter()
            .all(|v| v.resource.is_none() && v.display_text.is_none() && v.icon_url.is_none())
    );
    assert!(result.recommendations.is_empty());
    let serialized = serde_json::to_string(&result).unwrap();
    for ignored in [
        "singerId",
        "contentId",
        "highlightStr",
        "script",
        "unproved",
        "Unused Name",
    ] {
        assert!(!serialized.contains(ignored), "leaked {ignored}");
    }
}

#[test]
fn pc_search_suggestions_empty_lists_require_an_exact_successful_ack() {
    for value in [
        json!({"code":"000000"}),
        json!({"code":"000000","data":null}),
        json!({"code":"000000","data":{}}),
        json!({"code":"000000","data":{"singerList":null,"songList":[]}}),
    ] {
        assert!(parse_value(value).unwrap().suggestions.is_empty());
    }
    for value in [
        json!({"code":"999999","info":"private error","data":null}),
        json!({"data":{}}),
        json!({"code":0,"data":{}}),
        json!({"code":"000000","data":[]}),
        json!({"code":"000000","data":{"songList":false}}),
    ] {
        let error = parse_value(value).unwrap_err();
        assert_eq!(error.code, ErrorCode::UpstreamError);
        assert!(!error.message.contains("private error"));
    }
}

#[test]
fn pc_search_suggestions_invalid_tail_never_becomes_empty_or_partial_success() {
    for value in [
        json!({}),
        json!({"songName":""}),
        json!({"songName":"  "}),
        json!({"songName":"private\nword"}),
        json!({"songName":12}),
        json!({"songName":[],"singerName":"fallback"}),
        json!({"songName":"valid","singerName":false}),
        json!({"songName":"x".repeat(2049)}),
    ] {
        let mut body = reply();
        body["data"]["songList"].as_array_mut().unwrap().push(value);
        assert!(parse_value(body).is_err());
    }
}

#[test]
fn pc_search_suggestions_combined_limit_preserves_all_or_rejects_all() {
    let row = json!({"songName":"Repeated keyword"});
    let mut body = json!({"code":"000000","data":{"singerList":vec![row.clone(); 100],"songList":vec![row.clone(); 100]}});
    assert_eq!(parse_value(body.clone()).unwrap().suggestions.len(), 200);
    body["data"]["songList"].as_array_mut().unwrap().push(row);
    assert!(parse_value(body).is_err());
}
