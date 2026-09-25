//! Concept o/g1 deletes raw file IDs against the observed playlist version.
use super::*;

mod add;

const PATH: &str = "/cloudlist.service/v4/delete_songs";

#[derive(Deserialize)]
struct Response<T> {
    status: i64,
    error_code: Option<i64>,
    data: Option<T>,
}
#[derive(Deserialize)]
struct Receipt {
    userid: Number,
    listid: Number,
    list_ver: Number,
    pre_list_ver: Number,
    count: Number,
    #[serde(rename = "type")]
    kind: Option<Number>,
}

fn acknowledge(bytes: &[u8], uid: &str, list_id: u64, version: u64) -> Result<WriteAck> {
    let status: Response<IgnoredAny> = serde_json::from_slice(bytes).map_err(|_| malformed())?;
    if status.status != 1 || status.error_code.is_some_and(|v| v != 0) {
        let code = if status.status == 0 && status.error_code == Some(20017) {
            ErrorCode::AuthenticationRequired
        } else {
            ErrorCode::UpstreamError
        };
        return Err(error(code, "KuGou Concept track removal was rejected")
            .with_details(json!({"platform_code":status.error_code})));
    }
    let receipt = serde_json::from_slice::<Response<Receipt>>(bytes)
        .map_err(|_| malformed())?
        .data
        .ok_or_else(malformed)?;
    if receipt.userid.0.to_string() != uid
        || receipt.listid.0 != list_id
        || receipt.pre_list_ver.0 != version
        || receipt.list_ver.0 < version
        || receipt.kind.is_some_and(|v| v.0 != 0)
    {
        return Err(identity_conflict());
    }
    Ok(WriteAck {
        version: Some(receipt.list_ver.0),
        previous_version: Some(receipt.pre_list_ver.0),
        count: Some(receipt.count.0),
    })
}

impl KugouClient {
    pub(crate) async fn native_remove_concept_occurrence(
        &self,
        session: &NativeSession,
        list_id: u64,
        file_id: u64,
        version: u64,
    ) -> Result<WriteAck> {
        validate_session(session)?;
        if session.client != KugouLoginClient::Concept {
            return Err(error(
                ErrorCode::CapabilityNotSupported,
                "KuGou versioned single-occurrence removal requires a Concept credential",
            ));
        }
        if list_id == 0 || file_id == 0 {
            return Err(malformed());
        }
        let uid = session.user_id.parse::<u64>().map_err(|_| malformed())?;
        let body = crypto::encode(&json!({"userid":uid,"token":session.token,"listid":list_id,
            "list_ver":version,"type":0,"data":[{"fileid":file_id}]}))?;
        // Same verified ParamGenerator.n -> g1 framing as l0 and m0; no x-router,
        // token query or application-level retry of a potentially applied write.
        self.native_concept_plaintext_list(
            session,
            PATH,
            "concept_track_remove_single",
            body,
            |bytes| acknowledge(bytes, &session.user_id, list_id, version),
        )
        .await
    }
}

#[cfg(test)]
mod tests;
