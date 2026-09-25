//! Read-only metadata for an ordinary playlist whose creator is the selected account.
use super::*;

pub(in crate::client::native) const METADATA_PATH: &str = "/basedata.s";
const METADATA_HOST: &str = "mobilebasedata.kuwo.cn";

pub(in crate::client::native) fn validate_tags(tags: &[String]) -> Result<()> {
    let mut seen = std::collections::BTreeSet::new();
    if tags.len() > 128
        || tags.join(",").len() > 4096
        || tags.iter().any(|v| {
            v.trim().is_empty()
                || v.len() > 256
                || v.contains(',')
                || v.chars().any(char::is_control)
                || !seen.insert(v)
        })
    {
        return Err(kuwo_invalid_request("Kuwo playlist tags are invalid"));
    }
    Ok(())
}

impl KuwoClient {
    pub(in crate::client::native) async fn checked_playlist_metadata(
        &self,
        input: &KuwoNativeSessionInput,
        id: &str,
        check: &mut (impl FnMut() -> Result<()> + Send),
    ) -> Result<Metadata> {
        let query = metadata_query(input, id)?;
        check()?;
        let result = self
            .native_get_with_metadata(
                METADATA_HOST,
                METADATA_PATH,
                "native_playlist_metadata",
                format!(
                    "{}?{query}",
                    self.native_target(METADATA_HOST, METADATA_PATH)
                ),
                Some(session_metadata(input)?),
                |bytes| parse_metadata(bytes, input, id),
            )
            .await;
        check()?;
        result
    }
}
fn metadata_query(input: &KuwoNativeSessionInput, id: &str) -> Result<String> {
    let context = library::query(input, Section::Saved, 0, "")?;
    let mut q = url::form_urlencoded::Serializer::new(String::new());
    for (key, value) in url::form_urlencoded::parse(context.as_bytes()) {
        if !matches!(key.as_ref(), "f" | "type" | "uid" | "count" | "start") {
            q.append_pair(&key, &value);
        }
    }
    q.extend_pairs([
        ("type", "get_songlist_info2"),
        ("id", id),
        ("pos", "0"),
        ("province", ""),
        ("city", ""),
        ("apiv", "3"),
        ("aapiver", "1"),
        ("uid", input.user_id()),
        ("newuigroup", "0"),
    ]);
    Ok(q.finish())
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(in crate::client::native) struct Metadata {
    pub(in crate::client::native) name: String,
    pub(in crate::client::native) description: String,
    pub(in crate::client::native) tags: Vec<String>,
    pub(in crate::client::native) tag_ids: Option<String>,
    pub(in crate::client::native) small_pic: Option<String>,
    pub(in crate::client::native) big_pic: Option<String>,
    pub(in crate::client::native) count: u64,
    pub(in crate::client::native) online: bool,
    pub(in crate::client::native) playlist_type: Option<u64>,
}
#[derive(Deserialize)]
struct MetadataResponse {
    sl_data: MetadataWire,
    #[serde(default, deserialize_with = "deserialize_code")]
    code: Option<String>,
    #[serde(default, deserialize_with = "deserialize_code")]
    errcode: Option<String>,
    result: Option<String>,
}
#[derive(Deserialize)]
struct MetadataWire {
    #[serde(default, deserialize_with = "deserialize_code")]
    id: Option<String>,
    #[serde(deserialize_with = "deserialize_uid")]
    uid: String,
    title: String,
    desc: String,
    tag: String,
    tagid: Option<String>,
    pic: String,
    big_pic: String,
    #[serde(default, deserialize_with = "library::dto::unsigned")]
    total: Option<u64>,
    #[serde(default, deserialize_with = "deserialize_code")]
    igsl: Option<String>,
    #[serde(default, deserialize_with = "library::dto::unsigned")]
    playlist_type: Option<u64>,
}
pub(in crate::client::native) fn parse_metadata(
    bytes: &[u8],
    input: &KuwoNativeSessionInput,
    id: &str,
) -> Result<Metadata> {
    let response: MetadataResponse = serde_json::from_slice(bytes).map_err(|_| invalid())?;
    if response.code.as_deref().is_some_and(|v| v != "0")
        || response.errcode.as_deref().is_some_and(|v| v != "0")
        || response.result.as_deref().is_some_and(|v| v != "ok")
    {
        return Err(invalid());
    }
    let m = response.sl_data;
    if m.uid != input.user_id()
        || m.id.as_deref().is_some_and(|v| v != id)
        || !matches!(m.igsl.as_deref(), Some("" | "0" | "1"))
    {
        return Err(invalid());
    }
    let name = library::dto::text(Some(m.title), 1024, false, input)?
        .filter(|v| !v.trim().is_empty())
        .ok_or_else(invalid)?;
    let description = library::dto::text(Some(m.desc), 16384, true, input)?.unwrap_or_default();
    let raw_tags = library::dto::text(Some(m.tag), 4096, false, input)?.unwrap_or_default();
    let tags = if raw_tags.is_empty() {
        Vec::new()
    } else {
        raw_tags.split(',').map(str::to_owned).collect()
    };
    validate_tags(&tags).map_err(|_| invalid())?;
    let tag_ids = library::dto::text(m.tagid, 4096, false, input)?;
    if let Some(ids) = &tag_ids {
        let ids = ids.split(',').collect::<Vec<_>>();
        if ids.len() != tags.len() || ids.iter().any(|v| playlist::validate_id(v).is_err()) {
            return Err(invalid());
        }
    }
    Ok(Metadata {
        name,
        description,
        tags,
        tag_ids,
        small_pic: library::dto::picture(Some(m.pic), input)?,
        big_pic: library::dto::picture(Some(m.big_pic), input)?,
        count: m.total.ok_or_else(invalid)?,
        online: m.igsl.as_deref() == Some("1"),
        playlist_type: m.playlist_type,
    })
}

fn invalid() -> TuneWeaveError {
    kuwo_upstream_error("Kuwo playlist metadata response is invalid or incomplete")
}

impl Metadata {
    /// Directory counts may be unknown on reads; known values must agree with
    /// the complementary detail. Writes additionally require known visibility/count.
    pub(in crate::client::native) fn matches_directory(&self, p: &Playlist) -> bool {
        p.name == self.name
            && p.description == self.description
            && p.track_count.is_none_or(|count| count == self.count)
            && match &p.cover_url {
                Some(v) => self.small_pic.as_ref() == Some(v) || self.big_pic.as_ref() == Some(v),
                None => self.small_pic.is_none() && self.big_pic.is_none(),
            }
    }
}
