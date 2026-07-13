use std::{collections::BTreeMap, fmt};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::Platform;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthState {
    Waiting,
    Scanned,
    Confirmed,
    Expired,
    Failed,
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
        };
        let challenge = AuthChallengeRequest {
            account: "default".to_owned(),
            method: ChallengeMethod::Sms,
            principal: "13800138000".to_owned(),
            country_code: Some("86".to_owned()),
        };
        let output = format!("{password:?} {challenge:?}");
        assert!(!output.contains("secret@example.test"));
        assert!(!output.contains("password-secret"));
        assert!(!output.contains("13800138000"));
    }

    #[test]
    fn auth_state_only_marks_final_states_as_terminal() {
        assert!(!AuthState::Waiting.is_terminal());
        assert!(!AuthState::Scanned.is_terminal());
        assert!(AuthState::Confirmed.is_terminal());
        assert!(AuthState::Expired.is_terminal());
        assert!(AuthState::Failed.is_terminal());
    }
}
