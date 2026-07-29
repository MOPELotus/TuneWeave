use std::{
    collections::BTreeMap,
    fmt,
    fmt::Write as _,
    io::Read,
    sync::Arc,
    time::{Duration, Instant},
};

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use encoding_rs::GBK;
use flate2::read::ZlibDecoder;
use reqwest::{
    Client, Proxy, StatusCode,
    header::{ACCEPT, CONTENT_LENGTH, COOKIE, REFERER, SET_COOKIE},
    redirect::Policy,
};
use serde::{Deserialize, Serialize};
use serde_json::{Number, json};
use tokio::sync::Mutex;
use tuneweave_core::{
    AlbumSummary, ArtistSummary, ErrorCode, Extensions, Lyrics, Platform, Quality, ResourceRef,
    Result, Track, TuneWeaveError,
};
use url::Url;

const HOME_ENDPOINT: &str = "https://www.kuwo.cn/";
const SEARCH_ENDPOINT: &str = "https://www.kuwo.cn/search/searchMusicBykeyWord";
const TRACK_DETAIL_ENDPOINT: &str = "https://www.kuwo.cn/api/www/music/musicInfo";
const WORD_LYRIC_ENDPOINT: &str = "https://newlyric.kuwo.cn/newlyric.lrc";
const MOBILE_LYRIC_ENDPOINT: &str = "https://m.kuwo.cn/newh5/singles/songinfoandlrc";
const SEARCH_REFERER: &str = "https://www.kuwo.cn/search/list";
const WEB_REFERER: &str = "https://www.kuwo.cn/";
const WEB_SESSION_COOKIE: &str = "Hm_Iuvt_cdb524f42f23cer9b268564v7y735ewrq2324";
const ALBUM_IMAGE_PREFIX: &str = "https://img2.kuwo.cn/star/albumcover/";
const ARTIST_IMAGE_PREFIX: &str = "https://img1.kuwo.cn/star/starheads/";
const MAX_API_RESPONSE_BYTES: u64 = 8 * 1024 * 1024;
const MAX_LYRIC_RESPONSE_BYTES: u64 = 4 * 1024 * 1024;
const MAX_LYRIC_DECOMPRESSED_BYTES: u64 = 8 * 1024 * 1024;
const USER_AGENT: &str = "TuneWeave/0.1 (Kuwo public music provider)";
const WEB_SESSION_TTL: Duration = Duration::from_secs(30 * 60);
const SECRET_MULTIPLIER: u64 = 9_253;
const SECRET_INCREMENT: u64 = 23;
const SECRET_MODULUS: u64 = 2_147_483_647;
const EIGHT_DIGIT_FOLDED_SEED: u64 = 59_910_100;
const LYRIC_XOR_KEY: &[u8] = b"yeelion";

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
    web_session: Arc<Mutex<Option<KuwoWebSession>>>,
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

#[derive(Clone)]
struct KuwoWebSession {
    cookie_value: String,
    refresh_after: Instant,
}

enum KuwoSignedResponse {
    Body(Vec<u8>),
    SessionRejected,
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

#[derive(Serialize)]
struct KuwoTrackDetailQuery<'a> {
    mid: &'a str,
    #[serde(rename = "httpsStatus")]
    https_status: u8,
    #[serde(rename = "reqId")]
    request_id: String,
    plat: &'static str,
    from: &'static str,
}

#[derive(Serialize)]
struct KuwoMobileLyricQuery<'a> {
    #[serde(rename = "musicId")]
    music_id: &'a str,
    #[serde(rename = "httpsStatus")]
    https_status: u8,
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

#[derive(Default, Deserialize)]
#[serde(default)]
struct KuwoTrackDetailEnvelope {
    code: FlexibleText,
    msg: String,
    data: Option<KuwoTrackDetail>,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct KuwoTrackDetail {
    musicrid: String,
    rid: FlexibleText,
    name: String,
    track: FlexibleText,
    artist: String,
    artistid: FlexibleText,
    album: String,
    albumid: FlexibleText,
    duration: FlexibleText,
    #[serde(rename = "songTimeMinutes")]
    song_time_minutes: String,
    #[serde(rename = "releaseDate")]
    release_date: String,
    pic: String,
    pic120: String,
    albumpic: String,
    #[serde(rename = "hasLossless")]
    has_lossless: FlexibleBoolean,
    hasmv: FlexibleText,
    #[serde(rename = "mvPlayCnt")]
    mv_play_count: FlexibleText,
    pay: FlexibleText,
    #[serde(rename = "isListenFee")]
    listen_fee: FlexibleBoolean,
    online: FlexibleText,
    score100: FlexibleText,
    originalsongtype: FlexibleText,
    content_type: FlexibleText,
    ad_type: String,
    ad_subtype: FlexibleText,
    tme_musician_adtype: FlexibleText,
    isstar: FlexibleText,
    barrage: FlexibleText,
    #[serde(rename = "payInfo")]
    pay_info: Option<KuwoPayInfo>,
    mvpayinfo: Option<KuwoMvPayInfo>,
    albuminfo: String,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(untagged)]
enum FlexibleBoolean {
    Boolean(bool),
    String(String),
    Number(Number),
    #[default]
    Null,
}

impl FlexibleBoolean {
    fn get(&self) -> Option<bool> {
        match self {
            Self::Boolean(value) => Some(*value),
            Self::String(value) => match value.trim().to_ascii_lowercase().as_str() {
                "1" | "true" => Some(true),
                "0" | "false" => Some(false),
                _ => None,
            },
            Self::Number(value) => match value.as_i64() {
                Some(1) => Some(true),
                Some(0) => Some(false),
                _ => None,
            },
            Self::Null => None,
        }
    }
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct KuwoSignedRejection {
    success: Option<bool>,
    message: String,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct KuwoMobileLyricEnvelope {
    status: FlexibleText,
    msg: String,
    data: Option<KuwoMobileLyricData>,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct KuwoMobileLyricData {
    lrclist: Vec<KuwoMobileLyricLine>,
    songinfo: Option<KuwoMobileLyricSong>,
}

#[derive(Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct KuwoMobileLyricLine {
    line_lyric: String,
    time: FlexibleText,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct KuwoMobileLyricSong {
    id: FlexibleText,
    #[serde(rename = "musicrId")]
    music_rid: FlexibleText,
}

struct KuwoWordLyrics {
    text: String,
    byte_length: usize,
    marker_count: usize,
}

struct KuwoPlainLyrics {
    text: String,
    byte_length: usize,
    line_count: usize,
}

#[derive(Serialize)]
struct KuwoLyricSourceDiagnostics {
    available: bool,
    byte_length: Option<usize>,
    line_count: Option<usize>,
    word_marker_count: Option<usize>,
    error_code: Option<ErrorCode>,
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

    fn as_i64(&self) -> Option<i64> {
        match self {
            Self::String(value) => value.parse().ok(),
            Self::Number(value) => value.as_i64(),
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
        Ok(Self {
            http,
            web_session: Arc::new(Mutex::new(None)),
        })
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

    pub(crate) async fn track_detail(&self, music_id: &str) -> Result<Track> {
        for force_refresh in [false, true] {
            let response = self
                .signed_get_track_detail(music_id, force_refresh)
                .await?;
            let bytes = match response {
                KuwoSignedResponse::Body(bytes) => bytes,
                KuwoSignedResponse::SessionRejected if !force_refresh => continue,
                KuwoSignedResponse::SessionRejected => {
                    return Err(kuwo_upstream_error(
                        "Kuwo rejected a freshly established web session",
                    ));
                }
            };
            match parse_track_detail_response(&bytes, music_id) {
                Ok(track) => return Ok(track),
                Err(_) if !force_refresh && is_signed_session_rejection(&bytes) => continue,
                Err(error) => return Err(error),
            }
        }
        Err(kuwo_upstream_error(
            "Kuwo track detail exhausted its bounded session refresh",
        ))
    }

    pub(crate) async fn lyrics(&self, music_id: &str) -> Result<Lyrics> {
        let (word_result, plain_result) = tokio::join!(
            self.download_word_lyrics(music_id),
            self.download_mobile_lyrics(music_id)
        );
        let (word, word_diagnostics) = match word_result {
            Ok(word) => {
                let diagnostics = KuwoLyricSourceDiagnostics {
                    available: true,
                    byte_length: Some(word.byte_length),
                    line_count: Some(word.text.lines().count()),
                    word_marker_count: Some(word.marker_count),
                    error_code: None,
                };
                (Some(word), diagnostics)
            }
            Err(error) => (
                None,
                KuwoLyricSourceDiagnostics {
                    available: false,
                    byte_length: None,
                    line_count: None,
                    word_marker_count: None,
                    error_code: Some(error.code),
                },
            ),
        };
        let (mobile_plain, mobile_diagnostics) = match plain_result {
            Ok(plain) => {
                let diagnostics = KuwoLyricSourceDiagnostics {
                    available: true,
                    byte_length: Some(plain.byte_length),
                    line_count: Some(plain.line_count),
                    word_marker_count: None,
                    error_code: None,
                };
                (Some(plain), diagnostics)
            }
            Err(error) => (
                None,
                KuwoLyricSourceDiagnostics {
                    available: false,
                    byte_length: None,
                    line_count: None,
                    word_marker_count: None,
                    error_code: Some(error.code),
                },
            ),
        };
        if word.is_none() && mobile_plain.is_none() {
            return Err(kuwo_upstream_error(
                "Kuwo lyrics were unavailable from every public source",
            )
            .with_details(json!({
                "lrcx": word_diagnostics,
                "mobile_lrc": mobile_diagnostics,
            })));
        }

        let derived_plain = mobile_plain.is_none() && word.is_some();
        let plain = mobile_plain.map(|plain| plain.text).or_else(|| {
            word.as_ref()
                .and_then(|word| derive_plain_from_lrcx(&word.text).ok())
        });
        let word_synced = word.map(|word| word.text);
        let mut extensions = Extensions::new();
        extensions.insert(
            "sources".to_owned(),
            json!({
                "lrcx": word_diagnostics,
                "mobile_lrc": mobile_diagnostics,
            }),
        );
        extensions.insert("plain_derived_from_lrcx".to_owned(), json!(derived_plain));
        Ok(Lyrics {
            track_ref: ResourceRef::new(Platform::Kuwo, music_id.to_owned())
                .map_err(|_| kuwo_upstream_error("Kuwo lyrics received an invalid music ID"))?,
            plain,
            translated: None,
            romanized: None,
            word_synced,
            singing_annotations: None,
            singing_annotations_timestamp: None,
            format: if word_diagnostics.available {
                "lrcx".to_owned()
            } else {
                "lrc".to_owned()
            },
            contributors: Vec::new(),
            extensions,
        })
    }

    async fn download_word_lyrics(&self, music_id: &str) -> Result<KuwoWordLyrics> {
        let url = build_word_lyric_url(music_id)?;
        let response = self
            .http
            .get(url)
            .header(ACCEPT, "application/octet-stream, */*")
            .header(REFERER, WEB_REFERER)
            .send()
            .await
            .map_err(kuwo_network_error)?;
        let bytes = read_bounded_response_with_limit(
            response,
            "Kuwo word-synced lyrics",
            MAX_LYRIC_RESPONSE_BYTES,
        )
        .await?;
        let byte_length = bytes.len();
        let text = decode_word_lyrics(&bytes)?;
        let marker_count = count_lrcx_word_markers(&text);
        if marker_count == 0 {
            return Err(kuwo_upstream_error(
                "Kuwo LRCX response omitted word timing markers",
            ));
        }
        Ok(KuwoWordLyrics {
            text,
            byte_length,
            marker_count,
        })
    }

    async fn download_mobile_lyrics(&self, music_id: &str) -> Result<KuwoPlainLyrics> {
        let response = self
            .http
            .get(MOBILE_LYRIC_ENDPOINT)
            .header(ACCEPT, "application/json, text/plain")
            .header(REFERER, WEB_REFERER)
            .query(&KuwoMobileLyricQuery {
                music_id,
                https_status: 1,
            })
            .send()
            .await
            .map_err(kuwo_network_error)?;
        let bytes = read_bounded_response_with_limit(
            response,
            "Kuwo mobile lyrics",
            MAX_LYRIC_RESPONSE_BYTES,
        )
        .await?;
        let byte_length = bytes.len();
        let (text, line_count) = parse_mobile_lyrics(&bytes, music_id)?;
        Ok(KuwoPlainLyrics {
            text,
            byte_length,
            line_count,
        })
    }

    async fn signed_get_track_detail(
        &self,
        music_id: &str,
        force_refresh: bool,
    ) -> Result<KuwoSignedResponse> {
        let session = self.web_session(force_refresh).await?;
        let secret = new_web_secret(&session.cookie_value)?;
        let response = self
            .http
            .get(TRACK_DETAIL_ENDPOINT)
            .header(ACCEPT, "application/json")
            .header(REFERER, WEB_REFERER)
            .header(
                COOKIE,
                format!("{WEB_SESSION_COOKIE}={}", session.cookie_value),
            )
            .header("Secret", secret)
            .query(&KuwoTrackDetailQuery {
                mid: music_id,
                https_status: 1,
                request_id: new_request_id(),
                plat: "web_www",
                from: "",
            })
            .send()
            .await
            .map_err(kuwo_network_error)?;
        if matches!(
            response.status(),
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN
        ) {
            return Ok(KuwoSignedResponse::SessionRejected);
        }
        let bytes = read_bounded_response(response, "Kuwo track detail").await?;
        Ok(KuwoSignedResponse::Body(bytes))
    }

    async fn web_session(&self, force_refresh: bool) -> Result<KuwoWebSession> {
        let mut current = self.web_session.lock().await;
        if !force_refresh
            && let Some(session) = current.as_ref()
            && session.refresh_after > Instant::now()
        {
            return Ok(session.clone());
        }
        let response = self
            .http
            .get(HOME_ENDPOINT)
            .header(ACCEPT, "text/html,application/xhtml+xml")
            .send()
            .await
            .map_err(kuwo_network_error)?;
        let headers = response.headers().clone();
        let _body = read_bounded_response(response, "Kuwo web session").await?;
        let cookie_value = extract_web_session_cookie(&headers)?;
        let session = KuwoWebSession {
            cookie_value,
            refresh_after: Instant::now() + WEB_SESSION_TTL,
        };
        *current = Some(session.clone());
        Ok(session)
    }
}

fn build_word_lyric_url(music_id: &str) -> Result<Url> {
    let plaintext =
        format!("user=12345,web,web,web&requester=localhost&req=1&rid=MUSIC_{music_id}&lrcx=1");
    let encrypted = plaintext
        .bytes()
        .enumerate()
        .map(|(index, byte)| byte ^ LYRIC_XOR_KEY[index % LYRIC_XOR_KEY.len()])
        .collect::<Vec<_>>();
    let opaque_query = BASE64_STANDARD.encode(encrypted);
    let encoded_query =
        url::form_urlencoded::byte_serialize(opaque_query.as_bytes()).collect::<String>();
    let mut url = Url::parse(WORD_LYRIC_ENDPOINT)
        .map_err(|_| kuwo_upstream_error("Kuwo LRCX endpoint configuration is invalid"))?;
    url.set_query(Some(&encoded_query));
    Ok(url)
}

fn decode_word_lyrics(bytes: &[u8]) -> Result<String> {
    if !bytes.starts_with(b"tp=content") {
        return Err(kuwo_upstream_error(
            "Kuwo LRCX response returned an invalid content envelope",
        ));
    }
    let separator = bytes
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or_else(|| kuwo_upstream_error("Kuwo LRCX response omitted its content separator"))?;
    let compressed = bytes
        .get(separator + 4..)
        .filter(|compressed| !compressed.is_empty())
        .ok_or_else(|| kuwo_upstream_error("Kuwo LRCX response omitted compressed content"))?;
    let mut inflated = Vec::new();
    ZlibDecoder::new(compressed)
        .take(MAX_LYRIC_DECOMPRESSED_BYTES + 1)
        .read_to_end(&mut inflated)
        .map_err(|_| kuwo_upstream_error("Kuwo LRCX response could not be decompressed"))?;
    if u64::try_from(inflated.len()).unwrap_or(u64::MAX) > MAX_LYRIC_DECOMPRESSED_BYTES {
        return Err(kuwo_upstream_error(
            "Kuwo LRCX decompressed content exceeded the size limit",
        ));
    }
    let encoded = std::str::from_utf8(&inflated)
        .map(str::trim)
        .map_err(|_| kuwo_upstream_error("Kuwo LRCX content was not valid base64 text"))?;
    let mut decrypted = BASE64_STANDARD
        .decode(encoded)
        .map_err(|_| kuwo_upstream_error("Kuwo LRCX content contained invalid base64"))?;
    for (index, byte) in decrypted.iter_mut().enumerate() {
        *byte ^= LYRIC_XOR_KEY[index % LYRIC_XOR_KEY.len()];
    }
    let (decoded, had_errors) = GBK.decode_without_bom_handling(&decrypted);
    if had_errors {
        return Err(kuwo_upstream_error(
            "Kuwo LRCX content was not valid GB18030 text",
        ));
    }
    validate_lyric_text(&decoded)?;
    Ok(decoded.into_owned())
}

fn parse_mobile_lyrics(bytes: &[u8], requested_music_id: &str) -> Result<(String, usize)> {
    let envelope: KuwoMobileLyricEnvelope = serde_json::from_slice(bytes)
        .map_err(|_| kuwo_upstream_error("Kuwo mobile lyrics returned malformed JSON"))?;
    if envelope.status.as_i64() != Some(200) {
        return Err(
            kuwo_upstream_error("Kuwo mobile lyrics rejected the request").with_details(json!({
                "platform_code": envelope.status.as_text().map(|value| bounded_text(&value, 64)),
                "platform_message": bounded_text(&envelope.msg, 256),
            })),
        );
    }
    let data = envelope
        .data
        .ok_or_else(|| kuwo_upstream_error("Kuwo mobile lyrics omitted data"))?;
    let song = data
        .songinfo
        .ok_or_else(|| kuwo_upstream_error("Kuwo mobile lyrics omitted song identity"))?;
    let song_id = song
        .id
        .as_text()
        .filter(|id| canonical_positive_decimal(id).is_some())
        .ok_or_else(|| kuwo_upstream_error("Kuwo mobile lyrics omitted a valid song ID"))?;
    let music_rid = song
        .music_rid
        .as_text()
        .filter(|id| canonical_positive_decimal(id).is_some())
        .ok_or_else(|| kuwo_upstream_error("Kuwo mobile lyrics omitted a valid music ID"))?;
    if song_id != requested_music_id || music_rid != requested_music_id {
        return Err(kuwo_upstream_error(
            "Kuwo mobile lyrics returned a mismatched music ID",
        ));
    }
    if data.lrclist.is_empty() || data.lrclist.len() > 10_000 {
        return Err(kuwo_upstream_error(
            "Kuwo mobile lyrics returned an invalid line count",
        ));
    }

    let line_count = data.lrclist.len();
    let mut output = String::new();
    let mut previous_time = 0_u64;
    for (index, line) in data.lrclist.into_iter().enumerate() {
        let time = line
            .time
            .as_text()
            .and_then(|value| value.parse::<f64>().ok())
            .filter(|value| value.is_finite() && *value >= 0.0 && *value <= 86_400.0)
            .map(|value| (value * 1_000.0).round() as u64)
            .ok_or_else(|| kuwo_upstream_error("Kuwo mobile lyrics contained an invalid time"))?;
        if index > 0 && time < previous_time {
            return Err(kuwo_upstream_error(
                "Kuwo mobile lyrics were not ordered by time",
            ));
        }
        previous_time = time;
        if line.line_lyric.len() > 4_096
            || line
                .line_lyric
                .chars()
                .any(|character| character.is_control() && !matches!(character, '\t'))
        {
            return Err(kuwo_upstream_error(
                "Kuwo mobile lyrics contained invalid line text",
            ));
        }
        if index > 0 {
            output.push('\n');
        }
        let minutes = time / 60_000;
        let seconds = time % 60_000 / 1_000;
        let millis = time % 1_000;
        write!(
            &mut output,
            "[{minutes:02}:{seconds:02}.{millis:03}]{}",
            line.line_lyric
        )
        .map_err(|_| {
            TuneWeaveError::new(
                ErrorCode::InternalError,
                "failed to format Kuwo mobile lyrics",
            )
            .with_platform(Platform::Kuwo)
        })?;
    }
    validate_lyric_text(&output)?;
    Ok((output, line_count))
}

fn derive_plain_from_lrcx(word_synced: &str) -> Result<String> {
    let mut output = String::with_capacity(word_synced.len());
    for line in word_synced.lines() {
        if line.trim_start().starts_with("[kuwo:") {
            continue;
        }
        if !output.is_empty() {
            output.push('\n');
        }
        let mut rest = line;
        while let Some(open) = rest.find('<') {
            output.push_str(&rest[..open]);
            let after_open = &rest[open + 1..];
            let Some(close) = after_open.find('>') else {
                output.push_str(&rest[open..]);
                rest = "";
                break;
            };
            let marker = &after_open[..close];
            if is_lrcx_word_marker(marker) {
                rest = &after_open[close + 1..];
            } else {
                output.push('<');
                output.push_str(marker);
                output.push('>');
                rest = &after_open[close + 1..];
            }
        }
        output.push_str(rest);
    }
    validate_lyric_text(&output)?;
    Ok(output)
}

fn count_lrcx_word_markers(value: &str) -> usize {
    let mut count = 0_usize;
    let mut rest = value;
    while let Some(open) = rest.find('<') {
        let after_open = &rest[open + 1..];
        let Some(close) = after_open.find('>') else {
            break;
        };
        if is_lrcx_word_marker(&after_open[..close]) {
            count = count.saturating_add(1);
        }
        rest = &after_open[close + 1..];
    }
    count
}

fn is_lrcx_word_marker(value: &str) -> bool {
    let parts = value.split(',').collect::<Vec<_>>();
    (2..=3).contains(&parts.len())
        && parts.iter().all(|part| {
            let digits = part.strip_prefix('-').unwrap_or(part);
            !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit())
        })
}

fn validate_lyric_text(value: &str) -> Result<()> {
    if value.trim().is_empty()
        || value
            .chars()
            .any(|character| character.is_control() && !matches!(character, '\n' | '\r' | '\t'))
    {
        return Err(kuwo_upstream_error(
            "Kuwo lyric response contained invalid text",
        ));
    }
    Ok(())
}

fn parse_track_detail_response(bytes: &[u8], requested_music_id: &str) -> Result<Track> {
    let envelope: KuwoTrackDetailEnvelope = serde_json::from_slice(bytes)
        .map_err(|_| kuwo_upstream_error("Kuwo track detail returned malformed JSON"))?;
    if envelope.code.as_i64() != Some(200) {
        if envelope.code.as_i64() == Some(-1) && envelope.data.is_none() {
            return Err(TuneWeaveError::new(
                ErrorCode::ResourceNotFound,
                "Kuwo track was not found",
            )
            .with_platform(Platform::Kuwo));
        }
        return Err(
            kuwo_upstream_error("Kuwo track detail rejected the request").with_details(json!({
                "platform_code": envelope.code.as_text().map(|value| bounded_text(&value, 64)),
                "platform_message": bounded_text(&envelope.msg, 256),
            })),
        );
    }
    let detail = envelope
        .data
        .ok_or_else(|| kuwo_upstream_error("Kuwo track detail omitted data"))?;
    let musicrid = detail
        .musicrid
        .strip_prefix("MUSIC_")
        .and_then(canonical_positive_decimal)
        .ok_or_else(|| kuwo_upstream_error("Kuwo track detail omitted a stable music ID"))?;
    let rid_text = detail.rid.as_text();
    let rid = rid_text
        .as_deref()
        .and_then(canonical_positive_decimal)
        .ok_or_else(|| kuwo_upstream_error("Kuwo track detail omitted its numeric music ID"))?;
    if musicrid != requested_music_id || rid != requested_music_id {
        return Err(kuwo_upstream_error(
            "Kuwo track detail returned a mismatched music ID",
        ));
    }
    let name = nonempty(&detail.name)
        .ok_or_else(|| kuwo_upstream_error("Kuwo track detail omitted a track name"))?;
    let resource_ref = ResourceRef::new(Platform::Kuwo, requested_music_id.to_owned())
        .map_err(|_| kuwo_upstream_error("Kuwo track detail returned an invalid identity"))?;
    let mut track = Track::new(resource_ref, bounded_text(name, 512));
    track.artists = map_artists(&detail.artist, &detail.artistid);
    let cover_url = [
        detail.pic120.as_str(),
        detail.pic.as_str(),
        detail.albumpic.as_str(),
    ]
    .into_iter()
    .find_map(normalize_official_image_url);
    track.album = map_album(&detail.album, &detail.albumid, cover_url);
    track.duration_ms = detail
        .duration
        .as_u64()
        .and_then(|seconds| seconds.checked_mul(1_000));
    track.mv_ref = detail
        .hasmv
        .as_u64()
        .filter(|value| *value > 0)
        .and(detail.mvpayinfo.as_ref())
        .and_then(|info| info.vid.as_text())
        .as_deref()
        .and_then(canonical_positive_decimal)
        .and_then(|id| ResourceRef::new(Platform::Kuwo, id.to_owned()).ok());
    if detail.online.as_text().as_deref() == Some("0") {
        track.playable = Some(false);
    }
    if detail.has_lossless.get() == Some(true) {
        track.available_qualities.push(Quality::Lossless);
    }

    track
        .extensions
        .insert("backend".to_owned(), json!("current_web_music_info"));
    insert_flexible(&mut track.extensions, "track_number", &detail.track);
    insert_optional_text(
        &mut track.extensions,
        "song_time_minutes",
        &detail.song_time_minutes,
    );
    insert_optional_text(&mut track.extensions, "release_date", &detail.release_date);
    insert_flexible(&mut track.extensions, "online", &detail.online);
    insert_flexible(&mut track.extensions, "pay", &detail.pay);
    insert_flexible(&mut track.extensions, "score", &detail.score100);
    insert_flexible(
        &mut track.extensions,
        "original_song_type",
        &detail.originalsongtype,
    );
    insert_flexible(&mut track.extensions, "content_type", &detail.content_type);
    insert_optional_text(&mut track.extensions, "ad_type", &detail.ad_type);
    insert_flexible(&mut track.extensions, "ad_subtype", &detail.ad_subtype);
    insert_flexible(
        &mut track.extensions,
        "musician_ad_type",
        &detail.tme_musician_adtype,
    );
    insert_flexible(&mut track.extensions, "is_starred", &detail.isstar);
    insert_flexible(&mut track.extensions, "barrage", &detail.barrage);
    insert_flexible(
        &mut track.extensions,
        "mv_play_count",
        &detail.mv_play_count,
    );
    if let Some(value) = detail.has_lossless.get() {
        track
            .extensions
            .insert("has_lossless".to_owned(), json!(value));
    }
    if let Some(value) = detail.listen_fee.get() {
        track
            .extensions
            .insert("listen_fee".to_owned(), json!(value));
    }
    if let Some(pay_info) = detail.pay_info {
        track
            .extensions
            .insert("pay_info".to_owned(), json!(pay_info));
    }
    if let Some(mv_pay_info) = detail.mvpayinfo {
        track
            .extensions
            .insert("mv_pay_info".to_owned(), json!(mv_pay_info));
    }
    if let Some(description) = nonempty(&detail.albuminfo) {
        track.extensions.insert(
            "album_description".to_owned(),
            json!(bounded_text(description, 4_000)),
        );
    }
    Ok(track)
}

fn is_signed_session_rejection(bytes: &[u8]) -> bool {
    serde_json::from_slice::<KuwoSignedRejection>(bytes).is_ok_and(|response| {
        response.success == Some(false) && response.message == "The request is illegal!"
    })
}

fn extract_web_session_cookie(headers: &reqwest::header::HeaderMap) -> Result<String> {
    for header in headers.get_all(SET_COOKIE) {
        let Ok(header) = header.to_str() else {
            continue;
        };
        let Some(pair) = header.split(';').next() else {
            continue;
        };
        let Some((name, value)) = pair.split_once('=') else {
            continue;
        };
        if name.trim() == WEB_SESSION_COOKIE
            && (16..=128).contains(&value.len())
            && value.bytes().all(|byte| byte.is_ascii_alphanumeric())
        {
            return Ok(value.to_owned());
        }
    }
    Err(kuwo_upstream_error(
        "Kuwo web session omitted its current tracking cookie",
    ))
}

fn new_web_secret(cookie_value: &str) -> Result<String> {
    let nonce = rand::random_range(10_000_000_u64..=99_999_999);
    web_secret_for_nonce(cookie_value, nonce)
}

fn web_secret_for_nonce(cookie_value: &str, nonce: u64) -> Result<String> {
    if !(16..=128).contains(&cookie_value.len())
        || !cookie_value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric())
        || !(10_000_000..=99_999_999).contains(&nonce)
    {
        return Err(kuwo_upstream_error(
            "Kuwo web session cannot produce a valid request signature",
        ));
    }
    let mut state =
        (SECRET_MULTIPLIER * EIGHT_DIGIT_FOLDED_SEED + SECRET_INCREMENT) % SECRET_MODULUS;
    let mut secret = String::with_capacity(cookie_value.len() * 2 + 8);
    for byte in cookie_value.bytes() {
        let mask = state * 255 / SECRET_MODULUS;
        let encoded = u64::from(byte) ^ mask;
        write!(&mut secret, "{encoded:02x}").map_err(|_| {
            TuneWeaveError::new(
                ErrorCode::InternalError,
                "failed to encode Kuwo request signature",
            )
            .with_platform(Platform::Kuwo)
        })?;
        state = (SECRET_MULTIPLIER * state + SECRET_INCREMENT) % SECRET_MODULUS;
    }
    write!(&mut secret, "{nonce:08x}").map_err(|_| {
        TuneWeaveError::new(
            ErrorCode::InternalError,
            "failed to encode Kuwo request nonce",
        )
        .with_platform(Platform::Kuwo)
    })?;
    Ok(secret)
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

fn normalize_official_image_url(value: &str) -> Option<String> {
    let url = Url::parse(value.trim()).ok()?;
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || !matches!(
            url.host_str(),
            Some("img1.kuwo.cn" | "img2.kuwo.cn" | "img3.kuwo.cn")
        )
        || !["/star/albumcover/", "/star/starheads/", "/wmvpic/"]
            .iter()
            .any(|prefix| url.path().starts_with(prefix))
    {
        return None;
    }
    Some(url.into())
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
    read_bounded_response_with_limit(response, operation, MAX_API_RESPONSE_BYTES).await
}

async fn read_bounded_response_with_limit(
    response: reqwest::Response,
    operation: &str,
    limit: u64,
) -> Result<Vec<u8>> {
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
        .is_some_and(|length| length > limit)
    {
        return Err(kuwo_upstream_error(format!(
            "{operation} response exceeded the size limit"
        )));
    }
    let max_size = usize::try_from(limit).unwrap_or(usize::MAX);
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

    const TRACK_DETAIL_RESPONSE: &str = r#"{
      "code":200,
      "msg":"success",
      "data":{
        "musicrid":"MUSIC_228908",
        "rid":228908,
        "name":"晴天",
        "track":3,
        "artist":"周杰伦",
        "artistid":336,
        "album":"叶惠美",
        "albumid":1293,
        "duration":269,
        "songTimeMinutes":"04:29",
        "releaseDate":"2003-07-31",
        "pic":"https://img2.kuwo.cn/star/albumcover/500/s3s94/93/211513640.jpg",
        "pic120":"https://img2.kuwo.cn/star/albumcover/120/s3s94/93/211513640.jpg",
        "albumpic":"https://img2.kuwo.cn/star/albumcover/500/s3s94/93/211513640.jpg",
        "hasLossless":true,
        "hasmv":1,
        "mvPlayCnt":2199392,
        "pay":"16711935",
        "isListenFee":true,
        "online":1,
        "score100":"83",
        "originalsongtype":1,
        "content_type":"0",
        "ad_type":"",
        "ad_subtype":"0",
        "tme_musician_adtype":"0",
        "isstar":0,
        "barrage":"0",
        "payInfo":{
          "play":"1111",
          "nplay":"111111111111",
          "overseas_nplay":"0",
          "local_encrypt":"1",
          "limitfree":0,
          "refrain_start":84346,
          "extendAttr":0,
          "feeType":{"song":"1","vip":"1"},
          "down":"1111",
          "ndown":"111111111111",
          "download":"1111",
          "cannotDownload":0,
          "overseas_ndown":"0",
          "refrain_end":142843,
          "listen_fragment":"1",
          "cannotOnlinePlay":0,
          "paytype":0,
          "paytagindex":{"S":2,"F":3,"H":1}
        },
        "mvpayinfo":{"play":1,"vid":8132306,"down":1},
        "albuminfo":"专辑简介"
      }
    }"#;

    const MOBILE_LYRIC_RESPONSE: &str = r#"{
      "status":200,
      "msg":"成功",
      "data":{
        "lrclist":[
          {"lineLyric":"晴天 - 周杰伦","time":"0.0"},
          {"lineLyric":"词：周杰伦","time":"2.25"},
          {"lineLyric":"故事的小黄花","time":"98.880005"}
        ],
        "songinfo":{"id":"228908","musicrId":"228908"}
      }
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

    #[test]
    fn current_web_signature_matches_the_browser_algorithm_fixture() {
        assert_eq!(
            web_secret_for_nonce("0123456789abcdef0123456789abcdef", 12_345_678)
                .expect("create deterministic Kuwo signature"),
            "1361b99125125e1ce61cc1f328ded44b38fec3e403cfaa3031b91afbe5e200ea00bc614e"
        );
        assert!(web_secret_for_nonce("too-short", 12_345_678).is_err());
        assert!(web_secret_for_nonce("0123456789abcdef0123456789abcdef", 9_999_999).is_err());
    }

    #[test]
    fn web_session_accepts_only_the_fixed_alphanumeric_cookie() {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.append(
            SET_COOKIE,
            reqwest::header::HeaderValue::from_static(
                "unrelated=value; Path=/; Expires=Wed, 01 Jan 2031 00:00:00 GMT",
            ),
        );
        headers.append(
            SET_COOKIE,
            reqwest::header::HeaderValue::from_static(
                "Hm_Iuvt_cdb524f42f23cer9b268564v7y735ewrq2324=0123456789abcdef0123456789abcdef; Path=/; Expires=Wed, 01 Jan 2031 00:00:00 GMT",
            ),
        );
        assert_eq!(
            extract_web_session_cookie(&headers).expect("extract current Kuwo cookie"),
            "0123456789abcdef0123456789abcdef"
        );

        let mut unsafe_headers = reqwest::header::HeaderMap::new();
        unsafe_headers.insert(
            SET_COOKIE,
            reqwest::header::HeaderValue::from_static(
                "Hm_Iuvt_cdb524f42f23cer9b268564v7y735ewrq2324=value%0d%0aInjected; Path=/",
            ),
        );
        assert!(extract_web_session_cookie(&unsafe_headers).is_err());
    }

    #[test]
    fn track_detail_requires_both_platform_id_forms_to_match() {
        let track = parse_track_detail_response(TRACK_DETAIL_RESPONSE.as_bytes(), "228908")
            .expect("parse Kuwo track detail");
        assert_eq!(track.resource_ref.to_string(), "kuwo:228908");
        assert_eq!(track.name, "晴天");
        assert_eq!(
            track.artists[0]
                .resource_ref
                .as_ref()
                .map(ToString::to_string),
            Some("kuwo:336".to_owned())
        );
        assert_eq!(
            track
                .album
                .as_ref()
                .and_then(|album| album.cover_url.as_deref()),
            Some("https://img2.kuwo.cn/star/albumcover/120/s3s94/93/211513640.jpg")
        );
        assert_eq!(track.duration_ms, Some(269_000));
        assert_eq!(
            track.mv_ref.as_ref().map(ToString::to_string),
            Some("kuwo:8132306".to_owned())
        );
        assert_eq!(track.available_qualities, [Quality::Lossless]);
        assert_eq!(track.playable, None);
        assert_eq!(track.extensions.get("listen_fee"), Some(&json!(true)));

        let mismatched_musicrid = TRACK_DETAIL_RESPONSE.replace("MUSIC_228908", "MUSIC_3195905");
        assert!(parse_track_detail_response(mismatched_musicrid.as_bytes(), "228908").is_err());
        let mismatched_rid = TRACK_DETAIL_RESPONSE.replace("\"rid\":228908", "\"rid\":3195905");
        assert!(parse_track_detail_response(mismatched_rid.as_bytes(), "228908").is_err());
    }

    #[test]
    fn track_detail_classifies_not_found_and_signed_session_rejection() {
        let missing = r#"{"code":-1,"msg":"歌曲不存在"}"#;
        let error = parse_track_detail_response(missing.as_bytes(), "999999999999999999")
            .expect_err("missing Kuwo track");
        assert_eq!(error.code, ErrorCode::ResourceNotFound);

        let rejected = br#"{"success":false,"message":"The request is illegal!","now":"redacted"}"#;
        assert!(is_signed_session_rejection(rejected));
        assert!(!is_signed_session_rejection(
            br#"{"success":false,"message":"another error"}"#
        ));
    }

    #[test]
    fn detail_images_accept_only_fixed_https_kuwo_media_hosts() {
        assert_eq!(
            normalize_official_image_url(
                "https://img2.kuwo.cn/star/albumcover/120/s3s94/93/211513640.jpg"
            ),
            Some("https://img2.kuwo.cn/star/albumcover/120/s3s94/93/211513640.jpg".to_owned())
        );
        for invalid in [
            "http://img2.kuwo.cn/star/albumcover/a.jpg",
            "https://user@img2.kuwo.cn/star/albumcover/a.jpg",
            "https://img2.kuwo.cn:444/star/albumcover/a.jpg",
            "https://example.com/star/albumcover/a.jpg",
            "https://img2.kuwo.cn/other/a.jpg",
            "https://img2.kuwo.cn/star/albumcover/a.jpg?token=x",
        ] {
            assert_eq!(normalize_official_image_url(invalid), None);
        }
    }

    #[test]
    fn lrcx_query_matches_the_reference_protocol_fixture() {
        let url = build_word_lyric_url("228908").expect("build Kuwo LRCX URL");
        assert_eq!(url.scheme(), "https");
        assert_eq!(url.host_str(), Some("newlyric.kuwo.cn"));
        assert_eq!(url.path(), "/newlyric.lrc");
        let decoded =
            url::form_urlencoded::parse(url.query().expect("opaque LRCX query").as_bytes())
                .next()
                .expect("decoded LRCX query")
                .0;
        assert_eq!(
            decoded,
            "DBYAHlReXEpRUEAeCgxVEgAORRgLG0MXCRgaCwoRAB5UAwEaBAkEBhwaXxcAHVReSAsMAVEkOj0wJjpeW1dXSV1DABsMFkRU"
        );
    }

    #[test]
    fn lrcx_decoder_preserves_word_timings_and_derives_plain_without_overwrite() {
        use std::io::Write as _;

        let original = "[kuwo:127]\n[ti:Fixture]\n[00:00.000]<0,100>Hello<100,100> world";
        let mut encrypted = original.as_bytes().to_vec();
        for (index, byte) in encrypted.iter_mut().enumerate() {
            *byte ^= LYRIC_XOR_KEY[index % LYRIC_XOR_KEY.len()];
        }
        let encoded = BASE64_STANDARD.encode(encrypted);
        let mut encoder =
            flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        encoder
            .write_all(encoded.as_bytes())
            .expect("compress LRCX fixture");
        let mut envelope = b"tp=content\r\n\r\n".to_vec();
        envelope.extend(encoder.finish().expect("finish LRCX fixture"));

        let decoded = decode_word_lyrics(&envelope).expect("decode LRCX fixture");
        assert_eq!(decoded, original);
        assert_eq!(count_lrcx_word_markers(&decoded), 2);
        assert_eq!(
            derive_plain_from_lrcx(&decoded).expect("derive plain LRC"),
            "[ti:Fixture]\n[00:00.000]Hello world"
        );
    }

    #[test]
    fn mobile_lyrics_round_float_artifacts_and_require_matching_identity() {
        let (plain, count) = parse_mobile_lyrics(MOBILE_LYRIC_RESPONSE.as_bytes(), "228908")
            .expect("parse mobile lyrics");
        assert_eq!(count, 3);
        assert_eq!(
            plain,
            "[00:00.000]晴天 - 周杰伦\n[00:02.250]词：周杰伦\n[01:38.880]故事的小黄花"
        );

        let mismatched =
            MOBILE_LYRIC_RESPONSE.replace("\"musicrId\":\"228908\"", "\"musicrId\":\"3195905\"");
        assert!(parse_mobile_lyrics(mismatched.as_bytes(), "228908").is_err());
        let reversed = MOBILE_LYRIC_RESPONSE.replace("\"time\":\"98.880005\"", "\"time\":\"1\"");
        assert!(parse_mobile_lyrics(reversed.as_bytes(), "228908").is_err());
    }

    #[test]
    fn lrcx_marker_parser_removes_only_platform_word_timing_syntax() {
        assert!(is_lrcx_word_marker("0,100"));
        assert!(is_lrcx_word_marker("-10,20,30"));
        assert!(!is_lrcx_word_marker("0"));
        assert!(!is_lrcx_word_marker("0,word"));
        assert_eq!(
            derive_plain_from_lrcx("[00:00.000]<0,100>A <not,time>B")
                .expect("derive mixed marker line"),
            "[00:00.000]A <not,time>B"
        );
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

    #[tokio::test]
    #[ignore = "requires live Kuwo network access"]
    async fn live_public_track_detail_uses_a_current_anonymous_web_session() {
        let client = KuwoClient::test_client();
        let track = client
            .track_detail("228908")
            .await
            .expect("live Kuwo track detail");
        assert_eq!(track.resource_ref.to_string(), "kuwo:228908");
        assert_eq!(track.id, "228908");
        assert!(!track.name.trim().is_empty());
        assert!(track.duration_ms.is_some_and(|duration| duration > 0));
        assert!(track.artists.iter().any(|artist| !artist.name.is_empty()));
    }

    #[tokio::test]
    #[ignore = "requires live Kuwo network access"]
    async fn live_lyrics_keep_lrcx_ahead_of_plain_lrc() {
        let client = KuwoClient::test_client();
        let lyrics = client.lyrics("228908").await.expect("live Kuwo lyrics");
        assert_eq!(lyrics.track_ref.to_string(), "kuwo:228908");
        assert_eq!(lyrics.format, "lrcx");
        assert!(
            lyrics
                .word_synced
                .as_deref()
                .is_some_and(|text| count_lrcx_word_markers(text) > 0)
        );
        assert!(
            lyrics
                .plain
                .as_deref()
                .is_some_and(|text| { text.contains("[00:00.000]") && !text.contains("<0,") })
        );
    }
}
