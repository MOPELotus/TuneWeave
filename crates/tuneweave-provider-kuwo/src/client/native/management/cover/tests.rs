use super::*;
use crate::client::{
    catalog::tests::{json_response, response},
    native::{
        management::items::tests::snapshot,
        playlist::tests::{detail, directory},
        tests as fixture,
    },
};
use base64::engine::general_purpose::STANDARD;
use serde_json::Value;
use std::io::Cursor;

const KEY: [u8; 8] = *b"17894932";
pub(crate) const URL: &str = "https://img4.kuwo.cn/star/usercover/uploaded.jpg";
const THUMB: &str = "https://img4.kuwo.cn/star/usercover/thumb.jpg";
pub(crate) fn request(account: Option<&str>) -> ImageUploadRequest {
    let image = ::image::RgbaImage::from_fn(6, 4, |x, _| {
        if x < 3 {
            ::image::Rgba([240, 20, 10, 255])
        } else {
            ::image::Rgba([10, 20, 240, 255])
        }
    });
    let mut bytes = Cursor::new(Vec::new());
    ::image::DynamicImage::ImageRgba8(image)
        .write_to(&mut bytes, ::image::ImageFormat::Png)
        .unwrap();
    ImageUploadRequest {
        filename: "local-name.png".into(),
        content_type: "image/png".into(),
        data: bytes.into_inner(),
        image_size: None,
        crop_x: None,
        crop_y: None,
        account: account.map(str::to_owned),
    }
}
fn uploaded(plain: &[u8]) -> Vec<u8> {
    json_response(
        &json!({"status":200,"data":String::from_utf8(codec::fixture_response(plain,&KEY)).unwrap()}),
    )
}
pub(crate) fn flow(uid: &str) -> Vec<Vec<u8>> {
    let mut b = vec![json_response(&json!({"result":"ok"}))];
    b.extend(snapshot(uid, &[11, 22, 22, 33]));
    b.push(uploaded(
        &serde_json::to_vec(&json!({"picUrl":URL,"picThumbUrl":THUMB,"token":"never-export"}))
            .unwrap(),
    ));
    b.extend([
        json_response(&detail(uid, 4)),
        json_response(&directory(Some(4))),
        json_response(&json!({"errcode":0,"pid":101})),
    ]);
    let mut after = snapshot(uid, &[11, 22, 22, 33]);
    for (i, b) in after.iter_mut().enumerate() {
        if [0, 1, 4, 5].contains(&i) {
            let mut v = body(b);
            if let Some(rows) = v["plist"].as_array_mut() {
                rows[0]["pic"] = json!(URL);
            } else {
                v["sl_data"]["pic"] = json!(THUMB);
                v["sl_data"]["big_pic"] = json!(URL);
            }
            *b = json_response(&v);
        }
    }
    b.extend(after);
    b
}
fn body(b: &[u8]) -> Value {
    let start = b.windows(4).position(|v| v == b"\r\n\r\n").unwrap() + 4;
    serde_json::from_slice(&b[start..]).unwrap()
}
fn credential() -> ProviderCredential {
    fixture::credential_fixture("42", "selected-session")
        .caller()
        .unwrap()
}

#[test]
fn native_cover_independent_upload_key_vectors_leave_login_key_unchanged() {
    let vectors: Vec<Value> = serde_json::from_str(include_str!("test_vectors.json")).unwrap();
    for v in vectors {
        let plain = v["plain"].as_str().unwrap().as_bytes();
        assert_eq!(codec::seal_image_query(plain).unwrap(), v["encrypted"]);
        assert_ne!(
            codec::seal_image_query(plain).unwrap(),
            codec::seal_query(plain).unwrap()
        );
    }
}

#[tokio::test]
async fn native_cover_wire_uses_selected_account_jpeg_and_two_confirmed_writes() {
    let mut f = setup(flow("42")).await;
    let result = f
        .client
        .native_update_playlist_cover(&credential(), "101", &request(None))
        .await
        .unwrap();
    assert_eq!(result.image.url.as_deref(), Some(URL));
    assert_eq!(result.extensions["write_requests_dispatched"], 2);
    assert_eq!(result.extensions["playlist_write_outcome"], "confirmed");
    assert_eq!(
        result.image.extensions["crop"],
        json!({"x":1,"y":0,"size":4})
    );
    let seen = fixture::requests(&mut f, 17).await;
    assert_eq!(seen.iter().filter(|r| r.starts_with("POST ")).count(), 2);
    let target = seen[7].split_whitespace().nth(1).unwrap();
    let u = Url::parse(&format!("https://wapi.kuwo.cn{target}")).unwrap();
    assert_eq!(u.path(), "/openapi/v1/playlist/upload/playlistPic");
    let q = u.query_pairs().collect::<BTreeMap<_, _>>();
    for (k, v) in [
        ("appkey", "q7idgad1fr5t"),
        ("apiVer", "1"),
        ("loginUid", "42"),
        ("loginSid", "selected-session"),
        ("uid", "1234567890"),
        ("platform", "ar"),
    ] {
        assert_eq!(q[k], v);
    }
    assert!(!q.contains_key("android_id") && !q.contains_key("user"));
    let plain = json!({"dev_id":"00112233445546778899aabbccddeeff","dev_name":"TuneWeave","from":"android",
        "params":{"fileExt":"jpg","width":0},"sid":"selected-session","sx":"17894932","uid":"42"});
    assert!(target.ends_with(&format!(
        "&q={}",
        codec::seal_image_query(&serde_json::to_vec(&plain).unwrap()).unwrap()
    )));
    assert!(
        seen[7]
            .to_lowercase()
            .contains("content-type: application/json")
    );
    let b: Value = serde_json::from_str(seen[7].split_once("\r\n\r\n").unwrap().1).unwrap();
    assert_eq!(b.as_object().unwrap().len(), 1);
    let bytes = STANDARD.decode(b["params"].as_str().unwrap()).unwrap();
    assert_eq!(
        ::image::guess_format(&bytes).unwrap(),
        ::image::ImageFormat::Jpeg
    );
    let decoded = ::image::load_from_memory(&bytes).unwrap();
    assert_eq!((decoded.width(), decoded.height()), (700, 700));
    let edit: Value = serde_json::from_str(seen[10].split_once("\r\n\r\n").unwrap().1).unwrap();
    assert_eq!(
        edit,
        json!({"pid":101,"title":"自己的歌单","intro":"原始 + %20","tag":"流行,安静","pic":URL,"ispub":false})
    );
    assert!(
        seen.iter()
            .all(|r| !r.to_lowercase().contains("\r\ncookie:") && !r.contains("local-name"))
    );
    let result = serde_json::to_string(&result).unwrap();
    for secret in ["selected-session", "never-export", "local-name", "params"] {
        assert!(!result.contains(secret));
    }
}

#[tokio::test]
async fn native_cover_image_formats_crop_and_alpha_are_processed_locally() {
    for (format, mime) in [
        (::image::ImageFormat::Jpeg, "image/jpeg"),
        (::image::ImageFormat::Png, "image/png"),
        (::image::ImageFormat::Gif, "image/gif"),
        (::image::ImageFormat::Bmp, "image/bmp"),
    ] {
        let source = ::image::RgbImage::from_pixel(3, 2, ::image::Rgb([40, 80, 120]));
        let mut data = Cursor::new(Vec::new());
        ::image::DynamicImage::ImageRgb8(source)
            .write_to(&mut data, format)
            .unwrap();
        let mut r = request(None);
        r.data = data.into_inner();
        r.content_type = mime.into();
        image::validate("101", &r).unwrap();
        let prepared = image::prepare(&r).await.unwrap();
        assert_eq!(prepared.crop, (0, 0, 2));
        let decoded = ::image::load_from_memory(&prepared.jpeg).unwrap().to_rgb8();
        assert_eq!(decoded.dimensions(), (700, 700));
        assert!(
            decoded
                .get_pixel(350, 350)
                .0
                .iter()
                .zip([40, 80, 120])
                .all(|(a, b)| a.abs_diff(b) < 5)
        );
    }
    let mut r = request(None);
    r.image_size = Some(2);
    r.crop_x = Some(0);
    r.crop_y = Some(1);
    let p = image::prepare(&r).await.unwrap();
    assert_eq!(p.crop, (0, 1, 2));
    assert!(
        ::image::load_from_memory(&p.jpeg)
            .unwrap()
            .to_rgb8()
            .get_pixel(350, 350)[0]
            > 230
    );
    let mut data = Cursor::new(Vec::new());
    ::image::DynamicImage::ImageRgba8(::image::RgbaImage::from_pixel(
        1,
        1,
        ::image::Rgba([0, 0, 0, 0]),
    ))
    .write_to(&mut data, ::image::ImageFormat::Png)
    .unwrap();
    r.data = data.into_inner();
    r.image_size = None;
    r.crop_x = None;
    r.crop_y = None;
    let p = image::prepare(&r).await.unwrap();
    assert_eq!(
        ::image::load_from_memory(&p.jpeg)
            .unwrap()
            .to_rgb8()
            .get_pixel(0, 0)
            .0,
        [255, 255, 255]
    );
}

#[tokio::test]
async fn native_cover_invalid_images_and_crops_never_start_network() {
    for case in 0..11 {
        let f = setup(vec![]).await;
        let mut r = request(None);
        match case {
            0 => r.data.clear(),
            1 => r.content_type = "image/jpeg".into(),
            2 => r.image_size = Some(0),
            3 => r.crop_x = Some(0),
            4 => r.data = b"not an image".to_vec(),
            5 => {
                r.image_size = Some(2);
                r.crop_x = Some(u32::MAX);
            }
            6 => r.image_size = Some(7),
            7 => r.filename = "private\nname".into(),
            8 => r.data.truncate(24),
            9 => r.account = Some("personal".into()),
            _ => r.data = vec![0; image::MAX_INPUT + 1],
        }
        let e = f
            .client
            .native_update_playlist_cover(&credential(), "101", &r)
            .await
            .unwrap_err();
        assert_eq!(e.code, ErrorCode::InvalidRequest, "case {case}");
        assert!(!format!("{e:?}").contains("private\nname"));
        assert!(e.details.get("write_outcome").is_none());
    }
}

#[tokio::test]
async fn native_cover_upload_rejection_or_unsafe_response_never_saves_playlist() {
    for upload in [
        json_response(&json!({"status":202,"data":"selected-session"})),
        json_response(&json!({"data":""})),
        json_response(&json!({"status":200,"data":"invalid-base64"})),
        uploaded(br#"{"picUrl":"https://evil.invalid/a.jpg"}"#),
        uploaded(br#"{"picUrl":"https://img4.kuwo.cn/a.jpg?token=selected-session"}"#),
        uploaded(br#"{"picUrl":""}"#),
        uploaded(
            br#"{"picUrl":"https://img4.kuwo.cn/a.jpg","picUrl":"https://img4.kuwo.cn/b.jpg"}"#,
        ),
    ] {
        let mut bodies = flow("42");
        bodies[7] = upload;
        bodies.truncate(8);
        let mut f = setup(bodies).await;
        let e = f
            .client
            .native_update_playlist_cover(&credential(), "101", &request(None))
            .await
            .unwrap_err();
        assert_eq!(e.details["write_requests_dispatched"], 1);
        assert_eq!(e.details["playlist_write_requests_dispatched"], 0);
        assert_eq!(e.details["upload_outcome"], "unconfirmed");
        assert!(!e.retryable && !format!("{e:?}").contains("selected-session"));
        fixture::requests(&mut f, 8).await;
    }
    let mut b = flow("42");
    b[7] = uploaded(URL.as_bytes());
    // Without a supplied thumbnail, the actual readback must use the uploaded URL.
    for i in [12, 15] {
        let mut v = body(&b[i]);
        v["sl_data"]["pic"] = json!(URL);
        b[i] = json_response(&v);
    }
    let mut f = setup(b).await;
    f.client
        .native_update_playlist_cover(&credential(), "101", &request(None))
        .await
        .unwrap();
    fixture::requests(&mut f, 17).await;
}

#[tokio::test]
async fn native_cover_changed_metadata_after_upload_stops_before_save() {
    for (boundary, field) in [
        (8, "title"),
        (8, "tag"),
        (8, "big_pic"),
        (8, "igsl"),
        (9, "title"),
        (9, "ispub"),
    ] {
        let mut b = flow("42");
        let mut v = body(&b[boundary]);
        if boundary == 8 {
            v["sl_data"][field] = match field {
                "igsl" => json!("1"),
                "big_pic" => json!(URL),
                "tag" => json!("古典,安静"),
                _ => json!("changed"),
            };
        } else {
            v["plist"][0][field] = if field == "ispub" {
                json!(true)
            } else {
                json!("changed")
            };
        }
        b[boundary] = json_response(&v);
        b.truncate(boundary + 1);
        let mut f = setup(b).await;
        let e = f
            .client
            .native_update_playlist_cover(&credential(), "101", &request(None))
            .await
            .unwrap_err();
        assert_eq!(e.details["upload_outcome"], "confirmed");
        assert_eq!(e.details["playlist_write_outcome"], "not_dispatched");
        assert_eq!(e.details["write_requests_dispatched"], 1);
        fixture::requests(&mut f, boundary + 1).await;
    }
}

#[tokio::test]
async fn native_cover_save_failure_and_changed_readback_keep_both_writes_unconfirmed() {
    for boundary in 10..17 {
        let mut b = flow("42");
        b[boundary] = response(500, "application/json", "", b"selected-session");
        b.truncate(boundary + 1);
        let mut f = setup(b).await;
        let e = f
            .client
            .native_update_playlist_cover(&credential(), "101", &request(None))
            .await
            .unwrap_err();
        assert_eq!(e.details["write_requests_dispatched"], 2);
        assert_eq!(e.details["upload_outcome"], "confirmed");
        assert_eq!(e.details["playlist_write_outcome"], "unconfirmed");
        assert!(!e.retryable);
        fixture::requests(&mut f, boundary + 1).await;
    }
    for variant in ["cover", "tracks", "tags"] {
        let mut b = flow("42");
        for bytes in b.iter_mut().skip(11) {
            let mut v = body(bytes);
            if variant == "cover" {
                if v["sl_data"].is_object() {
                    v["sl_data"]["pic"] = json!("https://img4.kuwo.cn/other.jpg");
                    v["sl_data"]["big_pic"] = v["sl_data"]["pic"].clone();
                }
                if v["plist"].is_array() {
                    v["plist"][0]["pic"] = json!("https://img4.kuwo.cn/other.jpg");
                }
            } else if variant == "tracks" && v["info"].is_object() {
                v["info"]["musiclist"].as_array_mut().unwrap().reverse();
            } else if variant == "tags" && v["sl_data"].is_object() {
                v["sl_data"]["tag"] = json!("古典,安静");
            }
            *bytes = json_response(&v);
        }
        let mut f = setup(b).await;
        let e = f
            .client
            .native_update_playlist_cover(&credential(), "101", &request(None))
            .await
            .unwrap_err();
        assert_eq!(e.code, ErrorCode::Conflict);
        assert_eq!(e.details["write_requests_dispatched"], 2);
        fixture::requests(&mut f, 17).await;
    }
}

#[tokio::test]
async fn native_cover_published_or_not_owned_targets_never_upload() {
    let mut b = flow("42");
    for i in [2, 5] {
        let mut v = body(&b[i]);
        v["sl_data"]["igsl"] = json!("1");
        b[i] = json_response(&v);
    }
    b.truncate(7);
    let mut f = setup(b).await;
    let e = f
        .client
        .native_update_playlist_cover(&credential(), "101", &request(None))
        .await
        .unwrap_err();
    assert_eq!(e.code, ErrorCode::CapabilityNotSupported);
    assert!(e.details.get("write_outcome").is_none());
    fixture::requests(&mut f, 7).await;
    let mut b = flow("42");
    b[1] = json_response(&json!({"errcode":0,"plist":[]}));
    b.truncate(2);
    let mut f = setup(b).await;
    assert!(
        f.client
            .native_update_playlist_cover(&credential(), "101", &request(None))
            .await
            .is_err()
    );
    fixture::requests(&mut f, 2).await;
}

async fn setup(bodies: Vec<Vec<u8>>) -> fixture::Fixture {
    fixture::setup_gated_with_preparation(
        bodies.into_iter().map(|b| (b, None)).collect(),
        Duration::from_secs(30),
    )
    .await
}

#[tokio::test]
async fn native_cover_exif_orientation_precedes_square_crop() {
    let source =
        ::image::RgbImage::from_fn(3, 2, |x, y| ::image::Rgb([x as u8 * 80, y as u8 * 100, 0]));
    let mut bytes = Cursor::new(Vec::new());
    ::image::DynamicImage::ImageRgb8(source)
        .write_to(&mut bytes, ::image::ImageFormat::Jpeg)
        .unwrap();
    let jpeg = bytes.into_inner();
    // Independent TIFF little-endian orientation=6 APP1, no file or decoder helper.
    let exif = [
        b'E', b'x', b'i', b'f', 0, 0, b'I', b'I', 42, 0, 8, 0, 0, 0, 1, 0, 0x12, 1, 3, 0, 1, 0, 0,
        0, 6, 0, 0, 0, 0, 0, 0, 0,
    ];
    let mut encoded = vec![0xff, 0xd8, 0xff, 0xe1, 0, 34];
    encoded.extend(exif);
    encoded.extend(&jpeg[2..]);
    let mut r = request(None);
    r.content_type = "image/jpeg".into();
    r.data = encoded;
    image::validate("101", &r).unwrap();
    let prepared = image::prepare(&r).await.unwrap();
    assert_eq!((prepared.source_width, prepared.source_height), (2, 3));
    assert_eq!(prepared.crop, (0, 0, 2));
    let mut plain = r.clone();
    plain.data = jpeg;
    let original = image::prepare(&plain).await.unwrap();
    assert_eq!((original.source_width, original.source_height), (3, 2));
    assert_ne!(prepared.jpeg, original.jpeg);
}

#[tokio::test]
async fn native_cover_declared_dimensions_cannot_exceed_decode_budget() {
    let source = ::image::RgbImage::from_pixel(1, 1, ::image::Rgb([0, 0, 0]));
    let mut bytes = Cursor::new(Vec::new());
    ::image::DynamicImage::ImageRgb8(source)
        .write_to(&mut bytes, ::image::ImageFormat::Bmp)
        .unwrap();
    for (w, h) in [(8193_u32, 1_u32), (1, 8193), (5000, 5000)] {
        let mut r = request(None);
        r.content_type = "image/bmp".into();
        r.data = bytes.get_ref().clone();
        r.data[18..22].copy_from_slice(&w.to_le_bytes());
        r.data[22..26].copy_from_slice(&h.to_le_bytes());
        image::validate("101", &r).unwrap();
        assert_eq!(
            image::prepare(&r).await.err().unwrap().code,
            ErrorCode::InvalidRequest
        );
    }
}
