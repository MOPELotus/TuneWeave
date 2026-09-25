use super::*;
use crate::credential::validate_uid;

pub(crate) const PAGE_SIZE: u32 = 50;
pub(crate) const MAX_TRACKS: u64 = 10_000;
pub(crate) const MAX_PAGES: u32 = MAX_TRACKS as u32 / PAGE_SIZE;

fn invalid() -> TuneWeaveError {
    migu_upstream_error("Migu account playlist response is invalid")
}
fn id(value: &str) -> Result<&str> {
    let parsed = value.parse::<u64>().map_err(|_| invalid())?;
    if parsed == 0 || parsed.to_string() != value {
        return Err(invalid());
    }
    Ok(value)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Home {
    user_private_items: Vec<Navigation>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Navigation {
    title: String,
    action_url: Option<String>,
}

pub(crate) fn favorite_id(data: serde_json::Value) -> Result<String> {
    let home: Home = serde_json::from_value(data).map_err(|_| invalid())?;
    if home.user_private_items.len() > 128 {
        return Err(invalid());
    }
    let mut matches = home
        .user_private_items
        .into_iter()
        .filter(|item| item.title.trim() == "喜欢的音乐");
    let entry = matches.next().ok_or_else(|| {
        TuneWeaveError::new(
            ErrorCode::ResourceNotFound,
            "Migu account did not expose its favorite playlist",
        )
        .with_platform(Platform::Migu)
    })?;
    if matches.next().is_some() {
        return Err(invalid());
    }
    let action = entry.action_url.ok_or_else(invalid)?;
    if action.len() > 8192 || action.chars().any(char::is_control) {
        return Err(invalid());
    }
    let url = Url::parse(&action).map_err(|_| invalid())?;
    if !matches!(url.scheme(), "http" | "https" | "mgmusic")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return Err(invalid());
    }
    // This is an identifier carrier from the official private navigation. Never
    // navigate to it, authorize its host, forward its query, or export the URL.
    let mut ids = url
        .query_pairs()
        .filter(|(key, _)| key == "musicListId")
        .map(|(_, value)| value.into_owned());
    let value = ids.next().ok_or_else(invalid)?;
    if ids.next().is_some() {
        return Err(invalid());
    }
    id(&value)?;
    Ok(value)
}

pub(crate) fn detail(data: serde_json::Value, requested: &str) -> Result<Playlist> {
    let raw: MiguPlaylistInfo = serde_json::from_value(data.clone()).map_err(|_| invalid())?;
    let total = raw
        .music_num
        .as_ref()
        .and_then(FlexibleU64::get)
        .ok_or_else(invalid)?;
    if total > MAX_TRACKS
        || raw.tags.len() > 128
        || raw.summary.len() > 4000
        || raw.title.len() > 2048
        || raw.owner_name.len() > 2048
    {
        return Err(invalid());
    }
    if !raw.owner_id.is_empty() {
        validate_uid(&raw.owner_id).map_err(|_| invalid())?;
    }
    let bytes = serde_json::to_vec(&json!({"code":"000000","data":data})).map_err(|_| invalid())?;
    let mut playlist = parse_playlist_detail_response(&bytes, requested)?;
    // Account metadata is also used to confirm edits. Preserve legitimate
    // multiline descriptions instead of the public mapper's single-line form.
    playlist.description = raw
        .summary
        .chars()
        .filter(|c| !c.is_control() || matches!(c, '\n' | '\r' | '\t'))
        .collect();
    // A user is not an artist, and account navigation/promotion URLs are not data.
    if let Some(creator) = &mut playlist.creator {
        creator.resource_ref = None;
    }
    playlist.extensions.retain(|key, _| {
        matches!(
            key.as_str(),
            "resource_type"
                | "owner_id"
                | "owner_name"
                | "playlist_type"
                | "status"
                | "tag_items"
                | "statistics"
                | "have_private_picture"
        )
    });
    playlist
        .extensions
        .insert("backend".into(), json!("official_pc_account_playlist"));
    Ok(playlist)
}

pub(crate) struct TrackPage {
    pub page: MiguPlaylistTrackPage,
    pub owner: Option<String>,
    pub order_fields: Vec<Option<super::playlist_order::NativePlaylistSong>>,
}
pub(crate) fn tracks(data: serde_json::Value, requested: &str, page: u32) -> Result<TrackPage> {
    for key in ["musicListId", "playlistId"] {
        if let Some(value) = data.get(key) {
            if value.as_str() != Some(requested) {
                return Err(invalid());
            }
        }
    }
    // The public mapper accepts a missing songList as empty. Private reads must
    // distinguish an explicitly empty collection from an incomplete response.
    if !data
        .get("songList")
        .is_some_and(serde_json::Value::is_array)
    {
        return Err(invalid());
    }
    let owner = data
        .get("ownerId")
        .map(|v| {
            let value = v.as_str().ok_or_else(invalid)?;
            validate_uid(value).map_err(|_| invalid())?;
            Ok::<_, TuneWeaveError>(value.to_owned())
        })
        .transpose()?;
    let order_fields = data["songList"]
        .as_array()
        .ok_or_else(invalid)?
        .iter()
        .map(super::playlist_order::order_song)
        .collect();
    let bytes = serde_json::to_vec(&json!({"code":"000000","data":data})).map_err(|_| invalid())?;
    let parsed = parse_playlist_tracks_response(&bytes, page, PAGE_SIZE)?;
    if parsed.total > MAX_TRACKS {
        return Err(invalid());
    }
    Ok(TrackPage {
        page: parsed,
        owner,
        order_fields,
    })
}

#[cfg(test)]
pub(crate) mod tests;
