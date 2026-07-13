use std::{collections::BTreeMap, sync::Arc};

use crate::{ErrorCode, MusicProvider, Platform, ProviderDescriptor, Result, TuneWeaveError};

/// Startup-time registry for platform adapters.
#[derive(Clone, Default)]
pub struct ProviderRegistry {
    providers: BTreeMap<Platform, Arc<dyn MusicProvider>>,
}

impl ProviderRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register<P>(&mut self, provider: P) -> Result<()>
    where
        P: MusicProvider + 'static,
    {
        self.register_arc(Arc::new(provider))
    }

    pub fn register_arc(&mut self, provider: Arc<dyn MusicProvider>) -> Result<()> {
        let platform = provider.platform();
        if self.providers.contains_key(&platform) {
            return Err(TuneWeaveError::new(
                ErrorCode::Conflict,
                format!("provider {platform} is already registered"),
            )
            .with_platform(platform));
        }

        self.providers.insert(platform, provider);
        Ok(())
    }

    #[must_use]
    pub fn get(&self, platform: Platform) -> Option<Arc<dyn MusicProvider>> {
        self.providers.get(&platform).cloned()
    }

    pub fn require(&self, platform: Platform) -> Result<Arc<dyn MusicProvider>> {
        self.get(platform)
            .ok_or_else(|| TuneWeaveError::platform_unavailable(platform))
    }

    #[must_use]
    pub fn contains(&self, platform: Platform) -> bool {
        self.providers.contains_key(&platform)
    }

    #[must_use]
    pub fn descriptors(&self) -> Vec<ProviderDescriptor> {
        self.providers
            .values()
            .map(|provider| provider.descriptor())
            .collect()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.providers.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.providers.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use async_trait::async_trait;

    use super::*;
    use crate::Capability;

    struct TestProvider;

    #[async_trait]
    impl MusicProvider for TestProvider {
        fn platform(&self) -> Platform {
            Platform::Netease
        }

        fn name(&self) -> &'static str {
            "Test NetEase"
        }

        fn capabilities(&self) -> BTreeSet<Capability> {
            BTreeSet::from([Capability::SearchTracks])
        }
    }

    #[test]
    fn registers_and_discovers_provider() {
        let mut registry = ProviderRegistry::new();
        registry.register(TestProvider).expect("register provider");

        let provider = registry
            .require(Platform::Netease)
            .expect("registered provider");
        assert_eq!(provider.name(), "Test NetEase");
        assert!(provider.supports(Capability::SearchTracks));
        assert_eq!(registry.descriptors().len(), 1);
    }

    #[test]
    fn rejects_duplicate_platforms() {
        let mut registry = ProviderRegistry::new();
        registry.register(TestProvider).expect("register provider");
        let error = registry
            .register(TestProvider)
            .expect_err("duplicate must fail");
        assert_eq!(error.code, ErrorCode::Conflict);
    }
}
