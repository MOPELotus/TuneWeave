//! Concept m0's single-Playlist constructor uses plaintext v3, unlike batch deletion.
use super::*;

const PATH: &str = "/cloudlist.service/v3/delete_list";

#[derive(Deserialize)]
struct Receipt {
    userid: Number,
    total_ver: Number,
    pre_total_ver: Number,
    list_count: Number,
}

fn acknowledge(bytes: &[u8], uid: &str, version: u64) -> Result<ListAck> {
    let status: Response<IgnoredAny> = serde_json::from_slice(bytes).map_err(|_| malformed())?;
    if status.status != 1 || status.error_code.is_some_and(|v| v != 0) {
        let code = if status.status == 0 && status.error_code == Some(20017) {
            ErrorCode::AuthenticationRequired
        } else {
            ErrorCode::UpstreamError
        };
        return Err(error(code, "KuGou Concept playlist deletion was rejected")
            .with_details(json!({"platform_code": status.error_code})));
    }
    let receipt = serde_json::from_slice::<Response<Receipt>>(bytes)
        .map_err(|_| malformed())?
        .data
        .ok_or_else(malformed)?;
    if receipt.userid.0.to_string() != uid
        || receipt.pre_total_ver.0 != version
        || receipt.total_ver.0 < version
    {
        return Err(identity_conflict());
    }
    // The single-delete consumer has no returned listid or nested success code.
    // The provider binds the requested identity through complete library readback.
    Ok(ListAck {
        list_id: None,
        total_ver: Some(receipt.total_ver.0),
        previous_ver: Some(receipt.pre_total_ver.0),
        list_count: Some(receipt.list_count.0),
        gid: None,
    })
}

impl KugouClient {
    pub(in crate::account::library::management) async fn native_delete_concept_list(
        &self,
        session: &NativeSession,
        list_id: u64,
        kind: u8,
        version: u64,
    ) -> Result<ListAck> {
        validate_session(session)?;
        if session.client != KugouLoginClient::Concept {
            return Err(error(
                ErrorCode::CapabilityNotSupported,
                "KuGou v3 single deletion requires a Concept credential",
            ));
        }
        if list_id == 0 || kind > 1 {
            return Err(malformed());
        }
        let uid = session.user_id.parse::<u64>().map_err(|_| malformed())?;
        let body = crypto::encode(&json!({
            "userid":uid,"token":session.token,"listid":list_id,"total_ver":version,"type":kind,
        }))?;
        // Deliberately do not reproduce m0.d's transport retry for a mutation.
        self.native_concept_plaintext_list(
            session,
            PATH,
            "concept_playlist_delete_single",
            body,
            |bytes| acknowledge(bytes, &session.user_id, version),
        )
        .await
    }
}

#[cfg(test)]
mod tests;
