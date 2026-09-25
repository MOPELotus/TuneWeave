//! Complete track-reference permutations for ordinary and MYFAVORITE lists.
use super::*;
use crate::client::native::playlist::Snapshot;
use tuneweave_core::{PlaylistTrackOrderRequest, PlaylistTrackOrderResult};

#[cfg(test)]
pub(crate) mod tests;

pub(super) fn validate(id: &str, request: &PlaylistTrackOrderRequest) -> Result<()> {
    playlist::validate_id(id)?;
    if !(1..=playlist::MAX_TRACKS).contains(&request.track_refs.len()) {
        return Err(invalid_order());
    }
    for reference in &request.track_refs {
        if reference.platform() != Platform::Kuwo {
            return Err(invalid_order());
        }
        playlist::validate_id(reference.id())?;
    }
    Ok(())
}

impl KuwoClient {
    /// Reorders every track reference, retaining the count of each repeated RID.
    /// A confirmed result reports observed order, not an atomic upstream version.
    pub async fn native_reorder_playlist_tracks(
        &self,
        credential: &ProviderCredential,
        id: &str,
        request: &PlaylistTrackOrderRequest,
    ) -> Result<PlaylistTrackOrderResult> {
        match self
            .native_mutation(credential, Mutation::Sort(id, request))
            .await?
        {
            Outcome::TrackOrder(value) => Ok(*value),
            _ => unreachable!("track order mutation"),
        }
    }

    pub(super) async fn perform_native_track_sort(
        &self,
        input: &KuwoNativeSessionInput,
        id: &str,
        request: &PlaylistTrackOrderRequest,
        dispatched: &mut bool,
        check: &mut (impl FnMut() -> Result<()> + Send),
    ) -> Result<PlaylistTrackOrderResult> {
        // Owned selects Created or the actual Favorite; it never searches Saved.
        let before = self.fetch_native_account_playlist(input, Some(id), Some(Section::Owned), &mut *check)
            .await.map_err(|e| if e.code == ErrorCode::ResourceNotFound {
                TuneWeaveError::new(ErrorCode::PermissionDenied, "Kuwo sorting requires an ordinary or favorite list owned by the selected account").with_platform(Platform::Kuwo)
            } else { e })?;
        let target = match before
            .playlist
            .extensions
            .get("library_section")
            .and_then(|v| v.as_str())
        {
            Some("created") => items::Target::Created(id),
            Some("favorite") => items::Target::Favorite,
            _ => return Err(unconfirmed()),
        };
        if !items::stable_metadata(target, &before, &before) {
            return Err(unconfirmed());
        }
        let original: Vec<&str> = before.tracks.iter().map(|t| t.id.as_str()).collect();
        let wanted: Vec<&str> = request.track_refs.iter().map(ResourceRef::id).collect();
        let mut original_counts = original.clone();
        let mut wanted_counts = wanted.clone();
        original_counts.sort_unstable();
        wanted_counts.sort_unstable();
        if original_counts != wanted_counts {
            return Err(invalid_order());
        }
        if original == wanted {
            check()?;
            return Ok(result(before, request, false, false));
        }
        // IDs alone cannot detect metadata loss or changes among duplicate rows.
        let original_tracks = track_multiset(&before)?;
        let numbers = wanted
            .iter()
            .map(|id| id.parse::<i64>().map_err(|_| invalid_order()))
            .collect::<Result<Vec<_>>>()?;
        check()?;
        let ack = self
            .native_cloud_write(
                input,
                "pl3_sort",
                json!({"pid":id.parse::<i64>().map_err(|_| invalid_order())?,"data":numbers}),
                dispatched,
            )
            .await;
        check()?;
        let ack = ack?;
        if ack.pid.as_deref().is_some_and(|value| value != id) {
            return Err(unconfirmed());
        }
        let after = self
            .fetch_native_account_playlist(input, Some(id), Some(target.section()), &mut *check)
            .await?;
        if after
            .tracks
            .iter()
            .map(|t| t.id.as_str())
            .ne(wanted.iter().copied())
            || !items::stable_metadata(target, &before, &after)
            || track_multiset(&after)? != original_tracks
        {
            return Err(unconfirmed());
        }
        let cover_changed = items::cover_changed(&before, &after);
        check()?;
        Ok(result(after, request, true, cover_changed))
    }
}

fn track_multiset(snapshot: &Snapshot) -> Result<BTreeMap<Vec<u8>, usize>> {
    let mut rows = BTreeMap::new();
    for track in &snapshot.tracks {
        let key = serde_json::to_vec(track).map_err(|_| unconfirmed())?;
        *rows.entry(key).or_default() += 1;
    }
    Ok(rows)
}
fn result(
    after: Snapshot,
    request: &PlaylistTrackOrderRequest,
    changed: bool,
    cover_changed: bool,
) -> PlaylistTrackOrderResult {
    PlaylistTrackOrderResult {
        playlist_ref: after.playlist.resource_ref,
        track_refs: request.track_refs.clone(),
        snapshot_id: after
            .playlist
            .extensions
            .get("source_snapshot_id")
            .and_then(|v| v.as_str())
            .map(str::to_owned),
        extensions: Extensions::from([
            ("backend".into(), json!("native_account_library")),
            (
                "library_owner_id".into(),
                after.playlist.extensions["library_owner_id"].clone(),
            ),
            (
                "library_section".into(),
                after.playlist.extensions["library_section"].clone(),
            ),
            ("cloud_track_count".into(), json!(after.tracks.len())),
            ("confirmed".into(), json!(true)),
            ("changed".into(), json!(changed)),
            (
                "write_requests_dispatched".into(),
                json!(usize::from(changed)),
            ),
            ("cover_changed".into(), json!(cover_changed)),
            ("atomic".into(), json!(false)),
            (
                "ordering".into(),
                json!("requested_track_reference_sequence"),
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
fn invalid_order() -> TuneWeaveError {
    kuwo_invalid_request(
        "Kuwo sorting requires 1 to 10000 track references forming a complete permutation, including every duplicate",
    )
}
