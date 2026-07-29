use std::fmt;

use tuneweave_core::{Platform, ResourceRef, Result, TuneWeaveError};
use url::Url;

const MAX_IDENTITY_INPUT_BYTES: usize = 4_096;
const MAX_SHORT_CODE_BYTES: usize = 128;

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SodaTrackIdentity {
    id: String,
}

impl SodaTrackIdentity {
    pub fn parse(input: &str) -> Result<Self> {
        match classify_track_identity(input)? {
            SodaTrackIdentityInput::Direct(identity) => Ok(identity),
            SodaTrackIdentityInput::ShortLink(_) => Err(invalid_identity(
                "Soda short links must be resolved through SodaClient",
            )),
        }
    }

    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    #[must_use]
    pub fn canonical_url(&self) -> String {
        format!("https://www.qishui.com/track/{}", self.id)
    }

    pub fn resource_ref(&self) -> Result<ResourceRef> {
        ResourceRef::new(Platform::Soda, &self.id).map_err(|_| {
            invalid_identity("Soda track identity could not be converted to a resource reference")
        })
    }

    fn from_id(id: &str) -> Result<Self> {
        let id = canonical_track_id(id).ok_or_else(|| {
            invalid_identity("Soda track id must be a canonical positive integer")
        })?;
        Ok(Self { id: id.to_owned() })
    }
}

impl fmt::Display for SodaTrackIdentity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.id)
    }
}

pub(crate) enum SodaTrackIdentityInput {
    Direct(SodaTrackIdentity),
    ShortLink(Url),
}

pub(crate) fn classify_track_identity(input: &str) -> Result<SodaTrackIdentityInput> {
    let input = input.trim();
    if input.is_empty()
        || input.len() > MAX_IDENTITY_INPUT_BYTES
        || input.chars().any(char::is_control)
    {
        return Err(invalid_identity(
            "Soda track identity must contain a bounded printable value",
        ));
    }

    if let Some(id) = input.strip_prefix("soda:") {
        return SodaTrackIdentity::from_id(id).map(SodaTrackIdentityInput::Direct);
    }
    if canonical_track_id(input).is_some() {
        return SodaTrackIdentity::from_id(input).map(SodaTrackIdentityInput::Direct);
    }

    let url = Url::parse(input)
        .map_err(|_| invalid_identity("Soda track identity is not a supported official URL"))?;
    validate_url_authority(&url)?;
    if is_short_link(&url) {
        return Ok(SodaTrackIdentityInput::ShortLink(url));
    }
    parse_direct_track_url(&url).map(SodaTrackIdentityInput::Direct)
}

pub(crate) fn parse_short_redirect_location(value: &str) -> Result<SodaTrackIdentity> {
    if value.is_empty() || value.len() > MAX_IDENTITY_INPUT_BYTES {
        return Err(invalid_identity(
            "Soda short link returned an invalid redirect location",
        ));
    }
    let url = Url::parse(value)
        .map_err(|_| invalid_identity("Soda short link returned an invalid redirect location"))?;
    validate_url_authority(&url)?;
    if is_short_link(&url) {
        return Err(invalid_identity(
            "Soda short link returned another short link",
        ));
    }
    parse_direct_track_url(&url)
}

fn validate_url_authority(url: &Url) -> Result<()> {
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || !matches!(url.port(), None | Some(443))
        || url.fragment().is_some()
    {
        return Err(invalid_identity(
            "Soda track URL must use a trusted HTTPS origin",
        ));
    }
    Ok(())
}

fn is_short_link(url: &Url) -> bool {
    if url.host_str() != Some("qishui.douyin.com") || url.query().is_some() {
        return false;
    }
    let segments = nonempty_path_segments(url);
    segments.len() == 2
        && segments[0] == "s"
        && (4..=MAX_SHORT_CODE_BYTES).contains(&segments[1].len())
        && segments[1]
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

fn parse_direct_track_url(url: &Url) -> Result<SodaTrackIdentity> {
    let segments = nonempty_path_segments(url);
    match url.host_str() {
        Some("www.qishui.com") if segments.len() == 2 && segments[0] == "track" => {
            reject_identity_query(url)?;
            SodaTrackIdentity::from_id(segments[1])
        }
        Some("www.douyin.com")
            if segments.len() == 3 && segments[0] == "qishui" && segments[1] == "song" =>
        {
            reject_identity_query(url)?;
            SodaTrackIdentity::from_id(segments[2])
        }
        Some("music.douyin.com") if segments == ["qishui", "share", "track"] => {
            let mut track_ids = url
                .query_pairs()
                .filter(|(name, _)| name == "track_id")
                .map(|(_, value)| value.into_owned());
            let id = track_ids.next().ok_or_else(|| {
                invalid_identity("Soda share URL did not contain a track identity")
            })?;
            if track_ids.next().is_some() {
                return Err(invalid_identity(
                    "Soda share URL contained duplicate track identities",
                ));
            }
            SodaTrackIdentity::from_id(&id)
        }
        _ => Err(invalid_identity(
            "Soda track URL must use a supported official track path",
        )),
    }
}

fn reject_identity_query(url: &Url) -> Result<()> {
    if url
        .query_pairs()
        .any(|(name, _)| matches!(name.as_ref(), "track_id" | "id"))
    {
        return Err(invalid_identity(
            "Soda track URL contained a conflicting query identity",
        ));
    }
    Ok(())
}

fn nonempty_path_segments(url: &Url) -> Vec<&str> {
    url.path_segments()
        .map(|segments| segments.filter(|segment| !segment.is_empty()).collect())
        .unwrap_or_default()
}

fn canonical_track_id(value: &str) -> Option<&str> {
    let value = value.trim();
    (!value.is_empty()
        && !value.starts_with('0')
        && value.bytes().all(|byte| byte.is_ascii_digit())
        && value.parse::<u64>().is_ok_and(|value| value > 0))
    .then_some(value)
}

fn invalid_identity(message: impl Into<String>) -> TuneWeaveError {
    TuneWeaveError::invalid_request(message).with_platform(Platform::Soda)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TRACK_ID: &str = "7304719759323564095";

    #[test]
    fn direct_ids_and_official_track_urls_share_one_canonical_identity() {
        for input in [
            TRACK_ID.to_owned(),
            format!("soda:{TRACK_ID}"),
            format!("https://www.qishui.com/track/{TRACK_ID}"),
            format!("https://www.qishui.com/track/{TRACK_ID}/?from=share"),
            format!("https://www.douyin.com/qishui/song/{TRACK_ID}"),
            format!(
                "https://music.douyin.com/qishui/share/track?track_id={TRACK_ID}&auto_play_bgm=1"
            ),
        ] {
            let identity = SodaTrackIdentity::parse(&input).expect("valid Soda track identity");
            assert_eq!(identity.id(), TRACK_ID);
            assert_eq!(identity.to_string(), TRACK_ID);
            assert_eq!(
                identity
                    .resource_ref()
                    .expect("Soda resource reference")
                    .to_string(),
                format!("soda:{TRACK_ID}")
            );
            assert_eq!(
                identity.canonical_url(),
                format!("https://www.qishui.com/track/{TRACK_ID}")
            );
        }
    }

    #[test]
    fn short_links_require_the_bounded_client_redirect_resolver() {
        let input = classify_track_identity("https://qishui.douyin.com/s/iQeFw9cE/")
            .expect("valid Soda short link");
        assert!(matches!(input, SodaTrackIdentityInput::ShortLink(_)));
        assert!(SodaTrackIdentity::parse("https://qishui.douyin.com/s/iQeFw9cE/").is_err());
    }

    #[test]
    fn redirect_locations_accept_only_direct_official_track_destinations() {
        let location = format!(
            "https://music.douyin.com/qishui/share/track?track_id={TRACK_ID}&auto_play_bgm=1"
        );
        assert_eq!(
            parse_short_redirect_location(&location)
                .expect("valid redirect destination")
                .id(),
            TRACK_ID
        );
        for location in [
            "http://www.qishui.com/track/7304719759323564095",
            "https://evil.example/track/7304719759323564095",
            "https://qishui.douyin.com/s/another/",
            "//www.qishui.com/track/7304719759323564095",
        ] {
            assert!(
                parse_short_redirect_location(location).is_err(),
                "{location}"
            );
        }
    }

    #[test]
    fn ambiguous_malformed_and_untrusted_inputs_are_rejected() {
        for input in [
            "",
            "0",
            "07304719759323564095",
            "https://www.qishui.com.evil.example/track/7304719759323564095",
            "https://user@www.qishui.com/track/7304719759323564095",
            "https://www.qishui.com:444/track/7304719759323564095",
            "https://www.qishui.com/track/7304719759323564095/extra",
            "https://www.qishui.com/track/7304719759323564095?track_id=1",
            "https://music.douyin.com/qishui/share/track?track_id=7304719759323564095&track_id=1",
            "https://music.douyin.com/qishui/share/track?playlist_id=7304719759323564095",
            "https://qishui.douyin.com/s/a/",
            "https://qishui.douyin.com/s/iQeFw9cE/?next=https://evil.example",
        ] {
            assert!(classify_track_identity(input).is_err(), "{input}");
        }
    }
}
