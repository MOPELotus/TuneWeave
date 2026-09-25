//! Standard 20.8 normal CloudMusicSetFileRequestor, one bounded request.
use super::*;

const PATH: &str = "/cloudlist.service/v6/add_song";
const APP_ID: u16 = 1005;
const CLIENT_VERSION: u32 = 20809;
const MAX_ITEMS: usize = 100;

pub(crate) struct StandardAddedTrack {
    file_id: u64,
    sort: u64,
    hash: String,
    album_id: String,
    mixsongid: u64,
}

impl StandardAddedTrack {
    pub(crate) fn matches(&self, track: &Track) -> bool {
        track.platform == Platform::Kugou
            && track.id == self.mixsongid.to_string()
            && track.extensions.get("file_id").and_then(Value::as_u64) == Some(self.file_id)
            && track.extensions.get("sort").and_then(Value::as_u64) == Some(self.sort)
            && track
                .extensions
                .get("hash")
                .and_then(Value::as_str)
                .is_some_and(|hash| hash.eq_ignore_ascii_case(&self.hash))
            && track
                .album
                .as_ref()
                .and_then(|album| album.resource_ref.as_ref())
                .is_some_and(|album| {
                    album.platform() == Platform::Kugou && album.id() == self.album_id
                })
    }
}

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
    is_edit: Option<Number>,
    #[serde(default)]
    del_fileids: Vec<Number>,
    info: Vec<Item>,
}

#[derive(Deserialize)]
struct Item {
    fileid: Number,
    name: String,
    sort: Number,
    hash: String,
    album_id: Number,
    mixsongid: Number,
    code: Option<Number>,
    csong: Option<Number>,
}

fn acknowledge(
    bytes: &[u8],
    uid: &str,
    list_id: u64,
    version: u64,
    inputs: &[&StandardTrackInput],
) -> Result<(WriteAck, Vec<StandardAddedTrack>)> {
    let status: Response<IgnoredAny> = serde_json::from_slice(bytes).map_err(|_| malformed())?;
    if status.status != 1 || status.error_code.is_some_and(|v| v != 0) {
        let code = if status.status == 0 && status.error_code == Some(20017) {
            ErrorCode::AuthenticationRequired
        } else {
            ErrorCode::UpstreamError
        };
        return Err(error(code, "KuGou Standard track addition was rejected")
            .with_details(json!({"platform_code":status.error_code})));
    }
    let receipt = serde_json::from_slice::<Response<Receipt>>(bytes)
        .map_err(|_| malformed())?
        .data
        .ok_or_else(malformed)?;
    if receipt.userid.0.to_string() != uid
        || receipt.listid.0 != list_id
        || receipt.pre_list_ver.0 != version
        || receipt.list_ver.0 <= version
        || receipt.list_ver.0 > i32::MAX as u64
        || receipt.kind.is_some_and(|v| v.0 != 0)
    {
        return Err(identity_conflict());
    }
    if receipt.is_edit.is_some_and(|v| v.0 != 0) || !receipt.del_fileids.is_empty() {
        return Err(error(
            ErrorCode::UpstreamError,
            "KuGou Standard addition reported an unexpected replacement",
        ));
    }
    if receipt.info.len() != inputs.len() {
        return Err(malformed());
    }
    let mut expected = inputs
        .iter()
        .map(|input| (input.mixsongid, *input))
        .collect::<BTreeMap<_, _>>();
    let mut files = BTreeSet::new();
    let mut items = BTreeMap::new();
    for item in receipt.info {
        let input = expected.remove(&item.mixsongid.0).ok_or_else(malformed)?;
        // Standard's parser defaults a missing code to 1; cloudtool.e only
        // acknowledges that value. Never return a partial batch as success.
        if item.code.is_some_and(|v| v.0 != 1) {
            return Err(error(
                ErrorCode::UpstreamError,
                "KuGou Standard track item was not acknowledged",
            ));
        }
        if item.fileid.0 == 0
            || item.fileid.0 > i32::MAX as u64
            || !files.insert(item.fileid.0)
            || item.sort.0 > i32::MAX as u64
            || text(Some(item.name), 8192)?.is_none()
            || !item.hash.eq_ignore_ascii_case(&input.hash)
            || item.album_id.0.to_string() != input.album_id
            || item.csong.is_some_and(|v| v.0 != 0)
        {
            return Err(malformed());
        }
        items.insert(
            input.mixsongid,
            StandardAddedTrack {
                file_id: item.fileid.0,
                sort: item.sort.0,
                hash: item.hash,
                album_id: input.album_id.clone(),
                mixsongid: input.mixsongid,
            },
        );
    }
    // ACK order is not identity. Return receipts in the original input order,
    // including when the server replies in reversed wire order.
    let items = inputs
        .iter()
        .map(|input| items.remove(&input.mixsongid).ok_or_else(malformed))
        .collect::<Result<Vec<_>>>()?;
    Ok((
        WriteAck {
            version: Some(receipt.list_ver.0),
            previous_version: Some(version),
            count: Some(receipt.count.0),
        },
        items,
    ))
}

impl KugouClient {
    pub(crate) async fn native_add_standard_track(
        &self,
        session: &NativeSession,
        list_id: u64,
        version: u64,
        input: &TrackInput,
    ) -> Result<(WriteAck, StandardAddedTrack)> {
        let (ack, mut items) = self
            .native_add_standard_tracks(session, list_id, version, std::slice::from_ref(input))
            .await?;
        Ok((ack, items.pop().ok_or_else(malformed)?))
    }

    pub(crate) async fn native_add_standard_tracks(
        &self,
        session: &NativeSession,
        list_id: u64,
        version: u64,
        inputs: &[TrackInput],
    ) -> Result<(WriteAck, Vec<StandardAddedTrack>)> {
        validate_session(session)?;
        if session.client != KugouLoginClient::Standard {
            return Err(error(
                ErrorCode::CapabilityNotSupported,
                "KuGou versioned Standard addition requires its matching client profile",
            ));
        }
        if list_id == 0
            || list_id > i32::MAX as u64
            || version > i32::MAX as u64
            || inputs.is_empty()
            || inputs.len() > MAX_ITEMS
        {
            return Err(malformed());
        }
        let mut identities = BTreeSet::new();
        let inputs = inputs
            .iter()
            .map(|input| {
                let TrackWire::Standard(input) = &input.wire else {
                    return Err(malformed());
                };
                if input.mixsongid == 0
                    || input.mixsongid > i64::MAX as u64
                    || !identities.insert(input.mixsongid)
                {
                    return Err(malformed());
                }
                Ok(input)
            })
            .collect::<Result<Vec<_>>>()?;
        // Non-incremental cloudtool.e.r uses the original ordinal; the fresh
        // insertion preference 0 resolves to 2, so a.getPostRequestEntity emits
        // that array backwards. <=100 items fit its official 300-item request.
        let rows = inputs
            .iter()
            .enumerate()
            .rev()
            .map(|(index, input)| {
                let mut row = serde_json::to_value(input).map_err(|_| malformed())?;
                row["sort"] = json!(index);
                Ok(row)
            })
            .collect::<Result<Vec<_>>>()?;
        let body = crypto::encode(&json!({"userid":session.user_id,"token":session.token,
            "listid":list_id,"list_ver":version,"type":0,"slow_upload":1,
            "scene":"false;null","mode":1,"allow_part_fail":1,"data":rows}))?;
        let mut query = BTreeMap::from([
            // This dedicated config profile uses the add_song_v6 APK key;
            // it differs from the appid/clientver used by other Standard APIs.
            ("appid", APP_ID.to_string()),
            ("clientver", CLIENT_VERSION.to_string()),
            ("clienttime", (now_ms()? / 1000).to_string()),
            ("mid", session.device.mid.clone()),
            ("dfid", session.device.dfid().to_owned()),
            // Standard's bundled common_uuid bit 41 bans Android ID here.
            ("uuid", "-".to_owned()),
            ("userid", session.user_id.clone()),
            ("token", session.token.clone()),
        ]);
        // g2.w carries last_area/last_time only from an earlier server response.
        // This client has no such cache; do not fabricate region or timestamp.
        // This producer replaces mParams and bypasses g2.s, so no plat is added.
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
            acknowledge(&bytes, &session.user_id, list_id, version, &inputs)
        }
        .await;
        self.log_upstream_request(
            if inputs.len() == 1 {
                "standard_track_add_single"
            } else {
                "standard_track_add_batch"
            },
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
