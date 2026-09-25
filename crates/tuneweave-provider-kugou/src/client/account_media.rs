//! Account tracker responses are mapped independently from anonymous privilege responses.
use super::dto::Number;
use super::*;
use crate::account::media::{Behavior, Status, TrackerResponse};

pub(crate) struct Selection {
    track: Track,
    spec: SelectedMediaSpec,
    pub(crate) album_id: u64,
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
        let status = Status::parse(&response.bytes)?;
        if status.status != 1 || status.code() != 0 {
            return Err(status.rejection());
        }
        let data: Tracker = serde_json::from_slice(&response.bytes).map_err(|_| invalid())?;
        if !data.hash.eq_ignore_ascii_case(&self.spec.hash)
            || data
                .album_audio_id
                .is_some_and(|id| id.0.to_string() != self.track.id)
            || data.album_id.is_some_and(|id| id.0 != self.album_id)
        {
            return Err(invalid());
        }
        if !data.fail_process.is_empty() {
            return Err(denied("KuGou media requires additional authorization"));
        }
        let expected_duration = self
            .spec
            .duration_ms
            .filter(|n| *n > 0)
            .or(self.track.duration_ms.filter(|n| *n > 0))
            .ok_or_else(invalid)?;
        let duration = data
            .time_length
            .0
            .checked_mul(1000)
            .filter(|n| *n > 0)
            .ok_or_else(invalid)?;
        let size = data.file_size.0;
        let bitrate = data.bit_rate.0;
        if size == 0 || bitrate == 0 {
            return Err(invalid());
        }
        let trial = match data.hash_offset {
            None => None,
            Some(offset) => {
                let start = offset.start_ms.0;
                let end = offset.end_ms.0;
                if end <= start || end > expected_duration.saturating_add(999) {
                    return Err(invalid());
                }
                match (offset.start_byte, offset.end_byte) {
                    (Some(start), Some(end)) if end.0 >= start.0 => {
                        let length = end
                            .0
                            .checked_sub(start.0)
                            .and_then(|n| n.checked_add(1))
                            .ok_or_else(invalid)?;
                        if size < length {
                            return Err(invalid());
                        }
                    }
                    (None, None) => {}
                    _ => return Err(invalid()),
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
            return Err(invalid());
        }
        if let Some(window) = &trial {
            if duration.abs_diff(window.end_ms - window.start_ms) > 999
                && duration.abs_diff(expected_duration) > 999
            {
                return Err(invalid());
            }
            if behavior == Behavior::Download {
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
                return Err(denied(
                    "KuGou returned an unsupported or encrypted media format",
                ));
            }
        };
        let mut urls = Vec::new();
        let mut seen = BTreeSet::new();
        for value in data.url.0.into_iter().chain(data.backup_url.0) {
            if value.len() > 8192 || value.chars().any(char::is_control) {
                return Err(invalid());
            }
            let url = normalize_media_url(&value)?;
            response.check_url(&url)?;
            if seen.insert(url.clone()) {
                urls.push(url);
            }
        }
        if urls.is_empty() || urls.len() > 16 {
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
