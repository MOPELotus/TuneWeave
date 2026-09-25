use super::*;

const KEY: &str = "personal:avatar2d:tmp:used";
fn usage(value: serde_json::Value) -> String {
    reply(json!({"code":"000000","data":{KEY:value}}), None)
}
struct Flow {
    responses: Vec<String>,
    upload: usize,
    convert: usize,
    converted_mode: usize,
    converted_profile: usize,
    usage_write: Option<usize>,
    usage_read: usize,
    audit: usize,
}
fn flow(already_used: bool) -> Flow {
    let old = frames();
    let mut responses = old[..UPLOAD].to_vec();
    responses[5] = mode(1);
    responses.push(usage(json!(u8::from(already_used))));
    let upload = responses.len();
    responses.push(old[UPLOAD].clone());
    responses.push(native_profile("111"));
    responses.push(mode(1));
    let convert = responses.len();
    responses.push(reply(json!({"code":"000000"}), None));
    let converted_mode = responses.len();
    responses.push(mode(0));
    let converted_profile = responses.len();
    responses.push(native_profile("111"));
    let usage_write = if already_used {
        None
    } else {
        let at = responses.len();
        responses.push(reply(json!({"code":"000000"}), None));
        Some(at)
    };
    let usage_read = responses.len();
    responses.push(usage(json!(1)));
    let audit = responses.len();
    responses.push(old[10].clone());
    responses.push(old[11].clone());
    Flow {
        responses,
        upload,
        convert,
        converted_mode,
        converted_profile,
        usage_write,
        usage_read,
        audit,
    }
}

#[tokio::test]
async fn account_avatar_conversion_uses_official_form_and_usage_json_with_isolated_readback() {
    for selection in ["default", "named", "caller"] {
        for already_used in [false, true] {
            let f = flow(already_used);
            let mut h = server(f.responses.clone(), None).await;
            let (store, original, alias) = setup(&mut h.provider, selection);
            let result = h
                .provider
                .upload_account_avatar(&request(Some(alias)))
                .await
                .unwrap();
            assert_eq!(result.extensions["converted_to_static"], true);
            assert_eq!(result.extensions["write_outcome"], "pending_review");
            assert!(result.url.is_none() && result.image_id.is_none());
            let wire = h.requests.await.unwrap();
            assert_eq!(wire.len(), f.responses.len());
            assert_eq!(
                wire.iter().filter(|v| v.starts_with(b"POST ")).count(),
                if already_used { 2 } else { 3 }
            );
            assert!(
                header(&wire[f.upload])
                    .starts_with("POST /MIGUM2.0/v1.0/picUpload.do?syncOtherApp=false&type=00 ")
            );
            let headers = header(&wire[f.convert]);
            assert!(headers.starts_with("POST /user/api/avatar/convert/v1.0 HTTP/1.1"));
            assert!(headers.contains("content-type: application/x-www-form-urlencoded\r\n"));
            assert_eq!(&wire[f.convert][headers.len() + 4..], b"convertType=0");
            if let Some(at) = f.usage_write {
                let headers = header(&wire[at]);
                assert!(headers.starts_with("POST /personal/recommend/set/v1.0 HTTP/1.1"));
                assert!(headers.contains("content-type: application/json;charset=utf-8\r\n"));
                assert_eq!(
                    &wire[at][headers.len() + 4..],
                    br#"[{"functionKey":"personal:avatar2d:tmp:used","functionValue":1}]"#
                );
            }
            for at in [7, f.usage_read] {
                assert!(header(&wire[at]).starts_with("GET /personal/recommend/api/v1.0?functionKey=personal%3Aavatar2d%3Atmp%3Aused HTTP/1.1"));
            }
            for at in [Some(f.convert), f.usage_write].into_iter().flatten() {
                let headers = header(&wire[at]);
                for expected in [
                    "token: native-token-fixture\r\n",
                    "signversion: V005\r\n",
                    "sign: ",
                    "appid: music\r\n",
                    "os: Android\r\n",
                ] {
                    assert!(headers.contains(expected));
                }
                assert!(!headers.contains("pacmtoken"));
            }
            assert_eq!(read(&store, "other").token(), "unrelated-pacm");
            if selection == "caller" {
                assert_eq!(read(&store, alias), original);
                let update = h.provider.take_response_credential().unwrap().unwrap();
                assert_eq!(
                    MiguCredential::parse_caller(&update).unwrap(),
                    original.rotate("final-pacm".into()).unwrap()
                );
                assert!(!update.secret().contains("native-token-fixture"));
            } else {
                assert_eq!(
                    read(&store, alias),
                    original.rotate("final-pacm".into()).unwrap()
                );
                assert!(h.provider.take_response_credential().unwrap().is_none());
            }
        }
    }
}

#[tokio::test]
async fn account_avatar_conversion_invalid_usage_marker_prevents_upload() {
    for value in [json!(null), json!("1"), json!(2), json!(-1), json!({})] {
        let mut f = flow(false);
        f.responses[7] = usage(value);
        f.responses.truncate(8);
        let mut h = server(f.responses, None).await;
        let (_, _, alias) = setup(&mut h.provider, "named");
        let failure = h
            .provider
            .upload_account_avatar(&request(Some(alias)))
            .await
            .unwrap_err();
        assert_eq!(failure.code, ErrorCode::UpstreamError);
        assert!(failure.details.get("upload_requests_dispatched").is_none());
        assert!(
            h.requests
                .await
                .unwrap()
                .iter()
                .all(|r| !r.starts_with(b"POST "))
        );
    }
}

#[tokio::test]
async fn account_avatar_conversion_reports_partial_confirmation_without_retries() {
    let base = flow(false);
    for (at, replacement, code, converted, usage_confirmed) in [
        (base.convert - 1, mode(0), ErrorCode::Conflict, false, false),
        (
            base.convert,
            reply(json!({"code":"200004","info":"native-token-fixture"}), None),
            ErrorCode::PermissionDenied,
            false,
            false,
        ),
        (
            base.convert,
            reply(json!({"code":0}), None),
            ErrorCode::UpstreamError,
            false,
            false,
        ),
        (
            base.converted_mode,
            mode(1),
            ErrorCode::UpstreamError,
            false,
            false,
        ),
        (
            base.converted_profile,
            native_profile("222"),
            ErrorCode::PermissionDenied,
            false,
            false,
        ),
        (
            base.usage_write.unwrap(),
            reply(json!({"code":"200004"}), None),
            ErrorCode::PermissionDenied,
            true,
            false,
        ),
        (
            base.usage_read,
            usage(json!(0)),
            ErrorCode::UpstreamError,
            true,
            false,
        ),
        (
            base.audit,
            audit(json!(["9"])),
            ErrorCode::UpstreamError,
            true,
            true,
        ),
        (
            base.audit + 1,
            profile("222", ""),
            ErrorCode::AuthenticationRequired,
            true,
            true,
        ),
    ] {
        let mut responses = base.responses.clone();
        responses[at] = replacement;
        responses.truncate(at + 1);
        let mut h = server(responses, None).await;
        let (store, original, alias) = setup(&mut h.provider, "caller");
        let mut failure = h
            .provider
            .upload_account_avatar(&request(Some(alias)))
            .await
            .unwrap_err();
        assert_eq!(failure.code, code, "at {at}");
        assert!(!failure.retryable);
        assert_eq!(failure.details["write_outcome"], "unconfirmed");
        assert_eq!(failure.details["automatic_retry"], false);
        if at >= base.convert {
            assert_eq!(failure.details["mode_conversion_requests_dispatched"], 1);
            assert_eq!(failure.details["mode_conversion_confirmed"], converted);
        } else {
            assert!(
                failure
                    .details
                    .get("mode_conversion_requests_dispatched")
                    .is_none()
            );
        }
        if at >= base.usage_write.unwrap() {
            assert_eq!(failure.details["avatar_usage_requests_dispatched"], 1);
            assert_eq!(
                failure.details["avatar_usage_marker_confirmed"],
                usage_confirmed
            );
        } else {
            assert!(
                failure
                    .details
                    .get("avatar_usage_requests_dispatched")
                    .is_none()
            );
        }
        assert!(!failure.details.to_string().contains("native-token-fixture"));
        if matches!(
            code,
            ErrorCode::AuthenticationRequired | ErrorCode::Conflict
        ) {
            assert!(h.provider.take_response_credential().unwrap().is_none());
            assert!(failure.take_caller_credential_update().is_none());
        } else {
            let expected = original.rotate("verified-pacm".into()).unwrap();
            assert_eq!(
                MiguCredential::parse_caller(
                    &h.provider.take_response_credential().unwrap().unwrap()
                )
                .unwrap(),
                expected
            );
            assert_eq!(
                MiguCredential::parse_caller(&failure.take_caller_credential_update().unwrap())
                    .unwrap(),
                expected
            );
        }
        assert_eq!(read(&store, alias), original);
        let wire = h.requests.await.unwrap();
        let expected_posts =
            1 + usize::from(at >= base.convert) + usize::from(at >= base.usage_write.unwrap());
        assert_eq!(
            wire.iter().filter(|v| v.starts_with(b"POST ")).count(),
            expected_posts
        );
    }
}

#[tokio::test]
async fn account_avatar_conversion_observes_login_generation_at_each_write() {
    let base = flow(false);
    for at in [base.convert - 1, base.convert, base.usage_write.unwrap()] {
        let mut responses = base.responses.clone();
        responses.truncate(at + 1);
        let mut h = server(responses, Some(at)).await;
        let (store, _, alias) = setup(&mut h.provider, "named");
        let provider = h.provider.clone();
        let task =
            tokio::spawn(
                async move { provider.upload_account_avatar(&request(Some(alias))).await },
            );
        tokio::time::timeout(Duration::from_secs(5), h.seen)
            .await
            .unwrap()
            .unwrap();
        let next = MiguCredential::verified("111".into(), "replacement-login".into()).unwrap();
        store.put(&stored(alias, &next)).unwrap();
        h.release.send(()).unwrap();
        assert_eq!(task.await.unwrap().unwrap_err().code, ErrorCode::Conflict);
        assert_eq!(read(&store, alias), next);
        assert_eq!(h.requests.await.unwrap().len(), at + 1);
    }
}

#[tokio::test]
async fn account_avatar_conversion_cancellation_never_exports_undelivered_caller_rotation() {
    let base = flow(false);
    for at in [base.convert, base.usage_write.unwrap()] {
        let mut responses = base.responses.clone();
        responses.truncate(at + 1);
        let mut h = server(responses, Some(at)).await;
        let (store, original, alias) = setup(&mut h.provider, "caller");
        let provider = h.provider.clone();
        let task =
            tokio::spawn(
                async move { provider.upload_account_avatar(&request(Some(alias))).await },
            );
        tokio::time::timeout(Duration::from_secs(5), h.seen)
            .await
            .unwrap()
            .unwrap();
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        assert!(h.provider.take_response_credential().unwrap().is_none());
        assert_eq!(read(&store, alias), original);
        h.requests.abort();
    }
}
