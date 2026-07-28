use std::fmt;

use tuneweave_core::{Platform, ResourceRef, Result, TuneWeaveError};
use url::Url;

const MAX_INPUT_LENGTH: usize = 4096;
const MAX_NUMERIC_ID: u64 = (1_u64 << 51) - 1;
const BVID_ALPHABET: &str = "FcwAPNKTMug3GV5Lj7EJnHpWsx4tb8haYeviqBz6rkCy12mUSDQX9RdoZf";

/// A validated Bilibili video identity before platform metadata resolution.
///
/// Archive identities retain the caller's AID or BVID form. Episode and season
/// identities remain distinct so later metadata and playback requests can use
/// the correct PGC endpoint without losing the original source identity.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum BilibiliVideoIdentity {
    Aid(u64),
    Bvid(String),
    Episode(u64),
    Season(u64),
}

impl BilibiliVideoIdentity {
    pub fn parse(input: &str) -> Result<Self> {
        let input = input.trim();
        if input.is_empty() {
            return Err(invalid_identity(
                "Bilibili video identity must not be empty",
            ));
        }
        if input.len() > MAX_INPUT_LENGTH {
            return Err(invalid_identity("Bilibili video identity is too long"));
        }

        if input.contains("://") {
            return parse_video_url(input);
        }

        let input = input.strip_prefix("bilibili:").unwrap_or(input);
        parse_video_token(input)
    }

    #[must_use]
    pub fn canonical_id(&self) -> String {
        match self {
            Self::Aid(id) => format!("aid:{id}"),
            Self::Bvid(id) => format!("bvid:{id}"),
            Self::Episode(id) => format!("ep:{id}"),
            Self::Season(id) => format!("season:{id}"),
        }
    }

    pub fn resource_ref(&self) -> Result<ResourceRef> {
        ResourceRef::new(Platform::Bilibili, self.canonical_id()).map_err(|_| {
            TuneWeaveError::invalid_request("Bilibili video identity could not be normalized")
                .with_platform(Platform::Bilibili)
        })
    }
}

impl fmt::Display for BilibiliVideoIdentity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.canonical_id())
    }
}

fn parse_video_url(input: &str) -> Result<BilibiliVideoIdentity> {
    let url = Url::parse(input).map_err(|_| invalid_identity("Bilibili video URL is invalid"))?;
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
        || !matches!(
            url.host_str(),
            Some("bilibili.com" | "www.bilibili.com" | "m.bilibili.com")
        )
    {
        return Err(invalid_identity(
            "Bilibili video URL must use a trusted Bilibili web host",
        ));
    }

    let segments = url
        .path_segments()
        .map(|segments| {
            segments
                .filter(|segment| !segment.is_empty())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    if let Some(position) = segments.iter().position(|segment| *segment == "video")
        && let Some(token) = segments.get(position + 1)
    {
        return parse_archive_token(token);
    }
    if segments.len() >= 3 && segments[0] == "bangumi" && segments[1] == "play" {
        return parse_episode_token(segments[2]);
    }

    let mut identity = None;
    for (name, value) in url.query_pairs() {
        let parsed = match name.as_ref() {
            "aid" | "avid" => Some(BilibiliVideoIdentity::Aid(parse_numeric_id(&value, "AID")?)),
            "bvid" => Some(BilibiliVideoIdentity::Bvid(parse_bvid(&value)?)),
            "ep_id" => Some(BilibiliVideoIdentity::Episode(parse_numeric_id(
                &value,
                "episode ID",
            )?)),
            "season_id" => Some(BilibiliVideoIdentity::Season(parse_numeric_id(
                &value,
                "season ID",
            )?)),
            _ => None,
        };
        if let Some(parsed) = parsed {
            if identity.replace(parsed).is_some() {
                return Err(invalid_identity(
                    "Bilibili video URL contains conflicting identities",
                ));
            }
        }
    }
    identity.ok_or_else(|| invalid_identity("Bilibili video URL does not contain an identity"))
}

fn parse_video_token(input: &str) -> Result<BilibiliVideoIdentity> {
    if let Some((kind, value)) = input.split_once(':') {
        return match kind.to_ascii_lowercase().as_str() {
            "aid" | "av" => Ok(BilibiliVideoIdentity::Aid(parse_numeric_id(value, "AID")?)),
            "bvid" | "bv" => Ok(BilibiliVideoIdentity::Bvid(parse_bvid(value)?)),
            "ep" | "episode" => Ok(BilibiliVideoIdentity::Episode(parse_numeric_id(
                value,
                "episode ID",
            )?)),
            "ss" | "season" => Ok(BilibiliVideoIdentity::Season(parse_numeric_id(
                value,
                "season ID",
            )?)),
            _ => Err(invalid_identity("unsupported Bilibili video identity type")),
        };
    }
    if input
        .get(..2)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("av"))
    {
        return Ok(BilibiliVideoIdentity::Aid(parse_numeric_id(
            &input[2..],
            "AID",
        )?));
    }
    if input
        .get(..2)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("bv"))
    {
        return Ok(BilibiliVideoIdentity::Bvid(parse_bvid(input)?));
    }
    parse_episode_token(input)
}

fn parse_archive_token(input: &str) -> Result<BilibiliVideoIdentity> {
    if input
        .get(..2)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("av"))
    {
        return Ok(BilibiliVideoIdentity::Aid(parse_numeric_id(
            &input[2..],
            "AID",
        )?));
    }
    if input
        .get(..2)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("bv"))
    {
        return Ok(BilibiliVideoIdentity::Bvid(parse_bvid(input)?));
    }
    Err(invalid_identity(
        "Bilibili video URL must contain an AV or BV identity",
    ))
}

fn parse_episode_token(input: &str) -> Result<BilibiliVideoIdentity> {
    if input
        .get(..2)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("ep"))
    {
        return Ok(BilibiliVideoIdentity::Episode(parse_numeric_id(
            &input[2..],
            "episode ID",
        )?));
    }
    if input
        .get(..2)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("ss"))
    {
        return Ok(BilibiliVideoIdentity::Season(parse_numeric_id(
            &input[2..],
            "season ID",
        )?));
    }
    Err(invalid_identity(
        "Bilibili video identity must be AV, BV, EP, or SS",
    ))
}

fn parse_bvid(input: &str) -> Result<String> {
    let bytes = input.as_bytes();
    if bytes.len() != 12
        || !input.is_ascii()
        || !input[..2].eq_ignore_ascii_case("bv")
        || bytes[2] != b'1'
        || !input[3..]
            .chars()
            .all(|character| BVID_ALPHABET.contains(character))
    {
        return Err(invalid_identity("Bilibili BVID is invalid"));
    }
    Ok(format!("BV1{}", &input[3..]))
}

fn parse_numeric_id(input: &str, name: &str) -> Result<u64> {
    if input.is_empty() || !input.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(invalid_identity(format!("Bilibili {name} is invalid")));
    }
    let value = input
        .parse::<u64>()
        .map_err(|_| invalid_identity(format!("Bilibili {name} is invalid")))?;
    if value == 0 || value > MAX_NUMERIC_ID {
        return Err(invalid_identity(format!("Bilibili {name} is out of range")));
    }
    Ok(value)
}

fn invalid_identity(message: impl Into<String>) -> TuneWeaveError {
    TuneWeaveError::invalid_request(message).with_platform(Platform::Bilibili)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn direct_identities_keep_archive_and_episode_kinds_distinct() {
        assert_eq!(
            BilibiliVideoIdentity::parse("av170001").expect("AID"),
            BilibiliVideoIdentity::Aid(170001)
        );
        assert_eq!(
            BilibiliVideoIdentity::parse("bv1Q541167Qg").expect("BVID"),
            BilibiliVideoIdentity::Bvid("BV1Q541167Qg".to_owned())
        );
        assert_eq!(
            BilibiliVideoIdentity::parse("ep123").expect("episode"),
            BilibiliVideoIdentity::Episode(123)
        );
        assert_eq!(
            BilibiliVideoIdentity::parse("ss456").expect("season"),
            BilibiliVideoIdentity::Season(456)
        );
    }

    #[test]
    fn canonical_and_namespaced_identities_round_trip() {
        for (input, canonical) in [
            ("bilibili:aid:170001", "aid:170001"),
            ("bvid:BV1Q541167Qg", "bvid:BV1Q541167Qg"),
            ("episode:000123", "ep:123"),
            ("season:000456", "season:456"),
        ] {
            let identity = BilibiliVideoIdentity::parse(input).expect("valid identity");
            assert_eq!(identity.canonical_id(), canonical);
            assert_eq!(
                identity.resource_ref().expect("resource").to_string(),
                format!("bilibili:{canonical}")
            );
        }
    }

    #[test]
    fn trusted_video_urls_preserve_their_explicit_identity() {
        for (input, expected) in [
            (
                "https://www.bilibili.com/video/av170001?p=2",
                BilibiliVideoIdentity::Aid(170001),
            ),
            (
                "https://m.bilibili.com/video/BV1Q541167Qg",
                BilibiliVideoIdentity::Bvid("BV1Q541167Qg".to_owned()),
            ),
            (
                "https://www.bilibili.com/bangumi/play/ep123",
                BilibiliVideoIdentity::Episode(123),
            ),
            (
                "https://www.bilibili.com/bangumi/play/ss456",
                BilibiliVideoIdentity::Season(456),
            ),
            (
                "https://api.invalid.example/path?bvid=BV1Q541167Qg",
                BilibiliVideoIdentity::Bvid("BV1Q541167Qg".to_owned()),
            ),
        ] {
            if input.contains("invalid.example") {
                assert!(BilibiliVideoIdentity::parse(input).is_err());
            } else {
                assert_eq!(BilibiliVideoIdentity::parse(input).expect("URL"), expected);
            }
        }

        assert_eq!(
            BilibiliVideoIdentity::parse("https://www.bilibili.com/?ep_id=123")
                .expect("query identity"),
            BilibiliVideoIdentity::Episode(123)
        );
    }

    #[test]
    fn ambiguous_or_untrusted_inputs_are_rejected_without_network_access() {
        for input in [
            "123",
            "av0",
            "av-1",
            "BV1invalid000",
            "ep",
            "ss0",
            "qq:001",
            "https://b23.tv/short",
            "https://evil.example/video/BV1Q541167Qg",
            "https://user@www.bilibili.com/video/av170001",
            "https://www.bilibili.com:444/video/av170001",
            "https://www.bilibili.com/?aid=170001&ep_id=123",
            "https://space.bilibili.com/1/lists/2?type=season",
        ] {
            assert!(
                BilibiliVideoIdentity::parse(input).is_err(),
                "unexpectedly accepted {input}"
            );
        }
    }
}
