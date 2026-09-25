//! Submission-history deletion is distinct from playlist deletion or withdrawal.
use super::*;
use crate::client::native::playlist::Snapshot;
use tuneweave_core::{PlaylistSubmissionRecordDeleteRequest, PlaylistSubmissionRecordDeleteResult};

#[cfg(test)]
pub(crate) mod tests;

const DELETE_HOST: &str = "wapi.kuwo.cn";
const DELETE_PATH: &str = "/api/mobicase/playlist/userDelTgRecord";

#[derive(Default)]
pub(crate) struct Progress {
    dispatched: bool,
    acknowledged: bool,
    records_confirmed: bool,
}
impl Progress {
    pub(crate) fn failure(&self, mut error: TuneWeaveError) -> TuneWeaveError {
        if self.dispatched {
            let mut details = error
                .details
                .as_object_mut()
                .map(std::mem::take)
                .unwrap_or_default();
            details.extend([
                (
                    "write_outcome".into(),
                    json!(if self.records_confirmed {
                        "partial"
                    } else {
                        "unconfirmed"
                    }),
                ),
                ("write_requests_dispatched".into(), json!(1)),
                (
                    "record_delete_outcome".into(),
                    json!(if self.records_confirmed {
                        "confirmed"
                    } else if self.acknowledged {
                        "acknowledged"
                    } else {
                        "unconfirmed"
                    }),
                ),
                ("automatic_retry".into(), json!(false)),
            ]);
            error = error.retryable(false).with_details(json!(details));
        }
        error
    }
}

impl KuwoClient {
    /// Removes all observed submission-history records for one playlist. This
    /// does not request ordinary playlist deletion or establish withdrawal.
    pub async fn native_delete_playlist_submission_records(
        &self,
        credential: &ProviderCredential,
        id: &str,
        request: &PlaylistSubmissionRecordDeleteRequest,
    ) -> Result<PlaylistSubmissionRecordDeleteResult> {
        playlist::validate_id(id)?;
        if request.account.as_deref().is_some_and(|v| v != "default") {
            return Err(kuwo_invalid_request(
                "Kuwo SDK credentials cannot select a stored account",
            ));
        }
        let input = credential::NativeCredential::parse(credential)?.input()?;
        validate_session_metadata(&input)?;
        let mut progress = Progress::default();
        let result = tokio::time::timeout(self.submission_limits().budget, async {
            self.validate_native_session(&input).await?;
            self.perform_native_submission_record_delete(&input, id, &mut progress, || Ok(()))
                .await
        })
        .await
        .map_err(|_| submissions::timeout())
        .and_then(|v| v);
        result.map_err(|e| progress.failure(e))
    }

    pub(crate) async fn perform_native_submission_record_delete(
        &self,
        input: &KuwoNativeSessionInput,
        id: &str,
        progress: &mut Progress,
        mut check: impl FnMut() -> Result<()> + Send,
    ) -> Result<PlaylistSubmissionRecordDeleteResult> {
        let (before_records, _, _) = self
            .fetch_complete_native_submissions(input, &mut check)
            .await?;
        let before = self
            .submission_owned_playlist(input, id, &mut check)
            .await?;
        let removed = before_records
            .iter()
            .filter(|v| v.playlist_ref.id() == id)
            .count();
        if removed > 0 {
            check()?;
            let ack = self
                .dispatch_native_submission_record_delete(input, id, &mut progress.dispatched)
                .await;
            // Capture the old request's receipt before rejecting a changed account.
            progress.acknowledged = ack.is_ok();
            check()?;
            ack?;
        }
        let (after_records, snapshot_id, _) = self
            .fetch_complete_native_submissions(input, &mut check)
            .await?;
        if after_records.iter().any(|v| v.playlist_ref.id() == id) {
            return Err(changed());
        }
        progress.records_confirmed = progress.dispatched;
        let expected: Vec<_> = before_records
            .into_iter()
            .filter(|v| v.playlist_ref.id() != id)
            .collect();
        if expected != after_records {
            return Err(changed());
        }
        let after = self
            .submission_owned_playlist(input, id, &mut check)
            .await?;
        if before.is_some() != after.is_some() {
            return Err(changed().with_details(json!({
                "owned_playlist_present_before":before.is_some(),
                "owned_playlist_present_after":after.is_some(),
                "published_before":published(&before), "published_after":published(&after)
            })));
        }
        if let (Some(before), Some(after)) = (&before, &after) {
            let mut expected = before.detail.as_ref().ok_or_else(changed)?.clone();
            // Publication can change asynchronously. Report both observations,
            // without attributing the change to this history operation.
            expected.online = after.detail.as_ref().ok_or_else(changed)?.online;
            if Some(&expected) != after.detail.as_ref()
                || !edit::same_directory(&before.playlist, &after.playlist)
                || before.tracks != after.tracks
            {
                return Err(changed());
            }
        }
        check()?;
        let published_before = published(&before);
        let published_after = published(&after);
        Ok(PlaylistSubmissionRecordDeleteResult {
            playlist_ref: ResourceRef::new(Platform::Kuwo, id).map_err(|_| changed())?,
            confirmed: true,
            changed: removed > 0,
            removed_records: removed as u64,
            owned_playlist_present: after.is_some(),
            published: published_after,
            playlist: after.map(|v| v.playlist),
            extensions: Extensions::from([
                (
                    "backend".into(),
                    json!("native_playlist_submission_record_delete"),
                ),
                ("source_user_id".into(), json!(input.user_id())),
                (
                    "write_requests_dispatched".into(),
                    json!(u8::from(progress.dispatched)),
                ),
                (
                    "record_delete_outcome".into(),
                    json!(if progress.dispatched {
                        "confirmed"
                    } else {
                        "already_absent"
                    }),
                ),
                ("records_snapshot_id".into(), json!(snapshot_id)),
                ("published_before".into(), json!(published_before)),
                (
                    "publication_changed".into(),
                    json!(published_before.zip(published_after).map(|(a, b)| a != b)),
                ),
                (
                    "playlist_observation_scope".into(),
                    json!("ordinary_created_directory"),
                ),
                (
                    "consistency".into(),
                    json!("complete_records_and_independent_playlist_observations"),
                ),
                ("automatic_retry".into(), json!(false)),
                ("atomic".into(), json!(false)),
            ]),
        })
    }

    async fn submission_owned_playlist(
        &self,
        input: &KuwoNativeSessionInput,
        id: &str,
        check: &mut (impl FnMut() -> Result<()> + Send),
    ) -> Result<Option<Snapshot>> {
        let (directory, _) = self
            .native_library_items(input, Some(Section::Created), check)
            .await?;
        if let Some(row) = directory.into_iter().find(|v| v.id == id) {
            let snapshot = self
                .fetch_native_account_playlist(input, Some(id), Some(Section::Created), check)
                .await?;
            if !edit::same_directory(&row, &snapshot.playlist) {
                return Err(changed());
            }
            Ok(Some(snapshot))
        } else {
            // Two complete directory reads establish absence from this account's
            // ordinary library only, never global resource deletion.
            let (second, _) = self
                .native_library_items(input, Some(Section::Created), check)
                .await?;
            if second.iter().any(|v| v.id == id) {
                return Err(changed());
            }
            Ok(None)
        }
    }

    async fn dispatch_native_submission_record_delete(
        &self,
        input: &KuwoNativeSessionInput,
        id: &str,
        dispatched: &mut bool,
    ) -> Result<()> {
        let query = {
            let mut q = url::form_urlencoded::Serializer::new(String::new());
            q.extend_pairs([("loginUid", input.user_id()), ("sid", input.session_id())]);
            q.finish()
        };
        let request = self
            .http
            .post(format!(
                "{}?{query}",
                self.native_target(DELETE_HOST, DELETE_PATH)
            ))
            .header(ACCEPT, "application/json")
            .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(format!("ids={id}"));
        let started = Instant::now();
        let mut status = None;
        *dispatched = true;
        let result = async {
            let response = request
                .send()
                .await
                .map_err(|e| kuwo_network_error(e).retryable(false))?;
            status = Some(response.status());
            let bytes = read_response(response, false, false, ACK_LIMIT).await?;
            parse_ack(&bytes)
        }
        .await;
        self.log_upstream_request(
            "native_playlist_submission_record_delete",
            DELETE_HOST,
            DELETE_PATH,
            status,
            started,
            0,
            false,
            &result,
        );
        result
    }
}

fn published(snapshot: &Option<Snapshot>) -> Option<bool> {
    snapshot
        .as_ref()
        .and_then(|v| v.detail.as_ref())
        .map(|v| v.online)
}
#[derive(Deserialize)]
struct Ack {
    #[serde(default, deserialize_with = "deserialize_code")]
    code: Option<String>,
    data: Option<AckData>,
}
#[derive(Deserialize)]
struct AckData {
    result: String,
}
fn parse_ack(bytes: &[u8]) -> Result<()> {
    let ack: Ack = serde_json::from_slice(bytes).map_err(|_| invalid_ack())?;
    if ack.code.as_deref() == Some("-1001") {
        return Err(authentication_required());
    }
    if ack.code.as_deref().is_some_and(|v| v != "200")
        || ack.data.is_none_or(|v| v.result != "success")
    {
        return Err(invalid_ack());
    }
    Ok(())
}
fn invalid_ack() -> TuneWeaveError {
    kuwo_upstream_error("Kuwo submission record deletion acknowledgement is invalid or rejected")
}
fn changed() -> TuneWeaveError {
    TuneWeaveError::new(
        ErrorCode::Conflict,
        "Kuwo submission records or owned playlist changed during record deletion",
    )
    .with_platform(Platform::Kuwo)
}
