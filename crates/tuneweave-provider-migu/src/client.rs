use std::{collections::BTreeMap, fmt, time::Duration};

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
const RESOURCE_INFO_ENDPOINT: &str =
    "https://app.u.nf.migu.cn/MIGUM2.0/v1.0/content/resourceinfo.do";
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

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct MiguResourceInfoRequest<'a> {
    resource_id: &'a str,
    copyright_id: &'static str,
    resource_type: u8,
}

#[derive(Deserialize)]
struct MiguResourceEnvelope {
    #[serde(default)]
    code: String,
    #[serde(default)]
    info: String,
    #[serde(default)]
    resource: Vec<MiguResource>,
}

#[derive(Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct MiguResource {
    resource_type: String,
    copyright_id: String,
    content_id: String,
    song_id: String,
    song_name: String,
    singer_id: String,
    singer: String,
    album_id: String,
    album: String,
    album_imgs: Vec<MiguImage>,
    op_num_item: Option<MiguResourceStatistics>,
    tone_control: String,
    related_songs: Vec<MiguRelatedResource>,
    rate_formats: Vec<MiguRateFormat>,
    new_rate_formats: Vec<MiguRateFormat>,
    lrc_url: String,
    mrc_url: String,
    trc_url: String,
    tag_list: Vec<MiguResourceTag>,
    copyright: String,
    valid_status: Option<bool>,
    song_descs: String,
    song_alias_name: String,
    is_in_d_album: String,
    is_in_side_dalbum: String,
    is_in_sales_period: String,
    song_type: String,
    invalidate_date: String,
    dalbum_id: String,
    track_number: String,
    disc: String,
    vip_type: String,
    scope_ofcopyright: String,
    auditions_type: String,
    first_icon: String,
    translate_name: String,
    charge_auditions: String,
    old_charge_auditions: String,
    song_icon: String,
    auditions_length: Option<FlexibleU64>,
    auditions_start_time: Option<FlexibleU64>,
    code_rate: BTreeMap<String, MiguCodeRateRights>,
    vip_flag: String,
    is_download: String,
    copyright_type: String,
    has_mv: String,
    mv_copyright: String,
    top_quality: String,
    first_click: String,
    pre_sale: String,
    is_share: String,
    is_collection: String,
    length: String,
    auditions_flag: String,
    listen_flag: String,
    artists: Vec<MiguResourceArtist>,
    landscap_img: String,
    vip_start_time: String,
    vip_end_time: String,
    vip_logo: String,
    vip_download: String,
    first_publish: String,
    first_start_time: String,
    first_end_time: String,
    show_tag: Vec<String>,
    material_valid_status: Option<bool>,
    need_encrypt: String,
    forever_listen_flag: String,
    forever_listen: Option<bool>,
    action_img_url: String,
    is_recreate: String,
    has_associated_ring: Option<bool>,
    chorus_start_time: Option<FlexibleU64>,
    product_authorize_usage: String,
    audio_book: String,
    originals: Vec<MiguOriginalTrack>,
}

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
struct MiguImage {
    img_size_type: String,
    img: String,
    img_ori: String,
    webp_img: String,
}

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
struct MiguResourceArtist {
    id: String,
    name: String,
    name_spelling: String,
}

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
struct MiguRelatedResource {
    resource_type: String,
    resource_type_name: String,
    copyright_id: String,
    product_id: String,
}

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
struct MiguRateFormat {
    resource_type: String,
    format_type: String,
    format: String,
    size: Option<FlexibleU64>,
    file_type: String,
    price: String,
    android_file_type: String,
    ios_file_type: String,
    ios_size: Option<FlexibleU64>,
    android_size: Option<FlexibleU64>,
    ios_format: String,
    android_format: String,
    ios_accuracy_level: String,
    android_accuracy_level: String,
    show_tag: Vec<String>,
}

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
struct MiguResourceTag {
    resource_type: String,
    tag_id: String,
    tag_name: String,
}

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
struct MiguOriginalTrack {
    #[serde(rename = "type")]
    kind: String,
    song_id: String,
    song_name: String,
}

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
struct MiguCodeRateRights {
    code_rate_charge_auditions: String,
    code_rate_auditions_length: Option<FlexibleU64>,
    code_rate_charge_auditions_type: String,
    is_code_rate_download: String,
    code_rate_file_size: Option<FlexibleU64>,
    content_id_sq: String,
    quality_icon: String,
}

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
struct MiguResourceStatistics {
    play_num: Option<FlexibleU64>,
    play_num_desc: String,
    keep_num: Option<FlexibleU64>,
    keep_num_desc: String,
    comment_num: Option<FlexibleU64>,
    comment_num_desc: String,
    share_num: Option<FlexibleU64>,
    share_num_desc: String,
    live_play_num: Option<FlexibleU64>,
    live_play_num_desc: String,
}

#[derive(Serialize)]
struct MiguResourceRights {
    valid_status: Option<bool>,
    material_valid_status: Option<bool>,
    copyright: Option<String>,
    copyright_type: Option<String>,
    vip_type: Option<String>,
    vip_flag: Option<String>,
    vip_download: Option<String>,
    is_download: Option<String>,
    listen_flag: Option<String>,
    auditions_flag: Option<String>,
    auditions_type: Option<String>,
    charge_auditions: Option<String>,
    old_charge_auditions: Option<String>,
    auditions_length_seconds: Option<u64>,
    auditions_start_seconds: Option<u64>,
    top_quality: Option<String>,
    forever_listen_flag: Option<String>,
    forever_listen: Option<bool>,
    need_encrypt: Option<String>,
    code_rates: BTreeMap<String, MiguCodeRateRights>,
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

    pub(crate) async fn track_detail(&self, content_id: &str) -> Result<Track> {
        let response = self
            .http
            .get(RESOURCE_INFO_ENDPOINT)
            .header(ACCEPT, "application/json")
            .query(&MiguResourceInfoRequest {
                resource_id: content_id,
                copyright_id: "",
                resource_type: 2,
            })
            .send()
            .await
            .map_err(migu_network_error)?;
        let bytes = read_bounded_response(response, "Migu resource detail").await?;
        parse_resource_response(&bytes, content_id)
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

fn parse_resource_response(bytes: &[u8], requested_content_id: &str) -> Result<Track> {
    let envelope: MiguResourceEnvelope = serde_json::from_slice(bytes)
        .map_err(|_| migu_upstream_error("Migu resource detail returned malformed JSON"))?;
    if envelope.code != "000000" {
        return Err(
            migu_upstream_error("Migu resource detail rejected the request").with_details(json!({
                "platform_code": bounded_text(&envelope.code, 64),
                "platform_message": bounded_text(&envelope.info, 256),
            })),
        );
    }
    if envelope.resource.is_empty() {
        return Err(
            TuneWeaveError::new(ErrorCode::ResourceNotFound, "Migu track was not found")
                .with_platform(Platform::Migu),
        );
    }
    if envelope.resource.len() != 1 {
        return Err(migu_upstream_error(
            "Migu resource detail returned an unexpected number of tracks",
        ));
    }
    let resource =
        envelope.resource.into_iter().next().ok_or_else(|| {
            migu_upstream_error("Migu resource detail omitted the requested track")
        })?;
    if resource.content_id != requested_content_id {
        return Err(migu_upstream_error(
            "Migu resource detail returned a mismatched content ID",
        ));
    }
    map_resource_track(resource)
}

fn map_resource_track(resource: MiguResource) -> Result<Track> {
    let content_id = canonical_platform_id(&resource.content_id)
        .ok_or_else(|| migu_upstream_error("Migu resource detail omitted a stable content ID"))?;
    if resource.resource_type != "2" {
        return Err(migu_upstream_error(
            "Migu resource detail returned an incompatible resource type",
        ));
    }
    if canonical_platform_id(&resource.copyright_id).is_none() {
        return Err(migu_upstream_error(
            "Migu resource detail omitted a valid copyright ID",
        ));
    }
    let name = validated_name(&resource.song_name)
        .ok_or_else(|| migu_upstream_error("Migu resource detail omitted a track name"))?;
    let resource_ref = ResourceRef::new(Platform::Migu, content_id.to_owned())
        .map_err(|_| migu_upstream_error("Migu resource detail returned an invalid identity"))?;
    let mut track = Track::new(resource_ref, name);
    push_alias(&mut track.aliases, &track.name, &resource.song_alias_name);
    push_alias(&mut track.aliases, &track.name, &resource.translate_name);
    track.artists = map_resource_artists(&resource);
    track.album = map_resource_album(&resource);
    track.duration_ms = parse_duration_text(&resource.length)?;
    track.mv_ref = resource
        .related_songs
        .iter()
        .find(|related| related.resource_type == "D")
        .and_then(|related| canonical_platform_id(&related.product_id))
        .and_then(|id| ResourceRef::new(Platform::Migu, id.to_owned()).ok());
    track.available_qualities = mapped_rate_qualities(
        resource
            .rate_formats
            .iter()
            .chain(&resource.new_rate_formats),
    );
    if resource.valid_status == Some(false) || resource.material_valid_status == Some(false) {
        track.playable = Some(false);
    }

    track
        .extensions
        .insert("backend".to_owned(), json!("resourceinfo_v1"));
    insert_nonempty(
        &mut track.extensions,
        "copyright_id",
        &resource.copyright_id,
    );
    insert_nonempty(&mut track.extensions, "song_id", &resource.song_id);
    track
        .extensions
        .insert("resource_type".to_owned(), json!(resource.resource_type));
    insert_nonempty(&mut track.extensions, "song_type", &resource.song_type);
    insert_nonempty(
        &mut track.extensions,
        "invalidate_date",
        &resource.invalidate_date,
    );
    insert_nonempty(
        &mut track.extensions,
        "track_number",
        &resource.track_number,
    );
    insert_nonempty(&mut track.extensions, "disc", &resource.disc);
    insert_nonempty(
        &mut track.extensions,
        "tone_control",
        &resource.tone_control,
    );
    insert_nonempty(
        &mut track.extensions,
        "scope_of_copyright",
        &resource.scope_ofcopyright,
    );
    insert_nonempty(
        &mut track.extensions,
        "product_authorize_usage",
        &resource.product_authorize_usage,
    );
    insert_nonempty(&mut track.extensions, "audio_book", &resource.audio_book);
    insert_nonempty(
        &mut track.extensions,
        "digital_album_id",
        &resource.dalbum_id,
    );
    insert_nonempty(
        &mut track.extensions,
        "song_description",
        &resource.song_descs,
    );
    insert_optional_string(
        &mut track.extensions,
        "landscape_image_url",
        normalize_media_url(&resource.landscap_img),
    );
    insert_optional_string(
        &mut track.extensions,
        "action_image_url",
        normalize_media_url(&resource.action_img_url),
    );
    insert_safe_media_url(&mut track.extensions, "lrc_url", &resource.lrc_url);
    insert_safe_media_url(&mut track.extensions, "mrc_url", &resource.mrc_url);
    insert_safe_media_url(&mut track.extensions, "trc_url", &resource.trc_url);
    if let Some(value) = resource
        .chorus_start_time
        .as_ref()
        .and_then(FlexibleU64::get)
    {
        track
            .extensions
            .insert("chorus_start_ms".to_owned(), json!(value));
    }

    let rights = map_resource_rights(&resource);
    track.extensions.insert("rights".to_owned(), json!(rights));
    let rate_formats = bounded_rate_formats(&resource.rate_formats);
    if !rate_formats.is_empty() {
        track
            .extensions
            .insert("rate_formats".to_owned(), json!(rate_formats));
    }
    let new_rate_formats = bounded_rate_formats(&resource.new_rate_formats);
    if !new_rate_formats.is_empty() {
        track
            .extensions
            .insert("new_rate_formats".to_owned(), json!(new_rate_formats));
    }
    let related_resources = bounded_related_resources(&resource.related_songs);
    if !related_resources.is_empty() {
        track
            .extensions
            .insert("related_resources".to_owned(), json!(related_resources));
    }
    let tags = bounded_resource_tags(&resource.tag_list);
    if !tags.is_empty() {
        track.extensions.insert("tags".to_owned(), json!(tags));
    }
    if let Some(statistics) = resource.op_num_item.as_ref().map(bounded_statistics) {
        track
            .extensions
            .insert("statistics".to_owned(), json!(statistics));
    }
    let originals = bounded_original_tracks(&resource.originals);
    if !originals.is_empty() {
        track
            .extensions
            .insert("originals".to_owned(), json!(originals));
    }
    insert_resource_flags(&mut track.extensions, &resource);
    insert_string_list(&mut track.extensions, "show_tags", resource.show_tag);
    Ok(track)
}

fn map_resource_artists(resource: &MiguResource) -> Vec<ArtistSummary> {
    let mut artists = resource
        .artists
        .iter()
        .filter_map(|artist| {
            Some(ArtistSummary {
                resource_ref: canonical_platform_id(&artist.id)
                    .and_then(|id| ResourceRef::new(Platform::Migu, id.to_owned()).ok()),
                name: validated_name(&artist.name)?.to_owned(),
            })
        })
        .collect::<Vec<_>>();
    if artists.is_empty()
        && let Some(name) = validated_name(&resource.singer)
    {
        artists.push(ArtistSummary {
            resource_ref: canonical_platform_id(&resource.singer_id)
                .and_then(|id| ResourceRef::new(Platform::Migu, id.to_owned()).ok()),
            name: name.to_owned(),
        });
    }
    artists
}

fn map_resource_album(resource: &MiguResource) -> Option<AlbumSummary> {
    let name = validated_name(&resource.album)?;
    let resource_ref = canonical_platform_id(&resource.album_id)
        .and_then(|id| ResourceRef::new(Platform::Migu, id.to_owned()).ok());
    let cover_url = preferred_image(&resource.album_imgs);
    Some(AlbumSummary {
        resource_ref,
        name: name.to_owned(),
        cover_url,
    })
}

fn preferred_image(images: &[MiguImage]) -> Option<String> {
    ["03", "02", "01"]
        .into_iter()
        .find_map(|kind| {
            images
                .iter()
                .find(|image| image.img_size_type == kind)
                .and_then(normalized_image)
        })
        .or_else(|| images.iter().find_map(normalized_image))
}

fn normalized_image(image: &MiguImage) -> Option<String> {
    [&image.webp_img, &image.img, &image.img_ori]
        .into_iter()
        .find_map(|value| normalize_media_url(value))
}

fn parse_duration_text(value: &str) -> Result<Option<u64>> {
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }
    let parts = value
        .split(':')
        .map(|part| part.parse::<u64>())
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|_| migu_upstream_error("Migu resource detail returned an invalid duration"))?;
    let seconds = match parts.as_slice() {
        [minutes, seconds] if *seconds < 60 => minutes
            .checked_mul(60)
            .and_then(|value| value.checked_add(*seconds)),
        [hours, minutes, seconds] if *minutes < 60 && *seconds < 60 => hours
            .checked_mul(3_600)
            .and_then(|value| {
                minutes
                    .checked_mul(60)
                    .and_then(|part| value.checked_add(part))
            })
            .and_then(|value| value.checked_add(*seconds)),
        _ => None,
    }
    .ok_or_else(|| migu_upstream_error("Migu resource detail returned an invalid duration"))?;
    seconds
        .checked_mul(1_000)
        .map(Some)
        .ok_or_else(|| migu_upstream_error("Migu resource detail duration overflowed"))
}

fn push_alias(aliases: &mut Vec<String>, name: &str, candidate: &str) {
    if let Some(candidate) = validated_name(candidate)
        && candidate != name
        && !aliases.iter().any(|alias| alias == candidate)
    {
        aliases.push(candidate.to_owned());
    }
}

fn mapped_rate_qualities<'a>(
    formats: impl IntoIterator<Item = &'a MiguRateFormat>,
) -> Vec<Quality> {
    let mut qualities = Vec::new();
    for format in formats {
        let quality = match format.format_type.as_str() {
            "LQ" => Some(Quality::Low),
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

fn map_resource_rights(resource: &MiguResource) -> MiguResourceRights {
    let code_rates = resource
        .code_rate
        .iter()
        .take(32)
        .filter_map(|(key, value)| {
            Some((
                canonical_platform_id(key)?.to_owned(),
                MiguCodeRateRights {
                    code_rate_charge_auditions: bounded_text(&value.code_rate_charge_auditions, 32),
                    code_rate_auditions_length: value.code_rate_auditions_length.clone(),
                    code_rate_charge_auditions_type: bounded_text(
                        &value.code_rate_charge_auditions_type,
                        32,
                    ),
                    is_code_rate_download: bounded_text(&value.is_code_rate_download, 32),
                    code_rate_file_size: value.code_rate_file_size.clone(),
                    content_id_sq: canonical_platform_id(&value.content_id_sq)
                        .unwrap_or_default()
                        .to_owned(),
                    quality_icon: bounded_text(&value.quality_icon, 32),
                },
            ))
        })
        .collect();
    MiguResourceRights {
        valid_status: resource.valid_status,
        material_valid_status: resource.material_valid_status,
        copyright: bounded_optional(&resource.copyright, 32),
        copyright_type: bounded_optional(&resource.copyright_type, 32),
        vip_type: bounded_optional(&resource.vip_type, 32),
        vip_flag: bounded_optional(&resource.vip_flag, 32),
        vip_download: bounded_optional(&resource.vip_download, 32),
        is_download: bounded_optional(&resource.is_download, 32),
        listen_flag: bounded_optional(&resource.listen_flag, 32),
        auditions_flag: bounded_optional(&resource.auditions_flag, 32),
        auditions_type: bounded_optional(&resource.auditions_type, 32),
        charge_auditions: bounded_optional(&resource.charge_auditions, 32),
        old_charge_auditions: bounded_optional(&resource.old_charge_auditions, 32),
        auditions_length_seconds: resource
            .auditions_length
            .as_ref()
            .and_then(FlexibleU64::get),
        auditions_start_seconds: resource
            .auditions_start_time
            .as_ref()
            .and_then(FlexibleU64::get),
        top_quality: bounded_optional(&resource.top_quality, 32),
        forever_listen_flag: bounded_optional(&resource.forever_listen_flag, 32),
        forever_listen: resource.forever_listen,
        need_encrypt: bounded_optional(&resource.need_encrypt, 32),
        code_rates,
    }
}

fn bounded_rate_formats(values: &[MiguRateFormat]) -> Vec<MiguRateFormat> {
    values
        .iter()
        .take(32)
        .map(|value| MiguRateFormat {
            resource_type: bounded_text(&value.resource_type, 32),
            format_type: bounded_text(&value.format_type, 32),
            format: bounded_text(&value.format, 64),
            size: value.size.clone(),
            file_type: bounded_text(&value.file_type, 32),
            price: bounded_text(&value.price, 64),
            android_file_type: bounded_text(&value.android_file_type, 32),
            ios_file_type: bounded_text(&value.ios_file_type, 32),
            ios_size: value.ios_size.clone(),
            android_size: value.android_size.clone(),
            ios_format: bounded_text(&value.ios_format, 64),
            android_format: bounded_text(&value.android_format, 64),
            ios_accuracy_level: bounded_text(&value.ios_accuracy_level, 64),
            android_accuracy_level: bounded_text(&value.android_accuracy_level, 64),
            show_tag: bounded_string_list(&value.show_tag, 32, 128),
        })
        .collect()
}

fn bounded_related_resources(values: &[MiguRelatedResource]) -> Vec<MiguRelatedResource> {
    values
        .iter()
        .take(64)
        .filter_map(|value| {
            Some(MiguRelatedResource {
                resource_type: bounded_text(&value.resource_type, 32),
                resource_type_name: bounded_text(&value.resource_type_name, 128),
                copyright_id: canonical_platform_id(&value.copyright_id)?.to_owned(),
                product_id: canonical_platform_id(&value.product_id)?.to_owned(),
            })
        })
        .collect()
}

fn bounded_resource_tags(values: &[MiguResourceTag]) -> Vec<MiguResourceTag> {
    values
        .iter()
        .take(128)
        .filter_map(|value| {
            Some(MiguResourceTag {
                resource_type: bounded_text(&value.resource_type, 32),
                tag_id: canonical_platform_id(&value.tag_id)?.to_owned(),
                tag_name: validated_name(&value.tag_name)?.to_owned(),
            })
        })
        .collect()
}

fn bounded_statistics(value: &MiguResourceStatistics) -> MiguResourceStatistics {
    MiguResourceStatistics {
        play_num: value.play_num.clone(),
        play_num_desc: bounded_text(&value.play_num_desc, 128),
        keep_num: value.keep_num.clone(),
        keep_num_desc: bounded_text(&value.keep_num_desc, 128),
        comment_num: value.comment_num.clone(),
        comment_num_desc: bounded_text(&value.comment_num_desc, 128),
        share_num: value.share_num.clone(),
        share_num_desc: bounded_text(&value.share_num_desc, 128),
        live_play_num: value.live_play_num.clone(),
        live_play_num_desc: bounded_text(&value.live_play_num_desc, 128),
    }
}

fn bounded_original_tracks(values: &[MiguOriginalTrack]) -> Vec<MiguOriginalTrack> {
    values
        .iter()
        .take(128)
        .filter_map(|value| {
            Some(MiguOriginalTrack {
                kind: bounded_text(&value.kind, 32),
                song_id: canonical_platform_id(&value.song_id)?.to_owned(),
                song_name: validated_name(&value.song_name)?.to_owned(),
            })
        })
        .collect()
}

fn insert_resource_flags(extensions: &mut tuneweave_core::Extensions, resource: &MiguResource) {
    let flags = [
        ("is_in_digital_album", resource.is_in_d_album.as_str()),
        (
            "is_in_side_digital_album",
            resource.is_in_side_dalbum.as_str(),
        ),
        ("is_in_sales_period", resource.is_in_sales_period.as_str()),
        ("has_mv", resource.has_mv.as_str()),
        ("mv_copyright", resource.mv_copyright.as_str()),
        ("first_icon", resource.first_icon.as_str()),
        ("song_icon", resource.song_icon.as_str()),
        ("first_click", resource.first_click.as_str()),
        ("pre_sale", resource.pre_sale.as_str()),
        ("is_share", resource.is_share.as_str()),
        ("is_collection", resource.is_collection.as_str()),
        ("vip_start_time", resource.vip_start_time.as_str()),
        ("vip_end_time", resource.vip_end_time.as_str()),
        ("vip_logo", resource.vip_logo.as_str()),
        ("first_publish", resource.first_publish.as_str()),
        ("first_start_time", resource.first_start_time.as_str()),
        ("first_end_time", resource.first_end_time.as_str()),
        ("is_recreate", resource.is_recreate.as_str()),
    ];
    let values = flags
        .into_iter()
        .filter_map(|(key, value)| bounded_optional(value, 64).map(|value| (key, value)))
        .collect::<BTreeMap<_, _>>();
    if !values.is_empty() {
        extensions.insert("resource_flags".to_owned(), json!(values));
    }
    insert_optional(
        extensions,
        "has_associated_ring",
        resource.has_associated_ring,
    );
}

fn bounded_optional(value: &str, max_bytes: usize) -> Option<String> {
    let value = bounded_text(value, max_bytes);
    (!value.is_empty()).then_some(value)
}

fn insert_nonempty(extensions: &mut tuneweave_core::Extensions, key: &str, value: &str) {
    if let Some(value) = nonempty(value) {
        extensions.insert(key.to_owned(), json!(bounded_text(value, 2_048)));
    }
}

fn insert_optional_string(
    extensions: &mut tuneweave_core::Extensions,
    key: &str,
    value: Option<String>,
) {
    if let Some(value) = value {
        extensions.insert(key.to_owned(), json!(value));
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

    const RESOURCE_RESPONSE: &str = r#"{
      "code":"000000",
      "resource":[{
        "resourceType":"2",
        "copyrightId":"60054704028",
        "contentId":"600908000007288315",
        "songId":"1004202180",
        "songName":"告白气球",
        "singerId":"112",
        "singer":"周杰伦",
        "artists":[{"id":"112","name":"周杰伦","nameSpelling":"zhoujielun"}],
        "albumId":"1003767159",
        "album":"周杰伦的床边故事",
        "albumImgs":[
          {"imgSizeType":"01","img":"https://evil.example/cover"},
          {"imgSizeType":"03","img":"https://d.musicapp.migu.cn/data/oss/resource/cover.webp"}
        ],
        "opNumItem":{"playNum":357169016,"playNumDesc":"3.6亿","keepNum":"913608","keepNumDesc":"91.4万"},
        "toneControl":"111100",
        "relatedSongs":[
          {"resourceType":"E","resourceTypeName":"无损","copyrightId":"60054704028","productId":"600908000007288315"},
          {"resourceType":"D","resourceTypeName":"视频","copyrightId":"600547Y0291","productId":"600906000000389715"}
        ],
        "rateFormats":[
          {"resourceType":"3","formatType":"LQ","format":"000019","size":"1725628","fileType":"mp3"},
          {"resourceType":"2","formatType":"PQ","format":"020007","size":"3450883","fileType":"mp3"},
          {"resourceType":"2","formatType":"AV3A","format":"020041","size":"22426036","fileType":"m4a"}
        ],
        "newRateFormats":[
          {"resourceType":"2","formatType":"PQ","format":"020007","size":"3450883","fileType":"mp3"},
          {"resourceType":"2","formatType":"HQ","format":"020010","size":"8626889","fileType":"mp3"},
          {"resourceType":"E","formatType":"SQ","format":"011002","size":"25117488","androidFileType":"flac"},
          {"resourceType":"2","formatType":"AV3A","format":"020041","size":"22426036","fileType":"m4a"}
        ],
        "lrcUrl":"https://d.musicapp.migu.cn/data/oss/resource/lrc",
        "mrcUrl":"https://d.musicapp.migu.cn/data/oss/resource/mrc",
        "tagList":[{"resourceType":"2034","tagId":"1000001672","tagName":"流行"}],
        "copyright":"1",
        "validStatus":true,
        "materialValidStatus":true,
        "songAliasName":"Love Confession",
        "translateName":"Love Confession",
        "songType":"01",
        "invalidateDate":"2030-12-31",
        "trackNumber":"8",
        "disc":"Disc 1",
        "vipType":"1",
        "auditionsType":"04",
        "chargeAuditions":"1",
        "auditionsLength":60,
        "auditionsStartTime":"65",
        "codeRate":{"PQ":{"codeRateChargeAuditions":"1","codeRateAuditionsLength":60,"isCodeRateDownload":"0","codeRateFileSize":"3450883"}},
        "vipFlag":"1",
        "isDownload":"0",
        "copyrightType":"1",
        "hasMv":"1",
        "topQuality":"SQ",
        "length":"00:03:35",
        "auditionsFlag":"9",
        "listenFlag":"11",
        "showTag":["vip","isq"],
        "foreverListenFlag":"0",
        "foreverListen":false,
        "chorusStartTime":"149040",
        "originals":[{"type":"9","songId":"1125329696","songName":"告白气球 (Live)"}]
      }]
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
    fn resource_detail_maps_identity_aliases_formats_rights_and_related_mv() {
        let track = parse_resource_response(RESOURCE_RESPONSE.as_bytes(), "600908000007288315")
            .expect("parse resource detail");
        assert_eq!(track.resource_ref.to_string(), "migu:600908000007288315");
        assert_eq!(track.name, "告白气球");
        assert_eq!(track.aliases, vec!["Love Confession"]);
        assert_eq!(
            track.artists[0]
                .resource_ref
                .as_ref()
                .map(ToString::to_string)
                .as_deref(),
            Some("migu:112")
        );
        assert_eq!(
            track
                .album
                .as_ref()
                .and_then(|album| album.cover_url.as_deref()),
            Some("https://d.musicapp.migu.cn/data/oss/resource/cover.webp")
        );
        assert_eq!(track.duration_ms, Some(215_000));
        assert_eq!(
            track.mv_ref.as_ref().map(ToString::to_string).as_deref(),
            Some("migu:600906000000389715")
        );
        assert_eq!(
            track.available_qualities,
            vec![
                Quality::Low,
                Quality::Standard,
                Quality::High,
                Quality::Lossless
            ]
        );
        assert_eq!(track.playable, None);
        assert_eq!(track.extensions["backend"], "resourceinfo_v1");
        assert_eq!(
            track.extensions["new_rate_formats"][3]["formatType"],
            "AV3A"
        );
        assert_eq!(track.extensions["rights"]["auditions_start_seconds"], 65);
        assert_eq!(track.extensions["statistics"]["playNum"], 357169016);
        assert_eq!(track.extensions["originals"][0]["songId"], "1125329696");
    }

    #[test]
    fn resource_detail_rejects_missing_duplicate_and_mismatched_identities() {
        let missing = br#"{"code":"000000","resource":[]}"#;
        let error =
            parse_resource_response(missing, "600908000007288315").expect_err("missing track");
        assert_eq!(error.code, ErrorCode::ResourceNotFound);

        let mismatch = RESOURCE_RESPONSE.replace(
            "\"contentId\":\"600908000007288315\"",
            "\"contentId\":\"600908000007288316\"",
        );
        assert!(parse_resource_response(mismatch.as_bytes(), "600908000007288315").is_err());

        let duplicate = RESOURCE_RESPONSE.replace(
            "\"resource\":[{",
            "\"resource\":[{\"resourceType\":\"2\",\"contentId\":\"600908000007288315\",\"songName\":\"first\"},{",
        );
        assert!(parse_resource_response(duplicate.as_bytes(), "600908000007288315").is_err());
    }

    #[test]
    fn resource_detail_only_marks_explicit_invalid_material_unplayable() {
        let invalid = RESOURCE_RESPONSE.replace(
            "\"materialValidStatus\":true",
            "\"materialValidStatus\":false",
        );
        let track = parse_resource_response(invalid.as_bytes(), "600908000007288315")
            .expect("parse invalid material");
        assert_eq!(track.playable, Some(false));
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

    #[tokio::test]
    #[ignore = "requires live Migu network access"]
    async fn live_resource_detail_returns_current_identity_and_formats() {
        let client = MiguClient::test_client();
        let track = client
            .track_detail("600908000007288315")
            .await
            .expect("live Migu resource detail");
        assert_eq!(track.resource_ref.to_string(), "migu:600908000007288315");
        assert_eq!(track.extensions["copyright_id"], "60054704028");
        assert!(!track.available_qualities.is_empty());
    }
}
