//! Fixed public OpenAPI requests, independent of native and Web account sessions.
use super::*;

#[derive(Clone, Copy)]
pub(super) enum Endpoint {
    Album,
    AlbumTracks,
    Artist,
    ArtistCatalog,
    ArtistTracks,
    ArtistAlbums,
    ArtistVideos,
    Charts,
    ChartInfo,
    ChartPeriods,
    ChartTracks,
}
impl Endpoint {
    fn path(self) -> &'static str {
        match self {
            Self::Album => "/kmr/v2/albums",
            Self::AlbumTracks => "/v1/album_audio/lite",
            Self::Artist => "/kmr/v3/author",
            Self::ArtistCatalog => "/ocean/v6/singer/list",
            Self::ArtistTracks => "/openapi/kmr/v2/audio_group/author",
            Self::ArtistAlbums => "/kmr/v1/author/albums",
            Self::ArtistVideos => "/kmr/v1/author/videos",
            Self::Charts => "/ocean/v6/rank/list",
            Self::ChartInfo => "/ocean/v6/rank/info",
            Self::ChartPeriods => "/ocean/v6/rank/vol",
            Self::ChartTracks => "/openapi/kmr/v2/rank/audio",
        }
    }
    fn host(self) -> &'static str {
        if matches!(self, Self::ArtistVideos) {
            "openapicdn.kugou.com"
        } else if matches!(
            self,
            Self::ArtistTracks
                | Self::ArtistCatalog
                | Self::Charts
                | Self::ChartInfo
                | Self::ChartPeriods
                | Self::ChartTracks
        ) {
            "gateway.kugou.com"
        } else {
            "openapi.kugou.com"
        }
    }
    fn is_get(self) -> bool {
        matches!(
            self,
            Self::ArtistTracks
                | Self::ArtistCatalog
                | Self::ArtistVideos
                | Self::Charts
                | Self::ChartInfo
                | Self::ChartPeriods
        )
    }
    fn is_ocean(self) -> bool {
        matches!(
            self,
            Self::ArtistCatalog | Self::Charts | Self::ChartInfo | Self::ChartPeriods
        )
    }
    fn tid(self) -> Option<&'static str> {
        if matches!(self, Self::ArtistVideos) {
            None
        } else if matches!(self, Self::Album | Self::AlbumTracks) {
            Some("255")
        } else if matches!(
            self,
            Self::Charts | Self::ChartInfo | Self::ChartPeriods | Self::ChartTracks
        ) {
            Some("369")
        } else {
            Some("36")
        }
    }
    fn operation(self) -> &'static str {
        match self {
            Self::Album => "album_metadata",
            Self::AlbumTracks => "album_tracks",
            Self::Artist => "artist_metadata",
            Self::ArtistCatalog => "artist_catalogue",
            Self::ArtistTracks => "artist_tracks",
            Self::ArtistAlbums => "artist_albums",
            Self::ArtistVideos => "artist_videos",
            Self::Charts => "chart_catalogue",
            Self::ChartInfo => "chart_metadata",
            Self::ChartPeriods => "chart_periods",
            Self::ChartTracks => "chart_tracks",
        }
    }
}

impl KugouClient {
    pub(super) async fn public_openapi(
        &self,
        endpoint: Endpoint,
        body: &impl Serialize,
        device: &KugouDeviceIdentity,
    ) -> Result<Vec<u8>> {
        let bytes = serde_json::to_vec(body).map_err(|_| {
            TuneWeaveError::new(
                ErrorCode::InternalError,
                "Failed to serialize KuGou public catalogue request",
            )
            .with_platform(Platform::Kugou)
        })?;
        self.public_catalogue_request(endpoint, BTreeMap::new(), bytes, device)
            .await
    }

    pub(super) async fn public_catalogue_get(
        &self,
        endpoint: Endpoint,
        query: BTreeMap<&str, String>,
        device: &KugouDeviceIdentity,
    ) -> Result<Vec<u8>> {
        self.public_catalogue_request(endpoint, query, Vec::new(), device)
            .await
    }

    async fn public_catalogue_request(
        &self,
        endpoint: Endpoint,
        extra_query: BTreeMap<&str, String>,
        bytes: Vec<u8>,
        device: &KugouDeviceIdentity,
    ) -> Result<Vec<u8>> {
        let time = unix_seconds_now().to_string();
        let mut query = BTreeMap::from([
            ("appid", ANDROID_APP_ID.to_string()),
            ("clientver", ANDROID_CLIENT_VERSION.to_string()),
            ("clienttime", time.clone()),
            ("dfid", device.dfid().to_owned()),
            ("mid", device.mid.clone()),
            ("uuid", device.guid.clone()),
            ("userid", "0".to_owned()),
            ("token", String::new()),
        ]);
        for (key, value) in extra_query {
            if query.insert(key, value).is_some() {
                return Err(TuneWeaveError::new(
                    ErrorCode::InternalError,
                    "Conflicting KuGou catalogue parameters",
                )
                .with_platform(Platform::Kugou));
            }
        }
        query.insert(
            "signature",
            crate::signing::android_signature(&query, &bytes),
        );
        let url = format!("https://{}{}", endpoint.host(), endpoint.path());
        #[cfg(test)]
        let url = self
            .login_test_origin
            .as_ref()
            .map(|o| o.join(endpoint.path()).unwrap().to_string())
            .unwrap_or(url);
        let started = Instant::now();
        let mut status = None;
        let result = async {
            let request = if endpoint.is_get() {
                self.http.get(url)
            } else {
                self.http.post(url).body(bytes)
            };
            let request = if let Some(tid) = endpoint.tid() {
                request.header("kg-tid", tid)
            } else {
                request
            };
            let mut response = request
                .query(&query)
                .header("user-agent", ANDROID_USER_AGENT)
                .header(CONTENT_TYPE, "application/json")
                .header("mid", &device.mid)
                .header("dfid", device.dfid())
                .header("clienttime", time)
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
                    "KuGou public catalogue requires additional verification",
                )
                .with_platform(Platform::Kugou));
            }
            let content_type = response
                .headers()
                .get(CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.split(';').next())
                .map(str::trim);
            const LIMIT: usize = 1_048_576;
            if !(matches!(content_type, Some("application/json" | "text/plain"))
                || (endpoint.is_ocean() && content_type == Some("text/html")))
                || response.content_length().is_some_and(|v| v > LIMIT as u64)
            {
                return Err(kugou_upstream_error(
                    "KuGou public catalogue returned an invalid response",
                ));
            }
            let mut bytes = Vec::new();
            while let Some(chunk) = response.chunk().await.map_err(kugou_network_error)? {
                if bytes.len().saturating_add(chunk.len()) > LIMIT {
                    return Err(kugou_upstream_error(
                        "KuGou public catalogue exceeded its response limit",
                    ));
                }
                bytes.extend_from_slice(&chunk);
            }
            // Check the business envelope inside the logged operation, before DTO mapping.
            if endpoint.is_ocean() {
                check_ocean_status(&bytes)?;
            } else {
                check_status(&bytes)?;
            }
            Ok(bytes)
        }
        .await;
        self.log_upstream_request(
            endpoint.operation(),
            endpoint.host(),
            endpoint.path(),
            status,
            started,
            0,
            false,
            &result,
        );
        result
    }
}

#[derive(Deserialize)]
struct Status {
    status: i64,
    error_code: i64,
    errcode: Option<i64>,
}
pub(super) fn check_status(bytes: &[u8]) -> Result<()> {
    let e: Status = serde_json::from_slice(bytes)
        .map_err(|_| kugou_upstream_error("KuGou catalogue returned an invalid envelope"))?;
    if e.status != 1 || e.error_code != 0 || e.errcode.is_some_and(|v| v != 0) {
        let code = if e.error_code != 0 {
            e.error_code
        } else {
            e.errcode.unwrap_or(0)
        };
        return Err(
            kugou_upstream_error("KuGou public catalogue request was rejected")
                .with_details(json!({"platform_code":code})),
        );
    }
    Ok(())
}

#[derive(Deserialize)]
struct OceanStatus {
    status: i64,
    errcode: i64,
}
pub(super) fn check_ocean_status(bytes: &[u8]) -> Result<()> {
    let e: OceanStatus = serde_json::from_slice(bytes)
        .map_err(|_| kugou_upstream_error("KuGou chart returned an invalid envelope"))?;
    if e.status != 1 || e.errcode != 0 {
        return Err(kugou_upstream_error("KuGou chart request was rejected")
            .with_details(json!({"platform_code":e.errcode})));
    }
    Ok(())
}
