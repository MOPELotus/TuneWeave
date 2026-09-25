use tuneweave_core::{ArtistTrackListRequest, ArtistTrackOrder};

use super::*;

fn responses(kind: &str) -> Vec<String> {
    let info = json!({"status_info":{"now":1,"now_ts_ms":1000},"artist_info":{"id":"123","name":"Artist","count_tracks":52,"count_albums":52}});
    let pages=[(1..=50,true,"50"),(51..=52,false,"401")].into_iter().map(|(range,more,cursor)|{
        let mut p=json!({"status_info":{"now":1,"now_ts_ms":1000},"has_more":more,"next_cursor":cursor});
        p[kind]=json!(range.map(|i|json!({"id":i.to_string(),"name":format!("Work {i}"),"artists":[{"id":"123","name":"Artist"}]})).collect::<Vec<_>>());
        p
    });
    std::iter::once(info)
        .chain(pages)
        .map(|v| crate::test_http::json(&v.to_string(), None))
        .collect()
}

#[tokio::test]
async fn artist_catalogue_provider_slices_cross_page_tail_and_out_of_range_after_complete_read() {
    for kind in ["tracks", "albums"] {
        for offset in [49, 51, 100] {
            let (origin, server) = crate::test_http::serve(responses(kind)).await;
            let provider = SodaProvider::from_client(
                SodaClient::new(&SodaConfig::default())
                    .unwrap()
                    .with_auth_test_origin(origin),
            );
            let (ids, pagination) = if kind == "tracks" {
                let p = provider
                    .artist_tracks(
                        "123",
                        &ArtistTrackListRequest {
                            limit: 2,
                            offset,
                            account: None,
                            order: ArtistTrackOrder::PlatformDefault,
                        },
                    )
                    .await
                    .unwrap();
                (
                    p.items.into_iter().map(|x| x.id).collect::<Vec<_>>(),
                    p.pagination,
                )
            } else {
                let p = provider
                    .artist_albums("123", &PageRequest::new(2, offset))
                    .await
                    .unwrap();
                (
                    p.items.into_iter().map(|x| x.id).collect::<Vec<_>>(),
                    p.pagination,
                )
            };
            let expected = match offset {
                49 => vec!["50", "51"],
                51 => vec!["52"],
                _ => vec![],
            };
            assert_eq!(ids, expected);
            assert_eq!(pagination.total, Some(52));
            assert_eq!(
                pagination.next_offset,
                if offset == 49 { Some(51) } else { None }
            );
            assert_eq!(pagination.has_more, offset == 49);
            assert_eq!(pagination.extensions["complete_read"], true);
            assert_eq!(pagination.extensions["upstream_pages"], 2);
            assert_eq!(server.await.unwrap().len(), 3);
        }
    }
}

#[tokio::test]
async fn artist_catalogue_provider_rejects_scope_sort_identity_and_window_before_io() {
    let fixture = SessionFixture::new();
    let credential = test_soda_credential().bind_user("123456").unwrap();
    fixture.put("default", &credential);
    let mut provider = fixture.provider.clone();
    provider.client = provider
        .client
        .clone()
        .with_auth_test_origin(url::Url::parse("http://127.0.0.1:1/").unwrap());
    let good = ArtistTrackListRequest {
        limit: 10,
        offset: 0,
        account: None,
        order: ArtistTrackOrder::PlatformDefault,
    };
    for id in ["0", "0123", " 123", "123 ", "abc", "18446744073709551616"] {
        assert_eq!(
            provider.artist_tracks(id, &good).await.err().unwrap().code,
            ErrorCode::InvalidRequest
        );
    }
    for order in [ArtistTrackOrder::Hot, ArtistTrackOrder::Time] {
        assert_eq!(
            provider
                .artist_tracks(
                    "123",
                    &ArtistTrackListRequest {
                        order,
                        ..good.clone()
                    }
                )
                .await
                .err()
                .unwrap()
                .code,
            ErrorCode::InvalidRequest
        );
    }
    for (limit, offset) in [(0, 0), (101, 0), (100, u32::MAX)] {
        assert_eq!(
            provider
                .artist_tracks(
                    "123",
                    &ArtistTrackListRequest {
                        limit,
                        offset,
                        ..good.clone()
                    }
                )
                .await
                .err()
                .unwrap()
                .code,
            ErrorCode::InvalidRequest
        );
        assert_eq!(
            provider
                .artist_albums("123", &PageRequest::new(limit, offset))
                .await
                .err()
                .unwrap()
                .code,
            ErrorCode::InvalidRequest
        );
    }
    for account in [""] {
        assert_eq!(
            provider
                .artist_tracks(
                    "123",
                    &ArtistTrackListRequest {
                        account: Some(account.to_owned()),
                        ..good.clone()
                    }
                )
                .await
                .err()
                .unwrap()
                .code,
            ErrorCode::InvalidRequest
        );
        assert_eq!(
            provider
                .artist_albums(
                    "123",
                    &PageRequest {
                        account: Some(account.to_owned()),
                        ..PageRequest::new(10, 0)
                    }
                )
                .await
                .err()
                .unwrap()
                .code,
            ErrorCode::InvalidRequest
        );
    }
    let caller = provider
        .caller_credential_scope(&caller_from(&credential))
        .unwrap();
    assert_eq!(
        caller
            .artist_tracks(
                "123",
                &ArtistTrackListRequest {
                    account: Some("personal".into()),
                    ..good
                }
            )
            .await
            .err()
            .unwrap()
            .code,
        ErrorCode::InvalidRequest
    );
    assert_eq!(
        caller
            .artist_albums(
                "123",
                &PageRequest {
                    account: Some("personal".into()),
                    ..PageRequest::new(10, 0)
                }
            )
            .await
            .err()
            .unwrap()
            .code,
        ErrorCode::InvalidRequest
    );
    assert!(caller.take_response_credential().unwrap().is_none());
    assert_eq!(
        fixture.stored("default").unwrap().secret(),
        credential.serialize().unwrap()
    );
}

#[tokio::test]
async fn artist_catalogue_provider_ignores_default_credentials_and_discards_late_failures() {
    let mut fixture = SessionFixture::new();
    let credential = test_soda_credential().bind_user("123456").unwrap();
    fixture.put("default", &credential);
    let mut replies = responses("tracks");
    *replies.last_mut().unwrap() = crate::test_http::json(
        r#"{"status_code":1000016,"status_info":{"now":1,"now_ts_ms":1000}}"#,
        Some("sessionid_ss=unrelated; Path=/"),
    );
    let (origin, server) = crate::test_http::serve(replies).await;
    fixture.provider.client = fixture
        .provider
        .client
        .clone()
        .with_auth_test_origin(origin);
    assert_eq!(
        fixture
            .provider
            .artist_tracks(
                "123",
                &ArtistTrackListRequest {
                    limit: 1,
                    offset: 0,
                    account: None,
                    order: ArtistTrackOrder::PlatformDefault
                }
            )
            .await
            .err()
            .unwrap()
            .code,
        ErrorCode::UpstreamError
    );
    let seen = server.await.unwrap();
    assert_eq!(seen.len(), 3);
    assert!(
        seen.iter()
            .all(|r| !r.to_ascii_lowercase().contains("cookie:"))
    );
    assert_eq!(
        fixture.stored("default").unwrap().secret(),
        credential.serialize().unwrap()
    );
    assert!(
        fixture
            .provider
            .take_response_credential()
            .unwrap()
            .is_none()
    );
}
