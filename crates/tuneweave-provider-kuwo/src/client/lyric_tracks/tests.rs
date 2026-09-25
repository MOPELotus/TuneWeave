use super::*;
use crate::client::catalog::tests::{Fixture, home, json_response, requests, response, setup};
use flate2::{Compression, write::ZlibEncoder};
use std::io::Write;
use tuneweave_core::MusicProvider;

// Invented text only: no platform lyric content or account fixtures.
const TRANSLATED: &str = "[ml:1.0]\n[kuwo:027]\n[00:00.00]Title\n[00:01.25]译文甲\n[00:01.25]<300,-200>原文甲\n[00:02.00]\n[00:02.00]Credits\n[00:03.125]译文乙 &amp; 合唱\n[00:03.125]<600,-300>原文乙";
const ROMANIZED: &str = "[ml:1.0]\n[kuwo:110]\n[00:00.00]Title\n[00:01.25]roma a\n[00:01.25]<300,-200>原文甲\n[00:03.125]roma b\n[00:03.125]<600,-300>原文乙";

fn compress(bytes: &[u8]) -> Vec<u8> {
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(bytes).unwrap();
    encoder.finish().unwrap()
}
fn frame(text: &str, word: bool) -> Vec<u8> {
    let content = if word {
        BASE64_STANDARD
            .encode(
                text.bytes()
                    .enumerate()
                    .map(|(i, b)| b ^ LYRIC_XOR_KEY[i % LYRIC_XOR_KEY.len()])
                    .collect::<Vec<_>>(),
            )
            .into_bytes()
    } else {
        text.as_bytes().to_vec()
    };
    let mut body = format!("TP=content\r\nlrcx={}\r\n\r\n", u8::from(word)).into_bytes();
    body.extend(compress(&content));
    body
}
fn metadata(translated: u8, romanized: u8) -> serde_json::Value {
    json!({"code":200,"data":{"id":42,"hasdlrc":translated,"hasdlrcx":translated,"lrc_info":{"lrc_roma":romanized,"lrcx_roma":romanized}}})
}
fn content(text: &str) -> Vec<u8> {
    response(200, "application/octet-stream", "", &frame(text, true))
}
fn base() -> Lyrics {
    Lyrics {
        track_ref: ResourceRef::new(Platform::Kuwo, "42").unwrap(),
        plain: Some("[00:01.250]原文甲".into()),
        translated: None,
        romanized: None,
        word_synced: None,
        singing_annotations: None,
        singing_annotations_timestamp: None,
        format: "lrc".into(),
        contributors: vec![],
        extensions: Extensions::new(),
    }
}
fn both() -> LyricsRequest {
    LyricsRequest {
        translated: true,
        romanized: true,
        ..LyricsRequest::default()
    }
}

fn track() -> Track {
    let mut track = Track::new(
        ResourceRef::new(Platform::Kuwo, "42").unwrap(),
        "Title & 中文 + / ?",
    );
    track.artists = vec![
        ArtistSummary {
            resource_ref: None,
            name: "Artist A".into(),
        },
        ArtistSummary {
            resource_ref: None,
            name: "歌手 B".into(),
        },
    ];
    track.duration_ms = Some(256_000);
    track
}
fn registration() -> Vec<u8> {
    json_response(&json!({"code":200,"success":true,"data":{"appuid":"1234567890"}}))
}
async fn setup_tracks(mut responses: Vec<Vec<u8>>) -> (Fixture, KuwoNativeDevice) {
    responses.insert(0, registration());
    let mut f = setup(responses).await;
    let device = KuwoNativeDeviceStore::default()
        .initialize(&f.client)
        .await
        .unwrap();
    let wire = f.seen.recv().await.unwrap();
    assert!(wire.contains("/openapi/v1/app/userInitData/selectFavour?"));
    (f, device)
}

#[test]
fn native_lyric_tracks_extract_only_official_auxiliary_rows_from_lrc_and_lrcx() {
    for word in [false, true] {
        let translated = decode_content(&frame(TRANSLATED, word)).unwrap().unwrap();
        let romanized = decode_content(&frame(ROMANIZED, word)).unwrap().unwrap();
        assert_eq!(
            auxiliary_lines(&translated).unwrap().as_deref(),
            Some("[00:01.250]译文甲\n[00:03.125]译文乙 & 合唱")
        );
        assert_eq!(
            auxiliary_lines(&romanized).unwrap().as_deref(),
            Some("[00:01.250]roma a\n[00:03.125]roma b")
        );
    }
    // Android's one/two digit fractions are centiseconds; three digits are ms.
    assert_eq!(parse_line("[1:2.3]A").unwrap(), (62_030, "A"));
    assert_eq!(parse_line("[1:2.003]A").unwrap(), (62_003, "A"));
}

#[test]
fn native_lyric_tracks_do_not_relabel_originals_or_ambiguous_duplicate_timestamps() {
    for text in [
        "[00:01.00]ordinary lyric",
        "[ml:0]\n[00:01.00]first\n[00:01.00]second",
        "[ml:1]\n[00:01.00]first\n[00:01.00]second\n[00:01.00]third",
        "[ml:1]\n[00:01.00]same\n[00:01.00]same",
        "[ml:1]\n[00:02.00]later\n[00:01.00]earlier",
        "[ml:1]\n[00:01.00][00:02.00]multi stamp",
        "[ml:NaN]\n[00:01.00]bad",
        "[ml:1]\n[ml:1]\n[00:01.00]duplicate header",
        "[ml:1]\n[00:60.00]invalid time",
        "[ml:1]\n[00:01.00]auxiliary\n[00:01.00]",
    ] {
        assert!(auxiliary_lines(text).is_err(), "{text}");
    }
    assert_eq!(
        auxiliary_lines("[ml:1]\n[00:01.00]ordinary lyric").unwrap(),
        None
    );
    assert_eq!(
        auxiliary_lines("[ml:1]\n[00:01.00]\n[00:01.00]original").unwrap(),
        None
    );
}

#[test]
fn native_lyric_tracks_distinguish_explicit_none_from_malformed_or_candidate_envelopes() {
    assert_eq!(decode_content(b"TP=none\r\n").unwrap(), None);
    assert_eq!(decode_content(b"TP=none\r\n\r\n").unwrap(), None);
    for bytes in [
        b"".as_slice(),
        b"TP=list\r\n",
        b"TP=none\r\nextra",
        b"tp=content\r\nlrcx=1\r\n\r\n",
        b"TP=content\r\nlrcx=2\r\n\r\n",
    ] {
        assert!(decode_content(bytes).is_err());
    }
    let complete = frame(TRANSLATED, true);
    for cut in [1, 4, complete.len() / 2] {
        assert!(decode_content(&complete[..complete.len() - cut]).is_err());
    }
    let mut trailing = frame(TRANSLATED, true);
    trailing.extend(b"extra");
    assert!(decode_content(&trailing).is_err());
    let mut invalid_utf8 = b"TP=content\r\nlrcx=0\r\n\r\n".to_vec();
    invalid_utf8.extend(compress(&[255]));
    assert!(decode_content(&invalid_utf8).is_err());
    let mut oversized = b"TP=content\r\nlrcx=0\r\n\r\n".to_vec();
    oversized.extend(compress(&vec![
        b'x';
        MAX_LYRIC_DECOMPRESSED_BYTES as usize + 1
    ]));
    assert!(decode_content(&oversized).is_err());
}

#[test]
fn native_lyric_tracks_validate_metadata_identity_and_keep_missing_flags_unknown() {
    let supported = parse_metadata(&serde_json::to_vec(&metadata(1, 1)).unwrap(), "42").unwrap();
    assert_eq!(supported.translated, Some(true));
    assert_eq!(supported.romanized, Some(true));
    let unknown = parse_metadata(br#"{"code":200,"data":{"id":42}}"#, "42").unwrap();
    assert_eq!(unknown.translated, None);
    assert_eq!(unknown.romanized, None);
    assert!(parse_metadata(&serde_json::to_vec(&metadata(0, 0)).unwrap(), "43").is_err());
    assert!(parse_metadata(br#"{"code":200,"data":{"id":"042"}}"#, "42").is_err());
    assert!(parse_metadata(br#"{"code":200,"data":null}"#, "42").is_err());
    assert!(parse_metadata(&serde_json::to_vec(&metadata(2, 0)).unwrap(), "42").is_err());
}

#[tokio::test]
async fn native_lyric_tracks_keep_the_native_selector_separate_from_web_protocol_and_accounts() {
    let (mut f, device) = setup_tracks(vec![]).await;
    let track = track();
    let meta = metadata_query("42", &device);
    assert!(meta.contains("&uid=1234567890&"));
    assert!(meta.ends_with("&id=42"));
    for romanized in [false, true] {
        let query = content_query(&track, &device, romanized).unwrap();
        let params: BTreeMap<_, _> = url::form_urlencoded::parse(query.as_bytes())
            .into_owned()
            .collect();
        for (key, value) in [
            ("loginUid", "0"),
            ("loginSid", "0"),
            ("appuid", "1234567890"),
            ("user", device.device_user()),
            ("android_id", device.android_id()),
            ("type", "lyric"),
            ("req", "2"),
            ("lrcx", "1"),
            ("rid", "42"),
            ("encode", "utf8"),
            ("songname", "Title & 中文 + / ?"),
            ("artist", "Artist A&歌手 B"),
            ("duration", "256000"),
        ] {
            assert_eq!(params.get(key).map(String::as_str), Some(value));
        }
        assert_eq!(
            params.get("trans_type").map(String::as_str),
            romanized.then_some("roma")
        );
        assert!(!query.contains("requester=localhost"));
        assert_ne!(device.device_user(), device.android_id());
    }
    let mut invalid_track = track.clone();
    invalid_track.duration_ms = None;
    assert!(content_query(&invalid_track, &device, false).is_err());
    invalid_track.duration_ms = Some(u64::MAX);
    assert!(content_query(&invalid_track, &device, false).is_err());
    invalid_track.duration_ms = Some(256_000);
    invalid_track.id = "43".into();
    assert!(content_query(&invalid_track, &device, false).is_err());
    requests(&mut f, 0).await;
}

#[tokio::test]
async fn native_lyric_tracks_fetch_independent_tracks_without_credentials_and_preserve_plain() {
    let (mut f, device) = setup_tracks(vec![
        json_response(&metadata(1, 1)),
        content(TRANSLATED),
        content(ROMANIZED),
    ])
    .await;
    let mut lyrics = base();
    f.client
        .add_native_lyric_tracks("42", &both(), &mut lyrics, &device, &track())
        .await;
    assert_eq!(lyrics.plain.as_deref(), Some("[00:01.250]原文甲"));
    assert!(lyrics.translated.as_deref().unwrap().contains("译文甲"));
    assert!(lyrics.romanized.as_deref().unwrap().contains("roma a"));
    assert!(!lyrics.translated.as_deref().unwrap().contains("原文甲"));
    let seen = requests(&mut f, 3).await;
    assert!(seen[0].contains("/api/music/info?f=kuwo&q="));
    assert!(seen[1].contains("/mobi.s?f=kuwo&q="));
    assert_ne!(seen[1].lines().next(), seen[2].lines().next());
    for wire in seen {
        let lower = wire.to_ascii_lowercase();
        for forbidden in ["cookie:", "authorization:", "secret:", "loginserver"] {
            assert!(!lower.contains(forbidden));
        }
    }
}

#[tokio::test]
async fn native_lyric_tracks_do_not_fetch_unsupported_or_unknown_tracks() {
    for value in [metadata(0, 0), json!({"code":200,"data":{"id":42}})] {
        let (mut f, device) = setup_tracks(vec![json_response(&value)]).await;
        let mut lyrics = base();
        f.client
            .add_native_lyric_tracks("42", &both(), &mut lyrics, &device, &track())
            .await;
        assert!(lyrics.translated.is_none() && lyrics.romanized.is_none());
        assert_eq!(
            lyrics.extensions["native_lyric_tracks"]["translated"]["supported"],
            value["data"]["hasdlrc"]
                .as_u64()
                .map(|v| json!(v == 1))
                .unwrap_or(json!(null))
        );
        let diagnostics = &lyrics.extensions["native_lyric_tracks"];
        if value["data"].get("hasdlrc").is_some() {
            assert_eq!(diagnostics["translated"]["available"], false);
            assert_eq!(diagnostics["romanized"]["available"], false);
        } else {
            assert!(diagnostics["translated"].get("available").is_none());
            assert!(diagnostics["romanized"].get("available").is_none());
        }
        requests(&mut f, 1).await;
    }
}

#[tokio::test]
async fn native_lyric_tracks_preserve_other_sources_when_one_auxiliary_response_fails() {
    let (mut f, device) = setup_tracks(vec![
        json_response(&metadata(1, 1)),
        response(200, "application/octet-stream", "", b"TP=list\r\n"),
        content(ROMANIZED),
    ])
    .await;
    let mut lyrics = base();
    f.client
        .add_native_lyric_tracks("42", &both(), &mut lyrics, &device, &track())
        .await;
    assert!(lyrics.translated.is_none());
    assert!(lyrics.romanized.is_some());
    assert!(lyrics.plain.is_some());
    assert!(lyrics.extensions["native_lyric_tracks"]["translated"]["error_code"].is_string());
    requests(&mut f, 3).await;
}

#[tokio::test]
async fn native_lyric_tracks_request_only_selected_track_and_reject_accounts_before_network() {
    let (mut f, device) =
        setup_tracks(vec![json_response(&metadata(1, 1)), content(ROMANIZED)]).await;
    let mut lyrics = base();
    f.client
        .add_native_lyric_tracks(
            "42",
            &LyricsRequest {
                romanized: true,
                ..LyricsRequest::default()
            },
            &mut lyrics,
            &device,
            &track(),
        )
        .await;
    assert!(lyrics.translated.is_none());
    assert!(lyrics.romanized.is_some());
    requests(&mut f, 2).await;
    let f = setup(vec![]).await;
    let request = LyricsRequest {
        account: Some("account".into()),
        ..both()
    };
    assert!(
        f.provider
            .lyrics_with_options("42", &request)
            .await
            .is_err()
    );
    assert!(
        f.client
            .lyrics_with_options("42", &request, &KuwoNativeDeviceStore::default())
            .await
            .is_err()
    );
}

fn mobile_response() -> Vec<u8> {
    json_response(
        &json!({"status":200,"data":{"songinfo":{"id":"42","musicrId":"42"},"lrclist":[{"time":"1.25","lineLyric":"Original fixture"}]}}),
    )
}
fn detail_response() -> Vec<u8> {
    json_response(
        &json!({"code":200,"data":{"musicrid":"MUSIC_42","rid":42,"name":"Title & 中文 + / ?","artist":"Artist A&歌手 B","artistid":"1&2","duration":256}}),
    )
}

#[tokio::test]
async fn native_lyric_tracks_provider_options_read_detail_and_reuse_the_anonymous_installation() {
    // Both initial responses use the valid mobile body: LRCX reports its own
    // failure while mobile succeeds, regardless of concurrent request ordering.
    let mut f = setup(vec![
        mobile_response(),
        mobile_response(),
        registration(),
        home(),
        detail_response(),
        json_response(&metadata(1, 1)),
        content(TRANSLATED),
        content(ROMANIZED),
        mobile_response(),
        mobile_response(),
        detail_response(),
        json_response(&metadata(1, 1)),
        content(ROMANIZED),
    ])
    .await;
    let result = f.provider.lyrics_with_options("42", &both()).await.unwrap();
    assert_eq!(result.plain.as_deref(), Some("[00:01.250]Original fixture"));
    assert_eq!(
        result.translated.as_deref(),
        Some("[00:01.250]译文甲\n[00:03.125]译文乙 & 合唱")
    );
    assert_eq!(
        result.romanized.as_deref(),
        Some("[00:01.250]roma a\n[00:03.125]roma b")
    );
    let result = f
        .provider
        .lyrics_with_options(
            "42",
            &LyricsRequest {
                romanized: true,
                ..LyricsRequest::default()
            },
        )
        .await
        .unwrap();
    assert!(result.translated.is_none());
    assert!(result.romanized.is_some());
    let seen = requests(&mut f, 13).await;
    assert_eq!(
        seen.iter()
            .filter(|wire| wire.contains("/openapi/v1/app/userInitData/selectFavour?"))
            .count(),
        1
    );
    assert_eq!(
        seen.iter()
            .filter(|wire| wire.contains("/api/www/music/musicInfo?"))
            .count(),
        2
    );
    for wire in seen
        .iter()
        .filter(|wire| wire.contains("/mobi.s?") || wire.contains("/api/music/info?"))
    {
        let lower = wire.to_ascii_lowercase();
        assert!(
            !lower.contains("cookie:")
                && !lower.contains("secret:")
                && !lower.contains("authorization:")
        );
    }
}

#[tokio::test]
async fn native_lyric_tracks_context_failure_keeps_primary_and_default_options_do_no_extra_work() {
    let mut f = setup(vec![
        mobile_response(),
        mobile_response(),
        response(503, "application/json", "", b"{}"),
    ])
    .await;
    let result = f.provider.lyrics_with_options("42", &both()).await.unwrap();
    assert!(result.plain.is_some());
    assert!(result.translated.is_none() && result.romanized.is_none());
    assert!(result.extensions["native_lyric_tracks"]["context"]["error_code"].is_string());
    requests(&mut f, 3).await;
    let mut f = setup(vec![mobile_response(), mobile_response()]).await;
    let result = f
        .provider
        .lyrics_with_options("42", &LyricsRequest::default())
        .await
        .unwrap();
    assert!(!result.extensions.contains_key("native_lyric_tracks"));
    requests(&mut f, 2).await;
}
