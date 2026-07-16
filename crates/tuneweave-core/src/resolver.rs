use std::collections::BTreeSet;

use serde_json::json;

use crate::{
    Capability, ErrorCode, MatchAssessment, MediaStream, Platform, ProviderRegistry,
    ResolutionAttempt, ResolutionStatus, ResolveRequest, Result, SearchQuery, StreamRequest, Track,
    TuneWeaveError, assess_track_match,
};

const DEFAULT_STRICT_THRESHOLD: f64 = 0.82;
const DEFAULT_RELAXED_THRESHOLD: f64 = 0.65;
const DEFAULT_SEARCH_LIMIT: u32 = 10;

#[derive(Clone)]
pub struct StreamResolver {
    registry: ProviderRegistry,
    default_fallbacks: Vec<Platform>,
    strict_threshold: f64,
    relaxed_threshold: f64,
    search_limit: u32,
}

impl StreamResolver {
    #[must_use]
    pub fn new(registry: ProviderRegistry, default_fallbacks: Vec<Platform>) -> Self {
        Self {
            registry,
            default_fallbacks,
            strict_threshold: DEFAULT_STRICT_THRESHOLD,
            relaxed_threshold: DEFAULT_RELAXED_THRESHOLD,
            search_limit: DEFAULT_SEARCH_LIMIT,
        }
    }

    #[must_use]
    pub fn with_thresholds(mut self, strict: f64, relaxed: f64) -> Self {
        self.strict_threshold = strict.clamp(0.0, 1.0);
        self.relaxed_threshold = relaxed.clamp(0.0, 1.0);
        self
    }

    pub async fn resolve(&self, origin: &Track, request: &ResolveRequest) -> Result<MediaStream> {
        validate_origin(origin)?;
        let platforms = self.platform_sequence(origin.platform, request);
        let threshold = if request.strict_match {
            self.strict_threshold
        } else {
            self.relaxed_threshold
        };
        let mut attempts = Vec::new();
        let mut last_error = None;

        for platform in platforms.iter().copied() {
            let account = request.accounts.get(&platform).cloned();
            let Some(provider) = self.registry.get(platform) else {
                attempts.push(ResolutionAttempt {
                    platform,
                    account,
                    candidate: None,
                    match_score: None,
                    status: ResolutionStatus::Unavailable,
                    error: Some(format!("platform {platform} is not registered")),
                });
                continue;
            };

            let (candidate, match_score) = if platform == origin.platform {
                (origin.clone(), 1.0)
            } else {
                if !provider.supports(Capability::SearchTracks) {
                    let error = TuneWeaveError::unsupported(platform, Capability::SearchTracks);
                    attempts.push(failed_attempt(
                        platform,
                        account.clone(),
                        None,
                        None,
                        &error,
                    ));
                    last_error = Some(error);
                    continue;
                }
                let search = SearchQuery {
                    query: candidate_search_query(origin),
                    kind: crate::SearchKind::Track,
                    variant: crate::SearchVariant::Default,
                    limit: self.search_limit,
                    offset: 0,
                    account: account.clone(),
                };
                let page = match provider.search(&search).await {
                    Ok(page) => page,
                    Err(error) => {
                        attempts.push(failed_attempt(
                            platform,
                            account.clone(),
                            None,
                            None,
                            &error,
                        ));
                        last_error = Some(error);
                        continue;
                    }
                };
                let Some((candidate, assessment)) = best_candidate(origin, page.items, threshold)
                else {
                    let error = TuneWeaveError::new(
                        ErrorCode::MatchRejected,
                        format!("no {platform} candidate matched the origin track"),
                    )
                    .with_platform(platform)
                    .with_details(json!({ "origin_track": origin.resource_ref }));
                    attempts.push(failed_attempt(
                        platform,
                        account.clone(),
                        None,
                        None,
                        &error,
                    ));
                    last_error = Some(error);
                    continue;
                };
                if !assessment.accepted {
                    let error = TuneWeaveError::new(
                        ErrorCode::MatchRejected,
                        format!(
                            "best {platform} candidate scored {:.3} and was rejected",
                            assessment.score
                        ),
                    )
                    .with_platform(platform)
                    .with_details(json!({
                        "origin_track": origin.resource_ref,
                        "candidate": candidate.resource_ref,
                        "match_score": assessment.score,
                        "reasons": assessment.reasons
                    }));
                    attempts.push(failed_attempt(
                        platform,
                        account.clone(),
                        Some(candidate.resource_ref.clone()),
                        Some(assessment.score),
                        &error,
                    ));
                    last_error = Some(error);
                    continue;
                }
                (candidate, assessment.score)
            };

            if !provider.supports(Capability::AudioStream) {
                let error = TuneWeaveError::unsupported(platform, Capability::AudioStream);
                attempts.push(failed_attempt(
                    platform,
                    account.clone(),
                    Some(candidate.resource_ref.clone()),
                    Some(match_score),
                    &error,
                ));
                last_error = Some(error);
                continue;
            }

            let stream_request = StreamRequest {
                quality: request.quality,
                variant: request.variant,
                bitrate: request.bitrate,
                account: account.clone(),
            };
            match provider.stream(&candidate, &stream_request).await {
                Ok(mut stream) => {
                    attempts.push(ResolutionAttempt {
                        platform,
                        account,
                        candidate: Some(candidate.resource_ref.clone()),
                        match_score: Some(match_score),
                        status: ResolutionStatus::Success,
                        error: None,
                    });
                    stream.origin_track = Some(origin.resource_ref.clone());
                    stream.resolved_track = candidate.resource_ref.clone();
                    stream.resolved_platform = platform;
                    stream.match_score = Some(match_score);
                    stream.attempts = attempts;
                    return Ok(stream);
                }
                Err(error) => {
                    attempts.push(failed_attempt(
                        platform,
                        account,
                        Some(candidate.resource_ref.clone()),
                        Some(match_score),
                        &error,
                    ));
                    last_error = Some(error);
                }
            }
        }

        let mut error = last_error.unwrap_or_else(|| {
            TuneWeaveError::platform_unavailable(
                platforms.first().copied().unwrap_or(origin.platform),
            )
        });
        let cause = std::mem::take(&mut error.details);
        error.details = json!({
            "origin_track": origin.resource_ref,
            "attempts": attempts,
            "cause": cause
        });
        Err(error)
    }

    fn platform_sequence(&self, origin: Platform, request: &ResolveRequest) -> Vec<Platform> {
        let explicit = &request.playback_platforms;
        let first = explicit.first().copied().unwrap_or(origin);
        let mut candidates = vec![first];
        if request.fallback {
            if explicit.len() > 1 {
                candidates.extend(explicit.iter().skip(1).copied());
            } else {
                candidates.push(origin);
                candidates.extend(self.default_fallbacks.iter().copied());
            }
        }
        let mut seen = BTreeSet::new();
        candidates
            .into_iter()
            .filter(|platform| seen.insert(*platform))
            .collect()
    }
}

fn validate_origin(origin: &Track) -> Result<()> {
    if origin.platform != origin.resource_ref.platform() || origin.id != origin.resource_ref.id() {
        return Err(TuneWeaveError::invalid_request(
            "origin track platform, id, and reference must agree",
        )
        .with_details(json!({
            "platform": origin.platform,
            "id": origin.id,
            "track_ref": origin.resource_ref
        })));
    }
    Ok(())
}

fn candidate_search_query(origin: &Track) -> String {
    origin.artists.first().map_or_else(
        || origin.name.clone(),
        |artist| format!("{} {}", origin.name, artist.name),
    )
}

fn best_candidate(
    origin: &Track,
    candidates: Vec<Track>,
    threshold: f64,
) -> Option<(Track, MatchAssessment)> {
    candidates
        .into_iter()
        .map(|candidate| {
            let assessment = assess_track_match(origin, &candidate, threshold);
            (candidate, assessment)
        })
        .max_by(|(_, left), (_, right)| left.score.total_cmp(&right.score))
}

fn failed_attempt(
    platform: Platform,
    account: Option<String>,
    candidate: Option<crate::ResourceRef>,
    match_score: Option<f64>,
    error: &TuneWeaveError,
) -> ResolutionAttempt {
    ResolutionAttempt {
        platform,
        account,
        candidate,
        match_score,
        status: resolution_status(error.code),
        error: Some(error.message.clone()),
    }
}

fn resolution_status(code: ErrorCode) -> ResolutionStatus {
    match code {
        ErrorCode::AuthenticationRequired => ResolutionStatus::AuthenticationRequired,
        ErrorCode::PermissionDenied => ResolutionStatus::PermissionDenied,
        ErrorCode::MatchRejected => ResolutionStatus::NoMatch,
        ErrorCode::CapabilityNotSupported
        | ErrorCode::PlatformUnavailable
        | ErrorCode::ResourceNotFound => ResolutionStatus::Unavailable,
        ErrorCode::InvalidRequest
        | ErrorCode::Conflict
        | ErrorCode::RateLimited
        | ErrorCode::UpstreamError
        | ErrorCode::UpstreamTimeout
        | ErrorCode::InternalError => ResolutionStatus::UpstreamError,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use async_trait::async_trait;

    use crate::{ArtistSummary, Page, PageMeta, Quality, ResourceRef, StreamRequest, TrialWindow};

    use super::*;

    #[derive(Clone, Copy)]
    enum StreamBehavior {
        Success,
        AuthenticationRequired,
    }

    struct FakeProvider {
        platform: Platform,
        search_results: Vec<Track>,
        stream_behavior: StreamBehavior,
    }

    #[async_trait]
    impl crate::MusicProvider for FakeProvider {
        fn platform(&self) -> Platform {
            self.platform
        }

        fn name(&self) -> &'static str {
            "Fake provider"
        }

        fn capabilities(&self) -> BTreeSet<Capability> {
            BTreeSet::from([Capability::SearchTracks, Capability::AudioStream])
        }

        async fn search(&self, query: &SearchQuery) -> Result<Page<Track>> {
            Ok(Page {
                items: self.search_results.clone(),
                pagination: PageMeta {
                    limit: query.limit,
                    offset: query.offset,
                    total: Some(self.search_results.len() as u64),
                    next_offset: None,
                    has_more: false,
                    extensions: Default::default(),
                },
            })
        }

        async fn stream(&self, track: &Track, request: &StreamRequest) -> Result<MediaStream> {
            match self.stream_behavior {
                StreamBehavior::Success => Ok(fake_stream(track, request)),
                StreamBehavior::AuthenticationRequired => Err(TuneWeaveError::new(
                    ErrorCode::AuthenticationRequired,
                    "account is required",
                )
                .with_platform(self.platform)),
            }
        }
    }

    fn track(platform: Platform, id: &str, name: &str, artist: &str) -> Track {
        let mut track = Track::new(
            ResourceRef::new(platform, id).expect("valid reference"),
            name,
        );
        track.artists.push(ArtistSummary {
            resource_ref: None,
            name: artist.to_owned(),
        });
        track.duration_ms = Some(258_000);
        track
    }

    fn fake_stream(track: &Track, request: &StreamRequest) -> MediaStream {
        MediaStream {
            url: format!("https://{}.example.test/{}.flac", track.platform, track.id),
            backup_urls: Vec::new(),
            headers: BTreeMap::new(),
            expires_at: None,
            format: Some("flac".to_owned()),
            codec: Some("flac".to_owned()),
            bitrate: request.bitrate.or(Some(999_000)),
            size: None,
            duration_ms: track.duration_ms,
            requested_quality: request.quality,
            actual_quality: Quality::Lossless,
            trial: None::<TrialWindow>,
            origin_track: Some(track.resource_ref.clone()),
            resolved_track: track.resource_ref.clone(),
            resolved_platform: track.platform,
            match_score: None,
            attempts: Vec::new(),
        }
    }

    #[tokio::test]
    async fn falls_back_from_netease_auth_failure_to_matched_qq_track() {
        let origin = track(Platform::Netease, "185809", "反方向的钟", "周杰伦");
        let qq_track = track(Platform::Qq, "0039mid", "反方向的钟", "周杰伦");
        let mut registry = ProviderRegistry::new();
        registry
            .register(FakeProvider {
                platform: Platform::Netease,
                search_results: Vec::new(),
                stream_behavior: StreamBehavior::AuthenticationRequired,
            })
            .expect("register NetEase");
        registry
            .register(FakeProvider {
                platform: Platform::Qq,
                search_results: vec![qq_track],
                stream_behavior: StreamBehavior::Success,
            })
            .expect("register QQ");
        let resolver = StreamResolver::new(registry, vec![Platform::Qq]);
        let mut request = ResolveRequest {
            quality: Quality::Lossless,
            bitrate: Some(192_123),
            ..ResolveRequest::default()
        };
        request
            .accounts
            .insert(Platform::Qq, "green-diamond".to_owned());

        let stream = resolver.resolve(&origin, &request).await.expect("fallback");
        assert_eq!(stream.origin_track, Some(origin.resource_ref));
        assert_eq!(stream.resolved_track.to_string(), "qq:0039mid");
        assert_eq!(stream.resolved_platform, Platform::Qq);
        assert_eq!(stream.bitrate, Some(192_123));
        assert_eq!(stream.attempts.len(), 2);
        assert_eq!(
            stream.attempts[0].status,
            ResolutionStatus::AuthenticationRequired
        );
        assert_eq!(stream.attempts[1].status, ResolutionStatus::Success);
        assert_eq!(stream.attempts[1].account.as_deref(), Some("green-diamond"));
    }

    #[tokio::test]
    async fn rejects_cover_and_returns_to_origin_platform() {
        let origin = track(Platform::Netease, "185809", "反方向的钟", "周杰伦");
        let cover = track(Platform::Qq, "cover", "反方向的钟", "夏蔓蔓");
        let mut registry = ProviderRegistry::new();
        registry
            .register(FakeProvider {
                platform: Platform::Netease,
                search_results: Vec::new(),
                stream_behavior: StreamBehavior::Success,
            })
            .expect("register NetEase");
        registry
            .register(FakeProvider {
                platform: Platform::Qq,
                search_results: vec![cover],
                stream_behavior: StreamBehavior::Success,
            })
            .expect("register QQ");
        let resolver = StreamResolver::new(registry, vec![Platform::Netease]);
        let request = ResolveRequest {
            playback_platforms: vec![Platform::Qq],
            ..ResolveRequest::default()
        };

        let stream = resolver
            .resolve(&origin, &request)
            .await
            .expect("origin fallback");
        assert_eq!(stream.resolved_platform, Platform::Netease);
        assert_eq!(stream.attempts[0].status, ResolutionStatus::NoMatch);
        assert_eq!(stream.attempts[1].status, ResolutionStatus::Success);
    }

    #[tokio::test]
    async fn fallback_false_stops_after_the_preferred_platform() {
        let origin = track(Platform::Netease, "185809", "反方向的钟", "周杰伦");
        let mut registry = ProviderRegistry::new();
        registry
            .register(FakeProvider {
                platform: Platform::Netease,
                search_results: Vec::new(),
                stream_behavior: StreamBehavior::AuthenticationRequired,
            })
            .expect("register NetEase");
        let resolver = StreamResolver::new(registry, vec![Platform::Qq]);
        let request = ResolveRequest {
            fallback: false,
            ..ResolveRequest::default()
        };

        let error = resolver
            .resolve(&origin, &request)
            .await
            .expect_err("must stop");
        assert_eq!(error.code, ErrorCode::AuthenticationRequired);
        assert_eq!(error.details["attempts"].as_array().map(Vec::len), Some(1));
    }
}
