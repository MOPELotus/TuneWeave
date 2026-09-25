use super::*;

fn headers(values: &[&str]) -> HeaderMap {
    let mut headers = HeaderMap::new();
    for value in values {
        headers.append(SET_COOKIE, HeaderValue::from_str(value).unwrap());
    }
    headers
}

#[test]
fn voice_cookie_endpoints_rotate_sessions_without_widening_cookie_scope() {
    let mut jar = PassportCookies::default();
    jar.update(
        &headers(&["mgnd_session_id=initial; Domain=.migu.cn; Path=/"]),
        "/authn",
    )
    .unwrap();
    assert_eq!(
        jar.header("/login/send/voice").unwrap().unwrap(),
        "mgnd_session_id=initial"
    );
    jar.update(
        &headers(&[
            "mgnd_session_id=rotated; Domain=.migu.cn; Path=/",
            "mgnd_session_last_access=send-only",
        ]),
        "/login/send/voice",
    )
    .unwrap();
    assert!(
        jar.header("/login/send/voice")
            .unwrap()
            .unwrap()
            .to_str()
            .unwrap()
            .contains("send-only")
    );
    let verify = jar.header("/authn/voice/validate").unwrap().unwrap();
    assert!(verify.is_sensitive());
    assert_eq!(verify, "mgnd_session_id=rotated");
    jar.update(
        &headers(&["mgnd_session_create=verify-only"]),
        "/authn/voice/validate",
    )
    .unwrap();
    assert!(
        jar.header("/authn/voice/validate")
            .unwrap()
            .unwrap()
            .to_str()
            .unwrap()
            .contains("verify-only")
    );
    assert_eq!(
        jar.header("/authn").unwrap().unwrap(),
        "mgnd_session_id=rotated"
    );
    for path in [
        "/user/h5/token-validate/v3.0",
        "/login/send/voice/other",
        "/authn/voice/other",
    ] {
        assert!(jar.header(path).is_err());
        assert!(jar.update(&HeaderMap::new(), path).is_err());
    }
    jar.update(
        &headers(&["mgnd_session_id=shadow; Path=/"]),
        "/authn/voice/validate",
    )
    .unwrap();
    assert!(jar.header("/login/send/voice").is_err());
    assert!(jar.header("/authn/voice/validate").is_err());
}

#[test]
fn passport_cookie_updates_preserve_rotate_delete_and_restrict_paths() {
    let mut jar = PassportCookies::default();
    jar.update(
        &headers(&[
            "mgnd_session_id=first; Path=/; Secure; HttpOnly",
            "mgnd_session_create=key-only",
            "unwanted=discard; Path=/",
            "mgnd_session_last_access=foreign; Domain=example.com; Path=/",
        ]),
        "/password/publickey",
    )
    .unwrap();
    let key = jar.header("/password/publickey").unwrap().unwrap();
    assert!(key.is_sensitive());
    assert!(
        key.to_str()
            .unwrap()
            .contains("mgnd_session_create=key-only")
    );
    assert_eq!(
        jar.header("/login/dynamicpassword").unwrap().unwrap(),
        "mgnd_session_id=first"
    );
    jar.update(
        &headers(&[
            "mgnd_session_id=second; Path=/",
            "mgnd_session_last_access=verify-only; Path=/authn",
        ]),
        "/login/dynamicpassword",
    )
    .unwrap();
    assert_eq!(
        jar.header("/login/dynamicpassword").unwrap().unwrap(),
        "mgnd_session_id=second"
    );
    assert!(
        jar.header("/authn/dynamicpassword")
            .unwrap()
            .unwrap()
            .to_str()
            .unwrap()
            .contains("mgnd_session_last_access=verify-only")
    );
    jar.update(
        &headers(&[
            "mgnd_session_id=; Path=/; Max-Age=0",
            "mgnd_session_create=; Path=/password; Expires=Thu, 01 Jan 1970 00:00:00 GMT",
        ]),
        "/authn/dynamicpassword",
    )
    .unwrap();
    assert!(jar.header("/password/publickey").unwrap().is_none());
    assert!(jar.header("/login/dynamicpassword").unwrap().is_none());
    assert!(jar.header("/user/h5/token-validate/v3.0").is_err());
    assert!(!format!("{jar:?}").contains("verify-only"));
}

#[test]
fn passport_cookie_invalid_updates_are_atomic_and_ambiguous_sessions_are_never_sent() {
    let mut jar = PassportCookies::default();
    jar.update(
        &headers(&["mgnd_session_id=original; Path=/"]),
        "/password/publickey",
    )
    .unwrap();
    for bad in [
        "mgnd_session_last_access=x; Path=/; Path=/authn".into(),
        "mgnd_session_create=x; Path=/; Max-Age=invalid".into(),
        "mgnd_session_create=\"quoted\"; Path=/".into(),
        format!("mgnd_session_create={}; Path=/", "x".repeat(4097)),
    ] {
        assert!(
            jar.update(
                &headers(&["mgnd_session_id=changed; Path=/", &bad]),
                "/password/publickey"
            )
            .is_err()
        );
        assert_eq!(
            jar.header("/login/dynamicpassword").unwrap().unwrap(),
            "mgnd_session_id=original"
        );
    }
    assert!(
        jar.update(
            &headers(&["mgnd_session_id=x; Path=/", "mgnd_session_id=y; Path=/"]),
            "/password/publickey"
        )
        .is_err()
    );
    jar.update(
        &headers(&["mgnd_session_id=shadow; Domain=.migu.cn; Path=/"]),
        "/password/publickey",
    )
    .unwrap();
    assert!(jar.header("/authn/dynamicpassword").is_err());
    jar.update(
        &headers(&["mgnd_session_id=; Domain=.migu.cn; Path=/; Max-Age=-1"]),
        "/password/publickey",
    )
    .unwrap();
    assert_eq!(
        jar.header("/authn/dynamicpassword").unwrap().unwrap(),
        "mgnd_session_id=original"
    );
}

#[test]
fn passport_cookie_lifetimes_honor_max_age_and_expire_before_another_request() {
    let mut jar = PassportCookies::default();
    jar.update(
        &headers(&[
            "mgnd_session_id=alive; Path=/; Max-Age=60; Expires=Thu, 01 Jan 1970 00:00:00 GMT",
            "mgnd_session_create=gone; Path=/; Max-Age=0; Expires=Tue, 01 Jan 2030 00:00:00 GMT",
        ]),
        "/password/publickey",
    )
    .unwrap();
    assert_eq!(
        jar.header("/login/dynamicpassword").unwrap().unwrap(),
        "mgnd_session_id=alive"
    );
    for value in jar.values.values_mut() {
        value.expires = Some(SystemTime::now());
    }
    assert!(jar.header("/login/dynamicpassword").unwrap().is_none());
    jar.update(&HeaderMap::new(), "/authn/dynamicpassword")
        .unwrap();
    assert!(jar.values.is_empty());
}
