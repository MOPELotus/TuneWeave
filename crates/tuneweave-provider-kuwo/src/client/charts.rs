//! Current official web charts. Publication dates are observations, not snapshot tokens.
use super::*;
use catalog::Unsigned;
use std::collections::BTreeSet;
use tuneweave_core::{Chart, ChartCatalog, ChartCatalogView, ChartGroup};

pub(crate) const PAGE_SIZE: u32 = 20;

#[derive(Clone, Copy)]
pub(super) enum Endpoint {
    Menu,
    Tracks,
}
impl Endpoint {
    pub(super) const fn url(self) -> &'static str {
        match self {
            Self::Menu => "https://www.kuwo.cn/api/www/bang/bang/bangMenu",
            Self::Tracks => "https://www.kuwo.cn/api/www/bang/bang/musicList",
        }
    }
    pub(super) const fn path(self) -> &'static str {
        match self {
            Self::Menu => "/api/www/bang/bang/bangMenu",
            Self::Tracks => "/api/www/bang/bang/musicList",
        }
    }
    pub(super) const fn operation(self) -> &'static str {
        match self {
            Self::Menu => "chart_catalogue",
            Self::Tracks => "chart_tracks",
        }
    }
}

#[derive(Serialize)]
struct Query<'a> {
    #[serde(rename = "bangId", skip_serializing_if = "Option::is_none")]
    id: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pn: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    rn: Option<u32>,
    #[serde(rename = "httpsStatus")]
    https_status: u8,
    #[serde(rename = "reqId")]
    request_id: String,
    plat: &'static str,
    from: &'static str,
}

impl KuwoClient {
    async fn chart_bytes(
        &self,
        endpoint: Endpoint,
        id: Option<&str>,
        page: Option<u32>,
    ) -> Result<Vec<u8>> {
        for refresh in [false, true] {
            let response = self
                .signed_get(
                    KuwoSignedEndpoint::Chart(endpoint),
                    &Query {
                        id,
                        pn: page,
                        rn: page.map(|_| PAGE_SIZE),
                        https_status: 1,
                        request_id: new_request_id(),
                        plat: "web_www",
                        from: "",
                    },
                    "https://www.kuwo.cn/rankList",
                    refresh,
                    u8::from(refresh),
                )
                .await?;
            match response {
                KuwoSignedResponse::SessionRejected if !refresh => continue,
                KuwoSignedResponse::SessionRejected => return Err(invalid()),
                KuwoSignedResponse::Body(bytes) => {
                    if !refresh && is_signed_session_rejection(&bytes) {
                        continue;
                    }
                    return Ok(bytes);
                }
            }
        }
        Err(invalid())
    }
    pub(crate) async fn chart_catalogue(&self, view: ChartCatalogView) -> Result<ChartCatalog> {
        parse_catalogue(&self.chart_bytes(Endpoint::Menu, None, None).await?, view)
    }
    pub(crate) async fn chart_tracks_page(
        &self,
        id: &str,
        page: u32,
        tags: bool,
    ) -> Result<ChartPage> {
        parse_tracks(
            &self
                .chart_bytes(Endpoint::Tracks, Some(id), Some(page))
                .await?,
            id,
            page,
            tags,
        )
    }
}

#[derive(Deserialize)]
struct Envelope {
    code: i64,
    data: Option<serde_json::Value>,
}
fn data(bytes: &[u8], tracks: bool) -> Result<serde_json::Value> {
    let value: Envelope = serde_json::from_slice(bytes).map_err(|_| invalid())?;
    if tracks && value.code == -1 && value.data.is_none() {
        return Err(
            TuneWeaveError::new(ErrorCode::ResourceNotFound, "Kuwo chart was not found")
                .with_platform(Platform::Kuwo),
        );
    }
    if value.code != 200 {
        return Err(
            kuwo_upstream_error("Kuwo chart returned an unsuccessful business code")
                .with_details(json!({"upstream_code":value.code})),
        );
    }
    value.data.ok_or_else(invalid)
}
#[derive(Deserialize)]
struct Group {
    name: String,
    list: Vec<Entry>,
}
#[derive(Deserialize)]
struct Entry {
    sourceid: Unsigned,
    id: Unsigned,
    source: Unsigned,
    name: String,
    #[serde(default)]
    intro: String,
    pic: Option<String>,
    #[serde(rename = "pub")]
    publication: Option<String>,
}
fn parse_catalogue(bytes: &[u8], view: ChartCatalogView) -> Result<ChartCatalog> {
    let rows: Vec<Group> = serde_json::from_value(data(bytes, false)?).map_err(|_| invalid())?;
    if rows.len() > 64 {
        return Err(invalid());
    }
    let mut seen = BTreeSet::new();
    let mut groups = Vec::with_capacity(rows.len());
    for row in rows {
        let mut charts = Vec::with_capacity(row.list.len());
        for entry in row.list {
            let id = entry.sourceid.id()?;
            if entry.source.value()? != 2 || !seen.insert(id.clone()) || seen.len() > 1024 {
                return Err(invalid());
            }
            let mut extensions = Extensions::from([
                ("display_id".into(), json!(entry.id.id()?)),
                ("source".into(), json!(2)),
            ]);
            if let Some(label) = entry.publication {
                extensions.insert(
                    "publication_label".into(),
                    json!(catalog::text(&label, 128, false)?),
                );
            }
            charts.push(Chart {
                resource_ref: Some(
                    ResourceRef::new(Platform::Kuwo, format!("chart:{id}"))
                        .map_err(|_| invalid())?,
                ),
                platform: Platform::Kuwo,
                id: Some(id),
                name: required_text(&entry.name)?,
                description: if entry.intro.is_empty() {
                    String::new()
                } else {
                    catalog::text(&entry.intro, 8192, true)?
                },
                cover_url: entry.pic.as_deref().and_then(chart_cover),
                update_frequency: None,
                updated_at_ms: None,
                track_count: None,
                play_count: None,
                subscribed: None,
                playable: None,
                target_kind: Some("chart".into()),
                target_url: None,
                previews: vec![],
                extensions,
            });
        }
        groups.push(ChartGroup {
            code: None,
            name: required_text(&row.name)?,
            display_type: None,
            target_url: None,
            charts,
            extensions: Extensions::new(),
        });
    }
    Ok(ChartCatalog {
        platform: Platform::Kuwo,
        view,
        groups,
        extensions: Extensions::from([
            ("backend".into(), json!("current_web_chart_catalogue")),
            ("catalogue_scope".into(), json!("public")),
            ("period_scope".into(), json!("current")),
        ]),
    })
}

fn chart_cover(value: &str) -> Option<String> {
    if value.len() > 512
        || value.contains(['%', '\\'])
        || value.contains("..")
        || value.chars().any(char::is_whitespace)
    {
        return None;
    }
    let url = Url::parse(value).ok()?;
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return None;
    }
    let prefix = match url.host_str()? {
        "zimg.kuwo.cn" => "/bang/",
        "img1.kuwo.cn" | "img2.kuwo.cn" | "img3.kuwo.cn" | "img4.kuwo.cn" => "/star/upload/",
        _ => return None,
    };
    let path = url.path().strip_prefix(prefix)?;
    if path.is_empty()
        || path.split('/').any(str::is_empty)
        || !path
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'/' | b'_' | b'-' | b'.'))
        || !(path.ends_with(".png") || path.ends_with(".jpg"))
    {
        return None;
    }
    Some(url.into())
}

pub(crate) struct ChartPage {
    pub items: Vec<Track>,
    pub total: u64,
    pub publication: Option<String>,
}
#[derive(Deserialize)]
struct Songs {
    num: Unsigned,
    #[serde(rename = "pub")]
    publication: Option<String>,
    #[serde(rename = "musicList")]
    list: Vec<Song>,
}
#[derive(Deserialize)]
struct Song {
    #[serde(flatten)]
    detail: KuwoTrackDetail,
    trend: Option<String>,
    rank_change: Option<Unsigned>,
    #[serde(rename = "isNew")]
    is_new: Option<Unsigned>,
}
fn parse_tracks(bytes: &[u8], id: &str, page: u32, tags: bool) -> Result<ChartPage> {
    let body: Songs = serde_json::from_value(data(bytes, true)?).map_err(|_| invalid())?;
    let total = body.num.value()?;
    if page == 0 || total > u64::from(u32::MAX) {
        return Err(invalid());
    }
    let start = u64::from(page - 1) * u64::from(PAGE_SIZE);
    let expected = total.saturating_sub(start).min(u64::from(PAGE_SIZE));
    if body.list.len() as u64 != expected {
        return Err(invalid());
    }
    let publication = body
        .publication
        .as_deref()
        .map(catalog::date)
        .transpose()?
        .flatten();
    if total > 0 && start < total && publication.is_none() {
        return Err(invalid());
    }
    let mut items = Vec::with_capacity(body.list.len());
    for (index, row) in body.list.into_iter().enumerate() {
        let mut detail = row.detail;
        if detail
            .content_type
            .as_text()
            .is_some_and(|s| !s.is_empty() && s != "0")
            || !matches!(detail.ad_type.as_str(), "" | "0")
        {
            return Err(invalid());
        }
        let rid = detail.rid.as_text().ok_or_else(invalid)?;
        Unsigned::Text(rid.clone()).id()?;
        let credits = artists::credits(
            &detail.artist,
            &detail.artistid.as_text().ok_or_else(invalid)?,
        )?;
        detail.name = catalog::text(&detail.name, 512, false)?;
        if let Some(raw) = detail.duration.as_text() {
            Unsigned::Text(raw)
                .value()?
                .checked_mul(1000)
                .ok_or_else(invalid)?;
        }
        let trend = row
            .trend
            .as_deref()
            .map(|s| catalog::text(s, 32, false))
            .transpose()?;
        let change = row.rank_change.as_ref().map(Unsigned::value).transpose()?;
        let is_new = row.is_new.as_ref().map(Unsigned::value).transpose()?;
        let mut track = map_track_detail(detail, &rid, "current_web_chart_tracks")?;
        track.artists = credits;
        track.extensions.insert("chart_id".into(), json!(id));
        track
            .extensions
            .insert("chart_rank".into(), json!(start + index as u64 + 1));
        if let Some(date) = &publication {
            track
                .extensions
                .insert("chart_publication_date".into(), json!(date));
        }
        if tags {
            // isNew is absent in current responses. Preserve raw codes without
            // inventing previous ranks or treating missing flags as zero.
            if let Some(value) = trend {
                track
                    .extensions
                    .insert("chart_upstream_trend".into(), json!(value));
            }
            if let Some(value) = change {
                track
                    .extensions
                    .insert("chart_upstream_rank_change".into(), json!(value));
            }
            if let Some(value) = is_new {
                track
                    .extensions
                    .insert("chart_upstream_is_new".into(), json!(value));
            }
        }
        items.push(track);
    }
    Ok(ChartPage {
        items,
        total,
        publication,
    })
}
fn invalid() -> TuneWeaveError {
    kuwo_upstream_error("Kuwo chart response is invalid")
}

fn required_text(value: &str) -> Result<String> {
    let value = catalog::text(value, 512, false)?;
    if value.is_empty() {
        return Err(invalid());
    }
    Ok(value)
}
