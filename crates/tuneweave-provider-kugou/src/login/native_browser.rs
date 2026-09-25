//! Targets consumed by the official h5Verify page, not user-supplied URLs.
use super::*;

pub(crate) fn validate_target(value: &str) -> Result<()> {
    let valid = value.len() <= 4096
        && !value.chars().any(char::is_control)
        && if let Some(appid) = value.strip_prefix("KGCodeTX|") {
            !appid.is_empty() && appid.len() <= 32 && appid.bytes().all(|b| b.is_ascii_digit())
        } else if let Some(parameters) = value.strip_prefix("KGCodeGT|") {
            // The official page splits on '|', then passes these fields to initGeetest.
            !parameters.contains('|')
                && serde_json::from_str::<Value>(parameters).is_ok_and(|data| {
                    let text = |name: &str| {
                        data.get(name).and_then(Value::as_str).is_some_and(|s| {
                            !s.is_empty()
                                && s.len() <= 1024
                                && s.bytes().all(|b| b.is_ascii_graphic())
                        })
                    };
                    text("gt")
                        && text("challenge")
                        && data
                            .get("success")
                            .is_some_and(|v| v.is_boolean() || matches!(v.as_u64(), Some(0 | 1)))
                })
        } else {
            // These strings select a different branch in the official wrapper.
            !value.contains("KGCodeTX")
                && !value.contains("KGCodeGT")
                && value.bytes().all(|b| b.is_ascii_graphic())
                && Url::parse(value).is_ok_and(|url| {
                    url.scheme() == "https"
                        && url.host_str().is_some()
                        && url.username().is_empty()
                        && url.password().is_none()
                        && url.fragment().is_none()
                })
        };
    if valid {
        Ok(())
    } else {
        Err(malformed(
            "KuGou native password challenge returned an unsupported browser target",
        ))
    }
}
