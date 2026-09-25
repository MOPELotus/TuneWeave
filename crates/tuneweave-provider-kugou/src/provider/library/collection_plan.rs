//! Read-only input preparation for the still-disabled Concept subscription writer.
use super::*;
use crate::KugouLoginClient;
use crate::client::concept_collection::CollectionSyncPlan;

impl KugouProvider {
    // Deliberately not reachable from public routes until the separate privilege,
    // create and type-1 synchronization contracts are implemented and verified.
    #[allow(dead_code)]
    pub(super) async fn prepare_concept_collection(
        &self,
        read: &mut session::AccountRead,
        global_id: &str,
    ) -> Result<CollectionSyncPlan> {
        let session = read.session()?.clone();
        if session.client != KugouLoginClient::Concept {
            return Err(TuneWeaveError::new(
                ErrorCode::CapabilityNotSupported,
                "KuGou collection input preparation requires a Concept account",
            )
            .with_platform(Platform::Kugou));
        }
        self.check_account_read(read)?;
        let result = self
            .client
            .concept_collection_plan(global_id, &session.device, &session.user_id, || {
                self.check_account_read(read)
            })
            .await;
        self.check_account_read(read)?;
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::library::tests::store_account;
    use crate::provider::session::tests::{exchange, paused, profile, read, reply, server};
    use serde_json::Value;

    const GID: &str = "collection_1_222_88_0";
    fn metadata() -> Value {
        json!({"name":"Source","global_collection_id":GID,"list_create_userid":222,
            "list_create_listid":88,"specialid":900,"listid":88,"source":1,
            "is_publish":1,"count":0})
    }
    fn page() -> Value {
        json!({"userid":222,"listid":88,"list_ver":7,"count":0,
            "pagesize":100,"list_info":metadata(),"info":[]})
    }
    fn set_concept(provider: &mut KugouProvider) -> Arc<session::tests::Store> {
        let store = store_account(provider);
        let mut source = read(&store, "A").native().session.clone();
        source.client = KugouLoginClient::Concept;
        store
            .put(
                &KugouCredential::verified(source)
                    .unwrap()
                    .stored("A")
                    .unwrap(),
            )
            .unwrap();
        store
    }

    #[tokio::test]
    async fn concept_collection_plan_provider_supports_server_and_caller_without_business_writes() {
        for caller in [false, true] {
            let mut f = server(vec![
                exchange("111", "next").into(),
                profile("111").into(),
                reply(json!([metadata()])).into(),
                reply(json!([metadata()])).into(),
                reply(page()).into(),
            ])
            .await;
            let store = set_concept(&mut f.provider);
            let before = read(&store, "A");
            let other = read(&store, "B");
            let provider = if caller {
                f.provider.caller_scope(&before.caller().unwrap()).unwrap()
            } else {
                f.provider.clone()
            };
            let mut selected = provider
                .begin_native_read(if caller { "default" } else { "A" }, Some("111"))
                .await
                .unwrap();
            let plan = provider
                .prepare_concept_collection(&mut selected, GID)
                .await
                .unwrap();
            assert_eq!(serde_json::to_value(plan).unwrap()["occurrence_count"], 0);
            assert_eq!(read(&store, "B"), other);
            if caller {
                assert_eq!(read(&store, "A"), before);
            }
            let all = f.requests.await.unwrap();
            assert_eq!(all.len(), 5);
            assert!(all[2].starts_with("POST /v3/get_list_info?"));
            assert!(all[3].starts_with("POST /v1/get_list_info?"));
            assert!(all[4].starts_with("GET /pubsongs/v2/get_other_list_file_nofilt?"));
            assert!(
                all[2..]
                    .iter()
                    .all(|request| !request.contains("next") && !request.contains("original"))
            );
        }
    }

    #[tokio::test]
    async fn concept_collection_plan_provider_generation_change_stops_before_song_pages() {
        let (frame, release) = paused(reply(json!([metadata()])));
        let mut f = server(vec![
            exchange("111", "next").into(),
            profile("111").into(),
            reply(json!([metadata()])).into(),
            frame,
        ])
        .await;
        let store = set_concept(&mut f.provider);
        let provider = f.provider.clone();
        let task = tokio::spawn(async move {
            let mut selected = provider.begin_native_read("A", Some("111")).await.unwrap();
            provider
                .prepare_concept_collection(&mut selected, GID)
                .await
        });
        for _ in 0..4 {
            f.seen.recv().await.unwrap();
        }
        let mut newer = read(&store, "A").native().session.clone();
        newer.token = "replacement".into();
        let newer = KugouCredential::verified(newer).unwrap();
        store.put(&newer.stored("A").unwrap()).unwrap();
        release.send(()).unwrap();
        assert_eq!(task.await.unwrap().unwrap_err().code, ErrorCode::Conflict);
        assert_eq!(read(&store, "A"), newer);
        assert_eq!(f.requests.await.unwrap().len(), 4);
    }

    #[tokio::test]
    async fn concept_collection_plan_provider_standard_is_rejected_before_catalogue_io() {
        let mut f = server(vec![exchange("111", "next").into(), profile("111").into()]).await;
        store_account(&mut f.provider);
        let mut selected = f
            .provider
            .begin_native_read("A", Some("111"))
            .await
            .unwrap();
        assert_eq!(
            f.provider
                .prepare_concept_collection(&mut selected, GID)
                .await
                .unwrap_err()
                .code,
            ErrorCode::CapabilityNotSupported
        );
        assert_eq!(f.requests.await.unwrap().len(), 2);
    }
}
