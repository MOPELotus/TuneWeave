use super::*;
use std::collections::BTreeSet;
use tuneweave_core::{Page, PageMeta, PodcastEpisode, PodcastEpisodeListRequest};

const LIMIT: usize = 4 * 1024 * 1024;

#[cfg(test)]
mod tests;

impl KuwoClient {
    /// Reads native anchor programme metadata in the requested order, without media authorization.
    pub async fn podcast_episodes(
        &self,
        id: &str,
        request: &PodcastEpisodeListRequest,
    ) -> Result<Page<PodcastEpisode>> {
        let album = request_album(id, request.account.as_deref())?;
        if !(1..=100).contains(&request.limit)
            || request.offset.checked_add(request.limit).is_none()
        {
            return Err(kuwo_invalid_request(
                "Kuwo podcast episode pagination is outside the supported range",
            ));
        }
        let plain = query(album, request);
        let encoded = native::seal_catalog_query(plain.as_bytes())?;
        let target = format!(
            "{}?f=kuwo&q={encoded}",
            self.web_target("https://searchlist.kuwo.cn/r.s")
        );
        let started = Instant::now();
        let mut status = None;
        let result = tokio::time::timeout(Duration::from_secs(30), async {
            let response = self
                .http
                .get(target)
                .header(ACCEPT, "application/octet-stream")
                .send()
                .await
                .map_err(kuwo_network_error)?;
            status = Some(response.status());
            if !response.status().is_success() {
                return Err(kuwo_http_error(response.status()));
            }
            let mime = response
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.split(';').next());
            if !mime.is_some_and(|v| v.trim().eq_ignore_ascii_case("application/octet-stream")) {
                return Err(invalid());
            }
            let bytes =
                read_bounded_response_with_limit(response, "Kuwo podcast episodes", LIMIT as u64)
                    .await?;
            parse_page(&decode_frame(&bytes)?, album, request)
        })
        .await
        .unwrap_or_else(|_| {
            Err(TuneWeaveError::new(
                ErrorCode::UpstreamTimeout,
                "Kuwo podcast episodes exceeded the total time budget",
            )
            .with_platform(Platform::Kuwo))
        });
        self.log_upstream_request(
            "podcast_episode_list",
            "searchlist.kuwo.cn",
            "/r.s",
            status,
            started,
            0,
            false,
            &result,
        );
        result
    }
}

fn query(album: &str, request: &PodcastEpisodeListRequest) -> String {
    let order = if request.ascending { 2 } else { 1 };
    format!(
        "type=music_list&uid=0&vipsec=1&id={album}&key=album&presell=1&apiv=2&order={order}&epaor=1&rformat=json&start={}&count={}&hasmv=1&hasinner=1",
        request.offset, request.limit
    )
}

fn decode_frame(bytes: &[u8]) -> Result<Vec<u8>> {
    if bytes.len() < 14 || bytes.len() > LIMIT || !bytes.starts_with(b"sig=\r\n") {
        return Err(invalid());
    }
    let compressed = u32::from_le_bytes(bytes[6..10].try_into().map_err(|_| invalid())?) as usize;
    let expanded = u32::from_le_bytes(bytes[10..14].try_into().map_err(|_| invalid())?) as usize;
    if compressed == 0 || expanded == 0 || expanded > LIMIT {
        return Err(invalid());
    }
    let payload = bytes
        .get(14..14_usize.checked_add(compressed).ok_or_else(invalid)?)
        .ok_or_else(invalid)?;
    let mut decoder = flate2::bufread::ZlibDecoder::new(payload);
    let mut plain = Vec::new();
    decoder
        .by_ref()
        .take((expanded + 1) as u64)
        .read_to_end(&mut plain)
        .map_err(|_| invalid())?;
    if plain.len() != expanded || decoder.total_in() != compressed as u64 {
        return Err(invalid());
    }
    // The live server pads its buffer after the declared compressed segment.
    // These bytes are neither JSON nor metadata and must never reach the model.
    Ok(plain)
}

#[derive(Deserialize)]
struct EpisodePage {
    albumid: Unsigned,
    count: Unsigned,
    start: Unsigned,
    total: Unsigned,
    order: Unsigned,
    #[serde(rename = "type")]
    kind: String,
    musiclist: Vec<Episode>,
}

#[derive(Deserialize)]
struct Episode {
    musicrid: Unsigned,
    albumid: Unsigned,
    isstar: Unsigned,
    content_type: Unsigned,
    name: String,
    duration: Unsigned,
    track: Unsigned,
    artist: String,
    artistid: Unsigned,
    img: Option<String>,
    releasedate: Option<String>,
}

fn parse_page(
    bytes: &[u8],
    album: &str,
    request: &PodcastEpisodeListRequest,
) -> Result<Page<PodcastEpisode>> {
    let raw: EpisodePage = serde_json::from_slice(bytes).map_err(|_| invalid())?;
    let total = raw.total.value()?;
    if raw.albumid.id()? != album
        || raw.kind != "music"
        || raw.start.value()? != u64::from(request.offset)
        || raw.order.value()? != if request.ascending { 2 } else { 1 }
        || raw.count.value()? != raw.musiclist.len() as u64
        || raw.musiclist.len() > request.limit as usize
        || (u64::from(request.offset) < total
            && (raw.musiclist.is_empty()
                || raw.musiclist.len() as u64 > total - u64::from(request.offset)))
    {
        return Err(invalid());
    }
    let mut items = Vec::new();
    let mut ids = BTreeSet::new();
    let mut previous = None;
    for row in raw.musiclist {
        let id = row.musicrid.id()?;
        let serial = row.track.value()?;
        if row.albumid.id()? != album
            || row.isstar.value()? != 1
            || row.content_type.value()? != 0
            || !ids.insert(id.clone())
            || serial == 0
            || previous.is_some_and(|last| {
                if request.ascending {
                    serial <= last
                } else {
                    serial >= last
                }
            })
        {
            return Err(invalid());
        }
        previous = Some(serial);
        text(&row.name, 2048, false)?;
        text(&row.artist, 2048, false)?;
        if row.name.trim().is_empty() {
            return Err(invalid());
        }
        let reference =
            ResourceRef::new(Platform::Kuwo, format!("episode:{id}")).map_err(|_| invalid())?;
        let mut episode = PodcastEpisode::new(reference, row.name);
        episode.podcast_ref = Some(
            ResourceRef::new(Platform::Kuwo, format!("anchor:{album}")).map_err(|_| invalid())?,
        );
        episode.duration_ms = Some(
            row.duration
                .value()?
                .checked_mul(1000)
                .ok_or_else(invalid)?,
        );
        episode.serial_number = Some(serial);
        episode.cover_url = image(row.img)?;
        episode.published_at = row
            .releasedate
            .as_deref()
            .map(catalog::date)
            .transpose()?
            .flatten();
        let artist_id = row.artistid.value()?;
        if !row.artist.trim().is_empty() {
            episode.creator = Some(CreatorSummary {
                name: row.artist,
                resource_ref: if artist_id == 0 {
                    None
                } else {
                    Some(
                        ResourceRef::new(Platform::Kuwo, artist_id.to_string())
                            .map_err(|_| invalid())?,
                    )
                },
                avatar_url: None,
            });
        }
        episode
            .extensions
            .insert("source_music_id".into(), json!(id));
        episode
            .extensions
            .insert("backend".into(), json!("native_anchor_album"));
        // No stream or entitlement can be inferred from this anonymous catalogue page.
        items.push(episode);
    }
    // Native pagination repeats the last programme for an offset at/past total.
    // Validate those rows, then represent the exhausted page without duplicates.
    if u64::from(request.offset) >= total {
        items.clear();
    }
    let end = request
        .offset
        .checked_add(items.len() as u32)
        .ok_or_else(invalid)?;
    let more = u64::from(end) < total;
    Ok(Page {
        items,
        pagination: PageMeta {
            limit: request.limit,
            offset: request.offset,
            total: Some(total),
            has_more: more,
            next_offset: more.then_some(end),
            extensions: Extensions::from([("backend".into(), json!("native_anchor_album"))]),
        },
    })
}

fn invalid() -> TuneWeaveError {
    kuwo_upstream_error("Kuwo podcast episodes returned an invalid response")
}
