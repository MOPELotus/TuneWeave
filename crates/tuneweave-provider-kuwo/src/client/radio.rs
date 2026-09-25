//! Native FM broadcasts have their own identities and live-stream URLs, independent of Music.
use super::*;
use catalog::Unsigned;
use serde::de::DeserializeOwned;
use std::{collections::BTreeSet, future::Future};
use tuneweave_core::{
    Page, PageMeta, RadioCatalogOption, RadioStation, RadioStationListRequest, RadioTaxonomy,
    RadioTaxonomyRequest,
};

const BASE: &str = "https://wapi.kuwo.cn/api/fmradio/";
const BUDGET: Duration = Duration::from_secs(45);
const RESPONSE_LIMIT: u64 = 2 * 1024 * 1024;
const MAX_OPTIONS: usize = 256;
const MAX_STATIONS: usize = 10_000;

#[derive(Deserialize)]
struct Envelope<T> {
    code: i64,
    success: Option<bool>,
    data: T,
    #[serde(rename = "curTime")]
    time: Option<Unsigned>,
}

#[derive(Deserialize)]
struct Category {
    #[serde(rename = "categoryKey")]
    id: Unsigned,
    name: String,
}

#[derive(Deserialize)]
struct Region {
    #[serde(rename = "locationKey")]
    id: Unsigned,
    name: String,
}

#[derive(Deserialize)]
struct Station {
    channel_key: Unsigned,
    channel_name: String,
    channel_image_url: Option<String>,
    category_key: Option<Unsigned>,
    location_key: Option<Unsigned>,
    flow_url: Option<String>,
    hz: Option<String>,
    program_key: Option<Unsigned>,
    program_name: Option<String>,
    program_compere: Option<String>,
    listener_count: Option<Unsigned>,
}

enum Endpoint<'a> {
    Categories,
    Regions,
    Hot,
    Category(&'a str),
    Region(&'a str),
    Station(&'a str),
}

impl Endpoint<'_> {
    fn suffix(&self) -> String {
        match self {
            Self::Categories => "category_list".into(),
            Self::Regions => "location_list".into(),
            Self::Hot => "hot_list".into(),
            Self::Category(id) => format!("category_radio_list/{id}"),
            Self::Region(id) => format!("location_radio_list/{id}"),
            Self::Station(id) => format!("radio_info/{id}"),
        }
    }
    const fn log_path(&self) -> &'static str {
        match self {
            Self::Categories => "/api/fmradio/category_list",
            Self::Regions => "/api/fmradio/location_list",
            Self::Hot => "/api/fmradio/hot_list",
            Self::Category(_) => "/api/fmradio/category_radio_list/{id}",
            Self::Region(_) => "/api/fmradio/location_radio_list/{id}",
            Self::Station(_) => "/api/fmradio/radio_info/{id}",
        }
    }
}

impl KuwoClient {
    /// Anonymous native FM navigation and regions. Special navigation is kept outside genres.
    pub async fn radio_taxonomy(&self, request: &RadioTaxonomyRequest) -> Result<RadioTaxonomy> {
        anonymous(request.account.as_deref())?;
        bounded(async {
            let menu: Envelope<Vec<Category>> = self.fm_get(Endpoint::Categories).await?;
            let regions: Envelope<Vec<Region>> = self.fm_get(Endpoint::Regions).await?;
            if menu.data.len() > MAX_OPTIONS || regions.data.len() > MAX_OPTIONS {
                return Err(invalid());
            }
            let mut seen = BTreeSet::new();
            let mut categories = Vec::new();
            let mut navigation = Vec::new();
            for item in menu.data {
                let id = catalog_id(&item.id)?;
                text_field(&item.name, 256)?;
                if item.name.trim().is_empty() || !seen.insert(id.clone()) {
                    return Err(invalid());
                }
                let role = match id.as_str() {
                    "95" => Some("account_favorites"),
                    "96" => Some("artists"),
                    "97" => Some("local_recent"),
                    "98" => Some("regions"),
                    "99" => Some("hot_list"),
                    _ => None,
                };
                if let Some(role) = role {
                    navigation.push(json!({"id":id,"name":item.name,"role":role}));
                } else {
                    categories.push(RadioCatalogOption {
                        id,
                        name: item.name,
                        extensions: Extensions::new(),
                    });
                }
            }
            seen.clear();
            let regions = regions
                .data
                .into_iter()
                .map(|item| {
                    let id = catalog_id(&item.id)?;
                    text_field(&item.name, 256)?;
                    if item.name.trim().is_empty() || !seen.insert(id.clone()) {
                        return Err(invalid());
                    }
                    Ok(RadioCatalogOption {
                        id,
                        name: item.name,
                        extensions: Extensions::new(),
                    })
                })
                .collect::<Result<_>>()?;
            let mut extensions = metadata_scope();
            extensions.insert("native_navigation".into(), json!(navigation));
            extensions.insert(
                "category_scope".into(),
                json!("content_categories_in_current_native_menu"),
            );
            Ok(RadioTaxonomy {
                categories,
                regions,
                extensions,
            })
        })
        .await
    }

    /// One native FM list, windowed locally. Category/region filters cannot be combined.
    pub async fn radio_stations(
        &self,
        request: &RadioStationListRequest,
    ) -> Result<Page<RadioStation>> {
        anonymous(request.account.as_deref())?;
        if !(1..=100).contains(&request.limit)
            || request.offset.checked_add(request.limit).is_none()
            || request.cursor.is_some()
            || (request.category_id.is_some() && request.region_id.is_some())
        {
            return Err(kuwo_invalid_request(
                "Kuwo FM requires valid offset/limit, no cursor and at most one category or region",
            ));
        }
        let endpoint = if let Some(id) = request.category_id.as_deref() {
            request_catalog_id(id)?;
            if (95..=99).contains(&id.parse::<u32>().expect("validated category")) {
                return Err(kuwo_invalid_request(
                    "Kuwo FM special navigation is not a content category; omit filters for hot stations",
                ));
            }
            Endpoint::Category(id)
        } else if let Some(id) = request.region_id.as_deref() {
            request_catalog_id(id)?;
            Endpoint::Region(id)
        } else {
            Endpoint::Hot
        };
        bounded(async {
            let envelope: Envelope<Vec<Station>> = self.fm_get(endpoint).await?;
            if envelope.data.len() > MAX_STATIONS {
                return Err(invalid());
            }
            let time = response_time(envelope.time)?;
            let mut seen = BTreeSet::new();
            let mut stations = Vec::with_capacity(envelope.data.len());
            for item in envelope.data {
                let station = item.into_station(time)?;
                if !seen.insert(station.id.clone())
                    || request
                        .category_id
                        .as_ref()
                        .is_some_and(|id| station.extensions.get("category_id") != Some(&json!(id)))
                    || request
                        .region_id
                        .as_ref()
                        .is_some_and(|id| station.extensions.get("region_id") != Some(&json!(id)))
                {
                    return Err(invalid());
                }
                stations.push(station);
            }
            let total = stations.len() as u64;
            let items: Vec<_> = stations
                .into_iter()
                .skip(request.offset as usize)
                .take(request.limit as usize)
                .collect();
            let next = request.offset + items.len() as u32;
            let has_more = u64::from(next) < total;
            let mut extensions = metadata_scope();
            extensions.insert("pagination_scope".into(), json!("single_upstream_response"));
            extensions.insert("upstream_pages_fetched".into(), json!(1));
            extensions.insert("category_id".into(), json!(request.category_id));
            extensions.insert("region_id".into(), json!(request.region_id));
            Ok(Page {
                items,
                pagination: PageMeta {
                    limit: request.limit,
                    offset: request.offset,
                    total: Some(total),
                    has_more,
                    next_offset: has_more.then_some(next),
                    extensions,
                },
            })
        })
        .await
    }

    /// A broadcast station, accepting a canonical `fm:<channel_key>` or bare channel key.
    pub async fn radio_station(&self, id: &str, account: Option<&str>) -> Result<RadioStation> {
        anonymous(account)?;
        let id = id.strip_prefix("fm:").unwrap_or(id);
        request_id(id)?;
        bounded(async {
            let envelope: Envelope<Station> = self.fm_get(Endpoint::Station(id)).await?;
            if envelope.data.channel_key.id()? != id {
                return Err(invalid());
            }
            envelope.data.into_station(response_time(envelope.time)?)
        })
        .await
    }

    async fn fm_get<T: DeserializeOwned>(&self, endpoint: Endpoint<'_>) -> Result<Envelope<T>> {
        let started = Instant::now();
        let mut status = None;
        let outcome = async {
            // Native public reads have no login session, cookie bootstrap or Secret header.
            let mut request = self
                .http
                .get(self.web_target(&format!("{BASE}{}", endpoint.suffix())))
                .header(ACCEPT, "application/json");
            if matches!(endpoint, Endpoint::Categories) {
                request = request.query(&[("loginUid", "0")]);
            }
            let response = request.send().await.map_err(kuwo_network_error)?;
            status = Some(response.status());
            if !response.status().is_success() {
                return Err(kuwo_http_error(response.status()));
            }
            let mime = response
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.split(';').next());
            if !mime.is_some_and(|value| value.trim().eq_ignore_ascii_case("application/json")) {
                return Err(invalid());
            }
            let bytes =
                read_bounded_response_with_limit(response, "Kuwo FM", RESPONSE_LIMIT).await?;
            let body: Envelope<T> = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
            if body.code != 200 || body.success == Some(false) {
                return Err(invalid());
            }
            Ok(body)
        }
        .await;
        self.log_upstream_request(
            "native_fm_metadata",
            "wapi.kuwo.cn",
            endpoint.log_path(),
            status,
            started,
            0,
            false,
            &outcome,
        );
        outcome
    }
}

impl Station {
    fn into_station(self, time: Option<u64>) -> Result<RadioStation> {
        let id = self.channel_key.id()?;
        text_field(&self.channel_name, 512)?;
        if self.channel_name.trim().is_empty() {
            return Err(invalid());
        }
        let mut station = RadioStation::new(
            ResourceRef::new(Platform::Kuwo, format!("fm:{id}")).map_err(|_| invalid())?,
            self.channel_name,
        );
        station.extensions = metadata_scope();
        station.extensions.insert("channel_id".into(), json!(id));
        station
            .extensions
            .insert("response_time_ms".into(), json!(time));
        for (value, name) in [
            (self.category_key, "category_id"),
            (self.location_key, "region_id"),
        ] {
            if let Some(value) = value {
                station
                    .extensions
                    .insert(name.into(), json!(catalog_id(&value)?));
            }
        }
        let stream = optional_text(self.flow_url, 2048)?;
        if let Some(stream) = stream {
            // Pin the complete observed URL shape, including its station identity. Do not
            // accept foreign hosts, query credentials, encoded paths or another station.
            if stream != format!("https://hls-pull-fm.kuwo.cn/kuwofm/stream_key_{id}.m3u8") {
                return Err(invalid());
            }
            station.stream_url = Some(stream);
        }
        let image = optional_text(self.channel_image_url, 2048)?;
        station.cover_url =
            image.filter(|url| url == &format!("https://image.kuwo.cn/mobile/fmRadio/{id}.jpg"));
        station.current_program = optional_text(self.program_name, 2048)?;
        if let Some(host) = optional_text(self.program_compere, 512)? {
            station
                .extensions
                .insert("program_presenter".into(), json!(host));
        }
        if let Some(frequency) = optional_text(self.hz, 128)? {
            station.extensions.insert(
                "frequency_label".into(),
                json!(frequency.split(' ').next().unwrap_or_default()),
            );
        }
        if let Some(key) = self.program_key {
            station
                .extensions
                .insert("program_key".into(), json!(key.value()?.to_string()));
        }
        if let Some(count) = self.listener_count {
            station
                .extensions
                .insert("reported_listener_count".into(), json!(count.value()?));
        }
        // Category/region labels, subscription state and audio duration/codec are unknown.
        Ok(station)
    }
}

fn metadata_scope() -> Extensions {
    Extensions::from([
        ("backend".into(), json!("native_fm")),
        (
            "stream_validation".into(),
            json!("official_url_metadata_only"),
        ),
        (
            "upstream_maintenance_notice".into(),
            json!({
                "title":"广播电台服务停止维护",
                "url":"https://h5app.kuwo.cn/m/kwtemplatePage/index.html?id=1169",
                "observed_on":"2026-09-23"
            }),
        ),
    ])
}

fn anonymous(account: Option<&str>) -> Result<()> {
    if account.is_some() {
        return Err(kuwo_invalid_request(
            "Kuwo public FM metadata does not accept an account",
        ));
    }
    Ok(())
}

fn request_id(id: &str) -> Result<()> {
    if id
        .parse::<u64>()
        .ok()
        .is_none_or(|value| value == 0 || value.to_string() != id)
    {
        return Err(kuwo_invalid_request(
            "Kuwo FM requires a canonical positive ID",
        ));
    }
    Ok(())
}

fn request_catalog_id(id: &str) -> Result<()> {
    request_id(id)?;
    if id.parse::<i32>().is_err() {
        return Err(kuwo_invalid_request(
            "Kuwo FM category and region IDs must fit a positive signed 32-bit integer",
        ));
    }
    Ok(())
}

fn catalog_id(id: &Unsigned) -> Result<String> {
    let value = id.id()?;
    request_catalog_id(&value).map_err(|_| invalid())?;
    Ok(value)
}

fn response_time(time: Option<Unsigned>) -> Result<Option<u64>> {
    time.map(|value| value.value()).transpose()
}

fn text_field(value: &str, limit: usize) -> Result<()> {
    if value.len() > limit || value.chars().any(char::is_control) {
        return Err(invalid());
    }
    Ok(())
}

fn optional_text(value: Option<String>, limit: usize) -> Result<Option<String>> {
    value
        .map(|value| {
            text_field(&value, limit)?;
            Ok((!value.trim().is_empty()).then_some(value))
        })
        .transpose()
        .map(Option::flatten)
}

async fn bounded<T>(work: impl Future<Output = Result<T>>) -> Result<T> {
    tokio::time::timeout(BUDGET, work).await.map_err(|_| {
        TuneWeaveError::new(
            ErrorCode::UpstreamTimeout,
            "Kuwo FM exceeded the total time budget",
        )
        .with_platform(Platform::Kuwo)
    })?
}

fn invalid() -> TuneWeaveError {
    kuwo_upstream_error("Kuwo FM returned an invalid response")
}
