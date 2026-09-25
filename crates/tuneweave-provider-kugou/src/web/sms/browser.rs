//! Official H5 callback transport for SMS error 20028. No challenge is solved here.
use super::*;
use rand::{TryRng, rngs::SysRng};
use reqwest::header::HeaderValue;
use tuneweave_core::AuthBrowserChallenge;

pub(super) const ORIGIN: &str = "https://h5.kugou.com";

pub(super) fn challenge(
    reply: &Envelope,
    receipt: &KugouWebSmsChallenge,
) -> Result<AuthBrowserChallenge> {
    let data: String = serde_json::from_str(reply.data.as_ref().ok_or_else(malformed)?.get())
        .map_err(|_| malformed())?;
    if data.len() > 4096 || data.split('&').count() > 64 {
        return Err(malformed());
    }
    let mut event = None;
    for item in data.split('&') {
        if let Some(("eventid", value)) = item.split_once('=') {
            if event.is_some() {
                return Err(malformed());
            }
            let value = decode(value).map_err(|_| malformed())?;
            if value.is_empty() || value.len() > 512 || !value.bytes().all(|b| b.is_ascii_graphic())
            {
                return Err(malformed());
            }
            event = Some(value);
        }
    }
    let event = event.ok_or_else(malformed)?;
    let mut nonce = [0_u8; 32];
    SysRng.try_fill_bytes(&mut nonce).map_err(|_| {
        error(
            ErrorCode::InternalError,
            "Cannot create KuGou browser challenge",
        )
    })?;
    Ok(AuthBrowserChallenge {
        verification_id: hex::encode(nonce),
        // The official optional callbackName route posts dataJson to the parent.
        // A literal null return URL prevents choosing any caller-supplied redirect.
        url: format!(
            "{ORIGIN}/apps/verify-h5/dist/#/index/{}/1014/null/{}/TuneWeaveVerify",
            encode_segment(&event),
            encode_segment(&receipt.device.mid)
        ),
        message_origin: ORIGIN.into(),
        message_type: "kgVerifyCallbackData".into(),
        response_field: "dataJson".into(),
        remaining_attempts: 5_u8.saturating_sub(receipt.attempts),
    })
}

// decodeURIComponent semantics: decode once; '+' is a literal plus, not a space.
fn decode(input: &str) -> Result<String> {
    let mut out = Vec::with_capacity(input.len());
    let mut bytes = input.as_bytes().iter().copied();
    while let Some(b) = bytes.next() {
        out.push(if b == b'%' {
            let hi = bytes
                .next()
                .and_then(|b| (b as char).to_digit(16))
                .ok_or_else(invalid)?;
            let lo = bytes
                .next()
                .and_then(|b| (b as char).to_digit(16))
                .ok_or_else(invalid)?;
            (hi * 16 + lo) as u8
        } else {
            b
        });
    }
    String::from_utf8(out).map_err(|_| invalid())
}

fn encode_segment(input: &str) -> String {
    let mut out = String::new();
    for b in input.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&b) {
            out.push(b as char);
        } else {
            use std::fmt::Write;
            let _ = write!(out, "%{b:02X}");
        }
    }
    out
}

pub(super) struct Proof {
    pub header: Option<HeaderValue>,
    raw: String,
    encoded: String,
}
impl Proof {
    pub fn parse(response: &str) -> Result<Self> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Callback {
            status: u8,
            error_code: u32,
            #[serde(rename = "vType")]
            _verification_type: u16,
            #[serde(default, rename = "error_msg")]
            _message: Option<IgnoredAny>,
            verify_data: String,
        }
        if response.len() > 16 * 1024 {
            return Err(invalid());
        }
        let data: Callback = serde_json::from_str(response).map_err(|_| invalid())?;
        if data.status != 1 || data.error_code != 0 {
            return Err(invalid());
        }
        let raw = decode(&data.verify_data)?;
        if raw.len() > 8192
            || raw.trim() != raw
            || !raw.bytes().all(|b| b.is_ascii_graphic() || b == b' ')
        {
            return Err(invalid());
        }
        // Official encodeURIComponent(null/undefined) yields these literal strings.
        // Preserve them, and empty/no-header, without treating them as authentication.
        let header = if raw.is_empty() {
            None
        } else {
            let mut value = HeaderValue::from_str(&raw).map_err(|_| invalid())?;
            value.set_sensitive(true);
            Some(value)
        };
        Ok(Self {
            encoded: encode_segment(&raw),
            raw,
            header,
        })
    }

    pub fn reject_reflection(&self, bytes: &[u8]) -> Result<()> {
        if matches!(self.raw.as_str(), "" | "null" | "undefined") {
            return Ok(());
        }
        let contains = |bytes: &[u8]| {
            [self.raw.as_bytes(), self.encoded.as_bytes()]
                .into_iter()
                .any(|secret| bytes.windows(secret.len()).any(|w| w == secret))
        };
        // Inspect decoded string values too, so JSON escapes cannot hide an echo.
        fn reflected(value: &Value, contains: &impl Fn(&[u8]) -> bool) -> bool {
            match value {
                Value::String(s) => contains(s.as_bytes()),
                Value::Array(values) => values.iter().any(|v| reflected(v, contains)),
                Value::Object(values) => values
                    .iter()
                    .any(|(k, v)| contains(k.as_bytes()) || reflected(v, contains)),
                _ => false,
            }
        }
        if contains(bytes)
            || serde_json::from_slice::<Value>(bytes).is_ok_and(|v| reflected(&v, &contains))
        {
            return Err(malformed());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
