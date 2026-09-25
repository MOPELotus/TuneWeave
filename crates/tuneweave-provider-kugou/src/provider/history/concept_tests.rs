use super::*;
use crate::provider::library::tests::store_account;
use crate::provider::session::tests::{Store, exchange, paused, profile, raw, read, reply, server};
use std::{collections::BTreeMap, sync::Arc};
use tuneweave_core::{AccountCredentialStore, MusicProvider};
use url::Url;

fn account(provider: &mut KugouProvider) -> Arc<Store> {
    let store = store_account(provider);
    let mut session = read(&store, "A").native().session.clone();
    session.client = KugouLoginClient::Concept;
    store
        .put(
            &KugouCredential::verified(session)
                .unwrap()
                .stored("A")
                .unwrap(),
        )
        .unwrap();
    store
}
fn page(id: u64, more: u64, bp: &str) -> String {
    reply(
        json!({"userid":"111","has_more":more,"bp":bp,"songs":[{"mxid":id,"op":1,"ot":1800000000+id,"pc":2,"info":{"mixsongid":id,"name":format!("Song {id}"),"singername":"Singer"}}]}),
    )
}
fn request(caller: bool) -> PlaybackHistoryRequest {
    PlaybackHistoryRequest {
        period: PlaybackHistoryPeriod::AllTime,
        limit: 1,
        offset: 0,
        account: (!caller).then(|| "A".into()),
    }
}

#[tokio::test]
async fn concept_history_provider_rejects_web_credentials_before_account_io() {
    for caller in [false, true] {
        let mut f = server(vec![]).await;
        let saved = KugouCredential::verified_web(crate::web::WebSession::test_session(
            "111",
            "synthetic-web-history-token",
        ))
        .unwrap();
        let store = Arc::new(Store::default());
        store.put(&saved.stored("A").unwrap()).unwrap();
        f.provider.credential_store = Some(store.clone());
        let provider = if caller {
            f.provider.caller_scope(&saved.caller().unwrap()).unwrap()
        } else {
            f.provider.clone()
        };
        let mut error = provider
            .account_history(&request(caller))
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::CapabilityNotSupported);
        assert!(error.take_caller_credential_update().is_none());
        assert!(provider.take_response_credential().unwrap().is_none());
        assert_eq!(read(&store, "A"), saved);
        assert!(f.requests.await.unwrap().is_empty());
    }
}

#[tokio::test]
async fn concept_history_provider_reads_complete_main_device_history_for_both_owners() {
    for caller in [false, true] {
        let mut f = server(vec![
            exchange("111", "next").into(),
            profile("111").into(),
            page(1, 1, "next-cursor").into(),
            page(2, 0, "").into(),
        ])
        .await;
        let store = account(&mut f.provider);
        let saved = read(&store, "A");
        let other = read(&store, "B");
        let provider = if caller {
            f.provider.caller_scope(&saved.caller().unwrap()).unwrap()
        } else {
            f.provider.clone()
        };
        let result = provider.account_history(&request(caller)).await.unwrap();
        assert_eq!(result.items[0].track.id, "2");
        assert_eq!(result.pagination.total, Some(2));
        assert_eq!(result.pagination.next_offset, Some(1));
        assert_eq!(result.pagination.extensions["upstream_pages_fetched"], 2);
        assert_eq!(
            result.pagination.extensions["backend"],
            "concept_youth_history"
        );
        assert_eq!(result.pagination.extensions["device_type"], 1);
        assert!(!result.pagination.extensions.contains_key("source_classify"));
        assert!(!result.items[0].extensions.contains_key("source_classify"));
        assert!(
            !result.items[0]
                .extensions
                .contains_key("device_action_count")
        );
        assert_eq!(result.items[0].extensions["device_type"], 1);
        assert_eq!(read(&store, "B"), other);
        if caller {
            assert_eq!(read(&store, "A"), saved);
            assert!(provider.take_response_credential().unwrap().is_some());
        } else {
            assert_eq!(read(&store, "A").native().session.token, "next");
        }
        let all = f.requests.await.unwrap();
        assert_eq!(all.len(), 4);
        for r in &all[2..] {
            assert!(r.starts_with("GET /playhistory/youth/v1/get_songs?"));
            let u = Url::parse(&format!(
                "http://localhost{}",
                r.lines().next().unwrap().split_whitespace().nth(1).unwrap()
            ))
            .unwrap();
            let q = u.query_pairs().collect::<BTreeMap<_, _>>();
            assert_eq!(q["userid"], "111");
            assert_eq!(q["token"], "next");
        }
    }
}

#[tokio::test]
async fn concept_history_provider_foreign_page_never_returns_partial_history() {
    for caller in [false, true] {
        let mut f = server(vec![
            exchange("111", "next").into(),
            profile("111").into(),
            page(1, 1, "next").into(),
            reply(json!({"userid":"222","has_more":0,"songs":[]})).into(),
        ])
        .await;
        let store = account(&mut f.provider);
        let saved = read(&store, "A");
        let other = read(&store, "B");
        let mut session = saved.native().session.clone();
        session.token = "next".into();
        let rotated = saved.rotate(session).unwrap();
        let provider = if caller {
            f.provider.caller_scope(&saved.caller().unwrap()).unwrap()
        } else {
            f.provider.clone()
        };
        let mut e = provider
            .account_history(&request(caller))
            .await
            .unwrap_err();
        assert_eq!(e.code, ErrorCode::Conflict);
        assert!(e.take_caller_credential_update().is_none());
        // The verified exchange precedes history I/O and retains its token on
        // a later page conflict. Caller ownership never writes the server store.
        assert_eq!(read(&store, "A"), if caller { saved } else { rotated });
        assert_eq!(read(&store, "B"), other);
        assert!(provider.take_response_credential().unwrap().is_none());
        assert_eq!(f.requests.await.unwrap().len(), 4);
    }
}

#[tokio::test]
async fn concept_history_provider_relogin_rejects_late_first_or_following_page_success_and_failure()
{
    for caller in [false, true] {
        for following in [false, true] {
            for failed in [false, true] {
                let mut frames = vec![exchange("111", "next").into(), profile("111").into()];
                if following {
                    frames.push(page(1, 1, "next").into());
                }
                let (frame, resume) = paused(if failed {
                    raw(json!({"status":0,"error_code":20017}))
                } else {
                    page(2, 0, "")
                });
                frames.push(frame);
                let count = frames.len();
                let mut f = server(frames).await;
                let store = account(&mut f.provider);
                let saved = read(&store, "A");
                let provider = if caller {
                    f.provider.caller_scope(&saved.caller().unwrap()).unwrap()
                } else {
                    f.provider.clone()
                };
                let p = provider.clone();
                let task = tokio::spawn(async move { p.account_history(&request(caller)).await });
                for _ in 0..count {
                    f.seen.recv().await.unwrap();
                }
                let mut session = saved.native().session.clone();
                session.token = "replacement".into();
                let replacement = KugouCredential::verified(session).unwrap();
                if caller {
                    *provider.caller_credential.as_ref().unwrap().lock().unwrap() =
                        Some(replacement.clone());
                } else {
                    store.put(&replacement.stored("A").unwrap()).unwrap();
                }
                resume.send(()).unwrap();
                let mut e = task.await.unwrap().unwrap_err();
                assert_eq!(e.code, ErrorCode::Conflict);
                assert!(e.take_caller_credential_update().is_none());
                assert!(provider.take_response_credential().unwrap().is_none());
                assert_eq!(read(&store, "A"), if caller { saved } else { replacement });
                assert_eq!(f.requests.await.unwrap().len(), count);
            }
        }
    }
}
