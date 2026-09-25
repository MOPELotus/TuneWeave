use super::account_download::NativeAuthorization;
use super::playlist_tags::TagChange;
use super::*;
use crate::credential::error;
use tuneweave_core::ImageUploadRequest;

impl MiguClient {
    pub(crate) async fn write_native_playlist_cover(
        &self,
        auth: &NativeAuthorization,
        id: &str,
        request: &ImageUploadRequest,
    ) -> Result<()> {
        // The official Android producer sends a raw JPEG body. The native
        // interceptor sorts query names before signing, so keep this order.
        let value = self
            .native_image_post(
                "app.c.nf.migu.cn",
                "/MIGUM2.0/v1.0/picUpload.do",
                &auth.token,
                &auth.uid,
                vec![("resourceId", id), ("type", "02")],
                &request.data,
            )
            .await?;
        let code = value.get("code").and_then(serde_json::Value::as_str);
        if code != Some("000000") {
            let mut failure = error(
                if code.is_some_and(|code| {
                    matches!(
                        code,
                        "200000" | "200010" | "200013" | "200004" | "220000" | "290001"
                    )
                }) {
                    ErrorCode::PermissionDenied
                } else {
                    ErrorCode::UpstreamError
                },
                "Migu native playlist cover upload was not acknowledged",
            );
            if let Some(code) = code
                .filter(|code| code.len() == 6 && code.bytes().all(|byte| byte.is_ascii_digit()))
            {
                failure = failure.with_details(json!({"upstream_code":code}));
            }
            return Err(failure);
        }
        Ok(())
    }

    pub(crate) async fn write_native_playlist_metadata(
        &self,
        auth: &NativeAuthorization,
        id: &str,
        title: Option<&str>,
        description: Option<&str>,
        tag_change: Option<TagChange<'_>>,
    ) -> Result<()> {
        // The official tag picker emits a trailing '|' even for one addition.
        let addition = match tag_change {
            Some(TagChange::Add(tag)) => {
                Some((format!("{}|", tag.tag_id), format!("{}|", tag.tag_name)))
            }
            _ => None,
        };
        let mut fields = vec![("id", id), ("songflag", "0")];
        if let Some(title) = title {
            fields.push(("title", title));
        }
        if let Some(description) = description {
            fields.push(("info", description));
        }
        if let Some(TagChange::Remove(tag)) = tag_change {
            fields.push(("delTagIds", &tag.tag_id));
            fields.push(("delTagNames", &tag.tag_name));
        }
        if let Some((ids, names)) = &addition {
            fields.push(("addTagIds", ids));
            fields.push(("addTagNames", names));
        }
        let value = self
            .native_form(
                "app.u.nf.migu.cn",
                "/MIGUM2.0/v1.0/user/updateMusicList.do",
                &auth.token,
                &auth.uid,
                fields,
            )
            .await?;
        let code = value.get("code").and_then(serde_json::Value::as_str);
        if code != Some("000000") {
            let mut failure = error(
                if code.is_some_and(|code| {
                    matches!(
                        code,
                        "200000" | "200010" | "200013" | "200004" | "220000" | "290001"
                    )
                }) {
                    ErrorCode::PermissionDenied
                } else {
                    ErrorCode::UpstreamError
                },
                "Migu native playlist update was not acknowledged",
            );
            if let Some(code) = code
                .filter(|code| code.len() == 6 && code.bytes().all(|byte| byte.is_ascii_digit()))
            {
                failure = failure.with_details(json!({"upstream_code":code}));
            }
            return Err(failure);
        }
        // BaseVO guarantees the business code, not an updated playlist DTO.
        // Confirmation therefore always requires the provider's full readback.
        Ok(())
    }
}

#[derive(Clone, Copy)]
pub(crate) enum Write<'a> {
    Create(&'a str),
    Rename(&'a str, &'a str),
    Delete(&'a str),
    Add(&'a str, &'a [String]),
    Remove(&'a str, &'a [String]),
}
impl Write<'_> {
    pub(crate) fn path(self) -> &'static str {
        match self {
            Self::Create(_) => "/pc/open/api/music-list/add/v2.0",
            Self::Rename(..) | Self::Remove(..) => "/pc/user/h5-import-musiclist/v1.0",
            Self::Delete(_) => "/pc/v1.0/user/deleteMusicList.do",
            Self::Add(..) => "/pc/user/api/add-music-list-song/v1.0",
        }
    }
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Create(_) => "account_playlist_create",
            Self::Rename(..) => "account_playlist_rename",
            Self::Delete(_) => "account_playlist_delete",
            Self::Add(..) => "account_playlist_tracks_add",
            Self::Remove(..) => "account_playlist_tracks_remove",
        }
    }
    pub(crate) fn is_post(self) -> bool {
        !matches!(self, Self::Delete(_))
    }
    pub(crate) fn request(self, request: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match self {
            Self::Create(title) => {
                request.json(&json!({"title":title,"channel":"23","type":"self_build"}))
            }
            Self::Rename(id, title) => {
                request.json(&json!({"title":title,"channel":"23","id":id,"songflag":"0"}))
            }
            Self::Delete(id) => request.query(&[("channel", "23"), ("id", id)]),
            Self::Add(id, content_ids) => request.json(&json!({"id":id,"contentIds":content_ids})),
            Self::Remove(id, content_ids) => request.json(
                &json!({"channel":"23","songflag":"2","id":id,"contentId":content_ids.join("|")}),
            ),
        }
    }
}

pub(crate) fn acknowledged_id(
    fields: serde_json::Map<String, serde_json::Value>,
) -> Result<Option<String>> {
    fields
        .get("musicListId")
        .map(|value| {
            let id = value.as_str().ok_or_else(|| {
                migu_upstream_error("Migu playlist acknowledgement ID is invalid")
            })?;
            let numeric = id
                .parse::<u64>()
                .map_err(|_| migu_upstream_error("Migu playlist acknowledgement ID is invalid"))?;
            if numeric == 0 || numeric.to_string() != id {
                return Err(migu_upstream_error(
                    "Migu playlist acknowledgement ID is invalid",
                ));
            }
            Ok(id.to_owned())
        })
        .transpose()
}
