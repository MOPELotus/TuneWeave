//! Explicit native SID revocation; separate from local credential removal.
use super::*;

pub(super) const PATH: &str = "/US_NEW/kuwo/login/logout";

impl KuwoClient {
    pub(crate) async fn send_native_session_revocation(
        &self,
        input: &KuwoNativeSessionInput,
        before_send: impl FnOnce() -> Result<()>,
    ) -> Result<()> {
        validate_session_metadata(input)?;
        let key = self.native_response_key()?;
        let target = format!(
            "{}?f=ar&q={}",
            self.native_target(EXCHANGE_HOST, PATH),
            codec::seal_query(query(input, &key).as_bytes())?
        );
        self.native_get_with_metadata_hooks(
            EXCHANGE_HOST,
            PATH,
            "native_session_revoke",
            target,
            Some(session_metadata(input)?),
            (parse, before_send),
        )
        .await
    }
}

fn query(input: &KuwoNativeSessionInput, key: &[u8; 8]) -> String {
    // qs.h.c -> qs.h.h, with explicit SDK device metadata like native login.
    let mut query = url::form_urlencoded::Serializer::new(String::new());
    query.extend_pairs([
        ("uid", input.user_id()),
        ("sid", input.session_id()),
        ("src", CLIENT_SOURCE),
        ("version", CLIENT_VERSION),
        ("dev_id", input.device_id()),
        ("user", input.device_user()),
        ("dev_name", "TuneWeave SDK client"),
        ("devType", "SDK"),
        (
            "sx",
            std::str::from_utf8(key).expect("numeric protocol key"),
        ),
        ("from", "android"),
        ("devResolution", "0*0"),
    ]);
    query.finish()
}

fn parse(bytes: &[u8]) -> Result<()> {
    // Official UserInfoManager.d expects a plain result=ok receipt, not the
    // encrypted account-login envelope. Require the exact field, not a substring.
    let text = std::str::from_utf8(bytes).map_err(|_| invalid())?;
    let mut result = None;
    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        let (name, value) = line.split_once('=').ok_or_else(invalid)?;
        if name == "result" && result.replace(value).is_some() {
            return Err(invalid());
        }
    }
    if result == Some("ok") {
        Ok(())
    } else {
        Err(invalid())
    }
}

#[cfg(test)]
mod tests;
