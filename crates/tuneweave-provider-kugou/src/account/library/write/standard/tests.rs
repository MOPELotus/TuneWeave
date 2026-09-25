use super::*;
use crate::account::tests::{ok, request, server, session};
use tuneweave_core::AlbumSummary;

fn track() -> Track {
    let mut track = Track::new(ResourceRef::new(Platform::Kugou, "900").unwrap(), "Song");
    track.artists.push(ArtistSummary {
        resource_ref: None,
        name: "Artist".into(),
    });
    track.album = Some(AlbumSummary {
        resource_ref: Some(ResourceRef::new(Platform::Kugou, "42").unwrap()),
        name: "Album".into(),
        cover_url: None,
    });
    track.duration_ms = Some(216789);
    track.extensions.insert(
        "qualities".into(),
        json!({"standard":{"hash":"ABCDEF0123456789ABCDEF0123456789",
            "size":120000,"bitrate":128000,"format":"mp3"}}),
    );
    track
}

#[test]
fn standard_add_input_preserves_milliseconds_string_album_and_mp3_filename() {
    let row = TrackInput::from_standard_catalogue(&track(), "900").unwrap();
    assert_eq!(
        serde_json::to_value(row).unwrap(),
        json!({"number":1,"name":"Artist - Song.mp3","hash":"abcdef0123456789abcdef0123456789",
            "size":120000,"sort":0,"timelen":216789,"bitrate":128,"album_id":"42","mixsongid":900})
    );
    let mut edge = track();
    edge.duration_ms = Some(i32::MAX as u64);
    edge.extensions.get_mut("qualities").unwrap()["standard"]["size"] = json!(i32::MAX);
    assert!(TrackInput::from_standard_catalogue(&edge, "900").is_ok());
    edge.duration_ms = Some(0);
    edge.extensions.get_mut("qualities").unwrap()["standard"]["size"] = json!(0);
    edge.extensions.get_mut("qualities").unwrap()["standard"]["bitrate"] = json!(0);
    let wire =
        serde_json::to_value(TrackInput::from_standard_catalogue(&edge, "900").unwrap()).unwrap();
    assert_eq!(wire["timelen"], 0);
    assert_eq!(wire["size"], 0);
    assert_eq!(wire["bitrate"], 0);
}

#[test]
fn standard_add_input_bitrate_matches_official_short_threshold_and_fallback() {
    for (input, expected) in [
        (0, 0),
        (128, 128),
        (320, 320),
        (32767, 32767),
        (32768, 32),
        (128000, 128),
        (320000, 320),
        (32767999, 32767),
        (32768000, 128),
        (i32::MAX as u64, 128),
    ] {
        assert_eq!(wire_bitrate(Some(input)).unwrap(), expected, "{input}");
    }
    assert!(wire_bitrate(None).is_err());
    assert!(wire_bitrate(Some(i32::MAX as u64 + 1)).is_err());
    assert!(wire_bitrate(Some(u64::MAX)).is_err());
}

#[test]
fn standard_add_input_rejects_missing_overflowing_or_inconsistent_catalogue_fields() {
    for case in 0..14 {
        let mut input = track();
        match case {
            0 => input.duration_ms = None,
            1 => input.duration_ms = Some(i32::MAX as u64 + 1),
            2 => input.extensions.get_mut("qualities").unwrap()["standard"]["size"] = Value::Null,
            3 => {
                input.extensions.get_mut("qualities").unwrap()["standard"]["size"] =
                    json!(i32::MAX as u64 + 1)
            }
            4 => input.extensions.get_mut("qualities").unwrap()["standard"]["size"] = json!(-1),
            5 => {
                input.extensions.get_mut("qualities").unwrap()["standard"]["bitrate"] = Value::Null
            }
            6 => {
                input.extensions.get_mut("qualities").unwrap()["standard"]["bitrate"] = json!("128")
            }
            7 => input.album = None,
            8 => input.album.as_mut().unwrap().resource_ref = None,
            9 => {
                input.album.as_mut().unwrap().resource_ref =
                    Some(ResourceRef::new(Platform::Migu, "42").unwrap())
            }
            10 => {
                input.extensions.get_mut("qualities").unwrap()["standard"]["hash"] =
                    json!("invalid")
            }
            11 => {
                input.extensions.get_mut("qualities").unwrap()["standard"]["format"] = json!("flac")
            }
            12 => input.name = "a".repeat(8182),
            _ => input.id = "901".into(),
        }
        assert!(
            TrackInput::from_standard_catalogue(&input, "900").is_err(),
            "case {case}"
        );
    }
}

#[test]
fn standard_add_input_requires_a_canonical_positive_kugou_album_reference() {
    for (platform, id) in [
        (Platform::Migu, "42"),
        (Platform::Kugou, "0"),
        (Platform::Kugou, "042"),
        (Platform::Kugou, "18446744073709551616"),
    ] {
        let mut input = track();
        input.album.as_mut().unwrap().resource_ref = Some(ResourceRef::new(platform, id).unwrap());
        assert!(TrackInput::from_standard_catalogue(&input, "900").is_err());
    }
    let row = TrackInput::from_standard_catalogue(&track(), "900").unwrap();
    assert_eq!(serde_json::to_value(row).unwrap()["album_id"], "42");
}

#[tokio::test]
async fn standard_add_input_keeps_concept_wire_separate_and_rejects_profile_mismatch() {
    let track = track();
    let legacy = TrackInput::from_catalogue(&track, "900").unwrap();
    let wire = serde_json::to_value(&legacy).unwrap();
    assert_eq!(wire["name"], "Artist - Song");
    assert_eq!(wire["album_id"], 42);
    assert_eq!(wire["timelen"], 0);
    assert_eq!(wire["bitrate"], 0);

    let (client, task) = server(vec![]).await;
    let rows = [TrackInput::from_standard_catalogue(&track, "900").unwrap()];
    let result = client
        .native_write_tracks(&session(KugouLoginClient::Concept), 37, Write::Add(&rows))
        .await;
    let Err(error) = result else {
        panic!("Standard item crossed into Concept");
    };
    assert_eq!(error.code, ErrorCode::CapabilityNotSupported);
    assert!(task.await.unwrap().is_empty());

    let source = session(KugouLoginClient::Standard);
    let (client, task) = server(vec![ok(json!({"userid":123456789,"listid":37,"type":0,
        "pre_list_ver":3,"list_ver":4,"count":2}))])
    .await;
    let result = client
        .native_write_tracks(&source, 37, Write::Add(&rows))
        .await
        .unwrap();
    assert_eq!(result.version, Some(4));
    let requests = task.await.unwrap();
    assert_eq!(requests.len(), 1);
    let (_, body) = request(
        &requests[0],
        Endpoint::LibraryAdd,
        KugouLoginClient::Standard,
    );
    assert_eq!(body["data"][0]["timelen"], 216789);
    assert_eq!(body["data"][0]["bitrate"], 128);
    assert_eq!(body["data"][0]["album_id"], "42");
}
