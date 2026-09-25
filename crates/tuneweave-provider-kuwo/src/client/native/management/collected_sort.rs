//! Complete ordering of collected playlists, without inferring scope from their IDs.
use super::*;
use tuneweave_core::{PlaylistOrderRequest, PlaylistOrderResult};

#[cfg(test)]
pub(crate) mod tests;

pub(super) fn validate(request: &PlaylistOrderRequest) -> Result<()> {
    if !(1..=library::MAX_SAVED).contains(&request.playlist_refs.len()) {
        return Err(invalid_order());
    }
    let mut seen = std::collections::BTreeSet::new();
    for reference in &request.playlist_refs {
        if reference.platform() != Platform::Kuwo || !seen.insert(reference.id()) {
            return Err(invalid_order());
        }
        playlist::validate_id(reference.id())?;
    }
    Ok(())
}

impl KuwoClient {
    /// Reorders every collected playlist and confirms the full directory readback.
    /// Self-created playlists with the same IDs remain a separate directory.
    pub async fn native_reorder_collected_playlists(
        &self,
        credential: &ProviderCredential,
        request: &PlaylistOrderRequest,
    ) -> Result<PlaylistOrderResult> {
        match self
            .native_mutation(credential, Mutation::CollectedSort(request))
            .await?
        {
            Outcome::LibraryOrder(value) => Ok(value),
            _ => unreachable!("collected playlist order mutation"),
        }
    }

    pub(super) async fn perform_native_collected_sort(
        &self,
        input: &KuwoNativeSessionInput,
        request: &PlaylistOrderRequest,
        dispatched: &mut bool,
        check: &mut (impl FnMut() -> Result<()> + Send),
    ) -> Result<PlaylistOrderResult> {
        let (before, _) = self
            .native_library_items(input, Some(Section::Saved), check)
            .await?;
        if before.len() != request.playlist_refs.len() {
            return Err(invalid_order());
        }
        let by_id: std::collections::BTreeMap<_, _> =
            before.iter().map(|p| (p.id.as_str(), p)).collect();
        let expected = request
            .playlist_refs
            .iter()
            .map(|r| by_id.get(r.id()).copied().ok_or_else(invalid_order))
            .collect::<Result<Vec<_>>>()?;
        if before.iter().eq(expected.iter().copied()) {
            return Ok(result(request, input.user_id(), false));
        }
        let ids = request
            .playlist_refs
            .iter()
            .map(|r| r.id().parse::<i64>().map_err(|_| invalid_order()))
            .collect::<Result<Vec<_>>>()?;
        check()?;
        let ack = self
            .native_cloud_write(input, "pl3_sortfavorlist", json!({"data":ids}), dispatched)
            .await;
        check()?;
        // Never copy the official callback's optInt-on-empty-object default-zero success.
        ack?;
        let (after, _) = self
            .native_library_items(input, Some(Section::Saved), check)
            .await?;
        if !after.iter().eq(expected.iter().copied()) {
            return Err(unconfirmed());
        }
        Ok(result(request, input.user_id(), true))
    }
}

fn result(request: &PlaylistOrderRequest, owner: &str, changed: bool) -> PlaylistOrderResult {
    PlaylistOrderResult {
        playlist_refs: request.playlist_refs.clone(),
        extensions: Extensions::from([
            ("backend".into(), json!("native_account_library")),
            ("library_owner_id".into(), json!(owner)),
            ("library_section".into(), json!("collected")),
            ("confirmed".into(), json!(true)),
            ("changed".into(), json!(changed)),
            (
                "write_requests_dispatched".into(),
                json!(usize::from(changed)),
            ),
            ("ordering".into(), json!("complete_collected_directory")),
            ("atomic".into(), json!(false)),
            (
                "consistency".into(),
                json!(if changed {
                    "write_ack_and_complete_directory_readback"
                } else {
                    "complete_directory_read_no_write"
                }),
            ),
        ]),
    }
}
fn invalid_order() -> TuneWeaveError {
    kuwo_invalid_request(
        "Kuwo collected playlist ordering requires 1 to 1999 unique references covering the complete collection",
    )
}
