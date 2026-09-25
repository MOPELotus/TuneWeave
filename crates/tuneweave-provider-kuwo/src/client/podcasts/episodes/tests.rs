use super::*;
use crate::client::catalog::tests::{requests, response, setup};
use std::io::Write;
use tuneweave_core::MusicProvider;

fn request(offset: u32) -> PodcastEpisodeListRequest {
    PodcastEpisodeListRequest {
        limit: 3,
        offset,
        ascending: true,
        account: None,
    }
}
fn body(start: u32, serials: &[u64]) -> serde_json::Value {
    json!({"albumid":"9240675","count":serials.len(),"start":start,"total":"4","order":"2","type":"music",
        "musiclist":serials.iter().map(|serial| json!({"musicrid":(100+serial).to_string(),"albumid":"9240675",
        "isstar":"1","content_type":"0","name":format!("节目 {serial}"),"duration":"60","track":serial,
        "artist":"主播","artistid":"3194753","releasedate":"2022-02-27",
        "img":"http://img1.kuwo.cn/star/albumcover/300/x.jpg","payInfo":"unknown","sid":"do-not-export"})).collect::<Vec<_>>()})
}
fn frame(data: &[u8]) -> Vec<u8> {
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(data).unwrap();
    let compressed = encoder.finish().unwrap();
    let mut result = b"sig=\r\n".to_vec();
    result.extend_from_slice(&(compressed.len() as u32).to_le_bytes());
    result.extend_from_slice(&(data.len() as u32).to_le_bytes());
    result.extend(compressed);
    result.extend_from_slice(b"unrelated-server-buffer-data-must-not-be-exported");
    result
}
fn reply(data: &serde_json::Value) -> Vec<u8> {
    response(
        200,
        "application/octet-stream",
        "",
        &frame(&serde_json::to_vec(data).unwrap()),
    )
}

#[tokio::test]
async fn anchor_episodes_page_preserves_identity_order_and_anonymous_wire() {
    let mut f = setup(vec![
        reply(&body(0, &[1, 2, 3])),
        reply(&body(3, &[4])),
        reply(&body(4, &[4])),
    ])
    .await;
    let page = f
        .provider
        .podcast_episodes("anchor:9240675", &request(0))
        .await
        .unwrap();
    assert_eq!(page.pagination.total, Some(4));
    assert_eq!(page.pagination.next_offset, Some(3));
    assert_eq!(page.items[0].resource_ref.to_string(), "kuwo:episode:101");
    assert_eq!(
        page.items[0].podcast_ref.as_ref().unwrap().to_string(),
        "kuwo:anchor:9240675"
    );
    assert_eq!(page.items[0].duration_ms, Some(60_000));
    assert_eq!(page.items[0].serial_number, Some(1));
    assert!(
        page.items
            .iter()
            .all(|item| item.audio.is_none() && item.purchased.is_none() && item.paid.is_none())
    );
    let serialized = serde_json::to_string(&page).unwrap();
    assert!(!serialized.contains("do-not-export"));
    assert!(!serialized.contains("server-buffer"));
    let last = f
        .provider
        .podcast_episodes("anchor:9240675", &request(3))
        .await
        .unwrap();
    assert_eq!(last.items.len(), 1);
    assert!(!last.pagination.has_more);
    let exhausted = f
        .provider
        .podcast_episodes("anchor:9240675", &request(4))
        .await
        .unwrap();
    assert!(exhausted.items.is_empty());
    assert!(!exhausted.pagination.has_more);
    let wires = requests(&mut f, 3).await;
    // Independent pinned-DEX bit-permutation reference with the catalogue key.
    let encoded = "+abMGb7W2zZYBPNfoixEWfMZCNG822KsWyAZeyth6WhET83I4Ft+oc+aWjiLkUTTk3FtN+HbEGzrQLiaM1iNEgBWVhGUryEcR2EB3Xe1ZK2Lox3zy3nkO8vK4xskefrJRsvrCpt1szrltBVa8HMo0kW2fGOlvRVu13T/Ey6CrymKV97rgxWR6w==";
    assert!(wires[0].starts_with(&format!("GET /r.s?f=kuwo&q={encoded} ")));
    for wire in wires {
        for forbidden in ["cookie:", "secret:", "authorization:"] {
            assert!(!wire.to_lowercase().contains(forbidden));
        }
    }
}

#[test]
fn anchor_episodes_frame_respects_declared_lengths_checksum_and_expansion_limit() {
    let data = br#"{"data":"only declared bytes"}"#;
    let valid = frame(data);
    assert_eq!(decode_frame(&valid).unwrap(), data);
    for end in [0, 5, 13, 15] {
        assert!(decode_frame(&valid[..end]).is_err());
    }
    let mut bad = valid.clone();
    bad[10..14].copy_from_slice(&((LIMIT + 1) as u32).to_le_bytes());
    assert!(decode_frame(&bad).is_err());
    let mut bad = valid.clone();
    bad[10..14].copy_from_slice(&1_u32.to_le_bytes());
    assert!(decode_frame(&bad).is_err());
    let mut bad = valid.clone();
    bad[6..10].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(decode_frame(&bad).is_err());
    let mut bad = valid.clone();
    bad[16] ^= 127;
    assert!(decode_frame(&bad).is_err());
    let mut bad = valid;
    let n = u32::from_le_bytes(bad[6..10].try_into().unwrap());
    bad[6..10].copy_from_slice(&(n + 1).to_le_bytes());
    assert!(decode_frame(&bad).is_err());
}

#[test]
fn anchor_episodes_rejects_foreign_rows_inconsistent_pages_and_wrong_order() {
    let base = body(0, &[1, 2, 3]);
    for (key, value) in [
        ("albumid", json!("99")),
        ("start", json!(1)),
        ("order", json!(1)),
        ("count", json!(2)),
        ("type", json!("album")),
        ("total", json!(2)),
    ] {
        let mut data = base.clone();
        data[key] = value;
        assert!(
            parse_page(&serde_json::to_vec(&data).unwrap(), "9240675", &request(0)).is_err(),
            "{key}"
        );
    }
    for (key, value) in [
        ("albumid", json!("99")),
        ("isstar", json!(0)),
        ("content_type", json!(1)),
        ("musicrid", json!("101")),
        ("track", json!(1)),
        ("duration", json!(u64::MAX)),
    ] {
        let mut data = base.clone();
        data["musiclist"][1][key] = value;
        assert!(
            parse_page(&serde_json::to_vec(&data).unwrap(), "9240675", &request(0)).is_err(),
            "{key}"
        );
    }
    let mut descending = body(0, &[4, 3, 2]);
    descending["order"] = json!(1);
    let mut req = request(0);
    req.ascending = false;
    let page = parse_page(&serde_json::to_vec(&descending).unwrap(), "9240675", &req).unwrap();
    assert_eq!(page.items[0].serial_number, Some(4));
    assert!(
        parse_page(
            &serde_json::to_vec(&body(0, &[])).unwrap(),
            "9240675",
            &request(0)
        )
        .is_err()
    );
}

#[tokio::test]
async fn anchor_episodes_account_and_page_errors_are_rejected_before_io() {
    let mut f = setup(vec![]).await;
    for (limit, offset, account) in [
        (0, 0, None),
        (101, 0, None),
        (3, u32::MAX, None),
        (3, 0, Some("default".to_owned())),
    ] {
        let mut req = request(offset);
        req.limit = limit;
        req.account = account;
        assert_eq!(
            f.provider
                .podcast_episodes("anchor:9240675", &req)
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
    }
    let credential =
        crate::client::native::tests::credential_fixture("42", "private-anchor-session")
            .caller()
            .unwrap();
    let caller = f.provider.with_caller_credential(&credential).unwrap();
    assert_eq!(
        caller
            .podcast_episodes("anchor:9240675", &request(0))
            .await
            .unwrap_err()
            .code,
        ErrorCode::CapabilityNotSupported
    );
    requests(&mut f, 0).await;
}

#[tokio::test]
#[ignore = "real anonymous official programme metadata only; no account or media"]
async fn live_anchor_episodes_pagination_preserves_album_order_and_end() {
    let client = KuwoClient::new(&KuwoConfig::default()).unwrap();
    let first = client
        .podcast_episodes("anchor:9240675", &request(0))
        .await
        .unwrap();
    assert_eq!(first.items[0].serial_number, Some(1));
    let second = client
        .podcast_episodes("anchor:9240675", &request(3))
        .await
        .unwrap();
    assert!(
        second
            .items
            .iter()
            .all(|x| !first.items.iter().any(|a| a.resource_ref == x.resource_ref))
    );
    let total = u32::try_from(second.pagination.total.unwrap()).unwrap();
    let end = client
        .podcast_episodes("anchor:9240675", &request(total))
        .await
        .unwrap();
    assert!(end.items.is_empty());
    assert!(!end.pagination.has_more);
}
