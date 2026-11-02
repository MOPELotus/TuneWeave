use super::*;
fn page(ids: &[u64], more: bool, cursor: &str, total: u64, cookie: &str) -> String {
    crate::test_http::json(&json!({"status_info":{"now":1,"now_ts_ms":1000},
        "media":ids.iter().map(|id|json!({"id":id.to_string(),"type":"track","entity":{"track":{"id":id.to_string(),"name":"song"}},"play_start_timestamp":1704067200})).collect::<Vec<_>>(),
        "total_num":total,"has_more":more,"next_cursor":cursor}).to_string(), Some(cookie))
}
#[tokio::test]
async fn recent_history_traverses_native_cursors_and_rotates_only_selected_credentials() {
    for owner in ["personal", "default", "caller"] {
        let mut f = SessionFixture::new();
        let source = test_soda_credential().bind_user("123456").unwrap();
        for alias in ["personal", "default", "other"] {
            f.put(alias, &source);
        }
        let (origin, requests) = crate::test_http::serve(vec![
            account_reply("123456", Some("sessionid_ss=verified")),
            page(&[1, 2], true, "next", 3, "sessionid_ss=first"),
            page(&[3], false, "", 3, "sessionid_ss=final"),
        ])
        .await;
        f.provider.client = f.provider.client.clone().with_auth_test_origin(origin);
        let provider = if owner == "caller" {
            f.provider
                .caller_credential_scope(&caller_from(&source))
                .unwrap()
        } else {
            f.provider.clone()
        };
        let mut request = PageRequest::new(1, 2);
        request.account = (owner != "caller").then(|| owner.into());
        let page = provider.recent_track_history(&request).await.unwrap();
        assert_eq!(page.items[0].track.id, "3");
        assert_eq!(page.pagination.total, Some(3));
        assert!(!page.pagination.has_more);
        assert_eq!(
            page.items[0].played_at.as_deref(),
            Some("2024-01-01T00:00:00Z")
        );
        let requests = requests.await.unwrap();
        assert_eq!(requests.len(), 3);
        assert!(requests[1].contains("media_type=track"));
        assert!(requests[2].contains("cursor=next"));
        assert!(requests[2].contains("sessionid_ss=first"));
        assert_eq!(
            f.stored("other").unwrap().secret(),
            source.serialize().unwrap()
        );
        if owner == "caller" {
            assert!(
                provider
                    .take_response_credential()
                    .unwrap()
                    .unwrap()
                    .secret()
                    .contains("final")
            );
        } else {
            assert!(f.stored(owner).unwrap().secret().contains("final"));
        }
    }
}
#[tokio::test]
async fn recent_history_rejects_changed_total_and_late_replacement_without_partial_results() {
    let mut f = SessionFixture::new();
    let source = test_soda_credential().bind_user("123456").unwrap();
    f.put("personal", &source);
    let (origin, requests) = crate::test_http::serve(vec![
        account_reply("123456", None),
        page(&[1], true, "next", 2, "sessionid_ss=first"),
        page(&[2], false, "", 3, "sessionid_ss=poison"),
    ])
    .await;
    f.provider.client = f.provider.client.clone().with_auth_test_origin(origin);
    let mut request = PageRequest::new(2, 0);
    request.account = Some("personal".into());
    assert!(f.provider.recent_track_history(&request).await.is_err());
    requests.await.unwrap();
    assert!(!f.stored("personal").unwrap().secret().contains("poison"));
    assert!(f.provider.take_response_credential().unwrap().is_none());
    let paused = crate::test_http::serve_paused_at(
        vec![
            account_reply("123456", None),
            page(&[1], false, "", 1, "sessionid_ss=poison"),
        ],
        1,
    )
    .await;
    f.provider.client = f
        .provider
        .client
        .clone()
        .with_auth_test_origin(paused.origin);
    let provider = f.provider.clone();
    let task = tokio::spawn(async move { provider.recent_track_history(&request).await });
    paused.arrived.await.unwrap();
    let replacement = test_soda_credential().bind_user("999999").unwrap();
    f.put("personal", &replacement);
    paused.release.send(()).unwrap();
    assert!(task.await.unwrap().is_err());
    paused.requests.await.unwrap();
    assert_eq!(
        f.stored("personal").unwrap().secret(),
        replacement.serialize().unwrap()
    );
}
