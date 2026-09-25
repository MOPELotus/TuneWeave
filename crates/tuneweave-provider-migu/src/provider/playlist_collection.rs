use super::*;
use crate::client::library::Section;
use crate::credential::authentication_required;
use tuneweave_core::SubscriptionResult;

impl MiguProvider {
    pub(super) async fn set_collected_playlist(
        &self,
        id: &str,
        subscribed: bool,
        account: Option<&str>,
    ) -> Result<SubscriptionResult> {
        parse_playlist_id(id)?;
        let alias = account.unwrap_or("default");
        let (mut current, mut stored) =
            self.selected(alias)?.ok_or_else(authentication_required)?;
        let original = current.clone();
        let mut write_started = false;
        let result = async {
            self.verify_account_step(alias, &mut current, &mut stored, original.token())
                .await?;
            let title = if subscribed {
                let response = self
                    .client
                    .account_playlist_detail(id, current.token(), current.user_id())
                    .await?;
                self.accept_playlist_read(alias, &mut current, &mut stored, response)
                    .await?
                    .name
            } else {
                // Removing a saved entry must also work when its public playlist
                // has been deleted or its metadata is no longer accessible.
                String::new()
            };
            self.accept_read(&current, stored.as_ref(), &current)?;
            write_started = true;
            let response = self
                .client
                .set_account_playlist_collection(
                    id,
                    &title,
                    subscribed,
                    current.token(),
                    current.user_id(),
                )
                .await?;
            self.accept_playlist_read(alias, &mut current, &mut stored, response)
                .await?;
            let saved = self
                .read_library_section(Section::Saved, alias, &mut current, &mut stored)
                .await?;
            if saved.iter().any(|playlist| playlist.id == id) != subscribed {
                return Err(migu_upstream_error(
                    "Migu saved library did not confirm the requested collection state",
                ));
            }
            let response = self
                .client
                .account_playlist_collection_state(id, current.token(), current.user_id())
                .await?;
            let state = self
                .accept_playlist_read(alias, &mut current, &mut stored, response)
                .await?;
            if state != subscribed {
                return Err(migu_upstream_error(
                    "Migu explicit collection state disagrees with the saved library",
                ));
            }
            self.accept_read(&current, stored.as_ref(), &current)?;
            Ok(SubscriptionResult {
                resource_ref: tuneweave_core::ResourceRef::new(Platform::Migu, id)
                    .map_err(|_| migu_invalid_request("Migu playlist ID is invalid"))?,
                subscribed,
                extensions: Extensions::from([
                    ("backend".into(), json!("official_pc_playlist_collection")),
                    (
                        "verified_by".into(),
                        json!("complete_saved_library_and_explicit_collection_state"),
                    ),
                    ("source_user_id".into(), json!(current.user_id())),
                    ("resource_type".into(), json!("2021")),
                ]),
            })
        }
        .await;
        self.finish_account_read(&original, &current, stored.as_ref(), result)
            .map_err(|error| {
                if write_started {
                    let mut details = error.details.as_object().cloned().unwrap_or_default();
                    details.insert("operation".into(), json!("playlist_subscription"));
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

#[cfg(test)]
mod tests;
