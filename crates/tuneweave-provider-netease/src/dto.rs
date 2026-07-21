use serde::{Deserialize, Deserializer};
use std::collections::BTreeMap;

use serde_json::Value;

fn deserialize_nullable_string<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    Ok(Option::<String>::deserialize(deserializer)?.unwrap_or_default())
}

fn deserialize_nullable_vec<'de, D, T>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Ok(Option::<Vec<T>>::deserialize(deserializer)?.unwrap_or_default())
}

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
pub(crate) struct AudioMatchEnvelope {
    pub data: AudioMatchData,
}

#[derive(Debug, Deserialize)]
pub(crate) struct AudioMatchData {
    #[serde(rename = "queryId")]
    pub query_id: Option<Value>,
    #[serde(rename = "noMatchReason")]
    pub no_match_reason: Option<i64>,
    pub result: Option<Vec<Value>>,
    #[serde(rename = "type")]
    pub kind: Option<Value>,
    pub mv: Option<Value>,
    #[serde(rename = "moduleList")]
    pub module_list: Option<Value>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ImageUploadAllocationEnvelope {
    pub result: ImageUploadAllocation,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ImageUploadAllocation {
    #[serde(rename = "objectKey")]
    pub object_key: String,
    pub token: String,
    #[serde(rename = "docId")]
    pub document_id: Value,
}

#[derive(Debug, Deserialize)]
pub(crate) struct CloudUploadAllocationEnvelope {
    pub result: CloudUploadAllocation,
}

#[derive(Debug, Deserialize)]
pub(crate) struct CloudUploadAllocation {
    #[serde(rename = "objectKey")]
    pub object_key: String,
    pub token: String,
    #[serde(rename = "resourceId")]
    pub resource_id: Value,
}

#[derive(Debug, Deserialize)]
pub(crate) struct CloudUploadServersEnvelope {
    #[serde(default)]
    pub upload: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct CloudTracksEnvelope {
    pub data: Option<Vec<Value>>,
    pub count: Option<Value>,
    #[serde(rename = "hasMore")]
    pub has_more: Option<Value>,
    pub size: Option<Value>,
    #[serde(rename = "maxSize")]
    pub max_size: Option<Value>,
    #[serde(rename = "upgradeSign")]
    pub upgrade_sign: Option<Value>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct BannerEnvelope {
    #[serde(default)]
    pub banners: Vec<Value>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct BroadcastTaxonomyEnvelope {
    pub data: BroadcastTaxonomyData,
}

#[derive(Debug, Deserialize)]
pub(crate) struct BroadcastTaxonomyData {
    #[serde(rename = "categoryList", default)]
    pub categories: Vec<Value>,
    #[serde(rename = "regionList", default)]
    pub regions: Vec<Value>,
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
pub(crate) struct ArtistAlbumsEnvelope {
    #[serde(rename = "hotAlbums", default)]
    pub albums: Vec<Value>,
    pub more: Option<bool>,
    pub total: Option<u64>,
    pub artist: Option<Value>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ArtistTracksEnvelope {
    #[serde(default)]
    pub songs: Vec<Value>,
    pub more: Option<bool>,
    pub total: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ArtistSublistEnvelope {
    #[serde(default)]
    pub data: Vec<Value>,
    pub count: Option<u64>,
    #[serde(rename = "hasMore")]
    pub has_more: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ArtistTopTracksEnvelope {
    #[serde(default)]
    pub songs: Vec<Value>,
    #[serde(default)]
    pub privileges: Vec<Privilege>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ArtistOverviewEnvelope {
    pub artist: Value,
    #[serde(rename = "hotSongs", default)]
    pub hot_songs: Vec<Value>,
    pub more: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ArtistListEnvelope {
    #[serde(default)]
    pub artists: Vec<Value>,
    pub more: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ArtistListItem {
    pub id: u64,
    pub name: String,
    #[serde(default)]
    pub alias: Vec<String>,
    #[serde(rename = "transNames", default)]
    pub translated_names: Vec<String>,
    pub trans: Option<String>,
    #[serde(rename = "briefDesc")]
    pub brief_description: Option<String>,
    #[serde(rename = "img1v1Url")]
    pub avatar_url: Option<String>,
    #[serde(rename = "picUrl")]
    pub cover_url: Option<String>,
    #[serde(rename = "albumSize")]
    pub album_count: Option<u64>,
    #[serde(rename = "musicSize")]
    pub track_count: Option<u64>,
    #[serde(rename = "mvSize")]
    pub mv_count: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ArtistMvsEnvelope {
    #[serde(rename = "hasMore")]
    pub has_more: Option<bool>,
    #[serde(default)]
    pub mvs: Vec<Value>,
    pub time: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ArtistMvItem {
    pub id: u64,
    pub name: String,
    pub artist: Option<ArtistMvCreator>,
    #[serde(default)]
    pub artists: Vec<ArtistMvCreator>,
    #[serde(rename = "artistId")]
    pub artist_id: Option<u64>,
    #[serde(rename = "artistName")]
    pub artist_name: Option<String>,
    pub duration: Option<u64>,
    pub imgurl: Option<String>,
    pub cover: Option<String>,
    #[serde(rename = "imgurl16v9")]
    pub image_16x9_url: Option<String>,
    #[serde(rename = "briefDesc")]
    pub brief_description: Option<String>,
    #[serde(rename = "desc")]
    pub description: Option<String>,
    #[serde(rename = "playCount")]
    pub play_count: Option<u64>,
    #[serde(rename = "publishTime")]
    pub published_at: Option<String>,
    pub subed: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ArtistMvCreator {
    pub id: u64,
    pub name: String,
    #[serde(rename = "img1v1Url")]
    pub avatar_url: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ArtistVideosEnvelope {
    pub data: ArtistVideosData,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ArtistVideosData {
    pub page: ArtistVideosPage,
    #[serde(default)]
    pub records: Vec<Value>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ArtistVideosPage {
    pub cursor: Option<Value>,
    pub more: Option<bool>,
    pub size: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ArtistVideoRecord {
    pub id: Option<Value>,
    pub resource: ArtistVideoResource,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ArtistVideoResource {
    #[serde(rename = "mlogBaseData")]
    pub base: ArtistVideoBaseData,
    #[serde(rename = "mlogExtVO")]
    pub extension: Option<ArtistVideoExtension>,
    #[serde(rename = "userProfile")]
    pub user_profile: Option<ArtistVideoUserProfile>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ArtistVideoBaseData {
    pub id: Option<Value>,
    pub text: Option<String>,
    #[serde(rename = "originalTitle")]
    pub original_title: Option<String>,
    pub desc: Option<String>,
    #[serde(rename = "coverUrl")]
    pub cover_url: Option<String>,
    pub duration: Option<u64>,
    #[serde(rename = "pubTime")]
    pub published_at_ms: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ArtistVideoExtension {
    #[serde(default)]
    pub artists: Vec<ArtistVideoCreator>,
    #[serde(rename = "artistName")]
    pub artist_name: Option<String>,
    #[serde(rename = "playCount")]
    pub play_count: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ArtistVideoCreator {
    pub id: Option<Value>,
    #[serde(default)]
    pub name: String,
    #[serde(rename = "img1v1Url")]
    pub avatar_url: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct MvDetailEnvelope {
    pub data: MvDetailData,
    pub subed: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct MvDetailData {
    pub id: u64,
    pub name: String,
    #[serde(rename = "artistId")]
    pub artist_id: Option<u64>,
    #[serde(rename = "artistName")]
    pub artist_name: Option<String>,
    #[serde(default)]
    pub artists: Vec<VideoCreatorItem>,
    #[serde(rename = "briefDesc")]
    pub brief_description: Option<String>,
    pub desc: Option<String>,
    pub cover: Option<String>,
    pub duration: Option<u64>,
    #[serde(rename = "publishTime")]
    pub published_at: Option<String>,
    #[serde(rename = "playCount")]
    pub play_count: Option<u64>,
    #[serde(default)]
    pub brs: Vec<Value>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct CloudVideoDetailEnvelope {
    pub data: CloudVideoDetailData,
}

#[derive(Debug, Deserialize)]
pub(crate) struct CloudVideoDetailData {
    pub vid: Value,
    pub title: String,
    pub description: Option<String>,
    #[serde(rename = "coverUrl")]
    pub cover_url: Option<String>,
    #[serde(rename = "publishTime")]
    pub published_at: Option<u64>,
    #[serde(rename = "durationms")]
    pub duration_ms: Option<u64>,
    #[serde(rename = "playTime")]
    pub play_count: Option<u64>,
    pub subed: Option<bool>,
    pub creator: Option<VideoCreatorItem>,
    #[serde(default)]
    pub resolutions: Vec<Value>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct VideoCreatorItem {
    #[serde(rename = "userId", alias = "id")]
    pub id: Option<Value>,
    #[serde(rename = "nickname", alias = "name", default)]
    pub name: String,
    #[serde(rename = "avatarUrl", alias = "img1v1Url")]
    pub avatar_url: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct VideoStatsEnvelope {
    pub liked: Option<bool>,
    #[serde(rename = "likedCount")]
    pub liked_count: Option<u64>,
    #[serde(rename = "commentCount")]
    pub comment_count: Option<u64>,
    #[serde(rename = "shareCount")]
    pub share_count: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct MvUrlEnvelope {
    pub data: VideoUrlItem,
}

#[derive(Debug, Deserialize)]
pub(crate) struct CloudVideoUrlEnvelope {
    #[serde(default)]
    pub urls: Vec<VideoUrlItem>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct VideoUrlItem {
    pub id: Option<Value>,
    pub url: Option<String>,
    pub size: Option<u64>,
    pub validity: Option<u64>,
    pub expi: Option<u64>,
    pub r: Option<u32>,
    pub resolution: Option<u32>,
    pub code: Option<i64>,
    pub fee: Option<i64>,
    #[serde(rename = "mvFee")]
    pub mv_fee: Option<i64>,
    pub msg: Option<String>,
    pub md5: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ArtistVideoUserProfile {
    #[serde(rename = "userId")]
    pub user_id: Option<Value>,
    #[serde(default)]
    pub nickname: String,
    #[serde(rename = "avatarUrl")]
    pub avatar_url: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ArtistNewVideosEnvelope {
    pub data: ArtistNewVideosData,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ArtistNewVideosData {
    #[serde(rename = "hasMore")]
    pub has_more: Option<bool>,
    #[serde(rename = "newWorks", default)]
    pub new_works: Vec<Value>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ArtistNewVideoItem {
    pub id: Option<Value>,
    pub name: Option<String>,
    #[serde(alias = "imgurl")]
    pub cover: Option<String>,
    #[serde(rename = "playCount")]
    pub play_count: Option<u64>,
    #[serde(rename = "briefDesc")]
    pub brief_description: Option<String>,
    pub desc: Option<String>,
    #[serde(rename = "artistName")]
    pub artist_name: Option<String>,
    #[serde(rename = "artistImgUrl")]
    pub artist_image_url: Option<String>,
    #[serde(rename = "artistId")]
    pub artist_id: Option<Value>,
    #[serde(rename = "mvId")]
    pub mv_id: Option<Value>,
    #[serde(rename = "mvName")]
    pub mv_name: Option<String>,
    #[serde(rename = "mvCoverUrl", alias = "imgurl16v9")]
    pub mv_cover_url: Option<String>,
    pub duration: Option<u64>,
    #[serde(rename = "publishTime")]
    pub published_at: Option<Value>,
    #[serde(rename = "publishDate")]
    pub published_date: Option<String>,
    #[serde(default)]
    pub artists: Vec<ArtistVideoCreator>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ArtistNewTracksEnvelope {
    pub data: ArtistNewTracksData,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ArtistNewTracksData {
    #[serde(rename = "hasMore")]
    pub has_more: Option<bool>,
    #[serde(rename = "newSongCount")]
    pub new_song_count: Option<u64>,
    #[serde(rename = "newWorks", default)]
    pub new_works: Vec<Value>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ArtistNewWorksEnvelope {
    pub data: ArtistNewWorksData,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ArtistNewWorksData {
    #[serde(rename = "hasMore")]
    pub has_more: Option<bool>,
    #[serde(rename = "latestVisitTime")]
    pub latest_visit_time: Option<u64>,
    #[serde(rename = "newWorks", default)]
    pub new_works: Vec<Value>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ArtistNewTracksPlayAllEnvelope {
    pub data: ArtistNewTracksPlayAllData,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ArtistNewTracksPlayAllData {
    pub count: Option<u64>,
    #[serde(rename = "songList", default)]
    pub songs: Vec<Value>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ArtistDetailEnvelope {
    pub data: ArtistDetailData,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ArtistDetailData {
    pub artist: ArtistDetail,
    #[serde(rename = "videoCount")]
    pub video_count: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ArtistDetail {
    pub id: u64,
    pub name: String,
    #[serde(default)]
    pub alias: Vec<String>,
    #[serde(rename = "transNames", default)]
    pub translated_names: Vec<String>,
    #[serde(rename = "briefDesc")]
    pub brief_description: Option<String>,
    pub avatar: Option<String>,
    pub cover: Option<String>,
    #[serde(rename = "albumSize")]
    pub album_count: Option<u64>,
    #[serde(rename = "musicSize")]
    pub track_count: Option<u64>,
    #[serde(rename = "mvSize")]
    pub mv_count: Option<u64>,
    #[serde(default)]
    pub identities: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ArtistDescriptionEnvelope {
    #[serde(rename = "briefDesc")]
    pub brief_description: Option<String>,
    #[serde(default)]
    pub introduction: Vec<ArtistIntroduction>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ArtistIntroduction {
    #[serde(rename = "ti")]
    pub title: String,
    #[serde(rename = "txt")]
    pub text: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ArtistDynamicEnvelope {
    pub followed: Option<bool>,
    pub concert: Option<ArtistConcert>,
    #[serde(rename = "videoNum", default)]
    pub video_counts: Vec<ArtistVideoCount>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ArtistConcert {
    #[serde(rename = "onlineCount")]
    pub online_count: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ArtistVideoCount {
    pub cat: i64,
    pub num: u64,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ArtistFollowCountEnvelope {
    pub data: ArtistFollowCount,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ArtistFollowCount {
    #[serde(rename = "fansCnt")]
    pub follower_count: Option<u64>,
    pub follow: Option<bool>,
    #[serde(rename = "isFollow")]
    pub is_following: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ArtistFansEnvelope {
    #[serde(default)]
    pub data: Vec<Value>,
    #[serde(rename = "hasMore", alias = "more")]
    pub has_more: Option<bool>,
    #[serde(alias = "count")]
    pub total: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ArtistFanProfile {
    #[serde(rename = "userId")]
    pub user_id: u64,
    pub nickname: String,
    #[serde(rename = "avatarUrl")]
    pub avatar_url: Option<String>,
    pub signature: Option<String>,
    pub followed: Option<bool>,
    pub mutual: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SubscribedAlbumsEnvelope {
    #[serde(default)]
    pub data: Vec<Value>,
    #[serde(default, alias = "total")]
    pub count: Option<u64>,
    #[serde(rename = "hasMore")]
    pub has_more: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SubscribedVideosEnvelope {
    #[serde(default)]
    pub data: Vec<Value>,
    #[serde(default, alias = "total")]
    pub count: Option<u64>,
    #[serde(rename = "hasMore", alias = "more")]
    pub has_more: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct AlbumEntitlementsEnvelope {
    #[serde(default)]
    pub data: Vec<Value>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct TrackEntitlementData {
    pub id: u64,
    pub st: Option<i64>,
    pub fee: Option<i64>,
    pub pl: Option<u64>,
    pub dl: Option<u64>,
    pub maxbr: Option<u64>,
    #[serde(rename = "playMaxbr")]
    pub play_max_bitrate: Option<u64>,
    #[serde(rename = "downloadMaxbr")]
    pub download_max_bitrate: Option<u64>,
    #[serde(rename = "plLevel")]
    pub play_level: Option<String>,
    #[serde(rename = "dlLevel")]
    pub download_level: Option<String>,
    pub payed: Option<i64>,
    #[serde(rename = "chargeInfoList", default)]
    pub charge_info: Vec<ChargeInfo>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ChargeInfo {
    pub rate: Option<u64>,
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
pub(crate) struct DigitalAlbumChartEnvelope {
    #[serde(default)]
    pub products: Vec<Value>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct DigitalAlbumChartItem {
    #[serde(rename = "albumType")]
    pub album_type: Option<u8>,
    pub rank: Option<u32>,
    #[serde(rename = "rankIncr")]
    pub rank_change: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ChartCatalogEnvelope {
    #[serde(default)]
    pub list: Vec<Value>,
    #[serde(default)]
    pub data: Vec<Value>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ChartGroupItem {
    #[serde(rename = "categoryCode")]
    pub category_code: Option<String>,
    #[serde(default)]
    pub name: String,
    #[serde(rename = "displayType")]
    pub display_type: Option<String>,
    #[serde(rename = "frontDisplayType")]
    pub front_display_type: Option<String>,
    #[serde(rename = "targetUrl")]
    pub target_url: Option<String>,
    #[serde(default)]
    pub list: Vec<Value>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ChartItem {
    pub id: Option<Value>,
    #[serde(default)]
    pub name: String,
    pub description: Option<String>,
    #[serde(rename = "coverImgUrl")]
    pub cover_img_url: Option<String>,
    #[serde(rename = "coverUrl")]
    pub cover_url: Option<String>,
    #[serde(rename = "firstCoverUrl")]
    pub first_cover_url: Option<String>,
    #[serde(rename = "newFirstCoverUrl")]
    pub new_first_cover_url: Option<String>,
    #[serde(rename = "updateFrequency")]
    pub update_frequency: Option<String>,
    #[serde(rename = "updateTime")]
    pub update_time: Option<u64>,
    #[serde(rename = "trackCount")]
    pub track_count: Option<u64>,
    #[serde(rename = "playCount")]
    pub play_count: Option<u64>,
    pub subscribed: Option<bool>,
    #[serde(rename = "canPlay")]
    pub can_play: Option<bool>,
    #[serde(rename = "targetType")]
    pub target_type: Option<String>,
    #[serde(rename = "targetUrl")]
    pub target_url: Option<String>,
    #[serde(rename = "frontTargetUrl")]
    pub front_target_url: Option<String>,
    pub tracks: Option<Vec<Value>>,
    #[serde(rename = "trackRankList")]
    pub track_rank_list: Option<Vec<Value>>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ChartTextPreviewItem {
    pub first: Option<String>,
    pub second: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ChartRankPreviewItem {
    #[serde(rename = "trackId")]
    pub track_id: Option<Value>,
    #[serde(rename = "songName")]
    pub song_name: Option<String>,
    #[serde(rename = "itemName")]
    pub item_name: Option<String>,
    #[serde(rename = "artistName")]
    pub artist_name: Option<String>,
    #[serde(rename = "coverImgUrl")]
    pub cover_url: Option<String>,
    pub rank: Option<u32>,
    #[serde(rename = "lastRank")]
    pub last_rank: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ArtistChartEnvelope {
    pub list: ArtistChartList,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ArtistChartList {
    #[serde(default)]
    pub artists: Vec<Value>,
    #[serde(rename = "updateTime")]
    pub update_time: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct DimensionChartDetailEnvelope {
    pub data: DimensionChartDetailData,
}

#[derive(Debug, Deserialize)]
pub(crate) struct DimensionChartDetailData {
    #[serde(rename = "chartCode")]
    pub chart_code: Option<String>,
    #[serde(rename = "chartId")]
    pub chart_id: Option<String>,
    pub name: Option<String>,
    pub description: Option<String>,
    #[serde(rename = "coverUrl")]
    pub cover_url: Option<String>,
    #[serde(rename = "updateTime")]
    pub update_time: Option<u64>,
    #[serde(rename = "playCount")]
    pub play_count: Option<u64>,
    #[serde(rename = "shareCount")]
    pub share_count: Option<u64>,
    #[serde(rename = "commentCount")]
    pub comment_count: Option<u64>,
    #[serde(rename = "supportComment")]
    pub support_comment: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct DimensionChartTracksEnvelope {
    pub data: DimensionChartTracksData,
}

#[derive(Debug, Deserialize)]
pub(crate) struct DimensionChartTracksData {
    #[serde(rename = "chartCode")]
    pub chart_code: Option<String>,
    #[serde(rename = "chartId")]
    pub chart_id: Option<String>,
    #[serde(default)]
    pub charts: Vec<Value>,
    #[serde(rename = "groupNameMap", default)]
    pub group_name_map: BTreeMap<String, Value>,
    #[serde(rename = "periodUpdateTimeText")]
    pub period_update_time_text: Option<Value>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct DimensionChartTrackItem {
    pub collect: Option<bool>,
    #[serde(rename = "lastRank")]
    pub last_rank: Option<i64>,
    pub ratio: Option<Value>,
    pub reason: Option<String>,
    #[serde(rename = "reasonId")]
    pub reason_id: Option<Value>,
    pub score: Option<Value>,
    #[serde(rename = "songData")]
    pub song_data: Song,
    pub privilege: Option<Privilege>,
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
pub(crate) struct PersonalFmEnvelope {
    #[serde(default)]
    pub data: Vec<Song>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct PersonalizedEnvelope {
    pub result: Vec<Value>,
    pub category: Option<Value>,
    #[serde(rename = "hasTaste")]
    pub has_taste: Option<Value>,
    pub more: Option<Value>,
    pub offset: Option<Value>,
    pub name: Option<Value>,
    pub trp: Option<Value>,
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
    pub play_count: Option<Value>,
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

#[derive(Clone, Debug, Deserialize)]
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
    pub message: Option<String>,
    #[serde(rename = "freeTrialInfo")]
    pub free_trial_info: Option<FreeTrialInfo>,
}

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct FreeTrialInfo {
    pub start: Option<u64>,
    pub end: Option<u64>,
}

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct Song {
    pub id: u64,
    #[serde(default, deserialize_with = "deserialize_nullable_string")]
    pub name: String,
    #[serde(
        default,
        alias = "alias",
        deserialize_with = "deserialize_nullable_vec"
    )]
    pub alia: Vec<String>,
    #[serde(
        default,
        alias = "artists",
        deserialize_with = "deserialize_nullable_vec"
    )]
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
    #[serde(default, deserialize_with = "deserialize_nullable_string")]
    pub name: String,
}

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct Album {
    pub id: u64,
    #[serde(default, deserialize_with = "deserialize_nullable_string")]
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
