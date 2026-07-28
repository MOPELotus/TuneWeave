use std::{collections::BTreeSet, fmt, sync::Arc};

use async_trait::async_trait;
use serde_json::json;
use tuneweave_core::{
    AccountCredentialStore, AccountProfile, AuthState, Capability, CredentialMode, ErrorCode,
    MusicProvider, Platform, ProviderAuthResult, ProviderCredential, ProviderQrPoll,
    ProviderQrStart, Result, StoredAccountCredential, TuneWeaveError,
};

use crate::client::{BilibiliClient, BilibiliConfig, BilibiliCredential, BilibiliQrPoll};

const BILIBILI_CREDENTIAL_KIND: &str = "bilibili_cookie_v1";

#[derive(Clone)]
pub struct BilibiliProvider {
    client: BilibiliClient,
    credential_store: Option<Arc<dyn AccountCredentialStore>>,
    caller_credential: Option<BilibiliCredential>,
}

impl fmt::Debug for BilibiliProvider {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BilibiliProvider")
            .field(
                "credential_store_configured",
                &self.credential_store.is_some(),
            )
            .field(
                "caller_credential_configured",
                &self.caller_credential.is_some(),
            )
            .finish_non_exhaustive()
    }
}

impl BilibiliProvider {
    pub fn new(config: BilibiliConfig) -> Result<Self> {
        let credential_store = config.credential_store.clone();
        Ok(Self {
            client: BilibiliClient::new(&config)?,
            credential_store,
            caller_credential: None,
        })
    }

    #[must_use]
    pub fn from_client(client: BilibiliClient) -> Self {
        Self {
            client,
            credential_store: None,
            caller_credential: None,
        }
    }
}

#[async_trait]
impl MusicProvider for BilibiliProvider {
    fn platform(&self) -> Platform {
        Platform::Bilibili
    }

    fn name(&self) -> &'static str {
        "Bilibili"
    }

    fn with_caller_credential(
        &self,
        credential: &ProviderCredential,
    ) -> Result<Arc<dyn MusicProvider>> {
        Ok(Arc::new(self.caller_credential_scope(credential)?))
    }

    fn capabilities(&self) -> BTreeSet<Capability> {
        BTreeSet::from([Capability::QrLogin, Capability::CallerManagedCredentials])
    }

    async fn start_qr_login(&self, login_type: Option<&str>) -> Result<ProviderQrStart> {
        if let Some(login_type) = login_type.map(str::trim).filter(|value| !value.is_empty())
            && !matches!(login_type, "default" | "web" | "bilibili")
        {
            return Err(TuneWeaveError::invalid_request(format!(
                "unsupported Bilibili QR login type: {login_type}"
            ))
            .with_platform(Platform::Bilibili));
        }
        let start = self.client.create_qr_login().await?;
        Ok(ProviderQrStart {
            provider_transaction_id: start.qrcode_key,
            url: start.image_data_url.clone(),
            image_data_url: Some(start.image_data_url),
            expires_at: None,
        })
    }

    async fn poll_qr_login(
        &self,
        provider_transaction_id: &str,
        account: &str,
    ) -> Result<ProviderQrPoll> {
        self.poll_qr_login_with_mode(provider_transaction_id, account, CredentialMode::Server)
            .await
    }

    async fn poll_qr_login_with_mode(
        &self,
        provider_transaction_id: &str,
        account: &str,
        mode: CredentialMode,
    ) -> Result<ProviderQrPoll> {
        validate_bilibili_login_account(account, mode)?;
        match self.client.poll_qr_login(provider_transaction_id).await? {
            BilibiliQrPoll::Waiting => Ok(ProviderQrPoll {
                state: AuthState::Waiting,
                message: Some("waiting for Bilibili QR scan".to_owned()),
                profile: None,
                credential: None,
            }),
            BilibiliQrPoll::Scanned => Ok(ProviderQrPoll {
                state: AuthState::Scanned,
                message: Some("Bilibili QR scanned; waiting for confirmation".to_owned()),
                profile: None,
                credential: None,
            }),
            BilibiliQrPoll::Expired => Ok(ProviderQrPoll {
                state: AuthState::Expired,
                message: Some("Bilibili QR login expired".to_owned()),
                profile: None,
                credential: None,
            }),
            BilibiliQrPoll::Failed { code, message } => Ok(ProviderQrPoll {
                state: AuthState::Failed,
                message: Some(format!("{message} ({code})")),
                profile: None,
                credential: None,
            }),
            BilibiliQrPoll::Confirmed {
                credential,
                timestamp_ms,
            } => {
                let mut result = self.finish_authentication(account, &credential, mode)?;
                if let Some(timestamp_ms) = timestamp_ms {
                    result
                        .profile
                        .extensions
                        .insert("login_timestamp_ms".to_owned(), json!(timestamp_ms));
                }
                Ok(ProviderQrPoll {
                    state: AuthState::Confirmed,
                    message: Some("Bilibili account authenticated".to_owned()),
                    profile: Some(result.profile),
                    credential: result.credential,
                })
            }
        }
    }
}

impl BilibiliProvider {
    fn caller_credential_scope(&self, credential: &ProviderCredential) -> Result<Self> {
        Ok(Self {
            client: self.client.clone(),
            credential_store: None,
            caller_credential: Some(parse_bilibili_caller_credential(credential)?),
        })
    }

    fn finish_authentication(
        &self,
        account: &str,
        credential: &BilibiliCredential,
        mode: CredentialMode,
    ) -> Result<ProviderAuthResult> {
        validate_bilibili_login_account(account, mode)?;
        let secret = serde_json::to_string(credential).map_err(|_| {
            TuneWeaveError::new(
                ErrorCode::InternalError,
                "failed to serialize Bilibili account credential",
            )
            .with_platform(Platform::Bilibili)
        })?;
        let caller_credential = mode
            .returns_to_caller()
            .then(|| {
                ProviderCredential::new(Platform::Bilibili, BILIBILI_CREDENTIAL_KIND, &secret, None)
            })
            .transpose()?;
        if mode.persists_on_server() {
            let store = self.credential_store.as_ref().ok_or_else(|| {
                TuneWeaveError::new(
                    ErrorCode::InternalError,
                    "Bilibili account storage is not configured",
                )
                .with_platform(Platform::Bilibili)
            })?;
            store.put(&StoredAccountCredential::new(
                Platform::Bilibili,
                account,
                BILIBILI_CREDENTIAL_KIND,
                &secret,
            )?)?;
        }
        let mut profile = AccountProfile::authenticated(Platform::Bilibili, account);
        profile.user_id = Some(credential.user_id().to_owned());
        profile.extensions.insert(
            "credential_kind".to_owned(),
            json!(BILIBILI_CREDENTIAL_KIND),
        );
        Ok(ProviderAuthResult {
            profile,
            credential: caller_credential,
        })
    }

    #[cfg(test)]
    fn selected_credential(&self, account: &str) -> Result<BilibiliCredential> {
        if let Some(credential) = &self.caller_credential {
            if account == "default" {
                return Ok(credential.clone());
            }
            return Err(bilibili_authentication_required(
                account,
                "caller-managed Bilibili credentials do not expose server account aliases",
            ));
        }
        let store = self.credential_store.as_ref().ok_or_else(|| {
            bilibili_authentication_required(account, "Bilibili account storage is not configured")
        })?;
        let stored = store
            .load_platform(Platform::Bilibili)?
            .into_iter()
            .find(|credential| credential.account == account)
            .ok_or_else(|| {
                bilibili_authentication_required(account, "Bilibili account was not found")
            })?;
        if stored.kind != BILIBILI_CREDENTIAL_KIND {
            return Err(TuneWeaveError::new(
                ErrorCode::InternalError,
                "stored Bilibili credential has an unsupported kind",
            )
            .with_platform(Platform::Bilibili));
        }
        serde_json::from_str::<BilibiliCredential>(stored.secret())
            .map_err(|_| {
                TuneWeaveError::new(
                    ErrorCode::InternalError,
                    "stored Bilibili credential is malformed",
                )
                .with_platform(Platform::Bilibili)
            })?
            .normalize()
            .map_err(|_| {
                TuneWeaveError::new(
                    ErrorCode::InternalError,
                    "stored Bilibili credential is invalid",
                )
                .with_platform(Platform::Bilibili)
            })
    }
}

fn validate_bilibili_login_account(account: &str, mode: CredentialMode) -> Result<()> {
    let account = account.trim();
    if account.is_empty() {
        return Err(
            TuneWeaveError::invalid_request("Bilibili account alias cannot be empty")
                .with_platform(Platform::Bilibili),
        );
    }
    if account.len() > 64 {
        return Err(TuneWeaveError::invalid_request(
            "Bilibili account alias cannot exceed 64 bytes",
        )
        .with_platform(Platform::Bilibili));
    }
    if mode == CredentialMode::Client && account != "default" {
        return Err(TuneWeaveError::invalid_request(
            "client credential mode does not accept a server account alias",
        )
        .with_platform(Platform::Bilibili));
    }
    Ok(())
}

fn parse_bilibili_caller_credential(credential: &ProviderCredential) -> Result<BilibiliCredential> {
    if credential.platform != Platform::Bilibili {
        return Err(TuneWeaveError::invalid_request(
            "caller credential platform does not match Bilibili",
        )
        .with_platform(Platform::Bilibili));
    }
    if credential.kind != BILIBILI_CREDENTIAL_KIND {
        return Err(TuneWeaveError::invalid_request(
            "caller credential kind is not supported by Bilibili",
        )
        .with_platform(Platform::Bilibili));
    }
    if credential.expires_at.is_some() {
        return Err(TuneWeaveError::invalid_request(
            "caller Bilibili credential expiry does not match its payload",
        )
        .with_platform(Platform::Bilibili));
    }
    serde_json::from_str::<BilibiliCredential>(credential.secret())
        .map_err(|_| {
            TuneWeaveError::invalid_request("caller Bilibili credential payload is malformed")
                .with_platform(Platform::Bilibili)
        })?
        .normalize()
        .map_err(|_| {
            TuneWeaveError::invalid_request("caller Bilibili credential payload is invalid")
                .with_platform(Platform::Bilibili)
        })
}

#[cfg(test)]
fn bilibili_authentication_required(account: &str, message: &str) -> TuneWeaveError {
    TuneWeaveError::new(ErrorCode::AuthenticationRequired, message)
        .with_platform(Platform::Bilibili)
        .with_details(json!({ "account": account }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[derive(Default)]
    struct RecordingCredentialStore {
        credentials: Mutex<Vec<StoredAccountCredential>>,
    }

    impl AccountCredentialStore for RecordingCredentialStore {
        fn load_platform(&self, platform: Platform) -> Result<Vec<StoredAccountCredential>> {
            Ok(self
                .credentials
                .lock()
                .expect("credential store lock")
                .iter()
                .filter(|credential| credential.platform == platform)
                .cloned()
                .collect())
        }

        fn put(&self, credential: &StoredAccountCredential) -> Result<()> {
            let mut credentials = self.credentials.lock().expect("credential store lock");
            credentials.retain(|stored| {
                stored.platform != credential.platform || stored.account != credential.account
            });
            credentials.push(credential.clone());
            Ok(())
        }

        fn remove(&self, platform: Platform, account: &str) -> Result<bool> {
            let mut credentials = self.credentials.lock().expect("credential store lock");
            let before = credentials.len();
            credentials.retain(|stored| stored.platform != platform || stored.account != account);
            Ok(before != credentials.len())
        }
    }

    fn sample_credential() -> BilibiliCredential {
        BilibiliCredential {
            dede_user_id: "47275982".to_owned(),
            dede_user_id_ck_md5: "0123456789abcdef".to_owned(),
            sessdata: "private%2Csession".to_owned(),
            bili_jct: "0123456789abcdef0123456789abcdef".to_owned(),
            sid: Some("private-sid".to_owned()),
            refresh_token: "private-refresh".to_owned(),
        }
        .normalize()
        .expect("sample credential")
    }

    #[test]
    fn qr_login_supports_server_client_and_both_credential_ownership() {
        let credential = sample_credential();
        let client_only = BilibiliProvider::new(BilibiliConfig::default()).expect("provider");
        let client_result = client_only
            .finish_authentication("default", &credential, CredentialMode::Client)
            .expect("client-owned login");
        assert_eq!(client_result.profile.user_id.as_deref(), Some("47275982"));
        let caller = client_result.credential.expect("caller credential");
        assert_eq!(caller.kind, BILIBILI_CREDENTIAL_KIND);
        assert_eq!(
            client_only
                .finish_authentication("named", &credential, CredentialMode::Client)
                .expect_err("client mode rejects aliases")
                .code,
            ErrorCode::InvalidRequest
        );
        assert_eq!(
            client_only
                .finish_authentication("default", &credential, CredentialMode::Server)
                .expect_err("server mode requires storage")
                .code,
            ErrorCode::InternalError
        );

        let store = Arc::new(RecordingCredentialStore::default());
        let both = BilibiliProvider::new(BilibiliConfig {
            credential_store: Some(store.clone()),
            ..BilibiliConfig::default()
        })
        .expect("provider with storage");
        let both_result = both
            .finish_authentication("personal", &credential, CredentialMode::Both)
            .expect("both-owned login");
        let caller = both_result.credential.expect("both caller credential");
        let stored = store
            .load_platform(Platform::Bilibili)
            .expect("stored credentials");
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].account, "personal");
        assert_eq!(stored[0].secret(), caller.secret());
        assert!(!format!("{both:?}").contains("private"));
    }

    #[test]
    fn caller_credentials_are_strongly_validated_and_isolated_from_aliases() {
        let credential = sample_credential();
        let secret = serde_json::to_string(&credential).expect("credential JSON");
        let material =
            ProviderCredential::new(Platform::Bilibili, BILIBILI_CREDENTIAL_KIND, secret, None)
                .expect("provider credential");
        let provider = BilibiliProvider::new(BilibiliConfig::default()).expect("provider");
        let scoped = provider
            .caller_credential_scope(&material)
            .expect("caller scope");
        assert_eq!(
            scoped
                .selected_credential("default")
                .expect("caller credential"),
            credential
        );
        assert_eq!(
            scoped
                .selected_credential("server-alias")
                .expect_err("caller scope must isolate aliases")
                .code,
            ErrorCode::AuthenticationRequired
        );

        for invalid in [
            ProviderCredential::new(Platform::Qq, BILIBILI_CREDENTIAL_KIND, "{}", None)
                .expect("wrong platform material"),
            ProviderCredential::new(Platform::Bilibili, "cookie", "{}", None)
                .expect("wrong kind material"),
            ProviderCredential::new(Platform::Bilibili, BILIBILI_CREDENTIAL_KIND, "{}", Some(1))
                .expect("wrong expiry material"),
        ] {
            let error = provider
                .caller_credential_scope(&invalid)
                .expect_err("invalid caller credential must fail");
            assert_eq!(error.code, ErrorCode::InvalidRequest);
        }
    }

    #[tokio::test]
    #[ignore = "requires live Bilibili Passport access"]
    async fn live_provider_creates_a_qr_image_without_exposing_the_poll_key() {
        let provider = BilibiliProvider::new(BilibiliConfig::default()).expect("provider");
        let start = provider
            .start_qr_login(None)
            .await
            .expect("provider QR start");
        assert_eq!(start.provider_transaction_id.len(), 32);
        assert!(start.url.starts_with("data:image/svg+xml;base64,"));
        assert_eq!(start.image_data_url.as_deref(), Some(start.url.as_str()));
    }
}
