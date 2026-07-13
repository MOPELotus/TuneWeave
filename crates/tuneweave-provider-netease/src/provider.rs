use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    time::{SystemTime, UNIX_EPOCH},
};

use async_trait::async_trait;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use tuneweave_core::{
    AlbumSummary, ArtistSummary, Capability, ErrorCode, Extensions, LyricContributor, Lyrics,
    MediaStream, MusicProvider, Page, PageMeta, PageRequest, ParseResourceRefError, Platform,
    Playlist, Quality, ResourceRef, Result, SearchKind, SearchQuery, StreamRequest, Track,
    TrialWindow, TuneWeaveError,
};

use crate::{
    NeteaseClient, NeteaseConfig,
    dto::{
        AudioQuality, LyricText, LyricUser, LyricsEnvelope, PlaylistDetail, PlaylistEnvelope,
        Privilege, SearchEnvelope, Song, StreamData, StreamEnvelope, TrackEnvelope,
    },
};

#[derive(Clone)]
pub struct NeteaseProvider {
    client: NeteaseClient,
}

impl NeteaseProvider {
    pub fn new(config: NeteaseConfig) -> Result<Self> {
        Ok(Self {
            client: NeteaseClient::new(config)?,
        })
    }

    #[must_use]
    pub fn from_client(client: NeteaseClient) -> Self {
        Self { client }
    }

    async fn playlist_detail(&self, id: u64) -> Result<PlaylistDetail> {
        let response = self
            .client
            .request_eapi(
                "/api/v6/playlist/detail",
                json!({
                    "id": id,
                    "n": 100_000,
                    "s": 8
                }),
            )
            .await?;
        ensure_success(&response.body)?;
        let response: PlaylistEnvelope = parse_body(response.body)?;
        response.playlist.ok_or_else(|| {
            TuneWeaveError::new(
                ErrorCode::ResourceNotFound,
                "NetEase playlist was not found",
            )
            .with_platform(Platform::Netease)
            .with_details(json!({ "id": id }))
        })
    }
}

#[async_trait]
impl MusicProvider for NeteaseProvider {
    fn platform(&self) -> Platform {
        Platform::Netease
    }

    fn name(&self) -> &'static str {
        "NetEase Cloud Music"
    }

    fn capabilities(&self) -> BTreeSet<Capability> {
        BTreeSet::from([
            Capability::SearchTracks,
            Capability::TrackDetail,
            Capability::PlaylistRead,
            Capability::Lyrics,
            Capability::AudioStream,
        ])
    }

    async fn search(&self, query: &SearchQuery) -> Result<Page<Track>> {
        if query.kind != SearchKind::Track {
            return Err(TuneWeaveError::unsupported(
                Platform::Netease,
                capability_for_search(query.kind),
            ));
        }
        let keyword = query.query.trim();
        if keyword.is_empty() {
            return Err(TuneWeaveError::invalid_request(
                "search query cannot be empty",
            ));
        }
        let limit = query.limit.clamp(1, 100);
        let response = self
            .client
            .request_eapi(
                "/api/search/get",
                json!({
                    "s": keyword,
                    "type": 1,
                    "limit": limit,
                    "offset": query.offset
                }),
            )
            .await?;
        ensure_success(&response.body)?;
        let response: SearchEnvelope = parse_body(response.body)?;
        let count = response.result.songs.len() as u32;
        let next_offset = query.offset.saturating_add(count);
        let has_more = u64::from(next_offset) < response.result.song_count;
        let items = response
            .result
            .songs
            .into_iter()
            .map(|song| map_song(song, None))
            .collect::<Result<Vec<_>>>()?;

        Ok(Page {
            items,
            pagination: PageMeta {
                limit,
                offset: query.offset,
                total: Some(response.result.song_count),
                next_offset: has_more.then_some(next_offset),
                has_more,
            },
        })
    }

    async fn track(&self, id: &str, _account: Option<&str>) -> Result<Track> {
        let id = parse_numeric_id("track", id)?;
        let response = self
            .client
            .request_eapi(
                "/api/v3/song/detail",
                json!({
                    "c": format!(r#"[{{"id":{id}}}]"#)
                }),
            )
            .await?;
        ensure_success(&response.body)?;
        let response: TrackEnvelope = parse_body(response.body)?;
        let mut privileges = response
            .privileges
            .into_iter()
            .map(|privilege| (privilege.id, privilege))
            .collect::<HashMap<_, _>>();
        let song = response.songs.into_iter().next().ok_or_else(|| {
            TuneWeaveError::new(ErrorCode::ResourceNotFound, "NetEase track was not found")
                .with_platform(Platform::Netease)
                .with_details(json!({ "id": id }))
        })?;
        let privilege = privileges.remove(&song.id);
        map_song(song, privilege)
    }

    async fn playlist(&self, id: &str, _account: Option<&str>) -> Result<Playlist> {
        let id = parse_numeric_id("playlist", id)?;
        map_playlist(self.playlist_detail(id).await?)
    }

    async fn playlist_tracks(&self, id: &str, request: &PageRequest) -> Result<Page<Track>> {
        let id = parse_numeric_id("playlist", id)?;
        let playlist = self.playlist_detail(id).await?;
        let total = playlist.track_ids.len() as u64;
        let limit = request.limit.clamp(1, 100);
        let offset = request.offset;
        let selected_ids = playlist
            .track_ids
            .into_iter()
            .skip(offset as usize)
            .take(limit as usize)
            .map(|track| track.id)
            .collect::<Vec<_>>();
        let items = if selected_ids.is_empty() {
            Vec::new()
        } else {
            let request_tracks =
                Value::Array(selected_ids.iter().map(|id| json!({ "id": id })).collect())
                    .to_string();
            let response = self
                .client
                .request_eapi("/api/v3/song/detail", json!({ "c": request_tracks }))
                .await?;
            ensure_success(&response.body)?;
            let response: TrackEnvelope = parse_body(response.body)?;
            let mut songs = response
                .songs
                .into_iter()
                .map(|song| (song.id, song))
                .collect::<HashMap<_, _>>();
            let mut privileges = response
                .privileges
                .into_iter()
                .map(|privilege| (privilege.id, privilege))
                .collect::<HashMap<_, _>>();
            selected_ids
                .iter()
                .filter_map(|id| {
                    songs
                        .remove(id)
                        .map(|song| map_song(song, privileges.remove(id)))
                })
                .collect::<Result<Vec<_>>>()?
        };
        let consumed = selected_ids.len() as u32;
        let next_offset = offset.saturating_add(consumed);
        let has_more = u64::from(next_offset) < total;

        Ok(Page {
            items,
            pagination: PageMeta {
                limit,
                offset,
                total: Some(total),
                next_offset: has_more.then_some(next_offset),
                has_more,
            },
        })
    }

    async fn lyrics(&self, id: &str, _account: Option<&str>) -> Result<Lyrics> {
        let id = parse_numeric_id("track", id)?;
        let response = self
            .client
            .request_eapi(
                "/api/song/lyric/v1",
                json!({
                    "id": id,
                    "cp": false,
                    "tv": 0,
                    "lv": 0,
                    "rv": 0,
                    "kv": 0,
                    "yv": 0,
                    "ytv": 0,
                    "yrv": 0
                }),
            )
            .await?;
        ensure_success(&response.body)?;
        let response: LyricsEnvelope = parse_body(response.body)?;
        map_lyrics(id, response)
    }

    async fn stream(&self, track: &Track, request: &StreamRequest) -> Result<MediaStream> {
        if track.platform != Platform::Netease
            || track.resource_ref.platform() != Platform::Netease
            || track.resource_ref.id() != track.id
        {
            return Err(TuneWeaveError::invalid_request(
                "NetEase provider can only resolve NetEase tracks",
            )
            .with_platform(Platform::Netease)
            .with_details(json!({ "track_ref": track.resource_ref })));
        }
        let id = parse_numeric_id("track", &track.id)?;
        let response = self
            .client
            .request_eapi(
                "/api/song/enhance/player/url",
                json!({
                    "ids": Value::Array(vec![json!(id.to_string())]).to_string(),
                    "br": requested_bitrate(request.quality)
                }),
            )
            .await?;
        ensure_success(&response.body)?;
        let response: StreamEnvelope = parse_body(response.body)?;
        let stream = response
            .data
            .into_iter()
            .find(|stream| stream.id == id)
            .ok_or_else(|| {
                TuneWeaveError::new(
                    ErrorCode::UpstreamError,
                    "NetEase omitted the requested stream result",
                )
                .with_platform(Platform::Netease)
                .with_details(json!({ "id": id }))
            })?;
        map_stream(track, request, stream, self.client.is_authenticated())
    }
}

fn map_stream(
    track: &Track,
    request: &StreamRequest,
    stream: StreamData,
    authenticated: bool,
) -> Result<MediaStream> {
    let url = stream
        .url
        .as_deref()
        .map(str::trim)
        .filter(|url| !url.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| stream_unavailable(&stream, authenticated))?;
    let actual_quality = stream_quality(stream.level.as_deref(), stream.br);
    let trial = stream.free_trial_info.and_then(|trial| {
        let start_ms = trial.start?.checked_mul(1_000)?;
        let end_ms = trial.end?.checked_mul(1_000)?;
        (end_ms > start_ms).then_some(TrialWindow { start_ms, end_ms })
    });

    Ok(MediaStream {
        url,
        backup_urls: Vec::new(),
        headers: BTreeMap::new(),
        expires_at: stream
            .expi
            .filter(|expires_in_seconds| *expires_in_seconds > 0)
            .and_then(expiration_rfc3339),
        format: stream.kind.clone(),
        codec: stream.encode_type.or(stream.kind),
        bitrate: stream.br,
        size: stream.size,
        duration_ms: stream.time.or(track.duration_ms),
        requested_quality: request.quality,
        actual_quality,
        trial,
        origin_track: Some(track.resource_ref.clone()),
        resolved_track: track.resource_ref.clone(),
        resolved_platform: Platform::Netease,
        match_score: Some(1.0),
        attempts: Vec::new(),
    })
}

fn stream_unavailable(stream: &StreamData, authenticated: bool) -> TuneWeaveError {
    let code = if !authenticated && stream.fee.is_some_and(|fee| fee > 0) {
        ErrorCode::AuthenticationRequired
    } else if stream.code == Some(404) {
        ErrorCode::ResourceNotFound
    } else {
        ErrorCode::PermissionDenied
    };
    TuneWeaveError::new(code, "NetEase did not return a playable stream")
        .with_platform(Platform::Netease)
        .with_details(json!({
            "id": stream.id,
            "upstream_code": stream.code,
            "fee": stream.fee,
            "level": stream.level
        }))
}

fn requested_bitrate(quality: Quality) -> u64 {
    match quality {
        Quality::Low | Quality::Standard => 128_000,
        Quality::High => 320_000,
        Quality::Auto | Quality::Lossless | Quality::Hires | Quality::Spatial | Quality::Master => {
            999_000
        }
    }
}

fn stream_quality(level: Option<&str>, bitrate: Option<u64>) -> Quality {
    match level.unwrap_or_default().to_ascii_lowercase().as_str() {
        "standard" => Quality::Standard,
        "higher" | "exhigh" => Quality::High,
        "lossless" => Quality::Lossless,
        "hires" => Quality::Hires,
        "jyeffect" | "sky" | "dolby" => Quality::Spatial,
        "jymaster" => Quality::Master,
        _ => match bitrate {
            None | Some(0) => Quality::Auto,
            Some(1..=96_000) => Quality::Low,
            Some(96_001..=192_000) => Quality::Standard,
            Some(192_001..=500_000) => Quality::High,
            Some(500_001..) => Quality::Lossless,
        },
    }
}

fn expiration_rfc3339(expires_in_seconds: u64) -> Option<String> {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs();
    unix_rfc3339(now.checked_add(expires_in_seconds)?)
}

fn unix_rfc3339(timestamp: u64) -> Option<String> {
    let days = i64::try_from(timestamp / 86_400).ok()?;
    let seconds = timestamp % 86_400;
    let z = days.checked_add(719_468)?;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    if month <= 2 {
        year += 1;
    }
    if !(0..=9_999).contains(&year) {
        return None;
    }
    let hour = seconds / 3_600;
    let minute = (seconds % 3_600) / 60;
    let second = seconds % 60;
    Some(format!(
        "{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z"
    ))
}

fn map_lyrics(id: u64, lyrics: LyricsEnvelope) -> Result<Lyrics> {
    let track_ref = ResourceRef::new(Platform::Netease, id.to_string()).map_err(|error| {
        TuneWeaveError::new(
            ErrorCode::UpstreamError,
            format!("NetEase returned an invalid lyrics track id: {error}"),
        )
        .with_platform(Platform::Netease)
    })?;
    let plain = lyric_text(lyrics.lrc.as_ref());
    let translated = lyric_text(lyrics.tlyric.as_ref());
    let romanized = lyric_text(lyrics.romalrc.as_ref());
    let word_synced = lyric_text(lyrics.yrc.as_ref());
    let format = if plain.is_some() {
        "lrc"
    } else if word_synced.is_some() {
        "yrc"
    } else {
        "plain"
    }
    .to_owned();
    let mut contributors = Vec::new();
    if let Some(contributor) = map_lyric_user("lyrics", lyrics.lyric_user)? {
        contributors.push(contributor);
    }
    if let Some(contributor) = map_lyric_user("translation", lyrics.trans_user)? {
        contributors.push(contributor);
    }
    let mut extensions = Extensions::new();
    insert_extension(&mut extensions, "pure_music", lyrics.pure_music);
    insert_lyric_extension(
        &mut extensions,
        "word_synced_translated",
        lyrics.ytlrc.as_ref(),
    );
    insert_lyric_extension(
        &mut extensions,
        "word_synced_romanized",
        lyrics.yromalrc.as_ref(),
    );
    insert_lyric_version(&mut extensions, "plain_version", lyrics.lrc.as_ref());
    insert_lyric_version(
        &mut extensions,
        "translated_version",
        lyrics.tlyric.as_ref(),
    );
    insert_lyric_version(
        &mut extensions,
        "romanized_version",
        lyrics.romalrc.as_ref(),
    );
    insert_lyric_version(&mut extensions, "word_synced_version", lyrics.yrc.as_ref());

    Ok(Lyrics {
        track_ref,
        plain,
        translated,
        romanized,
        word_synced,
        format,
        contributors,
        extensions,
    })
}

fn lyric_text(lyrics: Option<&LyricText>) -> Option<String> {
    lyrics
        .and_then(|lyrics| lyrics.lyric.as_deref())
        .map(str::trim)
        .filter(|lyrics| !lyrics.is_empty())
        .map(str::to_owned)
}

fn map_lyric_user(role: &str, user: Option<LyricUser>) -> Result<Option<LyricContributor>> {
    let Some(user) = user else {
        return Ok(None);
    };
    let Some(name) = user
        .nickname
        .map(|name| name.trim().to_owned())
        .filter(|name| !name.is_empty())
    else {
        return Ok(None);
    };
    let resource_ref = user
        .id
        .or(user.userid)
        .or(user.user_id)
        .filter(|id| *id > 0)
        .map(|id| ResourceRef::new(Platform::Netease, id.to_string()))
        .transpose()
        .map_err(|error| {
            TuneWeaveError::new(
                ErrorCode::UpstreamError,
                format!("NetEase returned an invalid lyric contributor id: {error}"),
            )
            .with_platform(Platform::Netease)
        })?;
    Ok(Some(LyricContributor {
        role: role.to_owned(),
        resource_ref,
        name,
    }))
}

fn insert_lyric_extension(extensions: &mut Extensions, name: &str, lyrics: Option<&LyricText>) {
    if let Some(lyrics) = lyric_text(lyrics) {
        extensions.insert(name.to_owned(), json!(lyrics));
    }
}

fn insert_lyric_version(extensions: &mut Extensions, name: &str, lyrics: Option<&LyricText>) {
    if let Some(version) = lyrics.and_then(|lyrics| lyrics.version) {
        extensions.insert(name.to_owned(), json!(version));
    }
}

fn map_playlist(playlist: PlaylistDetail) -> Result<Playlist> {
    let resource_ref =
        ResourceRef::new(Platform::Netease, playlist.id.to_string()).map_err(|error| {
            TuneWeaveError::new(
                ErrorCode::UpstreamError,
                format!("NetEase returned an invalid playlist id: {error}"),
            )
            .with_platform(Platform::Netease)
        })?;
    let creator = playlist
        .creator
        .map(
            |creator| -> std::result::Result<ArtistSummary, ParseResourceRefError> {
                Ok(ArtistSummary {
                    resource_ref: Some(ResourceRef::new(
                        Platform::Netease,
                        creator.user_id.to_string(),
                    )?),
                    name: creator.nickname,
                })
            },
        )
        .transpose()
        .map_err(|error| {
            TuneWeaveError::new(
                ErrorCode::UpstreamError,
                format!("NetEase returned an invalid playlist creator id: {error}"),
            )
            .with_platform(Platform::Netease)
        })?;
    let mut extensions = Extensions::new();
    insert_extension(&mut extensions, "create_time_ms", playlist.create_time);
    insert_extension(&mut extensions, "update_time_ms", playlist.update_time);
    insert_extension(&mut extensions, "privacy", playlist.privacy);
    insert_extension(&mut extensions, "special_type", playlist.special_type);
    insert_extension(&mut extensions, "play_count", playlist.play_count);

    Ok(Playlist {
        resource_ref,
        platform: Platform::Netease,
        id: playlist.id.to_string(),
        name: playlist.name,
        description: playlist.description.unwrap_or_default(),
        cover_url: playlist.cover_img_url,
        creator,
        track_count: playlist
            .track_count
            .or(Some(playlist.track_ids.len() as u64)),
        tags: playlist.tags,
        subscribed: playlist.subscribed,
        created_at: None,
        updated_at: None,
        extensions,
    })
}

fn insert_extension<T: serde::Serialize>(
    extensions: &mut Extensions,
    name: &str,
    value: Option<T>,
) {
    if let Some(value) = value.and_then(|value| serde_json::to_value(value).ok()) {
        extensions.insert(name.to_owned(), value);
    }
}

fn map_song(song: Song, outer_privilege: Option<Privilege>) -> Result<Track> {
    let resource_ref =
        ResourceRef::new(Platform::Netease, song.id.to_string()).map_err(|error| {
            TuneWeaveError::new(
                ErrorCode::UpstreamError,
                format!("NetEase returned an invalid track id: {error}"),
            )
            .with_platform(Platform::Netease)
        })?;
    let available_qualities = map_qualities(&song);
    let artists = song
        .ar
        .into_iter()
        .map(
            |artist| -> std::result::Result<ArtistSummary, ParseResourceRefError> {
                Ok(ArtistSummary {
                    resource_ref: Some(ResourceRef::new(Platform::Netease, artist.id.to_string())?),
                    name: artist.name,
                })
            },
        )
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|error| {
            TuneWeaveError::new(
                ErrorCode::UpstreamError,
                format!("NetEase returned an invalid artist id: {error}"),
            )
            .with_platform(Platform::Netease)
        })?;
    let album = song
        .al
        .map(
            |album| -> std::result::Result<AlbumSummary, ParseResourceRefError> {
                Ok(AlbumSummary {
                    resource_ref: (album.id > 0)
                        .then(|| ResourceRef::new(Platform::Netease, album.id.to_string()))
                        .transpose()?,
                    name: album.name,
                    cover_url: album.pic_url,
                })
            },
        )
        .transpose()
        .map_err(|error| {
            TuneWeaveError::new(
                ErrorCode::UpstreamError,
                format!("NetEase returned an invalid album id: {error}"),
            )
            .with_platform(Platform::Netease)
        })?;
    let mv_ref = song
        .mv
        .filter(|id| *id > 0)
        .map(|id| ResourceRef::new(Platform::Netease, id.to_string()))
        .transpose()
        .map_err(|error| {
            TuneWeaveError::new(
                ErrorCode::UpstreamError,
                format!("NetEase returned an invalid MV id: {error}"),
            )
            .with_platform(Platform::Netease)
        })?;
    let privilege = outer_privilege.or(song.privilege);
    let playable = privilege
        .as_ref()
        .map(|privilege| privilege.st >= 0 && privilege.pl > 0)
        .or_else(|| song.st.map(|status| status >= 0));
    let mut extensions = Extensions::new();
    if let Some(fee) = song.fee {
        extensions.insert("fee".to_owned(), json!(fee));
    }
    if let Some(mark) = song.mark {
        extensions.insert("mark".to_owned(), json!(mark));
    }
    if let Some(privilege) = privilege {
        extensions.insert(
            "privilege".to_owned(),
            json!({
                "fee": privilege.fee,
                "max_bitrate": privilege.maxbr,
                "play_bitrate": privilege.pl,
                "status": privilege.st
            }),
        );
    }

    Ok(Track {
        resource_ref,
        platform: Platform::Netease,
        id: song.id.to_string(),
        name: song.name,
        aliases: song.alia,
        artists,
        album,
        duration_ms: song.dt,
        isrc: None,
        mv_ref,
        playable,
        available_qualities,
        extensions,
    })
}

fn map_qualities(song: &Song) -> Vec<Quality> {
    let mut qualities = Vec::new();
    if has_audio(&song.l) || has_audio(&song.m) || has_audio(&song.h) {
        qualities.push(Quality::Standard);
    }
    if has_audio(&song.m) || has_audio(&song.h) {
        qualities.push(Quality::High);
    }
    if has_audio(&song.sq) {
        qualities.push(Quality::Lossless);
    }
    if has_audio(&song.hr) {
        qualities.push(Quality::Hires);
    }
    qualities
}

fn has_audio(quality: &Option<AudioQuality>) -> bool {
    quality
        .as_ref()
        .is_some_and(|quality| quality.br.unwrap_or(1) > 0)
}

fn parse_numeric_id(resource: &str, id: &str) -> Result<u64> {
    id.parse().map_err(|_| {
        TuneWeaveError::invalid_request(format!(
            "NetEase {resource} id must be an unsigned integer"
        ))
        .with_platform(Platform::Netease)
        .with_details(json!({ "resource": resource, "id": id }))
    })
}

fn capability_for_search(kind: SearchKind) -> Capability {
    match kind {
        SearchKind::Track => Capability::SearchTracks,
        SearchKind::Album => Capability::SearchAlbums,
        SearchKind::Artist => Capability::SearchArtists,
        SearchKind::Playlist => Capability::SearchPlaylists,
        SearchKind::Video => Capability::SearchVideos,
    }
}

fn ensure_success(body: &Value) -> Result<()> {
    let code = body["code"]
        .as_i64()
        .or_else(|| body["code"].as_str().and_then(|code| code.parse().ok()))
        .unwrap_or(500);
    if code == 200 {
        return Ok(());
    }
    let message = body["message"]
        .as_str()
        .or_else(|| body["msg"].as_str())
        .unwrap_or("NetEase request failed");
    let error_code = match code {
        301 | 401 => ErrorCode::AuthenticationRequired,
        403 => ErrorCode::PermissionDenied,
        404 => ErrorCode::ResourceNotFound,
        429 => ErrorCode::RateLimited,
        _ => ErrorCode::UpstreamError,
    };
    Err(TuneWeaveError::new(error_code, message)
        .with_platform(Platform::Netease)
        .retryable(code == 429 || code >= 500)
        .with_details(json!({ "upstream_code": code })))
}

fn parse_body<T: DeserializeOwned>(body: Value) -> Result<T> {
    serde_json::from_value(body).map_err(|error| {
        TuneWeaveError::new(
            ErrorCode::UpstreamError,
            format!("failed to parse NetEase response: {error}"),
        )
        .with_platform(Platform::Netease)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_song() -> Song {
        serde_json::from_value(json!({
            "id": 123,
            "name": "反方向的钟",
            "alia": ["Clockwise"],
            "ar": [{"id": 6452, "name": "周杰伦"}],
            "al": {"id": 456, "name": "Jay", "picUrl": "https://example.test/cover.jpg"},
            "dt": 258000,
            "mv": 0,
            "fee": 1,
            "st": 0,
            "mark": 8192,
            "privilege": {"id": 123, "st": 0, "fee": 1, "pl": 320000, "maxbr": 999000},
            "l": {"br": 128000},
            "m": {"br": 192000},
            "h": {"br": 320000},
            "sq": {"br": 999000},
            "hr": null
        }))
        .expect("valid fixture")
    }

    #[test]
    fn maps_netease_song_to_unified_track() {
        let track = map_song(fixture_song(), None).expect("map song");
        assert_eq!(track.resource_ref.to_string(), "netease:123");
        assert_eq!(track.name, "反方向的钟");
        assert_eq!(track.artists[0].name, "周杰伦");
        assert_eq!(track.duration_ms, Some(258000));
        assert_eq!(
            track.available_qualities,
            vec![Quality::Standard, Quality::High, Quality::Lossless]
        );
        assert_eq!(track.playable, Some(true));
        assert_eq!(track.extensions["fee"], 1);
    }

    #[test]
    fn maps_legacy_search_song_shape() {
        let song = serde_json::from_value(json!({
            "id": 123,
            "name": "反方向的钟",
            "alias": ["Clockwise"],
            "artists": [{"id": 6452, "name": "周杰伦"}],
            "album": {"id": 456, "name": "Jay", "picUrl": "https://example.test/cover.jpg"},
            "duration": 258000,
            "mvid": 789,
            "fee": 1,
            "status": 0,
            "hMusic": {"bitrate": 320000},
            "sqMusic": {"bitrate": 999000}
        }))
        .expect("valid legacy search fixture");

        let track = map_song(song, None).expect("map legacy search song");
        assert_eq!(track.artists[0].name, "周杰伦");
        assert_eq!(track.album.expect("album").name, "Jay");
        assert_eq!(track.duration_ms, Some(258000));
        assert_eq!(track.mv_ref.expect("MV").to_string(), "netease:789");
        assert_eq!(track.playable, Some(true));
        assert_eq!(
            track.available_qualities,
            vec![Quality::Standard, Quality::High, Quality::Lossless]
        );
    }

    #[test]
    fn maps_netease_playlist_to_unified_model() {
        let playlist: PlaylistDetail = serde_json::from_value(json!({
            "id": 3778678,
            "name": "云音乐热歌榜",
            "description": "热门歌曲",
            "coverImgUrl": "https://example.test/playlist.jpg",
            "creator": {"userId": 1, "nickname": "网易云音乐"},
            "trackCount": 2,
            "tags": ["流行"],
            "subscribed": false,
            "createTime": 1378721408222_u64,
            "updateTime": 1783987200000_u64,
            "privacy": 0,
            "specialType": 10,
            "playCount": 12345,
            "trackIds": [{"id": 185809}, {"id": 186001}]
        }))
        .expect("valid playlist fixture");

        let playlist = map_playlist(playlist).expect("map playlist");
        assert_eq!(playlist.resource_ref.to_string(), "netease:3778678");
        assert_eq!(playlist.creator.expect("creator").name, "网易云音乐");
        assert_eq!(playlist.track_count, Some(2));
        assert_eq!(playlist.extensions["special_type"], 10);
    }

    #[test]
    fn maps_netease_lyrics_and_contributors() {
        let lyrics: LyricsEnvelope = serde_json::from_value(json!({
            "lrc": {"version": 12, "lyric": "[00:01.00]素胚勾勒出青花"},
            "tlyric": {"version": 3, "lyric": "[00:01.00]Blue and white porcelain"},
            "romalrc": {"version": 1, "lyric": "[00:01.00]su pei gou le"},
            "yrc": {"version": 7, "lyric": "[1000,2000](1000,500,0)素胚"},
            "ytlrc": {"version": 2, "lyric": "[1000,2000]Blue porcelain"},
            "yromalrc": null,
            "lyricUser": {"id": 10, "nickname": "歌词贡献者"},
            "transUser": {"userId": 11, "nickname": "翻译贡献者"},
            "pureMusic": false
        }))
        .expect("valid lyrics fixture");

        let lyrics = map_lyrics(185809, lyrics).expect("map lyrics");
        assert_eq!(lyrics.track_ref.to_string(), "netease:185809");
        assert_eq!(lyrics.format, "lrc");
        assert!(lyrics.plain.is_some_and(|lyrics| lyrics.contains("青花")));
        assert!(lyrics.word_synced.is_some());
        assert_eq!(lyrics.contributors.len(), 2);
        assert_eq!(lyrics.contributors[1].role, "translation");
        assert_eq!(lyrics.extensions["word_synced_version"], 7);
    }

    #[test]
    fn maps_netease_stream_quality_expiry_and_trial() {
        let track = Track::new(
            ResourceRef::new(Platform::Netease, "123").expect("track reference"),
            "测试歌曲",
        );
        let request = StreamRequest {
            quality: Quality::High,
            account: None,
        };
        let stream: StreamData = serde_json::from_value(json!({
            "id": 123,
            "url": "https://example.test/audio.mp3",
            "br": 320000,
            "size": 1024,
            "code": 200,
            "expi": 1200,
            "type": "mp3",
            "level": "exhigh",
            "encodeType": "mp3",
            "time": 258000,
            "fee": 1,
            "freeTrialInfo": {"start": 0, "end": 30}
        }))
        .expect("valid stream fixture");

        let stream = map_stream(&track, &request, stream, false).expect("map stream");
        assert_eq!(stream.requested_quality, Quality::High);
        assert_eq!(stream.actual_quality, Quality::High);
        assert_eq!(stream.bitrate, Some(320000));
        assert_eq!(stream.trial.expect("trial").end_ms, 30_000);
        assert!(
            stream
                .expires_at
                .is_some_and(|expires| expires.ends_with('Z'))
        );
    }

    #[test]
    fn reports_missing_paid_stream_as_authentication_required() {
        let track = Track::new(
            ResourceRef::new(Platform::Netease, "123").expect("track reference"),
            "测试歌曲",
        );
        let request = StreamRequest {
            quality: Quality::Lossless,
            account: None,
        };
        let stream: StreamData = serde_json::from_value(json!({
            "id": 123,
            "url": null,
            "br": 999000,
            "size": 0,
            "code": 200,
            "expi": 0,
            "type": null,
            "level": "lossless",
            "encodeType": null,
            "time": 258000,
            "fee": 1,
            "freeTrialInfo": null
        }))
        .expect("valid unavailable stream fixture");

        let error = map_stream(&track, &request, stream, false).expect_err("stream must fail");
        assert_eq!(error.code, ErrorCode::AuthenticationRequired);
    }

    #[test]
    fn formats_unix_time_without_a_datetime_dependency() {
        assert_eq!(unix_rfc3339(0).as_deref(), Some("1970-01-01T00:00:00Z"));
        assert_eq!(
            unix_rfc3339(1_704_067_200).as_deref(),
            Some("2024-01-01T00:00:00Z")
        );
    }

    #[tokio::test]
    #[ignore = "requires live NetEase access"]
    async fn live_provider_search_and_track_detail() {
        let provider = NeteaseProvider::new(NeteaseConfig::default()).expect("build provider");
        let page = provider
            .search(&SearchQuery::tracks("反方向的钟", 2, 0))
            .await
            .expect("live provider search");
        let first = page.items.first().expect("at least one song");
        assert!(!first.name.is_empty());
        assert!(!first.artists.is_empty());
        let detail = provider
            .track(&first.id, None)
            .await
            .expect("live track detail");
        assert_eq!(detail.id, first.id);
        assert!(!detail.name.is_empty());
        assert!(!detail.artists.is_empty());
    }

    #[tokio::test]
    #[ignore = "requires live NetEase access"]
    async fn live_public_playlist_and_tracks() {
        let provider = NeteaseProvider::new(NeteaseConfig::default()).expect("build provider");
        let playlist = provider
            .playlist("3778678", None)
            .await
            .expect("live playlist detail");
        assert_eq!(playlist.resource_ref.to_string(), "netease:3778678");
        assert!(!playlist.name.is_empty());

        let page = provider
            .playlist_tracks("3778678", &PageRequest::new(2, 0))
            .await
            .expect("live playlist tracks");
        assert_eq!(page.items.len(), 2);
        assert!(page.pagination.total.is_some_and(|total| total >= 2));
        assert!(page.items.iter().all(|track| !track.artists.is_empty()));
    }

    #[tokio::test]
    #[ignore = "requires live NetEase access"]
    async fn live_track_lyrics() {
        let provider = NeteaseProvider::new(NeteaseConfig::default()).expect("build provider");
        let lyrics = provider
            .lyrics("185809", None)
            .await
            .expect("live track lyrics");
        assert_eq!(lyrics.track_ref.to_string(), "netease:185809");
        assert!(lyrics.plain.is_some() || lyrics.word_synced.is_some());
    }

    #[tokio::test]
    #[ignore = "requires live NetEase access"]
    async fn live_public_track_stream() {
        let provider = NeteaseProvider::new(NeteaseConfig::default()).expect("build provider");
        let track = Track::new(
            ResourceRef::new(Platform::Netease, "2709812973").expect("track reference"),
            "live stream fixture",
        );
        let stream = provider
            .stream(
                &track,
                &StreamRequest {
                    quality: Quality::High,
                    account: None,
                },
            )
            .await
            .expect("live track stream");
        assert!(stream.url.starts_with("http"));
        assert_eq!(stream.resolved_track.to_string(), "netease:2709812973");
        assert!(stream.bitrate.is_some_and(|bitrate| bitrate > 0));
    }
}
