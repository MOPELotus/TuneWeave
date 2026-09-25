//! Track subscriptions target the unique MYFAVORITE cloud list, never its name alone.
use super::*;
use tuneweave_core::{
    PlaylistItemKind, PlaylistItemMutationAction, PlaylistItemMutationRequest,
    PlaylistItemMutationResult, SubscriptionResult,
};

#[cfg(test)]
pub(crate) mod tests;

pub(crate) fn request(
    id: &str,
    subscribed: bool,
    account: Option<&str>,
) -> Result<(PlaylistItemMutationAction, PlaylistItemMutationRequest)> {
    playlist::validate_id(id)?;
    Ok((
        if subscribed {
            PlaylistItemMutationAction::Add
        } else {
            PlaylistItemMutationAction::Remove
        },
        PlaylistItemMutationRequest {
            item_refs: vec![
                ResourceRef::new(Platform::Kuwo, id)
                    .map_err(|_| kuwo_invalid_request("Kuwo track reference is invalid"))?,
            ],
            kind: PlaylistItemKind::Track,
            account: account.map(str::to_owned),
        },
    ))
}

pub(crate) fn result(
    reference: ResourceRef,
    value: PlaylistItemMutationResult,
) -> SubscriptionResult {
    let mut extensions = value.extensions;
    extensions.extend([
        ("favorite_playlist_ref".into(), json!(value.playlist_ref)),
        ("source_snapshot_id".into(), json!(value.snapshot_id)),
        ("cloud_track_count".into(), json!(value.cloud_track_count)),
    ]);
    SubscriptionResult {
        resource_ref: reference,
        subscribed: value.action == PlaylistItemMutationAction::Add,
        extensions,
    }
}

impl KuwoClient {
    /// Confirms presence or absence in the selected account's real favorites list.
    /// A missing list is not created; cancellation cannot undo an upstream write.
    pub async fn native_set_track_subscription(
        &self,
        credential: &ProviderCredential,
        id: &str,
        subscribed: bool,
    ) -> Result<SubscriptionResult> {
        let (action, request) = request(id, subscribed, None)?;
        match self
            .native_mutation(
                credential,
                Mutation::Items(items::Change {
                    target: items::Target::Favorite,
                    action,
                    request: &request,
                }),
            )
            .await?
        {
            Outcome::Items(value) => Ok(result(request.item_refs[0].clone(), *value)),
            _ => unreachable!("favorite item mutation"),
        }
    }
}
