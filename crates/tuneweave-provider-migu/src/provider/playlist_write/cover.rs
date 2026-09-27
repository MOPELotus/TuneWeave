use super::metadata::PendingCallerUpdate;
use super::*;
use crate::provider::account_playlists::Snapshot;
use std::time::Duration;
use tuneweave_core::{ImageUploadRequest, ImageUploadResult, PlaylistCoverUpdateResult};

const MAX_COVER_BYTES: usize = 20 * 1024 * 1024;

fn validate_image(request: &ImageUploadRequest) -> Result<()> {
    if request.data.len() < 4
        || request.data.len() > MAX_COVER_BYTES
        || !request.data.starts_with(&[0xff, 0xd8, 0xff])
        || !request.data.ends_with(&[0xff, 0xd9])
        || !request.content_type.eq_ignore_ascii_case("image/jpeg")
        || request.filename.is_empty()
        || request.filename.len() > 255
        || request.filename.chars().any(char::is_control)
        || request.image_size.is_some()
        || request.crop_x.is_some()
        || request.crop_y.is_some()
    {
        return Err(migu_invalid_request(
            "Migu playlist covers require a complete JPEG of at most 20 MiB; image cropping is not supported by the native upload endpoint",
        ));
    }
    Ok(())
}

fn library_identity(playlists: &[Playlist]) -> Vec<(String, String)> {
    let mut identity = playlists
        .iter()
        .map(|playlist| (playlist.id.clone(), playlist.name.clone()))
        .collect::<Vec<_>>();
    identity.sort_unstable();
    identity
}

fn without_cover(snapshot: &Snapshot) -> Playlist {
    let mut playlist = snapshot.playlist.clone();
    playlist.cover_url = None;
    playlist
}

impl MiguProvider {
    pub(in crate::provider) async fn update_owned_playlist_cover(
        &self,
        id: &str,
        request: &ImageUploadRequest,
    ) -> Result<PlaylistCoverUpdateResult> {
        parse_playlist_id(id)?;
        validate_image(request)?;
        let mut s = self.playlist_write_session(request.account.as_deref())?;
        let mut pending = PendingCallerUpdate {
            provider: self,
            finished: false,
        };
        let result = tokio::time::timeout(Duration::from_secs(120), async {
            let (favorite, created) = self.write_preflight(&mut s).await?;
            ordinary(id, favorite.as_deref(), &created)?;
            let before = self
                .read_selected_account_playlist(
                    Some(id),
                    &s.alias,
                    &mut s.current,
                    &mut s.stored,
                )
                .await?;
            owner(&before.playlist, s.current.user_id())?;
            if !before.order_metadata_stable {
                return Err(migu_upstream_error(
                    "Migu playlist cover or tag metadata changed during the pre-upload read",
                ));
            }
            let before_library = library_identity(&created);
            let before_track_ids = before
                .track_ids()
                .into_iter()
                .map(str::to_owned)
                .collect::<Vec<_>>();
            let authorization = self.write_native_authorization(&mut s).await?;

            // picUpload binds the upload to resourceId and returns only a BaseVO
            // acknowledgement. Any later failure must remain non-retryable.
            self.accept_read(&s.current, s.stored.as_ref(), &s.current)?;
            s.dispatched = true;
            self.client
                .write_native_playlist_cover(&authorization, id, request)
                .await?;

            let after = self
                .read_selected_account_playlist(
                    Some(id),
                    &s.alias,
                    &mut s.current,
                    &mut s.stored,
                )
                .await?;
            owner(&after.playlist, s.current.user_id())?;
            let cover_url = after
                .playlist
                .cover_url
                .as_deref()
                .filter(|url| !url.is_empty())
                .ok_or_else(|| {
                    migu_upstream_error("Migu cover upload readback omitted the playlist cover")
                })?;
            // Cover, tag identities and the custom-cover marker must also
            // agree across the first and last metadata reads of each snapshot.
            if !after.order_metadata_stable
                || before.playlist.cover_url.as_deref() == Some(cover_url)
                || without_cover(&before) != without_cover(&after)
                || before_track_ids
                    != after
                        .track_ids()
                        .into_iter()
                        .map(str::to_owned)
                        .collect::<Vec<_>>()
            {
                return Err(migu_upstream_error(
                    "Migu cover upload readback did not confirm a changed cover while preserving the complete playlist",
                ));
            }
            let after_created = self.write_created(&mut s).await?;
            if library_identity(&after_created) != before_library {
                return Err(migu_upstream_error(
                    "Migu created library changed during the cover upload",
                ));
            }
            self.write_finish_identity(favorite.as_deref(), &mut s).await?;
            Ok(PlaylistCoverUpdateResult {
                playlist_ref: after.playlist.resource_ref.clone(),
                image: ImageUploadResult {
                    url: Some(cover_url.to_owned()),
                    image_id: None,
                    extensions: Extensions::new(),
                },
                extensions: Extensions::from([
                    ("backend".into(), json!("official_native_pic_upload")),
                    ("upload_requests_dispatched".into(), json!(1)),
                    (
                        "verified_by".into(),
                        json!("complete_playlist_and_created_library_readback"),
                    ),
                    ("source_user_id".into(), json!(s.current.user_id())),
                ]),
            })
        })
        .await
        .map_err(|_| migu_upstream_error("Migu playlist cover update exceeded its deadline"))
        .and_then(|result| result);

        let result = self.finish_playlist_write(&s, result).map_err(|mut error| {
            if !s.dispatched {
                return error;
            }
            let mut details = error.details.as_object().cloned().unwrap_or_default();
            details.insert("operation".into(), json!("playlist_cover"));
            details.insert("upload_requests_dispatched".into(), json!(1));
            details.insert("write_outcome".into(), json!("unconfirmed"));
            details.insert("automatic_retry".into(), json!(false));
            error = error
                .retryable(false)
                .with_details(serde_json::Value::Object(details));
            error
        });
        pending.finished = true;
        result
    }
}
