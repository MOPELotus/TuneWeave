use super::account_read::Read;
use super::*;
use crate::client::account_download::NativeAuthorization;
use crate::client::following_artists::{BACKEND, reject_secrets};
use crate::credential::{error, validate_uid};
use sha1::{Digest, Sha1};
use std::time::Duration;
use tuneweave_core::{Artist, ErrorCode};

const DEADLINE: Duration = Duration::from_secs(120);
const MAX_PAGES: u32 = 128;

impl MiguProvider {
    pub(super) async fn read_following_artists(
        &self,
        uid: Option<&str>,
        request: &PageRequest,
    ) -> Result<Page<Artist>> {
        if !(1..=100).contains(&request.limit)
            || request.offset.checked_add(request.limit).is_none()
        {
            return Err(migu_invalid_request(
                "Migu following artist pagination is invalid",
            ));
        }
        if let Some(uid) = uid {
            validate_uid(uid)?;
        }
        let mut read = Read::new(self, request.account.as_deref())?;
        if uid.is_some_and(|uid| uid != read.current.user_id()) {
            return Err(error(
                ErrorCode::PermissionDenied,
                "Migu following artists are available only for the selected account",
            ));
        }
        let result = tokio::time::timeout(DEADLINE, async {
            let session = read.native_session().await?;
            let reply = self
                .client
                .account_h5_token(read.current.token(), &session)
                .await;
            let candidate = read.accept(reply).await??;
            read.secrets.push(candidate.clone());
            let auth = self
                .client
                .validate_native_token(candidate, read.current.user_id())
                .await;
            read.check()?;
            let auth = auth?;
            let artists = self.complete_following_artists(&mut read, &auth).await?;
            read.start().await?;
            reject_secrets(&artists, &read.secrets)?;
            let confirmed = self.complete_following_artists(&mut read, &auth).await?;
            if artists != confirmed {
                return Err(error(
                    ErrorCode::Conflict,
                    "Migu following artists changed during the complete read",
                ));
            }
            read.start().await?;
            reject_secrets(&artists, &read.secrets)?;
            let snapshot = serde_json::to_vec(&(BACKEND, read.current.user_id(), &artists))
                .map_err(|_| {
                    error(
                        ErrorCode::InternalError,
                        "Migu following artist snapshot could not be serialized",
                    )
                })?;
            let total = artists.len() as u64;
            let items: Vec<_> = artists
                .into_iter()
                .skip(request.offset as usize)
                .take(request.limit as usize)
                .collect();
            let end = request.offset + items.len() as u32;
            let more = u64::from(end) < total;
            Ok(Page {
                items,
                pagination: PageMeta {
                    limit: request.limit,
                    offset: request.offset,
                    total: Some(total),
                    has_more: more,
                    next_offset: more.then_some(end),
                    extensions: Extensions::from([
                        ("backend".into(), json!(BACKEND)),
                        ("source_user_id".into(), json!(read.current.user_id())),
                        ("complete_read".into(), json!(true)),
                        ("consistency".into(), json!("two_complete_reads")),
                        (
                            "source_snapshot_id".into(),
                            json!(format!(
                                "migu-following-artists-{}",
                                hex::encode(Sha1::digest(snapshot))
                            )),
                        ),
                    ]),
                },
            })
        })
        .await
        .map_err(|_| {
            error(
                ErrorCode::UpstreamTimeout,
                "Migu following artists exceeded the total deadline",
            )
        })
        .and_then(|result| result);
        read.finish(result)
    }

    pub(super) async fn complete_following_artists(
        &self,
        read: &mut Read<'_>,
        auth: &NativeAuthorization,
    ) -> Result<Vec<Artist>> {
        let mut artists = Vec::new();
        let mut seen = BTreeSet::new();
        // Up to 128 nonempty pages plus the required successful empty sentinel.
        for page in 1..=MAX_PAGES + 1 {
            read.check()?;
            let result = self.client.native_following_artists(auth, page).await;
            read.check()?;
            let items = result?;
            reject_secrets(&items, &read.secrets)?;
            if items.is_empty() {
                return Ok(artists);
            }
            if page > MAX_PAGES {
                return Err(migu_upstream_error(
                    "Migu following artists exceeded the complete-read budget",
                ));
            }
            for artist in items {
                if !seen.insert(artist.id.clone()) {
                    return Err(error(
                        ErrorCode::Conflict,
                        "Migu following artist pages repeated an identity",
                    ));
                }
                artists.push(artist);
            }
        }
        unreachable!("the final page either terminates or exceeds the budget")
    }
}

#[cfg(test)]
mod tests;
