use tuneweave_core::Artist;

use super::*;

const ARTIST_SHARE_ENDPOINT: &str = "https://www.qishui.com/share/artist";
const ARTIST_BACKEND: &str = "official_web_artist_share";

#[derive(Deserialize)]
pub(super) struct SodaArtistMetadata {
    id: String,
    name: String,
    count_tracks: Option<u64>,
    simple_display_name: Option<String>,
    full_display_name: Option<String>,
    #[serde(default)]
    url_avatar: SodaImage,
    artist_profile: Option<ArtistProfile>,
    stats: Option<ArtistStatistics>,
    #[serde(default, rename = "hasError")]
    has_error: bool,
}

#[derive(Deserialize)]
struct ArtistProfile {
    #[serde(default)]
    alias: Vec<String>,
    intro: Option<String>,
    name_translation: Option<String>,
    nationality: Option<String>,
    born: Option<ArtistBirth>,
    career: Option<ArtistCareer>,
}

#[derive(Deserialize)]
struct ArtistBirth {
    birth_date: Option<String>,
}
#[derive(Deserialize)]
struct ArtistCareer {
    #[serde(default)]
    occupations: Vec<String>,
}
#[derive(Deserialize)]
struct ArtistStatistics {
    count_collected: Option<u64>,
}

pub(crate) struct SodaArtistPage {
    pub artist: Artist,
    pub featured_tracks: Vec<Track>,
}

#[derive(Deserialize)]
struct ArtistRouter {
    errors: Option<serde_json::Value>,
    #[serde(rename = "loaderData")]
    loader_data: ArtistLoader,
}
#[derive(Deserialize)]
struct ArtistLoader {
    artist_page: ShareArtistPage,
}
#[derive(Deserialize)]
struct ShareArtistPage {
    #[serde(rename = "artistInfo")]
    artist_info: SodaArtistMetadata,
    #[serde(rename = "trackList")]
    track_list: Vec<SodaTrack>,
}

impl SodaClient {
    pub(crate) async fn artist_page(&self, id: &str) -> Result<SodaArtistPage> {
        let started = Instant::now();
        let mut http_status = None;
        let result = async {
            let url = Url::parse(ARTIST_SHARE_ENDPOINT)
                .map_err(|_| soda_upstream_error("Soda artist endpoint is invalid"))?;
            let response = self
                .login_request(reqwest::Method::GET, url)
                .query(&[("artist_id", id)])
                .send()
                .await
                .map_err(soda_network_error)?;
            http_status = Some(response.status());
            if !response.status().is_success() {
                return Err(soda_http_error(response.status()));
            }
            if response
                .headers()
                .get(CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .is_some_and(|value| !value.to_ascii_lowercase().starts_with("text/html"))
            {
                return Err(soda_upstream_error(
                    "Soda artist share page returned an unexpected content type",
                ));
            }
            let body = read_bounded_response(response, "Soda artist share page").await?;
            parse_artist_page(&body, id)
        }
        .await;
        self.log_upstream_request(
            "artist_page",
            "www.qishui.com",
            "/share/artist",
            http_status,
            started,
            &result,
        );
        result
    }
}

fn parse_artist_page(body: &[u8], expected_id: &str) -> Result<SodaArtistPage> {
    let envelope: ArtistRouter = serde_json::from_slice(extract_router_json(body)?)
        .map_err(|_| soda_upstream_error("Soda artist share page contained invalid data"))?;
    if envelope.errors.is_some_and(|error| {
        !error.is_null() && !error.as_object().is_some_and(|error| error.is_empty())
    }) {
        return Err(soda_upstream_error(
            "Soda artist share page reported a loader failure",
        ));
    }
    let page = envelope.loader_data.artist_page;
    let artist = map_artist_metadata(page.artist_info, ARTIST_BACKEND)?;
    if artist.id != expected_id {
        return Err(soda_upstream_error(
            "Soda artist share page returned a different artist",
        ));
    }
    if page.track_list.len() > 100
        || artist
            .track_count
            .is_some_and(|count| count < page.track_list.len() as u64)
    {
        return Err(soda_upstream_error(
            "Soda artist share page returned an inconsistent track preview",
        ));
    }
    let mut seen = std::collections::BTreeSet::new();
    let featured_tracks = page
        .track_list
        .into_iter()
        .map(|source| {
            if !source.artists.iter().any(|artist| artist.id == expected_id)
                || !seen.insert(source.id.clone())
            {
                return Err(soda_upstream_error(
                    "Soda artist preview repeated a track or contained a different artist",
                ));
            }
            map_track(source, ARTIST_BACKEND)
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(SodaArtistPage {
        artist,
        featured_tracks,
    })
}

fn valid_text(text: &str, limit: usize) -> bool {
    text.len() <= limit
        && !text
            .chars()
            .any(|ch| ch.is_control() && ch != '\n' && ch != '\t')
}

pub(super) fn map_artist_metadata(
    source: SodaArtistMetadata,
    backend: &'static str,
) -> Result<Artist> {
    if source.has_error {
        return Err(TuneWeaveError::new(
            ErrorCode::ResourceNotFound,
            "Soda artist was not found or is not public",
        )
        .with_platform(Platform::Soda));
    }
    if source.id.len() > 64
        || canonical_positive_decimal(&source.id).is_none()
        || source.name.trim().is_empty()
        || !valid_text(&source.name, 1_000)
        || source.count_tracks.is_some_and(|count| count > 1_000_000)
        || [&source.simple_display_name, &source.full_display_name]
            .into_iter()
            .flatten()
            .any(|name| !valid_text(name, 2_000))
    {
        return Err(soda_upstream_error("Soda returned invalid artist metadata"));
    }
    let mut extensions = Extensions::from([("backend".to_owned(), json!(backend))]);
    for (key, name) in [
        ("simple_display_name", source.simple_display_name),
        ("full_display_name", source.full_display_name),
    ] {
        if let Some(name) = name.filter(|name| !name.trim().is_empty()) {
            extensions.insert(key.to_owned(), json!(name));
        }
    }
    if let Some(count) = source.stats.and_then(|stats| stats.count_collected) {
        extensions.insert("stats".to_owned(), json!({"count_collected":count}));
    }
    let mut aliases = Vec::new();
    let mut description = String::new();
    let mut identities = Vec::new();
    if let Some(profile) = source.artist_profile {
        if profile.alias.len() > 64
            || profile.alias.iter().any(|name| !valid_text(name, 1_000))
            || profile
                .intro
                .as_deref()
                .is_some_and(|text| !valid_text(text, 64 * 1024))
        {
            return Err(soda_upstream_error(
                "Soda artist biography exceeds supported bounds",
            ));
        }
        aliases = profile
            .alias
            .into_iter()
            .filter(|name| !name.trim().is_empty())
            .collect();
        description = profile.intro.unwrap_or_default();
        for (key, value, bound) in [
            ("name_translation", profile.name_translation, 1_000),
            ("nationality", profile.nationality, 128),
            (
                "birth_date",
                profile.born.and_then(|born| born.birth_date),
                128,
            ),
        ] {
            if let Some(value) = value {
                if !valid_text(&value, bound) {
                    return Err(soda_upstream_error(
                        "Soda artist biography field is invalid",
                    ));
                }
                if !value.trim().is_empty() {
                    extensions.insert(key.to_owned(), json!(value));
                }
            }
        }
        if let Some(career) = profile.career {
            if career.occupations.len() > 64
                || career.occupations.iter().any(|role| !valid_text(role, 256))
            {
                return Err(soda_upstream_error(
                    "Soda artist occupations exceed supported bounds",
                ));
            }
            identities = career
                .occupations
                .into_iter()
                .filter(|role| !role.trim().is_empty())
                .collect();
        }
    }
    Ok(Artist {
        resource_ref: ResourceRef::new(Platform::Soda, &source.id)
            .map_err(|_| soda_upstream_error("Soda artist identity is invalid"))?,
        platform: Platform::Soda,
        id: source.id,
        name: source.name,
        aliases,
        description,
        biography_sections: Vec::new(),
        avatar_url: normalize_image(&source.url_avatar),
        cover_url: None,
        album_count: None,
        track_count: source.count_tracks,
        mv_count: None,
        video_count: None,
        identities,
        extensions,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> serde_json::Value {
        json!({"errors":null,"loaderData":{"artist_page":{
            "artistInfo":{"id":"123","name":"Artist","count_tracks":9,"url_avatar":{"uri":"avatar","urls":["https://p3-luna.douyinpic.com/img/"]},"stats":{"count_collected":42},"user":{"id":"different-user","token":"not-public"},"artist_profile":{"alias":["Alias"],"intro":"An introduction with {braces}","name_translation":"Translated name","nationality":"Region","born":{"birth_date":"2000-01-01"},"career":{"occupations":["Singer"]}}},
            "trackList":[{"id":"456","name":"Featured","duration":10000,"artists":[{"id":"789","name":"Collaborator"},{"id":"123","name":"Artist"}]}],
            "metaData":{"headers":{"cookie":"not-public"}}
        }}})
    }

    fn html(value: &serde_json::Value) -> Vec<u8> {
        format!("<html><script>window._ROUTER_DATA = {value};</script></html>").into_bytes()
    }

    #[test]
    fn artist_share_maps_biography_without_conflating_artist_and_user_identities() {
        let page = parse_artist_page(&html(&fixture()), "123").unwrap();
        assert_eq!(page.artist.resource_ref.to_string(), "soda:123");
        assert_eq!(page.artist.aliases, ["Alias"]);
        assert_eq!(page.artist.description, "An introduction with {braces}");
        assert_eq!(page.artist.identities, ["Singer"]);
        assert_eq!(page.artist.track_count, Some(9));
        assert!(page.artist.album_count.is_none());
        assert!(page.artist.mv_count.is_none());
        assert_eq!(page.artist.extensions["birth_date"], "2000-01-01");
        assert_eq!(page.artist.extensions["stats"]["count_collected"], 42);
        assert_eq!(page.artist.extensions["backend"], ARTIST_BACKEND);
        assert_eq!(page.featured_tracks.len(), 1);
        assert_eq!(page.featured_tracks[0].resource_ref.to_string(), "soda:456");
        assert!(
            !serde_json::to_string(&page.artist)
                .unwrap()
                .contains("not-public")
        );
    }

    #[test]
    fn artist_share_rejects_identity_drift_incomplete_loaders_and_malformed_previews() {
        assert!(parse_artist_page(&html(&fixture()), "321").is_err());
        for (pointer, replacement) in [
            ("/errors", json!({"artist_page":{"error":"failed"}})),
            ("/loaderData/artist_page/artistInfo/id", json!("0123")),
            ("/loaderData/artist_page/artistInfo/name", json!("")),
            ("/loaderData/artist_page/artistInfo/count_tracks", json!(0)),
            (
                "/loaderData/artist_page/trackList/0/artists",
                json!([{"id":"789","name":"Other"}]),
            ),
            (
                "/loaderData/artist_page/artistInfo/artist_profile/intro",
                json!("x".repeat(65537)),
            ),
            (
                "/loaderData/artist_page/artistInfo/artist_profile/alias",
                json!(["bad\u{0000}alias"]),
            ),
        ] {
            let mut value = fixture();
            *value.pointer_mut(pointer).unwrap() = replacement;
            assert!(
                parse_artist_page(&html(&value), "123").is_err(),
                "accepted {pointer}"
            );
        }
        let mut value = fixture();
        let track = value["loaderData"]["artist_page"]["trackList"][0].clone();
        value["loaderData"]["artist_page"]["trackList"]
            .as_array_mut()
            .unwrap()
            .push(track);
        assert!(parse_artist_page(&html(&value), "123").is_err());
        assert!(parse_artist_page(&html(&json!({"loaderData":{}})), "123").is_err());
        assert!(parse_artist_page(b"<html>no router state</html>", "123").is_err());
    }

    #[test]
    fn artist_search_metadata_keeps_unknown_biography_and_counts_unknown() {
        let source = serde_json::from_value(json!({"id":"123","name":"Artist","simple_display_name":"Short","full_display_name":"Long"})).unwrap();
        let artist = map_artist_metadata(source, "official_android_artist_search").unwrap();
        assert!(artist.aliases.is_empty());
        assert!(artist.description.is_empty());
        assert!(artist.track_count.is_none());
        assert!(artist.identities.is_empty());
        assert_eq!(artist.extensions["full_display_name"], "Long");
    }
}
