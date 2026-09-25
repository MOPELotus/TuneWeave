use super::*;
use base64::{Engine, engine::general_purpose::STANDARD};
use tuneweave_core::AuthImageAnswerKind;

#[derive(Clone, Copy)]
pub(crate) enum ImageScene {
    Sms,
    Password,
}
impl ImageScene {
    fn value(self) -> &'static str {
        match self {
            Self::Sms => "2",
            Self::Password => "1",
        }
    }
}

#[derive(Clone)]
pub(crate) struct PassportImage {
    pub(crate) data_url: String,
    pub(crate) kind: AuthImageAnswerKind,
}
impl PassportImage {
    fn parse(value: serde_json::Value) -> Result<Self> {
        let invalid = || error(ErrorCode::UpstreamError, "Invalid Migu image challenge");
        let result = value.get("result").ok_or_else(invalid)?;
        let kind = match result.get("graphtype") {
            Some(v) if v.as_str() == Some("0") || v.as_u64() == Some(0) => {
                AuthImageAnswerKind::Arithmetic
            }
            Some(v) if v.as_str() == Some("1") || v.as_u64() == Some(1) => {
                AuthImageAnswerKind::Chinese
            }
            _ => return Err(invalid()),
        };
        let data_url = result
            .get("captchaurl")
            .and_then(serde_json::Value::as_str)
            .filter(|v| v.len() <= 44 * 1024)
            .ok_or_else(invalid)?;
        // The observed passport protocol returns inline JPEG. Never fetch an upstream URL
        // or expose an active MIME type as a challenge image.
        let encoded = data_url
            .strip_prefix("data:image/jpeg;base64,")
            .ok_or_else(invalid)?;
        let bytes = STANDARD.decode(encoded).map_err(|_| invalid())?;
        if bytes.len() > 32 * 1024 || !jpeg_envelope(&bytes) {
            return Err(invalid());
        }
        Ok(Self {
            data_url: data_url.into(),
            kind,
        })
    }

    pub(crate) fn validate_answer(&self, answer: &str) -> Result<()> {
        let valid = match self.kind {
            AuthImageAnswerKind::Alphanumeric => false,
            AuthImageAnswerKind::Arithmetic => {
                matches!(answer.len(), 1 | 2)
                    && answer.bytes().all(|v| v.is_ascii_digit())
                    && (answer.len() == 1 || !answer.starts_with('0'))
            }
            AuthImageAnswerKind::Chinese => {
                answer.len() <= 12
                    && (2..=4).contains(&answer.chars().count())
                    && answer
                        .chars()
                        .all(|v| ('\u{4e00}'..='\u{9fa5}').contains(&v))
            }
        };
        if valid {
            Ok(())
        } else {
            Err(error(
                ErrorCode::InvalidRequest,
                "Invalid answer format for Migu image challenge",
            ))
        }
    }
}

/// Check the JPEG container and frame bounds without decoding or transforming its pixels.
fn jpeg_envelope(bytes: &[u8]) -> bool {
    if !bytes.starts_with(&[0xff, 0xd8]) || !bytes.ends_with(&[0xff, 0xd9]) {
        return false;
    }
    let mut at = 2;
    let mut frame = false;
    while at + 4 <= bytes.len() {
        if bytes[at] != 0xff {
            return false;
        }
        let marker = bytes[at + 1];
        let len = usize::from(u16::from_be_bytes([bytes[at + 2], bytes[at + 3]]));
        if len < 2 || at + 2 + len > bytes.len() {
            return false;
        }
        if matches!(marker, 0xc0..=0xc2) {
            if len < 8 || frame {
                return false;
            }
            let height = u16::from_be_bytes([bytes[at + 5], bytes[at + 6]]);
            let width = u16::from_be_bytes([bytes[at + 7], bytes[at + 8]]);
            if !(1..=1024).contains(&height) || !(1..=1024).contains(&width) {
                return false;
            }
            frame = true;
        }
        if marker == 0xda {
            return frame && at + 2 + len < bytes.len() - 2;
        }
        if !matches!(marker, 0xc0..=0xc2 | 0xc4 | 0xdb | 0xdd | 0xe0..=0xef | 0xfe) {
            return false;
        }
        at += len + 2;
    }
    false
}

impl MiguClient {
    pub(crate) async fn sms_image(
        &self,
        chinese: bool,
        cookies: &mut PassportCookies,
        check: &(impl Fn() -> Result<()> + Sync),
    ) -> Result<PassportImage> {
        self.passport_image(ImageScene::Sms, chinese, cookies, check)
            .await
    }

    pub(crate) async fn passport_image(
        &self,
        scene: ImageScene,
        chinese: bool,
        cookies: &mut PassportCookies,
        check: &(impl Fn() -> Result<()> + Sync),
    ) -> Result<PassportImage> {
        check()?;
        let result = self
            .passport_request(
                Operation::ImageFetch,
                &[
                    ("imgcodeType", scene.value().into()),
                    ("showType", if chinese { "1" } else { "0" }.into()),
                    ("sourceid", "220029".into()),
                ],
                CookieContext::Session(cookies),
                None,
                |v, _| PassportImage::parse(v),
            )
            .await;
        check()?;
        result
    }

    pub(crate) async fn check_sms_image(
        &self,
        answer: &str,
        cookies: &mut PassportCookies,
        check: &(impl Fn() -> Result<()> + Sync),
    ) -> Result<()> {
        self.check_passport_image(ImageScene::Sms, answer, cookies, check)
            .await
    }

    pub(crate) async fn check_passport_image(
        &self,
        scene: ImageScene,
        answer: &str,
        cookies: &mut PassportCookies,
        check: &(impl Fn() -> Result<()> + Sync),
    ) -> Result<()> {
        check()?;
        let result = self
            .passport_request(
                Operation::ImageCheck,
                &[
                    ("isAsync", "true".into()),
                    ("captcha", answer.into()),
                    ("imgcodeType", scene.value().into()),
                    ("sourceid", "220029".into()),
                ],
                CookieContext::Session(cookies),
                None,
                |_, _| Ok(()),
            )
            .await;
        check()?;
        result
    }
}

#[cfg(test)]
pub(crate) mod tests;
