use super::*;
use crate::credential::validate_uid;

pub(crate) const PAGE_SIZE: usize = 20;
pub(crate) const MAX_PAGES: u32 = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Section {
    Created,
    Saved,
}
impl Section {
    pub(crate) fn path(self) -> &'static str {
        match self {
            Self::Created => "/user/h5/my-music-list/v1.0",
            Self::Saved => "/user/h5/user/collection/v1.0",
        }
    }
    pub(crate) fn backend(self) -> &'static str {
        match self {
            Self::Created => "official_h5_created_playlists",
            Self::Saved => "official_h5_saved_playlists",
        }
    }
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Created => "created",
            Self::Saved => "saved",
        }
    }
    pub(crate) fn query(self, page: u32) -> Vec<(&'static str, String)> {
        let mut query = vec![
            ("pageNo", page.to_string()),
            ("pageSize", PAGE_SIZE.to_string()),
        ];
        match self {
            Self::Created => query.push(("queryType", "0".into())),
            Self::Saved => query.extend([
                ("OPType", "03".into()),
                ("resourceType", "2021".into()),
                ("type", "1".into()),
            ]),
        }
        query
    }
}

#[derive(Debug)]
pub(crate) struct LibraryPage {
    pub items: Vec<Playlist>,
    pub total: Option<u64>,
    pub has_next: Option<bool>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Entry {
    music_list_id: String,
    title: String,
    music_num: Option<Count>,
    resource_type: Option<String>,
    img_item: Option<Image>,
    owner_id: Option<String>,
    owner_name: Option<String>,
}
#[derive(Deserialize)]
struct Image {
    img: String,
}
#[derive(Deserialize)]
#[serde(untagged)]
enum Count {
    Number(u64),
    Text(String),
}
impl Count {
    fn value(self) -> Result<u64> {
        match self {
            Self::Number(n) => Ok(n),
            Self::Text(s)
                if !s.is_empty() && s.len() <= 20 && s.bytes().all(|c| c.is_ascii_digit()) =>
            {
                s.parse().map_err(|_| invalid())
            }
            _ => Err(invalid()),
        }
    }
}
fn invalid() -> TuneWeaveError {
    migu_upstream_error("Migu account playlist data is invalid")
}
fn name(s: String) -> Result<String> {
    if s.trim().is_empty() || s.len() > 2048 || s.chars().any(char::is_control) {
        return Err(invalid());
    }
    Ok(s)
}

pub(crate) fn parse_page(
    section: Section,
    mut body: serde_json::Map<String, serde_json::Value>,
    uid: &str,
) -> Result<LibraryPage> {
    let field = match section {
        Section::Created => "list",
        Section::Saved => "collections",
    };
    let value = body.remove(field).ok_or_else(invalid)?;
    let values = value.as_array().ok_or_else(invalid)?;
    if values.len() > PAGE_SIZE {
        return Err(invalid());
    }
    let total = body
        .remove("totalCount")
        .filter(|v| !v.is_null())
        .map(|v| {
            serde_json::from_value::<Count>(v)
                .map_err(|_| invalid())?
                .value()
        })
        .transpose()?;
    if total.is_some_and(|n| n > u64::from(MAX_PAGES) * PAGE_SIZE as u64) {
        return Err(invalid());
    }
    let has_next = body
        .remove("hasNext")
        .filter(|v| !v.is_null())
        .map(|v| v.as_bool().ok_or_else(invalid))
        .transpose()?;
    let mut items = Vec::with_capacity(values.len());
    for value in values {
        let e: Entry = serde_json::from_value(value.clone()).map_err(|_| invalid())?;
        let id = canonical_platform_id(&e.music_list_id).ok_or_else(invalid)?;
        if e.resource_type.as_deref().is_some_and(|v| v != "2021") {
            return Err(invalid());
        }
        let mut extensions = Extensions::from([
            ("backend".into(), json!(section.backend())),
            ("library_section".into(), json!(section.name())),
            ("source_user_id".into(), json!(uid)),
        ]);
        if let Some(owner) = e.owner_id.filter(|s| !s.is_empty()) {
            validate_uid(&owner).map_err(|_| invalid())?;
            if section == Section::Created && owner != uid {
                return Err(invalid());
            }
            extensions.insert("owner_id".into(), json!(owner));
        }
        let creator = e
            .owner_name
            .filter(|s| !s.is_empty())
            .map(|s| {
                Ok(ArtistSummary {
                    resource_ref: None,
                    name: name(s)?,
                })
            })
            .transpose()?;
        let cover_url = e
            .img_item
            .filter(|i| !i.img.is_empty())
            .map(|i| normalize_media_url(&i.img).ok_or_else(invalid))
            .transpose()?;
        items.push(Playlist {
            resource_ref: ResourceRef::new(Platform::Migu, id).map_err(|_| invalid())?,
            platform: Platform::Migu,
            id: id.to_owned(),
            name: name(e.title)?,
            description: String::new(),
            cover_url,
            creator,
            track_count: e.music_num.map(Count::value).transpose()?,
            tags: Vec::new(),
            subscribed: (section == Section::Saved).then_some(true),
            created_at: None,
            updated_at: None,
            extensions,
        });
    }
    Ok(LibraryPage {
        items,
        total,
        has_next,
    })
}

#[cfg(test)]
mod tests;
