use super::*;
use sha1::{Digest, Sha1};
use tuneweave_core::{
    PlaylistOccurrenceOrderRequest, PlaylistOccurrenceOrderResult, PlaylistTrackOccurrence,
};

const SNAPSHOT_PREFIX: &str = "migu_occurrence_snapshot_v1_";
const OCCURRENCE_PREFIX: &str = "migu_occurrence_v1_";

fn opaque(value: &str, prefix: &str) -> bool {
    value.strip_prefix(prefix).is_some_and(|suffix| {
        suffix.len() == 40
            && suffix
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    })
}

fn changed() -> TuneWeaveError {
    error(
        ErrorCode::Conflict,
        "Migu playlist occurrence snapshot is stale or does not match the selected source",
    )
}

struct View {
    snapshot_id: String,
    ids: Vec<String>,
    songs: Vec<NativePlaylistSong>,
}

fn view(snapshot: &Snapshot, session: &WriteSession, caller: bool) -> Result<View> {
    let songs = native_occurrence_songs(snapshot)?;
    // The old public playlist fingerprint omits original songName/singer.
    // Include every original write field and every visible row in this new
    // private contract; bind the login generation without publishing it.
    let material = serde_json::to_vec(&json!({
        "version":1,"alias":session.alias,"caller":caller,
        "playlist":snapshot.playlist,"tracks":snapshot.ordered_tracks(),
        "write_dtos":songs.iter().map(|song|json!({
            "content_id":song.content_id,"song_id":song.song_id,
            "song_name":song.name,"singer":song.singer
        })).collect::<Vec<_>>()
    }))
    .map_err(|_| {
        error(
            ErrorCode::InternalError,
            "Migu occurrence snapshot could not be encoded",
        )
    })?;
    let snapshot_id = format!(
        "{SNAPSHOT_PREFIX}{}",
        session.original.playlist_occurrence_digest(&material)
    );
    let ids = (0..songs.len())
        .map(|position| {
            let mut digest = Sha1::new();
            digest.update(b"migu_playlist_occurrence_position_v1\0");
            digest.update(snapshot_id.as_bytes());
            digest.update((position as u64).to_be_bytes());
            format!("{OCCURRENCE_PREFIX}{}", hex::encode(digest.finalize()))
        })
        .collect();
    Ok(View {
        snapshot_id,
        ids,
        songs,
    })
}

fn permutation(view: &View, request: &PlaylistOccurrenceOrderRequest) -> Result<Vec<usize>> {
    if view.snapshot_id != request.snapshot_id || view.ids.len() != request.occurrence_ids.len() {
        return Err(changed());
    }
    let positions = view
        .ids
        .iter()
        .enumerate()
        .map(|(i, id)| (id.as_str(), i))
        .collect::<BTreeMap<_, _>>();
    let desired = request
        .occurrence_ids
        .iter()
        .map(|id| positions.get(id.as_str()).copied().ok_or_else(changed))
        .collect::<Result<Vec<_>>>()?;
    if desired.iter().collect::<BTreeSet<_>>().len() != desired.len() {
        return Err(changed());
    }
    // Two equal native DTO instances have no independently observable identity.
    // Preserve their relative source order instead of claiming that an opaque
    // local ID can prove a swap of indistinguishable upstream occurrences.
    let mut last = BTreeMap::new();
    for position in &desired {
        let song = &view.songs[*position];
        let key = (&song.content_id, &song.song_id, &song.name, &song.singer);
        if last
            .insert(key, *position)
            .is_some_and(|previous| previous > *position)
        {
            return Err(error(
                ErrorCode::CapabilityNotSupported,
                "Migu cannot confirm a swap of identical native occurrence DTOs",
            ));
        }
    }
    Ok(desired)
}

fn stable_tracks(snapshot: &Snapshot) -> Vec<Track> {
    snapshot
        .ordered_tracks()
        .iter()
        .cloned()
        .map(|mut track| {
            track.extensions.remove("playlist_position");
            track
        })
        .collect()
}

impl MiguProvider {
    pub(in crate::provider) async fn read_owned_playlist_occurrences(
        &self,
        id: &str,
        request: &PageRequest,
    ) -> Result<Page<PlaylistTrackOccurrence>> {
        parse_playlist_id(id)?;
        crate::provider::account_playlists::validate_page(request)?;
        let mut session = self.playlist_write_session(request.account.as_deref())?;
        let mut pending = PendingCallerUpdate {
            provider: self,
            finished: false,
        };
        let result = tokio::time::timeout(Duration::from_secs(120), async {
            let (favorite, created) = self.write_preflight(&mut session).await?;
            ordinary(id, &favorite, &created)?;
            let snapshot = self
                .read_selected_account_playlist(
                    Some(id),
                    &session.alias,
                    &mut session.current,
                    &mut session.stored,
                )
                .await?;
            owner(&snapshot.playlist, session.current.user_id())?;
            let view = view(&snapshot, &session, self.caller_credential.is_some())?;
            let after_created = self.write_created(&mut session).await?;
            if library_identity(&created) != library_identity(&after_created) {
                return Err(changed());
            }
            self.write_finish_identity(&favorite, &mut session).await?;
            let total = view.ids.len() as u64;
            let items = view
                .ids
                .into_iter()
                .zip(snapshot.ordered_tracks())
                .enumerate()
                .skip(request.offset as usize)
                .take(request.limit as usize)
                .map(|(position, (id, track))| PlaylistTrackOccurrence {
                    id,
                    position: position as u64,
                    track: Some(track.clone()),
                    extensions: Extensions::from([(
                        "identity_scope".into(),
                        json!("selected_login_snapshot_position"),
                    )]),
                })
                .collect::<Vec<_>>();
            let end = u64::from(request.offset) + items.len() as u64;
            Ok(Page {
                items,
                pagination: PageMeta {
                    limit: request.limit,
                    offset: request.offset,
                    total: Some(total),
                    has_more: end < total,
                    next_offset: (end < total).then_some(end as u32),
                    extensions: Extensions::from([
                        (
                            "backend".into(),
                            json!("official_native_playlist_occurrences"),
                        ),
                        ("source_snapshot_id".into(), json!(view.snapshot_id)),
                        ("complete_read".into(), json!(true)),
                        ("source_user_id".into(), json!(session.current.user_id())),
                    ]),
                },
            })
        })
        .await
        .map_err(|_| {
            error(
                ErrorCode::UpstreamTimeout,
                "Migu playlist occurrence read exceeded its total deadline",
            )
        })
        .and_then(|result| result);
        let result = self.finish_playlist_write(&session, result);
        pending.finished = true;
        result
    }

    pub(in crate::provider) async fn reorder_owned_playlist_occurrences(
        &self,
        id: &str,
        request: &PlaylistOccurrenceOrderRequest,
    ) -> Result<PlaylistOccurrenceOrderResult> {
        parse_playlist_id(id)?;
        if !opaque(&request.snapshot_id, SNAPSHOT_PREFIX)
            || request.occurrence_ids.len() > 10_000
            || request
                .occurrence_ids
                .iter()
                .any(|id| !opaque(id, OCCURRENCE_PREFIX))
            || request.occurrence_ids.iter().collect::<BTreeSet<_>>().len()
                != request.occurrence_ids.len()
        {
            return Err(migu_invalid_request(
                "Migu occurrence ordering requires a snapshot and a complete distinct opaque ID permutation",
            ));
        }
        let mut session = self.playlist_write_session(request.account.as_deref())?;
        let mut pending = PendingCallerUpdate {
            provider: self,
            finished: false,
        };
        let mut dispatched = 0;
        let mut confirmed = Vec::new();
        let result = tokio::time::timeout(Duration::from_secs(120),async {
            let (favorite,created) = self.write_preflight(&mut session).await?;
            ordinary(id,&favorite,&created)?;
            let mut previous = self.read_selected_account_playlist(Some(id),&session.alias,&mut session.current,&mut session.stored).await?;
            owner(&previous.playlist,session.current.user_id())?;
            let before = view(&previous,&session,self.caller_credential.is_some())?;
            let desired = permutation(&before,request)?;
            let moves = label_moves(&desired,&retained_positions(&desired))?;
            let mut expected_songs = before.songs;
            let mut expected_tracks = stable_tracks(&previous);
            let mut labels = (0..desired.len()).collect::<Vec<_>>();
            let original_cover = previous.playlist.cover_url.clone();
            if !moves.is_empty() {
                let auth = self.write_native_authorization(&mut session).await?;
                for (from,to) in &moves {
                    let previous_first = expected_songs.first().cloned();
                    let moved = expected_songs.remove(*from);
                    expected_songs.insert(*to,moved.clone());
                    let track = expected_tracks.remove(*from);
                    expected_tracks.insert(*to,track);
                    let occurrence = labels.remove(*from);
                    labels.insert(*to,occurrence);
                    self.accept_read(&session.current,session.stored.as_ref(),&session.current)?;
                    session.dispatched = true;
                    dispatched += 1;
                    self.client.move_native_playlist_song(&auth,id,&moved,*from,*to).await?;
                    self.accept_read(&session.current,session.stored.as_ref(),&session.current)?;
                    let after = self.read_selected_account_playlist(Some(id),&session.alias,&mut session.current,&mut session.stored).await?;
                    owner(&after.playlist,session.current.user_id())?;
                    let first_changed = previous_first.as_ref()!=expected_songs.first();
                    if native_occurrence_songs(&after)?!=expected_songs
                        || stable_tracks(&after)!=expected_tracks
                        || previous.playlist.cover_url.is_some() && after.playlist.cover_url.is_none()
                        || stable_metadata(&previous,first_changed)!=stable_metadata(&after,first_changed) {
                        return Err(migu_upstream_error("Migu occurrence movement did not confirm the complete DTO order and unchanged metadata"));
                    }
                    confirmed.push(json!({"occurrence_id":before.ids[occurrence],"old_position":from+1,"new_position":to+1}));
                    previous = after;
                }
            }
            let after_created = self.write_created(&mut session).await?;
            if library_identity(&created)!=library_identity(&after_created) { return Err(changed()); }
            self.write_finish_identity(&favorite,&mut session).await?;
            let after = view(&previous,&session,self.caller_credential.is_some())?;
            Ok(PlaylistOccurrenceOrderResult {
                playlist_ref:previous.playlist.resource_ref.clone(),
                occurrence_ids:after.ids,snapshot_id:after.snapshot_id,
                extensions:Extensions::from([
                    ("backend".into(),json!("official_native_playlist_occurrences")),
                    ("source_user_id".into(),json!(session.current.user_id())),
                    ("complete_read".into(),json!(true)),
                    ("moves_dispatched".into(),json!(dispatched)),
                    ("confirmed_moves".into(),json!(confirmed.clone())),
                    ("atomic".into(),json!(moves.len()<=1)),
                    ("cover_changed".into(),json!(original_cover!=previous.playlist.cover_url)),
                    ("identity_scope".into(),json!("selected_login_snapshot_position")),
                ]),
            })
        }).await.map_err(|_|error(ErrorCode::UpstreamTimeout,"Migu occurrence ordering exceeded its total deadline")).and_then(|result|result);
        let result = self
            .finish_playlist_write(&session, result)
            .map_err(|failure| {
                if !session.dispatched {
                    return failure;
                }
                let mut details = failure.details.as_object().cloned().unwrap_or_default();
                details.insert("operation".into(), json!("playlist_occurrence_order"));
                details.insert("moves_dispatched".into(), json!(dispatched));
                details.insert("confirmed_moves".into(), json!(confirmed));
                details.insert("automatic_retry".into(), json!(false));
                failure.with_details(serde_json::Value::Object(details))
            });
        pending.finished = true;
        result
    }
}
