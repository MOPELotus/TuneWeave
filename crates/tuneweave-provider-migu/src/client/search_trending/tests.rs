use super::*;
use serde_json::Value;

pub(crate) fn reply() -> Value {
    // An independent fixture executes the extracted PC filter/map and selection
    // consumer in Node. It verifies the distinction between raw value and word.
    json!({"code":"000000","data":{"hotWordItemList":[
        {"word":"猜你想搜"}, {"word":"Artist One"},
        {"word":"Discard","value":"年度听歌报告"},
        {"word":"Song One","score":999,"iconUrl":"https://outside.invalid/private"},
        {"word":"Artist One"}, {"word":"年度听歌报告","value":"something-else"}
    ]}})
}

fn parse_value(value: Value) -> Result<SearchTrendingList> {
    parse(
        &serde_json::to_vec(&value).unwrap(),
        SearchTrendingDetail::Full,
    )
}

#[test]
fn pc_search_trending_preserves_searchable_order_duplicates_and_exact_sentinels() {
    let result = parse_value(reply()).unwrap();
    assert_eq!(result.detail, SearchTrendingDetail::Full);
    assert_eq!(
        result
            .entries
            .iter()
            .map(|v| v.keyword.as_str())
            .collect::<Vec<_>>(),
        ["Artist One", "Song One", "Artist One", "年度听歌报告"]
    );
    assert_eq!(
        result.entries.iter().map(|v| v.rank).collect::<Vec<_>>(),
        [1, 2, 3, 4]
    );
    assert_eq!(result.extensions["rank_scope"], "searchable_response_order");
    assert_eq!(result.extensions["metadata_scope"], "keywords_only");
    assert!(result.entries.iter().all(|v| v.description.is_none()
        && v.score.is_none()
        && v.icon_type.is_none()
        && v.icon_url.is_none()
        && v.target_url.is_none()
        && v.extensions.is_empty()));
    let output = serde_json::to_string(&result).unwrap();
    for excluded in [
        "private",
        "outside.invalid",
        "999",
        "Discard",
        "something-else",
        "猜你想搜",
    ] {
        assert!(!output.contains(excluded), "leaked {excluded}");
    }
}

#[test]
fn pc_search_trending_empty_requires_exact_success_and_explicit_array() {
    for value in [
        json!({"code":"000000","data":{"hotWordItemList":[]}}),
        json!({"code":"000000","data":{"hotWordItemList":[{"word":"猜你想搜"},{"value":"年度听歌报告"}]}}),
    ] {
        assert!(parse_value(value).unwrap().entries.is_empty());
    }
    for value in [
        json!({"code":"000000"}),
        json!({"code":"000000","data":null}),
        json!({"code":"000000","data":{}}),
        json!({"code":"000000","data":{"hotWordItemList":null}}),
        json!({"code":"000000","data":{"hotWordItemList":{}}}),
        json!({"code":"111111","info":"private-error","data":{"hotWordItemList":[]}}),
        json!({"code":0,"data":{"hotWordItemList":[]}}),
        json!({"data":{"hotWordItemList":[]}}),
    ] {
        let error = parse_value(value).unwrap_err();
        assert_eq!(error.code, ErrorCode::UpstreamError);
        assert!(!error.message.contains("private-error"));
    }
}

#[test]
fn pc_search_trending_invalid_tail_never_returns_partial_success() {
    for row in [
        json!({}),
        json!({"word":null}),
        json!({"word":12}),
        json!({"word":""}),
        json!({"word":"  "}),
        json!({"word":"private\nword"}),
        json!({"word":"a".repeat(2049)}),
        json!(null),
    ] {
        let mut body = reply();
        body["data"]["hotWordItemList"]
            .as_array_mut()
            .unwrap()
            .push(row);
        assert!(parse_value(body).is_err());
    }
    // Raw value is not an identity, title or navigation target. Its exact annual
    // marker alone is consumed; unknown optional fields are never re-emitted.
    let result = parse_value(json!({"code":"000000","data":{"hotWordItemList":[
        {"word":"Word", "value":{"private":"ignored"}, "resourceId":"unproved"}
    ]}}))
    .unwrap();
    assert_eq!(result.entries[0].keyword, "Word");
    assert!(!serde_json::to_string(&result).unwrap().contains("private"));
}

#[test]
fn pc_search_trending_row_limit_applies_before_sentinel_filtering() {
    let mut body =
        json!({"code":"000000","data":{"hotWordItemList":vec![json!({"word":"Word"});200]}});
    assert_eq!(parse_value(body.clone()).unwrap().entries.len(), 200);
    body["data"]["hotWordItemList"]
        .as_array_mut()
        .unwrap()
        .push(json!({"value":"年度听歌报告"}));
    assert!(parse_value(body).is_err());
}
