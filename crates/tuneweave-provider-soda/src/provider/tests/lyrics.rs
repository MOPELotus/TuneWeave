use super::*;

#[tokio::test]
async fn account_lyrics_deliver_original_formats_and_chinese_translation_for_all_owners() {
    let id = "7304719759323564095";
    for owner in ["default", "personal", "caller"] {
        for format in ["lrc", "krc", "text"] {
            let word_timed = format == "krc";
            let mut fixture = SessionFixture::new();
            let source = test_soda_credential().bind_user("123456").unwrap();
            fixture.put("default", &source);
            fixture.put("personal", &source);
            let original = match format {
                "krc" => "[1250,1000]<0,500,0>Original",
                "text" => "Original\nSecond line",
                _ => "[00:01.25]Original",
            };
            let translation = "[00:01.25]译文";
            let mut body: serde_json::Value =
                serde_json::from_slice(&crate::client::test_account_track_fixture(false)).unwrap();
            body["lyric"] =
                json!({"type":format,"content":original,"translations":{"cn":translation}});
            let (origin, server) = crate::test_http::serve(vec![
                account_reply("123456", Some("sessionid_ss=verified")),
                crate::test_http::json(&body.to_string(), Some("sessionid_ss=lyrics")),
            ])
            .await;
            fixture.provider.client = fixture
                .provider
                .client
                .clone()
                .with_auth_test_origin(origin);
            let provider = if owner == "caller" {
                fixture
                    .provider
                    .caller_credential_scope(&caller_from(&source))
                    .unwrap()
            } else {
                fixture.provider.clone()
            };
            let lyrics = provider
                .lyrics_with_options(
                    id,
                    &LyricsRequest {
                        word_synced: true,
                        translated: true,
                        account: (owner != "caller").then(|| owner.to_owned()),
                        ..LyricsRequest::default()
                    },
                )
                .await
                .unwrap();
            assert_eq!(lyrics.format, format);
            if format == "text" {
                assert_eq!(lyrics.plain.as_deref(), Some(original));
            }
            assert_eq!(
                lyrics.word_synced.as_deref(),
                word_timed.then_some(original)
            );
            assert_eq!(lyrics.translated.as_deref(), Some(translation));
            assert_eq!(lyrics.extensions["backend"], "official_pc_track_v2");
            if owner == "caller" {
                let update = provider.take_response_credential().unwrap().unwrap();
                assert_eq!(
                    parse_soda_caller_credential(&update)
                        .unwrap()
                        .cookie_header()
                        .unwrap(),
                    "sessionid_ss=lyrics"
                );
            } else {
                let stored = fixture.stored(owner).unwrap();
                assert_eq!(
                    SodaCredential::parse(stored.secret())
                        .unwrap()
                        .cookie_header()
                        .unwrap(),
                    "sessionid_ss=lyrics"
                );
            }
            for untouched in ["default", "personal"].into_iter().filter(|a| *a != owner) {
                assert_eq!(
                    fixture.stored(untouched).unwrap().secret(),
                    source.serialize().unwrap()
                );
            }
            let requests = server.await.unwrap();
            assert_eq!(requests.len(), 2);
            assert!(requests[1].starts_with("POST /luna/pc/track_v2?"));
        }
    }
}

#[tokio::test]
#[ignore = "uses the official anonymous Soda lyrics endpoint"]
async fn live_provider_returns_official_chinese_lrc_with_word_timed_original() {
    let provider = SodaProvider::new(SodaConfig::default()).unwrap();
    let lyrics = provider
        .lyrics_with_options(
            "6746962557803694081",
            &LyricsRequest {
                translated: true,
                word_synced: true,
                ..LyricsRequest::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(lyrics.format, "krc");
    assert!(
        lyrics
            .word_synced
            .as_ref()
            .is_some_and(|content| content.contains('<'))
    );
    assert!(
        lyrics
            .translated
            .as_ref()
            .is_some_and(|content| content.starts_with("[00:"))
    );
    assert_eq!(lyrics.extensions["translated_format"], "lrc");
    assert!(lyrics.romanized.is_none());
}
