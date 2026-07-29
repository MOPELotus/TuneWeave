use std::{fmt, time::Duration};

use reqwest::{
    Client, Proxy, StatusCode,
    header::{ACCEPT, CONTENT_LENGTH},
    redirect::Policy,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tuneweave_core::{
    AlbumSummary, ArtistSummary, ErrorCode, Platform, Quality, ResourceRef, Result, Track,
    TuneWeaveError,
};
use url::Url;

const SEARCH_ENDPOINT: &str = "https://app.c.nf.migu.cn/bmw/search/song/v1.0";
const MEDIA_HOST: &str = "d.musicapp.migu.cn";
const MAX_API_RESPONSE_BYTES: u64 = 8 * 1024 * 1024;
const USER_AGENT: &str = "TuneWeave/0.1 (Migu public music provider)";

#[derive(Clone, Default)]
pub struct MiguConfig {
    pub proxy_url: Option<String>,
}

impl fmt::Debug for MiguConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MiguConfig")
            .field(
                "proxy_url",
                &self.proxy_url.as_ref().map(|_| "[configured]"),
            )
            .finish()
    }
}

#[derive(Clone)]
pub struct MiguClient {
    http: Client,
}

impl fmt::Debug for MiguClient {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("MiguClient").finish_non_exhaustive()
    }
}

#[derive(Debug)]
pub(crate) struct MiguSearchPage {
    pub tracks: Vec<Track>,
    pub has_next: bool,
    pub sequence: Option<String>,
    pub conditions: Vec<MiguSearchCondition>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MiguAudioFormat {
    #[serde(default)]
    resource_type: String,
    #[serde(default)]
    format_type: String,
    #[serde(default)]
    show_tags: Vec<String>,
    #[serde(default)]
    isize: Option<FlexibleU64>,
    #[serde(default)]
    asize: Option<FlexibleU64>,
    #[serde(default)]
    iformat: String,
    #[serde(default)]
    aformat: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MiguSearchCondition {
    #[serde(default)]
    id: String,
    #[serde(default)]
    title: String,
    #[serde(rename = "type", default)]
    kind: String,
    #[serde(default)]
    datas: Vec<MiguSearchConditionValue>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct MiguSearchConditionValue {
    #[serde(default)]
    title: String,
    #[serde(default)]
    condition_type: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct MiguSinger {
    #[serde(default)]
    id: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    img: String,
    #[serde(default)]
    name_spelling: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct MiguSearchTag {
    #[serde(default)]
    name: String,
    #[serde(rename = "type", default)]
    kind: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default)]
struct MiguSongExtension {
    disc: String,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct MiguSong {
    resource_type: String,
    content_id: String,
    song_id: String,
    song_name: String,
    mv_id: String,
    mv_copyright_type: Option<i64>,
    ring_tone_id: String,
    ring_copyright_id: String,
    show_tags: Vec<String>,
    song_pinyin: String,
    audio_formats: Vec<MiguAudioFormat>,
    duration: Option<FlexibleU64>,
    play_num_desc: String,
    copyright_id: String,
    copyright_type: Option<i64>,
    restrict_type: Option<i64>,
    album_id: String,
    album: String,
    album_pinyin: String,
    img1: String,
    img2: String,
    img3: String,
    download_tags: Vec<String>,
    singer_list: Vec<MiguSinger>,
    ext: MiguSongExtension,
    forever_listen_flag: String,
    forever_listen: Option<bool>,
    action_img_url: String,
    shock_ring_id: String,
    has_associated_ring: Option<bool>,
    chorus_start_time: Option<FlexibleU64>,
    product_authorize_usage: String,
    audio_book: String,
    lrc_url: String,
    mrc_url: String,
    search_tags: Vec<MiguSearchTag>,
    more: Vec<MiguSong>,
    highlights: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
struct MiguAlternateTrack {
    content_id: String,
    song_id: Option<String>,
    name: String,
    resource_type: String,
    artists: Vec<MiguAlternateSinger>,
    album_id: Option<String>,
    album: Option<String>,
    duration_seconds: Option<u64>,
    copyright_id: Option<String>,
    copyright_type: Option<i64>,
    restrict_type: Option<i64>,
    forever_listen: Option<bool>,
    audio_formats: Vec<MiguAudioFormat>,
    cover_url: Option<String>,
    lrc_url: Option<String>,
    mrc_url: Option<String>,
    highlights: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
struct MiguAlternateSinger {
    id: Option<String>,
    name: String,
    name_spelling: Option<String>,
    image_url: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(untagged)]
enum FlexibleU64 {
    Number(u64),
    String(String),
}

impl FlexibleU64 {
    fn get(&self) -> Option<u64> {
        match self {
            Self::Number(value) => Some(*value),
            Self::String(value) => value.parse().ok(),
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct MiguSearchRequest<'a> {
    page_no: u32,
    text: &'a str,
}

#[derive(Deserialize)]
struct MiguSearchEnvelope {
    #[serde(default)]
    code: String,
    #[serde(default)]
    info: String,
    data: Option<MiguSearchData>,
}

#[derive(Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct MiguSearchData {
    has_next: bool,
    items: Vec<MiguSearchItem>,
    conditions: Vec<MiguSearchCondition>,
    seq: String,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct MiguSearchItem {
    song: Option<MiguSong>,
}

impl MiguClient {
    pub fn new(config: &MiguConfig) -> Result<Self> {
        let mut builder = Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(20))
            .redirect(Policy::none())
            .user_agent(USER_AGENT);
        if let Some(proxy_url) = config.proxy_url.as_deref() {
            let proxy = Proxy::all(proxy_url).map_err(|_| {
                migu_invalid_request("Migu proxy configuration is not a valid proxy URL")
            })?;
            builder = builder.proxy(proxy);
        }
        let http = builder.build().map_err(|_| {
            TuneWeaveError::new(ErrorCode::InternalError, "failed to build Migu HTTP client")
                .with_platform(Platform::Migu)
        })?;
        Ok(Self { http })
    }

    #[cfg(test)]
    pub(crate) fn test_client() -> Self {
        Self::new(&MiguConfig::default()).expect("create Migu test client")
    }

    pub(crate) async fn search_tracks_page(
        &self,
        keyword: &str,
        page: u32,
    ) -> Result<MiguSearchPage> {
        let response = self
            .http
            .get(SEARCH_ENDPOINT)
            .header(ACCEPT, "application/json")
            .query(&MiguSearchRequest {
                page_no: page,
                text: keyword,
            })
            .send()
            .await
            .map_err(migu_network_error)?;
        let bytes = read_bounded_response(response, "Migu search").await?;
        parse_search_response(&bytes)
    }
}

fn parse_search_response(bytes: &[u8]) -> Result<MiguSearchPage> {
    let envelope: MiguSearchEnvelope = serde_json::from_slice(bytes)
        .map_err(|_| migu_upstream_error("Migu search returned malformed JSON"))?;
    if envelope.code != "000000" {
        return Err(
            migu_upstream_error("Migu search rejected the request").with_details(json!({
                "platform_code": bounded_text(&envelope.code, 64),
                "platform_message": bounded_text(&envelope.info, 256),
            })),
        );
    }
    let data = envelope
        .data
        .ok_or_else(|| migu_upstream_error("Migu search response omitted data"))?;
    let tracks = data
        .items
        .into_iter()
        .map(|item| {
            item.song
                .ok_or_else(|| migu_upstream_error("Migu search result omitted song data"))
                .and_then(map_song)
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(MiguSearchPage {
        tracks,
        has_next: data.has_next,
        sequence: nonempty(&data.seq).map(|value| bounded_text(value, 512)),
        conditions: bounded_conditions(data.conditions),
    })
}

fn map_song(song: MiguSong) -> Result<Track> {
    let content_id = canonical_platform_id(&song.content_id)
        .ok_or_else(|| migu_upstream_error("Migu search result omitted a stable content ID"))?;
    if song.resource_type != "2" {
        return Err(migu_upstream_error(
            "Migu song search returned an incompatible resource type",
        ));
    }
    let name = validated_name(&song.song_name)
        .ok_or_else(|| migu_upstream_error("Migu search result omitted a track name"))?;
    let resource_ref = ResourceRef::new(Platform::Migu, content_id.to_owned())
        .map_err(|_| migu_upstream_error("Migu search returned an invalid track identity"))?;
    let mut track = Track::new(resource_ref, name);
    track.artists = song.singer_list.iter().filter_map(map_artist).collect();
    track.album = map_album(&song);
    track.duration_ms = song
        .duration
        .as_ref()
        .and_then(FlexibleU64::get)
        .and_then(|seconds| seconds.checked_mul(1_000));
    track.mv_ref = canonical_platform_id(&song.mv_id)
        .and_then(|id| ResourceRef::new(Platform::Migu, id.to_owned()).ok());
    track.available_qualities = mapped_qualities(&song.audio_formats);

    let alternates = song
        .more
        .iter()
        .filter_map(map_alternate)
        .collect::<Vec<_>>();
    insert_nonempty(&mut track.extensions, "song_id", &song.song_id);
    track
        .extensions
        .insert("resource_type".to_owned(), json!(song.resource_type));
    insert_nonempty(&mut track.extensions, "copyright_id", &song.copyright_id);
    insert_optional(&mut track.extensions, "copyright_type", song.copyright_type);
    insert_optional(&mut track.extensions, "restrict_type", song.restrict_type);
    insert_optional(
        &mut track.extensions,
        "mv_copyright_type",
        song.mv_copyright_type,
    );
    insert_nonempty(&mut track.extensions, "ring_tone_id", &song.ring_tone_id);
    insert_nonempty(
        &mut track.extensions,
        "ring_copyright_id",
        &song.ring_copyright_id,
    );
    insert_nonempty(&mut track.extensions, "shock_ring_id", &song.shock_ring_id);
    insert_optional(
        &mut track.extensions,
        "has_associated_ring",
        song.has_associated_ring,
    );
    insert_nonempty(&mut track.extensions, "song_pinyin", &song.song_pinyin);
    insert_nonempty(&mut track.extensions, "album_pinyin", &song.album_pinyin);
    insert_nonempty(
        &mut track.extensions,
        "play_count_display",
        &song.play_num_desc,
    );
    insert_nonempty(
        &mut track.extensions,
        "forever_listen_flag",
        &song.forever_listen_flag,
    );
    insert_optional(&mut track.extensions, "forever_listen", song.forever_listen);
    insert_nonempty(
        &mut track.extensions,
        "product_authorize_usage",
        &song.product_authorize_usage,
    );
    insert_nonempty(&mut track.extensions, "audio_book", &song.audio_book);
    if let Some(value) = song.chorus_start_time.as_ref().and_then(FlexibleU64::get) {
        track
            .extensions
            .insert("chorus_start_ms".to_owned(), json!(value));
    }
    if !song.ext.disc.trim().is_empty() {
        track
            .extensions
            .insert("disc".to_owned(), json!(song.ext.disc.trim()));
    }
    insert_string_list(&mut track.extensions, "show_tags", song.show_tags);
    insert_string_list(&mut track.extensions, "download_tags", song.download_tags);
    insert_string_list(&mut track.extensions, "highlights", song.highlights);
    let search_tags = bounded_search_tags(&song.search_tags);
    if !search_tags.is_empty() {
        track
            .extensions
            .insert("search_tags".to_owned(), json!(search_tags));
    }
    let audio_formats = bounded_audio_formats(&song.audio_formats);
    if !audio_formats.is_empty() {
        track
            .extensions
            .insert("audio_formats".to_owned(), json!(audio_formats));
    }
    if !alternates.is_empty() {
        track
            .extensions
            .insert("alternate_versions".to_owned(), json!(alternates));
    }
    insert_safe_media_url(&mut track.extensions, "lrc_url", &song.lrc_url);
    insert_safe_media_url(&mut track.extensions, "mrc_url", &song.mrc_url);
    insert_safe_media_url(
        &mut track.extensions,
        "action_image_url",
        &song.action_img_url,
    );
    Ok(track)
}

fn map_artist(singer: &MiguSinger) -> Option<ArtistSummary> {
    let name = validated_name(&singer.name)?;
    let resource_ref = canonical_platform_id(&singer.id)
        .and_then(|id| ResourceRef::new(Platform::Migu, id.to_owned()).ok());
    Some(ArtistSummary {
        resource_ref,
        name: name.to_owned(),
    })
}

fn map_album(song: &MiguSong) -> Option<AlbumSummary> {
    let name = validated_name(&song.album)?;
    let resource_ref = canonical_platform_id(&song.album_id)
        .and_then(|id| ResourceRef::new(Platform::Migu, id.to_owned()).ok());
    let cover_url = [&song.img3, &song.img2, &song.img1]
        .into_iter()
        .find_map(|value| normalize_media_url(value));
    Some(AlbumSummary {
        resource_ref,
        name: name.to_owned(),
        cover_url,
    })
}

fn map_alternate(song: &MiguSong) -> Option<MiguAlternateTrack> {
    let content_id = canonical_platform_id(&song.content_id)?;
    let name = validated_name(&song.song_name)?;
    if song.resource_type != "2" {
        return None;
    }
    Some(MiguAlternateTrack {
        content_id: content_id.to_owned(),
        song_id: canonical_platform_id(&song.song_id).map(str::to_owned),
        name: name.to_owned(),
        resource_type: song.resource_type.clone(),
        artists: song
            .singer_list
            .iter()
            .filter_map(map_alternate_singer)
            .collect(),
        album_id: canonical_platform_id(&song.album_id).map(str::to_owned),
        album: validated_name(&song.album).map(str::to_owned),
        duration_seconds: song.duration.as_ref().and_then(FlexibleU64::get),
        copyright_id: canonical_platform_id(&song.copyright_id).map(str::to_owned),
        copyright_type: song.copyright_type,
        restrict_type: song.restrict_type,
        forever_listen: song.forever_listen,
        audio_formats: bounded_audio_formats(&song.audio_formats),
        cover_url: [&song.img3, &song.img2, &song.img1]
            .into_iter()
            .find_map(|value| normalize_media_url(value)),
        lrc_url: normalize_media_url(&song.lrc_url),
        mrc_url: normalize_media_url(&song.mrc_url),
        highlights: bounded_string_list(&song.highlights, 32, 256),
    })
}

fn map_alternate_singer(singer: &MiguSinger) -> Option<MiguAlternateSinger> {
    Some(MiguAlternateSinger {
        id: canonical_platform_id(&singer.id).map(str::to_owned),
        name: validated_name(&singer.name)?.to_owned(),
        name_spelling: validated_name(&singer.name_spelling).map(str::to_owned),
        image_url: normalize_media_url(&singer.img),
    })
}

fn mapped_qualities(formats: &[MiguAudioFormat]) -> Vec<Quality> {
    let mut qualities = Vec::new();
    for format in formats {
        let quality = match format.format_type.as_str() {
            "PQ" => Some(Quality::Standard),
            "HQ" => Some(Quality::High),
            "SQ" => Some(Quality::Lossless),
            "ZQ24" => Some(Quality::Hires),
            _ => None,
        };
        if let Some(quality) = quality
            && !qualities.contains(&quality)
        {
            qualities.push(quality);
        }
    }
    qualities
}

fn normalize_media_url(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    let url = if value.starts_with('/') {
        if !value.starts_with("/data/oss/") {
            return None;
        }
        Url::parse(&format!("https://{MEDIA_HOST}{value}")).ok()?
    } else {
        Url::parse(value).ok()?
    };
    if url.scheme() != "https"
        || url.host_str() != Some(MEDIA_HOST)
        || url.username() != ""
        || url.password().is_some()
        || !matches!(url.port(), None | Some(443))
        || !url.path().starts_with("/data/oss/")
    {
        return None;
    }
    Some(url.into())
}

fn canonical_platform_id(value: &str) -> Option<&str> {
    let value = value.trim();
    (!value.is_empty()
        && value.len() <= 64
        && value.bytes().all(|byte| byte.is_ascii_alphanumeric()))
    .then_some(value)
}

fn validated_name(value: &str) -> Option<&str> {
    let value = value.trim();
    (!value.is_empty()
        && value.len() <= 2_048
        && !value.chars().any(|character| character.is_control()))
    .then_some(value)
}

fn nonempty(value: &str) -> Option<&str> {
    let value = value.trim();
    (!value.is_empty()).then_some(value)
}

fn bounded_text(value: &str, max_bytes: usize) -> String {
    value
        .chars()
        .filter(|character| !character.is_control())
        .scan(0_usize, |used, character| {
            let next = used.saturating_add(character.len_utf8());
            (next <= max_bytes).then(|| {
                *used = next;
                character
            })
        })
        .collect()
}

fn bounded_string_list(values: &[String], max_items: usize, max_bytes: usize) -> Vec<String> {
    values
        .iter()
        .take(max_items)
        .map(|value| bounded_text(value, max_bytes))
        .filter(|value| !value.is_empty())
        .collect()
}

fn bounded_conditions(mut values: Vec<MiguSearchCondition>) -> Vec<MiguSearchCondition> {
    values.truncate(16);
    for condition in &mut values {
        condition.id = bounded_text(&condition.id, 128);
        condition.title = bounded_text(&condition.title, 256);
        condition.kind = bounded_text(&condition.kind, 128);
        condition.datas.truncate(32);
        for value in &mut condition.datas {
            value.title = bounded_text(&value.title, 256);
            value.condition_type = bounded_text(&value.condition_type, 128);
        }
    }
    values
}

fn bounded_search_tags(values: &[MiguSearchTag]) -> Vec<MiguSearchTag> {
    values
        .iter()
        .take(64)
        .map(|value| MiguSearchTag {
            name: bounded_text(&value.name, 256),
            kind: bounded_text(&value.kind, 128),
        })
        .filter(|value| !value.name.is_empty() || !value.kind.is_empty())
        .collect()
}

fn bounded_audio_formats(values: &[MiguAudioFormat]) -> Vec<MiguAudioFormat> {
    values
        .iter()
        .take(32)
        .map(|value| MiguAudioFormat {
            resource_type: bounded_text(&value.resource_type, 32),
            format_type: bounded_text(&value.format_type, 32),
            show_tags: bounded_string_list(&value.show_tags, 32, 128),
            isize: value.isize.clone(),
            asize: value.asize.clone(),
            iformat: bounded_text(&value.iformat, 64),
            aformat: bounded_text(&value.aformat, 64),
        })
        .collect()
}

fn insert_nonempty(extensions: &mut tuneweave_core::Extensions, key: &str, value: &str) {
    if let Some(value) = nonempty(value) {
        extensions.insert(key.to_owned(), json!(bounded_text(value, 2_048)));
    }
}

fn insert_optional<T: Serialize>(
    extensions: &mut tuneweave_core::Extensions,
    key: &str,
    value: Option<T>,
) {
    if let Some(value) = value {
        extensions.insert(key.to_owned(), json!(value));
    }
}

fn insert_string_list(extensions: &mut tuneweave_core::Extensions, key: &str, values: Vec<String>) {
    let values = bounded_string_list(&values, 64, 256);
    if !values.is_empty() {
        extensions.insert(key.to_owned(), json!(values));
    }
}

fn insert_safe_media_url(extensions: &mut tuneweave_core::Extensions, key: &str, value: &str) {
    if let Some(url) = normalize_media_url(value) {
        extensions.insert(key.to_owned(), json!(url));
    }
}

async fn read_bounded_response(
    mut response: reqwest::Response,
    operation: &str,
) -> Result<Vec<u8>> {
    let status = response.status();
    if !status.is_success() {
        return Err(migu_http_error(status));
    }
    if response
        .headers()
        .get(CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        .is_some_and(|length| length > MAX_API_RESPONSE_BYTES)
    {
        return Err(migu_upstream_error(format!(
            "{operation} response exceeded the size limit"
        )));
    }
    let max_size = usize::try_from(MAX_API_RESPONSE_BYTES).unwrap_or(usize::MAX);
    let initial_capacity = response
        .content_length()
        .and_then(|length| usize::try_from(length).ok())
        .unwrap_or_default()
        .min(max_size);
    let mut bytes = Vec::with_capacity(initial_capacity);
    while let Some(chunk) = response.chunk().await.map_err(migu_network_error)? {
        if bytes.len().saturating_add(chunk.len()) > max_size {
            return Err(migu_upstream_error(format!(
                "{operation} response exceeded the size limit"
            )));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

fn migu_network_error(error: reqwest::Error) -> TuneWeaveError {
    let code = if error.is_timeout() {
        ErrorCode::UpstreamTimeout
    } else {
        ErrorCode::UpstreamError
    };
    TuneWeaveError::new(code, "Migu API request failed")
        .with_platform(Platform::Migu)
        .retryable(true)
}

fn migu_http_error(status: StatusCode) -> TuneWeaveError {
    let code = if status == StatusCode::TOO_MANY_REQUESTS {
        ErrorCode::RateLimited
    } else {
        ErrorCode::UpstreamError
    };
    TuneWeaveError::new(code, format!("Migu API returned HTTP {status}"))
        .with_platform(Platform::Migu)
        .retryable(status.is_server_error() || status == StatusCode::TOO_MANY_REQUESTS)
}

fn migu_invalid_request(message: impl Into<String>) -> TuneWeaveError {
    TuneWeaveError::invalid_request(message).with_platform(Platform::Migu)
}

fn migu_upstream_error(message: impl Into<String>) -> TuneWeaveError {
    TuneWeaveError::new(ErrorCode::UpstreamError, message).with_platform(Platform::Migu)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEARCH_RESPONSE: &str = r#"{
      "code":"000000",
      "info":"操作成功",
      "data":{
        "hasNext":true,
        "seq":"safe-sequence",
        "conditions":[{"id":"1","title":"默认","type":"order","datas":[{"title":"最新","conditionType":"1"}]}],
        "items":[{"song":{
          "resourceType":"2",
          "contentId":"600908000007288315",
          "songId":"1004202180",
          "songName":"告白气球",
          "mvId":"600906000000389715",
          "audioFormats":[
            {"resourceType":"2","formatType":"PQ","isize":"3450883","aformat":"020007"},
            {"resourceType":"2","formatType":"HQ","asize":8626889,"aformat":"020010"},
            {"resourceType":"E","formatType":"SQ","aformat":"011002"},
            {"resourceType":"2","formatType":"ZQ24","aformat":"011005"},
            {"resourceType":"2","formatType":"AV3A","aformat":"020041"}
          ],
          "duration":"215",
          "copyrightId":"60054704028",
          "copyrightType":1,
          "restrictType":1,
          "albumId":"1003767159",
          "album":"周杰伦的床边故事",
          "img3":"/data/oss/resource/cover.webp",
          "singerList":[{"id":"112","name":"周杰伦","img":"https://d.musicapp.migu.cn/data/oss/resource/artist.webp"}],
          "foreverListenFlag":"0",
          "foreverListen":false,
          "lrcUrl":"https://evil.example/lrc",
          "mrcUrl":"https://d.musicapp.migu.cn/data/oss/resource/word",
          "more":[{
            "resourceType":"2",
            "contentId":"600913000007163490",
            "songId":"1125329696",
            "songName":"告白气球 (Live)",
            "duration":264,
            "audioFormats":[{"resourceType":"2","formatType":"AV3A","aformat":"020041"}],
            "singerList":[{"id":"112","name":"周杰伦"}],
            "albumId":"1125329686",
            "album":"巡回演唱会",
            "img3":"/data/oss/resource/live.webp",
            "highlights":["周杰伦"]
          }]
        }}]
      }
    }"#;

    #[test]
    fn search_mapping_preserves_stable_identity_metadata_and_exact_formats() {
        let page = parse_search_response(SEARCH_RESPONSE.as_bytes()).expect("parse search");
        assert!(page.has_next);
        assert_eq!(page.sequence.as_deref(), Some("safe-sequence"));
        assert_eq!(page.conditions.len(), 1);
        let track = &page.tracks[0];
        assert_eq!(track.resource_ref.to_string(), "migu:600908000007288315");
        assert_eq!(track.name, "告白气球");
        assert_eq!(track.duration_ms, Some(215_000));
        assert_eq!(track.artists[0].name, "周杰伦");
        assert_eq!(
            track
                .album
                .as_ref()
                .and_then(|album| album.cover_url.as_deref()),
            Some("https://d.musicapp.migu.cn/data/oss/resource/cover.webp")
        );
        assert_eq!(
            track.available_qualities,
            vec![
                Quality::Standard,
                Quality::High,
                Quality::Lossless,
                Quality::Hires
            ]
        );
        let formats = track.extensions["audio_formats"]
            .as_array()
            .expect("audio formats");
        assert_eq!(formats[4]["formatType"], "AV3A");
        assert!(!track.extensions.contains_key("lrc_url"));
        assert_eq!(
            track.extensions["mrc_url"],
            "https://d.musicapp.migu.cn/data/oss/resource/word"
        );
        assert_eq!(
            track.extensions["alternate_versions"][0]["content_id"],
            "600913000007163490"
        );
        assert_eq!(
            track.extensions["alternate_versions"][0]["audio_formats"][0]["formatType"],
            "AV3A"
        );
        assert_eq!(track.playable, None);
    }

    #[test]
    fn search_rejects_business_errors_and_malformed_song_identity() {
        let rejected = br#"{"code":"100001","info":"failed","data":null}"#;
        let error = parse_search_response(rejected).expect_err("business error");
        assert_eq!(error.code, ErrorCode::UpstreamError);
        assert_eq!(error.details["platform_code"], "100001");

        for bad in [
            br#"{"code":"000000","data":{"items":[{"song":{"resourceType":"2","contentId":"","songName":"name"}}]}}"#.as_slice(),
            br#"{"code":"000000","data":{"items":[{"song":{"resourceType":"V","contentId":"123","songName":"name"}}]}}"#.as_slice(),
            br#"{"code":"000000","data":{"items":[{"song":{"resourceType":"2","contentId":"123","songName":""}}]}}"#.as_slice(),
        ] {
            assert!(parse_search_response(bad).is_err());
        }
    }

    #[test]
    fn media_urls_accept_only_fixed_https_migu_storage_paths() {
        assert_eq!(
            normalize_media_url("/data/oss/resource/a.webp").as_deref(),
            Some("https://d.musicapp.migu.cn/data/oss/resource/a.webp")
        );
        assert_eq!(
            normalize_media_url("https://d.musicapp.migu.cn/data/oss/resource/a.webp").as_deref(),
            Some("https://d.musicapp.migu.cn/data/oss/resource/a.webp")
        );
        for value in [
            "http://d.musicapp.migu.cn/data/oss/a",
            "https://evil.example/data/oss/a",
            "https://d.musicapp.migu.cn:444/data/oss/a",
            "https://user@d.musicapp.migu.cn/data/oss/a",
            "/other/path",
        ] {
            assert!(normalize_media_url(value).is_none(), "{value} must fail");
        }
    }

    #[test]
    fn configuration_debug_redacts_proxy_credentials() {
        let config = MiguConfig {
            proxy_url: Some("http://secret@example.test:8080".to_owned()),
        };
        let debug = format!("{config:?}");
        assert!(!debug.contains("secret"));
        assert!(!debug.contains("example.test"));
    }

    #[tokio::test]
    #[ignore = "requires live Migu network access"]
    async fn live_public_search_returns_stable_tracks_over_https() {
        let client = MiguClient::test_client();
        let page = client
            .search_tracks_page("周杰伦", 1)
            .await
            .expect("live Migu search");
        assert!(!page.tracks.is_empty());
        assert!(
            page.tracks
                .iter()
                .all(|track| track.resource_ref.platform() == Platform::Migu)
        );
    }
}
