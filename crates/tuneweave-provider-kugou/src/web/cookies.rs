use reqwest::header::{HeaderMap, HeaderValue, SET_COOKIE};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use tuneweave_core::{ErrorCode, Result};

use super::{HOST, PATH, error, malformed};
use crate::{
    client::normalize_image_url,
    credential::{valid_secret, valid_uid},
};

/// Only the official login cookie is retained, including its actual HTTP scope and expiry.
#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WebCookie {
    value: String,
    domain: String,
    path: String,
    expires: Option<u64>,
}
impl std::fmt::Debug for WebCookie {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("WebCookie { fields: [redacted] }")
    }
}
pub(super) struct Identity {
    pub user_id: String,
    pub token: String,
    pub nickname: Option<String>,
    pub avatar_url: Option<String>,
}

impl WebCookie {
    pub(super) fn from_sms_fields(
        name: &str,
        domain: &str,
        path: &str,
        value: &str,
        now: u64,
    ) -> Result<Self> {
        // The official H5 writes these JSON fields as a one-day domain cookie.
        // Restrict it to the scope available to both the page and exchange host.
        if name != "KuGoo" || !matches!(domain, "kugou.com" | ".kugou.com") || path != "/" {
            return Err(malformed());
        }
        let cookie = Self {
            value: value.to_owned(),
            domain: "kugou.com".into(),
            path: "/".into(),
            expires: Some(now.checked_add(86400).ok_or_else(malformed)?),
        };
        cookie.identity()?;
        Ok(cookie)
    }

    pub(super) fn media_token(&self, now: u64) -> Result<String> {
        // getBaseInfo reads KuGoo on www.kugou.com, then explicitly signs its token
        // for wwwapi. A login-host-only cookie is not available to that browser flow.
        let _ = self.header(now)?;
        if self.domain != "kugou.com" || self.path != "/" {
            return Err(error(
                ErrorCode::CapabilityNotSupported,
                "KuGou Web cookie scope does not allow the official player",
            ));
        }
        Ok(self.identity()?.token)
    }

    pub(super) fn received(headers: &HeaderMap, now: u64) -> Result<Self> {
        let mut selected = None;
        let values = headers.get_all(SET_COOKIE);
        if values.iter().count() > 64 {
            return Err(malformed());
        }
        for header in values {
            let raw = header.to_str().map_err(|_| malformed())?;
            if raw.len() > 20_480 {
                return Err(malformed());
            }
            let mut parts = raw.split(';');
            let Some((name, value)) = parts.next().and_then(|v| v.trim().split_once('=')) else {
                continue;
            };
            if name != "KuGoo" {
                continue;
            }
            let mut domain = HOST.to_owned();
            let mut path = "/v1".to_owned();
            let mut expires = None;
            let mut max_age = None;
            let mut attrs = BTreeSet::new();
            for attr in parts {
                let (name, value) = attr.trim().split_once('=').unwrap_or((attr.trim(), ""));
                let name = name.to_ascii_lowercase();
                if matches!(name.as_str(), "domain" | "path" | "expires" | "max-age")
                    && !attrs.insert(name.clone())
                {
                    return Err(malformed());
                }
                match name.as_str() {
                    "domain" => {
                        domain = value
                            .strip_prefix('.')
                            .unwrap_or(value)
                            .to_ascii_lowercase()
                    }
                    "path" => {
                        if value.starts_with('/') {
                            path = value.to_owned();
                        }
                    }
                    "expires" => {
                        let time = httpdate::parse_http_date(value).map_err(|_| malformed())?;
                        expires = Some(
                            time.duration_since(std::time::UNIX_EPOCH)
                                .map_or(0, |v| v.as_secs()),
                        );
                    }
                    "max-age" => max_age = Some(value.parse::<i64>().map_err(|_| malformed())?),
                    _ => {}
                }
            }
            if !valid_scope(&domain, &path) {
                return Err(malformed());
            }
            if selected.is_some() {
                return Err(malformed());
            }
            if let Some(age) = max_age {
                expires = Some(if age <= 0 {
                    0
                } else {
                    now.checked_add(age as u64).ok_or_else(malformed)?
                });
            }
            if expires.is_some_and(|v| v <= now) || value.is_empty() {
                return Err(error(
                    ErrorCode::AuthenticationRequired,
                    "KuGou Web login cookie was removed or expired",
                ));
            }
            let cookie = Self {
                value: value.to_owned(),
                domain,
                path,
                expires,
            };
            cookie.identity()?;
            selected = Some(cookie);
        }
        selected.ok_or_else(malformed)
    }

    pub(super) fn identity(&self) -> Result<Identity> {
        if !valid_scope(&self.domain, &self.path)
            || self.value.is_empty()
            || self.value.len() > 16_384
            || !self
                .value
                .bytes()
                .all(|b| matches!(b, 0x21 | 0x23..=0x2b | 0x2d..=0x3a | 0x3c..=0x5b | 0x5d..=0x7e))
        {
            return Err(malformed());
        }
        let mut fields = BTreeMap::new();
        // The official reader splits the raw cookie on &, before decoding individual values.
        for part in self.value.split('&') {
            if part.is_empty() {
                continue;
            }
            let (key, value) = part.split_once('=').ok_or_else(malformed)?;
            if key.is_empty()
                || key.len() > 64
                || !key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
                || fields.insert(key, value).is_some()
                || fields.len() > 64
            {
                return Err(malformed());
            }
        }
        let user_id = fields.get("KugooID").ok_or_else(malformed)?.to_string();
        let token = fields.get("t").ok_or_else(malformed)?.to_string();
        if !valid_uid(&user_id) || !valid_secret(&token) {
            return Err(malformed());
        }
        if fields.get("a_id").is_some_and(|v| *v != "1014") {
            return Err(malformed());
        }
        if fields.get("ct").is_some_and(|v| !valid_uid(v)) {
            return Err(malformed());
        }
        let nickname = fields
            .get("NickName")
            .map(|v| text(v, 512))
            .transpose()?
            .flatten();
        if let Some(value) = fields.get("UserName") {
            text(value, 512)?;
        }
        let avatar_url = fields
            .get("Pic")
            .map(|v| text(v, 4096))
            .transpose()?
            .flatten()
            .map(|value| {
                if let Some(url) = normalize_image_url(&value) {
                    return Ok(url);
                }
                // The current official header also receives a dated image filename.
                if value.len() > 8
                    && value.as_bytes()[..8].iter().all(u8::is_ascii_digit)
                    && value
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
                {
                    return Ok(format!(
                        "https://imge.kugou.com/kugouicon/165/{}/{value}",
                        &value[..8]
                    ));
                }
                Err(malformed())
            })
            .transpose()?;
        Ok(Identity {
            user_id,
            token,
            nickname,
            avatar_url,
        })
    }

    pub(super) fn header(&self, now: u64) -> Result<HeaderValue> {
        self.identity()?;
        if self.expires.is_some_and(|v| v <= now) {
            return Err(error(
                ErrorCode::AuthenticationRequired,
                "KuGou Web session cookie expired",
            ));
        }
        let mut header =
            HeaderValue::from_str(&format!("KuGoo={}", self.value)).map_err(|_| malformed())?;
        header.set_sensitive(true);
        Ok(header)
    }
}

fn valid_scope(domain: &str, path: &str) -> bool {
    matches!(domain, HOST | "kugou.com")
        && path.len() <= 128
        && path.starts_with('/')
        && path.bytes().all(|b| b.is_ascii_graphic())
        && (PATH == path
            || PATH
                .strip_prefix(path)
                .is_some_and(|rest| path.ends_with('/') || rest.starts_with('/')))
}

fn text(value: &str, limit: usize) -> Result<Option<String>> {
    let raw = value.as_bytes();
    let mut bytes = Vec::new();
    let mut i = 0;
    while i < raw.len() {
        if raw[i] != b'%' {
            bytes.push(raw[i]);
            i += 1;
            continue;
        }
        if raw.get(i + 1) == Some(&b'u') {
            let first = unit(raw, i)?;
            i += 6;
            let units = if (0xd800..=0xdbff).contains(&first) {
                let second = unit(raw, i)?;
                i += 6;
                vec![first, second]
            } else {
                vec![first]
            };
            let decoded = String::from_utf16(&units).map_err(|_| malformed())?;
            bytes.extend_from_slice(decoded.as_bytes());
        } else {
            let hex = raw.get(i + 1..i + 3).ok_or_else(malformed)?;
            bytes.push(
                u8::from_str_radix(std::str::from_utf8(hex).map_err(|_| malformed())?, 16)
                    .map_err(|_| malformed())?,
            );
            i += 3;
        }
    }
    let value = String::from_utf8(bytes).map_err(|_| malformed())?;
    if value.len() > limit || value.chars().any(char::is_control) {
        return Err(malformed());
    }
    Ok((!value.trim().is_empty()).then_some(value))
}
fn unit(bytes: &[u8], offset: usize) -> Result<u16> {
    if bytes.get(offset..offset + 2) != Some(b"%u") {
        return Err(malformed());
    }
    let hex = bytes.get(offset + 2..offset + 6).ok_or_else(malformed)?;
    u16::from_str_radix(std::str::from_utf8(hex).map_err(|_| malformed())?, 16)
        .map_err(|_| malformed())
}

#[cfg(test)]
mod tests;
