use super::tracks::{PlaylistSnapshot, Target};
use super::*;

impl KugouProvider {
    async fn native_favorites(
        &self,
        uid: Option<&str>,
        account: Option<&str>,
    ) -> Result<PlaylistSnapshot> {
        let mut read = self
            .begin_native_read(account.unwrap_or("default"), uid)
            .await?;
        let result = self
            .read_native_playlist(&mut read, Target::Favorites)
            .await;
        let mut snapshot = self.finish_account_read(read, result)?;
        snapshot.playlist.extensions.extend([
            ("source_type".into(), json!("favorite_tracks")),
            ("favorite_kind".into(), json!("kugou")),
        ]);
        Ok(snapshot)
    }

    pub(in crate::provider) async fn native_favorite_playlist(
        &self,
        uid: Option<&str>,
        account: Option<&str>,
    ) -> Result<Playlist> {
        Ok(self.native_favorites(uid, account).await?.playlist)
    }
    pub(in crate::provider) async fn native_favorite_tracks(
        &self,
        uid: Option<&str>,
        request: &PageRequest,
    ) -> Result<Page<Track>> {
        if !(1..=100).contains(&request.limit)
            || request.offset.checked_add(request.limit).is_none()
        {
            return Err(kugou_invalid_request(
                "KuGou favorite track pagination is invalid",
            ));
        }
        Ok(self
            .native_favorites(uid, request.account.as_deref())
            .await?
            .into_page(request))
    }
}

#[cfg(test)]
mod tests;
