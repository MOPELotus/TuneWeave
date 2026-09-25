//! Official PC charts; catalogue metadata never grants account playback rights.
use super::*;
use std::collections::BTreeSet;
use tuneweave_core::{
    Chart, ChartCatalog, ChartCatalogView, ChartGroup, ChartPeriod, ChartTrackPreview,
};

mod period;

pub(crate) const INDEX_PATH: &str = "/pc/bmw/rank/rank-index/v1.0";
pub(crate) const TRACKS_PATH: &str = "/pc/bmw/rank/rank-info/v1.0";
const MAX_RESPONSE: u64 = 2 * 1024 * 1024;
const MAX_TRACKS: usize = 1000;

#[derive(Deserialize)]
struct Envelope<T> {
    code: String,
    data: Option<T>,
}
#[derive(Deserialize)]
struct Index {
    header: IndexHeader,
    contents: Vec<Group>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct IndexHeader {
    data_version: String,
    next_page_no: u32,
    next_page_no2: u32,
}
#[derive(Deserialize)]
struct Group {
    view: String,
    style: String,
    contents: Vec<ChartItem>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ChartItem {
    view: Option<String>,
    rank_id: String,
    rank_name: String,
    image_url: Option<String>,
    #[serde(default)]
    contents: Vec<Row>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Row {
    res_type: String,
    res_id: String,
    song_id: String,
    copyright_id: String,
    txt: String,
    txt2: Option<String>,
    txt5: Option<String>,
    img: Option<String>,
    song_data: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Detail {
    view: String,
    column_id: String,
    period_column_id: String,
    title: String,
    desc: String,
    update_time: String,
    title_pic: Option<String>,
    has_next_page: bool,
    total_count: u64,
    contents: Vec<Row>,
    rank_type_list: Option<Vec<String>>,
    day_rank_update_time: Option<String>,
    week_rank_update_time: Option<String>,
}
pub(crate) struct ChartTracks {
    pub items: Vec<Track>,
    pub extensions: Extensions,
}
fn invalid() -> TuneWeaveError {
    migu_upstream_error("Migu chart response has invalid identity or structure")
}
pub(crate) fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && !value.starts_with('0')
        && value.bytes().all(|b| b.is_ascii_digit())
}
fn text(value: &str, max: usize, multiline: bool) -> Result<String> {
    if value.len() > max
        || value
            .chars()
            .any(|c| c.is_control() && !(multiline && matches!(c, '\n' | '\r' | '\t')))
    {
        return Err(invalid());
    }
    Ok(value.to_owned())
}
fn name(value: &str) -> Result<String> {
    if value.trim().is_empty() {
        return Err(invalid());
    }
    text(value, 2048, false)
}
fn image(value: Option<&str>) -> Result<Option<String>> {
    let Some(value) = value.filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    if value.len() > 8192 || value.chars().any(char::is_control) {
        return Err(invalid());
    }
    let parsed = Url::parse(value).map_err(|_| invalid())?;
    if parsed.query().is_some() || parsed.fragment().is_some() {
        return Err(invalid());
    }
    albums::album_image_url(value).map(Some).ok_or_else(invalid)
}
fn rank_change(value: Option<&str>) -> Result<Option<i64>> {
    match value.filter(|s| !s.is_empty()) {
        None => Ok(None),
        Some(s) if s.len() <= 20 => {
            let n: i64 = s.parse().map_err(|_| invalid())?;
            if n.to_string() != s {
                return Err(invalid());
            }
            Ok(Some(n))
        }
        _ => Err(invalid()),
    }
}
fn bind_row(seen: &mut BTreeMap<String, (String, String)>, row: &Row) -> Result<()> {
    let binding = (row.song_id.clone(), row.copyright_id.clone());
    if seen.get(&row.res_id).is_some_and(|old| old != &binding) {
        return Err(invalid());
    }
    seen.insert(row.res_id.clone(), binding);
    Ok(())
}
fn row(row: Row, index: usize) -> Result<(Track, ChartTrackPreview)> {
    if row.res_type != "2"
        || !valid_id(&row.res_id)
        || !valid_id(&row.song_id)
        || canonical_platform_id(&row.copyright_id) != Some(row.copyright_id.as_str())
        || row.song_data.len() > 64 * 1024
    {
        return Err(invalid());
    }
    let song: MiguSong = serde_json::from_str(&row.song_data).map_err(|_| invalid())?;
    // These are distinct upstream identities. Embedded metadata must bind all three.
    if song.content_id != row.res_id
        || song.song_id != row.song_id
        || song.copyright_id != row.copyright_id
        || song.resource_type != row.res_type
    {
        return Err(invalid());
    }
    let preview = ChartTrackPreview {
        rank: Some(index as u32 + 1),
        previous_rank: None,
        rank_change: rank_change(row.txt5.as_deref())?,
        track_ref: Some(migu_track_ref(&row.res_id)?),
        name: name(&row.txt)?,
        byline: row
            .txt2
            .as_deref()
            .filter(|s| !s.is_empty())
            .map(name)
            .transpose()?,
        cover_url: image(row.img.as_deref())?,
        extensions: Extensions::from([
            ("song_id".into(), json!(row.song_id)),
            ("copyright_id".into(), json!(row.copyright_id)),
        ]),
    };
    let album_cover = [&song.img3, &song.img2, &song.img1]
        .into_iter()
        .find_map(|s| albums::album_image_url(s));
    let mut track = map_song(song)?;
    if let Some(album) = &mut track.album {
        album.cover_url = album_cover;
    }
    Ok((track, preview))
}
fn parse_catalogue(bytes: &[u8], view: ChartCatalogView) -> Result<ChartCatalog> {
    let envelope: Envelope<Index> = serde_json::from_slice(bytes).map_err(|_| invalid())?;
    if envelope.code != "000000" {
        return Err(migu_upstream_error(
            "Migu chart catalogue request was rejected",
        ));
    }
    let data = envelope.data.ok_or_else(invalid)?;
    // The official index API has no paging arguments; reject an unsupported continuation.
    if data.header.next_page_no != 1 || data.header.next_page_no2 != 1 || data.contents.len() > 32 {
        return Err(invalid());
    }
    let version = text(&data.header.data_version, 64, false)?;
    let mut group_ids = BTreeSet::new();
    let mut chart_ids = BTreeSet::new();
    let mut groups = Vec::new();
    for group in data.contents {
        if !matches!(group.view.as_str(), "ZJ-RANK-SPECIAL" | "ZJ-RANK-NORMAL")
            || !group_ids.insert(group.view.clone())
            || group.contents.len() > 256
        {
            return Err(invalid());
        }
        let mut charts = Vec::new();
        for item in group.contents {
            if !valid_id(&item.rank_id)
                || !chart_ids.insert(item.rank_id.clone())
                || item.view.as_deref().is_some_and(|s| s != "ZJ-Song-Scroll")
                || item.contents.len() > 100
            {
                return Err(invalid());
            }
            let mut previews = Vec::new();
            let mut seen = BTreeMap::new();
            for (index, item) in item.contents.into_iter().enumerate() {
                bind_row(&mut seen, &item)?;
                previews.push(row(item, index)?.1);
            }
            charts.push(Chart {
                resource_ref: Some(
                    ResourceRef::new(Platform::Migu, format!("chart:{}", item.rank_id))
                        .map_err(|_| invalid())?,
                ),
                platform: Platform::Migu,
                id: Some(item.rank_id),
                name: name(&item.rank_name)?,
                description: String::new(),
                cover_url: image(item.image_url.as_deref())?,
                update_frequency: None,
                updated_at_ms: None,
                track_count: None,
                play_count: None,
                subscribed: None,
                playable: None,
                target_kind: Some("chart".into()),
                target_url: None,
                previews,
                extensions: Extensions::new(),
            });
        }
        groups.push(ChartGroup {
            code: Some(group.view.clone()),
            name: name(&group.style)?,
            display_type: Some(group.view),
            target_url: None,
            charts,
            extensions: Extensions::new(),
        });
    }
    Ok(ChartCatalog {
        platform: Platform::Migu,
        view,
        groups,
        extensions: Extensions::from([
            ("backend".into(), json!("official_pc_rank_index")),
            ("source_data_version".into(), json!(version)),
            ("period_scope".into(), json!("current")),
        ]),
    })
}
fn parse_period_tracks(
    bytes: &[u8],
    id: &str,
    include_tags: bool,
    period: &ChartPeriod,
) -> Result<ChartTracks> {
    let envelope: Envelope<Detail> = serde_json::from_slice(bytes).map_err(|_| invalid())?;
    if envelope.code == "200000" && envelope.data.is_none() {
        return Err(TuneWeaveError::new(
            ErrorCode::ResourceNotFound,
            "Migu chart or requested period was not found",
        )
        .with_platform(Platform::Migu));
    }
    if envelope.code != "000000" {
        return Err(migu_upstream_error(
            "Migu chart tracks request was rejected",
        ));
    }
    let data = envelope.data.ok_or_else(invalid)?;
    if data.view != "ZJ-Song-Scroll"
        || data.column_id != id
        || data.has_next_page
        || data.total_count > MAX_TRACKS as u64
    {
        return Err(invalid());
    }
    if data.total_count != data.contents.len() as u64 {
        return Err(
            migu_upstream_error("Migu chart count disagrees with its complete response")
                .with_details(json!({
                    "reason": "chart_count_mismatch",
                    "declared_track_count": data.total_count,
                    "received_track_count": data.contents.len(),
                })),
        );
    }
    let period_metadata = period::metadata(&data, period)?;
    let updated = text(&data.update_time, 64, false)?;
    let mut seen = BTreeMap::new();
    let mut items = Vec::with_capacity(data.contents.len());
    for (index, source) in data.contents.into_iter().enumerate() {
        // The official free chart repeats some songs within one complete response.
        // Preserve every physical position, while rejecting contradictory identities.
        bind_row(&mut seen, &source)?;
        let (mut track, preview) = row(source, index)?;
        track.extensions.extend([
            ("chart_id".into(), json!(id)),
            ("chart_rank".into(), json!(index + 1)),
            ("chart_position".into(), json!(index)),
            ("chart_update_label".into(), json!(updated)),
            ("chart_period_id".into(), json!(data.period_column_id)),
            ("chart_requested_period".into(), json!(period)),
        ]);
        if include_tags && let Some(change) = preview.rank_change {
            track
                .extensions
                .insert("chart_rank_change".into(), json!(change));
        }
        items.push(track);
    }
    let mut result = ChartTracks {
        items,
        extensions: Extensions::from([
            ("backend".into(), json!("official_pc_rank_info")),
            ("chart_id".into(), json!(id)),
            ("chart_name".into(), json!(name(&data.title)?)),
            (
                "chart_description".into(),
                json!(text(&data.desc, 16384, true)?),
            ),
            (
                "chart_cover_url".into(),
                json!(image(data.title_pic.as_deref())?),
            ),
            ("update_label".into(), json!(updated)),
            ("period_scope".into(), json!(period.kind())),
            ("period_column_id".into(), json!(data.period_column_id)),
            ("complete_read".into(), json!(true)),
            ("upstream_unique_track_count".into(), json!(seen.len())),
            ("duplicates_preserved".into(), json!(true)),
            (
                "consistency_scope".into(),
                json!("single_complete_response"),
            ),
            (
                "pagination_scope".into(),
                json!("upstream_catalogue_positions"),
            ),
            ("upstream_pages_fetched".into(), json!(1)),
            ("include_tags".into(), json!(include_tags)),
        ]),
    };
    result.extensions.extend(period_metadata);
    Ok(result)
}

#[cfg(test)]
fn parse_tracks(bytes: &[u8], id: &str, include_tags: bool) -> Result<ChartTracks> {
    parse_period_tracks(bytes, id, include_tags, &ChartPeriod::Current)
}

impl MiguClient {
    async fn chart_bytes(&self, id: Option<&str>, period: &ChartPeriod) -> Result<Vec<u8>> {
        let path = if id.is_some() {
            TRACKS_PATH
        } else {
            INDEX_PATH
        };
        let started = Instant::now();
        let mut status = None;
        let result = async {
            let url = self.catalog_endpoint(&format!("https://app.u.nf.migu.cn{path}"))?;
            let mut request = self
                .http
                .get(url)
                .header(ACCEPT, "application/json")
                .header("referer", "https://music.migu.cn/")
                .header("origin", "https://music.migu.cn");
            if let Some(id) = id {
                let (kind, date) = period::query(period)?;
                request = request.query(&[("rankId", id), ("rankType", kind), ("period", &date)]);
            }
            let response = request.send().await.map_err(migu_network_error)?;
            status = Some(response.status());
            if response.status().is_success()
                && !response
                    .headers()
                    .get(reqwest::header::CONTENT_TYPE)
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.split(';').next())
                    .is_some_and(|v| v.trim().eq_ignore_ascii_case("application/json"))
            {
                return Err(invalid());
            }
            read_bounded_response_with_limit(response, "Migu chart", MAX_RESPONSE).await
        }
        .await;
        self.log_upstream_request(
            "chart_catalogue",
            "app.u.nf.migu.cn",
            path,
            status,
            started,
            &result,
        );
        result
    }
    pub(crate) async fn chart_catalogue(&self, view: ChartCatalogView) -> Result<ChartCatalog> {
        parse_catalogue(&self.chart_bytes(None, &ChartPeriod::Current).await?, view)
    }
    pub(crate) async fn complete_chart_tracks(
        &self,
        id: &str,
        tags: bool,
        period: &ChartPeriod,
    ) -> Result<ChartTracks> {
        parse_period_tracks(&self.chart_bytes(Some(id), period).await?, id, tags, period)
    }
}

#[cfg(test)]
pub(crate) mod tests;
