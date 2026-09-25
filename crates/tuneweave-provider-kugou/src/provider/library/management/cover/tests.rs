use super::*;
use crate::provider::library::tests::store_account;
use crate::provider::session::tests::{
    binary_server, credential, exchange, paused, profile, raw, read, reply,
};
use std::collections::BTreeMap;

const ID: &str = "cloudlist:111:0:37";
const PIC: &str = "https://imge.kugou.com/custom/400/new.jpg";
fn request(account: Option<&str>) -> ImageUploadRequest {
    let image = ::image::RgbImage::from_fn(600, 400, |x, _| {
        if x < 300 {
            ::image::Rgb([240, 20, 10])
        } else {
            ::image::Rgb([10, 20, 240])
        }
    });
    let mut bytes = std::io::Cursor::new(Vec::new());
    ::image::DynamicImage::ImageRgb8(image)
        .write_to(&mut bytes, ::image::ImageFormat::Png)
        .unwrap();
    ImageUploadRequest {
        filename: "cover.png".into(),
        content_type: "image/png".into(),
        data: bytes.into_inner(),
        image_size: Some(400),
        crop_x: Some(200),
        crop_y: Some(0),
        account: account.map(str::to_owned),
    }
}
fn row(id: u64, pic: &str) -> Value {
    json!({"listid":id,"type":0,"name":format!("Name {id}"),"is_def":0,"is_pri":1,
        "intro":"Keep","tags":"甲","sort":id,"count":0,"m_count":0,"list_ver":3,
        "global_collection_id":format!("collection_1_111_{id}_0"),"pic":pic})
}
fn library(rows: Vec<Value>, version: u64) -> String {
    reply(json!({"userid":111,"total_ver":version,"list_count":rows.len(),"info":rows}))
}
fn frames() -> Vec<String> {
    vec![
        exchange("111", "next"),
        profile("111"),
        library(
            vec![
                row(37, "https://imge.kugou.com/old.jpg"),
                row(3, "https://imge.kugou.com/other.jpg"),
            ],
            9,
        ),
        reply(json!({"authorization":"fixture-upload-authorization"})),
        raw(json!({"IsSuccess":true,"FileName":"new.jpg"})),
        reply(
            json!({"userid":111,"total_ver":10,"pre_total_ver":9,"list_count":2,"info":{"code":1,"listid":37,"type":0,"pic":PIC,"global_collection_id":"collection_1_111_37_0"}}),
        ),
        library(
            vec![row(37, PIC), row(3, "https://imge.kugou.com/other.jpg")],
            10,
        ),
    ]
}
fn concept_frames() -> Vec<String> {
    let mut responses = frames();
    responses[3] = reply(
        json!({"authorization":"","authorizations":["fixture-concept-authorization","unused"]}),
    );
    responses[5] = reply(
        json!({"userid":111,"total_ver":10,"pre_total_ver":9,"list_count":2,
        "info":{"listid":37,"type":0,"pic":PIC,"global_collection_id":"collection_1_111_37_0"}}),
    );
    responses
}
fn concept_credential(token: &str) -> KugouCredential {
    let mut session = credential("111", token).native().session.clone();
    session.client = KugouLoginClient::Concept;
    KugouCredential::verified(session).unwrap()
}
fn packet(bytes: &[u8]) -> (&str, BTreeMap<String, String>, &[u8]) {
    let split = bytes.windows(4).position(|v| v == b"\r\n\r\n").unwrap();
    let head = std::str::from_utf8(&bytes[..split]).unwrap();
    let target = head
        .lines()
        .next()
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap();
    let url = url::Url::parse(&format!("http://localhost{target}")).unwrap();
    (
        head,
        url.query_pairs().into_owned().collect(),
        &bytes[split + 4..],
    )
}

#[tokio::test]
async fn cover_preparation_decodes_supported_formats_and_bounds_output_and_crop() {
    for (format, mime) in [
        (::image::ImageFormat::Jpeg, "image/jpeg"),
        (::image::ImageFormat::Png, "image/png"),
        (::image::ImageFormat::Gif, "image/gif"),
        (::image::ImageFormat::Bmp, "image/bmp"),
    ] {
        let source = ::image::DynamicImage::ImageRgb8(::image::RgbImage::from_pixel(
            1200,
            1100,
            ::image::Rgb([30, 60, 90]),
        ));
        let mut encoded = std::io::Cursor::new(Vec::new());
        source.write_to(&mut encoded, format).unwrap();
        let mut r = request(None);
        r.data = encoded.into_inner();
        r.content_type = mime.into();
        r.image_size = None;
        r.crop_x = None;
        r.crop_y = None;
        image::validate(&r).unwrap();
        let prepared = image::prepare(&r, KugouLoginClient::Standard)
            .await
            .unwrap();
        assert_eq!(prepared.crop, (50, 0, 1100));
        assert_eq!(prepared.output_size, 1000);
        assert_eq!(
            ::image::guess_format(&prepared.jpeg).unwrap(),
            ::image::ImageFormat::Jpeg
        );
        assert_eq!(
            ::image::load_from_memory(&prepared.jpeg).unwrap().width(),
            1000
        );
    }
}

#[tokio::test]
async fn standard_cover_authorizes_uploads_saves_and_reads_back_for_store_and_caller_accounts() {
    for caller in [false, true] {
        let mut f = binary_server(frames().into_iter().map(Into::into).collect()).await;
        let store = store_account(&mut f.provider);
        let previous = read(&store, "A");
        let other = read(&store, "B");
        let provider = if caller {
            f.provider
                .caller_scope(&previous.caller().unwrap())
                .unwrap()
        } else {
            f.provider.clone()
        };
        let result = provider
            .update_playlist_cover(ID, &request(if caller { None } else { Some("A") }))
            .await
            .unwrap();
        assert_eq!(result.image.url.as_deref(), Some(PIC));
        assert_eq!(result.image.image_id.as_deref(), Some("new.jpg"));
        assert_eq!(result.extensions["readback_verified"], true);
        assert_eq!(read(&store, "B"), other);
        if caller {
            assert_eq!(read(&store, "A"), previous);
            assert!(provider.take_response_credential().unwrap().is_some());
        } else {
            assert_eq!(read(&store, "A").native().session.token, "next");
        }
        let packets = f.requests.await.unwrap();
        assert_eq!(packets.len(), 7);
        for (index, path) in [
            (3, "GET /v1/upload/auth?"),
            (4, "POST /imageupload/v3/stream.php?"),
            (5, "POST /cloudlist.service/v4/modify_list?"),
        ] {
            let (head, mut params, body) = packet(&packets[index]);
            assert!(head.starts_with(path));
            let signature = params.remove("signature").unwrap();
            let params_ref = params
                .iter()
                .map(|(k, v)| (k.as_str(), v.clone()))
                .collect();
            assert_eq!(
                signature,
                crate::signing::android_signature(&params_ref, if index == 5 { body } else { b"" })
            );
            if index == 3 {
                assert_eq!(params["token"], "next");
                assert_eq!(params["method"], "POST");
                assert!(body.is_empty());
            } else {
                assert!(!params.contains_key("token"));
                assert!(!params.contains_key("userid"));
            }
            if index == 4 {
                assert_eq!(params["authorization"], "fixture-upload-authorization");
                assert_eq!(params["iscovered"], "1");
                assert_eq!(params["body_empty"], "1");
                assert!(head.contains("content-type: application/octet-stream"));
                let decoded = ::image::load_from_memory(body).unwrap().to_rgb8();
                assert_eq!(decoded.dimensions(), (400, 400));
                assert!(decoded.get_pixel(350, 200)[2] > 200);
            }
            if index == 5 {
                assert!(head.to_ascii_lowercase().contains("kg-module: 27"));
                let data: Value = serde_json::from_slice(body).unwrap();
                assert_eq!(data["pic"], "custom/new.jpg");
                assert_eq!(data["total_ver"], 9);
                assert_eq!(data["listid"], 37);
                assert_eq!(data["list_create_gid"], "collection_1_111_37_0");
                assert_eq!(data["support_pub"], 1);
                for omitted in ["name", "intro", "tags", "sort", "is_pri", "token"] {
                    assert!(data.get(omitted).is_none());
                }
                assert!(!String::from_utf8_lossy(body).contains("next"));
            }
        }
    }
}

#[tokio::test]
async fn concept_cover_authorizes_uploads_saves_and_reads_back_for_store_and_caller_accounts() {
    for caller in [false, true] {
        let mut responses = concept_frames();
        if caller {
            // Unlike Standard code1, Concept's cover UI uses status plus pic.
            responses[5] = reply(json!({"code":0,"info":{"code":0,"pic":PIC}}));
        }
        let mut f = binary_server(responses.into_iter().map(Into::into).collect()).await;
        let store = store_account(&mut f.provider);
        store
            .put(&concept_credential("old").stored("A").unwrap())
            .unwrap();
        let previous = read(&store, "A");
        let other = read(&store, "B");
        let provider = if caller {
            f.provider
                .caller_scope(&previous.caller().unwrap())
                .unwrap()
        } else {
            f.provider.clone()
        };
        let source = ::image::DynamicImage::ImageRgb8(::image::RgbImage::from_pixel(
            1200,
            1100,
            ::image::Rgb([30, 60, 90]),
        ));
        let mut encoded = std::io::Cursor::new(Vec::new());
        source
            .write_to(&mut encoded, ::image::ImageFormat::Png)
            .unwrap();
        let mut r = request(if caller { None } else { Some("A") });
        r.data = encoded.into_inner();
        r.image_size = None;
        r.crop_x = None;
        r.crop_y = None;
        let result = provider.update_playlist_cover(ID, &r).await.unwrap();
        assert_eq!(result.image.url.as_deref(), Some(PIC));
        assert_eq!(result.image.image_id.as_deref(), Some("new.jpg"));
        assert_eq!(result.image.extensions["width"], 400);
        assert_eq!(result.image.extensions["height"], 400);
        assert_eq!(result.image.extensions["crop"], json!([50, 0, 1100]));
        assert_eq!(result.extensions["backend"], "concept_native_cover");
        assert_eq!(result.extensions["readback_verified"], true);
        assert_eq!(read(&store, "B"), other);
        if caller {
            assert_eq!(read(&store, "A"), previous);
            let renewed = provider.take_response_credential().unwrap().unwrap();
            let renewed = KugouCredential::parse_caller(&renewed).unwrap();
            assert_eq!(renewed.native().session.client, KugouLoginClient::Concept);
            assert_eq!(renewed.native().session.token, "next");
        } else {
            assert_eq!(read(&store, "A").native().session.token, "next");
            assert_eq!(
                read(&store, "A").native().session.client,
                KugouLoginClient::Concept
            );
        }
        let packets = f.requests.await.unwrap();
        assert_eq!(packets.len(), 7);
        for (index, path) in [
            (3, "GET /v1/upload/auth?"),
            (4, "POST /imageupload/v3/stream.php?"),
            (5, "POST /cloudlist.service/v4/modify_list?"),
        ] {
            let (head, mut params, body) = packet(&packets[index]);
            assert!(head.starts_with(path));
            assert_eq!(params["appid"], "3116");
            assert_eq!(params["clientver"], "11490");
            assert!(!params.contains_key("method"));
            let signature = params.remove("signature").unwrap();
            let params_ref = params
                .iter()
                .map(|(k, v)| (k.as_str(), v.clone()))
                .collect();
            assert_eq!(
                signature,
                crate::signing::concept_signature(&params_ref, if index == 5 { body } else { b"" })
            );
            if index != 5 {
                assert_eq!(params["userid"], "111");
                assert_eq!(params["token"], "next");
                assert_eq!(params["uuid"], "-");
            } else {
                assert!(!params.contains_key("token"));
                assert!(!params.contains_key("userid"));
            }
            if index == 3 {
                assert_eq!(params["buVerifyCode"], "a77f87462bdcf7db191809faa5b3cef2");
                assert!(body.is_empty());
            }
            if index == 4 {
                use md5::{Digest, Md5};
                let epoch = params["clienttime"].parse().unwrap();
                let local = chrono::DateTime::from_timestamp(epoch, 0)
                    .unwrap()
                    .with_timezone(&chrono::Local);
                assert_eq!(
                    params["md5"],
                    hex::encode(Md5::digest(format!(
                        "{}hewry678WEK23D",
                        local.format("%Y%m%d")
                    )))
                );
                assert_eq!(params["authorization"], "fixture-concept-authorization");
                assert_eq!(params["body_empty"], "1");
                assert!(!params.contains_key("iscovered"));
                assert!(head.contains("content-type: application/octet-stream"));
                let decoded = ::image::load_from_memory(body).unwrap().to_rgb8();
                assert_eq!(decoded.dimensions(), (400, 400));
            }
            if index == 5 {
                assert!(head.to_ascii_lowercase().contains("kg-module: 27"));
                let data: Value = serde_json::from_slice(body).unwrap();
                assert_eq!(data["userid"], 111);
                assert_eq!(data["pic"], "custom/new.jpg");
                assert_eq!(data["total_ver"], 9);
                assert_eq!(data["listid"], 37);
                assert_eq!(data["list_create_gid"], "collection_1_111_37_0");
                assert_eq!(data["support_pub"], 1);
                assert!(data["enckey"].as_str().is_some_and(|v| !v.is_empty()));
                assert!(data["encstr"].as_str().is_some_and(|v| !v.is_empty()));
                for omitted in ["name", "intro", "tags", "sort", "is_pri", "token"] {
                    assert!(data.get(omitted).is_none());
                }
                assert!(!String::from_utf8_lossy(body).contains("next"));
            }
        }
    }
}

#[tokio::test]
async fn concept_cover_requires_first_authorization_success_pic_identity_and_complete_readback() {
    for case in ["auth", "status", "pic", "identity", "readback"] {
        let mut all = concept_frames();
        match case {
            "auth" => {
                all[3] = reply(json!({"authorizations":["","secret-ticket"]}));
                all.truncate(4);
            }
            "status" => {
                all[5] = raw(json!({"status":0,"error_code":100,"data":{"info":{"pic":PIC}}}));
                all.truncate(6);
            }
            "pic" => {
                all[5] = reply(json!({"info":{"pic":""}}));
                all.truncate(6);
            }
            "identity" => {
                all[5] = reply(json!({"userid":999,"info":{"pic":PIC}}));
                all.truncate(6);
            }
            _ => {
                let mut changed = row(3, "https://imge.kugou.com/other.jpg");
                changed["name"] = json!("Unrelated changed");
                all[6] = library(vec![row(37, PIC), changed], 10);
            }
        }
        let count = all.len();
        let mut f = binary_server(all.into_iter().map(Into::into).collect()).await;
        let store = store_account(&mut f.provider);
        store
            .put(&concept_credential("old").stored("A").unwrap())
            .unwrap();
        let error = f
            .provider
            .update_playlist_cover(ID, &request(Some("A")))
            .await
            .unwrap_err();
        assert!(!format!("{error:?}").contains("secret-ticket"));
        assert!(!format!("{error:?}").contains("fixture-concept-authorization"));
        if case != "auth" {
            assert!(!error.retryable);
            assert_eq!(error.details["write_requests_dispatched"], 1);
            assert_eq!(error.details["uploaded_image_may_remain"], true);
        }
        assert_eq!(f.requests.await.unwrap().len(), count);
    }
}

#[tokio::test]
async fn concept_cover_relogin_during_authorization_or_upload_prevents_later_save() {
    for caller in [false, true] {
        for pause_index in [3, 4] {
            let mut all = concept_frames();
            all.truncate(pause_index + 1);
            let (last, resume) = paused(all.pop().unwrap());
            let mut all = all.into_iter().map(Into::into).collect::<Vec<_>>();
            all.push(last);
            let mut f = binary_server(all).await;
            let store = store_account(&mut f.provider);
            store
                .put(&concept_credential("old").stored("A").unwrap())
                .unwrap();
            let old = read(&store, "A");
            let provider = if caller {
                f.provider.caller_scope(&old.caller().unwrap()).unwrap()
            } else {
                f.provider.clone()
            };
            let task = {
                let p = provider.clone();
                tokio::spawn(async move {
                    p.update_playlist_cover(ID, &request(if caller { None } else { Some("A") }))
                        .await
                })
            };
            for _ in 0..=pause_index {
                f.seen.recv().await.unwrap();
            }
            let newer = concept_credential("newer-login");
            if caller {
                *provider.caller_credential.as_ref().unwrap().lock().unwrap() = Some(newer.clone());
            } else {
                store.put(&newer.stored("A").unwrap()).unwrap();
            }
            resume.send(()).unwrap();
            let error = task.await.unwrap().unwrap_err();
            assert_eq!(error.code, ErrorCode::Conflict);
            assert!(provider.take_response_credential().unwrap().is_none());
            if !caller {
                assert_eq!(read(&store, "A"), newer);
            }
            assert_eq!(f.requests.await.unwrap().len(), pause_index + 1);
        }
    }
}

#[tokio::test]
async fn standard_cover_rejects_incomplete_receipts_and_failed_or_unrelated_readbacks() {
    for case in [
        "auth",
        "upload",
        "code",
        "identity",
        "readback",
        "unrelated",
    ] {
        let mut all = frames();
        match case {
            "auth" => {
                all[3] = reply(json!({"authorization":""}));
                all.truncate(4);
            }
            "upload" => {
                all[4] = raw(json!({"IsSuccess":false,"Message":"fixture-upload-authorization"}));
                all.truncate(5);
            }
            "code" => {
                all[5] = reply(json!({"info":{"code":0,"pic":PIC}}));
                all.truncate(6);
            }
            "identity" => {
                all[5] = reply(json!({"userid":999,"info":{"code":1,"pic":PIC}}));
                all.truncate(6);
            }
            "readback" => {
                all[6] = library(
                    vec![
                        row(37, "https://imge.kugou.com/old.jpg"),
                        row(3, "https://imge.kugou.com/other.jpg"),
                    ],
                    10,
                );
            }
            _ => {
                let mut other = row(3, "https://imge.kugou.com/other.jpg");
                other["name"] = json!("Changed");
                all[6] = library(vec![row(37, PIC), other], 10);
            }
        }
        let count = all.len();
        let mut f = binary_server(all.into_iter().map(Into::into).collect()).await;
        store_account(&mut f.provider);
        let error = f
            .provider
            .update_playlist_cover(ID, &request(Some("A")))
            .await
            .unwrap_err();
        assert!(!format!("{error:?}").contains("fixture-upload-authorization"));
        if case != "auth" {
            assert!(!error.retryable);
            assert_eq!(error.details["upload_requests_dispatched"], 1);
        }
        assert_eq!(f.requests.await.unwrap().len(), count);
    }
}

#[tokio::test]
async fn standard_cover_relogin_during_authorization_or_save_prevents_late_effects_and_exports() {
    for caller in [false, true] {
        for pause_index in [3, 5] {
            let mut all = frames();
            all.truncate(pause_index + 1);
            let (last, resume) = paused(all.pop().unwrap());
            let mut all = all.into_iter().map(Into::into).collect::<Vec<_>>();
            all.push(last);
            let mut f = binary_server(all).await;
            let store = store_account(&mut f.provider);
            let old = read(&store, "A");
            let provider = if caller {
                f.provider.caller_scope(&old.caller().unwrap()).unwrap()
            } else {
                f.provider.clone()
            };
            let task = {
                let p = provider.clone();
                tokio::spawn(async move {
                    p.update_playlist_cover(ID, &request(if caller { None } else { Some("A") }))
                        .await
                })
            };
            for _ in 0..=pause_index {
                f.seen.recv().await.unwrap();
            }
            let newer = credential("111", "newer-login");
            if caller {
                *provider.caller_credential.as_ref().unwrap().lock().unwrap() = Some(newer.clone());
            } else {
                store.put(&newer.stored("A").unwrap()).unwrap();
            }
            resume.send(()).unwrap();
            let error = task.await.unwrap().unwrap_err();
            assert_eq!(error.code, ErrorCode::Conflict);
            assert!(provider.take_response_credential().unwrap().is_none());
            if !caller {
                assert_eq!(read(&store, "A"), newer);
            }
            assert_eq!(f.requests.await.unwrap().len(), pause_index + 1);
        }
    }
}

#[tokio::test]
async fn cancelled_cover_upload_never_dispatches_a_later_save() {
    let mut all = frames();
    all.truncate(5);
    let (last, resume) = paused(all.pop().unwrap());
    let mut all = all.into_iter().map(Into::into).collect::<Vec<_>>();
    all.push(last);
    let mut f = binary_server(all).await;
    store_account(&mut f.provider);
    let p = f.provider.clone();
    let task = tokio::spawn(async move { p.update_playlist_cover(ID, &request(Some("A"))).await });
    for _ in 0..5 {
        f.seen.recv().await.unwrap();
    }
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    resume.send(()).unwrap();
    assert_eq!(f.requests.await.unwrap().len(), 5);
}

#[tokio::test]
async fn cover_rechecks_login_after_waiting_for_bounded_image_preparation() {
    let permit = image::pause_preparation().await;
    let mut all = frames();
    all.truncate(3);
    let mut f = binary_server(all.into_iter().map(Into::into).collect()).await;
    let store = store_account(&mut f.provider);
    let p = f.provider.clone();
    let task = tokio::spawn(async move { p.update_playlist_cover(ID, &request(Some("A"))).await });
    for _ in 0..3 {
        f.seen.recv().await.unwrap();
    }
    let newer = credential("111", "newer-login");
    store.put(&newer.stored("A").unwrap()).unwrap();
    drop(permit);
    assert_eq!(task.await.unwrap().unwrap_err().code, ErrorCode::Conflict);
    assert_eq!(read(&store, "A"), newer);
    assert_eq!(f.requests.await.unwrap().len(), 3);
}

#[tokio::test]
async fn cover_preflight_rejects_web_system_lists_and_bad_images() {
    let mut f = binary_server(vec![]).await;
    let store = store_account(&mut f.provider);
    let web =
        KugouCredential::verified_web(crate::web::WebSession::test_session("111", "web-cookie"))
            .unwrap();
    store.put(&web.stored("A").unwrap()).unwrap();
    assert_eq!(
        f.provider
            .update_playlist_cover(ID, &request(Some("A")))
            .await
            .unwrap_err()
            .code,
        ErrorCode::CapabilityNotSupported
    );
    assert!(f.requests.await.unwrap().is_empty());
    let mut all = frames();
    let mut system = row(37, PIC);
    system["is_def"] = json!(1);
    all[2] = library(vec![system], 9);
    all.truncate(3);
    let mut f = binary_server(all.into_iter().map(Into::into).collect()).await;
    store_account(&mut f.provider);
    assert_eq!(
        f.provider
            .update_playlist_cover(ID, &request(Some("A")))
            .await
            .unwrap_err()
            .code,
        ErrorCode::PermissionDenied
    );
    assert_eq!(f.requests.await.unwrap().len(), 3);
    for case in ["mime", "empty", "crop", "small"] {
        let mut r = request(None);
        match case {
            "mime" => r.content_type = "image/jpeg".into(),
            "empty" => r.data.clear(),
            "crop" => r.crop_x = Some(u32::MAX),
            _ => r.image_size = Some(399),
        }
        let result = if image::validate(&r).is_err() {
            Err(())
        } else {
            image::prepare(&r, KugouLoginClient::Standard)
                .await
                .map(|_| ())
                .map_err(|_| ())
        };
        assert!(result.is_err());
    }
}
