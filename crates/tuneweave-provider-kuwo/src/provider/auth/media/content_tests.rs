use super::super::tests::{Store, received, replies, seed, setup, stored};
use super::*;
use crate::client::{
    catalog::tests::response,
    native::{
        media::{content::tests as audio, tests as data, trial::tests as trial},
        tests as fixture,
    },
};
use std::sync::atomic::Ordering;
use tokio::sync::Notify;
use tuneweave_core::AudioContent;

async fn invoke(p: &KuwoProvider, action: Action, account: Option<&str>) -> Result<AudioContent> {
    match action {
        Action::Play => {
            p.audio_content(&data::track(), &data::request(account))
                .await
        }
        Action::Download => {
            p.audio_download_content(&data::track(), &data::request(account))
                .await
        }
    }
}
fn bodies() -> Vec<Vec<u8>> {
    audio::flow(&audio::samples()[0], true).0
}

#[tokio::test]
async fn native_content_provider_preserves_three_owners_and_both_actions() {
    for scope in 0..3 {
        for (action, preview, master) in [
            (Action::Play, false, false),
            (Action::Download, false, false),
            (Action::Play, true, false),
            (Action::Play, false, true),
            (Action::Download, false, true),
        ] {
            let store = Arc::new(Store::default());
            let alias = if scope == 0 { "default" } else { "personal" };
            let selected = seed(&store, alias, "42", "selected-session");
            seed(&store, "other", "43", "other-session");
            let original = store.values.lock().unwrap().clone();
            store.forbid_reads.store(scope == 2, Ordering::SeqCst);
            let master_sample = audio::samples()
                .into_iter()
                .find(|s| s["quality"] == "master")
                .unwrap();
            let replies = if master {
                audio::flow(&master_sample, true).0
            } else if preview {
                trial::flow(true)
            } else {
                bodies()
            };
            let mut f = setup(replies.into_iter().map(|b| (b, None)).collect(), store).await;
            let p = if scope == 2 {
                f.provider
                    .caller_scope(&selected.caller().unwrap())
                    .unwrap()
            } else {
                f.provider.clone()
            };
            let request = tuneweave_core::StreamRequest {
                quality: if master {
                    tuneweave_core::Quality::Master
                } else {
                    tuneweave_core::Quality::Standard
                },
                ..data::request((scope != 2).then_some(alias))
            };
            let result = match action {
                Action::Play => p.audio_content(&data::track(), &request).await,
                Action::Download => p.audio_download_content(&data::track(), &request).await,
            }
            .unwrap();
            assert_eq!(
                result.content_type,
                if master { "audio/flac" } else { "audio/mpeg" }
            );
            assert_eq!(result.trial.is_some(), preview);
            assert_eq!(
                result.bytes.len(),
                if master {
                    master_sample["bytes"].as_u64().unwrap() as usize
                } else {
                    audio::samples()[0]["bytes"].as_u64().unwrap() as usize
                }
            );
            assert_eq!(*f.store.values.lock().unwrap(), original);
            assert!(p.take_response_credential().unwrap().is_none());
            if scope == 2 {
                assert_eq!(f.store.reads.load(Ordering::SeqCst), 0);
            }
            let calls = fixture::requests(&mut f.network, 4).await;
            assert_eq!(
                data::query(&calls[1])["action"],
                if action == Action::Play {
                    "play"
                } else {
                    "download"
                }
            );
            assert_eq!(data::query(&calls[2])["loginSid"], "selected-session");
        }
    }
    let store = Arc::new(Store::default());
    seed(&store, "default", "42", "selected-session");
    store.forbid_reads.store(true, Ordering::SeqCst);
    let f = setup(vec![], store).await;
    for action in [Action::Play, Action::Download] {
        assert_eq!(
            invoke(&f.provider, action, None).await.unwrap_err().code,
            ErrorCode::AuthenticationRequired
        );
    }
    assert_eq!(f.store.reads.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn native_content_late_results_and_errors_preserve_generation_at_all_four_boundaries() {
    late_content_results(false).await;
}
#[tokio::test]
async fn native_trial_late_results_preserve_generation_at_all_four_boundaries() {
    late_content_results(true).await;
}
async fn late_content_results(preview: bool) {
    for scope in 0..3 {
        for action in [Action::Play, Action::Download] {
            if preview && action == Action::Download {
                continue;
            }
            for boundary in 0..4 {
                for replacement in 0..3 {
                    for failed in [false, true] {
                        let store = Arc::new(Store::default());
                        let alias = if scope == 0 { "default" } else { "personal" };
                        let selected = seed(&store, alias, "42", "selected-session");
                        let other = seed(&store, "other", "43", "other-session")
                            .stored("other")
                            .unwrap();
                        let gate = Arc::new(Notify::new());
                        let replies = (if preview { trial::flow(true) } else { bodies() })
                            .into_iter()
                            .enumerate()
                            .map(|(i, b)| {
                                (
                                    if failed && i == boundary {
                                        response(401, "application/octet-stream", "", b"private")
                                    } else {
                                        b
                                    },
                                    (i == boundary).then(|| gate.clone()),
                                )
                            })
                            .collect();
                        let mut f = setup(replies, store).await;
                        let p = if scope == 2 {
                            f.provider
                                .caller_scope(&selected.caller().unwrap())
                                .unwrap()
                        } else {
                            f.provider.clone()
                        };
                        let background = p.clone();
                        let task = tokio::spawn(async move {
                            invoke(&background, action, (scope != 2).then_some(alias)).await
                        });
                        for _ in 0..=boundary {
                            received(&mut f).await;
                        }
                        if scope == 2 {
                            *p.caller_credential.as_ref().unwrap().lock().unwrap() =
                                if replacement == 0 {
                                    None
                                } else {
                                    Some(fixture::credential_fixture(
                                        if replacement == 1 { "42" } else { "44" },
                                        if replacement == 1 {
                                            "selected-session"
                                        } else {
                                            "new-session"
                                        },
                                    ))
                                };
                        } else if replacement == 0 {
                            f.provider.logout(alias).await.unwrap();
                        } else {
                            seed(
                                &f.store,
                                alias,
                                if replacement == 1 { "42" } else { "44" },
                                if replacement == 1 {
                                    "selected-session"
                                } else {
                                    "new-session"
                                },
                            );
                        }
                        let after = stored(&f.store, alias);
                        let caller_after = p
                            .caller_credential
                            .as_ref()
                            .map(|c| c.lock().unwrap().clone());
                        gate.notify_one();
                        assert_eq!(task.await.unwrap().unwrap_err().code, ErrorCode::Conflict);
                        assert_eq!(stored(&f.store, alias), after);
                        assert_eq!(stored(&f.store, "other"), Some(other));
                        assert_eq!(
                            p.caller_credential
                                .as_ref()
                                .map(|c| c.lock().unwrap().clone()),
                            caller_after
                        );
                        assert!(f.network.seen.try_recv().is_err());
                    }
                }
            }
        }
    }
}

#[tokio::test]
async fn native_content_cdn_401_does_not_clear_selected_credentials() {
    for scope in 0..3 {
        let store = Arc::new(Store::default());
        let selected = seed(&store, "personal", "42", "selected-session");
        let saved = stored(&store, "personal");
        let mut responses = bodies();
        responses[3] = response(401, "application/octet-stream", "", b"private");
        let mut f = setup(replies(responses), store).await;
        let p = if scope == 2 {
            f.provider
                .caller_scope(&selected.caller().unwrap())
                .unwrap()
        } else {
            f.provider.clone()
        };
        assert_eq!(
            invoke(&p, Action::Play, (scope != 2).then_some("personal"))
                .await
                .unwrap_err()
                .code,
            ErrorCode::UpstreamError
        );
        assert_eq!(stored(&f.store, "personal"), saved);
        if scope == 2 {
            assert!(
                p.caller_credential
                    .as_ref()
                    .unwrap()
                    .lock()
                    .unwrap()
                    .is_some()
            );
        }
        fixture::requests(&mut f.network, 4).await;
    }
}

#[tokio::test]
async fn native_content_cancel_and_timeout_leave_accounts_and_network_boundaries_intact() {
    for timed_out in [false, true] {
        for boundary in 0..4 {
            let store = Arc::new(Store::default());
            seed(&store, "personal", "42", "selected-session");
            let before = stored(&store, "personal");
            let gate = Arc::new(Notify::new());
            let mut f = setup(
                bodies()
                    .into_iter()
                    .enumerate()
                    .map(|(i, b)| (b, (i == boundary).then(|| gate.clone())))
                    .collect(),
                store,
            )
            .await;
            let p = f.provider.clone();
            let task = tokio::spawn(async move {
                p.read_native_content(
                    &data::track(),
                    &data::request(Some("personal")),
                    Action::Download,
                    if timed_out {
                        Duration::from_secs(1)
                    } else {
                        Duration::from_secs(10)
                    },
                )
                .await
            });
            for _ in 0..=boundary {
                received(&mut f).await;
            }
            if timed_out {
                assert_eq!(
                    task.await.unwrap().unwrap_err().code,
                    ErrorCode::UpstreamTimeout
                );
            } else {
                task.abort();
                assert!(task.await.unwrap_err().is_cancelled());
            }
            gate.notify_one();
            tokio::task::yield_now().await;
            assert!(f.network.seen.try_recv().is_err());
            assert_eq!(stored(&f.store, "personal"), before);
        }
    }
}

#[tokio::test]
async fn native_content_accounts_complete_in_reverse_order_without_sharing_credentials() {
    for caller in [false, true] {
        let store = Arc::new(Store::default());
        let first = seed(&store, "default", "42", "first-session");
        let second = seed(&store, "personal", "43", "second-session");
        let original = store.values.lock().unwrap().clone();
        let gate = Arc::new(Notify::new());
        let mut a = setup(
            bodies()
                .into_iter()
                .enumerate()
                .map(|(i, b)| (b, (i == 3).then(|| gate.clone())))
                .collect(),
            store.clone(),
        )
        .await;
        let mut b = setup(replies(bodies()), store.clone()).await;
        let first_provider = if caller {
            a.provider.caller_scope(&first.caller().unwrap()).unwrap()
        } else {
            a.provider.clone()
        };
        let second_provider = if caller {
            b.provider.caller_scope(&second.caller().unwrap()).unwrap()
        } else {
            b.provider.clone()
        };
        let pending = tokio::spawn(async move {
            invoke(
                &first_provider,
                Action::Play,
                (!caller).then_some("default"),
            )
            .await
        });
        let mut first_calls = Vec::new();
        for _ in 0..4 {
            first_calls.push(
                tokio::time::timeout(Duration::from_secs(3), a.network.seen.recv())
                    .await
                    .unwrap()
                    .unwrap(),
            );
        }
        let content = invoke(
            &second_provider,
            Action::Download,
            (!caller).then_some("personal"),
        )
        .await
        .unwrap();
        assert!(!pending.is_finished());
        gate.notify_one();
        assert_eq!(pending.await.unwrap().unwrap().bytes, content.bytes);
        (&mut a.network.server).await.unwrap();
        let second_calls = fixture::requests(&mut b.network, 4).await;
        for (calls, uid, sid) in [
            (&first_calls, "42", "first-session"),
            (&second_calls, "43", "second-session"),
        ] {
            assert_eq!(data::query(&calls[1])["uid"], uid);
            assert_eq!(data::query(&calls[2])["loginSid"], sid);
            assert!(!calls[3].contains(sid));
        }
        assert_eq!(*store.values.lock().unwrap(), original);
    }
}
