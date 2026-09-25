use super::*;
use rand::{TryRng, rngs::SysRng};
use tuneweave_core::{AccountProfile, ProviderCredential};

const KIND: &str = "kuwo_native_v1";
const MAX_BYTES: usize = 16_384;

/// A format-checked credential still requires independent network validation.
#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NativeCredential {
    version: u8,
    generation: String,
    user_id: String,
    session_id: String,
    app_uid: String,
    context: device::NativeDeviceContext,
}
impl fmt::Debug for NativeCredential {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("KuwoNativeCredential { fields: [redacted] }")
    }
}
impl NativeCredential {
    /// The caller must have independently verified this exact UID/SID first.
    pub(crate) fn verified(input: &KuwoNativeSessionInput) -> Result<Self> {
        if input
            .context
            .as_ref()
            .is_none_or(|context| context.device_user != input.device_user())
        {
            return Err(invalid_credential());
        }
        let mut bytes = [0; 32];
        SysRng.try_fill_bytes(&mut bytes).map_err(|_| {
            TuneWeaveError::new(
                ErrorCode::InternalError,
                "Kuwo credential randomness is unavailable",
            )
            .with_platform(Platform::Kuwo)
        })?;
        let mut generation = String::with_capacity(64);
        for byte in bytes {
            write!(generation, "{byte:02x}").map_err(|_| invalid_credential())?;
        }
        let credential = Self {
            version: 1,
            generation,
            user_id: input.user_id().into(),
            session_id: input.session_id().into(),
            app_uid: input.device_id().into(),
            context: input.context.clone().ok_or_else(invalid_credential)?,
        };
        credential.input()?;
        Ok(credential)
    }
    pub(crate) fn input(&self) -> Result<KuwoNativeSessionInput> {
        if self.version != 1
            || self.generation.len() != 64
            || !self
                .generation
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            || !self.context.valid()
            || self.app_uid.starts_with('0')
            || !self.app_uid.bytes().all(|b| b.is_ascii_digit())
            || !self.app_uid.parse::<u64>().is_ok_and(|id| id > 0)
        {
            return Err(invalid_credential());
        }
        let mut input = KuwoNativeSessionInput::new(
            &self.user_id,
            &self.session_id,
            &self.app_uid,
            &self.context.device_user,
        )
        .map_err(|_| invalid_credential())?;
        input.context = Some(self.context.clone());
        Ok(input)
    }
    pub(crate) fn parse(caller: &ProviderCredential) -> Result<Self> {
        if caller.platform != Platform::Kuwo
            || caller.kind != KIND
            || caller.expires_at.is_some()
            || caller.secret().len() > MAX_BYTES
        {
            return Err(invalid_credential());
        }
        let value: Self =
            serde_json::from_str(caller.secret()).map_err(|_| invalid_credential())?;
        value.input()?;
        Ok(value)
    }
    pub(crate) fn caller(&self) -> Result<ProviderCredential> {
        self.input()?;
        let secret = serde_json::to_string(self).map_err(|_| invalid_credential())?;
        if secret.len() > MAX_BYTES {
            return Err(invalid_credential());
        }
        ProviderCredential::new(Platform::Kuwo, KIND, secret, None)
    }
    pub(crate) fn same_login(&self, other: &Self) -> bool {
        self.version == other.version
            && self.generation == other.generation
            && self.user_id == other.user_id
            && self.app_uid == other.app_uid
            && self.context == other.context
    }
    pub(crate) fn rotate(&self, input: &KuwoNativeSessionInput) -> Result<Self> {
        if input.user_id() != self.user_id
            || input.device_id() != self.app_uid
            || input.device_user() != self.context.device_user
            || input.context.as_ref() != Some(&self.context)
        {
            return Err(invalid_credential());
        }
        let value = Self {
            session_id: input.session_id().into(),
            ..self.clone()
        };
        value.input()?;
        Ok(value)
    }
    pub(crate) fn stored(&self, account: &str) -> Result<tuneweave_core::StoredAccountCredential> {
        tuneweave_core::StoredAccountCredential::new(
            Platform::Kuwo,
            account,
            KIND,
            self.caller()?.into_secret(),
        )
    }
    pub(crate) fn parse_stored(value: &tuneweave_core::StoredAccountCredential) -> Result<Self> {
        Self::parse(&ProviderCredential::new(
            value.platform,
            &value.kind,
            value.secret(),
            None,
        )?)
    }
}
pub(crate) fn profile(input: &KuwoNativeSessionInput, nickname: Option<String>) -> AccountProfile {
    AccountProfile {
        platform: Platform::Kuwo,
        account: "default".into(),
        user_id: Some(input.user_id().into()),
        nickname,
        avatar_url: None,
        authenticated: true,
        extensions: BTreeMap::new(),
    }
}
fn invalid_credential() -> TuneWeaveError {
    kuwo_invalid_request("Kuwo native credential is invalid")
}
