use std::{collections::BTreeSet, time::SystemTime};

use async_trait::async_trait;
use serde_json::{Value, json};
use tuneweave_core::{
    AlbumSummary, ArtistSummary, Capability, ErrorCode, Extensions, MusicProvider, Page, PageMeta,
    Platform, Quality, ResourceRef, Result, SearchKind, SearchQuery, SearchVariant, Track,
    TuneWeaveError,
};

use crate::client::{QqApiRequest, QqApiResponse, QqClient, QqConfig};

const SEARCH_MODULE: &str = "music.search.SearchCgiService";
const SEARCH_METHOD: &str = "DoSearchForQQMusicMobile";
const UPSTREAM_PAGE_SIZE: u32 = 60;

#[derive(Clone)]
pub struct QqProvider {
    client: QqClient,
}

impl QqProvider {
    pub fn new(config: QqConfig) -> Result<Self> {
        Ok(Self {
            client: QqClient::new(config)?,
        })
    }

    pub const fn from_client(client: QqClient) -> Self {
        Self { client }
    }
}

#[async_trait]
impl MusicProvider for QqProvider {
    fn platform(&self) -> Platform {
        Platform::Qq
    }

    fn name(&self) -> &'static str {
        "QQ Music"
    }

    fn capabilities(&self) -> BTreeSet<Capability> {
        BTreeSet::from([Capability::SearchTracks])
    }

    async fn search(&self, query: &SearchQuery) -> Result<Page<Track>> {
        if query.kind != SearchKind::Track {
            return Err(TuneWeaveError::unsupported(
                Platform::Qq,
                capability_for_search(query.kind),
            ));
        }
        let keyword = query.query.trim();
        if keyword.is_empty() {
            return Err(
                TuneWeaveError::invalid_request("search query cannot be empty")
                    .with_platform(Platform::Qq),
            );
        }
        if query.variant != SearchVariant::Default {
            return Err(TuneWeaveError::invalid_request(
                "QQ typed search only supports the default variant",
            )
            .with_platform(Platform::Qq)
            .with_details(json!({ "variant": query.variant })));
        }
        if let Some(account) = query
            .account
            .as_deref()
            .map(str::trim)
            .filter(|account| !account.is_empty())
        {
            return Err(TuneWeaveError::new(
                ErrorCode::AuthenticationRequired,
                "QQ account selection is not available before QQ login is configured",
            )
            .with_platform(Platform::Qq)
            .with_details(json!({ "account": account })));
        }
        let limit = query.limit.clamp(1, 100);
        let search_id = generate_search_id()?;
        let first_page = query.offset / UPSTREAM_PAGE_SIZE + 1;
        let skip = query.offset % UPSTREAM_PAGE_SIZE;
        let page_count = skip.saturating_add(limit).div_ceil(UPSTREAM_PAGE_SIZE);
        let requests = (0..page_count)
            .map(|page_offset| {
                typed_search_request(
                    keyword,
                    &search_id,
                    0,
                    first_page.saturating_add(page_offset),
                )
            })
            .collect::<Vec<_>>();
        let responses = self.client.request_android(&requests).await?;
        map_track_search_response(query.offset, limit, skip, responses)
    }
}

fn typed_search_request(
    keyword: &str,
    search_id: &str,
    search_type: i64,
    page: u32,
) -> QqApiRequest {
    QqApiRequest::new(
        SEARCH_MODULE,
        SEARCH_METHOD,
        json!({
            "searchid": search_id,
            "query": keyword,
            "search_type": search_type,
            "num_per_page": UPSTREAM_PAGE_SIZE,
            "page_num": page,
            "highlight": false,
            "grp": true
        }),
    )
}

fn map_track_search_response(
    offset: u32,
    limit: u32,
    skip: u32,
    responses: Vec<QqApiResponse>,
) -> Result<Page<Track>> {
    let first = responses
        .first()
        .ok_or_else(|| qq_data_error("QQ track search returned no response"))?;
    ensure_data_success(&first.data, "QQ track search")?;
    let total = first
        .data
        .pointer("/meta/sum")
        .and_then(json_u64)
        .ok_or_else(|| qq_data_error("QQ track search response is missing total count"))?;
    let mut raw_items = Vec::new();
    for response in &responses {
        ensure_data_success(&response.data, "QQ track search")?;
        let items = response
            .data
            .pointer("/body/item_song")
            .and_then(Value::as_array)
            .ok_or_else(|| qq_data_error("QQ track search response is missing item_song"))?;
        raw_items.extend(items.iter().cloned());
    }
    let available = raw_items
        .into_iter()
        .skip(usize::try_from(skip).unwrap_or(usize::MAX))
        .take(usize::try_from(limit).unwrap_or(usize::MAX))
        .collect::<Vec<_>>();
    if total > u64::from(offset) && available.is_empty() {
        return Err(qq_data_error(
            "QQ track search reported results but returned an empty item list",
        ));
    }
    let items = available
        .into_iter()
        .map(map_track)
        .collect::<Result<Vec<_>>>()?;
    let consumed = u32::try_from(items.len()).unwrap_or(u32::MAX);
    let next_offset = offset.saturating_add(consumed);
    let has_more = u64::from(next_offset) < total && consumed > 0;
    let mut extensions = Extensions::new();
    extensions.insert("upstream_page_size".to_owned(), json!(UPSTREAM_PAGE_SIZE));
    extensions.insert(
        "upstream_responses".to_owned(),
        Value::Array(responses.into_iter().map(|response| response.raw).collect()),
    );
    Ok(Page {
        items,
        pagination: PageMeta {
            limit,
            offset,
            total: Some(total),
            next_offset: has_more.then_some(next_offset),
            has_more,
            extensions,
        },
    })
}

fn map_track(raw: Value) -> Result<Track> {
    let mid = nonempty_string(raw.get("mid"));
    let numeric_id = value_as_string(raw.get("id"));
    let id = mid
        .clone()
        .or_else(|| numeric_id.clone())
        .ok_or_else(|| qq_data_error("QQ track search item is missing both MID and numeric ID"))?;
    let name = ["title_main", "title", "name"]
        .into_iter()
        .find_map(|field| nonempty_string(raw.get(field)))
        .ok_or_else(|| qq_data_error("QQ track search item is missing its title"))?;
    let resource_ref = qq_ref(&id, "track")?;
    let artists = raw
        .get("singer")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|artist| map_artist_summary(artist).transpose())
        .collect::<Result<Vec<_>>>()?;
    let album = raw
        .get("album")
        .map(map_album_summary)
        .transpose()?
        .flatten();
    let duration_ms = raw
        .get("interval")
        .and_then(json_u64)
        .map(|seconds| seconds.saturating_mul(1_000));
    let mv_ref = raw
        .get("mv")
        .and_then(|mv| nonempty_string(mv.get("vid")).or_else(|| value_as_string(mv.get("id"))))
        .filter(|id| id != "0")
        .map(|id| qq_ref(&id, "MV"))
        .transpose()?;
    let file = raw.get("file").cloned().unwrap_or(Value::Null);
    let available_qualities = map_available_qualities(&file);
    let mut aliases = Vec::new();
    if let Some(subtitle) = nonempty_string(raw.get("subtitle")) {
        aliases.push(subtitle);
    }
    if let Some(title_extra) = nonempty_string(raw.get("title_extra")) {
        if !aliases.contains(&title_extra) {
            aliases.push(title_extra);
        }
    }
    let mut extensions = Extensions::new();
    insert_some(&mut extensions, "numeric_id", numeric_id);
    insert_some(&mut extensions, "mid", mid);
    insert_some(
        &mut extensions,
        "media_mid",
        nonempty_string(file.get("media_mid")),
    );
    insert_value(&mut extensions, "song_type", raw.get("type"));
    insert_value(&mut extensions, "status", raw.get("status"));
    insert_value(&mut extensions, "pay", raw.get("pay"));
    insert_value(&mut extensions, "file", raw.get("file"));
    insert_value(&mut extensions, "search_content", raw.get("content"));
    extensions.insert("search_item".to_owned(), raw);
    Ok(Track {
        resource_ref,
        platform: Platform::Qq,
        id,
        name,
        aliases,
        artists,
        album,
        duration_ms,
        isrc: None,
        mv_ref,
        playable: None,
        available_qualities,
        extensions,
    })
}

fn map_artist_summary(raw: &Value) -> Result<Option<ArtistSummary>> {
    let Some(name) = ["name", "title", "singerName"]
        .into_iter()
        .find_map(|field| nonempty_string(raw.get(field)))
    else {
        return Ok(None);
    };
    let id = nonempty_string(raw.get("mid")).or_else(|| value_as_string(raw.get("id")));
    Ok(Some(ArtistSummary {
        resource_ref: id.map(|id| qq_ref(&id, "artist")).transpose()?,
        name,
    }))
}

fn map_album_summary(raw: &Value) -> Result<Option<AlbumSummary>> {
    let Some(name) = ["name", "title", "albumName"]
        .into_iter()
        .find_map(|field| nonempty_string(raw.get(field)))
    else {
        return Ok(None);
    };
    let mid = nonempty_string(raw.get("mid"));
    let id = mid.clone().or_else(|| value_as_string(raw.get("id")));
    Ok(Some(AlbumSummary {
        resource_ref: id.map(|id| qq_ref(&id, "album")).transpose()?,
        name,
        cover_url: mid.map(|mid| qq_cover_url("T002", &mid)),
    }))
}

fn map_available_qualities(file: &Value) -> Vec<Quality> {
    let mut qualities = Vec::new();
    push_quality(
        &mut qualities,
        Quality::Low,
        any_positive(
            file,
            &["size_24aac", "size_48aac", "size_96aac", "size_96ogg"],
        ),
    );
    push_quality(
        &mut qualities,
        Quality::Standard,
        any_positive(file, &["size_128mp3"]),
    );
    push_quality(
        &mut qualities,
        Quality::High,
        any_positive(file, &["size_192ogg", "size_192aac", "size_320mp3"]),
    );
    push_quality(
        &mut qualities,
        Quality::Lossless,
        any_positive(file, &["size_flac"]),
    );
    let modern = file
        .get("size_new")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    push_quality(&mut qualities, Quality::Master, positive_index(modern, 0));
    push_quality(
        &mut qualities,
        Quality::Surround,
        positive_index(modern, 2) || positive_index(modern, 6),
    );
    push_quality(
        &mut qualities,
        Quality::Dolby,
        any_positive(file, &["size_dolby"]),
    );
    qualities
}

fn push_quality(qualities: &mut Vec<Quality>, quality: Quality, available: bool) {
    if available && !qualities.contains(&quality) {
        qualities.push(quality);
    }
}

fn any_positive(value: &Value, fields: &[&str]) -> bool {
    fields.iter().any(|field| {
        value
            .get(*field)
            .and_then(json_u64)
            .is_some_and(|size| size > 0)
    })
}

fn positive_index(values: &[Value], index: usize) -> bool {
    values
        .get(index)
        .and_then(json_u64)
        .is_some_and(|size| size > 0)
}

fn generate_search_id() -> Result<String> {
    let duration = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_err(|error| {
            TuneWeaveError::new(
                ErrorCode::InternalError,
                format!("system clock is before the Unix epoch: {error}"),
            )
            .with_platform(Platform::Qq)
        })?;
    let random_high = rand::random_range(1_u64..=20).saturating_mul(18_014_398_509_481_984);
    let random_low = rand::random_range(0_u64..=4_194_304).saturating_mul(4_294_967_296);
    let millis_of_day = u64::try_from(duration.as_millis() % 86_400_000).unwrap_or(0);
    Ok(random_high
        .saturating_add(random_low)
        .saturating_add(millis_of_day)
        .to_string())
}

fn ensure_data_success(data: &Value, context: &str) -> Result<()> {
    let code = data
        .get("code")
        .and_then(|value| {
            value
                .as_i64()
                .or_else(|| value.as_str().and_then(|value| value.parse().ok()))
        })
        .ok_or_else(|| qq_data_error(format!("{context} is missing a valid data code")))?;
    if code == 0 {
        Ok(())
    } else {
        Err(TuneWeaveError::new(
            ErrorCode::UpstreamError,
            format!("{context} failed with code {code}"),
        )
        .with_platform(Platform::Qq)
        .with_details(json!({ "platform_code": code })))
    }
}

fn qq_ref(id: &str, kind: &str) -> Result<ResourceRef> {
    ResourceRef::new(Platform::Qq, id).map_err(|error| {
        TuneWeaveError::new(
            ErrorCode::UpstreamError,
            format!("QQ returned an invalid {kind} identifier: {error}"),
        )
        .with_platform(Platform::Qq)
    })
}

fn qq_cover_url(kind: &str, mid: &str) -> String {
    format!("https://y.gtimg.cn/music/photo_new/{kind}R300x300M000{mid}.jpg")
}

fn nonempty_string(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn value_as_string(value: Option<&Value>) -> Option<String> {
    match value {
        Some(Value::String(value)) => {
            let value = value.trim();
            (!value.is_empty()).then(|| value.to_owned())
        }
        Some(Value::Number(value)) => Some(value.to_string()),
        _ => None,
    }
}

fn json_u64(value: &Value) -> Option<u64> {
    value
        .as_u64()
        .or_else(|| value.as_i64().and_then(|value| u64::try_from(value).ok()))
        .or_else(|| value.as_str().and_then(|value| value.parse().ok()))
}

fn insert_some(extensions: &mut Extensions, key: &str, value: Option<String>) {
    if let Some(value) = value {
        extensions.insert(key.to_owned(), Value::String(value));
    }
}

fn insert_value(extensions: &mut Extensions, key: &str, value: Option<&Value>) {
    if let Some(value) = value.filter(|value| !value.is_null()) {
        extensions.insert(key.to_owned(), value.clone());
    }
}

fn qq_data_error(message: impl Into<String>) -> TuneWeaveError {
    TuneWeaveError::new(ErrorCode::UpstreamError, message)
        .with_platform(Platform::Qq)
        .retryable(true)
}

const fn capability_for_search(kind: SearchKind) -> Capability {
    match kind {
        SearchKind::Track => Capability::SearchTracks,
        SearchKind::Album => Capability::SearchAlbums,
        SearchKind::Artist => Capability::SearchArtists,
        SearchKind::Playlist => Capability::SearchPlaylists,
        SearchKind::User => Capability::SearchUsers,
        SearchKind::Mv => Capability::SearchMvs,
        SearchKind::Lyric => Capability::SearchLyrics,
        SearchKind::RadioStation => Capability::SearchRadioStations,
        SearchKind::Podcast => Capability::SearchPodcasts,
        SearchKind::Video => Capability::SearchVideos,
        SearchKind::Mixed => Capability::SearchMixed,
        SearchKind::Voice => Capability::SearchVoices,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn response(data: Value) -> QqApiResponse {
        QqApiResponse {
            data: data.clone(),
            raw: json!({"code": 0, "req_0": {"code": 0, "data": data}}),
        }
    }

    fn sample_track(id: u64, mid: &str, title: &str) -> Value {
        json!({
            "id": id,
            "mid": mid,
            "type": 1,
            "title": title,
            "title_main": title,
            "subtitle": "电影插曲",
            "singer": [{"id": 4558, "mid": "0025NhlN2yWrP4", "name": "周杰伦"}],
            "album": {"id": 8220, "mid": "000MkMni19ClKG", "name": "叶惠美"},
            "mv": {"id": 293791, "vid": "w0026q7f01a"},
            "file": {
                "media_mid": "003Qui1q2u1Zho",
                "size_128mp3": 1,
                "size_320mp3": 2,
                "size_flac": 3,
                "size_new": [4, 0, 5]
            },
            "pay": {"pay_play": 1},
            "interval": 269,
            "status": 0
        })
    }

    #[test]
    fn track_mapping_preserves_every_qq_identifier() {
        let track = map_track(sample_track(97_773, "0039MnYb0qxYhV", "晴天")).expect("map track");
        assert_eq!(track.resource_ref.to_string(), "qq:0039MnYb0qxYhV");
        assert_eq!(track.extensions["numeric_id"], "97773");
        assert_eq!(track.extensions["mid"], "0039MnYb0qxYhV");
        assert_eq!(track.extensions["media_mid"], "003Qui1q2u1Zho");
        assert_eq!(track.extensions["song_type"], 1);
        assert_eq!(track.duration_ms, Some(269_000));
        assert_eq!(track.mv_ref.expect("MV ref").to_string(), "qq:w0026q7f01a");
        assert!(track.available_qualities.contains(&Quality::Standard));
        assert!(track.available_qualities.contains(&Quality::High));
        assert!(track.available_qualities.contains(&Quality::Lossless));
        assert!(track.available_qualities.contains(&Quality::Master));
        assert!(track.available_qualities.contains(&Quality::Surround));
    }

    #[test]
    fn page_mapping_supports_non_aligned_offsets_across_two_upstream_pages() {
        let first = (0..60)
            .map(|id| sample_track(id, &format!("mid{id}"), &format!("track{id}")))
            .collect::<Vec<_>>();
        let second = (60..120)
            .map(|id| sample_track(id, &format!("mid{id}"), &format!("track{id}")))
            .collect::<Vec<_>>();
        let third = (120..160)
            .map(|id| sample_track(id, &format!("mid{id}"), &format!("track{id}")))
            .collect::<Vec<_>>();
        let page = map_track_search_response(
            50,
            100,
            50,
            vec![
                response(json!({"code": 0, "meta": {"sum": 200}, "body": {"item_song": first}})),
                response(json!({"code": 0, "meta": {"sum": 200}, "body": {"item_song": second}})),
                response(json!({"code": 0, "meta": {"sum": 200}, "body": {"item_song": third}})),
            ],
        )
        .expect("map page");
        assert_eq!(page.items.len(), 100);
        assert_eq!(page.items[0].name, "track50");
        assert_eq!(page.items[99].name, "track149");
        assert_eq!(page.pagination.next_offset, Some(150));
    }

    #[test]
    fn reported_nonempty_result_cannot_be_silently_empty() {
        let error = map_track_search_response(
            0,
            10,
            0,
            vec![response(json!({
                "code": 0,
                "meta": {"sum": 1},
                "body": {"item_song": []}
            }))],
        )
        .expect_err("empty result must fail");
        assert_eq!(error.code, ErrorCode::UpstreamError);
    }

    #[test]
    fn missing_search_data_code_cannot_be_silently_successful() {
        let error = map_track_search_response(
            0,
            10,
            0,
            vec![response(json!({
                "meta": {"sum": 0},
                "body": {"item_song": []}
            }))],
        )
        .expect_err("missing data code must fail");
        assert_eq!(error.code, ErrorCode::UpstreamError);
        assert!(error.message.contains("missing a valid data code"));
    }

    #[tokio::test]
    async fn unsupported_variant_and_unconfigured_account_fail_before_network() {
        let provider = QqProvider::new(QqConfig::default()).expect("provider");
        let mut query = SearchQuery::tracks("周杰伦", 2, 0);
        query.variant = SearchVariant::Cloud;
        let variant_error = provider.search(&query).await.expect_err("variant failure");
        assert_eq!(variant_error.code, ErrorCode::InvalidRequest);

        query.variant = SearchVariant::Default;
        query.account = Some("green-diamond".to_owned());
        let account_error = provider.search(&query).await.expect_err("account failure");
        assert_eq!(account_error.code, ErrorCode::AuthenticationRequired);
    }

    #[tokio::test]
    #[ignore = "requires live QQ Music services"]
    async fn live_track_search_returns_real_metadata() {
        let provider = QqProvider::new(QqConfig {
            device_path: std::env::var_os("TUNEWEAVE_QQ_LIVE_DEVICE").map(Into::into),
            ..QqConfig::default()
        })
        .expect("provider");
        let page = provider
            .search(&SearchQuery::tracks("周杰伦", 2, 0))
            .await
            .expect("live search");
        assert_eq!(page.items.len(), 2);
        assert!(page.pagination.total.is_some_and(|total| total > 0));
        assert!(page.items.iter().all(|track| !track.name.is_empty()));
        assert!(
            page.items
                .iter()
                .all(|track| track.extensions.contains_key("media_mid"))
        );
    }
}
