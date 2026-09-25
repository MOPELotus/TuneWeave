//! Selected-account created and collected playlists. No cloud synchronization.
use super::*;
use library::{Section, validate_request};
use sha1::{Digest, Sha1};
use tuneweave_core::{Page, PageMeta, PageRequest, ProviderCredential};

mod collected;
#[cfg(test)]
pub(crate) mod collected_tests;
mod dto;
#[cfg(test)]
pub(crate) mod favorite_tests;
mod favorites;
pub(in crate::client::native) mod metadata;
#[cfg(test)]
mod metadata_tests;
#[cfg(test)]
pub(crate) mod tests;

const MAX_PAGES: u64 = 15;
pub(in crate::client::native) const MAX_TRACKS: usize = 10_000;
const MAX_BYTES: usize = 16 * 1024 * 1024;
pub(super) const COLLECTED_PATH: &str = "/list.s";

pub(crate) struct Snapshot {
    pub playlist: Playlist,
    pub(in crate::client::native) tracks: Vec<Track>,
    pub(in crate::client::native) detail: Option<metadata::Metadata>,
}
impl Snapshot {
    pub(crate) fn into_page(self, request: &PageRequest) -> Page<Track> {
        let total = self.tracks.len() as u64;
        let items: Vec<_> = self
            .tracks
            .into_iter()
            .skip(request.offset as usize)
            .take(request.limit as usize)
            .collect();
        let end = u64::from(request.offset) + items.len() as u64;
        Page {
            items,
            pagination: PageMeta {
                limit: request.limit,
                offset: request.offset,
                total: Some(total),
                has_more: end < total,
                next_offset: (end < total).then_some(end as u32),
                extensions: self.playlist.extensions,
            },
        }
    }
}

impl KuwoClient {
    /// Reads one ordinary playlist belonging to the independently validated native
    /// account, including its complete contents before returning metadata.
    /// Collected playlists and the special favorites list have separate SDK methods.
    pub async fn native_created_playlist(
        &self,
        credential: &ProviderCredential,
        id: &str,
    ) -> Result<Playlist> {
        Ok(self
            .native_playlist_snapshot(credential, Some(id), Section::Created)
            .await?
            .playlist)
    }

    /// Reads the complete self-created playlist twice, preserving order and duplicate
    /// songs, before applying the requested window. This is not an atomic snapshot.
    pub async fn native_created_playlist_tracks(
        &self,
        credential: &ProviderCredential,
        id: &str,
        request: &PageRequest,
    ) -> Result<Page<Track>> {
        validate_request(request)?;
        if request.account.as_deref().is_some_and(|v| v != "default") {
            return Err(kuwo_invalid_request(
                "Kuwo SDK credentials cannot select a stored account",
            ));
        }
        Ok(self
            .native_playlist_snapshot(credential, Some(id), Section::Created)
            .await?
            .into_page(request))
    }

    /// Reads a playlist present in the independently validated account's complete
    /// collection directory. The collecting account is not its creator.
    pub async fn native_collected_playlist(
        &self,
        credential: &ProviderCredential,
        id: &str,
    ) -> Result<Playlist> {
        Ok(self
            .native_playlist_snapshot(credential, Some(id), Section::Saved)
            .await?
            .playlist)
    }

    /// Completes two ordered content reads and checks collection membership again
    /// before returning a window. Mixed non-music lists are explicitly unsupported.
    pub async fn native_collected_playlist_tracks(
        &self,
        credential: &ProviderCredential,
        id: &str,
        request: &PageRequest,
    ) -> Result<Page<Track>> {
        validate_request(request)?;
        if request.account.as_deref().is_some_and(|v| v != "default") {
            return Err(kuwo_invalid_request(
                "Kuwo SDK credentials cannot select a stored account",
            ));
        }
        Ok(self
            .native_playlist_snapshot(credential, Some(id), Section::Saved)
            .await?
            .into_page(request))
    }

    async fn native_playlist_snapshot(
        &self,
        credential: &ProviderCredential,
        id: Option<&str>,
        section: Section,
    ) -> Result<Snapshot> {
        if let Some(id) = id {
            validate_id(id)?;
        }
        let input = credential::NativeCredential::parse(credential)?.input()?;
        validate_session_metadata(&input)?;
        self.validate_native_session(&input).await?;
        self.fetch_native_account_playlist(&input, id, Some(section), || Ok(()))
            .await
    }

    pub(crate) async fn fetch_native_account_playlist(
        &self,
        input: &KuwoNativeSessionInput,
        id: Option<&str>,
        section: Option<Section>,
        mut check: impl FnMut() -> Result<()> + Send,
    ) -> Result<Snapshot> {
        if id.is_none() && section != Some(Section::Favorite) {
            return Err(invalid());
        }
        tokio::time::timeout(Duration::from_secs(120), async {
            let (before, selected, before_pages) = self
                .selected_playlist(input, id, section, &mut check)
                .await?;
            if before
                .track_count
                .is_some_and(|count| count > MAX_TRACKS as u64)
            {
                return Err(invalid());
            }
            let before_detail = if selected == Section::Created {
                let detail = self
                    .checked_playlist_metadata(input, &before.id, &mut check)
                    .await?;
                if !detail.matches_directory(&before) {
                    return Err(changed());
                }
                if detail.count > MAX_TRACKS as u64 {
                    return Err(invalid());
                }
                Some(detail)
            } else {
                None
            };
            let expected_count = before_detail
                .as_ref()
                .map(|m| m.count)
                .or(before.track_count);
            let (tracks, pages) = self
                .playlist_traversal(input, &before.id, selected, expected_count, &mut check)
                .await?;
            let (second, second_pages) = self
                .playlist_traversal(input, &before.id, selected, expected_count, &mut check)
                .await?;
            if tracks != second || pages != second_pages {
                return Err(changed());
            }
            let after_detail = if let Some(original) = &before_detail {
                let detail = self
                    .checked_playlist_metadata(input, &before.id, &mut check)
                    .await?;
                if original != &detail {
                    return Err(changed());
                }
                Some(detail)
            } else {
                None
            };
            let (after, _, after_pages) = self
                .selected_playlist(input, Some(&before.id), Some(selected), &mut check)
                .await?;
            if metadata(&before) != metadata(&after)
                || after_detail
                    .as_ref()
                    .is_some_and(|m| !m.matches_directory(&after))
            {
                return Err(changed());
            }
            check()?;
            // Local content identity only, not a signature, credential, or upstream
            // version. It lets independent metadata/item calls detect changes.
            let backend = match selected {
                Section::Created => "native_created_playlist",
                Section::Saved => "native_collected_playlist",
                Section::Favorite => "native_favorite_playlist",
                Section::Owned => return Err(invalid()),
            };
            let mut complete_metadata = metadata(&after);
            if let Some(detail) = &after_detail {
                complete_metadata["editable_metadata"] =
                    serde_json::to_value(detail).map_err(|_| invalid())?;
            }
            let encoded =
                serde_json::to_vec(&(input.user_id(), backend, complete_metadata, &tracks))
                    .map_err(|_| invalid())?;
            let mut snapshot_id = format!("kuwo-{backend}-");
            for byte in Sha1::digest(&encoded) {
                write!(&mut snapshot_id, "{byte:02x}").expect("writing to a string cannot fail");
            }
            let mut playlist = after;
            let detail_pages = if let Some(detail) = &after_detail {
                playlist.tags = detail.tags.clone();
                playlist
                    .extensions
                    .insert("editable_metadata_verified".into(), json!(true));
                2
            } else {
                0
            };
            playlist.track_count = Some(tracks.len() as u64);
            playlist.extensions.extend([
                ("backend".into(), json!(backend)),
                ("source_snapshot_id".into(), json!(snapshot_id)),
                ("complete_read".into(), json!(true)),
                (
                    "consistency".into(),
                    json!(if detail_pages == 2 {
                        "two_equal_content_and_metadata_reads_and_directory_recheck"
                    } else {
                        "two_equal_reads_and_directory_recheck"
                    }),
                ),
                (
                    "upstream_pages_fetched".into(),
                    json!(pages + second_pages + before_pages + after_pages + detail_pages),
                ),
            ]);
            Ok(Snapshot {
                playlist,
                tracks,
                detail: after_detail,
            })
        })
        .await
        .map_err(|_| {
            TuneWeaveError::new(
                ErrorCode::UpstreamTimeout,
                "Kuwo account playlist read timed out",
            )
            .with_platform(Platform::Kuwo)
        })?
    }

    async fn selected_playlist(
        &self,
        input: &KuwoNativeSessionInput,
        id: Option<&str>,
        section: Option<Section>,
        check: &mut (impl FnMut() -> Result<()> + Send),
    ) -> Result<(Playlist, Section, u64)> {
        let mut pages = 0;
        let sections = section.map_or_else(|| vec![Section::Owned, Section::Saved], |s| vec![s]);
        for selected in sections {
            let (items, fetched) = self
                .native_library_items(input, Some(selected), check)
                .await?;
            pages += fetched;
            if let Some(playlist) = items.into_iter().find(|p| id.is_none_or(|id| p.id == id)) {
                let selected = if selected == Section::Owned {
                    match playlist
                        .extensions
                        .get("library_section")
                        .and_then(|v| v.as_str())
                    {
                        Some("created") => Section::Created,
                        Some("favorite") => Section::Favorite,
                        _ => return Err(invalid()),
                    }
                } else {
                    selected
                };
                return Ok((playlist, selected, pages));
            }
        }
        Err(TuneWeaveError::new(
            ErrorCode::ResourceNotFound,
            "Kuwo playlist is not in the selected account's requested library section",
        )
        .with_platform(Platform::Kuwo))
    }

    async fn playlist_traversal(
        &self,
        input: &KuwoNativeSessionInput,
        id: &str,
        section: Section,
        expected_count: Option<u64>,
        check: &mut (impl FnMut() -> Result<()> + Send),
    ) -> Result<(Vec<Track>, u64)> {
        match section {
            Section::Created | Section::Favorite => {
                let (mut tracks, pages) = self
                    .native_playlist_traversal(input, id, expected_count, check)
                    .await?;
                if section == Section::Favorite {
                    for track in &mut tracks {
                        track
                            .extensions
                            .insert("backend".into(), json!("native_favorite_playlist"));
                    }
                }
                Ok((tracks, pages))
            }
            Section::Saved => {
                self.native_collected_playlist_traversal(input, id, expected_count, check)
                    .await
            }
            Section::Owned => Err(invalid()),
        }
    }

    async fn native_playlist_traversal(
        &self,
        input: &KuwoNativeSessionInput,
        id: &str,
        expected_count: Option<u64>,
        check: &mut (impl FnMut() -> Result<()> + Send),
    ) -> Result<(Vec<Track>, u64)> {
        let mut tracks = Vec::new();
        let mut pages = None;
        let mut bytes = 0;
        for pn in 0..MAX_PAGES {
            check()?;
            let result = self
                .native_cloud_playlist_page(input, id, pn, expected_count == Some(0))
                .await;
            check()?;
            let page = result?;
            if pages.is_some_and(|expected| expected != page.pages) {
                return Err(changed());
            }
            pages = Some(page.pages);
            bytes += serde_json::to_vec(&page.tracks)
                .map_err(|_| invalid())?
                .len();
            if bytes > MAX_BYTES || tracks.len() + page.tracks.len() > MAX_TRACKS {
                return Err(invalid());
            }
            if page.tracks.is_empty() && !(pn == 0 && page.pages == 1 && expected_count == Some(0))
            {
                return Err(invalid());
            }
            // Duplicate song IDs are legitimate occurrences, including across pages.
            tracks.extend(page.tracks);
            if pn + 1 == page.pages {
                if expected_count.is_some_and(|count| count != tracks.len() as u64) {
                    return Err(changed());
                }
                return Ok((tracks, pn + 1));
            }
        }
        Err(invalid())
    }

    async fn native_cloud_playlist_page(
        &self,
        input: &KuwoNativeSessionInput,
        id: &str,
        pn: u64,
        known_empty: bool,
    ) -> Result<dto::Contents> {
        let time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| invalid())?
            .as_millis()
            .to_string();
        let directory_query = library::query(input, Section::Created, 0, &time)?;
        let target = {
            let mut query = url::form_urlencoded::Serializer::new(String::new());
            query.extend_pairs([("op", "pl3_getlist"), ("pid", id), ("sig", "0")]);
            for (key, value) in url::form_urlencoded::parse(directory_query.as_bytes()) {
                if key != "op" && key != "recommend" {
                    query.append_pair(&key, &value);
                }
            }
            query.append_pair("pn", &pn.to_string());
            format!(
                "{}?{}",
                self.native_target("nplserver.kuwo.cn", library::OWNED_PATH),
                query.finish()
            )
        };
        self.native_get_with_metadata(
            "nplserver.kuwo.cn",
            library::OWNED_PATH,
            "native_cloud_playlist",
            target,
            Some(session_metadata(input)?),
            |bytes| dto::parse(bytes, input, id, known_empty),
        )
        .await
    }
}

pub(crate) fn validate_id(id: &str) -> Result<()> {
    if !id
        .parse::<u64>()
        .ok()
        .is_some_and(|n| n > 0 && n <= i64::MAX as u64 && n.to_string() == id)
    {
        return Err(kuwo_invalid_request("Kuwo native playlist ID is invalid"));
    }
    Ok(())
}
fn metadata(playlist: &Playlist) -> serde_json::Value {
    json!({"id":playlist.id,"name":playlist.name,"description":playlist.description,"cover":playlist.cover_url,
        "count":playlist.track_count,"owner":playlist.extensions.get("owner_id"),"public":playlist.extensions.get("is_public")})
}
fn invalid() -> TuneWeaveError {
    kuwo_upstream_error("Kuwo native playlist returned an invalid or incomplete response")
}
fn changed() -> TuneWeaveError {
    kuwo_upstream_error("Kuwo native playlist changed during complete reading")
}
