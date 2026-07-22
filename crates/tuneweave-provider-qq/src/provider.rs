use std::{collections::BTreeSet, sync::Arc, time::SystemTime};

use async_trait::async_trait;
use serde_json::{Value, json};
use tuneweave_core::{
    AccountCredentialStore, Album, AlbumSummary, Artist, ArtistSummary, AudioCdnDispatch,
    AudioCdnNode, AudioFileAccess, AudioFileBatch, AudioFileRequest, Capability, CreatorSummary,
    ErrorCode, Extensions, Lyrics, LyricsRequest, MusicProvider, Page, PageMeta, Platform,
    Playlist, Podcast, PodcastEpisode, Quality, ResourceRef, Result, SearchItem, SearchKind,
    SearchOpaqueItem, SearchQuery, SearchSuggestion, SearchSuggestionClient, SearchSuggestionList,
    SearchSuggestionRequest, SearchTrendingDetail, SearchTrendingEntry, SearchTrendingList,
    SearchTrendingRequest, SearchVariant, Track, TuneWeaveError, User, Video,
};

use crate::client::{QqApiRequest, QqApiResponse, QqClient, QqConfig, QqCredential};
use crate::qrc::decrypt_qrc;

const SEARCH_MODULE: &str = "music.search.SearchCgiService";
const SEARCH_METHOD: &str = "DoSearchForQQMusicMobile";
const SMARTBOX_MODULE: &str = "music.smartboxCgi.SmartBoxCgi";
const SMARTBOX_METHOD: &str = "GetSmartBoxResult";
const HOTKEY_MODULE: &str = "music.musicsearch.HotkeyService";
const HOTKEY_METHOD: &str = "GetHotkeyForQQMusicMobile";
const QUERY_SONG_MODULE: &str = "music.trackInfo.UniformRuleCtrl";
const QUERY_SONG_METHOD: &str = "CgiGetTrackInfo";
const SONG_DETAIL_MODULE: &str = "music.pf_song_detail_svr";
const SONG_DETAIL_METHOD: &str = "get_song_detail_yqq";
const LYRIC_MODULE: &str = "music.musichallSong.PlayLyricInfo";
const LYRIC_METHOD: &str = "GetPlayLyricInfo";
const CDN_DISPATCH_MODULE: &str = "music.audioCdnDispatch.cdnDispatch";
const CDN_DISPATCH_METHOD: &str = "GetCdnDispatch";
const SONG_URL_MODULE: &str = "music.vkey.GetVkey";
const SONG_URL_METHOD: &str = "UrlGetVkey";
const ENCRYPTED_SONG_URL_MODULE: &str = "music.vkey.GetEVkey";
const ENCRYPTED_SONG_URL_METHOD: &str = "CgiGetEVkey";
const QQ_CREDENTIAL_KIND: &str = "qq_credential_v1";
const MAX_SONG_URL_ITEMS: usize = 100;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct QqAudioFileSpec {
    name: &'static str,
    prefix: &'static str,
    extension: &'static str,
    encrypted: bool,
    codec: &'static str,
    bitrate: Option<u64>,
    quality: Option<Quality>,
}

const fn qq_file_spec(
    name: &'static str,
    prefix: &'static str,
    extension: &'static str,
    encrypted: bool,
    codec: &'static str,
    bitrate: Option<u64>,
    quality: Option<Quality>,
) -> QqAudioFileSpec {
    QqAudioFileSpec {
        name,
        prefix,
        extension,
        encrypted,
        codec,
        bitrate,
        quality,
    }
}

// Indices 0..=43 exactly match QQMusicApi's public Web integer mapping. Index 44 exposes
// SpecialSongFileType.TRY_OGG_640, which is public in the SDK but absent from that Web tuple.
const QQ_AUDIO_FILE_SPECS: &[QqAudioFileSpec] = &[
    qq_file_spec(
        "dts_x",
        "DT03",
        ".mp4",
        false,
        "dts",
        None,
        Some(Quality::Surround),
    ),
    qq_file_spec(
        "master",
        "AI00",
        ".flac",
        false,
        "flac",
        None,
        Some(Quality::Master),
    ),
    qq_file_spec(
        "atmos_2",
        "Q000",
        ".flac",
        false,
        "flac",
        None,
        Some(Quality::Hires),
    ),
    qq_file_spec(
        "atmos_5_1",
        "Q001",
        ".flac",
        false,
        "flac",
        None,
        Some(Quality::Spatial),
    ),
    qq_file_spec(
        "atmos_7_1",
        "Q003",
        ".ogg",
        false,
        "ogg",
        None,
        Some(Quality::Spatial),
    ),
    qq_file_spec(
        "dolby_atmos",
        "D004",
        ".mp4",
        false,
        "eac3",
        None,
        Some(Quality::Dolby),
    ),
    qq_file_spec(
        "nac",
        "TL01",
        ".nac",
        false,
        "nac",
        None,
        Some(Quality::Higher),
    ),
    qq_file_spec(
        "flac",
        "F000",
        ".flac",
        false,
        "flac",
        None,
        Some(Quality::Lossless),
    ),
    qq_file_spec(
        "ogg_640",
        "O801",
        ".ogg",
        false,
        "ogg",
        Some(640_000),
        Some(Quality::Lossless),
    ),
    qq_file_spec(
        "ogg_320",
        "O800",
        ".ogg",
        false,
        "ogg",
        Some(320_000),
        Some(Quality::High),
    ),
    qq_file_spec(
        "ogg_192",
        "O600",
        ".ogg",
        false,
        "ogg",
        Some(192_000),
        Some(Quality::Higher),
    ),
    qq_file_spec(
        "ogg_96",
        "O400",
        ".ogg",
        false,
        "ogg",
        Some(96_000),
        Some(Quality::Low),
    ),
    qq_file_spec(
        "mp3_320",
        "M800",
        ".mp3",
        false,
        "mp3",
        Some(320_000),
        Some(Quality::High),
    ),
    qq_file_spec(
        "mp3_128",
        "M500",
        ".mp3",
        false,
        "mp3",
        Some(128_000),
        Some(Quality::Standard),
    ),
    qq_file_spec(
        "aac_192",
        "C600",
        ".m4a",
        false,
        "aac",
        Some(192_000),
        Some(Quality::Higher),
    ),
    qq_file_spec(
        "aac_96",
        "C400",
        ".m4a",
        false,
        "aac",
        Some(96_000),
        Some(Quality::Low),
    ),
    qq_file_spec(
        "aac_48",
        "C200",
        ".m4a",
        false,
        "aac",
        Some(48_000),
        Some(Quality::Low),
    ),
    qq_file_spec(
        "encrypted_dts_x",
        "DTM3",
        ".mmp4",
        true,
        "dts",
        None,
        Some(Quality::Surround),
    ),
    qq_file_spec(
        "encrypted_vinyl",
        "V0M0",
        ".mflac",
        true,
        "flac",
        None,
        Some(Quality::Hires),
    ),
    qq_file_spec(
        "encrypted_master",
        "AIM0",
        ".mflac",
        true,
        "flac",
        None,
        Some(Quality::Master),
    ),
    qq_file_spec(
        "encrypted_atmos_2",
        "Q0M0",
        ".mflac",
        true,
        "flac",
        None,
        Some(Quality::Hires),
    ),
    qq_file_spec(
        "encrypted_atmos_5_1",
        "Q0M1",
        ".mflac",
        true,
        "flac",
        None,
        Some(Quality::Spatial),
    ),
    qq_file_spec(
        "encrypted_atmos_7_1",
        "Q0M3",
        ".mgg",
        true,
        "ogg",
        None,
        Some(Quality::Spatial),
    ),
    qq_file_spec(
        "encrypted_dolby_atmos",
        "D0M4",
        ".mmp4",
        true,
        "eac3",
        None,
        Some(Quality::Dolby),
    ),
    qq_file_spec(
        "encrypted_nac",
        "TLM1",
        ".mnac",
        true,
        "nac",
        None,
        Some(Quality::Higher),
    ),
    qq_file_spec(
        "encrypted_flac",
        "F0M0",
        ".mflac",
        true,
        "flac",
        None,
        Some(Quality::Lossless),
    ),
    qq_file_spec(
        "encrypted_ogg_640",
        "O8M1",
        ".mgg",
        true,
        "ogg",
        Some(640_000),
        Some(Quality::Lossless),
    ),
    qq_file_spec(
        "encrypted_ogg_320",
        "O8M0",
        ".mgg",
        true,
        "ogg",
        Some(320_000),
        Some(Quality::High),
    ),
    qq_file_spec(
        "encrypted_ogg_192",
        "O6M0",
        ".mgg",
        true,
        "ogg",
        Some(192_000),
        Some(Quality::Higher),
    ),
    qq_file_spec(
        "encrypted_ogg_96",
        "O4M0",
        ".mgg",
        true,
        "ogg",
        Some(96_000),
        Some(Quality::Low),
    ),
    qq_file_spec(
        "trial",
        "RS02",
        ".mp3",
        false,
        "mp3",
        None,
        Some(Quality::Standard),
    ),
    qq_file_spec("accompaniment", "O801", ".ogg", false, "ogg", None, None),
    qq_file_spec("multi_track", "O601", ".ogg", false, "ogg", None, None),
    qq_file_spec("piano", "AI01", ".ogg", false, "ogg", None, None),
    qq_file_spec("music_box", "AI02", ".ogg", false, "ogg", None, None),
    qq_file_spec("guzheng", "AI03", ".ogg", false, "ogg", None, None),
    qq_file_spec("qudi", "AI04", ".ogg", false, "ogg", None, None),
    qq_file_spec("hulusi", "AI05", ".ogg", false, "ogg", None, None),
    qq_file_spec("suona", "AI06", ".ogg", false, "ogg", None, None),
    qq_file_spec("handpan", "AI07", ".ogg", false, "ogg", None, None),
    qq_file_spec("electric_guitar", "AI08", ".ogg", false, "ogg", None, None),
    qq_file_spec("drums", "AI09", ".ogg", false, "ogg", None, None),
    qq_file_spec("kazoo", "A200", ".ogg", false, "ogg", None, None),
    qq_file_spec("therapy", "AA01", ".ogg", false, "ogg", None, None),
    qq_file_spec(
        "trial_ogg_640",
        "O802",
        ".ogg",
        false,
        "ogg",
        Some(640_000),
        Some(Quality::Lossless),
    ),
];

struct PreparedQqAudioFile {
    track_ref: ResourceRef,
    mid: String,
    media_id: Option<String>,
    song_type: i64,
    spec: &'static QqAudioFileSpec,
    filename: String,
}

#[derive(Clone, Copy)]
struct TypedSearchSpec {
    code: i64,
    item_pointer: &'static str,
    context: &'static str,
    upstream_page_size: u32,
    sparse: bool,
}

type SearchItemMapper = fn(Value) -> Result<SearchItem>;

struct TypedSearchBatch {
    limit: u32,
    skip: u32,
    responses: Vec<QqApiResponse>,
    search_id: String,
    highlight: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum QqTrackIdentifier {
    Numeric(u64),
    Mid(String),
}

const TRACK_SEARCH: TypedSearchSpec = TypedSearchSpec {
    code: 0,
    item_pointer: "/body/item_song",
    context: "QQ track search",
    upstream_page_size: 60,
    sparse: false,
};
const ARTIST_SEARCH: TypedSearchSpec = TypedSearchSpec {
    code: 1,
    item_pointer: "/body/singer",
    context: "QQ artist search",
    upstream_page_size: 40,
    sparse: false,
};
const ALBUM_SEARCH: TypedSearchSpec = TypedSearchSpec {
    code: 2,
    item_pointer: "/body/item_album",
    context: "QQ album search",
    upstream_page_size: 60,
    sparse: false,
};
const PLAYLIST_SEARCH: TypedSearchSpec = TypedSearchSpec {
    code: 3,
    item_pointer: "/body/item_songlist",
    context: "QQ playlist search",
    upstream_page_size: 30,
    sparse: true,
};
const MV_SEARCH: TypedSearchSpec = TypedSearchSpec {
    code: 4,
    item_pointer: "/body/item_mv",
    context: "QQ MV search",
    upstream_page_size: 60,
    sparse: false,
};
const LYRIC_SEARCH: TypedSearchSpec = TypedSearchSpec {
    code: 7,
    item_pointer: "/body/item_song",
    context: "QQ lyric search",
    upstream_page_size: 60,
    sparse: false,
};
const USER_SEARCH: TypedSearchSpec = TypedSearchSpec {
    code: 8,
    item_pointer: "/body/item_user",
    context: "QQ user search",
    upstream_page_size: 10,
    sparse: false,
};
const PODCAST_SEARCH: TypedSearchSpec = TypedSearchSpec {
    code: 15,
    item_pointer: "/body/item_audio",
    context: "QQ podcast search",
    upstream_page_size: 10,
    sparse: false,
};
const VOICE_SEARCH: TypedSearchSpec = TypedSearchSpec {
    code: 18,
    item_pointer: "/body/item_song",
    context: "QQ podcast episode search",
    upstream_page_size: 10,
    sparse: false,
};

#[derive(Clone)]
pub struct QqProvider {
    client: QqClient,
    credential_store: Option<Arc<dyn AccountCredentialStore>>,
}

impl QqProvider {
    pub fn new(config: QqConfig) -> Result<Self> {
        let credential_store = config.credential_store.clone();
        Ok(Self {
            client: QqClient::new(config)?,
            credential_store,
        })
    }

    pub const fn from_client(client: QqClient) -> Self {
        Self {
            client,
            credential_store: None,
        }
    }
}

#[async_trait]
impl MusicProvider for QqProvider {
    fn platform(&self) -> Platform {
        Platform::Qq
    }

    fn name(&self) -> &'static str {
        "QQ Music"
    }

    fn capabilities(&self) -> BTreeSet<Capability> {
        BTreeSet::from([
            Capability::SearchTracks,
            Capability::SearchArtists,
            Capability::SearchAlbums,
            Capability::SearchPlaylists,
            Capability::SearchMvs,
            Capability::SearchLyrics,
            Capability::SearchUsers,
            Capability::SearchPodcasts,
            Capability::SearchVoices,
            Capability::SearchSuggestions,
            Capability::SearchTrending,
            Capability::TrackDetail,
            Capability::Lyrics,
            Capability::AudioCdnDispatch,
            Capability::AudioFileAccess,
        ])
    }

    async fn search(&self, query: &SearchQuery) -> Result<Page<Track>> {
        if query.kind != SearchKind::Track {
            return Err(TuneWeaveError::unsupported(
                Platform::Qq,
                capability_for_search(query.kind),
            ));
        }
        let batch = self.typed_search(query, TRACK_SEARCH).await?;
        let page =
            map_track_search_response(query.offset, batch.limit, batch.skip, batch.responses)?;
        Ok(with_search_context(page, batch.search_id, batch.highlight))
    }

    async fn search_catalog(&self, query: &SearchQuery) -> Result<Page<SearchItem>> {
        if query.kind == SearchKind::Track {
            let page = self.search(query).await?;
            return Ok(Page {
                items: page.items.into_iter().map(SearchItem::Track).collect(),
                pagination: page.pagination,
            });
        }
        let (spec, mapper): (TypedSearchSpec, SearchItemMapper) = match query.kind {
            SearchKind::Artist => (ARTIST_SEARCH, map_artist_search_item),
            SearchKind::Album => (ALBUM_SEARCH, map_album_search_item),
            SearchKind::Playlist => (PLAYLIST_SEARCH, map_playlist_search_item),
            SearchKind::Mv => (MV_SEARCH, map_mv_search_item),
            SearchKind::Lyric => (LYRIC_SEARCH, map_lyric_search_item),
            SearchKind::User => (USER_SEARCH, map_user_search_item),
            SearchKind::Podcast => (PODCAST_SEARCH, map_podcast_search_item),
            SearchKind::Voice => (VOICE_SEARCH, map_voice_search_item),
            kind => {
                return Err(TuneWeaveError::unsupported(
                    Platform::Qq,
                    capability_for_search(kind),
                ));
            }
        };
        let batch = self.typed_search(query, spec).await?;
        let page = map_catalog_search_response(
            query.offset,
            batch.limit,
            batch.skip,
            batch.responses,
            spec,
            mapper,
        )?;
        Ok(with_search_context(page, batch.search_id, batch.highlight))
    }

    async fn search_suggestions(
        &self,
        request: &SearchSuggestionRequest,
    ) -> Result<SearchSuggestionList> {
        let query = request.query.trim();
        if query.is_empty() {
            return Err(
                TuneWeaveError::invalid_request("search suggestion query cannot be empty")
                    .with_platform(Platform::Qq),
            );
        }
        validate_qq_public_account(request.account.as_deref(), "QQ search suggestions")?;
        match request.client {
            SearchSuggestionClient::Mobile => {
                let search_id = generate_search_id()?;
                let response = self
                    .client
                    .request_android(&[smartbox_request(query, &search_id)])
                    .await?
                    .into_iter()
                    .next()
                    .ok_or_else(|| qq_data_error("QQ SmartBox returned no response"))?;
                map_smartbox_response(query, request.client, &search_id, response)
            }
            SearchSuggestionClient::Web => {
                let response = self.client.request_quick_search(query).await?;
                map_quick_search_response(query, response)
            }
            SearchSuggestionClient::Pc => Err(TuneWeaveError::invalid_request(
                "QQ search suggestions do not expose a PC-specific upstream branch",
            )
            .with_platform(Platform::Qq)
            .with_details(json!({ "allowed": ["web", "mobile"] }))),
        }
    }

    async fn trending_searches(
        &self,
        request: &SearchTrendingRequest,
    ) -> Result<SearchTrendingList> {
        validate_qq_public_account(request.account.as_deref(), "QQ trending searches")?;
        let search_id = generate_search_id()?;
        let response = self
            .client
            .request_android(&[hotkey_request(&search_id)])
            .await?
            .into_iter()
            .next()
            .ok_or_else(|| qq_data_error("QQ hotkey service returned no response"))?;
        map_hotkey_response(request.detail, &search_id, response)
    }

    async fn track(&self, id: &str, account: Option<&str>) -> Result<Track> {
        validate_qq_public_account(account, "QQ track detail")?;
        let (request, identifier) = song_detail_request(id)?;
        let response = self
            .client
            .request_web(&[request])
            .await?
            .into_iter()
            .next()
            .ok_or_else(|| qq_data_error("QQ song detail returned no response"))?;
        map_song_detail_response(&identifier, response)
    }

    async fn tracks(&self, ids: &[String], account: Option<&str>) -> Result<Vec<Track>> {
        validate_qq_public_account(account, "QQ track details")?;
        let (request, identifiers) = query_song_request(ids)?;
        let response = self
            .client
            .request_android(&[request])
            .await?
            .into_iter()
            .next()
            .ok_or_else(|| qq_data_error("QQ track query returned no response"))?;
        map_query_song_response(&identifiers, response)
    }

    async fn lyrics(&self, id: &str, account: Option<&str>) -> Result<Lyrics> {
        self.qq_lyrics(
            id,
            &LyricsRequest {
                account: account.map(str::to_owned),
                ..LyricsRequest::default()
            },
        )
        .await
    }

    async fn lyrics_with_options(&self, id: &str, request: &LyricsRequest) -> Result<Lyrics> {
        self.qq_lyrics(id, request).await
    }

    async fn audio_cdn_dispatch(&self, account: Option<&str>) -> Result<AudioCdnDispatch> {
        validate_qq_public_account(account, "QQ audio CDN dispatch")?;
        let (request, guid) = cdn_dispatch_request();
        let response = self
            .client
            .request_android(&[request])
            .await?
            .into_iter()
            .next()
            .ok_or_else(|| qq_data_error("QQ CDN dispatch returned no response"))?;
        map_cdn_dispatch_response(&guid, response)
    }

    async fn audio_files(&self, request: &AudioFileRequest) -> Result<AudioFileBatch> {
        let credential = self.qq_credential(request.account.as_deref())?;
        let prepared = self.prepare_audio_file_items(request).await?;
        let default_spec = parse_qq_audio_file_spec(request.default_spec.as_deref())?;
        let (api_request, guid) = song_urls_request(&prepared, default_spec, credential.as_ref());
        let response = self
            .client
            .request_android_with_credential(&[api_request], credential.as_ref())
            .await?
            .into_iter()
            .next()
            .ok_or_else(|| qq_data_error("QQ song URL service returned no response"))?;
        map_song_urls_response(&prepared, default_spec, &guid, response)
    }
}

impl QqProvider {
    fn qq_credential(&self, account: Option<&str>) -> Result<Option<QqCredential>> {
        let Some(account) = account.map(str::trim).filter(|account| !account.is_empty()) else {
            return Ok(None);
        };
        if account.len() > 64 {
            return Err(
                TuneWeaveError::invalid_request("QQ account alias cannot exceed 64 bytes")
                    .with_platform(Platform::Qq),
            );
        }
        let store = self.credential_store.as_ref().ok_or_else(|| {
            qq_authentication_required(account, "QQ account storage is not configured")
        })?;
        let stored = store
            .load_platform(Platform::Qq)?
            .into_iter()
            .find(|credential| credential.account == account)
            .ok_or_else(|| qq_authentication_required(account, "QQ account was not found"))?;
        if stored.kind != QQ_CREDENTIAL_KIND {
            return Err(TuneWeaveError::new(
                ErrorCode::InternalError,
                "stored QQ account has an unsupported credential format",
            )
            .with_platform(Platform::Qq)
            .with_details(json!({ "account": account })));
        }
        let credential = serde_json::from_str::<QqCredential>(stored.secret())
            .map_err(|_| {
                TuneWeaveError::new(
                    ErrorCode::InternalError,
                    "stored QQ account credential is malformed",
                )
                .with_platform(Platform::Qq)
                .with_details(json!({ "account": account }))
            })?
            .normalize()?;
        Ok(Some(credential))
    }

    async fn prepare_audio_file_items(
        &self,
        request: &AudioFileRequest,
    ) -> Result<Vec<PreparedQqAudioFile>> {
        if request.items.is_empty() {
            return Err(TuneWeaveError::invalid_request(
                "audio file request must contain at least one item",
            )
            .with_platform(Platform::Qq));
        }
        if request.items.len() > MAX_SONG_URL_ITEMS {
            return Err(TuneWeaveError::invalid_request(format!(
                "QQ audio file request cannot exceed {MAX_SONG_URL_ITEMS} items"
            ))
            .with_platform(Platform::Qq));
        }
        let default_spec = parse_qq_audio_file_spec(request.default_spec.as_deref())?;
        let numeric_ids = request
            .items
            .iter()
            .filter(|item| {
                item.track_ref
                    .id()
                    .chars()
                    .all(|character| character.is_ascii_digit())
            })
            .map(|item| item.track_ref.id().to_owned())
            .collect::<Vec<_>>();
        let resolved_numeric = if numeric_ids.is_empty() {
            Vec::new()
        } else {
            self.tracks(&numeric_ids, None).await?
        };
        let mut resolved_numeric = resolved_numeric.into_iter();
        request
            .items
            .iter()
            .map(|item| {
                if item.track_ref.platform() != Platform::Qq {
                    return Err(TuneWeaveError::invalid_request(
                        "QQ audio file request contains a non-QQ track reference",
                    )
                    .with_platform(Platform::Qq));
                }
                let mid = if item
                    .track_ref
                    .id()
                    .chars()
                    .all(|character| character.is_ascii_digit())
                {
                    resolved_numeric
                        .next()
                        .and_then(|track| {
                            track
                                .extensions
                                .get("mid")
                                .and_then(Value::as_str)
                                .map(str::to_owned)
                        })
                        .ok_or_else(|| qq_data_error("QQ track lookup did not return a MID"))?
                } else {
                    item.track_ref.id().to_owned()
                };
                validate_qq_media_id(&mid, "song MID")?;
                let media_id = item
                    .media_id
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(|value| {
                        validate_qq_media_id(value, "media MID")?;
                        Ok(value.to_owned())
                    })
                    .transpose()?;
                let spec = item
                    .spec
                    .as_deref()
                    .map(|spec| parse_qq_audio_file_spec(Some(spec)))
                    .transpose()?
                    .unwrap_or(default_spec);
                let filename = match media_id.as_deref() {
                    Some(media_id) => {
                        format!("{}{media_id}{}", spec.prefix, spec.extension)
                    }
                    None => format!("{}{mid}{mid}{}", spec.prefix, spec.extension),
                };
                Ok(PreparedQqAudioFile {
                    track_ref: item.track_ref.clone(),
                    mid,
                    media_id,
                    song_type: item.song_type.unwrap_or(0),
                    spec,
                    filename,
                })
            })
            .collect()
    }

    async fn qq_lyrics(&self, id: &str, options: &LyricsRequest) -> Result<Lyrics> {
        validate_qq_public_account(options.account.as_deref(), "QQ lyrics")?;
        let (request, identifier) = lyric_request(id, options)?;
        let response = self
            .client
            .request_android(&[request])
            .await?
            .into_iter()
            .next()
            .ok_or_else(|| qq_data_error("QQ lyric service returned no response"))?;
        map_lyric_response(&identifier, options, response)
    }

    async fn typed_search(
        &self,
        query: &SearchQuery,
        spec: TypedSearchSpec,
    ) -> Result<TypedSearchBatch> {
        let keyword = validate_search_query(query)?;
        let limit = query.limit.clamp(1, 100);
        let search_id = query
            .search_id
            .as_deref()
            .map(str::trim)
            .filter(|search_id| !search_id.is_empty())
            .map(str::to_owned)
            .map_or_else(generate_search_id, Ok)?;
        let first_page = query.offset / spec.upstream_page_size + 1;
        let skip = query.offset % spec.upstream_page_size;
        let page_count = skip.saturating_add(limit).div_ceil(spec.upstream_page_size);
        let requests = (0..page_count)
            .map(|page_offset| {
                typed_search_request(
                    keyword,
                    &search_id,
                    spec.code,
                    first_page.saturating_add(page_offset),
                    spec.upstream_page_size,
                    query.highlight,
                )
            })
            .collect::<Vec<_>>();
        let responses = self.client.request_android(&requests).await?;
        Ok(TypedSearchBatch {
            limit,
            skip,
            responses,
            search_id,
            highlight: query.highlight,
        })
    }
}

fn validate_search_query(query: &SearchQuery) -> Result<&str> {
    let keyword = query.query.trim();
    if keyword.is_empty() {
        return Err(
            TuneWeaveError::invalid_request("search query cannot be empty")
                .with_platform(Platform::Qq),
        );
    }
    if query.variant != SearchVariant::Default {
        return Err(TuneWeaveError::invalid_request(
            "QQ typed search only supports the default variant",
        )
        .with_platform(Platform::Qq)
        .with_details(json!({ "variant": query.variant })));
    }
    if let Some(account) = query
        .account
        .as_deref()
        .map(str::trim)
        .filter(|account| !account.is_empty())
    {
        return Err(TuneWeaveError::new(
            ErrorCode::AuthenticationRequired,
            "QQ account selection is not available before QQ login is configured",
        )
        .with_platform(Platform::Qq)
        .with_details(json!({ "account": account })));
    }
    Ok(keyword)
}

fn validate_qq_public_account(account: Option<&str>, context: &str) -> Result<()> {
    let Some(account) = account
        .map(str::trim)
        .filter(|account| !account.is_empty() && *account != "default")
    else {
        return Ok(());
    };
    Err(TuneWeaveError::new(
        ErrorCode::AuthenticationRequired,
        format!("{context} cannot select a QQ account before QQ login is configured"),
    )
    .with_platform(Platform::Qq)
    .with_details(json!({ "account": account })))
}

fn typed_search_request(
    keyword: &str,
    search_id: &str,
    search_type: i64,
    page: u32,
    page_size: u32,
    highlight: bool,
) -> QqApiRequest {
    QqApiRequest::new(
        SEARCH_MODULE,
        SEARCH_METHOD,
        json!({
            "searchid": search_id,
            "query": keyword,
            "search_type": search_type,
            "num_per_page": page_size,
            "page_num": page,
            "highlight": highlight,
            "grp": true
        }),
    )
}

fn with_search_context<T>(mut page: Page<T>, search_id: String, highlight: bool) -> Page<T> {
    page.pagination
        .extensions
        .insert("search_id".to_owned(), Value::String(search_id));
    page.pagination
        .extensions
        .insert("highlight".to_owned(), Value::Bool(highlight));
    page
}

fn smartbox_request(keyword: &str, search_id: &str) -> QqApiRequest {
    QqApiRequest::new(
        SMARTBOX_MODULE,
        SMARTBOX_METHOD,
        json!({
            "search_id": search_id,
            "query": keyword,
            "num_per_page": 0,
            "page_idx": 0
        }),
    )
}

fn map_smartbox_response(
    query: &str,
    client: SearchSuggestionClient,
    requested_search_id: &str,
    response: QqApiResponse,
) -> Result<SearchSuggestionList> {
    if !response.data.is_object() {
        return Err(qq_data_error("QQ SmartBox response data is not an object"));
    }
    let mut suggestions = qq_optional_array(&response.data, "items", "QQ SmartBox")?
        .into_iter()
        .map(|raw| map_smartbox_keyword_suggestion(raw, "items"))
        .collect::<Result<Vec<_>>>()?;
    let recommendations = qq_optional_array(&response.data, "vec_related_items", "QQ SmartBox")?
        .into_iter()
        .map(|raw| map_smartbox_keyword_suggestion(raw, "related"))
        .collect::<Result<Vec<_>>>()?;
    let mut direct = qq_optional_array(&response.data, "vec_direct_items", "QQ SmartBox")?
        .into_iter()
        .enumerate()
        .map(|(index, raw)| {
            let position = raw
                .get("insert_pos")
                .and_then(json_u64)
                .and_then(|position| usize::try_from(position).ok())
                .unwrap_or(usize::MAX);
            map_smartbox_direct_suggestion(raw).map(|suggestion| (position, index, suggestion))
        })
        .collect::<Result<Vec<_>>>()?;
    direct.sort_by_key(|(position, index, _)| (*position, *index));
    let mut previous_position = None;
    let mut same_position_count = 0_usize;
    for (position, _, suggestion) in direct {
        if previous_position == Some(position) {
            same_position_count = same_position_count.saturating_add(1);
        } else {
            previous_position = Some(position);
            same_position_count = 0;
        }
        let target = position
            .saturating_add(same_position_count)
            .min(suggestions.len());
        suggestions.insert(target, suggestion);
    }
    let search_id = nonempty_string(response.data.get("search_id"))
        .unwrap_or_else(|| requested_search_id.to_owned());
    Ok(SearchSuggestionList {
        query: query.to_owned(),
        client,
        suggestions,
        recommendations,
        extensions: Extensions::from([
            ("search_id".to_owned(), Value::String(search_id)),
            ("response".to_owned(), response.raw),
        ]),
    })
}

fn qq_optional_array(container: &Value, field: &str, context: &str) -> Result<Vec<Value>> {
    match container.get(field) {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Array(values)) => Ok(values.clone()),
        Some(_) => Err(qq_data_error(format!(
            "{context} response field {field} is not an array"
        ))),
    }
}

fn map_smartbox_keyword_suggestion(raw: Value, bucket: &'static str) -> Result<SearchSuggestion> {
    let keyword = smartbox_keyword(&raw)
        .ok_or_else(|| qq_data_error(format!("QQ SmartBox {bucket} item is missing its hint")))?;
    let resource_type = smartbox_resource_type(&raw);
    let kind = resource_type.as_deref().and_then(smartbox_search_kind);
    let display_text = ["hint_hilight", "display", "description"]
        .into_iter()
        .find_map(|field| nonempty_string(raw.get(field)))
        .filter(|value| value != &keyword);
    let icon_url = ["icon", "pic_url", "cover_url"]
        .into_iter()
        .find_map(|field| nonempty_string(raw.get(field)));
    Ok(SearchSuggestion {
        keyword,
        kind,
        display_text,
        icon_url,
        resource: None,
        extensions: Extensions::from([
            ("bucket".to_owned(), json!(bucket)),
            ("response".to_owned(), raw),
        ]),
    })
}

fn map_smartbox_direct_suggestion(raw: Value) -> Result<SearchSuggestion> {
    let keyword = smartbox_keyword(&raw).ok_or_else(|| {
        qq_data_error("QQ SmartBox direct item is missing its search history, title, or hint")
    })?;
    let resource_type = smartbox_resource_type(&raw);
    let kind = resource_type.as_deref().and_then(smartbox_search_kind);
    let resource = map_smartbox_direct_resource(&raw, kind, &keyword)?;
    let display_text = ["title", "hint", "desc"]
        .into_iter()
        .find_map(|field| nonempty_string(raw.get(field)))
        .filter(|value| value != &keyword);
    let icon_url = ["cover_url", "pic_url", "icon"]
        .into_iter()
        .find_map(|field| nonempty_string(raw.get(field)));
    Ok(SearchSuggestion {
        keyword,
        kind,
        display_text,
        icon_url,
        resource,
        extensions: Extensions::from([
            ("bucket".to_owned(), json!("direct")),
            ("response".to_owned(), raw),
        ]),
    })
}

fn smartbox_keyword(raw: &Value) -> Option<String> {
    raw.pointer("/custom_info/search_history")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .or_else(|| {
            ["hint", "keyword", "search_word", "title", "name", "query"]
                .into_iter()
                .find_map(|field| nonempty_string(raw.get(field)))
        })
}

fn smartbox_resource_type(raw: &Value) -> Option<String> {
    ["res_type", "restype", "resource_type"]
        .into_iter()
        .find_map(|field| nonempty_string(raw.get(field)))
}

fn smartbox_search_kind(value: &str) -> Option<SearchKind> {
    match value.trim().to_ascii_lowercase().as_str() {
        "song" | "track" => Some(SearchKind::Track),
        "singer" | "artist" => Some(SearchKind::Artist),
        "album" => Some(SearchKind::Album),
        "songlist" | "playlist" => Some(SearchKind::Playlist),
        "user" => Some(SearchKind::User),
        "mv" => Some(SearchKind::Mv),
        "video" => Some(SearchKind::Video),
        "audio_album" | "podcast" => Some(SearchKind::Podcast),
        "audio" | "voice" => Some(SearchKind::Voice),
        _ => None,
    }
}

fn map_smartbox_direct_resource(
    raw: &Value,
    kind: Option<SearchKind>,
    keyword: &str,
) -> Result<Option<SearchItem>> {
    let Some(kind) = kind else {
        return Ok(None);
    };
    let mid = nonempty_string(raw.pointer("/custom_info/mid"));
    let numeric_id = value_as_string(raw.get("direct_id"));
    let id = mid
        .clone()
        .or_else(|| numeric_id.clone())
        .or_else(|| value_as_string(raw.get("docid")));
    if kind == SearchKind::Artist {
        if let Some(id) = id.clone() {
            let mut extensions = Extensions::new();
            insert_some(&mut extensions, "mid", mid);
            insert_some(&mut extensions, "numeric_id", numeric_id);
            extensions.insert("smartbox_item".to_owned(), raw.clone());
            return Ok(Some(SearchItem::Artist(Artist {
                resource_ref: qq_ref(&id, "artist")?,
                platform: Platform::Qq,
                id,
                name: keyword.to_owned(),
                aliases: Vec::new(),
                description: nonempty_string(raw.get("desc")).unwrap_or_default(),
                biography_sections: Vec::new(),
                avatar_url: ["cover_url", "pic_url"]
                    .into_iter()
                    .find_map(|field| nonempty_string(raw.get(field))),
                cover_url: None,
                album_count: None,
                track_count: None,
                mv_count: None,
                video_count: None,
                identities: Vec::new(),
                extensions,
            })));
        }
    }
    Ok(Some(SearchItem::Opaque(SearchOpaqueItem {
        platform: Platform::Qq,
        kind: smartbox_resource_type(raw).unwrap_or_else(|| "direct".to_owned()),
        id,
        title: Some(keyword.to_owned()),
        extensions: Extensions::from([("response".to_owned(), raw.clone())]),
    })))
}

fn map_quick_search_response(query: &str, response: Value) -> Result<SearchSuggestionList> {
    for field in ["code", "subcode"] {
        let code = response
            .get(field)
            .and_then(json_i64)
            .ok_or_else(|| qq_data_error(format!("QQ quick search is missing {field}")))?;
        if code != 0 {
            return Err(TuneWeaveError::new(
                ErrorCode::UpstreamError,
                format!("QQ quick search failed with {field}={code}"),
            )
            .with_platform(Platform::Qq)
            .with_details(json!({ "field": field, "platform_code": code })));
        }
    }
    let data = response
        .get("data")
        .and_then(Value::as_object)
        .ok_or_else(|| qq_data_error("QQ quick search is missing its data object"))?;
    let mut sections = Vec::new();
    for (section_name, section) in data {
        let Some(section_object) = section.as_object() else {
            continue;
        };
        let known_kind = quick_search_kind(section_name, section);
        let itemlist = match section_object.get("itemlist") {
            Some(Value::Array(items)) => items.clone(),
            Some(_) => {
                return Err(qq_data_error(format!(
                    "QQ quick search section {section_name} itemlist is not an array"
                )));
            }
            None if known_kind.is_some() => {
                return Err(qq_data_error(format!(
                    "QQ quick search section {section_name} is missing itemlist"
                )));
            }
            None => continue,
        };
        let order = section.get("order").and_then(json_i64).unwrap_or(i64::MAX);
        sections.push((order, section_name.clone(), known_kind, itemlist, section));
    }
    sections.sort_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(&right.1)));
    let mut suggestions = Vec::new();
    for (_, section_name, kind, items, section) in sections {
        for raw in items {
            suggestions.push(map_quick_search_suggestion(
                &section_name,
                kind,
                section,
                raw,
            )?);
        }
    }
    Ok(SearchSuggestionList {
        query: query.to_owned(),
        client: SearchSuggestionClient::Web,
        suggestions,
        recommendations: Vec::new(),
        extensions: Extensions::from([("response".to_owned(), response)]),
    })
}

fn quick_search_kind(section_name: &str, section: &Value) -> Option<SearchKind> {
    smartbox_search_kind(section_name).or_else(|| {
        section
            .get("type")
            .and_then(json_i64)
            .and_then(|kind| match kind {
                1 => Some(SearchKind::Track),
                2 => Some(SearchKind::Artist),
                3 => Some(SearchKind::Album),
                4 => Some(SearchKind::Mv),
                _ => None,
            })
    })
}

fn map_quick_search_suggestion(
    section_name: &str,
    kind: Option<SearchKind>,
    section: &Value,
    raw: Value,
) -> Result<SearchSuggestion> {
    if !raw.is_object() {
        return Err(qq_data_error(format!(
            "QQ quick search section {section_name} contains a non-object item"
        )));
    }
    let keyword = ["name", "title", "query"]
        .into_iter()
        .find_map(|field| nonempty_string(raw.get(field)))
        .ok_or_else(|| {
            qq_data_error(format!(
                "QQ quick search section {section_name} item is missing its name"
            ))
        })?;
    let resource = map_quick_search_resource(section_name, kind, &keyword, &raw)?;
    let mut extensions = Extensions::new();
    extensions.insert("section".to_owned(), Value::String(section_name.to_owned()));
    insert_value(&mut extensions, "section_name", section.get("name"));
    insert_value(&mut extensions, "section_order", section.get("order"));
    insert_value(&mut extensions, "section_type", section.get("type"));
    insert_value(&mut extensions, "section_count", section.get("count"));
    extensions.insert("response".to_owned(), raw.clone());
    Ok(SearchSuggestion {
        keyword,
        kind,
        display_text: nonempty_string(raw.get("singer")),
        icon_url: ["pic", "cover_url", "pic_url"]
            .into_iter()
            .find_map(|field| nonempty_string(raw.get(field))),
        resource: Some(resource),
        extensions,
    })
}

fn map_quick_search_resource(
    section_name: &str,
    kind: Option<SearchKind>,
    keyword: &str,
    raw: &Value,
) -> Result<SearchItem> {
    let Some(kind) = kind else {
        return Ok(quick_search_opaque_resource(section_name, keyword, raw));
    };
    let mut adapted = raw.clone();
    let singer = nonempty_string(raw.get("singer"));
    match kind {
        SearchKind::Track | SearchKind::Lyric => {
            if let Some(singer) = singer {
                adapted["singer"] = json!([{ "name": singer }]);
            }
            map_track(adapted).map(SearchItem::Track)
        }
        SearchKind::Artist => map_artist_search_item(adapted),
        SearchKind::Album => {
            if let Some(singer) = singer {
                adapted["singer_list"] = json!([{ "name": singer }]);
            }
            map_album_search_item(adapted)
        }
        SearchKind::Playlist => map_playlist_search_item(adapted),
        SearchKind::User => map_user_search_item(adapted),
        SearchKind::Mv | SearchKind::Video => {
            if let Some(singer) = singer {
                adapted["singername"] = Value::String(singer);
            }
            map_mv_search_item(adapted)
        }
        SearchKind::Podcast => map_podcast_search_item(adapted),
        SearchKind::Voice => map_voice_search_item(adapted),
        SearchKind::RadioStation | SearchKind::Mixed => {
            Ok(quick_search_opaque_resource(section_name, keyword, raw))
        }
    }
}

fn quick_search_opaque_resource(section_name: &str, keyword: &str, raw: &Value) -> SearchItem {
    SearchItem::Opaque(SearchOpaqueItem {
        platform: Platform::Qq,
        kind: section_name.to_owned(),
        id: ["mid", "id", "docid"]
            .into_iter()
            .find_map(|field| value_as_string(raw.get(field))),
        title: Some(keyword.to_owned()),
        extensions: Extensions::from([("response".to_owned(), raw.clone())]),
    })
}

fn hotkey_request(search_id: &str) -> QqApiRequest {
    QqApiRequest::new(
        HOTKEY_MODULE,
        HOTKEY_METHOD,
        json!({"search_id": search_id}),
    )
}

fn query_song_request(ids: &[String]) -> Result<(QqApiRequest, Vec<QqTrackIdentifier>)> {
    if ids.is_empty() {
        return Err(TuneWeaveError::invalid_request(
            "QQ track query must contain at least one ID or MID",
        )
        .with_platform(Platform::Qq));
    }
    let values = ids
        .iter()
        .map(|value| parse_qq_track_identifier(value))
        .collect::<Result<Vec<_>>>()?;
    let numeric = values
        .iter()
        .filter_map(|value| match value {
            QqTrackIdentifier::Numeric(value) => Some(*value),
            QqTrackIdentifier::Mid(_) => None,
        })
        .collect::<Vec<_>>();
    if !numeric.is_empty() && numeric.len() != values.len() {
        return Err(TuneWeaveError::invalid_request(
            "QQ track query cannot mix numeric IDs and MIDs",
        )
        .with_platform(Platform::Qq)
        .with_details(json!({ "ids": ids })));
    }
    let mut param = json!({
        "types": vec![0; values.len()],
        "modify_stamp": vec![0; values.len()],
        "ctx": 0,
        "client": 1
    });
    if numeric.len() == values.len() {
        param["ids"] = json!(numeric);
    } else {
        param["mids"] = json!(
            values
                .iter()
                .map(|value| match value {
                    QqTrackIdentifier::Mid(value) => value.clone(),
                    QqTrackIdentifier::Numeric(_) =>
                        unreachable!("mixed identifiers were rejected"),
                })
                .collect::<Vec<_>>()
        );
    }
    Ok((
        QqApiRequest::new(QUERY_SONG_MODULE, QUERY_SONG_METHOD, param),
        values,
    ))
}

fn song_detail_request(id: &str) -> Result<(QqApiRequest, QqTrackIdentifier)> {
    let identifier = parse_qq_track_identifier(id)?;
    let param = match &identifier {
        QqTrackIdentifier::Numeric(id) => json!({ "song_id": id }),
        QqTrackIdentifier::Mid(mid) => json!({ "song_mid": mid }),
    };
    Ok((
        QqApiRequest::new(SONG_DETAIL_MODULE, SONG_DETAIL_METHOD, param),
        identifier,
    ))
}

fn lyric_request(id: &str, options: &LyricsRequest) -> Result<(QqApiRequest, QqTrackIdentifier)> {
    let identifier = parse_qq_track_identifier(id)?;
    let mut param = json!({
        "crypt": 1,
        "lrc_t": 0,
        "qrc": options.word_synced,
        "qrc_t": 0,
        "roma": options.romanized,
        "roma_t": 0,
        "trans": options.translated,
        "trans_t": 0,
        "type": options.song_type.unwrap_or(1),
        "ct": 11,
        "cv": 14090008
    });
    match &identifier {
        QqTrackIdentifier::Numeric(id) => param["songId"] = json!(id),
        QqTrackIdentifier::Mid(mid) => param["songMid"] = json!(mid),
    }
    Ok((
        QqApiRequest::new(LYRIC_MODULE, LYRIC_METHOD, param),
        identifier,
    ))
}

fn cdn_dispatch_request() -> (QqApiRequest, String) {
    let guid = hex::encode(rand::random::<[u8; 16]>());
    (
        QqApiRequest::new(
            CDN_DISPATCH_MODULE,
            CDN_DISPATCH_METHOD,
            json!({
                "guid": guid,
                "uid": "0",
                "use_new_domain": 1,
                "use_ipv6": 1
            }),
        ),
        guid,
    )
}

fn parse_qq_audio_file_spec(value: Option<&str>) -> Result<&'static QqAudioFileSpec> {
    let Some(value) = value else {
        return Ok(&QQ_AUDIO_FILE_SPECS[13]);
    };
    let value = value.trim();
    if value.is_empty() {
        return Err(
            TuneWeaveError::invalid_request("QQ audio file spec cannot be empty")
                .with_platform(Platform::Qq),
        );
    }
    if value.chars().all(|character| character.is_ascii_digit()) {
        let index = value.parse::<usize>().map_err(|_| {
            TuneWeaveError::invalid_request("QQ audio file spec index is invalid")
                .with_platform(Platform::Qq)
        })?;
        return QQ_AUDIO_FILE_SPECS.get(index).ok_or_else(|| {
            TuneWeaveError::invalid_request(format!(
                "QQ audio file spec index must be between 0 and {}",
                QQ_AUDIO_FILE_SPECS.len().saturating_sub(1)
            ))
            .with_platform(Platform::Qq)
        });
    }
    let normalized = value.to_ascii_lowercase().replace(['-', ' '], "_");
    let canonical = match normalized.as_str() {
        "atmos_51" => "atmos_5_1",
        "atmos_71" => "atmos_7_1",
        "atmos_db" => "dolby_atmos",
        "acc_192" => "aac_192",
        "acc_96" => "aac_96",
        "acc_48" => "aac_48",
        "encrypted_atmos_51" => "encrypted_atmos_5_1",
        "encrypted_atmos_71" => "encrypted_atmos_7_1",
        "encrypted_atmos_db" => "encrypted_dolby_atmos",
        "try" => "trial",
        "try_ogg_640" => "trial_ogg_640",
        "accom" => "accompaniment",
        "multi" => "multi_track",
        "bayin" => "music_box",
        "shoudie" => "handpan",
        "guitar" => "electric_guitar",
        value => value,
    };
    QQ_AUDIO_FILE_SPECS
        .iter()
        .find(|spec| spec.name == canonical)
        .ok_or_else(|| {
            TuneWeaveError::invalid_request(format!("unsupported QQ audio file spec: {value}"))
                .with_platform(Platform::Qq)
        })
}

fn song_urls_request(
    items: &[PreparedQqAudioFile],
    default_spec: &QqAudioFileSpec,
    credential: Option<&QqCredential>,
) -> (QqApiRequest, String) {
    let guid = hex::encode(rand::random::<[u8; 16]>());
    let (module, method) = if default_spec.encrypted {
        (ENCRYPTED_SONG_URL_MODULE, ENCRYPTED_SONG_URL_METHOD)
    } else {
        (SONG_URL_MODULE, SONG_URL_METHOD)
    };
    (
        QqApiRequest::new(
            module,
            method,
            json!({
                "uin": credential.map(QqCredential::string_music_id).unwrap_or(""),
                "filename": items.iter().map(|item| item.filename.as_str()).collect::<Vec<_>>(),
                "guid": guid,
                "songmid": items.iter().map(|item| item.mid.as_str()).collect::<Vec<_>>(),
                "songtype": items.iter().map(|item| item.song_type).collect::<Vec<_>>(),
                "ctx": 0
            }),
        ),
        guid,
    )
}

fn validate_qq_media_id(value: &str, name: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .chars()
            .all(|character| character.is_ascii_alphanumeric())
    {
        return Err(TuneWeaveError::invalid_request(format!(
            "QQ {name} must contain 1 to 128 ASCII letters or digits"
        ))
        .with_platform(Platform::Qq));
    }
    Ok(())
}

fn parse_qq_track_identifier(value: &str) -> Result<QqTrackIdentifier> {
    let value = value.trim();
    if value.is_empty() {
        return Err(
            TuneWeaveError::invalid_request("QQ track identifier cannot be empty")
                .with_platform(Platform::Qq),
        );
    }
    if value.chars().all(|character| character.is_ascii_digit()) {
        value
            .parse::<u64>()
            .map(QqTrackIdentifier::Numeric)
            .map_err(|_| {
                TuneWeaveError::invalid_request("QQ numeric track ID is out of range")
                    .with_platform(Platform::Qq)
                    .with_details(json!({ "id": value }))
            })
    } else {
        Ok(QqTrackIdentifier::Mid(value.to_owned()))
    }
}

fn map_query_song_response(
    requested: &[QqTrackIdentifier],
    response: QqApiResponse,
) -> Result<Vec<Track>> {
    let raw_tracks = response
        .data
        .get("tracks")
        .and_then(Value::as_array)
        .ok_or_else(|| qq_data_error("QQ track query response is missing its tracks array"))?;
    let mapped = raw_tracks
        .iter()
        .cloned()
        .map(map_track)
        .collect::<Result<Vec<_>>>()?;
    let mut tracks = Vec::with_capacity(requested.len());
    for (index, identifier) in requested.iter().enumerate() {
        let mut track = mapped
            .iter()
            .find(|track| track_matches_identifier(track, identifier))
            .cloned()
            .ok_or_else(|| qq_track_not_found(&qq_track_identifier_value(identifier)))?;
        track
            .extensions
            .insert("query_index".to_owned(), json!(index));
        track.extensions.insert(
            "query_identifier_kind".to_owned(),
            json!(match identifier {
                QqTrackIdentifier::Numeric(_) => "numeric_id",
                QqTrackIdentifier::Mid(_) => "mid",
            }),
        );
        tracks.push(track);
    }
    Ok(tracks)
}

fn map_song_detail_response(
    requested: &QqTrackIdentifier,
    response: QqApiResponse,
) -> Result<Track> {
    let raw_track = response
        .data
        .get("track_info")
        .filter(|value| value.is_object())
        .cloned()
        .ok_or_else(|| qq_track_not_found(&qq_track_identifier_value(requested)))?;
    validate_song_detail_info(response.data.get("info"))?;
    if response
        .data
        .get("extras")
        .is_some_and(|extras| !extras.is_object() && !extras.is_null())
    {
        return Err(qq_data_error(
            "QQ song detail extras field is not an object",
        ));
    }
    let mut track = map_track(raw_track)?;
    if !track_matches_identifier(&track, requested) {
        return Err(qq_data_error(
            "QQ song detail returned a different track than requested",
        ));
    }
    track.extensions.insert(
        "detail_identifier_kind".to_owned(),
        json!(match requested {
            QqTrackIdentifier::Numeric(_) => "numeric_id",
            QqTrackIdentifier::Mid(_) => "mid",
        }),
    );
    track.extensions.insert(
        "detail_info".to_owned(),
        response
            .data
            .get("info")
            .cloned()
            .unwrap_or_else(|| json!({})),
    );
    track.extensions.insert(
        "detail_extras".to_owned(),
        response
            .data
            .get("extras")
            .cloned()
            .unwrap_or_else(|| json!({})),
    );
    track
        .extensions
        .insert("detail_response".to_owned(), response.raw);
    Ok(track)
}

fn map_lyric_response(
    requested: &QqTrackIdentifier,
    options: &LyricsRequest,
    response: QqApiResponse,
) -> Result<Lyrics> {
    let song_id = response
        .data
        .get("songID")
        .and_then(json_u64)
        .filter(|song_id| *song_id > 0)
        .ok_or_else(|| qq_track_not_found(&qq_track_identifier_value(requested)))?;
    if matches!(requested, QqTrackIdentifier::Numeric(requested_id) if *requested_id != song_id) {
        return Err(qq_data_error(
            "QQ lyric service returned a different track than requested",
        ));
    }
    let crypt = match response.data.get("crypt") {
        None | Some(Value::Null) => 0,
        Some(value) => json_i64(value)
            .filter(|value| matches!(value, 0 | 1))
            .ok_or_else(|| qq_data_error("QQ lyric response has an invalid crypt flag"))?,
    };
    let actual_qrc = match response.data.get("qrc") {
        None | Some(Value::Null) => options.word_synced,
        Some(value) => json_bool(value)
            .ok_or_else(|| qq_data_error("QQ lyric response has an invalid qrc flag"))?,
    };
    let lyric = decode_lyric_field(response.data.get("lyric"), crypt, "lyric", true)?;
    let translated = decode_lyric_field(response.data.get("trans"), crypt, "trans", false)?;
    let romanized = decode_lyric_field(response.data.get("roma"), crypt, "roma", false)?;
    let (plain, word_synced, format) = if actual_qrc && lyric.is_some() {
        (None, lyric, "qrc")
    } else if lyric.is_some() {
        (lyric, None, "lrc")
    } else {
        (None, None, "plain")
    };
    let track_id = match requested {
        QqTrackIdentifier::Numeric(_) => song_id.to_string(),
        QqTrackIdentifier::Mid(mid) => mid.clone(),
    };
    let track_ref = ResourceRef::new(Platform::Qq, track_id).map_err(|error| {
        qq_data_error(format!(
            "QQ returned an invalid lyric track identity: {error}"
        ))
    })?;
    let mut extensions = Extensions::from([
        ("numeric_id".to_owned(), json!(song_id.to_string())),
        (
            "identifier_kind".to_owned(),
            json!(match requested {
                QqTrackIdentifier::Numeric(_) => "numeric_id",
                QqTrackIdentifier::Mid(_) => "mid",
            }),
        ),
        ("actual_qrc".to_owned(), json!(actual_qrc)),
        ("crypt".to_owned(), json!(crypt)),
        (
            "requested_options".to_owned(),
            json!({
                "qrc": options.word_synced,
                "trans": options.translated,
                "roma": options.romanized,
                "song_type": options.song_type.unwrap_or(1)
            }),
        ),
    ]);
    for (extension, upstream) in [
        ("song_name", "songName"),
        ("song_type", "songType"),
        ("singer_name", "singerName"),
        ("lrc_time", "lrc_t"),
        ("qrc_time", "qrc_t"),
        ("translated_time", "trans_t"),
        ("romanized_time", "roma_t"),
        ("lyric_style", "lyric_style"),
        ("classical", "classical"),
        ("introduction_title", "introduceTitle"),
        ("introduction_text", "introduceText"),
        ("track", "track"),
        ("start_timestamp", "startTs"),
        ("translation_source", "transSource"),
        ("has_contributor", "hasContributor"),
        ("has_translation_contributor", "hasTransContributor"),
        ("has_multiple_translations", "hasMultiTrans"),
    ] {
        insert_value(&mut extensions, extension, response.data.get(upstream));
    }
    extensions.insert("response".to_owned(), response.raw);
    Ok(Lyrics {
        track_ref,
        plain,
        translated,
        romanized,
        word_synced,
        format: format.to_owned(),
        contributors: Vec::new(),
        extensions,
    })
}

fn decode_lyric_field(
    value: Option<&Value>,
    crypt: i64,
    name: &str,
    required: bool,
) -> Result<Option<String>> {
    let Some(value) = value else {
        if required {
            return Err(qq_data_error(format!(
                "QQ lyric response is missing its {name} field"
            )));
        }
        return Ok(None);
    };
    let value = value
        .as_str()
        .ok_or_else(|| qq_data_error(format!("QQ lyric {name} field is not a string")))?;
    if value.trim().is_empty() {
        return Ok(None);
    }
    let decoded = if crypt == 1 {
        decrypt_qrc(value.trim())
            .map_err(|error| qq_data_error(format!("QQ lyric {name} decryption failed: {error}")))?
    } else {
        value.to_owned()
    };
    Ok((!decoded.trim().is_empty()).then_some(decoded))
}

fn map_cdn_dispatch_response(guid: &str, response: QqApiResponse) -> Result<AudioCdnDispatch> {
    let retcode = required_i64(&response.data, "retcode", "QQ CDN dispatch")?;
    if retcode != 0 {
        return Err(
            qq_data_error(format!("QQ CDN dispatch returned business code {retcode}"))
                .with_details(json!({ "retcode": retcode })),
        );
    }
    let roots = response
        .data
        .get("sip")
        .and_then(Value::as_array)
        .ok_or_else(|| qq_data_error("QQ CDN dispatch response is missing its sip array"))?
        .iter()
        .map(|value| {
            value
                .as_str()
                .ok_or_else(|| qq_data_error("QQ CDN dispatch sip entry is not a string"))
                .and_then(validate_cdn_root)
        })
        .collect::<Result<Vec<_>>>()?;
    if roots.is_empty() {
        return Err(qq_data_error(
            "QQ CDN dispatch response contains no CDN roots",
        ));
    }
    let nodes = match response.data.get("sipinfo") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(nodes)) => nodes.iter().map(map_cdn_node).collect::<Result<Vec<_>>>()?,
        Some(_) => {
            return Err(qq_data_error(
                "QQ CDN dispatch sipinfo field is not an array",
            ));
        }
    };
    let test_file = response
        .data
        .get("keepalivefile")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| qq_data_error("QQ CDN dispatch response is missing its keepalive file"))?;
    if reqwest::Url::parse(test_file).is_ok()
        || test_file.starts_with("//")
        || test_file.starts_with("\\\\")
    {
        return Err(qq_data_error(
            "QQ CDN dispatch keepalive file must be a relative path",
        ));
    }
    Ok(AudioCdnDispatch {
        roots,
        nodes,
        test_file: test_file.to_owned(),
        expires_in_seconds: required_positive_u64(&response.data, "expiration", "QQ CDN dispatch")?,
        refresh_after_seconds: required_positive_u64(
            &response.data,
            "refreshTime",
            "QQ CDN dispatch",
        )?,
        cache_for_seconds: required_positive_u64(&response.data, "cacheTime", "QQ CDN dispatch")?,
        extensions: Extensions::from([
            ("request_guid".to_owned(), json!(guid)),
            ("retcode".to_owned(), json!(retcode)),
            ("response".to_owned(), response.raw),
        ]),
    })
}

fn map_song_urls_response(
    requested: &[PreparedQqAudioFile],
    default_spec: &QqAudioFileSpec,
    guid: &str,
    response: QqApiResponse,
) -> Result<AudioFileBatch> {
    let expires_in_seconds = response
        .data
        .get("expiration")
        .and_then(json_u64)
        .ok_or_else(|| qq_data_error("QQ song URL response is missing a valid expiration"))?;
    let entries = response
        .data
        .get("midurlinfo")
        .and_then(Value::as_array)
        .ok_or_else(|| qq_data_error("QQ song URL response is missing its midurlinfo array"))?;
    if entries.len() != requested.len() {
        return Err(qq_data_error(format!(
            "QQ song URL response returned {} entries for {} requests",
            entries.len(),
            requested.len()
        )));
    }
    let files = requested
        .iter()
        .zip(entries)
        .map(|(request, entry)| map_song_url_entry(request, entry))
        .collect::<Result<Vec<_>>>()?;
    let (module, method) = if default_spec.encrypted {
        (ENCRYPTED_SONG_URL_MODULE, ENCRYPTED_SONG_URL_METHOD)
    } else {
        (SONG_URL_MODULE, SONG_URL_METHOD)
    };
    Ok(AudioFileBatch {
        expires_in_seconds,
        files,
        extensions: Extensions::from([
            ("default_spec".to_owned(), json!(default_spec.name)),
            ("request_guid".to_owned(), json!(guid)),
            ("module".to_owned(), json!(module)),
            ("method".to_owned(), json!(method)),
            ("response".to_owned(), response.raw),
        ]),
    })
}

fn map_song_url_entry(request: &PreparedQqAudioFile, entry: &Value) -> Result<AudioFileAccess> {
    let object = entry
        .as_object()
        .ok_or_else(|| qq_data_error("QQ song URL entry is not an object"))?;
    let mid = required_string_field(object.get("songmid"), "QQ song URL songmid")?;
    if mid != request.mid {
        return Err(qq_data_error(
            "QQ song URL response MID does not match its request",
        ));
    }
    let filename = required_string_field(object.get("filename"), "QQ song URL filename")?;
    if filename != request.filename {
        return Err(qq_data_error(
            "QQ song URL response filename does not match its request",
        ));
    }
    let relative_url = optional_string(object.get("purl"), "QQ song URL purl")?;
    let relative_url = validate_relative_media_url(&relative_url)?;
    let access_token = nonempty_optional_string(object.get("vkey"), "QQ song URL vkey")?;
    let decryption_key = nonempty_optional_string(object.get("ekey"), "QQ song URL ekey")?;
    let platform_code = object
        .get("result")
        .and_then(json_i64)
        .ok_or_else(|| qq_data_error("QQ song URL entry is missing a valid result code"))?;
    if platform_code == 0 && relative_url.is_none() {
        return Err(qq_data_error(
            "QQ song URL entry reports success without a relative URL",
        ));
    }
    if platform_code == 0 && request.spec.encrypted && decryption_key.is_none() {
        return Err(qq_data_error(
            "QQ encrypted song URL reports success without a decryption key",
        ));
    }
    let available = platform_code == 0 && relative_url.is_some();
    Ok(AudioFileAccess {
        track_ref: request.track_ref.clone(),
        spec: request.spec.name.to_owned(),
        filename,
        relative_url,
        access_token,
        decryption_key,
        available,
        encrypted: request.spec.encrypted,
        format: request.spec.extension.trim_start_matches('.').to_owned(),
        codec: request.spec.codec.to_owned(),
        bitrate: request.spec.bitrate,
        quality: request.spec.quality,
        platform_code,
        extensions: Extensions::from([
            ("request_mid".to_owned(), json!(request.mid)),
            ("request_media_id".to_owned(), json!(request.media_id)),
            ("request_song_type".to_owned(), json!(request.song_type)),
            ("response".to_owned(), entry.clone()),
        ]),
    })
}

fn required_string_field(value: Option<&Value>, context: &str) -> Result<String> {
    value
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| qq_data_error(format!("{context} field is not a string")))
}

fn validate_relative_media_url(value: &str) -> Result<Option<String>> {
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }
    if reqwest::Url::parse(value).is_ok() || value.starts_with("//") || value.starts_with("\\\\") {
        return Err(qq_data_error("QQ song URL purl must be a relative URL"));
    }
    Ok(Some(value.to_owned()))
}

fn map_cdn_node(value: &Value) -> Result<AudioCdnNode> {
    let object = value
        .as_object()
        .ok_or_else(|| qq_data_error("QQ CDN dispatch node is not an object"))?;
    let url = optional_string(object.get("cdn"), "QQ CDN dispatch node cdn")?;
    let url = if url.trim().is_empty() {
        String::new()
    } else {
        validate_cdn_root(&url)?
    };
    Ok(AudioCdnNode {
        url,
        quic: optional_u64(object.get("quic"), "QQ CDN dispatch node quic")?,
        ip_stack: optional_u64(object.get("ipstack"), "QQ CDN dispatch node ipstack")?,
        quic_host: nonempty_optional_string(
            object.get("quichost"),
            "QQ CDN dispatch node quichost",
        )?,
        plaintext_quic: optional_u64(
            object.get("plaintextquic"),
            "QQ CDN dispatch node plaintextquic",
        )?,
        encrypted_quic: optional_u64(
            object.get("encryptquic"),
            "QQ CDN dispatch node encryptquic",
        )?,
        extensions: Extensions::from([("response".to_owned(), value.clone())]),
    })
}

fn validate_cdn_root(value: &str) -> Result<String> {
    let value = value.trim();
    let url = reqwest::Url::parse(value)
        .map_err(|_| qq_data_error("QQ CDN dispatch returned an invalid CDN root URL"))?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(qq_data_error(
            "QQ CDN dispatch returned an unsafe CDN root URL",
        ));
    }
    Ok(value.to_owned())
}

fn required_i64(value: &Value, field: &str, context: &str) -> Result<i64> {
    value
        .get(field)
        .and_then(json_i64)
        .ok_or_else(|| qq_data_error(format!("{context} is missing a valid {field} field")))
}

fn required_positive_u64(value: &Value, field: &str, context: &str) -> Result<u64> {
    value
        .get(field)
        .and_then(json_u64)
        .filter(|value| *value > 0)
        .ok_or_else(|| qq_data_error(format!("{context} is missing a positive {field} field")))
}

fn optional_u64(value: Option<&Value>, context: &str) -> Result<u64> {
    match value {
        None | Some(Value::Null) => Ok(0),
        Some(value) => json_u64(value)
            .ok_or_else(|| qq_data_error(format!("{context} field is not an unsigned integer"))),
    }
}

fn optional_string(value: Option<&Value>, context: &str) -> Result<String> {
    match value {
        None | Some(Value::Null) => Ok(String::new()),
        Some(Value::String(value)) => Ok(value.clone()),
        Some(_) => Err(qq_data_error(format!("{context} field is not a string"))),
    }
}

fn nonempty_optional_string(value: Option<&Value>, context: &str) -> Result<Option<String>> {
    let value = optional_string(value, context)?.trim().to_owned();
    Ok((!value.is_empty()).then_some(value))
}

fn qq_authentication_required(account: &str, message: &str) -> TuneWeaveError {
    TuneWeaveError::new(ErrorCode::AuthenticationRequired, message)
        .with_platform(Platform::Qq)
        .with_details(json!({ "account": account }))
}

fn validate_song_detail_info(info: Option<&Value>) -> Result<()> {
    let Some(info) = info.filter(|value| !value.is_null()) else {
        return Ok(());
    };
    let info = info
        .as_object()
        .ok_or_else(|| qq_data_error("QQ song detail info field is not an object"))?;
    for name in ["company", "genre", "intro", "lan", "pub_time"] {
        let Some(section) = info.get(name).filter(|value| !value.is_null()) else {
            continue;
        };
        let section = section.as_object().ok_or_else(|| {
            qq_data_error(format!("QQ song detail {name} section is not an object"))
        })?;
        let Some(content) = section.get("content").filter(|value| !value.is_null()) else {
            continue;
        };
        let content = content.as_array().ok_or_else(|| {
            qq_data_error(format!(
                "QQ song detail {name} content field is not an array"
            ))
        })?;
        for item in content {
            let valid = item.get("id").and_then(json_i64).is_some()
                && item.get("value").and_then(Value::as_str).is_some()
                && item.get("show_type").and_then(json_i64).is_some()
                && item.get("jumpurl").and_then(Value::as_str).is_some();
            if !valid {
                return Err(qq_data_error(format!(
                    "QQ song detail {name} contains an invalid content item"
                )));
            }
        }
    }
    Ok(())
}

fn track_matches_identifier(track: &Track, identifier: &QqTrackIdentifier) -> bool {
    match identifier {
        QqTrackIdentifier::Numeric(id) => {
            track
                .extensions
                .get("numeric_id")
                .and_then(|value| value_as_string(Some(value)))
                .and_then(|value| value.parse::<u64>().ok())
                == Some(*id)
        }
        QqTrackIdentifier::Mid(mid) => track
            .extensions
            .get("mid")
            .and_then(Value::as_str)
            .is_some_and(|value| value == mid),
    }
}

fn qq_track_identifier_value(identifier: &QqTrackIdentifier) -> String {
    match identifier {
        QqTrackIdentifier::Numeric(value) => value.to_string(),
        QqTrackIdentifier::Mid(value) => value.clone(),
    }
}

fn qq_track_not_found(id: &str) -> TuneWeaveError {
    TuneWeaveError::new(ErrorCode::ResourceNotFound, "QQ track was not found")
        .with_platform(Platform::Qq)
        .with_details(json!({ "id": id }))
}

fn map_hotkey_response(
    detail: SearchTrendingDetail,
    requested_search_id: &str,
    response: QqApiResponse,
) -> Result<SearchTrendingList> {
    let code = response
        .data
        .get("ret_code")
        .and_then(json_i64)
        .ok_or_else(|| qq_data_error("QQ hotkey response is missing ret_code"))?;
    if code != 0 {
        return Err(TuneWeaveError::new(
            ErrorCode::UpstreamError,
            format!("QQ hotkey service failed with code {code}"),
        )
        .with_platform(Platform::Qq)
        .with_details(json!({ "platform_code": code })));
    }
    let hotkeys = response
        .data
        .get("vec_hotkey")
        .and_then(Value::as_array)
        .ok_or_else(|| qq_data_error("QQ hotkey response is missing vec_hotkey"))?;
    let entries = hotkeys
        .iter()
        .enumerate()
        .map(|(index, raw)| map_hotkey_entry(detail, index, raw.clone()))
        .collect::<Result<Vec<_>>>()?;
    let mut extensions = Extensions::new();
    extensions.insert(
        "search_id".to_owned(),
        Value::String(requested_search_id.to_owned()),
    );
    insert_value(&mut extensions, "experiment_id", response.data.get("expid"));
    insert_value(
        &mut extensions,
        "hotkey_time",
        response.data.get("hotkey_time"),
    );
    insert_value(
        &mut extensions,
        "track_list_id",
        response.data.get("track_list_id"),
    );
    extensions.insert("response".to_owned(), response.raw);
    Ok(SearchTrendingList {
        detail,
        entries,
        extensions,
    })
}

fn map_hotkey_entry(
    detail: SearchTrendingDetail,
    index: usize,
    raw: Value,
) -> Result<SearchTrendingEntry> {
    let keyword = ["query", "title"]
        .into_iter()
        .find_map(|field| nonempty_string(raw.get(field)))
        .ok_or_else(|| qq_data_error("QQ hotkey entry is missing its query"))?;
    let full = detail == SearchTrendingDetail::Full;
    let mut extensions = Extensions::new();
    insert_some(
        &mut extensions,
        "display_title",
        nonempty_string(raw.get("title")),
    );
    insert_some(
        &mut extensions,
        "cover_url",
        ["cover_pic_url", "pic_url"]
            .into_iter()
            .find_map(|field| nonempty_string(raw.get(field))),
    );
    insert_value(&mut extensions, "hotkey_id", raw.get("hotkey_id"));
    insert_value(&mut extensions, "direct_id", raw.get("direct_id"));
    insert_value(
        &mut extensions,
        "track_id",
        raw.pointer("/custom_param/track_id"),
    );
    insert_value(&mut extensions, "kind", raw.get("kind"));
    insert_value(&mut extensions, "need_top", raw.get("need_top"));
    insert_value(&mut extensions, "order_info", raw.get("order_info"));
    insert_value(&mut extensions, "sequence", raw.get("seqence"));
    insert_value(&mut extensions, "source", raw.get("source"));
    insert_value(&mut extensions, "type", raw.get("type"));
    extensions.insert("response".to_owned(), raw.clone());
    Ok(SearchTrendingEntry {
        rank: u32::try_from(index.saturating_add(1)).unwrap_or(u32::MAX),
        keyword,
        description: full
            .then(|| nonempty_string(raw.get("description")))
            .flatten(),
        score: full.then(|| raw.get("score").and_then(json_u64)).flatten(),
        icon_type: full
            .then(|| raw.pointer("/seqence/seqence_kind").and_then(json_i64))
            .flatten(),
        icon_url: full
            .then(|| {
                ["desc_icon_url", "gif_url"]
                    .into_iter()
                    .find_map(|field| nonempty_string(raw.get(field)))
            })
            .flatten(),
        target_url: full.then(|| nonempty_string(raw.get("jump_url"))).flatten(),
        extensions,
    })
}

fn map_track_search_response(
    offset: u32,
    limit: u32,
    skip: u32,
    responses: Vec<QqApiResponse>,
) -> Result<Page<Track>> {
    let (raw_items, pagination) =
        collect_search_items(offset, limit, skip, responses, TRACK_SEARCH)?;
    let items = raw_items
        .into_iter()
        .map(map_track)
        .collect::<Result<Vec<_>>>()?;
    Ok(Page { items, pagination })
}

fn map_catalog_search_response(
    offset: u32,
    limit: u32,
    skip: u32,
    responses: Vec<QqApiResponse>,
    spec: TypedSearchSpec,
    mapper: SearchItemMapper,
) -> Result<Page<SearchItem>> {
    let (raw_items, pagination) = collect_search_items(offset, limit, skip, responses, spec)?;
    let items = raw_items
        .into_iter()
        .map(mapper)
        .collect::<Result<Vec<_>>>()?;
    Ok(Page { items, pagination })
}

fn collect_search_items(
    offset: u32,
    limit: u32,
    skip: u32,
    responses: Vec<QqApiResponse>,
    spec: TypedSearchSpec,
) -> Result<(Vec<Value>, PageMeta)> {
    let first = responses
        .first()
        .ok_or_else(|| qq_data_error(format!("{} returned no response", spec.context)))?;
    ensure_data_success(&first.data, spec.context)?;
    let total = first
        .data
        .pointer("/meta/sum")
        .and_then(json_u64)
        .ok_or_else(|| {
            qq_data_error(format!("{} response is missing total count", spec.context))
        })?;
    let window_start = u64::from(offset);
    let window_end = if total <= window_start {
        window_start
    } else {
        window_start.saturating_add(u64::from(limit)).min(total)
    };
    let first_page_start = window_start.saturating_sub(u64::from(skip));
    let mut available = Vec::new();
    let mut upstream_item_counts = Vec::with_capacity(responses.len());
    let mut omitted_slots = 0_u64;
    for (index, response) in responses.iter().enumerate() {
        ensure_data_success(&response.data, spec.context)?;
        let response_total = response
            .data
            .pointer("/meta/sum")
            .and_then(json_u64)
            .ok_or_else(|| {
                qq_data_error(format!("{} response is missing total count", spec.context))
            })?;
        if response_total != total {
            return Err(qq_data_error(format!(
                "{} returned inconsistent total counts",
                spec.context
            )));
        }
        let items = response
            .data
            .pointer(spec.item_pointer)
            .and_then(Value::as_array)
            .ok_or_else(|| {
                qq_data_error(format!(
                    "{} response is missing {}",
                    spec.context, spec.item_pointer
                ))
            })?;
        if items.len() > usize::try_from(spec.upstream_page_size).unwrap_or(usize::MAX) {
            return Err(qq_data_error(format!(
                "{} returned more items than its requested page size",
                spec.context
            )));
        }
        upstream_item_counts.push(items.len());
        let page_start = first_page_start.saturating_add(
            u64::try_from(index)
                .unwrap_or(u64::MAX)
                .saturating_mul(u64::from(spec.upstream_page_size)),
        );
        let slot_start = page_start.max(window_start);
        let slot_end = page_start
            .saturating_add(u64::from(spec.upstream_page_size))
            .min(window_end);
        if slot_start >= slot_end {
            continue;
        }
        let item_start = usize::try_from(slot_start.saturating_sub(page_start))
            .unwrap_or(usize::MAX)
            .min(items.len());
        let item_end = usize::try_from(slot_end.saturating_sub(page_start))
            .unwrap_or(usize::MAX)
            .min(items.len());
        available.extend(items[item_start..item_end].iter().cloned());
        let requested_slots = slot_end.saturating_sub(slot_start);
        let returned_slots = u64::try_from(item_end.saturating_sub(item_start)).unwrap_or(u64::MAX);
        omitted_slots =
            omitted_slots.saturating_add(requested_slots.saturating_sub(returned_slots));
    }
    if !spec.sparse && omitted_slots > 0 {
        return Err(qq_data_error(format!(
            "{} omitted items inside the requested result window",
            spec.context
        )));
    }
    let next_offset = u32::try_from(window_end).ok();
    let has_more = window_end < total && next_offset.is_some_and(|next| next > offset);
    let mut extensions = Extensions::new();
    extensions.insert(
        "upstream_page_size".to_owned(),
        json!(spec.upstream_page_size),
    );
    extensions.insert("pagination_basis".to_owned(), json!("upstream_slots"));
    extensions.insert("omitted_slots".to_owned(), json!(omitted_slots));
    extensions.insert(
        "upstream_item_counts".to_owned(),
        json!(upstream_item_counts),
    );
    extensions.insert(
        "upstream_responses".to_owned(),
        Value::Array(responses.into_iter().map(|response| response.raw).collect()),
    );
    Ok((
        available,
        PageMeta {
            limit,
            offset,
            total: Some(total),
            next_offset: has_more.then_some(next_offset.expect("checked above")),
            has_more,
            extensions,
        },
    ))
}

fn map_track(raw: Value) -> Result<Track> {
    let mid = nonempty_string(raw.get("mid"));
    let numeric_id = value_as_string(raw.get("id"));
    let id = mid
        .clone()
        .or_else(|| numeric_id.clone())
        .ok_or_else(|| qq_data_error("QQ track search item is missing both MID and numeric ID"))?;
    let name = ["title_main", "title", "name"]
        .into_iter()
        .find_map(|field| nonempty_string(raw.get(field)))
        .ok_or_else(|| qq_data_error("QQ track search item is missing its title"))?;
    let resource_ref = qq_ref(&id, "track")?;
    let artists = raw
        .get("singer")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|artist| map_artist_summary(artist).transpose())
        .collect::<Result<Vec<_>>>()?;
    let album = raw
        .get("album")
        .map(map_album_summary)
        .transpose()?
        .flatten();
    let duration_ms = raw
        .get("interval")
        .and_then(json_u64)
        .map(|seconds| seconds.saturating_mul(1_000));
    let mv_ref = raw
        .get("mv")
        .and_then(|mv| nonempty_string(mv.get("vid")).or_else(|| value_as_string(mv.get("id"))))
        .filter(|id| id != "0")
        .map(|id| qq_ref(&id, "MV"))
        .transpose()?;
    let file = raw.get("file").cloned().unwrap_or(Value::Null);
    let available_qualities = map_available_qualities(&file);
    let mut aliases = Vec::new();
    if let Some(subtitle) = nonempty_string(raw.get("subtitle")) {
        aliases.push(subtitle);
    }
    if let Some(title_extra) = nonempty_string(raw.get("title_extra")) {
        if !aliases.contains(&title_extra) {
            aliases.push(title_extra);
        }
    }
    let mut extensions = Extensions::new();
    insert_some(&mut extensions, "numeric_id", numeric_id);
    insert_some(&mut extensions, "mid", mid);
    insert_some(
        &mut extensions,
        "media_mid",
        nonempty_string(file.get("media_mid")),
    );
    insert_value(&mut extensions, "song_type", raw.get("type"));
    insert_value(&mut extensions, "status", raw.get("status"));
    insert_value(&mut extensions, "pay", raw.get("pay"));
    insert_value(&mut extensions, "file", raw.get("file"));
    insert_value(&mut extensions, "search_content", raw.get("content"));
    extensions.insert("search_item".to_owned(), raw);
    Ok(Track {
        resource_ref,
        platform: Platform::Qq,
        id,
        name,
        aliases,
        artists,
        album,
        duration_ms,
        isrc: None,
        mv_ref,
        playable: None,
        available_qualities,
        extensions,
    })
}

fn map_artist_search_item(raw: Value) -> Result<SearchItem> {
    let mid = ["mid", "singerMID", "singerMid", "singer_mid"]
        .into_iter()
        .find_map(|field| nonempty_string(raw.get(field)));
    let numeric_id = ["id", "singerID", "singerId", "singer_id"]
        .into_iter()
        .find_map(|field| value_as_string(raw.get(field)));
    let id = mid
        .clone()
        .or_else(|| numeric_id.clone())
        .ok_or_else(|| qq_data_error("QQ artist search item is missing both MID and numeric ID"))?;
    let name = ["name", "title", "singerName"]
        .into_iter()
        .find_map(|field| nonempty_string(raw.get(field)))
        .ok_or_else(|| qq_data_error("QQ artist search item is missing its name"))?;
    let avatar_url = ["singerPic", "pic", "avatar"]
        .into_iter()
        .find_map(|field| nonempty_string(raw.get(field)))
        .or_else(|| mid.as_deref().map(|mid| qq_cover_url("T001", mid)));
    let mut extensions = Extensions::new();
    insert_some(&mut extensions, "numeric_id", numeric_id);
    insert_some(&mut extensions, "mid", mid);
    insert_value(&mut extensions, "type", raw.get("type"));
    insert_value(&mut extensions, "identity", raw.get("identity"));
    insert_value(&mut extensions, "followed", raw.get("isFollow"));
    insert_value(&mut extensions, "uin", raw.get("uin"));
    insert_value(&mut extensions, "pmid", raw.get("pmid"));
    extensions.insert("search_item".to_owned(), raw.clone());
    Ok(SearchItem::Artist(Artist {
        resource_ref: qq_ref(&id, "artist")?,
        platform: Platform::Qq,
        id,
        name,
        aliases: Vec::new(),
        description: nonempty_string(raw.get("subtitle")).unwrap_or_default(),
        biography_sections: Vec::new(),
        avatar_url,
        cover_url: None,
        album_count: ["albumNum", "album_num"]
            .into_iter()
            .find_map(|field| raw.get(field).and_then(json_u64)),
        track_count: ["songNum", "song_num"]
            .into_iter()
            .find_map(|field| raw.get(field).and_then(json_u64)),
        mv_count: ["mvNum", "mv_num"]
            .into_iter()
            .find_map(|field| raw.get(field).and_then(json_u64)),
        video_count: None,
        identities: Vec::new(),
        extensions,
    }))
}

fn map_album_search_item(raw: Value) -> Result<SearchItem> {
    let mid = ["mid", "albumMid", "albumMID", "albummid"]
        .into_iter()
        .find_map(|field| nonempty_string(raw.get(field)));
    let numeric_id = ["id", "albumID"]
        .into_iter()
        .find_map(|field| value_as_string(raw.get(field)));
    let id = mid
        .clone()
        .or_else(|| numeric_id.clone())
        .ok_or_else(|| qq_data_error("QQ album search item is missing both MID and numeric ID"))?;
    let name = ["name", "title", "albumName"]
        .into_iter()
        .find_map(|field| nonempty_string(raw.get(field)))
        .ok_or_else(|| qq_data_error("QQ album search item is missing its name"))?;
    let aliases = ["subtitle", "albumTranName"]
        .into_iter()
        .find_map(|field| nonempty_string(raw.get(field)))
        .into_iter()
        .collect();
    let artists = raw
        .get("singer_list")
        .or_else(|| raw.get("singerList"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|artist| map_artist_summary(artist).transpose())
        .collect::<Result<Vec<_>>>()?;
    let cover_url = ["pic", "picurl", "cover_url"]
        .into_iter()
        .find_map(|field| nonempty_string(raw.get(field)))
        .or_else(|| {
            ["pmid", "logo"]
                .into_iter()
                .find_map(|field| nonempty_string(raw.get(field)))
                .or_else(|| mid.clone())
                .map(|pmid| qq_cover_url("T002", &pmid))
        });
    let description = raw
        .pointer("/desc_detail/desc")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .or_else(|| nonempty_string(raw.get("description2")))
        .unwrap_or_default();
    let mut extensions = Extensions::new();
    insert_some(&mut extensions, "numeric_id", numeric_id);
    insert_some(&mut extensions, "mid", mid);
    insert_value(&mut extensions, "pmid", raw.get("pmid"));
    insert_value(
        &mut extensions,
        "album_type",
        raw.pointer("/core_album_config/album_type")
            .or_else(|| raw.get("type")),
    );
    insert_value(
        &mut extensions,
        "award_label",
        raw.pointer("/core_album_config/award_label")
            .or_else(|| raw.get("award_label")),
    );
    insert_value(&mut extensions, "hotness", raw.get("hotness"));
    insert_value(&mut extensions, "audio_play", raw.get("audio_play"));
    extensions.insert("search_item".to_owned(), raw.clone());
    Ok(SearchItem::Album(Album {
        resource_ref: qq_ref(&id, "album")?,
        platform: Platform::Qq,
        id,
        name,
        aliases,
        artists,
        description,
        cover_url,
        published_at: ["time_public", "publish_date", "publishDate"]
            .into_iter()
            .find_map(|field| nonempty_string(raw.get(field))),
        track_count: ["song_num", "songNum", "songnum"]
            .into_iter()
            .find_map(|field| raw.get(field).and_then(json_u64)),
        company: nonempty_string(raw.get("company")),
        kind: raw
            .pointer("/core_album_config/album_type")
            .and_then(|value| value_as_string(Some(value)))
            .or_else(|| value_as_string(raw.get("type"))),
        extensions,
    }))
}

fn map_playlist_search_item(raw: Value) -> Result<SearchItem> {
    let id = ["id", "dissid", "tid"]
        .into_iter()
        .find_map(|field| value_as_string(raw.get(field)))
        .filter(|value| value != "0")
        .ok_or_else(|| qq_data_error("QQ playlist search item is missing its ID"))?;
    let name = ["title", "name", "dissname"]
        .into_iter()
        .find_map(|field| nonempty_string(raw.get(field)))
        .ok_or_else(|| qq_data_error("QQ playlist search item is missing its name"))?;
    let creator = raw
        .get("creator")
        .map(map_playlist_creator)
        .transpose()?
        .flatten()
        .or(map_playlist_creator(&raw)?);
    let tags = raw
        .get("tags")
        .or_else(|| raw.get("tag_list"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|tag| match tag {
            Value::String(tag) => Some(tag.as_str()),
            Value::Object(tag) => tag
                .get("name")
                .or_else(|| tag.get("title"))
                .and_then(Value::as_str),
            _ => None,
        })
        .map(str::trim)
        .filter(|tag| !tag.is_empty())
        .map(str::to_owned)
        .collect();
    let mut extensions = Extensions::new();
    insert_value(&mut extensions, "dir_id", raw.get("dirid"));
    insert_value(&mut extensions, "dir_type", raw.get("dirtype"));
    insert_value(&mut extensions, "listen_count", raw.get("listennum"));
    insert_value(&mut extensions, "nickname", raw.get("nickname"));
    insert_value(&mut extensions, "uin", raw.get("uin"));
    insert_value(&mut extensions, "type", raw.get("type"));
    insert_value(&mut extensions, "hotness", raw.get("hotness"));
    extensions.insert("search_item".to_owned(), raw.clone());
    Ok(SearchItem::Playlist(Playlist {
        resource_ref: qq_ref(&id, "playlist")?,
        platform: Platform::Qq,
        id,
        name,
        description: ["subhead", "description", "desc"]
            .into_iter()
            .find_map(|field| nonempty_string(raw.get(field)))
            .unwrap_or_default(),
        cover_url: ["picurl", "logo", "cover_url"]
            .into_iter()
            .find_map(|field| nonempty_string(raw.get(field))),
        creator,
        track_count: ["songnum", "song_num", "songNum"]
            .into_iter()
            .find_map(|field| raw.get(field).and_then(json_u64)),
        tags,
        subscribed: None,
        created_at: nonempty_string(raw.get("createtime")),
        updated_at: nonempty_string(raw.get("modifytime")),
        extensions,
    }))
}

fn map_mv_search_item(raw: Value) -> Result<SearchItem> {
    let vid = nonempty_string(raw.get("vid"));
    let numeric_id = ["id", "mvid", "sid"]
        .into_iter()
        .find_map(|field| value_as_string(raw.get(field)));
    let id = vid
        .clone()
        .or_else(|| numeric_id.clone())
        .ok_or_else(|| qq_data_error("QQ MV search item is missing both VID and numeric ID"))?;
    let title = ["title", "name", "mvname"]
        .into_iter()
        .find_map(|field| nonempty_string(raw.get(field)))
        .ok_or_else(|| qq_data_error("QQ MV search item is missing its title"))?;
    let singer_mid = ["singermid", "singerMid", "singer_mid"]
        .into_iter()
        .find_map(|field| nonempty_string(raw.get(field)));
    let singer_id = ["singerid", "singerId", "singer_id"]
        .into_iter()
        .find_map(|field| value_as_string(raw.get(field)));
    let creators = ["singername", "singerName", "singer_name"]
        .into_iter()
        .find_map(|field| nonempty_string(raw.get(field)))
        .map(|name| {
            let creator_id = singer_mid.clone().or_else(|| singer_id.clone());
            Ok(CreatorSummary {
                resource_ref: creator_id
                    .as_deref()
                    .map(|id| qq_ref(id, "MV creator"))
                    .transpose()?,
                name,
                avatar_url: singer_mid.as_deref().map(|mid| qq_cover_url("T001", mid)),
            })
        })
        .transpose()?
        .into_iter()
        .collect();
    let mut extensions = Extensions::new();
    insert_some(&mut extensions, "numeric_id", numeric_id);
    insert_some(&mut extensions, "vid", vid);
    insert_some(&mut extensions, "singer_numeric_id", singer_id);
    insert_some(&mut extensions, "singer_mid", singer_mid);
    insert_value(
        &mut extensions,
        "mv_type",
        raw.get("type").or_else(|| raw.get("vt")),
    );
    extensions.insert("search_item".to_owned(), raw.clone());
    Ok(SearchItem::Video(Video {
        resource_ref: qq_ref(&id, "MV")?,
        platform: Platform::Qq,
        id,
        title,
        creators,
        description: ["desc", "description"]
            .into_iter()
            .find_map(|field| nonempty_string(raw.get(field)))
            .unwrap_or_default(),
        cover_url: ["pic", "cover", "picurl"]
            .into_iter()
            .find_map(|field| nonempty_string(raw.get(field))),
        duration_ms: raw
            .get("duration")
            .and_then(json_u64)
            .map(|seconds| seconds.saturating_mul(1_000)),
        published_at: ["publish_date", "publishDate", "pubdate"]
            .into_iter()
            .find_map(|field| nonempty_string(raw.get(field))),
        play_count: ["play_count", "playCount", "listennum"]
            .into_iter()
            .find_map(|field| raw.get(field).and_then(json_u64)),
        subscribed: None,
        extensions,
    }))
}

fn map_lyric_search_item(raw: Value) -> Result<SearchItem> {
    map_track(raw).map(SearchItem::Track)
}

fn map_user_search_item(raw: Value) -> Result<SearchItem> {
    let encrypted_id = [
        "encrypt_uin",
        "encryptUin",
        "EncryptUin",
        "encrypted_uin",
        "euin",
    ]
    .into_iter()
    .find_map(|field| nonempty_string(raw.get(field)));
    let numeric_id = ["uin", "Uin", "id"]
        .into_iter()
        .find_map(|field| value_as_string(raw.get(field)));
    let id = encrypted_id
        .clone()
        .or_else(|| numeric_id.clone())
        .ok_or_else(|| qq_data_error("QQ user search item is missing its account ID"))?;
    let name = ["nick", "nickname", "name", "Name", "NickName"]
        .into_iter()
        .find_map(|field| nonempty_string(raw.get(field)))
        .ok_or_else(|| qq_data_error("QQ user search item is missing its name"))?;
    let mut extensions = Extensions::new();
    insert_some(&mut extensions, "encrypted_uin", encrypted_id);
    insert_some(&mut extensions, "numeric_uin", numeric_id);
    insert_value(&mut extensions, "user_type", raw.get("user_type"));
    insert_value(&mut extensions, "identity", raw.get("identity"));
    extensions.insert("search_item".to_owned(), raw.clone());
    Ok(SearchItem::User(User {
        resource_ref: qq_ref(&id, "user")?,
        platform: Platform::Qq,
        id,
        name,
        avatar_url: ["avatar", "avatar_url", "Avatar", "AvatarUrl", "pic"]
            .into_iter()
            .find_map(|field| nonempty_string(raw.get(field))),
        signature: ["signature", "Signature", "desc"]
            .into_iter()
            .find_map(|field| nonempty_string(raw.get(field))),
        followed: ["isFollow", "followed", "is_follow"]
            .into_iter()
            .find_map(|field| raw.get(field).and_then(json_bool)),
        mutual: ["isMutual", "mutual", "is_mutual"]
            .into_iter()
            .find_map(|field| raw.get(field).and_then(json_bool)),
        extensions,
    }))
}

fn map_podcast_search_item(raw: Value) -> Result<SearchItem> {
    let mid = ["mid", "albumMid", "albumMID", "albummid"]
        .into_iter()
        .find_map(|field| nonempty_string(raw.get(field)));
    let numeric_id = ["id", "albumID"]
        .into_iter()
        .find_map(|field| value_as_string(raw.get(field)));
    let id = mid.clone().or_else(|| numeric_id.clone()).ok_or_else(|| {
        qq_data_error("QQ podcast search item is missing both MID and numeric ID")
    })?;
    let name = ["name", "title", "albumName"]
        .into_iter()
        .find_map(|field| nonempty_string(raw.get(field)))
        .ok_or_else(|| qq_data_error("QQ podcast search item is missing its name"))?;
    let creator = raw
        .get("singer_list")
        .or_else(|| raw.get("singerList"))
        .and_then(Value::as_array)
        .map(|creators| first_creator(creators))
        .transpose()?
        .flatten();
    let cover_url = ["pic", "picurl", "cover_url"]
        .into_iter()
        .find_map(|field| nonempty_string(raw.get(field)))
        .or_else(|| {
            ["pmid", "logo"]
                .into_iter()
                .find_map(|field| nonempty_string(raw.get(field)))
                .or_else(|| mid.clone())
                .map(|pmid| qq_cover_url("T002", &pmid))
        });
    let mut extensions = Extensions::new();
    insert_some(&mut extensions, "numeric_id", numeric_id);
    insert_some(&mut extensions, "mid", mid);
    insert_value(&mut extensions, "audio_play", raw.get("audio_play"));
    insert_value(&mut extensions, "hotness", raw.get("hotness"));
    insert_value(
        &mut extensions,
        "album_config",
        raw.get("core_album_config"),
    );
    extensions.insert("search_item".to_owned(), raw.clone());
    Ok(SearchItem::Podcast(Podcast {
        resource_ref: qq_ref(&id, "podcast")?,
        platform: Platform::Qq,
        id,
        name,
        description: raw
            .pointer("/desc_detail/desc")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
            .or_else(|| {
                ["description2", "description"]
                    .into_iter()
                    .find_map(|field| nonempty_string(raw.get(field)))
            })
            .unwrap_or_default(),
        cover_url,
        creator,
        category: nonempty_string(raw.get("category")),
        secondary_category: None,
        episode_count: ["song_num", "songNum", "songnum"]
            .into_iter()
            .find_map(|field| raw.get(field).and_then(json_u64)),
        subscriber_count: None,
        play_count: raw.pointer("/audio_play/play_num").and_then(json_u64),
        subscribed: None,
        paid: None,
        purchased: None,
        price: None,
        created_at: ["publish_date", "time_public", "publishDate"]
            .into_iter()
            .find_map(|field| nonempty_string(raw.get(field))),
        extensions,
    }))
}

fn map_voice_search_item(raw: Value) -> Result<SearchItem> {
    let track = map_track(raw.clone())?;
    let creator = raw
        .get("singer")
        .and_then(Value::as_array)
        .map(|creators| first_creator(creators))
        .transpose()?
        .flatten();
    let podcast_ref = raw
        .get("album")
        .and_then(|album| {
            nonempty_string(album.get("mid")).or_else(|| value_as_string(album.get("id")))
        })
        .filter(|id| id != "0")
        .map(|id| qq_ref(&id, "podcast"))
        .transpose()?;
    let mut extensions = Extensions::new();
    insert_value(&mut extensions, "song_type", raw.get("type"));
    insert_value(&mut extensions, "pay", raw.get("pay"));
    extensions.insert("search_item".to_owned(), raw.clone());
    Ok(SearchItem::PodcastEpisode(Box::new(PodcastEpisode {
        resource_ref: track.resource_ref.clone(),
        platform: Platform::Qq,
        id: track.id.clone(),
        podcast_ref,
        name: track.name.clone(),
        description: ["desc", "content"]
            .into_iter()
            .find_map(|field| nonempty_string(raw.get(field)))
            .unwrap_or_default(),
        cover_url: track
            .album
            .as_ref()
            .and_then(|album| album.cover_url.clone()),
        creator,
        duration_ms: track.duration_ms,
        published_at: ["time_public", "publish_date"]
            .into_iter()
            .find_map(|field| nonempty_string(raw.get(field))),
        serial_number: ["index_album", "index"]
            .into_iter()
            .find_map(|field| raw.get(field).and_then(json_u64)),
        listener_count: ["play_count", "listennum"]
            .into_iter()
            .find_map(|field| raw.get(field).and_then(json_u64)),
        liked_count: None,
        comment_count: None,
        share_count: None,
        subscribed: None,
        has_lyrics: None,
        paid: None,
        purchased: None,
        audio: Some(track),
        extensions,
    })))
}

fn map_creator(raw: &Value) -> Result<Option<CreatorSummary>> {
    let Some(name) = ["name", "title", "singerName", "nickname", "nick"]
        .into_iter()
        .find_map(|field| nonempty_string(raw.get(field)))
    else {
        return Ok(None);
    };
    let mid = ["mid", "singerMid", "singerMID", "singer_mid"]
        .into_iter()
        .find_map(|field| nonempty_string(raw.get(field)));
    let id = mid.clone().or_else(|| {
        ["id", "singerId", "singerID", "uin"]
            .into_iter()
            .find_map(|field| value_as_string(raw.get(field)))
            .filter(|id| id != "0")
    });
    Ok(Some(CreatorSummary {
        resource_ref: id.as_deref().map(|id| qq_ref(id, "creator")).transpose()?,
        name,
        avatar_url: ["avatar", "avatar_url", "pic", "singerPic"]
            .into_iter()
            .find_map(|field| nonempty_string(raw.get(field)))
            .or_else(|| mid.as_deref().map(|mid| qq_cover_url("T001", mid))),
    }))
}

fn first_creator(values: &[Value]) -> Result<Option<CreatorSummary>> {
    for value in values {
        if let Some(creator) = map_creator(value)? {
            return Ok(Some(creator));
        }
    }
    Ok(None)
}

fn map_playlist_creator(raw: &Value) -> Result<Option<ArtistSummary>> {
    let Some(name) = ["name", "nickname", "nick"]
        .into_iter()
        .find_map(|field| nonempty_string(raw.get(field)))
    else {
        return Ok(None);
    };
    let id = ["uin", "id"]
        .into_iter()
        .find_map(|field| value_as_string(raw.get(field)))
        .filter(|id| id != "0");
    Ok(Some(ArtistSummary {
        resource_ref: id.map(|id| qq_ref(&id, "playlist creator")).transpose()?,
        name,
    }))
}

fn map_artist_summary(raw: &Value) -> Result<Option<ArtistSummary>> {
    let Some(name) = ["name", "title", "singerName"]
        .into_iter()
        .find_map(|field| nonempty_string(raw.get(field)))
    else {
        return Ok(None);
    };
    let id = nonempty_string(raw.get("mid")).or_else(|| value_as_string(raw.get("id")));
    Ok(Some(ArtistSummary {
        resource_ref: id.map(|id| qq_ref(&id, "artist")).transpose()?,
        name,
    }))
}

fn map_album_summary(raw: &Value) -> Result<Option<AlbumSummary>> {
    let Some(name) = ["name", "title", "albumName"]
        .into_iter()
        .find_map(|field| nonempty_string(raw.get(field)))
    else {
        return Ok(None);
    };
    let mid = nonempty_string(raw.get("mid"));
    let id = mid.clone().or_else(|| value_as_string(raw.get("id")));
    Ok(Some(AlbumSummary {
        resource_ref: id.map(|id| qq_ref(&id, "album")).transpose()?,
        name,
        cover_url: mid.map(|mid| qq_cover_url("T002", &mid)),
    }))
}

fn map_available_qualities(file: &Value) -> Vec<Quality> {
    let mut qualities = Vec::new();
    push_quality(
        &mut qualities,
        Quality::Low,
        any_positive(
            file,
            &["size_24aac", "size_48aac", "size_96aac", "size_96ogg"],
        ),
    );
    push_quality(
        &mut qualities,
        Quality::Standard,
        any_positive(file, &["size_128mp3"]),
    );
    push_quality(
        &mut qualities,
        Quality::High,
        any_positive(file, &["size_192ogg", "size_192aac", "size_320mp3"]),
    );
    push_quality(
        &mut qualities,
        Quality::Lossless,
        any_positive(file, &["size_flac"]),
    );
    let modern = file
        .get("size_new")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    push_quality(&mut qualities, Quality::Master, positive_index(modern, 0));
    push_quality(
        &mut qualities,
        Quality::Surround,
        positive_index(modern, 2) || positive_index(modern, 6),
    );
    push_quality(
        &mut qualities,
        Quality::Dolby,
        any_positive(file, &["size_dolby"]),
    );
    qualities
}

fn push_quality(qualities: &mut Vec<Quality>, quality: Quality, available: bool) {
    if available && !qualities.contains(&quality) {
        qualities.push(quality);
    }
}

fn any_positive(value: &Value, fields: &[&str]) -> bool {
    fields.iter().any(|field| {
        value
            .get(*field)
            .and_then(json_u64)
            .is_some_and(|size| size > 0)
    })
}

fn positive_index(values: &[Value], index: usize) -> bool {
    values
        .get(index)
        .and_then(json_u64)
        .is_some_and(|size| size > 0)
}

fn generate_search_id() -> Result<String> {
    let duration = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_err(|error| {
            TuneWeaveError::new(
                ErrorCode::InternalError,
                format!("system clock is before the Unix epoch: {error}"),
            )
            .with_platform(Platform::Qq)
        })?;
    let random_high = rand::random_range(1_u64..=20).saturating_mul(18_014_398_509_481_984);
    let random_low = rand::random_range(0_u64..=4_194_304).saturating_mul(4_294_967_296);
    let millis_of_day = u64::try_from(duration.as_millis() % 86_400_000).unwrap_or(0);
    Ok(random_high
        .saturating_add(random_low)
        .saturating_add(millis_of_day)
        .to_string())
}

fn ensure_data_success(data: &Value, context: &str) -> Result<()> {
    let code = data
        .get("code")
        .and_then(|value| {
            value
                .as_i64()
                .or_else(|| value.as_str().and_then(|value| value.parse().ok()))
        })
        .ok_or_else(|| qq_data_error(format!("{context} is missing a valid data code")))?;
    if code == 0 {
        Ok(())
    } else {
        Err(TuneWeaveError::new(
            ErrorCode::UpstreamError,
            format!("{context} failed with code {code}"),
        )
        .with_platform(Platform::Qq)
        .with_details(json!({ "platform_code": code })))
    }
}

fn qq_ref(id: &str, kind: &str) -> Result<ResourceRef> {
    ResourceRef::new(Platform::Qq, id).map_err(|error| {
        TuneWeaveError::new(
            ErrorCode::UpstreamError,
            format!("QQ returned an invalid {kind} identifier: {error}"),
        )
        .with_platform(Platform::Qq)
    })
}

fn qq_cover_url(kind: &str, mid: &str) -> String {
    format!("https://y.gtimg.cn/music/photo_new/{kind}R300x300M000{mid}.jpg")
}

fn nonempty_string(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn value_as_string(value: Option<&Value>) -> Option<String> {
    match value {
        Some(Value::String(value)) => {
            let value = value.trim();
            (!value.is_empty()).then(|| value.to_owned())
        }
        Some(Value::Number(value)) => Some(value.to_string()),
        _ => None,
    }
}

fn json_u64(value: &Value) -> Option<u64> {
    value
        .as_u64()
        .or_else(|| value.as_i64().and_then(|value| u64::try_from(value).ok()))
        .or_else(|| value.as_str().and_then(|value| value.parse().ok()))
}

fn json_i64(value: &Value) -> Option<i64> {
    value
        .as_i64()
        .or_else(|| value.as_u64().and_then(|value| i64::try_from(value).ok()))
        .or_else(|| value.as_str().and_then(|value| value.parse().ok()))
}

fn json_bool(value: &Value) -> Option<bool> {
    value.as_bool().or_else(|| match json_u64(value) {
        Some(0) => Some(false),
        Some(1) => Some(true),
        _ => value.as_str().and_then(|value| match value.trim() {
            "true" => Some(true),
            "false" => Some(false),
            _ => None,
        }),
    })
}

fn insert_some(extensions: &mut Extensions, key: &str, value: Option<String>) {
    if let Some(value) = value {
        extensions.insert(key.to_owned(), Value::String(value));
    }
}

fn insert_value(extensions: &mut Extensions, key: &str, value: Option<&Value>) {
    if let Some(value) = value.filter(|value| !value.is_null()) {
        extensions.insert(key.to_owned(), value.clone());
    }
}

fn qq_data_error(message: impl Into<String>) -> TuneWeaveError {
    TuneWeaveError::new(ErrorCode::UpstreamError, message)
        .with_platform(Platform::Qq)
        .retryable(true)
}

const fn capability_for_search(kind: SearchKind) -> Capability {
    match kind {
        SearchKind::Track => Capability::SearchTracks,
        SearchKind::Album => Capability::SearchAlbums,
        SearchKind::Artist => Capability::SearchArtists,
        SearchKind::Playlist => Capability::SearchPlaylists,
        SearchKind::User => Capability::SearchUsers,
        SearchKind::Mv => Capability::SearchMvs,
        SearchKind::Lyric => Capability::SearchLyrics,
        SearchKind::RadioStation => Capability::SearchRadioStations,
        SearchKind::Podcast => Capability::SearchPodcasts,
        SearchKind::Video => Capability::SearchVideos,
        SearchKind::Mixed => Capability::SearchMixed,
        SearchKind::Voice => Capability::SearchVoices,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct StaticCredentialStore {
        credentials: Vec<tuneweave_core::StoredAccountCredential>,
    }

    impl AccountCredentialStore for StaticCredentialStore {
        fn load_platform(
            &self,
            platform: Platform,
        ) -> Result<Vec<tuneweave_core::StoredAccountCredential>> {
            Ok(self
                .credentials
                .iter()
                .filter(|credential| credential.platform == platform)
                .cloned()
                .collect())
        }

        fn put(&self, _credential: &tuneweave_core::StoredAccountCredential) -> Result<()> {
            Err(TuneWeaveError::new(
                ErrorCode::InternalError,
                "static credential store is read-only",
            ))
        }

        fn remove(&self, _platform: Platform, _account: &str) -> Result<bool> {
            Err(TuneWeaveError::new(
                ErrorCode::InternalError,
                "static credential store is read-only",
            ))
        }
    }

    fn response(data: Value) -> QqApiResponse {
        QqApiResponse {
            data: data.clone(),
            raw: json!({"code": 0, "req_0": {"code": 0, "data": data}}),
        }
    }

    fn sample_track(id: u64, mid: &str, title: &str) -> Value {
        json!({
            "id": id,
            "mid": mid,
            "type": 1,
            "title": title,
            "title_main": title,
            "subtitle": "电影插曲",
            "singer": [{"id": 4558, "mid": "0025NhlN2yWrP4", "name": "周杰伦"}],
            "album": {"id": 8220, "mid": "000MkMni19ClKG", "name": "叶惠美"},
            "mv": {"id": 293791, "vid": "w0026q7f01a"},
            "file": {
                "media_mid": "003Qui1q2u1Zho",
                "size_128mp3": 1,
                "size_320mp3": 2,
                "size_flac": 3,
                "size_new": [4, 0, 5]
            },
            "pay": {"pay_play": 1},
            "interval": 269,
            "status": 0
        })
    }

    fn search_query(kind: SearchKind, limit: u32, offset: u32) -> SearchQuery {
        SearchQuery {
            query: "周杰伦".to_owned(),
            kind,
            variant: SearchVariant::Default,
            limit,
            offset,
            account: None,
            search_id: None,
            highlight: false,
        }
    }

    #[test]
    fn track_mapping_preserves_every_qq_identifier() {
        let track = map_track(sample_track(97_773, "0039MnYb0qxYhV", "晴天")).expect("map track");
        assert_eq!(track.resource_ref.to_string(), "qq:0039MnYb0qxYhV");
        assert_eq!(track.extensions["numeric_id"], "97773");
        assert_eq!(track.extensions["mid"], "0039MnYb0qxYhV");
        assert_eq!(track.extensions["media_mid"], "003Qui1q2u1Zho");
        assert_eq!(track.extensions["song_type"], 1);
        assert_eq!(track.duration_ms, Some(269_000));
        assert_eq!(track.mv_ref.expect("MV ref").to_string(), "qq:w0026q7f01a");
        assert!(track.available_qualities.contains(&Quality::Standard));
        assert!(track.available_qualities.contains(&Quality::High));
        assert!(track.available_qualities.contains(&Quality::Lossless));
        assert!(track.available_qualities.contains(&Quality::Master));
        assert!(track.available_qualities.contains(&Quality::Surround));
    }

    #[test]
    fn query_song_request_keeps_numeric_and_mid_batches_distinct() {
        let numeric_ids = vec!["100".to_owned(), "97773".to_owned()];
        let (request, identifiers) = query_song_request(&numeric_ids).expect("numeric request");
        assert_eq!(request.module, QUERY_SONG_MODULE);
        assert_eq!(request.method, QUERY_SONG_METHOD);
        assert_eq!(request.param["ids"], json!([100, 97773]));
        assert!(request.param.get("mids").is_none());
        assert_eq!(request.param["types"], json!([0, 0]));
        assert_eq!(request.param["modify_stamp"], json!([0, 0]));
        assert_eq!(request.param["ctx"], 0);
        assert_eq!(request.param["client"], 1);
        assert_eq!(
            identifiers,
            [
                QqTrackIdentifier::Numeric(100),
                QqTrackIdentifier::Numeric(97_773)
            ]
        );

        let mids = vec!["003w2xz20QlUZt".to_owned(), "0039MnYb0qxYhV".to_owned()];
        let (request, identifiers) = query_song_request(&mids).expect("MID request");
        assert_eq!(request.param["mids"], json!(mids));
        assert!(request.param.get("ids").is_none());
        assert_eq!(
            identifiers,
            [
                QqTrackIdentifier::Mid("003w2xz20QlUZt".to_owned()),
                QqTrackIdentifier::Mid("0039MnYb0qxYhV".to_owned())
            ]
        );
    }

    #[test]
    fn query_song_mapping_restores_requested_order_and_duplicate_entries() {
        let first = sample_track(97_773, "0039MnYb0qxYhV", "晴天");
        let second = sample_track(100, "003w2xz20QlUZt", "可爱女人");
        let requested = [
            QqTrackIdentifier::Mid("003w2xz20QlUZt".to_owned()),
            QqTrackIdentifier::Mid("0039MnYb0qxYhV".to_owned()),
            QqTrackIdentifier::Mid("003w2xz20QlUZt".to_owned()),
        ];
        let tracks =
            map_query_song_response(&requested, response(json!({"tracks": [first, second]})))
                .expect("map query-song response");
        assert_eq!(
            tracks
                .iter()
                .map(|track| track.resource_ref.to_string())
                .collect::<Vec<_>>(),
            [
                "qq:003w2xz20QlUZt",
                "qq:0039MnYb0qxYhV",
                "qq:003w2xz20QlUZt"
            ]
        );
        assert_eq!(tracks[0].extensions["query_index"], 0);
        assert_eq!(tracks[1].extensions["query_identifier_kind"], "mid");
        assert_eq!(tracks[2].extensions["query_index"], 2);
    }

    #[test]
    fn query_song_mapping_rejects_missing_catalogs_and_requested_tracks() {
        let requested = [QqTrackIdentifier::Numeric(100)];
        let malformed = map_query_song_response(&requested, response(json!({})))
            .expect_err("missing tracks array");
        assert_eq!(malformed.code, ErrorCode::UpstreamError);

        let missing = map_query_song_response(
            &requested,
            response(json!({
                "tracks": [sample_track(97_773, "0039MnYb0qxYhV", "晴天")]
            })),
        )
        .expect_err("requested track is absent");
        assert_eq!(missing.code, ErrorCode::ResourceNotFound);
        assert_eq!(missing.details["id"], "100");
    }

    #[test]
    fn song_detail_request_keeps_numeric_id_and_mid_branches_exact() {
        let (numeric, identifier) = song_detail_request("100").expect("numeric detail request");
        assert_eq!(numeric.module, SONG_DETAIL_MODULE);
        assert_eq!(numeric.method, SONG_DETAIL_METHOD);
        assert_eq!(numeric.param, json!({"song_id": 100}));
        assert_eq!(identifier, QqTrackIdentifier::Numeric(100));

        let (mid, identifier) = song_detail_request("003w2xz20QlUZt").expect("MID detail request");
        assert_eq!(mid.module, SONG_DETAIL_MODULE);
        assert_eq!(mid.method, SONG_DETAIL_METHOD);
        assert_eq!(mid.param, json!({"song_mid": "003w2xz20QlUZt"}));
        assert_eq!(
            identifier,
            QqTrackIdentifier::Mid("003w2xz20QlUZt".to_owned())
        );
    }

    #[test]
    fn song_detail_mapping_keeps_rich_sections_extras_and_complete_response() {
        let track = sample_track(100, "003w2xz20QlUZt", "可爱女人");
        let data = json!({
            "track_info": track,
            "info": {
                "company": {"content": [{
                    "id": 1,
                    "value": "杰威尔音乐",
                    "show_type": 0,
                    "jumpurl": ""
                }]},
                "genre": {"content": [{
                    "id": 2,
                    "value": "流行",
                    "show_type": 0,
                    "jumpurl": ""
                }]},
                "intro": {"content": []},
                "lan": {"content": [{
                    "id": 3,
                    "value": "国语",
                    "show_type": 0,
                    "jumpurl": ""
                }]},
                "pub_time": {"content": [{
                    "id": 4,
                    "value": "2000-11-07",
                    "show_type": 0,
                    "jumpurl": ""
                }]}
            },
            "extras": {"album_name": "Jay"}
        });
        let mapped = map_song_detail_response(
            &QqTrackIdentifier::Numeric(100),
            QqApiResponse {
                data: data.clone(),
                raw: json!({"code": 0, "data": data}),
            },
        )
        .expect("map rich song detail");
        assert_eq!(mapped.resource_ref.to_string(), "qq:003w2xz20QlUZt");
        assert_eq!(mapped.extensions["detail_identifier_kind"], "numeric_id");
        assert_eq!(
            mapped.extensions["detail_info"]["company"]["content"][0]["value"],
            "杰威尔音乐"
        );
        assert_eq!(
            mapped.extensions["detail_info"]["genre"]["content"][0]["value"],
            "流行"
        );
        assert_eq!(
            mapped.extensions["detail_info"]["lan"]["content"][0]["value"],
            "国语"
        );
        assert_eq!(
            mapped.extensions["detail_info"]["pub_time"]["content"][0]["value"],
            "2000-11-07"
        );
        assert_eq!(mapped.extensions["detail_extras"]["album_name"], "Jay");
        assert_eq!(mapped.extensions["detail_response"]["code"], 0);
    }

    #[test]
    fn song_detail_mapping_rejects_missing_mismatched_and_malformed_data() {
        let requested = QqTrackIdentifier::Numeric(100);
        let missing = map_song_detail_response(&requested, response(json!({})))
            .expect_err("missing track detail");
        assert_eq!(missing.code, ErrorCode::ResourceNotFound);

        let mismatch = map_song_detail_response(
            &requested,
            response(json!({
                "track_info": sample_track(97_773, "0039MnYb0qxYhV", "晴天")
            })),
        )
        .expect_err("mismatched track detail");
        assert_eq!(mismatch.code, ErrorCode::UpstreamError);

        for malformed in [
            json!({"info": []}),
            json!({"info": {"company": []}}),
            json!({"info": {"company": {"content": {}}}}),
            json!({"info": {"company": {"content": [{}]}}}),
            json!({"extras": []}),
        ] {
            let mut data = malformed;
            data["track_info"] = sample_track(100, "003w2xz20QlUZt", "可爱女人");
            let error = map_song_detail_response(&requested, response(data))
                .expect_err("malformed song detail");
            assert_eq!(error.code, ErrorCode::UpstreamError);
        }
    }

    #[test]
    fn lyric_request_preserves_every_option_and_both_identifier_branches() {
        let options = LyricsRequest {
            word_synced: true,
            translated: true,
            romanized: true,
            song_type: Some(7),
            account: None,
        };
        let (numeric, identifier) = lyric_request("100", &options).expect("numeric lyric request");
        assert_eq!(numeric.module, LYRIC_MODULE);
        assert_eq!(numeric.method, LYRIC_METHOD);
        assert_eq!(
            numeric.param,
            json!({
                "crypt": 1,
                "lrc_t": 0,
                "qrc": true,
                "qrc_t": 0,
                "roma": true,
                "roma_t": 0,
                "trans": true,
                "trans_t": 0,
                "type": 7,
                "ct": 11,
                "cv": 14090008,
                "songId": 100
            })
        );
        assert_eq!(identifier, QqTrackIdentifier::Numeric(100));

        let (mid, identifier) =
            lyric_request("000akynZ2Rbro5", &LyricsRequest::default()).expect("MID lyric request");
        assert_eq!(mid.param["songMid"], "000akynZ2Rbro5");
        assert_eq!(mid.param["qrc"], false);
        assert_eq!(mid.param["trans"], false);
        assert_eq!(mid.param["roma"], false);
        assert_eq!(mid.param["type"], 1);
        assert!(mid.param.get("songId").is_none());
        assert_eq!(
            identifier,
            QqTrackIdentifier::Mid("000akynZ2Rbro5".to_owned())
        );
    }

    #[test]
    fn lyric_mapping_never_lets_line_sync_override_word_sync() {
        let requested = QqTrackIdentifier::Numeric(100);
        let options = LyricsRequest::default();
        let lyrics = map_lyric_response(
            &requested,
            &options,
            response(json!({
                "songID": 100,
                "songName": "测试歌",
                "songType": 1,
                "singerName": "测试歌手",
                "crypt": 0,
                "qrc": 1,
                "lyric": "<QrcInfos><LyricInfo LyricContent=\"[0,1000](0,500)逐字\"/></QrcInfos>",
                "trans": "[00:00.00]translation",
                "roma": "[00:00.00]romanization",
                "hasContributor": 1
            })),
        )
        .expect("map word-synced lyrics");
        assert_eq!(lyrics.track_ref.to_string(), "qq:100");
        assert_eq!(lyrics.format, "qrc");
        assert!(lyrics.word_synced.is_some());
        assert!(lyrics.plain.is_none());
        assert_eq!(lyrics.translated.as_deref(), Some("[00:00.00]translation"));
        assert_eq!(lyrics.romanized.as_deref(), Some("[00:00.00]romanization"));
        assert_eq!(lyrics.extensions["actual_qrc"], true);
        assert_eq!(lyrics.extensions["song_type"], 1);
        assert_eq!(lyrics.extensions["has_contributor"], 1);
        assert_eq!(lyrics.extensions["response"]["code"], 0);

        let line_synced = map_lyric_response(
            &requested,
            &options,
            response(json!({
                "songID": 100,
                "crypt": 0,
                "qrc": 0,
                "lyric": "[00:00.00]逐行歌词"
            })),
        )
        .expect("map line-synced lyrics");
        assert_eq!(line_synced.format, "lrc");
        assert!(line_synced.plain.is_some());
        assert!(line_synced.word_synced.is_none());
    }

    #[test]
    fn lyric_mapping_rejects_wrong_identity_flags_fields_and_ciphertext() {
        let requested = QqTrackIdentifier::Numeric(100);
        let options = LyricsRequest::default();
        for malformed in [
            json!({"songID": 101, "crypt": 0, "qrc": 0, "lyric": "text"}),
            json!({"songID": 100, "crypt": 2, "qrc": 0, "lyric": "text"}),
            json!({"songID": 100, "crypt": 0, "qrc": 2, "lyric": "text"}),
            json!({"songID": 100, "crypt": 0, "qrc": 0}),
            json!({"songID": 100, "crypt": 0, "qrc": 0, "lyric": []}),
            json!({"songID": 100, "crypt": 1, "qrc": 1, "lyric": "00"}),
        ] {
            let error = map_lyric_response(&requested, &options, response(malformed))
                .expect_err("malformed lyric response");
            assert!(matches!(
                error.code,
                ErrorCode::UpstreamError | ErrorCode::ResourceNotFound
            ));
        }
    }

    #[test]
    fn cdn_dispatch_request_uses_a_fresh_lowercase_guid_and_exact_controls() {
        let (request, guid) = cdn_dispatch_request();
        assert_eq!(request.module, CDN_DISPATCH_MODULE);
        assert_eq!(request.method, CDN_DISPATCH_METHOD);
        assert_eq!(guid.len(), 32);
        assert!(
            guid.chars()
                .all(|character| character.is_ascii_hexdigit() && !character.is_ascii_uppercase())
        );
        assert_eq!(
            request.param,
            json!({
                "guid": guid,
                "uid": "0",
                "use_new_domain": 1,
                "use_ipv6": 1
            })
        );
        let (_, other_guid) = cdn_dispatch_request();
        assert_ne!(guid, other_guid);
    }

    #[test]
    fn cdn_dispatch_mapping_preserves_roots_nodes_timers_and_raw_response() {
        let dispatch = map_cdn_dispatch_response(
            "0123456789abcdef0123456789abcdef",
            response(json!({
                "retcode": 0,
                "sip": [
                    "http://aqqmusic.tc.qq.com/",
                    "http://aqqmusic.tc.qq.com/",
                    "https://sjy6.stream.qqmusic.qq.com/"
                ],
                "sipinfo": [
                    {
                        "cdn": "http://aqqmusic.tc.qq.com/",
                        "quic": 0,
                        "ipstack": 3,
                        "quichost": "aqqmusic.tc.qq.com",
                        "plaintextquic": 0,
                        "encryptquic": 1,
                        "future": "kept"
                    },
                    {}
                ],
                "keepalivefile": "C400test.m4a?vkey=public",
                "expiration": 86400,
                "refreshTime": 1800,
                "cacheTime": 86400
            })),
        )
        .expect("map CDN dispatch");
        assert_eq!(dispatch.roots.len(), 3);
        assert_eq!(dispatch.roots[0], dispatch.roots[1]);
        assert_eq!(dispatch.nodes.len(), 2);
        assert_eq!(dispatch.nodes[0].ip_stack, 3);
        assert_eq!(dispatch.nodes[0].encrypted_quic, 1);
        assert_eq!(
            dispatch.nodes[0].quic_host.as_deref(),
            Some("aqqmusic.tc.qq.com")
        );
        assert_eq!(dispatch.nodes[0].extensions["response"]["future"], "kept");
        assert_eq!(dispatch.nodes[1].url, "");
        assert_eq!(dispatch.expires_in_seconds, 86400);
        assert_eq!(dispatch.refresh_after_seconds, 1800);
        assert_eq!(dispatch.cache_for_seconds, 86400);
        assert_eq!(
            dispatch.extensions["request_guid"],
            "0123456789abcdef0123456789abcdef"
        );
        assert_eq!(dispatch.extensions["response"]["code"], 0);
    }

    #[test]
    fn cdn_dispatch_mapping_rejects_unusable_or_malformed_catalogs() {
        let base = json!({
            "retcode": 0,
            "sip": ["https://aqqmusic.tc.qq.com/"],
            "sipinfo": [],
            "keepalivefile": "C400test.m4a?vkey=public",
            "expiration": 86400,
            "refreshTime": 1800,
            "cacheTime": 86400
        });
        let mut fixtures = Vec::new();
        for (field, value) in [
            ("retcode", json!(1)),
            ("sip", json!([])),
            ("sip", json!(["ftp://example.test/file/"])),
            ("sip", json!(["https://user:secret@example.test/"])),
            ("sipinfo", json!({})),
            ("sipinfo", json!([[]])),
            ("keepalivefile", json!("https://attacker.test/file")),
            ("expiration", json!(0)),
            ("refreshTime", json!("invalid")),
            ("cacheTime", json!(null)),
        ] {
            let mut fixture = base.clone();
            fixture[field] = value;
            fixtures.push(fixture);
        }
        let mut missing_sip = base;
        missing_sip.as_object_mut().expect("object").remove("sip");
        fixtures.push(missing_sip);
        for fixture in fixtures {
            let error = map_cdn_dispatch_response("guid", response(fixture))
                .expect_err("invalid CDN dispatch");
            assert_eq!(error.code, ErrorCode::UpstreamError);
        }
    }

    #[test]
    fn audio_file_specs_cover_every_sdk_variant_and_web_integer_mapping() {
        assert_eq!(QQ_AUDIO_FILE_SPECS.len(), 45);
        let names = QQ_AUDIO_FILE_SPECS
            .iter()
            .map(|spec| spec.name)
            .collect::<BTreeSet<_>>();
        assert_eq!(names.len(), QQ_AUDIO_FILE_SPECS.len());
        assert_eq!(
            parse_qq_audio_file_spec(None).expect("default").name,
            "mp3_128"
        );
        assert_eq!(
            parse_qq_audio_file_spec(Some("0")).expect("index").name,
            "dts_x"
        );
        assert_eq!(
            parse_qq_audio_file_spec(Some("43"))
                .expect("web index")
                .name,
            "therapy"
        );
        assert_eq!(
            parse_qq_audio_file_spec(Some("44"))
                .expect("SDK extension")
                .name,
            "trial_ogg_640"
        );
        assert_eq!(
            parse_qq_audio_file_spec(Some("ACC_96"))
                .expect("upstream alias")
                .name,
            "aac_96"
        );
        assert_eq!(
            parse_qq_audio_file_spec(Some("ATMOS_51"))
                .expect("upstream alias")
                .name,
            "atmos_5_1"
        );
        assert_eq!(
            parse_qq_audio_file_spec(Some("ENCRYPTED_ATMOS_DB"))
                .expect("upstream alias")
                .name,
            "encrypted_dolby_atmos"
        );
        assert_eq!(
            parse_qq_audio_file_spec(Some("TRY"))
                .expect("upstream alias")
                .name,
            "trial"
        );
        assert_eq!(
            parse_qq_audio_file_spec(Some("BAYIN"))
                .expect("upstream alias")
                .name,
            "music_box"
        );
        assert!(parse_qq_audio_file_spec(Some("45")).is_err());
        assert!(parse_qq_audio_file_spec(Some("future_codec")).is_err());
        assert!(parse_qq_audio_file_spec(Some(" ")).is_err());
    }

    #[test]
    fn song_urls_request_preserves_default_family_per_item_overrides_and_credential_uin() {
        let credential = serde_json::from_value::<QqCredential>(json!({
            "musicid": 123456,
            "str_musicid": "123456",
            "musickey": "Q_H_L_private"
        }))
        .expect("credential")
        .normalize()
        .expect("valid credential");
        let items = vec![
            PreparedQqAudioFile {
                track_ref: qq_ref("003w2xz20QlUZt", "track").expect("ref"),
                mid: "003w2xz20QlUZt".to_owned(),
                media_id: Some("media001".to_owned()),
                song_type: 7,
                spec: parse_qq_audio_file_spec(Some("encrypted_flac")).expect("spec"),
                filename: "F0M0media001.mflac".to_owned(),
            },
            PreparedQqAudioFile {
                track_ref: qq_ref("000akynZ2Rbro5", "track").expect("ref"),
                mid: "000akynZ2Rbro5".to_owned(),
                media_id: None,
                song_type: 0,
                spec: parse_qq_audio_file_spec(Some("encrypted_ogg_640")).expect("spec"),
                filename: "O8M1000akynZ2Rbro5000akynZ2Rbro5.mgg".to_owned(),
            },
        ];
        let default = parse_qq_audio_file_spec(Some("encrypted_flac")).expect("default");
        let (request, guid) = song_urls_request(&items, default, Some(&credential));
        assert_eq!(request.module, ENCRYPTED_SONG_URL_MODULE);
        assert_eq!(request.method, ENCRYPTED_SONG_URL_METHOD);
        assert_eq!(request.param["uin"], "123456");
        assert_eq!(
            request.param["filename"],
            json!(["F0M0media001.mflac", "O8M1000akynZ2Rbro5000akynZ2Rbro5.mgg"])
        );
        assert_eq!(
            request.param["songmid"],
            json!(["003w2xz20QlUZt", "000akynZ2Rbro5"])
        );
        assert_eq!(request.param["songtype"], json!([7, 0]));
        assert_eq!(request.param["ctx"], 0);
        assert_eq!(request.param["guid"], guid);
        assert_eq!(guid.len(), 32);

        let normal_default = parse_qq_audio_file_spec(Some("mp3_128")).expect("normal");
        let (normal, other_guid) = song_urls_request(&items, normal_default, None);
        assert_eq!(normal.module, SONG_URL_MODULE);
        assert_eq!(normal.method, SONG_URL_METHOD);
        assert_eq!(normal.param["uin"], "");
        assert_ne!(guid, other_guid);
    }

    #[test]
    fn song_urls_mapping_preserves_success_denial_encryption_and_raw_items() {
        let requested = vec![
            PreparedQqAudioFile {
                track_ref: qq_ref("003w2xz20QlUZt", "track").expect("ref"),
                mid: "003w2xz20QlUZt".to_owned(),
                media_id: Some("003w2xz20QlUZt".to_owned()),
                song_type: 1,
                spec: parse_qq_audio_file_spec(Some("encrypted_ogg_640")).expect("spec"),
                filename: "O8M1003w2xz20QlUZt.mgg".to_owned(),
            },
            PreparedQqAudioFile {
                track_ref: qq_ref("000akynZ2Rbro5", "track").expect("ref"),
                mid: "000akynZ2Rbro5".to_owned(),
                media_id: None,
                song_type: 0,
                spec: parse_qq_audio_file_spec(Some("encrypted_flac")).expect("spec"),
                filename: "F0M0000akynZ2Rbro5000akynZ2Rbro5.mflac".to_owned(),
            },
        ];
        let default = parse_qq_audio_file_spec(Some("encrypted_flac")).expect("default");
        let batch = map_song_urls_response(
            &requested,
            default,
            "0123456789abcdef0123456789abcdef",
            response(json!({
                "expiration": 80400,
                "midurlinfo": [{
                    "songmid": "003w2xz20QlUZt",
                    "filename": "O8M1003w2xz20QlUZt.mgg",
                    "purl": "O8M1003w2xz20QlUZt.mgg?vkey=temporary",
                    "vkey": "temporary",
                    "ekey": "decrypt-temporary",
                    "result": 0,
                    "future": "kept"
                }, {
                    "songmid": "000akynZ2Rbro5",
                    "filename": "F0M0000akynZ2Rbro5000akynZ2Rbro5.mflac",
                    "purl": "",
                    "vkey": "",
                    "ekey": "",
                    "result": 104003
                }]
            })),
        )
        .expect("map song URLs");
        assert_eq!(batch.expires_in_seconds, 80400);
        assert_eq!(batch.files.len(), 2);
        assert!(batch.files[0].available);
        assert!(batch.files[0].encrypted);
        assert_eq!(batch.files[0].quality, Some(Quality::Lossless));
        assert_eq!(batch.files[0].bitrate, Some(640_000));
        assert_eq!(
            batch.files[0].decryption_key.as_deref(),
            Some("decrypt-temporary")
        );
        assert_eq!(batch.files[0].extensions["response"]["future"], "kept");
        assert!(!batch.files[1].available);
        assert_eq!(batch.files[1].platform_code, 104003);
        assert_eq!(batch.files[1].relative_url, None);
        assert_eq!(batch.extensions["module"], ENCRYPTED_SONG_URL_MODULE);
        assert_eq!(batch.extensions["response"]["code"], 0);
    }

    #[test]
    fn song_urls_mapping_rejects_misaligned_unsafe_and_false_success_responses() {
        let requested = vec![PreparedQqAudioFile {
            track_ref: qq_ref("003w2xz20QlUZt", "track").expect("ref"),
            mid: "003w2xz20QlUZt".to_owned(),
            media_id: Some("003w2xz20QlUZt".to_owned()),
            song_type: 1,
            spec: parse_qq_audio_file_spec(Some("encrypted_flac")).expect("spec"),
            filename: "F0M0003w2xz20QlUZt.mflac".to_owned(),
        }];
        let default = parse_qq_audio_file_spec(Some("encrypted_flac")).expect("default");
        let valid_entry = json!({
            "songmid": "003w2xz20QlUZt",
            "filename": "F0M0003w2xz20QlUZt.mflac",
            "purl": "F0M0003w2xz20QlUZt.mflac?vkey=temporary",
            "vkey": "temporary",
            "ekey": "decrypt-temporary",
            "result": 0
        });
        let mut wrong_mid = valid_entry.clone();
        wrong_mid["songmid"] = json!("different");
        let mut fixtures = vec![
            json!({"expiration": 7200}),
            json!({"expiration": 7200, "midurlinfo": []}),
            json!({"expiration": 7200, "midurlinfo": [wrong_mid]}),
        ];
        for (field, value) in [
            ("filename", json!("different.mflac")),
            ("purl", json!("https://attacker.test/file")),
            ("purl", json!("")),
            ("ekey", json!("")),
            ("result", json!("invalid")),
        ] {
            let mut entry = valid_entry.clone();
            entry[field] = value;
            fixtures.push(json!({"expiration": 7200, "midurlinfo": [entry]}));
        }
        for fixture in fixtures {
            let error = map_song_urls_response(&requested, default, "guid", response(fixture))
                .expect_err("invalid song URL response");
            assert_eq!(error.code, ErrorCode::UpstreamError);
        }
    }

    #[tokio::test]
    async fn audio_files_reject_named_accounts_before_network_without_a_store() {
        let provider = QqProvider::new(QqConfig::default()).expect("provider");
        let error = provider
            .audio_files(&AudioFileRequest {
                items: vec![tuneweave_core::AudioFileRequestItem {
                    track_ref: qq_ref("003w2xz20QlUZt", "track").expect("ref"),
                    spec: Some("mp3_128".to_owned()),
                    song_type: None,
                    media_id: None,
                }],
                default_spec: None,
                account: Some("vip".to_owned()),
            })
            .await
            .expect_err("account store required");
        assert_eq!(error.code, ErrorCode::AuthenticationRequired);
        assert_eq!(error.details["account"], "vip");
    }

    #[test]
    fn qq_account_alias_loads_the_exact_stored_credential_without_exposing_secrets() {
        let stored = tuneweave_core::StoredAccountCredential::new(
            Platform::Qq,
            "green-vip",
            QQ_CREDENTIAL_KIND,
            serde_json::to_string(&json!({
                "musicid": 123456,
                "str_musicid": "123456",
                "musickey": "Q_H_L_private"
            }))
            .expect("credential JSON"),
        )
        .expect("stored credential");
        let provider = QqProvider::new(QqConfig {
            credential_store: Some(Arc::new(StaticCredentialStore {
                credentials: vec![stored],
            })),
            ..QqConfig::default()
        })
        .expect("provider");
        let credential = provider
            .qq_credential(Some("green-vip"))
            .expect("load credential")
            .expect("credential present");
        assert_eq!(credential.string_music_id(), "123456");
        assert_eq!(credential.login_type, 2);
        assert!(!format!("{credential:?}").contains("Q_H_L_private"));
        let error = provider
            .qq_credential(Some("missing"))
            .expect_err("missing alias");
        assert_eq!(error.code, ErrorCode::AuthenticationRequired);
    }

    #[test]
    fn artist_mapping_preserves_counts_identity_and_raw_search_fields() {
        let item = map_artist_search_item(json!({
            "singerID": 4558,
            "singerMID": "0025NhlN2yWrP4",
            "singerName": "周杰伦",
            "singerPic": "https://example.test/artist.jpg",
            "songNum": 1013,
            "albumNum": 43,
            "mvNum": 10426,
            "subtitle": "歌曲:1013  专辑:43  视频:10426",
            "type": 0
        }))
        .expect("map artist");
        let SearchItem::Artist(artist) = item else {
            panic!("expected artist");
        };
        assert_eq!(artist.resource_ref.to_string(), "qq:0025NhlN2yWrP4");
        assert_eq!(artist.extensions["numeric_id"], "4558");
        assert_eq!(artist.track_count, Some(1013));
        assert_eq!(artist.album_count, Some(43));
        assert_eq!(artist.mv_count, Some(10426));
        assert_eq!(
            artist.avatar_url.as_deref(),
            Some("https://example.test/artist.jpg")
        );
        assert_eq!(artist.extensions["search_item"]["type"], 0);
    }

    #[test]
    fn album_mapping_keeps_mid_numeric_id_artists_date_and_platform_fields() {
        let item = map_album_search_item(json!({
            "id": 60671,
            "mid": "0024bjiL2aocxT",
            "name": "十一月的萧邦",
            "subtitle": "November's Chopin",
            "time_public": "2005-11-01",
            "pmid": "0024bjiL2aocxT_5",
            "pic": "https://example.test/album.jpg",
            "desc_detail": {"desc": "专辑介绍"},
            "core_album_config": {"album_type": 1},
            "singer_list": [{"id": 4558, "mid": "0025NhlN2yWrP4", "name": "周杰伦"}],
            "award_label": "殿堂史诗唱片"
        }))
        .expect("map album");
        let SearchItem::Album(album) = item else {
            panic!("expected album");
        };
        assert_eq!(album.resource_ref.to_string(), "qq:0024bjiL2aocxT");
        assert_eq!(album.extensions["numeric_id"], "60671");
        assert_eq!(album.aliases, ["November's Chopin"]);
        assert_eq!(album.description, "专辑介绍");
        assert_eq!(album.published_at.as_deref(), Some("2005-11-01"));
        assert_eq!(album.kind.as_deref(), Some("1"));
        assert_eq!(
            album.artists[0]
                .resource_ref
                .as_ref()
                .expect("artist ref")
                .to_string(),
            "qq:0025NhlN2yWrP4"
        );
        assert_eq!(album.extensions["award_label"], "殿堂史诗唱片");
    }

    #[test]
    fn playlist_mapping_preserves_owner_counts_and_complete_raw_item() {
        let item = map_playlist_search_item(json!({
            "dissid": "7039749142",
            "dissname": "百听不厌的周杰伦",
            "logo": "https://example.test/playlist.jpg",
            "description": "99首",
            "subhead": "周杰伦精选歌单",
            "songnum": 99,
            "listennum": 406419550,
            "nickname": "今晚月色很美",
            "uin": "2904004371",
            "createtime": "2019-06-28",
            "modifytime": "2019-08-16",
            "dirtype": 0
        }))
        .expect("map playlist");
        let SearchItem::Playlist(playlist) = item else {
            panic!("expected playlist");
        };
        assert_eq!(playlist.resource_ref.to_string(), "qq:7039749142");
        assert_eq!(playlist.track_count, Some(99));
        assert_eq!(
            playlist
                .creator
                .as_ref()
                .expect("creator")
                .resource_ref
                .as_ref()
                .expect("creator ref")
                .to_string(),
            "qq:2904004371"
        );
        assert_eq!(playlist.description, "周杰伦精选歌单");
        assert_eq!(playlist.created_at.as_deref(), Some("2019-06-28"));
        assert_eq!(playlist.updated_at.as_deref(), Some("2019-08-16"));
        assert_eq!(playlist.extensions["listen_count"], 406419550_u64);
        assert_eq!(playlist.extensions["search_item"]["dirtype"], 0);
    }

    #[test]
    fn mv_mapping_prefers_vid_and_preserves_creator_counts_and_raw_item() {
        let item = map_mv_search_item(json!({
            "id": 293791,
            "vid": "w0026q7f01a",
            "title": "晴天",
            "pic": "https://example.test/mv.jpg",
            "play_count": 120108934,
            "duration": 317,
            "publish_date": "2003-07-29",
            "singerid": 4558,
            "singermid": "0025NhlN2yWrP4",
            "singername": "周杰伦",
            "type": 0
        }))
        .expect("map MV");
        let SearchItem::Video(video) = item else {
            panic!("expected video");
        };
        assert_eq!(video.resource_ref.to_string(), "qq:w0026q7f01a");
        assert_eq!(video.extensions["numeric_id"], "293791");
        assert_eq!(video.duration_ms, Some(317_000));
        assert_eq!(video.play_count, Some(120108934));
        assert_eq!(video.published_at.as_deref(), Some("2003-07-29"));
        assert_eq!(video.creators[0].name, "周杰伦");
        assert_eq!(
            video.creators[0]
                .resource_ref
                .as_ref()
                .expect("creator ref")
                .to_string(),
            "qq:0025NhlN2yWrP4"
        );
        assert_eq!(video.extensions["search_item"]["type"], 0);
    }

    #[test]
    fn lyric_mapping_keeps_the_full_track_and_search_hit_content() {
        let mut raw = sample_track(97_773, "0039MnYb0qxYhV", "晴天");
        raw["content"] = json!("故事的小黄花\n从出生那年就飘着");
        let item = map_lyric_search_item(raw).expect("map lyric hit");
        let SearchItem::Track(track) = item else {
            panic!("expected track");
        };
        assert_eq!(track.resource_ref.to_string(), "qq:0039MnYb0qxYhV");
        assert_eq!(
            track.extensions["search_content"],
            "故事的小黄花\n从出生那年就飘着"
        );
        assert_eq!(track.extensions["media_mid"], "003Qui1q2u1Zho");
    }

    #[test]
    fn user_mapping_prefers_the_encrypted_homepage_identity() {
        let item = map_user_search_item(json!({
            "encrypt_uin": "ow6yoK6v7Kcl",
            "uin": "12345678",
            "nick": "听歌用户",
            "avatar": "https://example.test/user.jpg",
            "signature": "音乐是生活",
            "isFollow": 1,
            "isMutual": 0,
            "identity": 2
        }))
        .expect("map user");
        let SearchItem::User(user) = item else {
            panic!("expected user");
        };
        assert_eq!(user.resource_ref.to_string(), "qq:ow6yoK6v7Kcl");
        assert_eq!(user.extensions["numeric_uin"], "12345678");
        assert_eq!(
            user.avatar_url.as_deref(),
            Some("https://example.test/user.jpg")
        );
        assert_eq!(user.followed, Some(true));
        assert_eq!(user.mutual, Some(false));
        assert_eq!(user.extensions["search_item"]["identity"], 2);
    }

    #[test]
    fn podcast_mapping_keeps_show_identity_creator_and_episode_count() {
        let item = map_podcast_search_item(json!({
            "id": 9001,
            "mid": "004PodcastMid",
            "name": "音乐播客",
            "pic": "https://example.test/podcast.jpg",
            "desc_detail": {"desc": "节目专辑介绍"},
            "publish_date": "2026-01-02",
            "song_num": 42,
            "singer_list": [{"id": 4558, "mid": "0025NhlN2yWrP4", "name": "周杰伦"}],
            "audio_play": {"play_num": 12345}
        }))
        .expect("map podcast");
        let SearchItem::Podcast(podcast) = item else {
            panic!("expected podcast");
        };
        assert_eq!(podcast.resource_ref.to_string(), "qq:004PodcastMid");
        assert_eq!(podcast.extensions["numeric_id"], "9001");
        assert_eq!(podcast.description, "节目专辑介绍");
        assert_eq!(podcast.episode_count, Some(42));
        assert_eq!(podcast.play_count, Some(12345));
        assert_eq!(
            podcast
                .creator
                .as_ref()
                .and_then(|creator| creator.resource_ref.as_ref())
                .expect("creator ref")
                .to_string(),
            "qq:0025NhlN2yWrP4"
        );
    }

    #[test]
    fn voice_mapping_exposes_a_playable_podcast_episode_without_losing_track_data() {
        let mut raw = sample_track(8001, "004VoiceMid", "一期节目");
        raw["type"] = json!(2);
        raw["content"] = json!("节目内容简介");
        let item = map_voice_search_item(raw).expect("map podcast episode");
        let SearchItem::PodcastEpisode(episode) = item else {
            panic!("expected podcast episode");
        };
        assert_eq!(episode.resource_ref.to_string(), "qq:004VoiceMid");
        assert_eq!(episode.description, "节目内容简介");
        assert_eq!(episode.duration_ms, Some(269_000));
        assert_eq!(
            episode
                .podcast_ref
                .as_ref()
                .expect("podcast ref")
                .to_string(),
            "qq:000MkMni19ClKG"
        );
        let audio = episode.audio.as_ref().expect("playable track");
        assert_eq!(audio.resource_ref.to_string(), "qq:004VoiceMid");
        assert_eq!(audio.extensions["media_mid"], "003Qui1q2u1Zho");
        assert_eq!(episode.extensions["song_type"], 2);
    }

    #[test]
    fn page_mapping_supports_non_aligned_offsets_across_two_upstream_pages() {
        let first = (0..60)
            .map(|id| sample_track(id, &format!("mid{id}"), &format!("track{id}")))
            .collect::<Vec<_>>();
        let second = (60..120)
            .map(|id| sample_track(id, &format!("mid{id}"), &format!("track{id}")))
            .collect::<Vec<_>>();
        let third = (120..160)
            .map(|id| sample_track(id, &format!("mid{id}"), &format!("track{id}")))
            .collect::<Vec<_>>();
        let page = map_track_search_response(
            50,
            100,
            50,
            vec![
                response(json!({"code": 0, "meta": {"sum": 200}, "body": {"item_song": first}})),
                response(json!({"code": 0, "meta": {"sum": 200}, "body": {"item_song": second}})),
                response(json!({"code": 0, "meta": {"sum": 200}, "body": {"item_song": third}})),
            ],
        )
        .expect("map page");
        assert_eq!(page.items.len(), 100);
        assert_eq!(page.items[0].name, "track50");
        assert_eq!(page.items[99].name, "track149");
        assert_eq!(page.pagination.next_offset, Some(150));
    }

    #[test]
    fn catalog_mapping_uses_each_category_safe_page_width_and_exact_slicing() {
        assert_eq!(ARTIST_SEARCH.upstream_page_size, 40);
        assert_eq!(ALBUM_SEARCH.upstream_page_size, 60);
        assert_eq!(PLAYLIST_SEARCH.upstream_page_size, 30);
        assert_eq!(MV_SEARCH.upstream_page_size, 60);
        assert_eq!(LYRIC_SEARCH.upstream_page_size, 60);
        assert_eq!(USER_SEARCH.upstream_page_size, 10);
        assert_eq!(PODCAST_SEARCH.upstream_page_size, 10);
        assert_eq!(VOICE_SEARCH.upstream_page_size, 10);
        let first = (0..30)
            .map(|id| json!({"id": id + 1, "title": format!("playlist{id}")}))
            .collect::<Vec<_>>();
        let second = (30..60)
            .map(|id| json!({"id": id + 1, "title": format!("playlist{id}")}))
            .collect::<Vec<_>>();
        let page = map_catalog_search_response(
            25,
            20,
            25,
            vec![
                response(
                    json!({"code": 0, "meta": {"sum": 100}, "body": {"item_songlist": first}}),
                ),
                response(
                    json!({"code": 0, "meta": {"sum": 100}, "body": {"item_songlist": second}}),
                ),
            ],
            PLAYLIST_SEARCH,
            map_playlist_search_item,
        )
        .expect("map playlist page");
        assert_eq!(page.items.len(), 20);
        let SearchItem::Playlist(first) = &page.items[0] else {
            panic!("expected playlist");
        };
        let SearchItem::Playlist(last) = &page.items[19] else {
            panic!("expected playlist");
        };
        assert_eq!(first.name, "playlist25");
        assert_eq!(last.name, "playlist44");
        assert_eq!(page.pagination.next_offset, Some(45));
        assert_eq!(page.pagination.extensions["upstream_page_size"], 30);
        assert_eq!(page.pagination.extensions["omitted_slots"], 0);
    }

    #[test]
    fn typed_search_preserves_the_upstream_session_and_highlight_controls() {
        let request = typed_search_request("周杰伦", "session-42", 8, 3, 10, true);
        assert_eq!(request.module, SEARCH_MODULE);
        assert_eq!(request.method, SEARCH_METHOD);
        assert_eq!(request.param["searchid"], "session-42");
        assert_eq!(request.param["search_type"], 8);
        assert_eq!(request.param["page_num"], 3);
        assert_eq!(request.param["num_per_page"], 10);
        assert_eq!(request.param["highlight"], true);

        let page = with_search_context(
            Page {
                items: Vec::<SearchItem>::new(),
                pagination: PageMeta {
                    limit: 10,
                    offset: 20,
                    total: Some(0),
                    next_offset: None,
                    has_more: false,
                    extensions: Extensions::new(),
                },
            },
            "session-42".to_owned(),
            true,
        );
        assert_eq!(page.pagination.extensions["search_id"], "session-42");
        assert_eq!(page.pagination.extensions["highlight"], true);
    }

    #[test]
    fn smartbox_mapping_keeps_keyword_related_and_direct_result_buckets() {
        let request = smartbox_request("周杰伦", "session-42");
        assert_eq!(request.module, SMARTBOX_MODULE);
        assert_eq!(request.method, SMARTBOX_METHOD);
        assert_eq!(request.param["search_id"], "session-42");
        assert_eq!(request.param["query"], "周杰伦");
        assert_eq!(request.param["num_per_page"], 0);
        assert_eq!(request.param["page_idx"], 0);

        let data = json!({
            "items": [{
                "docid": "17675977119827593594",
                "hint": "周杰伦 晴天",
                "hint_hilight": "<em>周杰伦</em> 晴天",
                "res_type": "search",
                "pre_search": false,
                "score": 9584.28
            }],
            "vec_related_items": [{
                "hint": "周杰伦 七里香",
                "res_type": "search"
            }],
            "vec_direct_items": [{
                "direct_id": 4558,
                "restype": "singer",
                "insert_pos": 0,
                "title": "歌手: 周杰伦",
                "hint": "歌手: 周杰伦",
                "cover_url": "https://example.test/artist.jpg",
                "custom_info": {
                    "mid": "0025NhlN2yWrP4",
                    "search_history": "周杰伦"
                }
            }],
            "search_id": "341894897306691299",
            "total_num": 174
        });
        let result = map_smartbox_response(
            "周杰伦",
            SearchSuggestionClient::Mobile,
            "requested-session",
            QqApiResponse {
                data: data.clone(),
                raw: json!({"code": 0, "data": data}),
            },
        )
        .expect("map SmartBox response");
        assert_eq!(result.query, "周杰伦");
        assert_eq!(result.client, SearchSuggestionClient::Mobile);
        assert_eq!(result.suggestions.len(), 2);
        assert_eq!(result.suggestions[0].keyword, "周杰伦");
        assert_eq!(result.suggestions[0].kind, Some(SearchKind::Artist));
        let Some(SearchItem::Artist(artist)) = &result.suggestions[0].resource else {
            panic!("expected direct artist resource");
        };
        assert_eq!(artist.resource_ref.to_string(), "qq:0025NhlN2yWrP4");
        assert_eq!(artist.extensions["numeric_id"], "4558");
        assert_eq!(result.suggestions[0].extensions["bucket"], "direct");
        assert_eq!(result.suggestions[1].keyword, "周杰伦 晴天");
        assert_eq!(
            result.suggestions[1].display_text.as_deref(),
            Some("<em>周杰伦</em> 晴天")
        );
        assert_eq!(result.recommendations.len(), 1);
        assert_eq!(result.recommendations[0].keyword, "周杰伦 七里香");
        assert_eq!(result.extensions["search_id"], "341894897306691299");
        assert_eq!(result.extensions["response"]["data"]["total_num"], 174);
    }

    #[test]
    fn smartbox_mapping_rejects_malformed_buckets_instead_of_faking_empty_results() {
        let data = json!({"items": {}, "search_id": "session"});
        let error = map_smartbox_response(
            "周杰伦",
            SearchSuggestionClient::Mobile,
            "requested-session",
            QqApiResponse {
                data: data.clone(),
                raw: json!({"code": 0, "data": data}),
            },
        )
        .expect_err("malformed SmartBox bucket");
        assert_eq!(error.code, ErrorCode::UpstreamError);
        assert_eq!(error.platform, Some(Platform::Qq));
    }

    #[test]
    fn quick_search_mapping_orders_dynamic_sections_and_keeps_typed_resources() {
        let result = map_quick_search_response(
            "周杰伦",
            json!({
                "code": 0,
                "subcode": 0,
                "data": {
                    "mv": {
                        "count": 1,
                        "name": "MV",
                        "order": 3,
                        "type": 4,
                        "itemlist": [{
                            "docid": "293791",
                            "id": "293791",
                            "mid": "00061J2t0b0PPW",
                            "name": "晴天",
                            "singer": "周杰伦",
                            "vid": "w0026q7f01a"
                        }]
                    },
                    "album": {
                        "count": 1,
                        "name": "专辑",
                        "order": 2,
                        "type": 3,
                        "itemlist": [{
                            "docid": "1713",
                            "id": "1713",
                            "mid": "002Neh8l0uciQZ",
                            "name": "叶惠美",
                            "singer": "周杰伦"
                        }]
                    },
                    "ticket": {
                        "count": 1,
                        "name": "未来资源",
                        "order": 4,
                        "type": 99,
                        "itemlist": [{"id": "future-1", "name": "未来入口"}]
                    },
                    "singer": {
                        "count": 1,
                        "name": "歌手",
                        "order": 1,
                        "type": 2,
                        "itemlist": [{
                            "docid": "4558",
                            "id": "4558",
                            "mid": "0025NhlN2yWrP4",
                            "name": "周杰伦",
                            "pic": "https://example.test/artist.jpg",
                            "singer": "周杰伦"
                        }]
                    },
                    "song": {
                        "count": 1,
                        "name": "单曲",
                        "order": 0,
                        "type": 1,
                        "itemlist": [{
                            "docid": "97773",
                            "id": "97773",
                            "mid": "0039MnYb0qxYhV",
                            "name": "晴天",
                            "singer": "周杰伦"
                        }]
                    }
                }
            }),
        )
        .expect("map quick search");

        assert_eq!(result.client, SearchSuggestionClient::Web);
        assert_eq!(result.suggestions.len(), 5);
        assert_eq!(
            result
                .suggestions
                .iter()
                .map(|suggestion| suggestion.kind)
                .collect::<Vec<_>>(),
            [
                Some(SearchKind::Track),
                Some(SearchKind::Artist),
                Some(SearchKind::Album),
                Some(SearchKind::Mv),
                None,
            ]
        );

        let Some(SearchItem::Track(track)) = &result.suggestions[0].resource else {
            panic!("expected track suggestion");
        };
        assert_eq!(track.resource_ref.to_string(), "qq:0039MnYb0qxYhV");
        assert_eq!(track.artists[0].name, "周杰伦");

        let Some(SearchItem::Artist(artist)) = &result.suggestions[1].resource else {
            panic!("expected artist suggestion");
        };
        assert_eq!(artist.resource_ref.to_string(), "qq:0025NhlN2yWrP4");

        let Some(SearchItem::Album(album)) = &result.suggestions[2].resource else {
            panic!("expected album suggestion");
        };
        assert_eq!(album.resource_ref.to_string(), "qq:002Neh8l0uciQZ");
        assert_eq!(album.artists[0].name, "周杰伦");

        let Some(SearchItem::Video(video)) = &result.suggestions[3].resource else {
            panic!("expected MV suggestion");
        };
        assert_eq!(video.resource_ref.to_string(), "qq:w0026q7f01a");
        assert_eq!(video.creators[0].name, "周杰伦");

        let Some(SearchItem::Opaque(future)) = &result.suggestions[4].resource else {
            panic!("expected opaque future suggestion");
        };
        assert_eq!(future.kind, "ticket");
        assert_eq!(future.id.as_deref(), Some("future-1"));
        assert_eq!(result.suggestions[4].extensions["section_order"], 4);
        assert_eq!(result.extensions["response"]["code"], 0);
    }

    #[test]
    fn quick_search_mapping_rejects_business_failures_and_malformed_known_sections() {
        for response in [
            json!({"code": 1001, "subcode": 0, "data": {}}),
            json!({"code": 0, "subcode": 2, "data": {}}),
            json!({"code": 0, "subcode": 0}),
            json!({
                "code": 0,
                "subcode": 0,
                "data": {"song": {"order": 0, "type": 1}}
            }),
            json!({
                "code": 0,
                "subcode": 0,
                "data": {"song": {"order": 0, "type": 1, "itemlist": {}}}
            }),
        ] {
            let error = map_quick_search_response("周杰伦", response)
                .expect_err("invalid quick search response");
            assert_eq!(error.code, ErrorCode::UpstreamError);
            assert_eq!(error.platform, Some(Platform::Qq));
        }
    }

    #[test]
    fn hotkey_mapping_preserves_rank_detail_and_platform_metadata() {
        let request = hotkey_request("session-42");
        assert_eq!(request.module, HOTKEY_MODULE);
        assert_eq!(request.method, HOTKEY_METHOD);
        assert_eq!(request.param, json!({"search_id": "session-42"}));

        let data = json!({
            "ret_code": 0,
            "expid": "1462002",
            "hotkey_time": "20260722108",
            "track_list_id": "20260722108",
            "vec_hotkey": [{
                "query": "周杰伦",
                "title": "716周杰伦日",
                "description": "一年一度周杰伦日",
                "score": "809039",
                "hotkey_id": "3.2.1.0:周杰伦",
                "direct_id": 97773,
                "cover_pic_url": "https://example.test/album.jpg",
                "custom_param": {"track_id": "97773"},
                "seqence": {"seqence_kind": 4, "seqence_value": 1},
                "kind": 1,
                "need_top": 1,
                "source": 2,
                "type": 3
            }, {
                "query": "无人之岛",
                "title": "无人之岛",
                "description": "正在热搜",
                "score": "498012",
                "seqence": {"seqence_kind": 2, "seqence_value": 4}
            }]
        });
        let full = map_hotkey_response(
            SearchTrendingDetail::Full,
            "session-42",
            QqApiResponse {
                data: data.clone(),
                raw: json!({"code": 0, "data": data.clone()}),
            },
        )
        .expect("map full hotkeys");
        assert_eq!(full.entries.len(), 2);
        assert_eq!(full.entries[0].rank, 1);
        assert_eq!(full.entries[0].keyword, "周杰伦");
        assert_eq!(
            full.entries[0].description.as_deref(),
            Some("一年一度周杰伦日")
        );
        assert_eq!(full.entries[0].score, Some(809_039));
        assert_eq!(full.entries[0].icon_type, Some(4));
        assert_eq!(full.entries[0].extensions["display_title"], "716周杰伦日");
        assert_eq!(full.entries[0].extensions["track_id"], "97773");
        assert_eq!(full.entries[1].rank, 2);
        assert_eq!(full.extensions["search_id"], "session-42");
        assert_eq!(full.extensions["hotkey_time"], "20260722108");
        assert_eq!(full.extensions["response"]["data"]["ret_code"], 0);

        let brief = map_hotkey_response(
            SearchTrendingDetail::Brief,
            "session-43",
            QqApiResponse {
                data: data.clone(),
                raw: json!({"code": 0, "data": data}),
            },
        )
        .expect("map brief hotkeys");
        assert_eq!(brief.entries[0].keyword, "周杰伦");
        assert_eq!(brief.entries[0].description, None);
        assert_eq!(brief.entries[0].score, None);
        assert_eq!(brief.entries[0].icon_type, None);
        assert_eq!(brief.entries[0].extensions["direct_id"], 97773);
    }

    #[test]
    fn hotkey_mapping_rejects_business_failures_and_missing_catalogs() {
        for data in [json!({"ret_code": 1001}), json!({"ret_code": 0})] {
            let error = map_hotkey_response(
                SearchTrendingDetail::Full,
                "session",
                QqApiResponse {
                    data: data.clone(),
                    raw: json!({"code": 0, "data": data}),
                },
            )
            .expect_err("invalid hotkey response");
            assert_eq!(error.code, ErrorCode::UpstreamError);
            assert_eq!(error.platform, Some(Platform::Qq));
        }
    }

    #[tokio::test]
    async fn suggestions_reject_unsupported_clients_and_accounts_before_network_access() {
        let provider = QqProvider::new(QqConfig::default()).expect("provider");
        assert!(
            provider
                .capabilities()
                .contains(&Capability::SearchSuggestions)
        );
        assert!(
            provider
                .capabilities()
                .contains(&Capability::SearchTrending)
        );
        let mut request = SearchSuggestionRequest {
            query: "周杰伦".to_owned(),
            client: SearchSuggestionClient::Pc,
            account: None,
        };
        let error = provider
            .search_suggestions(&request)
            .await
            .expect_err("PC client has no dedicated upstream branch");
        assert_eq!(error.code, ErrorCode::InvalidRequest);

        request.client = SearchSuggestionClient::Mobile;
        request.account = Some("green-diamond".to_owned());
        let error = provider
            .search_suggestions(&request)
            .await
            .expect_err("unconfigured account");
        assert_eq!(error.code, ErrorCode::AuthenticationRequired);

        let error = provider
            .trending_searches(&SearchTrendingRequest {
                detail: SearchTrendingDetail::Full,
                account: Some("green-diamond".to_owned()),
            })
            .await
            .expect_err("unconfigured trending account");
        assert_eq!(error.code, ErrorCode::AuthenticationRequired);
    }

    #[tokio::test]
    async fn track_queries_reject_invalid_batches_and_accounts_before_network_access() {
        let provider = QqProvider::new(QqConfig::default()).expect("provider");
        assert!(provider.capabilities().contains(&Capability::TrackDetail));

        let empty = provider
            .tracks(&[], None)
            .await
            .expect_err("empty track batch");
        assert_eq!(empty.code, ErrorCode::InvalidRequest);

        let mixed = provider
            .tracks(&["100".to_owned(), "003w2xz20QlUZt".to_owned()], None)
            .await
            .expect_err("mixed numeric and MID batch");
        assert_eq!(mixed.code, ErrorCode::InvalidRequest);

        let account = provider
            .track("100", Some("green-diamond"))
            .await
            .expect_err("unconfigured account");
        assert_eq!(account.code, ErrorCode::AuthenticationRequired);
    }

    #[test]
    fn sparse_playlist_pages_advance_by_upstream_slots_without_duplicates() {
        let first = (0..29)
            .map(|id| json!({"id": id + 1, "title": format!("playlist{id}")}))
            .collect::<Vec<_>>();
        let second = (30..59)
            .map(|id| json!({"id": id + 1, "title": format!("playlist{id}")}))
            .collect::<Vec<_>>();
        let page = map_catalog_search_response(
            25,
            20,
            25,
            vec![
                response(
                    json!({"code": 0, "meta": {"sum": 100}, "body": {"item_songlist": first}}),
                ),
                response(
                    json!({"code": 0, "meta": {"sum": 100}, "body": {"item_songlist": second}}),
                ),
            ],
            PLAYLIST_SEARCH,
            map_playlist_search_item,
        )
        .expect("map sparse playlist page");
        assert_eq!(page.items.len(), 19);
        let SearchItem::Playlist(first) = &page.items[0] else {
            panic!("expected playlist");
        };
        let SearchItem::Playlist(last) = &page.items[18] else {
            panic!("expected playlist");
        };
        assert_eq!(first.name, "playlist25");
        assert_eq!(last.name, "playlist44");
        assert_eq!(page.pagination.next_offset, Some(45));
        assert_eq!(page.pagination.extensions["omitted_slots"], 1);
        assert_eq!(
            page.pagination.extensions["upstream_item_counts"],
            json!([29, 29])
        );
    }

    #[test]
    fn reported_nonempty_result_cannot_be_silently_empty() {
        let error = map_track_search_response(
            0,
            10,
            0,
            vec![response(json!({
                "code": 0,
                "meta": {"sum": 1},
                "body": {"item_song": []}
            }))],
        )
        .expect_err("empty result must fail");
        assert_eq!(error.code, ErrorCode::UpstreamError);
    }

    #[test]
    fn missing_search_data_code_cannot_be_silently_successful() {
        let error = map_track_search_response(
            0,
            10,
            0,
            vec![response(json!({
                "meta": {"sum": 0},
                "body": {"item_song": []}
            }))],
        )
        .expect_err("missing data code must fail");
        assert_eq!(error.code, ErrorCode::UpstreamError);
        assert!(error.message.contains("missing a valid data code"));
    }

    #[tokio::test]
    async fn unsupported_variant_and_unconfigured_account_fail_before_network() {
        let provider = QqProvider::new(QqConfig::default()).expect("provider");
        let mut query = SearchQuery::tracks("周杰伦", 2, 0);
        query.variant = SearchVariant::Cloud;
        let variant_error = provider.search(&query).await.expect_err("variant failure");
        assert_eq!(variant_error.code, ErrorCode::InvalidRequest);

        query.variant = SearchVariant::Default;
        query.account = Some("green-diamond".to_owned());
        let account_error = provider.search(&query).await.expect_err("account failure");
        assert_eq!(account_error.code, ErrorCode::AuthenticationRequired);

        let mut album_query = search_query(SearchKind::Album, 2, 0);
        album_query.variant = SearchVariant::Legacy;
        let variant_error = provider
            .search_catalog(&album_query)
            .await
            .expect_err("catalog variant failure");
        assert_eq!(variant_error.code, ErrorCode::InvalidRequest);

        album_query.variant = SearchVariant::Default;
        album_query.account = Some("green-diamond".to_owned());
        let account_error = provider
            .search_catalog(&album_query)
            .await
            .expect_err("catalog account failure");
        assert_eq!(account_error.code, ErrorCode::AuthenticationRequired);
    }

    #[tokio::test]
    #[ignore = "requires live QQ Music services"]
    async fn live_track_search_returns_real_metadata() {
        let provider = QqProvider::new(QqConfig {
            device_path: std::env::var_os("TUNEWEAVE_QQ_LIVE_DEVICE").map(Into::into),
            ..QqConfig::default()
        })
        .expect("provider");
        let page = provider
            .search(&SearchQuery::tracks("周杰伦", 2, 0))
            .await
            .expect("live search");
        assert_eq!(page.items.len(), 2);
        assert!(page.pagination.total.is_some_and(|total| total > 0));
        assert!(page.items.iter().all(|track| !track.name.is_empty()));
        assert!(
            page.items
                .iter()
                .all(|track| track.extensions.contains_key("media_mid"))
        );
    }

    #[tokio::test]
    #[ignore = "requires live QQ Music services"]
    async fn live_batch_track_query_accepts_numeric_ids_and_mids() {
        let provider = QqProvider::new(QqConfig {
            device_path: std::env::var_os("TUNEWEAVE_QQ_LIVE_DEVICE").map(Into::into),
            ..QqConfig::default()
        })
        .expect("provider");
        let numeric = provider
            .tracks(&["100".to_owned(), "100".to_owned()], None)
            .await
            .expect("numeric batch");
        assert_eq!(numeric.len(), 2);
        assert_eq!(numeric[0].resource_ref, numeric[1].resource_ref);
        assert_eq!(numeric[0].extensions["numeric_id"], "100");

        let mids = provider
            .tracks(&["003w2xz20QlUZt".to_owned()], None)
            .await
            .expect("MID batch");
        assert_eq!(mids.len(), 1);
        assert_eq!(mids[0].resource_ref.to_string(), "qq:003w2xz20QlUZt");
        assert!(mids[0].extensions["media_mid"].as_str().is_some());
        assert!(mids[0].extensions.contains_key("song_type"));
    }

    #[tokio::test]
    #[ignore = "requires live QQ Music services"]
    async fn live_rich_track_detail_accepts_numeric_id_and_mid() {
        let provider = QqProvider::new(QqConfig::default()).expect("provider");
        let numeric = provider.track("100", None).await.expect("numeric detail");
        assert_eq!(numeric.extensions["numeric_id"], "100");
        assert_eq!(numeric.extensions["detail_identifier_kind"], "numeric_id");
        assert!(numeric.extensions["detail_info"].is_object());
        assert!(numeric.extensions["detail_extras"].is_object());
        assert_eq!(numeric.extensions["detail_response"]["code"], 0);

        let mid = provider
            .track("003w2xz20QlUZt", None)
            .await
            .expect("MID detail");
        assert_eq!(mid.resource_ref.to_string(), "qq:003w2xz20QlUZt");
        assert_eq!(mid.extensions["detail_identifier_kind"], "mid");
        assert_eq!(mid.extensions["detail_response"]["code"], 0);
    }

    #[tokio::test]
    #[ignore = "requires live QQ Music services"]
    async fn live_lyrics_cover_lrc_qrc_translation_romanization_id_and_mid() {
        let provider = QqProvider::new(QqConfig {
            device_path: std::env::var_os("TUNEWEAVE_QQ_LIVE_DEVICE").map(Into::into),
            ..QqConfig::default()
        })
        .expect("provider");
        let line_synced = provider.lyrics("100", None).await.expect("numeric LRC");
        assert_eq!(line_synced.track_ref.to_string(), "qq:100");
        assert_eq!(line_synced.format, "lrc");
        assert!(line_synced.plain.is_some());
        assert!(line_synced.word_synced.is_none());

        let rich = provider
            .lyrics_with_options(
                "000akynZ2Rbro5",
                &LyricsRequest {
                    word_synced: true,
                    translated: true,
                    romanized: true,
                    song_type: Some(1),
                    account: None,
                },
            )
            .await
            .expect("MID rich lyrics");
        assert_eq!(rich.track_ref.to_string(), "qq:000akynZ2Rbro5");
        assert_eq!(rich.format, "qrc");
        assert!(rich.word_synced.is_some());
        assert!(rich.plain.is_none());
        assert!(rich.translated.is_some());
        assert!(rich.romanized.is_some());
        assert_eq!(rich.extensions["numeric_id"], "213086592");
        assert_eq!(rich.extensions["requested_options"]["qrc"], true);
        assert_eq!(rich.extensions["response"]["code"], 0);
    }

    #[tokio::test]
    #[ignore = "requires live QQ Music services"]
    async fn live_cdn_dispatch_returns_usable_roots_nodes_and_timers() {
        let provider = QqProvider::new(QqConfig {
            device_path: std::env::var_os("TUNEWEAVE_QQ_LIVE_DEVICE").map(Into::into),
            ..QqConfig::default()
        })
        .expect("provider");
        let dispatch = provider
            .audio_cdn_dispatch(None)
            .await
            .expect("live CDN dispatch");
        assert!(!dispatch.roots.is_empty());
        assert!(
            dispatch
                .roots
                .iter()
                .all(|root| root.starts_with("http://") || root.starts_with("https://"))
        );
        assert!(!dispatch.nodes.is_empty());
        assert!(!dispatch.test_file.is_empty());
        assert!(dispatch.expires_in_seconds > 0);
        assert!(dispatch.refresh_after_seconds > 0);
        assert!(dispatch.cache_for_seconds > 0);
        assert_eq!(
            dispatch.extensions["request_guid"].as_str().map(str::len),
            Some(32)
        );
        assert_eq!(dispatch.extensions["response"]["code"], 0);
    }

    #[tokio::test]
    #[ignore = "requires live QQ Music services"]
    async fn live_song_urls_cover_normal_encrypted_and_every_special_file_spec() {
        let provider = QqProvider::new(QqConfig {
            device_path: std::env::var_os("TUNEWEAVE_QQ_LIVE_DEVICE").map(Into::into),
            ..QqConfig::default()
        })
        .expect("provider");
        let reference = qq_ref("003w2xz20QlUZt", "track").expect("reference");
        for (range, default_spec) in [
            (0..17, "mp3_128"),
            (17..30, "encrypted_flac"),
            (30..45, "trial"),
        ] {
            let specs = &QQ_AUDIO_FILE_SPECS[range];
            let batch = provider
                .audio_files(&AudioFileRequest {
                    items: specs
                        .iter()
                        .map(|spec| tuneweave_core::AudioFileRequestItem {
                            track_ref: reference.clone(),
                            spec: Some(spec.name.to_owned()),
                            song_type: Some(1),
                            media_id: Some("003w2xz20QlUZt".to_owned()),
                        })
                        .collect(),
                    default_spec: Some(default_spec.to_owned()),
                    account: None,
                })
                .await
                .expect("live song URLs");
            assert_eq!(batch.files.len(), specs.len());
            assert!(batch.expires_in_seconds > 0);
            for (file, spec) in batch.files.iter().zip(specs) {
                assert_eq!(file.spec, spec.name);
                assert_eq!(file.encrypted, spec.encrypted);
                assert_eq!(file.available, file.platform_code == 0);
                if file.available {
                    assert!(file.relative_url.is_some());
                    assert!(file.access_token.is_some());
                    if file.encrypted {
                        assert!(file.decryption_key.is_some());
                    }
                }
            }
            assert_eq!(batch.extensions["response"]["code"], 0);
        }
    }

    #[tokio::test]
    #[ignore = "requires live QQ Music services"]
    async fn live_mobile_search_suggestions_keep_keywords_and_direct_results() {
        let provider = QqProvider::new(QqConfig {
            device_path: std::env::var_os("TUNEWEAVE_QQ_LIVE_DEVICE").map(Into::into),
            ..QqConfig::default()
        })
        .expect("provider");
        let result = provider
            .search_suggestions(&SearchSuggestionRequest {
                query: "周杰伦".to_owned(),
                client: SearchSuggestionClient::Mobile,
                account: Some("default".to_owned()),
            })
            .await
            .expect("live smartbox suggestions");
        assert_eq!(result.query, "周杰伦");
        assert_eq!(result.client, SearchSuggestionClient::Mobile);
        assert!(!result.suggestions.is_empty());
        assert!(
            result
                .suggestions
                .iter()
                .all(|suggestion| !suggestion.keyword.is_empty())
        );
        assert!(result.extensions["search_id"].as_str().is_some());
        assert!(result.extensions["response"]["data"].is_object());
    }

    #[tokio::test]
    #[ignore = "requires live QQ Music services"]
    async fn live_web_quick_search_returns_typed_sections() {
        let provider = QqProvider::new(QqConfig::default()).expect("provider");
        let result = provider
            .search_suggestions(&SearchSuggestionRequest {
                query: "周杰伦".to_owned(),
                client: SearchSuggestionClient::Web,
                account: None,
            })
            .await
            .expect("live quick search");
        assert_eq!(result.client, SearchSuggestionClient::Web);
        assert!(!result.suggestions.is_empty());
        assert!(
            result
                .suggestions
                .iter()
                .all(|item| item.resource.is_some())
        );
        for expected in [
            SearchKind::Track,
            SearchKind::Artist,
            SearchKind::Album,
            SearchKind::Mv,
        ] {
            assert!(
                result
                    .suggestions
                    .iter()
                    .any(|suggestion| suggestion.kind == Some(expected)),
                "missing {expected:?} quick-search section"
            );
        }
        assert_eq!(result.extensions["response"]["code"], 0);
        assert_eq!(result.extensions["response"]["subcode"], 0);
    }

    #[tokio::test]
    #[ignore = "requires live QQ Music services"]
    async fn live_hotkeys_return_ranked_rich_entries() {
        let provider = QqProvider::new(QqConfig {
            device_path: std::env::var_os("TUNEWEAVE_QQ_LIVE_DEVICE").map(Into::into),
            ..QqConfig::default()
        })
        .expect("provider");
        let result = provider
            .trending_searches(&SearchTrendingRequest {
                detail: SearchTrendingDetail::Full,
                account: Some("default".to_owned()),
            })
            .await
            .expect("live hotkeys");
        assert_eq!(result.detail, SearchTrendingDetail::Full);
        assert!(!result.entries.is_empty());
        assert!(
            result
                .entries
                .iter()
                .enumerate()
                .all(
                    |(index, entry)| entry.rank == u32::try_from(index + 1).unwrap_or(u32::MAX)
                        && !entry.keyword.is_empty()
                )
        );
        assert_eq!(result.extensions["response"]["data"]["ret_code"], 0);
    }

    #[tokio::test]
    #[ignore = "requires live QQ Music services"]
    async fn live_user_podcast_and_voice_search_share_one_batch() {
        let client = QqClient::new(QqConfig {
            device_path: std::env::var_os("TUNEWEAVE_QQ_LIVE_DEVICE").map(Into::into),
            ..QqConfig::default()
        })
        .expect("client");
        let search_id = generate_search_id().expect("search ID");
        let categories: [(SearchKind, TypedSearchSpec, SearchItemMapper); 3] = [
            (SearchKind::User, USER_SEARCH, map_user_search_item),
            (SearchKind::Podcast, PODCAST_SEARCH, map_podcast_search_item),
            (SearchKind::Voice, VOICE_SEARCH, map_voice_search_item),
        ];
        let requests = categories
            .iter()
            .map(|(_, spec, _)| {
                typed_search_request(
                    "周杰伦",
                    &search_id,
                    spec.code,
                    1,
                    spec.upstream_page_size,
                    false,
                )
            })
            .collect::<Vec<_>>();
        let responses = client
            .request_android(&requests)
            .await
            .expect("live batched catalog search");

        for ((kind, spec, mapper), response) in categories.into_iter().zip(responses) {
            let page = map_catalog_search_response(0, 2, 0, vec![response], spec, mapper)
                .expect("map live catalog response");
            assert_eq!(page.items.len(), 2);
            assert!(page.pagination.total.is_some_and(|total| total > 0));
            assert!(page.items.iter().all(|item| match (kind, item) {
                (SearchKind::User, SearchItem::User(user)) => !user.name.is_empty(),
                (SearchKind::Podcast, SearchItem::Podcast(podcast)) => !podcast.name.is_empty(),
                (SearchKind::Voice, SearchItem::PodcastEpisode(episode)) => {
                    !episode.name.is_empty() && episode.audio.is_some()
                }
                _ => false,
            }));
        }
    }

    #[tokio::test]
    #[ignore = "requires live QQ Music services"]
    async fn live_all_typed_search_categories_return_typed_catalogs() {
        let provider = QqProvider::new(QqConfig {
            device_path: std::env::var_os("TUNEWEAVE_QQ_LIVE_DEVICE").map(Into::into),
            ..QqConfig::default()
        })
        .expect("provider");
        for kind in [
            SearchKind::Artist,
            SearchKind::Album,
            SearchKind::Playlist,
            SearchKind::Mv,
            SearchKind::Lyric,
            SearchKind::User,
            SearchKind::Podcast,
            SearchKind::Voice,
        ] {
            let page = provider
                .search_catalog(&search_query(kind, 2, 0))
                .await
                .expect("live catalog search");
            assert_eq!(page.items.len(), 2);
            assert!(page.pagination.total.is_some_and(|total| total > 0));
            assert!(page.items.iter().all(|item| {
                match (kind, item) {
                    (SearchKind::Artist, SearchItem::Artist(artist)) => !artist.name.is_empty(),
                    (SearchKind::Album, SearchItem::Album(album)) => !album.name.is_empty(),
                    (SearchKind::Playlist, SearchItem::Playlist(playlist)) => {
                        !playlist.name.is_empty()
                    }
                    (SearchKind::Mv, SearchItem::Video(video)) => !video.title.is_empty(),
                    (SearchKind::Lyric, SearchItem::Track(track)) => track
                        .extensions
                        .get("search_content")
                        .and_then(Value::as_str)
                        .is_some_and(|content| !content.is_empty()),
                    (SearchKind::User, SearchItem::User(user)) => !user.name.is_empty(),
                    (SearchKind::Podcast, SearchItem::Podcast(podcast)) => !podcast.name.is_empty(),
                    (SearchKind::Voice, SearchItem::PodcastEpisode(episode)) => {
                        !episode.name.is_empty() && episode.audio.is_some()
                    }
                    _ => false,
                }
            }));
        }
    }
}
