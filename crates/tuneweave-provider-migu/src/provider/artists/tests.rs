use super::*;
use crate::client::artists::tests::{album, bio, data, info, track};
use crate::provider::catalog::tests::server;
use serde_json::Value;
fn response(v: Value) -> String {
    let body = json!({"code":"000000","data":v}).to_string();
    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}
fn tracks() -> Vec<Value> {
    (1..=53).map(|n| track(&n.to_string(), "112")).collect()
}
fn album_rows() -> Vec<Value> {
    (0..23)
        .map(|n| album(&(n / 2 + 1).to_string(), n % 2 != 0))
        .collect()
}
fn frames(operation: ArtistOperation, total: Option<u64>) -> Vec<String> {
    let items = if operation == ArtistOperation::Songs {
        tracks()
    } else {
        album_rows()
    };
    let width = if operation == ArtistOperation::Songs {
        50
    } else {
        10
    };
    let mut result = vec![response(info(
        "112",
        if operation == ArtistOperation::Songs {
            total
        } else {
            None
        },
        if operation == ArtistOperation::Albums {
            total
        } else {
            None
        },
    ))];
    let pages = items.chunks(width).collect::<Vec<_>>();
    for (i, chunk) in pages.iter().enumerate() {
        result.push(response(data(
            operation,
            "112",
            i as u32 + 1,
            chunk.to_vec(),
            i + 1 < pages.len(),
        )));
    }
    result
}
fn request(limit: u32, offset: u32) -> ArtistTrackListRequest {
    ArtistTrackListRequest {
        limit,
        offset,
        account: None,
        order: ArtistTrackOrder::PlatformDefault,
    }
}

#[tokio::test]
async fn full_artist_tracks_use_fixed_https_protocol_parameters_before_slicing() {
    for total in [None, Some(53)] {
        for (limit, offset) in [(5, 48), (3, 51), (2, 99)] {
            let (p, requests) = server(frames(ArtistOperation::Songs, total)).await;
            let result = p
                .artist_tracks("112", &request(limit, offset))
                .await
                .unwrap();
            assert_eq!(result.pagination.total, Some(53));
            assert_eq!(result.pagination.extensions["order"], "platform_default");
            assert_eq!(
                result
                    .items
                    .iter()
                    .map(|v| v.id.clone())
                    .collect::<Vec<_>>(),
                (1..=53)
                    .skip(offset as usize)
                    .take(limit as usize)
                    .map(|n| n.to_string())
                    .collect::<Vec<_>>()
            );
            let requests = requests.await.unwrap();
            assert_eq!(requests.len(), 3);
            assert!(requests[0].starts_with("GET /pc/bmw/singer/info/v1.1?singerId=112 "));
            for (i, r) in requests[1..].iter().enumerate() {
                assert!(r.starts_with(&format!(
                    "GET /pc/bmw/singer/song/v1.0?singerId=112&pageNo={}&type=1 ",
                    i + 1
                )));
            }
            for r in requests {
                assert!(!r.contains("cookie:"));
                assert!(!r.contains("pacmtoken:"));
            }
        }
    }
}

#[tokio::test]
async fn artist_album_types_read_all_mixed_pages_and_keep_equal_numeric_ids_separate() {
    for digital in [false, true] {
        for total in [None, Some(23)] {
            for offset in [0, 8, 99] {
                let (p, requests) = server(frames(ArtistOperation::Albums, total)).await;
                let r = PageRequest::new(4, offset);
                let value = if digital {
                    serde_json::to_value(p.artist_digital_albums("112", &r).await.unwrap()).unwrap()
                } else {
                    serde_json::to_value(p.artist_albums("112", &r).await.unwrap()).unwrap()
                };
                let count = if digital { 11 } else { 12 };
                assert_eq!(value["pagination"]["total"], count);
                assert_eq!(value["pagination"]["extensions"]["upstream_raw_count"], 23);
                assert_eq!(
                    value["items"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|v| v["id"].as_str().unwrap().to_owned())
                        .collect::<Vec<_>>(),
                    (1..=count)
                        .skip(offset as usize)
                        .take(4)
                        .map(|n| n.to_string())
                        .collect::<Vec<_>>()
                );
                assert_eq!(requests.await.unwrap().len(), 4);
            }
        }
    }
}

#[tokio::test]
async fn artist_detail_and_overview_combine_verified_metadata_with_biography_and_explicit_preview()
{
    let (p, requests) = server(vec![
        response(info("112", Some(53), Some(23))),
        response(bio()),
    ])
    .await;
    let artist = p.artist("112", None).await.unwrap();
    assert_eq!(artist.description, "First line\nSecond line");
    assert_eq!(artist.biography_sections.len(), 1);
    assert_eq!(requests.await.unwrap().len(), 2);
    let mut replies = frames(ArtistOperation::Songs, Some(53));
    replies.push(response(bio()));
    let (p, requests) = server(replies).await;
    let view = p.artist_overview("112", None).await.unwrap();
    assert_eq!(view.featured_tracks.len(), 10);
    assert!(view.has_more_tracks);
    assert_eq!(view.extensions["order"], "platform_default");
    assert_eq!(view.artist.track_count, Some(53));
    assert_eq!(requests.await.unwrap().len(), 4);
}

#[tokio::test]
async fn artist_later_page_identity_continuation_count_and_transport_errors_do_not_return_partial_lists()
 {
    for operation in [ArtistOperation::Songs, ArtistOperation::Albums] {
        for mutation in 0..6 {
            let total = if operation == ArtistOperation::Songs {
                53
            } else {
                23
            };
            let width = if operation == ArtistOperation::Songs {
                50
            } else {
                10
            };
            let items = if operation == ArtistOperation::Songs {
                tracks()
            } else {
                album_rows()
            };
            let mut last = data(
                operation,
                "112",
                2,
                items[width..].iter().take(width).cloned().collect(),
                false,
            );
            match mutation {
                0 => {
                    if operation == ArtistOperation::Songs {
                        last["contents"][1]["contents"][0] = track("1", "112");
                    } else {
                        last["contents"][0] = album("1", false);
                    }
                }
                1 => last["header"]["nextPageUrl"] = json!("http://outside.invalid/page"),
                2 => last["contents"] = json!([]),
                3 => {
                    if operation == ArtistOperation::Songs {
                        last["contents"][1]["contents"][0]["songItem"]["singerList"][0]["id"] =
                            json!("113");
                    } else {
                        last["contents"][0]["resType"] = json!("2");
                    }
                }
                4 => {
                    last["header"]["nextPageUrl"] = json!(format!(
                        "http://app.c.nf.migu.cn/MIGUM3.0/bmw/singer/{}/v1.0?singerId=112&pageNo=2{}",
                        if operation == ArtistOperation::Songs {
                            "song"
                        } else {
                            "album"
                        },
                        if operation == ArtistOperation::Songs {
                            "&type=1"
                        } else {
                            ""
                        }
                    ))
                }
                _ => {}
            }
            let mut replies = vec![
                response(info("112", Some(total), Some(total))),
                response(data(operation, "112", 1, items[..width].to_vec(), true)),
            ];
            replies.push(if mutation == 5 {
                "HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    .into()
            } else {
                response(last)
            });
            let (p, requests) = server(replies).await;
            let result = if operation == ArtistOperation::Songs {
                p.artist_tracks("112", &request(1, 0)).await.map(|_| ())
            } else {
                p.artist_albums("112", &PageRequest::new(1, 0))
                    .await
                    .map(|_| ())
            };
            assert!(result.is_err());
            assert_eq!(requests.await.unwrap().len(), 3);
        }
    }
}

#[tokio::test]
async fn artist_zero_counts_unknown_counts_and_read_budget_are_explicit() {
    for operation in [ArtistOperation::Songs, ArtistOperation::Albums] {
        let (p, requests) = server(vec![
            response(info("112", Some(0), Some(0))),
            response(data(operation, "112", 1, vec![], false)),
        ])
        .await;
        let count = if operation == ArtistOperation::Songs {
            p.artist_tracks("112", &request(1, 0))
                .await
                .unwrap()
                .items
                .len()
        } else {
            p.artist_albums("112", &PageRequest::new(1, 0))
                .await
                .unwrap()
                .items
                .len()
        };
        assert_eq!(count, 0);
        assert_eq!(requests.await.unwrap().len(), 2);
    }
    let (p, requests) = server(vec![response(info("112", Some(6401), Some(1281)))]).await;
    assert!(p.artist_tracks("112", &request(1, 0)).await.is_err());
    assert_eq!(requests.await.unwrap().len(), 1);
    let mut replies = vec![response(info("112", None, None))];
    for number in 1..=128 {
        replies.push(response(data(
            ArtistOperation::Albums,
            "112",
            number,
            vec![album(&number.to_string(), true)],
            true,
        )));
    }
    let (p, requests) = server(replies).await;
    assert!(
        p.artist_albums("112", &PageRequest::new(1, 0))
            .await
            .is_err()
    );
    assert_eq!(requests.await.unwrap().len(), 129);
}

#[tokio::test]
async fn artist_public_methods_reject_accounts_caller_credentials_orders_and_invalid_pagination_before_network()
 {
    let (p, requests) = server(vec![]).await;
    for uid in ["", "0112", "x/112", &"1".repeat(65)] {
        assert!(p.artist(uid, None).await.is_err());
    }
    assert!(p.artist("112", Some("A")).await.is_err());
    assert!(p.artist_overview("112", Some("A")).await.is_err());
    for (limit, offset) in [(0, 0), (101, 0), (2, u32::MAX)] {
        assert!(
            p.artist_tracks("112", &request(limit, offset))
                .await
                .is_err()
        );
        assert!(
            p.artist_albums("112", &PageRequest::new(limit, offset))
                .await
                .is_err()
        );
        assert!(
            p.artist_digital_albums("112", &PageRequest::new(limit, offset))
                .await
                .is_err()
        );
    }
    for order in [ArtistTrackOrder::Hot, ArtistTrackOrder::Time] {
        let mut r = request(1, 0);
        r.order = order;
        assert!(p.artist_tracks("112", &r).await.is_err());
    }
    let c = crate::credential::MiguCredential::verified("111".into(), "fixture".into()).unwrap();
    let p = p.caller_scope(&c.caller().unwrap()).unwrap();
    assert!(p.artist("112", None).await.is_err());
    assert!(p.artist_overview("112", None).await.is_err());
    assert!(p.artist_tracks("112", &request(1, 0)).await.is_err());
    assert!(
        p.artist_albums("112", &PageRequest::new(1, 0))
            .await
            .is_err()
    );
    assert!(
        p.artist_digital_albums("112", &PageRequest::new(1, 0))
            .await
            .is_err()
    );
    assert!(requests.await.unwrap().is_empty());
}

#[tokio::test]
#[ignore = "requires official public Migu artist endpoints; no account or audio"]
async fn live_migu_artist_complete_catalogs_preserve_actual_identity_counts_and_album_types() {
    let p = MiguProvider::new(MiguConfig::default()).unwrap();
    let artist = p.artist("112", None).await.unwrap();
    assert_eq!(artist.name, "周杰伦");
    assert!(!artist.description.is_empty());
    let tracks = p.artist_tracks("112", &request(5, 48)).await.unwrap();
    assert_eq!(tracks.items.len(), 5);
    assert_eq!(tracks.pagination.total, artist.track_count);
    assert!(tracks.items.iter().all(|t| {
        t.artists
            .iter()
            .any(|a| a.resource_ref.as_ref().is_some_and(|r| r.id() == "112"))
    }));
    let ordinary = p
        .artist_albums("112", &PageRequest::new(3, 0))
        .await
        .unwrap();
    let digital = p
        .artist_digital_albums("112", &PageRequest::new(3, 0))
        .await
        .unwrap();
    assert!(!ordinary.items.is_empty());
    assert!(!digital.items.is_empty());
    assert_eq!(
        ordinary.pagination.total.unwrap() + digital.pagination.total.unwrap(),
        artist.album_count.unwrap()
    );
    assert!(digital.items.iter().all(|v| v.purchased.is_none()));
}
