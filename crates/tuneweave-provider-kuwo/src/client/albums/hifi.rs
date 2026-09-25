use super::*;
use tuneweave_core::{AlbumListRequest, Page, PageMeta};

const HOST: &str = "wapi.kuwo.cn";
const PATH: &str = "/openapi/v1/pagehome/hifi/getZhizhenInfo";
const ENDPOINT: &str = "https://wapi.kuwo.cn/openapi/v1/pagehome/hifi/getZhizhenInfo";
const PAGE_SIZE: u32 = 18;

#[derive(Clone, Copy)]
enum Sort {
    Latest,
    Hot,
}

impl Sort {
    fn validate(request: &AlbumListRequest) -> Result<Self> {
        if request.account.is_some()
            || request.area.is_some()
            || !(1..=100).contains(&request.limit)
            || request.offset.checked_add(request.limit).is_none()
        {
            return Err(kuwo_invalid_request(
                "Kuwo HiFi album lists require an anonymous request, no area filter and limit 1..100",
            ));
        }
        match request.catalog.as_deref() {
            Some("hifi_latest") => Ok(Self::Latest),
            Some("hifi_hot") => Ok(Self::Hot),
            _ => Err(kuwo_invalid_request(
                "Kuwo album catalogue must be hifi_latest or hifi_hot",
            )),
        }
    }

    const fn id(self) -> &'static str {
        match self {
            Self::Latest => "4001",
            Self::Hot => "4002",
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Self::Latest => "最新",
            Self::Hot => "最热",
        }
    }

    const fn catalog(self) -> &'static str {
        match self {
            Self::Latest => "hifi_latest",
            Self::Hot => "hifi_hot",
        }
    }
}

impl KuwoClient {
    pub async fn native_hifi_albums(
        &self,
        request: &AlbumListRequest,
        device: &KuwoNativeDevice,
    ) -> Result<Page<Album>> {
        let sort = Sort::validate(request)?;
        self.hifi_album_window(request, device, sort).await
    }

    pub(crate) async fn hifi_albums_with_device_store(
        &self,
        request: &AlbumListRequest,
        devices: &KuwoNativeDeviceStore,
    ) -> Result<Page<Album>> {
        let sort = Sort::validate(request)?;
        let device = devices.initialize(self).await?;
        self.hifi_album_window(request, &device, sort).await
    }

    async fn hifi_album_window(
        &self,
        request: &AlbumListRequest,
        device: &KuwoNativeDevice,
        sort: Sort,
    ) -> Result<Page<Album>> {
        // Sorted out-of-range responses reset total to zero. Establish the real
        // catalogue count from page one before requesting a later window.
        let seed = self.hifi_album_page(device, sort, 1).await?;
        let total = seed.total;
        let first = request.offset / PAGE_SIZE + 1;
        let skip = (request.offset % PAGE_SIZE) as usize;
        let mut seen = seed
            .items
            .iter()
            .map(|item| item.id.clone())
            .collect::<BTreeSet<_>>();
        let mut items = Vec::with_capacity(request.limit as usize);
        if first == 1 {
            items.extend(
                seed.items
                    .into_iter()
                    .skip(skip)
                    .take(request.limit as usize),
            );
        }
        let mut fetched = 1;
        let pages = (request.offset % PAGE_SIZE + request.limit).div_ceil(PAGE_SIZE);
        let last = total.div_ceil(u64::from(PAGE_SIZE));
        // At most seven requested pages plus the seed page, even at a large offset.
        for page in first.max(2)..first + pages {
            if u64::from(request.offset) >= total || u64::from(page) > last {
                break;
            }
            let result = self.hifi_album_page(device, sort, page).await?;
            if result.total != total {
                return Err(kuwo_upstream_error(
                    "Kuwo HiFi album catalogue total changed during pagination",
                ));
            }
            for item in &result.items {
                if !seen.insert(item.id.clone()) {
                    return Err(kuwo_upstream_error(
                        "Kuwo HiFi album catalogue repeated an identity across pages",
                    ));
                }
            }
            fetched += 1;
            items.extend(
                result
                    .items
                    .into_iter()
                    .skip(if page == first { skip } else { 0 })
                    .take(request.limit as usize - items.len()),
            );
        }
        let end = request.offset + items.len() as u32;
        let has_more = u64::from(end) < total;
        Ok(Page {
            items,
            pagination: PageMeta {
                limit: request.limit,
                offset: request.offset,
                total: Some(total),
                has_more,
                next_offset: has_more.then_some(end),
                extensions: Extensions::from([
                    ("backend".into(), json!("native_hifi_album_catalogue")),
                    ("catalog".into(), json!(sort.catalog())),
                    ("upstream_sort_id".into(), json!(sort.id())),
                    ("upstream_page_size".into(), json!(PAGE_SIZE)),
                    ("upstream_pages_fetched".into(), json!(fetched)),
                    ("metadata_scope".into(), json!("catalogue_only")),
                ]),
            },
        })
    }

    async fn hifi_album_page(
        &self,
        device: &KuwoNativeDevice,
        sort: Sort,
        page: u32,
    ) -> Result<HifiPage> {
        let started = Instant::now();
        let mut status = None;
        let result = async {
            let response = self
                .http
                .get(self.web_target(ENDPOINT))
                .header(ACCEPT, "application/json")
                .query(&query(device, sort, page))
                .send()
                .await
                .map_err(kuwo_network_error)?;
            status = Some(response.status());
            let bytes = catalog::read_response(response).await?;
            parse(&bytes, sort, page)
        }
        .await;
        self.log_upstream_request(
            "hifi_album_catalogue",
            HOST,
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

fn query(device: &KuwoNativeDevice, sort: Sort, page: u32) -> Vec<(&'static str, String)> {
    [
        ("user", device.device_user().to_owned()),
        ("android_id", device.android_id().to_owned()),
        ("prod", "kwplayer_ar_12.2.2.0".into()),
        ("corp", "kuwo".into()),
        ("newver", "3".into()),
        ("vipver", "12.2.2.0".into()),
        (
            "source",
            "kwplayer_ar_12.2.2.0_newpcguanwangmobile.apk".into(),
        ),
        ("p2p", "1".into()),
        ("q36", "f2ce3c2ef68ddfd1b2bea7ed00001f314716".into()),
        ("approval", "false".into()),
        ("loginUid", "0".into()),
        ("loginSid", "0".into()),
        ("appuid", device.app_uid().to_owned()),
        ("allpay", "0".into()),
        ("notrace", "1".into()),
        ("oaid", "".into()),
        ("vipMode", "0".into()),
        ("plat", "ar".into()),
        ("uid", device.app_uid().to_owned()),
        ("pn", page.to_string()),
        ("rn", PAGE_SIZE.to_string()),
        ("sort", sort.id().into()),
    ]
    .into()
}

struct HifiPage {
    total: u64,
    items: Vec<Album>,
}

#[derive(Deserialize)]
struct HifiEnvelope {
    code: i64,
    success: bool,
    data: Option<HifiData>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct HifiData {
    total: Unsigned,
    album_list: Vec<HifiAlbum>,
    tag_list: Vec<TagGroup>,
}

#[derive(Deserialize)]
struct TagGroup {
    key: String,
    tags: Vec<Tag>,
}

#[derive(Deserialize)]
struct Tag {
    id: Unsigned,
    name: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct HifiAlbum {
    id: Unsigned,
    name: String,
    artist: String,
    artist_id: Option<Unsigned>,
    img: Option<String>,
    #[serde(rename = "type")]
    kind: String,
    #[serde(alias = "isstar")]
    is_star: Unsigned,
    #[serde(rename = "content_type")]
    content_type: Option<Unsigned>,
    hires_status: Option<Unsigned>,
}

fn parse(bytes: &[u8], sort: Sort, page: u32) -> Result<HifiPage> {
    let body: HifiEnvelope = serde_json::from_slice(bytes).map_err(|_| invalid_hifi())?;
    if body.code != 200 || !body.success {
        return Err(invalid_hifi());
    }
    let data = body.data.ok_or_else(invalid_hifi)?;
    let total = data.total.value()?;
    let start = u64::from(page - 1) * u64::from(PAGE_SIZE);
    if data.album_list.len() as u64 != total.saturating_sub(start).min(u64::from(PAGE_SIZE))
        || data.tag_list.len() > 16
    {
        return Err(invalid_hifi());
    }
    // The IDs are supplied by the official filter DTO. Verify their current
    // meaning instead of silently exposing an unrelated or default random feed.
    let sorts = data
        .tag_list
        .iter()
        .filter(|tag| tag.key == "sort")
        .collect::<Vec<_>>();
    if sorts.len() != 1 || sorts[0].tags.len() > 128 {
        return Err(invalid_hifi());
    }
    let mut tag_ids = BTreeSet::new();
    let mut selected = false;
    for tag in &sorts[0].tags {
        let id = tag.id.id()?;
        if !tag_ids.insert(id.clone()) {
            return Err(invalid_hifi());
        }
        if id == sort.id() {
            if catalog::text(&tag.name, 128, false)? != sort.label() {
                return Err(invalid_hifi());
            }
            selected = true;
        }
    }
    if !selected {
        return Err(invalid_hifi());
    }
    let mut seen = BTreeSet::new();
    let mut items = Vec::with_capacity(data.album_list.len());
    for row in data.album_list {
        let id = row.id.id()?;
        let name = catalog::text(&row.name, 512, false)?;
        if !seen.insert(id.clone())
            || name.is_empty()
            || row.kind != "album"
            || row.is_star.value()? != 0
            || row
                .content_type
                .as_ref()
                .map(Unsigned::value)
                .transpose()?
                .is_some_and(|value| value != 0)
        {
            return Err(invalid_hifi());
        }
        let artists = if let Some(artist_id) = row.artist_id {
            artists::credits(&row.artist, &artist_id.id()?)?
        } else {
            let name = catalog::text(&row.artist, 1024, false)?;
            if name.is_empty() {
                Vec::new()
            } else {
                vec![ArtistSummary {
                    resource_ref: None,
                    name,
                }]
            }
        };
        let mut extensions = Extensions::from([
            ("backend".into(), json!("native_hifi_album_catalogue")),
            ("source_type".into(), json!("album")),
            ("metadata_scope".into(), json!("catalogue_only")),
        ]);
        if let Some(hires) = row.hires_status {
            let value = hires.value()?;
            if value > 1 {
                return Err(invalid_hifi());
            }
            extensions.insert("catalogue_hires_status".into(), json!(value));
        }
        items.push(Album {
            resource_ref: ResourceRef::new(Platform::Kuwo, id.clone())
                .map_err(|_| invalid_hifi())?,
            platform: Platform::Kuwo,
            id,
            name,
            artists,
            aliases: Vec::new(),
            description: String::new(),
            cover_url: row.img.as_deref().and_then(cover),
            published_at: None,
            track_count: None,
            company: None,
            kind: None,
            extensions,
        });
    }
    Ok(HifiPage { total, items })
}

fn invalid_hifi() -> TuneWeaveError {
    kuwo_upstream_error("Kuwo HiFi album catalogue returned invalid metadata")
}

fn cover(value: &str) -> Option<String> {
    let mut url = Url::parse(value.trim()).ok()?;
    if url.scheme() == "http" {
        url.set_scheme("https").ok()?;
    }
    normalize_official_image_url(url.as_str())
}

#[cfg(test)]
mod tests;
