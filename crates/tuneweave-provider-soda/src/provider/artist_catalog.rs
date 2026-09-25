use tuneweave_core::{ArtistTrackListRequest, ArtistTrackOrder};

use super::*;

impl SodaProvider {
    pub(super) async fn read_artist_tracks(
        &self,
        id: &str,
        request: &ArtistTrackListRequest,
    ) -> Result<Page<Track>> {
        validate_identity(id)?;
        validate_window(request.limit, request.offset)?;
        if request.order != ArtistTrackOrder::PlatformDefault {
            return Err(soda_invalid_request(
                "Soda complete artist tracks require platform_default order",
            ));
        }
        let catalogue = if request.account.is_some() || self.caller_credential.is_some() {
            self.read_account_artist_catalogue::<Track>(
                id,
                request.account.as_deref(),
                std::time::Duration::from_secs(60),
            )
            .await?
        } else {
            self.client.artist_catalog_tracks(id).await?
        };
        Ok(window(catalogue, id, request.limit, request.offset))
    }

    pub(super) async fn read_artist_albums(
        &self,
        id: &str,
        request: &PageRequest,
    ) -> Result<Page<Album>> {
        validate_identity(id)?;
        validate_window(request.limit, request.offset)?;
        let catalogue = if request.account.is_some() || self.caller_credential.is_some() {
            self.read_account_artist_catalogue::<Album>(
                id,
                request.account.as_deref(),
                std::time::Duration::from_secs(60),
            )
            .await?
        } else {
            self.client.artist_catalog_albums(id).await?
        };
        Ok(window(catalogue, id, request.limit, request.offset))
    }

    pub(super) async fn read_account_artist_catalogue<
        T: crate::client::artist_catalog::CatalogueItem + Send,
    >(
        &self,
        id: &str,
        account: Option<&str>,
        deadline: std::time::Duration,
    ) -> Result<crate::client::SodaArtistCatalogue<T>> {
        validate_identity(id)?;
        let alias = account.unwrap_or("default");
        let (mut source, mut stored) = self
            .selected_credential(alias)?
            .ok_or_else(soda_authentication_required)?;
        let initial = source.clone();
        let mut pending = PendingCredentialUpdate::new(self.response_credential.clone());
        let result = tokio::time::timeout(deadline, async {
            let verified = self.client.account(alias, &source).await;
            self.ensure_account_snapshot_current(Some(alias), &source)?;
            let verified = verified?;
            self.advance_library_credential(&mut source, &mut stored, verified.credential)?;
            let selected = source.clone();
            let catalogue = self
                .client
                .collect_artist_catalogue::<T, _>(id, Some(selected), |expected, update| {
                    self.ensure_account_snapshot_current(Some(alias), expected)?;
                    if let Some(updated) = update {
                        self.advance_library_credential(&mut source, &mut stored, updated)?;
                    }
                    Ok(())
                })
                .await;
            self.ensure_account_snapshot_current(Some(alias), &source)?;
            let catalogue = catalogue?;
            crate::account::reject_secrets(
                &serde_json::to_value(&catalogue.items).map_err(|_| {
                    soda_upstream_error("Soda artist catalogue metadata could not be encoded")
                })?,
                &[initial],
            )?;
            Ok(catalogue)
        })
        .await
        .map_err(|_| {
            TuneWeaveError::new(
                ErrorCode::UpstreamTimeout,
                "Soda account artist catalogue exceeded its total time budget",
            )
            .with_platform(Platform::Soda)
            .retryable(true)
        })??;
        pending.complete();
        Ok(result)
    }
}

pub(super) fn validate_identity(id: &str) -> Result<()> {
    if id
        .parse::<u64>()
        .ok()
        .filter(|n| *n > 0 && n.to_string() == id)
        .is_none()
    {
        return Err(soda_invalid_request(
            "Soda artist ID must be a canonical positive decimal",
        ));
    }
    Ok(())
}

fn validate_window(limit: u32, offset: u32) -> Result<()> {
    if !(1..=100).contains(&limit) || offset.checked_add(limit).is_none() {
        return Err(soda_invalid_request(
            "Soda artist catalogue requires limit1-100 and a bounded offset",
        ));
    }
    Ok(())
}

fn window<T>(
    catalogue: crate::client::SodaArtistCatalogue<T>,
    artist_id: &str,
    limit: u32,
    offset: u32,
) -> Page<T> {
    let total = catalogue.items.len() as u64;
    let items: Vec<T> = catalogue
        .items
        .into_iter()
        .skip(offset as usize)
        .take(limit as usize)
        .collect();
    let consumed = offset + items.len() as u32;
    let has_more = (consumed as u64) < total;
    let mut page = Page {
        items,
        pagination: PageMeta {
            limit,
            offset,
            total: Some(total),
            next_offset: has_more.then_some(consumed),
            has_more,
            extensions: Extensions::from([
                ("backend".to_owned(), json!(catalogue.backend)),
                ("complete_read".to_owned(), json!(true)),
                ("artist_id".to_owned(), json!(artist_id)),
                ("reported_total".to_owned(), json!(catalogue.reported_total)),
                ("upstream_pages".to_owned(), json!(catalogue.upstream_pages)),
            ]),
        },
    };
    if let Some(uid) = catalogue.source_user_id {
        page.pagination
            .extensions
            .insert("source_user_id".into(), json!(uid));
        page.pagination
            .extensions
            .insert("authenticated".into(), json!(true));
    }
    page
}
