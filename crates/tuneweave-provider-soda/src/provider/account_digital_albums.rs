use super::*;
use crate::client::{ACCOUNT_DIGITAL_ALBUMS_BACKEND as BACKEND, SodaAccountDigitalAlbums};

const TOTAL_BUDGET: std::time::Duration = std::time::Duration::from_secs(45);
const MAX_ITEMS: u32 = 10_000;

impl SodaProvider {
    pub(super) async fn read_account_digital_albums(
        &self,
        request: &PageRequest,
    ) -> Result<Page<DigitalAlbum>> {
        if !(1..=100).contains(&request.limit)
            || request.offset > MAX_ITEMS
            || request.offset.checked_add(request.limit).is_none()
        {
            return Err(soda_invalid_request(
                "Soda purchased-albums page requires limit 1-100 and offset at most 10000",
            ));
        }

        let alias = request.account.as_deref().unwrap_or("default");
        let (mut source, mut stored) = self
            .selected_credential(alias)?
            .ok_or_else(soda_authentication_required)?;
        let initial = source.clone();
        let user_id = source
            .user_id()
            .ok_or_else(soda_authentication_required)?
            .to_owned();
        let mut pending = PendingCredentialUpdate::new(self.response_credential.clone());

        let outcome = tokio::time::timeout(TOTAL_BUDGET, async {
            let verified = self.client.account(alias, &source).await;
            self.ensure_account_snapshot_current(Some(alias), &source)?;
            self.advance_library_credential(&mut source, &mut stored, verified?.credential)?;
            if source.user_id() != Some(user_id.as_str()) {
                return Err(soda_session_changed());
            }

            let snapshot = self.client.account_digital_albums(&source).await;
            self.ensure_account_snapshot_current(Some(alias), &source)?;
            let SodaAccountDigitalAlbums {
                mut albums,
                credential,
            } = snapshot?;
            self.advance_library_credential(&mut source, &mut stored, credential)?;
            if source.user_id() != Some(user_id.as_str()) {
                return Err(soda_session_changed());
            }

            for album in &mut albums {
                album
                    .extensions
                    .insert("source_user_id".to_owned(), json!(user_id.as_str()));
            }
            let snapshot_value = serde_json::to_value(&albums).map_err(|_| {
                TuneWeaveError::new(
                    ErrorCode::InternalError,
                    "Soda purchased-albums snapshot could not be encoded",
                )
                .with_platform(Platform::Soda)
            })?;
            crate::account::reject_secrets(&snapshot_value, &[initial.clone(), source.clone()])?;
            let snapshot_id = source.source_snapshot_fingerprint(
                &serde_json::to_vec(&json!({
                    "version": 1,
                    "source_user_id": user_id.as_str(),
                    "albums": snapshot_value,
                }))
                .map_err(|_| {
                    TuneWeaveError::new(
                        ErrorCode::InternalError,
                        "Soda purchased-albums snapshot could not be fingerprinted",
                    )
                    .with_platform(Platform::Soda)
                })?,
            );
            for album in &mut albums {
                album.extensions.insert(
                    "source_snapshot_id".to_owned(),
                    json!(format!("soda_digital_albums_v1_{snapshot_id}")),
                );
            }
            self.ensure_account_snapshot_current(Some(alias), &source)?;

            let total = albums.len() as u64;
            let items = albums
                .into_iter()
                .skip(request.offset as usize)
                .take(request.limit as usize)
                .collect::<Vec<_>>();
            let end = u64::from(request.offset) + items.len() as u64;
            let has_more = end < total;
            let next_offset =
                (has_more && !items.is_empty()).then_some(request.offset + items.len() as u32);
            Ok(Page {
                items,
                pagination: PageMeta {
                    limit: request.limit,
                    offset: request.offset,
                    total: Some(total),
                    next_offset,
                    has_more,
                    extensions: Extensions::from([
                        ("backend".to_owned(), json!(BACKEND)),
                        ("source_user_id".to_owned(), json!(user_id.as_str())),
                        ("complete_snapshot".to_owned(), json!(true)),
                        (
                            "source_snapshot_id".to_owned(),
                            json!(format!("soda_digital_albums_v1_{snapshot_id}")),
                        ),
                        ("upstream_requests".to_owned(), json!(1)),
                    ]),
                },
            })
        })
        .await;

        self.ensure_account_snapshot_current(Some(alias), &source)?;
        let result = match outcome {
            Ok(result) => result,
            Err(_) => Err(TuneWeaveError::new(
                ErrorCode::UpstreamTimeout,
                "Soda purchased-albums read exceeded its total time budget",
            )
            .with_platform(Platform::Soda)
            .retryable(true)),
        };
        if let Err(error) = &result {
            self.discard_response_credential_after_error(error.code)?;
        }
        pending.complete();
        result
    }
}
