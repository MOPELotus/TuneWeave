use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use crate::Track;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct MatchAssessment {
    pub score: f64,
    pub accepted: bool,
    pub hard_rejected: bool,
    pub reasons: Vec<String>,
}

/// Conservatively determines whether two provider tracks describe the same recording.
#[must_use]
pub fn assess_track_match(origin: &Track, candidate: &Track, threshold: f64) -> MatchAssessment {
    let mut weighted_score = 0.0;
    let mut total_weight = 0.0;
    let mut hard_rejected = false;
    let mut reasons = Vec::new();

    let title_score = text_similarity(&origin.name, &candidate.name);
    add_feature(&mut weighted_score, &mut total_weight, title_score, 0.50);
    if title_score < 0.60 {
        hard_rejected = true;
        reasons.push(format!("title similarity is too low ({title_score:.3})"));
    }

    let origin_artists = origin
        .artists
        .iter()
        .map(|artist| artist.name.as_str())
        .filter(|artist| !artist.trim().is_empty())
        .collect::<Vec<_>>();
    let candidate_artists = candidate
        .artists
        .iter()
        .map(|artist| artist.name.as_str())
        .filter(|artist| !artist.trim().is_empty())
        .collect::<Vec<_>>();
    if !origin_artists.is_empty() {
        let artist_score = best_text_match(&origin_artists, &candidate_artists);
        add_feature(&mut weighted_score, &mut total_weight, artist_score, 0.25);
        if artist_score < 0.60 {
            hard_rejected = true;
            reasons.push(format!(
                "primary artist similarity is too low ({artist_score:.3})"
            ));
        }
    }

    if let (Some(origin_album), Some(candidate_album)) = (&origin.album, &candidate.album) {
        let album_score = text_similarity(&origin_album.name, &candidate_album.name);
        add_feature(&mut weighted_score, &mut total_weight, album_score, 0.10);
        if album_score < 0.45 {
            reasons.push(format!("album similarity is low ({album_score:.3})"));
        }
    }

    if let (Some(origin_duration), Some(candidate_duration)) =
        (origin.duration_ms, candidate.duration_ms)
    {
        let difference = origin_duration.abs_diff(candidate_duration);
        let duration_score = duration_similarity(difference);
        add_feature(&mut weighted_score, &mut total_weight, duration_score, 0.15);
        if difference > 15_000 {
            hard_rejected = true;
            reasons.push(format!("duration differs by {difference} ms"));
        } else if difference > 5_000 {
            reasons.push(format!("duration differs by {difference} ms"));
        }
    }

    if let (Some(origin_isrc), Some(candidate_isrc)) = (&origin.isrc, &candidate.isrc) {
        let isrc_matches = origin_isrc
            .trim()
            .eq_ignore_ascii_case(candidate_isrc.trim());
        add_feature(
            &mut weighted_score,
            &mut total_weight,
            f64::from(isrc_matches),
            0.50,
        );
        if !isrc_matches {
            hard_rejected = true;
            reasons.push("ISRC values do not match".to_owned());
        }
    }

    let origin_versions = version_tags(origin);
    let candidate_versions = version_tags(candidate);
    if origin_versions != candidate_versions {
        hard_rejected = true;
        reasons.push(format!(
            "version tags differ (origin: {}, candidate: {})",
            display_tags(&origin_versions),
            display_tags(&candidate_versions)
        ));
    }

    let score = if total_weight > 0.0 {
        (weighted_score / total_weight).clamp(0.0, 1.0)
    } else {
        0.0
    };
    if score < threshold {
        reasons.push(format!(
            "aggregate score {score:.3} is below threshold {threshold:.3}"
        ));
    }

    MatchAssessment {
        score,
        accepted: !hard_rejected && score >= threshold,
        hard_rejected,
        reasons,
    }
}

fn add_feature(total: &mut f64, weight_total: &mut f64, score: f64, weight: f64) {
    *total += score * weight;
    *weight_total += weight;
}

fn best_text_match(left: &[&str], right: &[&str]) -> f64 {
    left.iter()
        .flat_map(|left| right.iter().map(move |right| text_similarity(left, right)))
        .fold(0.0, f64::max)
}

fn text_similarity(left: &str, right: &str) -> f64 {
    let left = normalized_chars(left);
    let right = normalized_chars(right);
    if left.is_empty() || right.is_empty() {
        return 0.0;
    }
    if left == right {
        return 1.0;
    }
    if left.len() == 1 || right.len() == 1 {
        return 0.0;
    }

    let left_pairs = bigram_counts(&left);
    let right_pairs = bigram_counts(&right);
    let overlap = left_pairs
        .iter()
        .map(|(pair, left_count)| {
            right_pairs
                .get(pair)
                .map_or(0, |right_count| (*left_count).min(*right_count))
        })
        .sum::<usize>();
    let left_total = left_pairs.values().sum::<usize>();
    let right_total = right_pairs.values().sum::<usize>();
    (2 * overlap) as f64 / (left_total + right_total) as f64
}

fn normalized_chars(value: &str) -> Vec<char> {
    value
        .chars()
        .flat_map(char::to_lowercase)
        .filter(|character| character.is_alphanumeric())
        .collect()
}

fn bigram_counts(characters: &[char]) -> BTreeMap<(char, char), usize> {
    let mut counts = BTreeMap::new();
    for pair in characters.windows(2) {
        *counts.entry((pair[0], pair[1])).or_default() += 1;
    }
    counts
}

fn duration_similarity(difference_ms: u64) -> f64 {
    match difference_ms {
        0..=1_500 => 1.0,
        1_501..=3_000 => 0.95,
        3_001..=5_000 => 0.80,
        5_001..=10_000 => 0.50,
        10_001..=15_000 => 0.20,
        _ => 0.0,
    }
}

fn version_tags(track: &Track) -> BTreeSet<&'static str> {
    const TAGS: [(&str, &str); 16] = [
        ("live", "live"),
        ("现场", "live"),
        ("remix", "remix"),
        ("伴奏", "instrumental"),
        ("instrumental", "instrumental"),
        ("纯音乐", "instrumental"),
        ("翻唱", "cover"),
        ("cover", "cover"),
        ("acoustic", "acoustic"),
        ("unplugged", "acoustic"),
        ("demo", "demo"),
        ("sped up", "speed"),
        ("slowed", "speed"),
        ("nightcore", "speed"),
        ("remaster", "remaster"),
        ("重制", "remaster"),
    ];
    let title = std::iter::once(track.name.as_str())
        .chain(track.aliases.iter().map(String::as_str))
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
    TAGS.into_iter()
        .filter_map(|(needle, tag)| title.contains(needle).then_some(tag))
        .collect()
}

fn display_tags(tags: &BTreeSet<&str>) -> String {
    if tags.is_empty() {
        "none".to_owned()
    } else {
        tags.iter().copied().collect::<Vec<_>>().join(",")
    }
}

#[cfg(test)]
mod tests {
    use crate::{ArtistSummary, Platform, ResourceRef, Track};

    use super::*;

    fn track(platform: Platform, id: &str, name: &str, artist: &str, duration_ms: u64) -> Track {
        let mut track = Track::new(
            ResourceRef::new(platform, id).expect("valid reference"),
            name,
        );
        track.artists.push(ArtistSummary {
            resource_ref: None,
            name: artist.to_owned(),
        });
        track.duration_ms = Some(duration_ms);
        track
    }

    #[test]
    fn accepts_same_recording_across_platforms() {
        let origin = track(Platform::Netease, "1", "反方向的钟", "周杰伦", 258_000);
        let candidate = track(Platform::Qq, "mid", "反方向的钟", "周杰伦", 258_800);
        let assessment = assess_track_match(&origin, &candidate, 0.82);
        assert!(assessment.accepted);
        assert!(assessment.score > 0.98);
    }

    #[test]
    fn rejects_same_title_by_different_artist() {
        let origin = track(Platform::Netease, "1", "反方向的钟", "周杰伦", 258_000);
        let cover = track(Platform::Qq, "cover", "反方向的钟", "夏蔓蔓", 258_000);
        let assessment = assess_track_match(&origin, &cover, 0.82);
        assert!(!assessment.accepted);
        assert!(assessment.hard_rejected);
        assert!(
            assessment
                .reasons
                .iter()
                .any(|reason| reason.contains("artist"))
        );
    }

    #[test]
    fn rejects_live_or_remix_version_mismatch() {
        let origin = track(Platform::Netease, "1", "晴天", "周杰伦", 269_000);
        let live = track(Platform::Qq, "live", "晴天 (Live)", "周杰伦", 270_000);
        let assessment = assess_track_match(&origin, &live, 0.70);
        assert!(!assessment.accepted);
        assert!(
            assessment
                .reasons
                .iter()
                .any(|reason| reason.contains("version tags"))
        );
    }

    #[test]
    fn rejects_conflicting_isrc_even_when_metadata_matches() {
        let mut origin = track(Platform::Netease, "1", "晴天", "周杰伦", 269_000);
        let mut candidate = track(Platform::Qq, "mid", "晴天", "周杰伦", 269_000);
        origin.isrc = Some("TW-A53-03-00001".to_owned());
        candidate.isrc = Some("TW-A53-03-99999".to_owned());
        let assessment = assess_track_match(&origin, &candidate, 0.82);
        assert!(!assessment.accepted);
        assert!(assessment.hard_rejected);
    }

    #[test]
    fn tolerates_punctuation_and_small_duration_drift() {
        let origin = track(Platform::Netease, "1", "青花瓷", "周杰伦", 239_000);
        let candidate = track(Platform::Qq, "mid", "青花瓷！", "周杰伦", 241_000);
        assert!(assess_track_match(&origin, &candidate, 0.82).accepted);
    }
}
