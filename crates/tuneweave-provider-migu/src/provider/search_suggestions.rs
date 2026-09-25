use super::*;
use tuneweave_core::{
    ErrorCode, SearchSuggestionClient, SearchSuggestionList, SearchSuggestionRequest,
};

impl MiguProvider {
    pub(super) async fn read_pc_search_suggestions(
        &self,
        request: &SearchSuggestionRequest,
    ) -> Result<SearchSuggestionList> {
        self.require_public_source()?;
        if request.account.is_some() {
            return Err(migu_invalid_request(
                "Migu PC search suggestions do not accept an account",
            ));
        }
        if request.client != SearchSuggestionClient::Pc {
            return Err(TuneWeaveError::unsupported(
                Platform::Migu,
                Capability::SearchSuggestions,
            ));
        }
        let query = crate::client::search_suggestions::validate_query(&request.query)?;
        tokio::time::timeout(
            std::time::Duration::from_secs(45),
            self.client.pc_search_suggestions(query),
        )
        .await
        .map_err(|_| {
            TuneWeaveError::new(
                ErrorCode::UpstreamTimeout,
                "Migu search suggestions exceeded the total time budget",
            )
            .with_platform(Platform::Migu)
        })?
    }
}

#[cfg(test)]
mod tests;
