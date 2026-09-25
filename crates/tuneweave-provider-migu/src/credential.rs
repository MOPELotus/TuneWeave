use std::fmt;

use rand::{TryRng, rngs::SysRng};
use serde::{Deserialize, Serialize};
use tuneweave_core::{ErrorCode, Platform, ProviderCredential, Result, TuneWeaveError};

pub(crate) const KIND: &str = "migu_pacm_v1";
pub(crate) const MAX_TOKEN_BYTES: usize = 16_384;
const MAX_CREDENTIAL_BYTES: usize = 65_536;

/// A music-session credential. Passport login tokens and browser cookies are not retained.
#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MiguCredential {
    version: u8,
    generation: String,
    user_id: String,
    pacm: String,
}

impl fmt::Debug for MiguCredential {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("MiguCredential { fields: [redacted] }")
    }
}

impl MiguCredential {
    pub(crate) fn verified(user_id: String, pacm: String) -> Result<Self> {
        validate_uid(&user_id)?;
        validate_token(&pacm)?;
        let mut generation = [0; 32];
        SysRng.try_fill_bytes(&mut generation).map_err(|_| {
            error(
                ErrorCode::InternalError,
                "Migu session randomness is unavailable",
            )
        })?;
        Ok(Self {
            version: 1,
            generation: hex::encode(generation),
            user_id,
            pacm,
        })
    }

    pub(crate) fn parse(secret: &str) -> Result<Self> {
        if secret.len() > MAX_CREDENTIAL_BYTES {
            return Err(invalid_credential());
        }
        let value: Self = serde_json::from_str(secret).map_err(|_| invalid_credential())?;
        if value.version != 1
            || value.generation.len() != 64
            || !value
                .generation
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(invalid_credential());
        }
        validate_uid(&value.user_id)?;
        validate_token(&value.pacm)?;
        Ok(value)
    }

    pub(crate) fn parse_caller(value: &ProviderCredential) -> Result<Self> {
        if value.platform != Platform::Migu || value.kind != KIND || value.expires_at.is_some() {
            return Err(invalid_credential());
        }
        Self::parse(value.secret())
    }

    pub(crate) fn serialize(&self) -> Result<String> {
        serde_json::to_string(self).map_err(|_| {
            error(
                ErrorCode::InternalError,
                "Migu credential serialization failed",
            )
        })
    }

    pub(crate) fn caller(&self) -> Result<ProviderCredential> {
        ProviderCredential::new(Platform::Migu, KIND, self.serialize()?, None)
    }

    pub(crate) fn user_id(&self) -> &str {
        &self.user_id
    }
    pub(crate) fn token(&self) -> &str {
        &self.pacm
    }

    pub(crate) fn rotate(&self, token: String) -> Result<Self> {
        validate_token(&token)?;
        Ok(Self {
            pacm: token,
            ..self.clone()
        })
    }

    pub(crate) fn same_login(&self, other: &Self) -> bool {
        self.generation == other.generation && self.user_id == other.user_id
    }

    // A content fingerprint bound to this login, not a bearer or an upstream
    // revision. PACM rotation preserves it; a new login invalidates old views.
    pub(crate) fn playlist_occurrence_digest(&self, material: &[u8]) -> String {
        use sha1::{Digest, Sha1};
        let mut digest = Sha1::new();
        digest.update(b"migu_playlist_occurrence_snapshot_v1\0");
        digest.update(self.generation.as_bytes());
        digest.update([0_u8]);
        digest.update(self.user_id.as_bytes());
        digest.update([0_u8]);
        digest.update(material);
        hex::encode(digest.finalize())
    }
}

pub(crate) fn import_cookie(value: &str) -> Result<String> {
    if value.is_empty()
        || value.len() > MAX_CREDENTIAL_BYTES
        || !value.is_ascii()
        || value.bytes().any(|b| b.is_ascii_control())
    {
        return Err(invalid_credential());
    }
    let mut pacm = None;
    for part in value.split(';') {
        let (name, token) = part.trim().split_once('=').ok_or_else(invalid_credential)?;
        if name.is_empty()
            || !name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b))
        {
            return Err(invalid_credential());
        }
        if name == "pacmtoken" {
            if pacm.is_some() {
                return Err(invalid_credential());
            }
            validate_token(token)?;
            pacm = Some(token.to_owned());
        }
    }
    pacm.ok_or_else(invalid_credential)
}

pub(crate) fn validate_token(value: &str) -> Result<()> {
    // RFC6265 cookie-octets also form a safe single HTTP header value.
    if value.is_empty()
        || value.len() > MAX_TOKEN_BYTES
        || matches!(value, "null" | "undefined")
        || !value
            .bytes()
            .all(|b| matches!(b, 0x21 | 0x23..=0x2b | 0x2d..=0x3a | 0x3c..=0x5b | 0x5d..=0x7e))
    {
        return Err(invalid_credential());
    }
    Ok(())
}

pub(crate) fn validate_uid(value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 128
        || matches!(value, "0" | "null" | "undefined")
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
    {
        return Err(invalid_credential());
    }
    Ok(())
}

pub(crate) fn error(code: ErrorCode, message: &'static str) -> TuneWeaveError {
    TuneWeaveError::new(code, message).with_platform(Platform::Migu)
}
pub(crate) fn authentication_required() -> TuneWeaveError {
    error(
        ErrorCode::AuthenticationRequired,
        "The selected Migu session is not authenticated",
    )
}
fn invalid_credential() -> TuneWeaveError {
    error(ErrorCode::InvalidRequest, "Migu credential is invalid")
}
