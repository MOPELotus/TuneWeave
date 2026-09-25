//! Account-bound ordinary music rights and native media resolution.
use super::*;
use crate::provider::parse_music_id;
use std::collections::BTreeMap;
use tuneweave_core::{Extensions, ProviderCredential, ResourceRef, TrialWindow};

mod codec;
pub(crate) mod content;
#[cfg(test)]
pub(crate) mod dtsx_tests;
#[cfg(test)]
mod master_tests;
#[cfg(test)]
mod quality_tests;
mod response;
mod rights;
#[cfg(test)]
pub(crate) mod sing_along_tests;
#[cfg(test)]
pub(crate) mod spatial_tests;
#[cfg(test)]
pub(crate) mod tests;
pub(crate) mod trial;
#[cfg(test)]
pub(crate) mod vinyl_tests;

pub(super) const RIGHTS_PATH: &str = "/music.pay";
pub(super) const MEDIA_PATH: &str = "/mobi.s";
pub(super) const MAX_RESPONSE: usize = 256 * 1024;
pub(crate) const BUDGET: Duration = Duration::from_secs(60);
const RIGHTS_HOST: &str = "musicpay.kuwo.cn";
const MEDIA_HOST: &str = "anymatch.kuwo.cn";

#[derive(Clone, Copy, Eq, PartialEq)]
pub(crate) enum Action {
    Play,
    Download,
}
impl Action {
    fn rights(self) -> &'static str {
        match self {
            Self::Play => "play",
            Self::Download => "download",
        }
    }
    fn mode(self) -> &'static str {
        match self {
            Self::Play => "audition",
            Self::Download => "download",
        }
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Spec {
    tag: &'static str,
    format: &'static str,
    rights_format: &'static str,
    selector: u32,
    quality: Quality,
    bitrate: Option<u64>,
}
const LOW: Spec = Spec {
    tag: "L",
    format: "aac",
    rights_format: "AAC48",
    selector: 48,
    quality: Quality::Low,
    bitrate: Some(48_000),
};
const STANDARD: Spec = Spec {
    tag: "H",
    format: "mp3",
    rights_format: "MP3128",
    selector: 128,
    quality: Quality::Standard,
    bitrate: Some(128_000),
};
const HIGH: Spec = Spec {
    tag: "S",
    format: "mp3",
    rights_format: "MP3H",
    selector: 320,
    quality: Quality::High,
    bitrate: Some(320_000),
};
// 2000 is an upstream quality selector, not the measured bitrate of a FLAC file.
const LOSSLESS: Spec = Spec {
    tag: "F",
    format: "flac",
    rights_format: "ALFLAC",
    selector: 2000,
    quality: Quality::Lossless,
    bitrate: None,
};
// Like 2000 for lossless, 4000 denotes an upstream resource tier. It does not
// establish a measured 4 Mbps bitrate or a particular sample rate/bit depth.
const HI_RES: Spec = Spec {
    tag: "HR",
    format: "flac",
    rights_format: "HIRFLAC",
    selector: 4000,
    quality: Quality::Hires,
    bitrate: None,
};
// ZPLY is the native master tier. ZP (20000) is a different resource family.
// MFLAC is encrypted transport; its decrypted bytes must pass FLAC validation.
const MASTER: Spec = Spec {
    tag: "ZPLY",
    format: "mflac",
    rights_format: "ZPLY",
    selector: 20900,
    quality: Quality::Master,
    bitrate: None,
};
// The native ZPGA201 tier is "至臻全景声". Its selector is not a measured
// bitrate or a promise of a particular channel layout. Other ZPGA tiers and
// BCMS accompaniment have separate resource contracts.
const SPATIAL: Spec = Spec {
    tag: "ZPGA201",
    format: "mflac",
    rights_format: "ZPGA201",
    selector: 20201,
    quality: Quality::Spatial,
    bitrate: None,
};
// VINYL is the native vinyl resource tier, distinct from ordinary
// lossless and ZPLY master. 23000 selects that version, not its bitrate.
const VINYL: Spec = Spec {
    tag: "VINYL",
    format: "flac",
    rights_format: "VINYL",
    selector: 23000,
    quality: Quality::Vinyl,
    bitrate: None,
};

// DTSX selects encrypted MMP4. Its decrypted MP4 is delivered without PCM
// decoding; neither this selector nor transport identity certifies playback
// or full DTS-UHD Profile 2 conformance. 25000 is not an output bitrate.
const DTSX: Spec = Spec {
    tag: "DTSX",
    format: "mmp4",
    rights_format: "DTSX",
    selector: 25000,
    quality: Quality::Dtsx,
    bitrate: None,
};

// BCMS is a separately authorized four-stem sidecar. Its selector is not
// an output bitrate or the quality of the ordinary track used to request it.
const BCMS: Spec = Spec {
    tag: "BCMS",
    format: "mgg",
    rights_format: "BCMS",
    selector: 22000,
    quality: Quality::Auto,
    bitrate: None,
};

pub(crate) fn validate_request(request: &StreamRequest) -> Result<()> {
    specs(request).map(|_| ())
}
fn specs(request: &StreamRequest) -> Result<Vec<Spec>> {
    if !matches!(
        request.variant,
        StreamVariant::Default | StreamVariant::SingAlong
    ) || request.immersive_type.is_some()
    {
        return Err(unsupported(
            "Kuwo native media does not support this stream variant",
        ));
    }
    if request.variant == StreamVariant::SingAlong {
        if request.bitrate.is_some() {
            return Err(kuwo_invalid_request(
                "Kuwo sing-along does not accept a requested output bitrate",
            ));
        }
        return match request.quality {
            Quality::Auto => Ok(vec![HI_RES, LOSSLESS, HIGH, STANDARD, LOW]),
            Quality::Low => Ok(vec![LOW]),
            Quality::Standard => Ok(vec![STANDARD]),
            Quality::Higher | Quality::High => Ok(vec![HIGH]),
            Quality::Lossless => Ok(vec![LOSSLESS]),
            Quality::Hires => Ok(vec![HI_RES]),
            _ => Err(unsupported(
                "Kuwo sing-along requires an ordinary main resource",
            )),
        };
    }
    let choices = match request.quality {
        Quality::Auto => vec![MASTER, HI_RES, LOSSLESS, HIGH, STANDARD, LOW],
        Quality::Low => vec![LOW],
        Quality::Standard => vec![STANDARD],
        Quality::Higher | Quality::High => vec![HIGH],
        Quality::Lossless => vec![LOSSLESS],
        Quality::Hires => vec![HI_RES],
        Quality::Master => vec![MASTER],
        Quality::Spatial => vec![SPATIAL],
        Quality::Vinyl => vec![VINYL],
        Quality::Dtsx => vec![DTSX],
        _ => {
            return Err(unsupported(
                "Kuwo native media does not yet support this audio quality",
            ));
        }
    };
    if let Some(bitrate) = request.bitrate {
        let choices: Vec<_> = choices
            .into_iter()
            .filter(|spec| spec.bitrate == Some(bitrate))
            .collect();
        if choices.is_empty() {
            return Err(kuwo_invalid_request(
                "Kuwo native media bitrate conflicts with its supported quality",
            ));
        }
        return Ok(choices);
    }
    Ok(choices)
}

pub(crate) fn availability_request(request: &TrackAvailabilityRequest) -> Result<StreamRequest> {
    let result = StreamRequest {
        quality: Quality::Auto,
        bitrate: (request.bitrate != TrackAvailabilityRequest::DEFAULT_BITRATE)
            .then_some(request.bitrate),
        account: request.account.clone(),
        ..StreamRequest::default()
    };
    validate_request(&result)?;
    Ok(result)
}

pub(crate) enum Outcome {
    Allowed {
        url: String,
        backups: Vec<String>,
        format: &'static str,
        bitrate: Option<u64>,
        quality: Quality,
        duration_ms: u64,
        key: Option<content::Key>,
        trial: Option<TrialWindow>,
    },
    Denied {
        code: Option<i64>,
        message: &'static str,
    },
}
impl Outcome {
    pub(crate) fn stream(self, track: &Track, request: &StreamRequest) -> Result<MediaStream> {
        match self {
            Self::Allowed { key: Some(_), .. } => Err(unsupported(
                "Kuwo encrypted audio requires the audio content endpoint",
            )),
            Self::Allowed {
                url,
                backups,
                format,
                bitrate,
                quality,
                duration_ms,
                key: None,
                trial,
            } => Ok(MediaStream {
                url,
                backup_urls: backups,
                headers: BTreeMap::new(),
                expires_at: None,
                format: Some(format.into()),
                codec: Some(format.into()),
                bitrate,
                size: None,
                duration_ms: Some(duration_ms),
                requested_quality: request.quality,
                actual_quality: quality,
                trial,
                origin_track: Some(track.resource_ref.clone()),
                resolved_track: track.resource_ref.clone(),
                resolved_platform: Platform::Kuwo,
                match_score: Some(1.0),
                attempts: Vec::new(),
            }),
            Self::Denied { code, message } => {
                Err(TuneWeaveError::new(ErrorCode::PermissionDenied, message)
                    .with_platform(Platform::Kuwo)
                    .with_details(
                        json!({"platform_code":code,"scope":"native_account","full_track":false}),
                    )
                    .retryable(false))
            }
        }
    }
    pub(crate) fn download(self, track: &Track, request: &StreamRequest) -> MediaDownload {
        let mut extensions = extensions();
        if request.variant == StreamVariant::SingAlong {
            extensions.insert("requested_variant".into(), json!("sing_along"));
        }
        let (available, url, format, bitrate, quality, duration_ms, code, message) = match self {
            Self::Allowed { trial: Some(_), .. } => (
                false,
                None,
                None,
                None,
                Quality::Auto,
                None,
                Some(200),
                Some("Kuwo previews cannot satisfy download authorization".into()),
            ),
            Self::Allowed { key: Some(_), .. } => {
                extensions.insert("content_delivery".into(), json!("download_content"));
                (
                    false,
                    None,
                    None,
                    None,
                    Quality::Auto,
                    None,
                    Some(200),
                    Some("Kuwo encrypted audio requires the download content endpoint".into()),
                )
            }
            Self::Allowed {
                url,
                format,
                bitrate,
                quality,
                duration_ms,
                ..
            } => (
                true,
                Some(url),
                Some(format.to_owned()),
                bitrate,
                quality,
                Some(duration_ms),
                Some(200),
                None,
            ),
            Self::Denied { code, message } => (
                false,
                None,
                None,
                None,
                Quality::Auto,
                None,
                code,
                Some(message.to_owned()),
            ),
        };
        extensions.insert("full_track".into(), json!(available));
        MediaDownload {
            track_ref: track.resource_ref.clone(),
            platform: Platform::Kuwo,
            available,
            url,
            headers: BTreeMap::new(),
            expires_at: None,
            codec: format.clone(),
            format,
            bitrate,
            size: None,
            duration_ms,
            requested_quality: request.quality,
            actual_quality: quality,
            platform_code: code,
            fee: None,
            message,
            extensions,
        }
    }
    pub(crate) fn availability(
        self,
        track_ref: ResourceRef,
        request: &TrackAvailabilityRequest,
    ) -> TrackAvailability {
        let mut extensions = extensions();
        if matches!(&self, Self::Allowed { key: Some(_), .. }) {
            extensions.insert("content_delivery".into(), json!("audio_content"));
        }
        let (playable, actual_bitrate, code, message) = match self {
            Self::Allowed {
                trial: Some(window),
                bitrate,
                ..
            } => {
                extensions.insert("preview_available".into(), json!(true));
                extensions.insert("preview_start_ms".into(), json!(window.start_ms));
                extensions.insert(
                    "preview_duration_ms".into(),
                    json!(window.end_ms - window.start_ms),
                );
                extensions.insert("preview_actual_bitrate".into(), json!(bitrate));
                extensions.insert(
                    "authorization_source".into(),
                    json!("musicpay_and_audition"),
                );
                (
                    false,
                    None,
                    Some(200),
                    "Kuwo only authorized a preview for this account",
                )
            }
            Self::Allowed { bitrate, .. } => (true, bitrate, Some(200), "ok"),
            Self::Denied { code, message } => (false, None, code, message),
        };
        extensions.insert("full_track".into(), json!(playable));
        TrackAvailability {
            track_ref,
            playable,
            requested_bitrate: request.bitrate,
            actual_bitrate,
            platform_code: code,
            message: message.into(),
            extensions,
        }
    }
}
fn extensions() -> Extensions {
    BTreeMap::from([
        ("scope".into(), json!("native_account")),
        (
            "authorization_source".into(),
            json!("musicpay_and_native_media"),
        ),
    ])
}

impl KuwoClient {
    /// Resolves full audio or an explicitly marked authorized preview for this
    /// exact native credential. No account
    /// storage, SID rotation, public fallback or media-body download is performed.
    pub async fn native_stream(
        &self,
        credential: &ProviderCredential,
        track: &Track,
        request: &StreamRequest,
    ) -> Result<MediaStream> {
        let id = canonical_media_track_id(track)?;
        let input = credential::NativeCredential::parse(credential)?.input()?;
        self.fetch_native_media(&input, id, request, Action::Play, BUDGET, || Ok(()))
            .await?
            .stream(track, request)
    }
    /// Queries download rights separately; playback authorization is not reused.
    pub async fn native_download(
        &self,
        credential: &ProviderCredential,
        track: &Track,
        request: &StreamRequest,
    ) -> Result<MediaDownload> {
        let id = canonical_media_track_id(track)?;
        let input = credential::NativeCredential::parse(credential)?.input()?;
        Ok(self
            .fetch_native_media(&input, id, request, Action::Download, BUDGET, || Ok(()))
            .await?
            .download(track, request))
    }
    /// Full playback availability, verified through rights and media resolution.
    pub async fn native_track_availability(
        &self,
        credential: &ProviderCredential,
        id: &str,
        request: &TrackAvailabilityRequest,
    ) -> Result<TrackAvailability> {
        let id = parse_music_id(id)?;
        let stream = availability_request(request)?;
        let input = credential::NativeCredential::parse(credential)?.input()?;
        Ok(self
            .fetch_native_media(&input, id, &stream, Action::Play, BUDGET, || Ok(()))
            .await?
            .availability(kuwo_track_ref(id)?, request))
    }
    pub(crate) async fn fetch_native_media<F>(
        &self,
        input: &KuwoNativeSessionInput,
        id: &str,
        request: &StreamRequest,
        action: Action,
        budget: Duration,
        guard: F,
    ) -> Result<Outcome>
    where
        F: Fn() -> Result<()> + Sync,
    {
        let choices = specs(request)?;
        let id = parse_music_id(id)?;
        validate_session_metadata(input)?;
        let operation = async {
            guard()?;
            let validation = self.validate_native_session(input).await;
            guard()?;
            validation?;
            let result = self
                .fetch_native_media_rights(
                    input,
                    id,
                    &choices,
                    action,
                    request.variant == StreamVariant::SingAlong,
                )
                .await;
            guard()?;
            let grant = match result? {
                rights::Decision::Preview => {
                    guard()?;
                    let result = self.fetch_native_trial(input, id).await;
                    guard()?;
                    return result;
                }
                rights::Decision::Denied(outcome) => return Ok(outcome),
                rights::Decision::Granted(grant) => grant,
            };
            let values = media_query(input, id, &grant, action);
            let encoded = codec::seal(values)?;
            guard()?;
            let target = format!(
                "{}?f=kwxs&q={encoded}",
                self.native_target(MEDIA_HOST, MEDIA_PATH)
            );
            let result = self
                .native_get(
                    MEDIA_HOST,
                    MEDIA_PATH,
                    "native_account_media",
                    target,
                    |bytes| {
                        if grant.bc_token.is_some() {
                            response::parse_sing_along(bytes, input, id, grant.spec)
                        } else {
                            response::parse(bytes, input, id, grant.spec)
                        }
                    },
                )
                .await;
            guard()?;
            result
        };
        let result = tokio::time::timeout(budget, operation).await;
        guard()?;
        result.unwrap_or_else(|_| {
            Err(TuneWeaveError::new(
                ErrorCode::UpstreamTimeout,
                "Kuwo native media exceeded its time budget",
            )
            .with_platform(Platform::Kuwo)
            .retryable(false))
        })
    }
}

fn media_query(
    input: &KuwoNativeSessionInput,
    id: &str,
    grant: &rights::Grant,
    action: Action,
) -> BTreeMap<&'static str, String> {
    let mut values: BTreeMap<_, _> = [
        ("user", input.device_user()),
        ("prod", "kwplayer_ar_12.2.2.0"),
        ("corp", "kuwo"),
        ("newver", "3"),
        ("vipver", CLIENT_VERSION),
        ("source", CLIENT_SOURCE),
        ("p2p", "1"),
        ("q36", device::FALLBACK_Q36),
        ("approval", "false"),
        ("loginUid", input.user_id()),
        ("loginSid", input.session_id()),
        ("appuid", input.device_id()),
        ("allpay", "0"),
        ("notrace", "1"),
        ("oaid", ""),
        ("vipMode", "0"),
        ("type", "convert_url_with_sign"),
        // Native special tiers use these preferences with their exact
        // selectors. A lower-quality or different-format response is rejected.
        (
            "format",
            if grant.bc_token.is_some() {
                if grant.spec.format == "flac" {
                    "flac|mp3|aac"
                } else {
                    "mp3|aac"
                }
            } else if grant.spec == DTSX {
                "flac|mp3|aac"
            } else if grant.spec.format == "mflac" || grant.spec == VINYL {
                "mp3|aac"
            } else {
                grant.spec.format
            },
        ),
        ("sig", "0"),
        ("rid", id),
        ("priority", "bitrate"),
        ("network", "WIFI"),
        ("localUid", "-1"),
        ("mode", action.mode()),
        ("token", grant.token.as_str()),
        ("bc_token", grant.bc_token.as_deref().unwrap_or("")),
        ("uid", input.device_id()),
        ("downloadPay", grant.download_pay.as_str()),
        ("playPay", grant.play_pay.as_str()),
        ("payUid", input.user_id()),
        ("isstar", "false"),
        ("surl", "1"),
        ("apiv", "1"),
        ("aiOp", ""),
        ("dev_type", "TuneWeave"),
    ]
    .into_iter()
    .map(|(k, v)| (k, v.to_owned()))
    .collect();
    if let Some(context) = &input.context {
        values.insert("android_id", context.android_id.clone());
    }
    values.insert(
        "br",
        format!("{}k{}", grant.spec.selector, grant.spec.format),
    );
    values.insert("timestamp", grant.timestamp.to_string());
    values
}
fn invalid() -> TuneWeaveError {
    kuwo_upstream_error("Kuwo native media response is invalid").retryable(false)
}
fn unsupported(message: &'static str) -> TuneWeaveError {
    TuneWeaveError::new(ErrorCode::CapabilityNotSupported, message)
        .with_platform(Platform::Kuwo)
        .retryable(false)
}
