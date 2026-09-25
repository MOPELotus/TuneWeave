//! Selected-account native playlist directories. No cloud sync or library writes.
use super::*;
use std::collections::BTreeSet;
use tuneweave_core::{Page, PageMeta, PageRequest, ProviderCredential};

pub(super) mod dto;
#[cfg(test)]
pub(crate) mod tests;
pub(super) const OWNED_PATH: &str = "/pl.svc";
pub(super) const SAVED_PATH: &str = "/openapi/v1/user/quanzi";
pub(super) const MAX_RESPONSE: usize = 2 * 1024 * 1024;
const MAX_LIBRARY_BYTES: usize = 8 * 1024 * 1024;
pub(in crate::client::native) const MAX_OWNED: usize = 4096;
const PAGE_SIZE: usize = 20;
const MAX_PAGES: usize = 100;
// The last physical page must be short (possibly empty) to prove completion.
pub(in crate::client::native) const MAX_SAVED: usize = PAGE_SIZE * MAX_PAGES - 1;

#[derive(Clone, Copy, Eq, PartialEq)]
pub(crate) enum Section {
    Created,
    Saved,
    Favorite,
    /// Internal lookup only; public created directories still exclude favorites.
    Owned,
}
impl Section {
    fn name(self) -> &'static str {
        match self {
            Self::Created => "created",
            Self::Saved => "collected",
            Self::Favorite => "favorite",
            Self::Owned => "owned",
        }
    }
}

impl KuwoClient {
    /// Independently validates the native account and reads ordinary created then
    /// collected playlists. This is a directory, not playlist contents or Uni import.
    pub async fn native_account_playlists(
        &self,
        credential: &ProviderCredential,
        request: &PageRequest,
    ) -> Result<Page<Playlist>> {
        self.native_library(credential, request, None).await
    }
    /// Reads the current native account's ordinary created playlist directory.
    pub async fn native_created_playlists(
        &self,
        credential: &ProviderCredential,
        request: &PageRequest,
    ) -> Result<Page<Playlist>> {
        self.native_library(credential, request, Some(Section::Created))
            .await
    }
    /// Reads all physical collection pages before applying the requested window.
    pub async fn native_collected_playlists(
        &self,
        credential: &ProviderCredential,
        request: &PageRequest,
    ) -> Result<Page<Playlist>> {
        self.native_library(credential, request, Some(Section::Saved))
            .await
    }
    async fn native_library(
        &self,
        credential: &ProviderCredential,
        request: &PageRequest,
        section: Option<Section>,
    ) -> Result<Page<Playlist>> {
        validate_request(request)?;
        if request.account.as_deref().is_some_and(|v| v != "default") {
            return Err(kuwo_invalid_request(
                "Kuwo SDK credentials cannot select a stored account",
            ));
        }
        let input = credential::NativeCredential::parse(credential)?.input()?;
        validate_session_metadata(&input)?;
        self.validate_native_session(&input).await?;
        self.fetch_native_library(&input, request, section, || Ok(()))
            .await
    }

    pub(crate) async fn fetch_native_library(
        &self,
        input: &KuwoNativeSessionInput,
        request: &PageRequest,
        section: Option<Section>,
        mut check: impl FnMut() -> Result<()> + Send,
    ) -> Result<Page<Playlist>> {
        tokio::time::timeout(Duration::from_secs(60), async {
            let (all, pages) = self
                .native_library_items(input, section, &mut check)
                .await?;
            check()?;
            let total = all.len() as u64;
            let items: Vec<_> = all
                .into_iter()
                .skip(request.offset as usize)
                .take(request.limit as usize)
                .collect();
            let end = request.offset + items.len() as u32;
            let has_more = u64::from(end) < total;
            Ok(Page {
                items,
                pagination: PageMeta {
                    limit: request.limit,
                    offset: request.offset,
                    total: Some(total),
                    has_more,
                    next_offset: has_more.then_some(end),
                    extensions: Extensions::from([
                        ("backend".into(), json!("native_account_library")),
                        ("library_owner_id".into(), json!(input.user_id())),
                        (
                            "library_section".into(),
                            json!(section.map_or("all", Section::name)),
                        ),
                        ("complete_read".into(), json!(true)),
                        ("consistency".into(), json!("single_complete_traversal")),
                        ("upstream_pages_fetched".into(), json!(pages)),
                        ("collected_page_size".into(), json!(PAGE_SIZE)),
                    ]),
                },
            })
        })
        .await
        .map_err(|_| {
            TuneWeaveError::new(
                ErrorCode::UpstreamTimeout,
                "Kuwo native library read timed out",
            )
            .with_platform(Platform::Kuwo)
        })?
    }

    pub(super) async fn native_library_items(
        &self,
        input: &KuwoNativeSessionInput,
        section: Option<Section>,
        check: &mut (impl FnMut() -> Result<()> + Send),
    ) -> Result<(Vec<Playlist>, u64)> {
        let mut all = Vec::new();
        let mut bytes = 0;
        let mut pages = 0;
        let sections = section.map_or_else(|| vec![Section::Created, Section::Saved], |s| vec![s]);
        for selected in sections {
            let mut seen = BTreeSet::new();
            let mut start = 0;
            for index in 0..MAX_PAGES {
                check()?;
                let response = self.native_library_page(input, selected, start).await;
                // Check late failures as well as successes before using this page.
                check()?;
                let page = response?;
                pages += 1;
                bytes += serde_json::to_vec(&page).map_err(|_| invalid())?.len();
                if bytes > MAX_LIBRARY_BYTES {
                    return Err(invalid());
                }
                for item in &page {
                    if !seen.insert(item.id.clone()) {
                        return Err(invalid());
                    }
                }
                let count = page.len();
                all.extend(page);
                if selected != Section::Saved || count < PAGE_SIZE {
                    break;
                }
                if index + 1 == MAX_PAGES {
                    return Err(invalid());
                }
                start += count as u32;
            }
        }
        check()?;
        Ok((all, pages))
    }

    pub(super) async fn native_library_page(
        &self,
        input: &KuwoNativeSessionInput,
        section: Section,
        start: u32,
    ) -> Result<Vec<Playlist>> {
        let metadata = session_metadata(input)?;
        let (host, path) = match section {
            Section::Created | Section::Favorite | Section::Owned => {
                ("nplserver.kuwo.cn", OWNED_PATH)
            }
            Section::Saved => ("wapi.kuwo.cn", SAVED_PATH),
        };
        let time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| invalid())?
            .as_millis()
            .to_string();
        let query = query(input, section, start, &time)?;
        let target = format!("{}?{query}", self.native_target(host, path));
        self.native_get_with_metadata(
            host,
            path,
            "native_library",
            target,
            Some(metadata),
            |bytes| dto::parse(bytes, input, section),
        )
        .await
    }
}

pub(crate) fn validate_request(request: &PageRequest) -> Result<()> {
    if !(1..=100).contains(&request.limit) || request.offset.checked_add(request.limit).is_none() {
        return Err(kuwo_invalid_request("Kuwo library pagination is invalid"));
    }
    Ok(())
}
pub(super) fn query(
    input: &KuwoNativeSessionInput,
    section: Section,
    start: u32,
    time: &str,
) -> Result<String> {
    let mut q = url::form_urlencoded::Serializer::new(String::new());
    match section {
        Section::Created | Section::Favorite | Section::Owned => {
            q.extend_pairs([
                ("op", "pl3_getuserlists"),
                ("recommend", "1"),
                ("uid", input.user_id()),
                ("sid", input.session_id()),
                ("encode", "utf-8"),
                ("plat", "ar"),
                ("devid", input.device_id()),
                ("user", input.device_user()),
                ("prod", "kwplayer_ar_12.2.2.0"),
                ("source", CLIENT_SOURCE),
                ("corp", "kuwo"),
                ("locationid", "1"),
                ("approval", "false"),
                ("city", ""),
                ("province", ""),
                ("imei", input.device_user()),
                ("ttime", time),
                ("devicetype", "TuneWeave"),
            ]);
        }
        Section::Saved => {
            let context = input.context.as_ref().ok_or_else(|| {
                kuwo_invalid_request("Kuwo library requires its native installation context")
            })?;
            q.extend_pairs([
                ("user", input.device_user()),
                ("android_id", context.android_id.as_str()),
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
            ]);
        }
    }
    // SDK privacy/device defaults; these flags do not assert account membership.
    q.extend_pairs([
        ("allpay", "0"),
        ("notrace", "1"),
        ("oaid", ""),
        ("vipMode", "0"),
    ]);
    if section == Section::Saved {
        q.extend_pairs([
            ("f", "web"),
            ("type", "get_like_sl"),
            ("uid", input.user_id()),
            ("count", "20"),
            ("start", &start.to_string()),
        ]);
    }
    Ok(q.finish())
}
fn invalid() -> TuneWeaveError {
    kuwo_upstream_error("Kuwo native library response is invalid or exceeds its read budget")
}
