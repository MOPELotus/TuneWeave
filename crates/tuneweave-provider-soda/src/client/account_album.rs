use crate::login::SodaCredential;

use super::*;

pub(crate) const BACKEND: &str = "official_pc_account_album";

pub(crate) struct SodaAccountAlbum {
    pub page: SodaAlbumPage,
    pub credential: SodaCredential,
}

#[derive(Deserialize)]
struct Business {
    status_code: Option<i64>,
    status_info: Option<BusinessInfo>,
}

#[derive(Deserialize)]
struct BusinessInfo {
    status_code: Option<i64>,
}

#[derive(Deserialize)]
struct Envelope {
    status_info: Status,
    album_info: AlbumInfo,
    tracks: Vec<SodaTrack>,
    #[serde(default)]
    has_more: bool,
}

#[derive(Deserialize)]
struct Status {
    now: u64,
    now_ts_ms: u64,
}

#[derive(Deserialize)]
struct AlbumInfo {
    count_tracks: u64,
    #[serde(flatten)]
    metadata: SodaAlbumMetadata,
}

impl SodaClient {
    pub(crate) async fn account_album(
        &self,
        id: &str,
        credential: &SodaCredential,
    ) -> Result<SodaAccountAlbum> {
        if canonical_positive_decimal(id) != Some(id) {
            return Err(soda_invalid_request(
                "Soda album ID must be a canonical positive decimal",
            ));
        }
        if credential.user_id().is_none() {
            return Err(authentication_required());
        }
        let started = Instant::now();
        let mut status = None;
        let result = async {
            let device = self.login_device()?;
            let mut url = Url::parse("https://api.qishui.com")
                .map_err(|_| invalid("Soda album endpoint is invalid"))?;
            url.set_path(&format!("/luna/pc/albums/{id}"));
            url.query_pairs_mut()
                .append_pair("ignore_tracks", "false")
                .append_pair("aid", SODA_APP_ID)
                .append_pair("app_name", "luna_pc")
                .append_pair("device_platform", "windows")
                .append_pair("channel", "official")
                .append_pair("version_name", "2.1.0")
                .append_pair("version_code", "20010000")
                .append_pair("device_id", &device.device_id)
                .append_pair("iid", &device.install_id);
            let response = self
                .send_login_request(
                    self.login_request(reqwest::Method::GET, url)
                        .header(reqwest::header::ACCEPT, "application/json")
                        .header(reqwest::header::COOKIE, credential.cookie_header()?),
                )
                .await?;
            status = Some(response.status());
            if response.status() == StatusCode::UNAUTHORIZED {
                return Err(authentication_required());
            }
            if response.status() != StatusCode::OK {
                return Err(soda_http_error(response.status()));
            }
            if !response
                .headers()
                .get(CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .is_some_and(|v| {
                    v.split(';')
                        .next()
                        .unwrap_or_default()
                        .trim()
                        .eq_ignore_ascii_case("application/json")
                })
            {
                return Err(invalid(
                    "Soda account album returned an unexpected content type",
                ));
            }
            let headers = response.headers().clone();
            let bytes = read_bounded_response(response, "Soda account album").await?;
            let page = parse(&bytes, id)?;
            let credential = credential.with_response_cookies(&headers)?;
            Ok(SodaAccountAlbum { page, credential })
        }
        .await;
        self.log_upstream_request(
            "account_album",
            "api.qishui.com",
            "/luna/pc/albums/{album_id}",
            status,
            started,
            &result,
        );
        result
    }
}

fn parse(body: &[u8], id: &str) -> Result<SodaAlbumPage> {
    let business: Business = serde_json::from_slice(body)
        .map_err(|_| invalid("Soda account album returned malformed data"))?;
    let codes = [
        business.status_code,
        business.status_info.and_then(|s| s.status_code),
    ];
    if codes.contains(&Some(1_000_016)) {
        return Err(authentication_required());
    }
    if codes.into_iter().flatten().any(|n| n != 0) {
        return Err(invalid("Soda account album request was rejected"));
    }
    let mut response: Envelope = serde_json::from_slice(body)
        .map_err(|_| invalid("Soda account album omitted complete metadata"))?;
    if response.status_info.now == 0
        || response.status_info.now_ts_ms / 1000 != response.status_info.now
        || response.has_more
        || response.tracks.len() > 10_000
        || response.album_info.metadata.has_error
    {
        return Err(invalid(
            "Soda account album returned invalid status or incomplete tracks",
        ));
    }
    response.album_info.metadata.count_tracks = response.album_info.count_tracks;
    let mut album = map_album_metadata(
        &response.album_info.metadata,
        id,
        response.tracks.len() as u64,
    )?;
    album
        .extensions
        .insert("backend".to_owned(), json!(BACKEND));
    let tracks = response
        .tracks
        .into_iter()
        .enumerate()
        .map(|(position, mut source)| {
            if source.album.id != id
                || (!source.media_type.is_empty() && source.media_type != "track")
            {
                return Err(invalid(
                    "Soda account album returned a different album or media type",
                ));
            }
            if source.album.name.is_empty() {
                source.album.name = album.name.clone();
            }
            let mut track = map_track(source, BACKEND)?;
            track
                .extensions
                .insert("album_position".to_owned(), json!(position));
            Ok(track)
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(SodaAlbumPage { album, tracks })
}

fn invalid(message: &'static str) -> TuneWeaveError {
    soda_upstream_error(message)
}
fn authentication_required() -> TuneWeaveError {
    TuneWeaveError::new(
        ErrorCode::AuthenticationRequired,
        "Soda album session is not authenticated",
    )
    .with_platform(Platform::Soda)
}

#[cfg(test)]
mod tests;

#[cfg(test)]
pub(crate) fn test_fixture() -> serde_json::Value {
    let tracks = ["11", "22", "22"].into_iter().map(|id|json!({"id":id,"name":format!("Song {id}"),"duration":1000,
        "album":{"id":"900","name":"Album"},"artists":[{"id":"456","name":"Artist"}],"media_type":"track",
        "video_model":"private-player-material","url_player_info":"https://example.invalid/private-key"})).collect::<Vec<_>>();
    json!({"status_info":{"now":1,"now_ts_ms":1000},
        "album_info":{"id":"900","name":"Album","count_tracks":3,"artists":[{"id":"456","name":"Artist"}],"state":{"is_collected":true}},
        "tracks":tracks})
}
