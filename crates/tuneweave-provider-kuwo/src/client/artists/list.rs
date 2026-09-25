use super::*;
use std::collections::BTreeSet;
use tuneweave_core::{ArtistArea, ArtistCategory, ArtistGenre, ArtistListRequest, Page, PageMeta};

const ENDPOINT: &str = "https://wapi.kuwo.cn/api/www/artist/artistInfo";
const PATH: &str = "/api/www/artist/artistInfo";
const LIST_PAGE_SIZE: u32 = 60;

struct Selection {
    category: u8,
    prefix: String,
}

impl Selection {
    fn new(request: &ArtistListRequest) -> Result<Self> {
        if request.account.is_some()
            || request.genre != ArtistGenre::All
            || !(1..=100).contains(&request.limit)
            || request.offset.checked_add(request.limit).is_none()
        {
            return Err(kuwo_invalid_request(
                "Kuwo singer lists require an anonymous request, genre=all and limit 1..100",
            ));
        }
        // These are combined categories in the official singer page, not independent
        // region/gender filters. Its Japanese/Korean category cannot express either alone.
        let category = match (request.area, request.category) {
            (ArtistArea::All, ArtistCategory::All) => 0,
            (ArtistArea::Chinese, ArtistCategory::Male) => 1,
            (ArtistArea::Chinese, ArtistCategory::Female) => 2,
            (ArtistArea::Chinese, ArtistCategory::Group) => 3,
            (ArtistArea::JapaneseKorean, ArtistCategory::Male) => 4,
            (ArtistArea::JapaneseKorean, ArtistCategory::Female) => 5,
            (ArtistArea::JapaneseKorean, ArtistCategory::Group) => 6,
            (ArtistArea::Western, ArtistCategory::Male) => 7,
            (ArtistArea::Western, ArtistCategory::Female) => 8,
            (ArtistArea::Western, ArtistCategory::Group) => 9,
            (ArtistArea::Other, ArtistCategory::All) => 10,
            _ => {
                return Err(kuwo_invalid_request(
                    "Kuwo singer lists do not support this area/category combination",
                ));
            }
        };
        let prefix = match request.initial.as_deref().unwrap_or("") {
            "" => String::new(),
            // The official consumer supplies the literal %23 to Axios's params encoder.
            // Sending the decoded # instead returns a different (empty) catalogue.
            "#" => "%23".into(),
            value if value.len() == 1 && value.as_bytes()[0].is_ascii_alphabetic() => {
                value.to_ascii_uppercase()
            }
            _ => {
                return Err(kuwo_invalid_request(
                    "Kuwo singer initial must be one ASCII letter, #, or empty",
                ));
            }
        };
        Ok(Self { category, prefix })
    }
}

#[derive(Serialize)]
struct ListQuery<'a> {
    category: u8,
    prefix: &'a str,
    pn: u32,
    rn: u32,
    #[serde(rename = "httpsStatus")]
    https_status: u8,
    #[serde(rename = "reqId")]
    request_id: String,
    plat: &'static str,
    from: &'static str,
}

impl KuwoClient {
    pub async fn artists(&self, request: &ArtistListRequest) -> Result<Page<Artist>> {
        let selection = Selection::new(request)?;
        let first = request.offset / LIST_PAGE_SIZE + 1;
        let skip = (request.offset % LIST_PAGE_SIZE) as usize;
        let count = (request.offset % LIST_PAGE_SIZE + request.limit).div_ceil(LIST_PAGE_SIZE);
        let mut items = Vec::with_capacity(request.limit as usize);
        let mut total = None;
        let mut seen = BTreeSet::new();
        let mut fetched = 0;
        // At most three fixed-size upstream pages are needed for an arbitrary 100-row window.
        for page in first..first + count {
            let result = self.singer_list_page(&selection, page).await?;
            if total.is_some_and(|value| value != result.total) {
                return Err(kuwo_upstream_error(
                    "Kuwo singer catalogue total changed during pagination",
                ));
            }
            total = Some(result.total);
            fetched += 1;
            for artist in &result.items {
                if !seen.insert(artist.id.clone()) {
                    return Err(kuwo_upstream_error(
                        "Kuwo singer catalogue repeated an artist identity",
                    ));
                }
            }
            items.extend(
                result
                    .items
                    .into_iter()
                    .skip(if page == first { skip } else { 0 })
                    .take(request.limit as usize - items.len()),
            );
            if u64::from(page) * u64::from(LIST_PAGE_SIZE) >= result.total {
                break;
            }
        }
        let end = request.offset + items.len() as u32;
        let has_more = total.is_some_and(|value| u64::from(end) < value);
        Ok(Page {
            items,
            pagination: PageMeta {
                limit: request.limit,
                offset: request.offset,
                total,
                has_more,
                next_offset: has_more.then_some(end),
                extensions: Extensions::from([
                    ("backend".into(), json!("current_web_artist_list")),
                    ("catalog_scope".into(), json!("official_web_singers")),
                    ("order".into(), json!("platform_default")),
                    ("upstream_category".into(), json!(selection.category)),
                    ("upstream_page_size".into(), json!(LIST_PAGE_SIZE)),
                    ("upstream_pages_fetched".into(), json!(fetched)),
                ]),
            },
        })
    }

    async fn singer_list_page(
        &self,
        selection: &Selection,
        page: u32,
    ) -> Result<ArtistPage<Artist>> {
        let started = Instant::now();
        let mut status = None;
        let result = async {
            // The browser's absolute wapi request carries no www Cookie or Secret.
            let response = self
                .http
                .get(self.web_target(ENDPOINT))
                .header(ACCEPT, "application/json")
                .header(REFERER, "https://www.kuwo.cn/singers")
                .query(&ListQuery {
                    category: selection.category,
                    prefix: &selection.prefix,
                    pn: page,
                    rn: LIST_PAGE_SIZE,
                    https_status: 1,
                    request_id: new_request_id(),
                    plat: "web_www",
                    from: "",
                })
                .send()
                .await
                .map_err(kuwo_network_error)?;
            status = Some(response.status());
            let bytes = catalog::read_response(response).await?;
            parse(&bytes, page)
        }
        .await;
        self.log_upstream_request(
            "artist_list",
            "wapi.kuwo.cn",
            PATH,
            status,
            started,
            0,
            false,
            &result,
        );
        result
    }
}

#[derive(Deserialize)]
struct SingerList {
    total: Unsigned,
    #[serde(rename = "artistList")]
    artists: Vec<Singer>,
}

#[derive(Deserialize)]
struct Singer {
    #[serde(flatten)]
    artist: ArtistDto,
    aartist: Option<String>,
    #[serde(rename = "albumNum")]
    album_count: Option<Unsigned>,
    #[serde(rename = "mvNum")]
    mv_count: Option<Unsigned>,
    #[serde(rename = "isStar")]
    is_star: Option<Unsigned>,
}

fn parse(bytes: &[u8], page: u32) -> Result<ArtistPage<Artist>> {
    let body: SingerList = serde_json::from_value(data(bytes, false)?).map_err(|_| invalid())?;
    let total = body.total.value()?;
    let start = u64::from(page - 1) * u64::from(LIST_PAGE_SIZE);
    if body.artists.len() as u64 != total.saturating_sub(start).min(u64::from(LIST_PAGE_SIZE)) {
        return Err(kuwo_upstream_error(
            "Kuwo singer catalogue page length disagrees with its total",
        ));
    }
    let mut items = Vec::with_capacity(body.artists.len());
    for row in body.artists {
        let SearchItem::Artist(mut artist) = catalog::map_artist(row.artist)? else {
            unreachable!("artist mapper")
        };
        artist.album_count = row.album_count.as_ref().map(Unsigned::value).transpose()?;
        artist.mv_count = row.mv_count.as_ref().map(Unsigned::value).transpose()?;
        if let Some(alias) = row.aartist {
            let alias = catalog::text(&alias, 512, false)?;
            if !alias.is_empty() && alias != artist.name {
                artist.aliases.push(alias);
            }
        }
        if let Some(is_star) = row.is_star {
            artist
                .extensions
                .insert("source_is_star".into(), json!(is_star.value()?));
        }
        artist
            .extensions
            .insert("backend".into(), json!("current_web_artist_list"));
        items.push(artist);
    }
    Ok(ArtistPage { items, total })
}

#[cfg(test)]
mod tests;
