use super::*;
use crate::account::cloud::tests::{Frame, binary, plaintext, server};
use crate::account::tests::session;

pub(crate) fn singer(id: u64) -> Value {
    json!({"id":id,"name":format!("Singer {id}"),"img":"https://singerimg.kugou.com/uploadpic/{size}/artist.jpg",
        "fansnum":123,"followtime":1700000000,"userid":999999,"identity":3})
}
pub(crate) fn full(rows: Vec<Value>) -> Value {
    json!({"status":1,"error_code":0,"need_update":1,"version":37,"data":{"singerlist":rows}})
}
pub(crate) fn encrypted(value: Value) -> Frame {
    let cipher = Cipher::random().unwrap();
    binary(
        200,
        "application/octet-stream",
        cipher.encode(&serde_json::to_vec(&value).unwrap()).unwrap(),
    )
}

#[test]
fn followed_artists_parse_preserves_order_and_separates_singer_linked_user_and_account() {
    let mut first = singer(42);
    first["unknown"] = json!({"token":"secret-marker"});
    let page = parse(&full(vec![first, singer(7)]).to_string().into_bytes()).unwrap();
    assert_eq!(page.version, 37);
    assert_eq!(
        page.items.iter().map(|a| a.id.as_str()).collect::<Vec<_>>(),
        ["42", "7"]
    );
    let artist = &page.items[0];
    assert_eq!(artist.resource_ref.id(), "42");
    assert_eq!(artist.extensions["linked_user_id"], "999999");
    assert_eq!(artist.extensions["fans_count"], 123);
    assert_eq!(artist.extensions["identity_code"], 3);
    assert_eq!(artist.extensions["followed"], true);
    assert!(artist.identities.is_empty());
    assert!(artist.album_count.is_none());
    assert!(
        !serde_json::to_string(&page.items)
            .unwrap()
            .contains("secret-marker")
    );
    assert!(
        parse(full(vec![]).to_string().as_bytes())
            .unwrap()
            .items
            .is_empty()
    );
}

#[test]
fn followed_artists_require_full_snapshot_and_do_not_invent_missing_or_duplicate_identity() {
    for invalid in [
        json!({"status":1,"need_update":0,"version":0}),
        json!({"status":1,"need_update":1,"version":1,"data":{}}),
        json!({"status":1,"need_update":1,"data":{"singerlist":[]}}),
        full(vec![singer(7), singer(7)]),
        full(vec![singer(0)]),
    ] {
        assert!(parse(invalid.to_string().as_bytes()).is_err());
    }
    for (key, value) in [
        ("name", json!("")),
        ("id", json!("01")),
        ("fansnum", json!(-1)),
        ("img", json!("https://example.invalid/avatar")),
    ] {
        let mut row = singer(7);
        row[key] = value;
        assert!(
            parse(full(vec![row]).to_string().as_bytes()).is_err(),
            "{key}"
        );
    }
    let mut row = json!({"id":7,"name":"Minimal"});
    let page = parse(full(vec![row.clone()]).to_string().as_bytes()).unwrap();
    assert!(page.items[0].avatar_url.is_none());
    assert!(!page.items[0].extensions.contains_key("fans_count"));
    row["userid"] = json!(0);
    assert!(
        !parse(full(vec![row]).to_string().as_bytes()).unwrap().items[0]
            .extensions
            .contains_key("linked_user_id")
    );
    let good = full(vec![singer(7)]).to_string();
    for bad in [
        good.replace("\"need_update\":1", "\"need_update\":1,\"need_update\":1"),
        good.replace("\"id\":7", "\"id\":7,\"id\":8"),
    ] {
        assert!(parse(bad.as_bytes()).is_err());
    }
}

#[test]
fn followed_artists_reject_over_budget_and_preserve_business_error_without_raw_payload() {
    let rows = (1..=MAX_ARTISTS + 1)
        .map(|id| json!({"id":id,"name":"Singer"}))
        .collect();
    assert!(parse(full(rows).to_string().as_bytes()).is_err());
    for code in [20017, 20010] {
        let e = parse(
            json!({"status":0,"error_code":code,"data":"secret-marker"})
                .to_string()
                .as_bytes(),
        )
        .err()
        .unwrap();
        assert_eq!(
            e.code,
            if code == 20017 {
                ErrorCode::AuthenticationRequired
            } else {
                ErrorCode::UpstreamError
            }
        );
        assert!(!format!("{e:?}").contains("secret-marker"));
    }
}

#[tokio::test]
async fn followed_artists_wire_uses_binary_version_zero_and_rsa_identity_without_plain_credentials()
{
    let f = server(vec![encrypted(full(vec![singer(7)]))]).await;
    let source = session(KugouLoginClient::Standard);
    let snapshot = f.client.native_followed_artists(&source).await.unwrap();
    assert_eq!(snapshot.items[0].id, "7");
    let requests = f.requests.await.unwrap();
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    let target = request
        .head
        .lines()
        .next()
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap();
    let url = url::Url::parse(&format!("http://localhost{target}")).unwrap();
    assert_eq!(url.path(), FOLLOW_PATH);
    let query = url.query_pairs().collect::<BTreeMap<_, _>>();
    assert_eq!(
        query.keys().map(|k| k.as_ref()).collect::<Vec<_>>(),
        [
            "appid",
            "clienttime",
            "clientver",
            "dfid",
            "key",
            "mid",
            "p"
        ]
    );
    assert_eq!(query["appid"], source.client.appid().to_string());
    assert_eq!(query["clientver"], source.client.clientver().to_string());
    assert_eq!(
        query["key"],
        format!(
            "{:x}",
            Md5::digest(format!(
                "{}{}{}{}",
                source.client.appid(),
                ANDROID_SALT,
                source.client.clientver(),
                query["clienttime"]
            ))
        )
    );
    assert_eq!(query["p"].len(), 256);
    assert_eq!(query["p"], query["p"].to_ascii_uppercase());
    assert!(!request.head.contains(&source.token));
    for omitted in ["cookie:", "authorization:", "x-router:"] {
        assert!(!request.head.to_ascii_lowercase().contains(omitted));
    }
    assert_eq!(plaintext(request), json!({"version":0}));
    // Independent OpenSSL AES-128-CBC vector for synthetic seed a1B2c3.
    assert_eq!(
        hex::encode(&request.body),
        "636d1bac9bf60078ed081608bac036b3"
    );
}

#[tokio::test]
async fn followed_artists_transport_rejects_plain_success_truncation_mime_redirect_and_size() {
    for case in ["plain", "truncated", "mime", "redirect", "large", "auth"] {
        let frame=match case {
            "plain"=>binary(200,"application/json",full(vec![]).to_string().into_bytes()),
            "truncated"=>binary(200,"application/octet-stream",vec![1;15]),
            "mime"=>binary(200,"text/html",vec![]),
            "redirect"=>Frame::from("HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:1/secret\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_owned()),
            "auth"=>binary(200,"application/json",br#"{"status":0,"error_code":20017}"#.to_vec()),
            _=>binary(200,"application/octet-stream",vec![0;MAX_BYTES+1]),
        };
        let f = server(vec![frame]).await;
        let e = f
            .client
            .native_followed_artists(&session(KugouLoginClient::Standard))
            .await
            .err()
            .unwrap();
        assert_eq!(
            e.code,
            if case == "auth" {
                ErrorCode::AuthenticationRequired
            } else {
                ErrorCode::UpstreamError
            },
            "{case}"
        );
        assert_eq!(f.requests.await.unwrap().len(), 1);
    }
    let f = server(vec![]).await;
    assert_eq!(
        f.client
            .native_followed_artists(&session(KugouLoginClient::Concept))
            .await
            .err()
            .unwrap()
            .code,
        ErrorCode::CapabilityNotSupported
    );
    assert!(f.requests.await.unwrap().is_empty());
}
