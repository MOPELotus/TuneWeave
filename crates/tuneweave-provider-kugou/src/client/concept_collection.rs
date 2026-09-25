//! Concept 5.2.9 public collection input; this module never authorizes or writes.
use super::dto::Number;
use super::*;
use crate::signing::concept_signature;

const METADATA_PATH: &str = "/v1/get_list_info";
const SONGS_PATH: &str = "/pubsongs/v2/get_other_list_file_nofilt";
const CLIENT_VERSION: &str = "11590";
const PAGE_SIZE: usize = 100;
const MAX_ITEMS: usize = 10_000;
const RESPONSE_LIMIT: usize = 1024 * 1024;
const TOTAL_LIMIT: usize = 16 * RESPONSE_LIMIT;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct SourceIdentity {
    global_collection_id: String,
    creator_id: u64,
    creator_list_id: u64,
    special_id: u64,
    source: u64,
}

#[derive(Debug, Serialize)]
pub(crate) struct CollectionSyncPlan {
    source: SourceIdentity,
    /// The observed source version, not a transaction or authorization token.
    source_version: u64,
    occurrence_count: usize,
    /// Unmodified first rows, with canonical identity separately checked.
    songs: Vec<SourceSong>,
}

#[derive(Debug, Deserialize, Serialize)]
struct SourceSong {
    mixsongid: Number,
    hash: String,
    album_id: Number,
    #[serde(flatten)]
    fields: BTreeMap<String, Value>,
}

#[derive(Deserialize)]
struct Envelope<T> {
    status: Number,
    error_code: Option<Number>,
    data: T,
}

#[derive(Deserialize)]
struct Metadata {
    global_collection_id: String,
    list_create_userid: Number,
    list_create_listid: Number,
    specialid: Number,
    source: Number,
    is_publish: Number,
    count: Number,
}

#[derive(Deserialize)]
struct CatalogPage {
    count: Number,
    list_ver: Number,
    // The active response parser distinguishes these from list_info identities.
    // Retain and compare them across pages; do not equate listid with specialid.
    userid: Number,
    listid: Number,
    #[serde(rename = "page")]
    _page: Option<Number>,
    pagesize: Option<Number>,
    list_info: Metadata,
    info: Vec<SourceSong>,
}

fn malformed() -> TuneWeaveError {
    kugou_upstream_error("KuGou Concept collection input is incomplete or inconsistent")
}

fn changed() -> TuneWeaveError {
    TuneWeaveError::new(
        ErrorCode::Conflict,
        "KuGou Concept collection identity or contents changed while reading",
    )
    .with_platform(Platform::Kugou)
}

fn parse<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Result<T> {
    let envelope: Envelope<T> = serde_json::from_slice(bytes).map_err(|_| malformed())?;
    if envelope.status.0 != 1 || envelope.error_code.is_some_and(|n| n.0 != 0) {
        return Err(malformed());
    }
    Ok(envelope.data)
}

impl Metadata {
    fn identity(&self, expected: &str) -> Result<SourceIdentity> {
        if self.global_collection_id != expected
            || validate_collection_id(&self.global_collection_id).is_err()
            || self.list_create_userid.0 == 0
            || self.list_create_userid.0 > i64::MAX as u64
            || self.list_create_listid.0 == 0
            || self.list_create_listid.0 > i32::MAX as u64
            || self.specialid.0 == 0
            || self.specialid.0 > i32::MAX as u64
            || self.source.0 != 1
            || self.is_publish.0 != 1
            || self.count.0 > MAX_ITEMS as u64
        {
            return Err(malformed());
        }
        Ok(SourceIdentity {
            global_collection_id: self.global_collection_id.clone(),
            creator_id: self.list_create_userid.0,
            creator_list_id: self.list_create_listid.0,
            special_id: self.specialid.0,
            source: self.source.0,
        })
    }
}

struct Preparation {
    source: SourceIdentity,
    total: usize,
    offset: usize,
    version: Option<(u64, u64, u64)>,
    identities: BTreeMap<u64, (String, u64)>,
    songs: Vec<SourceSong>,
    pages: BTreeSet<Vec<(u64, String, u64)>>,
    used: usize,
}

impl Preparation {
    fn new(bytes: &[u8], expected: &str) -> Result<Self> {
        let mut rows: Vec<Metadata> = parse(bytes)?;
        if rows.len() != 1 || bytes.len() > RESPONSE_LIMIT {
            return Err(malformed());
        }
        let row = rows.pop().ok_or_else(malformed)?;
        Ok(Self {
            source: row.identity(expected)?,
            total: row.count.0 as usize,
            offset: 0,
            version: None,
            identities: BTreeMap::new(),
            songs: Vec::new(),
            pages: BTreeSet::new(),
            used: bytes.len(),
        })
    }

    fn push(&mut self, bytes: &[u8], requested_offset: usize) -> Result<bool> {
        self.used = self.used.checked_add(bytes.len()).ok_or_else(malformed)?;
        if bytes.len() > RESPONSE_LIMIT
            || self.used > TOTAL_LIMIT
            || requested_offset != self.offset
            || self.offset > self.total
        {
            return Err(malformed());
        }
        let page: CatalogPage = parse(bytes)?;
        if page.list_info.identity(&self.source.global_collection_id)? != self.source
            || page.count.0 != self.total as u64
            || page.list_info.count.0 != self.total as u64
        {
            return Err(changed());
        }
        let version = (page.userid.0, page.listid.0, page.list_ver.0);
        if page.userid.0 != self.source.creator_id
            || page.listid.0 == 0
            || page.list_ver.0 == 0
            || self.version.is_some_and(|old| old != version)
        {
            return Err(changed());
        }
        if page.pagesize.is_some_and(|n| n.0 != PAGE_SIZE as u64)
            || page.info.len() != (self.total - self.offset).min(PAGE_SIZE)
        {
            return Err(malformed());
        }
        self.version = Some(version);
        let count = page.info.len();
        let fingerprint = page
            .info
            .iter()
            .map(|song| {
                (
                    song.mixsongid.0,
                    song.hash.to_ascii_lowercase(),
                    song.album_id.0,
                )
            })
            .collect();
        // No verified cursor echo exists. An identical nonempty page cannot
        // prove progress, even if an actual playlist could contain that pattern.
        if count != 0 && !self.pages.insert(fingerprint) {
            return Err(changed());
        }
        for song in page.info {
            if song.mixsongid.0 == 0
                || song.mixsongid.0 > i64::MAX as u64
                || song.album_id.0 == 0
                || song.album_id.0 > i32::MAX as u64
                || song.hash.len() != 32
                || !song.hash.bytes().all(|b| b.is_ascii_hexdigit())
            {
                return Err(malformed());
            }
            let identity = (song.hash.to_ascii_lowercase(), song.album_id.0);
            if let Some(previous) = self.identities.get(&song.mixsongid.0) {
                if previous != &identity {
                    return Err(changed());
                }
            } else {
                self.identities.insert(song.mixsongid.0, identity);
                self.songs.push(song);
            }
        }
        self.offset += count;
        Ok(self.offset == self.total)
    }

    fn finish(self) -> Result<CollectionSyncPlan> {
        if self.offset != self.total {
            return Err(malformed());
        }
        Ok(CollectionSyncPlan {
            source: self.source,
            source_version: self.version.ok_or_else(malformed)?.2,
            occurrence_count: self.offset,
            songs: self.songs,
        })
    }
}

impl KugouClient {
    pub(crate) async fn concept_collection_plan(
        &self,
        global_id: &str,
        device: &KugouDeviceIdentity,
        collector_id: &str,
        mut observe: impl FnMut() -> Result<()> + Send,
    ) -> Result<CollectionSyncPlan> {
        validate_collection_id(global_id)?;
        if !device.valid() || !crate::credential::valid_uid(collector_id) {
            return Err(malformed());
        }
        let result = tokio::time::timeout(Duration::from_secs(45), async {
            observe()?;
            let hint = self.collection_hint(global_id, device).await;
            observe()?;
            let (hint, hint_bytes) = hint?;
            let expected = hint.identity(global_id)?;
            if expected.creator_id.to_string() == collector_id {
                return Err(TuneWeaveError::new(
                    ErrorCode::PermissionDenied,
                    "KuGou cannot subscribe to the selected account's own playlist",
                )
                .with_platform(Platform::Kugou));
            }
            observe()?;
            let response = self.concept_collection_metadata(&expected, device).await;
            observe()?;
            let mut preparation = Preparation::new(&response?, global_id)?;
            if preparation.source != expected || preparation.total as u64 != hint.count.0 {
                return Err(changed());
            }
            preparation.used = preparation
                .used
                .checked_add(hint_bytes)
                .ok_or_else(malformed)?;
            // Even an empty metadata result must be confirmed by a song page.
            for _ in 0..(MAX_ITEMS / PAGE_SIZE).max(1) {
                observe()?;
                let bytes = self
                    .concept_collection_page(&preparation.source, preparation.offset, device)
                    .await;
                observe()?;
                if preparation.push(&bytes?, preparation.offset)? {
                    return preparation.finish();
                }
            }
            Err(malformed())
        })
        .await
        .unwrap_or_else(|_| {
            Err(kugou_upstream_error(
                "KuGou collection preparation timed out",
            ))
        });
        observe()?;
        result
    }

    async fn concept_collection_metadata(
        &self,
        source: &SourceIdentity,
        device: &KugouDeviceIdentity,
    ) -> Result<Vec<u8>> {
        let body = serde_json::to_vec(&json!({"data":[{
            "userid":source.creator_id,"specialid":source.special_id,
            "global_collection_id":source.global_collection_id
        }]}))
        .map_err(|_| malformed())?;
        let mut query = BTreeMap::from([
            ("appid", "3116".into()),
            ("clientver", CLIENT_VERSION.into()),
            ("clienttime", unix_seconds_now().to_string()),
            ("mid", device.mid.clone()),
            ("dfid", device.dfid().to_owned()),
            ("uuid", "-".into()),
        ]);
        query.insert("signature", concept_signature(&query, &body));
        let url = self.collection_url("pubsongs.kugou.com", METADATA_PATH);
        let request = self
            .http
            .post(url)
            .query(&query)
            .header(CONTENT_TYPE, "application/json;charset=utf-8")
            .body(body);
        self.read_collection_request(request, "pubsongs.kugou.com", METADATA_PATH)
            .await
    }

    async fn collection_hint(
        &self,
        global_id: &str,
        device: &KugouDeviceIdentity,
    ) -> Result<(Metadata, usize)> {
        // Reuse the public detail protocol solely to discover source identities.
        // Passing the existing device avoids registration and credential refresh.
        let endpoint = AndroidEndpoint::PlaylistDetail;
        let started = Instant::now();
        let mut status = None;
        let result = async {
            let response = self
                .post_android(
                    endpoint,
                    &PlaylistDetailRequest {
                        data: [PlaylistCollectionIdentity {
                            global_collection_id: global_id,
                        }],
                        userid: 0,
                        token: "",
                    },
                    device,
                )
                .await?;
            status = Some(response.status());
            let bytes = collection_response(response).await?;
            let hint = parse_playlist_detail_response(&bytes, global_id)?;
            Ok((metadata_from_hint(hint)?, bytes.len()))
        }
        .await;
        self.log_android_request(endpoint, status, started, &result);
        result
    }

    async fn concept_collection_page(
        &self,
        source: &SourceIdentity,
        offset: usize,
        device: &KugouDeviceIdentity,
    ) -> Result<Vec<u8>> {
        let mut query = BTreeMap::from([
            ("appid", "3116".into()),
            ("clientver", CLIENT_VERSION.into()),
            ("area_code", "1".into()),
            ("module", "CloudMusic".into()),
            ("type", "0".into()),
            ("need_sort", "1".into()),
            ("need_rd", "0".into()),
            ("userid", source.creator_id.to_string()),
            ("global_collection_id", source.global_collection_id.clone()),
            ("specialid", source.special_id.to_string()),
            ("begin_idx", offset.to_string()),
            ("pagesize", PAGE_SIZE.to_string()),
            ("mode", "1".into()),
        ]);
        query.insert("signature", concept_signature(&query, &[]));
        // ParamGenerator.T's documented fallback uses these ordinary headers;
        // there is no account token or attempted NativeParams/JNI emulation.
        let request = self
            .http
            .get(self.collection_url("gateway.kugou.com", SONGS_PATH))
            .query(&query)
            .header("mid", &device.mid)
            .header("dfid", device.dfid())
            .header("clienttime", unix_seconds_now().to_string());
        self.read_collection_request(request, "gateway.kugou.com", SONGS_PATH)
            .await
    }

    fn collection_url(&self, host: &str, path: &str) -> String {
        #[cfg(test)]
        if let Some(origin) = &self.login_test_origin {
            return origin.join(path).unwrap().to_string();
        }
        format!("https://{host}{path}")
    }

    async fn read_collection_request(
        &self,
        request: reqwest::RequestBuilder,
        host: &'static str,
        path: &'static str,
    ) -> Result<Vec<u8>> {
        let started = Instant::now();
        let mut status = None;
        let result = async {
            let response = request.send().await.map_err(kugou_network_error)?;
            status = Some(response.status());
            collection_response(response).await
        }
        .await;
        self.log_upstream_request(
            "concept_collection_prepare",
            host,
            path,
            status,
            started,
            0,
            false,
            &result,
        );
        result
    }
}

fn metadata_from_hint(hint: Playlist) -> Result<Metadata> {
    let creator = hint
        .creator
        .as_ref()
        .and_then(|v| v.resource_ref.as_ref())
        .filter(|v| v.platform() == Platform::Kugou)
        .ok_or_else(malformed)?;
    let creator_id = creator.id().parse::<u64>().map_err(|_| malformed())?;
    if creator_id.to_string() != creator.id() {
        return Err(malformed());
    }
    let number = |key| {
        hint.extensions
            .get(key)
            .and_then(Value::as_u64)
            .map(Number)
            .ok_or_else(malformed)
    };
    Ok(Metadata {
        global_collection_id: hint.id.clone(),
        list_create_userid: Number(creator_id),
        list_create_listid: number("list_create_id")?,
        specialid: number("special_id")?,
        source: number("source")?,
        is_publish: number("published")?,
        count: Number(hint.track_count.ok_or_else(malformed)?),
    })
}

async fn collection_response(mut response: reqwest::Response) -> Result<Vec<u8>> {
    if !response.status().is_success() {
        return Err(kugou_http_error(response.status()));
    }
    if response.headers().contains_key("ssa-code") {
        return Err(TuneWeaveError::new(
            ErrorCode::PermissionDenied,
            "KuGou collection input requires additional verification",
        )
        .with_platform(Platform::Kugou));
    }
    if response
        .content_length()
        .is_some_and(|n| n > RESPONSE_LIMIT as u64)
    {
        return Err(malformed());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(kugou_network_error)? {
        if bytes.len().saturating_add(chunk.len()) > RESPONSE_LIMIT {
            return Err(malformed());
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests;
