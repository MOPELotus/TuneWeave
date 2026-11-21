use serde_json::{Map, Value, json};
use tuneweave_core::{ErrorCode, Platform, Result, TuneWeaveError};
use url::Url;

pub(crate) fn parse(bytes: &[u8], uid: u64) -> Result<Vec<Value>> {
    let response: Value = serde_json::from_slice(bytes).map_err(|_| invalid_response())?;
    let code = response["code"].as_i64().ok_or_else(invalid_response)?;
    if code != 0 {
        let kind = match code {
            -101 => ErrorCode::AuthenticationRequired,
            -403 => ErrorCode::PermissionDenied,
            -404 => ErrorCode::ResourceNotFound,
            -352 | -412 | -429 | -509 => ErrorCode::RateLimited,
            _ => ErrorCode::UpstreamError,
        };
        return Err(
            TuneWeaveError::new(kind, "Bilibili space feed API rejected the request")
                .with_platform(Platform::Bilibili)
                .retryable(kind == ErrorCode::RateLimited)
                .with_details(json!({ "api_code": code })),
        );
    }
    let items = response["data"]["items"]
        .as_array()
        .ok_or_else(invalid_response)?;
    Ok(items
        .iter()
        .filter_map(|item| project_item(item, Some(uid)))
        .collect())
}

fn invalid_response() -> TuneWeaveError {
    TuneWeaveError::new(
        ErrorCode::UpstreamError,
        "Invalid Bilibili space feed response",
    )
    .with_platform(Platform::Bilibili)
}

fn decimal(value: &Value) -> Option<u64> {
    value.as_u64().or_else(|| value.as_str()?.parse().ok())
}

fn project_item(value: &Value, requested_author: Option<u64>) -> Option<Value> {
    let id = match &value["id_str"] {
        Value::String(id)
            if id.len() <= 22
                && id.bytes().all(|byte| byte.is_ascii_digit())
                && id.bytes().any(|byte| byte != b'0') =>
        {
            id.to_owned()
        }
        Value::Number(id) => id.as_u64().filter(|id| *id != 0)?.to_string(),
        _ => return None,
    };
    let kind = value["type"].as_str()?;
    if !matches!(
        kind,
        "DYNAMIC_TYPE_AV"
            | "DYNAMIC_TYPE_WORD"
            | "DYNAMIC_TYPE_DRAW"
            | "DYNAMIC_TYPE_ARTICLE"
            | "DYNAMIC_TYPE_FORWARD"
            | "DYNAMIC_TYPE_LIVE_RCMD"
    ) {
        return None;
    }
    let modules = &value["modules"];
    let author = &modules["module_author"];
    let mid = decimal(&author["mid"]).filter(|mid| *mid != 0)?;
    if requested_author.is_some_and(|uid| mid != uid) {
        return None;
    }
    let dynamic = &modules["module_dynamic"];
    let stat = &modules["module_stat"];
    let orig = (requested_author.is_some() && kind == "DYNAMIC_TYPE_FORWARD")
        .then(|| project_item(&value["orig"], None))
        .flatten();
    Some(json!({
        "id_str": id,
        "type": kind,
        "modules": {
            "module_author": {
                "mid": mid.to_string(),
                "name": author["name"].as_str(),
                "pub_ts": decimal(&author["pub_ts"]),
                "face": link(&author["face"]),
            },
            "module_dynamic": {
                "desc": description(&dynamic["desc"]),
                "major": major(&dynamic["major"]),
            },
            "module_stat": {
                "like": { "count": count(&stat["like"]["count"]) },
                "comment": { "count": count(&stat["comment"]["count"]) },
                "forward": { "count": count(&stat["forward"]["count"]) },
            },
        },
        "orig": orig,
    }))
}

fn description(value: &Value) -> Value {
    if let Some(text) = value.as_str() {
        return json!(text);
    }
    let nodes = match &value["rich_text_nodes"] {
        Value::Array(nodes) => Value::Array(
            nodes
                .iter()
                .filter_map(|node| node["text"].as_str().map(|text| json!({"text": text})))
                .collect(),
        ),
        Value::String(text) => json!(text),
        _ => Value::Null,
    };
    json!({ "title": value["title"].as_str(), "text": value["text"].as_str(), "rich_text_nodes": nodes })
}

fn pictures(value: &Value) -> Option<Vec<Value>> {
    Some(
        value
            .as_array()?
            .iter()
            .filter_map(|picture| {
                let url = link(&picture["url"]).or_else(|| link(&picture["src"]))?;
                Some(json!({"url": url}))
            })
            .collect(),
    )
}

fn major(value: &Value) -> Map<String, Value> {
    let mut projected = Map::new();
    if let Some(opus) = value.get("opus").filter(|item| item.is_object()) {
        projected.insert(
            "opus".into(),
            json!({
                "title": opus["title"].as_str(),
                "summary": description(&opus["summary"]),
                "pics": pictures(&opus["pics"]),
                "jump_url": link(&opus["jump_url"]),
            }),
        );
    }
    if let Some(draw) = value.get("draw").filter(|item| item.is_object()) {
        projected.insert("draw".into(), json!({ "items": pictures(&draw["items"]) }));
    }
    for kind in ["archive", "article"] {
        if let Some(item) = value.get(kind).filter(|item| item.is_object()) {
            let covers = item["covers"]
                .as_array()
                .map(|covers| covers.iter().filter_map(link).collect::<Vec<_>>());
            projected.insert(kind.into(), json!({
                "title": item["title"].as_str(),
                "desc": description(&item["desc"]),
                "jump_url": link(&item["jump_url"]),
                "cover": link(&item["cover"]),
                "covers": covers,
                "stat": { "play": count(&item["stat"]["play"]), "danmaku": count(&item["stat"]["danmaku"]) },
            }));
        }
    }
    if let Some(content) = value["live_rcmd"]["content"]
        .as_str()
        .and_then(|content| serde_json::from_str::<Value>(content).ok())
    {
        let info = live_info(&content["live_play_info"]);
        projected.insert(
            "live_rcmd".into(),
            json!({"content": json!({"live_play_info": info}).to_string()}),
        );
    }
    if let Some(live) = value.get("live").filter(|item| item.is_object()) {
        projected.insert("live".into(), live_info(live));
    }
    projected
}

fn live_info(value: &Value) -> Value {
    json!({
        "title": value["title"].as_str(),
        "link": link(&value["link"]),
        "jump_url": link(&value["jump_url"]),
        "cover": link(&value["cover"]),
        "parent_area_name": value["parent_area_name"].as_str(),
        "area_name": value["area_name"].as_str(),
        "live_start_time": decimal(&value["live_start_time"]),
        "watched_show": { "num": count(&value["watched_show"]["num"]) },
    })
}

fn count(value: &Value) -> Value {
    if let Some(number) = decimal(value) {
        return json!(number);
    }
    json!(value.as_str().filter(|text| {
        !text.is_empty()
            && text.len() <= 24
            && text
                .chars()
                .all(|c| c.is_ascii_digit() || matches!(c, '.' | '万' | '亿'))
    }))
}

fn link(value: &Value) -> Option<String> {
    let raw = value.as_str().filter(|url| url.len() <= 4096)?;
    let mut url = if raw.starts_with("//") {
        Url::parse(&format!("https:{raw}"))
    } else {
        Url::parse(raw)
    }
    .ok()?;
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
    {
        return None;
    }
    let host = url.host_str()?;
    if !["bilibili.com", "b23.tv", "hdslb.com", "biliimg.com"]
        .iter()
        .any(|domain| {
            host == *domain
                || host
                    .strip_suffix(domain)
                    .is_some_and(|prefix| prefix.ends_with('.'))
        })
    {
        return None;
    }
    url.set_scheme("https").ok()?;
    url.set_query(None);
    url.set_fragment(None);
    Some(url.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selects_author_and_discards_non_contract_fields() {
        let orig = json!({
            "id_str": "11", "type": "DYNAMIC_TYPE_DRAW",
            "modules": {
                "module_author": {"mid": 4, "name": "other", "Cookie": "local-secret"},
                "module_dynamic": {"major": {"draw": {"items": [
                    {"src":"//i0.hdslb.com/image.jpg?refresh_token=local-secret"},
                    {"url":"https://bilibili.com.example/image.jpg"},
                    {"url":"https://secret@i0.hdslb.com/image.jpg"}
                ]}}}
            },
            "orig": { "id_str": "12", "type": "DYNAMIC_TYPE_WORD" },
        });
        let item = json!({
            "id_str": "90", "type": "DYNAMIC_TYPE_FORWARD",
            "modules": {"module_author": {"mid": 3, "name":"author"}, "private": "local-secret"},
            "orig": orig,
            "refresh_token": "local-secret",
        });
        let fixture = json!({"code":0,"data":{"items":[item,orig]}});
        let result = parse(fixture.to_string().as_bytes(), 3).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0]["id_str"], "90");
        assert_eq!(result[0]["orig"]["modules"]["module_author"]["mid"], "4");
        assert_eq!(
            result[0]["orig"]["modules"]["module_dynamic"]["major"]["draw"]["items"],
            json!([
                {"url":"https://i0.hdslb.com/image.jpg"}
            ])
        );
        assert!(result[0]["orig"]["orig"].is_null());
        assert!(!json!(result).to_string().contains("local-secret"));
    }

    #[test]
    fn preserves_text_and_gallery_fallbacks_when_opus_fields_are_missing() {
        let item = json!({
            "id_str": "1234567890123456789012", "type": "DYNAMIC_TYPE_DRAW",
            "modules": {
                "module_author": { "mid": "3", "pub_ts": 1700000000 },
                "module_dynamic": {
                    "desc": { "text": "fallback text" },
                    "major": {
                        "opus": { "title": "title" },
                        "draw": { "items": [{ "src": "//i0.hdslb.com/pic.jpg" }] },
                    },
                }
            }
        });
        let fixture = json!({"code":0,"data":{"items":[item]}});
        let result = parse(fixture.to_string().as_bytes(), 3).unwrap();
        assert_eq!(result[0]["id_str"], "1234567890123456789012");
        let dynamic = &result[0]["modules"]["module_dynamic"];
        assert!(dynamic["desc"]["rich_text_nodes"].is_null());
        assert_eq!(dynamic["desc"]["text"], "fallback text");
        assert!(dynamic["major"]["opus"]["pics"].is_null());
        assert_eq!(
            dynamic["major"]["draw"]["items"][0]["url"],
            "https://i0.hdslb.com/pic.jpg"
        );
    }

    #[test]
    fn preserves_upstream_authentication_and_rate_limits() {
        for (code, expected) in [
            (-101, ErrorCode::AuthenticationRequired),
            (-412, ErrorCode::RateLimited),
            (-403, ErrorCode::PermissionDenied),
        ] {
            let fixture = json!({"code":code,"message":"local-secret"});
            let result = parse(fixture.to_string().as_bytes(), 3).unwrap_err();
            assert_eq!(result.code, expected);
            assert!(!result.to_string().contains("local-secret"));
        }
    }
}
