//! Concept l0 ordinary creation uses plaintext v4 JSON and platform visibility.
use super::*;

const PATH: &str = "/cloudlist.service/v4/add_list";

#[derive(Deserialize)]
struct Receipt {
    userid: Number,
    total_ver: Number,
    pre_total_ver: Number,
    list_count: Number,
    info: Info,
}
#[derive(Deserialize)]
struct Info {
    code: Number,
    listid: Number,
    #[serde(rename = "type")]
    kind: Number,
    name: String,
    source: Option<Number>,
    global_collection_id: Option<String>,
}

fn acknowledge(bytes: &[u8], uid: &str, name: &str, version: u64) -> Result<ListAck> {
    let status: Response<IgnoredAny> = serde_json::from_slice(bytes).map_err(|_| malformed())?;
    if status.status != 1 || status.error_code.is_some_and(|v| v != 0) {
        let code = if status.status == 0 && status.error_code == Some(20017) {
            ErrorCode::AuthenticationRequired
        } else {
            ErrorCode::UpstreamError
        };
        return Err(error(code, "KuGou Concept playlist creation was rejected")
            .with_details(json!({"platform_code":status.error_code})));
    }
    let receipt = serde_json::from_slice::<Response<Receipt>>(bytes)
        .map_err(|_| malformed())?
        .data
        .ok_or_else(malformed)?;
    if receipt.info.code.0 != 1 {
        return Err(error(
            ErrorCode::UpstreamError,
            "KuGou Concept playlist creation was not acknowledged",
        ));
    }
    if receipt.userid.0.to_string() != uid
        || receipt.info.kind.0 != 0
        || receipt.info.name != name
        || receipt.info.source.is_some_and(|v| v.0 != 1)
        || receipt.pre_total_ver.0 != version
        || receipt.total_ver.0 < version
    {
        return Err(identity_conflict());
    }
    Ok(ListAck {
        list_id: Some(positive(receipt.info.listid)?),
        total_ver: Some(receipt.total_ver.0),
        previous_ver: Some(receipt.pre_total_ver.0),
        list_count: Some(receipt.list_count.0),
        gid: gid(receipt.info.global_collection_id)?,
    })
}

impl KugouClient {
    pub(crate) async fn native_create_concept_list(
        &self,
        session: &NativeSession,
        name: &str,
        version: u64,
    ) -> Result<ListAck> {
        validate_session(session)?;
        if session.client != KugouLoginClient::Concept {
            return Err(error(
                ErrorCode::CapabilityNotSupported,
                "KuGou v4 default creation requires a Concept credential",
            ));
        }
        // The official worker truncates names to 60 UTF-8 bytes. Reject instead
        // of silently changing the caller's requested name.
        checked_text(name, 60, false)?;
        let uid = session.user_id.parse::<u64>().map_err(|_| malformed())?;
        let body = crypto::encode(
            &json!({"userid":uid,"token":session.token,"total_ver":version,
            "name":name,"type":0,"source":1,"list_create_userid":0,"list_create_listid":0}),
        )?;
        self.native_concept_plaintext_list(
            session,
            PATH,
            "concept_playlist_create_default",
            body,
            |bytes| acknowledge(bytes, &session.user_id, name, version),
        )
        .await
    }
}

#[cfg(test)]
mod tests;
