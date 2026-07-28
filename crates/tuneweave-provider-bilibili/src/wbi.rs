use std::collections::BTreeMap;

use md5::{Digest, Md5};
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use tuneweave_core::{Platform, Result, TuneWeaveError};
use url::Url;

const MIXIN_ORDER: [usize; 64] = [
    46, 47, 18, 2, 53, 8, 23, 32, 15, 50, 10, 31, 58, 3, 45, 35, 27, 43, 5, 49, 33, 9, 42, 19, 29,
    28, 14, 39, 12, 38, 41, 13, 37, 48, 7, 16, 24, 55, 40, 61, 26, 17, 0, 1, 60, 51, 30, 4, 22, 25,
    54, 21, 56, 59, 6, 63, 57, 62, 11, 36, 20, 34, 44, 52,
];
const WBI_COMPONENT: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'~');
const MAX_PARAMETER_COUNT: usize = 64;
const MAX_PARAMETER_LENGTH: usize = 4096;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct WbiKeys {
    mixin: String,
}

impl WbiKeys {
    pub(crate) fn from_image_urls(image_url: &str, sub_url: &str) -> Result<Self> {
        let image = wbi_key_from_url(image_url, "image")?;
        let sub = wbi_key_from_url(sub_url, "sub")?;
        let raw = format!("{image}{sub}");
        let bytes = raw.as_bytes();
        let mixin = MIXIN_ORDER
            .iter()
            .take(32)
            .map(|position| char::from(bytes[*position]))
            .collect();
        Ok(Self { mixin })
    }

    pub(crate) fn sign(&self, parameters: &[(String, String)], timestamp: u64) -> Result<String> {
        if timestamp == 0 {
            return Err(invalid_wbi("Bilibili WBI timestamp is invalid"));
        }
        if parameters.len() > MAX_PARAMETER_COUNT {
            return Err(invalid_wbi("Bilibili WBI request has too many parameters"));
        }

        let mut sorted = BTreeMap::new();
        for (name, value) in parameters {
            validate_parameter(name, value)?;
            if matches!(name.as_str(), "w_rid" | "wts")
                || sorted.insert(name.clone(), value.clone()).is_some()
            {
                return Err(invalid_wbi(
                    "Bilibili WBI request contains duplicate or reserved parameters",
                ));
            }
        }
        let timestamp = timestamp.to_string();
        sorted.insert("wts".to_owned(), timestamp);

        let query = sorted
            .iter()
            .map(|(name, value)| {
                let filtered = value
                    .chars()
                    .filter(|character| !matches!(character, '!' | '\'' | '(' | ')' | '*'))
                    .collect::<String>();
                format!(
                    "{}={}",
                    utf8_percent_encode(name, WBI_COMPONENT),
                    utf8_percent_encode(&filtered, WBI_COMPONENT)
                )
            })
            .collect::<Vec<_>>()
            .join("&");
        let signature = hex::encode(Md5::digest(format!("{query}{}", self.mixin).as_bytes()));
        Ok(format!("{query}&w_rid={signature}"))
    }
}

fn wbi_key_from_url(value: &str, kind: &str) -> Result<String> {
    let url = Url::parse(value)
        .map_err(|_| invalid_wbi(format!("Bilibili WBI {kind} URL is invalid")))?;
    let trusted_host = url
        .host_str()
        .is_some_and(|host| host == "hdslb.com" || host.ends_with(".hdslb.com"));
    if url.scheme() != "https"
        || url.port().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
        || !trusted_host
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(invalid_wbi(format!("Bilibili WBI {kind} URL is untrusted")));
    }
    let mut segments = url
        .path_segments()
        .ok_or_else(|| invalid_wbi(format!("Bilibili WBI {kind} URL does not contain a key")))?;
    if segments.next() != Some("bfs") || segments.next() != Some("wbi") {
        return Err(invalid_wbi(format!(
            "Bilibili WBI {kind} URL has an invalid path"
        )));
    }
    let file = segments
        .next()
        .filter(|_| segments.next().is_none())
        .ok_or_else(|| invalid_wbi(format!("Bilibili WBI {kind} URL does not contain a key")))?;
    let key = file
        .strip_suffix(".png")
        .ok_or_else(|| invalid_wbi(format!("Bilibili WBI {kind} URL has an invalid key")))?;
    if key.len() != 32 || !key.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(invalid_wbi(format!(
            "Bilibili WBI {kind} URL has an invalid key"
        )));
    }
    Ok(key.to_ascii_lowercase())
}

fn validate_parameter(name: &str, value: &str) -> Result<()> {
    if name.is_empty()
        || name.len() > 128
        || value.len() > MAX_PARAMETER_LENGTH
        || name
            .bytes()
            .any(|byte| !byte.is_ascii_alphanumeric() && byte != b'_')
        || value.chars().any(char::is_control)
    {
        return Err(invalid_wbi("Bilibili WBI request parameter is invalid"));
    }
    Ok(())
}

fn invalid_wbi(message: impl Into<String>) -> TuneWeaveError {
    TuneWeaveError::invalid_request(message).with_platform(Platform::Bilibili)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_keys() -> WbiKeys {
        WbiKeys::from_image_urls(
            "https://i0.hdslb.com/bfs/wbi/7cd084941338484aae1ad9425b84077c.png",
            "https://i0.hdslb.com/bfs/wbi/4932caff0ff746eab6f01bf08b70ac45.png",
        )
        .expect("keys")
    }

    #[test]
    fn official_mixin_and_signature_vector_matches() {
        let keys = fixture_keys();
        assert_eq!(keys.mixin, "ea1db124af3c7062474693fa704f4ff8");
        assert_eq!(
            keys.sign(
                &[
                    ("foo".to_owned(), "114".to_owned()),
                    ("bar".to_owned(), "514".to_owned()),
                    ("zab".to_owned(), "1919810".to_owned()),
                ],
                1_702_204_169,
            )
            .expect("signature"),
            "bar=514&foo=114&wts=1702204169&zab=1919810&w_rid=8f6f2b5b3d485fe1886cec6a0be8c5d4"
        );
    }

    #[test]
    fn unicode_spaces_and_filtered_characters_use_web_encoding() {
        let query = fixture_keys()
            .sign(
                &[
                    ("foo".to_owned(), "one one four".to_owned()),
                    ("bar".to_owned(), "五一四!'()*".to_owned()),
                ],
                1_702_204_169,
            )
            .expect("signature");
        assert!(
            query
                .starts_with("bar=%E4%BA%94%E4%B8%80%E5%9B%9B&foo=one%20one%20four&wts=1702204169")
        );
        assert!(!query.contains('+'));
        assert!(!query.contains("%e4"));
    }

    #[test]
    fn key_urls_and_request_parameters_are_strictly_bounded() {
        for (image, sub) in [
            (
                "http://i0.hdslb.com/bfs/wbi/7cd084941338484aae1ad9425b84077c.png",
                "https://i0.hdslb.com/bfs/wbi/4932caff0ff746eab6f01bf08b70ac45.png",
            ),
            (
                "https://evil.example/bfs/wbi/7cd084941338484aae1ad9425b84077c.png",
                "https://i0.hdslb.com/bfs/wbi/4932caff0ff746eab6f01bf08b70ac45.png",
            ),
            (
                "https://i0.hdslb.com/bfs/wbi/short.png",
                "https://i0.hdslb.com/bfs/wbi/4932caff0ff746eab6f01bf08b70ac45.png",
            ),
        ] {
            assert!(WbiKeys::from_image_urls(image, sub).is_err());
        }

        let keys = fixture_keys();
        assert!(keys.sign(&[("wts".to_owned(), "1".to_owned())], 1).is_err());
        assert!(
            keys.sign(
                &[
                    ("same".to_owned(), "1".to_owned()),
                    ("same".to_owned(), "2".to_owned()),
                ],
                1,
            )
            .is_err()
        );
    }
}
