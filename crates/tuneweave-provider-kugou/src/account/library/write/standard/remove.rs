//! Standard 20.8 non-incremental CloudMusicDeleteFileRequestor.
use super::*;

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
        return Err(error(
            if status.status == 0 && status.error_code == Some(20017) {
                ErrorCode::AuthenticationRequired
            } else {
                ErrorCode::UpstreamError
            },
            "KuGou Standard track removal was rejected",
        )
        .with_details(json!({"platform_code":status.error_code})));
    }
    // v0.b consumes a playlist receipt, not add_song's per-item codes.
    // The provider must confirm the entire requested delta by full readback.
    let receipt = serde_json::from_slice::<Response<Receipt>>(bytes)
        .map_err(|_| malformed())?
        .data
        .ok_or_else(malformed)?;
    if receipt.userid.0.to_string() != uid
        || receipt.listid.0 != list_id
        || receipt.pre_list_ver.0 != version
        || receipt.list_ver.0 <= version
        || receipt.list_ver.0 > i32::MAX as u64
        || receipt.count.0 > i32::MAX as u64
        || receipt.kind.is_some_and(|v| v.0 != 0)
    {
        return Err(identity_conflict());
    }
    Ok(WriteAck {
        version: Some(receipt.list_ver.0),
        previous_version: Some(version),
        count: Some(receipt.count.0),
    })
}

impl KugouClient {
    pub(crate) async fn native_remove_standard_tracks(
        &self,
        session: &NativeSession,
        list_id: u64,
        version: u64,
        files: &[u64],
    ) -> Result<WriteAck> {
        validate_session(session)?;
        if session.client != KugouLoginClient::Standard {
            return Err(error(
                ErrorCode::CapabilityNotSupported,
                "KuGou versioned Standard removal requires its matching client profile",
            ));
        }
        // This is a local request budget, not a claimed upstream batch limit.
        if list_id == 0
            || list_id > i32::MAX as u64
            || version > i32::MAX as u64
            || files.is_empty()
            || files.len() > WRITE_BATCH_SIZE
            || files.iter().any(|id| *id == 0 || *id > i32::MAX as u64)
            || files.iter().collect::<BTreeSet<_>>().len() != files.len()
        {
            return Err(error(
                ErrorCode::InvalidRequest,
                "KuGou Standard removal input is invalid",
            ));
        }
        let uid = session.user_id.parse::<i64>().map_err(|_| malformed())?;
        let body = crypto::encode(&json!({"userid":uid,"token":session.token,
            "listid":list_id,"list_ver":version,"type":0,"scene":"false,2",
            "data":files.iter().map(|id|json!({"fileid":id})).collect::<Vec<_>>()}))?;
        // ParamGenerator.p uses the fixed APK profile and common_uuid bit 41.
        // v0.a replaces mParams, overrides s(), and does not add query credentials.
        let mut query = BTreeMap::from([
            ("appid", "1005".to_owned()),
            ("clientver", "20809".to_owned()),
            ("clienttime", (now_ms()? / 1000).to_string()),
            ("mid", session.device.mid.clone()),
            ("dfid", session.device.dfid().to_owned()),
            ("uuid", "-".to_owned()),
        ]);
        // g2.w only adds cached server region/time; no such cache exists here.
        query.insert("signature", android_signature(&query, &body));
        let url = format!("https://{HOST}{PATH}");
        #[cfg(test)]
        let url = self
            .login_test_origin
            .as_ref()
            .map(|origin| origin.join(PATH).unwrap().to_string())
            .unwrap_or(url);
        let started = Instant::now();
        let mut status = None;
        let result = async {
            let response = self
                .http
                .post(url)
                .query(&query)
                .header(CONTENT_TYPE, "application/json;charset=utf-8")
                .body(body)
                .send()
                .await
                .map_err(network_error)?;
            status = Some(response.status());
            let bytes = read_response_with_limit(response, 1_048_576).await?;
            acknowledge(&bytes, &session.user_id, list_id, version)
        }
        .await;
        self.log_upstream_request(
            "standard_track_remove",
            HOST,
            PATH,
            status,
            started,
            0,
            false,
            &result,
        );
        result
    }
}

#[cfg(test)]
mod tests;
