use std::{collections::BTreeMap, fmt};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{Platform, ProviderCredential};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthState {
    Waiting,
    Scanned,
    Confirmed,
    Expired,
    Failed,
}

/// Selects who owns a credential created by a successful authentication flow.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CredentialMode {
    /// Persist the credential under the server-side `(platform, account)` key.
    #[default]
    Server,
    /// Return the credential to the caller without persisting it on the server.
    Client,
    /// Persist and return the exact same credential generation.
    Both,
}

impl CredentialMode {
    #[must_use]
    pub const fn persists_on_server(self) -> bool {
        matches!(self, Self::Server | Self::Both)
    }

    #[must_use]
    pub const fn returns_to_caller(self) -> bool {
        matches!(self, Self::Client | Self::Both)
    }
}

impl AuthState {
    #[must_use]
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Confirmed | Self::Expired | Self::Failed)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrincipalType {
    Email,
    Phone,
    Username,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PasswordFormat {
    #[default]
    Plain,
    Md5,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChallengeMethod {
    Sms,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AccountProfile {
    pub platform: Platform,
    pub account: String,
    pub user_id: Option<String>,
    pub nickname: Option<String>,
    pub avatar_url: Option<String>,
    pub authenticated: bool,
    pub extensions: BTreeMap<String, Value>,
}

/// Provider authentication output before its credential is wrapped for a public API response.
#[derive(Clone, Debug, PartialEq)]
pub struct ProviderAuthResult {
    pub profile: AccountProfile,
    pub credential: Option<ProviderCredential>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProviderLogoutResult {
    /// Whether a server-owned account alias was removed.
    pub removed: bool,
    /// Whether the caller must discard the credential it supplied.
    pub caller_credential_discard_required: bool,
}

impl ProviderAuthResult {
    #[must_use]
    pub const fn server_managed(profile: AccountProfile) -> Self {
        Self {
            profile,
            credential: None,
        }
    }
}

impl AccountProfile {
    #[must_use]
    pub fn authenticated(platform: Platform, account: impl Into<String>) -> Self {
        Self {
            platform,
            account: account.into(),
            user_id: None,
            nickname: None,
            avatar_url: None,
            authenticated: true,
            extensions: BTreeMap::new(),
        }
    }
}

#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct PasswordLoginRequest {
    pub account: String,
    pub principal_type: PrincipalType,
    pub principal: String,
    pub password: String,
    #[serde(default)]
    pub password_format: PasswordFormat,
    pub country_code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secure_captcha: Option<String>,
}

impl fmt::Debug for PasswordLoginRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PasswordLoginRequest")
            .field("account", &self.account)
            .field("principal_type", &self.principal_type)
            .field("principal", &"[redacted]")
            .field("password", &"[redacted]")
            .field("password_format", &self.password_format)
            .field("country_code", &self.country_code)
            .field("has_secure_captcha", &self.secure_captcha.is_some())
            .finish()
    }
}

#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct AuthChallengeRequest {
    pub account: String,
    pub method: ChallengeMethod,
    pub principal: String,
    pub country_code: Option<String>,
}

impl fmt::Debug for AuthChallengeRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AuthChallengeRequest")
            .field("account", &self.account)
            .field("method", &self.method)
            .field("principal", &"[redacted]")
            .field("country_code", &self.country_code)
            .finish()
    }
}

/// The result of validating a one-time authentication challenge without creating a session.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AuthChallengeValidation {
    pub method: ChallengeMethod,
    pub valid: bool,
    pub platform_code: Option<String>,
    pub message: Option<String>,
    pub extensions: BTreeMap<String, Value>,
}

#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct AuthPrincipalStatusRequest {
    pub account: String,
    pub principal_type: PrincipalType,
    pub principal: String,
    pub country_code: Option<String>,
}

impl fmt::Debug for AuthPrincipalStatusRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AuthPrincipalStatusRequest")
            .field("account", &self.account)
            .field("principal_type", &self.principal_type)
            .field("principal", &"[redacted]")
            .field("country_code", &self.country_code)
            .finish()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AuthPrincipalStatus {
    pub principal_type: PrincipalType,
    pub exists: bool,
    pub has_password: Option<bool>,
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
    pub platform_code: Option<String>,
    pub extensions: BTreeMap<String, Value>,
}

#[derive(Clone, Eq, PartialEq)]
pub struct ProviderQrStart {
    pub provider_transaction_id: String,
    pub url: String,
    pub image_data_url: Option<String>,
    pub expires_at: Option<String>,
}

impl fmt::Debug for ProviderQrStart {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProviderQrStart")
            .field("provider_transaction_id", &"[redacted]")
            .field("url", &"[redacted]")
            .field("has_image_data_url", &self.image_data_url.is_some())
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProviderQrPoll {
    pub state: AuthState,
    pub message: Option<String>,
    pub profile: Option<AccountProfile>,
    /// Present only for a confirmed caller-managed login result.
    #[serde(skip)]
    pub credential: Option<ProviderCredential>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sensitive_auth_requests_are_redacted_in_debug_output() {
        let password = PasswordLoginRequest {
            account: "default".to_owned(),
            principal_type: PrincipalType::Email,
            principal: "secret@example.test".to_owned(),
            password: "password-secret".to_owned(),
            password_format: PasswordFormat::Plain,
            country_code: None,
            secure_captcha: Some("secure-captcha-secret".to_owned()),
        };
        let challenge = AuthChallengeRequest {
            account: "default".to_owned(),
            method: ChallengeMethod::Sms,
            principal: "13800138000".to_owned(),
            country_code: Some("86".to_owned()),
        };
        let status = AuthPrincipalStatusRequest {
            account: "default".to_owned(),
            principal_type: PrincipalType::Phone,
            principal: "13900139000".to_owned(),
            country_code: Some("86".to_owned()),
        };
        let output = format!("{password:?} {challenge:?} {status:?}");
        assert!(!output.contains("secret@example.test"));
        assert!(!output.contains("password-secret"));
        assert!(!output.contains("secure-captcha-secret"));
        assert!(!output.contains("13800138000"));
        assert!(!output.contains("13900139000"));
    }

    #[test]
    fn auth_state_only_marks_final_states_as_terminal() {
        assert!(!AuthState::Waiting.is_terminal());
        assert!(!AuthState::Scanned.is_terminal());
        assert!(AuthState::Confirmed.is_terminal());
        assert!(AuthState::Expired.is_terminal());
        assert!(AuthState::Failed.is_terminal());
    }

    #[test]
    fn credential_modes_preserve_the_default_server_ownership_contract() {
        assert_eq!(CredentialMode::default(), CredentialMode::Server);
        assert!(CredentialMode::Server.persists_on_server());
        assert!(!CredentialMode::Server.returns_to_caller());
        assert!(!CredentialMode::Client.persists_on_server());
        assert!(CredentialMode::Client.returns_to_caller());
        assert!(CredentialMode::Both.persists_on_server());
        assert!(CredentialMode::Both.returns_to_caller());
        assert_eq!(
            serde_json::from_str::<CredentialMode>("\"client\"").expect("client mode"),
            CredentialMode::Client
        );
    }
}
