use super::*;
use std::collections::{VecDeque, btree_map::Entry};

fn songs(count: usize) -> Vec<NativePlaylistSong> {
    (0..count)
        .map(|id| NativePlaylistSong {
            content_id: id.to_string(),
            song_id: format!("song{id}"),
            name: format!("Song {id}"),
            singer: "Artist".into(),
        })
        .collect()
}

#[test]
fn native_playlist_order_minimum_plan_matches_independent_exhaustive_shortest_paths() {
    // Find true shortest distances through the complete six-song move graph.
    // This oracle does not calculate or reuse an increasing subsequence.
    let original = (0..6).collect::<Vec<_>>();
    let mut distances = BTreeMap::from([(original.clone(), 0)]);
    let mut queue = VecDeque::from([original]);
    while let Some(state) = queue.pop_front() {
        let next_distance = distances[&state] + 1;
        for from in 0..state.len() {
            for to in 0..state.len() {
                let mut next = state.clone();
                let moved = next.remove(from);
                next.insert(to, moved);
                if let Entry::Vacant(entry) = distances.entry(next.clone()) {
                    entry.insert(next_distance);
                    queue.push_back(next);
                }
            }
        }
    }
    assert_eq!(distances.len(), 720);
    let original = songs(6);
    for (target, minimum) in distances {
        let desired = target.iter().map(usize::to_string).collect::<Vec<_>>();
        let moves = move_plan(&original, &desired).unwrap();
        assert_eq!(moves.len(), minimum, "target {target:?}");
        let mut actual = (0..6).collect::<Vec<_>>();
        for (from, to) in moves {
            assert_ne!(from, to);
            let moved = actual.remove(from);
            actual.insert(to, moved);
        }
        assert_eq!(actual, target);
    }
}

#[test]
fn native_playlist_order_minimum_plan_handles_full_length_rotations_and_exact_move_budget() {
    let original = songs(10_000);
    for distance in [1, 16, 17, 9_984, 9_999] {
        let mut desired = original
            .iter()
            .map(|song| song.content_id.clone())
            .collect::<Vec<_>>();
        desired.rotate_left(distance);
        let result = move_plan(&original, &desired);
        let minimum = distance.min(original.len() - distance);
        if minimum > MAX_MOVES {
            assert_eq!(result.unwrap_err().code, ErrorCode::CapabilityNotSupported);
            continue;
        }
        let moves = result.unwrap();
        assert_eq!(moves.len(), minimum);
        let mut actual = original
            .iter()
            .map(|song| song.content_id.clone())
            .collect::<Vec<_>>();
        for (from, to) in moves {
            let moved = actual.remove(from);
            actual.insert(to, moved);
        }
        assert_eq!(actual, desired);
    }
}

#[test]
fn native_playlist_order_duplicates_match_independent_bfs_minimum() {
    // The oracle enumerates actual remove/insert moves of values, not labelled
    // occurrences, LCS or LIS. Stable ordinal pairing is deliberately not used.
    for original in [
        vec![0, 0, 1, 1, 2, 2],
        vec![0, 0, 0, 1, 1, 2],
        vec![0, 0, 0, 0, 0, 1],
        vec![0, 1, 0, 1],
    ] {
        let mut distances = BTreeMap::from([(original.clone(), 0)]);
        let mut queue = VecDeque::from([original.clone()]);
        while let Some(state) = queue.pop_front() {
            let next_distance = distances[&state] + 1;
            for from in 0..state.len() {
                for to in 0..state.len() {
                    let mut next = state.clone();
                    let moved = next.remove(from);
                    next.insert(to, moved);
                    if let Entry::Vacant(entry) = distances.entry(next.clone()) {
                        entry.insert(next_distance);
                        queue.push_back(next);
                    }
                }
            }
        }
        let catalogue = songs(3);
        let source = original
            .iter()
            .map(|id| catalogue[*id].clone())
            .collect::<Vec<_>>();
        for (target, minimum) in distances {
            let desired = target.iter().map(usize::to_string).collect::<Vec<_>>();
            let moves = move_plan(&source, &desired).unwrap();
            assert_eq!(
                moves.len(),
                minimum,
                "source {original:?}, target {target:?}"
            );
            let mut actual = original.clone();
            for (from, to) in moves {
                assert_ne!(from, to);
                let moved = actual.remove(from);
                actual.insert(to, moved);
            }
            assert_eq!(actual, target);
        }
    }
}

#[test]
fn native_playlist_order_duplicates_preserve_counts_and_bound_large_plans() {
    let mut source = songs(34);
    source.push(source[33].clone());
    for (distance, allowed) in [(16, true), (17, false)] {
        let mut target = source
            .iter()
            .map(|song| song.content_id.clone())
            .collect::<Vec<_>>();
        target.rotate_left(distance);
        let result = move_plan(&source, &target);
        if allowed {
            assert_eq!(result.unwrap().len(), distance);
        } else {
            assert_eq!(result.unwrap_err().code, ErrorCode::CapabilityNotSupported);
        }
    }
    let mut changed = source
        .iter()
        .map(|song| song.content_id.clone())
        .collect::<Vec<_>>();
    changed[34] = "0".into();
    assert_eq!(
        move_plan(&source, &changed).unwrap_err().code,
        ErrorCode::InvalidRequest
    );

    let mut source = songs(9_999);
    source.push(source[0].clone());
    let mut target = source
        .iter()
        .map(|song| song.content_id.clone())
        .collect::<Vec<_>>();
    assert!(move_plan(&source, &target).unwrap().is_empty());
    target.rotate_right(1);
    let moves = move_plan(&source, &target).unwrap();
    assert_eq!(moves.len(), 1);
    for (from, to) in moves {
        let moved = source.remove(from);
        source.insert(to, moved);
    }
    assert_eq!(
        source
            .iter()
            .map(|song| &song.content_id)
            .collect::<Vec<_>>(),
        target.iter().collect::<Vec<_>>()
    );
}
