use std::{collections::BTreeSet, time::SystemTime};

use async_trait::async_trait;
use serde_json::{Value, json};
use tuneweave_core::{
    Album, AlbumSummary, Artist, ArtistSummary, Capability, CreatorSummary, ErrorCode, Extensions,
    MusicProvider, Page, PageMeta, Platform, Playlist, Quality, ResourceRef, Result, SearchItem,
    SearchKind, SearchQuery, SearchVariant, Track, TuneWeaveError, Video,
};

use crate::client::{QqApiRequest, QqApiResponse, QqClient, QqConfig};

const SEARCH_MODULE: &str = "music.search.SearchCgiService";
const SEARCH_METHOD: &str = "DoSearchForQQMusicMobile";

#[derive(Clone, Copy)]
struct TypedSearchSpec {
    code: i64,
    item_pointer: &'static str,
    context: &'static str,
    upstream_page_size: u32,
    sparse: bool,
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

#[derive(Clone)]
pub struct QqProvider {
    client: QqClient,
}

impl QqProvider {
    pub fn new(config: QqConfig) -> Result<Self> {
        Ok(Self {
            client: QqClient::new(config)?,
        })
    }

    pub const fn from_client(client: QqClient) -> Self {
        Self { client }
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
        ])
    }

    async fn search(&self, query: &SearchQuery) -> Result<Page<Track>> {
        if query.kind != SearchKind::Track {
            return Err(TuneWeaveError::unsupported(
                Platform::Qq,
                capability_for_search(query.kind),
            ));
        }
        let (limit, skip, responses) = self.typed_search(query, TRACK_SEARCH).await?;
        map_track_search_response(query.offset, limit, skip, responses)
    }

    async fn search_catalog(&self, query: &SearchQuery) -> Result<Page<SearchItem>> {
        if query.kind == SearchKind::Track {
            let page = self.search(query).await?;
            return Ok(Page {
                items: page.items.into_iter().map(SearchItem::Track).collect(),
                pagination: page.pagination,
            });
        }
        let (spec, mapper): (TypedSearchSpec, fn(Value) -> Result<SearchItem>) = match query.kind {
            SearchKind::Artist => (ARTIST_SEARCH, map_artist_search_item),
            SearchKind::Album => (ALBUM_SEARCH, map_album_search_item),
            SearchKind::Playlist => (PLAYLIST_SEARCH, map_playlist_search_item),
            SearchKind::Mv => (MV_SEARCH, map_mv_search_item),
            SearchKind::Lyric => (LYRIC_SEARCH, map_lyric_search_item),
            kind => {
                return Err(TuneWeaveError::unsupported(
                    Platform::Qq,
                    capability_for_search(kind),
                ));
            }
        };
        let (limit, skip, responses) = self.typed_search(query, spec).await?;
        map_catalog_search_response(query.offset, limit, skip, responses, spec, mapper)
    }
}

impl QqProvider {
    async fn typed_search(
        &self,
        query: &SearchQuery,
        spec: TypedSearchSpec,
    ) -> Result<(u32, u32, Vec<QqApiResponse>)> {
        let keyword = validate_search_query(query)?;
        let limit = query.limit.clamp(1, 100);
        let search_id = generate_search_id()?;
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
                )
            })
            .collect::<Vec<_>>();
        let responses = self.client.request_android(&requests).await?;
        Ok((limit, skip, responses))
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

fn typed_search_request(
    keyword: &str,
    search_id: &str,
    search_type: i64,
    page: u32,
    page_size: u32,
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
            "highlight": false,
            "grp": true
        }),
    )
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
    mapper: fn(Value) -> Result<SearchItem>,
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
    async fn live_artist_album_playlist_mv_and_lyric_search_return_typed_catalogs() {
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
                    _ => false,
                }
            }));
        }
    }
}
