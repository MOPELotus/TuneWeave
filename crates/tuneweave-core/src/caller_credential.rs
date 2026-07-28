use std::fmt;

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};

use crate::{Platform, Result, TuneWeaveError};

pub const CALLER_CREDENTIAL_FORMAT: &str = "tuneweave_credential_v1";
pub const CALLER_CREDENTIAL_HEADER: &str = "x-tuneweave-credential";

const CALLER_CREDENTIAL_PREFIX: &str = "twc1_";
const CALLER_CREDENTIAL_VERSION: u8 = 1;
const MAX_CALLER_CREDENTIAL_DECODED_BYTES: usize = 64 * 1024;
const MAX_CALLER_CREDENTIAL_ENCODED_BYTES: usize =
    MAX_CALLER_CREDENTIAL_DECODED_BYTES.div_ceil(3) * 4;
const MAX_PROVIDER_CREDENTIAL_KIND_BYTES: usize = 128;

/// A provider-owned credential before it is placed in the public opaque envelope.
#[derive(Clone, Eq, PartialEq)]
pub struct ProviderCredential {
    pub platform: Platform,
    pub kind: String,
    secret: String,
    pub expires_at: Option<u64>,
}

impl ProviderCredential {
    pub fn new(
        platform: Platform,
        kind: impl Into<String>,
        secret: impl Into<String>,
        expires_at: Option<u64>,
    ) -> Result<Self> {
        let credential = Self {
            platform,
            kind: kind.into(),
            secret: secret.into(),
            expires_at,
        };
        credential.validate()?;
        Ok(credential)
    }

    #[must_use]
    pub fn secret(&self) -> &str {
        &self.secret
    }

    #[must_use]
    pub fn into_secret(self) -> String {
        self.secret
    }

    #[must_use]
    pub fn is_expired_at(&self, unix_seconds: u64) -> bool {
        self.expires_at
            .is_some_and(|expires_at| unix_seconds >= expires_at)
    }

    fn validate(&self) -> Result<()> {
        if self.kind.is_empty() || self.kind.len() > MAX_PROVIDER_CREDENTIAL_KIND_BYTES {
            return Err(TuneWeaveError::invalid_request(format!(
                "provider credential kind must contain 1 to {MAX_PROVIDER_CREDENTIAL_KIND_BYTES} bytes"
            )));
        }
        if !self
            .kind
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
        {
            return Err(TuneWeaveError::invalid_request(
                "provider credential kind contains unsupported characters",
            ));
        }
        if self.secret.is_empty() {
            return Err(TuneWeaveError::invalid_request(
                "provider credential secret cannot be empty",
            ));
        }
        if self.secret.len() > MAX_CALLER_CREDENTIAL_DECODED_BYTES {
            return Err(TuneWeaveError::invalid_request(
                "provider credential secret exceeds the caller credential size limit",
            ));
        }
        Ok(())
    }
}

impl fmt::Debug for ProviderCredential {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProviderCredential")
            .field("platform", &self.platform)
            .field("kind", &self.kind)
            .field("has_secret", &true)
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

/// A versioned bearer credential returned to and later supplied by an API caller.
#[derive(Clone, Eq, PartialEq, Serialize)]
pub struct CallerCredential {
    pub format: String,
    pub platform: Platform,
    pub value: String,
    pub expires_at: Option<u64>,
}

impl CallerCredential {
    pub fn issue(credential: &ProviderCredential) -> Result<Self> {
        credential.validate()?;
        let envelope = CredentialEnvelope {
            version: CALLER_CREDENTIAL_VERSION,
            platform: credential.platform,
            kind: credential.kind.clone(),
            secret: credential.secret.clone(),
            expires_at: credential.expires_at,
        };
        let encoded = serde_json::to_vec(&envelope).map_err(|_| {
            TuneWeaveError::invalid_request("provider credential could not be encoded")
        })?;
        if encoded.len() > MAX_CALLER_CREDENTIAL_DECODED_BYTES {
            return Err(TuneWeaveError::invalid_request(
                "provider credential exceeds the caller credential size limit",
            ));
        }
        Ok(Self {
            format: CALLER_CREDENTIAL_FORMAT.to_owned(),
            platform: credential.platform,
            value: format!(
                "{CALLER_CREDENTIAL_PREFIX}{}",
                URL_SAFE_NO_PAD.encode(encoded)
            ),
            expires_at: credential.expires_at,
        })
    }

    pub fn parse(value: &str) -> Result<ProviderCredential> {
        let encoded = value
            .strip_prefix(CALLER_CREDENTIAL_PREFIX)
            .ok_or_else(|| {
                TuneWeaveError::invalid_request("caller credential has an unsupported format")
            })?;
        if encoded.is_empty() || encoded.len() > MAX_CALLER_CREDENTIAL_ENCODED_BYTES {
            return Err(TuneWeaveError::invalid_request(
                "caller credential exceeds the supported size",
            ));
        }
        let decoded = URL_SAFE_NO_PAD.decode(encoded).map_err(|_| {
            TuneWeaveError::invalid_request("caller credential is not valid base64url")
        })?;
        if decoded.len() > MAX_CALLER_CREDENTIAL_DECODED_BYTES
            || URL_SAFE_NO_PAD.encode(&decoded) != encoded
        {
            return Err(TuneWeaveError::invalid_request(
                "caller credential is not in canonical form",
            ));
        }
        let envelope = serde_json::from_slice::<CredentialEnvelope>(&decoded).map_err(|_| {
            TuneWeaveError::invalid_request("caller credential payload is malformed")
        })?;
        if envelope.version != CALLER_CREDENTIAL_VERSION {
            return Err(TuneWeaveError::invalid_request(
                "caller credential has an unsupported version",
            ));
        }
        ProviderCredential::new(
            envelope.platform,
            envelope.kind,
            envelope.secret,
            envelope.expires_at,
        )
    }
}

impl fmt::Debug for CallerCredential {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CallerCredential")
            .field("format", &self.format)
            .field("platform", &self.platform)
            .field("value", &"[redacted]")
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct CredentialEnvelope {
    version: u8,
    platform: Platform,
    kind: String,
    secret: String,
    expires_at: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caller_credentials_round_trip_without_exposing_secrets_in_debug() {
        let material = ProviderCredential::new(
            Platform::Qq,
            "qq_credential_v1",
            r#"{"musickey":"private-key"}"#,
            Some(1_800_000_000),
        )
        .expect("provider credential");
        let caller = CallerCredential::issue(&material).expect("caller credential");
        assert_eq!(caller.format, CALLER_CREDENTIAL_FORMAT);
        assert_eq!(caller.platform, Platform::Qq);
        assert!(caller.value.starts_with(CALLER_CREDENTIAL_PREFIX));
        assert_eq!(caller.expires_at, material.expires_at);
        assert_eq!(
            CallerCredential::parse(&caller.value).expect("parse"),
            material
        );
        assert!(!format!("{caller:?}").contains("private-key"));
        assert!(
            !format!(
                "{:?}",
                CallerCredential::parse(&caller.value).expect("parse")
            )
            .contains("private-key")
        );
    }

    #[test]
    fn caller_credentials_reject_unknown_noncanonical_and_malformed_envelopes() {
        for value in [
            "",
            "raw-cookie",
            "twc2_Zm9v",
            "twc1_",
            "twc1_***",
            "twc1_Zm9v=",
            "twc1_Zm9v",
        ] {
            let error = CallerCredential::parse(value).expect_err("invalid credential");
            assert_eq!(error.code, crate::ErrorCode::InvalidRequest);
            if !value.is_empty() {
                assert!(!error.message.contains(value));
            }
        }

        for envelope in [
            serde_json::json!({
                "version": 2,
                "platform": "qq",
                "kind": "qq_credential_v1",
                "secret": "secret",
                "expires_at": null
            }),
            serde_json::json!({
                "version": 1,
                "platform": "qq",
                "kind": "qq_credential_v1",
                "secret": "secret",
                "expires_at": null,
                "unexpected": true
            }),
            serde_json::json!({
                "version": 1,
                "platform": "qq",
                "kind": "bad kind",
                "secret": "secret",
                "expires_at": null
            }),
        ] {
            let value = format!(
                "{CALLER_CREDENTIAL_PREFIX}{}",
                URL_SAFE_NO_PAD.encode(serde_json::to_vec(&envelope).expect("encode fixture"))
            );
            assert!(CallerCredential::parse(&value).is_err());
        }
    }

    #[test]
    fn caller_credentials_enforce_decoded_size_and_expiry_without_guessing_time() {
        let oversized = ProviderCredential::new(
            Platform::Netease,
            "cookie",
            "x".repeat(MAX_CALLER_CREDENTIAL_DECODED_BYTES),
            None,
        )
        .expect("material itself remains structurally valid");
        assert!(CallerCredential::issue(&oversized).is_err());

        let credential =
            ProviderCredential::new(Platform::Netease, "cookie", "MUSIC_U=private", Some(100))
                .expect("credential");
        assert!(!credential.is_expired_at(99));
        assert!(credential.is_expired_at(100));
        assert!(credential.is_expired_at(101));
    }
}
