use super::*;
use tuneweave_core::{ErrorCode, SimilarArtistList, SimilarArtistRequest};

impl MiguProvider {
    pub(super) async fn read_similar_artists(
        &self,
        id: &str,
        request: &SimilarArtistRequest,
    ) -> Result<SimilarArtistList> {
        self.require_public_source()?;
        if request.account.is_some()
            || !crate::client::videos::valid_id(id)
            || !(1..=100).contains(&request.limit)
        {
            return Err(migu_invalid_request(
                "Migu similar artists require a canonical artist ID, limit 1-100 and no account",
            ));
        }
        tokio::time::timeout(std::time::Duration::from_secs(45), async {
            let source = self.client.artist_info(id).await?;
            if !self.client.similar_artist_module_allowed(id).await? {
                return Err(TuneWeaveError::new(
                    ErrorCode::PermissionDenied,
                    "Migu disabled similar artists for this resource",
                )
                .with_platform(Platform::Migu));
            }
            let mut artists = self.client.similar_artist_catalogue(id).await?;
            let count = artists.len();
            artists.truncate(request.limit as usize);
            Ok(SimilarArtistList {
                artist_ref: source.resource_ref,
                requested_limit: request.limit,
                artists,
                extensions: Extensions::from([
                    ("backend".into(), json!("official_artist_index")),
                    ("catalogue_scope".into(), json!("public")),
                    ("complete_read".into(), json!(true)),
                    ("upstream_count".into(), json!(count)),
                    ("module_policy_checked".into(), json!(true)),
                    ("order".into(), json!("upstream_response_order")),
                ]),
            })
        })
        .await
        .map_err(|_| {
            TuneWeaveError::new(
                ErrorCode::UpstreamTimeout,
                "Migu similar artists exceeded the total time budget",
            )
            .with_platform(Platform::Migu)
        })?
    }
}

#[cfg(test)]
mod tests;
