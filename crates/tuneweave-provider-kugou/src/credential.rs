use std::fmt;

use rand::{TryRng, rngs::SysRng};
use serde::{Deserialize, Serialize};
use tuneweave_core::{ErrorCode, Platform, ProviderCredential, Result, TuneWeaveError};

use crate::{KugouLoginClient, device::KugouDeviceIdentity};

pub(crate) const KIND: &str = "kugou_native_v1";
const MAX_CREDENTIAL_BYTES: usize = 65_536;

/// Native app tokens are distinct from Web cookies and per-song authorization.
#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NativeSession {
    pub client: KugouLoginClient,
    pub device: KugouDeviceIdentity,
    pub user_id: String,
    pub token: String,
    pub vip_token: Option<String>,
    pub t1: Option<String>,
}

impl NativeSession {
    pub(crate) fn valid(&self) -> bool {
        self.client != KugouLoginClient::Web
            && self.device.valid()
            && valid_uid(&self.user_id)
            && valid_secret(&self.token)
            && self.vip_token.as_deref().is_none_or(valid_secret)
            && self.t1.as_deref().is_none_or(valid_secret)
    }
}

impl fmt::Debug for NativeSession {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeSession")
            .field("client", &self.client)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct KugouCredential {
    version: u8,
    generation: String,
    pub session: NativeSession,
}

impl fmt::Debug for KugouCredential {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("KugouCredential { fields: [redacted] }")
    }
}

impl KugouCredential {
    pub(crate) fn same_login(&self, other: &Self) -> bool {
        self.generation == other.generation
            && self.session.client == other.session.client
            && self.session.user_id == other.session.user_id
            && self.session.device.guid == other.session.device.guid
            && self.session.device.mid == other.session.device.mid
    }

    pub(crate) fn stored(&self, account: &str) -> Result<tuneweave_core::StoredAccountCredential> {
        tuneweave_core::StoredAccountCredential::new(
            Platform::Kugou,
            account,
            KIND,
            self.caller()?.into_secret(),
        )
    }

    pub(crate) fn parse_stored(value: &tuneweave_core::StoredAccountCredential) -> Result<Self> {
        Self::parse_caller(&ProviderCredential::new(
            value.platform,
            &value.kind,
            value.secret(),
            None,
        )?)
    }
    /// Only call after the exchange and authenticated self-profile both succeed.
    pub(crate) fn verified(session: NativeSession) -> Result<Self> {
        if !session.valid() {
            return Err(invalid());
        }
        let mut generation = [0; 32];
        SysRng.try_fill_bytes(&mut generation).map_err(|_| {
            TuneWeaveError::new(
                ErrorCode::InternalError,
                "KuGou session randomness is unavailable",
            )
            .with_platform(Platform::Kugou)
        })?;
        Ok(Self {
            version: 1,
            generation: hex::encode(generation),
            session,
        })
    }

    pub(crate) fn parse_caller(credential: &ProviderCredential) -> Result<Self> {
        if credential.platform != Platform::Kugou
            || credential.kind != KIND
            || credential.expires_at.is_some()
            || credential.secret().len() > MAX_CREDENTIAL_BYTES
        {
            return Err(invalid());
        }
        let value: Self = serde_json::from_str(credential.secret()).map_err(|_| invalid())?;
        if value.version != 1
            || value.generation.len() != 64
            || !value
                .generation
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            || !value.session.valid()
        {
            return Err(invalid());
        }
        Ok(value)
    }

    pub(crate) fn caller(&self) -> Result<ProviderCredential> {
        let secret = serde_json::to_string(self).map_err(|_| {
            TuneWeaveError::new(ErrorCode::InternalError, "KuGou credential encoding failed")
                .with_platform(Platform::Kugou)
        })?;
        if secret.len() > MAX_CREDENTIAL_BYTES {
            return Err(invalid());
        }
        ProviderCredential::new(Platform::Kugou, KIND, secret, None)
    }

    pub(crate) fn rotate(&self, session: NativeSession) -> Result<Self> {
        if !session.valid()
            || session.client != self.session.client
            || session.user_id != self.session.user_id
            || session.device.guid != self.session.device.guid
            || session.device.mid != self.session.device.mid
        {
            return Err(invalid());
        }
        Ok(Self {
            session,
            ..self.clone()
        })
    }
}

pub(crate) fn valid_secret(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 16_384
        && !matches!(value, "null" | "undefined")
        && value.bytes().all(|b| (0x21..=0x7e).contains(&b))
}

pub(crate) fn valid_uid(value: &str) -> bool {
    !value.starts_with('0')
        && value.bytes().all(|b| b.is_ascii_digit())
        && value.parse::<u64>().is_ok()
}

fn invalid() -> TuneWeaveError {
    TuneWeaveError::new(
        ErrorCode::InvalidRequest,
        "KuGou native credential is invalid",
    )
    .with_platform(Platform::Kugou)
}

/// Dispatch by credential kind before strict deserialization. No untagged JSON buffering:
/// duplicate identity fields must still be rejected by each concrete credential schema.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum AccountCredential {
    Native(KugouCredential),
    Web(WebCredential),
}

#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WebCredential {
    version: u8,
    generation: String,
    pub session: crate::web::WebSession,
}
impl fmt::Debug for WebCredential {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("WebCredential { fields: [redacted] }")
    }
}

impl AccountCredential {
    pub(crate) fn verified(session: NativeSession) -> Result<Self> {
        KugouCredential::verified(session).map(Self::Native)
    }
    pub(crate) fn verified_web(session: crate::web::WebSession) -> Result<Self> {
        if !session.valid() {
            return Err(invalid());
        }
        let mut bytes = [0; 32];
        SysRng.try_fill_bytes(&mut bytes).map_err(|_| {
            TuneWeaveError::new(
                ErrorCode::InternalError,
                "KuGou session randomness unavailable",
            )
            .with_platform(Platform::Kugou)
        })?;
        Ok(Self::Web(WebCredential {
            version: 1,
            generation: hex::encode(bytes),
            session,
        }))
    }
    pub(crate) fn parse_caller(value: &ProviderCredential) -> Result<Self> {
        if value.kind == KIND {
            return KugouCredential::parse_caller(value).map(Self::Native);
        }
        if value.platform != Platform::Kugou
            || value.kind != crate::web::KIND
            || value.expires_at.is_some()
            || value.secret().len() > MAX_CREDENTIAL_BYTES
        {
            return Err(invalid());
        }
        let web: WebCredential = serde_json::from_str(value.secret()).map_err(|_| invalid())?;
        if web.version != 1
            || web.generation.len() != 64
            || !web
                .generation
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            || !web.session.valid()
        {
            return Err(invalid());
        }
        Ok(Self::Web(web))
    }
    pub(crate) fn caller(&self) -> Result<ProviderCredential> {
        match self {
            Self::Native(v) => v.caller(),
            Self::Web(v) => {
                let secret = serde_json::to_string(v).map_err(|_| invalid())?;
                if secret.len() > MAX_CREDENTIAL_BYTES {
                    return Err(invalid());
                }
                ProviderCredential::new(Platform::Kugou, crate::web::KIND, secret, None)
            }
        }
    }
    pub(crate) fn stored(&self, account: &str) -> Result<tuneweave_core::StoredAccountCredential> {
        if let Self::Native(value) = self {
            return value.stored(account);
        }
        let caller = self.caller()?;
        tuneweave_core::StoredAccountCredential::new(
            Platform::Kugou,
            account,
            &caller.kind,
            caller.secret(),
        )
    }
    pub(crate) fn parse_stored(value: &tuneweave_core::StoredAccountCredential) -> Result<Self> {
        if value.kind == KIND {
            return KugouCredential::parse_stored(value).map(Self::Native);
        }
        Self::parse_caller(&ProviderCredential::new(
            value.platform,
            &value.kind,
            value.secret(),
            None,
        )?)
    }
    pub(crate) fn same_login(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Native(a), Self::Native(b)) => a.same_login(b),
            (Self::Web(a), Self::Web(b)) => {
                a.generation == b.generation
                    && a.session.user_id == b.session.user_id
                    && a.session.device.guid == b.session.device.guid
                    && a.session.device.mid == b.session.device.mid
            }
            _ => false,
        }
    }
    pub(crate) fn user_id(&self) -> &str {
        match self {
            Self::Native(v) => &v.session.user_id,
            Self::Web(v) => &v.session.user_id,
        }
    }
    pub(crate) fn rotate_web(&self, session: crate::web::WebSession) -> Result<Self> {
        let Self::Web(old) = self else {
            return Err(invalid());
        };
        let next = Self::Web(WebCredential {
            session,
            ..old.clone()
        });
        if !next.same_login(self) || !matches!(&next, Self::Web(v) if v.session.valid()) {
            return Err(invalid());
        }
        Ok(next)
    }
    #[cfg(test)]
    pub(crate) fn native(&self) -> &KugouCredential {
        match self {
            Self::Native(v) => v,
            Self::Web(_) => panic!("expected native fixture"),
        }
    }
    #[cfg(test)]
    pub(crate) fn rotate(&self, session: NativeSession) -> Result<Self> {
        self.native().rotate(session).map(Self::Native)
    }
}
