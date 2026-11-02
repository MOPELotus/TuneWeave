use super::*;
use tuneweave_core::{
    RecommendationStyleCategory, RecommendationStyleSelection, RecommendationStyleTag,
};
const CONFIG: &str = "/api/homepage/daily/song/config/get";
const SONGS: &str = "/api/homepage/category/daily/song/list";
const SAVE: &str = "/api/homepage/daily/song/tag/save";
const DAILY: &str = "/api/v3/discovery/recommend/songs";

impl NeteaseProvider {
    fn current_style_client(&self, account: Option<&str>, original: &NeteaseClient) -> Result<()> {
        let current = self.client_for(account)?;
        let unchanged = if original.is_authenticated() {
            current.same_upload_session(original)
        } else {
            current.configured_cookie() == original.configured_cookie()
                && current.device_id() == original.device_id()
        };
        if unchanged {
            Ok(())
        } else {
            Err(TuneWeaveError::new(
                ErrorCode::Conflict,
                "NetEase account changed during recommendation operation",
            )
            .with_platform(Platform::Netease))
        }
    }
    pub(super) async fn read_recommendation_styles(
        &self,
        account: Option<&str>,
    ) -> Result<Vec<RecommendationStyleCategory>> {
        let client = self.client_for(account)?;
        require_authenticated_client(&client, "recommendation style catalogue")?;
        let response = client.request_xeapi(CONFIG, json!({})).await?;
        self.current_style_client(account, &client)?;
        ensure_success(&response.body)?;
        parse_categories(&response.body)
    }
    pub(super) async fn read_recommendation_style_selection(
        &self,
        account: Option<&str>,
    ) -> Result<RecommendationStyleSelection> {
        let client = self.client_for(account)?;
        require_authenticated_client(&client, "recommendation style selection")?;
        let response = client.request_xeapi(SONGS, json!({})).await?;
        self.current_style_client(account, &client)?;
        ensure_account_access(&client, &response.body, "recommendation style selection")?;
        parse_selection(&response.body)
    }
    pub(super) async fn read_native_recommendations(
        &self,
        request: &PageRequest,
        daily: bool,
    ) -> Result<Page<Track>> {
        if !(1..=100).contains(&request.limit) {
            return Err(TuneWeaveError::invalid_request(
                "recommendation limit must be 1 to 100",
            ));
        }
        let client = self.client_for(request.account.as_deref())?;
        require_authenticated_client(&client, "native recommendations")?;
        let response = client
            .request_xeapi(
                if daily { DAILY } else { SONGS },
                if daily {
                    json!({"ispush":false})
                } else {
                    json!({})
                },
            )
            .await?;
        self.current_style_client(request.account.as_deref(), &client)?;
        ensure_account_access(&client, &response.body, "native recommendations")?;
        map_tracks(request, response.body)
    }
    pub(super) async fn write_recommendation_style(
        &self,
        selection: &RecommendationStyleSelection,
        account: Option<&str>,
    ) -> Result<RecommendationStyleSelection> {
        validate_selection(selection)?;
        let client = self.client_for(account)?;
        require_authenticated_client(&client, "recommendation style update")?;
        let mut dispatched = false;
        let result=tokio::time::timeout(Duration::from_secs(45),async {
            let response=client.request_xeapi(CONFIG,json!({})).await?;
            self.current_style_client(account,&client)?;
            ensure_account_access(&client,&response.body,"recommendation style catalogue")?;
            let categories=parse_categories(&response.body)?;
            let category=categories.iter().find(|category|category.id==selection.category_id).ok_or_else(||TuneWeaveError::invalid_request("unknown recommendation category"))?;
            if selection.tag_ids.iter().any(|id|!category.tags.iter().any(|tag|tag.id==*id)) {return Err(TuneWeaveError::invalid_request("recommendation tag does not belong to its category"));}
            let payload=json!({"tags":json!({"tagIds":selection.tag_ids,"categoryId":selection.category_id}).to_string()});
            self.current_style_client(account,&client)?;
            dispatched=true;
            let response=client.request_xeapi(SAVE,payload).await?;
            self.current_style_client(account,&client)?;
            ensure_account_access(&client,&response.body,"recommendation style update")?;
            let response=client.request_xeapi(SONGS,json!({})).await?;
            self.current_style_client(account,&client)?;
            ensure_account_access(&client,&response.body,"recommendation style readback")?;
            let actual=parse_selection(&response.body)?;
            if actual.category_id!=selection.category_id || actual.tag_ids.iter().collect::<BTreeSet<_>>()!=selection.tag_ids.iter().collect::<BTreeSet<_>>() {return Err(style_error("selection readback did not match"));}
            Ok(actual)
        }).await.unwrap_or_else(|_|Err(TuneWeaveError::new(ErrorCode::UpstreamTimeout,"NetEase recommendation update exceeded its deadline").with_platform(Platform::Netease)));
        result.map_err(|mut error| {
            if dispatched {
                error.details["write_outcome"] = json!("unconfirmed");
                error.retryable = false;
            }
            error
        })
    }
}
fn style_error(message: &str) -> TuneWeaveError {
    TuneWeaveError::new(
        ErrorCode::UpstreamError,
        format!("NetEase recommendation {message}"),
    )
    .with_platform(Platform::Netease)
}
fn identifier(value: &Value, key: &str) -> Result<u64> {
    value
        .get(key)
        .and_then(json_u64)
        .ok_or_else(|| style_error("identity is invalid"))
}
fn name(value: &Value, key: &str) -> Result<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty() && s.len() <= 512 && !s.chars().any(char::is_control))
        .map(str::to_owned)
        .ok_or_else(|| style_error("name is invalid"))
}
fn parse_categories(body: &Value) -> Result<Vec<RecommendationStyleCategory>> {
    let list = body
        .pointer("/data/categorys")
        .and_then(Value::as_array)
        .filter(|v| !v.is_empty() && v.len() <= 128)
        .ok_or_else(|| style_error("category list is invalid"))?;
    let mut ids = BTreeSet::new();
    list.iter()
        .map(|raw| {
            let id = identifier(raw, "categoryId")?;
            if !ids.insert(id) {
                return Err(style_error("category identity repeats"));
            }
            let list = raw
                .get("tagVOList")
                .and_then(Value::as_array)
                .filter(|v| v.len() <= 256)
                .ok_or_else(|| style_error("tag list is invalid"))?;
            let mut tag_ids = BTreeSet::new();
            let tags = list
                .iter()
                .map(|raw| {
                    let tag = identifier(raw, "tagId")?;
                    if identifier(raw, "categoryId")? != id || !tag_ids.insert(tag) {
                        return Err(style_error("tag identity is inconsistent"));
                    }
                    Ok(RecommendationStyleTag {
                        id: tag,
                        name: name(raw, "tagName")?,
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            Ok(RecommendationStyleCategory {
                id,
                name: name(raw, "categoryName")?,
                tags,
            })
        })
        .collect()
}
fn parse_selection(body: &Value) -> Result<RecommendationStyleSelection> {
    let selected = body
        .pointer("/data/tags")
        .ok_or_else(|| style_error("selected tags are missing"))?;
    let category_id = identifier(selected, "categoryId")?;
    let tags = selected
        .get("tagVOList")
        .and_then(Value::as_array)
        .ok_or_else(|| style_error("selected tag list is invalid"))?;
    let tag_ids = tags
        .iter()
        .map(|tag| {
            if identifier(tag, "categoryId")? != category_id {
                return Err(style_error("selected category is inconsistent"));
            }
            identifier(tag, "tagId")
        })
        .collect::<Result<Vec<_>>>()?;
    let selection = RecommendationStyleSelection {
        category_id,
        tag_ids,
    };
    validate_selection(&selection).map_err(|_| style_error("selected tag set is invalid"))?;
    Ok(selection)
}
fn validate_selection(selection: &RecommendationStyleSelection) -> Result<()> {
    if selection.tag_ids.is_empty()
        || selection.tag_ids.len() > 64
        || selection.tag_ids.iter().collect::<BTreeSet<_>>().len() != selection.tag_ids.len()
    {
        return Err(TuneWeaveError::invalid_request(
            "select 1 to 64 distinct recommendation tags",
        ));
    }
    Ok(())
}
fn map_tracks(request: &PageRequest, body: Value) -> Result<Page<Track>> {
    let rows = body
        .pointer("/data/dailySongs")
        .and_then(Value::as_array)
        .filter(|v| v.len() <= 1000)
        .ok_or_else(|| style_error("song list is invalid"))?;
    let total = rows.len() as u64;
    let tracks = rows
        .iter()
        .map(|song| parse_body::<Song>(song.clone()).and_then(|song| map_song(song, None)))
        .collect::<Result<Vec<_>>>()?;
    let items = tracks
        .into_iter()
        .skip(request.offset as usize)
        .take(request.limit as usize)
        .collect::<Vec<_>>();
    let next = request
        .offset
        .checked_add(items.len() as u32)
        .ok_or_else(|| style_error("offset overflow"))?;
    let has_more = u64::from(next) < total && !items.is_empty();
    Ok(Page {
        items,
        pagination: PageMeta {
            limit: request.limit,
            offset: request.offset,
            total: Some(total),
            next_offset: has_more.then_some(next),
            has_more,
            extensions: Extensions::from([("complete_read".into(), json!(true))]),
        },
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn selection_and_catalogue_identity_are_bound() {
        assert!(
            validate_selection(&RecommendationStyleSelection {
                category_id: 1,
                tag_ids: vec![2, 2]
            })
            .is_err()
        );
        let body = json!({"data":{"categorys":[{"categoryId":1,"categoryName":"style","tagVOList":[{"categoryId":2,"tagId":3,"tagName":"tag"}]}]}});
        assert!(parse_categories(&body).is_err());
        assert!(
            parse_selection(
                &json!({"data":{"tags":{"categoryId":1,"tagVOList":[{"categoryId":2,"tagId":3}]}}})
            )
            .is_err()
        );
    }
}
