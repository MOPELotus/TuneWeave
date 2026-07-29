use std::{collections::BTreeMap, fmt, time::Duration};

use reqwest::{
    Client, Proxy, StatusCode,
    header::{ACCEPT, CONTENT_LENGTH, REFERER},
    redirect::Policy,
};
use serde::{Deserialize, Serialize};
use serde_json::{Number, json};
use tuneweave_core::{
    AlbumSummary, ArtistSummary, ErrorCode, Platform, Quality, ResourceRef, Result, Track,
    TuneWeaveError,
};

const SEARCH_ENDPOINT: &str = "https://www.kuwo.cn/search/searchMusicBykeyWord";
const SEARCH_REFERER: &str = "https://www.kuwo.cn/search/list";
const ALBUM_IMAGE_PREFIX: &str = "https://img2.kuwo.cn/star/albumcover/";
const ARTIST_IMAGE_PREFIX: &str = "https://img1.kuwo.cn/star/starheads/";
const MAX_API_RESPONSE_BYTES: u64 = 8 * 1024 * 1024;
const USER_AGENT: &str = "TuneWeave/0.1 (Kuwo public music provider)";

#[derive(Clone, Default)]
pub struct KuwoConfig {
    pub proxy_url: Option<String>,
}

impl fmt::Debug for KuwoConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("KuwoConfig")
            .field(
                "proxy_url",
                &self.proxy_url.as_ref().map(|_| "[configured]"),
            )
            .finish()
    }
}

#[derive(Clone)]
pub struct KuwoClient {
    http: Client,
}

impl fmt::Debug for KuwoClient {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("KuwoClient").finish_non_exhaustive()
    }
}

#[derive(Debug)]
pub(crate) struct KuwoSearchPage {
    pub tracks: Vec<Track>,
    pub total: u64,
}

#[derive(Serialize)]
struct KuwoSearchQuery<'a> {
    vipver: u8,
    client: &'static str,
    ft: &'static str,
    cluster: u8,
    strategy: u16,
    encoding: &'static str,
    rformat: &'static str,
    mobi: u8,
    issubtitle: u8,
    show_copyright_off: u8,
    pn: u32,
    rn: u32,
    all: &'a str,
    #[serde(rename = "httpsStatus")]
    https_status: u8,
    #[serde(rename = "reqId")]
    request_id: String,
    plat: &'static str,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct KuwoSearchEnvelope {
    #[serde(rename = "PN")]
    page: FlexibleText,
    #[serde(rename = "RN")]
    page_size: FlexibleText,
    #[serde(rename = "TOTAL")]
    total: FlexibleText,
    abslist: Vec<KuwoSearchTrack>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default)]
struct KuwoSearchTrack {
    #[serde(rename = "MUSICRID")]
    music_rid: String,
    #[serde(rename = "SONGNAME")]
    song_name: String,
    #[serde(rename = "NAME")]
    name: String,
    #[serde(rename = "ALIAS")]
    alias: String,
    #[serde(rename = "SUBTITLE")]
    subtitle: String,
    #[serde(rename = "ARTIST")]
    artist: String,
    #[serde(rename = "ARTISTID")]
    artist_id: FlexibleText,
    #[serde(rename = "AARTIST")]
    romanized_artist: String,
    #[serde(rename = "ALBUM")]
    album: String,
    #[serde(rename = "ALBUMID")]
    album_id: FlexibleText,
    #[serde(rename = "DURATION")]
    duration_seconds: FlexibleText,
    #[serde(rename = "ONLINE")]
    online: FlexibleText,
    #[serde(rename = "PAY")]
    pay: FlexibleText,
    #[serde(rename = "MVFLAG")]
    mv_flag: FlexibleText,
    #[serde(rename = "FORMAT")]
    format: String,
    #[serde(rename = "MINFO")]
    media_info: String,
    #[serde(rename = "N_MINFO")]
    new_media_info: String,
    #[serde(rename = "web_albumpic_short")]
    album_picture_path: String,
    #[serde(rename = "web_artistpic_short")]
    artist_picture_path: String,
    #[serde(rename = "originalsongtype")]
    original_song_type: FlexibleText,
    #[serde(rename = "content_type")]
    content_type: FlexibleText,
    #[serde(rename = "ad_type")]
    ad_type: String,
    #[serde(rename = "ad_subtype")]
    ad_subtype: String,
    #[serde(rename = "tme_musician_adtype")]
    musician_ad_type: FlexibleText,
    #[serde(rename = "payInfo")]
    pay_info: Option<KuwoPayInfo>,
    mvpayinfo: Option<KuwoMvPayInfo>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default)]
struct KuwoPayInfo {
    #[serde(rename = "cannotDownload")]
    cannot_download: FlexibleText,
    #[serde(rename = "cannotOnlinePlay")]
    cannot_online_play: FlexibleText,
    down: FlexibleText,
    download: FlexibleText,
    #[serde(rename = "extendAttr")]
    extend_attr: FlexibleText,
    #[serde(rename = "feeType")]
    fee_type: KuwoFeeType,
    limitfree: FlexibleText,
    listen_fragment: FlexibleText,
    local_encrypt: FlexibleText,
    ndown: FlexibleText,
    nplay: FlexibleText,
    overseas_ndown: FlexibleText,
    overseas_nplay: FlexibleText,
    paytagindex: BTreeMap<String, i64>,
    paytype: FlexibleText,
    play: FlexibleText,
    refrain_end: FlexibleText,
    refrain_start: FlexibleText,
    tips_intercept: FlexibleText,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default)]
struct KuwoFeeType {
    album: FlexibleText,
    bookvip: FlexibleText,
    song: FlexibleText,
    vip: FlexibleText,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default)]
struct KuwoMvPayInfo {
    down: FlexibleText,
    download: FlexibleText,
    play: FlexibleText,
    vid: FlexibleText,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(untagged)]
enum FlexibleText {
    String(String),
    Number(Number),
    Boolean(bool),
    #[default]
    Null,
}

impl FlexibleText {
    fn as_text(&self) -> Option<String> {
        match self {
            Self::String(value) => nonempty(value).map(str::to_owned),
            Self::Number(value) => Some(value.to_string()),
            Self::Boolean(value) => Some(value.to_string()),
            Self::Null => None,
        }
    }

    fn as_u64(&self) -> Option<u64> {
        match self {
            Self::String(value) => value.parse().ok(),
            Self::Number(value) => value.as_u64(),
            Self::Boolean(_) | Self::Null => None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct KuwoMediaSpec {
    level: String,
    bitrate: Option<u64>,
    format: Option<String>,
    size: Option<String>,
    source: &'static str,
}

impl KuwoClient {
    pub fn new(config: &KuwoConfig) -> Result<Self> {
        let mut builder = Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(20))
            .redirect(Policy::none())
            .user_agent(USER_AGENT);
        if let Some(proxy_url) = config.proxy_url.as_deref() {
            let proxy = Proxy::all(proxy_url).map_err(|_| {
                kuwo_invalid_request("Kuwo proxy configuration is not a valid proxy URL")
            })?;
            builder = builder.proxy(proxy);
        }
        let http = builder.build().map_err(|_| {
            TuneWeaveError::new(ErrorCode::InternalError, "failed to build Kuwo HTTP client")
                .with_platform(Platform::Kuwo)
        })?;
        Ok(Self { http })
    }

    #[cfg(test)]
    pub(crate) fn test_client() -> Self {
        Self::new(&KuwoConfig::default()).expect("create Kuwo test client")
    }

    pub(crate) async fn search_tracks_page(
        &self,
        keyword: &str,
        page: u32,
        page_size: u32,
    ) -> Result<KuwoSearchPage> {
        let query = KuwoSearchQuery {
            vipver: 1,
            client: "kt",
            ft: "music",
            cluster: 0,
            strategy: 2012,
            encoding: "utf8",
            rformat: "json",
            mobi: 1,
            issubtitle: 1,
            show_copyright_off: 1,
            pn: page,
            rn: page_size,
            all: keyword,
            https_status: 1,
            request_id: new_request_id(),
            plat: "web_www",
        };
        let response = self
            .http
            .get(SEARCH_ENDPOINT)
            .header(ACCEPT, "application/json, text/plain")
            .header(REFERER, SEARCH_REFERER)
            .query(&query)
            .send()
            .await
            .map_err(kuwo_network_error)?;
        let bytes = read_bounded_response(response, "Kuwo search").await?;
        parse_search_response(&bytes, page, page_size)
    }
}

fn parse_search_response(
    bytes: &[u8],
    requested_page: u32,
    requested_size: u32,
) -> Result<KuwoSearchPage> {
    let envelope: KuwoSearchEnvelope = serde_json::from_slice(bytes)
        .map_err(|_| kuwo_upstream_error("Kuwo search returned malformed JSON"))?;
    let page = envelope
        .page
        .as_u64()
        .ok_or_else(|| kuwo_upstream_error("Kuwo search omitted a valid page number"))?;
    let page_size = envelope
        .page_size
        .as_u64()
        .ok_or_else(|| kuwo_upstream_error("Kuwo search omitted a valid page size"))?;
    let total = envelope
        .total
        .as_u64()
        .ok_or_else(|| kuwo_upstream_error("Kuwo search omitted a valid total count"))?;
    if page != u64::from(requested_page) || page_size != u64::from(requested_size) {
        return Err(kuwo_upstream_error(
            "Kuwo search returned mismatched pagination",
        ));
    }
    let returned = u64::try_from(envelope.abslist.len()).unwrap_or(u64::MAX);
    if returned > page_size {
        return Err(kuwo_upstream_error(
            "Kuwo search exceeded the requested page size",
        ));
    }
    let page_start = page
        .checked_mul(page_size)
        .ok_or_else(|| kuwo_upstream_error("Kuwo search page position overflowed"))?;
    if (returned == 0 && page_start < total) || page_start.saturating_add(returned) > total {
        return Err(kuwo_upstream_error(
            "Kuwo search returned inconsistent pagination",
        ));
    }
    let tracks = envelope
        .abslist
        .into_iter()
        .map(map_search_track)
        .collect::<Result<Vec<_>>>()?;
    Ok(KuwoSearchPage { tracks, total })
}

fn map_search_track(item: KuwoSearchTrack) -> Result<Track> {
    let track_id = item
        .music_rid
        .strip_prefix("MUSIC_")
        .and_then(canonical_positive_decimal)
        .ok_or_else(|| kuwo_upstream_error("Kuwo search result omitted a stable music ID"))?;
    let name = nonempty(&item.song_name)
        .or_else(|| nonempty(&item.name))
        .ok_or_else(|| kuwo_upstream_error("Kuwo search result omitted a track name"))?;
    let resource_ref = ResourceRef::new(Platform::Kuwo, track_id.to_owned())
        .map_err(|_| kuwo_upstream_error("Kuwo search returned an invalid track identity"))?;
    let mut track = Track::new(resource_ref, bounded_text(name, 512));
    push_alias(&mut track.aliases, &track.name, &item.alias);
    push_alias(&mut track.aliases, &track.name, &item.subtitle);
    track.artists = map_artists(&item.artist, &item.artist_id);
    let album_cover = normalize_image_path(&item.album_picture_path, ALBUM_IMAGE_PREFIX);
    track.album = map_album(&item.album, &item.album_id, album_cover.clone());
    track.duration_ms = item
        .duration_seconds
        .as_u64()
        .and_then(|seconds| seconds.checked_mul(1_000));
    track.mv_ref = item
        .mvpayinfo
        .as_ref()
        .and_then(|info| info.vid.as_text())
        .as_deref()
        .and_then(canonical_positive_decimal)
        .and_then(|id| ResourceRef::new(Platform::Kuwo, id.to_owned()).ok());
    if item.online.as_text().as_deref() == Some("0") {
        track.playable = Some(false);
    }

    let media_specs = parse_media_specs(&item.media_info, "legacy")
        .into_iter()
        .chain(parse_media_specs(&item.new_media_info, "current"))
        .fold(Vec::<KuwoMediaSpec>::new(), |mut specs, spec| {
            if !specs.iter().any(|existing| {
                existing.level == spec.level
                    && existing.bitrate == spec.bitrate
                    && existing.format == spec.format
                    && existing.size == spec.size
            }) {
                specs.push(spec);
            }
            specs
        });
    track.available_qualities = map_qualities(&media_specs);
    track
        .extensions
        .insert("backend".to_owned(), json!("current_web_search"));
    insert_optional_text(
        &mut track.extensions,
        "artist_romanization",
        &item.romanized_artist,
    );
    insert_optional_text(
        &mut track.extensions,
        "artist_image_url",
        &normalize_image_path(&item.artist_picture_path, ARTIST_IMAGE_PREFIX).unwrap_or_default(),
    );
    insert_flexible(&mut track.extensions, "online", &item.online);
    insert_flexible(&mut track.extensions, "pay", &item.pay);
    insert_flexible(
        &mut track.extensions,
        "original_song_type",
        &item.original_song_type,
    );
    insert_flexible(&mut track.extensions, "content_type", &item.content_type);
    insert_optional_text(&mut track.extensions, "ad_type", &item.ad_type);
    insert_optional_text(&mut track.extensions, "ad_subtype", &item.ad_subtype);
    insert_flexible(
        &mut track.extensions,
        "musician_ad_type",
        &item.musician_ad_type,
    );
    insert_optional_text(&mut track.extensions, "catalog_format", &item.format);
    if !media_specs.is_empty() {
        track
            .extensions
            .insert("media_specs".to_owned(), json!(media_specs));
    }
    if let Some(pay_info) = item.pay_info {
        track
            .extensions
            .insert("pay_info".to_owned(), json!(pay_info));
    }
    if let Some(mv_pay_info) = item.mvpayinfo {
        track
            .extensions
            .insert("mv_pay_info".to_owned(), json!(mv_pay_info));
    }
    if let Some(cover) = album_cover {
        track
            .extensions
            .insert("album_image_url".to_owned(), json!(cover));
    }
    insert_flexible(&mut track.extensions, "mv_flag", &item.mv_flag);
    Ok(track)
}

fn map_artists(name: &str, id: &FlexibleText) -> Vec<ArtistSummary> {
    let names = bounded_parts(name, '&', 16);
    let ids = id
        .as_text()
        .map(|value| bounded_parts(&value, '&', 16))
        .unwrap_or_default();
    if names.is_empty() {
        return Vec::new();
    }
    if names.len() == ids.len() {
        return names
            .into_iter()
            .zip(ids)
            .map(|(name, id)| ArtistSummary {
                resource_ref: canonical_positive_decimal(&id)
                    .and_then(|id| ResourceRef::new(Platform::Kuwo, id.to_owned()).ok()),
                name,
            })
            .collect();
    }
    vec![ArtistSummary {
        resource_ref: id
            .as_text()
            .as_deref()
            .and_then(canonical_positive_decimal)
            .and_then(|id| ResourceRef::new(Platform::Kuwo, id.to_owned()).ok()),
        name: bounded_text(name.trim(), 512),
    }]
}

fn map_album(name: &str, id: &FlexibleText, cover_url: Option<String>) -> Option<AlbumSummary> {
    let name = nonempty(name)?;
    Some(AlbumSummary {
        resource_ref: id
            .as_text()
            .as_deref()
            .and_then(canonical_positive_decimal)
            .and_then(|id| ResourceRef::new(Platform::Kuwo, id.to_owned()).ok()),
        name: bounded_text(name, 512),
        cover_url,
    })
}

fn parse_media_specs(value: &str, source: &'static str) -> Vec<KuwoMediaSpec> {
    value
        .split(';')
        .take(64)
        .filter_map(|record| {
            let fields = record
                .split(',')
                .filter_map(|field| field.split_once(':'))
                .map(|(key, value)| (key.trim(), value.trim()))
                .collect::<BTreeMap<_, _>>();
            let level = fields
                .get("level")
                .copied()
                .filter(|level| valid_code(level, 32))?;
            let format = fields
                .get("format")
                .copied()
                .filter(|format| valid_code(format, 32))
                .map(str::to_owned);
            let size = fields
                .get("size")
                .copied()
                .filter(|size| valid_display_text(size, 64))
                .map(str::to_owned);
            Some(KuwoMediaSpec {
                level: level.to_owned(),
                bitrate: fields
                    .get("bitrate")
                    .and_then(|bitrate| bitrate.parse().ok()),
                format,
                size,
                source,
            })
        })
        .collect()
}

fn map_qualities(specs: &[KuwoMediaSpec]) -> Vec<Quality> {
    let mut qualities = Vec::new();
    for quality in [
        Quality::Low,
        Quality::Standard,
        Quality::High,
        Quality::Lossless,
        Quality::Hires,
        Quality::Surround,
    ] {
        if specs
            .iter()
            .any(|spec| quality_for_level(&spec.level) == Some(quality))
        {
            qualities.push(quality);
        }
    }
    qualities
}

fn quality_for_level(level: &str) -> Option<Quality> {
    match level.to_ascii_lowercase().as_str() {
        "s" => Some(Quality::Low),
        "h" => Some(Quality::Standard),
        "p" => Some(Quality::High),
        "ff" => Some(Quality::Lossless),
        "hr" => Some(Quality::Hires),
        "dtsx" => Some(Quality::Surround),
        _ => None,
    }
}

fn normalize_image_path(value: &str, prefix: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty()
        || value.len() > 512
        || value.starts_with('/')
        || value.contains("..")
        || value.contains('\\')
        || value.contains('?')
        || value.contains('#')
        || value.chars().any(char::is_control)
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'.' | b'_' | b'-'))
    {
        return None;
    }
    Some(format!("{prefix}{value}"))
}

fn push_alias(aliases: &mut Vec<String>, name: &str, value: &str) {
    let Some(value) = nonempty(value) else {
        return;
    };
    let value = bounded_text(value, 512);
    if value != name && !aliases.iter().any(|alias| alias == &value) {
        aliases.push(value);
    }
}

fn bounded_parts(value: &str, separator: char, limit: usize) -> Vec<String> {
    value
        .split(separator)
        .take(limit)
        .filter_map(nonempty)
        .map(|part| bounded_text(part, 512))
        .collect()
}

fn canonical_positive_decimal(value: &str) -> Option<&str> {
    let value = value.trim();
    value
        .parse::<u64>()
        .ok()
        .filter(|parsed| *parsed > 0 && parsed.to_string() == value)
        .map(|_| value)
}

fn valid_code(value: &str, limit: usize) -> bool {
    !value.is_empty()
        && value.len() <= limit
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
}

fn valid_display_text(value: &str, limit: usize) -> bool {
    !value.is_empty() && value.len() <= limit && !value.chars().any(char::is_control)
}

fn nonempty(value: &str) -> Option<&str> {
    let value = value.trim();
    (!value.is_empty()).then_some(value)
}

fn bounded_text(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}

fn insert_optional_text(extensions: &mut tuneweave_core::Extensions, key: &str, value: &str) {
    if let Some(value) = nonempty(value) {
        extensions.insert(key.to_owned(), json!(bounded_text(value, 512)));
    }
}

fn insert_flexible(extensions: &mut tuneweave_core::Extensions, key: &str, value: &FlexibleText) {
    if let Some(value) = value.as_text() {
        extensions.insert(key.to_owned(), json!(bounded_text(&value, 512)));
    }
}

fn new_request_id() -> String {
    let mut bytes = rand::random::<[u8; 16]>();
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0],
        bytes[1],
        bytes[2],
        bytes[3],
        bytes[4],
        bytes[5],
        bytes[6],
        bytes[7],
        bytes[8],
        bytes[9],
        bytes[10],
        bytes[11],
        bytes[12],
        bytes[13],
        bytes[14],
        bytes[15]
    )
}

async fn read_bounded_response(response: reqwest::Response, operation: &str) -> Result<Vec<u8>> {
    let mut response = response;
    let status = response.status();
    if !status.is_success() {
        return Err(kuwo_http_error(status));
    }
    if response
        .headers()
        .get(CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        .is_some_and(|length| length > MAX_API_RESPONSE_BYTES)
    {
        return Err(kuwo_upstream_error(format!(
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
    while let Some(chunk) = response.chunk().await.map_err(kuwo_network_error)? {
        if bytes.len().saturating_add(chunk.len()) > max_size {
            return Err(kuwo_upstream_error(format!(
                "{operation} response exceeded the size limit"
            )));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

fn kuwo_network_error(error: reqwest::Error) -> TuneWeaveError {
    let code = if error.is_timeout() {
        ErrorCode::UpstreamTimeout
    } else {
        ErrorCode::UpstreamError
    };
    TuneWeaveError::new(code, "Kuwo API request failed")
        .with_platform(Platform::Kuwo)
        .retryable(true)
}

fn kuwo_http_error(status: StatusCode) -> TuneWeaveError {
    let code = if status == StatusCode::TOO_MANY_REQUESTS {
        ErrorCode::RateLimited
    } else {
        ErrorCode::UpstreamError
    };
    TuneWeaveError::new(code, format!("Kuwo API returned HTTP {status}"))
        .with_platform(Platform::Kuwo)
        .retryable(status.is_server_error() || status == StatusCode::TOO_MANY_REQUESTS)
}

fn kuwo_invalid_request(message: impl Into<String>) -> TuneWeaveError {
    TuneWeaveError::invalid_request(message).with_platform(Platform::Kuwo)
}

fn kuwo_upstream_error(message: impl Into<String>) -> TuneWeaveError {
    TuneWeaveError::new(ErrorCode::UpstreamError, message).with_platform(Platform::Kuwo)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEARCH_RESPONSE: &str = r#"{
      "PN":"0",
      "RN":"2",
      "TOTAL":"3600",
      "abslist":[
        {
          "MUSICRID":"MUSIC_228908",
          "SONGNAME":"晴天",
          "NAME":"晴天",
          "ALIAS":"",
          "SUBTITLE":"《左手上篮》动画片尾曲",
          "ARTIST":"周杰伦",
          "ARTISTID":"336",
          "AARTIST":"Jay Chou",
          "ALBUM":"叶惠美",
          "ALBUMID":"1293",
          "DURATION":"269",
          "ONLINE":"1",
          "PAY":"16711935",
          "MVFLAG":"1",
          "FORMAT":"wma",
          "MINFO":"level:ff,bitrate:2000,format:flac,size:52.83Mb;level:p,bitrate:320,format:mp3,size:10.29Mb;level:h,bitrate:128,format:mp3,size:4.12Mb;level:s,bitrate:48,format:aac,size:1.57Mb",
          "N_MINFO":"level:dtsx,bitrate:25000,format:mmp4,size:7.36Mb;level:ff,bitrate:2000,format:flac,size:52.83Mb",
          "web_albumpic_short":"120/s3s94/93/211513640.jpg",
          "web_artistpic_short":"120/s4s56/58/291211030.jpg",
          "originalsongtype":"1",
          "content_type":"0",
          "ad_type":"",
          "ad_subtype":"",
          "tme_musician_adtype":"0",
          "payInfo":{
            "cannotDownload":"0",
            "cannotOnlinePlay":"0",
            "feeType":{"album":"0","bookvip":"0","song":"1","vip":"1"},
            "listen_fragment":"1",
            "paytype":3,
            "refrain_start":"84346",
            "refrain_end":"142843"
          },
          "mvpayinfo":{"play":"1","vid":"8132306"},
          "TAG":"http://untrusted.example/song.wma"
        },
        {
          "MUSICRID":"MUSIC_3195905",
          "SONGNAME":"红尘客栈",
          "ARTIST":"周杰伦",
          "ARTISTID":"336",
          "ALBUM":"十二新作",
          "ALBUMID":"264333",
          "DURATION":"274",
          "ONLINE":"0"
        }
      ]
    }"#;

    #[test]
    fn search_maps_stable_identity_metadata_rights_and_known_quality_tiers() {
        let page =
            parse_search_response(SEARCH_RESPONSE.as_bytes(), 0, 2).expect("parse Kuwo search");
        assert_eq!(page.total, 3_600);
        assert_eq!(page.tracks.len(), 2);
        let first = &page.tracks[0];
        assert_eq!(first.resource_ref.to_string(), "kuwo:228908");
        assert_eq!(first.name, "晴天");
        assert_eq!(first.aliases, ["《左手上篮》动画片尾曲"]);
        assert_eq!(
            first.artists[0]
                .resource_ref
                .as_ref()
                .map(ToString::to_string),
            Some("kuwo:336".to_owned())
        );
        assert_eq!(
            first.album.as_ref().map(|album| album.name.as_str()),
            Some("叶惠美")
        );
        assert_eq!(first.duration_ms, Some(269_000));
        assert_eq!(
            first.mv_ref.as_ref().map(ToString::to_string),
            Some("kuwo:8132306".to_owned())
        );
        assert_eq!(
            first.available_qualities,
            [
                Quality::Low,
                Quality::Standard,
                Quality::High,
                Quality::Lossless,
                Quality::Surround
            ]
        );
        assert_eq!(first.playable, None);
        assert_eq!(page.tracks[1].playable, Some(false));
        assert!(first.extensions.contains_key("pay_info"));
        assert!(first.extensions.contains_key("media_specs"));
        assert!(
            !serde_json::to_string(first)
                .expect("serialize track")
                .contains("untrusted.example")
        );
    }

    #[test]
    fn search_rejects_identity_and_pagination_drift() {
        let wrong_page = SEARCH_RESPONSE.replace("\"PN\":\"0\"", "\"PN\":\"1\"");
        assert!(parse_search_response(wrong_page.as_bytes(), 0, 2).is_err());

        let wrong_size = SEARCH_RESPONSE.replace("\"RN\":\"2\"", "\"RN\":\"20\"");
        assert!(parse_search_response(wrong_size.as_bytes(), 0, 2).is_err());

        let invalid_id = SEARCH_RESPONSE.replace("MUSIC_228908", "228908");
        assert!(parse_search_response(invalid_id.as_bytes(), 0, 2).is_err());
    }

    #[test]
    fn media_specs_merge_without_promoting_unknown_platform_levels() {
        let specs = parse_media_specs(
            "level:p,bitrate:320,format:mp3,size:10Mb;level:zply,bitrate:20900,format:mflac,size:160Mb;level:p,bitrate:320,format:mp3,size:10Mb",
            "current",
        );
        assert_eq!(specs.len(), 3);
        assert_eq!(map_qualities(&specs), [Quality::High]);
    }

    #[test]
    fn image_paths_are_confined_to_official_fixed_prefixes() {
        assert_eq!(
            normalize_image_path("120/s3s94/93/211513640.jpg", ALBUM_IMAGE_PREFIX),
            Some("https://img2.kuwo.cn/star/albumcover/120/s3s94/93/211513640.jpg".to_owned())
        );
        for invalid in [
            "../secret",
            "/absolute.jpg",
            "https://example.com/a.jpg",
            "a.jpg?token=x",
            "a\\b.jpg",
        ] {
            assert_eq!(normalize_image_path(invalid, ALBUM_IMAGE_PREFIX), None);
        }
    }

    #[test]
    fn request_ids_are_uuid_v4_shaped() {
        let id = new_request_id();
        assert_eq!(id.len(), 36);
        assert_eq!(&id[14..15], "4");
        assert!(matches!(&id[19..20], "8" | "9" | "a" | "b"));
    }

    #[tokio::test]
    #[ignore = "requires live Kuwo network access"]
    async fn live_public_track_search_returns_stable_results() {
        let client = KuwoClient::test_client();
        let page = client
            .search_tracks_page("周杰伦", 0, 3)
            .await
            .expect("live Kuwo search");
        assert_eq!(page.tracks.len(), 3);
        assert!(page.total >= 3);
        assert!(page.tracks.iter().all(|track| {
            track.platform == Platform::Kuwo
                && track.resource_ref.platform() == Platform::Kuwo
                && canonical_positive_decimal(track.resource_ref.id()).is_some()
                && !track.name.trim().is_empty()
        }));
    }
}
