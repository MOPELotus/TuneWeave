use super::*;
use crate::KugouLoginClient;
use crate::provider::session::authentication_required;
use tuneweave_core::PlaylistVisibilityUpdateRequest;

fn visibility_matches(before: &Playlist, after: &Playlist, private: bool) -> Result<()> {
    let mut expected = stable(before);
    expected
        .extensions
        .insert("is_private".into(), json!(private));
    let mut actual = stable(after);
    expected.extensions.remove("list_ver");
    actual.extensions.remove("list_ver");
    if expected != actual || before.extensions.get("sort") != after.extensions.get("sort") {
        return Err(library_changed());
    }
    Ok(())
}

impl KugouProvider {
    pub(in crate::provider) async fn native_update_playlist_visibility(
        &self,
        id: &str,
        request: &PlaylistVisibilityUpdateRequest,
    ) -> Result<PlaylistMutationResult> {
        request
            .validate()
            .map_err(|e| e.with_platform(Platform::Kugou))?;
        let (uid, kind, list_id) = parse_reference(id)?;
        if kind != 0 {
            return Err(denied());
        }
        let private = request.visibility == PlaylistVisibility::Private;
        let account = request.account.as_deref().unwrap_or("default");
        let (selected, _) = self
            .selected(account)?
            .ok_or_else(authentication_required)?;
        if !matches!(&selected, KugouCredential::Native(v) if v.session.client == KugouLoginClient::Standard)
        {
            return Err(unsupported(
                "KuGou playlist visibility requires a Standard native credential",
            ));
        }
        let mut read = self.begin_native_read(account, Some(uid)).await?;
        let mut dispatched = 0;
        let outcome = async {
            // Recheck the actual refreshed selection in case the alias changed
            // between the no-network client check and begin_native_read.
            if read.session()?.client != KugouLoginClient::Standard {
                return Err(unsupported(
                    "KuGou playlist visibility requires a Standard native credential",
                ));
            }
            let before = self.read_native_library(&mut read).await?;
            let selected = find(&before, id)?;
            ordinary(selected)?;
            // The official presenter also disables collaboration when making a
            // list private. This action intentionally supports only lists whose
            // existing non-collaborative status is explicit in the full library.
            if selected
                .extensions
                .get("is_mutual")
                .and_then(Value::as_bool)
                != Some(false)
            {
                return Err(unsupported(
                    "KuGou visibility changes require an explicitly non-collaborative playlist",
                ));
            }
            let current = selected
                .extensions
                .get("is_private")
                .and_then(Value::as_bool)
                .ok_or_else(|| unsupported("KuGou playlist omitted its current visibility"))?;
            let sort = selected
                .extensions
                .get("sort")
                .and_then(Value::as_u64)
                .ok_or_else(|| unsupported("KuGou playlist omitted its current sort position"))?;
            if current == private {
                return Ok(result(
                    selected.clone(),
                    PlaylistMutationAction::Update,
                    0,
                    before.version.total_ver,
                ));
            }
            self.check_account_read(&mut read)?;
            dispatched += 1;
            let ack = self
                .client
                .native_write_list(
                    read.session()?,
                    ListWrite::Visibility {
                        list_id,
                        name: &selected.name,
                        private,
                        sort,
                        total_ver: before.version.total_ver,
                    },
                )
                .await?;
            self.check_account_read(&mut read)?;
            let after = self.read_native_library(&mut read).await?;
            let updated = find(&after, id)?;
            visibility_matches(selected, updated, private)?;
            unrelated(&before, &after, id)?;
            acknowledge(&ack, &before, &after, Some(updated))?;
            Ok(result(
                updated.clone(),
                PlaylistMutationAction::Update,
                dispatched,
                after.version.total_ver,
            ))
        }
        .await;
        let refs = [ResourceRef::new(Platform::Kugou, id).map_err(|_| library_changed())?];
        self.finish_account_read(read, outcome)
            .map_err(|e| write_error(e, dispatched, &[], &refs, &[]))
    }
}
