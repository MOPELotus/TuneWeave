use super::*;

impl KuwoClient {
    /// Reads the actual MYFAVORITE cloud playlist of the independently validated
    /// native account. Missing or ambiguous system lists are not fabricated.
    pub async fn native_favorite_playlist(
        &self,
        credential: &ProviderCredential,
    ) -> Result<Playlist> {
        Ok(self
            .native_playlist_snapshot(credential, None, Section::Favorite)
            .await?
            .playlist)
    }

    /// Reads both complete ordered favorite-song traversals and rechecks the
    /// system list identity before applying a window. No synchronization or writes.
    pub async fn native_favorite_tracks(
        &self,
        credential: &ProviderCredential,
        request: &PageRequest,
    ) -> Result<Page<Track>> {
        validate_request(request)?;
        if request.account.as_deref().is_some_and(|v| v != "default") {
            return Err(kuwo_invalid_request(
                "Kuwo SDK credentials cannot select a stored account",
            ));
        }
        Ok(self
            .native_playlist_snapshot(credential, None, Section::Favorite)
            .await?
            .into_page(request))
    }
}
