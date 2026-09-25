use super::*;
use crate::client::artist_directory::Selection;
use tuneweave_core::{ArtistCatalog, ArtistCatalogRequest, ErrorCode};

impl MiguProvider {
    pub(super) async fn read_artist_directory(
        &self,
        request: &ArtistCatalogRequest,
    ) -> Result<ArtistCatalog> {
        self.require_public_source()?;
        if request.account.is_some() {
            return Err(migu_invalid_request(
                "Migu public artist directory does not accept an account",
            ));
        }
        // The official directory has no All area/category or genre selector.
        // Reject unsupported semantics before even requesting the taxonomy.
        let selection = Selection::new(request.area, request.category, request.genre)?;
        tokio::time::timeout(
            std::time::Duration::from_secs(45),
            self.client.artist_directory(selection),
        )
        .await
        .map_err(|_| {
            TuneWeaveError::new(
                ErrorCode::UpstreamTimeout,
                "Migu artist directory exceeded the total time budget",
            )
            .with_platform(Platform::Migu)
        })?
    }
}

#[cfg(test)]
mod tests;
