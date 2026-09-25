//! Concept n0/g1 metadata protocol. The v4 cover producer is separate.
use super::*;
use crate::account::cloud::Cipher;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};

mod batch_delete;
mod create;
mod delete;

const PATH: &str = "/cloudlist.service/v1/modify_list";
const APP_KEY: &str = "LnT6xpN3khm36zse0QzvmgTZ3waWdRSA";

enum BodyEncoding {
    Binary,
    Base64,
}

// EntityUtils.toString(entity, "utf-8") decodes the ciphertext before signing.
// Java consumes an encoded UTF-16 surrogate (or its incomplete two-byte prefix)
// as one replacement. Rust's lossy decoder instead replaces each byte there.
fn signed_body(bytes: &[u8]) -> String {
    let mut out = String::new();
    let mut start = 0;
    let mut pos = 0;
    while pos + 1 < bytes.len() {
        if bytes[pos] == 0xed && (0xa0..=0xbf).contains(&bytes[pos + 1]) {
            out.push_str(&String::from_utf8_lossy(&bytes[start..pos]));
            out.push('\u{fffd}');
            pos += 2;
            if bytes.get(pos).is_some_and(|v| (0x80..=0xbf).contains(v)) {
                pos += 1;
            }
            start = pos;
        } else {
            pos += 1;
        }
    }
    out.push_str(&String::from_utf8_lossy(&bytes[start..]));
    out
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
    total_ver: Number,
    pre_total_ver: Number,
    list_count: Number,
    info: ReceiptInfo,
}
#[derive(Deserialize)]
struct ReceiptInfo {
    code: Number,
    listid: Number,
    #[serde(rename = "type")]
    kind: Number,
    name: String,
    sort: Number,
}

fn acknowledge(bytes: &[u8], uid: &str, list_id: u64, edit: &ListEdit) -> Result<ListAck> {
    // n0.c consumes status and data.info.code; error_code is not mandatory in
    // its success response. Explicit gateway failures still take precedence.
    let status: Response<IgnoredAny> = serde_json::from_slice(bytes).map_err(|_| malformed())?;
    if status.status != 1 || status.error_code.is_some_and(|v| v != 0) {
        let code = if status.status == 0 && status.error_code == Some(20017) {
            ErrorCode::AuthenticationRequired
        } else {
            ErrorCode::UpstreamError
        };
        return Err(error(code, "KuGou Concept metadata request was rejected")
            .with_details(json!({"platform_code": status.error_code})));
    }
    let receipt = serde_json::from_slice::<Response<Receipt>>(bytes)
        .map_err(|_| malformed())?
        .data
        .ok_or_else(malformed)?;
    if receipt.info.code.0 != 1 {
        return Err(error(
            ErrorCode::UpstreamError,
            "KuGou Concept metadata edit was not acknowledged",
        ));
    }
    if receipt.userid.0.to_string() != uid
        || receipt.info.listid.0 != list_id
        || receipt.info.kind.0 != 0
        || receipt.info.name != edit.name
        || receipt.info.sort.0 != edit.sort
        || receipt.pre_total_ver.0 != edit.total_ver
        || receipt.total_ver.0 < receipt.pre_total_ver.0
    {
        return Err(identity_conflict());
    }
    Ok(ListAck {
        list_id: Some(list_id),
        total_ver: Some(receipt.total_ver.0),
        previous_ver: Some(receipt.pre_total_ver.0),
        list_count: Some(receipt.list_count.0),
        gid: None,
    })
}

impl KugouClient {
    // l0 create and m0 single-delete replace k.v's query with ParamGenerator.n
    // (false), then g1 signs the exact plaintext body. Neither uses n0's AES
    // transport below. Callers supply only their fixed official path.
    pub(in crate::account::library) async fn native_concept_plaintext_list<T>(
        &self,
        session: &NativeSession,
        path: &'static str,
        operation: &'static str,
        body: Vec<u8>,
        decode: impl FnOnce(&[u8]) -> Result<T>,
    ) -> Result<T> {
        let mut query = BTreeMap::from([
            ("appid", session.client.appid().to_string()),
            ("clientver", session.client.clientver().to_string()),
            ("mid", session.device.mid.clone()),
            ("dfid", session.device.dfid().to_owned()),
            // A2(true) -> SecretAccess.getSafeUUID returns this literal.
            ("uuid", "-".to_owned()),
            ("clienttime", (now_ms()? / 1000).to_string()),
        ]);
        query.insert("signature", concept_signature(&query, &body));
        let url = format!("https://{HOST}{path}");
        #[cfg(test)]
        let url = self
            .login_test_origin
            .as_ref()
            .map(|o| o.join(path).unwrap().to_string())
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
            let bytes = read_response_with_limit(response, RESPONSE_LIMIT).await?;
            decode(&bytes)
        }
        .await;
        self.log_upstream_request(operation, HOST, path, status, started, 0, false, &result);
        result
    }

    pub(in crate::account::library::management) async fn native_modify_concept_list(
        &self,
        session: &NativeSession,
        list_id: u64,
        edit: &ListEdit,
    ) -> Result<ListAck> {
        validate_session(session)?;
        if session.client != KugouLoginClient::Concept {
            return Err(error(
                ErrorCode::CapabilityNotSupported,
                "KuGou v1 metadata editing requires a Concept credential",
            ));
        }
        if list_id == 0 {
            return Err(malformed());
        }
        checked_text(&edit.name, 1024, false)?;
        checked_text(&edit.intro, 16384, true)?;
        checked_text(&edit.tags, 4096, true)?;
        let body = crypto::encode(&json!({
            "total_ver":edit.total_ver,"listid":list_id,"type":0,"name":edit.name,
            "sort":edit.sort,"tags":edit.tags,"intro":edit.intro,
        }))?;
        self.native_concept_encrypted_list(
            session,
            PATH,
            "concept_playlist_metadata",
            &body,
            BodyEncoding::Binary,
            |bytes| acknowledge(bytes, &session.user_id, list_id, edit),
        )
        .await
    }

    // n0 metadata and m0's ArrayList branch retain g1/k's AES identity query.
    // Plaintext create/single-delete deliberately use the separate helper above.
    async fn native_concept_encrypted_list<T>(
        &self,
        session: &NativeSession,
        path: &'static str,
        operation: &'static str,
        body: &[u8],
        encoding: BodyEncoding,
        decode: impl FnOnce(&[u8]) -> Result<T>,
    ) -> Result<T> {
        let cipher = Cipher::random()?;
        let encrypted = cipher.encode(body)?;
        // m0 uses useraccount.utils.c.b then StringEntity; n0 uses ByteArrayEntity.
        let body = match encoding {
            BodyEncoding::Binary => encrypted,
            BodyEncoding::Base64 => BASE64.encode(encrypted).into_bytes(),
        };
        let seconds = now_ms()? / 1000;
        let mut params = BTreeMap::from([
            ("appid", session.client.appid().to_string()),
            ("clientver", session.client.clientver().to_string()),
            ("mid", session.device.mid.clone()),
            ("clienttime", seconds.to_string()),
            (
                "key",
                format!(
                    "{:x}",
                    Md5::digest(format!(
                        "{}{APP_KEY}{}{seconds}",
                        session.client.appid(),
                        session.client.clientver()
                    ))
                ),
            ),
            ("dfid", session.device.dfid().to_owned()),
            ("p", cipher.portrait(session)?),
        ]);
        // b1.p adds last_area/time only from a prior routing cache. This client
        // has no such cache; omit both, matching the official empty-cache case.
        params.insert(
            "signature",
            concept_signature(&params, signed_body(&body).as_bytes()),
        );
        let url = format!("https://{HOST}{path}");
        #[cfg(test)]
        let url = self
            .login_test_origin
            .as_ref()
            .map(|o| o.join(path).unwrap().to_string())
            .unwrap_or(url);
        let started = Instant::now();
        let mut status = None;
        let result = async {
            let response = self
                .http
                .post(url)
                .query(&params)
                .header(CONTENT_TYPE, "application/json;charset=utf-8")
                .body(body)
                .send()
                .await
                .map_err(network_error)?;
            status = Some(response.status());
            let bytes = read_response_with_types(
                response,
                RESPONSE_LIMIT,
                &["application/json", "application/octet-stream"],
            )
            .await?;
            decode(&cipher.decode(&bytes)?)
        }
        .await;
        self.log_upstream_request(operation, HOST, path, status, started, 0, false, &result);
        result
    }
}

#[cfg(test)]
mod tests;
