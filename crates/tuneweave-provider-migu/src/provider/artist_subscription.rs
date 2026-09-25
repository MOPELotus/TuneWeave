use super::account_read::Read;
use super::*;
use crate::client::account_download::NativeAuthorization;
use crate::client::following_artists::reject_secrets;
use crate::credential::error;
use std::time::Duration;
use tuneweave_core::{Artist, ErrorCode, ResourceRef, SubscriptionResult};

const DEADLINE: Duration = Duration::from_secs(120);

impl MiguProvider {
    pub(super) async fn change_artist_subscription(
        &self,
        id: &str,
        subscribed: bool,
        account: Option<&str>,
    ) -> Result<SubscriptionResult> {
        if id.is_empty()
            || id.len() > 64
            || id.starts_with('0')
            || !id.bytes().all(|byte| byte.is_ascii_digit())
        {
            return Err(migu_invalid_request("Migu artist ID is invalid"));
        }
        let mut read = Read::new(self, account)?;
        let mut dispatched = false;
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
            let before = self.stable_artist_subscriptions(&mut read, &auth).await?;
            let already = before.iter().any(|artist| artist.id == id);
            if already != subscribed {
                if subscribed {
                    // Confirm a new singer through its existing public catalogue
                    // contract. No PACM/native token goes to that request. Removing
                    // a saved singer and no-op requests need no public metadata.
                    read.check()?;
                    let artist = self.client.artist_info(id).await;
                    read.check()?;
                    let artist = artist?;
                    reject_secrets(std::slice::from_ref(&artist), &read.secrets)?;
                }
                read.check()?;
                dispatched = true;
                let ack = self
                    .client
                    .set_native_artist_subscription(&auth, id, subscribed)
                    .await;
                read.check()?;
                ack?;
                let after = self.stable_artist_subscriptions(&mut read, &auth).await?;
                if after.iter().any(|artist| artist.id == id) != subscribed
                    || !before
                        .iter()
                        .filter(|artist| artist.id != id)
                        .map(|artist| &artist.id)
                        .eq(after
                            .iter()
                            .filter(|artist| artist.id != id)
                            .map(|artist| &artist.id))
                {
                    return Err(migu_upstream_error(
                        "Migu complete artist readback did not confirm the single requested change",
                    ));
                }
            }
            Ok(SubscriptionResult {
                resource_ref: ResourceRef::new(Platform::Migu, id)
                    .map_err(|_| migu_invalid_request("Migu artist ID is invalid"))?,
                subscribed,
                extensions: Extensions::from([
                    (
                        "backend".into(),
                        json!("official_native_artist_subscription"),
                    ),
                    ("source_user_id".into(), json!(read.current.user_id())),
                    ("resource_type".into(), json!("2002")),
                    ("write_performed".into(), json!(dispatched)),
                    (
                        "verified_by".into(),
                        json!("two_complete_selected_account_directory_reads"),
                    ),
                ]),
            })
        })
        .await
        .map_err(|_| {
            error(
                ErrorCode::UpstreamTimeout,
                "Migu artist subscription exceeded the total deadline",
            )
        })
        .and_then(|result| result);
        read.finish(result).map_err(|failure| {
            if dispatched {
                let mut details = failure.details.as_object().cloned().unwrap_or_default();
                details.insert("operation".into(), json!("artist_subscription"));
                details.insert("write_outcome".into(), json!("unconfirmed"));
                details.insert("retry_safe".into(), json!(false));
                failure
                    .retryable(false)
                    .with_details(serde_json::Value::Object(details))
            } else {
                failure
            }
        })
    }

    async fn stable_artist_subscriptions(
        &self,
        read: &mut Read<'_>,
        auth: &NativeAuthorization,
    ) -> Result<Vec<Artist>> {
        let first = self.complete_following_artists(read, auth).await?;
        read.start().await?;
        reject_secrets(&first, &read.secrets)?;
        let second = self.complete_following_artists(read, auth).await?;
        if first != second {
            return Err(error(
                ErrorCode::Conflict,
                "Migu artist subscriptions changed during the complete read",
            ));
        }
        read.start().await?;
        reject_secrets(&first, &read.secrets)?;
        Ok(first)
    }
}

#[cfg(test)]
mod tests;
