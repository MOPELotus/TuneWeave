use super::super::tests::{Store, received, replies, seed, setup, stored};
use super::*;
use crate::client::native::{
    media::{sing_along_tests as sing, tests as data},
    tests as fixture,
};
use std::sync::atomic::Ordering;
use tokio::sync::Notify;

#[tokio::test]
async fn native_sing_along_anonymous_stream_and_download_reject_before_network() {
    for action in [Action::Play, Action::Download] {
        let store = Arc::new(Store::default());
        store.forbid_reads.store(true, Ordering::SeqCst);
        let mut f = setup(Vec::new(), store).await;
        let request = StreamRequest {
            variant: tuneweave_core::StreamVariant::SingAlong,
            ..StreamRequest::default()
        };
        let error = match action {
            Action::Play => f
                .provider
                .stream(&data::track(), &request)
                .await
                .unwrap_err(),
            Action::Download => f
                .provider
                .download(&data::track(), &request)
                .await
                .unwrap_err(),
        };
        assert_eq!(error.code, ErrorCode::InvalidRequest);
        assert_eq!(f.store.reads.load(Ordering::SeqCst), 0);
        assert!(f.store.values.lock().unwrap().is_empty());
        fixture::requests(&mut f.network, 0).await;
    }
}

#[tokio::test]
async fn native_sing_along_provider_delivers_for_all_credential_sources_and_both_actions() {
    for scope in 0..3 {
        for action in [Action::Play, Action::Download] {
            let store = Arc::new(Store::default());
            let alias = if scope == 0 { "default" } else { "personal" };
            let selected = seed(&store, alias, "42", "selected-session");
            seed(&store, "other", "43", "other-session");
            let before = store.values.lock().unwrap().clone();
            store.forbid_reads.store(scope == 2, Ordering::SeqCst);
            let (bodies, mut request) = sing::flow();
            request.account = (scope != 2).then(|| alias.to_owned());
            let mut f = setup(replies(bodies), store).await;
            let p = if scope == 2 {
                f.provider
                    .caller_scope(&selected.caller().unwrap())
                    .unwrap()
            } else {
                f.provider.clone()
            };
            let result = match action {
                Action::Play => p.audio_content(&data::track(), &request).await,
                Action::Download => p.audio_download_content(&data::track(), &request).await,
            }
            .unwrap();
            assert_eq!(result.content_type, "audio/wav");
            assert_eq!(result.filename, "kuwo-67474-sing-along.wav");
            assert_eq!(result.bytes.len(), 44 + 11_025 * 4);
            assert!(result.trial.is_none());
            assert_eq!(*f.store.values.lock().unwrap(), before);
            assert!(p.take_response_credential().unwrap().is_none());
            if scope == 2 {
                assert_eq!(f.store.reads.load(Ordering::SeqCst), 0);
            }
            let calls = fixture::requests(&mut f.network, 4).await;
            assert_eq!(data::query(&calls[1])["quality"], "BCMS");
            assert_eq!(
                data::query(&calls[1])["action"],
                if action == Action::Play {
                    "play"
                } else {
                    "download"
                }
            );
            assert_eq!(data::query(&calls[2])["br"], "128kmp3");
            assert_eq!(
                data::query(&calls[2])["bc_token"],
                "ddddddddddddddddeeeeeeeeeeeeeeee"
            );
            assert_eq!(data::query(&calls[2])["loginSid"], "selected-session");
        }
    }
}

#[tokio::test]
async fn native_sing_along_provider_discards_late_content_when_selected_session_changes() {
    for scope in 0..2 {
        for action in [Action::Play, Action::Download] {
            let store = Arc::new(Store::default());
            let selected = seed(&store, "personal", "42", "selected-session");
            let other = seed(&store, "other", "43", "other-session")
                .stored("other")
                .unwrap();
            store.forbid_reads.store(scope == 1, Ordering::SeqCst);
            let gate = Arc::new(Notify::new());
            let (bodies, mut request) = sing::flow();
            request.account = (scope == 0).then(|| "personal".to_owned());
            let mut f = setup(
                bodies
                    .into_iter()
                    .enumerate()
                    .map(|(i, body)| (body, (i == 3).then(|| gate.clone())))
                    .collect(),
                store,
            )
            .await;
            let p = if scope == 1 {
                f.provider
                    .caller_scope(&selected.caller().unwrap())
                    .unwrap()
            } else {
                f.provider.clone()
            };
            let background = p.clone();
            let task = tokio::spawn(async move {
                match action {
                    Action::Play => background.audio_content(&data::track(), &request).await,
                    Action::Download => {
                        background
                            .audio_download_content(&data::track(), &request)
                            .await
                    }
                }
            });
            for _ in 0..4 {
                received(&mut f).await;
            }
            if scope == 1 {
                *p.caller_credential.as_ref().unwrap().lock().unwrap() = None;
            } else {
                seed(&f.store, "personal", "44", "replacement-session");
            }
            let after = stored(&f.store, "personal");
            gate.notify_one();
            assert_eq!(task.await.unwrap().unwrap_err().code, ErrorCode::Conflict);
            assert_eq!(stored(&f.store, "personal"), after);
            assert_eq!(stored(&f.store, "other"), Some(other));
            assert!(f.network.seen.try_recv().is_err());
        }
    }
}
