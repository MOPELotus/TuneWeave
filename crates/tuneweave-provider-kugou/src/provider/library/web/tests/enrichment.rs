use super::*;
use serde_json::Value;

const HASH: &str = "abcdef0123456789abcdef0123456789";
fn song(hash: &str, id: serde_json::Value) -> serde_json::Value {
    json!({"hash":hash,"album_audio_id":id,"userid":111,"song_name":"Resolved song",
        "timelength":65000,"play_url":"https://unused.invalid/never-fetch"})
}
fn info(data: serde_json::Value) -> String {
    raw(json!({"status":1,"err_code":0,"data":data}))
}
fn base_frames(rows: serde_json::Value) -> Vec<Frame> {
    let mut frames = occurrence_frames();
    frames[3] = raw(rows);
    frames.into_iter().map(Frame::from).collect()
}
fn entry(hash: &str, name: &str) -> serde_json::Value {
    json!({"fileHash":hash,"fileName":name,"fileTimeLen":65000})
}

#[tokio::test]
async fn legacy_web_enrichment_preserves_local_page_duplicates_and_missing_ids_for_all_owners() {
    for owner in ["default", "named", "caller"] {
        let rows = json!([
            entry(&"c".repeat(32), "Outside page"),
            entry(HASH, "First"),
            entry(&HASH.to_ascii_uppercase(), "Duplicate"),
            entry(&"b".repeat(32), "No catalogue ID")
        ]);
        let mut frames = base_frames(rows);
        frames.push(info(song(HASH, json!(901))).into());
        frames.push(info(song(&"b".repeat(32), Value::Null)).into());
        let mut fixture = server(frames).await;
        let store = Arc::new(Store::default());
        let original = source("111", "original-private-secret");
        let other = credential("999", "other-native-secret");
        store.put(&other.stored("other").unwrap()).unwrap();
        fixture.provider.credential_store = Some(store.clone());
        let provider = if owner == "caller" {
            fixture
                .provider
                .caller_scope(&original.caller().unwrap())
                .unwrap()
        } else {
            store.put(&original.stored(owner).unwrap()).unwrap();
            fixture.provider.clone()
        };
        let page = provider
            .playlist_track_occurrences(
                "legacy_web_collection:111:MQ",
                &PageRequest {
                    account: (owner != "caller").then(|| owner.to_owned()),
                    limit: 3,
                    offset: 1,
                },
            )
            .await
            .unwrap();
        assert_eq!(page.pagination.total, Some(4));
        assert_eq!(
            page.pagination.extensions["total_scope"],
            "returned_legacy_track_occurrences"
        );
        assert_eq!(
            page.items.iter().map(|v| v.position).collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
        assert_eq!(page.items[0].track.as_ref().unwrap().id, "901");
        assert_eq!(page.items[1].track.as_ref().unwrap().id, "901");
        assert_ne!(page.items[0].id, page.items[1].id);
        assert_eq!(page.items[1].extensions["file_name"], "Duplicate");
        assert!(page.items[2].track.is_none());
        assert!(
            !serde_json::to_string(&page)
                .unwrap()
                .contains("never-fetch")
        );
        assert_eq!(read(&store, "other"), other);
        assert_eq!(
            provider.take_response_credential().unwrap().is_some(),
            owner == "caller"
        );
        let requests = fixture.requests.await.unwrap();
        assert_eq!(requests.len(), 7);
        for (request, hash) in requests[5..].iter().zip([HASH.to_owned(), "b".repeat(32)]) {
            assert!(request.starts_with("GET /play/songinfo?"));
            let target = request
                .lines()
                .next()
                .unwrap()
                .split_whitespace()
                .nth(1)
                .unwrap();
            let url = url::Url::parse(&format!("http://localhost{target}")).unwrap();
            let mut query: std::collections::BTreeMap<String, String> =
                url.query_pairs().into_owned().collect();
            assert_eq!(query["hash"], hash);
            assert_eq!(query["userid"], "111");
            assert_eq!(query["token"], "tracks-verified-secret");
            assert!(!query.contains_key("album_audio_id"));
            assert!(!query.contains_key("album_id"));
            assert!(!query.contains_key("encode_album_audio_id"));
            let signature = query.remove("signature").unwrap();
            assert_eq!(
                signature,
                crate::signing::web_signature(
                    &query.iter().map(|(k, v)| (k.as_str(), v.clone())).collect(),
                    b""
                )
            );
            assert!(!request.to_ascii_lowercase().contains("cookie:"));
            assert!(!request.contains("other-native-secret"));
        }
    }
}

#[tokio::test]
async fn legacy_web_enrichment_rejects_bad_resolutions_and_reflected_credentials() {
    for case in ["hash", "uid", "multiple", "secret", "failure"] {
        let mut data = song(HASH, json!(901));
        match case {
            "hash" => data["hash"] = json!("b".repeat(32)),
            "uid" => data["userid"] = json!(222),
            "multiple" => data["album_audio_id"] = json!([901, 902]),
            "secret" => data["song_name"] = json!("reflected tracks-verified-secret"),
            _ => (),
        }
        let mut frames = base_frames(json!([entry(HASH, "Original")]));
        frames.push(if case == "failure" {
            raw(json!({"status":0,"err_code":30020,"data":{}})).into()
        } else {
            info(data).into()
        });
        let fixture = server(frames).await;
        let provider = fixture
            .provider
            .caller_scope(&source("111", "original-private-secret").caller().unwrap())
            .unwrap();
        let error = provider
            .playlist_track_occurrences("legacy_web_collection:111:MQ", &track_request(None))
            .await
            .unwrap_err();
        assert!(matches!(
            error.code,
            ErrorCode::UpstreamError | ErrorCode::PermissionDenied
        ));
        assert_eq!(fixture.requests.await.unwrap().len(), 6);
    }
}

#[tokio::test]
async fn legacy_web_enrichment_checks_late_success_and_failure_against_selected_generation() {
    for caller in [false, true] {
        for failed in [false, true] {
            let mut frames = base_frames(json!([entry(HASH, "Original")]));
            let (last, resume) = paused(if failed {
                raw(json!({"status":0,"err_code":30020,"data":{}}))
            } else {
                info(song(HASH, json!(901)))
            });
            frames.push(last);
            let mut fixture = server(frames).await;
            let store = Arc::new(Store::default());
            let original = source("111", "original-private-secret");
            store.put(&original.stored("A").unwrap()).unwrap();
            fixture.provider.credential_store = Some(store.clone());
            let provider = if caller {
                fixture
                    .provider
                    .caller_scope(&original.caller().unwrap())
                    .unwrap()
            } else {
                fixture.provider.clone()
            };
            let p = provider.clone();
            let task = tokio::spawn(async move {
                p.playlist_track_occurrences(
                    "legacy_web_collection:111:MQ",
                    &track_request(if caller { None } else { Some("A") }),
                )
                .await
            });
            for _ in 0..6 {
                fixture.seen.recv().await.unwrap();
            }
            let replacement = source("111", "replacement-login");
            if caller {
                *provider.caller_credential.as_ref().unwrap().lock().unwrap() =
                    Some(replacement.clone());
            } else {
                store.put(&replacement.stored("A").unwrap()).unwrap();
            }
            resume.send(()).unwrap();
            let mut error = task.await.unwrap().unwrap_err();
            assert_eq!(error.code, ErrorCode::Conflict);
            assert!(error.take_caller_credential_update().is_none());
            assert!(provider.take_response_credential().unwrap().is_none());
            assert_eq!(
                read(&store, "A"),
                if caller { original } else { replacement }
            );
            assert_eq!(fixture.requests.await.unwrap().len(), 6);
        }
    }
}
