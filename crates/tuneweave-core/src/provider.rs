use std::collections::BTreeSet;

use async_trait::async_trait;

use crate::{
    Capability, MediaStream, Page, Platform, ProviderDescriptor, Result, SearchQuery,
    StreamRequest, Track, TuneWeaveError,
};

/// A dynamically registered music platform adapter.
#[async_trait]
pub trait MusicProvider: Send + Sync {
    fn platform(&self) -> Platform;

    fn name(&self) -> &'static str;

    fn capabilities(&self) -> BTreeSet<Capability>;

    fn descriptor(&self) -> ProviderDescriptor {
        ProviderDescriptor {
            platform: self.platform(),
            name: self.name().to_owned(),
            capabilities: self.capabilities().into_iter().collect(),
        }
    }

    fn supports(&self, capability: Capability) -> bool {
        self.capabilities().contains(&capability)
    }

    async fn search(&self, _query: &SearchQuery) -> Result<Page<Track>> {
        Err(TuneWeaveError::unsupported(
            self.platform(),
            Capability::SearchTracks,
        ))
    }

    async fn track(&self, _id: &str, _account: Option<&str>) -> Result<Track> {
        Err(TuneWeaveError::unsupported(
            self.platform(),
            Capability::TrackDetail,
        ))
    }

    async fn stream(&self, _track: &Track, _request: &StreamRequest) -> Result<MediaStream> {
        Err(TuneWeaveError::unsupported(
            self.platform(),
            Capability::AudioStream,
        ))
    }
}
