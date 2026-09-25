use super::assets::{Audio, map_audio};
use super::dto::{Number, optional_text, required_text, resource};
use super::openapi::{Endpoint, check_ocean_status, check_status};
use super::*;
use tuneweave_core::{
    Capability, Chart, ChartCatalog, ChartCatalogView, ChartGroup, ChartPeriod, ChartPeriodSummary,
    ChartTrackPreview,
};

pub(crate) mod periods;

const PAGE_SIZE: u32 = 100;
const MAX_PAGES: u32 = 128;
const MAX_NODES: usize = 1024;

#[derive(Deserialize)]
struct Envelope<T> {
    data: T,
    total: Option<Number>,
    extra: Option<Extra>,
}
#[derive(Deserialize)]
struct Extra {
    resp: Option<Counts>,
}
#[derive(Deserialize)]
struct Counts {
    all_total: Option<Number>,
    #[serde(default)]
    rank_tag: Vec<Tag>,
}
#[derive(Deserialize)]
struct Tag {
    #[serde(rename = "type")]
    kind: Number,
    desc: String,
}
#[derive(Deserialize)]
struct List {
    total: Number,
    timestamp: Option<Number>,
    info: Vec<Node>,
}
#[derive(Deserialize)]
struct Node {
    rankid: Option<Number>,
    rankname: String,
    rank_cid: Option<Number>,
    #[serde(default)]
    children: Vec<Node>,
    haschildren: Option<Number>,
    classify: Option<Number>,
    ranktype: Option<Number>,
    intro: Option<String>,
    imgurl: Option<String>,
    img_cover: Option<String>,
    img_9: Option<String>,
    update_frequency: Option<String>,
    rank_id_publish_date: Option<String>,
    issue: Option<Number>,
    play_times: Option<Number>,
    jump_url: Option<String>,
    extra: Option<Extra>,
    #[serde(default)]
    songinfo: Vec<Preview>,
}
#[derive(Deserialize)]
struct Preview {
    album_audio_id: Number,
    name: String,
    author: Option<String>,
    album_cover: Option<String>,
    trans_param: Option<PreviewCover>,
}
#[derive(Deserialize)]
struct PreviewCover {
    union_cover: Option<String>,
}
#[derive(Deserialize)]
struct Info {
    rankid: Number,
    rankname: String,
    rank_cid: Number,
    ranktype: Option<Number>,
    zone: Option<String>,
    extra: Option<Extra>,
}
struct Snapshot {
    id: u64,
    period: u64,
    name: String,
    total: Option<u64>,
    tags: Vec<Value>,
    rank_type: Option<u64>,
    zone: Option<String>,
}
pub(crate) struct ChartTracks {
    pub items: Vec<Track>,
    pub total: u64,
    pub period: u64,
    pub name: String,
    pub pages: u32,
    pub tags: Vec<Value>,
    pub selected_period: Option<ChartPeriodSummary>,
}

impl KugouClient {
    pub(crate) async fn public_charts(&self, view: ChartCatalogView) -> Result<ChartCatalog> {
        let device = self.device_identity()?;
        let bytes = self
            .public_catalogue_get(
                Endpoint::Charts,
                BTreeMap::from([
                    ("plat", "2".into()),
                    ("withsong", "1".into()),
                    ("parentid", "0".into()),
                ]),
                &device,
            )
            .await?;
        parse_catalogue(&bytes, view)
    }
    pub(crate) async fn complete_chart_tracks(
        &self,
        id: u64,
        include_tags: bool,
        period: &ChartPeriod,
    ) -> Result<ChartTracks> {
        let device = self.device_identity()?;
        let mut snapshot = self.chart_info(id, 0, "", &device).await?;
        let selected_period = match period {
            ChartPeriod::Current => None,
            ChartPeriod::Id { id: requested } => {
                let periods = self.periods_for(&snapshot, &device).await?;
                let selected = periods
                    .items
                    .into_iter()
                    .find(|p| p.period == *period)
                    .ok_or_else(|| {
                        TuneWeaveError::new(
                            ErrorCode::ResourceNotFound,
                            "KuGou did not list the requested period for this chart",
                        )
                        .with_platform(Platform::Kugou)
                    })?;
                let requested = periods::period_id(requested)?;
                let historical = self
                    .chart_info(
                        id,
                        requested,
                        snapshot.zone.as_deref().unwrap_or_default(),
                        &device,
                    )
                    .await?;
                if historical.period != requested || historical.zone != snapshot.zone {
                    return Err(invalid());
                }
                snapshot = historical;
                Some(selected)
            }
            _ => {
                return Err(TuneWeaveError::unsupported(
                    Platform::Kugou,
                    Capability::ChartHistoricalTracks,
                ));
            }
        };
        let mut expected = snapshot.total;
        let mut items = vec![];
        let mut seen = BTreeSet::new();
        for page in 1..=MAX_PAGES {
            let bytes=self.public_openapi(Endpoint::ChartTracks,&json!({
                "show_portrait_mv":1,"show_type_total":1,"filter_original_remarks":1,"area_code":1,
                "pagesize":PAGE_SIZE,"rank_cid":snapshot.period,"type":1,"page":page,"rank_id":id
            }), &device).await?;
            let (tracks, total) = parse_tracks(&bytes, &snapshot, page, include_tags)?;
            if expected.is_some_and(|v| v != total) {
                return Err(invalid());
            }
            expected = Some(total);
            for track in tracks {
                if !seen.insert(track.id.clone()) {
                    return Err(invalid());
                }
                items.push(track);
            }
            if items.len() as u64 == total {
                return Ok(ChartTracks {
                    items,
                    total,
                    period: snapshot.period,
                    name: snapshot.name,
                    pages: page,
                    tags: if include_tags { snapshot.tags } else { vec![] },
                    selected_period,
                });
            }
        }
        Err(invalid())
    }
}

fn parse_catalogue(bytes: &[u8], view: ChartCatalogView) -> Result<ChartCatalog> {
    check_ocean_status(bytes)?;
    let e: Envelope<List> = serde_json::from_slice(bytes).map_err(|_| invalid())?;
    if e.data.total.0 != e.data.info.len() as u64 || e.data.info.len() > MAX_NODES {
        return Err(invalid());
    }
    let mut groups = vec![];
    let mut seen = BTreeSet::new();
    let mut node_count = 0;
    map_nodes(
        e.data.info,
        "酷狗榜单".into(),
        None,
        vec![],
        &mut groups,
        &mut seen,
        &mut node_count,
    )?;
    let mut extensions = Extensions::from([
        ("backend".into(), json!("official_ocean_chart_catalogue")),
        ("upstream_total".into(), json!(e.data.total.0)),
        ("chart_count".into(), json!(seen.len())),
    ]);
    if let Some(n) = e.data.timestamp {
        extensions.insert("response_time_seconds".into(), json!(n.0));
    }
    Ok(ChartCatalog {
        platform: Platform::Kugou,
        view,
        groups,
        extensions,
    })
}

fn map_nodes(
    nodes: Vec<Node>,
    name: String,
    code: Option<String>,
    path: Vec<(usize, String)>,
    groups: &mut Vec<ChartGroup>,
    seen: &mut BTreeSet<u64>,
    count: &mut usize,
) -> Result<()> {
    if path.len() > 8 {
        return Err(invalid());
    }
    let index = groups.len();
    groups.push(ChartGroup {
        code,
        name,
        display_type: None,
        target_url: None,
        charts: vec![],
        extensions: Extensions::from([
            (
                "parent_path".into(),
                json!(path.iter().map(|(_, name)| name).collect::<Vec<_>>()),
            ),
            (
                "source_path".into(),
                json!(path.iter().map(|(index, _)| index).collect::<Vec<_>>()),
            ),
        ]),
    });
    for (position, node) in nodes.into_iter().enumerate() {
        *count += 1;
        if *count > MAX_NODES {
            return Err(invalid());
        }
        if node
            .haschildren
            .as_ref()
            .is_some_and(|n| n.0 != u64::from(!node.children.is_empty()))
        {
            return Err(invalid());
        }
        if node.children.is_empty() {
            let id = node
                .rankid
                .as_ref()
                .map(|n| n.0)
                .filter(|n| *n > 0)
                .ok_or_else(invalid)?;
            if !seen.insert(id) {
                return Err(invalid());
            }
            let mut chart = map_chart(node, id)?;
            let mut source_path = path.iter().map(|(index, _)| *index).collect::<Vec<_>>();
            source_path.push(position);
            chart
                .extensions
                .insert("source_path".into(), json!(source_path));
            groups[index].charts.push(chart);
        } else {
            let title = required_text(node.rankname)?;
            let mut child_path = path.clone();
            child_path.push((position, title.clone()));
            map_nodes(
                node.children,
                title,
                node.rankid.filter(|n| n.0 > 0).map(|n| n.0.to_string()),
                child_path,
                groups,
                seen,
                count,
            )?;
        }
    }
    Ok(())
}
fn map_chart(n: Node, id: u64) -> Result<Chart> {
    if n.songinfo.len() > 100 {
        return Err(invalid());
    }
    let previews = n
        .songinfo
        .into_iter()
        .enumerate()
        .map(|(i, p)| {
            let cover = p
                .album_cover
                .or_else(|| p.trans_param.and_then(|v| v.union_cover));
            Ok(ChartTrackPreview {
                rank: None,
                previous_rank: None,
                rank_change: None,
                track_ref: Some(resource(p.album_audio_id.id()?)?),
                name: required_text(p.name)?,
                byline: optional_text(p.author, 1024)?,
                cover_url: cover.as_deref().and_then(normalize_image_url),
                extensions: Extensions::from([("preview_position".into(), json!(i))]),
            })
        })
        .collect::<Result<_>>()?;
    let counts = n.extra.and_then(|e| e.resp);
    let mut extensions = Extensions::new();
    for (key, value) in [
        ("rank_cid", n.rank_cid),
        ("classify", n.classify),
        ("rank_type", n.ranktype),
        ("issue", n.issue),
    ] {
        if let Some(n) = value {
            extensions.insert(key.into(), json!(n.0));
        }
    }
    if let Some(s) = optional_text(n.rank_id_publish_date, 128)? {
        extensions.insert("published_at".into(), json!(s));
    }
    let (track_count, tags) = match counts {
        Some(c) => (c.all_total.map(|n| n.0), map_tags(c.rank_tag)?),
        None => (None, vec![]),
    };
    if !tags.is_empty() {
        extensions.insert("chart_tags".into(), json!(tags));
    }
    Ok(Chart {
        resource_ref: Some(resource(format!("chart:{id}"))?),
        platform: Platform::Kugou,
        id: Some(id.to_string()),
        name: required_text(n.rankname)?,
        description: optional_text(n.intro, 131072)?.unwrap_or_default(),
        cover_url: [n.imgurl, n.img_cover, n.img_9]
            .into_iter()
            .flatten()
            .find_map(|s| normalize_image_url(&s)),
        update_frequency: optional_text(n.update_frequency, 1024)?,
        updated_at_ms: None,
        track_count,
        play_count: n.play_times.map(|n| n.0),
        subscribed: None,
        playable: None,
        target_kind: Some("chart".into()),
        target_url: n.jump_url.as_deref().and_then(display_url),
        previews,
        extensions,
    })
}
fn display_url(value: &str) -> Option<String> {
    if value.len() > 8192 || value.chars().any(char::is_control) {
        return None;
    }
    let mut url = Url::parse(value).ok()?;
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
        || url
            .port()
            .is_some_and(|p| p != 443 && !(p == 80 && url.scheme() == "http"))
        || !matches!(
            url.host_str(),
            Some("h5.kugou.com" | "www.kugou.com" | "m.kugou.com" | "kugou.com")
        )
    {
        return None;
    }
    url.set_scheme("https").ok()?;
    url.set_port(None).ok()?;
    Some(url.into())
}
fn map_tags(tags: Vec<Tag>) -> Result<Vec<Value>> {
    if tags.len() > 100 {
        return Err(invalid());
    }
    tags.into_iter()
        .map(|t| Ok(json!({"type":t.kind.0,"text":required_text(t.desc)?})))
        .collect()
}
fn parse_info(bytes: &[u8], id: u64) -> Result<Snapshot> {
    check_ocean_status(bytes)?;
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct MissingInfo {
        #[serde(rename = "timestamp")]
        _timestamp: Number,
    }
    if serde_json::from_slice::<Envelope<MissingInfo>>(bytes).is_ok() {
        return Err(
            TuneWeaveError::new(ErrorCode::ResourceNotFound, "KuGou chart was not found")
                .with_platform(Platform::Kugou),
        );
    }
    let e: Envelope<Info> = serde_json::from_slice(bytes).map_err(|_| invalid())?;
    let d = e.data;
    if d.rankid.0 != id || id == 0 || d.rank_cid.0 == 0 {
        return Err(invalid());
    }
    let (total, tags) = match d.extra.and_then(|e| e.resp) {
        Some(c) => (c.all_total.map(|n| n.0), map_tags(c.rank_tag)?),
        None => (None, vec![]),
    };
    if total.is_some_and(|t| t > u64::from(PAGE_SIZE) * u64::from(MAX_PAGES)) {
        return Err(invalid());
    }
    Ok(Snapshot {
        id,
        period: d.rank_cid.0,
        name: required_text(d.rankname)?,
        total,
        tags,
        rank_type: d.ranktype.map(|n| n.0),
        zone: d.zone.map(periods::zone).transpose()?,
    })
}

#[derive(Deserialize)]
struct TrackData {
    total: Number,
    songlist: Vec<Song>,
}
#[derive(Deserialize)]
struct Song {
    album_audio_id: Number,
    audio_id: Option<Number>,
    songname: String,
    album_id: Option<Number>,
    album_info: Option<SongAlbum>,
    authors: Vec<Author>,
    audio_info: Option<Audio>,
    rank_cid: Number,
    business: Business,
    #[serde(default)]
    remarks: Vec<Remark>,
}
#[derive(Deserialize)]
struct SongAlbum {
    album_name: String,
    sizable_cover: Option<String>,
}
#[derive(Deserialize)]
struct Author {
    author_id: Option<Number>,
    author_name: String,
}
#[derive(Deserialize)]
struct Remark {
    #[serde(rename = "type")]
    kind: Number,
    remark: String,
}
#[derive(Deserialize)]
struct Business {
    rank_id: Number,
    parent_id: Number,
    sort: Number,
    original_index: Option<Number>,
    last_sort: Option<Number>,
    last_original_index: Option<Number>,
    rank_count: Option<Number>,
    issue: Option<String>,
    rank_id_publish_date: Option<String>,
    recommend_reason: Option<String>,
}
fn parse_tracks(
    bytes: &[u8],
    snapshot: &Snapshot,
    page: u32,
    include_tags: bool,
) -> Result<(Vec<Track>, u64)> {
    check_status(bytes)?;
    let e: Envelope<TrackData> = serde_json::from_slice(bytes).map_err(|_| invalid())?;
    let total = e.data.total.0;
    let offset = u64::from(page.checked_sub(1).ok_or_else(invalid)?) * u64::from(PAGE_SIZE);
    if page > MAX_PAGES
        || total > u64::from(PAGE_SIZE) * u64::from(MAX_PAGES)
        || e.total.map(|n| n.0) != Some(total)
        || e.extra
            .and_then(|e| e.resp)
            .and_then(|c| c.all_total)
            .is_some_and(|n| n.0 != total)
        || e.data.songlist.len() as u64 != total.saturating_sub(offset).min(u64::from(PAGE_SIZE))
    {
        return Err(invalid());
    }
    let tracks = e
        .data
        .songlist
        .into_iter()
        .enumerate()
        .map(|(i, s)| map_song(s, snapshot, offset + i as u64, include_tags))
        .collect::<Result<_>>()?;
    Ok((tracks, total))
}
fn map_song(s: Song, snapshot: &Snapshot, position: u64, include_tags: bool) -> Result<Track> {
    let b = s.business;
    if s.rank_cid.0 != snapshot.period
        || b.rank_id.0 != snapshot.period
        || b.parent_id.0 != snapshot.id
        || b.sort.0 == 0
        || b.original_index
            .as_ref()
            .map_or(b.sort.0 != position + 1, |n| n.0 != position + 1)
    {
        return Err(invalid());
    }
    let id = s.album_audio_id.id()?;
    let mut t = Track::new(resource(id.clone())?, required_text(s.songname)?);
    if s.authors.is_empty() || s.authors.len() > 100 {
        return Err(invalid());
    }
    let mut seen = BTreeSet::new();
    for a in s.authors {
        let id = a.author_id.filter(|n| n.0 > 0).map(|n| n.0);
        if id.is_some_and(|id| !seen.insert(id)) {
            return Err(invalid());
        }
        t.artists.push(ArtistSummary {
            resource_ref: id.map(|id| resource(id.to_string())).transpose()?,
            name: required_text(a.author_name)?,
        });
    }
    let album_id = s.album_id.filter(|n| n.0 > 0).map(|n| n.0);
    if album_id.is_some() || s.album_info.is_some() {
        let (name, cover) = match s.album_info {
            Some(a) => (
                required_text(a.album_name)?,
                a.sizable_cover.as_deref().and_then(normalize_image_url),
            ),
            None => (String::new(), None),
        };
        t.album = Some(AlbumSummary {
            resource_ref: album_id.map(|id| resource(id.to_string())).transpose()?,
            name,
            cover_url: cover,
        });
    }
    t.extensions.insert("album_audio_id".into(), json!(id));
    if let Some(n) = s.audio_id.filter(|n| n.0 > 0) {
        t.extensions
            .insert("audio_id".into(), json!(n.0.to_string()));
    }
    t.extensions
        .insert("chart_id".into(), json!(snapshot.id.to_string()));
    t.extensions
        .insert("rank_cid".into(), json!(snapshot.period.to_string()));
    t.extensions
        .insert("chart_position".into(), json!(position));
    t.extensions.insert("chart_rank".into(), json!(b.sort.0));
    t.extensions
        .insert("detail_backend".into(), json!("openapi_rank_audio_v2"));
    for (key, n) in [
        ("original_index", b.original_index),
        ("last_sort", b.last_sort),
        ("last_original_index", b.last_original_index),
        ("rank_count", b.rank_count),
    ] {
        if let Some(n) = n {
            t.extensions.insert(key.into(), json!(n.0));
        }
    }
    for (key, s) in [
        ("issue", b.issue),
        ("rank_published_at", b.rank_id_publish_date),
    ] {
        if let Some(s) = optional_text(s, 128)? {
            t.extensions.insert(key.into(), json!(s));
        }
    }
    if include_tags {
        if s.remarks.len() > 100 {
            return Err(invalid());
        }
        let remarks = s
            .remarks
            .into_iter()
            .map(|r| Ok(json!({"type":r.kind.0,"text":required_text(r.remark)?})))
            .collect::<Result<Vec<_>>>()?;
        if !remarks.is_empty() {
            t.extensions.insert("chart_remarks".into(), json!(remarks));
        }
        if let Some(s) = optional_text(b.recommend_reason, 1024)? {
            t.extensions.insert("recommend_reason".into(), json!(s));
        }
    }
    if let Some(a) = s.audio_info {
        map_audio(a, &mut t)?;
    }
    Ok(t)
}
fn invalid() -> TuneWeaveError {
    kugou_upstream_error("KuGou chart returned inconsistent identity, period, ranking or metadata")
}

#[cfg(test)]
pub(crate) mod tests;
