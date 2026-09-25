use super::*;
use tuneweave_core::{
    PlaylistCatalogTag, PlaylistCatalogTagGroup, PlaylistCatalogTaxonomy,
    PlaylistCatalogTaxonomyRequest,
};

const MAX_GROUPS: usize = 32;
const MAX_TAGS: usize = 1024;

impl KuwoClient {
    pub async fn playlist_catalog_taxonomy(
        &self,
        request: &PlaylistCatalogTaxonomyRequest,
    ) -> Result<PlaylistCatalogTaxonomy> {
        if request.account.is_some() {
            return Err(kuwo_invalid_request(
                "Kuwo playlist catalogue taxonomy requires an anonymous request",
            ));
        }
        tokio::time::timeout(BUDGET, self.fetch_playlist_catalog_taxonomy())
            .await
            .map_err(|_| {
                TuneWeaveError::new(
                    ErrorCode::UpstreamTimeout,
                    "Kuwo playlist catalogue taxonomy timed out",
                )
                .with_platform(Platform::Kuwo)
            })?
    }

    pub(super) async fn fetch_playlist_catalog_taxonomy(&self) -> Result<PlaylistCatalogTaxonomy> {
        for refresh in [false, true] {
            let response = self
                .signed_get(
                    KuwoSignedEndpoint::PlaylistCatalogTaxonomy,
                    &TaxonomyQuery {
                        login_uid: 0,
                        login_sid: 0,
                        https_status: 1,
                        request_id: new_request_id(),
                        plat: "web_www",
                        from: "",
                    },
                    "https://www.kuwo.cn/playlists",
                    refresh,
                    u8::from(refresh),
                )
                .await?;
            match response {
                KuwoSignedResponse::SessionRejected if !refresh => continue,
                KuwoSignedResponse::SessionRejected => return Err(invalid()),
                KuwoSignedResponse::Body(bytes) => {
                    if !refresh && is_signed_session_rejection(&bytes) {
                        continue;
                    }
                    return parse(&bytes);
                }
            }
        }
        Err(invalid())
    }
}

#[derive(Serialize)]
struct TaxonomyQuery {
    #[serde(rename = "loginUid")]
    login_uid: u8,
    #[serde(rename = "loginSid")]
    login_sid: u8,
    #[serde(rename = "httpsStatus")]
    https_status: u8,
    #[serde(rename = "reqId")]
    request_id: String,
    plat: &'static str,
    from: &'static str,
}
#[derive(Deserialize)]
struct Envelope {
    code: i64,
    data: Option<Vec<Group>>,
}
#[derive(Deserialize)]
struct Group {
    id: Unsigned,
    name: String,
    #[serde(rename = "type")]
    kind: String,
    mdigest: Unsigned,
    data: Vec<Tag>,
}
#[derive(Deserialize)]
struct Tag {
    id: Unsigned,
    name: String,
    digest: Unsigned,
}

pub(super) fn parse(bytes: &[u8]) -> Result<PlaylistCatalogTaxonomy> {
    let body: Envelope = serde_json::from_slice(bytes).map_err(|_| invalid())?;
    if body.code != 200 {
        return Err(invalid().with_details(json!({"upstream_code": body.code})));
    }
    let mut groups = body.data.ok_or_else(invalid)?;
    if groups.len() > MAX_GROUPS {
        return Err(invalid());
    }
    let mut tag_count = 0_usize;
    for group in &groups {
        tag_count = tag_count
            .checked_add(group.data.len())
            .ok_or_else(invalid)?;
        if tag_count > MAX_TAGS {
            return Err(invalid());
        }
    }
    // The fixed official /playlists consumer uses slice(0, length - 1). Its last
    // navigation group is not displayed as music playlist tags; do not hard-code
    // that group's current ID/name or the current six visible groups/67 tags.
    let omitted = usize::from(groups.pop().is_some());
    let mut group_ids = BTreeSet::new();
    let mut tag_ids = BTreeSet::new();
    let groups = groups
        .into_iter()
        .map(|group| {
            let id = group.id.id()?;
            let name = catalog::text(&group.name, 512, false)?;
            if !group_ids.insert(id.clone())
                || name.is_empty()
                || group.kind != "list"
                || group.mdigest.value()? != 5
            {
                return Err(invalid());
            }
            let tags = group
                .data
                .into_iter()
                .map(|tag| {
                    let id = tag.id.id()?;
                    let name = catalog::text(&tag.name, 512, false)?;
                    if !tag_ids.insert(id.clone())
                        || name.is_empty()
                        || tag.digest.value()? != 10000
                    {
                        return Err(invalid());
                    }
                    Ok(PlaylistCatalogTag {
                        id,
                        name,
                        extensions: Extensions::from([("upstream_digest".into(), json!(10000))]),
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            Ok(PlaylistCatalogTagGroup {
                id,
                name,
                tags,
                extensions: Extensions::from([
                    ("upstream_type".into(), json!("list")),
                    ("upstream_digest".into(), json!(5)),
                ]),
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(PlaylistCatalogTaxonomy {
        platform: Platform::Kuwo,
        groups,
        extensions: Extensions::from([
            ("backend".into(), json!("current_web_playlist_taxonomy")),
            ("scope".into(), json!("official_web_visible_playlist_tags")),
            ("omitted_trailing_groups".into(), json!(omitted)),
            (
                "consistency_scope".into(),
                json!("single_taxonomy_response"),
            ),
        ]),
    })
}
