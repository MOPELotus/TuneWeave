//! Request-local account/song grants. They are never stored as login credentials.
use super::library::Number;
use super::*;
use md5::{Digest, Md5};

const AUTH_HOST: &str = "trackercdngz.kugou.com";
const KG_THASH: &str = "5d816a0";
const KG_RF: &str = "B9EDA08A64250DEFFBCADDEE00F8F25F";

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Behavior {
    Play,
    Download,
}
impl Behavior {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Play => "play",
            Self::Download => "download",
        }
    }
}

// Deliberately neither Clone, Serialize nor Debug: consume each grant once.
pub(crate) struct UserAuthorization {
    session: NativeSession,
    auth: String,
}
pub(crate) struct SongAuthorization {
    user: UserAuthorization,
    id: u64,
    hash: String,
    auth: String,
    open_time: String,
}
pub(crate) struct TrackerResponse {
    pub(crate) bytes: Vec<u8>,
    secrets: Vec<String>,
}
impl TrackerResponse {
    pub(crate) fn check_url(&self, url: &str) -> Result<()> {
        let decoded = url::form_urlencoded::parse(url.as_bytes())
            .flat_map(|(k, v)| [k.into_owned(), v.into_owned()])
            .collect::<Vec<_>>();
        if self
            .secrets
            .iter()
            .any(|secret| url.contains(secret) || decoded.iter().any(|part| part.contains(secret)))
        {
            return Err(malformed());
        }
        Ok(())
    }
}

#[derive(Deserialize)]
struct AuthData {
    auth: String,
    userid: Option<Number>,
    module_id: Option<Number>,
    album_audio_id: Option<Number>,
    hash: Option<String>,
    open_time: Option<Value>,
}

#[derive(Deserialize)]
pub(crate) struct Status {
    pub(crate) status: i64,
    pub(crate) error_code: Option<i64>,
    pub(crate) errcode: Option<i64>,
}
impl Status {
    pub(crate) fn parse(bytes: &[u8]) -> Result<Self> {
        let value: Self = serde_json::from_slice(bytes).map_err(|_| malformed())?;
        if matches!((value.error_code, value.errcode), (Some(a), Some(b)) if a != b) {
            return Err(malformed());
        }
        Ok(value)
    }
    pub(crate) fn code(&self) -> i64 {
        self.error_code.or(self.errcode).unwrap_or(0)
    }
    pub(crate) fn rejection(&self) -> TuneWeaveError {
        let code = if self.code() == 20017 {
            ErrorCode::AuthenticationRequired
        } else if matches!(self.status, 2 | 3) || self.code() == 35002 {
            ErrorCode::PermissionDenied
        } else {
            ErrorCode::UpstreamError
        };
        error(code, "KuGou did not authorize this account media request")
            .with_details(json!({"platform_status":self.status,"platform_code":self.code()}))
    }
}

fn auth_data(bytes: &[u8], session: &NativeSession) -> Result<AuthData> {
    let result = auth_data_inner(bytes, session);
    if result.is_err() {
        #[cfg(debug_assertions)]
        eprintln!(
            "DIAGNOSTIC kugou_media_auth_shape={}",
            safe_auth_diagnostic(bytes)
        );
    }
    result
}

fn auth_data_inner(bytes: &[u8], session: &NativeSession) -> Result<AuthData> {
    let status = Status::parse(bytes)?;
    if status.status != 1 || status.code() != 0 {
        return Err(status.rejection());
    }
    #[derive(Deserialize)]
    struct Envelope {
        data: AuthData,
    }
    let wire: Envelope = serde_json::from_slice(bytes).map_err(|_| malformed())?;
    let data = wire.data;
    if !valid_secret(&data.auth) {
        return Err(malformed());
    }
    if data
        .userid
        .is_some_and(|uid| uid.0.to_string() != session.user_id)
        || data.module_id.is_some_and(|module| module.0 != 51)
    {
        return Err(identity_conflict());
    }
    Ok(data)
}

#[cfg(debug_assertions)]
fn safe_auth_shape(value: &Value) -> Value {
    fn fields(value: &Value) -> Value {
        let Some(object) = value.as_object() else {
            return json!({"type":match value { Value::Null=>"null", Value::Bool(_)=>"bool", Value::Number(_)=>"number", Value::String(_)=>"string", Value::Array(_)=>"array", Value::Object(_)=>"object" }});
        };
        let mut result = serde_json::Map::new();
        for (key, value) in object {
            let kind = match value {
                Value::Null => "null",
                Value::Bool(_) => "bool",
                Value::Number(_) => "number",
                Value::String(_) => "string",
                Value::Array(_) => "array",
                Value::Object(_) => "object",
            };
            result.insert(key.clone(), json!(kind));
        }
        Value::Object(result)
    }
    let mut result = serde_json::Map::new();
    result.insert("top_level".into(), fields(value));
    if let Some(data) = value.get("data") {
        result.insert("data_fields".into(), fields(data));
    }
    for name in ["status", "error_code", "errcode"] {
        if let Some(field) = value.get(name)
            && (field.is_number() || field.is_boolean())
        {
            result.insert(name.into(), field.clone());
        }
    }
    Value::Object(result)
}

#[cfg(debug_assertions)]
fn safe_auth_diagnostic(bytes: &[u8]) -> Value {
    serde_json::from_slice::<Value>(bytes)
        .ok()
        .map(|value| safe_auth_shape(&value))
        .unwrap_or_else(|| json!({"kind":"non_json","bytes":bytes.len()}))
}

fn hash(value: &str) -> Result<String> {
    if value.len() != 32 || !value.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(error(
            ErrorCode::InvalidRequest,
            "KuGou media hash is invalid",
        ));
    }
    Ok(value.to_ascii_lowercase())
}

impl KugouClient {
    pub(crate) async fn native_user_authorization(
        &self,
        session: &NativeSession,
    ) -> Result<UserAuthorization> {
        let bytes = self
            .account_media_get(session, "/v1/user_verify", BTreeMap::new())
            .await?;
        let data = auth_data(&bytes, session)?;
        Ok(UserAuthorization {
            session: session.clone(),
            auth: data.auth,
        })
    }

    pub(crate) async fn native_song_authorization(
        &self,
        session: &NativeSession,
        user: UserAuthorization,
        id: u64,
        requested_hash: &str,
    ) -> Result<SongAuthorization> {
        if session != &user.session {
            return Err(identity_conflict());
        }
        if id == 0 {
            return Err(error(
                ErrorCode::InvalidRequest,
                "KuGou media track identity is invalid",
            ));
        }
        let hash = hash(requested_hash)?;
        let extra = BTreeMap::from([
            ("authorization", user.auth.clone()),
            ("album_audio_id", id.to_string()),
            ("hash", hash.clone()),
        ]);
        let bytes = self
            .account_media_get(session, "/v1/authorization", extra)
            .await?;
        let data = auth_data(&bytes, session)?;
        if data.album_audio_id.is_some_and(|value| value.0 != id)
            || data
                .hash
                .as_ref()
                .is_some_and(|value| !value.eq_ignore_ascii_case(&hash))
        {
            return Err(identity_conflict());
        }
        let open_time = match data.open_time {
            Some(Value::String(value)) if valid_secret(&value) && value != "0" => value,
            Some(Value::Number(value)) => value
                .as_u64()
                .filter(|n| *n != 0)
                .ok_or_else(malformed)?
                .to_string(),
            _ => return Err(malformed()),
        };
        Ok(SongAuthorization {
            user,
            id,
            hash,
            auth: data.auth,
            open_time,
        })
    }

    pub(crate) async fn native_audio_tracker(
        &self,
        session: &NativeSession,
        grant: SongAuthorization,
        album_id: u64,
        quality: &str,
        behavior: Behavior,
    ) -> Result<TrackerResponse> {
        if session != &grant.user.session {
            return Err(identity_conflict());
        }
        if !matches!(quality, "128" | "320" | "flac" | "high" | "super") {
            return Err(error(
                ErrorCode::InvalidRequest,
                "KuGou media quality is invalid",
            ));
        }
        let concept = session.client == KugouLoginClient::Concept;
        let salt = if concept {
            "185672dd44712f60bb1736df5a377e82"
        } else {
            "57ae12eb6890223e355ccfcb74edf70d"
        };
        let key = format!(
            "{:x}",
            Md5::digest(format!(
                "{}{salt}{}{}{}",
                grant.hash,
                session.client.appid(),
                session.device.mid,
                session.user_id
            ))
        );
        let extra = BTreeMap::from([
            ("album_id", album_id.to_string()),
            ("area_code", "1".into()),
            ("module", String::new()),
            ("hash", grant.hash),
            ("need_m", "0".into()),
            ("ssa_flag", "is_fromtrack".into()),
            ("version", "11430".into()),
            ("open_time", grant.open_time.clone()),
            ("ptype", "0".into()),
            ("need_ogg", "1".into()),
            (
                "page_id",
                if concept { "967177915" } else { "151369488" }.into(),
            ),
            ("auth", grant.auth.clone()),
            ("mtype", "0".into()),
            ("quality", quality.into()),
            ("album_audio_id", grant.id.to_string()),
            ("behavior", behavior.name().into()),
            ("pid", if concept { "411" } else { "2" }.into()),
            ("cmd", "26".into()),
            (
                "ppage_id",
                if concept {
                    "356753938,823673182,967485191"
                } else {
                    "463467626,350369493,788954147"
                }
                .into(),
            ),
            ("pidversion", "3001".into()),
            ("cdnBackup", "1".into()),
            ("key", key),
        ]);
        let bytes = self
            .account_media_get(session, "/tracker/v5/url", extra)
            .await?;
        let mut secrets = vec![
            session.token.clone(),
            grant.user.auth,
            grant.auth,
            grant.open_time,
        ];
        secrets.extend(session.vip_token.iter().cloned());
        secrets.extend(session.t1.iter().cloned());
        Ok(TrackerResponse { bytes, secrets })
    }

    async fn account_media_get(
        &self,
        session: &NativeSession,
        path: &'static str,
        extra: BTreeMap<&str, String>,
    ) -> Result<Vec<u8>> {
        validate_session(session)?;
        let (host, operation) = match path {
            "/v1/user_verify" => (AUTH_HOST, "native_media_user_authorization"),
            "/v1/authorization" => (AUTH_HOST, "native_media_song_authorization"),
            "/tracker/v5/url" => (HOST, "native_media_tracker"),
            _ => return Err(malformed()),
        };
        let seconds = now_ms()? / 1000;
        let mut query = BTreeMap::from([
            ("appid", session.client.appid().to_string()),
            (
                "clientver",
                if path == "/v1/user_verify" {
                    session.client.clientver().to_string()
                } else {
                    "11561".into()
                },
            ),
            ("clienttime", seconds.to_string()),
            ("dfid", session.device.dfid().into()),
            ("mid", session.device.mid.clone()),
            ("uuid", "-".into()),
            ("userid", session.user_id.clone()),
            ("token", session.token.clone()),
            ("module_id", "51".into()),
        ]);
        for (key, value) in extra {
            if query.insert(key, value).is_some() {
                return Err(malformed());
            }
        }
        let signature = match session.client {
            KugouLoginClient::Standard => android_signature(&query, &[]),
            KugouLoginClient::Concept => concept_signature(&query, &[]),
            KugouLoginClient::Web => return Err(malformed()),
        };
        query.insert("signature", signature);
        let target = format!("https://{host}{path}");
        #[cfg(test)]
        let target = self
            .login_test_origin
            .as_ref()
            .map(|origin| origin.join(path).unwrap().to_string())
            .unwrap_or(target);
        let started = Instant::now();
        let mut status = None;
        let result = async {
            let response = self
                .http
                .get(target)
                .header("accept", "application/json")
                .header(
                    "user-agent",
                    "Android15-1070-11083-46-0-DiscoveryDRADProtocol-wifi",
                )
                .header("dfid", session.device.dfid())
                .header("mid", &session.device.mid)
                .header("clienttime", seconds)
                .header("kg-rc", "1")
                .header("kg-thash", KG_THASH)
                .header("kg-rec", "1")
                .header("kg-rf", KG_RF)
                .query(&query)
                .send()
                .await
                .map_err(network_error)?;
            status = Some(response.status());
            let content_type = response
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.split(';').next())
                .map(str::trim)
                .filter(|value| {
                    !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_graphic())
                })
                .map(str::to_owned)
                .unwrap_or_else(|| "unknown".to_owned());
            let _http_status = status.map_or(0, |value| value.as_u16());
            let _json_content_type = content_type.eq_ignore_ascii_case("application/json");
            let _oversized_declared_body = response
                .headers()
                .get(reqwest::header::CONTENT_LENGTH)
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.parse::<u64>().ok())
                .is_some_and(|length| length > RESPONSE_LIMIT as u64);
            let _additional_verification_header = response
                .headers()
                .get("ssa-code")
                .is_some_and(|value| value.as_bytes() != b"0" && !value.is_empty());
            // The live auth endpoint returned HTTP 200 with a non-JSON MIME type.
            // Allow text/html at the transport boundary, but continue to bound
            // the body and require the strict JSON auth schema below.
            let bytes = match read_response_with_types(
                response,
                RESPONSE_LIMIT,
                &["application/json", "text/html"],
            )
            .await
            {
                Ok(bytes) => bytes,
                Err(error) => {
                    #[cfg(debug_assertions)]
                    eprintln!(
                        "DIAGNOSTIC kugou_media_body_intake_error stage={path} http_status={_http_status} json_content_type={_json_content_type} oversized_declared_body={_oversized_declared_body} additional_verification_header={_additional_verification_header}"
                    );
                    return Err(error);
                }
            };
            // Log business rejection as a rejection, even when its HTTP status is 200.
            let status = match Status::parse(&bytes) {
                Ok(status) => status,
                Err(error) => {
                    #[cfg(debug_assertions)]
                    eprintln!(
                        "DIAGNOSTIC kugou_media_status_shape stage={path} http_status={} content_type={content_type} bytes={} shape={}",
                        _http_status,
                        bytes.len(),
                        safe_auth_diagnostic(&bytes)
                    );
                    return Err(error);
                }
            };
            if status.status != 1 || status.code() != 0 {
                #[cfg(debug_assertions)]
                eprintln!(
                    "DIAGNOSTIC kugou_media_status_rejection stage={path} http_status={_http_status} platform_status={} platform_code={}",
                    status.status,
                    status.code()
                );
                return Err(status.rejection());
            }
            Ok(bytes)
        }
        .await;
        self.log_upstream_request(operation, host, path, status, started, 0, false, &result);
        result
    }
}

#[cfg(test)]
pub(crate) mod tests;
