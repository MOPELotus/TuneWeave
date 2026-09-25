//! The signed-in native user's followed artist directory.
use super::*;
use serde::Deserialize;
use std::collections::BTreeSet;
use tuneweave_core::{Artist, Page, PageMeta, PageRequest, ResourceRef};

#[cfg(test)]
pub(crate) mod tests;

const PATH: &str = super::library::SAVED_PATH;
const HOST: &str = "wapi.kuwo.cn";
const REQUEST_SIZE: usize = 1000;

impl KuwoClient {
    pub(crate) async fn fetch_following_artists(
        &self,
        input: &KuwoNativeSessionInput,
        request: &PageRequest,
        mut check: impl FnMut() -> Result<()> + Send,
    ) -> Result<Page<Artist>> {
        super::library::validate_request(request)?;
        validate_session_metadata(input)?;
        tokio::time::timeout(Duration::from_secs(60), async {
            let artists = self
                .fetch_following_artist_snapshot(input, &mut check)
                .await?;
            let total = artists.len() as u64;
            let items: Vec<_> = artists
                .into_iter()
                .skip(request.offset as usize)
                .take(request.limit as usize)
                .collect();
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
                        ("backend".into(), json!("native_following_artists")),
                        ("library_owner_id".into(), json!(input.user_id())),
                        ("complete_read".into(), json!(true)),
                        ("consistency".into(), json!("single_complete_traversal")),
                        ("upstream_page_size".into(), json!(REQUEST_SIZE)),
                    ]),
                },
            })
        })
        .await
        .map_err(|_| {
            TuneWeaveError::new(
                ErrorCode::UpstreamTimeout,
                "Kuwo followed-artist directory read timed out",
            )
            .with_platform(Platform::Kuwo)
        })?
    }

    pub(crate) async fn fetch_following_artist_snapshot(
        &self,
        input: &KuwoNativeSessionInput,
        mut check: impl FnMut() -> Result<()> + Send,
    ) -> Result<Vec<Artist>> {
        validate_session_metadata(input)?;
        check()?;
        let artists = self.native_following_artist_snapshot(input).await;
        check()?;
        artists
    }

    pub(crate) async fn set_following_artist(
        &self,
        input: &KuwoNativeSessionInput,
        id: &str,
        subscribed: bool,
        dispatched: &mut bool,
        check: impl FnOnce() -> Result<()>,
    ) -> Result<()> {
        validate_artist_id(id)?;
        validate_session_metadata(input)?;
        let query = {
            let mut query = url::form_urlencoded::Serializer::new(String::new());
            query.extend_pairs([
                (
                    "type",
                    if subscribed {
                        "click_like"
                    } else {
                        "cancel_like"
                    },
                ),
                ("uid", input.user_id()),
                ("digest", "4"),
                ("sid", id),
                ("loginSid", input.session_id()),
                ("newver", "3"),
            ]);
            query.finish()
        };
        let target = format!("{}?{query}", self.native_target(HOST, PATH));
        self.native_get_with_metadata_hooks(
            HOST,
            PATH,
            "native_artist_subscription",
            target,
            Some(session_metadata(input)?),
            (
                |bytes| {
                    if bytes.iter().all(u8::is_ascii_whitespace) {
                        Err(invalid())
                    } else {
                        // The response body is not proof of the resulting state;
                        // the provider requires a complete directory readback.
                        Ok(())
                    }
                },
                || {
                    check()?;
                    *dispatched = true;
                    Ok(())
                },
            ),
        )
        .await
    }

    async fn native_following_artist_snapshot(
        &self,
        input: &KuwoNativeSessionInput,
    ) -> Result<Vec<Artist>> {
        let metadata = session_metadata(input)?;
        let query = {
            let mut query = url::form_urlencoded::Serializer::new(String::new());
            query.extend_pairs([
                ("type", "get_like_list"),
                ("uid", input.user_id()),
                ("digest", "4"),
                ("start", "0"),
                ("count", "1000"),
                ("loginSid", input.session_id()),
                ("newver", "3"),
            ]);
            query.finish()
        };
        let target = format!("{}?{query}", self.native_target(HOST, PATH));
        self.native_get_with_metadata(
            HOST,
            PATH,
            "native_following_artists",
            target,
            Some(metadata),
            |bytes| parse(bytes, input),
        )
        .await
    }
}

pub(crate) fn validate_artist_id(id: &str) -> Result<()> {
    match id.parse::<u64>() {
        Ok(value) if value > 0 && value.to_string() == id => Ok(()),
        _ => Err(kuwo_invalid_request(
            "Kuwo artist ID must be a positive decimal ID",
        )),
    }
}

#[derive(Deserialize)]
struct Response {
    result: String,
    #[serde(default, deserialize_with = "deserialize_code")]
    errcode: Option<String>,
    #[serde(default, deserialize_with = "deserialize_code")]
    uid: Option<String>,
    data: Vec<Entry>,
}

#[derive(Deserialize)]
struct Entry {
    #[serde(default, deserialize_with = "deserialize_code")]
    id: Option<String>,
    name: String,
    #[serde(default)]
    img: Option<String>,
    #[serde(default, rename = "AARTIST")]
    alias: Option<String>,
    #[serde(default, deserialize_with = "deserialize_code")]
    musiccnt: Option<String>,
    #[serde(default, deserialize_with = "deserialize_code")]
    albumcnt: Option<String>,
    #[serde(default, deserialize_with = "deserialize_code")]
    mvcnt: Option<String>,
    #[serde(default, deserialize_with = "deserialize_code")]
    followers: Option<String>,
    #[serde(default, deserialize_with = "deserialize_code")]
    digest: Option<String>,
    #[serde(default, deserialize_with = "deserialize_code")]
    szb: Option<String>,
}

fn parse(bytes: &[u8], input: &KuwoNativeSessionInput) -> Result<Vec<Artist>> {
    let response: Response = serde_json::from_slice(bytes).map_err(|_| invalid())?;
    if response.result != "ok"
        || response.errcode.as_deref().is_some_and(|code| code != "0")
        || response
            .uid
            .as_deref()
            .is_some_and(|uid| uid != input.user_id())
        || response.data.len() >= REQUEST_SIZE
    {
        // The official app requests 1000 rows in one call. A full response may
        // be truncated; root `total` has no established completion semantics.
        return Err(invalid());
    }
    let mut seen = BTreeSet::new();
    response
        .data
        .into_iter()
        .map(|entry| map_entry(entry, input, &mut seen))
        .collect()
}

fn map_entry(
    entry: Entry,
    input: &KuwoNativeSessionInput,
    seen: &mut BTreeSet<String>,
) -> Result<Artist> {
    if entry.szb.as_deref() == Some("1") || entry.digest.as_deref() != Some("4") {
        // The native page can also contain AnchorInfo. The public model has no
        // anchor resource, so never silently omit or mislabel one.
        return Err(invalid());
    }
    let id = entry.id.as_deref().ok_or_else(invalid)?;
    let parsed = id.parse::<u64>().map_err(|_| invalid())?;
    if parsed == 0 || parsed.to_string() != id || !seen.insert(id.to_owned()) {
        return Err(invalid());
    }
    let name = strict_text(&entry.name, 512, input)?;
    if name.is_empty() {
        return Err(invalid());
    }
    let alias = entry
        .alias
        .as_deref()
        .map(|value| strict_text(value, 512, input))
        .transpose()?
        .filter(|value| !value.is_empty() && value != &name);
    let count = |value: Option<String>| {
        value
            .filter(|value| !value.is_empty())
            .map(|value| parse_count(&value))
            .transpose()
    };
    let mut extensions = Extensions::new();
    if let Some(followers) = count(entry.followers)? {
        extensions.insert("followers".into(), json!(followers));
    }
    let avatar_url = entry
        .img
        .as_deref()
        .map(|value| strict_text(value, 2048, input))
        .transpose()?
        .and_then(|value| super::super::catalog::artist_image(&value));
    Ok(Artist {
        resource_ref: ResourceRef::new(Platform::Kuwo, id).map_err(|_| invalid())?,
        platform: Platform::Kuwo,
        id: id.to_owned(),
        name,
        aliases: alias.into_iter().collect(),
        description: String::new(),
        biography_sections: Vec::new(),
        avatar_url,
        cover_url: None,
        album_count: count(entry.albumcnt)?,
        track_count: count(entry.musiccnt)?,
        mv_count: count(entry.mvcnt)?,
        video_count: None,
        identities: Vec::new(),
        extensions,
    })
}

fn strict_text(value: &str, limit: usize, input: &KuwoNativeSessionInput) -> Result<String> {
    if value.len() > limit
        || value.chars().any(|character| character.is_control())
        || echoes_secret(value, input.session_id())
    {
        return Err(invalid());
    }
    Ok(value.trim().to_owned())
}

fn parse_count(value: &str) -> Result<u64> {
    let count = value.parse::<u64>().map_err(|_| invalid())?;
    if count.to_string() != value {
        return Err(invalid());
    }
    Ok(count)
}

fn invalid() -> TuneWeaveError {
    kuwo_upstream_error("Kuwo followed-artist response is invalid or incomplete")
}
