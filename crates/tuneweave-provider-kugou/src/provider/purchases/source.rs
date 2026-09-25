//! UID-scoped purchased songs as a read-only Uni source. Goods are not entitlements.
use super::*;
use tuneweave_core::{PlaylistPlayableItem, ResourceRef};

const SOURCE: &str = "purchased_tracks";

fn source_extensions(id: &str, page: &Page<Item>) -> Result<Extensions> {
    let ext = &page.pagination.extensions;
    if ext
        .get("library_owner_id")
        .and_then(serde_json::Value::as_str)
        != Some(id)
        || ext
            .get("complete_read")
            .and_then(serde_json::Value::as_bool)
            != Some(true)
        || ext
            .get("source_snapshot_id")
            .and_then(serde_json::Value::as_str)
            .is_none_or(str::is_empty)
    {
        return Err(changed());
    }
    // Check the whole source, including records outside the requested window.
    // Account purchase records still expose unresolved goods; Uni requires a real Track.
    if ext
        .get("unresolved_entries")
        .and_then(serde_json::Value::as_u64)
        != Some(0)
    {
        return Err(TuneWeaveError::new(
            ErrorCode::UpstreamError,
            "KuGou purchased source contains unresolved catalogue records",
        )
        .with_platform(Platform::Kugou)
        .with_details(
            json!({"source_type":SOURCE,"unresolved_entries":ext.get("unresolved_entries")}),
        ));
    }
    let mut extensions = ext.clone();
    extensions.insert("source_type".into(), json!(SOURCE));
    extensions.insert("source_user_id".into(), json!(id));
    Ok(extensions)
}

impl KugouProvider {
    pub(in crate::provider) async fn purchased_tracks_source(
        &self,
        id: &str,
        account: Option<&str>,
    ) -> Result<Playlist> {
        let page = self
            .native_purchases_for_owner(
                Kind::Tracks,
                &PageRequest {
                    limit: 1,
                    offset: 0,
                    account: account.map(str::to_owned),
                },
                Some(id),
            )
            .await?;
        let extensions = source_extensions(id, &page)?;
        Ok(Playlist {
            resource_ref: ResourceRef::new(Platform::Kugou, id).map_err(|_| changed())?,
            platform: Platform::Kugou,
            id: id.to_owned(),
            name: "Purchased tracks".into(),
            description: String::new(),
            cover_url: None,
            creator: None,
            track_count: page.pagination.total,
            tags: Vec::new(),
            subscribed: None,
            created_at: None,
            updated_at: None,
            extensions,
        })
    }

    pub(in crate::provider) async fn purchased_tracks_source_items(
        &self,
        id: &str,
        request: &PageRequest,
    ) -> Result<Page<PlaylistPlayableItem>> {
        let mut page = self
            .native_purchases_for_owner(Kind::Tracks, request, Some(id))
            .await?;
        page.pagination.extensions = source_extensions(id, &page)?;
        let items = page
            .items
            .into_iter()
            .map(|item| match item {
                Item::Track(PurchasedTrack {
                    track: Some(track), ..
                }) => Ok(PlaylistPlayableItem::Track(track)),
                _ => Err(changed()),
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Page {
            items,
            pagination: page.pagination,
        })
    }
}

#[cfg(test)]
mod tests;
