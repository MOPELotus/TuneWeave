use super::*;
use tuneweave_core::{
    CollectedPodcastFeedItem, PodcastCategoryBranch, PodcastSubcategory,
    validate_upstream_media_url,
};

const CATEGORY_TREE: &str = "/api/dj/radio/category/list";
const DISLIKE_LIST: &str = "/api/music/dislike/get";
const COLLECTED_FEED: &str = "/api/mlog/mycollect/bytime/get";

impl NeteaseProvider {
    pub(super) async fn read_podcast_category_tree(
        &self,
        account: Option<&str>,
    ) -> Result<PodcastCategoryTree> {
        let client = if account.is_some() {
            let client = self.client_for(account)?;
            require_authenticated_client(&client, "podcast category tree")?;
            client
        } else {
            self.anonymous_discovery_client.clone()
        };
        let response = client.request_xeapi(CATEGORY_TREE, json!({})).await;
        if client.is_authenticated() {
            self.check_discovery_session(account, &client)?;
        }
        let response = response?;
        check_success(&response.body, "podcast category tree")?;
        parse_category_tree(&response.body)
    }

    pub(super) async fn read_recommendation_dislikes(
        &self,
        account: Option<&str>,
    ) -> Result<RecommendationDislikeList> {
        let body = self
            .read_private_discovery(
                account,
                DISLIKE_LIST,
                json!({}),
                "recommendation dislike list",
            )
            .await?;
        parse_dislike_list(&body)
    }

    pub(super) async fn read_collected_podcast_feed(
        &self,
        request: &CollectedPodcastFeedRequest,
    ) -> Result<CollectedPodcastFeedPage> {
        let cursor = request.cursor.as_deref().map(parse_cursor).transpose()?;
        if !(1..=100).contains(&request.limit) {
            return Err(TuneWeaveError::invalid_request(
                "NetEase collected podcast feed limit must be 1 to 100",
            )
            .with_platform(Platform::Netease));
        }

        let body = self
            .read_private_discovery(
                request.account.as_deref(),
                COLLECTED_FEED,
                json!({
                    "time": request.cursor.as_deref().unwrap_or("-1"),
                    "limit": request.limit,
                    "mlogType": 3,
                }),
                "collected podcast feed",
            )
            .await?;
        parse_collected_feed(&body, request.limit, cursor)
    }

    async fn read_private_discovery(
        &self,
        account: Option<&str>,
        path: &str,
        payload: Value,
        operation: &str,
    ) -> Result<Value> {
        let client = self.client_for(account)?;
        require_authenticated_client(&client, operation)?;
        let response = client.request_xeapi(path, payload).await;
        self.check_discovery_session(account, &client)?;
        let response = response?;
        check_success(&response.body, operation)?;
        Ok(response.body)
    }

    fn check_discovery_session(
        &self,
        account: Option<&str>,
        original: &NeteaseClient,
    ) -> Result<()> {
        if self.client_for(account)?.same_upload_session(original) {
            Ok(())
        } else {
            Err(TuneWeaveError::new(
                ErrorCode::Conflict,
                "NetEase account changed during discovery lookup",
            )
            .with_platform(Platform::Netease))
        }
    }
}

fn check_success(body: &Value, operation: &str) -> Result<()> {
    ensure_success_for(body, &format!("NetEase {operation} request failed"))
}

fn malformed(operation: &str, field: &str) -> TuneWeaveError {
    TuneWeaveError::new(
        ErrorCode::UpstreamError,
        format!("NetEase {operation} returned invalid {field}"),
    )
    .with_platform(Platform::Netease)
}

fn category_id(value: &Value) -> Option<String> {
    // IDs such as "0020141111" and the branch-local "000" are opaque strings.
    match value {
        Value::String(id)
            if !id.is_empty() && id.len() <= 64 && id.bytes().all(|b| b.is_ascii_digit()) =>
        {
            Some(id.clone())
        }
        Value::Number(value) => value.as_u64().map(|number| number.to_string()),
        _ => None,
    }
}

fn visible_text(value: &Value) -> Option<String> {
    value
        .as_str()
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
}

fn cover_url(value: &Value) -> Option<String> {
    let url = value.as_str()?;
    validate_upstream_media_url(url).ok()?;
    Some(url.to_owned())
}

fn parse_category_tree(body: &Value) -> Result<PodcastCategoryTree> {
    const OP: &str = "podcast category tree";
    let branches = body
        .pointer("/data/parentCategoryVoList")
        .and_then(Value::as_array)
        .ok_or_else(|| malformed(OP, "parent category list"))?;
    let categories = branches
        .iter()
        .map(|branch| {
            let id = category_id(&branch["id"]).ok_or_else(|| malformed(OP, "category ID"))?;
            let name =
                visible_text(&branch["name"]).ok_or_else(|| malformed(OP, "category name"))?;
            let children = branch["subCategoryVO"]
                .as_array()
                .ok_or_else(|| malformed(OP, "subcategory list"))?
                .iter()
                .map(|child| {
                    Ok(PodcastSubcategory {
                        id: category_id(&child["id"])
                            .ok_or_else(|| malformed(OP, "subcategory ID"))?,
                        name: visible_text(&child["name"])
                            .ok_or_else(|| malformed(OP, "subcategory name"))?,
                    })
                })
                .collect::<Result<_>>()?;
            Ok(PodcastCategoryBranch {
                id,
                name,
                description: visible_text(&branch["desc"]),
                cover_url: cover_url(&branch["coverUrl"]),
                children,
            })
        })
        .collect::<Result<_>>()?;
    Ok(PodcastCategoryTree { categories })
}

fn parse_dislike_list(body: &Value) -> Result<RecommendationDislikeList> {
    const OP: &str = "recommendation dislike list";
    let data = &body["data"];
    let ids = |field: &str| -> Result<Vec<String>> {
        data[field]
            .as_array()
            .ok_or_else(|| malformed(OP, field))?
            .iter()
            .map(|value| {
                category_id(value)
                    .filter(|id| id.bytes().any(|b| b != b'0'))
                    .ok_or_else(|| malformed(OP, field))
            })
            .collect()
    };
    let checkpoint = match &data["checkPoint"] {
        Value::Null => None,
        Value::String(value) if value.is_empty() => None,
        Value::String(value) if value.len() <= 512 && !value.chars().any(char::is_control) => {
            Some(value.clone())
        }
        Value::Number(value) if value.is_i64() || value.is_u64() => Some(value.to_string()),
        _ => return Err(malformed(OP, "checkpoint")),
    };
    Ok(RecommendationDislikeList {
        song_ids: ids("songIdList")?,
        artist_ids: ids("artistIdList")?,
        podcast_ids: ids("podcastIds")?,
        checkpoint,
    })
}

fn parse_cursor(value: &str) -> Result<i64> {
    value
        .parse::<i64>()
        .ok()
        .filter(|time| *time > 0 && time.to_string() == value)
        .ok_or_else(|| {
            TuneWeaveError::invalid_request(
                "NetEase collected podcast feed cursor must be a positive decimal timestamp",
            )
            .with_platform(Platform::Netease)
        })
}

fn parse_collected_feed(
    body: &Value,
    limit: u32,
    cursor: Option<i64>,
) -> Result<CollectedPodcastFeedPage> {
    const OP: &str = "collected podcast feed";
    let data = &body["data"];
    let items = data["feeds"]
        .as_array()
        .filter(|rows| rows.len() <= limit as usize)
        .ok_or_else(|| malformed(OP, "feed list"))?
        .iter()
        .map(parse_feed_item)
        .collect::<Result<Vec<_>>>()?;
    let has_more = data["more"]
        .as_bool()
        .ok_or_else(|| malformed(OP, "continuation flag"))?;
    let next_cursor = if has_more {
        Some(
            json_i64(&data["time"])
                .filter(|time| *time > 0 && cursor.is_none_or(|previous| *time < previous))
                .ok_or_else(|| malformed(OP, "non-advancing continuation time"))?
                .to_string(),
        )
    } else {
        None
    };
    let total = match &data["totalCount"] {
        Value::Null => None,
        value => Some(json_u64(value).ok_or_else(|| malformed(OP, "total count"))?),
    };
    Ok(CollectedPodcastFeedPage {
        items,
        next_cursor,
        has_more,
        total,
    })
}

fn parse_feed_item(row: &Value) -> Result<CollectedPodcastFeedItem> {
    const OP: &str = "collected podcast feed";
    if !row.is_object() {
        return Err(malformed(OP, "feed item"));
    }
    let base = row
        .pointer("/resource/mlogBaseData")
        .unwrap_or(&Value::Null);
    let feed_id = [&row["id"], &row["resourceId"], &base["id"]]
        .into_iter()
        .find_map(feed_id);
    // Intentionally expose only known display/identity fields, never the raw
    // personalized feed, embedded stream URLs, or unknown resource metadata.
    let mut extensions = Extensions::new();
    for (name, candidates) in [
        ("title", [&base["title"], &row["title"]]),
        ("text", [&base["text"], &row["text"]]),
    ] {
        if let Some(value) = candidates.into_iter().find_map(visible_text) {
            extensions.insert(name.to_owned(), json!(value));
        }
    }
    if let Some(value) = [&base["coverUrl"], &row["coverUrl"]]
        .into_iter()
        .find_map(cover_url)
    {
        extensions.insert("cover_url".to_owned(), json!(value));
    }
    if let Some(value) = [&base["pubTime"], &row["pubTime"]]
        .into_iter()
        .find_map(json_u64)
    {
        extensions.insert("publish_time".to_owned(), json!(value));
    }
    Ok(CollectedPodcastFeedItem {
        feed_id,
        extensions,
    })
}

fn feed_id(value: &Value) -> Option<String> {
    match value {
        Value::String(id)
            if !id.is_empty()
                && id.len() <= 128
                && id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-')) =>
        {
            Some(id.clone())
        }
        Value::Number(value) => value.as_u64().filter(|id| *id > 0).map(|id| id.to_string()),
        _ => None,
    }
}

#[cfg(test)]
#[path = "podcast_discovery/tests.rs"]
mod tests;
