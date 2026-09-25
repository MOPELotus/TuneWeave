//! Public artist-index recommendations, gated by the official resource module query.
use super::*;
use serde_json::Value;
use std::collections::BTreeSet;
use tuneweave_core::Artist;

pub(crate) const MODULE_PATH: &str = "/user/api/resource-module/query/v1.0";
pub(crate) const INDEX_PATH: &str = "/bmw/singer/index/v1.0";
const MAX_RESPONSE: u64 = 2 * 1024 * 1024;
const MAX_ARTISTS: usize = 200;

fn invalid() -> TuneWeaveError {
    migu_upstream_error("Migu similar-artist response has invalid identity or structure")
}

fn data(bytes: &[u8]) -> Result<Value> {
    #[derive(Deserialize)]
    struct Envelope {
        code: String,
        data: Value,
    }
    let envelope: Envelope = serde_json::from_slice(bytes).map_err(|_| invalid())?;
    if envelope.code != "000000" || !envelope.data.is_object() {
        return Err(invalid());
    }
    Ok(envelope.data)
}

fn module_allowed(data: Value, uid: &str) -> Result<bool> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Module {
        resource_id: String,
        resource_type: String,
        white_list: Option<Vec<String>>,
        black_list: Option<Vec<String>>,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Modules {
        resource_module_query_list: Vec<Module>,
    }
    let data: Modules = serde_json::from_value(data).map_err(|_| invalid())?;
    let [row] = data.resource_module_query_list.as_slice() else {
        return Err(invalid());
    };
    if row.resource_id != uid || row.resource_type != "2002" {
        return Err(invalid());
    }
    for list in [&row.white_list, &row.black_list].into_iter().flatten() {
        let mut seen = BTreeSet::new();
        if list.len() > 128
            || list.iter().any(|s| {
                s.is_empty() || s.len() > 128 || s.chars().any(char::is_control) || !seen.insert(s)
            })
        {
            return Err(invalid());
        }
    }
    let has = |v: &Option<Vec<String>>| {
        v.as_ref()
            .is_some_and(|v| v.iter().any(|s| s == "similarSinger"))
    };
    // FunctionWhiteOrBlackListUtil: whitelist wins; absent/null lists are no restrictions.
    Ok(!has(&row.black_list) || has(&row.white_list))
}

fn text<'a>(value: &'a Value, key: &str, limit: usize) -> Result<&'a str> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty() && s.len() <= limit && !s.chars().any(char::is_control))
        .ok_or_else(invalid)
}

fn catalogue(data: Value, source: &str) -> Result<Vec<Artist>> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Header {
        next_page_url: Option<String>,
        next_page_no: Option<u32>,
        next_page_no2: Option<u32>,
        update: Option<bool>,
        has_next: Option<bool>,
        has_next_page: Option<bool>,
    }
    #[derive(Deserialize)]
    struct Index {
        header: Header,
        contents: Vec<Value>,
    }
    let data: Index = serde_json::from_value(data).map_err(|_| invalid())?;
    if data.contents.len() > 64
        || data
            .header
            .next_page_url
            .as_deref()
            .is_some_and(|s| !s.is_empty())
        || data.header.next_page_no.is_some_and(|n| n != 1)
        || data.header.next_page_no2.is_some_and(|n| n != 1)
        || data.header.update == Some(true)
        || data.header.has_next == Some(true)
        || data.header.has_next_page == Some(true)
    {
        return Err(invalid());
    }
    let mut section = None;
    for value in &data.contents {
        match text(value, "view", 128)? {
            "ZJ-Singer-Scroll" => {
                if section.is_some() {
                    return Err(invalid());
                }
                section = Some(
                    value
                        .get("contents")
                        .and_then(Value::as_array)
                        .ok_or_else(invalid)?,
                );
            }
            "ZJ-Title" | "ZJ-Singer-Intro-Scroll" | "ZJ-Singer-StarImg-Scroll" => {}
            _ => return Err(invalid()),
        }
    }
    // Only an explicit recommendation array establishes this narrow view's emptiness.
    let rows = section.ok_or_else(invalid)?;
    if rows.len() > MAX_ARTISTS {
        return Err(invalid());
    }
    let mut seen = BTreeSet::new();
    let mut result = Vec::with_capacity(rows.len());
    for row in rows {
        let id = text(row, "resId", 64)?;
        if !super::videos::valid_id(id)
            || id == source
            || !seen.insert(id)
            || text(row, "resType", 8)? != "2002"
            || text(row, "txt2", 64)? != id
            || text(row, "view", 64)? != "ZJ-Singer-Item"
        {
            return Err(invalid());
        }
        let route = Url::parse(text(row, "action", 512)?).map_err(|_| invalid())?;
        let pairs = route.query_pairs().collect::<Vec<_>>();
        if route.scheme() != "mgmusic"
            || route.host_str() != Some("singer-info")
            || !route.path().is_empty()
            || !route.username().is_empty()
            || route.password().is_some()
            || route.port().is_some()
            || route.fragment().is_some()
            || pairs.len() != 1
            || pairs[0].0 != "id"
            || pairs[0].1 != id
        {
            return Err(invalid());
        }
        let avatar = match row.get("img") {
            None | Some(Value::Null) => None,
            Some(Value::String(s)) if s.is_empty() => None,
            Some(Value::String(s)) => {
                let url = Url::parse(s).map_err(|_| invalid())?;
                if s.len() > 8192
                    || s.chars().any(char::is_control)
                    || url.scheme() != "https"
                    || url.host_str() != Some(MEDIA_HOST)
                    || !url.username().is_empty()
                    || url.password().is_some()
                    || url.port().is_some()
                    || url.query().is_some()
                    || url.fragment().is_some()
                    || !url
                        .path()
                        .strip_prefix("/data/oss/resource/")
                        .is_some_and(|tail| {
                            !tail.is_empty()
                                && tail.split('/').all(|p| {
                                    !p.is_empty()
                                        && !matches!(p, "." | "..")
                                        && p.bytes().all(|b| {
                                            b.is_ascii_alphanumeric()
                                                || matches!(b, b'.' | b'_' | b'-')
                                        })
                                })
                        })
                {
                    return Err(invalid());
                }
                Some(s.clone())
            }
            _ => return Err(invalid()),
        };
        result.push(Artist {
            resource_ref: ResourceRef::new(Platform::Migu, id).map_err(|_| invalid())?,
            platform: Platform::Migu,
            id: id.into(),
            name: text(row, "txt", 2048)?.into(),
            aliases: vec![],
            description: String::new(),
            biography_sections: vec![],
            avatar_url: avatar,
            cover_url: None,
            track_count: None,
            album_count: None,
            mv_count: None,
            video_count: None,
            identities: vec![],
            extensions: Extensions::from([("backend".into(), json!("official_artist_index"))]),
        });
    }
    Ok(result)
}

impl MiguClient {
    async fn similar_response(&self, uid: &str, module: bool) -> Result<Value> {
        let path = if module { MODULE_PATH } else { INDEX_PATH };
        let started = Instant::now();
        let mut status = None;
        let result = async {
            let url = self.catalog_endpoint(&format!("https://app.c.nf.migu.cn{path}"))?;
            let request = if module {
                // This POST only queries resource-display policy; it is not a subscription/write.
                self.http.post(url).json(&json!({"resourceModuleQueryParam":[{"resourceId":uid,"resourceType":"2002","responseType":"module"}]})).header(reqwest::header::CONTENT_TYPE,"application/json; charset=utf-8")
            } else { self.http.get(url).query(&[("singerId",uid)]) };
            let response = request.header(ACCEPT,"application/json").send().await.map_err(migu_network_error)?;
            status = Some(response.status());
            if response.status().is_success() && !response.headers().get(reqwest::header::CONTENT_TYPE)
                .and_then(|v|v.to_str().ok()).and_then(|v|v.split(';').next()).is_some_and(|v|v.trim().eq_ignore_ascii_case("application/json")) { return Err(invalid()); }
            let bytes = read_bounded_response_with_limit(response,"Migu similar artists",MAX_RESPONSE).await?;
            data(&bytes)
        }.await;
        self.log_upstream_request(
            "similar_artists",
            "app.c.nf.migu.cn",
            path,
            status,
            started,
            &result,
        );
        result
    }

    pub(crate) async fn similar_artist_module_allowed(&self, uid: &str) -> Result<bool> {
        module_allowed(self.similar_response(uid, true).await?, uid)
    }
    pub(crate) async fn similar_artist_catalogue(&self, uid: &str) -> Result<Vec<Artist>> {
        catalogue(self.similar_response(uid, false).await?, uid)
    }
}

#[cfg(test)]
pub(crate) mod tests;
