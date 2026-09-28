use super::*;
use crate::credential::{MiguCredential, authentication_required};
use tuneweave_core::{StoredAccountCredential, SubscriptionResult};

impl MiguProvider {
    async fn favorite_before_write(
        &self,
        alias: &str,
        current: &mut MiguCredential,
        stored: &mut Option<StoredAccountCredential>,
    ) -> Result<Option<String>> {
        let token = current.token().to_owned();
        self.verify_account_step(alias, current, stored, &token)
            .await?;
        let response = self
            .client
            .account_favorite_id_for_write(current.token(), current.user_id())
            .await?;
        let Some(id) = self
            .accept_playlist_read(alias, current, stored, response)
            .await?
        else {
            // The single-track like endpoints do not take a playlist ID. If the
            // home page omits it, use the account-scoped explicit state endpoint
            // for pre- and post-write confirmation rather than guessing an ID.
            return Ok(None);
        };
        let response = self
            .client
            .account_playlist_detail(&id, current.token(), current.user_id())
            .await?;
        let playlist = self
            .accept_playlist_read(alias, current, stored, response)
            .await?;
        if playlist
            .extensions
            .get("owner_id")
            .and_then(serde_json::Value::as_str)
            != Some(current.user_id())
        {
            return Err(migu_upstream_error(
                "Migu favorite playlist does not belong to the selected user",
            ));
        }
        Ok(Some(id))
    }

    async fn read_favorite_track_state(
        &self,
        id: &str,
        alias: &str,
        current: &mut MiguCredential,
        stored: &mut Option<StoredAccountCredential>,
    ) -> Result<bool> {
        let response = self
            .client
            .account_favorite_state(id, current.token(), current.user_id())
            .await?;
        self.accept_playlist_read(alias, current, stored, response)
            .await
    }

    pub(super) async fn set_favorite_track(
        &self,
        id: &str,
        subscribed: bool,
        account: Option<&str>,
    ) -> Result<SubscriptionResult> {
        parse_content_id(id)?;
        let alias = account.unwrap_or("default");
        let (mut current, mut stored) =
            self.selected(alias)?.ok_or_else(authentication_required)?;
        let original = current.clone();
        let mut write_started = false;
        let result = async {
            let favorite_id = self
                .favorite_before_write(alias, &mut current, &mut stored)
                .await?;
            if favorite_id.is_none()
                && self
                    .read_favorite_track_state(id, alias, &mut current, &mut stored)
                    .await?
                    == subscribed
            {
                self.accept_read(&current, stored.as_ref(), &current)?;
                return favorite_subscription_result(id, subscribed, &current, None, false);
            }
            self.accept_read(&current, stored.as_ref(), &current)?;
            write_started = true;
            let response = self
                .client
                .set_account_favorite(id, subscribed, current.token(), current.user_id())
                .await?;
            self.accept_playlist_read(alias, &mut current, &mut stored, response)
                .await?;
            if let Some(favorite_id) = favorite_id.as_deref() {
                // Continue with the original account chain: never select another
                // login between the mutation and its complete collection readback.
                let snapshot = self
                    .read_selected_account_playlist(None, alias, &mut current, &mut stored)
                    .await?;
                if snapshot.playlist.id != favorite_id || snapshot.contains_track(id) != subscribed
                {
                    return Err(migu_upstream_error(
                        "Migu favorite collection did not confirm the requested state",
                    ));
                }
            }
            let state = self
                .read_favorite_track_state(id, alias, &mut current, &mut stored)
                .await?;
            if state != subscribed {
                return Err(migu_upstream_error(
                    "Migu explicit favorite state disagrees with the requested state",
                ));
            }
            self.accept_read(&current, stored.as_ref(), &current)?;
            favorite_subscription_result(id, subscribed, &current, favorite_id.as_deref(), true)
        }
        .await;
        self.finish_account_read(&original, &current, stored.as_ref(), result)
            .map_err(|error| {
                if write_started {
                    let mut details = error.details.as_object().cloned().unwrap_or_default();
                    details.insert("operation".into(), json!("track_subscription"));
                    details.insert("write_outcome".into(), json!("unconfirmed"));
                    error
                        .retryable(false)
                        .with_details(serde_json::Value::Object(details))
                } else {
                    error
                }
            })
    }
}

fn favorite_subscription_result(
    id: &str,
    subscribed: bool,
    current: &MiguCredential,
    favorite_id: Option<&str>,
    write_performed: bool,
) -> Result<SubscriptionResult> {
    let mut extensions = Extensions::from([
        ("backend".to_owned(), json!("official_pc_track_collection")),
        (
            "verified_by".to_owned(),
            json!(if favorite_id.is_some() {
                "complete_favorite_playlist_and_explicit_track_state"
            } else {
                "explicit_single_track_state"
            }),
        ),
        ("write_performed".to_owned(), json!(write_performed)),
        ("source_user_id".to_owned(), json!(current.user_id())),
    ]);
    if let Some(favorite_id) = favorite_id {
        extensions.insert(
            "favorite_playlist_ref".to_owned(),
            json!(
                tuneweave_core::ResourceRef::new(Platform::Migu, favorite_id)
                    .map_err(|_| migu_invalid_request("Migu favorite playlist ID is invalid"))?
            ),
        );
    }
    Ok(SubscriptionResult {
        resource_ref: tuneweave_core::ResourceRef::new(Platform::Migu, id)
            .map_err(|_| migu_invalid_request("Migu content ID is invalid"))?,
        subscribed,
        extensions,
    })
}

#[cfg(test)]
pub(super) mod tests;
