use super::super::tests::{Store, received, replies, seed, setup, stored};
use super::*;
use crate::client::native::{
    media::{dtsx_tests as dtsx, tests as data},
    tests as fixture,
};
use std::sync::atomic::Ordering;
use tokio::sync::Notify;

#[tokio::test]
async fn native_dtsx_anonymous_sdk_and_provider_reject_before_io_or_account_reads() {
    let store = Arc::new(Store::default());
    store.forbid_reads.store(true, Ordering::SeqCst);
    let mut f = setup(vec![], store).await;
    let request = dtsx::request();
    for sdk in [false, true] {
        assert_eq!(
            if sdk {
                f.network.client.stream(&data::track(), &request).await
            } else {
                f.provider.stream(&data::track(), &request).await
            }
            .unwrap_err()
            .code,
            ErrorCode::InvalidRequest
        );
        assert_eq!(
            if sdk {
                f.network.client.download(&data::track(), &request).await
            } else {
                f.provider.download(&data::track(), &request).await
            }
            .unwrap_err()
            .code,
            ErrorCode::InvalidRequest
        );
    }
    assert_eq!(
        f.provider
            .audio_content(&data::track(), &request)
            .await
            .unwrap_err()
            .code,
        ErrorCode::AuthenticationRequired
    );
    assert_eq!(
        f.provider
            .audio_download_content(&data::track(), &request)
            .await
            .unwrap_err()
            .code,
        ErrorCode::AuthenticationRequired
    );
    assert_eq!(f.store.reads.load(Ordering::SeqCst), 0);
    assert!(f.store.values.lock().unwrap().is_empty());
    fixture::requests(&mut f.network, 0).await;
}

#[tokio::test]
async fn native_dtsx_provider_metadata_honors_all_credential_sources_and_independent_actions() {
    for scope in 0..3 {
        for action in [Action::Play, Action::Download] {
            let store = Arc::new(Store::default());
            let alias = if scope == 0 { "default" } else { "personal" };
            let selected = seed(&store, alias, "42", "selected-session");
            seed(&store, "other", "43", "other-session");
            let before = store.values.lock().unwrap().clone();
            store.forbid_reads.store(scope == 2, Ordering::SeqCst);
            let mut f = setup(replies(dtsx::flow()), store).await;
            let provider = if scope == 2 {
                f.provider
                    .caller_scope(&selected.caller().unwrap())
                    .unwrap()
            } else {
                f.provider.clone()
            };
            let mut request = dtsx::request();
            request.account = (scope != 2).then(|| alias.to_owned());
            match action {
                Action::Play => assert_eq!(
                    provider
                        .stream(&data::track(), &request)
                        .await
                        .unwrap_err()
                        .code,
                    ErrorCode::CapabilityNotSupported
                ),
                Action::Download => {
                    let result = provider.download(&data::track(), &request).await.unwrap();
                    assert!(!result.available && result.url.is_none() && result.bitrate.is_none());
                    assert_eq!(result.extensions["content_delivery"], "download_content");
                }
            }
            assert_eq!(*f.store.values.lock().unwrap(), before);
            assert!(provider.take_response_credential().unwrap().is_none());
            if scope == 2 {
                assert_eq!(f.store.reads.load(Ordering::SeqCst), 0);
            }
            let calls = fixture::requests(&mut f.network, 3).await;
            assert_eq!(data::query(&calls[1])["quality"], "DTSX");
            assert_eq!(
                data::query(&calls[1])["action"],
                if action == Action::Play {
                    "play"
                } else {
                    "download"
                }
            );
            assert_eq!(data::query(&calls[2])["br"], "25000kmmp4");
            assert_eq!(data::query(&calls[2])["loginSid"], "selected-session");
            assert!(calls.iter().all(|call| !call.contains("other-session")));
        }
    }
}

#[tokio::test]
async fn native_dtsx_provider_rejects_partial_content_for_both_actions_without_cdn_fetch() {
    for action in [Action::Play, Action::Download] {
        let store = Arc::new(Store::default());
        seed(&store, "personal", "42", "selected-session");
        let mut bodies = dtsx::flow();
        let mut media = dtsx::media();
        media["data"]["type"] = json!(1);
        bodies[2] = crate::client::catalog::tests::json_response(&media);
        let mut f = setup(replies(bodies), store).await;
        let mut request = dtsx::request();
        request.account = Some("personal".into());
        let result = match action {
            Action::Play => f.provider.audio_content(&data::track(), &request).await,
            Action::Download => {
                f.provider
                    .audio_download_content(&data::track(), &request)
                    .await
            }
        };
        assert_eq!(result.unwrap_err().code, ErrorCode::PermissionDenied);
        fixture::requests(&mut f.network, 3).await;
    }
}

#[tokio::test]
async fn native_dtsx_provider_discards_late_grant_when_selected_account_or_caller_is_invalidated() {
    for caller in [false, true] {
        for action in [Action::Play, Action::Download] {
            let store = Arc::new(Store::default());
            let selected = seed(&store, "personal", "42", "selected-session");
            let other = seed(&store, "other", "43", "other-session")
                .stored("other")
                .unwrap();
            store.forbid_reads.store(caller, Ordering::SeqCst);
            let gate = Arc::new(Notify::new());
            let mut f = setup(
                dtsx::flow()
                    .into_iter()
                    .enumerate()
                    .map(|(i, body)| (body, (i == 2).then(|| gate.clone())))
                    .collect(),
                store,
            )
            .await;
            let provider = if caller {
                f.provider
                    .caller_scope(&selected.caller().unwrap())
                    .unwrap()
            } else {
                f.provider.clone()
            };
            let mut request = dtsx::request();
            request.account = (!caller).then(|| "personal".into());
            let background = provider.clone();
            let task = tokio::spawn(async move {
                match action {
                    Action::Play => background
                        .stream(&data::track(), &request)
                        .await
                        .map(|_| ()),
                    Action::Download => background
                        .download(&data::track(), &request)
                        .await
                        .map(|_| ()),
                }
            });
            for _ in 0..3 {
                received(&mut f).await;
            }
            if caller {
                *provider.caller_credential.as_ref().unwrap().lock().unwrap() = None;
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
