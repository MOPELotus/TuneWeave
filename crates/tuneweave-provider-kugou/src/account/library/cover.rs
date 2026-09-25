//! Native cover authorization, client-specific JPEG upload and cloud-list attachment.
use super::management::{AckPolicy, ListAck, acknowledge, random_seed, token_fields};
use super::*;
use md5::{Digest, Md5};

// Version of the Standard APK that supplies these cover protocols.
const CLIENT_VERSION: u32 = 20809;
const CONCEPT_CLIENT_VERSION: u32 = 11490;

pub(crate) struct CoverAuthorization(String);
pub(crate) struct UploadedCover(String);
impl UploadedCover {
    pub(crate) fn filename(&self) -> &str {
        &self.0
    }
}
pub(crate) struct CoverAck {
    pub(crate) list: ListAck,
    pub(crate) url: String,
}

#[derive(Clone, Copy)]
enum Route {
    Authorization,
    Upload,
    Save,
}
impl Route {
    fn host(self) -> &'static str {
        match self {
            Self::Authorization => "bsstrackercdngz.kugou.com",
            Self::Upload => "imgphpulssl.kugou.com",
            Self::Save => HOST,
        }
    }
    fn path(self) -> &'static str {
        match self {
            Self::Authorization => "/v1/upload/auth",
            Self::Upload => "/imageupload/v3/stream.php",
            Self::Save => "/cloudlist.service/v4/modify_list",
        }
    }
    fn operation(self) -> &'static str {
        match self {
            Self::Authorization => "native_cover_authorization",
            Self::Upload => "native_cover_upload",
            Self::Save => "native_cover_save",
        }
    }
}

fn native(session: &NativeSession) -> Result<()> {
    validate_session(session)?;
    if session.client == KugouLoginClient::Web {
        return Err(error(
            ErrorCode::CapabilityNotSupported,
            "KuGou cover upload requires a native app credential",
        ));
    }
    Ok(())
}
fn parameters(session: &NativeSession, seconds: u64) -> BTreeMap<&'static str, String> {
    BTreeMap::from([
        ("appid", session.client.appid().to_string()),
        (
            "clientver",
            if session.client == KugouLoginClient::Concept {
                CONCEPT_CLIENT_VERSION
            } else {
                CLIENT_VERSION
            }
            .to_string(),
        ),
        ("clienttime", seconds.to_string()),
        ("mid", session.device.mid.clone()),
        ("dfid", session.device.dfid().to_owned()),
    ])
}
fn authorization_parameters(
    session: &NativeSession,
    seconds: u64,
) -> BTreeMap<&'static str, String> {
    let mut params = parameters(session, seconds);
    params.extend([
        ("bucket", "custom".into()),
        ("loginType", "1".into()),
        ("extranet", "1".into()),
        (
            "buVerifyCode",
            hex::encode(Md5::digest(
                format!("{}custom770d48f3413de7b8", session.client.appid()).as_bytes(),
            )),
        ),
        ("userid", session.user_id.clone()),
        ("token", session.token.clone()),
        // Both archived clients explicitly enable their safe UUID placeholder.
        ("uuid", "-".into()),
    ]);
    if session.client == KugouLoginClient::Standard {
        params.insert("method", "POST".into());
    }
    params
}
fn authorization(bytes: &[u8], client: KugouLoginClient) -> Result<CoverAuthorization> {
    #[derive(Deserialize)]
    struct Auth {
        authorization: Option<String>,
        authorizations: Option<Vec<String>>,
    }
    let wire: Auth = data(bytes)?;
    if wire
        .authorizations
        .as_ref()
        .is_some_and(|values| values.len() > 64)
    {
        return Err(malformed());
    }
    let value = wire.authorization.filter(|value| !value.trim().is_empty());
    // Concept AuthData.b uses element zero, never a search for another ticket.
    let value = value
        .or_else(|| {
            (client == KugouLoginClient::Concept)
                .then(|| {
                    wire.authorizations
                        .and_then(|values| values.into_iter().next())
                })
                .flatten()
        })
        .ok_or_else(malformed)?;
    if value.trim().is_empty() || value.len() > 16_384 || value.chars().any(char::is_control) {
        return Err(malformed());
    }
    Ok(CoverAuthorization(value))
}

fn concept_upload_parameters(
    session: &NativeSession,
    authorization: CoverAuthorization,
    time: chrono::DateTime<chrono::FixedOffset>,
) -> Result<BTreeMap<&'static str, String>> {
    use chrono::Datelike;
    if !(1970..=9999).contains(&time.year()) {
        return Err(malformed());
    }
    let seconds = u64::try_from(time.timestamp()).map_err(|_| malformed())?;
    let mut params = parameters(session, seconds);
    params.extend([
        ("uuid", "-".into()),
        ("userid", session.user_id.clone()),
        ("token", session.token.clone()),
        ("type", "custom".into()),
        ("extendName", ".jpg".into()),
        (
            "md5",
            hex::encode(Md5::digest(format!(
                "{}hewry678WEK23D",
                time.format("%Y%m%d")
            ))),
        ),
        ("jsonResponse", "1".into()),
        ("authorization", authorization.0),
        ("body_empty", "1".into()),
    ]);
    Ok(params)
}
fn uploaded(bytes: &[u8]) -> Result<UploadedCover> {
    #[derive(Deserialize)]
    struct Wire {
        #[serde(rename = "IsSuccess")]
        success: bool,
        #[serde(rename = "FileName")]
        filename: Option<String>,
    }
    let wire: Wire = serde_json::from_slice(bytes).map_err(|_| malformed())?;
    if !wire.success {
        return Err(error(
            ErrorCode::UpstreamError,
            "KuGou cover upload was not acknowledged",
        ));
    }
    let filename = wire.filename.ok_or_else(malformed)?;
    if filename.is_empty()
        || filename.len() > 1024
        || filename.split('/').any(|s| {
            s.is_empty()
                || s == "."
                || s == ".."
                || !s
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
        })
    {
        return Err(malformed());
    }
    Ok(UploadedCover(filename))
}

impl KugouClient {
    pub(crate) async fn native_cover_authorization(
        &self,
        session: &NativeSession,
    ) -> Result<CoverAuthorization> {
        native(session)?;
        let bytes = self
            .cover_request(
                Route::Authorization,
                session.client,
                authorization_parameters(session, now_ms()? / 1000),
                Vec::new(),
            )
            .await?;
        authorization(&bytes, session.client)
    }
    pub(crate) async fn native_upload_cover(
        &self,
        session: &NativeSession,
        authorization: CoverAuthorization,
        jpeg: Vec<u8>,
    ) -> Result<UploadedCover> {
        native(session)?;
        if session.client == KugouLoginClient::Concept {
            // Official Q() uses the local wall clock when no network-layer epoch
            // correction is established. Snapshot the request environment's date
            // after authorization, without reading an unrelated HTTP Date header.
            let params = concept_upload_parameters(
                session,
                authorization,
                chrono::Local::now().fixed_offset(),
            )?;
            return uploaded(
                &self
                    .cover_request(Route::Upload, session.client, params, jpeg)
                    .await?,
            );
        }
        let mut params = parameters(session, now_ms()? / 1000);
        params.extend([
            ("uuid", "-".into()),
            ("type", "custom".into()),
            ("extendName", ".jpg".into()),
            ("iscovered", "1".into()),
            ("jsonResponse", "1".into()),
            ("authorization", authorization.0),
            ("body_empty", "1".into()),
        ]);
        uploaded(
            &self
                .cover_request(Route::Upload, session.client, params, jpeg)
                .await?,
        )
    }
    pub(crate) async fn native_save_cover(
        &self,
        session: &NativeSession,
        list_id: u64,
        total_ver: u64,
        global_id: &str,
        upload: &UploadedCover,
    ) -> Result<CoverAck> {
        native(session)?;
        if list_id == 0 || gid(Some(global_id.to_owned()))?.is_none() {
            return Err(malformed());
        }
        let (enckey, encstr) = token_fields(session.client, &session.token, &random_seed()?)?;
        let body = crypto::encode(
            &json!({"userid":session.user_id.parse::<u64>().map_err(|_| malformed())?,
            "total_ver":total_ver,"type":0,"listid":list_id,"pic":format!("custom/{}",upload.0),
            "list_create_gid":global_id,"support_pub":1,"enckey":enckey,"encstr":encstr}),
        )?;
        let bytes = self
            .cover_request(
                Route::Save,
                session.client,
                parameters(session, now_ms()? / 1000),
                body,
            )
            .await?;
        let list = acknowledge(
            &bytes,
            &session.user_id,
            0,
            Some(list_id),
            if session.client == KugouLoginClient::Concept {
                AckPolicy::ConceptCover
            } else {
                AckPolicy::StandardChange
            },
        )?;
        #[derive(Deserialize)]
        struct Info {
            pic: String,
        }
        #[derive(Deserialize)]
        struct Saved {
            info: Info,
        }
        let saved: Saved = data(&bytes)?;
        if saved.info.pic.len() > 4096 {
            return Err(malformed());
        }
        let url = normalize_image_url(&saved.info.pic).ok_or_else(malformed)?;
        Ok(CoverAck { list, url })
    }
    async fn cover_request(
        &self,
        route: Route,
        client: KugouLoginClient,
        mut params: BTreeMap<&str, String>,
        body: Vec<u8>,
    ) -> Result<Vec<u8>> {
        // body_empty=1 deliberately signs no JPEG bytes for the upload endpoint.
        let signed_body = if matches!(route, Route::Save) {
            body.as_slice()
        } else {
            &[]
        };
        let signature = if client == KugouLoginClient::Concept {
            concept_signature(&params, signed_body)
        } else {
            android_signature(&params, signed_body)
        };
        params.insert("signature", signature);
        let url = format!("https://{}{}", route.host(), route.path());
        #[cfg(test)]
        let url = self
            .login_test_origin
            .as_ref()
            .map(|origin| origin.join(route.path()).unwrap().to_string())
            .unwrap_or(url);
        let started = Instant::now();
        let mut status = None;
        let result = async {
            let request = if matches!(route, Route::Authorization) {
                self.http.get(url)
            } else {
                self.http.post(url)
            };
            let request = request.query(&params).header("accept", "application/json");
            let request = match route {
                Route::Authorization => request,
                Route::Upload => request
                    .header(CONTENT_TYPE, "application/octet-stream")
                    .body(body),
                Route::Save => request
                    .header(CONTENT_TYPE, "application/json")
                    .header("KG-MODULE", "27")
                    .body(body),
            };
            let response = request.send().await.map_err(network_error)?;
            status = Some(response.status());
            read_response_with_limit(response, RESPONSE_LIMIT).await
        }
        .await;
        self.log_upstream_request(
            route.operation(),
            route.host(),
            route.path(),
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
