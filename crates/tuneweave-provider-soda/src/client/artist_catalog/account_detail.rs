use super::*;
use tuneweave_core::ArtistOverview;

const BACKEND: &str = "official_pc_account_artist_detail";
const PREVIEW_BACKEND: &str = "official_pc_account_artist_preview";

#[derive(Debug)]
pub(crate) struct SodaPcArtistDetail {
    pub overview: ArtistOverview,
    pub credential: Option<SodaCredential>,
}

#[derive(Deserialize)]
struct Detail {
    artist_info: ArtistInfo,
    #[serde(default)]
    hot_tracks: Vec<SodaTrack>,
    #[serde(default)]
    has_more_tracks: bool,
}

#[derive(Deserialize)]
struct ArtistInfo {
    #[serde(flatten)]
    metadata: super::super::artist::SodaArtistMetadata,
    count_albums: Option<u64>,
    state: Option<ArtistState>,
    user: Option<LinkedUser>,
}

#[derive(Deserialize)]
struct ArtistState {
    is_collected: Option<bool>,
    blocked_by_me: Option<bool>,
}

#[derive(Deserialize)]
struct LinkedUser {
    id: String,
    artist_id: Option<String>,
}

impl SodaClient {
    pub(crate) async fn pc_artist_detail(
        &self,
        id: &str,
        source: Option<&SodaCredential>,
    ) -> Result<SodaPcArtistDetail> {
        if canonical_positive_decimal(id) != Some(id) {
            return Err(soda_invalid_request(
                "Soda artist ID must be a canonical positive decimal",
            ));
        }
        if source.is_some_and(|s| s.user_id().is_none()) {
            return Err(authentication_required());
        }
        let response = self
            .artist_catalog_request(id, None, "", MAX_API_RESPONSE_BYTES as usize, source)
            .await?;
        let mut overview = parse(&response.body, id)?;
        let credential = source
            .map(|s| s.with_response_cookies(&response.headers))
            .transpose()?;
        if let Some(source) = source {
            let user_id = source.user_id().ok_or_else(authentication_required)?;
            for extensions in [&mut overview.extensions, &mut overview.artist.extensions] {
                mark(extensions, user_id, BACKEND);
            }
            for track in &mut overview.featured_tracks {
                mark(&mut track.extensions, user_id, PREVIEW_BACKEND);
            }
            let sources = std::iter::once(source)
                .chain(credential.iter())
                .cloned()
                .collect::<Vec<_>>();
            crate::account::reject_secrets(
                &serde_json::to_value(&overview)
                    .map_err(|_| invalid("Soda artist detail metadata could not be encoded"))?,
                &sources,
            )?;
        }
        Ok(SodaPcArtistDetail {
            overview,
            credential,
        })
    }
}

fn mark(extensions: &mut Extensions, user_id: &str, backend: &str) {
    extensions.insert("backend".into(), json!(backend));
    extensions.insert("source_user_id".into(), json!(user_id));
    extensions.insert("authenticated".into(), json!(true));
}

fn parse(body: &[u8], id: &str) -> Result<ArtistOverview> {
    PageState::parse(body)?;
    let response: Detail = serde_json::from_slice(body)
        .map_err(|_| invalid("Soda artist detail omitted valid metadata or preview"))?;
    let mut artist = super::super::artist::map_artist_metadata(
        response.artist_info.metadata,
        "official_pc_artist_detail",
    )?;
    artist.album_count = response.artist_info.count_albums;
    if artist.id != id
        || artist.album_count.is_some_and(|n| n > 1_000_000)
        || response.hot_tracks.len() > 100
        || artist.track_count.is_some_and(|n| {
            n < response.hot_tracks.len() as u64
                || response.has_more_tracks != (n > response.hot_tracks.len() as u64)
        })
    {
        return Err(invalid(
            "Soda artist detail identity or preview completeness is inconsistent",
        ));
    }
    if let Some(user) = response.artist_info.user {
        if canonical_positive_decimal(&user.id) != Some(user.id.as_str())
            || user.artist_id.as_deref().is_some_and(|linked| linked != id)
        {
            return Err(invalid(
                "Soda artist detail returned an inconsistent linked user",
            ));
        }
        artist
            .extensions
            .insert("linked_user_id".into(), json!(user.id));
    }
    if let Some(state) = response.artist_info.state {
        let mut fields = Extensions::new();
        if let Some(value) = state.is_collected {
            fields.insert("is_collected".into(), json!(value));
        }
        if let Some(value) = state.blocked_by_me {
            fields.insert("blocked_by_me".into(), json!(value));
        }
        if !fields.is_empty() {
            artist
                .extensions
                .insert("account_state".into(), json!(fields));
        }
    }
    let mut seen = BTreeSet::new();
    let featured_tracks = response
        .hot_tracks
        .into_iter()
        .map(|source| {
            validate_credits(&source.artists, id)?;
            if !seen.insert(source.id.clone()) {
                return Err(invalid("Soda artist preview repeated a track"));
            }
            map_track(source, "official_pc_artist_preview")
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(ArtistOverview {
        artist,
        featured_tracks,
        has_more_tracks: response.has_more_tracks,
        extensions: Extensions::from([
            ("backend".into(), json!("official_pc_artist_detail")),
            ("preview_scope".into(), json!("hot_tracks")),
        ]),
    })
}

#[cfg(test)]
pub(crate) fn fixture() -> serde_json::Value {
    json!({"status_info":{"now":1,"now_ts_ms":1000},"artist_info":{"id":"123","name":"Artist","count_tracks":3,"count_albums":7,
        "state":{"is_collected":true,"blocked_by_me":false},"user":{"id":"456","artist_id":"123","secret":"ignored-linked-user-secret"},
        "artist_profile":{"alias":["Alias"],"intro":"A biography","nationality":"Region"}},
        "hot_tracks":[{"id":"11","name":"First","duration":1000,"artists":[{"id":"123","name":"Artist"},{"id":"789","name":"Collaborator"}]},
        {"id":"22","name":"Second","duration":2000,"artists":[{"id":"123","name":"Artist"}]}],"has_more_tracks":true,
        "hot_albums":[{"secret":"ignored-hot-album-secret"}]})
}

#[cfg(test)]
mod tests;
