use std::{collections::BTreeMap, fmt};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{Platform, ProviderCredential, TuneWeaveError};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthState {
    Waiting,
    Scanned,
    VerificationRequired,
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

/// Selects the provider protocol used to deliver an authentication challenge.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthChallengeBackend {
    /// Use the provider's normal challenge-delivery protocol.
    #[default]
    Standard,
    /// Use a provider's middle-layer login challenge protocol when available.
    Middle,
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

/// Evidence about the exact credential selected for an explicit upstream revocation.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionRevocationState {
    /// Authenticated before the request; independently rejected afterwards.
    Invalidated,
    /// Independently rejected before a revocation request was necessary.
    AlreadyInvalid,
    /// No server alias existed; says nothing about upstream credentials elsewhere.
    NoStoredSession,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ProviderSessionRevocationResult {
    pub state: SessionRevocationState,
    pub removed: bool,
    pub caller_credential_discard_required: bool,
    /// The send was started, not proof that the upstream received it.
    pub revocation_request_started: bool,
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

/// Credentials supplied for an explicit, identity-verified import. This is not an HTTP
/// request template or an already trusted TuneWeave credential generation.
#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ImportedCredential {
    Cookie { value: String },
}

impl fmt::Debug for ImportedCredential {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ImportedCredential::Cookie { value: [redacted] }")
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CredentialImportRequest {
    pub account: String,
    pub credential: ImportedCredential,
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

/// Explicit password transport. `Default` preserves each provider's existing flow;
/// selecting a transport never authorizes fallback to another one.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PasswordLoginBackend {
    #[default]
    Default,
    Web,
    Native,
}

#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct PasswordLoginRequest {
    #[serde(default)]
    pub backend: PasswordLoginBackend,
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

impl PasswordLoginRequest {
    /// Accepts an unspecified transport or the backend implemented by this entrypoint.
    pub fn require_backend(
        &self,
        platform: Platform,
        backend: PasswordLoginBackend,
    ) -> crate::Result<()> {
        if self.backend == PasswordLoginBackend::Default || self.backend == backend {
            Ok(())
        } else {
            Err(
                TuneWeaveError::invalid_request("Unsupported password login backend")
                    .with_platform(platform),
            )
        }
    }
}

impl fmt::Debug for PasswordLoginRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PasswordLoginRequest")
            .field("backend", &self.backend)
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
    #[serde(default)]
    pub backend: AuthChallengeBackend,
    pub principal: String,
    pub country_code: Option<String>,
    /// Explicit permission for providers whose SMS login may create an account.
    /// Currently supported by Kuwo and KuGou; other providers reject `true` as unsupported.
    #[serde(default)]
    pub allow_account_creation: bool,
    /// Explicit acceptance of platform policies required by a selected login backend.
    /// Currently required by Kuwo's Web SMS login; all other flows reject `true`.
    #[serde(default)]
    pub accept_platform_policies: bool,
}

impl AuthChallengeRequest {
    /// Rejects this option for a provider that does not implement it.
    pub fn reject_account_creation_option(&self, platform: Platform) -> crate::Result<()> {
        if self.allow_account_creation {
            return Err(crate::TuneWeaveError::invalid_request(
                "This provider does not support the allow_account_creation option",
            )
            .with_platform(platform));
        }
        Ok(())
    }

    /// Rejects platform-policy acceptance when the selected flow does not require it.
    pub fn reject_platform_policies_option(&self, platform: Platform) -> crate::Result<()> {
        if self.accept_platform_policies {
            return Err(crate::TuneWeaveError::invalid_request(
                "This login backend does not support the accept_platform_policies option",
            )
            .with_platform(platform));
        }
        Ok(())
    }
}

impl fmt::Debug for AuthChallengeRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AuthChallengeRequest")
            .field("allow_account_creation", &self.allow_account_creation)
            .field("accept_platform_policies", &self.accept_platform_policies)
            .field("account", &self.account)
            .field("method", &self.method)
            .field("backend", &self.backend)
            .field("principal", &"[redacted]")
            .field("country_code", &self.country_code)
            .finish()
    }
}

/// An in-memory receipt for one challenge delivery, including its original ownership.
/// Keep this value until verification; it is not a durable login credential or an HTTP payload.
/// Stateful providers must also verify the binding against their own transaction store.
#[derive(Clone, Eq, PartialEq)]
pub struct ProviderAuthChallenge {
    platform: Platform,
    request: AuthChallengeRequest,
    credential_mode: CredentialMode,
    provider_transaction_id: Option<String>,
}

impl ProviderAuthChallenge {
    #[must_use]
    pub fn stateless(
        platform: Platform,
        request: AuthChallengeRequest,
        credential_mode: CredentialMode,
    ) -> Self {
        Self {
            platform,
            request,
            credential_mode,
            provider_transaction_id: None,
        }
    }

    pub fn stateful(
        platform: Platform,
        request: AuthChallengeRequest,
        credential_mode: CredentialMode,
        provider_transaction_id: String,
    ) -> crate::Result<Self> {
        if provider_transaction_id.is_empty()
            || provider_transaction_id.len() > 1024
            || !provider_transaction_id
                .bytes()
                .all(|b| b.is_ascii_graphic())
        {
            return Err(crate::TuneWeaveError::invalid_request(
                "Invalid provider authentication challenge handle",
            )
            .with_platform(platform));
        }
        Ok(Self {
            platform,
            request,
            credential_mode,
            provider_transaction_id: Some(provider_transaction_id),
        })
    }

    #[must_use]
    pub const fn platform(&self) -> Platform {
        self.platform
    }
    #[must_use]
    pub const fn request(&self) -> &AuthChallengeRequest {
        &self.request
    }
    #[must_use]
    pub const fn credential_mode(&self) -> CredentialMode {
        self.credential_mode
    }
    #[must_use]
    pub fn provider_transaction_id(&self) -> Option<&str> {
        self.provider_transaction_id.as_deref()
    }
}

impl fmt::Debug for ProviderAuthChallenge {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProviderAuthChallenge")
            .field("platform", &self.platform)
            .field("method", &self.request.method)
            .field("backend", &self.request.backend)
            .field("credential_mode", &self.credential_mode)
            .field("stateful", &self.provider_transaction_id.is_some())
            .finish_non_exhaustive()
    }
}

/// The kind of answer a user must read from an image challenge.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthImageAnswerKind {
    Arithmetic,
    Chinese,
    Alphanumeric,
}

/// Public image instructions. Provider cookies and transaction handles remain private.
#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct AuthImageChallenge {
    pub image_data_url: String,
    pub answer_kind: AuthImageAnswerKind,
    pub remaining_attempts: u8,
    pub refresh_after_secs: u64,
}
impl fmt::Debug for AuthImageChallenge {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AuthImageChallenge")
            .field("answer_kind", &self.answer_kind)
            .field("remaining_attempts", &self.remaining_attempts)
            .field("refresh_after_secs", &self.refresh_after_secs)
            .finish_non_exhaustive()
    }
}

/// Human interaction on an official provider page. The caller must check both
/// message origin and the exact iframe window before forwarding its payload.
/// This is a pending challenge, never proof that an account is authenticated.
#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct AuthBrowserChallenge {
    pub verification_id: String,
    pub url: String,
    pub message_origin: String,
    pub message_type: String,
    pub response_field: String,
    pub remaining_attempts: u8,
}
impl fmt::Debug for AuthBrowserChallenge {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AuthBrowserChallenge")
            .field("remaining_attempts", &self.remaining_attempts)
            .finish_non_exhaustive()
    }
}

/// Nonterminal progress; no account profile or login credential can be attached.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum AuthChallengeStatus {
    Waiting,
    VerificationRequired { verification: AuthImageChallenge },
    BrowserVerificationRequired { verification: AuthBrowserChallenge },
    AccountSelectionRequired { accounts: Vec<AuthAccountChoice> },
}

/// Accounts offered by the upstream after it verifies the original SMS proof.
/// A choice is not a login result and contains no session credential.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AuthAccountChoice {
    pub user_id: String,
    pub nickname: Option<String>,
    pub avatar_url: Option<String>,
}

#[derive(Debug)]
pub enum AuthChallengeProgress {
    Pending(AuthChallengeStatus),
    Confirmed(ProviderAuthResult),
}

#[derive(Clone, Eq, PartialEq, Serialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum AuthChallengeAction {
    SubmitCode {
        code: String,
    },
    SubmitImage {
        answer: String,
    },
    RefreshImage,
    SelectAccount {
        user_id: String,
        code: String,
    },
    SubmitBrowser {
        verification_id: String,
        code: String,
        response: String,
    },
}
impl<'de> Deserialize<'de> for AuthChallengeAction {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        // Empty struct variants enforce unknown-field rejection for actions without inputs.
        #[derive(Deserialize)]
        #[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
        enum Wire {
            SubmitCode {
                code: String,
            },
            SubmitImage {
                answer: String,
            },
            RefreshImage {},
            SelectAccount {
                user_id: String,
                code: String,
            },
            SubmitBrowser {
                verification_id: String,
                code: String,
                response: String,
            },
        }
        Ok(match Wire::deserialize(d)? {
            Wire::SubmitCode { code } => Self::SubmitCode { code },
            Wire::SubmitImage { answer } => Self::SubmitImage { answer },
            Wire::RefreshImage {} => Self::RefreshImage,
            Wire::SelectAccount { user_id, code } => Self::SelectAccount { user_id, code },
            Wire::SubmitBrowser {
                verification_id,
                code,
                response,
            } => Self::SubmitBrowser {
                verification_id,
                code,
                response,
            },
        })
    }
}
impl fmt::Debug for AuthChallengeAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::SubmitCode { .. } => "SubmitCode { code: [redacted] }",
            Self::SubmitImage { .. } => "SubmitImage { answer: [redacted] }",
            Self::RefreshImage => "RefreshImage",
            Self::SelectAccount { .. } => "SelectAccount { fields: [redacted] }",
            Self::SubmitBrowser { .. } => "SubmitBrowser { fields: [redacted] }",
        })
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

/// Requests a challenge for an operation on an already-authenticated account.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AuthSecurityChallengeRequest {
    pub account: String,
    pub method: ChallengeMethod,
    pub country_code: Option<String>,
}

/// Confirms that a provider accepted one challenge-delivery request.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AuthChallengeDelivery {
    pub method: ChallengeMethod,
    pub sent: bool,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verification: Option<QrVerification>,
    pub state: AuthState,
    pub message: Option<String>,
    pub profile: Option<AccountProfile>,
    /// Present only for a confirmed caller-managed login result.
    #[serde(skip)]
    pub credential: Option<ProviderCredential>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QrVerificationMethod {
    Sms,
    UpSms,
}

/// Public instructions for continuing a QR transaction. Provider tickets remain private.
#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct QrVerification {
    pub methods: Vec<QrVerificationMethod>,
    pub masked_destination: Option<String>,
    pub resend_after_secs: Option<u64>,
    pub up_sms: Option<UpSmsInstructions>,
}

impl fmt::Debug for QrVerification {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("QrVerification")
            .field("methods", &self.methods)
            .field("resend_after_secs", &self.resend_after_secs)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct UpSmsInstructions {
    pub destination: String,
    pub message: String,
}

impl fmt::Debug for UpSmsInstructions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("UpSmsInstructions { destination: [redacted], message: [redacted] }")
    }
}

#[derive(Clone, Eq, PartialEq, Serialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum QrVerificationAction {
    SendSms,
    SubmitSms { code: String },
    VerifyUpSms,
}

impl<'de> Deserialize<'de> for QrVerificationAction {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        // Empty struct variants reject extra keys; Serde's internally tagged unit variants
        // accept them even when the enum specifies deny_unknown_fields.
        #[derive(Deserialize)]
        #[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
        enum WireAction {
            #[serde(rename = "send_sms")]
            Send {},
            #[serde(rename = "submit_sms")]
            Submit { code: String },
            #[serde(rename = "verify_up_sms")]
            VerifyUp {},
        }
        Ok(match WireAction::deserialize(deserializer)? {
            WireAction::Send {} => Self::SendSms,
            WireAction::Submit { code } => Self::SubmitSms { code },
            WireAction::VerifyUp {} => Self::VerifyUpSms,
        })
    }
}

impl fmt::Debug for QrVerificationAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::SendSms => "SendSms",
            Self::SubmitSms { .. } => "SubmitSms { code: [redacted] }",
            Self::VerifyUpSms => "VerifyUpSms",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credential_import_is_typed_and_redacted() {
        let credential: ImportedCredential =
            serde_json::from_str(r#"{"kind":"cookie","value":"sessionid_ss=private"}"#).unwrap();
        let request = CredentialImportRequest {
            account: "personal".to_owned(),
            credential,
        };
        assert!(!format!("{request:?}").contains("sessionid_ss=private"));
        for input in [
            r#"{"kind":"cookie","value":"a=b","url":"https://example.test"}"#,
            r#"{"kind":"cookie","value":"a=b","generation":"chosen"}"#,
            r#"{"kind":"cookie","value":null}"#,
            r#"{"kind":"unknown","value":"a=b"}"#,
        ] {
            assert!(
                serde_json::from_str::<ImportedCredential>(input).is_err(),
                "{input}"
            );
        }
    }

    #[test]
    fn provider_challenge_receipt_keeps_binding_but_redacts_private_material() {
        let request = AuthChallengeRequest {
            allow_account_creation: false,
            accept_platform_policies: false,
            account: "private-alias".into(),
            method: ChallengeMethod::Sms,
            backend: AuthChallengeBackend::Standard,
            principal: "private-phone".into(),
            country_code: Some("86".into()),
        };
        let receipt = ProviderAuthChallenge::stateful(
            Platform::Migu,
            request.clone(),
            CredentialMode::Both,
            "private-handle".into(),
        )
        .unwrap();
        assert_eq!(receipt.platform(), Platform::Migu);
        assert_eq!(receipt.request(), &request);
        assert_eq!(receipt.credential_mode(), CredentialMode::Both);
        assert_eq!(receipt.provider_transaction_id(), Some("private-handle"));
        let debug = format!("{receipt:?}");
        for secret in ["private-alias", "private-phone", "private-handle"] {
            assert!(!debug.contains(secret));
        }
        for bad in [
            String::new(),
            "x y".into(),
            "x\ny".into(),
            "é".into(),
            "x".repeat(1025),
        ] {
            let error = ProviderAuthChallenge::stateful(
                Platform::Migu,
                request.clone(),
                CredentialMode::Both,
                bad,
            )
            .unwrap_err();
            assert_eq!(error.code, crate::ErrorCode::InvalidRequest);
        }
    }

    #[test]
    fn qr_verification_actions_are_strict_and_redact_codes() {
        let action: QrVerificationAction =
            serde_json::from_str(r#"{"action":"submit_sms","code":"864209"}"#).unwrap();
        assert!(!format!("{action:?}").contains("864209"));
        for input in [
            r#"{"action":"send_sms","account":"other"}"#,
            r#"{"action":"submit_sms"}"#,
            r#"{"action":"send_sms","token":"upstream-token"}"#,
            r#"{"action":"verify_up_sms","token":"upstream-token"}"#,
            r#"{"action":"send_sms","code":"864209"}"#,
            r#"{"action":"submit_sms","code":"864209","account":"other"}"#,
            r#"{"action":"unknown"}"#,
        ] {
            assert!(
                serde_json::from_str::<QrVerificationAction>(input).is_err(),
                "{input}"
            );
        }
        for action in [
            QrVerificationAction::SendSms,
            QrVerificationAction::VerifyUpSms,
            action,
        ] {
            let json = serde_json::to_string(&action).unwrap();
            assert_eq!(
                serde_json::from_str::<QrVerificationAction>(&json).unwrap(),
                action
            );
        }
        assert!(!AuthState::VerificationRequired.is_terminal());
        assert_eq!(
            serde_json::to_string(&AuthState::VerificationRequired).unwrap(),
            "\"verification_required\""
        );
    }

    #[test]
    fn sensitive_auth_requests_are_redacted_in_debug_output() {
        let password = PasswordLoginRequest {
            backend: Default::default(),
            account: "default".to_owned(),
            principal_type: PrincipalType::Email,
            principal: "secret@example.test".to_owned(),
            password: "password-secret".to_owned(),
            password_format: PasswordFormat::Plain,
            country_code: None,
            secure_captcha: Some("secure-captcha-secret".to_owned()),
        };
        let challenge = AuthChallengeRequest {
            allow_account_creation: false,
            accept_platform_policies: false,
            account: "default".to_owned(),
            method: ChallengeMethod::Sms,
            backend: AuthChallengeBackend::Standard,
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
    fn challenge_backend_defaults_to_standard_and_round_trips_middle() {
        let standard: AuthChallengeRequest = serde_json::from_value(serde_json::json!({
            "account": "default",
            "method": "sms",
            "principal": "13800138000",
            "country_code": "86"
        }))
        .expect("default challenge backend");
        assert_eq!(standard.backend, AuthChallengeBackend::Standard);

        let middle: AuthChallengeRequest = serde_json::from_value(serde_json::json!({
            "account": "default",
            "method": "sms",
            "backend": "middle",
            "principal": "13800138000",
            "country_code": "86"
        }))
        .expect("middle challenge backend");
        assert_eq!(middle.backend, AuthChallengeBackend::Middle);
    }

    #[test]
    fn account_creation_permission_is_explicit_and_part_of_the_original_challenge_binding() {
        let input =
            serde_json::json!({"account":"default","method":"sms","principal":"private-phone"});
        let original: AuthChallengeRequest = serde_json::from_value(input.clone()).unwrap();
        assert!(!original.allow_account_creation);
        original
            .reject_account_creation_option(Platform::Migu)
            .unwrap();
        let mut permitted = input.clone();
        permitted["allow_account_creation"] = serde_json::json!(true);
        let permitted: AuthChallengeRequest = serde_json::from_value(permitted).unwrap();
        assert!(permitted.allow_account_creation);
        assert!(
            permitted
                .reject_account_creation_option(Platform::Migu)
                .is_err()
        );
        let receipt = ProviderAuthChallenge::stateful(
            Platform::Kuwo,
            permitted.clone(),
            CredentialMode::Both,
            "private-handle".into(),
        )
        .unwrap();
        assert_ne!(receipt.request(), &original);
        assert_eq!(
            serde_json::from_value::<AuthChallengeRequest>(
                serde_json::to_value(&permitted).unwrap()
            )
            .unwrap(),
            permitted
        );
        for value in [
            serde_json::json!("true"),
            serde_json::json!(1),
            serde_json::Value::Null,
        ] {
            let mut bad = input.clone();
            bad["allow_account_creation"] = value;
            assert!(serde_json::from_value::<AuthChallengeRequest>(bad).is_err());
        }
        for secret in ["private-phone", "private-handle"] {
            assert!(!format!("{receipt:?} {permitted:?}").contains(secret));
        }
    }

    #[test]
    fn account_security_challenges_do_not_contain_a_caller_supplied_principal() {
        let request = AuthSecurityChallengeRequest {
            account: "personal".to_owned(),
            method: ChallengeMethod::Sms,
            country_code: Some("86".to_owned()),
        };
        let value = serde_json::to_value(&request).expect("security challenge request");
        assert_eq!(value["account"], "personal");
        assert_eq!(value["method"], "sms");
        assert_eq!(value["country_code"], "86");
        assert!(value.get("principal").is_none());
        assert!(value.get("phone").is_none());
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
    #[test]
    fn challenge_actions_reject_extra_duplicate_or_mixed_inputs_and_redact_answers() {
        for action in [
            AuthChallengeAction::SubmitCode {
                code: "private-code".into(),
            },
            AuthChallengeAction::SubmitImage {
                answer: "private-answer".into(),
            },
            AuthChallengeAction::RefreshImage,
            AuthChallengeAction::SubmitBrowser {
                verification_id: "private-id".into(),
                code: "private-code".into(),
                response: "private-response".into(),
            },
            AuthChallengeAction::SelectAccount {
                user_id: "private-user".into(),
                code: "private-code".into(),
            },
        ] {
            let wire = serde_json::to_string(&action).unwrap();
            assert_eq!(
                serde_json::from_str::<AuthChallengeAction>(&wire).unwrap(),
                action
            );
            assert!(!format!("{action:?}").contains("private"));
        }
        for wire in [
            r#"{"action":"refresh_image","answer":"x"}"#,
            r#"{"action":"refresh_image","account":"A"}"#,
            r#"{"action":"submit_image","answer":"42","code":"123456"}"#,
            r#"{"action":"submit_image","answer":"42","answer":"12"}"#,
            r#"{"action":"submit_code","code":"123456","principal":"other"}"#,
            r#"{"action":"submit_code","code":123456}"#,
            r#"{"action":"submit_image"}"#,
            r#"{"action":"select_account","user_id":"1","code":"123456","account":"other"}"#,
            r#"{"action":"select_account","user_id":"1","user_id":"2","code":"123456"}"#,
            r#"{"action":"select_account","user_id":"1"}"#,
            r#"{"action":"submit_browser","verification_id":"a","code":"123456","response":"{}","user_id":"2"}"#,
            r#"{"action":"submit_browser","verification_id":"a","code":"123456","response":"{}","response":"{}"}"#,
            r#"{"action":"submit_browser","verification_id":"a","code":"123456"}"#,
        ] {
            assert!(
                serde_json::from_str::<AuthChallengeAction>(wire).is_err(),
                "{wire}"
            );
        }
    }

    #[test]
    fn intermediate_challenge_progress_has_no_credentials_and_image_debug_is_redacted() {
        let verification = AuthImageChallenge {
            image_data_url: "private-image".into(),
            answer_kind: AuthImageAnswerKind::Arithmetic,
            remaining_attempts: 5,
            refresh_after_secs: 2,
        };
        assert!(!format!("{verification:?}").contains("private-image"));
        let wire = serde_json::to_value(AuthChallengeStatus::VerificationRequired { verification })
            .unwrap();
        assert_eq!(wire["state"], "verification_required");
        assert!(wire.get("profile").is_none() && wire.get("credential").is_none());
    }

    #[test]
    fn browser_challenge_is_nonterminal_and_debug_hides_binding_and_url() {
        let verification = AuthBrowserChallenge {
            verification_id: "private-id".into(),
            url: "https://example.test/private-event".into(),
            message_origin: "https://example.test".into(),
            message_type: "callback".into(),
            response_field: "dataJson".into(),
            remaining_attempts: 4,
        };
        let status = AuthChallengeStatus::BrowserVerificationRequired { verification };
        assert!(!format!("{status:?}").contains("private"));
        let wire = serde_json::to_value(&status).unwrap();
        assert_eq!(wire["state"], "browser_verification_required");
        assert!(wire.get("profile").is_none() && wire.get("credential").is_none());
        assert_eq!(
            serde_json::from_value::<AuthChallengeStatus>(wire).unwrap(),
            status
        );
    }
}
