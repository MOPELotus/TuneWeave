use super::*;
use tuneweave_core::{ErrorCode, SearchTrendingList, SearchTrendingRequest};

impl MiguProvider {
    pub(super) async fn read_pc_search_trending(
        &self,
        request: &SearchTrendingRequest,
    ) -> Result<SearchTrendingList> {
        self.require_public_source()?;
        if request.account.is_some() {
            return Err(migu_invalid_request(
                "Migu PC search hot words do not accept an account",
            ));
        }
        tokio::time::timeout(
            std::time::Duration::from_secs(45),
            self.client.pc_search_trending(request.detail),
        )
        .await
        .map_err(|_| {
            TuneWeaveError::new(
                ErrorCode::UpstreamTimeout,
                "Migu search hot words exceeded the total time budget",
            )
            .with_platform(Platform::Migu)
        })?
    }
}

#[cfg(test)]
mod tests;
