use super::*;
use crate::client::discovery::TAGS;
use tuneweave_core::{
    PlaylistCatalogKind, PlaylistCatalogRequest, PlaylistCatalogTag, PlaylistCatalogTagGroup,
    PlaylistCatalogTaxonomy, PlaylistCatalogTaxonomyRequest,
};

impl SodaProvider {
    fn require_public_discovery(&self, account: Option<&str>) -> Result<()> {
        if account.is_some() || self.caller_credential.is_some() {
            return Err(soda_invalid_request(
                "Soda public discovery does not accept account credentials",
            ));
        }
        Ok(())
    }
    pub(super) fn discovery_taxonomy(
        &self,
        request: &PlaylistCatalogTaxonomyRequest,
    ) -> Result<PlaylistCatalogTaxonomy> {
        self.require_public_discovery(request.account.as_deref())?;
        let mut groups: Vec<PlaylistCatalogTagGroup> = Vec::new();
        for (id, name, group) in TAGS {
            let index = groups
                .iter()
                .position(|g| g.name == group)
                .unwrap_or_else(|| {
                    groups.push(PlaylistCatalogTagGroup {
                        id: format!("group{}", groups.len()),
                        name: group.into(),
                        tags: Vec::new(),
                        extensions: Extensions::new(),
                    });
                    groups.len() - 1
                });
            groups[index].tags.push(PlaylistCatalogTag {
                id: id.into(),
                name: name.into(),
                extensions: Extensions::new(),
            });
        }
        Ok(PlaylistCatalogTaxonomy {
            platform: Platform::Soda,
            groups,
            extensions: Extensions::from([("catalog_scope".into(), json!("curated_sub_channels"))]),
        })
    }
    pub(super) async fn read_discovery(
        &self,
        request: &PlaylistCatalogRequest,
    ) -> Result<Page<Playlist>> {
        self.require_public_discovery(request.account.as_deref())?;
        let tag = request
            .tag_id
            .as_deref()
            .filter(|id| TAGS.iter().any(|(known, _, _)| known == id))
            .ok_or_else(|| soda_invalid_request("Soda discovery requires a known tag_id"))?;
        if request.catalog != PlaylistCatalogKind::Tag
            || request.offset != 0
            || !(1..=100).contains(&request.limit)
        {
            return Err(soda_invalid_request(
                "Soda discovery supports tag recommendation windows with offset=0 and limit=1..100",
            ));
        }
        let tag = tag
            .parse::<u32>()
            .map_err(|_| soda_invalid_request("Invalid Soda discovery tag"))?;
        let (rows, upstream_more) = tokio::time::timeout(
            std::time::Duration::from_secs(45),
            self.client.discovery_playlists(tag),
        )
        .await
        .map_err(|_| {
            TuneWeaveError::new(
                ErrorCode::UpstreamTimeout,
                "Soda discovery exceeded its time budget",
            )
            .with_platform(Platform::Soda)
        })??;
        let window_count = rows.len();
        Ok(Page {
            items: rows.into_iter().take(request.limit as usize).collect(),
            pagination: PageMeta {
                limit: request.limit,
                offset: 0,
                total: None,
                next_offset: None,
                has_more: false,
                extensions: Extensions::from([
                    ("result_scope".into(), json!("recommendation_window")),
                    ("upstream_has_more".into(), json!(upstream_more)),
                    ("window_count".into(), json!(window_count)),
                ]),
            },
        })
    }
}
