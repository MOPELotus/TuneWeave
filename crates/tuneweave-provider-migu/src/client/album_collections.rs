use super::albums::AlbumKind;
use super::*;
use tuneweave_core::{Album, DigitalAlbum};

pub(crate) const PAGE_SIZE: usize = 10;
pub(crate) const MAX_PAGES: u32 = 64;

pub(crate) enum CollectedAlbum {
    Ordinary(Album),
    Digital(DigitalAlbum),
}
impl CollectedAlbum {
    pub(crate) fn identity(&self) -> (&'static str, &str) {
        match self {
            Self::Ordinary(album) => ("2003", &album.id),
            Self::Digital(album) => ("5", &album.id),
        }
    }
}
pub(crate) struct CollectionPage {
    pub items: Vec<CollectedAlbum>,
    pub total: Option<u64>,
    pub has_next: Option<bool>,
}
fn invalid() -> TuneWeaveError {
    migu_upstream_error("Migu album collection response is invalid")
}
fn canonical(value: &str) -> Result<&str> {
    if value.is_empty()
        || value.len() > 64
        || value.starts_with('0')
        || !value.bytes().all(|b| b.is_ascii_digit())
    {
        return Err(invalid());
    }
    Ok(value)
}
fn text(value: &str) -> Result<String> {
    if value.trim().is_empty() || value.len() > 2048 || value.chars().any(char::is_control) {
        return Err(invalid());
    }
    Ok(value.trim().to_owned())
}
pub(crate) fn title(kind: AlbumKind, data: serde_json::Value, id: &str) -> Result<String> {
    let key = match kind {
        AlbumKind::Ordinary => "albumId",
        AlbumKind::Digital => "contentId",
    };
    if data.get("resourceType").and_then(serde_json::Value::as_str) != Some(kind.resource_type())
        || data.get(key).and_then(serde_json::Value::as_str) != Some(id)
    {
        return Err(invalid());
    }
    text(
        data.get("title")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(invalid)?,
    )
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Entry {
    resource_type: String,
    album_id: Option<String>,
    content_id: Option<String>,
    title: String,
    singer: Option<String>,
    singer_id: Option<String>,
    img_items: Option<Vec<Image>>,
}
#[derive(Deserialize)]
struct Image {
    img: Option<String>,
}
#[derive(Deserialize)]
struct Fields {
    collections: Vec<Entry>,
    #[serde(rename = "totalCount")]
    total: Option<Count>,
    #[serde(rename = "hasNext")]
    has_next: Option<bool>,
}
#[derive(Deserialize)]
#[serde(untagged)]
enum Count {
    Number(u64),
    Text(String),
}
fn artists(names: Option<&str>, ids: Option<&str>) -> Result<Vec<ArtistSummary>> {
    let names = names.filter(|v| !v.is_empty());
    let ids = ids.filter(|v| !v.is_empty());
    let Some(names) = names else {
        return if ids.is_some() {
            Err(invalid())
        } else {
            Ok(Vec::new())
        };
    };
    if names.len() > 8192 || ids.is_some_and(|v| v.len() > 4096) {
        return Err(invalid());
    }
    let names = names.split('|').map(text).collect::<Result<Vec<_>>>()?;
    if names.len() > 32 {
        return Err(invalid());
    }
    let ids = ids
        .map(|ids| {
            ids.split('|')
                .map(|id| canonical(id).map(str::to_owned))
                .collect::<Result<Vec<_>>>()
        })
        .transpose()?;
    if ids.as_ref().is_some_and(|ids| ids.len() != names.len()) {
        return Err(invalid());
    }
    names
        .into_iter()
        .enumerate()
        .map(|(i, name)| {
            Ok(ArtistSummary {
                resource_ref: ids
                    .as_ref()
                    .map(|ids| ResourceRef::new(Platform::Migu, &ids[i]).map_err(|_| invalid()))
                    .transpose()?,
                name,
            })
        })
        .collect()
}
pub(crate) fn parse_page(
    fields: serde_json::Map<String, serde_json::Value>,
    uid: &str,
) -> Result<CollectionPage> {
    let raw: Fields =
        serde_json::from_value(serde_json::Value::Object(fields)).map_err(|_| invalid())?;
    if raw.collections.len() > PAGE_SIZE {
        return Err(invalid());
    }
    let total = raw
        .total
        .map(|value| match value {
            Count::Number(n) => Ok(n),
            Count::Text(s)
                if !s.is_empty() && s.len() <= 20 && s.bytes().all(|b| b.is_ascii_digit()) =>
            {
                s.parse().map_err(|_| invalid())
            }
            _ => Err(invalid()),
        })
        .transpose()?;
    if total.is_some_and(|total| total > u64::from(MAX_PAGES) * PAGE_SIZE as u64) {
        return Err(invalid());
    }
    let mut items = Vec::new();
    for entry in raw.collections {
        let id = match entry.resource_type.as_str() {
            "2003" => canonical(entry.album_id.as_deref().ok_or_else(invalid)?)?,
            "5" => canonical(entry.content_id.as_deref().ok_or_else(invalid)?)?,
            _ => return Err(invalid()),
        };
        let name = text(&entry.title)?;
        let artists = artists(entry.singer.as_deref(), entry.singer_id.as_deref())?;
        let images = entry.img_items.unwrap_or_default();
        if images.len() > 64 {
            return Err(invalid());
        }
        let mut cover_url = None;
        for image in images {
            if let Some(value) = image.img.filter(|v| !v.is_empty()) {
                let normalized = super::albums::album_image_url(&value).ok_or_else(invalid)?;
                if cover_url.is_none() {
                    cover_url = Some(normalized);
                }
            }
        }
        let resource_ref = ResourceRef::new(Platform::Migu, id).map_err(|_| invalid())?;
        let extensions = Extensions::from([
            ("backend".into(), json!("official_pc_album_collections")),
            ("resource_type".into(), json!(entry.resource_type)),
            ("source_user_id".into(), json!(uid)),
            ("subscribed".into(), json!(true)),
        ]);
        items.push(if entry.resource_type == "2003" {
            CollectedAlbum::Ordinary(Album {
                resource_ref,
                platform: Platform::Migu,
                id: id.into(),
                name,
                aliases: vec![],
                artists,
                description: String::new(),
                cover_url,
                published_at: None,
                track_count: None,
                company: None,
                kind: None,
                extensions,
            })
        } else {
            CollectedAlbum::Digital(DigitalAlbum {
                resource_ref,
                platform: Platform::Migu,
                id: id.into(),
                name,
                artists,
                description: String::new(),
                cover_url,
                published_at: None,
                price: None,
                is_free: None,
                purchasable: None,
                purchased: None,
                sale_count: None,
                track_count: None,
                tags: vec![],
                extensions,
            })
        });
    }
    Ok(CollectionPage {
        items,
        total,
        has_next: raw.has_next,
    })
}

#[cfg(test)]
pub(crate) mod tests;
