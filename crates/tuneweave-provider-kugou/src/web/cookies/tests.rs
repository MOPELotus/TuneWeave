use super::*;

const VALUE: &str = "KugooID=111&t=synthetic-web-token&a_id=1014&ct=1700000000&NickName=%u6D4B%u8BD5%uD83C%uDFB5&Pic=https%3A%2F%2Fimge.kugou.com%2Favatar.jpg";
fn received(values: &[String]) -> Result<WebCookie> {
    let mut headers = HeaderMap::new();
    for value in values {
        headers.append(SET_COOKIE, value.parse().unwrap());
    }
    WebCookie::received(&headers, 1700000000)
}
fn cookie(value: &str, attrs: &str) -> String {
    format!("KuGoo={value}; {attrs}")
}

#[test]
fn official_cookie_fields_decode_without_evaluating_javascript_and_remain_private() {
    let value = received(&[cookie(
        VALUE,
        "Domain=.kugou.com; Path=/; Secure; HttpOnly; Max-Age=60",
    )])
    .unwrap();
    let identity = value.identity().unwrap();
    assert_eq!(identity.user_id, "111");
    assert_eq!(identity.nickname.as_deref(), Some("测试🎵"));
    assert_eq!(
        identity.avatar_url.as_deref(),
        Some("https://imge.kugou.com/avatar.jpg")
    );
    assert_eq!(identity.token, "synthetic-web-token");
    let header = value.header(1700000059).unwrap();
    assert!(header.is_sensitive());
    assert_eq!(header.to_str().unwrap(), format!("KuGoo={VALUE}"));
    assert_eq!(
        value.header(1700000060).unwrap_err().code,
        ErrorCode::AuthenticationRequired
    );
    assert!(!format!("{value:?}").contains("synthetic-web-token"));
    assert_eq!(
        text("%E6%B5%8B%E8%AF%95+a", 512).unwrap().as_deref(),
        Some("测试+a")
    );
    assert_eq!(
        text("%27%29%3Balert%281%29%3B%28%27", 512)
            .unwrap()
            .as_deref(),
        Some("');alert(1);('")
    );
    for invalid in [
        "%",
        "%gg",
        "%FF",
        "%uD800",
        "%uD800%u0041",
        "%uDC00",
        "%00",
        "%0A",
    ] {
        assert!(text(invalid, 512).is_err(), "{invalid}");
    }
}

#[test]
fn cookie_scope_expiry_and_duplicate_headers_never_select_an_ambiguous_identity() {
    for attrs in [
        "Domain=evil.com; Path=/",
        "Domain=com; Path=/",
        "Domain=www.kugou.com; Path=/",
        "Path=/v11",
        "Path=/other",
        "Domain=kugou.com; Domain=kugou.com",
        "Path=/; Path=/v1",
        "Max-Age=no",
        "Expires=bad",
        "Max-Age=1; Max-Age=2",
    ] {
        assert!(received(&[cookie(VALUE, attrs)]).is_err(), "{attrs}");
    }
    for attrs in [
        "",
        "Domain=kugou.com; Path=/",
        "Domain=loginservice.kugou.com; Path=/v1",
        "Path=/v1/",
    ] {
        assert!(received(&[cookie(VALUE, attrs)]).is_ok(), "{attrs}");
    }
    for attrs in [
        "Max-Age=0",
        "Max-Age=-1",
        "Expires=Thu, 01 Jan 1970 00:00:00 GMT",
    ] {
        assert_eq!(
            received(&[cookie(VALUE, attrs)]).unwrap_err().code,
            ErrorCode::AuthenticationRequired
        );
    }
    assert!(
        received(&[cookie(
            VALUE,
            "Expires=Thu, 01 Jan 1970 00:00:00 GMT; Max-Age=60"
        )])
        .is_ok()
    );
    assert!(received(&[cookie(VALUE, "Path=/"), cookie(VALUE, "Path=/v1")]).is_err());
    assert!(received(&["unrelated=synthetic; Path=/".to_owned()]).is_err());
    assert!(
        received(&[
            "unrelated=synthetic; Path=/".to_owned(),
            cookie(VALUE, "Path=/")
        ])
        .is_ok()
    );
}

#[test]
fn cookie_identity_fields_are_strict_and_cannot_be_injected_or_reencoded() {
    for value in [
        "t=secret",
        "KugooID=111",
        "KugooID=0111&t=secret",
        "KugooID=111&t=null",
        "KugooID=111&t=secret&a_id=1005",
        "KugooID=111&t=secret&ct=NaN",
        "KugooID=111&t=secret&KugooID=222",
        "KugooID=111&t=one&t=two",
        "KugooID%3D111%26t%3Dsecret",
        "KugooID=111&t=secret&Pic=https://evil.com/a.jpg",
        "KugooID=111&t=secret&NickName=%0a",
        "KugooID=111&t=secret&unknown=1&unknown=2",
    ] {
        assert!(received(&[cookie(value, "Path=/")]).is_err(), "{value}");
    }
    assert!(
        received(&[cookie(
            &format!("KugooID=111&t={}", "a".repeat(16384)),
            "Path=/"
        )])
        .is_err()
    );
    let c = received(&[cookie(
        "KugooID=111&t=secret&Pic=20260101synthetic.jpg",
        "Path=/",
    )])
    .unwrap();
    assert_eq!(
        c.identity().unwrap().avatar_url.as_deref(),
        Some("https://imge.kugou.com/kugouicon/165/20260101/20260101synthetic.jpg")
    );
}
