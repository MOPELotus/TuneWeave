use super::*;
use crate::client::native::library::validate_request;
use std::time::Duration;
use tuneweave_core::{Artist, ResourceRef, SubscriptionResult};

#[cfg(test)]
mod tests;

const ARTIST_SUBSCRIPTION_DEADLINE: Duration = Duration::from_secs(120);

impl KuwoProvider {
    pub(in crate::provider) async fn read_following_artists(
        &self,
        uid: Option<&str>,
        request: &PageRequest,
    ) -> Result<Page<Artist>> {
        validate_request(request)?;
        let account = request.account.as_deref().unwrap_or("default");
        let selected = self
            .selected(account)?
            .ok_or_else(authentication_required)?;
        let input = selected.credential.input()?;
        if uid.is_some_and(|uid| uid != input.user_id()) {
            return Err(TuneWeaveError::new(
                ErrorCode::PermissionDenied,
                "Kuwo followed artists are available only for the selected account",
            )
            .with_platform(Platform::Kuwo));
        }
        crate::client::native::validate_session_metadata(&input)?;
        let validation = self.client.validate_native_session(&input).await;
        self.finish_selected(account, &selected, validation)?;
        let result = self
            .client
            .fetch_following_artists(&input, request, || self.check_selection(account, &selected))
            .await;
        self.finish_selected(account, &selected, result)
    }

    pub(in crate::provider) async fn set_following_artist(
        &self,
        id: &str,
        subscribed: bool,
        account: Option<&str>,
    ) -> Result<SubscriptionResult> {
        crate::client::native::following_artists::validate_artist_id(id)?;
        let account = account.unwrap_or("default");
        let selected = self
            .selected(account)?
            .ok_or_else(authentication_required)?;
        let input = selected.credential.input()?;
        crate::client::native::validate_session_metadata(&input)?;
        let mut dispatched = false;

        let outcome = tokio::time::timeout(ARTIST_SUBSCRIPTION_DEADLINE, async {
            let validation = self.client.validate_native_session(&input).await;
            self.finish_selected(account, &selected, validation)?;

            let before = self
                .client
                .fetch_following_artist_snapshot(&input, || {
                    self.check_selection(account, &selected)
                })
                .await;
            let before = self.finish_selected(account, &selected, before)?;
            let already_subscribed = before.iter().any(|artist| artist.id == id);

            if already_subscribed != subscribed {
                let ack = self
                    .client
                    .set_following_artist(
                        &input,
                        id,
                        subscribed,
                        &mut dispatched,
                        || self.check_selection(account, &selected),
                    )
                    .await;
                self.finish_selected(account, &selected, ack)?;

                let after = self
                    .client
                    .fetch_following_artist_snapshot(&input, || {
                        self.check_selection(account, &selected)
                    })
                    .await;
                let after = self.finish_selected(account, &selected, after)?;
                let target_matches = after.iter().any(|artist| artist.id == id) == subscribed;
                let before_others = before
                    .iter()
                    .filter(|artist| artist.id != id)
                    .map(|artist| artist.id.as_str());
                let after_others = after
                    .iter()
                    .filter(|artist| artist.id != id)
                    .map(|artist| artist.id.as_str());
                if !target_matches || !before_others.eq(after_others) {
                    return Err(TuneWeaveError::new(
                        ErrorCode::Conflict,
                        "Kuwo artist subscription readback did not confirm the single requested change",
                    )
                    .with_platform(Platform::Kuwo));
                }
            }
            Ok(SubscriptionResult {
                resource_ref: ResourceRef::new(Platform::Kuwo, id)
                    .map_err(|_| kuwo_invalid_request("Kuwo artist reference is invalid"))?,
                subscribed,
                extensions: Extensions::from([
                    (
                        "backend".into(),
                        json!("official_native_artist_subscription"),
                    ),
                    ("source_user_id".into(), json!(input.user_id())),
                    ("write_performed".into(), json!(dispatched)),
                    (
                        "write_requests_dispatched".into(),
                        json!(u8::from(dispatched)),
                    ),
                    (
                        "verified_by".into(),
                        json!("complete_selected_account_directory_read"),
                    ),
                    ("atomic".into(), json!(false)),
                ]),
            })
        })
        .await;

        let result = match outcome {
            Ok(result) => result,
            Err(_) => self.check_selection(account, &selected).and_then(|()| {
                Err(TuneWeaveError::new(
                    ErrorCode::UpstreamTimeout,
                    "Kuwo artist subscription exceeded the total deadline",
                )
                .with_platform(Platform::Kuwo))
            }),
        };
        result.map_err(|mut error| {
            if dispatched {
                let mut details = error.details.as_object().cloned().unwrap_or_default();
                details.insert("operation".into(), json!("artist_subscription"));
                details.insert("write_requests_dispatched".into(), json!(1));
                details.insert("write_outcome".into(), json!("unconfirmed"));
                details.insert("automatic_retry".into(), json!(false));
                error = error
                    .retryable(false)
                    .with_details(serde_json::Value::Object(details));
            }
            error
        })
    }
}
