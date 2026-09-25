//! Canonical positions for every ordinary self-created playlist.
use super::*;
use library::dto::ordering::Snapshot;
use tuneweave_core::{PlaylistOrderRequest, PlaylistOrderResult};

#[cfg(test)]
pub(crate) mod tests;

pub(super) fn validate(request: &PlaylistOrderRequest) -> Result<()> {
    if !(1..=library::MAX_OWNED).contains(&request.playlist_refs.len()) {
        return Err(invalid_order());
    }
    let mut seen = std::collections::BTreeSet::new();
    for r in &request.playlist_refs {
        if r.platform() != Platform::Kuwo || !seen.insert(r.id()) {
            return Err(invalid_order());
        }
        playlist::validate_id(r.id())?;
    }
    Ok(())
}

impl KuwoClient {
    /// Reorders the complete ordinary created directory; system and collected
    /// lists are excluded. Success is confirmed using explicit upstream positions.
    pub async fn native_reorder_account_playlists(
        &self,
        credential: &ProviderCredential,
        request: &PlaylistOrderRequest,
    ) -> Result<PlaylistOrderResult> {
        match self
            .native_mutation(credential, Mutation::LibrarySort(request))
            .await?
        {
            Outcome::LibraryOrder(value) => Ok(value),
            _ => unreachable!("library order mutation"),
        }
    }

    pub(super) async fn perform_native_library_sort(
        &self,
        input: &KuwoNativeSessionInput,
        request: &PlaylistOrderRequest,
        dispatched: &mut bool,
        check: &mut (impl FnMut() -> Result<()> + Send),
    ) -> Result<PlaylistOrderResult> {
        check()?;
        let before = self.native_order_directory(input).await;
        check()?;
        let before = before?;
        if before.positions.len() != request.playlist_refs.len()
            || request
                .playlist_refs
                .iter()
                .any(|r| !before.positions.contains_key(r.id()))
        {
            return Err(invalid_order());
        }
        if ordered(&before, request) {
            return Ok(result(request, input.user_id(), false));
        }
        let positions = request
            .playlist_refs
            .iter()
            .enumerate()
            .map(|(i, r)| (r.id().to_owned(), json!(i + 1)))
            .collect::<serde_json::Map<String, serde_json::Value>>();
        check()?;
        let ack = self
            .native_cloud_write(
                input,
                "pl3_sortlist",
                serde_json::Value::Object(positions),
                dispatched,
            )
            .await;
        check()?;
        ack?;
        check()?;
        let after = self.native_order_directory(input).await;
        check()?;
        let after = after?;
        if before.stable_rows != after.stable_rows || !ordered(&after, request) {
            return Err(unconfirmed());
        }
        Ok(result(request, input.user_id(), true))
    }

    async fn native_order_directory(&self, input: &KuwoNativeSessionInput) -> Result<Snapshot> {
        let time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| invalid())?
            .as_millis()
            .to_string();
        let query = library::query(input, Section::Owned, 0, &time)?;
        self.native_get_with_metadata(
            HOST,
            library::OWNED_PATH,
            "native_library_order_directory",
            format!("{}?{query}", self.native_target(HOST, library::OWNED_PATH)),
            Some(session_metadata(input)?),
            |bytes| library::dto::ordering::parse(bytes, input),
        )
        .await
    }
}
fn ordered(snapshot: &Snapshot, request: &PlaylistOrderRequest) -> bool {
    snapshot.positions.len() == request.playlist_refs.len()
        && request
            .playlist_refs
            .iter()
            .enumerate()
            .all(|(i, r)| snapshot.positions.get(r.id()) == Some(&Some((i + 1) as i32)))
}
fn result(request: &PlaylistOrderRequest, owner: &str, changed: bool) -> PlaylistOrderResult {
    PlaylistOrderResult {
        playlist_refs: request.playlist_refs.clone(),
        extensions: Extensions::from([
            ("backend".into(), json!("native_account_library")),
            ("library_owner_id".into(), json!(owner)),
            ("library_section".into(), json!("created")),
            ("confirmed".into(), json!(true)),
            ("changed".into(), json!(changed)),
            (
                "write_requests_dispatched".into(),
                json!(usize::from(changed)),
            ),
            ("ordering".into(), json!("explicit_turn_one_based")),
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
        "Kuwo library ordering requires 1 to 4096 unique references covering every ordinary created playlist",
    )
}
