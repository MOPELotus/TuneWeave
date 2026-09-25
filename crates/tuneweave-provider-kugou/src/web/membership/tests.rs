use super::*;
use serde_json::json;

#[test]
fn membership_web_keeps_role_dates_and_unknown_status_without_native_codes() {
    for (role, active) in [
        (0, Some(false)),
        (1, Some(true)),
        (2, Some(true)),
        (11, Some(true)),
        (13, Some(true)),
        (31, Some(false)),
        (33, Some(false)),
        (999, None),
    ] {
        let member=parse(json!({"role":role,"vipEndTime":"2026-12-31 23:59:59","rawVipEndTime":"2026-12-31 23:59:59","musicEndTime":"2027-01-01 00:00:00","musicUsed":"20","token":"not-public","phone":"not-public"}).to_string().as_bytes(),"111").unwrap();
        assert_eq!(member.active, active);
        assert!(member.level.is_none());
        assert_eq!(member.expires_at.as_deref(), Some("2026-12-31 23:59:59"));
        assert_eq!(member.extensions["membership_details"]["musicUsed"], 20);
        assert!(
            !serde_json::to_string(&member)
                .unwrap()
                .contains("not-public")
        );
    }
}

#[test]
fn membership_web_denial_missing_role_and_malformed_values_never_mean_inactive() {
    let error = parse(br#"{"errno":105,"error_code":20017}"#, "111").unwrap_err();
    assert_eq!(error.code, ErrorCode::AuthenticationRequired);
    for body in [
        br#"{}"#.as_slice(),
        br#"{"status":1,"data":{"vip_type":0}}"#,
        br#"{"role":0,"role":1}"#,
        br#"{"role":false}"#,
        br#"{"role":"01"}"#,
        br#"{"role":1,"vipEndTime":1}"#,
        br#"{"role":1,"vipEndTime":"bad\nvalue"}"#,
        br#"{"role":0,"errno":99}"#,
    ] {
        assert!(parse(body, "111").is_err());
    }
    assert!(parse(&vec![b' '; LIMIT + 1], "111").is_err());
}
