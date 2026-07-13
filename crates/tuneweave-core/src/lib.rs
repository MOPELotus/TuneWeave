//! Platform-neutral domain types and provider interfaces for TuneWeave.

mod capability;
mod error;
mod model;
mod platform;
mod provider;
mod registry;

pub use capability::Capability;
pub use error::{ErrorCode, Result, TuneWeaveError};
pub use model::{
    AlbumSummary, ArtistSummary, Extensions, MediaStream, Page, PageMeta, Playlist,
    ProviderDescriptor, Quality, ResolutionAttempt, ResolutionStatus, SearchKind, SearchQuery,
    StreamRequest, Track, TrialWindow,
};
pub use platform::{ParsePlatformError, ParseResourceRefError, Platform, ResourceRef};
pub use provider::MusicProvider;
pub use registry::ProviderRegistry;
