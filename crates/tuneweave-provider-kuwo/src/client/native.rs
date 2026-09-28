//! Native session exchange and independent validation, without account storage.
use super::*;
use reqwest::header::{CONTENT_TYPE, RETRY_AFTER};
use serde::de::{MapAccess, Visitor, value::MapAccessDeserializer};
use std::time::{SystemTime, UNIX_EPOCH};

mod codec;
mod device;
pub use device::{KuwoNativeDevice, KuwoNativeDeviceStore};
pub(crate) mod credential;
pub(crate) mod following_artists;
pub(crate) mod library;
pub(crate) mod management;
pub(crate) mod media;
pub(crate) mod membership;
pub(crate) mod password;
pub(crate) mod playlist;
pub(crate) mod profile;
pub(crate) mod revocation;
pub(crate) mod sms;
pub(crate) mod submissions;
pub use sms::{KuwoNativeSmsChallenge, KuwoNativeSmsRequest};
mod tables;
#[cfg(test)]
pub(crate) mod tests;

const EXCHANGE_HOST: &str = "i.kuwo.cn";
const EXCHANGE_PATH: &str = "/US_NEW/kuwo/login/auto_login";
const VALIDATE_HOST: &str = "loginserver.kuwo.cn";
const VALIDATE_PATH: &str = "/u.s";
const CLIENT_VERSION: &str = "12.2.2.0";
const CLIENT_SOURCE: &str = "kwplayer_ar_12.2.2.0_newpcguanwangmobile.apk";

#[cfg(debug_assertions)]
pub(crate) fn diagnostic_response_shape(bytes: &[u8]) -> serde_json::Value {
    use serde_json::{Map, Value, json};
    let Ok(value) = serde_json::from_slice::<Value>(bytes) else {
        return json!({"kind":"non_json","bytes":bytes.len()});
    };
    fn kind(value: &Value) -> &'static str {
        match value {
            Value::Null => "null",
            Value::Bool(_) => "bool",
            Value::Number(_) => "number",
            Value::String(_) => "string",
            Value::Array(_) => "array",
            Value::Object(_) => "object",
        }
    }
    fn fields(value: &Value) -> Map<String, Value> {
        value.as_object().map_or_else(Map::new, |object| {
            object
                .iter()
                .map(|(key, value)| (key.clone(), json!(kind(value))))
                .collect()
        })
    }
    let mut result = Map::new();
    result.insert("top_level".into(), Value::Object(fields(&value)));
    if let Some(data) = value.get("data") {
        result.insert("data_fields".into(), Value::Object(fields(data)));
        if let Some(items) = data.as_array() {
            result.insert("data_count".into(), json!(items.len()));
            if let Some(first) = items.first() {
                result.insert("data_item_fields".into(), Value::Object(fields(first)));
            }
        }
    }
    if let Some(info) = value.get("info") {
        result.insert("info_count".into(), json!(info.as_array().map(Vec::len)));
        if let Some(first) = info.as_array().and_then(|items| items.first()) {
            result.insert("info_item_fields".into(), Value::Object(fields(first)));
        }
    }
    if let Some(items) = value.get("plist").and_then(Value::as_array) {
        result.insert("plist_count".into(), json!(items.len()));
        let mut types = std::collections::BTreeMap::<String, usize>::new();
        for item in items {
            let kind = item
                .get("type")
                .and_then(Value::as_str)
                .filter(|kind| {
                    !kind.is_empty()
                        && kind.len() <= 32
                        && kind.bytes().all(|byte| {
                            byte.is_ascii_uppercase()
                                || byte.is_ascii_digit()
                                || matches!(byte, b'_' | b'-')
                        })
                })
                .unwrap_or("other");
            *types.entry(kind.to_owned()).or_default() += 1;
        }
        result.insert("plist_types".into(), json!(types));
        result.insert(
            "plist_item_shapes".into(),
            Value::Array(
                items
                    .iter()
                    .take(8)
                    .map(|item| Value::Object(fields(item)))
                    .collect(),
            ),
        );
    }
    if let Some(meta) = value.get("meta") {
        result.insert("meta_fields".into(), Value::Object(fields(meta)));
    }
    Value::Object(result)
}

pub(super) fn seal_catalog_query(plain: &[u8]) -> Result<String> {
    codec::seal_catalog_query(plain)
}

/// Unverified native UID, SID, app device ID and separate native `user` identifier.
///
/// These are not Web cookies or a TuneWeave account credential. Creating this
/// value does not authenticate it. The SDK never persists or logs these fields.
#[derive(Clone, Eq, PartialEq)]
pub struct KuwoNativeSessionInput {
    user_id: String,
    session_id: String,
    device_id: String,
    device_user: String,
    context: Option<device::NativeDeviceContext>,
}
impl fmt::Debug for KuwoNativeSessionInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("KuwoNativeSessionInput { fields: [redacted] }")
    }
}
impl KuwoNativeSessionInput {
    pub fn new(
        user_id: &str,
        session_id: &str,
        device_id: &str,
        device_user: &str,
    ) -> Result<Self> {
        if !valid_uid(user_id)
            || !valid_secret(session_id)
            || !printable(device_id, 128)
            || !printable(device_user, 128)
        {
            return Err(kuwo_invalid_request("Kuwo native session input is invalid"));
        }
        Ok(Self {
            user_id: user_id.into(),
            session_id: session_id.into(),
            device_id: device_id.into(),
            device_user: device_user.into(),
            context: None,
        })
    }
    #[must_use]
    pub fn user_id(&self) -> &str {
        &self.user_id
    }
    #[must_use]
    pub fn session_id(&self) -> &str {
        &self.session_id
    }
    #[must_use]
    pub fn device_id(&self) -> &str {
        &self.device_id
    }
    #[must_use]
    pub fn device_user(&self) -> &str {
        &self.device_user
    }
}

/// A platform exchange result awaiting independent session validation.
///
/// This is deliberately not an authenticated AccountProfile or ProviderCredential.
/// Validate `session()` before accepting it. An account manager must additionally
/// guard its own source generation at every network boundary.
pub struct KuwoNativeSessionExchange {
    session: KuwoNativeSessionInput,
    nickname: Option<String>,
}
impl fmt::Debug for KuwoNativeSessionExchange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(
            "KuwoNativeSessionExchange { fields: [redacted], independently_validated: false }",
        )
    }
}
impl KuwoNativeSessionExchange {
    #[must_use]
    pub const fn session(&self) -> &KuwoNativeSessionInput {
        &self.session
    }
    #[must_use]
    pub fn nickname(&self) -> Option<&str> {
        self.nickname.as_deref()
    }
    #[must_use]
    pub fn into_session(self) -> KuwoNativeSessionInput {
        self.session
    }
}

impl KuwoClient {
    /// Exchanges an existing native session and checks its returned UID.
    ///
    /// No password, SMS, Web cookie, anonymous music session or stored account is
    /// used. This may return a replacement SID; independently validate it before
    /// accepting login. No automatic retry or HTTP downgrade is performed.
    pub async fn exchange_native_session(
        &self,
        input: &KuwoNativeSessionInput,
    ) -> Result<KuwoNativeSessionExchange> {
        let key = self.native_response_key()?;
        let plain = exchange_query(input, &key);
        self.native_exchange_request(&plain, &key, &input.user_id, input)
            .await
    }

    /// Checks the exact native UID/SID pair independently at the official validator.
    ///
    /// Only the explicit business result `ok` succeeds. No session or account is
    /// stored; a caller managing account aliases must enforce its own generation.
    pub async fn validate_native_session(&self, input: &KuwoNativeSessionInput) -> Result<()> {
        if self.native_session_authenticated(input).await? {
            Ok(())
        } else {
            Err(authentication_required())
        }
    }

    pub(crate) async fn native_session_authenticated(
        &self,
        input: &KuwoNativeSessionInput,
    ) -> Result<bool> {
        let query = {
            let mut query = url::form_urlencoded::Serializer::new(String::new());
            query.extend_pairs([
                ("type", "new_validate_ext"),
                ("uid", input.user_id()),
                ("sid", input.session_id()),
                ("dev_id", input.device_id()),
                ("dev_key", input.device_id()),
                ("req_enc", "utf8"),
                ("res_enc", "utf8"),
                ("user", input.device_user()),
                ("prod", "kwplayer_ar_12.2.2.0"),
                ("corp", "kuwo"),
                ("newver", "3"),
                ("vipver", CLIENT_VERSION),
                ("source", CLIENT_SOURCE),
                ("p2p", "1"),
                ("loginUid", input.user_id()),
                ("loginSid", input.session_id()),
                ("appuid", input.device_id()),
            ]);
            if let Some(context) = &input.context {
                query.extend_pairs([
                    ("android_id", context.android_id.as_str()),
                    ("q36", device::FALLBACK_Q36),
                    ("approval", "false"),
                ]);
            }
            query.finish()
        };
        let target = format!(
            "{}?{query}",
            self.native_target(VALIDATE_HOST, VALIDATE_PATH),
        );
        self.native_get(
            VALIDATE_HOST,
            VALIDATE_PATH,
            "native_session_validate",
            target,
            parse_validation_state,
        )
        .await
    }

    fn native_response_key(&self) -> Result<[u8; 8]> {
        #[cfg(test)]
        if let Some(key) = self.native_test_response_key {
            return Ok(key);
        }
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| invalid())?
            .as_millis()
            .to_string();
        now.as_bytes()
            .get(..8)
            .and_then(|bytes| bytes.try_into().ok())
            .ok_or_else(invalid)
    }

    async fn native_exchange_request(
        &self,
        plain: &str,
        key: &[u8; 8],
        expected_uid: &str,
        input: &KuwoNativeSessionInput,
    ) -> Result<KuwoNativeSessionExchange> {
        let encoded = codec::seal_query(plain.as_bytes())?;
        // The official client appends raw Base64. Form-encoding the outer q again
        // changes its wire representation; inner values have already been encoded.
        let target = format!(
            "{}?f=ar&q={encoded}",
            self.native_target(EXCHANGE_HOST, EXCHANGE_PATH)
        );
        self.native_get(
            EXCHANGE_HOST,
            EXCHANGE_PATH,
            "native_session_exchange",
            target,
            |body| {
                let decoded = codec::open_response(body, key)?;
                parse_exchange(&decoded, expected_uid, input)
            },
        )
        .await
    }

    fn native_target(&self, host: &str, path: &str) -> String {
        #[cfg(test)]
        if let Some(origin) = &self.web_test_origin {
            return origin.join(path).unwrap().to_string();
        }
        format!("https://{host}{path}")
    }

    async fn native_get<T>(
        &self,
        host: &'static str,
        path: &'static str,
        operation: &'static str,
        target: String,
        parse: impl FnOnce(&[u8]) -> Result<T>,
    ) -> Result<T> {
        self.native_get_with_metadata(host, path, operation, target, None, parse)
            .await
    }

    async fn native_get_with_metadata<T>(
        &self,
        host: &'static str,
        path: &'static str,
        operation: &'static str,
        target: String,
        metadata: Option<String>,
        parse: impl FnOnce(&[u8]) -> Result<T>,
    ) -> Result<T> {
        self.native_get_with_metadata_hooks(
            host,
            path,
            operation,
            target,
            metadata,
            (parse, || Ok(())),
        )
        .await
    }

    async fn native_get_with_metadata_hooks<T>(
        &self,
        host: &'static str,
        path: &'static str,
        operation: &'static str,
        target: String,
        metadata: Option<String>,
        hooks: (impl FnOnce(&[u8]) -> Result<T>, impl FnOnce() -> Result<()>),
    ) -> Result<T> {
        let (parse, before_send) = hooks;
        let started = Instant::now();
        let mut status = None;
        let result = async {
            let mut request = self.http.get(target).header(ACCEPT, "application/json");
            if let Some(metadata) = metadata {
                request = request.header("Cookies", metadata);
            }
            before_send()?;
            let response = request
                .send()
                .await
                .map_err(|error| kuwo_network_error(error).retryable(false))?;
            status = Some(response.status());
            let max_bytes = if path == library::OWNED_PATH
                || path == library::SAVED_PATH
                || path == playlist::COLLECTED_PATH
                || path == playlist::metadata::METADATA_PATH
            {
                library::MAX_RESPONSE
            } else if path == media::RIGHTS_PATH || path == media::MEDIA_PATH {
                media::MAX_RESPONSE
            } else {
                codec::MAX_RESPONSE
            };
            let _response_mime = response
                .headers()
                .get(CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.split(';').next())
                .map(str::trim);
            let _response_mime_class = match _response_mime {
                Some(value) if value.eq_ignore_ascii_case("application/json") => "json",
                Some(value) if value.eq_ignore_ascii_case("text/plain") => "text_plain",
                Some(value) if value.eq_ignore_ascii_case("text/html") => "text_html",
                Some(_) => "other",
                None => "missing",
            };
            let _response_size_class = match response.content_length() {
                None => "unknown",
                Some(size) if size <= max_bytes as u64 => "within_limit",
                Some(_) => "over_limit",
            };
            let bytes = match read_response(
                response,
                path == VALIDATE_PATH
                    || path == profile::PATH
                    || path == membership::PATH
                    || path == revocation::PATH,
                path == playlist::COLLECTED_PATH
                    || path == revocation::PATH
                    || path == media::RIGHTS_PATH
                    || path == media::trial::PATH
                    || operation == "native_cloud_playlist",
                max_bytes,
            )
            .await
            {
                Ok(bytes) => bytes,
                Err(error) => {
                    #[cfg(debug_assertions)]
                    if operation == "native_cloud_playlist" {
                        eprintln!(
                            "DIAGNOSTIC kuwo_native_response_read status={} mime_class={_response_mime_class} content_length={_response_size_class}",
                            status.map_or(0, |value| value.as_u16())
                        );
                    }
                    return Err(error);
                }
            };
            parse(&bytes)
        }
        .await;
        self.log_upstream_request(operation, host, path, status, started, 0, false, &result);
        result
    }
}

fn exchange_query(input: &KuwoNativeSessionInput, key: &[u8; 8]) -> String {
    let mut query = url::form_urlencoded::Serializer::new(String::new());
    query.extend_pairs([
        ("uid", input.user_id()),
        ("sid", input.session_id()),
        ("username", ""),
        ("password", ""),
        ("dev_id", input.device_id()),
        ("dev_name", "TuneWeave"),
        ("user", input.device_user()),
        ("src", CLIENT_SOURCE),
        ("urlencode", "1"),
        ("devResolution", "0*0"),
        (
            "devType",
            if input.context.is_some() {
                "SDK"
            } else {
                "arr"
            },
        ),
        (
            "sx",
            std::str::from_utf8(key).expect("numeric protocol key"),
        ),
        ("from", "android"),
        ("version", CLIENT_VERSION),
    ]);
    query.finish()
}

pub(crate) fn validate_session_metadata(input: &KuwoNativeSessionInput) -> Result<()> {
    // Commas delimit the official plural Cookies header. No embedded UID/SID
    // assignments from a caller's otherwise printable opaque session are allowed.
    if [input.session_id(), input.device_user(), input.device_id()]
        .iter()
        .any(|value| value.contains(','))
    {
        return Err(kuwo_invalid_request(
            "Kuwo native account metadata is invalid",
        ));
    }
    Ok(())
}
fn session_metadata(input: &KuwoNativeSessionInput) -> Result<String> {
    validate_session_metadata(input)?;
    Ok(format!(
        "user={},ct=11,cv=12220,chid=newpcguanwangmobile,QIMEI36={},tmeAppID=kwplayer,loginUid={},loginSid={},appUid={},rom=TuneWeave/SDK/SDK,",
        input.device_user(),
        device::FALLBACK_Q36,
        input.user_id(),
        input.session_id(),
        input.device_id()
    ))
}

async fn read_response(
    mut response: reqwest::Response,
    validation: bool,
    plain_json: bool,
    max_bytes: usize,
) -> Result<Vec<u8>> {
    if response.status() != StatusCode::OK {
        let code = match response.status() {
            StatusCode::UNAUTHORIZED => ErrorCode::AuthenticationRequired,
            StatusCode::FORBIDDEN => ErrorCode::PermissionDenied,
            StatusCode::TOO_MANY_REQUESTS => ErrorCode::RateLimited,
            _ => ErrorCode::UpstreamError,
        };
        let mut error = TuneWeaveError::new(code, "Kuwo native session HTTP request failed")
            .with_platform(Platform::Kuwo)
            .with_details(json!({"http_status":response.status().as_u16()}));
        if code == ErrorCode::RateLimited {
            error.details["retry_after_secs"] = json!(
                response
                    .headers()
                    .get(RETRY_AFTER)
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.parse::<u64>().ok())
                    .unwrap_or(2)
                    .clamp(2, 300)
            );
        }
        return Err(error);
    }
    let mime = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(';').next())
        .map(str::trim);
    if !mime.is_some_and(|v| {
        v.eq_ignore_ascii_case("application/json")
            || (validation && v.eq_ignore_ascii_case("text/html"))
            || (plain_json && v.eq_ignore_ascii_case("text/plain"))
    }) || response
        .content_length()
        .is_some_and(|size| size > max_bytes as u64)
    {
        return Err(invalid());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| kuwo_network_error(error).retryable(false))?
    {
        if bytes.len().saturating_add(chunk.len()) > max_bytes {
            return Err(invalid());
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

#[derive(Deserialize)]
struct ExchangeBody {
    ret: Option<String>,
    result: Option<String>,
    sid: Option<String>,
    #[serde(rename = "userInfo")]
    user_info: Option<NestedUser>,
    #[serde(default, deserialize_with = "deserialize_code")]
    status: Option<String>,
    #[serde(default, rename = "enum", deserialize_with = "deserialize_code")]
    error_enum: Option<String>,
}
fn deserialize_code<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> std::result::Result<Option<String>, D::Error> {
    let value = Option::<serde_json::Value>::deserialize(d)?;
    let code = match value {
        None => return Ok(None),
        Some(serde_json::Value::String(value))
            if value.len() <= 32
                && value.is_ascii()
                && !value.bytes().any(|b| b.is_ascii_control()) =>
        {
            value
        }
        Some(serde_json::Value::Number(value)) if value.is_i64() || value.is_u64() => {
            value.to_string()
        }
        _ => return Err(serde::de::Error::custom("invalid native code")),
    };
    Ok(Some(code))
}
#[derive(Deserialize)]
struct NativeUser {
    #[serde(deserialize_with = "deserialize_uid")]
    uid: String,
    #[serde(rename = "nickName")]
    nickname: Option<String>,
}
struct NestedUser(NativeUser);
impl<'de> Deserialize<'de> for NestedUser {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        struct UserVisitor;
        impl<'de> Visitor<'de> for UserVisitor {
            type Value = NestedUser;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a native user object or JSON string")
            }
            fn visit_str<E: serde::de::Error>(
                self,
                value: &str,
            ) -> std::result::Result<Self::Value, E> {
                serde_json::from_str(value)
                    .map(NestedUser)
                    .map_err(|_| E::custom("invalid native user"))
            }
            fn visit_map<M: MapAccess<'de>>(
                self,
                map: M,
            ) -> std::result::Result<Self::Value, M::Error> {
                NativeUser::deserialize(MapAccessDeserializer::new(map)).map(NestedUser)
            }
        }
        d.deserialize_any(UserVisitor)
    }
}
fn deserialize_uid<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> std::result::Result<String, D::Error> {
    struct UidVisitor;
    impl Visitor<'_> for UidVisitor {
        type Value = String;
        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("a positive native user ID")
        }
        fn visit_u64<E: serde::de::Error>(self, value: u64) -> std::result::Result<String, E> {
            self.visit_str(&value.to_string())
        }
        fn visit_str<E: serde::de::Error>(self, value: &str) -> std::result::Result<String, E> {
            if valid_uid(value) {
                Ok(value.into())
            } else {
                Err(E::custom("invalid native UID"))
            }
        }
    }
    d.deserialize_any(UidVisitor)
}

fn parse_exchange(
    bytes: &[u8],
    expected_uid: &str,
    input: &KuwoNativeSessionInput,
) -> Result<KuwoNativeSessionExchange> {
    let body: ExchangeBody = serde_json::from_slice(bytes).map_err(|_| invalid())?;
    let (user, sid) = parse_session_body(body)?;
    if user.uid != expected_uid {
        return Err(invalid());
    }
    let mut session =
        KuwoNativeSessionInput::new(&user.uid, &sid, input.device_id(), input.device_user())
            .map_err(|_| invalid())?;
    session.context = input.context.clone();
    if user.nickname.as_ref().is_some_and(|name| {
        !valid_nickname(name)
            || echoes_secret(name, input.session_id())
            || echoes_secret(name, &sid)
    }) {
        return Err(invalid());
    }
    Ok(KuwoNativeSessionExchange {
        session,
        nickname: user.nickname.filter(|name| !name.is_empty()),
    })
}

fn parse_session_body(body: ExchangeBody) -> Result<(NativeUser, String)> {
    if body
        .ret
        .as_ref()
        .zip(body.result.as_ref())
        .is_some_and(|(a, b)| a != b)
    {
        return Err(invalid());
    }
    let outcome = body
        .ret
        .as_deref()
        .or(body.result.as_deref())
        .ok_or_else(invalid)?;
    if outcome != "succ" {
        if outcome == "fail"
            && body.status.as_deref() == Some("1136")
            && body.error_enum.as_deref() == Some("3")
            && body.sid.is_none()
            && body.user_info.is_none()
        {
            return Err(authentication_required());
        }
        return Err(invalid());
    }
    let user = body.user_info.ok_or_else(invalid)?.0;
    let sid = body.sid.ok_or_else(invalid)?;
    if matches!(body.status.as_deref(), Some("1136" | "1157" | "1159")) || !valid_secret(&sid) {
        return Err(invalid());
    }
    Ok((user, sid))
}
fn valid_nickname(value: &str) -> bool {
    value.len() <= 1024 && !value.chars().any(char::is_control)
}

#[derive(Deserialize)]
struct ValidationBody {
    result: String,
    reason: Option<String>,
}
#[cfg(test)]
fn parse_validation(bytes: &[u8]) -> Result<()> {
    if parse_validation_state(bytes)? {
        Ok(())
    } else {
        Err(authentication_required())
    }
}
fn parse_validation_state(bytes: &[u8]) -> Result<bool> {
    let body: ValidationBody = serde_json::from_slice(bytes).map_err(|_| invalid())?;
    if body.result == "ok" && body.reason.as_deref().is_none_or(str::is_empty) {
        return Ok(true);
    }
    if body.result == "fail"
        && matches!(
            body.reason.as_deref(),
            Some("error_user_not_exist" | "error_user_invalid")
        )
    {
        return Ok(false);
    }
    Err(invalid())
}

fn valid_uid(value: &str) -> bool {
    !value.starts_with('0')
        && value.bytes().all(|b| b.is_ascii_digit())
        && value.parse::<i32>().is_ok_and(|v| v > 0)
}
fn echoes_secret(value: &str, secret: &str) -> bool {
    value.contains(secret)
        || url::form_urlencoded::parse(value.as_bytes())
            .any(|(key, value)| key.contains(secret) || value.contains(secret))
}
fn printable(value: &str, max: usize) -> bool {
    !value.is_empty() && value.len() <= max && value.bytes().all(|b| (0x21..=0x7e).contains(&b))
}
fn valid_secret(value: &str) -> bool {
    printable(value, 4096) && !matches!(value, "0" | "null" | "undefined")
}
fn invalid() -> TuneWeaveError {
    kuwo_upstream_error("Kuwo native session response is invalid")
}
fn authentication_required() -> TuneWeaveError {
    TuneWeaveError::new(
        ErrorCode::AuthenticationRequired,
        "Kuwo native session is not valid",
    )
    .with_platform(Platform::Kuwo)
}
