//! Short-lived passport cookies. Never used for music hosts or durable credentials.
use crate::credential::{error, validate_token};
use reqwest::header::{HeaderMap, HeaderValue, SET_COOKIE};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    time::{Duration, SystemTime},
};
use tuneweave_core::{ErrorCode, Result, TuneWeaveError};

#[derive(Clone, Default)]
pub(crate) struct PassportCookies {
    values: BTreeMap<(String, String, String), Cookie>,
}
#[derive(Clone)]
struct Cookie {
    value: String,
    expires: Option<SystemTime>,
}
impl fmt::Debug for PassportCookies {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PassportCookies")
            .field("count", &self.values.len())
            .finish_non_exhaustive()
    }
}
const PATHS: [&str; 10] = [
    "/password/publickey",
    "/authn",
    "/login/dynamicpassword",
    "/authn/dynamicpassword",
    "/captcha/graph/risk",
    "/captcha/graph/check",
    "/login/second/password",
    "/authn/second/dynamicpassword",
    "/login/send/voice",
    "/authn/voice/validate",
];
fn matches_path(cookie: &str, request: &str) -> bool {
    request == cookie
        || request
            .strip_prefix(cookie)
            .is_some_and(|rest| cookie.ends_with('/') || rest.starts_with('/'))
}
fn invalid() -> TuneWeaveError {
    error(
        ErrorCode::UpstreamError,
        "Invalid or ambiguous Migu passport cookies",
    )
}

impl PassportCookies {
    pub(crate) fn update(&mut self, headers: &HeaderMap, request_path: &str) -> Result<()> {
        if !PATHS.contains(&request_path) {
            return Err(invalid());
        }
        let now = SystemTime::now();
        let mut next = self.clone();
        next.values
            .retain(|_, v| v.expires.is_none_or(|time| time > now));
        let mut seen = BTreeSet::new();
        for header in headers.get_all(SET_COOKIE) {
            let raw = header.to_str().map_err(|_| invalid())?;
            let mut parts = raw.split(';');
            let Some((name, value)) = parts.next().and_then(|v| v.trim().split_once('=')) else {
                continue;
            };
            if !matches!(
                name,
                "mgnd_session_id" | "mgnd_session_create" | "mgnd_session_last_access"
            ) {
                continue;
            }
            if value.len() > 4096 {
                return Err(invalid());
            }
            let mut domain = "passport.migu.cn".to_owned();
            let mut path = request_path
                .rsplit_once('/')
                .map(|v| v.0)
                .filter(|v| !v.is_empty())
                .unwrap_or("/")
                .to_owned();
            let mut attrs = BTreeSet::new();
            let mut max_age = None;
            let mut expires = None;
            for attr in parts {
                let (key, val) = attr.trim().split_once('=').unwrap_or((attr.trim(), ""));
                let key = key.to_ascii_lowercase();
                if matches!(key.as_str(), "domain" | "path" | "max-age" | "expires")
                    && !attrs.insert(key.clone())
                {
                    return Err(invalid());
                }
                match key.as_str() {
                    "domain" => domain = val.strip_prefix('.').unwrap_or(val).to_ascii_lowercase(),
                    "path" => {
                        if val.starts_with('/') {
                            path = val.to_owned();
                        }
                    }
                    "max-age" => max_age = Some(val.parse::<i64>().map_err(|_| invalid())?),
                    "expires" => {
                        expires = Some(httpdate::parse_http_date(val).map_err(|_| invalid())?)
                    }
                    _ => {}
                }
            }
            if !matches!(domain.as_str(), "passport.migu.cn" | "migu.cn")
                || !PATHS.iter().any(|v| matches_path(&path, v))
            {
                continue;
            }
            if path.len() > 128 || !path.bytes().all(|b| b.is_ascii_graphic()) {
                return Err(invalid());
            }
            let key = (name.to_owned(), domain, path);
            if !seen.insert(key.clone()) {
                return Err(invalid());
            }
            if max_age.is_some_and(|v| v <= 0)
                || (max_age.is_none() && expires.is_some_and(|v| v <= now))
            {
                next.values.remove(&key);
                continue;
            }
            validate_token(value).map_err(|_| invalid())?;
            if let Some(seconds) = max_age {
                expires = Some(
                    now.checked_add(Duration::from_secs(seconds as u64))
                        .ok_or_else(invalid)?,
                );
            }
            next.values.insert(
                key,
                Cookie {
                    value: value.into(),
                    expires,
                },
            );
            if next.values.len() > 24 {
                return Err(invalid());
            }
        }
        *self = next;
        Ok(())
    }

    pub(crate) fn header(&self, request_path: &str) -> Result<Option<HeaderValue>> {
        if !PATHS.contains(&request_path) {
            return Err(invalid());
        }
        let now = SystemTime::now();
        let mut names = BTreeSet::new();
        let mut values = Vec::new();
        for ((name, _, path), cookie) in &self.values {
            if cookie.expires.is_some_and(|v| v <= now) || !matches_path(path, request_path) {
                continue;
            }
            // Do not let ambiguous same-name paths/domains change upstream session selection.
            if !names.insert(name) {
                return Err(invalid());
            }
            values.push(format!("{name}={}", cookie.value));
        }
        if values.is_empty() {
            return Ok(None);
        }
        let mut value = HeaderValue::from_str(&values.join("; ")).map_err(|_| invalid())?;
        value.set_sensitive(true);
        Ok(Some(value))
    }
}

#[cfg(test)]
mod tests;
