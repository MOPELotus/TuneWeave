use super::*;

const HOST: &str = "wapi.kuwo.cn";
pub(super) const PATH: &str = "/openapi/v1/app/userInitData/selectFavour";
const XOR_KEY: &[u8; 8] = b"yeelion ";

impl KuwoClient {
    pub(in crate::client::native) async fn register_native_device(
        &self,
        context: &NativeDeviceContext,
        old_app_uid: Option<&str>,
    ) -> Result<String> {
        if !valid_id(&context.device_user)
            || !valid_id(&context.android_id)
            || context.device_user == context.android_id
            || old_app_uid.is_some_and(|id| !valid_app_uid(id))
        {
            return Err(state_error());
        }
        let plain = registration_plain(context, old_app_uid);
        let encoded: Vec<u8> = plain
            .bytes()
            .enumerate()
            .map(|(i, byte)| byte ^ XOR_KEY[i % 8])
            .collect();
        let query = {
            let mut query = url::form_urlencoded::Serializer::new(String::new());
            query.extend_pairs([
                ("len", plain.len().to_string()),
                ("token", BASE64_STANDARD.encode(encoded)),
                ("appuid", old_app_uid.unwrap_or("0").to_owned()),
            ]);
            query.finish()
        };
        let target = format!("{}?{query}", self.native_target(HOST, PATH));
        self.native_get(
            HOST,
            PATH,
            "native_device_register",
            target,
            parse_registration,
        )
        .await
    }
}

fn registration_plain(context: &NativeDeviceContext, old_app_uid: Option<&str>) -> String {
    let prefix = old_app_uid
        .map(|id| format!("&uid={id}"))
        .unwrap_or_default();
    format!(
        "{prefix}&new_user=1&mac={device}&hd={device}&android_id={android}&oaid=&q36={FALLBACK_Q36}&vmac=&ver=kwplayer_ar_{CLIENT_VERSION}&src={CLIENT_SOURCE}&process=false&dev=TuneWeave SDK client",
        device = context.device_user,
        android = context.android_id,
    )
}

#[derive(Deserialize)]
struct RegistrationEnvelope {
    code: i64,
    success: bool,
    data: Option<RegistrationData>,
}
#[derive(Deserialize)]
struct RegistrationData {
    #[serde(deserialize_with = "app_uid")]
    appuid: String,
}
fn app_uid<'de, D: serde::Deserializer<'de>>(d: D) -> std::result::Result<String, D::Error> {
    let id =
        deserialize_code(d)?.ok_or_else(|| serde::de::Error::custom("missing app device ID"))?;
    if valid_app_uid(&id) {
        Ok(id)
    } else {
        Err(serde::de::Error::custom("invalid app device ID"))
    }
}
fn parse_registration(bytes: &[u8]) -> Result<String> {
    let body: RegistrationEnvelope = serde_json::from_slice(bytes).map_err(|_| invalid())?;
    if body.code != 200 || !body.success {
        return Err(invalid());
    }
    Ok(body.data.ok_or_else(invalid)?.appuid)
}

#[cfg(test)]
pub(super) fn parse_fixture(bytes: &[u8]) -> Result<String> {
    parse_registration(bytes)
}
