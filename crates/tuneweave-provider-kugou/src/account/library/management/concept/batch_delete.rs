//! m0's ArrayList constructor: encrypted v2, not repeated plaintext v3 deletes.
use super::*;

const PATH: &str = "/cloudlist.service/v2/delete_multy_list";

#[derive(Deserialize)]
struct Receipt {
    userid: Number,
    total_ver: Number,
    pre_total_ver: Number,
    list_count: Number,
    info: Vec<Item>,
}
#[derive(Deserialize)]
struct Item {
    code: Number,
    listid: Number,
    #[serde(rename = "type")]
    kind: Option<Number>,
}

fn acknowledge(bytes: &[u8], uid: &str, ids: &[u64], version: u64) -> Result<ListAck> {
    let status: Response<IgnoredAny> = serde_json::from_slice(bytes).map_err(|_| malformed())?;
    if status.status != 1 || status.error_code.is_some_and(|v| v != 0) {
        let code = if status.status == 0 && status.error_code == Some(20017) {
            ErrorCode::AuthenticationRequired
        } else {
            ErrorCode::UpstreamError
        };
        return Err(error(code, "KuGou Concept batch deletion was rejected")
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
    let expected = ids.iter().copied().collect::<BTreeSet<_>>();
    let mut observed = BTreeSet::new();
    for item in receipt.info {
        if !expected.contains(&item.listid.0)
            || !observed.insert(item.listid.0)
            || item.kind.is_some_and(|v| v.0 != 0)
        {
            return Err(identity_conflict());
        }
        if item.code.0 != 1 {
            return Err(error(
                ErrorCode::UpstreamError,
                "KuGou Concept batch deletion did not acknowledge every target",
            ));
        }
    }
    // The APK's per-item consumer checks code/listid. A root status alone does
    // not prove every requested item succeeded; partial receipts remain unconfirmed.
    if observed != expected {
        return Err(identity_conflict());
    }
    Ok(ListAck {
        list_id: None,
        total_ver: Some(receipt.total_ver.0),
        previous_ver: Some(receipt.pre_total_ver.0),
        list_count: Some(receipt.list_count.0),
        gid: None,
    })
}

impl KugouClient {
    pub(crate) async fn native_delete_concept_lists(
        &self,
        session: &NativeSession,
        ids: &[u64],
        version: u64,
    ) -> Result<ListAck> {
        validate_session(session)?;
        if session.client != KugouLoginClient::Concept {
            return Err(error(
                ErrorCode::CapabilityNotSupported,
                "KuGou encrypted batch deletion requires a Concept credential",
            ));
        }
        if !(2..=100).contains(&ids.len())
            || ids.contains(&0)
            || ids.iter().collect::<BTreeSet<_>>().len() != ids.len()
        {
            return Err(error(
                ErrorCode::InvalidRequest,
                "KuGou Concept batch deletion requires 2 to 100 distinct playlist IDs",
            ));
        }
        let body = crypto::encode(&json!({
            "total_ver":version,
            "data":ids.iter().map(|id| json!({"listid":id,"type":0})).collect::<Vec<_>>(),
        }))?;
        // No transport retry: a failed acknowledgement cannot prove no deletion occurred.
        self.native_concept_encrypted_list(
            session,
            PATH,
            "concept_playlist_delete_batch",
            &body,
            BodyEncoding::Base64,
            |bytes| acknowledge(bytes, &session.user_id, ids, version),
        )
        .await
    }
}

#[cfg(test)]
mod tests;
