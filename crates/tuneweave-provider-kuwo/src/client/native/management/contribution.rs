//! Explicit metadata-save and submission steps; acceptance is not publication.
use super::*;
use crate::client::native::playlist::{Snapshot, metadata::Metadata};
use tuneweave_core::{PlaylistSubmissionRequest, PlaylistSubmissionResult};

pub(crate) mod recommendation;
#[cfg(test)]
pub(crate) mod tests;

const SUBMIT_HOST: &str = "wapi.kuwo.cn";
const SUBMIT_PATH: &str = "/api/mobicase/playlist/taglist";

#[derive(Default)]
pub(crate) struct Progress {
    pub(crate) metadata_dispatched: bool,
    pub(crate) metadata_confirmed: bool,
    pub(crate) submission_dispatched: bool,
    pub(crate) submission_accepted: bool,
}
impl Progress {
    fn count(&self) -> u8 {
        u8::from(self.metadata_dispatched) + u8::from(self.submission_dispatched)
    }
    pub(crate) fn failure(&self, mut error: TuneWeaveError) -> TuneWeaveError {
        if self.count() != 0 {
            let mut details = error
                .details
                .as_object_mut()
                .map(std::mem::take)
                .unwrap_or_default();
            details.extend([
                (
                    "write_outcome".into(),
                    json!(if self.submission_accepted || self.metadata_confirmed {
                        "partial"
                    } else {
                        "unconfirmed"
                    }),
                ),
                ("write_requests_dispatched".into(), json!(self.count())),
                (
                    "playlist_write_outcome".into(),
                    json!(if self.metadata_confirmed {
                        "confirmed"
                    } else if self.metadata_dispatched {
                        "unconfirmed"
                    } else {
                        "not_dispatched"
                    }),
                ),
                (
                    "submission_outcome".into(),
                    json!(if self.submission_accepted {
                        "accepted"
                    } else if self.submission_dispatched {
                        "unconfirmed"
                    } else {
                        "not_dispatched"
                    }),
                ),
                ("automatic_retry".into(), json!(false)),
            ]);
            error = error.retryable(false).with_details(json!(details));
        }
        error
    }
}

pub(crate) fn validate(id: &str, request: &PlaylistSubmissionRequest) -> Result<()> {
    playlist::validate_id(id)?;
    recommendation::validate(request.recommendation.as_deref())?;
    if has_edit(request) {
        edit::validate_update(id, &update_request(request))?;
    }
    Ok(())
}
fn has_edit(r: &PlaylistSubmissionRequest) -> bool {
    r.name.is_some() || r.description.is_some() || r.tags.is_some()
}
fn update_request(r: &PlaylistSubmissionRequest) -> PlaylistUpdateRequest {
    PlaylistUpdateRequest {
        name: r.name.clone(),
        description: r.description.clone(),
        tags: r.tags.clone(),
        account: r.account.clone(),
        ..PlaylistUpdateRequest::default()
    }
}
pub(crate) fn validate_secrets(
    r: &PlaylistSubmissionRequest,
    input: &KuwoNativeSessionInput,
) -> Result<()> {
    if r.name
        .iter()
        .chain(r.description.iter())
        .chain(r.tags.iter().flatten())
        .chain(r.recommendation.iter())
        .any(|v| echoes_secret(v, input.session_id()))
    {
        return Err(kuwo_invalid_request(
            "Kuwo submission metadata contains private session material",
        ));
    }
    Ok(())
}

impl KuwoClient {
    /// Submit an ordinary public playlist for review, optionally saving metadata
    /// first. Any dispatched step can have effects even if a later step fails.
    pub async fn native_submit_playlist(
        &self,
        credential: &ProviderCredential,
        id: &str,
        request: &PlaylistSubmissionRequest,
    ) -> Result<PlaylistSubmissionResult> {
        validate(id, request)?;
        if request.account.as_deref().is_some_and(|v| v != "default") {
            return Err(kuwo_invalid_request(
                "Kuwo SDK credentials cannot select a stored account",
            ));
        }
        let input = credential::NativeCredential::parse(credential)?.input()?;
        validate_session_metadata(&input)?;
        validate_secrets(request, &input)?;
        let mut progress = Progress::default();
        let result = tokio::time::timeout(self.submission_limits().budget, async {
            self.validate_native_session(&input).await?;
            self.perform_native_submission(&input, id, request, &mut progress, || Ok(()))
                .await
        })
        .await
        .map_err(|_| submissions::timeout())
        .and_then(|v| v);
        result.map_err(|e| progress.failure(e))
    }

    pub(crate) async fn perform_native_submission(
        &self,
        input: &KuwoNativeSessionInput,
        id: &str,
        request: &PlaylistSubmissionRequest,
        progress: &mut Progress,
        mut check: impl FnMut() -> Result<()> + Send,
    ) -> Result<PlaylistSubmissionResult> {
        let (prior_records, _, _) = self
            .fetch_complete_native_submissions(input, &mut check)
            .await?;
        let mut before = self
            .fetch_native_account_playlist(input, Some(id), Some(Section::Created), &mut check)
            .await?;
        let original = before.detail.as_ref().ok_or_else(changed)?;
        let mut expected = original.clone();
        if let Some(v) = &request.name {
            expected.name = v.clone();
        }
        if let Some(v) = &request.description {
            expected.description = v.clone();
        }
        if let Some(v) = &request.tags {
            expected.tags = v.clone();
        }
        eligible(&before, &expected, request.recommendation.is_some())?;
        let author = if let Some(recommendation) = &request.recommendation {
            Some(
                self.prepare_recommended_submission(
                    input,
                    id,
                    &expected,
                    recommendation,
                    &mut check,
                )
                .await?,
            )
        } else {
            None
        };
        if has_edit(request) || author.is_some() {
            let mut expected_directory = before.playlist.clone();
            expected_directory.name = expected.name.clone();
            expected_directory.description = expected.description.clone();
            let payload = json!({"pid":id.parse::<i64>().map_err(|_|changed())?,
                "title":expected.name,"intro":expected.description,"tag":expected.tags.join(","),
                "pic":before.playlist.cover_url,"ispub":true});
            check()?;
            let ack = if let Some(author) = &author {
                self.save_recommended_submission_metadata(
                    input,
                    id,
                    &expected,
                    &before.playlist,
                    author,
                    &mut progress.metadata_dispatched,
                )
                .await
                .map(|()| None)
            } else {
                self.native_cloud_write(
                    input,
                    "pl3_editlist",
                    payload,
                    &mut progress.metadata_dispatched,
                )
                .await
                .map(|ack| ack.pid)
            };
            check()?;
            if ack?.as_deref().is_some_and(|v| v != id) {
                return Err(changed());
            }
            let saved = self
                .fetch_native_account_playlist(input, Some(id), Some(Section::Created), &mut check)
                .await?;
            let actual = saved.detail.as_ref().ok_or_else(changed)?;
            if request.tags.is_some() {
                expected.tag_ids = actual.tag_ids.clone();
            }
            if author.is_some() {
                // The explicit resubmission flow observes publication independently.
                expected.online = actual.online;
            }
            if expected != *actual
                || !edit::same_directory(&expected_directory, &saved.playlist)
                || before.tracks != saved.tracks
            {
                return Err(changed());
            }
            progress.metadata_confirmed = true;
            before = saved;
        }
        check()?;
        let acknowledgement = self
            .dispatch_native_submission(
                input,
                id,
                request.recommendation.as_deref().unwrap_or(""),
                &mut progress.submission_dispatched,
            )
            .await;
        // Preserve what the original request acknowledged even if its account was
        // replaced while the response was in flight. Never return its data as the new account.
        progress.submission_accepted = acknowledgement.is_ok();
        check()?;
        acknowledgement?;
        let (records, record_snapshot, _) = self
            .fetch_complete_native_submissions(input, &mut check)
            .await?;
        let after = self
            .fetch_native_account_playlist(input, Some(id), Some(Section::Created), &mut check)
            .await?;
        let actual = after.detail.as_ref().ok_or_else(changed)?;
        let mut expected = before.detail.as_ref().ok_or_else(changed)?.clone();
        // Submission may change publication asynchronously; observe, do not predict it.
        expected.online = actual.online;
        if expected != *actual
            || !edit::same_directory(&before.playlist, &after.playlist)
            || before.tracks != after.tracks
        {
            return Err(changed());
        }
        check()?;
        let records: Vec<_> = records
            .into_iter()
            .filter(|r| r.playlist_ref.id() == id)
            .collect();
        Ok(PlaylistSubmissionResult {
            playlist_ref: after.playlist.resource_ref.clone(),
            accepted: true,
            metadata_updated: progress.metadata_confirmed,
            published: Some(actual.online),
            playlist: after.playlist,
            records,
            extensions: Extensions::from([
                ("backend".into(), json!("native_playlist_submission")),
                ("source_user_id".into(), json!(input.user_id())),
                ("write_requests_dispatched".into(), json!(progress.count())),
                (
                    "playlist_write_outcome".into(),
                    json!(if progress.metadata_confirmed {
                        "confirmed"
                    } else {
                        "not_dispatched"
                    }),
                ),
                ("submission_outcome".into(), json!("accepted")),
                (
                    "recommendation_submitted".into(),
                    json!(request.recommendation.is_some()),
                ),
                (
                    "prior_record_count".into(),
                    json!(
                        prior_records
                            .iter()
                            .filter(|r| r.playlist_ref.id() == id)
                            .count()
                    ),
                ),
                ("records_snapshot_id".into(), json!(record_snapshot)),
                ("records_correlated_to_request".into(), json!(false)),
                (
                    "consistency".into(),
                    json!("submission_ack_and_independent_records_and_playlist_readback"),
                ),
                ("atomic".into(), json!(false)),
                ("automatic_retry".into(), json!(false)),
            ]),
        })
    }

    async fn dispatch_native_submission(
        &self,
        input: &KuwoNativeSessionInput,
        id: &str,
        recommendation: &str,
        dispatched: &mut bool,
    ) -> Result<()> {
        let query = submission_query(input, id, recommendation);
        let request = self
            .http
            .get(format!(
                "{}?{query}",
                self.native_target(SUBMIT_HOST, SUBMIT_PATH)
            ))
            .header(ACCEPT, "application/json");
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
            parse_submission_ack(&bytes)
        }
        .await;
        self.log_upstream_request(
            "native_playlist_submission",
            SUBMIT_HOST,
            SUBMIT_PATH,
            status,
            started,
            0,
            false,
            &result,
        );
        result
    }
}

fn eligible(snapshot: &Snapshot, expected: &Metadata, recommended: bool) -> Result<()> {
    if snapshot
        .playlist
        .extensions
        .get("is_public")
        .and_then(serde_json::Value::as_bool)
        != Some(true)
    {
        return Err(kuwo_invalid_request(
            "Kuwo submission requires an explicitly public owned playlist; change visibility separately",
        ));
    }
    // Official o2.k counts Chinese U+4E00..U+9FA5 as1 and each other Java char as0.5.
    let doubled: usize = expected
        .name
        .encode_utf16()
        .map(|c| if (0x4e00..=0x9fa5).contains(&c) { 2 } else { 1 })
        .sum();
    if (recommended && expected.name.trim_matches(|c| (c as u32) <= 32) != expected.name)
        || !(14..=if recommended { 32 } else { 40 }).contains(&doubled)
        || expected.description.trim().is_empty()
        || expected.tags.is_empty()
        || snapshot.playlist.cover_url.is_none()
        || snapshot.tracks.len() < 10
    {
        return Err(kuwo_invalid_request(if recommended {
            "Kuwo recommended submission requires a title with weighted length 7 to 16, a description, tags, a cover and at least 10 cloud tracks"
        } else {
            "Kuwo submission requires a title with weighted length 7 to 20, a description, tags, a cover and at least 10 cloud tracks"
        }));
    }
    Ok(())
}
fn submission_query(input: &KuwoNativeSessionInput, id: &str, recommendation: &str) -> String {
    let mut q = url::form_urlencoded::Serializer::new(String::new());
    q.extend_pairs([
        ("type", "user_songlist_up"),
        ("id", id),
        ("source", CLIENT_SOURCE),
        ("loginUid", input.user_id()),
        ("loginSid", input.session_id()),
        ("sid", input.session_id()),
        ("prod", "kwplayer_ar_12.2.2.0"),
        ("platform", "ar"),
        ("uid", input.device_id()),
        ("corp", "kuwo"),
        ("approval", "false"),
        ("q36", device::FALLBACK_Q36),
        ("vipver", CLIENT_VERSION),
        ("newver", "3"),
        ("allpay", "0"),
        ("notrace", "1"),
        ("oaid", ""),
        ("vipMode", "0"),
        ("apiVer", "1"),
        ("recWord", recommendation),
    ]);
    q.finish()
}
#[derive(Deserialize)]
struct SubmissionAck {
    result: String,
}
fn parse_submission_ack(bytes: &[u8]) -> Result<()> {
    let ack: SubmissionAck = serde_json::from_slice(bytes).map_err(|_| invalid_ack())?;
    if !ack.result.eq_ignore_ascii_case("success") {
        return Err(invalid_ack());
    }
    Ok(())
}
fn invalid_ack() -> TuneWeaveError {
    kuwo_upstream_error("Kuwo submission acknowledgement is invalid or rejected")
}
fn changed() -> TuneWeaveError {
    TuneWeaveError::new(
        ErrorCode::Conflict,
        "Kuwo submission playlist state changed or could not be independently confirmed",
    )
    .with_platform(Platform::Kuwo)
}
