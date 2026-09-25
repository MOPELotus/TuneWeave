use super::*;
use crate::KugouLoginClient;
use tuneweave_core::Artist;

mod write;

impl KugouProvider {
    pub(super) async fn read_followed_artists(
        &self,
        uid: Option<&str>,
        request: &PageRequest,
    ) -> Result<Page<Artist>> {
        if !(1..=100).contains(&request.limit)
            || request.offset.checked_add(request.limit).is_none()
        {
            return Err(kugou_invalid_request(
                "Invalid KuGou followed-artist pagination",
            ));
        }
        let account = request.account.as_deref().unwrap_or("default");
        if let Some((KugouCredential::Native(native), _)) = self.selected(account)?
            && native.session.client != KugouLoginClient::Standard
        {
            return Err(TuneWeaveError::unsupported(
                Platform::Kugou,
                Capability::AccountFollowingArtists,
            ));
        }
        let mut read = self.begin_native_read(account, uid).await?;
        let outcome = async {
            self.check_account_read(&mut read)?;
            let snapshot = self.client.native_followed_artists(read.session()?).await;
            self.check_account_read(&mut read)?;
            let snapshot = snapshot?;
            let total = snapshot.items.len() as u64;
            let items: Vec<_> = snapshot
                .items
                .into_iter()
                .skip(request.offset as usize)
                .take(request.limit as usize)
                .collect();
            let next = request.offset + items.len() as u32;
            let has_more = u64::from(next) < total;
            Ok(Page {
                items,
                pagination: PageMeta {
                    limit: request.limit,
                    offset: request.offset,
                    total: Some(total),
                    has_more,
                    next_offset: has_more.then_some(next),
                    extensions: Extensions::from([
                        ("backend".into(), json!("standard_followed_singers")),
                        ("library_owner_id".into(), json!(read.session()?.user_id)),
                        ("source_version".into(), json!(snapshot.version)),
                        ("complete_read".into(), json!(true)),
                        (
                            "pagination_source".into(),
                            json!("local_full_snapshot_slice"),
                        ),
                        ("order".into(), json!("upstream")),
                    ]),
                },
            })
        }
        .await;
        self.finish_account_read(read, outcome)
    }
}

#[cfg(test)]
mod tests;
