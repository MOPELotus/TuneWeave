//! The official Web curated playlist directory. Login and personalized feeds are separate.
use super::*;
use catalog::Unsigned;
use std::collections::BTreeSet;
use tuneweave_core::{Page, PageMeta, PlaylistCatalogKind, PlaylistCatalogRequest};

mod taxonomy;

const PAGE_SIZE: u32 = 20;
const BUDGET: Duration = Duration::from_secs(45);

impl KuwoClient {
    pub async fn playlist_catalog(
        &self,
        request: &PlaylistCatalogRequest,
    ) -> Result<Page<Playlist>> {
        if request.account.is_some()
            || !(1..=100).contains(&request.limit)
            || request.offset.checked_add(request.limit).is_none()
        {
            return Err(kuwo_invalid_request(
                "Kuwo playlist catalogues require an anonymous request and limit 1..100",
            ));
        }
        match (request.catalog, request.tag_id.as_deref()) {
            (PlaylistCatalogKind::Latest | PlaylistCatalogKind::Hot, None) => {}
            (PlaylistCatalogKind::Tag, Some(id)) if canonical_positive_decimal(id) == Some(id) => {}
            _ => {
                return Err(kuwo_invalid_request(
                    "Kuwo tag catalogues require one canonical tag_id; latest/hot do not accept tag_id",
                ));
            }
        }
        tokio::time::timeout(BUDGET, self.playlist_catalog_window(request))
            .await
            .map_err(|_| {
                TuneWeaveError::new(
                    ErrorCode::UpstreamTimeout,
                    "Kuwo playlist catalogue timed out",
                )
                .with_platform(Platform::Kuwo)
            })?
    }

    async fn playlist_catalog_window(
        &self,
        request: &PlaylistCatalogRequest,
    ) -> Result<Page<Playlist>> {
        let order = match request.catalog {
            PlaylistCatalogKind::Latest => Some("new"),
            PlaylistCatalogKind::Hot => Some("hot"),
            PlaylistCatalogKind::Tag => {
                let id = request.tag_id.as_deref().ok_or_else(invalid)?;
                let taxonomy = self.fetch_playlist_catalog_taxonomy().await?;
                if !taxonomy
                    .groups
                    .iter()
                    .flat_map(|group| &group.tags)
                    .any(|tag| tag.id == id)
                {
                    return Err(kuwo_invalid_request(
                        "Kuwo playlist tag is not present in the current official taxonomy",
                    ));
                }
                None
            }
        };
        let first = request.offset / PAGE_SIZE + 1;
        let skip = request.offset % PAGE_SIZE;
        let page_count = (skip + request.limit).div_ceil(PAGE_SIZE);
        let mut items = Vec::with_capacity(request.limit as usize);
        let mut seen = BTreeSet::new();
        let mut total = None;
        let mut fetched = 0;
        // At most six physical pages; validate all rows, including those outside the window.
        for page in first..first + page_count {
            let result = self
                .playlist_catalog_page(order, request.tag_id.as_deref(), page)
                .await?;
            if total.is_some_and(|value| value != result.total) {
                return Err(kuwo_upstream_error(
                    "Kuwo playlist catalogue total changed between pages",
                ));
            }
            total = Some(result.total);
            fetched += 1;
            for item in &result.items {
                if !seen.insert(item.id.clone()) {
                    return Err(kuwo_upstream_error(
                        "Kuwo playlist catalogue repeated a playlist identity",
                    ));
                }
            }
            items.extend(
                result
                    .items
                    .into_iter()
                    .skip(if page == first { skip as usize } else { 0 })
                    .take(request.limit as usize - items.len()),
            );
            if u64::from(page) * u64::from(PAGE_SIZE) >= result.total {
                break;
            }
        }
        let end = request.offset + items.len() as u32;
        let has_more = total.is_some_and(|value| u64::from(end) < value);
        let mut extensions = Extensions::from([
            ("backend".into(), json!("current_web_playlist_catalogue")),
            ("catalog".into(), json!(request.catalog)),
            (
                "catalog_scope".into(),
                json!("official_web_curated_playlists"),
            ),
            ("upstream_page_size".into(), json!(PAGE_SIZE)),
            ("upstream_pages_fetched".into(), json!(fetched)),
            (
                "consistency_scope".into(),
                json!("matching_totals_and_distinct_ids"),
            ),
        ]);
        if let Some(order) = order {
            extensions.insert("upstream_order".into(), json!(order));
        } else {
            extensions.insert("tag_id".into(), json!(request.tag_id));
            extensions.insert(
                "taxonomy_validation".into(),
                json!("fresh_visible_tag_membership"),
            );
        }
        Ok(Page {
            items,
            pagination: PageMeta {
                limit: request.limit,
                offset: request.offset,
                total,
                has_more,
                next_offset: has_more.then_some(end),
                extensions,
            },
        })
    }

    async fn playlist_catalog_page(
        &self,
        order: Option<&'static str>,
        id: Option<&str>,
        page: u32,
    ) -> Result<CatalogPage> {
        for refresh in [false, true] {
            let response = self
                .signed_get(
                    if id.is_some() {
                        KuwoSignedEndpoint::PlaylistCatalogTag
                    } else {
                        KuwoSignedEndpoint::PlaylistCatalog
                    },
                    &Query {
                        login_uid: 0,
                        login_sid: 0,
                        pn: page,
                        rn: PAGE_SIZE,
                        order,
                        id,
                        https_status: 1,
                        request_id: new_request_id(),
                        plat: "web_www",
                        from: "",
                    },
                    "https://www.kuwo.cn/playlists",
                    refresh,
                    u8::from(refresh),
                )
                .await?;
            match response {
                KuwoSignedResponse::SessionRejected if !refresh => continue,
                KuwoSignedResponse::SessionRejected => return Err(invalid()),
                KuwoSignedResponse::Body(bytes) => {
                    if !refresh && is_signed_session_rejection(&bytes) {
                        continue;
                    }
                    return parse(&bytes, page);
                }
            }
        }
        Err(invalid())
    }
}

#[derive(Serialize)]
struct Query<'a> {
    #[serde(rename = "loginUid")]
    login_uid: u8,
    #[serde(rename = "loginSid")]
    login_sid: u8,
    pn: u32,
    rn: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    order: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<&'a str>,
    #[serde(rename = "httpsStatus")]
    https_status: u8,
    #[serde(rename = "reqId")]
    request_id: String,
    plat: &'static str,
    from: &'static str,
}

struct CatalogPage {
    total: u64,
    items: Vec<Playlist>,
}
#[derive(Deserialize)]
struct Envelope {
    code: i64,
    data: Option<Data>,
}
#[derive(Deserialize)]
struct Data {
    total: Unsigned,
    pn: Unsigned,
    rn: Unsigned,
    data: Vec<Row>,
}
#[derive(Deserialize)]
struct Row {
    id: Unsigned,
    name: String,
    digest: Unsigned,
    radio_id: String,
    uid: Option<Unsigned>,
    uname: Option<String>,
    img: Option<String>,
    total: Option<Unsigned>,
    listencnt: Option<Unsigned>,
    favorcnt: Option<Unsigned>,
    desc: Option<String>,
    info: Option<String>,
}

fn invalid() -> TuneWeaveError {
    kuwo_upstream_error("Kuwo returned an invalid playlist catalogue")
}

fn parse(bytes: &[u8], page: u32) -> Result<CatalogPage> {
    let body: Envelope = serde_json::from_slice(bytes).map_err(|_| invalid())?;
    if body.code != 200 {
        return Err(invalid().with_details(json!({"upstream_code": body.code})));
    }
    let data = body.data.ok_or_else(invalid)?;
    let total = data.total.value()?;
    let start = u64::from(page - 1) * u64::from(PAGE_SIZE);
    if data.pn.value()? != u64::from(page)
        || data.rn.value()? != u64::from(PAGE_SIZE)
        || data.data.len() as u64 != total.saturating_sub(start).min(u64::from(PAGE_SIZE))
    {
        return Err(invalid());
    }
    let items = data.data.into_iter().map(map).collect::<Result<Vec<_>>>()?;
    Ok(CatalogPage { total, items })
}

fn map(row: Row) -> Result<Playlist> {
    if row.digest.value()? != 8 || !row.radio_id.is_empty() {
        return Err(invalid());
    }
    let id = row.id.id()?;
    let name = catalog::text(&row.name, 512, false)?;
    if name.is_empty() {
        return Err(invalid());
    }
    let mut extensions =
        Extensions::from([("backend".into(), json!("current_web_playlist_catalogue"))]);
    if let Some(uid) = row.uid {
        let uid = uid.value()?;
        if uid > 0 {
            extensions.insert("creator_uid".into(), json!(uid.to_string()));
        }
    }
    for (key, value) in [
        ("listen_count", row.listencnt),
        ("favorite_count", row.favorcnt),
    ] {
        if let Some(value) = value {
            extensions.insert(key.into(), json!(value.value()?));
        }
    }
    // This is a user name. The generic creator summary has no artist reference.
    let creator_name = catalog::text(row.uname.as_deref().unwrap_or(""), 512, false)?;
    let desc = catalog::text(row.desc.as_deref().unwrap_or(""), 8000, true)?;
    let info = catalog::text(row.info.as_deref().unwrap_or(""), 8000, true)?;
    Ok(Playlist {
        resource_ref: ResourceRef::new(Platform::Kuwo, id.clone()).map_err(|_| invalid())?,
        platform: Platform::Kuwo,
        id,
        name,
        description: if desc.is_empty() { info } else { desc },
        cover_url: row.img.as_deref().and_then(normalize_playlist_image_url),
        creator: (!creator_name.is_empty()).then_some(ArtistSummary {
            resource_ref: None,
            name: creator_name,
        }),
        track_count: row.total.map(|value| value.value()).transpose()?,
        tags: vec![],
        subscribed: None,
        created_at: None,
        updated_at: None,
        extensions,
    })
}

#[cfg(test)]
mod tag_tests;
#[cfg(test)]
mod tests;
