//! Cloud-list ordering uses RSA-wrapped sessions and AES bodies, not Android JSON signatures.
use super::*;
use crate::client::{
    decrypt_device_registration_response, encrypt_device_profile, rsa_pkcs1_v15_encrypt_for_client,
};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use md5::{Digest, Md5};
use std::collections::BTreeSet;

#[derive(Clone, Copy)]
enum Endpoint {
    Lists,
    Tracks,
}
impl Endpoint {
    fn path(self) -> &'static str {
        match self {
            Self::Lists => "/v1/modify_list_sort",
            Self::Tracks => "/v1/modify_song_sort",
        }
    }
    fn operation(self) -> &'static str {
        match self {
            Self::Lists => "native_playlist_order",
            Self::Tracks => "native_playlist_occurrence_order",
        }
    }
}
#[derive(Serialize)]
pub(crate) struct ListPosition {
    pub(crate) listid: u64,
    #[serde(rename = "type")]
    pub(crate) kind: u8,
    pub(crate) sort: u64,
}
#[derive(Serialize)]
struct FilePosition {
    fileid: u64,
    sort: usize,
}
pub(crate) struct SortAck {
    pub(crate) version: Option<u64>,
    pub(crate) previous_version: Option<u64>,
}
#[derive(Deserialize)]
struct WireAck {
    userid: Option<library::Number>,
    listid: Option<library::Number>,
    #[serde(rename = "type")]
    kind: Option<library::Number>,
    total_ver: Option<library::Number>,
    pre_total_ver: Option<library::Number>,
    list_ver: Option<library::Number>,
    pre_list_ver: Option<library::Number>,
    code: Option<library::Number>,
}
pub(super) struct Cipher {
    seed: String,
}
impl Cipher {
    pub(super) fn random() -> Result<Self> {
        Ok(Self {
            seed: library::management::random_seed()?,
        })
    }
    pub(super) fn encode(&self, body: &[u8]) -> Result<Vec<u8>> {
        BASE64
            .decode(encrypt_device_profile(body, &self.seed)?)
            .map_err(|_| malformed())
    }
    pub(super) fn portrait(&self, session: &NativeSession) -> Result<String> {
        #[derive(Serialize)]
        struct Portrait<'a> {
            aes: &'a str,
            uid: u64,
            token: &'a str,
        }
        let plain = crypto::encode(&Portrait {
            aes: &self.seed,
            uid: session.user_id.parse().map_err(|_| malformed())?,
            token: &session.token,
        })?;
        Ok(rsa_pkcs1_v15_encrypt_for_client(session.client, &plain)?.to_ascii_uppercase())
    }
    pub(super) fn decode(&self, bytes: &[u8]) -> Result<Vec<u8>> {
        match decrypt_device_registration_response(bytes, &self.seed) {
            Ok(plain) => Ok(plain),
            Err(_) => {
                // Gateways can return explicit JSON failures without encryption.
                // Plain JSON success is never accepted as a cloud acknowledgement.
                let status = serde_json::from_slice::<Envelope<IgnoredAny>>(bytes)
                    .map_err(|_| malformed())?;
                if status.status != 1 || status.error_code != 0 {
                    return Err(data::<Value>(bytes).err().unwrap_or_else(malformed));
                }
                Err(error(
                    ErrorCode::UpstreamError,
                    "KuGou encrypted account protocol returned an unencrypted success response",
                ))
            }
        }
    }
}
impl KugouClient {
    pub(crate) async fn native_reorder_lists(
        &self,
        session: &NativeSession,
        version: u64,
        positions: &[ListPosition],
    ) -> Result<SortAck> {
        if positions.is_empty()
            || positions.len() > library::PAGE_SIZE * library::MAX_PAGES as usize
        {
            return Err(malformed());
        }
        let mut ids = BTreeSet::new();
        let mut sorts = BTreeSet::new();
        if positions.iter().any(|r| {
            r.listid == 0
                || r.kind > 1
                || !ids.insert((r.kind, r.listid))
                || !sorts.insert((r.kind, r.sort))
        }) {
            return Err(malformed());
        }
        self.cloud_order(
            session,
            Endpoint::Lists,
            None,
            &json!({"total_ver":version,"data":positions}),
        )
        .await
    }
    pub(crate) async fn native_reorder_files(
        &self,
        session: &NativeSession,
        list_id: u64,
        kind: u8,
        version: u64,
        ids: &[u64],
    ) -> Result<SortAck> {
        if list_id == 0
            || kind > 1
            || ids.is_empty()
            || ids.len()
                > library::tracks::TRACK_PAGE_SIZE * library::tracks::MAX_TRACK_PAGES as usize
            || ids.contains(&0)
            || ids.iter().copied().collect::<BTreeSet<_>>().len() != ids.len()
        {
            return Err(malformed());
        }
        let rows = ids
            .iter()
            .enumerate()
            .map(|(sort, id)| FilePosition { fileid: *id, sort })
            .collect::<Vec<_>>();
        self.cloud_order(
            session,
            Endpoint::Tracks,
            Some((list_id, kind)),
            &json!({"listid":list_id,"list_ver":version,"type":kind,"data":rows}),
        )
        .await
    }
    async fn cloud_order(
        &self,
        session: &NativeSession,
        endpoint: Endpoint,
        target: Option<(u64, u8)>,
        body: &Value,
    ) -> Result<SortAck> {
        validate_session(session)?;
        let cipher = Cipher::random()?;
        #[cfg(test)]
        let cipher = if let Some(seed) = &self.cloud_test_seed {
            Cipher { seed: seed.clone() }
        } else {
            cipher
        };
        let seconds = now_ms()? / 1000;
        let appkey = match session.client {
            KugouLoginClient::Standard => crate::signing::ANDROID_SALT,
            KugouLoginClient::Concept => "LnT6xpN3khm36zse0QzvmgTZ3waWdRSA",
            KugouLoginClient::Web => return Err(malformed()),
        };
        let key = format!(
            "{:x}",
            Md5::digest(format!(
                "{}{appkey}{}{seconds}",
                session.client.appid(),
                session.client.clientver()
            ))
        );
        let params = BTreeMap::from([
            ("appid", session.client.appid().to_string()),
            ("clientver", session.client.clientver().to_string()),
            ("mid", session.device.mid.clone()),
            ("dfid", session.device.dfid().to_owned()),
            ("clienttime", seconds.to_string()),
            ("key", key),
            ("p", cipher.portrait(session)?),
        ]);
        let encrypted = cipher.encode(&crypto::encode(body)?)?;
        let url = format!("https://{HOST}{}", endpoint.path());
        #[cfg(test)]
        let url = self
            .login_test_origin
            .as_ref()
            .map(|o| o.join(endpoint.path()).unwrap().to_string())
            .unwrap_or(url);
        let started = Instant::now();
        let mut status = None;
        let result = async {
            let response = self
                .http
                .post(url)
                .query(&params)
                .header(CONTENT_TYPE, "application/json;charset=utf-8")
                .header("x-router", "cloudlist.service.kugou.com")
                .header("mid", &session.device.mid)
                .header("dfid", session.device.dfid())
                .header("clienttime", seconds)
                .header("kg-rc", "1")
                .header("kg-rec", "1")
                .body(encrypted)
                .send()
                .await
                .map_err(network_error)?;
            status = Some(response.status());
            let bytes = read_response_with_types(
                response,
                1_048_576,
                &["application/json", "application/octet-stream"],
            )
            .await?;
            let plain = cipher.decode(&bytes)?;
            let ack: WireAck = data(&plain)?;
            if ack
                .userid
                .is_some_and(|v| v.0.to_string() != session.user_id)
            {
                return Err(identity_conflict());
            }
            if let Some((id, kind)) = target {
                if ack.listid.is_some_and(|v| v.0 != id)
                    || ack.kind.is_some_and(|v| v.0 != u64::from(kind))
                {
                    return Err(identity_conflict());
                }
            }
            if ack.code.is_some_and(|v| v.0 != 0) {
                return Err(error(
                    ErrorCode::UpstreamError,
                    "KuGou cloud order was not fully acknowledged",
                ));
            }
            let (version, previous) = match endpoint {
                Endpoint::Lists => (ack.total_ver, ack.pre_total_ver),
                Endpoint::Tracks => (ack.list_ver, ack.pre_list_ver),
            };
            Ok(SortAck {
                version: version.map(|v| v.0),
                previous_version: previous.map(|v| v.0),
            })
        }
        .await;
        self.log_upstream_request(
            endpoint.operation(),
            HOST,
            endpoint.path(),
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
pub(crate) mod tests;
