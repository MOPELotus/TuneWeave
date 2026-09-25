use super::*;

fn asset(hash: char, height: Option<u32>, source: &str) -> Asset {
    Asset {
        source_key: source.into(),
        hash: hash.to_string().repeat(32),
        bitrate: Some(800),
        size: Some(1000),
        width: None,
        height,
    }
}
fn privilege(hash: char) -> Value {
    json!({"hash":hash.to_string().repeat(32),"id":"17","status":1,"privilege":0,"pay_type":0,"fail_process":0,"info":{"filesize":"1000","bitrate":800}})
}
fn privileges(items: Vec<Value>) -> Vec<u8> {
    serde_json::to_vec(&json!({"status":1,"error_code":0,"data":items})).unwrap()
}
fn tracker() -> Value {
    json!({"status":1,"privileges":{"a".repeat(32):0},"data":{"a".repeat(32):{
        "filesize":"1000","downurl":"https://mvwebfs.tx.kugou.com/a?auth=test%2Bvalue",
        "backupdownurl":["https://mvwebfs.tx.kugou.com/a?auth=test%2Bvalue","https://mvwebfs.ali.kugou.com/a?auth=backup"]}}})
}
fn decode(v: Value) -> Result<(i64, Vec<String>)> {
    parse_tracker(&serde_json::to_vec(&v).unwrap(), &"A".repeat(32), 1000)
}

#[test]
fn selection_uses_real_heights_and_keeps_unknown_dimensions_unknown() {
    let mut assets = [
        asset('A', Some(432), "mkv_sd"),
        asset('B', Some(1080), "fhd"),
        asset('C', Some(432), "sd"),
        asset('D', None, "fhd_265"),
        asset('E', Some(720), "hd"),
    ];
    assets.sort_by_key(|a| selection_key(a, 480));
    assert_eq!(
        assets
            .iter()
            .map(|a| a.source_key.as_str())
            .collect::<Vec<_>>(),
        ["sd", "mkv_sd", "hd", "fhd", "fhd_265"]
    );
    assets.sort_by_key(|a| selection_key(a, 2160));
    assert_eq!(assets[0].height, Some(1080));
    assets.sort_by_key(|a| selection_key(a, 1));
    assert_eq!(assets[0].height, Some(432));
}

#[test]
fn privileges_bind_actual_identity_size_and_bitrate_and_restore_hash_order() {
    let a = [asset('A', Some(432), "sd"), asset('B', Some(720), "hd")];
    let p = parse_privileges(&privileges(vec![privilege('b'), privilege('a')]), "17", &a).unwrap();
    assert_eq!(p[0].hash, "a".repeat(32));
    for (key, value) in [
        ("id", json!(0)),
        ("id", json!(18)),
        ("hash", json!("C".repeat(32))),
        ("status", json!(2)),
        ("info", json!({"filesize":999,"bitrate":800})),
        ("info", json!({"filesize":1000,"bitrate":801})),
        ("info", Value::Null),
    ] {
        let mut p = privilege('A');
        p[key] = value;
        assert!(
            parse_privileges(&privileges(vec![p]), "17", &a[..1]).is_err(),
            "{key}"
        );
    }
    for items in [
        vec![],
        vec![privilege('A'), privilege('a')],
        vec![privilege('A')],
    ] {
        assert!(parse_privileges(&privileges(items), "17", &a).is_err());
    }
}

#[test]
fn denied_privileges_and_unknown_hash_success_cannot_grant_media() {
    let a = [asset('A', Some(432), "sd")];
    let mut p = privilege('A');
    p["status"] = json!(0);
    p["privilege"] = json!(10);
    p["info"] = Value::Null;
    let parsed = parse_privileges(&privileges(vec![p]), "17", &a).unwrap();
    assert_eq!(parsed[0].status.0, 0);
    let p = json!({"hash":"A".repeat(32),"id":0,"status":1,"privilege":0,"pay_type":0,"fail_process":0,"info":{"filesize":0,"bitrate":0},"_errno":6});
    assert!(parse_privileges(&privileges(vec![p]), "17", &a).is_err());
}

#[test]
fn tracker_keeps_exact_https_urls_and_deduplicates_backup_order() {
    let (code, urls) = decode(tracker()).unwrap();
    assert_eq!(code, 0);
    assert_eq!(
        urls,
        [
            "https://mvwebfs.tx.kugou.com/a?auth=test%2Bvalue",
            "https://mvwebfs.ali.kugou.com/a?auth=backup"
        ]
    );
    for bad in [
        "http://fsmvpc.kugou.com/a",
        "https://evil.invalid/a",
        "https://mvwebfs.kugou.com.evil.invalid/a",
        "https://127.0.0.1/a",
        "https://user@mvwebfs.tx.kugou.com/a",
        "https://mvwebfs.tx.kugou.com:444/a",
        "https://mvwebfs.tx.kugou.com/a#fragment",
        " https://mvwebfs.tx.kugou.com/a",
        "https://mvwebfs.tx.kugou.com/a\n",
        "https://mvwebfs.tx.kugou.com\\evil",
    ] {
        let mut t = tracker();
        t["data"]["a".repeat(32)]["backupdownurl"] = json!([bad]);
        assert!(decode(t).is_err(), "{bad}");
    }
}

#[test]
fn tracker_denial_and_empty_urls_are_unavailable_while_missing_resource_is_an_error() {
    let mut t = tracker();
    t["privileges"]["a".repeat(32)] = json!(10);
    assert_eq!(decode(t).unwrap(), (10, vec![]));
    let mut t = tracker();
    t["data"]["a".repeat(32)] = json!({"filesize":1000,"downurl":"","backupdownurl":[]});
    assert!(decode(t).unwrap().1.is_empty());
    assert_eq!(
        decode(json!({"status":0,"errcode":40002,"privileges":{"a".repeat(32):0}}))
            .unwrap_err()
            .code,
        ErrorCode::ResourceNotFound
    );
    assert!(
        decode(json!({"status":0,"privileges":{"a".repeat(32):0}}))
            .unwrap()
            .1
            .is_empty()
    );
}

#[test]
fn tracker_accepts_the_verified_uploaded_video_host_without_trusting_its_siblings() {
    let mut t = tracker();
    t["data"]["a".repeat(32)]["downurl"] =
        json!("https://kgv.stream.tencentmusic.com/ugc?auth=test");
    assert_eq!(
        decode(t).unwrap().1[0],
        "https://kgv.stream.tencentmusic.com/ugc?auth=test"
    );
    for host in [
        "other.stream.tencentmusic.com",
        "stream.tencentmusic.com",
        "kgv.stream.tencentmusic.com.evil.invalid",
    ] {
        let mut t = tracker();
        t["data"]["a".repeat(32)]["downurl"] = json!(format!("https://{host}/ugc"));
        assert!(decode(t).is_err());
    }
}

#[test]
fn tracker_rejects_ambiguous_hashes_wrong_sizes_and_malformed_success() {
    let a = "a".repeat(32);
    for (key, value) in [
        ("status", json!(2)),
        ("error_code", json!(9)),
        ("data", json!({})),
        ("privileges", json!({"b".repeat(32):0})),
        ("privileges", Value::Null),
    ] {
        let mut t = tracker();
        t[key] = value;
        assert!(decode(t).is_err(), "{key}");
    }
    for size in [Value::Null, json!(0), json!(999), json!("01000"), json!(-1)] {
        let mut t = tracker();
        t["data"][&a]["filesize"] = size;
        assert!(decode(t).is_err());
    }
    let original = tracker().to_string();
    for replacement in [
        format!("\"{a}\":0,\"{a}\":0"),
        format!("\"{a}\":0,\"{}\":0", a.to_uppercase()),
    ] {
        let raw = original.replace(&format!("\"{a}\":0"), &replacement);
        assert!(parse_tracker(raw.as_bytes(), &a.to_uppercase(), 1000).is_err());
    }
    let mut t = tracker();
    t["data"]["b".repeat(32)] = t["data"][&a].clone();
    assert!(decode(t).is_err());
}
