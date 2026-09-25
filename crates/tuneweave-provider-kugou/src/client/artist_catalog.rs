//! The official grouped directory, with its hot section kept separate from initials.
use super::dto::{Number, optional_text, required_text, resource};
use super::openapi::{Endpoint, check_ocean_status};
use super::*;
use tuneweave_core::{
    Artist, ArtistArea, ArtistCatalog, ArtistCatalogFilterOption, ArtistCatalogFilters,
    ArtistCatalogRequest, ArtistCategory, ArtistGenre,
};

const HOT_SIZE: usize = 200;
const MAX_ARTISTS: usize = 10_000;

#[derive(Deserialize)]
struct Envelope {
    data: Directory,
}
#[derive(Deserialize)]
struct Directory {
    info: Vec<Group>,
    enu_list: Filters,
    timestamp: Option<Number>,
}
#[derive(Deserialize)]
struct Group {
    title: String,
    singer: Vec<Singer>,
}
#[derive(Deserialize)]
struct Singer {
    singerid: Number,
    singername: String,
    intro: Option<String>,
    imgurl: Option<String>,
    songcount: Option<Number>,
    albumcount: Option<Number>,
    mvcount: Option<Number>,
    fanscount: Option<Number>,
}
#[derive(Deserialize)]
struct Filters {
    types: Vec<Filter>,
    sextypes: Vec<Filter>,
}
#[derive(Deserialize)]
struct Filter {
    key: Number,
    value: String,
    musician: Option<Number>,
}

fn selection(request: &ArtistCatalogRequest) -> Result<(u64, u64)> {
    if request.account.is_some()
        || request.genre != ArtistGenre::All
        || request.area == ArtistArea::HongKongTaiwan
        || request.area == ArtistArea::JapaneseKorean
    {
        return Err(TuneWeaveError::invalid_request(
            "KuGou artist directory requires an anonymous source and supported area/category filters",
        )
        .with_platform(Platform::Kugou));
    }
    let area = match request.area {
        ArtistArea::All => 0,
        ArtistArea::Chinese => 1,
        ArtistArea::Western => 2,
        ArtistArea::Other => 4,
        ArtistArea::Japanese => 5,
        ArtistArea::Korean => 6,
        ArtistArea::HongKongTaiwan => unreachable!(),
        ArtistArea::JapaneseKorean => unreachable!(),
    };
    let category = match request.category {
        ArtistCategory::All => 0,
        ArtistCategory::Male => 1,
        ArtistCategory::Female => 2,
        ArtistCategory::Group => 3,
    };
    Ok((area, category))
}

impl KugouClient {
    pub(crate) async fn public_artist_catalog(
        &self,
        request: &ArtistCatalogRequest,
    ) -> Result<ArtistCatalog> {
        let (area, category) = selection(request)?;
        let query = BTreeMap::from([
            ("type", area.to_string()),
            ("sextype", category.to_string()),
            ("musician", "0".into()),
            ("showtype", "2".into()),
            ("with_discuss", "0".into()),
            ("is_thumb", "1".into()),
            ("hotsize", HOT_SIZE.to_string()),
        ]);
        let bytes = self
            .public_catalogue_get(Endpoint::ArtistCatalog, query, &self.device_identity()?)
            .await?;
        parse_catalogue(&bytes, request)
    }
}

fn invalid() -> TuneWeaveError {
    kugou_upstream_error("KuGou artist directory returned invalid or inconsistent metadata")
}

fn filters(rows: Vec<Filter>, selected: u64, area: bool) -> Result<Vec<ArtistCatalogFilterOption>> {
    if rows.len() > 64 {
        return Err(invalid());
    }
    let mut seen = BTreeSet::new();
    let mut options = vec![];
    let mut found = false;
    for row in rows {
        let musician = row.musician.map_or(0, |n| n.0);
        if !seen.insert((row.key.0, musician)) {
            return Err(invalid());
        }
        let name = required_text(row.value)?;
        if musician != 0 {
            continue;
        }
        let id = if area {
            match row.key.0 {
                0 => "all",
                1 => "chinese",
                2 => "western",
                4 => "other",
                5 => "japanese",
                6 => "korean",
                _ => continue,
            }
        } else {
            match row.key.0 {
                0 => "all",
                1 => "male",
                2 => "female",
                3 => "group",
                _ => continue,
            }
        };
        found |= row.key.0 == selected;
        options.push(ArtistCatalogFilterOption {
            id: id.into(),
            name,
            extensions: Extensions::from([("upstream_key".into(), json!(row.key.0))]),
        });
    }
    if !found {
        return Err(invalid());
    }
    Ok(options)
}

fn parse_catalogue(bytes: &[u8], request: &ArtistCatalogRequest) -> Result<ArtistCatalog> {
    let (area, category) = selection(request)?;
    check_ocean_status(bytes)?;
    let Envelope { data } = serde_json::from_slice(bytes).map_err(|_| invalid())?;
    if data.info.len() > 28 {
        return Err(invalid());
    }
    let mut catalogue = ArtistCatalog {
        platform: Platform::Kugou,
        area: request.area,
        category: request.category,
        genre: request.genre,
        featured_artists: vec![],
        artists: vec![],
        filters: ArtistCatalogFilters {
            areas: filters(data.enu_list.types, area, true)?,
            categories: filters(data.enu_list.sextypes, category, false)?,
            genres: vec![],
            initials: vec![],
            extensions: Extensions::new(),
        },
        extensions: Extensions::from([
            ("backend".into(), json!("official_grouped_artist_directory")),
            ("catalog_scope".into(), json!("upstream_grouped_directory")),
            ("upstream_hot_size".into(), json!(HOT_SIZE)),
        ]),
    };
    if let Some(timestamp) = data.timestamp {
        catalogue
            .extensions
            .insert("upstream_timestamp".into(), json!(timestamp.0));
    }
    let mut titles = BTreeSet::new();
    let mut hot_ids = BTreeSet::new();
    let mut artist_ids = BTreeSet::new();
    let mut names = BTreeMap::new();
    for group in data.info {
        let hot = group.title == "热门";
        let initial = group.title == "#"
            || (group.title.len() == 1 && group.title.as_bytes()[0].is_ascii_uppercase());
        if (!hot && !initial) || !titles.insert(group.title.clone()) {
            return Err(invalid());
        }
        if hot && group.singer.len() > HOT_SIZE {
            return Err(invalid());
        }
        if !hot {
            catalogue.filters.initials.push(ArtistCatalogFilterOption {
                id: group.title.clone(),
                name: group.title.clone(),
                extensions: Extensions::new(),
            });
        }
        for row in group.singer {
            let id = row.singerid.id()?;
            let name = required_text(row.singername)?;
            // A hot singer may also appear in its initial group; both views retain it.
            let seen = if hot { &mut hot_ids } else { &mut artist_ids };
            if !seen.insert(id.clone())
                || names
                    .insert(id.clone(), name.clone())
                    .is_some_and(|old| old != name)
                || hot_ids.len() + artist_ids.len() > MAX_ARTISTS
            {
                return Err(invalid());
            }
            let mut extensions = Extensions::from([("directory_group".into(), json!(group.title))]);
            if let Some(fans) = row.fanscount {
                extensions.insert("fans_count".into(), json!(fans.0));
            }
            let artist = Artist {
                resource_ref: resource(id.clone())?,
                platform: Platform::Kugou,
                id,
                name,
                aliases: vec![],
                description: optional_text(row.intro, 131_072)?.unwrap_or_default(),
                biography_sections: vec![],
                avatar_url: row.imgurl.as_deref().and_then(normalize_image_url),
                cover_url: None,
                album_count: row.albumcount.map(|n| n.0),
                track_count: row.songcount.map(|n| n.0),
                mv_count: row.mvcount.map(|n| n.0),
                video_count: None,
                identities: vec![],
                extensions,
            };
            if hot {
                catalogue.featured_artists.push(artist);
            } else {
                catalogue.artists.push(artist);
            }
        }
    }
    Ok(catalogue)
}

#[cfg(test)]
pub(crate) mod tests;
