use super::metadata::PendingCallerUpdate;
use super::*;
use crate::client::playlist_order::NativePlaylistSong;
use crate::provider::account_playlists::Snapshot;
use std::collections::{BTreeMap, VecDeque};
use std::time::Duration;
use tuneweave_core::{PlaylistTrackOrderRequest, PlaylistTrackOrderResult};

const MAX_MOVES: usize = 16;

fn desired_ids(request: &PlaylistTrackOrderRequest) -> Result<Vec<String>> {
    if request.track_refs.is_empty() || request.track_refs.len() > 10_000 {
        return Err(migu_invalid_request(
            "Migu playlist ordering requires 1 to 10000 complete track references",
        ));
    }
    for reference in &request.track_refs {
        if reference.platform() != Platform::Migu {
            return Err(migu_invalid_request(
                "Migu playlist ordering requires Migu track references",
            ));
        }
        parse_content_id(reference.id())?;
    }
    Ok(request
        .track_refs
        .iter()
        .map(|reference| reference.id().to_owned())
        .collect())
}

fn native_songs(snapshot: &Snapshot) -> Result<Vec<NativePlaylistSong>> {
    let songs = native_occurrence_songs(snapshot)?;
    let mut contents = BTreeMap::new();
    for fields in &songs {
        if contents
            .insert(&fields.content_id, fields)
            .is_some_and(|previous| previous != fields)
        {
            return Err(error(
                ErrorCode::CapabilityNotSupported,
                "Migu track-reference ordering cannot distinguish heterogeneous occurrences",
            ));
        }
    }
    Ok(songs)
}

fn native_occurrence_songs(snapshot: &Snapshot) -> Result<Vec<NativePlaylistSong>> {
    let ids = snapshot.track_ids();
    if !snapshot.order_metadata_stable || ids.len() != snapshot.order_fields.len() {
        return Err(migu_upstream_error(
            "Migu playlist order snapshot is incomplete or changed during its read",
        ));
    }
    let mut songs = BTreeMap::new();
    snapshot.order_fields.iter().zip(ids).map(|(fields, id)| {
        let fields = fields.as_ref().ok_or_else(|| error(ErrorCode::CapabilityNotSupported,
            "Migu playlist ordering requires complete original song identities, titles and singer names"))?;
        if fields.content_id != id
            || songs.insert(&fields.song_id, &fields.content_id).is_some_and(|previous| previous != &fields.content_id) {
            return Err(error(ErrorCode::CapabilityNotSupported,
                "Migu playlist ordering cannot distinguish heterogeneous occurrences or conflicting native song identities"));
        }
        Ok(fields.clone())
    }).collect()
}

fn retained_positions(desired_positions: &[usize]) -> Vec<bool> {
    let mut tails: Vec<usize> = Vec::new();
    let mut predecessor = vec![None; desired_positions.len()];
    for (index, position) in desired_positions.iter().enumerate() {
        let at = tails.partition_point(|previous| desired_positions[*previous] < *position);
        if at > 0 {
            predecessor[index] = Some(tails[at - 1]);
        }
        if at == tails.len() {
            tails.push(index);
        } else {
            tails[at] = index;
        }
    }
    let mut retained = vec![false; desired_positions.len()];
    let mut current = tails.last().copied();
    while let Some(index) = current {
        retained[index] = true;
        current = predecessor[index];
    }
    retained
}

// With repeated values, matching the first source occurrence to the first
// target occurrence is not necessarily minimal: ABAB -> BABA takes one move,
// but stable ordinal pairing followed by LIS would require two. Recover a
// genuine LCS using a bounded insertion/deletion distance. At most sixteen
// movements imply at most thirty-two edits, hence this fixed diagonal band.
fn repeated_order_labels(
    songs: &[NativePlaylistSong],
    desired: &[String],
) -> Result<(Vec<usize>, Vec<bool>)> {
    const EDITS: usize = 2 * MAX_MOVES;
    const WIDTH: usize = 2 * EDITS + 1;
    const UNREACHABLE: u8 = (EDITS + 1) as u8;
    let count = songs.len();
    let mut distances = vec![UNREACHABLE; (count + 1) * WIDTH];
    let read = |distances: &[u8], i: usize, j: usize| {
        if i.abs_diff(j) > EDITS {
            UNREACHABLE
        } else {
            distances[i * WIDTH + j + EDITS - i]
        }
    };
    for i in 0..=count {
        for j in i.saturating_sub(EDITS)..=(i + EDITS).min(count) {
            let mut distance = UNREACHABLE;
            if i == 0 && j == 0 {
                distance = 0;
            }
            if i > 0 {
                distance = distance.min(read(&distances, i - 1, j).saturating_add(1));
            }
            if j > 0 {
                distance = distance.min(read(&distances, i, j - 1).saturating_add(1));
            }
            if i > 0 && j > 0 && songs[i - 1].content_id == desired[j - 1] {
                distance = distance.min(read(&distances, i - 1, j - 1));
            }
            distances[i * WIDTH + j + EDITS - i] = distance;
        }
    }
    if read(&distances, count, count) as usize > EDITS {
        return Err(error(
            ErrorCode::CapabilityNotSupported,
            "Migu playlist ordering is limited to sixteen individually verified movements",
        ));
    }
    let mut labels = vec![None; count];
    let mut retained = vec![false; count];
    let mut used = vec![false; count];
    let (mut i, mut j) = (count, count);
    while i != 0 || j != 0 {
        let distance = read(&distances, i, j);
        if i > 0
            && j > 0
            && songs[i - 1].content_id == desired[j - 1]
            && distance == read(&distances, i - 1, j - 1)
        {
            labels[j - 1] = Some(i - 1);
            retained[j - 1] = true;
            used[i - 1] = true;
            i -= 1;
            j -= 1;
        } else if i > 0 && distance == read(&distances, i - 1, j).saturating_add(1) {
            i -= 1;
        } else if j > 0 && distance == read(&distances, i, j - 1).saturating_add(1) {
            j -= 1;
        } else {
            return Err(migu_upstream_error(
                "Migu playlist order could not be planned",
            ));
        }
    }
    let mut remaining = BTreeMap::<&str, VecDeque<usize>>::new();
    for (position, song) in songs.iter().enumerate() {
        if !used[position] {
            remaining
                .entry(&song.content_id)
                .or_default()
                .push_back(position);
        }
    }
    let labels = labels
        .into_iter()
        .zip(desired)
        .map(|(label, id)| {
            label
                .or_else(|| remaining.get_mut(id.as_str()).and_then(VecDeque::pop_front))
                .ok_or_else(|| migu_upstream_error("Migu playlist order could not be planned"))
        })
        .collect::<Result<Vec<_>>>()?;
    Ok((labels, retained))
}

fn move_plan(songs: &[NativePlaylistSong], desired: &[String]) -> Result<Vec<(usize, usize)>> {
    let counts = |values: Vec<&str>| {
        let mut counts = BTreeMap::new();
        for id in values {
            *counts.entry(id.to_owned()).or_insert(0usize) += 1;
        }
        counts
    };
    let source_counts = counts(songs.iter().map(|song| song.content_id.as_str()).collect());
    if songs.len() != desired.len()
        || source_counts != counts(desired.iter().map(String::as_str).collect())
    {
        return Err(migu_invalid_request(
            "Migu playlist ordering requires exactly the complete existing track multiset",
        ));
    }
    let (labels, retained) = if source_counts.values().any(|count| *count > 1) {
        repeated_order_labels(songs, desired)?
    } else {
        // Unmoved songs retain their relative order. Keeping a longest increasing
        // subsequence of original positions therefore minimizes the move count.
        // This also permits a long playlist's first song to move to its end once,
        // instead of pulling every following song forward individually.
        let positions = songs
            .iter()
            .enumerate()
            .map(|(index, song)| (song.content_id.as_str(), index))
            .collect::<BTreeMap<_, _>>();
        let desired_positions = desired
            .iter()
            .map(|id| positions[id.as_str()])
            .collect::<Vec<_>>();
        let retained = retained_positions(&desired_positions);
        (desired_positions, retained)
    };
    label_moves(&labels, &retained)
}

fn label_moves(labels: &[usize], retained: &[bool]) -> Result<Vec<(usize, usize)>> {
    let required_moves = labels.len() - retained.iter().filter(|keep| **keep).count();
    if required_moves > MAX_MOVES {
        return Err(error(
            ErrorCode::CapabilityNotSupported,
            "Migu playlist ordering is limited to sixteen individually verified movements",
        ));
    }
    // Source positions label only this read's interchangeable DTOs. They are
    // never exported as stable occurrence IDs or accepted from the caller.
    let mut order = (0..labels.len()).collect::<Vec<_>>();
    let mut moves = Vec::with_capacity(required_moves);
    for (index, label) in labels.iter().enumerate() {
        if retained[index] {
            continue;
        }
        let from = order
            .iter()
            .position(|value| value == label)
            .ok_or_else(|| migu_upstream_error("Migu playlist order could not be planned"))?;
        // Insert after the desired predecessor's CURRENT position. Its final
        // index may still shift when a later unwanted song is moved out.
        let insertion = if index == 0 {
            0
        } else {
            order
                .iter()
                .position(|value| *value == labels[index - 1])
                .ok_or_else(|| migu_upstream_error("Migu playlist order could not be planned"))?
                + 1
        };
        let to = insertion - usize::from(from < insertion);
        if from != to {
            let moved = order.remove(from);
            order.insert(to, moved);
            moves.push((from, to));
        }
    }
    if moves.len() != required_moves || order != labels {
        return Err(migu_upstream_error(
            "Migu playlist order could not be planned",
        ));
    }
    Ok(moves)
}

mod occurrences;

fn stable_metadata(snapshot: &Snapshot, first_changed: bool) -> Playlist {
    let mut playlist = snapshot.playlist.clone();
    playlist.extensions.remove("source_snapshot_id");
    // Official BatchActivityActivityModel updates the derived cover when a
    // movement touches position zero. A changed first song may change cover;
    // all other metadata and the private-cover flag must remain unchanged.
    if first_changed {
        playlist.cover_url = None;
    }
    playlist
}

fn library_identity(playlists: &[Playlist]) -> Vec<(String, String)> {
    let mut identities = playlists
        .iter()
        .map(|playlist| (playlist.id.clone(), playlist.name.clone()))
        .collect::<Vec<_>>();
    identities.sort_unstable();
    identities
}

impl MiguProvider {
    pub(in crate::provider) async fn reorder_owned_playlist_tracks(
        &self,
        id: &str,
        request: &PlaylistTrackOrderRequest,
    ) -> Result<PlaylistTrackOrderResult> {
        parse_playlist_id(id)?;
        let desired = desired_ids(request)?;
        let mut session = self.playlist_write_session(request.account.as_deref())?;
        let mut pending = PendingCallerUpdate {
            provider: self,
            finished: false,
        };
        let mut dispatched = 0;
        let mut confirmed = Vec::new();
        let result = tokio::time::timeout(Duration::from_secs(120), async {
            let (favorite, created) = self.write_preflight(&mut session).await?;
            ordinary(id, &favorite, &created)?;
            let mut previous = self.read_selected_account_playlist(Some(id), &session.alias, &mut session.current, &mut session.stored).await?;
            owner(&previous.playlist, session.current.user_id())?;
            let original_cover = previous.playlist.cover_url.clone();
            let mut expected = native_songs(&previous)?;
            let moves = move_plan(&expected, &desired)?;
            if !moves.is_empty() {
                let auth = self.write_native_authorization(&mut session).await?;
                for (from, to) in &moves {
                    let moved = expected.remove(*from);
                    expected.insert(*to, moved.clone());
                    self.accept_read(&session.current, session.stored.as_ref(), &session.current)?;
                    session.dispatched = true;
                    dispatched += 1;
                    self.client.move_native_playlist_song(&auth, id, &moved, *from, *to).await?;
                    self.accept_read(&session.current, session.stored.as_ref(), &session.current)?;
                    let after = self.read_selected_account_playlist(Some(id), &session.alias, &mut session.current, &mut session.stored).await?;
                    owner(&after.playlist, session.current.user_id())?;
                    let first_changed = previous.track_ids().first() != after.track_ids().first();
                    if native_songs(&after)? != expected
                        || previous.playlist.cover_url.is_some() && after.playlist.cover_url.is_none()
                        || stable_metadata(&previous, first_changed) != stable_metadata(&after, first_changed) {
                        return Err(migu_upstream_error("Migu playlist movement did not confirm the exact complete order and unchanged metadata"));
                    }
                    confirmed.push(json!({"content_id":moved.content_id,"song_id":moved.song_id,"old_position":from+1,"new_position":to+1}));
                    previous = after;
                }
            }
            let after_created = self.write_created(&mut session).await?;
            if library_identity(&created) != library_identity(&after_created) {
                return Err(migu_upstream_error("Migu created playlist library changed during track ordering"));
            }
            self.write_finish_identity(&favorite, &mut session).await?;
            Ok(PlaylistTrackOrderResult {
                playlist_ref: previous.playlist.resource_ref.clone(),
                track_refs: request.track_refs.clone(),
                snapshot_id: previous.playlist.extensions.get("source_snapshot_id").and_then(serde_json::Value::as_str).map(str::to_owned),
                extensions: Extensions::from([
                    ("backend".into(), json!("official_native_playlist_track_move")),
                    ("source_user_id".into(), json!(session.current.user_id())),
                    ("verified_by".into(), json!("native_uid_and_each_complete_playlist_readback_and_created_library")),
                    ("moves_dispatched".into(), json!(dispatched)),
                    ("confirmed_moves".into(), json!(confirmed.clone())),
                    ("atomic".into(), json!(moves.len() <= 1)),
                    ("cover_changed".into(), json!(original_cover != previous.playlist.cover_url)),
                ]),
            })
        }).await.map_err(|_| error(ErrorCode::UpstreamTimeout, "Migu playlist ordering exceeded its total deadline")).and_then(|value|value);
        let result = self
            .finish_playlist_write(&session, result)
            .map_err(|failure| {
                if !session.dispatched {
                    return failure;
                }
                let mut details = failure.details.as_object().cloned().unwrap_or_default();
                details.insert("operation".into(), json!("playlist_track_order"));
                details.insert("moves_dispatched".into(), json!(dispatched));
                details.insert("confirmed_moves".into(), json!(confirmed));
                details.insert("automatic_retry".into(), json!(false));
                failure.with_details(serde_json::Value::Object(details))
            });
        pending.finished = true;
        result
    }
}

#[cfg(test)]
mod tests;
