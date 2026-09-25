use super::account_read::Read;
use super::*;
use crate::credential::error;
use std::time::Duration;
use tuneweave_core::{ErrorCode, ImageUploadRequest, ImageUploadResult};

fn validate_image(request: &ImageUploadRequest) -> Result<()> {
    if request.data.len() < 4
        || request.data.len() > 20 * 1024 * 1024
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
        return Err(error(
            ErrorCode::InvalidRequest,
            "Migu avatars require a complete JPEG of at most 20 MiB; clearing and server-side cropping are not supported",
        ));
    }
    Ok(())
}

impl MiguProvider {
    pub(super) async fn upload_static_account_avatar(
        &self,
        request: &ImageUploadRequest,
    ) -> Result<ImageUploadResult> {
        validate_image(request)?;
        let mut read = Read::new(self, request.account.as_deref())?;
        let mut dispatched = false;
        let mut conversion_dispatched = false;
        let mut conversion_confirmed = false;
        let mut usage_dispatched = false;
        let mut usage_confirmed = false;
        let result = tokio::time::timeout(Duration::from_secs(120), async {
            let session = read.native_session().await?;
            let reply = self
                .client
                .account_h5_token(read.current.token(), &session)
                .await;
            let candidate = read.accept(reply).await??;
            read.secrets.push(candidate.clone());
            let auth = self
                .client
                .validate_native_token(candidate, read.current.user_id())
                .await;
            read.check()?;
            let auth = auth?;
            self.client.native_avatar_profile(&auth).await?;
            read.check()?;
            let static_mode = self.client.native_avatar_static_mode(&auth).await;
            read.check()?;
            let was_static = static_mode?;
            let pending = self.client.native_avatar_pending(&auth).await;
            read.check()?;
            if pending? {
                return Err(error(
                    ErrorCode::Conflict,
                    "Migu already has an avatar awaiting review",
                ));
            }
            let already_used_static = if was_static {
                true
            } else {
                let used = self.client.native_avatar_used_static(&auth).await;
                read.check()?;
                used?
            };

            // This POST directly submits the avatar. Never replay after a
            // transport failure or a failed independent readback.
            dispatched = true;
            self.client
                .upload_native_account_avatar(&auth, &request.data)
                .await?;
            read.check()?;
            self.client.native_avatar_profile(&auth).await?;
            read.check()?;
            let static_mode = self.client.native_avatar_static_mode(&auth).await;
            read.check()?;
            if static_mode? != was_static {
                return Err(error(
                    ErrorCode::Conflict,
                    "Migu avatar mode changed during upload",
                ));
            }
            if !was_static {
                conversion_dispatched = true;
                self.client.convert_native_avatar_to_static(&auth).await?;
                read.check()?;
                let static_mode = self.client.native_avatar_static_mode(&auth).await;
                read.check()?;
                if !static_mode? {
                    return Err(error(
                        ErrorCode::UpstreamError,
                        "Migu avatar mode readback did not confirm static mode",
                    ));
                }
                self.client.native_avatar_profile(&auth).await?;
                read.check()?;
                conversion_confirmed = true;
                if !already_used_static {
                    usage_dispatched = true;
                    self.client.mark_native_avatar_static_used(&auth).await?;
                    read.check()?;
                }
                let used = self.client.native_avatar_used_static(&auth).await;
                read.check()?;
                if !used? {
                    return Err(error(
                        ErrorCode::UpstreamError,
                        "Migu avatar usage marker readback was not confirmed",
                    ));
                }
                usage_confirmed = true;
            }
            let pending = self.client.native_avatar_pending(&auth).await;
            read.check()?;
            if !pending? {
                return Err(error(
                    ErrorCode::UpstreamError,
                    "Migu avatar readback did not confirm the new review submission",
                ));
            }
            read.start().await?;
            // The official success UI says the image appears after review.
            // An existing profile URL is not the uploaded avatar's URL.
            Ok(ImageUploadResult {
                url: None,
                image_id: None,
                extensions: Extensions::from([
                    ("backend".into(), json!("official_native_pic_upload")),
                    ("source_user_id".into(), json!(read.current.user_id())),
                    ("write_outcome".into(), json!("pending_review")),
                    ("upload_requests_dispatched".into(), json!(1)),
                    ("automatic_retry".into(), json!(false)),
                    ("converted_to_static".into(), json!(conversion_confirmed)),
                    (
                        "verified_by".into(),
                        json!("same_uid_profile_and_new_avatar_audit"),
                    ),
                ]),
            })
        })
        .await
        .map_err(|_| {
            error(
                ErrorCode::UpstreamTimeout,
                "Migu avatar upload exceeded its deadline",
            )
        })
        .and_then(|result| result);
        read.finish(result).map_err(|mut failure| {
            if dispatched {
                let mut details = failure.details.as_object().cloned().unwrap_or_default();
                details.insert("operation".into(), json!("account_avatar"));
                details.insert("upload_requests_dispatched".into(), json!(1));
                details.insert("write_outcome".into(), json!("unconfirmed"));
                details.insert("automatic_retry".into(), json!(false));
                if conversion_dispatched {
                    details.insert(
                        "mode_conversion_confirmed".into(),
                        json!(conversion_confirmed),
                    );
                    details.insert("mode_conversion_requests_dispatched".into(), json!(1));
                }
                if usage_dispatched {
                    details.insert(
                        "avatar_usage_marker_confirmed".into(),
                        json!(usage_confirmed),
                    );
                    details.insert("avatar_usage_requests_dispatched".into(), json!(1));
                }
                failure = failure
                    .retryable(false)
                    .with_details(serde_json::Value::Object(details));
            }
            failure
        })
    }
}

#[cfg(test)]
mod tests;
