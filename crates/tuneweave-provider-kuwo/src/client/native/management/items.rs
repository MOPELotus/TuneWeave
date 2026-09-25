//! Native membership changes, confirmed against complete ordered playlist reads.
use super::*;
use crate::client::native::playlist::Snapshot;
use tuneweave_core::{
    PlaylistItemKind, PlaylistItemMutationAction, PlaylistItemMutationRequest,
    PlaylistItemMutationResult,
};

#[cfg(test)]
pub(crate) mod tests;

pub(crate) struct Change<'a> {
    pub(crate) target: Target<'a>,
    pub(crate) action: PlaylistItemMutationAction,
    pub(crate) request: &'a PlaylistItemMutationRequest,
}
#[derive(Clone, Copy)]
pub(crate) enum Target<'a> {
    Created(&'a str),
    Favorite,
}
impl<'a> Target<'a> {
    pub(super) fn section(self) -> Section {
        match self {
            Self::Created(_) => Section::Created,
            Self::Favorite => Section::Favorite,
        }
    }
    fn id(self) -> Option<&'a str> {
        match self {
            Self::Created(id) => Some(id),
            Self::Favorite => None,
        }
    }
}
impl Change<'_> {
    pub(super) fn validate(&self) -> Result<()> {
        if let Some(id) = self.target.id() {
            playlist::validate_id(id)?;
        }
        if self.request.kind != PlaylistItemKind::Track {
            return Err(TuneWeaveError::new(
                ErrorCode::CapabilityNotSupported,
                "Kuwo native playlist mutations currently support tracks",
            )
            .with_platform(Platform::Kuwo));
        }
        if !(1..=100).contains(&self.request.item_refs.len()) {
            return Err(kuwo_invalid_request(
                "Kuwo item changes require 1 to 100 references",
            ));
        }
        let mut seen = std::collections::BTreeSet::new();
        for reference in &self.request.item_refs {
            if reference.platform() != Platform::Kuwo || !seen.insert(reference.id()) {
                return Err(kuwo_invalid_request(
                    "Kuwo item references must be unique Kuwo references",
                ));
            }
            playlist::validate_id(reference.id())?;
        }
        Ok(())
    }
}

impl KuwoClient {
    /// Ensures tracks are present or absent in an ordinary owned playlist.
    /// Existing additions are left in place; removal addresses every occurrence.
    /// Native insertion order is observed, not forced by additional sort writes.
    pub async fn native_mutate_playlist_items(
        &self,
        credential: &ProviderCredential,
        id: &str,
        action: PlaylistItemMutationAction,
        request: &PlaylistItemMutationRequest,
    ) -> Result<PlaylistItemMutationResult> {
        match self
            .native_mutation(
                credential,
                Mutation::Items(Change {
                    target: Target::Created(id),
                    action,
                    request,
                }),
            )
            .await?
        {
            Outcome::Items(value) => Ok(*value),
            _ => unreachable!("item mutation operation"),
        }
    }

    pub(super) async fn perform_native_item_mutation(
        &self,
        input: &KuwoNativeSessionInput,
        change: &Change<'_>,
        dispatched: &mut bool,
        check: &mut (impl FnMut() -> Result<()> + Send),
    ) -> Result<PlaylistItemMutationResult> {
        let before = self
            .fetch_native_account_playlist(
                input,
                change.target.id(),
                Some(change.target.section()),
                &mut *check,
            )
            .await
            .map_err(|e| {
                if matches!(change.target, Target::Created(_))
                    && e.code == ErrorCode::ResourceNotFound
                {
                    TuneWeaveError::new(ErrorCode::PermissionDenied,
                    "Kuwo item changes require an ordinary playlist owned by the selected account")
                    .with_platform(Platform::Kuwo)
                } else {
                    e
                }
            })?;
        if !stable_metadata(change.target, &before, &before) {
            return Err(unconfirmed());
        }
        let id = before.playlist.id.clone();
        let before_ids = ids(&before);
        // The official remove-by-reference path expands all matching occurrences
        // before serializing data[]. Do not collapse these repeated cloud RIDs.
        let payload_ids: Vec<&str> = change
            .request
            .item_refs
            .iter()
            .flat_map(|r| match change.action {
                PlaylistItemMutationAction::Add => {
                    if before_ids.contains(&r.id()) {
                        Vec::new()
                    } else {
                        vec![r.id()]
                    }
                }
                PlaylistItemMutationAction::Remove => before_ids
                    .iter()
                    .copied()
                    .filter(|id| *id == r.id())
                    .collect(),
            })
            .collect();
        if payload_ids.is_empty() {
            check()?;
            return Ok(result(change, before, input.user_id(), false, 0, false));
        }
        if change.action == PlaylistItemMutationAction::Add {
            if before.detail.as_ref().is_some_and(|detail| detail.online) {
                return Err(TuneWeaveError::new(
                    ErrorCode::CapabilityNotSupported,
                    "Adding to a published Kuwo playlist requires contribution management",
                )
                .with_platform(Platform::Kuwo));
            }
            if before_ids.len() + payload_ids.len() > playlist::MAX_TRACKS {
                return Err(kuwo_invalid_request(
                    "Kuwo item change exceeds the complete-read track limit",
                ));
            }
        }
        if change.action == PlaylistItemMutationAction::Remove
            && before.detail.as_ref().is_some_and(|detail| detail.online)
            && before_ids.len().saturating_sub(payload_ids.len()) < 10
        {
            // The official removal UI warns that an approved list goes offline
            // below ten cloud songs. Removal expands every matching occurrence.
            return Err(TuneWeaveError::new(
                ErrorCode::CapabilityNotSupported,
                "Removing these tracks from a published Kuwo playlist requires explicit delisting management",
            ).with_platform(Platform::Kuwo));
        }
        let numbers = payload_ids
            .iter()
            .map(|id| id.parse::<i64>().map_err(|_| unconfirmed()))
            .collect::<Result<Vec<_>>>()?;
        let operation = match change.action {
            PlaylistItemMutationAction::Add => "pl3_add",
            PlaylistItemMutationAction::Remove => "pl3_delete",
        };
        check()?;
        let ack = self
            .native_cloud_write(
                input,
                operation,
                json!({"pid":id.parse::<i64>().map_err(|_| unconfirmed())?,"data":numbers}),
                dispatched,
            )
            .await;
        check()?;
        let ack = ack?;
        if ack.pid.as_deref().is_some_and(|value| value != id) {
            return Err(unconfirmed());
        }
        let after = self
            .fetch_native_account_playlist(
                input,
                Some(&id),
                Some(change.target.section()),
                &mut *check,
            )
            .await?;
        let after_ids = ids(&after);
        let affected: std::collections::BTreeSet<&str> = payload_ids.iter().copied().collect();
        let ordered_unchanged = match change.action {
            PlaylistItemMutationAction::Add => {
                after_ids.len() == before_ids.len() + payload_ids.len()
                    && affected
                        .iter()
                        .all(|id| after_ids.iter().filter(|v| *v == id).count() == 1)
                    && after_ids
                        .iter()
                        .copied()
                        .filter(|id| !affected.contains(id))
                        .collect::<Vec<_>>()
                        == before_ids
            }
            PlaylistItemMutationAction::Remove => {
                after_ids
                    == before_ids
                        .iter()
                        .copied()
                        .filter(|id| !affected.contains(id))
                        .collect::<Vec<_>>()
            }
        };
        if !ordered_unchanged || !stable_metadata(change.target, &before, &after) {
            return Err(unconfirmed());
        }
        let cover_changed = cover_changed(&before, &after);
        check()?;
        Ok(result(
            change,
            after,
            input.user_id(),
            true,
            payload_ids.len(),
            cover_changed,
        ))
    }
}

fn ids(snapshot: &Snapshot) -> Vec<&str> {
    snapshot.tracks.iter().map(|t| t.id.as_str()).collect()
}
pub(super) fn cover_changed(before: &Snapshot, after: &Snapshot) -> bool {
    before.playlist.cover_url != after.playlist.cover_url
        || match (&before.detail, &after.detail) {
            (Some(a), Some(b)) => a.small_pic != b.small_pic || a.big_pic != b.big_pic,
            _ => false,
        }
}
pub(super) fn stable_metadata(target: Target<'_>, before: &Snapshot, after: &Snapshot) -> bool {
    if matches!(target, Target::Favorite) {
        return before.detail.is_none()
            && after.detail.is_none()
            && before.playlist.resource_ref == after.playlist.resource_ref
            && before.playlist.name == after.playlist.name
            && before.playlist.description == after.playlist.description
            && before.playlist.extensions.get("library_section") == Some(&json!("favorite"))
            && before.playlist.extensions.get("upstream_type") == Some(&json!("MYFAVORITE"))
            && before.playlist.extensions.get("is_favorite") == Some(&json!(true))
            && [
                "owner_id",
                "library_owner_id",
                "library_section",
                "upstream_type",
                "is_favorite",
                "is_public",
                "turn",
            ]
            .iter()
            .all(|k| before.playlist.extensions.get(*k) == after.playlist.extensions.get(*k));
    }
    let (Some(a), Some(b)) = (&before.detail, &after.detail) else {
        return false;
    };
    before.playlist.resource_ref == after.playlist.resource_ref
        && ["owner_id", "library_section", "is_public"]
            .iter()
            .all(|k| before.playlist.extensions.get(*k) == after.playlist.extensions.get(*k))
        && a.name == b.name
        && a.description == b.description
        && a.tags == b.tags
        && a.tag_ids == b.tag_ids
        && a.online == b.online
        && a.playlist_type == b.playlist_type
}
fn result(
    change: &Change<'_>,
    after: Snapshot,
    owner: &str,
    changed: bool,
    sent_occurrences: usize,
    cover_changed: bool,
) -> PlaylistItemMutationResult {
    PlaylistItemMutationResult {
        playlist_ref: after.playlist.resource_ref,
        item_refs: change.request.item_refs.clone(),
        kind: PlaylistItemKind::Track,
        action: change.action,
        snapshot_id: after
            .playlist
            .extensions
            .get("source_snapshot_id")
            .and_then(|v| v.as_str())
            .map(str::to_owned),
        cloud_track_count: Some(after.tracks.len() as u64),
        extensions: Extensions::from([
            ("backend".into(), json!("native_account_library")),
            ("library_owner_id".into(), json!(owner)),
            ("confirmed".into(), json!(true)),
            ("changed".into(), json!(changed)),
            (
                "write_requests_dispatched".into(),
                json!(usize::from(changed)),
            ),
            ("sent_occurrences".into(), json!(sent_occurrences)),
            ("cover_changed".into(), json!(cover_changed)),
            ("atomic".into(), json!(false)),
            (
                "order".into(),
                json!(match change.action {
                    PlaylistItemMutationAction::Add => "platform_native_insertion",
                    PlaylistItemMutationAction::Remove => "remaining_order_preserved",
                }),
            ),
            (
                "consistency".into(),
                json!(if changed {
                    "complete_ordered_reads_before_and_after_write"
                } else {
                    "complete_ordered_read_no_write"
                }),
            ),
        ]),
    }
}
