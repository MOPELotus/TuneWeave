use serde::{Deserialize, Serialize};

use crate::{Quality, ResourceRef, Result, TuneWeaveError};

/// One actual listening session, reported when the caller decides to submit it.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ScrobbleRequest {
    /// Actual listening time in milliseconds, excluding pauses and seeks.
    pub played_ms: u64,
    /// Total track length in milliseconds.
    pub duration_ms: u64,
    /// Actual media bitrate in bits per second (not kilobits per second).
    pub bitrate: u64,
    /// Actual playback quality; automatic selection is not a measured quality.
    pub quality: Quality,
    pub account: Option<String>,
}

impl ScrobbleRequest {
    pub fn validate(&self) -> Result<()> {
        if self.played_ms == 0 || self.duration_ms == 0 || self.played_ms > self.duration_ms {
            return Err(TuneWeaveError::invalid_request(
                "scrobble requires 0 < played_ms <= duration_ms; report repeated plays separately",
            ));
        }
        // Keep integer millisecond values exact when converted to upstream JSON seconds.
        if self.duration_ms > u64::from(u32::MAX) * 1000 {
            return Err(TuneWeaveError::invalid_request("duration_ms is too large"));
        }
        if self.bitrate == 0 || self.bitrate > u64::from(u32::MAX) {
            return Err(TuneWeaveError::invalid_request(
                "bitrate must be a positive 32-bit value in bit/s",
            ));
        }
        if self.quality == Quality::Auto {
            return Err(TuneWeaveError::invalid_request(
                "scrobble quality must be the actual playback quality, not auto",
            ));
        }
        Ok(())
    }
}

/// Acknowledges upstream acceptance, not an eventual listening-chart increment.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ScrobbleResult {
    pub track_ref: ResourceRef,
    pub accepted: bool,
    pub played_ms: u64,
    pub duration_ms: u64,
    pub bitrate: u64,
    pub quality: Quality,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn validates_measured_playback_and_rejects_unknown_or_missing_fields() {
        let good =
            json!({"played_ms":90123,"duration_ms":210456,"bitrate":320000,"quality":"high"});
        let request: ScrobbleRequest = serde_json::from_value(good.clone()).unwrap();
        request.validate().unwrap();
        assert_eq!(request.played_ms, 90123);
        for (field, value) in [
            ("played_ms", json!(0)),
            ("played_ms", json!(210457)),
            ("duration_ms", json!(0)),
            ("duration_ms", json!(u64::MAX)),
            ("bitrate", json!(0)),
            ("bitrate", json!(u64::MAX)),
            ("quality", json!("auto")),
        ] {
            let mut bad = good.clone();
            bad[field] = value;
            assert!(
                serde_json::from_value::<ScrobbleRequest>(bad)
                    .unwrap()
                    .validate()
                    .is_err()
            );
        }
        for field in ["played_ms", "duration_ms", "bitrate", "quality"] {
            let mut bad = good.clone();
            bad.as_object_mut().unwrap().remove(field);
            assert!(serde_json::from_value::<ScrobbleRequest>(bad).is_err());
        }
        for (field, value) in [
            ("played_ms", json!(-1)),
            ("played_ms", json!(1.5)),
            ("cookie", json!("secret")),
        ] {
            let mut bad = good.clone();
            bad[field] = value;
            assert!(serde_json::from_value::<ScrobbleRequest>(bad).is_err());
        }
    }
}
