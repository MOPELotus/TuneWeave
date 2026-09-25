use super::*;
use crate::provider::catalog::tests::server;
use serde_json::Value;
use tuneweave_core::ErrorCode;

fn metadata(digital: bool, id: &str, count: Option<u64>) -> Value {
    if digital {
        json!({"resourceType":"5","contentId":id,"itemId":"999","title":"Digital album","totalCount":count.map(|n|n.to_string()),"songItems":[]})
    } else {
        json!({"resourceType":"2003","albumId":id,"title":"Ordinary album","totalCount":count.map(|n|n.to_string())})
    }
}
fn song(id: u32, album: &str) -> Value {
    json!({"resourceType":"2","contentId":id.to_string(),"songId":format!("song{id}"),"copyrightId":format!("copyright{id}"),"songName":format!("Song{id}"),"albumId":album,"album":"Actual ordinary album"})
}
fn tracks(digital: bool, ids: &[u32], total: Option<u64>, more: bool) -> Value {
    let mut value = json!({"songList":ids.iter().map(|id|song(*id,if digital {"444"} else {"77"})).collect::<Vec<_>>(),"totalCount":total});
    if !digital {
        value["hasNext"] = json!(more);
    }
    value
}
fn response(data: Value) -> String {
    raw_response(json!({"code":"000000","data":data}))
}
fn raw_response(value: Value) -> String {
    let body = value.to_string();
    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

#[tokio::test]
async fn album_tracks_complete_variable_pages_before_slicing_and_preserve_original_associations() {
    for digital in [false, true] {
        for offset in [0, 1, 2, 10, u32::MAX] {
            let mut replies = vec![response(metadata(digital, "77", None))];
            if digital {
                replies.push(response(tracks(true, &[11, 22, 22], Some(3), false)));
            } else {
                replies.push(response(tracks(false, &[11, 22], Some(3), true)));
                replies.push(response(tracks(false, &[22], Some(3), false)));
            }
            let count = replies.len();
            let (provider, requests) = server(replies).await;
            let request = PageRequest::new(1, offset);
            let page = if digital {
                provider.digital_album_tracks("77", &request).await
            } else {
                provider.album_tracks("77", &request).await
            }
            .unwrap();
            let expected: Vec<_> = ["11", "22", "22"]
                .into_iter()
                .skip(offset as usize)
                .take(1)
                .collect();
            assert_eq!(
                page.items.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(),
                expected
            );
            assert_eq!(page.pagination.total, Some(3));
            assert_eq!(page.pagination.has_more, offset < 2);
            assert_eq!(page.pagination.extensions["complete_snapshot"], true);
            assert_eq!(
                page.pagination.extensions["collection_type"],
                if digital { "digital_album" } else { "album" }
            );
            for track in page.items {
                assert_eq!(
                    track.album.unwrap().resource_ref.unwrap().to_string(),
                    if digital { "migu:444" } else { "migu:77" }
                );
                assert!(track.extensions.contains_key("song_id"));
            }
            let requests = requests.await.unwrap();
            assert_eq!(requests.len(), count);
            for (index, request) in requests.iter().enumerate() {
                let url = url::Url::parse(&format!(
                    "https://app.c.nf.migu.cn{}",
                    request
                        .lines()
                        .next()
                        .unwrap()
                        .split_whitespace()
                        .nth(1)
                        .unwrap()
                ))
                .unwrap();
                let params: std::collections::BTreeMap<_, _> =
                    url.query_pairs().into_owned().collect();
                assert_eq!(params[if digital { "dAlbumId" } else { "albumId" }], "77");
                assert_eq!(
                    url.path(),
                    match (digital, index == 0) {
                        (true, true) => "/MIGUM3.0/resource/dalbum/v2.0",
                        (true, false) => "/MIGUM3.0/resource/dalbum/song/v2.0",
                        (false, true) => "/MIGUM3.0/resource/album/v2.0",
                        (false, false) => "/MIGUM3.0/resource/album/song/v2.0",
                    }
                );
                if !digital && index > 0 {
                    assert_eq!(params["pageNo"], index.to_string());
                } else {
                    assert!(!params.contains_key("pageNo"));
                }
                assert!(!request.to_ascii_lowercase().contains("cookie:"));
            }
        }
    }
}

#[tokio::test]
async fn album_details_reject_success_without_data_and_mismatched_or_wrong_type_metadata() {
    for digital in [false, true] {
        for mutation in ["missing", "id", "type", "status"] {
            let mut body = json!({"code":"000000","data":metadata(digital,"77",Some(1))});
            match mutation {
                "missing" => {
                    body.as_object_mut().unwrap().remove("data");
                }
                "id" => body["data"][if digital { "contentId" } else { "albumId" }] = json!("999"),
                "type" => body["data"]["resourceType"] = json!("2"),
                "status" => body["code"] = json!("111111"),
                _ => unreachable!(),
            }
            let (provider, requests) = server(vec![raw_response(body)]).await;
            let error = if digital {
                provider.digital_album("77", None).await.map(|_| ())
            } else {
                provider.album("77", None).await.map(|_| ())
            }
            .unwrap_err();
            assert_eq!(
                error.code,
                if mutation == "missing" {
                    ErrorCode::ResourceNotFound
                } else {
                    ErrorCode::UpstreamError
                }
            );
            assert_eq!(requests.await.unwrap().len(), 1);
        }
    }
}

#[tokio::test]
async fn album_track_reads_reject_incomplete_counts_wrong_identity_and_continuation() {
    for digital in [false, true] {
        for mutation in [
            "changed_total",
            "early_end",
            "missing_total",
            "empty_more",
            "wrong_track_type",
            "wrong_album",
            "oversize",
            "unknown_continuation",
        ] {
            if digital && mutation == "wrong_album" {
                continue;
            }
            let mut data = tracks(digital, &[11, 22], Some(2), false);
            let mut metadata_count = Some(2);
            match mutation {
                "changed_total" => data["totalCount"] = json!(3),
                "early_end" => {
                    data["songList"] = json!([song(11, if digital { "444" } else { "77" })])
                }
                "missing_total" => {
                    data.as_object_mut().unwrap().remove("totalCount");
                    metadata_count = None;
                }
                "empty_more" => {
                    data["songList"] = json!([]);
                    data["hasNext"] = json!(true);
                }
                "wrong_track_type" => data["songList"][0]["resourceType"] = json!("5"),
                "wrong_album" => data["songList"][0]["albumId"] = json!("444"),
                "oversize" => data["songList"] = json!(vec![song(11, "77"); 1001]),
                "unknown_continuation" => {
                    if digital {
                        data["hasNext"] = json!(true);
                    } else {
                        data.as_object_mut().unwrap().remove("hasNext");
                    }
                }
                _ => unreachable!(),
            }
            let (provider, requests) = server(vec![
                response(metadata(digital, "77", metadata_count)),
                response(data),
            ])
            .await;
            let error = if digital {
                provider
                    .digital_album_tracks("77", &PageRequest::new(1, 0))
                    .await
            } else {
                provider.album_tracks("77", &PageRequest::new(1, 0)).await
            }
            .unwrap_err();
            assert_eq!(error.code, ErrorCode::UpstreamError, "{digital} {mutation}");
            assert_eq!(requests.await.unwrap().len(), 2);
        }
    }
}

#[tokio::test]
async fn ordinary_album_traversal_detects_repeated_pages_and_enforces_its_request_budget() {
    let (provider, requests) = server(vec![
        response(metadata(false, "77", Some(4))),
        response(tracks(false, &[11, 22], Some(4), true)),
        response(tracks(false, &[11, 22], Some(4), false)),
    ])
    .await;
    assert_eq!(
        provider
            .album_tracks("77", &PageRequest::new(1, 0))
            .await
            .unwrap_err()
            .code,
        ErrorCode::UpstreamError
    );
    assert_eq!(requests.await.unwrap().len(), 3);
    let mut replies = vec![response(metadata(false, "77", Some(65)))];
    for id in 1..=64 {
        replies.push(response(tracks(false, &[id], Some(65), true)));
    }
    let (provider, requests) = server(replies).await;
    assert_eq!(
        provider
            .album_tracks("77", &PageRequest::new(1, 0))
            .await
            .unwrap_err()
            .code,
        ErrorCode::UpstreamError
    );
    assert_eq!(requests.await.unwrap().len(), 65);
}

#[tokio::test]
async fn album_inputs_are_rejected_before_network_and_empty_counted_albums_are_valid() {
    let provider = MiguProvider::new(MiguConfig::default()).unwrap();
    for digital in [false, true] {
        for (id, account, limit) in [
            ("01", None, 20),
            ("77", Some("personal"), 20),
            ("", None, 20),
            ("77", None, 0),
            ("77", None, 101),
        ] {
            let request = PageRequest {
                account: account.map(str::to_owned),
                ..PageRequest::new(limit, 0)
            };
            let error = if digital {
                provider.digital_album_tracks(id, &request).await
            } else {
                provider.album_tracks(id, &request).await
            }
            .unwrap_err();
            assert_eq!(error.code, ErrorCode::InvalidRequest);
        }
        let (provider, requests) = server(vec![
            response(metadata(digital, "77", Some(0))),
            response(tracks(digital, &[], Some(0), false)),
        ])
        .await;
        let page = if digital {
            provider
                .digital_album_tracks("77", &PageRequest::new(20, 0))
                .await
        } else {
            provider.album_tracks("77", &PageRequest::new(20, 0)).await
        }
        .unwrap();
        assert!(page.items.is_empty());
        assert_eq!(page.pagination.total, Some(0));
        assert!(!page.pagination.has_more);
        assert_eq!(requests.await.unwrap().len(), 2);
    }
}

#[tokio::test]
async fn mixed_album_search_preserves_types_positions_and_identity_namespace() {
    let mut first = Vec::new();
    for id in 1..=20 {
        first.push(if id % 2 == 0 {
            json!({"album":metadata(false,&id.to_string(),None)})
        } else {
            json!({"dalbum":metadata(true,&id.to_string(),Some(1))})
        });
    }
    let second = vec![
        json!({"album":metadata(false,"21",Some(0))}),
        json!({"dalbum":metadata(true,"21",None)}),
    ];
    let (provider, requests) = server(vec![
        raw_response(json!({"code":"000000","data":{"hasNext":true,"items":first}})),
        raw_response(json!({"code":"000000","data":{"hasNext":false,"items":second}})),
    ])
    .await;
    let result = provider
        .search_catalog(&SearchQuery {
            kind: SearchKind::Album,
            ..SearchQuery::tracks("albums", 5, 17)
        })
        .await
        .unwrap();
    assert_eq!(
        result
            .items
            .iter()
            .map(|v| match v {
                SearchItem::Album(a) => ("album", a.id.as_str()),
                SearchItem::DigitalAlbum(a) => ("digital_album", a.id.as_str()),
                _ => panic!("wrong type"),
            })
            .collect::<Vec<_>>(),
        [
            ("album", "18"),
            ("digital_album", "19"),
            ("album", "20"),
            ("album", "21"),
            ("digital_album", "21")
        ]
    );
    assert!(!result.pagination.has_more);
    assert!(result.pagination.total.is_none());
    let value = serde_json::to_value(&result.items).unwrap();
    assert_eq!(value[1]["type"], "digital_album");
    assert!(value[4]["data"]["track_count"].is_null());
    for request in requests.await.unwrap() {
        assert!(request.starts_with("GET /bmw/search/album/v1.0?"));
        assert!(request.contains("typeOrder=0"));
    }
}

#[tokio::test]
#[ignore = "requires public Migu album endpoints; no account or audio"]
async fn live_migu_albums_preserve_search_detail_and_track_identities() {
    let provider = MiguProvider::new(MiguConfig::default()).unwrap();
    let page = provider
        .search_catalog(&SearchQuery {
            kind: SearchKind::Album,
            ..SearchQuery::tracks("周杰伦", 30, 0)
        })
        .await
        .unwrap();
    assert_eq!(page.items.len(), 30);
    let ordinary = page
        .items
        .iter()
        .find_map(|v| {
            if let SearchItem::Album(v) = v {
                Some(v)
            } else {
                None
            }
        })
        .unwrap();
    let digital = page
        .items
        .iter()
        .find_map(|v| {
            if let SearchItem::DigitalAlbum(v) = v {
                Some(v)
            } else {
                None
            }
        })
        .unwrap();
    let album = provider.album(&ordinary.id, None).await.unwrap();
    assert_eq!(album.resource_ref, ordinary.resource_ref);
    assert_eq!(album.name, ordinary.name);
    let tracks = provider
        .album_tracks(&ordinary.id, &PageRequest::new(100, 0))
        .await
        .unwrap();
    assert_eq!(tracks.pagination.total, album.track_count);
    assert!(!tracks.items.is_empty());
    let product = provider.digital_album(&digital.id, None).await.unwrap();
    assert_eq!(product.resource_ref, digital.resource_ref);
    assert_eq!(product.name, digital.name);
    assert!(product.purchased.is_none());
    assert!(product.cover_url.is_some());
    let tracks = provider
        .digital_album_tracks(&digital.id, &PageRequest::new(100, 0))
        .await
        .unwrap();
    assert_eq!(tracks.pagination.total, product.track_count);
    assert!(!tracks.items.is_empty());
    assert!(tracks.items.iter().all(|t| {
        t.album
            .as_ref()
            .and_then(|a| a.resource_ref.as_ref())
            .is_some_and(|r| r.id() != product.id)
    }));
}
