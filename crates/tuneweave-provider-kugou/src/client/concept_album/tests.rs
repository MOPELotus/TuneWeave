use super::*;

fn song() -> Value {
    json!({"base":{"album_audio_id":900,"album_id":42,"is_publish":1,
        "author_name":"Artist","audio_name":"Song"},"audio_info":{
        "hash":"ABCDEF0123456789ABCDEF0123456789","extname":"mp3","duration":216789,
        "filesize":120000,"bitrate":128}})
}
fn page(rows: Vec<Value>, total: u64) -> Vec<u8> {
    serde_json::to_vec(&json!({"status":1,"error_code":0,"data":{"total":total,"songs":rows}}))
        .unwrap()
}
fn input(row: Value) -> Result<ConceptAlbumTrack> {
    map(serde_json::from_value(row).map_err(|_| invalid())?)
}

#[test]
fn concept_add_catalogue_preserves_same_row_identity_units_and_filename() {
    let data = parse(&page(vec![song()], 1), 42, 1).unwrap();
    let track = map(data.songs.into_iter().next().unwrap()).unwrap();
    assert_eq!(track.mixsongid, 900);
    assert_eq!(track.album_id, "42");
    assert_eq!(track.name, "Artist - Song.mp3");
    assert_eq!(track.hash, "abcdef0123456789abcdef0123456789");
    assert_eq!(
        (track.timelen, track.size, track.bitrate),
        (216789, 120000, 128)
    );
    let mut value = song();
    value["base"]["is_publish"] = json!(3);
    value["audio_info"]["bitrate"] = json!(32767);
    assert_eq!(input(value).unwrap().bitrate, 32767);
}

#[test]
fn concept_add_catalogue_rejects_missing_units_overflow_unpublished_and_non_mp3_rows() {
    for field in ["hash", "extname", "duration", "filesize", "bitrate"] {
        let mut value = song();
        value["audio_info"].as_object_mut().unwrap().remove(field);
        assert!(input(value).is_err(), "{field}");
    }
    for (field, value) in [
        ("duration", json!(0)),
        ("duration", json!(i32::MAX as u64 + 1)),
        ("filesize", json!(0)),
        ("filesize", json!(i32::MAX as u64 + 1)),
        ("bitrate", json!(0)),
        ("bitrate", json!(32768)),
        ("bitrate", json!(128000)),
        ("duration", json!("0216789")),
        ("bitrate", json!(-1)),
        ("extname", json!("flac")),
        ("hash", json!("missing")),
    ] {
        let mut row = song();
        row["audio_info"][field] = value;
        assert!(input(row).is_err(), "{field}");
    }
    for (field, value) in [
        ("is_publish", json!(0)),
        ("audio_name", json!("")),
        ("author_name", json!("\0")),
        ("audio_name", json!("x".repeat(8192))),
    ] {
        let mut row = song();
        row["base"][field] = value;
        assert!(input(row).is_err(), "{field}");
    }
}

#[test]
fn concept_add_catalogue_pages_require_exact_counts_identity_and_bounds() {
    assert!(parse(&page(vec![], 0), 42, 1).is_ok());
    for (total, count, p) in [(21, 20, 1), (21, 1, 2), (1280, 20, 64)] {
        assert!(parse(&page(vec![song(); count], total), 42, p).is_ok());
    }
    for (total, count, p) in [(21, 1, 1), (21, 2, 2), (1281, 20, 1), (0, 0, 2)] {
        assert!(parse(&page(vec![song(); count], total), 42, p).is_err());
    }
    for (field, value) in [
        ("album_id", json!(43)),
        ("album_id", json!("042")),
        ("album_audio_id", json!(0)),
        ("album_audio_id", json!(u64::MAX)),
    ] {
        let mut row = song();
        row["base"][field] = value;
        assert!(parse(&page(vec![row], 1), 42, 1).is_err());
    }
    for wire in [
        b"{\"status\":0,\"error_code\":20017}".as_slice(),
        b"{\"status\":1,\"status\":1,\"data\":{\"total\":0,\"songs\":[]}}",
    ] {
        assert!(parse(wire, 42, 1).is_err());
    }
}
