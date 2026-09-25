//! Upload and save are separate writes, confirmed only by complete readback.
use super::*;
use tuneweave_core::{ImageUploadRequest, ImageUploadResult, PlaylistCoverUpdateResult};

pub(crate) mod image;
#[cfg(test)]
pub(crate) mod tests;
mod upload;

#[derive(Default)]
pub(crate) struct Progress {
    upload_dispatched: bool,
    upload_confirmed: bool,
    playlist_dispatched: bool,
}
impl Progress {
    pub(crate) fn failure(&self, mut error: TuneWeaveError) -> TuneWeaveError {
        let count = u8::from(self.upload_dispatched) + u8::from(self.playlist_dispatched);
        if count != 0 {
            let mut details = error
                .details
                .as_object_mut()
                .map(std::mem::take)
                .unwrap_or_default();
            details.extend([
                ("write_outcome".into(), json!("unconfirmed")),
                ("write_requests_dispatched".into(), json!(count)),
                (
                    "upload_requests_dispatched".into(),
                    json!(u8::from(self.upload_dispatched)),
                ),
                (
                    "playlist_write_requests_dispatched".into(),
                    json!(u8::from(self.playlist_dispatched)),
                ),
                (
                    "upload_outcome".into(),
                    json!(if self.upload_confirmed {
                        "confirmed"
                    } else {
                        "unconfirmed"
                    }),
                ),
                (
                    "playlist_write_outcome".into(),
                    json!(if self.playlist_dispatched {
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

impl KuwoClient {
    /// Converts an image locally, uploads it once and saves an ordinary owned
    /// playlist's cover. An error after either dispatch does not imply rollback.
    pub async fn native_update_playlist_cover(
        &self,
        credential: &ProviderCredential,
        id: &str,
        request: &ImageUploadRequest,
    ) -> Result<PlaylistCoverUpdateResult> {
        image::validate(id, request)?;
        if request.account.as_deref().is_some_and(|v| v != "default") {
            return Err(kuwo_invalid_request(
                "Kuwo SDK credentials cannot select a stored account",
            ));
        }
        let input = credential::NativeCredential::parse(credential)?.input()?;
        validate_session_metadata(&input)?;
        let prepared = image::prepare(request).await?;
        self.validate_native_session(&input).await?;
        let mut progress = Progress::default();
        self.perform_native_cover(&input, id, &prepared, &mut progress, || Ok(()))
            .await
            .map_err(|e| progress.failure(e))
    }

    pub(crate) async fn perform_native_cover(
        &self,
        input: &KuwoNativeSessionInput,
        id: &str,
        prepared: &image::Prepared,
        progress: &mut Progress,
        mut check: impl FnMut() -> Result<()> + Send,
    ) -> Result<PlaylistCoverUpdateResult> {
        tokio::time::timeout(Duration::from_secs(120), async {
            let before = self
                .fetch_native_account_playlist(input, Some(id), Some(Section::Created), &mut check)
                .await?;
            let metadata = before.detail.as_ref().ok_or_else(unconfirmed)?;
            if metadata.online {
                return Err(TuneWeaveError::new(
                    ErrorCode::CapabilityNotSupported,
                    "Kuwo covers for published playlists require contribution management",
                )
                .with_platform(Platform::Kuwo));
            }
            let public = before
                .playlist
                .extensions
                .get("is_public")
                .and_then(serde_json::Value::as_bool)
                .ok_or_else(unconfirmed)?;
            check()?;
            let uploaded = self
                .upload_native_cover(input, prepared, &mut progress.upload_dispatched)
                .await;
            check()?;
            let uploaded = uploaded?;
            progress.upload_confirmed = true;
            // Uploading may take time. Do not overwrite metadata changed meanwhile.
            let current = self
                .checked_playlist_metadata(input, id, &mut check)
                .await?;
            if &current != metadata {
                return Err(unconfirmed());
            }
            check()?;
            let directory = self.native_management_directory(input).await;
            check()?;
            let (directory, _) = directory?;
            let current = directory
                .iter()
                .find(|p| {
                    p.id == id && p.extensions.get("library_section") == Some(&json!("created"))
                })
                .ok_or_else(unconfirmed)?;
            if !edit::same_directory(&before.playlist, current) {
                return Err(unconfirmed());
            }
            let payload = json!({"pid":id.parse::<i64>().map_err(|_|unconfirmed())?,
                "title":metadata.name,"intro":metadata.description,"tag":metadata.tags.join(","),
                "pic":uploaded.url,"ispub":public});
            check()?;
            let ack = self
                .native_cloud_write(
                    input,
                    "pl3_editlist",
                    payload,
                    &mut progress.playlist_dispatched,
                )
                .await;
            check()?;
            if ack?.pid.as_deref().is_some_and(|v| v != id) {
                return Err(unconfirmed());
            }
            let after = self
                .fetch_native_account_playlist(input, Some(id), Some(Section::Created), &mut check)
                .await?;
            verify(&before, &after, &uploaded)?;
            check()?;
            Ok(PlaylistCoverUpdateResult {
                playlist_ref: after.playlist.resource_ref,
                image: ImageUploadResult {
                    url: Some(uploaded.url),
                    image_id: None,
                    extensions: Extensions::from([
                        ("thumbnail_url".into(), json!(uploaded.thumbnail)),
                        ("content_type".into(), json!("image/jpeg")),
                        ("width".into(), json!(700)),
                        ("height".into(), json!(700)),
                        ("source_width".into(), json!(prepared.source_width)),
                        ("source_height".into(), json!(prepared.source_height)),
                        (
                            "crop".into(),
                            json!({"x":prepared.crop.0,"y":prepared.crop.1,"size":prepared.crop.2}),
                        ),
                    ]),
                },
                extensions: Extensions::from([
                    ("backend".into(), json!("native_account_library")),
                    ("library_owner_id".into(), json!(input.user_id())),
                    ("confirmed".into(), json!(true)),
                    ("atomic".into(), json!(false)),
                    ("write_requests_dispatched".into(), json!(2)),
                    ("upload_requests_dispatched".into(), json!(1)),
                    ("playlist_write_requests_dispatched".into(), json!(1)),
                    ("upload_outcome".into(), json!("confirmed")),
                    ("playlist_write_outcome".into(), json!("confirmed")),
                    (
                        "source_snapshot_id".into(),
                        after.playlist.extensions["source_snapshot_id"].clone(),
                    ),
                    (
                        "consistency".into(),
                        json!("upload_ack_write_ack_and_complete_playlist_readback"),
                    ),
                ]),
            })
        })
        .await
        .map_err(|_| {
            TuneWeaveError::new(
                ErrorCode::UpstreamTimeout,
                "Kuwo playlist cover operation timed out",
            )
            .with_platform(Platform::Kuwo)
        })?
    }
}

fn verify(
    before: &playlist::Snapshot,
    after: &playlist::Snapshot,
    uploaded: &upload::Uploaded,
) -> Result<()> {
    let mut a = before.playlist.clone();
    let mut b = after.playlist.clone();
    a.cover_url = None;
    b.cover_url = None;
    a.extensions.remove("source_snapshot_id");
    b.extensions.remove("source_snapshot_id");
    if a != b || before.tracks != after.tracks {
        return Err(unconfirmed());
    }
    let mut a = before.detail.clone().ok_or_else(unconfirmed)?;
    let mut b = after.detail.clone().ok_or_else(unconfirmed)?;
    let covers = [&after.playlist.cover_url, &b.small_pic, &b.big_pic];
    if !covers.iter().any(|v| v.as_deref() == Some(&uploaded.url))
        || covers.iter().any(|v| {
            v.as_ref()
                .is_some_and(|v| v != &uploaded.url && Some(v) != uploaded.thumbnail.as_ref())
        })
    {
        return Err(unconfirmed());
    }
    a.small_pic = None;
    a.big_pic = None;
    b.small_pic = None;
    b.big_pic = None;
    if a != b {
        return Err(unconfirmed());
    }
    Ok(())
}
