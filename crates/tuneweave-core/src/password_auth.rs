//! Stateful password verification without retaining a password in its receipt.
use crate::{
    AuthAccountChoice, AuthBrowserChallenge, AuthImageChallenge, CredentialMode, PasswordFormat,
    PasswordLoginRequest, Platform, PrincipalType, ProviderAuthResult, Result, TuneWeaveError,
};
use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Clone, Eq, PartialEq)]
pub struct PasswordLoginIdentity {
    pub backend: crate::PasswordLoginBackend,
    pub account: String,
    pub principal_type: PrincipalType,
    pub principal: String,
    pub password_format: PasswordFormat,
    pub country_code: Option<String>,
}
impl From<&PasswordLoginRequest> for PasswordLoginIdentity {
    fn from(request: &PasswordLoginRequest) -> Self {
        Self {
            backend: request.backend,
            account: request.account.clone(),
            principal_type: request.principal_type,
            principal: request.principal.clone(),
            password_format: request.password_format,
            country_code: request.country_code.clone(),
        }
    }
}
impl fmt::Debug for PasswordLoginIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PasswordLoginIdentity")
            .field("backend", &self.backend)
            .field("principal_type", &self.principal_type)
            .field("password_format", &self.password_format)
            .finish_non_exhaustive()
    }
}

/// Immutable ownership of one password login. Neither the password nor provider cookies
/// belong in this receipt; it is only valid while the provider's transaction is live.
#[derive(Clone, Eq, PartialEq)]
pub struct ProviderPasswordChallenge {
    platform: Platform,
    identity: PasswordLoginIdentity,
    credential_mode: CredentialMode,
    provider_transaction_id: String,
}
impl ProviderPasswordChallenge {
    pub fn new(
        platform: Platform,
        identity: PasswordLoginIdentity,
        credential_mode: CredentialMode,
        provider_transaction_id: String,
    ) -> Result<Self> {
        if provider_transaction_id.is_empty()
            || provider_transaction_id.len() > 1024
            || !provider_transaction_id
                .bytes()
                .all(|v| v.is_ascii_graphic())
        {
            return Err(
                TuneWeaveError::invalid_request("Invalid password challenge handle")
                    .with_platform(platform),
            );
        }
        Ok(Self {
            platform,
            identity,
            credential_mode,
            provider_transaction_id,
        })
    }
    #[must_use]
    pub const fn platform(&self) -> Platform {
        self.platform
    }
    #[must_use]
    pub const fn identity(&self) -> &PasswordLoginIdentity {
        &self.identity
    }
    #[must_use]
    pub const fn credential_mode(&self) -> CredentialMode {
        self.credential_mode
    }
    #[must_use]
    pub fn provider_transaction_id(&self) -> &str {
        &self.provider_transaction_id
    }
}
impl fmt::Debug for ProviderPasswordChallenge {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProviderPasswordChallenge")
            .field("platform", &self.platform)
            .field("credential_mode", &self.credential_mode)
            .finish_non_exhaustive()
    }
}

/// The client opens the official provider page in an isolated WebView and
/// implements only this protocol's documented bridge commands. No password or
/// application credential may be exposed to the page.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PasswordBrowserProtocol {
    /// external.superCall: 122 returns version, 124 returns mid, and 252
    /// delivers the close/ticket JSON. Accept only this challenge's page/window.
    KugouNativeBridge,
}

/// Browser context supplied by a client for a human-operated login challenge.
/// This is request context, not proof that the browser is trusted.
#[derive(Clone, Eq, PartialEq)]
pub struct PasswordLoginContext {
    pub user_agent: String,
}

impl PasswordLoginContext {
    pub fn new(platform: Platform, user_agent: String) -> Result<Self> {
        if user_agent.is_empty()
            || user_agent.len() > 512
            || !user_agent.bytes().all(|byte| (b' '..=b'~').contains(&byte))
        {
            return Err(TuneWeaveError::invalid_request(
                "Invalid password login browser user agent",
            )
            .with_platform(platform));
        }
        Ok(Self { user_agent })
    }
}

impl fmt::Debug for PasswordLoginContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PasswordLoginContext")
            .field("has_user_agent", &!self.user_agent.is_empty())
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PasswordSliderProtocol {
    MiguPassportV1,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PasswordSliderFingerprintAlgorithm {
    Fingerprint2Murmur128,
}

/// Instructions for preparing a real browser fingerprint before requesting slider images.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PasswordSliderPreparation {
    pub protocol: PasswordSliderProtocol,
    pub fingerprint_algorithm: PasswordSliderFingerprintAlgorithm,
    pub fingerprint_library_version: String,
    pub exclude_canvas: bool,
    pub exclude_webgl: bool,
}

/// Two passive challenge images and a server-bound identifier for one image generation.
#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PasswordSliderChallenge {
    pub verification_id: String,
    pub background_image_data_url: String,
    pub piece_image_data_url: String,
    pub picture_width: u32,
    pub picture_height: u32,
    pub remaining_attempts: u8,
    pub refresh_after_secs: u64,
}

impl fmt::Debug for PasswordSliderChallenge {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PasswordSliderChallenge")
            .field("picture_width", &self.picture_width)
            .field("picture_height", &self.picture_height)
            .field("remaining_attempts", &self.remaining_attempts)
            .field("refresh_after_secs", &self.refresh_after_secs)
            .finish_non_exhaustive()
    }
}

/// One real pointer event sampled while a person drags the slider piece.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PasswordSliderPointerSample {
    pub at_ms: u64,
    pub x: i32,
    pub y: i32,
}

/// Browser event times and page coordinates from one human drag. Provider-owned
/// image/session fields are deliberately absent and must be injected by the provider.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PasswordSliderGesture {
    pub load_time_ms: u64,
    pub started_at_ms: u64,
    pub ended_at_ms: u64,
    pub start_x: i32,
    pub start_y: i32,
    pub end_x: i32,
    pub end_y: i32,
    pub points: Vec<PasswordSliderPointerSample>,
}

impl PasswordSliderGesture {
    pub fn validate(&self) -> Result<()> {
        let invalid = || {
            TuneWeaveError::invalid_request("Invalid human slider gesture")
                .with_platform(Platform::Migu)
        };
        if self.load_time_ms > self.started_at_ms
            || self.started_at_ms >= self.ended_at_ms
            || self.points.is_empty()
            || self.points.len() > 4096
            || self.start_x == self.end_x
        {
            return Err(invalid());
        }
        let mut previous = self.started_at_ms;
        for point in &self.points {
            if point.at_ms < previous || point.at_ms > self.ended_at_ms {
                return Err(invalid());
            }
            previous = point.at_ms;
        }
        Ok(())
    }
}

#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "method", rename_all = "snake_case")]
pub enum PasswordVerification {
    Image {
        image: AuthImageChallenge,
    },
    Browser {
        protocol: PasswordBrowserProtocol,
        verification_id: String,
        url: String,
        device_id: String,
        client_version: u32,
        remaining_attempts: u8,
    },
    Sms {
        masked_destination: String,
        remaining_attempts: u8,
        resend_after_secs: u64,
    },
    /// Automated voice call verification uses its own delivery and validation protocol.
    Voice {
        masked_destination: String,
        remaining_attempts: u8,
        resend_after_secs: u64,
    },
    /// Human browser verification while completing a secondary SMS login.
    /// Continue with a fresh SMS code, never the original account password.
    SmsBrowser {
        verification: AuthBrowserChallenge,
    },
    /// Candidate accounts returned only after the upstream verifies the SMS proof.
    /// The selection itself is not an authenticated session.
    AccountSelection {
        accounts: Vec<AuthAccountChoice>,
        remaining_attempts: u8,
    },
    /// The client must fingerprint its real browser before the provider can load images.
    SliderPreparation {
        preparation: PasswordSliderPreparation,
    },
    /// A specific image pair is ready for human interaction.
    Slider {
        challenge: PasswordSliderChallenge,
    },
}
impl fmt::Debug for PasswordVerification {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Image { image } => f.debug_struct("Image").field("image", image).finish(),
            Self::Browser {
                protocol,
                remaining_attempts,
                ..
            } => f
                .debug_struct("Browser")
                .field("protocol", protocol)
                .field("remaining_attempts", remaining_attempts)
                .finish_non_exhaustive(),
            Self::Sms {
                remaining_attempts,
                resend_after_secs,
                ..
            } => f
                .debug_struct("Sms")
                .field("remaining_attempts", remaining_attempts)
                .field("resend_after_secs", resend_after_secs)
                .finish_non_exhaustive(),
            Self::Voice {
                remaining_attempts,
                resend_after_secs,
                ..
            } => f
                .debug_struct("Voice")
                .field("remaining_attempts", remaining_attempts)
                .field("resend_after_secs", resend_after_secs)
                .finish_non_exhaustive(),
            Self::SmsBrowser { verification } => f
                .debug_struct("SmsBrowser")
                .field("verification", verification)
                .finish(),
            Self::AccountSelection {
                accounts,
                remaining_attempts,
            } => f
                .debug_struct("AccountSelection")
                .field("account_count", &accounts.len())
                .field("remaining_attempts", remaining_attempts)
                .finish_non_exhaustive(),
            Self::SliderPreparation { preparation } => f
                .debug_struct("SliderPreparation")
                .field("protocol", &preparation.protocol)
                .finish_non_exhaustive(),
            Self::Slider { challenge } => f
                .debug_struct("Slider")
                .field("challenge", challenge)
                .finish(),
        }
    }
}

#[derive(Clone, Debug)]
pub enum PasswordLoginProgress {
    Pending {
        challenge: ProviderPasswordChallenge,
        verification: PasswordVerification,
    },
    Confirmed(ProviderAuthResult),
}

#[derive(Clone, Eq, PartialEq, Serialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum PasswordChallengeAction {
    SubmitImage {
        answer: String,
        password: String,
    },
    SubmitBrowser {
        verification_id: String,
        response: String,
        password: String,
    },
    RefreshImage,
    SubmitSms {
        code: String,
    },
    SubmitVoice {
        code: String,
    },
    SubmitSmsBrowser {
        verification_id: String,
        code: String,
        response: String,
    },
    ResendSms,
    ResendVoice,
    SelectAccount {
        user_id: String,
        code: String,
    },
    PrepareSlider {
        fingerprint: String,
    },
    RefreshSlider {
        verification_id: String,
    },
    SubmitSlider {
        verification_id: String,
        password: String,
        gesture: PasswordSliderGesture,
    },
}
impl<'de> Deserialize<'de> for PasswordChallengeAction {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
        enum Wire {
            SubmitImage {
                answer: String,
                password: String,
            },
            SubmitBrowser {
                verification_id: String,
                response: String,
                password: String,
            },
            RefreshImage {},
            SubmitSms {
                code: String,
            },
            SubmitVoice {
                code: String,
            },
            SubmitSmsBrowser {
                verification_id: String,
                code: String,
                response: String,
            },
            ResendSms {},
            ResendVoice {},
            SelectAccount {
                user_id: String,
                code: String,
            },
            PrepareSlider {
                fingerprint: String,
            },
            RefreshSlider {
                verification_id: String,
            },
            SubmitSlider {
                verification_id: String,
                password: String,
                gesture: PasswordSliderGesture,
            },
        }
        Ok(match Wire::deserialize(d)? {
            Wire::SubmitImage { answer, password } => Self::SubmitImage { answer, password },
            Wire::SubmitBrowser {
                verification_id,
                response,
                password,
            } => Self::SubmitBrowser {
                verification_id,
                response,
                password,
            },
            Wire::RefreshImage {} => Self::RefreshImage,
            Wire::SubmitSms { code } => Self::SubmitSms { code },
            Wire::SubmitVoice { code } => Self::SubmitVoice { code },
            Wire::SubmitSmsBrowser {
                verification_id,
                code,
                response,
            } => Self::SubmitSmsBrowser {
                verification_id,
                code,
                response,
            },
            Wire::ResendSms {} => Self::ResendSms,
            Wire::ResendVoice {} => Self::ResendVoice,
            Wire::SelectAccount { user_id, code } => Self::SelectAccount { user_id, code },
            Wire::PrepareSlider { fingerprint } => Self::PrepareSlider { fingerprint },
            Wire::RefreshSlider { verification_id } => Self::RefreshSlider { verification_id },
            Wire::SubmitSlider {
                verification_id,
                password,
                gesture,
            } => Self::SubmitSlider {
                verification_id,
                password,
                gesture,
            },
        })
    }
}
impl fmt::Debug for PasswordChallengeAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::SubmitImage { .. } => "SubmitImage { answer: [redacted], password: [redacted] }",
            Self::SubmitBrowser { .. } => "SubmitBrowser { verification_id: [redacted], response: [redacted], password: [redacted] }",
            Self::RefreshImage => "RefreshImage",
            Self::SubmitSms { .. } => "SubmitSms { code: [redacted] }",
            Self::SubmitVoice { .. } => "SubmitVoice { code: [redacted] }",
            Self::SubmitSmsBrowser { .. } => "SubmitSmsBrowser { verification_id: [redacted], code: [redacted], response: [redacted] }",
            Self::ResendSms => "ResendSms",
            Self::ResendVoice => "ResendVoice",
            Self::SelectAccount { .. } => "SelectAccount { user_id: [redacted], code: [redacted] }",
            Self::PrepareSlider { .. } => "PrepareSlider { fingerprint: [redacted] }",
            Self::RefreshSlider { .. } => "RefreshSlider { verification_id: [redacted] }",
            Self::SubmitSlider { .. } => {
                "SubmitSlider { verification_id: [redacted], password: [redacted], gesture: [redacted] }"
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn password_backend_defaults_and_remains_bound_to_the_challenge_identity() {
        use crate::PasswordLoginBackend;
        let wire = serde_json::json!({"account":"default","principal_type":"username","principal":"synthetic","password":"secret"});
        let mut request: PasswordLoginRequest = serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(request.backend, PasswordLoginBackend::Default);
        let old_identity = PasswordLoginIdentity::from(&request);
        for backend in [PasswordLoginBackend::Web, PasswordLoginBackend::Native] {
            request.backend = backend;
            assert!(request.require_backend(Platform::Kugou, backend).is_ok());
            assert!(
                request
                    .require_backend(Platform::Migu, PasswordLoginBackend::Default)
                    .is_err()
            );
            let encoded = serde_json::to_value(&request).unwrap();
            assert_eq!(
                serde_json::from_value::<PasswordLoginRequest>(encoded).unwrap(),
                request
            );
            assert_ne!(PasswordLoginIdentity::from(&request), old_identity);
        }
        for backend in [serde_json::json!("middle"), serde_json::Value::Null] {
            let mut invalid = wire.clone();
            invalid["backend"] = backend;
            assert!(serde_json::from_value::<PasswordLoginRequest>(invalid).is_err());
        }
    }
    #[test]
    fn password_actions_reject_cross_flow_duplicate_and_extra_parameters() {
        for wire in [
            r#"{"action":"refresh_image","password":"secret"}"#,
            r#"{"action":"resend_sms","account":"B"}"#,
            r#"{"action":"resend_voice","account":"B"}"#,
            r#"{"action":"submit_image","answer":"42"}"#,
            r#"{"action":"submit_image","answer":"42","password":"a","password":"b"}"#,
            r#"{"action":"submit_sms","code":"1234","principal":"other"}"#,
            r#"{"action":"submit_sms","code":1234}"#,
            r#"{"action":"submit_voice","code":1234}"#,
            r#"{"action":"submit_voice","code":"1234","password":"secret"}"#,
            r#"{"action":"submit_code","code":"1234"}"#,
            r#"{"action":"submit_browser","verification_id":"id","response":"{}"}"#,
            r#"{"action":"submit_browser","verification_id":"id","response":"{}","password":"p","backend":"web"}"#,
            r#"{"action":"select_account","user_id":"123"}"#,
            r#"{"action":"select_account","user_id":"123","code":"123456","account":"B"}"#,
            r#"{"action":"select_account","user_id":"123","code":"123456","password":"p"}"#,
            r#"{"action":"select_account","user_id":"123","code":"123456","code":"654321"}"#,
            r#"{"action":"submit_sms_browser","verification_id":"id","response":"{}"}"#,
            r#"{"action":"submit_sms_browser","verification_id":"id","code":"123456","response":{}}"#,
            r#"{"action":"submit_sms_browser","verification_id":"id","code":"123456","response":"{}","password":"p"}"#,
            r#"{"action":"submit_sms_browser","verification_id":"id","code":"123456","response":"{}","backend":"native"}"#,
            r#"{"action":"submit_sms_browser","verification_id":"id","code":"123456","response":"{}","code":"654321"}"#,
            r#"{"action":"prepare_slider","fingerprint":"abc","password":"p"}"#,
            r#"{"action":"refresh_slider","verification_id":"id","fingerprint":"fp"}"#,
            r#"{"action":"submit_slider","verification_id":"id","password":"p","gesture":{},"account":"other"}"#,
        ] {
            assert!(
                serde_json::from_str::<PasswordChallengeAction>(wire).is_err(),
                "{wire}"
            );
        }
        for action in [
            PasswordChallengeAction::SubmitImage {
                answer: "private-answer".into(),
                password: "private-password".into(),
            },
            PasswordChallengeAction::RefreshImage,
            PasswordChallengeAction::SubmitBrowser {
                verification_id: "private-id".into(),
                response: "private-response".into(),
                password: "private-password".into(),
            },
            PasswordChallengeAction::SubmitSms {
                code: "private-code".into(),
            },
            PasswordChallengeAction::SubmitVoice {
                code: "private-code".into(),
            },
            PasswordChallengeAction::SubmitSmsBrowser {
                verification_id: "private-id".into(),
                code: "private-code".into(),
                response: "private-response".into(),
            },
            PasswordChallengeAction::ResendSms,
            PasswordChallengeAction::ResendVoice,
            PasswordChallengeAction::SelectAccount {
                user_id: "private-user".into(),
                code: "private-code".into(),
            },
            PasswordChallengeAction::PrepareSlider {
                fingerprint: "private-fingerprint".into(),
            },
            PasswordChallengeAction::RefreshSlider {
                verification_id: "private-verification".into(),
            },
            PasswordChallengeAction::SubmitSlider {
                verification_id: "private-verification".into(),
                password: "private-password".into(),
                gesture: PasswordSliderGesture {
                    load_time_ms: 100,
                    started_at_ms: 110,
                    ended_at_ms: 150,
                    start_x: 1,
                    start_y: 2,
                    end_x: 20,
                    end_y: 2,
                    points: vec![PasswordSliderPointerSample {
                        at_ms: 130,
                        x: 10,
                        y: 2,
                    }],
                },
            },
        ] {
            assert!(!format!("{action:?}").contains("private"));
            assert_eq!(
                serde_json::from_str::<PasswordChallengeAction>(
                    &serde_json::to_string(&action).unwrap()
                )
                .unwrap(),
                action
            );
        }
    }
    #[test]
    fn password_voice_verification_is_distinct_and_redacts_the_destination() {
        let verification = PasswordVerification::Voice {
            masked_destination: "+86 138****0000".into(),
            remaining_attempts: 5,
            resend_after_secs: 60,
        };
        assert!(!format!("{verification:?}").contains("138"));
        let wire = serde_json::to_value(&verification).unwrap();
        assert_eq!(wire["method"], "voice");
        assert_eq!(wire["masked_destination"], "+86 138****0000");
        assert_eq!(wire["remaining_attempts"], 5);
        assert_eq!(wire["resend_after_secs"], 60);
        assert_eq!(
            serde_json::from_value::<PasswordVerification>(wire).unwrap(),
            verification
        );
    }
    #[test]
    fn password_sms_browser_keeps_the_callback_contract_and_redacts_its_material() {
        let verification = PasswordVerification::SmsBrowser {
            verification: AuthBrowserChallenge {
                verification_id: "private-id".into(),
                url: "https://example.invalid/private-event".into(),
                message_origin: "https://example.invalid".into(),
                message_type: "kgVerifyCallbackData".into(),
                response_field: "dataJson".into(),
                remaining_attempts: 3,
            },
        };
        assert!(!format!("{verification:?}").contains("private"));
        let wire = serde_json::to_value(&verification).unwrap();
        assert_eq!(wire["method"], "sms_browser");
        assert_eq!(wire["verification"]["verification_id"], "private-id");
        assert_eq!(wire["verification"]["message_type"], "kgVerifyCallbackData");
        assert_eq!(wire["verification"]["response_field"], "dataJson");
        assert_eq!(
            serde_json::from_value::<PasswordVerification>(wire).unwrap(),
            verification
        );
    }
    #[test]
    fn password_account_selection_is_nonterminal_and_redacts_candidate_details() {
        let verification = PasswordVerification::AccountSelection {
            accounts: vec![AuthAccountChoice {
                user_id: "private-user".into(),
                nickname: Some("private-nickname".into()),
                avatar_url: Some("https://example.invalid/private-avatar".into()),
            }],
            remaining_attempts: 4,
        };
        assert!(!format!("{verification:?}").contains("private"));
        let wire = serde_json::to_value(&verification).unwrap();
        assert_eq!(wire["method"], "account_selection");
        assert_eq!(wire["accounts"][0]["user_id"], "private-user");
        assert_eq!(
            serde_json::from_value::<PasswordVerification>(wire).unwrap(),
            verification
        );
    }

    #[test]
    fn migu_slider_contract_is_typed_bound_and_redacts_private_material() {
        let context =
            PasswordLoginContext::new(Platform::Migu, "Mozilla/5.0 (Example Browser)".into())
                .unwrap();
        assert!(!format!("{context:?}").contains("Mozilla"));
        for invalid in [
            String::new(),
            "browser\nInjected: yes".into(),
            "x".repeat(513),
        ] {
            assert!(PasswordLoginContext::new(Platform::Migu, invalid).is_err());
        }

        let preparation = PasswordVerification::SliderPreparation {
            preparation: PasswordSliderPreparation {
                protocol: PasswordSliderProtocol::MiguPassportV1,
                fingerprint_algorithm: PasswordSliderFingerprintAlgorithm::Fingerprint2Murmur128,
                fingerprint_library_version: "1.5.1".into(),
                exclude_canvas: true,
                exclude_webgl: true,
            },
        };
        let preparation_wire = serde_json::to_value(&preparation).unwrap();
        assert_eq!(preparation_wire["method"], "slider_preparation");
        assert_eq!(
            serde_json::from_value::<PasswordVerification>(preparation_wire).unwrap(),
            preparation
        );

        let challenge = PasswordSliderChallenge {
            verification_id: "private-verification-id".into(),
            background_image_data_url: "data:image/png;base64,private-background".into(),
            piece_image_data_url: "data:image/png;base64,private-piece".into(),
            picture_width: 320,
            picture_height: 160,
            remaining_attempts: 5,
            refresh_after_secs: 2,
        };
        assert!(!format!("{challenge:?}").contains("private"));
        let verification = PasswordVerification::Slider {
            challenge: challenge.clone(),
        };
        let wire = serde_json::to_value(&verification).unwrap();
        assert_eq!(wire["method"], "slider");
        assert_eq!(
            wire["challenge"]["verification_id"],
            "private-verification-id"
        );
        assert_eq!(
            serde_json::from_value::<PasswordVerification>(wire).unwrap(),
            verification
        );

        let gesture = PasswordSliderGesture {
            load_time_ms: 1_000,
            started_at_ms: 1_050,
            ended_at_ms: 1_320,
            start_x: 20,
            start_y: 25,
            end_x: 245,
            end_y: 27,
            points: vec![
                PasswordSliderPointerSample {
                    at_ms: 1_100,
                    x: 80,
                    y: 26,
                },
                PasswordSliderPointerSample {
                    at_ms: 1_220,
                    x: 190,
                    y: 27,
                },
            ],
        };
        assert!(gesture.validate().is_ok());
        let mut invalid = gesture.clone();
        invalid.points[1].at_ms = 1_099;
        assert!(invalid.validate().is_err());
        invalid = gesture.clone();
        invalid.ended_at_ms = invalid.started_at_ms;
        assert!(invalid.validate().is_err());

        let submit: PasswordChallengeAction = serde_json::from_value(serde_json::json!({
            "action":"submit_slider",
            "verification_id":"private-verification-id",
            "password":"private-password",
            "gesture":gesture
        }))
        .unwrap();
        assert!(!format!("{submit:?}").contains("private"));
        assert_eq!(
            serde_json::from_value::<PasswordChallengeAction>(
                serde_json::to_value(&submit).unwrap()
            )
            .unwrap(),
            submit
        );
    }
    #[test]
    fn password_receipt_contains_no_password_and_debug_hides_binding_material() {
        let request = PasswordLoginRequest {
            backend: Default::default(),
            account: "private-account".into(),
            principal_type: PrincipalType::Username,
            principal: "private-principal".into(),
            password: "private-password".into(),
            password_format: PasswordFormat::Plain,
            country_code: None,
            secure_captcha: None,
        };
        let receipt = ProviderPasswordChallenge::new(
            Platform::Migu,
            PasswordLoginIdentity::from(&request),
            CredentialMode::Both,
            "private-ticket".into(),
        )
        .unwrap();
        assert!(!format!("{receipt:?} {:?}", receipt.identity()).contains("private"));
        assert_eq!(receipt.identity().principal, request.principal);
        assert_eq!(receipt.credential_mode(), CredentialMode::Both);
        for handle in [String::new(), "a\n".into(), "a".repeat(1025)] {
            assert!(
                ProviderPasswordChallenge::new(
                    Platform::Migu,
                    receipt.identity().clone(),
                    CredentialMode::Client,
                    handle
                )
                .is_err()
            );
        }
    }
}
