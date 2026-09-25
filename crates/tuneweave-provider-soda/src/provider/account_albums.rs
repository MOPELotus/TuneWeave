use super::*;
use crate::client::SodaAccountAlbum;

const TOTAL_BUDGET: std::time::Duration = std::time::Duration::from_secs(45);

impl SodaAccountAlbum {
    pub(super) fn into_page(self, request: &PageRequest) -> Page<Track> {
        let extensions = self.page.album.extensions.clone();
        let mut page = soda_album_track_page(self.page.tracks, request);
        for key in [
            "backend",
            "source_user_id",
            "source_snapshot_id",
            "complete_read",
        ] {
            if let Some(value) = extensions.get(key) {
                page.pagination
                    .extensions
                    .insert(key.to_owned(), value.clone());
            }
        }
        page
    }
}

impl SodaProvider {
    pub(super) async fn read_account_album(
        &self,
        id: &str,
        account: Option<&str>,
    ) -> Result<Option<SodaAccountAlbum>> {
        if account.is_none() && self.caller_credential.is_none() {
            return Ok(None);
        }
        let alias = account.unwrap_or("default");
        let (mut source, mut stored) = self
            .selected_credential(alias)?
            .ok_or_else(soda_authentication_required)?;
        let mut pending = PendingCredentialUpdate::new(self.response_credential.clone());
        let result = tokio::time::timeout(TOTAL_BUDGET, async {
            let verified = self.client.account(alias, &source).await;
            self.ensure_account_snapshot_current(Some(alias), &source)?;
            let verified = verified?;
            self.advance_library_credential(&mut source, &mut stored, verified.credential)?;
            let snapshot = self.client.account_album(id, &source).await;
            self.ensure_account_snapshot_current(Some(alias), &source)?;
            let mut snapshot = snapshot?;
            self.advance_library_credential(&mut source, &mut stored, snapshot.credential.clone())?;
            let user_id = source.user_id().ok_or_else(soda_authentication_required)?;
            snapshot
                .page
                .album
                .extensions
                .insert("source_user_id".to_owned(), json!(user_id));
            snapshot
                .page
                .album
                .extensions
                .insert("complete_read".to_owned(), json!(true));
            let material = serde_json::to_vec(&json!({
                "version":1,"source_type":"album","user_id":user_id,"album":snapshot.page.album,
                "tracks":snapshot.page.tracks.iter().map(|t|&t.resource_ref).collect::<Vec<_>>(),
            }))
            .map_err(|_| {
                TuneWeaveError::new(
                    ErrorCode::InternalError,
                    "Soda album snapshot could not be encoded",
                )
                .with_platform(Platform::Soda)
            })?;
            let snapshot_id = format!(
                "soda_album_v1_{}",
                source.source_snapshot_fingerprint(&material)
            );
            snapshot
                .page
                .album
                .extensions
                .insert("source_snapshot_id".to_owned(), json!(snapshot_id));
            self.ensure_account_snapshot_current(Some(alias), &source)?;
            Ok(snapshot)
        })
        .await
        .map_err(|_| {
            TuneWeaveError::new(
                ErrorCode::UpstreamTimeout,
                "Soda account album exceeded its total time budget",
            )
            .with_platform(Platform::Soda)
            .retryable(true)
        })?;
        let snapshot = result?;
        pending.complete();
        Ok(Some(snapshot))
    }
}
