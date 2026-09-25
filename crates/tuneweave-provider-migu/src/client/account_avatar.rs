use super::account_download::NativeAuthorization;
use super::*;
use crate::credential::{error, validate_uid};

pub(super) const MODE_PATH: &str = "/user/api/avatar/query/v1.0";
pub(super) const AUDIT_PATH: &str = "/user/getAuditStatus/v1.0";
pub(super) const PROFILE_PATH: &str = "/MIGUM3.0/user/user-info/v1.0";
pub(super) const CONVERT_PATH: &str = "/user/api/avatar/convert/v1.0";
pub(super) const USAGE_PATH: &str = "/personal/recommend/api/v1.0";
pub(super) const USAGE_WRITE_PATH: &str = "/personal/recommend/set/v1.0";
const USAGE_KEY: &str = "personal:avatar2d:tmp:used";
// AvatarVersionInfo in the official 8.9.1 avatar plugin, in insertion order.
const CV: &str = r#"{"cvs":[{"cv":3,"styleId":"1"},{"cv":3,"styleId":"2"},{"cv":3,"styleId":"3"},{"cv":3,"styleId":"4"}]}"#;

fn acknowledged(value: &serde_json::Value) -> Result<()> {
    let code = value.get("code").and_then(serde_json::Value::as_str);
    if code == Some("000000") {
        return Ok(());
    }
    // 200004 is the upload producer's explicit rejected/notice branch.
    let mut failure = error(
        if code == Some("200004") {
            ErrorCode::PermissionDenied
        } else {
            ErrorCode::UpstreamError
        },
        "Migu native avatar request was not acknowledged",
    );
    if let Some(code) =
        code.filter(|code| code.len() == 6 && code.bytes().all(|byte| byte.is_ascii_digit()))
    {
        failure = failure.with_details(json!({"upstream_code":code}));
    }
    Err(failure)
}

impl MiguClient {
    pub(crate) async fn native_avatar_used_static(
        &self,
        auth: &NativeAuthorization,
    ) -> Result<bool> {
        let value = self
            .native_get(
                "app.c.nf.migu.cn",
                USAGE_PATH,
                &auth.token,
                Some(&auth.uid),
                vec![("functionKey", USAGE_KEY)],
                false,
            )
            .await?;
        acknowledged(&value)?;
        match value
            .get("data")
            .and_then(|data| data.get(USAGE_KEY))
            .and_then(serde_json::Value::as_u64)
        {
            Some(0) => Ok(false),
            Some(1) => Ok(true),
            _ => Err(migu_upstream_error(
                "Migu avatar usage marker is missing or invalid",
            )),
        }
    }

    pub(crate) async fn convert_native_avatar_to_static(
        &self,
        auth: &NativeAuthorization,
    ) -> Result<()> {
        let value = self
            .native_form(
                "app.c.nf.migu.cn",
                CONVERT_PATH,
                &auth.token,
                &auth.uid,
                vec![("convertType", "0")],
            )
            .await?;
        acknowledged(&value)
    }

    pub(crate) async fn mark_native_avatar_static_used(
        &self,
        auth: &NativeAuthorization,
    ) -> Result<()> {
        // UpdateProfilePhotoModeParam as emitted by JSON.toJSONString in the
        // official conversion callback. This is one avatar-use flag only.
        let value = self
            .native_json(
                "app.c.nf.migu.cn",
                USAGE_WRITE_PATH,
                &auth.token,
                &auth.uid,
                br#"[{"functionKey":"personal:avatar2d:tmp:used","functionValue":1}]"#,
            )
            .await?;
        acknowledged(&value)
    }

    pub(crate) async fn native_avatar_static_mode(
        &self,
        auth: &NativeAuthorization,
    ) -> Result<bool> {
        let value = self
            .native_get(
                "app.u.nf.migu.cn",
                MODE_PATH,
                &auth.token,
                Some(&auth.uid),
                vec![("uid", &auth.uid), ("sceneType", "crbt"), ("cv", CV)],
                false,
            )
            .await?;
        acknowledged(&value)?;
        match value
            .pointer("/data/avatarIconType")
            .and_then(serde_json::Value::as_u64)
        {
            Some(0) => Ok(true),
            Some(1) => Ok(false),
            _ => Err(migu_upstream_error(
                "Migu avatar mode is missing or invalid",
            )),
        }
    }

    pub(crate) async fn native_avatar_pending(&self, auth: &NativeAuthorization) -> Result<bool> {
        let value = self
            .native_get(
                "app.c.nf.migu.cn",
                AUDIT_PATH,
                &auth.token,
                Some(&auth.uid),
                vec![],
                false,
            )
            .await?;
        acknowledged(&value)?;
        // The edit UI checks the per-field types, not the global status flag.
        let types: Vec<String> =
            serde_json::from_value(value.pointer("/data/types").cloned().ok_or_else(|| {
                migu_upstream_error("Migu avatar audit response omitted its field types")
            })?)
            .map_err(|_| migu_upstream_error("Migu avatar audit field types are invalid"))?;
        Ok(types.iter().any(|kind| kind == "4"))
    }

    pub(crate) async fn native_avatar_profile(&self, auth: &NativeAuthorization) -> Result<()> {
        self.native_profile_response(auth).await.map(|_| ())
    }

    pub(super) async fn native_profile_response(
        &self,
        auth: &NativeAuthorization,
    ) -> Result<serde_json::Value> {
        let value = self
            .native_get(
                "app.u.nf.migu.cn",
                PROFILE_PATH,
                &auth.token,
                Some(&auth.uid),
                vec![("userId", &auth.uid)],
                true,
            )
            .await?;
        acknowledged(&value)?;
        // queryBatchUserInfo returns userInfoItem at the root, unlike
        // token-validate. Never infer the owner from the requested UID.
        let uid = value
            .pointer("/userInfoItem/userId")
            .and_then(serde_json::Value::as_str)
            .filter(|uid| validate_uid(uid).is_ok())
            .ok_or_else(|| {
                migu_upstream_error("Migu avatar profile omitted its account identity")
            })?;
        if uid != auth.uid {
            return Err(error(
                ErrorCode::PermissionDenied,
                "Migu avatar profile belongs to a different account",
            ));
        }
        Ok(value)
    }

    pub(crate) async fn upload_native_account_avatar(
        &self,
        auth: &NativeAuthorization,
        bytes: &[u8],
    ) -> Result<()> {
        let value = self
            .native_image_post(
                "app.c.nf.migu.cn",
                "/MIGUM2.0/v1.0/picUpload.do",
                &auth.token,
                &auth.uid,
                vec![("syncOtherApp", "false"), ("type", "00")],
                bytes,
            )
            .await?;
        acknowledged(&value)
    }
}
