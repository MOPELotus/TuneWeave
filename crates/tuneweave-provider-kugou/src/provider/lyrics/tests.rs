use super::*;
use crate::provider::library::tests::store_account;
use crate::provider::session::tests::{
    Frame, credential, exchange, paused, profile, raw, read, reply, server,
};
use crate::{KugouLoginClient, signing};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use std::{collections::BTreeMap, io::Write};

const HASH: &str = "abcdef0123456789abcdef0123456789";
const KEY: &str = "BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB";
const TOKEN: &str = "rotated-lyric-token";

fn search() -> String {
    raw(
        json!({"status":200,"errcode":200,"proposal":"20","candidates":[
        {"id":"20","accesskey":KEY,"song":"Trusted song","singer":"Artist","duration":123456}]}),
    )
}
fn download(krc: bool) -> String {
    let content = if krc {
        let languages = BASE64.encode(br#"{"version":1,"content":[{"type":1,"lyricContent":[["translation"]]},{"type":0,"lyricContent":[["romanized"]]}]}"#);
        let text = format!("[language:{languages}]\n[1000,1000]<0,1000,0>word\n");
        let mut encoder =
            flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(text.as_bytes()).unwrap();
        let mut bytes = encoder.finish().unwrap();
        let key = [
            64, 71, 97, 119, 94, 50, 116, 71, 81, 54, 49, 45, 206, 210, 110, 105,
        ];
        for (i, b) in bytes.iter_mut().enumerate() {
            *b ^= key[i % key.len()];
        }
        let mut value = b"krc1".to_vec();
        value.extend(bytes);
        BASE64.encode(value)
    } else {
        BASE64.encode("[00:01.00]plain line\n")
    };
    raw(
        json!({"status":200,"error_code":0,"id":"20","fmt":if krc {"krc"} else {"lrc"},"content":content}),
    )
}
fn frames() -> Vec<Frame> {
    vec![exchange("111",TOKEN).into(), profile("111").into(),
        reply(json!([{"__status":1,"base":{"album_audio_id":901,"audio_id":1901,"songname":"Trusted song","author_name":"Artist"}}])).into(),
        reply(json!([{"audio_id":1901,"audio_name":"Artist - Trusted song","hash":HASH,"filesize":1975000,"bitrate":128,"timelength":123456}])).into(),
        search().into(), download(true).into(), download(false).into()]
}
fn params(raw: &str) -> BTreeMap<String, String> {
    let path = raw.split_whitespace().nth(1).unwrap();
    url::Url::parse(&format!("https://lyrics.kugou.com{path}"))
        .unwrap()
        .query_pairs()
        .into_owned()
        .collect()
}

#[tokio::test]
async fn native_account_lyrics_keep_rich_tracks_and_bind_downloads_to_selected_session() {
    for caller in [false, true] {
        for kind in [KugouLoginClient::Standard, KugouLoginClient::Concept] {
            let mut f = server(frames()).await;
            f.provider.client.register_test_device();
            let store = store_account(&mut f.provider);
            let other = read(&store, "B");
            let mut selected = credential("111", "initial-lyric-token").native().clone();
            selected.session.client = kind;
            let expected_device = selected.session.device.clone();
            store.put(&selected.stored("A").unwrap()).unwrap();
            let before = read(&store, "A");
            let provider = if caller {
                f.provider
                    .caller_scope(&selected.caller().unwrap())
                    .unwrap()
            } else {
                f.provider.clone()
            };
            let result = provider
                .lyrics_with_options(
                    "901",
                    &LyricsRequest {
                        account: (!caller).then(|| "A".into()),
                        word_synced: true,
                        translated: true,
                        romanized: true,
                        ..Default::default()
                    },
                )
                .await
                .unwrap();
            assert_eq!(result.format, "krc");
            assert_eq!(result.plain.as_deref(), Some("[00:01.00]plain line\n"));
            assert!(result.word_synced.unwrap().contains("<0,1000,0>word"));
            assert!(result.translated.unwrap().contains("translation"));
            assert!(result.romanized.unwrap().contains("romanized"));
            assert!(
                !serde_json::to_string(&result.extensions)
                    .unwrap()
                    .contains(KEY)
            );
            assert_eq!(read(&store, "B"), other);
            if caller {
                assert_eq!(read(&store, "A"), before);
                let updated = provider.take_response_credential().unwrap().unwrap();
                assert_eq!(
                    KugouCredential::parse_caller(&updated)
                        .unwrap()
                        .native()
                        .session
                        .token,
                    TOKEN
                );
            } else {
                assert_eq!(read(&store, "A").native().session.token, TOKEN);
                assert!(provider.take_response_credential().unwrap().is_none());
            }
            let requests = f.requests.await.unwrap();
            assert_eq!(requests.len(), 7);
            for raw in &requests[2..5] {
                assert!(!raw.contains(TOKEN));
            }
            let search = params(&requests[4]);
            assert_eq!(search["hash"], HASH);
            assert_eq!(search["duration"], "123456");
            assert_eq!(search["album_audio_id"], "901");
            assert_eq!(search["keyword"], "Artist - Trusted song");
            assert!(!search.contains_key("userid"));
            assert!(!search.contains_key("mid"));
            for (i, raw) in requests[4..].iter().enumerate() {
                let mut query = params(raw);
                assert_eq!(query["appid"], kind.appid().to_string());
                assert_eq!(query["clientver"], kind.clientver().to_string());
                assert!(raw.contains(&format!("mid: {}", expected_device.mid)));
                assert!(!raw.to_ascii_lowercase().contains("cookie:"));
                let signature = query.remove("signature").unwrap();
                let signing_query = query.iter().map(|(k, v)| (k.as_str(), v.clone())).collect();
                let expected = if kind == KugouLoginClient::Concept {
                    signing::concept_signature(&signing_query, &[])
                } else {
                    signing::android_signature(&signing_query, &[])
                };
                assert_eq!(signature, expected);
                if i > 0 {
                    assert_eq!(query["userid"], "111");
                    assert_eq!(query["token"], TOKEN);
                    assert_eq!(query["accesskey"], KEY);
                    assert_eq!(query["id"], "20");
                    assert_eq!(query["mid"], expected_device.mid);
                    assert_eq!(query["fmt"], if i == 1 { "krc" } else { "lrc" });
                }
            }
        }
    }
}

#[tokio::test]
async fn account_lyrics_accept_upstream_plain_content_for_requested_krc() {
    let mut input = frames();
    input[5] = download(false).into();
    let mut f = server(input).await;
    f.provider.client.register_test_device();
    store_account(&mut f.provider);
    let result = f.provider.lyrics("901", Some("A")).await.unwrap();
    assert_eq!(result.format, "lrc");
    assert!(result.word_synced.is_none());
    assert!(result.plain.unwrap().contains("plain line"));
    assert_eq!(f.requests.await.unwrap().len(), 7);
}

#[tokio::test]
async fn lyric_errors_stop_before_other_formats_and_preserve_only_valid_rotation() {
    for caller in [false, true] {
        for step in [4, 5, 6] {
            for (body, code) in [
                ("HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into(),ErrorCode::AuthenticationRequired),
                ("HTTP/1.1 429 Too Many Requests\r\nRetry-After: 6\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into(),ErrorCode::RateLimited),
                ("HTTP/1.1 302 Found\r\nLocation: https://other.invalid/\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into(),ErrorCode::UpstreamError),
                ("HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into(),ErrorCode::UpstreamError),
                ("HTTP/1.1 200 OK\r\nssa-code: 100\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into(),ErrorCode::PermissionDenied),
                (raw(json!({"status":200,"id":"20","fmt":"lrc","content":BASE64.encode("[00:00.00]line")})),ErrorCode::UpstreamError),
                (raw(json!({"status":500,"errcode":500,"error_code":500,"info":TOKEN,"errmsg":TOKEN})),ErrorCode::UpstreamError),
                (raw(json!({"status":200,"errcode":200,"error_code":0,"id":"999","fmt":"krc","content":"bad-base64"})),if step == 4 {ErrorCode::ResourceNotFound} else {ErrorCode::UpstreamError}),
            ] {
                let mut input = frames(); input.truncate(step+1); input[step] = body.into();
                let mut f = server(input).await; f.provider.client.register_test_device(); let store = store_account(&mut f.provider);
                let before = read(&store,"A"); let other = read(&store,"B");
                let provider = if caller {f.provider.caller_scope(&credential("111","original").caller().unwrap()).unwrap()} else {f.provider.clone()};
                let mut error = provider.lyrics("901",(!caller).then_some("A")).await.unwrap_err();
                assert_eq!(error.code,code); assert!(!error.details.to_string().contains(TOKEN));
                assert_eq!(read(&store,"B"),other);
                let auth = code == ErrorCode::AuthenticationRequired;
                assert_eq!(provider.take_response_credential().unwrap().is_some(),caller && !auth);
                assert_eq!(error.take_caller_credential_update().is_some(),caller && !auth);
                if caller { assert_eq!(read(&store,"A"),before); }
                else if auth { assert!(!store.values.lock().unwrap().contains_key("A")); }
                else { assert_eq!(read(&store,"A").native().session.token,TOKEN); }
                assert_eq!(f.requests.await.unwrap().len(),step+1);
            }
        }
    }
}

#[tokio::test]
async fn late_lyric_replies_cannot_cross_logout_or_relogin() {
    for caller in [false, true] {
        for step in [4, 5, 6] {
            let body = if step == 4 {
                search()
            } else {
                download(step == 5)
            };
            let (held, release) = paused(body);
            let mut input = frames();
            input.truncate(step + 1);
            input[step] = held;
            let mut f = server(input).await;
            f.provider.client.register_test_device();
            let store = store_account(&mut f.provider);
            let provider = if caller {
                f.provider
                    .caller_scope(&credential("111", "original").caller().unwrap())
                    .unwrap()
            } else {
                f.provider.clone()
            };
            let active = provider.clone();
            let task =
                tokio::spawn(async move { active.lyrics("901", (!caller).then_some("A")).await });
            for _ in 0..=step {
                f.seen.recv().await.unwrap();
            }
            let replacement = credential("111", "new-login");
            if caller {
                *provider.caller_credential.as_ref().unwrap().lock().unwrap() = None;
            } else {
                store.put(&replacement.stored("A").unwrap()).unwrap();
            }
            release.send(()).unwrap();
            let error = task.await.unwrap().unwrap_err();
            assert_eq!(error.code, ErrorCode::Conflict);
            assert!(provider.take_response_credential().unwrap().is_none());
            if !caller {
                assert_eq!(read(&store, "A"), replacement);
            }
            assert_eq!(f.requests.await.unwrap().len(), step + 1);
        }
    }
}

#[tokio::test]
async fn invalid_lyric_options_and_alias_mixing_make_no_upstream_requests() {
    let f = server(vec![]).await;
    let caller = f
        .provider
        .caller_scope(&credential("111", "original").caller().unwrap())
        .unwrap();
    for id in ["0", "01", "+1", " hash"] {
        assert_eq!(
            caller.lyrics(id, None).await.unwrap_err().code,
            ErrorCode::InvalidRequest
        );
    }
    for request in [
        LyricsRequest {
            singing_annotations: true,
            ..Default::default()
        },
        LyricsRequest {
            song_type: Some(1),
            ..Default::default()
        },
        LyricsRequest {
            account: Some("A".into()),
            ..Default::default()
        },
    ] {
        assert_eq!(
            caller
                .lyrics_with_options("901", &request)
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
    }
    assert!(f.requests.await.unwrap().is_empty());
}
