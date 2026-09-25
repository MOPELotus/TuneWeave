//! The official MG3D byte transform. MGM/local encryption is a different format.
use super::*;
use crate::credential::error;
use md5::{Digest, Md5};

pub(crate) mod flac;

pub(crate) const MAX_CONTENT: u64 = 64 * 1024 * 1024;
// Public application format constant recovered from official setAppCode with
// the APK's public signing certificate. It is not an account or media key.
const APPLICATION_CODE: &[u8; 32] = b"AC89EC47A70B76F307CB39A0D74BCCB0";

pub(crate) fn derive(key: &[u8; 32]) -> [u8; 32] {
    let mut digest = Md5::new();
    digest.update(APPLICATION_CODE);
    digest.update(key);
    hex::encode_upper(digest.finalize())
        .as_bytes()
        .try_into()
        .expect("MD5 hex length")
}

pub(crate) fn decode(bytes: &mut [u8], key: &[u8; 32], offset: usize) {
    for (index, byte) in bytes.iter_mut().enumerate() {
        *byte = byte.wrapping_sub(key[(offset % 32 + index % 32) % 32]);
    }
}

impl MiguClient {
    pub(crate) async fn fetch_download_content(
        &self,
        grant: &super::account_download::DownloadContentGrant,
    ) -> Result<Vec<u8>> {
        let size = grant
            .media
            .size
            .filter(|size| *size > 0 && *size <= MAX_CONTENT)
            .ok_or_else(|| {
                error(
                    ErrorCode::CapabilityNotSupported,
                    "Migu download content exceeds the supported size limit",
                )
            })?;
        let source = grant
            .media
            .url
            .as_deref()
            .ok_or_else(|| migu_upstream_error("Migu download grant omitted its URL"))?;
        let validated = super::account_download::validate_download_url(source)?;
        let url = Url::parse(&validated)
            .map_err(|_| migu_upstream_error("Migu download URL is invalid"))?;
        #[cfg(test)]
        let url = if let Some(origin) = &self.catalog_test_origin {
            let mut target = origin.join(url.path()).expect("fixture path");
            target.set_query(url.query());
            target
        } else {
            url
        };
        // BaseDownloadTask's independent CDN client sends no account headers.
        // The shared client has redirects/retries disabled and no cookie jar.
        let response = self
            .http
            .get(url)
            .header(ACCEPT, "application/octet-stream")
            .timeout(Duration::from_secs(60))
            .send()
            .await
            .map_err(|e| {
                error(
                    if e.is_timeout() {
                        ErrorCode::UpstreamTimeout
                    } else {
                        ErrorCode::UpstreamError
                    },
                    "Migu download content request failed",
                )
            })?;
        if response.status() != StatusCode::OK {
            return Err(error(
                if response.status() == StatusCode::TOO_MANY_REQUESTS {
                    ErrorCode::RateLimited
                } else {
                    ErrorCode::UpstreamError
                },
                "Migu download content request was not accepted",
            ));
        }
        if response
            .headers()
            .contains_key(reqwest::header::CONTENT_RANGE)
            || response
                .headers()
                .get(reqwest::header::CONTENT_ENCODING)
                .is_some_and(|value| value.as_bytes() != b"identity")
            || response
                .content_length()
                .is_some_and(|actual| actual != size)
        {
            return Err(migu_upstream_error(
                "Migu download content has conflicting transfer metadata",
            ));
        }
        let mime = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.split(';').next())
            .map(str::trim);
        let expected = match grant.media.format.as_deref() {
            Some("mp3") => "audio/mpeg",
            Some("flac") => "audio/flac",
            _ => {
                return Err(migu_upstream_error(
                    "Migu download grant has an unsupported format",
                ));
            }
        };
        if mime != Some("application/octet-stream") && mime != Some(expected) {
            return Err(migu_upstream_error(
                "Migu download content has an unsupported content type",
            ));
        }
        let bytes = super::account::read_session_body_limited(response, size).await?;
        if bytes.len() as u64 != size {
            return Err(migu_upstream_error("Migu download content is incomplete"));
        }
        Ok(bytes)
    }
}

/// Full MP3 framing and duration check, not PCM decoding or playback acceptance.
pub(crate) fn inspect_mp3(
    bytes: &[u8],
    duration_ms: u64,
    mut check: impl FnMut() -> Result<()>,
) -> Result<()> {
    let invalid =
        || migu_upstream_error("Migu content is not the authorized complete MP3 rendition");
    let mut data = bytes;
    if data.starts_with(b"ID3") {
        let header = data.get(..10).ok_or_else(invalid)?;
        if !(2..=4).contains(&header[3])
            || header[4] == 255
            || header[6..].iter().any(|b| b & 128 != 0)
        {
            return Err(invalid());
        }
        let size = header[6..]
            .iter()
            .fold(0_usize, |size, byte| (size << 7) | usize::from(*byte));
        if size > 1024 * 1024 {
            return Err(invalid());
        }
        let footer = usize::from(header[3] == 4 && header[5] & 16 != 0) * 10;
        data = data.get(10 + size + footer..).ok_or_else(invalid)?;
    }
    let mut signature = None;
    let mut samples = 0_u64;
    let mut sample_rate = 0;
    let mut count = 0;
    while !data.is_empty() {
        if count % 256 == 0 {
            check()?;
        }
        if data.len() == 128 && data.starts_with(b"TAG") {
            break;
        }
        let h = data.get(..4).ok_or_else(invalid)?;
        let version = (h[1] >> 3) & 3;
        let rate_index = (h[2] >> 2) & 3;
        let bitrate_index = usize::from(h[2] >> 4);
        if h[0] != 255
            || h[1] & 0xe6 != 0xe2
            || version == 1
            || rate_index == 3
            || !(1..15).contains(&bitrate_index)
        {
            return Err(invalid());
        }
        let current = (version, rate_index, h[3] >> 6 == 3);
        if signature.is_some_and(|value| value != current) {
            return Err(invalid());
        }
        signature = Some(current);
        let table = if version == 3 {
            [
                0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320,
            ]
        } else {
            [0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160]
        };
        let rate = [44100, 48000, 32000][usize::from(rate_index)]
            / match version {
                3 => 1,
                2 => 2,
                _ => 4,
            };
        let size = (if version == 3 { 144 } else { 72 }) * table[bitrate_index] * 1000 / rate
            + usize::from((h[2] >> 1) & 1);
        data = data.get(size..).ok_or_else(invalid)?;
        count += 1;
        samples += if version == 3 { 1152 } else { 576 };
        sample_rate = rate as u64;
    }
    if count < 2
        || duration_ms == 0
        || sample_rate == 0
        || (samples * 1000 / sample_rate).abs_diff(duration_ms) > 1500
    {
        return Err(invalid());
    }
    check()
}

#[cfg(test)]
mod tests;
