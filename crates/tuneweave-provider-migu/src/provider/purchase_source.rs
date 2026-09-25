use super::*;
use crate::credential::{authentication_required, validate_uid};
use tuneweave_core::{ErrorCode, PlaylistPlayableItem, PurchasedTrack, ResourceRef};

const SOURCE_TYPE: &str = "purchased_tracks";

impl MiguProvider {
    pub(super) async fn purchased_tracks_source(
        &self,
        id: &str,
        account: Option<&str>,
    ) -> Result<Playlist> {
        self.verify_selected_purchase_identity(id, account)?;
        let request = PageRequest {
            limit: 1,
            offset: 0,
            account: account.map(str::to_owned),
        };
        let page = self.read_purchased_tracks(&request).await?;
        verify_purchase_source_identity(id, &page)?;
        let snapshot = page
            .pagination
            .extensions
            .get("source_snapshot_id")
            .cloned()
            .ok_or_else(|| {
                migu_upstream_error("Migu purchased source omitted its content snapshot")
            })?;
        let resource_ref = ResourceRef::new(Platform::Migu, id).map_err(|_| {
            migu_upstream_error("Migu purchased source returned an invalid account identity")
        })?;
        let extensions = Extensions::from([
            ("backend".to_owned(), json!("pacm_song_ordered_v2")),
            ("source_type".to_owned(), json!(SOURCE_TYPE)),
            ("purchase_kind".to_owned(), json!("tracks")),
            ("source_user_id".to_owned(), json!(id)),
            ("source_snapshot_id".to_owned(), snapshot),
            ("complete_read".to_owned(), json!(true)),
        ]);
        Ok(Playlist {
            resource_ref,
            platform: Platform::Migu,
            id: id.to_owned(),
            name: "Purchased tracks".to_owned(),
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

    pub(super) async fn purchased_tracks_source_items(
        &self,
        id: &str,
        request: &PageRequest,
    ) -> Result<Page<PlaylistPlayableItem>> {
        self.verify_selected_purchase_identity(id, request.account.as_deref())?;
        let page = self.read_purchased_tracks(request).await?;
        verify_purchase_source_identity(id, &page)?;
        let mut items = Vec::with_capacity(page.items.len());
        for purchase in page.items {
            let track = purchase.track.ok_or_else(|| {
                TuneWeaveError::new(
                    ErrorCode::UpstreamError,
                    "Migu purchased source contains an unresolved track",
                )
                .with_platform(Platform::Migu)
                .with_details(json!({
                    "source_type": SOURCE_TYPE,
                    "resource_ref": purchase.extensions.get("resource_ref"),
                    "content_id": purchase.extensions.get("content_id"),
                }))
            })?;
            items.push(PlaylistPlayableItem::Track(track));
        }
        Ok(Page {
            items,
            pagination: page.pagination,
        })
    }

    fn verify_selected_purchase_identity(&self, id: &str, account: Option<&str>) -> Result<()> {
        validate_uid(id)?;
        let alias = account.unwrap_or("default");
        let (selected, _) = self.selected(alias)?.ok_or_else(authentication_required)?;
        if selected.user_id() != id {
            return Err(TuneWeaveError::new(
                ErrorCode::PermissionDenied,
                "Migu purchased source does not belong to the selected account",
            )
            .with_platform(Platform::Migu)
            .with_details(json!({
                "source_type": SOURCE_TYPE,
                "requested_user_id": id,
                "selected_user_id": selected.user_id(),
            })));
        }
        Ok(())
    }
}

fn verify_purchase_source_identity(id: &str, page: &Page<PurchasedTrack>) -> Result<()> {
    let actual = page
        .pagination
        .extensions
        .get("source_user_id")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| migu_upstream_error("Migu purchased source omitted its account identity"))?;
    if actual != id {
        return Err(TuneWeaveError::new(
            ErrorCode::PermissionDenied,
            "Migu purchased source does not belong to the requested account",
        )
        .with_platform(Platform::Migu)
        .with_details(json!({
            "source_type": SOURCE_TYPE,
            "requested_user_id": id,
            "actual_user_id": actual,
        })));
    }
    Ok(())
}
