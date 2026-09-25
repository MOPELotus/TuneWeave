use super::*;
use std::sync::atomic::{AtomicBool, Ordering};
use tuneweave_core::AudioContent;

pub(crate) mod dtsx;
mod format;
mod key;
pub(crate) mod sing_along;
pub(crate) use key::Key;
#[cfg(test)]
pub(crate) mod tests;

/// Ordinary tracks are bounded to 128 MiB; master and vinyl may use 512 MiB.
/// At most two transfers/conversions
/// are in flight. A cancelled CPU job retains its slot until it stops.
const MAX_BYTES: usize = 128 * 1024 * 1024;
const MAX_LARGE_FLAC_BYTES: usize = 512 * 1024 * 1024;
pub(crate) const CONTENT_BUDGET: Duration = Duration::from_secs(180);
static SLOTS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(2);

struct Control {
    cancelled: AtomicBool,
    deadline: std::time::Instant,
}
impl Control {
    fn check(&self) -> Result<()> {
        if self.cancelled.load(Ordering::Relaxed) || std::time::Instant::now() >= self.deadline {
            return Err(timeout());
        }
        Ok(())
    }
}
struct Cancel(Arc<Control>);
impl Drop for Cancel {
    fn drop(&mut self) {
        self.0.cancelled.store(true, Ordering::Relaxed);
    }
}
fn timeout() -> TuneWeaveError {
    TuneWeaveError::new(
        ErrorCode::UpstreamTimeout,
        "Kuwo audio content exceeded its time budget",
    )
    .with_platform(Platform::Kuwo)
    .retryable(false)
}

impl KuwoClient {
    /// Delivers authorized full audio or marked preview bytes, including native encrypted media.
    pub async fn native_audio_content(
        &self,
        credential: &ProviderCredential,
        track: &Track,
        request: &StreamRequest,
    ) -> Result<AudioContent> {
        self.native_content(credential, track, request, Action::Play)
            .await
    }
    /// Delivers bytes only after a separate download entitlement check.
    pub async fn native_download_content(
        &self,
        credential: &ProviderCredential,
        track: &Track,
        request: &StreamRequest,
    ) -> Result<AudioContent> {
        self.native_content(credential, track, request, Action::Download)
            .await
    }
    async fn native_content(
        &self,
        credential: &ProviderCredential,
        track: &Track,
        request: &StreamRequest,
        action: Action,
    ) -> Result<AudioContent> {
        canonical_media_track_id(track)?;
        let input = credential::NativeCredential::parse(credential)?.input()?;
        self.fetch_native_content(&input, track, request, action, CONTENT_BUDGET, || Ok(()))
            .await
    }
    pub(crate) async fn fetch_native_content<F>(
        &self,
        input: &KuwoNativeSessionInput,
        track: &Track,
        request: &StreamRequest,
        action: Action,
        budget: Duration,
        guard: F,
    ) -> Result<AudioContent>
    where
        F: Fn() -> Result<()> + Sync,
    {
        validate_request(request)?;
        let id = canonical_media_track_id(track)?;
        validate_session_metadata(input)?;
        let control = Arc::new(Control {
            cancelled: AtomicBool::new(false),
            deadline: std::time::Instant::now() + budget,
        });
        let _cancel = Cancel(control.clone());
        let operation = async {
            guard()?;
            control.check()?;
            let permit = SLOTS.acquire().await.map_err(|_| invalid())?;
            guard()?;
            control.check()?;
            let outcome = self
                .fetch_native_media(input, id, request, action, BUDGET, &guard)
                .await;
            guard()?;
            let (url, format, key, trial, limit) = match outcome? {
                Outcome::Allowed {
                    url,
                    format,
                    key,
                    trial,
                    quality,
                    ..
                } => (
                    url,
                    format,
                    key,
                    trial,
                    if matches!(quality, Quality::Master | Quality::Vinyl) {
                        MAX_LARGE_FLAC_BYTES
                    } else {
                        MAX_BYTES
                    },
                ),
                denied @ Outcome::Denied { .. } => {
                    return Err(denied
                        .stream(track, request)
                        .expect_err("denied authorization"));
                }
            };
            if action == Action::Download && trial.is_some() {
                return Err(TuneWeaveError::new(
                    ErrorCode::PermissionDenied,
                    "Kuwo previews cannot satisfy download authorization",
                )
                .with_platform(Platform::Kuwo));
            }
            let fetched = self.fetch_media_body(&url, limit, &guard).await;
            guard()?;
            let mut bytes = fetched?;
            let control = control.clone();
            let preview = trial.clone();
            let result = tokio::task::spawn_blocking(move || {
                let _permit = permit;
                for (i, chunk) in bytes.chunks_mut(64 * 1024).enumerate() {
                    control.check()?;
                    if let Some(key) = &key {
                        key.transform((i * 64 * 1024) as u64, chunk)?;
                    }
                }
                control.check()?;
                if format == "mgg" {
                    if preview.is_some() {
                        return Err(invalid());
                    }
                    let wave = sing_along::convert(&bytes, &control)?;
                    return Ok((wave, "audio/wav", "wav"));
                }
                let inspected = format::inspect(&bytes, format, &control);
                control.check()?;
                let (mime, extension) = inspected?;
                if let Some(window) = &preview {
                    format::validate_preview_duration(&bytes, window, &control)?;
                }
                Ok((bytes, mime, extension))
            })
            .await
            .map_err(|_| invalid());
            guard()?;
            let (bytes, mime, extension) = result??;
            Ok(AudioContent {
                track_ref: track.resource_ref.clone(),
                bytes,
                content_type: mime.into(),
                filename: if request.variant == StreamVariant::SingAlong {
                    format!("kuwo-{id}-sing-along.{extension}")
                } else if trial.is_some() {
                    format!("kuwo-{id}-preview.{extension}")
                } else {
                    format!("kuwo-{id}.{extension}")
                },
                trial,
            })
        };
        let result = tokio::time::timeout(budget, operation).await;
        guard()?;
        result.unwrap_or_else(|_| Err(timeout()))
    }
    async fn fetch_media_body<F>(&self, url: &str, limit: usize, guard: &F) -> Result<Vec<u8>>
    where
        F: Fn() -> Result<()> + Sync,
    {
        // This URL has already passed the native media allowlist. No session,
        // cookies, signed request fields, retries or redirects go to the CDN.
        #[cfg(test)]
        let mapped = self.web_test_origin.as_ref().map(|origin| {
            let parsed = Url::parse(url).expect("validated media URL");
            origin
                .join(parsed.path())
                .expect("fixture media path")
                .to_string()
        });
        #[cfg(test)]
        let url = mapped.as_deref().unwrap_or(url);
        guard()?;
        let fetched = self
            .media_http
            .get(url)
            .header(reqwest::header::ACCEPT, "application/octet-stream, audio/*")
            .header(reqwest::header::ACCEPT_ENCODING, "identity")
            .timeout(CONTENT_BUDGET)
            .send()
            .await;
        guard()?;
        let mut response =
            fetched.map_err(|e| if e.is_timeout() { timeout() } else { invalid() })?;
        if response.status() != StatusCode::OK {
            return Err(invalid());
        }
        let headers = response.headers();
        if headers
            .get(reqwest::header::CONTENT_ENCODING)
            .is_some_and(|v| v.as_bytes() != b"identity")
        {
            return Err(invalid());
        }
        let mime = headers
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.split(';').next())
            .map(str::trim)
            .unwrap_or("");
        if !matches!(
            mime,
            "application/octet-stream"
                | "audio/mpeg"
                | "audio/mp3"
                | "audio/aac"
                | "audio/mp4"
                | "audio/x-m4a"
                | "audio/flac"
                | "audio/x-flac"
                | "audio/ogg"
                | "application/ogg"
        ) {
            return Err(invalid());
        }
        let length = response.content_length();
        if length.is_some_and(|n| n == 0 || n > limit as u64) {
            return Err(invalid());
        }
        let mut bytes = Vec::new();
        loop {
            guard()?;
            let chunk = response.chunk().await;
            guard()?;
            let Some(chunk) =
                chunk.map_err(|e| if e.is_timeout() { timeout() } else { invalid() })?
            else {
                break;
            };
            if chunk.len() > limit - bytes.len() {
                return Err(invalid());
            }
            bytes.extend_from_slice(&chunk);
        }
        if bytes.is_empty() || length.is_some_and(|n| n != bytes.len() as u64) {
            return Err(invalid());
        }
        Ok(bytes)
    }
}
