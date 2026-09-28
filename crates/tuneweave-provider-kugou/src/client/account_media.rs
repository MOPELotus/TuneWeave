//! Account tracker responses are mapped independently from anonymous privilege responses.
use super::dto::Number;
use super::*;
use crate::account::media::{Behavior, Status, TrackerResponse};

pub(crate) struct Selection {
    track: Track,
    spec: SelectedMediaSpec,
    pub(crate) album_id: u64,
}

#[cfg(debug_assertions)]
fn safe_tracker_shape(bytes: &[u8]) -> Value {
    const FIELDS: &[&str] = &[
        "status",
        "error_code",
        "errcode",
        "hash",
        "std_hash",
        "std_hash_time",
        "is_hash_backup",
        "quality_demotion",
        "is_quality_demotion",
        "album_audio_id",
        "albumAudioId",
        "album_id",
        "albumId",
        "url",
        "backupUrl",
        "timeLength",
        "fileSize",
        "bitRate",
        "extName",
        "fail_process",
        "hash_offset",
    ];

    fn kind(value: &Value) -> &'static str {
        match value {
            Value::Null => "null",
            Value::Bool(_) => "bool",
            Value::Number(_) => "number",
            Value::String(_) => "string",
            Value::Array(_) => "array",
            Value::Object(_) => "object",
        }
    }

    let Ok(value) = serde_json::from_slice::<Value>(bytes) else {
        return json!({"json": false});
    };
    let mut fields = serde_json::Map::new();
    for name in FIELDS {
        fields.insert(
            (*name).into(),
            json!(value.get(*name).map(kind).unwrap_or("missing")),
        );
    }
    json!({"json": true, "field_types": fields})
}

#[cfg(debug_assertions)]
fn diagnostic_tracker_failure(
    stage: &'static str,
    bytes: &[u8],
    status: Option<&Status>,
    checks: &[(&'static str, bool)],
) {
    let status_parsed = status.is_some();
    let platform_status = status.map_or(-1, |value| value.status);
    let platform_code = status.map_or(-1, Status::code);
    let checks = checks
        .iter()
        .map(|(name, passed)| format!("{name}={passed}"))
        .collect::<Vec<_>>()
        .join(",");
    eprintln!(
        "DIAGNOSTIC kugou_media_tracker stage={stage} status_parsed={status_parsed} platform_status={platform_status} platform_code={platform_code} shape={} checks={checks}",
        safe_tracker_shape(bytes)
    );
}

impl Selection {
    // The provider supplies metadata freshly read from the fixed catalogue endpoint.
    pub(crate) fn new(track: Track, request: &StreamRequest) -> Result<Self> {
        validate_track(&track)?;
        validate_request(request)?;
        let mut catalogue_request = request.clone();
        catalogue_request.account = None;
        let spec = select_media_spec(&track, &catalogue_request)?;
        let album_id = canonical_album_id(&track)?;
        Ok(Self {
            track,
            spec,
            album_id,
        })
    }
    pub(crate) fn hash(&self) -> &str {
        &self.spec.hash
    }
    pub(crate) fn quality(&self) -> &str {
        self.spec.tracker_quality
    }

    pub(crate) fn lower(&self) -> Result<Option<Self>> {
        let quality = match self.spec.actual_quality {
            Quality::Master => Quality::Hires,
            Quality::Hires => Quality::Lossless,
            Quality::Lossless => Quality::High,
            Quality::High => Quality::Standard,
            _ => return Ok(None),
        };
        match Self::new(
            self.track.clone(),
            &StreamRequest {
                quality,
                ..Default::default()
            },
        ) {
            Ok(mut lower) => {
                lower.spec.requested_quality = self.spec.requested_quality;
                Ok(Some(lower))
            }
            Err(error) if error.code == ErrorCode::ResourceNotFound => Ok(None),
            Err(error) => Err(error),
        }
    }

    pub(crate) fn map(self, response: TrackerResponse, behavior: Behavior) -> Result<MediaStream> {
        let status = match Status::parse(&response.bytes) {
            Ok(status) => status,
            Err(error) => {
                #[cfg(debug_assertions)]
                diagnostic_tracker_failure("status_parse", &response.bytes, None, &[]);
                return Err(error);
            }
        };
        if status.status != 1 || status.code() != 0 {
            #[cfg(debug_assertions)]
            diagnostic_tracker_failure(
                "status_rejected",
                &response.bytes,
                Some(&status),
                &[
                    ("success_status", status.status == 1),
                    ("success_code", status.code() == 0),
                ],
            );
            return Err(status.rejection());
        }
        let data: Tracker = match serde_json::from_slice(&response.bytes) {
            Ok(data) => data,
            Err(_) => {
                #[cfg(debug_assertions)]
                diagnostic_tracker_failure(
                    "tracker_decode",
                    &response.bytes,
                    Some(&status),
                    &[("tracker_shape_valid", false)],
                );
                return Err(invalid());
            }
        };
        let hash_match = data.hash.eq_ignore_ascii_case(&self.spec.hash);
        let _std_hash_match = data
            .std_hash
            .as_deref()
            .is_some_and(|hash| hash.eq_ignore_ascii_case(&self.spec.hash));
        let expected_duration = self
            .spec
            .duration_ms
            .filter(|duration| *duration > 0)
            .or(self.track.duration_ms.filter(|duration| *duration > 0));
        let audio_id_match = data
            .album_audio_id
            .is_none_or(|id| id.0.to_string() == self.track.id);
        let album_id_match = data.album_id.is_none_or(|id| id.0 == self.album_id);
        if !hash_match || !audio_id_match || !album_id_match {
            #[cfg(debug_assertions)]
            diagnostic_tracker_failure(
                "identity_validation",
                &response.bytes,
                Some(&status),
                &[
                    ("hash_match", hash_match),
                    ("std_hash_present", data.std_hash.is_some()),
                    ("std_hash_match", _std_hash_match),
                    ("std_hash_time_present", data._std_hash_time.is_some()),
                    (
                        "standard_quality",
                        self.spec.actual_quality == Quality::Standard,
                    ),
                    ("identity_hash_match", hash_match),
                    ("hash_backup_present", data._is_hash_backup.is_some()),
                    (
                        "hash_backup_marked",
                        data._is_hash_backup.is_some_and(|value| value.0 != 0),
                    ),
                    ("audio_id_present", data.album_audio_id.is_some()),
                    ("audio_id_match", audio_id_match),
                    ("album_id_present", data.album_id.is_some()),
                    ("album_id_match", album_id_match),
                ],
            );
            return Err(invalid());
        }
        if !data.fail_process.is_empty() {
            #[cfg(debug_assertions)]
            diagnostic_tracker_failure(
                "additional_authorization",
                &response.bytes,
                Some(&status),
                &[("fail_process_empty", false)],
            );
            return Err(denied("KuGou media requires additional authorization"));
        }
        let Some(expected_duration) = expected_duration else {
            #[cfg(debug_assertions)]
            diagnostic_tracker_failure(
                "expected_duration",
                &response.bytes,
                Some(&status),
                &[("expected_duration_present", false)],
            );
            return Err(invalid());
        };
        let Some(duration) = data.time_length.0.checked_mul(1000).filter(|n| *n > 0) else {
            #[cfg(debug_assertions)]
            diagnostic_tracker_failure(
                "time_length",
                &response.bytes,
                Some(&status),
                &[("duration_positive_and_convertible", false)],
            );
            return Err(invalid());
        };
        let size = data.file_size.0;
        let bitrate = data.bit_rate.0;
        if size == 0 || bitrate == 0 {
            #[cfg(debug_assertions)]
            diagnostic_tracker_failure(
                "media_dimensions",
                &response.bytes,
                Some(&status),
                &[
                    ("size_nonzero", size != 0),
                    ("bitrate_nonzero", bitrate != 0),
                ],
            );
            return Err(invalid());
        }
        let trial = match data.hash_offset {
            None => None,
            Some(offset) => {
                let start = offset.start_ms.0;
                let end = offset.end_ms.0;
                if end <= start || end > expected_duration.saturating_add(999) {
                    #[cfg(debug_assertions)]
                    diagnostic_tracker_failure(
                        "trial_time_range",
                        &response.bytes,
                        Some(&status),
                        &[
                            ("end_after_start", end > start),
                            (
                                "end_within_expected_duration",
                                end <= expected_duration.saturating_add(999),
                            ),
                        ],
                    );
                    return Err(invalid());
                }
                match (offset.start_byte, offset.end_byte) {
                    (Some(start), Some(end)) if end.0 >= start.0 => {
                        let Some(length) =
                            end.0.checked_sub(start.0).and_then(|n| n.checked_add(1))
                        else {
                            #[cfg(debug_assertions)]
                            diagnostic_tracker_failure(
                                "trial_byte_range_length",
                                &response.bytes,
                                Some(&status),
                                &[("byte_range_length_representable", false)],
                            );
                            return Err(invalid());
                        };
                        if size < length {
                            #[cfg(debug_assertions)]
                            diagnostic_tracker_failure(
                                "trial_byte_range",
                                &response.bytes,
                                Some(&status),
                                &[("byte_range_within_file", size >= length)],
                            );
                            return Err(invalid());
                        }
                    }
                    (None, None) => {}
                    _ => {
                        #[cfg(debug_assertions)]
                        diagnostic_tracker_failure(
                            "trial_byte_range_shape",
                            &response.bytes,
                            Some(&status),
                            &[("byte_range_pair_consistent", false)],
                        );
                        return Err(invalid());
                    }
                }
                if start == 0 && end.saturating_add(999) >= expected_duration {
                    None
                } else {
                    Some(TrialWindow {
                        start_ms: start,
                        end_ms: end,
                    })
                }
            }
        };
        if trial.is_none() && duration.abs_diff(expected_duration) > 999 {
            #[cfg(debug_assertions)]
            diagnostic_tracker_failure(
                "full_duration_match",
                &response.bytes,
                Some(&status),
                &[("duration_matches_catalogue", false)],
            );
            return Err(invalid());
        }
        if let Some(window) = &trial {
            let trial_duration_match = duration.abs_diff(window.end_ms - window.start_ms) <= 999;
            let full_duration_match = duration.abs_diff(expected_duration) <= 999;
            if !trial_duration_match && !full_duration_match {
                #[cfg(debug_assertions)]
                diagnostic_tracker_failure(
                    "trial_duration_match",
                    &response.bytes,
                    Some(&status),
                    &[
                        ("duration_matches_trial", trial_duration_match),
                        ("duration_matches_catalogue", full_duration_match),
                    ],
                );
                return Err(invalid());
            }
            if behavior == Behavior::Download {
                #[cfg(debug_assertions)]
                diagnostic_tracker_failure(
                    "trial_download_rejected",
                    &response.bytes,
                    Some(&status),
                    &[("full_media_authorized", false)],
                );
                return Err(denied(
                    "KuGou only authorized a trial; full download is unavailable",
                ));
            }
        }
        let format = data.extension.to_ascii_lowercase();
        let (codec, actual_quality) = match format.as_str() {
            "flac" | "ape"
                if matches!(
                    self.spec.actual_quality,
                    Quality::Lossless | Quality::Hires | Quality::Master
                ) =>
            {
                (Some(format.clone()), self.spec.actual_quality)
            }
            "mp3" | "ogg" | "aac" | "m4a" if bitrate <= 512_000 => {
                let actual = if bitrate <= 96_000 {
                    Quality::Low
                } else if bitrate <= 128_000 {
                    Quality::Standard
                } else {
                    Quality::High
                };
                if self.spec.actual_quality == Quality::Standard && actual == Quality::High {
                    #[cfg(debug_assertions)]
                    diagnostic_tracker_failure(
                        "quality_consistency",
                        &response.bytes,
                        Some(&status),
                        &[
                            ("format_supported", true),
                            ("bitrate_supported", bitrate <= 512_000),
                            ("quality_consistent", false),
                        ],
                    );
                    return Err(invalid());
                }
                let codec = match format.as_str() {
                    "mp3" => Some("mp3".into()),
                    "aac" => Some("aac".into()),
                    _ => None,
                };
                (codec, actual)
            }
            // KGM/encrypted downloads need their own supported decoder and rights contract.
            _ => {
                #[cfg(debug_assertions)]
                diagnostic_tracker_failure(
                    "format_support",
                    &response.bytes,
                    Some(&status),
                    &[
                        ("format_supported", false),
                        ("compressed_bitrate_supported", bitrate <= 512_000),
                        (
                            "lossless_selection",
                            matches!(
                                self.spec.actual_quality,
                                Quality::Lossless | Quality::Hires | Quality::Master
                            ),
                        ),
                    ],
                );
                return Err(denied(
                    "KuGou returned an unsupported or encrypted media format",
                ));
            }
        };
        let mut urls = Vec::new();
        let mut seen = BTreeSet::new();
        for value in data.url.0.into_iter().chain(data.backup_url.0) {
            if value.len() > 8192 || value.chars().any(char::is_control) {
                #[cfg(debug_assertions)]
                diagnostic_tracker_failure(
                    "url_shape",
                    &response.bytes,
                    Some(&status),
                    &[("url_length_and_controls_valid", false)],
                );
                return Err(invalid());
            }
            let url = match normalize_media_url(&value) {
                Ok(url) => url,
                Err(error) => {
                    #[cfg(debug_assertions)]
                    diagnostic_tracker_failure(
                        "url_normalization",
                        &response.bytes,
                        Some(&status),
                        &[("url_normalized", false)],
                    );
                    return Err(error);
                }
            };
            let url_validation = response.check_url(&url);
            #[cfg(debug_assertions)]
            if url_validation.is_err() {
                diagnostic_tracker_failure(
                    "url_authorization_material_check",
                    &response.bytes,
                    Some(&status),
                    &[("url_contains_no_session_grant", false)],
                );
            }
            url_validation?;
            if seen.insert(url.clone()) {
                urls.push(url);
            }
        }
        if urls.is_empty() || urls.len() > 16 {
            #[cfg(debug_assertions)]
            diagnostic_tracker_failure(
                "url_count",
                &response.bytes,
                Some(&status),
                &[("url_count_valid", !urls.is_empty() && urls.len() <= 16)],
            );
            return Err(invalid());
        }
        let url = urls.remove(0);
        Ok(MediaStream {
            url,
            backup_urls: urls,
            headers: BTreeMap::new(),
            expires_at: None,
            format: Some(format),
            codec,
            bitrate: Some(bitrate),
            size: Some(size),
            duration_ms: Some(duration),
            requested_quality: self.spec.requested_quality,
            actual_quality,
            trial,
            origin_track: Some(self.track.resource_ref.clone()),
            resolved_track: self.track.resource_ref,
            resolved_platform: Platform::Kugou,
            match_score: Some(1.0),
            attempts: Vec::new(),
        })
    }
}

pub(crate) fn validate_track(track: &Track) -> Result<u64> {
    let id = canonical_track_id(track)?;
    if track.id != track.resource_ref.id() {
        return Err(kugou_invalid_media_request(
            "KuGou track identities are inconsistent",
        ));
    }
    Ok(id)
}
pub(crate) fn validate_request(request: &StreamRequest) -> Result<()> {
    if request.variant != StreamVariant::Default
        || matches!(request.quality, Quality::Vinyl | Quality::Dtsx)
        || request.immersive_type.is_some()
        || request.bitrate.is_some_and(|n| !(1..=320_000).contains(&n))
        || (request.bitrate.is_none()
            && matches!(
                request.quality,
                Quality::Surround | Quality::Spatial | Quality::Dolby | Quality::Vivid
            ))
    {
        return Err(kugou_invalid_media_request(
            "KuGou account media selection is unsupported",
        ));
    }
    Ok(())
}

#[derive(Default, Deserialize)]
#[serde(from = "UrlList")]
struct Urls(Vec<String>);
#[derive(Deserialize)]
#[serde(untagged)]
enum UrlList {
    Single(String),
    Multiple(Vec<String>),
}
impl From<UrlList> for Urls {
    fn from(list: UrlList) -> Self {
        Self(match list {
            UrlList::Single(s) => vec![s],
            UrlList::Multiple(v) => v,
        })
    }
}
#[derive(Deserialize)]
struct Tracker {
    hash: String,
    #[serde(default)]
    std_hash: Option<String>,
    #[serde(default, rename = "std_hash_time")]
    _std_hash_time: Option<Number>,
    #[serde(default, rename = "is_hash_backup")]
    _is_hash_backup: Option<Number>,
    album_audio_id: Option<Number>,
    album_id: Option<Number>,
    url: Urls,
    #[serde(default, rename = "backupUrl")]
    backup_url: Urls,
    #[serde(rename = "timeLength")]
    time_length: Number,
    #[serde(rename = "fileSize")]
    file_size: Number,
    #[serde(rename = "bitRate")]
    bit_rate: Number,
    #[serde(rename = "extName")]
    extension: String,
    #[serde(default)]
    fail_process: Vec<String>,
    hash_offset: Option<Offset>,
}
#[derive(Deserialize)]
struct Offset {
    start_ms: Number,
    end_ms: Number,
    start_byte: Option<Number>,
    end_byte: Option<Number>,
}
fn invalid() -> TuneWeaveError {
    kugou_upstream_error("KuGou authorized media response is incomplete or inconsistent")
}
fn denied(message: &'static str) -> TuneWeaveError {
    TuneWeaveError::new(ErrorCode::PermissionDenied, message).with_platform(Platform::Kugou)
}

#[cfg(test)]
mod tests;
