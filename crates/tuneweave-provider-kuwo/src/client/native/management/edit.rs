use super::*;
#[cfg(test)]
use crate::client::native::playlist::metadata::{METADATA_PATH, parse_metadata};
use crate::client::native::playlist::metadata::{Metadata, validate_tags};
use tuneweave_core::PlaylistMetadataUpdateVariant;

#[cfg(test)]
pub(crate) mod tests;
#[cfg(test)]
pub(crate) mod visibility_tests;

pub(super) enum Edit<'a> {
    Metadata(&'a PlaylistUpdateRequest),
    Visibility(&'a PlaylistVisibilityUpdateRequest),
}

pub(super) fn validate_update(id: &str, r: &PlaylistUpdateRequest) -> Result<()> {
    playlist::validate_id(id)?;
    if r.variant != PlaylistMetadataUpdateVariant::Default {
        return Err(TuneWeaveError::new(
            ErrorCode::CapabilityNotSupported,
            "Kuwo native metadata updates support the default variant",
        )
        .with_platform(Platform::Kuwo));
    }
    if r.name.is_none() && r.description.is_none() && r.tags.is_none() {
        return Err(kuwo_invalid_request("Kuwo playlist update has no fields"));
    }
    if r.name
        .as_ref()
        .is_some_and(|v| v.trim().is_empty() || v.len() > 1024 || v.chars().any(char::is_control))
    {
        return Err(kuwo_invalid_request("Kuwo playlist name is invalid"));
    }
    if r.description.as_ref().is_some_and(|v| {
        v.len() > 16384
            || v.chars()
                .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
    }) {
        return Err(kuwo_invalid_request("Kuwo playlist description is invalid"));
    }
    if let Some(tags) = &r.tags {
        validate_tags(tags)?;
    }
    Ok(())
}

impl KuwoClient {
    /// Changes selected fields on an ordinary owned playlist, preserving other
    /// known metadata. Only a strict acknowledgement and full readback confirm it.
    pub async fn native_update_playlist(
        &self,
        credential: &ProviderCredential,
        id: &str,
        request: &PlaylistUpdateRequest,
    ) -> Result<PlaylistMutationResult> {
        match self
            .native_mutation(credential, Mutation::Update(id, request))
            .await?
        {
            Outcome::Playlist(result) => Ok(*result),
            Outcome::Deleted(_)
            | Outcome::Items(_)
            | Outcome::TrackOrder(_)
            | Outcome::LibraryOrder(_)
            | Outcome::Subscription(_) => {
                unreachable!("update operation")
            }
        }
    }

    /// Explicitly makes an ordinary owned playlist public or private, retaining
    /// its known metadata and confirming the result with metadata and directory reads.
    pub async fn native_update_playlist_visibility(
        &self,
        credential: &ProviderCredential,
        id: &str,
        request: &PlaylistVisibilityUpdateRequest,
    ) -> Result<PlaylistMutationResult> {
        match self
            .native_mutation(credential, Mutation::Visibility(id, request))
            .await?
        {
            Outcome::Playlist(result) => Ok(*result),
            Outcome::Deleted(_)
            | Outcome::Items(_)
            | Outcome::TrackOrder(_)
            | Outcome::LibraryOrder(_)
            | Outcome::Subscription(_) => {
                unreachable!("visibility operation")
            }
        }
    }

    pub(super) async fn perform_native_update(
        &self,
        input: &KuwoNativeSessionInput,
        id: &str,
        request: Edit<'_>,
        before: Vec<Playlist>,
        dispatched: &mut bool,
        check: &mut (impl FnMut() -> Result<()> + Send),
    ) -> Result<Box<PlaylistMutationResult>> {
        let before = ordinary(&before, id)?.clone();
        let metadata = self.checked_playlist_metadata(input, id, check).await?;
        consistent(&before, &metadata)?;
        // Official ordinary edits re-submit published lists for review. Keep that
        // additional write and publication transition out of metadata/visibility edits.
        if metadata.online {
            return Err(TuneWeaveError::new(
                ErrorCode::CapabilityNotSupported,
                "Editing a published Kuwo playlist requires explicit contribution review management",
            )
            .with_platform(Platform::Kuwo));
        }
        // Recheck the selected ordinary list after obtaining its complementary
        // metadata. The upstream has no conditional-write or transaction token.
        check()?;
        let current = self.native_management_directory(input).await;
        check()?;
        let (current, _) = current?;
        let current = ordinary(&current, id)?;
        if !same_directory(&before, current) {
            return Err(unconfirmed());
        }
        let mut expected = metadata.clone();
        let mut public = before
            .extensions
            .get("is_public")
            .and_then(serde_json::Value::as_bool)
            .ok_or_else(invalid)?;
        let mut preserve_tag_ids = true;
        match request {
            Edit::Metadata(r) => {
                if let Some(name) = &r.name {
                    expected.name = name.clone();
                }
                if let Some(description) = &r.description {
                    expected.description = description.clone();
                }
                if let Some(tags) = &r.tags {
                    expected.tags = tags.clone();
                    preserve_tag_ids = false;
                }
            }
            Edit::Visibility(r) => {
                r.validate()?;
                public = r.visibility == PlaylistVisibility::Public;
            }
        }
        let payload = json!({"pid":id.parse::<i64>().map_err(|_|invalid())?,"title":expected.name,
            "intro":expected.description,"tag":expected.tags.join(","),
            "pic":before.cover_url.as_deref().unwrap_or(""),"ispub":public});
        check()?;
        let ack = self
            .native_cloud_write(input, "pl3_editlist", payload, dispatched)
            .await;
        check()?;
        let ack = ack?;
        if ack.pid.as_deref().is_some_and(|v| v != id) {
            return Err(unconfirmed());
        }
        let after_metadata = self.checked_playlist_metadata(input, id, check).await?;
        if expected.name != after_metadata.name
            || expected.description != after_metadata.description
            || expected.tags != after_metadata.tags
            || expected.small_pic != after_metadata.small_pic
            || expected.big_pic != after_metadata.big_pic
            || expected.count != after_metadata.count
            || expected.online != after_metadata.online
            || expected.playlist_type != after_metadata.playlist_type
            || (preserve_tag_ids && expected.tag_ids != after_metadata.tag_ids)
        {
            return Err(unconfirmed());
        }
        check()?;
        let after = self.native_management_directory(input).await;
        check()?;
        let (after, _) = after?;
        let mut after = ordinary(&after, id)?.clone();
        let mut expected_directory = before;
        expected_directory
            .extensions
            .insert("is_public".into(), json!(public));
        expected_directory.name = expected.name;
        expected_directory.description = expected.description;
        if !same_directory(&expected_directory, &after) {
            return Err(unconfirmed());
        }
        consistent(&after, &after_metadata)?;
        after.tags = after_metadata.tags;
        after
            .extensions
            .insert("editable_metadata_verified".into(), json!(true));
        Ok(Box::new(PlaylistMutationResult {
            playlist_ref: after.resource_ref.clone(),
            action: PlaylistMutationAction::Update,
            playlist: Some(after),
            extensions: Extensions::from([
                ("backend".into(), json!("native_account_library")),
                ("library_owner_id".into(), json!(input.user_id())),
                ("confirmed".into(), json!(true)),
                ("write_requests_dispatched".into(), json!(1)),
                ("atomic".into(), json!(false)),
                (
                    "consistency".into(),
                    json!("write_ack_metadata_and_directory_readback"),
                ),
            ]),
        }))
    }
}
fn ordinary<'a>(items: &'a [Playlist], id: &str) -> Result<&'a Playlist> {
    items
        .iter()
        .find(|p| p.id == id && p.extensions.get("library_section") == Some(&json!("created")))
        .ok_or_else(|| {
            TuneWeaveError::new(
                ErrorCode::PermissionDenied,
                "Kuwo metadata changes require an ordinary playlist owned by the selected account",
            )
            .with_platform(Platform::Kuwo)
        })
}
fn consistent(p: &Playlist, m: &Metadata) -> Result<()> {
    if !m.matches_directory(p)
        || p.track_count.is_none()
        || p.extensions
            .get("is_public")
            .and_then(serde_json::Value::as_bool)
            .is_none()
    {
        return Err(unconfirmed());
    }
    Ok(())
}
pub(super) fn same_directory(a: &Playlist, b: &Playlist) -> bool {
    a.id == b.id
        && a.name == b.name
        && a.description == b.description
        && a.cover_url == b.cover_url
        && a.track_count == b.track_count
        && a.extensions.get("is_public") == b.extensions.get("is_public")
        && a.extensions.get("owner_id") == b.extensions.get("owner_id")
}

fn invalid() -> TuneWeaveError {
    kuwo_upstream_error("Kuwo playlist metadata response is invalid or incomplete")
}
