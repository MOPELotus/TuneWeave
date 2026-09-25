use super::*;
use crate::account::following::FollowingSnapshot;
use tuneweave_core::{ResourceRef, SubscriptionResult};

fn contains(snapshot: &FollowingSnapshot, id: &str) -> bool {
    snapshot.items.iter().any(|artist| artist.id == id)
}

fn verify_delta(
    before: &FollowingSnapshot,
    after: &FollowingSnapshot,
    id: &str,
    subscribed: bool,
) -> Result<()> {
    let unrelated = |snapshot: &FollowingSnapshot| {
        snapshot
            .items
            .iter()
            .filter(|a| a.id != id)
            .map(|a| a.id.clone())
            .collect::<Vec<_>>()
    };
    if after.version < before.version
        || contains(after, id) != subscribed
        || unrelated(before) != unrelated(after)
    {
        return Err(TuneWeaveError::new(
            ErrorCode::Conflict,
            "KuGou artist subscription could not be confirmed by complete readback",
        )
        .with_platform(Platform::Kugou));
    }
    Ok(())
}

fn unconfirmed(
    mut error: TuneWeaveError,
    dispatched: bool,
    reference: &ResourceRef,
) -> TuneWeaveError {
    if dispatched {
        let mut details = error.details.as_object().cloned().unwrap_or_default();
        details.insert("operation".into(), json!("artist_subscription"));
        details.insert("write_outcome".into(), json!("unconfirmed"));
        details.insert("write_requests_dispatched".into(), json!(1));
        details.insert("unconfirmed_refs".into(), json!([reference]));
        error = error
            .retryable(false)
            .with_details(serde_json::Value::Object(details));
    }
    error
}

impl KugouProvider {
    pub(in crate::provider) async fn native_set_artist_subscription(
        &self,
        id: &str,
        subscribed: bool,
        account: Option<&str>,
    ) -> Result<SubscriptionResult> {
        let singer_id = id
            .parse::<u64>()
            .ok()
            .filter(|n| *n > 0 && *n <= i64::MAX as u64 && n.to_string() == id)
            .ok_or_else(|| {
                kugou_invalid_request("KuGou artist id must be a canonical positive signed integer")
            })?;
        let reference = ResourceRef::new(Platform::Kugou, id)
            .map_err(|_| kugou_invalid_request("Invalid KuGou artist reference"))?;
        let account = account.unwrap_or("default");
        if let Some((KugouCredential::Native(native), _)) = self.selected(account)?
            && native.session.client != KugouLoginClient::Standard
        {
            return Err(TuneWeaveError::unsupported(
                Platform::Kugou,
                Capability::ArtistSubscriptionWrite,
            ));
        }
        let mut read = self.begin_native_read(account, None).await?;
        let mut dispatched = false;
        let outcome = async {
            self.check_account_read(&mut read)?;
            let before = self.client.native_followed_artists(read.session()?).await;
            self.check_account_read(&mut read)?;
            let before = before?;
            let changed = contains(&before, id) != subscribed;
            let version = if changed {
                self.check_account_read(&mut read)?;
                dispatched = true;
                let ack = self
                    .client
                    .native_write_artist_subscription(read.session()?, singer_id, subscribed)
                    .await;
                self.check_account_read(&mut read)?;
                ack?;
                let after = self.client.native_followed_artists(read.session()?).await;
                self.check_account_read(&mut read)?;
                let after = after?;
                verify_delta(&before, &after, id, subscribed)?;
                after.version
            } else {
                before.version
            };
            Ok(SubscriptionResult {
                resource_ref: reference.clone(),
                subscribed,
                extensions: Extensions::from([
                    ("changed".into(), json!(changed)),
                    ("backend".into(), json!("standard_followed_singers")),
                    (
                        "verified_by".into(),
                        json!("complete_standard_followed_singers_delta"),
                    ),
                    ("source_version".into(), json!(version)),
                    (
                        "write_requests_dispatched".into(),
                        json!(u8::from(dispatched)),
                    ),
                    ("atomic".into(), json!(false)),
                ]),
            })
        }
        .await;
        self.finish_account_read(read, outcome)
            .map_err(|e| unconfirmed(e, dispatched, &reference))
    }
}

#[cfg(test)]
mod tests;
