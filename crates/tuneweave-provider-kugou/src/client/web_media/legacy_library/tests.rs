use super::*;

const HASH: &str = "abcdef0123456789abcdef0123456789";
fn metadata() -> Value {
    json!({"hash":HASH,"album_audio_id":"901","userid":"111",
        "song_name":"Official song","audio_name":"Artist - Official song","timelength":65000,
        "play_url":"https://unused.invalid/signed?token=never-publish","is_free_part":1})
}

#[test]
fn legacy_web_enrichment_maps_only_resolved_metadata_without_media_or_entitlements() {
    let track = map_metadata(metadata(), &HASH.to_ascii_uppercase(), "111")
        .unwrap()
        .unwrap();
    assert_eq!(track.id, "901");
    assert_eq!(track.name, "Official song");
    assert_eq!(track.duration_ms, Some(65000));
    assert_eq!(track.extensions["hash"], HASH);
    assert_eq!(
        track.extensions["identity_source"],
        "legacy_web_hash_songinfo"
    );
    let serialized = serde_json::to_string(&track).unwrap();
    assert!(!serialized.contains("never-publish"));
    assert!(!serialized.contains("play_url"));
    assert!(!track.extensions.contains_key("qualities"));
    for id in [Value::Null, json!(0)] {
        let mut data = metadata();
        data["album_audio_id"] = id;
        assert!(map_metadata(data, HASH, "111").unwrap().is_none());
    }
    let mut data = metadata();
    data.as_object_mut().unwrap().remove("album_audio_id");
    assert!(map_metadata(data, HASH, "111").unwrap().is_none());
    let mut data = metadata();
    data["song_name"] = json!("");
    assert_eq!(
        map_metadata(data, HASH, "111").unwrap().unwrap().name,
        "Artist - Official song"
    );
}

#[test]
fn legacy_web_enrichment_rejects_foreign_hash_uid_and_ambiguous_or_invalid_identity() {
    for (field, value) in [
        ("hash", json!("b".repeat(32))),
        ("userid", json!(222)),
        ("album_audio_id", json!([901, 902])),
        ("album_audio_id", json!("0901")),
        ("album_audio_id", json!(-1)),
        ("timelength", json!("65.5")),
        ("song_name", json!("bad\nname")),
    ] {
        let mut data = metadata();
        data[field] = value;
        assert!(map_metadata(data, HASH, "111").is_err(), "{field}");
    }
    assert!(!valid_hash("opaque-cloud-hash"));
}
