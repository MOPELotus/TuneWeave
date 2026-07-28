use std::collections::BTreeSet;

use async_trait::async_trait;
use tuneweave_core::{
    Capability, MusicProvider, Platform, ProviderQrStart, Result, TuneWeaveError,
};

use crate::client::{BilibiliClient, BilibiliConfig};

#[derive(Clone)]
pub struct BilibiliProvider {
    client: BilibiliClient,
}

impl BilibiliProvider {
    pub fn new(config: BilibiliConfig) -> Result<Self> {
        Ok(Self {
            client: BilibiliClient::new(&config)?,
        })
    }

    #[must_use]
    pub fn from_client(client: BilibiliClient) -> Self {
        Self { client }
    }
}

#[async_trait]
impl MusicProvider for BilibiliProvider {
    fn platform(&self) -> Platform {
        Platform::Bilibili
    }

    fn name(&self) -> &'static str {
        "Bilibili"
    }

    fn capabilities(&self) -> BTreeSet<Capability> {
        BTreeSet::new()
    }

    async fn start_qr_login(&self, login_type: Option<&str>) -> Result<ProviderQrStart> {
        if let Some(login_type) = login_type.map(str::trim).filter(|value| !value.is_empty())
            && !matches!(login_type, "default" | "web" | "bilibili")
        {
            return Err(TuneWeaveError::invalid_request(format!(
                "unsupported Bilibili QR login type: {login_type}"
            ))
            .with_platform(Platform::Bilibili));
        }
        let start = self.client.create_qr_login().await?;
        Ok(ProviderQrStart {
            provider_transaction_id: start.qrcode_key,
            url: start.image_data_url.clone(),
            image_data_url: Some(start.image_data_url),
            expires_at: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore = "requires live Bilibili Passport access"]
    async fn live_provider_creates_a_qr_image_without_exposing_the_poll_key() {
        let provider = BilibiliProvider::new(BilibiliConfig::default()).expect("provider");
        let start = provider
            .start_qr_login(None)
            .await
            .expect("provider QR start");
        assert_eq!(start.provider_transaction_id.len(), 32);
        assert!(start.url.starts_with("data:image/svg+xml;base64,"));
        assert_eq!(start.image_data_url.as_deref(), Some(start.url.as_str()));
    }
}
