use super::*;
use tuneweave_core::{Album, DigitalAlbum, PurchasedAlbum};

pub(crate) const PAGE_SIZE: usize = 50;
pub(crate) const MAX_PAGES: u32 = 128;
pub(crate) const BACKEND: &str = "pc_album_subscription_v1";

pub(crate) struct AlbumPage {
    pub items: Vec<PurchasedAlbum>,
    pub has_next: bool,
    pub bytes: usize,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Fields {
    resources: Vec<Entry>,
    has_next_page: bool,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Entry {
    content_id: String,
    resource_type: String,
    copyright_id: Option<String>,
    title: Option<String>,
    singer: Option<String>,
    total_count: Option<Count>,
    img_items: Option<Vec<Image>>,
}
#[derive(Deserialize)]
struct Image {
    img: Option<String>,
}
#[derive(Deserialize)]
#[serde(untagged)]
enum Count {
    Number(u64),
    Text(String),
}
fn invalid() -> TuneWeaveError {
    migu_upstream_error("Migu purchased album response is invalid")
}
fn text(value: Option<String>, maximum: usize) -> Result<Option<String>> {
    let Some(value) = value else {
        return Ok(None);
    };
    if value.len() > maximum || value.chars().any(char::is_control) {
        return Err(invalid());
    }
    Ok((!value.trim().is_empty()).then(|| value.trim().to_owned()))
}
fn identity(value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 64
        || value.starts_with('0')
        || !value.bytes().all(|b| b.is_ascii_digit())
    {
        return Err(invalid());
    }
    Ok(())
}
pub(crate) fn parse_page(value: serde_json::Value, uid: &str, bytes: usize) -> Result<AlbumPage> {
    let data: Fields = serde_json::from_value(value).map_err(|_| invalid())?;
    if data.resources.len() > PAGE_SIZE || (data.resources.is_empty() && data.has_next_page) {
        return Err(invalid());
    }
    let items = data
        .resources
        .into_iter()
        .map(|row| map_item(row, uid))
        .collect::<Result<_>>()?;
    Ok(AlbumPage {
        items,
        has_next: data.has_next_page,
        bytes,
    })
}
fn map_item(row: Entry, uid: &str) -> Result<PurchasedAlbum> {
    identity(&row.content_id)?;
    let digital = match row.resource_type.as_str() {
        "2003" => false,
        "5" => true,
        _ => return Err(invalid()),
    };
    let name = text(row.title, 2048)?;
    // The response supplies one display credit, without separately bound singer IDs.
    let artists = text(row.singer, 8192)?
        .into_iter()
        .map(|name| ArtistSummary {
            resource_ref: None,
            name,
        })
        .collect::<Vec<_>>();
    let count = row
        .total_count
        .map(|n| match n {
            Count::Number(n) => Ok(n),
            Count::Text(s)
                if !s.is_empty() && s.len() <= 20 && s.bytes().all(|b| b.is_ascii_digit()) =>
            {
                s.parse().map_err(|_| invalid())
            }
            _ => Err(invalid()),
        })
        .transpose()?;
    let images = row.img_items.unwrap_or_default();
    if images.len() > 64 {
        return Err(invalid());
    }
    let mut cover_url = None;
    for image in images {
        if let Some(value) = image.img.filter(|v| !v.is_empty()) {
            let url = super::albums::album_image_url(&value).ok_or_else(invalid)?;
            if cover_url.is_none() {
                cover_url = Some(url);
            }
        }
    }
    let reference = ResourceRef::new(Platform::Migu, &row.content_id).map_err(|_| invalid())?;
    let mut extensions = Extensions::from([
        ("backend".into(), json!(BACKEND)),
        ("source_user_id".into(), json!(uid)),
        ("resource_type".into(), json!(row.resource_type)),
        ("content_id".into(), json!(row.content_id)),
        ("resource_ref".into(), json!(reference)),
        ("purchase_kind".into(), json!("album_subscriptions")),
        (
            "catalogue_kind".into(),
            json!(if digital { "digital_album" } else { "album" }),
        ),
        ("catalogue_resolved".into(), json!(name.is_some())),
    ]);
    if let Some(value) = text(row.copyright_id, 128)? {
        if !value.bytes().all(|b| b.is_ascii_alphanumeric()) {
            return Err(invalid());
        }
        extensions.insert("copyright_id".into(), json!(value));
    }
    let album = name.as_ref().filter(|_| !digital).map(|name| Album {
        resource_ref: reference.clone(),
        platform: Platform::Migu,
        id: row.content_id.clone(),
        name: name.clone(),
        aliases: vec![],
        artists: artists.clone(),
        description: String::new(),
        cover_url: cover_url.clone(),
        published_at: None,
        track_count: count,
        company: None,
        kind: None,
        extensions: extensions.clone(),
    });
    let digital_album = name.as_ref().filter(|_| digital).map(|name| {
        Box::new(DigitalAlbum {
            resource_ref: reference,
            platform: Platform::Migu,
            id: row.content_id,
            name: name.clone(),
            artists: artists.clone(),
            description: String::new(),
            cover_url: cover_url.clone(),
            published_at: None,
            price: None,
            is_free: None,
            purchasable: None,
            purchased: None,
            sale_count: None,
            track_count: count,
            tags: vec![],
            extensions: extensions.clone(),
        })
    });
    Ok(PurchasedAlbum {
        album,
        digital_album,
        name,
        artists,
        cover_url,
        extensions,
    })
}

#[cfg(test)]
mod tests;
