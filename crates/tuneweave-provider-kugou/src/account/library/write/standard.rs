//! Standard 20.8 CloudMusicSetFileRequestor item serialization.
use super::*;

mod add;
mod remove;

#[derive(Serialize)]
pub(super) struct StandardTrackInput {
    number: u8,
    name: String,
    hash: String,
    size: i32,
    sort: u8,
    timelen: i32,
    bitrate: i16,
    album_id: String,
    pub(super) mixsongid: u64,
}

impl StandardTrackInput {
    pub(super) fn from_catalogue(track: &Track, expected: &str) -> Result<Self> {
        let common = CatalogueTrackInput::from_catalogue(track, expected)?;
        let asset = track
            .extensions
            .get("qualities")
            .and_then(|q| q.get("standard"))
            .ok_or_else(malformed)?;
        if asset.get("format").and_then(Value::as_str) != Some("mp3") {
            return Err(malformed());
        }
        let album = track
            .album
            .as_ref()
            .and_then(|a| a.resource_ref.as_ref())
            .ok_or_else(malformed)?;
        // This ordinary catalogue slice requires a known album identity; the
        // legacy mapper's unknown-album 0 sentinel is not established here.
        if album.platform() != Platform::Kugou || !valid_uid(album.id()) {
            return Err(malformed());
        }
        let timelen = checked_int(track.duration_ms)?;
        let size = checked_int(asset.get("size").and_then(Value::as_u64))?;
        let bitrate = wire_bitrate(asset.get("bitrate").and_then(Value::as_u64))?;
        let name = text(Some(format!("{}.mp3", common.name)), 8192)?.ok_or_else(malformed)?;
        // The official selected-playlist source copies KGSong milliseconds into
        // KGMusic and casts to Java int. Check the range instead of wrapping.
        // This wire mapping does not establish a new album_audio_id/mixsongid
        // equivalence for the separate direct track-detail lookup.
        Ok(Self {
            number: 1,
            name,
            hash: common.hash,
            size,
            sort: 0,
            timelen,
            bitrate,
            album_id: album.id().to_owned(),
            mixsongid: common.mixsongid,
        })
    }
}

fn checked_int(value: Option<u64>) -> Result<i32> {
    i32::try_from(value.ok_or_else(malformed)?).map_err(|_| malformed())
}

fn wire_bitrate(value: Option<u64>) -> Result<i16> {
    // cloudtool.e.q: values already fitting short are retained; larger Java
    // int values are divided by 1000. Only a still-too-large result uses 128.
    let raw = checked_int(value)?;
    let normalized = if raw <= i32::from(i16::MAX) {
        raw
    } else {
        let kbps = raw / 1000;
        if kbps <= i32::from(i16::MAX) {
            kbps
        } else {
            128
        }
    };
    i16::try_from(normalized).map_err(|_| malformed())
}

#[cfg(test)]
mod tests;
