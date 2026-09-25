use super::*;
use md5::{Digest, Md5};
use std::time::Duration;
use tuneweave_core::PlaylistTrackOccurrence;

const BUDGET: Duration = Duration::from_secs(45);

#[derive(Clone, Copy, PartialEq, Eq)]
enum TrackRead {
    Occurrences,
    Tracks,
    Source,
}

struct Pending {
    response: Arc<Mutex<Option<ProviderCredential>>>,
    complete: bool,
}
impl Drop for Pending {
    fn drop(&mut self) {
        if !self.complete {
            if let Ok(mut response) = self.response.lock() {
                *response = None;
            }
        }
    }
}

impl KugouProvider {
    pub(in crate::provider) async fn selected_account_playlists(
        &self,
        request: &PageRequest,
    ) -> Result<Page<Playlist>> {
        let source = self.selected(request.account.as_deref().unwrap_or("default"))?;
        if matches!(source, Some((KugouCredential::Web(_), _))) {
            self.legacy_web_account_playlists(request).await
        } else {
            self.native_account_playlists(None, None, request).await
        }
    }

    async fn legacy_web_account_playlists(&self, request: &PageRequest) -> Result<Page<Playlist>> {
        if !(1..=100).contains(&request.limit)
            || request.offset.checked_add(request.limit).is_none()
        {
            return Err(kugou_invalid_request(
                "KuGou Web library pagination is invalid",
            ));
        }
        let (all, storage_bytes) = self
            .legacy_web_library_snapshot(request.account.as_deref(), None, BUDGET)
            .await?;
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
                    ("source".into(), json!("legacy_web_collection")),
                    (
                        "pagination_source".into(),
                        json!("single_response_local_slice"),
                    ),
                    ("total_scope".into(), json!("returned_legacy_lists")),
                    ("storage_used_bytes".into(), json!(storage_bytes)),
                ]),
            },
        })
    }

    pub(in crate::provider) async fn legacy_web_playlist_metadata(
        &self,
        id: &str,
        account: Option<&str>,
    ) -> Result<Playlist> {
        let (uid, _) = crate::web::library::parse_reference(id)?;
        let (items, _) = self
            .legacy_web_library_snapshot(account, Some(uid), BUDGET)
            .await?;
        items.into_iter().find(|item| item.id == id).ok_or_else(|| {
            TuneWeaveError::new(
                ErrorCode::ResourceNotFound,
                "KuGou legacy Web collection is absent from the selected account directory",
            )
            .with_platform(Platform::Kugou)
        })
    }

    async fn legacy_web_library_snapshot(
        &self,
        account: Option<&str>,
        expected_uid: Option<&str>,
        budget: Duration,
    ) -> Result<(Vec<Playlist>, u64)> {
        let mut pending = Pending {
            response: self.response_credential.clone(),
            complete: false,
        };
        let (mut current, mut stored) = self
            .selected(account.unwrap_or("default"))?
            .ok_or_else(session::authentication_required)?;
        if !matches!(current, KugouCredential::Web(_)) {
            return Err(TuneWeaveError::unsupported(
                Platform::Kugou,
                Capability::AccountPlaylists,
            ));
        }
        if expected_uid.is_some_and(|uid| uid != current.user_id()) {
            return Err(TuneWeaveError::new(
                ErrorCode::PermissionDenied,
                "KuGou legacy Web collection belongs to a different account",
            )
            .with_platform(Platform::Kugou));
        }
        let mut seen = vec![current.clone()];
        let mut accepted = false;
        let operation = tokio::time::timeout(budget, async {
            let KugouCredential::Web(web) = &current else {
                return Err(session::state_error());
            };
            let exchange = self.client.refresh_web_session(&web.session).await;
            self.apply_read(&current, &mut stored, &current)?;
            let next = current.rotate_web(exchange?)?;
            seen.push(next.clone());
            self.apply_read(&current, &mut stored, &next)?;
            current = next;
            accepted = true;
            let KugouCredential::Web(web) = &current else {
                return Err(session::state_error());
            };
            let response = self.client.legacy_web_library(&web.session).await;
            self.apply_read(&current, &mut stored, &current)?;
            let response = response?;
            let candidate = current.rotate_web(response.candidate)?;
            seen.push(candidate.clone());
            let KugouCredential::Web(web) = &candidate else {
                return Err(session::state_error());
            };
            // The legacy directory has no UID. Revalidate the exact candidate
            // Cookie independently before releasing any returned collection.
            let verified = self.client.refresh_web_session(&web.session).await;
            self.apply_read(&current, &mut stored, &current)?;
            let next = current.rotate_web(verified?)?;
            seen.push(next.clone());
            reject_secrets(&response.items, &seen)?;
            self.apply_read(&current, &mut stored, &next)?;
            current = next;
            Ok((response.items, response.storage_bytes))
        })
        .await;
        let result = match operation {
            Ok(result) => result,
            Err(_) => {
                return Err(TuneWeaveError::new(
                    ErrorCode::UpstreamTimeout,
                    "KuGou legacy Web directory exceeded its total time budget",
                )
                .with_platform(Platform::Kugou)
                .retryable(true));
            }
        };
        match result {
            Ok(result) => {
                self.apply_read(&current, &mut stored, &current)?;
                pending.complete = true;
                Ok(result)
            }
            Err(error) => {
                let mut error = self.finish_read_error(
                    &current,
                    &mut stored,
                    error,
                    accepted,
                    self.caller_credential.is_some(),
                );
                if matches!(
                    error.code,
                    ErrorCode::InternalError
                        | ErrorCode::AuthenticationRequired
                        | ErrorCode::Conflict
                        | ErrorCode::UpstreamTimeout
                ) {
                    error.take_caller_credential_update();
                }
                Err(error)
            }
        }
    }

    pub(in crate::provider) async fn legacy_web_playlist_occurrences(
        &self,
        id: &str,
        request: &PageRequest,
    ) -> Result<Page<PlaylistTrackOccurrence>> {
        self.legacy_web_playlist_window(id, request, TrackRead::Occurrences)
            .await
            .map(|(_, page)| page)
    }

    pub(in crate::provider) async fn legacy_web_playlist_tracks(
        &self,
        id: &str,
        request: &PageRequest,
    ) -> Result<Page<Track>> {
        let (_, page) = self
            .legacy_web_playlist_window(id, request, TrackRead::Tracks)
            .await?;
        Ok(Page {
            // Missing IDs were rejected inside the account transaction, before
            // its success/credential boundary. Do not filter raw occurrences.
            items: page
                .items
                .into_iter()
                .map(|item| item.track.ok_or_else(|| unresolved_track(item.position)))
                .collect::<Result<Vec<_>>>()?,
            pagination: page.pagination,
        })
    }

    pub(in crate::provider) async fn legacy_web_playlist_source(
        &self,
        id: &str,
        account: Option<&str>,
    ) -> Result<Playlist> {
        // Source identity covers the complete raw response. Resolving every
        // track here would exceed the page budget and is unnecessary: each
        // subsequent Track page requires an ID for every returned occurrence.
        self.legacy_web_playlist_window(
            id,
            &PageRequest {
                account: account.map(str::to_owned),
                limit: 1,
                offset: 0,
            },
            TrackRead::Source,
        )
        .await
        .map(|(playlist, _)| playlist)
    }

    async fn legacy_web_playlist_window(
        &self,
        id: &str,
        request: &PageRequest,
        kind: TrackRead,
    ) -> Result<(Playlist, Page<PlaylistTrackOccurrence>)> {
        let (uid, list_id) = crate::web::library::parse_reference(id)?;
        if !(1..=100).contains(&request.limit)
            || request.offset.checked_add(request.limit).is_none()
        {
            return Err(kugou_invalid_request(
                "KuGou legacy Web track pagination is invalid",
            ));
        }
        let mut pending = Pending {
            response: self.response_credential.clone(),
            complete: false,
        };
        let (mut current, mut stored) = self
            .selected(request.account.as_deref().unwrap_or("default"))?
            .ok_or_else(session::authentication_required)?;
        if !matches!(current, KugouCredential::Web(_)) {
            return Err(TuneWeaveError::unsupported(
                Platform::Kugou,
                Capability::PlaylistOccurrenceRead,
            ));
        }
        if uid != current.user_id() {
            return Err(TuneWeaveError::new(
                ErrorCode::PermissionDenied,
                "KuGou legacy Web collection belongs to a different account",
            )
            .with_platform(Platform::Kugou));
        }

        let mut seen = vec![current.clone()];
        let mut accepted = false;
        let operation = tokio::time::timeout(BUDGET, async {
            let KugouCredential::Web(web) = &current else {
                return Err(session::state_error());
            };
            let exchange = self.client.refresh_web_session(&web.session).await;
            self.apply_read(&current, &mut stored, &current)?;
            let next = current.rotate_web(exchange?)?;
            seen.push(next.clone());
            self.apply_read(&current, &mut stored, &next)?;
            current = next;
            accepted = true;

            let KugouCredential::Web(web) = &current else {
                return Err(session::state_error());
            };
            let directory = self.client.legacy_web_library(&web.session).await;
            self.apply_read(&current, &mut stored, &current)?;
            let directory = directory?;
            let candidate = current.rotate_web(directory.candidate)?;
            seen.push(candidate.clone());
            self.apply_read(&current, &mut stored, &candidate)?;
            current = candidate;
            reject_secrets(&directory.items, &seen)?;

            let mut playlist = directory
                .items
                .into_iter()
                .find(|playlist| playlist.id == id)
                .ok_or_else(|| {
                    TuneWeaveError::new(
                        ErrorCode::ResourceNotFound,
                        "KuGou legacy Web collection is absent from the selected account directory",
                    )
                    .with_platform(Platform::Kugou)
                })?;

            // type=16 has no UID in its response. Independently revalidate the exact
            // candidate before using it to fetch type=17 contents.
            let KugouCredential::Web(web) = &current else {
                return Err(session::state_error());
            };
            let verified = self.client.refresh_web_session(&web.session).await;
            self.apply_read(&current, &mut stored, &current)?;
            let next = current.rotate_web(verified?)?;
            seen.push(next.clone());
            self.apply_read(&current, &mut stored, &next)?;
            current = next;

            let KugouCredential::Web(web) = &current else {
                return Err(session::state_error());
            };
            let response = self
                .client
                .legacy_web_library_tracks(&web.session, &list_id)
                .await;
            self.apply_read(&current, &mut stored, &current)?;
            let response = response?;
            let candidate = current.rotate_web(response.candidate)?;
            seen.push(candidate.clone());
            self.apply_read(&current, &mut stored, &candidate)?;
            current = candidate;

            // type=17 does not echo the owner or a list revision. Verify the exact
            // response Cookie independently and retain only source-level occurrences.
            let KugouCredential::Web(web) = &current else {
                return Err(session::state_error());
            };
            let verified = self.client.refresh_web_session(&web.session).await;
            self.apply_read(&current, &mut stored, &current)?;
            let next = current.rotate_web(verified?)?;
            seen.push(next.clone());
            self.apply_read(&current, &mut stored, &next)?;
            current = next;

            let snapshot_bytes = serde_json::to_vec(&(
                &playlist.id,
                response
                    .items
                    .iter()
                    .map(|item| (&item.file_hash, &item.file_name, item.duration_ms))
                    .collect::<Vec<_>>(),
            ))
            .map_err(|_| session::state_error())?;
            let snapshot_id = format!("kugou-legacy-web-raw-{:x}", Md5::digest(&snapshot_bytes));
            reject_track_secrets(&response.items, id, &playlist, &seen)?;

            // Only the returned local page is enriched. Each distinct hash is
            // resolved once per operation; original duplicates and order remain.
            // At most 100 bounded songinfo replies share the existing 45s deadline.
            let mut resolved = std::collections::BTreeMap::new();
            for item in response.items.iter().skip(request.offset as usize).take(
                if kind == TrackRead::Source {
                    0
                } else {
                    request.limit as usize
                },
            ) {
                let hash = item.file_hash.to_ascii_lowercase();
                if resolved.contains_key(&hash) {
                    continue;
                }
                let KugouCredential::Web(web) = &current else {
                    return Err(session::state_error());
                };
                let outcome = self
                    .client
                    .legacy_web_resolve_hash(&web.session, &hash)
                    .await;
                self.apply_read(&current, &mut stored, &current)?;
                let track = outcome?;
                if let Some(track) = &track {
                    // Only allowlisted metadata leaves the SDK, and it must not
                    // reflect any credential observed during this transaction.
                    reject_track_secrets(
                        &[crate::web::library::WebLibraryTrack {
                            file_hash: track.id.clone(),
                            file_name: track.name.clone(),
                            duration_ms: track.duration_ms.unwrap_or(0),
                        }],
                        id,
                        &playlist,
                        &seen,
                    )?;
                }
                resolved.insert(hash, track);
            }

            let total = response.items.len() as u64;
            playlist.track_count = Some(total);
            playlist.extensions.extend(Extensions::from([
                ("library_owner_id".into(), json!(uid)),
                ("source_snapshot_id".into(), json!(snapshot_id)),
                ("complete_read".into(), json!(true)),
                ("upstream_pages_fetched".into(), json!(1)),
                ("ordering".into(), json!("upstream_response_order")),
                (
                    "snapshot_consistency".into(),
                    json!("single_unversioned_response"),
                ),
                (
                    "total_scope".into(),
                    json!("returned_legacy_track_occurrences"),
                ),
                (
                    "pagination_source".into(),
                    json!("single_response_local_slice"),
                ),
            ]));
            let items = response
                .items
                .into_iter()
                .enumerate()
                .skip(request.offset as usize)
                .take(request.limit as usize)
                .map(|(position, item)| PlaylistTrackOccurrence {
                    id: legacy_occurrence_id(id, position as u64, &item.file_hash),
                    position: position as u64,
                    track: resolved
                        .get(&item.file_hash.to_ascii_lowercase())
                        .cloned()
                        .flatten(),
                    extensions: Extensions::from([
                        ("source".into(), json!("legacy_web_collection_tracks")),
                        ("file_hash".into(), json!(item.file_hash)),
                        ("file_name".into(), json!(item.file_name)),
                        ("duration_ms".into(), json!(item.duration_ms)),
                    ]),
                })
                .collect::<Vec<_>>();
            if kind == TrackRead::Tracks
                && let Some(item) = items.iter().find(|item| item.track.is_none())
            {
                return Err(unresolved_track(item.position));
            }
            let end = request.offset + items.len() as u32;
            let has_more = u64::from(end) < total;
            let page = Page {
                items,
                pagination: PageMeta {
                    limit: request.limit,
                    offset: request.offset,
                    total: Some(total),
                    has_more,
                    next_offset: has_more.then_some(end),
                    extensions: Extensions::from([
                        ("backend".into(), json!("legacy_web_collection_tracks")),
                        ("source".into(), json!("legacy_web_collection")),
                        ("playlist_ref".into(), json!(playlist.id)),
                        ("source_snapshot_id".into(), json!(snapshot_id)),
                        ("complete_read".into(), json!(true)),
                        ("upstream_pages_fetched".into(), json!(1)),
                        ("ordering".into(), json!("upstream_response_order")),
                        (
                            "snapshot_consistency".into(),
                            json!("single_unversioned_response"),
                        ),
                        (
                            "total_scope".into(),
                            json!("returned_legacy_track_occurrences"),
                        ),
                        (
                            "pagination_source".into(),
                            json!("single_response_local_slice"),
                        ),
                    ]),
                },
            };
            Ok((playlist, page))
        })
        .await;

        let result = match operation {
            Ok(result) => result,
            Err(_) => {
                return Err(TuneWeaveError::new(
                    ErrorCode::UpstreamTimeout,
                    "KuGou legacy Web track read exceeded its total time budget",
                )
                .with_platform(Platform::Kugou)
                .retryable(true));
            }
        };
        match result {
            Ok(page) => {
                self.apply_read(&current, &mut stored, &current)?;
                pending.complete = true;
                Ok(page)
            }
            Err(error) => {
                let mut error = self.finish_read_error(
                    &current,
                    &mut stored,
                    error,
                    accepted,
                    self.caller_credential.is_some(),
                );
                if matches!(
                    error.code,
                    ErrorCode::InternalError
                        | ErrorCode::AuthenticationRequired
                        | ErrorCode::Conflict
                        | ErrorCode::UpstreamTimeout
                ) {
                    error.take_caller_credential_update();
                }
                Err(error)
            }
        }
    }
}

fn unresolved_track(position: u64) -> TuneWeaveError {
    TuneWeaveError::new(
        ErrorCode::CapabilityNotSupported,
        "KuGou legacy Web occurrence has no verified canonical Track identity",
    )
    .with_platform(Platform::Kugou)
    .with_details(json!({"unresolved_occurrence_position":position}))
}

fn legacy_occurrence_id(playlist: &str, position: u64, file_hash: &str) -> String {
    let bytes = format!("{playlist}\0{position}\0{file_hash}");
    format!("legacy_entry:{:x}", Md5::digest(bytes.as_bytes()))
}

fn reject_secrets(items: &[Playlist], credentials: &[KugouCredential]) -> Result<()> {
    let mut secrets = Vec::new();
    for credential in credentials {
        let KugouCredential::Web(web) = credential else {
            return Err(session::state_error());
        };
        secrets.extend(web.session.membership_secrets()?);
        secrets.push(credential.caller()?.into_secret());
    }
    let originals = secrets.clone();
    secrets.extend(
        originals.iter().map(|value| {
            url::form_urlencoded::byte_serialize(value.as_bytes()).collect::<String>()
        }),
    );
    // Only names and opaque IDs originate in the directory. Check both the raw
    // IDs and their encoded references before producing public metadata.
    for playlist in items {
        let values: [&str; 3] = [
            &playlist.name,
            &playlist.id,
            playlist
                .extensions
                .get("legacy_list_id")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(session::state_error)?,
        ];
        if values.iter().any(|value| {
            secrets
                .iter()
                .any(|secret| !secret.is_empty() && value.contains(secret))
        }) {
            return Err(TuneWeaveError::new(
                ErrorCode::UpstreamError,
                "KuGou legacy Web directory contains private account material",
            )
            .with_platform(Platform::Kugou));
        }
    }
    Ok(())
}

fn reject_track_secrets(
    items: &[crate::web::library::WebLibraryTrack],
    playlist_ref: &str,
    playlist: &Playlist,
    credentials: &[KugouCredential],
) -> Result<()> {
    let mut secrets = Vec::new();
    for credential in credentials {
        let KugouCredential::Web(web) = credential else {
            return Err(session::state_error());
        };
        secrets.extend(web.session.membership_secrets()?);
        secrets.push(credential.caller()?.into_secret());
    }
    let originals = secrets.clone();
    secrets.extend(
        originals.iter().map(|value| {
            url::form_urlencoded::byte_serialize(value.as_bytes()).collect::<String>()
        }),
    );
    for item in items {
        let values: [&str; 5] = [
            &item.file_hash,
            &item.file_name,
            playlist_ref,
            &playlist.name,
            playlist
                .extensions
                .get("legacy_list_id")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(session::state_error)?,
        ];
        if values.iter().any(|value| {
            secrets
                .iter()
                .any(|secret| !secret.is_empty() && value.contains(secret))
        }) {
            return Err(TuneWeaveError::new(
                ErrorCode::UpstreamError,
                "KuGou legacy Web track list contains private account material",
            )
            .with_platform(Platform::Kugou));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
