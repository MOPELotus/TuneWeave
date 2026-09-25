//! Concept AlbumDataModel catalogue input for one ordinary playlist addition.
use super::dto::Number;
use super::*;
use crate::{KugouLoginClient, signing::concept_signature};

const PATH: &str = "/kmr/v1/album_songlist";
const PAGE_SIZE: u64 = 20;
const MAX_ITEMS: u64 = 1280;
const RESPONSE_LIMIT: usize = 1_048_576;
const TOTAL_LIMIT: usize = 4 * RESPONSE_LIMIT;

pub(crate) struct ConceptAlbumTrack {
    pub(crate) name: String,
    pub(crate) hash: String,
    pub(crate) size: i32,
    pub(crate) timelen: i32,
    pub(crate) bitrate: i16,
    pub(crate) album_id: String,
    pub(crate) mixsongid: u64,
}

#[derive(Deserialize)]
struct Envelope {
    status: Number,
    error_code: Option<Number>,
    data: Catalogue,
}
#[derive(Deserialize)]
struct Catalogue {
    total: Number,
    page: Option<Number>,
    pagesize: Option<Number>,
    songs: Vec<Song>,
}
#[derive(Deserialize)]
struct Song {
    base: Base,
    audio_info: Option<Audio>,
}
#[derive(Deserialize)]
struct Base {
    album_audio_id: Number,
    album_id: Number,
    is_publish: Option<Number>,
    author_name: Option<String>,
    audio_name: Option<String>,
}
#[derive(Deserialize)]
struct Audio {
    hash: String,
    extname: String,
    duration: Number,
    filesize: Number,
    bitrate: Number,
}

impl KugouClient {
    pub(crate) async fn concept_album_track(
        &self,
        expected: u64,
        mut observe: impl FnMut(usize) -> Result<()> + Send,
    ) -> Result<ConceptAlbumTrack> {
        if expected == 0 || expected > i64::MAX as u64 {
            return Err(invalid());
        }
        let result = tokio::time::timeout(Duration::from_secs(30), async {
            let device = self.device_identity()?;
            observe(0)?;
            // This anonymous lookup supplies only an album hint. Every write
            // field below must come from the exact Concept AlbumDataModel row.
            let bytes = async {
                let response = self
                    .post_android(
                        AndroidEndpoint::TrackMetadata,
                        &TrackMetadataRequest {
                            data: [TrackMetadataIdentity {
                                entity_id: expected,
                            }],
                            fields: "base",
                        },
                        &device,
                    )
                    .await?;
                read_public(response).await
            }
            .await;
            observe(bytes.as_ref().map_or(0, Vec::len))?;
            let bytes = bytes?;
            let mut used = bytes.len();
            let hint = parse_track_metadata(&bytes, expected)?;
            let album = hint
                .base
                .and_then(|base| base.album_id)
                .and_then(|n| n.as_u64())
                .filter(|id| *id > 0 && *id <= i32::MAX as u64)
                .ok_or_else(invalid)?;
            let mut total = None;
            let mut signatures = BTreeSet::new();
            let mut found = None;
            for page in 1..=MAX_ITEMS / PAGE_SIZE {
                observe(0)?;
                let bytes = self.concept_album_page(album, page, &device).await;
                observe(bytes.as_ref().map_or(0, Vec::len))?;
                let bytes = bytes?;
                used = used.checked_add(bytes.len()).ok_or_else(invalid)?;
                if used > TOTAL_LIMIT {
                    return Err(invalid());
                }
                let data = parse(&bytes, album, page)?;
                if total.is_some_and(|v| v != data.total.0) {
                    return Err(invalid());
                }
                total = Some(data.total.0);
                let ids: Vec<_> = data.songs.iter().map(|v| v.base.album_audio_id.0).collect();
                if !data.songs.is_empty() && !signatures.insert(ids) {
                    return Err(invalid());
                }
                for row in data.songs {
                    if row.base.album_audio_id.0 == expected {
                        if found.is_some() {
                            return Err(invalid());
                        }
                        found = Some(map(row)?);
                    }
                }
                if page * PAGE_SIZE >= data.total.0 {
                    return found.ok_or_else(invalid);
                }
            }
            Err(invalid())
        })
        .await
        .unwrap_or_else(|_| {
            Err(kugou_upstream_error(
                "KuGou Concept catalogue exceeded its deadline",
            ))
        });
        // A timeout or a failed public response must also observe a concurrent
        // account change before the caller handles the error.
        observe(0)?;
        result
    }

    async fn concept_album_page(
        &self,
        album: u64,
        page: u64,
        device: &KugouDeviceIdentity,
    ) -> Result<Vec<u8>> {
        let body = serde_json::to_vec(&json!({"fields":"musical","album_id":album.to_string(),
            "page":page.to_string(),"pagesize":PAGE_SIZE.to_string(),"is_buy":"0"}))
        .map_err(|_| invalid())?;
        let profile = KugouLoginClient::Concept;
        let mut query = BTreeMap::from([
            ("appid", profile.appid().to_string()),
            ("clientver", profile.clientver().to_string()),
            ("mid", device.mid.clone()),
            ("dfid", device.dfid().to_owned()),
            ("uuid", "-".into()),
            ("clienttime", unix_seconds_now().to_string()),
        ]);
        query.insert("signature", concept_signature(&query, &body));
        let url = format!("https://openapi.kugou.com{PATH}");
        #[cfg(test)]
        let url = self
            .login_test_origin
            .as_ref()
            .map(|origin| origin.join(PATH).unwrap().to_string())
            .unwrap_or(url);
        let started = Instant::now();
        let mut status = None;
        let result = async {
            let response = self
                .http
                .post(url)
                .query(&query)
                .header(CONTENT_TYPE, "application/json;charset=utf-8")
                .header("kg-tid", "221")
                .body(body)
                .send()
                .await
                .map_err(kugou_network_error)?;
            status = Some(response.status());
            read_public(response).await
        }
        .await;
        self.log_upstream_request(
            "concept_album_track_input",
            "openapi.kugou.com",
            PATH,
            status,
            started,
            0,
            false,
            &result,
        );
        result
    }
}

async fn read_public(mut response: reqwest::Response) -> Result<Vec<u8>> {
    if !response.status().is_success() {
        return Err(kugou_http_error(response.status()));
    }
    let content_type = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(';').next())
        .map(str::trim);
    if response.headers().contains_key("ssa-code")
        || !matches!(content_type, Some("application/json" | "text/plain"))
        || response
            .content_length()
            .is_some_and(|v| v > RESPONSE_LIMIT as u64)
    {
        return Err(invalid());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(kugou_network_error)? {
        if bytes.len().saturating_add(chunk.len()) > RESPONSE_LIMIT {
            return Err(invalid());
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

fn parse(bytes: &[u8], album: u64, page: u64) -> Result<Catalogue> {
    if !(1..=MAX_ITEMS / PAGE_SIZE).contains(&page) {
        return Err(invalid());
    }
    let value: Envelope = serde_json::from_slice(bytes).map_err(|_| invalid())?;
    let data = value.data;
    let offset = (page - 1) * PAGE_SIZE;
    if value.status.0 != 1
        || value.error_code.is_some_and(|v| v.0 != 0)
        || data.total.0 > MAX_ITEMS
        || offset > data.total.0
        || data.page.is_some_and(|v| v.0 != page)
        || data.pagesize.is_some_and(|v| v.0 != PAGE_SIZE)
        || data.songs.len() as u64 != (data.total.0 - offset).min(PAGE_SIZE)
        || data.songs.iter().any(|v| {
            v.base.album_id.0 != album
                || v.base.album_audio_id.0 == 0
                || v.base.album_audio_id.0 > i64::MAX as u64
        })
    {
        return Err(invalid());
    }
    Ok(data)
}

fn map(row: Song) -> Result<ConceptAlbumTrack> {
    if !row.base.is_publish.is_some_and(|v| matches!(v.0, 1 | 3)) {
        return Err(invalid());
    }
    let audio = row.audio_info.ok_or_else(invalid)?;
    if audio.extname != "mp3"
        || audio.hash.len() != 32
        || !audio.hash.bytes().all(|b| b.is_ascii_hexdigit())
        || audio.duration.0 == 0
        || audio.filesize.0 == 0
        || audio.bitrate.0 == 0
    {
        return Err(invalid());
    }
    let author = row.base.author_name.ok_or_else(invalid)?;
    let title = row.base.audio_name.ok_or_else(invalid)?;
    if [&author, &title]
        .iter()
        .any(|v| v.trim().is_empty() || v.chars().any(char::is_control))
    {
        return Err(invalid());
    }
    let name = format!("{author} - {title}.mp3");
    if name.len() > 8192 {
        return Err(invalid());
    }
    Ok(ConceptAlbumTrack {
        name,
        hash: audio.hash.to_ascii_lowercase(),
        size: i32::try_from(audio.filesize.0).map_err(|_| invalid())?,
        timelen: i32::try_from(audio.duration.0).map_err(|_| invalid())?,
        // Concept casts this catalogue bitrate directly to a short, unlike
        // Standard's threshold conversion. Reject overflow instead of wrapping.
        bitrate: i16::try_from(audio.bitrate.0).map_err(|_| invalid())?,
        album_id: row.base.album_id.0.to_string(),
        mixsongid: row.base.album_audio_id.0,
    })
}
fn invalid() -> TuneWeaveError {
    kugou_upstream_error("KuGou Concept album input was incomplete or inconsistent")
}

#[cfg(test)]
mod tests;
