use super::*;
use crate::client::{
    catalog::tests::response,
    native::tests::{requests, setup},
};

#[tokio::test]
async fn revocation_uses_exact_selected_native_identity_and_one_send() {
    let input =
        KuwoNativeSessionInput::new("42", "sid+&42", "123456789", "device-user-123").unwrap();
    let mut f = setup(vec![response(200, "text/plain", "", b"result=ok\r\n")]).await;
    let sent = std::sync::atomic::AtomicBool::new(false);
    f.client
        .send_native_session_revocation(&input, || {
            sent.store(true, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        })
        .await
        .unwrap();
    assert!(sent.load(std::sync::atomic::Ordering::SeqCst));
    let plain = query(&input, b"17894932");
    assert_eq!(
        plain,
        "uid=42&sid=sid%2B%2642&src=kwplayer_ar_12.2.2.0_newpcguanwangmobile.apk&version=12.2.2.0&dev_id=123456789&user=device-user-123&dev_name=TuneWeave+SDK+client&devType=SDK&sx=17894932&from=android&devResolution=0*0"
    );
    let seen = requests(&mut f, 1).await;
    assert_eq!(
        seen[0].lines().next().unwrap(),
        format!(
            "GET {PATH}?f=ar&q={} HTTP/1.1",
            codec::seal_query(plain.as_bytes()).unwrap()
        )
    );
    assert!(seen[0].contains("loginUid=42,loginSid=sid+&42,appUid=123456789,"));
    assert!(!seen[0].to_lowercase().contains("\r\ncookie:"));
}

#[test]
fn revocation_receipt_rejects_substring_duplicate_and_ambiguous_success() {
    for value in ["result=ok", "result=ok\r\n", "result=ok\nmessage=done"] {
        assert!(parse(value.as_bytes()).is_ok());
    }
    for value in [
        "",
        "result=okay",
        "result=fail",
        "notresult=ok",
        "result=ok\nresult=fail",
        "result=ok\nresult=ok",
        "<html>result=ok</html>",
        "{\"result\":\"ok\"}",
    ] {
        assert!(parse(value.as_bytes()).is_err(), "{value}");
    }
}

#[tokio::test]
async fn revocation_pre_send_guard_prevents_network() {
    let f = setup(vec![]).await;
    let input =
        KuwoNativeSessionInput::new("42", "sid-42", "123456789", "device-user-123").unwrap();
    assert!(
        f.client
            .send_native_session_revocation(&input, || Err(kuwo_invalid_request("changed")))
            .await
            .is_err()
    );
}
