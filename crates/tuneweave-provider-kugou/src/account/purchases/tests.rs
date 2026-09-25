use super::*;
use crate::account::tests::{frame, ok, request, server, session};

fn song() -> Value {
    json!({"id":10,"good_scid":555,"album_audio_id":901,"songname":"Purchased song","author_name":"Singer A / Singer B",
    "album_id":70,"album_cover":"https://imge.kugou.com/{size}/cover.jpg","audio_info":{"album_id":"70","album_name":"Album","duration":123456,"hash_128":"1234567890abcdef1234567890abcdef","hash_flac":"ABCDEF1234567890ABCDEF1234567890"}})
}
fn album() -> Value {
    json!({"id":20,"album_id":70,"album_name":"Purchased album","singer_name":"Singer","cover":"https://imge.kugou.com/{size}/album.jpg","buy_total":99,"is_publish":1,"mp_count":0})
}
fn envelope(goods: Vec<Value>, total: u64) -> Vec<u8> {
    serde_json::to_vec(&json!({"status":1,"error_code":0,"data":{"goods":goods,"total":total}}))
        .unwrap()
}

#[test]
fn purchase_dtos_preserve_goods_identity_exact_duration_and_unknown_entitlements() {
    let p = parse(&envelope(vec![song()], 1), "123456789", Kind::Tracks, 1).unwrap();
    let Item::Track(item) = &p.items[0] else {
        panic!()
    };
    let track = item.track.as_ref().unwrap();
    assert_eq!(track.id, "901");
    assert_eq!(item.extensions["goods_id"], "10");
    assert_eq!(item.extensions["good_scid"], "555");
    assert_eq!(track.duration_ms, Some(123456));
    assert_eq!(
        track
            .album
            .as_ref()
            .unwrap()
            .resource_ref
            .as_ref()
            .unwrap()
            .id(),
        "70"
    );
    assert_eq!(track.artists[0].name, "Singer A / Singer B");
    assert!(track.artists[0].resource_ref.is_none());
    assert!(track.playable.is_none());
    assert!(track.available_qualities.is_empty());
    assert_eq!(
        item.extensions["catalogue_hashes"]["hash_flac"],
        "abcdef1234567890abcdef1234567890"
    );
    let bytes = envelope(vec![album()], 1);
    let p = parse(&bytes, "123456789", Kind::Albums, 1).unwrap();
    assert_eq!(p.response_bytes, bytes.len());
    let Item::Album(item) = &p.items[0] else {
        panic!()
    };
    let album = item.album.as_ref().unwrap();
    assert_eq!(item.extensions["buy_total"], 99);
    assert_eq!(item.extensions["is_publish"], 1);
    assert_eq!(item.extensions["mp_count"], 0);
    assert!(item.digital_album.is_none());
    assert_eq!(album.id, "70");
    assert!(album.track_count.is_none());
    assert!(album.published_at.is_none());
}
#[test]
fn purchases_keep_unresolved_records_without_treating_goods_or_audio_ids_as_catalogue_refs() {
    for kind in [Kind::Tracks, Kind::Albums] {
        let mut item = if kind == Kind::Tracks {
            song()
        } else {
            album()
        };
        item.as_object_mut()
            .unwrap()
            .remove(if kind == Kind::Tracks {
                "album_audio_id"
            } else {
                "album_id"
            });
        let p = parse(&envelope(vec![item], 1), "123456789", kind, 1).unwrap();
        assert!(p.items[0].catalogue_id().is_none());
        assert!(!p.items[0].identity().unwrap().is_empty());
        assert_eq!(p.items[0].extensions()["catalogue_resolved"], false);
    }
    let mut item = song();
    item["songname"] = Value::Null;
    let p = parse(&envelope(vec![item], 1), "123456789", Kind::Tracks, 1).unwrap();
    assert!(p.items[0].catalogue_id().is_none());
}
#[test]
fn strict_purchase_shapes_reject_incomplete_pages_deleted_rows_bad_ids_and_foreign_metadata() {
    for kind in [Kind::Tracks, Kind::Albums] {
        for case in [
            "wrong_total",
            "missing_goods",
            "wrong_owner",
            "bad_id",
            "deleted",
            "bad_cover",
            "missing_identity",
            "wrong_type",
        ] {
            let mut row = if kind == Kind::Tracks {
                song()
            } else {
                album()
            };
            match case {
                "bad_id" => row["id"] = json!("010"),
                "deleted" => row["deleted"] = json!(1),
                "bad_cover" => {
                    row[if kind == Kind::Tracks {
                        "album_cover"
                    } else {
                        "cover"
                    }] = json!("https://example.invalid/cover.jpg")
                }
                "missing_identity" => {
                    row.as_object_mut().unwrap().remove("id");
                    row.as_object_mut().unwrap().remove("album_audio_id");
                    row.as_object_mut().unwrap().remove("good_scid");
                    row.as_object_mut().unwrap().remove("album_id");
                }
                "wrong_type" => {
                    row[if kind == Kind::Tracks {
                        "songname"
                    } else {
                        "album_name"
                    }] = json!([])
                }
                _ => {}
            }
            let mut value = json!({"status":1,"error_code":0,"data":{"goods":[row],"total":1}});
            if case == "wrong_total" {
                value["data"]["total"] = json!(2);
            }
            if case == "missing_goods" {
                value["data"].as_object_mut().unwrap().remove("goods");
            }
            if case == "wrong_owner" {
                value["data"]["userid"] = json!(999);
            }
            assert!(
                parse(&serde_json::to_vec(&value).unwrap(), "123456789", kind, 1).is_err(),
                "{kind:?}:{case}"
            );
        }
        assert!(
            parse(&envelope(vec![], 0), "123456789", kind, 1)
                .unwrap()
                .items
                .is_empty()
        );
    }
    let mut item = song();
    item["audio_info"]["album_id"] = json!(999);
    assert_eq!(
        parse(&envelope(vec![item], 1), "123456789", Kind::Tracks, 1)
            .err()
            .unwrap()
            .code,
        ErrorCode::Conflict
    );
    assert!(parse(&envelope(vec![], 6401), "123456789", Kind::Tracks, 1).is_err());
    for field in ["buy_total", "is_publish", "mp_count"] {
        let mut row = album();
        row[field] = json!("not-a-number");
        assert!(parse(&envelope(vec![row], 1), "123456789", Kind::Albums, 1).is_err());
    }
}
#[tokio::test]
async fn native_purchase_requests_use_each_client_signature_and_exact_endpoint_parameters() {
    for client_kind in [KugouLoginClient::Standard, KugouLoginClient::Concept] {
        for kind in [Kind::Tracks, Kind::Albums] {
            let (client,requests)=server(vec![ok(json!({"userid":123456789,"goods":[],"total":0,"page":2,"pagesize":kind.page_size()}))]).await;
            client
                .native_purchases_page(&session(client_kind), kind, 2)
                .await
                .unwrap();
            let all = requests.await.unwrap();
            assert_eq!(all.len(), 1);
            let (query, body) = request(&all[0], kind.endpoint(), client_kind);
            assert_eq!(body["appid"], client_kind.appid());
            assert_eq!(body["clientver"], client_kind.clientver().to_string());
            assert_eq!(body["userid"], 123456789);
            assert_eq!(body["page"], 2);
            assert_eq!(body["pagesize"], kind.page_size());
            assert_eq!(body["deleted"], 0);
            if kind == Kind::Albums && client_kind == KugouLoginClient::Standard {
                assert_eq!(body["use_custom_sort"], 1);
            } else {
                assert!(body.get("use_custom_sort").is_none());
            }
            if kind == Kind::Tracks {
                assert_eq!(body["need_audio_info"], 1);
                assert_eq!(body["area_code"], "1");
            } else {
                assert!(body.get("need_audio_info").is_none());
                assert!(body.get("area_code").is_none());
            }
            assert!(!query.contains_key("plat"));
            assert!(!query.contains_key("p"));
            assert!(!query.contains_key("last_time"));
        }
    }
}
#[tokio::test]
async fn purchase_transport_rejects_authentication_mime_size_and_redirect_failures_without_retry() {
    for (response, code) in [
        (
            frame(
                200,
                "Content-Type: application/json\r\n",
                br#"{"status":0,"error_code":20017}"#.to_vec(),
            ),
            ErrorCode::AuthenticationRequired,
        ),
        (
            frame(200, "Content-Type: text/html\r\n", envelope(vec![], 0)),
            ErrorCode::UpstreamError,
        ),
        (
            frame(
                200,
                "Content-Type: application/json\r\n",
                vec![b' '; 1_048_577],
            ),
            ErrorCode::UpstreamError,
        ),
        (
            frame(
                302,
                "Location: https://example.invalid/redirect\r\n",
                vec![],
            ),
            ErrorCode::UpstreamError,
        ),
    ] {
        let (client, requests) = server(vec![response]).await;
        assert_eq!(
            client
                .native_purchases_page(&session(KugouLoginClient::Standard), Kind::Tracks, 1)
                .await
                .err()
                .unwrap()
                .code,
            code
        );
        assert_eq!(requests.await.unwrap().len(), 1);
    }
}
