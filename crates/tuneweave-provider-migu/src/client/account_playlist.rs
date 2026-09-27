use super::*;
use crate::credential::validate_uid;

pub(crate) const PAGE_SIZE: u32 = 50;
pub(crate) const MAX_TRACKS: u64 = 10_000;
pub(crate) const MAX_PAGES: u32 = MAX_TRACKS as u32 / PAGE_SIZE;

fn invalid() -> TuneWeaveError {
    migu_upstream_error("Migu account playlist response is invalid")
}

fn favorite_identity_unavailable() -> TuneWeaveError {
    TuneWeaveError::new(
        ErrorCode::CapabilityNotSupported,
        "Migu did not provide a stable identity for this account's favorite playlist",
    )
    .with_platform(Platform::Migu)
}

#[cfg(debug_assertions)]
#[derive(Debug)]
struct SafeHomeDiagnostic {
    root_keys: Vec<String>,
    private_items_type: &'static str,
    private_items_count: Option<usize>,
    favorite_navigation_count: Option<usize>,
    favorite_navigation_field_types: Vec<String>,
    favorite_action_url_string_count: Option<usize>,
    favorite_action_url_length: Option<usize>,
    favorite_action_url_controls_present: Option<bool>,
    favorite_action_url_parse_valid: Option<bool>,
    favorite_action_url_scheme: Option<&'static str>,
    favorite_action_url_identity_empty: Option<bool>,
    favorite_action_url_fragment_absent: Option<bool>,
    favorite_music_list_id_count: Option<usize>,
    favorite_music_list_id_canonical: Option<bool>,
    created_lists_type: &'static str,
    created_lists_count: Option<usize>,
    collected_lists_type: &'static str,
    collected_lists_count: Option<usize>,
}

#[cfg(debug_assertions)]
impl std::fmt::Display for SafeHomeDiagnostic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "root_keys={:?} private_items_type={} private_items_count={:?} favorite_navigation_count={:?} favorite_navigation_field_types={:?} favorite_action_url_string_count={:?} favorite_action_url_length={:?} favorite_action_url_controls_present={:?} favorite_action_url_parse_valid={:?} favorite_action_url_scheme={:?} favorite_action_url_identity_empty={:?} favorite_action_url_fragment_absent={:?} favorite_music_list_id_count={:?} favorite_music_list_id_canonical={:?} created_lists_type={} created_lists_count={:?} collected_lists_type={} collected_lists_count={:?}",
            self.root_keys,
            self.private_items_type,
            self.private_items_count,
            self.favorite_navigation_count,
            self.favorite_navigation_field_types,
            self.favorite_action_url_string_count,
            self.favorite_action_url_length,
            self.favorite_action_url_controls_present,
            self.favorite_action_url_parse_valid,
            self.favorite_action_url_scheme,
            self.favorite_action_url_identity_empty,
            self.favorite_action_url_fragment_absent,
            self.favorite_music_list_id_count,
            self.favorite_music_list_id_canonical,
            self.created_lists_type,
            self.created_lists_count,
            self.collected_lists_type,
            self.collected_lists_count,
        )
    }
}

#[cfg(debug_assertions)]
fn json_kind(value: Option<&serde_json::Value>) -> &'static str {
    match value {
        None => "missing",
        Some(serde_json::Value::Null) => "null",
        Some(serde_json::Value::Bool(_)) => "boolean",
        Some(serde_json::Value::Number(_)) => "number",
        Some(serde_json::Value::String(_)) => "string",
        Some(serde_json::Value::Array(_)) => "array",
        Some(serde_json::Value::Object(_)) => "object",
    }
}

#[cfg(debug_assertions)]
fn safe_home_diagnostic(data: &serde_json::Value) -> SafeHomeDiagnostic {
    let mut root_keys = data
        .as_object()
        .into_iter()
        .flat_map(|object| object.keys())
        .filter(|key| {
            !key.is_empty()
                && key.len() <= 64
                && key
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        })
        .take(32)
        .cloned()
        .collect::<Vec<_>>();
    root_keys.sort();

    let private_items = data.get("userPrivateItems");
    let rows = private_items.and_then(serde_json::Value::as_array);
    let favorite_rows = rows.map(|rows| {
        rows.iter()
            .filter(|row| {
                row.get("title")
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|title| title.trim() == "喜欢的音乐")
            })
            .collect::<Vec<_>>()
    });
    let created_lists = data.pointer("/myCreatedMusicLists/createdMusicLists");
    let favorite_action = favorite_rows
        .as_ref()
        .and_then(|rows| rows.first())
        .and_then(|row| row.get("actionUrl"))
        .and_then(serde_json::Value::as_str);
    let parsed_action = favorite_action.and_then(|action| Url::parse(action).ok());
    let music_list_ids = parsed_action.as_ref().map(|url| {
        url.query_pairs()
            .filter(|(key, _)| key == "musicListId")
            .map(|(_, value)| value.into_owned())
            .collect::<Vec<_>>()
    });
    let favorite_navigation_field_types = favorite_rows
        .as_ref()
        .and_then(|rows| rows.first())
        .and_then(|row| row.as_object())
        .map(|object| {
            object
                .iter()
                .filter(|(key, _)| {
                    !key.is_empty()
                        && key.len() <= 64
                        && key
                            .bytes()
                            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
                })
                .take(32)
                .map(|(key, value)| format!("{key}:{}", json_kind(Some(value))))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let collected_lists = data.pointer("/myCollectedMusicLists/collectMusicLists");

    SafeHomeDiagnostic {
        root_keys,
        private_items_type: json_kind(private_items),
        private_items_count: rows.map(|rows| rows.len()),
        favorite_navigation_count: favorite_rows.as_ref().map(Vec::len),
        favorite_navigation_field_types,
        favorite_action_url_string_count: favorite_rows.as_ref().map(|rows| {
            rows.iter()
                .filter(|row| {
                    row.get("actionUrl")
                        .and_then(serde_json::Value::as_str)
                        .is_some()
                })
                .count()
        }),
        favorite_action_url_length: favorite_action.map(str::len),
        favorite_action_url_controls_present: favorite_action
            .map(|action| action.chars().any(char::is_control)),
        favorite_action_url_parse_valid: favorite_action.map(|_| parsed_action.is_some()),
        favorite_action_url_scheme: parsed_action.as_ref().map(|url| match url.scheme() {
            "http" => "http",
            "https" => "https",
            "mgmusic" => "mgmusic",
            _ => "other",
        }),
        favorite_action_url_identity_empty: parsed_action
            .as_ref()
            .map(|url| url.username().is_empty() && url.password().is_none()),
        favorite_action_url_fragment_absent: parsed_action
            .as_ref()
            .map(|url| url.fragment().is_none()),
        favorite_music_list_id_count: music_list_ids.as_ref().map(Vec::len),
        favorite_music_list_id_canonical: music_list_ids.as_ref().and_then(|ids| ids.first()).map(
            |value| music_list_ids.as_ref().is_some_and(|ids| ids.len() == 1) && id(value).is_ok(),
        ),
        created_lists_type: json_kind(created_lists),
        created_lists_count: created_lists
            .and_then(serde_json::Value::as_array)
            .map(|rows| rows.len()),
        collected_lists_type: json_kind(collected_lists),
        collected_lists_count: collected_lists
            .and_then(serde_json::Value::as_array)
            .map(|rows| rows.len()),
    }
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
    let result = favorite_id_inner(&data);
    #[cfg(debug_assertions)]
    if let Err(error) = &result {
        eprintln!(
            "DIAGNOSTIC migu_favorite_identity failure_code={:?} shape={}",
            error.code,
            safe_home_diagnostic(&data)
        );
    }
    result
}

fn favorite_id_inner(data: &serde_json::Value) -> Result<String> {
    let home: Home = serde_json::from_value(data.clone()).map_err(|_| invalid())?;
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
    let Some(action) = entry
        .action_url
        .as_deref()
        .filter(|action| !action.trim().is_empty())
    else {
        return Err(favorite_identity_unavailable());
    };
    favorite_id_from_action(action)
}

pub(crate) fn favorite_id_for_write(data: serde_json::Value) -> Result<Option<String>> {
    let result = favorite_id_for_write_inner(&data);
    #[cfg(debug_assertions)]
    if let Err(error) = &result {
        eprintln!(
            "DIAGNOSTIC migu_favorite_identity failure_code={:?} shape={}",
            error.code,
            safe_home_diagnostic(&data)
        );
    }
    result
}

fn favorite_id_for_write_inner(data: &serde_json::Value) -> Result<Option<String>> {
    let home: Home = serde_json::from_value(data.clone()).map_err(|_| invalid())?;
    if home.user_private_items.len() > 128 {
        return Err(invalid());
    }
    let mut matches = home
        .user_private_items
        .into_iter()
        .filter(|item| item.title.trim() == "喜欢的音乐");
    let Some(entry) = matches.next() else {
        return Ok(None);
    };
    if matches.next().is_some() {
        return Err(invalid());
    }
    let Some(action) = entry
        .action_url
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    else {
        return Ok(None);
    };
    favorite_id_from_action(action).map(Some)
}

fn favorite_id_from_action(action: &str) -> Result<String> {
    if action.len() > 8192 || action.chars().any(char::is_control) {
        return Err(invalid());
    }
    let url = Url::parse(action).map_err(|_| invalid())?;
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
