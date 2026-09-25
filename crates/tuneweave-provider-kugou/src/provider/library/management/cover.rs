use super::*;
use crate::KugouLoginClient;
use crate::provider::session::authentication_required;
use tuneweave_core::{ImageUploadRequest, ImageUploadResult, PlaylistCoverUpdateResult};

mod image;

impl KugouProvider {
    pub(in crate::provider) async fn native_update_playlist_cover(
        &self,
        id: &str,
        request: &ImageUploadRequest,
    ) -> Result<PlaylistCoverUpdateResult> {
        let (uid, kind, list_id) = parse_reference(id)?;
        if kind != 0 {
            return Err(denied());
        }
        image::validate(request)?;
        let account = request.account.as_deref().unwrap_or("default");
        let (selected, _) = self
            .selected(account)?
            .ok_or_else(authentication_required)?;
        let client = match &selected {
            KugouCredential::Native(v) => v.session.client,
            _ => {
                return Err(unsupported(
                    "KuGou playlist cover updates require a native app credential",
                ));
            }
        };
        let mut read = self.begin_native_read(account, Some(uid)).await?;
        let mut upload_dispatched = false;
        let mut save_dispatched = false;
        let outcome = async {
            let before = self.read_native_library(&mut read).await?;
            let selected = find(&before, id)?;
            ordinary(selected)?;
            let gid = selected
                .extensions
                .get("global_collection_id")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    unsupported("KuGou cover update requires the playlist global identity")
                })?;
            let prepared = image::prepare(request, client).await?;
            self.check_account_read(&mut read)?;
            let authorization = self
                .client
                .native_cover_authorization(read.session()?)
                .await?;
            self.check_account_read(&mut read)?;
            upload_dispatched = true;
            let uploaded = self
                .client
                .native_upload_cover(read.session()?, authorization, prepared.jpeg)
                .await?;
            self.check_account_read(&mut read)?;
            save_dispatched = true;
            let ack = self
                .client
                .native_save_cover(
                    read.session()?,
                    list_id,
                    before.version.total_ver,
                    gid,
                    &uploaded,
                )
                .await?;
            self.check_account_read(&mut read)?;
            let after = self.read_native_library(&mut read).await?;
            let updated = find(&after, id)?;
            let mut expected = stable(selected);
            expected.cover_url = Some(ack.url.clone());
            let mut actual = stable(updated);
            expected.extensions.remove("list_ver");
            actual.extensions.remove("list_ver");
            if expected != actual
                || selected.extensions.get("sort") != updated.extensions.get("sort")
            {
                return Err(library_changed());
            }
            unrelated(&before, &after, id)?;
            acknowledge(&ack.list, &before, &after, Some(updated))?;
            Ok(PlaylistCoverUpdateResult {
                playlist_ref: ResourceRef::new(Platform::Kugou, id)
                    .map_err(|_| library_changed())?,
                image: ImageUploadResult {
                    url: Some(ack.url),
                    image_id: Some(uploaded.filename().to_owned()),
                    extensions: Extensions::from([
                        ("width".into(), json!(prepared.output_size)),
                        ("height".into(), json!(prepared.output_size)),
                        ("crop".into(), json!(prepared.crop)),
                    ]),
                },
                extensions: Extensions::from([
                    (
                        "backend".into(),
                        json!(if client == KugouLoginClient::Concept {
                            "concept_native_cover"
                        } else {
                            "standard_native_cover"
                        }),
                    ),
                    ("upload_requests_dispatched".into(), json!(1)),
                    ("write_requests_dispatched".into(), json!(1)),
                    ("readback_verified".into(), json!(true)),
                    ("total_ver".into(), json!(after.version.total_ver)),
                ]),
            })
        }
        .await;
        self.finish_account_read(read, outcome)
            .map_err(|mut error| {
                if upload_dispatched {
                    let mut details = error.details.as_object().cloned().unwrap_or_default();
                    details.insert("operation".into(), json!("playlist_cover"));
                    details.insert("upload_requests_dispatched".into(), json!(1));
                    details.insert(
                        "write_requests_dispatched".into(),
                        json!(u8::from(save_dispatched)),
                    );
                    details.insert(
                        "write_outcome".into(),
                        json!(if save_dispatched {
                            "unconfirmed"
                        } else {
                            "not_attempted"
                        }),
                    );
                    details.insert("uploaded_image_may_remain".into(), json!(true));
                    error = error.retryable(false).with_details(Value::Object(details));
                }
                error
            })
    }
}

#[cfg(test)]
mod tests;
