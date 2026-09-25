//! The legacy Web player's hash-only songinfo metadata resolution.
use super::*;

#[derive(Deserialize)]
struct Metadata {
    hash: String,
    album_audio_id: Option<Number>,
    userid: Option<Number>,
    song_name: Option<String>,
    audio_name: Option<String>,
    timelength: Option<Number>,
}

fn valid_hash(hash: &str) -> bool {
    hash.len() == 32 && hash.bytes().all(|b| b.is_ascii_hexdigit())
}

fn map_metadata(data: Value, hash: &str, uid: &str) -> Result<Option<Track>> {
    let value: Metadata = serde_json::from_value(data).map_err(|_| invalid())?;
    if !valid_hash(&value.hash)
        || !value.hash.eq_ignore_ascii_case(hash)
        || value.userid.is_some_and(|v| v.0.to_string() != uid)
    {
        return Err(invalid());
    }
    // The official consumer also handles songs lacking album_audio_id using
    // album+hash. Keep such rows as raw occurrences, never invent a Track ID.
    let Some(id) = value.album_audio_id.filter(|v| v.0 > 0) else {
        return Ok(None);
    };
    let title = value
        .song_name
        .filter(|v| !v.trim().is_empty())
        .or(value.audio_name)
        .filter(|v| !v.trim().is_empty() && v.len() <= 4096 && !v.chars().any(char::is_control))
        .ok_or_else(invalid)?;
    let reference = ResourceRef::new(Platform::Kugou, id.0.to_string()).map_err(|_| invalid())?;
    let mut track = Track::new(reference, title);
    track.duration_ms = value.timelength.map(|v| v.0).filter(|v| *v > 0);
    track
        .extensions
        .insert("hash".into(), json!(hash.to_ascii_lowercase()));
    track
        .extensions
        .insert("identity_source".into(), json!("legacy_web_hash_songinfo"));
    // This is the current server-resolved catalogue identity, not proof of
    // an original saved album version, a quality, or a playback entitlement.
    Ok(Some(track))
}

impl KugouClient {
    pub(crate) async fn legacy_web_resolve_hash(
        &self,
        session: &WebSession,
        hash: &str,
    ) -> Result<Option<Track>> {
        // Legacy cloud files can carry opaque non-catalogue hashes. Existing
        // occurrence reading remains available without forwarding those IDs.
        if !valid_hash(hash) {
            return Ok(None);
        }
        let token = session.media_token()?;
        let data = self
            .web_song_info(session, &token, BTreeMap::from([("hash", hash.to_owned())]))
            .await?;
        map_metadata(data, hash, &session.user_id)
    }
}

#[cfg(test)]
mod tests;
