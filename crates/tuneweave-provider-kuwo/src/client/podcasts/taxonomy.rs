use super::*;
use std::collections::BTreeSet;

const CATEGORY_ENDPOINT: &str = "https://wapi.kuwo.cn/api/fm/category/all";
const CATEGORY_PATH: &str = "/api/fm/category/all";
const CATEGORY_LIST_ENDPOINT: &str = "https://wapi.kuwo.cn/api/fm/category/list";
const CATEGORY_LIST_PATH: &str = "/api/fm/category/list";
const CATEGORY_PAGE_SIZE: u64 = 10;
const MAX_CATEGORIES: usize = 32;
const MAX_CATEGORY_RESPONSE_BYTES: u64 = 2 * 1024 * 1024;

#[derive(Deserialize)]
struct CategoryEnvelope {
    code: i64,
    data: Option<CategoryData>,
}

#[derive(Deserialize)]
struct CategoryData {
    list: Vec<RawCategory>,
}

#[derive(Deserialize)]
struct RawCategory {
    id: Unsigned,
    name: String,
    #[serde(rename = "type")]
    native_type: String,
    icon: Option<String>,
    #[serde(rename = "linkUrl")]
    link_url: Option<String>,
    desc: Option<String>,
}

#[derive(Deserialize)]
struct CategoryPageEnvelope {
    code: i64,
    data: Option<RawCategoryPage>,
}

#[derive(Deserialize)]
struct RawCategoryPage {
    total: Unsigned,
    pn: Unsigned,
    rn: Unsigned,
    list: Vec<RawCategoryAlbum>,
}

#[derive(Deserialize)]
struct RawCategoryAlbum {
    id: Unsigned,
    #[serde(rename = "type")]
    item_type: String,
    isstar: Unsigned,
    name: String,
    desc: Option<String>,
    pic: Option<String>,
    artist: Option<String>,
    #[serde(rename = "artistId")]
    artist_id: Option<Unsigned>,
    #[serde(rename = "musicCount")]
    music_count: Option<Unsigned>,
    #[serde(rename = "collectCount")]
    collect_count: Option<Unsigned>,
    #[serde(rename = "playCount")]
    play_count: Option<Unsigned>,
}

impl KuwoClient {
    pub async fn podcast_categories(
        &self,
        request: &PodcastTaxonomyRequest,
    ) -> Result<PodcastTaxonomy> {
        if request.account.is_some() {
            return Err(kuwo_invalid_request(
                "Kuwo podcast categories are anonymous and do not accept an account",
            ));
        }
        if request.kind != PodcastTaxonomyKind::All {
            return Err(unsupported(
                "Kuwo does not expose a separate non-hot podcast taxonomy",
            ));
        }

        let bytes = self
            .request_category_json(CATEGORY_ENDPOINT, CATEGORY_PATH, "podcast_categories", &[])
            .await?;
        parse_categories(&bytes)
    }

    pub async fn podcasts(&self, request: &PodcastListRequest) -> Result<Page<Podcast>> {
        if request.account.is_some() {
            return Err(kuwo_invalid_request(
                "Kuwo podcast catalogs are anonymous and do not accept an account",
            ));
        }
        let (sort, catalog_name) = match request.catalog {
            PodcastCatalog::CategoryHot => (1, "category_hot"),
            PodcastCatalog::CategoryNewest => (2, "category_newest"),
            _ => {
                return Err(unsupported(
                    "Kuwo exposes only hot and recently-updated catalogs inside an anchor category",
                ));
            }
        };
        if request.page.is_some() {
            return Err(unsupported(
                "Kuwo anchor category catalogs do not accept a separate page parameter",
            ));
        }
        if !(1..=100).contains(&request.limit) {
            return Err(kuwo_invalid_request(
                "Kuwo podcast catalog limit must be between 1 and 100",
            ));
        }
        if request.offset % request.limit != 0 {
            return Err(kuwo_invalid_request(
                "Kuwo category podcast offset must align to the requested page size",
            ));
        }
        let next_offset = request
            .offset
            .checked_add(request.limit)
            .ok_or_else(|| kuwo_invalid_request("Kuwo podcast catalog offset overflowed"))?;
        let category_id = request
            .category_id
            .as_deref()
            .ok_or_else(|| kuwo_invalid_request("Kuwo category catalog requires category_id"))?;
        if category_id
            .parse::<u64>()
            .ok()
            .filter(|id| *id > 0)
            .is_none_or(|id| id.to_string() != category_id)
        {
            return Err(kuwo_invalid_request(
                "Kuwo podcast category ID must be a positive decimal ID",
            ));
        }

        let taxonomy_bytes = self
            .request_category_json(CATEGORY_ENDPOINT, CATEGORY_PATH, "podcast_catalog", &[])
            .await?;
        let taxonomy = parse_categories(&taxonomy_bytes)?;
        let category = taxonomy
            .categories
            .into_iter()
            .find(|category| {
                category.id == category_id
                    && category
                        .extensions
                        .get("native_type")
                        .and_then(serde_json::Value::as_str)
                        == Some("cat")
            })
            .ok_or_else(|| {
                TuneWeaveError::new(
                    ErrorCode::ResourceNotFound,
                    "Kuwo podcast category was not found",
                )
                .with_platform(Platform::Kuwo)
            })?;
        let page = request.offset / request.limit + 1;
        let query = [
            ("parentId", format!("cat.{category_id}")),
            ("pn", page.to_string()),
            ("rn", request.limit.to_string()),
            ("sort", sort.to_string()),
        ];
        let bytes = self
            .request_category_json(
                CATEGORY_LIST_ENDPOINT,
                CATEGORY_LIST_PATH,
                "podcast_catalog",
                &query,
            )
            .await?;
        let (total, mut items) = parse_category_album_page(
            category_id,
            &category,
            &bytes,
            page,
            u64::from(request.limit),
            true,
        )?;
        if u64::from(request.offset) >= total {
            items.clear();
        }
        let returned_count = items.len();
        let has_more = u64::from(next_offset) < total;
        Ok(Page {
            items,
            pagination: PageMeta {
                limit: request.limit,
                offset: request.offset,
                total: Some(total),
                next_offset: has_more.then_some(next_offset),
                has_more,
                extensions: Extensions::from([
                    ("backend".to_owned(), json!("kuwo_fm_category_list")),
                    ("catalog".to_owned(), json!(catalog_name)),
                    ("category_id".to_owned(), json!(category_id)),
                    ("upstream_page".to_owned(), json!(page)),
                    ("upstream_page_size".to_owned(), json!(request.limit)),
                    ("returned_count".to_owned(), json!(returned_count)),
                ]),
            },
        })
    }

    pub async fn podcast_category_recommendations(
        &self,
        account: Option<&str>,
    ) -> Result<PodcastCategoryRecommendations> {
        if account.is_some() {
            return Err(kuwo_invalid_request(
                "Kuwo podcast category recommendations are anonymous and do not accept an account",
            ));
        }

        let bytes = self
            .request_category_json(
                CATEGORY_ENDPOINT,
                CATEGORY_PATH,
                "podcast_category_recommendations",
                &[],
            )
            .await?;
        let taxonomy = parse_categories(&bytes)?;
        let categories: Vec<_> = taxonomy
            .categories
            .into_iter()
            .filter(|category| {
                category
                    .extensions
                    .get("native_type")
                    .and_then(serde_json::Value::as_str)
                    == Some("cat")
            })
            .collect();
        if categories.len() > MAX_CATEGORIES {
            return Err(invalid());
        }

        let mut sections = Vec::with_capacity(categories.len());
        for category in categories {
            let category_id = category.id.clone();
            let query = [
                ("parentId", format!("cat.{category_id}")),
                ("pn", "1".to_owned()),
                ("rn", CATEGORY_PAGE_SIZE.to_string()),
                ("sort", "1".to_owned()),
            ];
            let bytes = self
                .request_category_json(
                    CATEGORY_LIST_ENDPOINT,
                    CATEGORY_LIST_PATH,
                    "podcast_category_recommendations",
                    &query,
                )
                .await?;
            sections.push(parse_category_recommendation(category, &bytes)?);
        }

        Ok(PodcastCategoryRecommendations {
            sections,
            extensions: Extensions::from([
                ("backend".to_owned(), json!("kuwo_fm_category_list")),
                ("sort".to_owned(), json!("hot")),
                ("page".to_owned(), json!(1)),
                ("page_size".to_owned(), json!(CATEGORY_PAGE_SIZE)),
                ("first_page_only".to_owned(), json!(true)),
            ]),
        })
    }

    async fn request_category_json(
        &self,
        endpoint: &str,
        path: &'static str,
        operation: &'static str,
        query: &[(&str, String)],
    ) -> Result<Vec<u8>> {
        let started = Instant::now();
        let mut status = None;
        let result = tokio::time::timeout(Duration::from_secs(10), async {
            let mut request = self
                .http
                .get(self.web_target(endpoint))
                .header(ACCEPT, "application/json");
            if !query.is_empty() {
                request = request.query(query);
            }
            let response = request.send().await.map_err(kuwo_network_error)?;
            status = Some(response.status());
            let mime = response
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.split(';').next());
            if !mime.is_some_and(|value| value.trim().eq_ignore_ascii_case("application/json")) {
                return Err(invalid());
            }
            read_bounded_response_with_limit(
                response,
                "Kuwo podcast category response",
                MAX_CATEGORY_RESPONSE_BYTES,
            )
            .await
        })
        .await
        .unwrap_or_else(|_| {
            Err(TuneWeaveError::new(
                ErrorCode::UpstreamTimeout,
                "Kuwo podcast category request exceeded the time budget",
            )
            .with_platform(Platform::Kuwo))
        });
        self.log_upstream_request(
            operation,
            "wapi.kuwo.cn",
            path,
            status,
            started,
            0,
            false,
            &result,
        );
        result
    }
}

fn parse_categories(bytes: &[u8]) -> Result<PodcastTaxonomy> {
    let envelope: CategoryEnvelope = serde_json::from_slice(bytes).map_err(|_| invalid())?;
    if envelope.code != 200 {
        return Err(invalid());
    }
    let raw = envelope.data.ok_or_else(invalid)?.list;
    if raw.is_empty() || raw.len() > MAX_CATEGORIES {
        return Err(invalid());
    }

    let categories = raw
        .into_iter()
        .map(map_category)
        .collect::<Result<Vec<_>>>()?;
    let mut ids = BTreeSet::new();
    if categories
        .iter()
        .any(|category| !ids.insert(category.id.as_str()))
    {
        return Err(invalid());
    }
    Ok(PodcastTaxonomy {
        categories,
        extensions: Extensions::from([
            ("backend".to_owned(), json!("kuwo_fm_category_all")),
            ("kind".to_owned(), json!("all")),
            ("scope".to_owned(), json!("official_classification_page")),
        ]),
    })
}

fn map_category(raw: RawCategory) -> Result<PodcastCategory> {
    let id = raw.id.id()?;
    text(&raw.name, 256, false)?;
    if raw.name.trim().is_empty() || raw.native_type.trim().is_empty() {
        return Err(invalid());
    }
    text(&raw.native_type, 64, false)?;
    if let Some(description) = &raw.desc {
        text(description, 4096, true)?;
    }
    let icon_url = raw.icon.filter(|value| !value.is_empty());
    let icon_url = icon_url.map(category_image).transpose()?;
    let mut extensions = Extensions::from([
        ("native_type".to_owned(), json!(raw.native_type)),
        (
            "description".to_owned(),
            json!(raw.desc.unwrap_or_default()),
        ),
    ]);
    if let Some(link) = raw.link_url.filter(|value| !value.is_empty()) {
        extensions.insert("official_link".to_owned(), json!(category_link(&link)?));
    }
    Ok(PodcastCategory {
        id,
        name: raw.name,
        icon_url,
        extensions,
    })
}

fn parse_category_recommendation(
    category: PodcastCategory,
    bytes: &[u8],
) -> Result<PodcastCategoryRecommendation> {
    let (total, podcasts) =
        parse_category_album_page(&category.id, &category, bytes, 1, CATEGORY_PAGE_SIZE, false)?;
    let returned = u64::try_from(podcasts.len()).map_err(|_| invalid())?;
    Ok(PodcastCategoryRecommendation {
        category,
        podcasts,
        extensions: Extensions::from([
            ("total".to_owned(), json!(total)),
            ("returned_count".to_owned(), json!(returned)),
            ("page".to_owned(), json!(1)),
            ("page_size".to_owned(), json!(CATEGORY_PAGE_SIZE)),
            ("sort".to_owned(), json!("hot")),
            ("complete".to_owned(), json!(total <= returned)),
        ]),
    })
}

fn parse_category_album_page(
    requested_category: &str,
    category: &PodcastCategory,
    bytes: &[u8],
    requested_page: u32,
    requested_page_size: u64,
    allow_empty_page: bool,
) -> Result<(u64, Vec<Podcast>)> {
    if category.id != requested_category || requested_page == 0 || requested_page_size == 0 {
        return Err(invalid());
    }
    let envelope: CategoryPageEnvelope = serde_json::from_slice(bytes).map_err(|_| invalid())?;
    if envelope.code != 200 {
        return Err(invalid());
    }
    let page = envelope.data.ok_or_else(invalid)?;
    let total = page.total.value()?;
    if page.pn.value()? != u64::from(requested_page) || page.rn.value()? != requested_page_size {
        return Err(invalid());
    }
    let page_start = u64::from(requested_page - 1)
        .checked_mul(requested_page_size)
        .ok_or_else(invalid)?;
    let returned = u64::try_from(page.list.len()).map_err(|_| invalid())?;
    if returned > requested_page_size
        || returned > total
        || (!allow_empty_page && page_start < total && returned == 0)
    {
        return Err(invalid());
    }
    let podcasts = page
        .list
        .into_iter()
        .map(|item| map_category_album(item, category))
        .collect::<Result<Vec<_>>>()?;
    Ok((total, podcasts))
}

fn map_category_album(raw: RawCategoryAlbum, category: &PodcastCategory) -> Result<Podcast> {
    let id = raw.id.id()?;
    if raw.item_type != "album" || raw.isstar.value()? != 1 {
        return Err(invalid());
    }
    text(&raw.name, 2048, false)?;
    if raw.name.trim().is_empty() {
        return Err(invalid());
    }
    let resource =
        ResourceRef::new(Platform::Kuwo, format!("anchor:{id}")).map_err(|_| invalid())?;
    let mut podcast = Podcast::new(resource, raw.name);
    podcast.cover_url = image(raw.pic.filter(|value| !value.is_empty()))?;
    podcast.episode_count = raw.music_count.map(|value| value.value()).transpose()?;
    podcast.subscriber_count = raw.collect_count.map(|value| value.value()).transpose()?;
    podcast.play_count = raw.play_count.map(|value| value.value()).transpose()?;
    if let Some(name) = raw.artist.filter(|value| !value.trim().is_empty()) {
        text(&name, 2048, false)?;
        let resource_ref = raw
            .artist_id
            .map(|value| value.id())
            .transpose()?
            .map(|id| ResourceRef::new(Platform::Kuwo, id).map_err(|_| invalid()))
            .transpose()?;
        podcast.creator = Some(CreatorSummary {
            resource_ref,
            name,
            avatar_url: None,
        });
    }
    if let Some(description) = raw.desc.filter(|value| !value.is_empty()) {
        text(&description, 16 * 1024, true)?;
        podcast
            .extensions
            .insert("latest_episode_description".to_owned(), json!(description));
    }
    podcast.extensions.extend([
        ("backend".to_owned(), json!("kuwo_fm_category_album")),
        ("category_id".to_owned(), json!(category.id)),
        ("category_name".to_owned(), json!(category.name)),
    ]);
    Ok(podcast)
}

fn category_image(value: String) -> Result<String> {
    text(&value, 2048, false)?;
    let url = Url::parse(&value).map_err(|_| invalid())?;
    if !matches!(url.scheme(), "https" | "http")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || !url.host_str().is_some_and(|host| {
            [
                "img1.kuwo.cn",
                "img2.kuwo.cn",
                "img3.kuwo.cn",
                "img4.kuwo.cn",
                "kwimg1.kuwo.cn",
                "kwimg2.kuwo.cn",
                "kwimg3.kuwo.cn",
                "kwimg4.kuwo.cn",
            ]
            .contains(&host)
        })
        || !url.path().starts_with("/star/upload/")
    {
        return Err(invalid());
    }
    Ok(value)
}

fn category_link(value: &str) -> Result<String> {
    text(value, 2048, false)?;
    let url = Url::parse(value).map_err(|_| invalid())?;
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
        || url.host_str() != Some("kweex.kuwo.cn")
        || url.path() != "/500005/web/KwCategoryPage.html"
        || url.fragment().is_some()
    {
        return Err(invalid());
    }
    let mut category_id = None;
    for (key, value) in url.query_pairs() {
        match key.as_ref() {
            "categoryId" if category_id.is_none() => {
                category_id = value.parse::<u64>().ok().filter(|id| *id > 0);
            }
            "from" if value == "diantai-more" => {}
            _ => return Err(invalid()),
        }
    }
    if category_id.is_none() {
        return Err(invalid());
    }
    Ok(value.to_owned())
}

fn invalid() -> TuneWeaveError {
    kuwo_upstream_error("Kuwo podcast category returned an invalid response")
}

fn unsupported(message: &str) -> TuneWeaveError {
    TuneWeaveError::new(ErrorCode::CapabilityNotSupported, message).with_platform(Platform::Kuwo)
}
