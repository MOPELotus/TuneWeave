//! Anonymous video metadata. Video and associated audio identities remain distinct.
use super::dto::{Number, optional_text, required_text, resource};
use super::*;
use tuneweave_core::{
    CreatorSummary, SearchItem, Video, VideoDetail, VideoResolution, VideoResourceKind,
};

pub(crate) const DETAIL_PAGE_SIZE: usize = 20;
const PATH: &str = "/v1/video";

#[derive(Serialize)]
struct DetailBody<'a> {
    appid: u16,
    clientver: u32,
    clienttime: u64,
    mid: &'a str,
    uuid: String,
    dfid: &'a str,
    token: &'static str,
    key: String,
    show_resolution: u8,
    data: Vec<DetailId<'a>>,
}
#[derive(Serialize)]
struct DetailId<'a> {
    video_id: &'a str,
}

impl KugouClient {
    pub(crate) async fn public_video_details(
        &self,
        ids: &[String],
        kind: VideoResourceKind,
    ) -> Result<Vec<VideoDetail>> {
        if ids.is_empty() || ids.len() > DETAIL_PAGE_SIZE || ids.iter().any(|id| !canonical_id(id))
        {
            return Err(kugou_invalid_media_request(
                "Invalid KuGou video detail batch",
            ));
        }
        let device = self.device_identity()?;
        let time = unix_seconds_now();
        let body = DetailBody {
            appid: ANDROID_APP_ID,
            clientver: ANDROID_CLIENT_VERSION,
            clienttime: time,
            mid: &device.mid,
            uuid: md5_hex(format!("{}{}", device.dfid(), device.mid)),
            dfid: device.dfid(),
            token: "",
            key: md5_hex(format!(
                "{ANDROID_APP_ID}{ANDROID_SIGNATURE_SALT}{ANDROID_CLIENT_VERSION}{time}"
            )),
            show_resolution: 1,
            data: ids.iter().map(|id| DetailId { video_id: id }).collect(),
        };
        let bytes = serde_json::to_vec(&body).map_err(|_| malformed())?;
        // This endpoint signs only the exact body. Default Android query parameters are absent.
        let signature = crate::signing::android_signature(&BTreeMap::new(), &bytes);
        let url = format!("{ANDROID_GATEWAY}{PATH}");
        #[cfg(test)]
        let url = self
            .login_test_origin
            .as_ref()
            .map(|o| o.join(PATH).unwrap().to_string())
            .unwrap_or(url);
        let started = Instant::now();
        let mut status = None;
        let result = async {
            let mut response = self
                .http
                .post(url)
                .query(&[("signature", signature)])
                .header("x-router", "kmr.service.kugou.com")
                .header("user-agent", ANDROID_USER_AGENT)
                .header(CONTENT_TYPE, "application/json")
                .header("mid", &device.mid)
                .header("dfid", device.dfid())
                .header("clienttime", time.to_string())
                .body(bytes)
                .send()
                .await
                .map_err(kugou_network_error)?;
            status = Some(response.status());
            if !response.status().is_success() {
                return Err(kugou_http_error(response.status()));
            }
            if response.headers().contains_key("ssa-code") {
                return Err(TuneWeaveError::new(
                    ErrorCode::PermissionDenied,
                    "KuGou video metadata requires additional verification",
                )
                .with_platform(Platform::Kugou));
            }
            const LIMIT: usize = 1_048_576;
            let mime = response
                .headers()
                .get(CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.split(';').next())
                .map(str::trim);
            if !matches!(mime, Some("application/json" | "text/plain"))
                || response.content_length().is_some_and(|v| v > LIMIT as u64)
            {
                return Err(malformed());
            }
            let mut bytes = Vec::new();
            while let Some(chunk) = response.chunk().await.map_err(kugou_network_error)? {
                if bytes.len().saturating_add(chunk.len()) > LIMIT {
                    return Err(malformed());
                }
                bytes.extend_from_slice(&chunk);
            }
            parse_details(&bytes, ids, kind)
        }
        .await;
        self.log_upstream_request(
            "video_metadata",
            "gateway.kugou.com",
            PATH,
            status,
            started,
            0,
            false,
            &result,
        );
        result
    }

    pub(super) async fn enrich_mv_search(&self, items: &mut [SearchItem]) -> Result<()> {
        if items.is_empty() {
            return Ok(());
        }
        let ids = items
            .iter()
            .map(|v| match v {
                SearchItem::Video(v) => Ok(v.id.clone()),
                _ => Err(malformed()),
            })
            .collect::<Result<Vec<_>>>()?;
        let details = self
            .public_video_details(&ids, VideoResourceKind::Mv)
            .await?;
        for (item, detail) in items.iter_mut().zip(details) {
            let SearchItem::Video(video) = item else {
                return Err(malformed());
            };
            if video.id != detail.video.id {
                return Err(malformed());
            }
            if let (Some(search_ms), Some(detail_ms)) =
                (video.duration_ms, detail.video.duration_ms)
            {
                if search_ms / 1000 != detail_ms / 1000 {
                    return Err(malformed());
                }
            }
            for key in ["audio_id", "album_audio_id"] {
                if let (Some(a), Some(b)) =
                    (video.extensions.get(key), detail.video.extensions.get(key))
                {
                    if a != b {
                        return Err(malformed());
                    }
                }
            }
            video.cover_url = detail.video.cover_url.or_else(|| video.cover_url.take());
            video.duration_ms = detail.video.duration_ms.or(video.duration_ms);
            video
                .extensions
                .insert("detail_enriched".into(), json!(true));
        }
        Ok(())
    }
}

pub(crate) fn canonical_id(id: &str) -> bool {
    id.parse::<u64>()
        .is_ok_and(|n| n > 0 && n.to_string() == id)
}
fn video_ref(id: &str, kind: VideoResourceKind) -> Result<ResourceRef> {
    resource(format!(
        "{}:{id}",
        if kind == VideoResourceKind::Mv {
            "mv"
        } else {
            "video"
        }
    ))
}
fn malformed() -> TuneWeaveError {
    kugou_upstream_error("KuGou video returned invalid metadata")
}
fn missing() -> TuneWeaveError {
    TuneWeaveError::new(
        ErrorCode::ResourceNotFound,
        "KuGou video was not found or is unpublished",
    )
    .with_platform(Platform::Kugou)
}

#[derive(Deserialize)]
struct Envelope {
    data: Vec<Entry>,
}
#[derive(Deserialize)]
#[serde(untagged)]
enum Entry {
    Found(Box<VideoDto>),
    Missing(Empty),
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Empty {}
#[derive(Deserialize)]
struct Author {
    author_id: Number,
    author_name: String,
    sizable_avatar: Option<String>,
}
#[derive(Deserialize)]
struct VideoDto {
    video_id: Number,
    video_name: String,
    timelength: Option<Number>,
    audio_timelength: Option<Number>,
    audio_id: Option<Number>,
    album_audio_id: Option<Number>,
    audio_hash: Option<String>,
    authors: Option<Vec<Author>>,
    author_name: Option<String>,
    user_id: Option<Number>,
    user_name: Option<String>,
    user_avatar: Option<String>,
    intro: Option<String>,
    remark: Option<String>,
    other_description: Option<String>,
    hdpic: Option<String>,
    cover: Option<String>,
    publish_date: Option<String>,
    play_times: Option<Number>,
    history_heat: Option<Number>,
    heat: Option<Number>,
    collection_total: Option<Number>,
    download_total: Option<Number>,
    is_publish: Option<Number>,
    deleted: Option<Number>,
    #[serde(rename = "type")]
    type_code: Option<Number>,
    track: Option<Number>,
    is_short: Option<Number>,
    #[serde(flatten)]
    assets: AssetFields,
}
#[derive(Deserialize)]
struct AssetFields {
    #[serde(rename = "ld_hash")]
    ld_hash: Option<String>,
    #[serde(rename = "ld_bitrate")]
    ld_bitrate: Option<Number>,
    #[serde(rename = "ld_filesize")]
    ld_filesize: Option<Number>,
    #[serde(rename = "ld_width")]
    #[serde(default, deserialize_with = "optional_dimension")]
    ld_width: Option<Number>,
    #[serde(rename = "ld_height")]
    #[serde(default, deserialize_with = "optional_dimension")]
    ld_height: Option<Number>,
    #[serde(rename = "sd_hash")]
    sd_hash: Option<String>,
    #[serde(rename = "sd_bitrate")]
    sd_bitrate: Option<Number>,
    #[serde(rename = "sd_filesize")]
    sd_filesize: Option<Number>,
    #[serde(rename = "sd_width")]
    #[serde(default, deserialize_with = "optional_dimension")]
    sd_width: Option<Number>,
    #[serde(rename = "sd_height")]
    #[serde(default, deserialize_with = "optional_dimension")]
    sd_height: Option<Number>,
    #[serde(rename = "qhd_hash")]
    qhd_hash: Option<String>,
    #[serde(rename = "qhd_bitrate")]
    qhd_bitrate: Option<Number>,
    #[serde(rename = "qhd_filesize")]
    qhd_filesize: Option<Number>,
    #[serde(rename = "qhd_width")]
    #[serde(default, deserialize_with = "optional_dimension")]
    qhd_width: Option<Number>,
    #[serde(rename = "qhd_height")]
    #[serde(default, deserialize_with = "optional_dimension")]
    qhd_height: Option<Number>,
    #[serde(rename = "hd_hash")]
    hd_hash: Option<String>,
    #[serde(rename = "hd_bitrate")]
    hd_bitrate: Option<Number>,
    #[serde(rename = "hd_filesize")]
    hd_filesize: Option<Number>,
    #[serde(rename = "hd_width")]
    #[serde(default, deserialize_with = "optional_dimension")]
    hd_width: Option<Number>,
    #[serde(rename = "hd_height")]
    #[serde(default, deserialize_with = "optional_dimension")]
    hd_height: Option<Number>,
    #[serde(rename = "fhd_hash")]
    fhd_hash: Option<String>,
    #[serde(rename = "fhd_bitrate")]
    fhd_bitrate: Option<Number>,
    #[serde(rename = "fhd_filesize")]
    fhd_filesize: Option<Number>,
    #[serde(rename = "fhd_width")]
    #[serde(default, deserialize_with = "optional_dimension")]
    fhd_width: Option<Number>,
    #[serde(rename = "fhd_height")]
    #[serde(default, deserialize_with = "optional_dimension")]
    fhd_height: Option<Number>,
    #[serde(rename = "mkv_sd_hash")]
    mkv_sd_hash: Option<String>,
    #[serde(rename = "mkv_sd_bitrate")]
    mkv_sd_bitrate: Option<Number>,
    #[serde(rename = "mkv_sd_filesize")]
    mkv_sd_filesize: Option<Number>,
    #[serde(rename = "mkv_sd_width")]
    #[serde(default, deserialize_with = "optional_dimension")]
    mkv_sd_width: Option<Number>,
    #[serde(rename = "mkv_sd_height")]
    #[serde(default, deserialize_with = "optional_dimension")]
    mkv_sd_height: Option<Number>,
    #[serde(rename = "mkv_qhd_hash")]
    mkv_qhd_hash: Option<String>,
    #[serde(rename = "mkv_qhd_bitrate")]
    mkv_qhd_bitrate: Option<Number>,
    #[serde(rename = "mkv_qhd_filesize")]
    mkv_qhd_filesize: Option<Number>,
    #[serde(rename = "mkv_qhd_width")]
    #[serde(default, deserialize_with = "optional_dimension")]
    mkv_qhd_width: Option<Number>,
    #[serde(rename = "mkv_qhd_height")]
    #[serde(default, deserialize_with = "optional_dimension")]
    mkv_qhd_height: Option<Number>,
    #[serde(rename = "sd_hash_265")]
    sd_265_hash: Option<String>,
    #[serde(rename = "sd_bitrate_265")]
    sd_265_bitrate: Option<Number>,
    #[serde(rename = "sd_filesize_265")]
    sd_265_filesize: Option<Number>,
    #[serde(rename = "sd_width_265")]
    #[serde(default, deserialize_with = "optional_dimension")]
    sd_265_width: Option<Number>,
    #[serde(rename = "sd_height_265")]
    #[serde(default, deserialize_with = "optional_dimension")]
    sd_265_height: Option<Number>,
    #[serde(rename = "qhd_hash_265")]
    qhd_265_hash: Option<String>,
    #[serde(rename = "qhd_bitrate_265")]
    qhd_265_bitrate: Option<Number>,
    #[serde(rename = "qhd_filesize_265")]
    qhd_265_filesize: Option<Number>,
    #[serde(rename = "qhd_width_265")]
    #[serde(default, deserialize_with = "optional_dimension")]
    qhd_265_width: Option<Number>,
    #[serde(rename = "qhd_height_265")]
    #[serde(default, deserialize_with = "optional_dimension")]
    qhd_265_height: Option<Number>,
    #[serde(rename = "hd_hash_265")]
    hd_265_hash: Option<String>,
    #[serde(rename = "hd_bitrate_265")]
    hd_265_bitrate: Option<Number>,
    #[serde(rename = "hd_filesize_265")]
    hd_265_filesize: Option<Number>,
    #[serde(rename = "hd_width_265")]
    #[serde(default, deserialize_with = "optional_dimension")]
    hd_265_width: Option<Number>,
    #[serde(rename = "hd_height_265")]
    #[serde(default, deserialize_with = "optional_dimension")]
    hd_265_height: Option<Number>,
    #[serde(rename = "fhd_hash_265")]
    fhd_265_hash: Option<String>,
    #[serde(rename = "fhd_bitrate_265")]
    fhd_265_bitrate: Option<Number>,
    #[serde(rename = "fhd_filesize_265")]
    fhd_265_filesize: Option<Number>,
    #[serde(rename = "fhd_width_265")]
    #[serde(default, deserialize_with = "optional_dimension")]
    fhd_265_width: Option<Number>,
    #[serde(rename = "fhd_height_265")]
    #[serde(default, deserialize_with = "optional_dimension")]
    fhd_265_height: Option<Number>,
}
#[derive(Serialize)]
struct Asset {
    source_key: &'static str,
    hash: String,
    bitrate: Option<u64>,
    size: Option<u64>,
    width: Option<u32>,
    height: Option<u32>,
}
fn optional_dimension<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<Option<Number>, D::Error> {
    match Option::<Value>::deserialize(deserializer)? {
        None => Ok(None),
        Some(Value::String(s)) if s.is_empty() => Ok(None),
        Some(v) => Number::try_from(v)
            .map(Some)
            .map_err(serde::de::Error::custom),
    }
}
fn positive(n: Option<Number>) -> Option<u64> {
    n.map(|v| v.0).filter(|v| *v > 0)
}
fn hash(value: Option<String>) -> Result<Option<String>> {
    match value.filter(|v| !v.is_empty()) {
        None => Ok(None),
        Some(s) if s.len() == 32 && s.bytes().all(|c| c.is_ascii_hexdigit()) => {
            Ok(Some(s.to_ascii_uppercase()))
        }
        _ => Err(malformed()),
    }
}
fn push_asset(
    assets: &mut Vec<Asset>,
    source_key: &'static str,
    h: Option<String>,
    bitrate: Option<Number>,
    size: Option<Number>,
    width: Option<Number>,
    height: Option<Number>,
) -> Result<()> {
    let h = hash(h)?;
    let dimension = |n| {
        positive(n)
            .map(|v| u32::try_from(v).map_err(|_| malformed()))
            .transpose()
    };
    let width = dimension(width)?;
    let height = dimension(height)?;
    if let Some(hash) = h {
        assets.push(Asset {
            source_key,
            hash,
            bitrate: positive(bitrate),
            size: positive(size),
            width,
            height,
        });
    }
    Ok(())
}
fn assets(a: AssetFields) -> Result<Vec<Asset>> {
    let mut assets = Vec::new();
    push_asset(
        &mut assets,
        "ld",
        a.ld_hash,
        a.ld_bitrate,
        a.ld_filesize,
        a.ld_width,
        a.ld_height,
    )?;
    push_asset(
        &mut assets,
        "sd",
        a.sd_hash,
        a.sd_bitrate,
        a.sd_filesize,
        a.sd_width,
        a.sd_height,
    )?;
    push_asset(
        &mut assets,
        "qhd",
        a.qhd_hash,
        a.qhd_bitrate,
        a.qhd_filesize,
        a.qhd_width,
        a.qhd_height,
    )?;
    push_asset(
        &mut assets,
        "hd",
        a.hd_hash,
        a.hd_bitrate,
        a.hd_filesize,
        a.hd_width,
        a.hd_height,
    )?;
    push_asset(
        &mut assets,
        "fhd",
        a.fhd_hash,
        a.fhd_bitrate,
        a.fhd_filesize,
        a.fhd_width,
        a.fhd_height,
    )?;
    push_asset(
        &mut assets,
        "mkv_sd",
        a.mkv_sd_hash,
        a.mkv_sd_bitrate,
        a.mkv_sd_filesize,
        a.mkv_sd_width,
        a.mkv_sd_height,
    )?;
    push_asset(
        &mut assets,
        "mkv_qhd",
        a.mkv_qhd_hash,
        a.mkv_qhd_bitrate,
        a.mkv_qhd_filesize,
        a.mkv_qhd_width,
        a.mkv_qhd_height,
    )?;
    push_asset(
        &mut assets,
        "sd_265",
        a.sd_265_hash,
        a.sd_265_bitrate,
        a.sd_265_filesize,
        a.sd_265_width,
        a.sd_265_height,
    )?;
    push_asset(
        &mut assets,
        "qhd_265",
        a.qhd_265_hash,
        a.qhd_265_bitrate,
        a.qhd_265_filesize,
        a.qhd_265_width,
        a.qhd_265_height,
    )?;
    push_asset(
        &mut assets,
        "hd_265",
        a.hd_265_hash,
        a.hd_265_bitrate,
        a.hd_265_filesize,
        a.hd_265_width,
        a.hd_265_height,
    )?;
    push_asset(
        &mut assets,
        "fhd_265",
        a.fhd_265_hash,
        a.fhd_265_bitrate,
        a.fhd_265_filesize,
        a.fhd_265_width,
        a.fhd_265_height,
    )?;
    Ok(assets)
}
fn insert_number(e: &mut Extensions, key: &str, n: Option<Number>) {
    if let Some(n) = n {
        e.insert(key.into(), json!(n.0));
    }
}
fn insert_id(e: &mut Extensions, key: &str, n: Option<Number>) {
    if let Some(n) = positive(n) {
        e.insert(key.into(), json!(n.to_string()));
    }
}

fn parse_details(
    bytes: &[u8],
    ids: &[String],
    kind: VideoResourceKind,
) -> Result<Vec<VideoDetail>> {
    super::openapi::check_status(bytes)?;
    let envelope: Envelope = serde_json::from_slice(bytes).map_err(|_| malformed())?;
    if envelope.data.len() != ids.len() {
        return Err(malformed());
    }
    let expected: BTreeSet<_> = ids.iter().map(String::as_str).collect();
    if expected.len() != ids.len() {
        return Err(malformed());
    }
    let mut found = BTreeMap::new();
    let mut absent = false;
    for entry in envelope.data {
        let dto = match entry {
            Entry::Missing(_) => {
                absent = true;
                continue;
            }
            Entry::Found(v) => *v,
        };
        let id = dto.video_id.0.to_string();
        if !expected.contains(id.as_str()) || found.contains_key(&id) {
            return Err(malformed());
        }
        found.insert(id, map_detail(dto, kind)?);
    }
    if absent {
        return Err(missing());
    }
    ids.iter()
        .map(|id| found.remove(id).ok_or_else(malformed))
        .collect()
}
fn map_detail(v: VideoDto, kind: VideoResourceKind) -> Result<VideoDetail> {
    if v.is_publish.as_ref().is_some_and(|n| n.0 > 1) || v.deleted.as_ref().is_some_and(|n| n.0 > 1)
    {
        return Err(malformed());
    }
    if v.is_publish.as_ref().is_some_and(|n| n.0 == 0)
        || v.deleted.as_ref().is_some_and(|n| n.0 == 1)
    {
        return Err(missing());
    }
    let id = v.video_id.id()?;
    let mut creators = Vec::new();
    let mut seen = BTreeSet::new();
    if let Some(authors) = v.authors {
        if authors.len() > 100 {
            return Err(malformed());
        }
        for a in authors {
            let resource_ref = if a.author_id.0 == 0 {
                None
            } else {
                let id = a.author_id.id()?;
                if !seen.insert(id.clone()) {
                    return Err(malformed());
                }
                Some(resource(id)?)
            };
            creators.push(CreatorSummary {
                resource_ref,
                name: required_text(a.author_name)?,
                avatar_url: a.sizable_avatar.as_deref().and_then(normalize_image_url),
            });
        }
    }
    let mut extensions = Extensions::new();
    let uploader_name = optional_text(v.user_name, 1024)?;
    let author_name = optional_text(v.author_name, 1024)?;
    if let Some(uid) = positive(v.user_id) {
        extensions.insert("uploader".into(), json!({"id":uid.to_string(),
            "name":uploader_name.or(author_name), "avatar_url":v.user_avatar.as_deref().and_then(normalize_image_url)}));
    } else if creators.is_empty() {
        if let Some(name) = author_name {
            creators.push(CreatorSummary {
                resource_ref: None,
                name,
                avatar_url: None,
            });
        }
    }
    insert_id(&mut extensions, "audio_id", v.audio_id);
    insert_id(&mut extensions, "album_audio_id", v.album_audio_id);
    if let Some(h) = hash(v.audio_hash)? {
        extensions.insert("audio_hash".into(), json!(h));
    }
    for (key, n) in [
        ("audio_duration_ms", v.audio_timelength),
        ("history_heat", v.history_heat),
        ("heat", v.heat),
        ("collection_total", v.collection_total),
        ("download_total", v.download_total),
        ("type_code", v.type_code),
        ("track_code", v.track),
        ("is_short_code", v.is_short),
    ] {
        insert_number(&mut extensions, key, n);
    }
    for (key, text) in [
        ("remark", v.remark),
        ("other_description", v.other_description),
    ] {
        if let Some(text) = optional_text(text, 8192)? {
            extensions.insert(key.into(), json!(text));
        }
    }
    let assets = assets(v.assets)?;
    let resolutions = assets
        .iter()
        .filter_map(|a| {
            a.height.map(|height| VideoResolution {
                resolution: height,
                width: a.width,
                height: Some(height),
                size: a.size,
                format: None,
                extensions: Extensions::from([
                    ("source_key".into(), json!(a.source_key)),
                    ("hash".into(), json!(a.hash)),
                    ("bitrate".into(), json!(a.bitrate)),
                ]),
            })
        })
        .collect();
    extensions.insert(
        "catalogue_assets".into(),
        serde_json::to_value(assets).map_err(|_| malformed())?,
    );
    Ok(VideoDetail {
        kind,
        video: Video {
            resource_ref: video_ref(&id, kind)?,
            platform: Platform::Kugou,
            id,
            title: required_text(v.video_name)?,
            creators,
            description: optional_text(v.intro, 32768)?.unwrap_or_default(),
            cover_url: v
                .hdpic
                .as_deref()
                .and_then(normalize_image_url)
                .or_else(|| v.cover.as_deref().and_then(normalize_image_url)),
            duration_ms: positive(v.timelength),
            published_at: optional_text(v.publish_date, 128)?,
            play_count: v.play_times.map(|v| v.0),
            subscribed: None,
            extensions,
        },
        resolutions,
        extensions: Extensions::from([
            ("backend".into(), json!("official_video_metadata")),
            ("catalogue_only".into(), json!(true)),
        ]),
    })
}

#[derive(Deserialize)]
pub(super) struct MvHit {
    #[serde(rename = "MvID")]
    id: Number,
    #[serde(rename = "MvName")]
    name: String,
    #[serde(rename = "Singers")]
    singers: Option<Vec<SearchSinger>>,
    #[serde(rename = "SingerName")]
    singer_name: Option<String>,
    #[serde(rename = "Duration")]
    duration: Option<Number>,
    #[serde(rename = "Pic")]
    pic: Option<String>,
    #[serde(rename = "Description")]
    description: Option<String>,
    #[serde(rename = "PublishDate")]
    publish_date: Option<String>,
    #[serde(rename = "AudioID")]
    audio_id: Option<Number>,
    #[serde(rename = "MixSongID")]
    album_audio_id: Option<Number>,
    #[serde(rename = "AlbumID")]
    album_id: Option<Number>,
    #[serde(rename = "MvHash")]
    mv_hash: Option<String>,
    #[serde(rename = "FileHash")]
    file_hash: Option<String>,
    #[serde(rename = "MvHashMark")]
    mv_hash_mark: Option<String>,
    #[serde(rename = "HistoryHeat")]
    history_heat: Option<Number>,
    #[serde(rename = "MvHot")]
    heat: Option<Number>,
    #[serde(rename = "MvTrac")]
    track: Option<Number>,
    #[serde(rename = "MvType")]
    type_code: Option<Number>,
    #[serde(rename = "Isshort")]
    is_short: Option<Number>,
    #[serde(rename = "IsOfficial")]
    is_official: Option<Number>,
    #[serde(rename = "IsUgc")]
    is_ugc: Option<Number>,
    #[serde(rename = "Userid")]
    user_id: Option<Number>,
    #[serde(rename = "Username")]
    user_name: Option<String>,
}
#[derive(Deserialize)]
struct SearchSinger {
    id: Number,
    name: String,
}

pub(super) fn map_mv(v: MvHit) -> Result<SearchItem> {
    let id = v.id.id()?;
    let mut creators = Vec::new();
    let mut seen = BTreeSet::new();
    if let Some(singers) = v.singers {
        if singers.len() > 100 {
            return Err(malformed());
        }
        for singer in singers {
            let resource_ref = if singer.id.0 == 0 {
                None
            } else {
                let id = singer.id.id()?;
                if !seen.insert(id.clone()) {
                    return Err(malformed());
                }
                Some(resource(id)?)
            };
            creators.push(CreatorSummary {
                resource_ref,
                name: required_text(singer.name)?,
                avatar_url: None,
            });
        }
    }
    if creators.is_empty() {
        if let Some(name) = optional_text(v.singer_name, 1024)? {
            creators.push(CreatorSummary {
                resource_ref: None,
                name,
                avatar_url: None,
            });
        }
    }
    let mut extensions = Extensions::new();
    insert_id(&mut extensions, "audio_id", v.audio_id);
    insert_id(&mut extensions, "album_audio_id", v.album_audio_id);
    insert_id(&mut extensions, "album_id", v.album_id);
    for (key, value) in [("mv_hash", v.mv_hash), ("search_file_hash", v.file_hash)] {
        if let Some(h) = hash(value)? {
            extensions.insert(key.into(), json!(h));
        }
    }
    if let Some(text) = optional_text(v.mv_hash_mark, 64)? {
        extensions.insert("mv_hash_mark".into(), json!(text));
    }
    for (key, n) in [
        ("history_heat", v.history_heat),
        ("heat", v.heat),
        ("track_code", v.track),
        ("type_code", v.type_code),
        ("is_short_code", v.is_short),
        ("is_official_code", v.is_official),
        ("is_ugc_code", v.is_ugc),
    ] {
        insert_number(&mut extensions, key, n);
    }
    if let Some(uid) = positive(v.user_id) {
        extensions.insert(
            "uploader".into(),
            json!({"id":uid.to_string(), "name":optional_text(v.user_name, 1024)?}),
        );
    }
    let duration_ms = positive(v.duration)
        .map(|n| n.checked_mul(1000).ok_or_else(malformed))
        .transpose()?;
    if let Some(ms) = duration_ms {
        extensions.insert("search_duration_seconds".into(), json!(ms / 1000));
    }
    Ok(SearchItem::Video(Video {
        resource_ref: video_ref(&id, VideoResourceKind::Mv)?,
        platform: Platform::Kugou,
        id,
        title: required_text(v.name)?,
        creators,
        description: optional_text(v.description, 8192)?.unwrap_or_default(),
        cover_url: v.pic.as_deref().and_then(normalize_image_url),
        duration_ms,
        published_at: optional_text(v.publish_date, 128)?,
        play_count: None,
        subscribed: None,
        extensions,
    }))
}

#[cfg(test)]
mod tests;
