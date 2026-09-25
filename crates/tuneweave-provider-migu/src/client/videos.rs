use super::*;
use serde_json::Value;
use tuneweave_core::{CreatorSummary, Video, VideoDetail, VideoResourceKind, VideoStats};

pub(crate) const SEARCH_PATH: &str = "/bmw/search/video/v1.0";
pub(crate) const ARTIST_PATH: &str = "/MIGUM3.0/bmw/singer/mv/v1.0";
const DETAIL_PATH: &str = "/MIGUM2.0/v1.0/content/resourceinfo.do";
const INFO_PATH: &str = "/pc/bmw/singer/info/v1.1";
const MAX_RESPONSE: u64 = 2 * 1024 * 1024;
pub(crate) struct MvPage {
    pub items: Vec<Video>,
    pub more: bool,
}
pub(crate) struct MvDetail {
    pub detail: VideoDetail,
    pub stats: VideoStats,
}
fn invalid() -> TuneWeaveError {
    migu_upstream_error("Migu MV response has invalid identity or structure")
}
pub(crate) fn valid_id(s: &str) -> bool {
    !s.is_empty() && s.len() <= 64 && !s.starts_with('0') && s.bytes().all(|b| b.is_ascii_digit())
}
fn string<'a>(v: &'a Value, key: &str) -> Result<&'a str> {
    v.get(key).and_then(Value::as_str).ok_or_else(invalid)
}
fn text(s: &str, max: usize) -> Result<String> {
    if s.trim().is_empty() || s.len() > max || s.chars().any(char::is_control) {
        return Err(invalid());
    }
    Ok(s.trim().into())
}
fn optional_text(v: &Value, key: &str, max: usize) -> Result<Option<String>> {
    match v.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) if s.is_empty() => Ok(None),
        Some(Value::String(s)) => text(s, max).map(Some),
        _ => Err(invalid()),
    }
}
fn count(v: &Value, key: &str) -> Result<Option<u64>> {
    match v.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v.as_u64().map(Some).ok_or_else(invalid),
    }
}
fn picture(v: &Value, key: &str) -> Result<Option<String>> {
    optional_text(v, key, 8192)?
        .map(|s| mv_image_url(&s).ok_or_else(invalid))
        .transpose()
}
fn mv_image_url(value: &str) -> Option<String> {
    if let Some(url) = super::albums::album_image_url(value) {
        return Some(url);
    }
    // Older MV covers use an extensionless resource-service image path.
    let url = Url::parse(value).ok()?;
    if value.len() > 8192
        || url.scheme() != "https"
        || url.host_str() != Some("d.musicapp.migu.cn")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return None;
    }
    let parts = url
        .path()
        .strip_prefix("/data/resource-service/file-down/")?
        .split('/')
        .collect::<Vec<_>>();
    if parts.len() != 4
        || !parts.iter().all(|p| {
            p.len() == 2
                && p.bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        })
    {
        return None;
    }
    Some(url.into())
}
fn pictures(v: &Value, key: &str) -> Result<Option<String>> {
    let Some(images) = v.get(key).filter(|v| !v.is_null()) else {
        return Ok(None);
    };
    let images = images
        .as_array()
        .filter(|v| v.len() <= 64)
        .ok_or_else(invalid)?;
    let mut first = None;
    for image in images {
        let value = picture(image, "img")?;
        if first.is_none() {
            first = value;
        }
    }
    Ok(first)
}
fn duration(value: &str) -> Result<u64> {
    let fields: Vec<_> = value.split(':').collect();
    if fields.len() != 3
        || fields[0].is_empty()
        || fields[0].len() > 3
        || fields[1].len() != 2
        || fields[2].len() != 2
        || fields
            .iter()
            .any(|s| !s.bytes().all(|b| b.is_ascii_digit()))
    {
        return Err(invalid());
    }
    let nums = fields
        .iter()
        .map(|s| s.parse::<u64>().map_err(|_| invalid()))
        .collect::<Result<Vec<_>>>()?;
    if nums[1] >= 60 || nums[2] >= 60 {
        return Err(invalid());
    }
    duration_ms((nums[0] * 3600 + nums[1] * 60 + nums[2]) * 1000)
}
fn duration_ms(v: u64) -> Result<u64> {
    if v > 7 * 24 * 3600 * 1000 {
        Err(invalid())
    } else {
        Ok(v)
    }
}
fn base(id: &str, title: &str, backend: &str) -> Result<Video> {
    if !valid_id(id) {
        return Err(invalid());
    }
    Ok(Video {
        resource_ref: ResourceRef::new(Platform::Migu, id).map_err(|_| invalid())?,
        platform: Platform::Migu,
        id: id.into(),
        title: text(title, 4096)?,
        creators: vec![],
        description: String::new(),
        cover_url: None,
        duration_ms: None,
        published_at: None,
        play_count: None,
        subscribed: None,
        extensions: Extensions::from([
            ("backend".into(), json!(backend)),
            ("resource_type".into(), json!("D")),
            ("catalogue_scope".into(), json!("public")),
        ]),
    })
}
fn display_credit(name: &str) -> Result<CreatorSummary> {
    Ok(CreatorSummary {
        resource_ref: None,
        name: text(name, 8192)?,
        avatar_url: None,
    })
}
pub(crate) fn parse_detail(root: Value, expected: &str) -> Result<MvDetail> {
    let items = root
        .get("resource")
        .and_then(Value::as_array)
        .ok_or_else(invalid)?;
    if items.is_empty() {
        return Err(
            TuneWeaveError::new(ErrorCode::ResourceNotFound, "Migu MV was not found")
                .with_platform(Platform::Migu),
        );
    }
    let [v] = items.as_slice() else {
        return Err(invalid());
    };
    if string(v, "resourceType")? != "D" || string(v, "contentId")? != expected {
        return Err(invalid());
    }
    let mut video = base(expected, string(v, "songName")?, "official_mv_resource_v1")?;
    video.cover_url = pictures(v, "imgs")?;
    video.duration_ms = optional_text(v, "migumvDuration", 16)?
        .map(|s| duration(&s))
        .transpose()?;
    if let Some(s) = optional_text(v, "singer", 8192)? {
        video.creators.push(display_credit(&s)?);
    }
    if let Some(s) = v.get("summary").filter(|v| !v.is_null()) {
        let s = s.as_str().ok_or_else(invalid)?;
        if s.len() > 16384
            || s.chars()
                .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
        {
            return Err(invalid());
        }
        video.description = s.trim().to_owned();
    }
    for key in [
        "copyrightId",
        "mvCopyrightId",
        "songId",
        "singerId",
        "aspectRatio",
        "copyright",
        "mvType",
    ] {
        if let Some(s) = optional_text(v, key, 1024)? {
            video.extensions.insert(key.into(), json!(s));
        }
    }
    let op = match v.get("opNumItem") {
        None | Some(Value::Null) => &Value::Null,
        Some(v) if v.is_object() => v,
        _ => return Err(invalid()),
    };
    video.play_count = count(op, "playNum")?;
    let mut formats = Vec::new();
    if let Some(raw) = v.get("rateFormats").filter(|v| !v.is_null()) {
        for f in raw
            .as_array()
            .filter(|a| a.len() <= 32)
            .ok_or_else(invalid)?
        {
            if string(f, "resourceType")? != "D" {
                return Err(invalid());
            }
            let mut format = serde_json::Map::new();
            for key in ["formatType", "format", "fileType"] {
                format.insert(key.into(), json!(text(string(f, key)?, 64)?));
            }
            if let Some(s) = optional_text(f, "size", 20)? {
                if !s.bytes().all(|b| b.is_ascii_digit()) {
                    return Err(invalid());
                }
                format.insert(
                    "size".into(),
                    json!(s.parse::<u64>().map_err(|_| invalid())?),
                );
            }
            // Catalogue URLs are neither playback authorization nor public output.
            formats.push(Value::Object(format));
        }
    }
    video
        .extensions
        .insert("catalogue_formats".into(), json!(formats));
    let stats = VideoStats {
        video_ref: video.resource_ref.clone(),
        kind: VideoResourceKind::Mv,
        liked: None,
        favorited: None,
        coins_contributed: None,
        view_count: video.play_count,
        danmaku_count: None,
        like_count: count(op, "thumbNum")?,
        coin_count: None,
        favorite_count: count(op, "keepNum")?,
        comment_count: count(op, "commentNum")?,
        share_count: count(op, "shareNum")?,
        extensions: Extensions::from([
            ("backend".into(), json!("official_mv_resource_v1")),
            ("catalogue_scope".into(), json!("public")),
        ]),
    };
    Ok(MvDetail {
        detail: VideoDetail {
            kind: VideoResourceKind::Mv,
            video,
            resolutions: vec![],
            extensions: Extensions::new(),
        },
        stats,
    })
}
pub(crate) fn parse_search(data: Value) -> Result<MvPage> {
    let more = data
        .get("hasNext")
        .and_then(Value::as_bool)
        .ok_or_else(invalid)?;
    let rows: &[Value] = match data.get("items") {
        None if !more => &[],
        Some(Value::Array(a)) => a,
        _ => return Err(invalid()),
    };
    if rows.len() > 20 || more && rows.len() != 20 {
        return Err(invalid());
    }
    let mut items = Vec::new();
    for row in rows {
        if row.as_object().is_none_or(|o| o.len() != 1) {
            return Err(invalid());
        }
        let v = row.get("video").ok_or_else(invalid)?;
        if string(v, "resourceType")? != "D" {
            return Err(invalid());
        }
        let mut video = base(
            string(v, "contentId")?,
            string(v, "title")?,
            "bmw_mv_search_v1",
        )?;
        video.cover_url = picture(v, "showImg")?;
        video.duration_ms = count(v, "duration")?.map(duration_ms).transpose()?;
        if let Some(raw) = v.get("user").filter(|v| !v.is_null()) {
            let users = raw
                .as_array()
                .filter(|v| v.len() <= 128)
                .ok_or_else(invalid)?;
            let mut sources = Vec::new();
            for u in users {
                let mut credit = display_credit(string(u, "nickName")?)?;
                credit.avatar_url = picture(u, "avatar")?;
                // videoUserId is a role-dependent identity, not necessarily an artist ID.
                sources.push(json!({"type":count(u,"type")?,"video_user_id":optional_text(u,"videoUserId",128)?}));
                video.creators.push(credit);
            }
            video
                .extensions
                .insert("creator_sources".into(), json!(sources));
        }
        items.push(video);
    }
    Ok(MvPage { items, more })
}
pub(crate) fn parse_artist(data: Value, artist: &str, page: u32) -> Result<MvPage> {
    let header = data
        .get("header")
        .filter(|v| v.is_object())
        .ok_or_else(invalid)?;
    let next = optional_text(header, "nextPageUrl", 2048)?;
    if let Some(next) = &next {
        let url = Url::parse(next).map_err(|_| invalid())?;
        if !matches!(url.scheme(), "http" | "https")
            || url.host_str() != Some("app.c.nf.migu.cn")
            || url.path() != ARTIST_PATH
            || !url.username().is_empty()
            || url.password().is_some()
            || url.port().is_some()
            || url.fragment().is_some()
        {
            return Err(invalid());
        }
        let pairs = url.query_pairs().collect::<Vec<_>>();
        let expected = BTreeMap::from([
            ("singerId".to_owned(), artist.to_owned()),
            (
                "pageNo".to_owned(),
                page.checked_add(1).ok_or_else(invalid)?.to_string(),
            ),
        ]);
        if pairs.len() != 2
            || pairs
                .into_iter()
                .map(|(k, v)| (k.into_owned(), v.into_owned()))
                .collect::<BTreeMap<_, _>>()
                != expected
        {
            return Err(invalid());
        }
    }
    let rows = data
        .get("contents")
        .and_then(Value::as_array)
        .ok_or_else(invalid)?;
    if rows.len() > 10 || next.is_some() && rows.len() != 10 {
        return Err(invalid());
    }
    let mut items = Vec::new();
    for v in rows {
        if string(v, "view")? != "ZJ-Mv" || string(v, "resType")? != "D" {
            return Err(invalid());
        }
        let mut video = base(
            string(v, "resId")?,
            string(v, "txt")?,
            "official_artist_mv_v1",
        )?;
        if string(v, "action")? != format!("mgmusic://mv-info?id={}", video.id) {
            return Err(invalid());
        }
        video.cover_url = picture(v, "img")?;
        video.duration_ms = optional_text(v, "txt4", 16)?
            .map(|s| duration(&s))
            .transpose()?;
        if let Some(s) = optional_text(v, "txt2", 8192)? {
            video.creators.push(display_credit(&s)?);
        }
        video
            .extensions
            .insert("source_artist_id".into(), json!(artist));
        if let Some(s) = optional_text(v, "txt3", 64)? {
            video
                .extensions
                .insert("play_count_display".into(), json!(s));
        }
        items.push(video);
    }
    Ok(MvPage {
        items,
        more: next.is_some(),
    })
}
impl MiguClient {
    pub(super) async fn mv_response(
        &self,
        host: &'static str,
        path: &'static str,
        query: &[(&str, String)],
        remaining: &mut u64,
    ) -> Result<Value> {
        let started = Instant::now();
        let mut status = None;
        let result = async {
            if *remaining == 0 {
                return Err(migu_upstream_error("Migu MV response budget exhausted"));
            }
            let url = Url::parse(&format!("https://{host}{path}")).map_err(|_| invalid())?;
            #[cfg(test)]
            let url = if let Some(origin) = &self.catalog_test_origin {
                origin.join(path).map_err(|_| invalid())?
            } else {
                url
            };
            let response = self
                .http
                .get(url)
                .header(ACCEPT, "application/json")
                .query(query)
                .send()
                .await
                .map_err(migu_network_error)?;
            status = Some(response.status());
            if response.status().is_success()
                && !response
                    .headers()
                    .get(reqwest::header::CONTENT_TYPE)
                    .and_then(|v| v.to_str().ok())
                    .and_then(|s| s.split(';').next())
                    .is_some_and(|s| s.trim().eq_ignore_ascii_case("application/json"))
            {
                return Err(invalid());
            }
            let bytes =
                read_bounded_response_with_limit(response, "Migu MV", MAX_RESPONSE.min(*remaining))
                    .await?;
            *remaining -= bytes.len() as u64;
            let root: Value = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
            if root.get("code").and_then(Value::as_str) != Some("000000") {
                return Err(migu_upstream_error("Migu MV request was rejected"));
            }
            Ok(root)
        }
        .await;
        self.log_upstream_request("mv_catalogue", host, path, status, started, &result);
        result
    }
    pub(crate) async fn mv_detail(&self, id: &str, budget: &mut u64) -> Result<MvDetail> {
        parse_detail(
            self.mv_response(
                "c.musicapp.migu.cn",
                DETAIL_PATH,
                &[
                    ("resourceId", id.into()),
                    ("resourceType", "D".into()),
                    ("needSimple", "01".into()),
                ],
                budget,
            )
            .await?,
            id,
        )
    }
    pub(crate) async fn mv_search(
        &self,
        keyword: &str,
        order: u32,
        page: u32,
        budget: &mut u64,
    ) -> Result<MvPage> {
        let root = self
            .mv_response(
                "app.c.nf.migu.cn",
                SEARCH_PATH,
                &[
                    ("pageNo", page.to_string()),
                    ("text", keyword.into()),
                    ("typeOrder", order.to_string()),
                ],
                budget,
            )
            .await?;
        parse_search(root.get("data").cloned().ok_or_else(invalid)?)
    }
    pub(crate) async fn mv_artist_info(&self, id: &str, budget: &mut u64) -> Result<()> {
        let root = self
            .mv_response(
                "app.c.nf.migu.cn",
                INFO_PATH,
                &[("singerId", id.into())],
                budget,
            )
            .await?;
        let data = serde_json::from_value(root.get("data").cloned().ok_or_else(invalid)?)
            .map_err(|_| invalid())?;
        super::artists::metadata(data, id).map(|_| ())
    }
    pub(crate) async fn mv_artist_page(
        &self,
        id: &str,
        page: u32,
        budget: &mut u64,
    ) -> Result<MvPage> {
        let root = self
            .mv_response(
                "app.c.nf.migu.cn",
                ARTIST_PATH,
                &[("singerId", id.into()), ("pageNo", page.to_string())],
                budget,
            )
            .await?;
        parse_artist(root.get("data").cloned().ok_or_else(invalid)?, id, page)
    }
}

#[cfg(test)]
pub(crate) mod tests;
