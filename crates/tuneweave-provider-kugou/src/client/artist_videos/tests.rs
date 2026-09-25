use super::*;

fn entry(id: u64) -> Value {
    json!({"video_id":id,"video_name":"Video","timelength":140783,
        "album_audio_id":82,"audio_timelength":"269000","user_id":"99",
        "author_name":"Uploader","h264":{"sd_hash":"A".repeat(32)}})
}
fn body(total: u64, page: u32) -> Value {
    let start = u64::from(page - 1) * 30;
    json!({"status":1,"error_code":0,"errcode":0,"total":total,"extra":{"page_total":total},
        "data":(start..total.min(start+30)).map(|n| entry(n+1)).collect::<Vec<_>>()})
}
fn decode(v: Value, page: u32) -> Result<VideoPage> {
    parse(v.to_string().as_bytes(), page)
}

#[test]
fn artist_video_page_preserves_video_duration_and_associated_identities_without_inventing_creators()
{
    let p = decode(body(1, 1), 1).unwrap();
    assert_eq!(p.total, 1);
    assert_eq!(p.items[0].id, "1");
    assert_eq!(p.items[0].duration_ms, Some(140783));
    assert_eq!(p.items[0].audio_id, None);
    assert_eq!(p.items[0].album_audio_id.as_deref(), Some("82"));
    assert_eq!(p.items[0].uploader_id.as_deref(), Some("99"));
    let mut v = body(1, 1);
    v["data"][0] =
        json!({"video_id":"1","video_name":"Video","user_id":0,"album_audio_id":0,"timelength":0});
    let p = decode(v, 1).unwrap();
    assert_eq!(p.items[0].duration_ms, None);
    assert_eq!(p.items[0].album_audio_id, None);
    assert_eq!(p.items[0].uploader_id, None);
}

#[test]
fn artist_video_pages_require_exact_physical_counts_including_tail_and_out_of_range() {
    for (total, page, count) in [(35, 1, 30), (35, 2, 5), (35, 3, 0), (0, 1, 0)] {
        assert_eq!(decode(body(total, page), page).unwrap().items.len(), count);
    }
    for (key, value) in [
        ("total", json!(29)),
        ("data", json!([])),
        ("extra", json!({"page_total":31})),
    ] {
        let mut v = body(30, 1);
        v[key] = value;
        assert!(decode(v, 1).is_err());
    }
    let mut v = body(30, 1);
    v["data"].as_array_mut().unwrap().pop();
    assert!(decode(v, 1).is_err());
    assert!(decode(body(30, 1), 0).is_err());
    let mut v = body(1, 1);
    v.as_object_mut().unwrap().remove("total");
    assert!(decode(v, 1).is_err());
}

#[test]
fn artist_video_pages_reject_duplicate_or_malformed_identities_and_duplicate_typed_fields() {
    let mut v = body(2, 1);
    v["data"][1]["video_id"] = json!(1);
    assert!(decode(v, 1).is_err());
    for value in [
        json!(0),
        json!(-1),
        json!("01"),
        json!("+1"),
        json!(true),
        json!(1.5),
        json!("18446744073709551616"),
    ] {
        let mut v = body(1, 1);
        v["data"][0]["video_id"] = value;
        assert!(decode(v, 1).is_err());
    }
    let v = body(1, 1).to_string();
    for (old, new) in [
        ("\"video_id\":1", "\"video_id\":1,\"video_id\":2"),
        ("\"total\":1", "\"total\":1,\"total\":1"),
        (
            "\"timelength\":140783",
            "\"timelength\":140783,\"timelength\":269000",
        ),
    ] {
        assert!(parse(v.replace(old, new).as_bytes(), 1).is_err());
    }
}

#[test]
fn artist_video_errors_do_not_become_empty_success_or_expose_upstream_messages() {
    for data in [json!([]), json!(null), json!("private")] {
        let e = decode(
            json!({"status":0,"error_code":51,"errmsg":"private","total":0,"data":data}),
            1,
        )
        .err()
        .unwrap();
        assert_eq!(e.code, ErrorCode::UpstreamError);
        assert_eq!(e.details["platform_code"], 51);
        assert!(!format!("{e:?}").contains("private"));
    }
    let mut v = body(1, 1);
    v["errcode"] = json!(12);
    assert_eq!(decode(v, 1).err().unwrap().details["platform_code"], 12);
    let mut v = body(1, 1);
    v["data"][0]["video_name"] = json!("");
    assert!(decode(v, 1).is_err());
}
