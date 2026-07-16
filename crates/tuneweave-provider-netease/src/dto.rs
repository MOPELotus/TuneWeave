use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Deserialize)]
pub(crate) struct SearchEnvelope {
    pub result: SearchResult,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SearchResult {
    #[serde(default)]
    pub songs: Vec<Song>,
    #[serde(rename = "songCount", default)]
    pub song_count: u64,
}

#[derive(Debug, Deserialize)]
pub(crate) struct TrackEnvelope {
    #[serde(default)]
    pub songs: Vec<Song>,
    #[serde(default)]
    pub privileges: Vec<Privilege>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct AlbumEnvelope {
    pub album: AlbumDetail,
    #[serde(default)]
    pub songs: Vec<Song>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct AlbumListEnvelope {
    #[serde(default)]
    pub albums: Vec<Value>,
    pub total: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct AlbumDetail {
    pub id: u64,
    pub name: String,
    #[serde(default, alias = "alias")]
    pub alia: Vec<String>,
    #[serde(default)]
    pub artists: Vec<Artist>,
    pub description: Option<String>,
    #[serde(rename = "picUrl")]
    pub pic_url: Option<String>,
    #[serde(rename = "publishTime")]
    pub publish_time: Option<u64>,
    pub size: Option<u64>,
    pub company: Option<String>,
    #[serde(rename = "type")]
    pub kind: Option<String>,
    #[serde(rename = "subType")]
    pub sub_type: Option<String>,
    pub paid: Option<bool>,
    #[serde(rename = "onSale")]
    pub on_sale: Option<bool>,
    pub mark: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct AlbumStatsEnvelope {
    #[serde(rename = "isSub")]
    pub subscribed: Option<bool>,
    #[serde(rename = "subCount")]
    pub subscriber_count: Option<u64>,
    #[serde(rename = "commentCount")]
    pub comment_count: Option<u64>,
    #[serde(rename = "shareCount")]
    pub share_count: Option<u64>,
    #[serde(rename = "likedCount")]
    pub like_count: Option<u64>,
    #[serde(rename = "onSale")]
    pub on_sale: Option<bool>,
    #[serde(rename = "subTime")]
    pub subscribed_at: Option<u64>,
    #[serde(rename = "albumGameInfo")]
    pub album_game_info: Option<Value>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct DigitalAlbumEnvelope {
    pub album: Option<DigitalAlbumInfo>,
    pub product: Option<DigitalAlbumProduct>,
    #[serde(rename = "canBuy")]
    pub can_buy: Option<bool>,
    #[serde(rename = "hasAlbum")]
    pub has_album: Option<bool>,
    #[serde(rename = "boughtCnt")]
    pub bought_count: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct DigitalAlbumInfo {
    #[serde(rename = "albumId")]
    pub album_id: u64,
    #[serde(rename = "albumName")]
    pub album_name: String,
    #[serde(rename = "artistId")]
    pub artist_id: Option<u64>,
    #[serde(rename = "artistName")]
    pub artist_name: Option<String>,
    #[serde(rename = "artistNames")]
    pub artist_names: Option<String>,
    #[serde(rename = "coverUrl")]
    pub cover_url: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct DigitalAlbumProduct {
    pub price: Option<f64>,
    #[serde(rename = "isFree")]
    pub is_free: Option<bool>,
    #[serde(rename = "pubTime")]
    pub publish_time: Option<u64>,
    #[serde(rename = "saleNum")]
    pub sale_count: Option<u64>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub descr: Vec<DigitalAlbumDescription>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct DigitalAlbumDescription {
    pub resource: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct DigitalAlbumListEnvelope {
    #[serde(default)]
    pub products: Vec<Value>,
    #[serde(rename = "albumProducts", default)]
    pub album_products: Vec<Value>,
    #[serde(rename = "hasNextPage")]
    pub has_next_page: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct DigitalAlbumListItem {
    #[serde(rename = "albumId")]
    pub album_id: u64,
    #[serde(rename = "albumName")]
    pub album_name: String,
    #[serde(rename = "artistName")]
    pub artist_name: Option<String>,
    #[serde(rename = "coverUrl")]
    pub cover_url: Option<String>,
    pub price: Option<f64>,
    #[serde(rename = "pubTime")]
    pub publish_time: Option<u64>,
    #[serde(rename = "saleNum", alias = "sales")]
    pub sale_count: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct PlaylistEnvelope {
    pub playlist: Option<PlaylistDetail>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct UserPlaylistsEnvelope {
    #[serde(default)]
    pub playlist: Vec<PlaylistDetail>,
    #[serde(default)]
    pub more: bool,
}

#[derive(Debug, Deserialize)]
pub(crate) struct LikedTracksEnvelope {
    #[serde(default)]
    pub ids: Vec<u64>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct PlayHistoryEnvelope {
    #[serde(rename = "allData", default)]
    pub all_data: Vec<PlayHistoryRecord>,
    #[serde(rename = "weekData", default)]
    pub week_data: Vec<PlayHistoryRecord>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct PlayHistoryRecord {
    pub song: Song,
    #[serde(rename = "playCount")]
    pub play_count: Option<u64>,
    pub score: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct RecommendedTracksEnvelope {
    pub data: RecommendedTracksData,
}

#[derive(Debug, Deserialize)]
pub(crate) struct RecommendedTracksData {
    #[serde(rename = "dailySongs", default)]
    pub daily_songs: Vec<Song>,
    #[serde(rename = "recommendReasons", default)]
    pub recommend_reasons: Vec<RecommendationReason>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct RecommendationReason {
    #[serde(rename = "songId")]
    pub song_id: u64,
    pub reason: Option<String>,
    #[serde(rename = "reasonId")]
    pub reason_id: Option<Value>,
    #[serde(rename = "targetUrl")]
    pub target_url: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct RecommendedPlaylistsEnvelope {
    #[serde(default)]
    pub recommend: Vec<PlaylistDetail>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct PlaylistDetail {
    pub id: u64,
    pub name: String,
    pub description: Option<String>,
    #[serde(rename = "coverImgUrl", alias = "picUrl")]
    pub cover_img_url: Option<String>,
    pub creator: Option<PlaylistCreator>,
    #[serde(rename = "trackCount")]
    pub track_count: Option<u64>,
    #[serde(default)]
    pub tags: Vec<String>,
    pub subscribed: Option<bool>,
    #[serde(rename = "createTime")]
    pub create_time: Option<u64>,
    #[serde(rename = "updateTime")]
    pub update_time: Option<u64>,
    pub privacy: Option<i64>,
    #[serde(rename = "specialType")]
    pub special_type: Option<i64>,
    #[serde(rename = "playCount")]
    pub play_count: Option<u64>,
    pub copywriter: Option<String>,
    pub alg: Option<String>,
    #[serde(rename = "trackIds", default)]
    pub track_ids: Vec<PlaylistTrackId>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct PlaylistCreator {
    #[serde(rename = "userId")]
    pub user_id: u64,
    pub nickname: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct PlaylistTrackId {
    pub id: u64,
}

#[derive(Debug, Deserialize)]
pub(crate) struct LyricsEnvelope {
    pub lrc: Option<LyricText>,
    pub tlyric: Option<LyricText>,
    pub romalrc: Option<LyricText>,
    pub yrc: Option<LyricText>,
    pub ytlrc: Option<LyricText>,
    pub yromalrc: Option<LyricText>,
    #[serde(rename = "lyricUser")]
    pub lyric_user: Option<LyricUser>,
    #[serde(rename = "transUser")]
    pub trans_user: Option<LyricUser>,
    #[serde(rename = "pureMusic")]
    pub pure_music: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct LyricText {
    pub lyric: Option<String>,
    pub version: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct LyricUser {
    pub id: Option<u64>,
    pub userid: Option<u64>,
    #[serde(rename = "userId")]
    pub user_id: Option<u64>,
    pub nickname: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct StreamEnvelope {
    #[serde(default)]
    pub data: Vec<StreamData>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct StreamData {
    pub id: u64,
    pub url: Option<String>,
    pub br: Option<u64>,
    pub size: Option<u64>,
    pub code: Option<i64>,
    pub expi: Option<u64>,
    #[serde(rename = "type")]
    pub kind: Option<String>,
    pub level: Option<String>,
    #[serde(rename = "encodeType")]
    pub encode_type: Option<String>,
    pub time: Option<u64>,
    pub fee: Option<i64>,
    #[serde(rename = "freeTrialInfo")]
    pub free_trial_info: Option<FreeTrialInfo>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct FreeTrialInfo {
    pub start: Option<u64>,
    pub end: Option<u64>,
}

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct Song {
    pub id: u64,
    pub name: String,
    #[serde(default, alias = "alias")]
    pub alia: Vec<String>,
    #[serde(default, alias = "artists")]
    pub ar: Vec<Artist>,
    #[serde(alias = "album")]
    pub al: Option<Album>,
    #[serde(alias = "duration")]
    pub dt: Option<u64>,
    #[serde(alias = "mvid")]
    pub mv: Option<u64>,
    pub fee: Option<i64>,
    #[serde(alias = "status")]
    pub st: Option<i64>,
    pub mark: Option<u64>,
    pub privilege: Option<Privilege>,
    #[serde(alias = "lMusic")]
    pub l: Option<AudioQuality>,
    #[serde(alias = "mMusic")]
    pub m: Option<AudioQuality>,
    #[serde(alias = "hMusic")]
    pub h: Option<AudioQuality>,
    #[serde(alias = "sqMusic")]
    pub sq: Option<AudioQuality>,
    #[serde(alias = "hrMusic")]
    pub hr: Option<AudioQuality>,
}

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct Artist {
    pub id: u64,
    pub name: String,
}

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct Album {
    pub id: u64,
    pub name: String,
    #[serde(rename = "picUrl")]
    pub pic_url: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct Privilege {
    pub id: u64,
    #[serde(default)]
    pub st: i64,
    #[serde(default)]
    pub fee: i64,
    #[serde(default)]
    pub pl: u64,
    #[serde(default)]
    pub maxbr: u64,
}

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct AudioQuality {
    #[serde(alias = "bitrate")]
    pub br: Option<u64>,
}
