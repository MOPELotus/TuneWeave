use super::*;

const TEXT: &str = "[ti:曲目]\n[00:01.00]第一行 50%\n[00:02.00]第二行 + 原文\n";
fn lyric() -> Value {
    // Lyrics must not require an audio URL, format, bitrate, or download grant.
    json!({"hash":HASH,"album_audio_id":901,"is_free_part":0,"lyrics":TEXT})
}
fn wanted(caller: bool) -> LyricsRequest {
    LyricsRequest {
        account: (!caller).then(|| "A".into()),
        word_synced: true,
        translated: true,
        romanized: true,
        ..Default::default()
    }
}
fn tail(value: Value, second: bool) -> Vec<Frame> {
    let mut result = vec![];
    if second {
        let mut first = lyric();
        first["encode_album_audio_id"] = json!("verified901");
        result.push(media(first).into());
    }
    result.push(media(value).into());
    result
}

#[tokio::test]
async fn web_lyrics_use_selected_browser_session_and_keep_only_available_tracks() {
    for caller in [false, true] {
        for second in [false, true] {
            let mut value = lyric();
            value["newlyrics"] = json!({"unverified":"not a supported lyric track"});
            value["token"] = json!("ignored-extra-field");
            let mut responses = tail(value.clone(), second);
            let last = responses.len() - 1;
            let response = value;
            responses[last] = response_with_cookies(
                json!({"status":1,"err_code":0,"data":response}),
                &[cookie("999", "ignored-song-cookie")],
            )
            .into();
            let (f, provider, store) = setup(frames(responses), caller).await;
            let result = provider
                .lyrics_with_options("901", &wanted(caller))
                .await
                .unwrap();
            assert_eq!(result.track_ref.to_string(), "kugou:901");
            assert_eq!(result.plain.as_deref(), Some(TEXT));
            assert_eq!(result.format, "lrc");
            assert!(
                result.word_synced.is_none()
                    && result.translated.is_none()
                    && result.romanized.is_none()
            );
            assert!(result.contributors.is_empty() && result.extensions.is_empty());
            let output = serde_json::to_string(&result).unwrap();
            for secret in [
                TOKEN,
                "ignored-song-cookie",
                "ignored-extra-field",
                "not a supported lyric track",
            ] {
                assert!(!output.contains(secret));
            }
            check_rotation(&provider, &store, caller);
            let requests = f.requests.await.unwrap();
            assert_eq!(requests.len(), if second { 5 } else { 4 });
            let login = params(&requests[0]);
            for raw in &requests[1..3] {
                assert!(!raw.contains(TOKEN));
            }
            for (i, raw) in requests[3..].iter().enumerate() {
                assert!(raw.starts_with("GET /play/songinfo?"));
                assert!(!raw.to_ascii_lowercase().contains("cookie:"));
                let mut q = query(raw);
                let signature = q.remove("signature").unwrap();
                assert_eq!(
                    signature,
                    crate::signing::web_signature(
                        &q.iter().map(|(k, v)| (k.as_str(), v.clone())).collect(),
                        b""
                    )
                );
                for (key, value) in [
                    ("userid", "111"),
                    ("token", TOKEN),
                    ("appid", "1014"),
                    ("platid", "4"),
                    ("srcappid", "2919"),
                    ("clientver", "20000"),
                ] {
                    assert_eq!(q[key], value);
                }
                assert_eq!(q["mid"], login["mid"]);
                assert_eq!(q["dfid"], login["dfid"]);
                if i == 0 {
                    assert!(q["hash"].eq_ignore_ascii_case(HASH));
                    assert_eq!(q["album_audio_id"], "901");
                } else {
                    assert_eq!(q["encode_album_audio_id"], "verified901");
                    assert!(!q.contains_key("hash"));
                }
                assert!(!q.contains_key("fmt") && !q.contains_key("accesskey"));
            }
        }
    }
}

#[tokio::test]
async fn web_lyrics_distinguish_missing_content_malformed_data_and_album_restrictions() {
    for caller in [false, true] {
        for second in [false, true] {
            let mut cases = vec![];
            // Construct separately to keep the rejected payload and expected code paired.
            for (key, bad, code) in [
                ("lyrics", json!(""), ErrorCode::ResourceNotFound),
                ("lyrics", json!("  \n"), ErrorCode::ResourceNotFound),
                ("lyrics", json!(null), ErrorCode::UpstreamError),
                ("lyrics", json!(["line"]), ErrorCode::UpstreamError),
                ("lyrics", json!("bad\u{0000}text"), ErrorCode::UpstreamError),
                ("is_free_part", json!(2), ErrorCode::UpstreamError),
                ("is_free_part", json!(null), ErrorCode::UpstreamError),
                ("has_privilege", json!(false), ErrorCode::PermissionDenied),
                ("is_publish", json!(0), ErrorCode::PermissionDenied),
            ] {
                let mut value = lyric();
                value[key] = bad;
                cases.push((value, code));
            }
            let mut missing = lyric();
            missing.as_object_mut().unwrap().remove("lyrics");
            cases.push((missing, ErrorCode::UpstreamError));
            for kind in [Some("album"), None, Some("")] {
                let mut value = lyric();
                value["is_free_part"] = json!(1);
                if let Some(kind) = kind {
                    value["store_type"] = json!(kind);
                }
                cases.push((
                    value,
                    if kind == Some("album") {
                        ErrorCode::PermissionDenied
                    } else {
                        ErrorCode::UpstreamError
                    },
                ));
            }
            for (value, code) in cases {
                let (f, provider, store) = setup(frames(tail(value, second)), caller).await;
                let error = provider
                    .lyrics_with_options("901", &wanted(caller))
                    .await
                    .unwrap_err();
                assert_eq!(error.code, code);
                assert!(!format!("{error:?}").contains(TEXT));
                check_rotation(&provider, &store, caller);
                assert_eq!(f.requests.await.unwrap().len(), if second { 5 } else { 4 });
            }
        }
        let mut value = lyric();
        value["is_free_part"] = json!(1);
        value["store_type"] = json!("audio");
        let (f, provider, store) = setup(frames(tail(value, false)), caller).await;
        assert_eq!(
            provider
                .lyrics("901", (!caller).then_some("A"))
                .await
                .unwrap()
                .plain
                .as_deref(),
            Some(TEXT)
        );
        check_rotation(&provider, &store, caller);
        assert_eq!(f.requests.await.unwrap().len(), 4);
    }
}

#[tokio::test]
async fn web_lyric_identity_drift_stops_without_native_or_anonymous_fallback() {
    for caller in [false, true] {
        for second in [false, true] {
            for (key, bad) in [
                ("hash", json!("ffffffffffffffffffffffffffffffff")),
                ("album_audio_id", json!(902)),
                ("userid", json!(222)),
                ("encode_album_audio_id", json!("bad&arg=2")),
            ] {
                let mut value = lyric();
                value[key] = bad;
                let (f, provider, store) = setup(frames(tail(value, second)), caller).await;
                assert_eq!(
                    provider
                        .lyrics_with_options("901", &wanted(caller))
                        .await
                        .unwrap_err()
                        .code,
                    ErrorCode::UpstreamError
                );
                check_rotation(&provider, &store, caller);
                assert_eq!(f.requests.await.unwrap().len(), if second { 5 } else { 4 });
            }
        }
    }
}

#[tokio::test]
async fn web_lyric_http_and_business_failures_preserve_only_valid_rotation() {
    for caller in [false, true] {
        for second in [false, true] {
            for (wire,code) in [
                ("HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into(),ErrorCode::AuthenticationRequired),
                ("HTTP/1.1 429 Too Many Requests\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into(),ErrorCode::RateLimited),
                ("HTTP/1.1 302 Found\r\nLocation: https://other.invalid\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into(),ErrorCode::UpstreamError),
                ("HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into(),ErrorCode::UpstreamError),
                ("HTTP/1.1 200 OK\r\nssa-code: 1\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into(),ErrorCode::PermissionDenied),
                (raw(json!({"status":0,"err_code":30020,"error_msg":TOKEN,"data":lyric()})),ErrorCode::PermissionDenied),
                (raw(json!({"status":0,"err_code":30022,"data":lyric()})),ErrorCode::PermissionDenied),
                (raw(json!({"status":1,"err_code":0,"error_code":500,"data":lyric()})),ErrorCode::UpstreamError),
                (raw(json!({"status":1,"err_code":0,"data":{"lyrics":"x".repeat(1024*1024)}})),ErrorCode::UpstreamError),
            ] {
                let mut input=tail(lyric(),second);let last=input.len()-1;input[last]=wire.into();
                let (f,provider,store)=setup(frames(input),caller).await;let before=read(&store,"A");
                let mut error=provider.lyrics_with_options("901",&wanted(caller)).await.unwrap_err();assert_eq!(error.code,code);
                assert!(!error.details.to_string().contains(TOKEN));
                let auth=code==ErrorCode::AuthenticationRequired;
                assert_eq!(provider.take_response_credential().unwrap().is_some(),caller && !auth);
                assert_eq!(error.take_caller_credential_update().is_some(),caller && !auth);
                if caller {assert_eq!(read(&store,"A"),before);}
                else if auth {assert!(!store.values.lock().unwrap().contains_key("A"));}
                else {let KugouCredential::Web(current)=read(&store,"A") else {panic!()};assert_eq!(current.session.media_token().unwrap(),TOKEN);}
                assert_eq!(read(&store,"B").native().session.user_id,"222");
                assert_eq!(f.requests.await.unwrap().len(),if second {5} else {4});
            }
        }
    }
}

#[tokio::test]
async fn web_lyric_late_success_and_errors_do_not_survive_logout_or_relogin() {
    for caller in [false, true] {
        for boundary in [2, 3, 4] {
            for failed in [false, true] {
                let mut first = lyric();
                first["encode_album_audio_id"] = json!("verified901");
                let mut all = frames(vec![media(first).into(), media(lyric()).into()]);
                let payload = if failed {
                    raw(json!({"status":0,"err_code":30020}))
                } else if boundary == 2 {
                    reply(
                        json!([{"audio_id":1901,"audio_name":"Artist - Trusted song","hash":HASH,"filesize":1975000,"bitrate":128,"timelength":123456}]),
                    )
                } else {
                    media(lyric())
                };
                let (frame, release) = paused(payload);
                all.truncate(boundary);
                all.push(frame);
                let (mut f, provider, store) = setup(all, caller).await;
                let active = provider.clone();
                let task = tokio::spawn(async move {
                    active.lyrics_with_options("901", &wanted(caller)).await
                });
                for _ in 0..=boundary {
                    f.seen.recv().await.unwrap();
                }
                let replacement = web("111", "replacement-lyric-login");
                if caller {
                    *provider.caller_credential.as_ref().unwrap().lock().unwrap() = None;
                } else {
                    store.put(&replacement.stored("A").unwrap()).unwrap();
                }
                release.send(()).unwrap();
                assert_eq!(task.await.unwrap().unwrap_err().code, ErrorCode::Conflict);
                assert!(provider.take_response_credential().unwrap().is_none());
                if !caller {
                    assert_eq!(read(&store, "A"), replacement);
                }
                assert_eq!(f.requests.await.unwrap().len(), boundary + 1);
            }
        }
    }
}

#[tokio::test]
async fn web_lyric_inputs_cookie_scope_and_token_echoes_are_rejected() {
    let (f, provider, _) = setup(vec![], true).await;
    for input in [
        LyricsRequest {
            account: Some("A".into()),
            ..Default::default()
        },
        LyricsRequest {
            singing_annotations: true,
            ..Default::default()
        },
        LyricsRequest {
            song_type: Some(0),
            ..Default::default()
        },
    ] {
        assert_eq!(
            provider
                .lyrics_with_options("901", &input)
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
    }
    assert_eq!(
        provider.lyrics("0901", None).await.unwrap_err().code,
        ErrorCode::InvalidRequest
    );
    assert!(f.requests.await.unwrap().is_empty());
    for caller in [false, true] {
        let mut all = frames(vec![]);
        all[0] = web_reply("111", TOKEN)
            .replace(
                "Domain=.kugou.com; Path=/;",
                "Domain=loginservice.kugou.com; Path=/;",
            )
            .into();
        let (f, provider, _) = setup(all, caller).await;
        assert_eq!(
            provider
                .lyrics_with_options("901", &wanted(caller))
                .await
                .unwrap_err()
                .code,
            ErrorCode::CapabilityNotSupported
        );
        assert_eq!(f.requests.await.unwrap().len(), 3);
        for text in [
            format!("[00:00.00]{TOKEN}"),
            format!(
                "[00:00.00]{}",
                TOKEN
                    .bytes()
                    .map(|b| format!("%{b:02X}"))
                    .collect::<String>()
            ),
        ] {
            let mut value = lyric();
            value["lyrics"] = json!(text);
            let (f, provider, store) = setup(frames(tail(value, false)), caller).await;
            let error = provider
                .lyrics_with_options("901", &wanted(caller))
                .await
                .unwrap_err();
            assert_eq!(error.code, ErrorCode::UpstreamError);
            assert!(!error.details.to_string().contains(TOKEN));
            check_rotation(&provider, &store, caller);
            assert_eq!(f.requests.await.unwrap().len(), 4);
        }
    }
}
