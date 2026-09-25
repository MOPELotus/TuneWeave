use super::*;

pub(crate) fn card(kind: &str, active: &str, date: &str) -> serde_json::Value {
    json!({"cardMemberIdentity":{"identityFullPinYin":kind,"name":"Product","title":"您已开通会员"},"memberIdentityRightsItem":{"identityFullPinYin":active,"identityName":"Account status"},"identityItems":[{"identityFullPinYin":active,"name":"Subscription","memberItems":[{"notInForce":false,"payType":"01","validTime":date}]}]})
}
fn parse(cards: Vec<serde_json::Value>) -> MembershipSummary {
    parse_membership(json!({"memberCards":cards}), "111").unwrap()
}

#[test]
fn anonymous_product_titles_do_not_prove_membership_and_unknown_identities_stay_unknown() {
    let template = json!({"cardMemberIdentity":{"identityFullPinYin":"tianlaisvip","name":"Product","title":"您已开通会员"}});
    let none = json!({"cardMemberIdentity":{"identityFullPinYin":"baijinhuiyuan"},"memberIdentityRightsItem":{"identityFullPinYin":"feihuiyuan"},"identityItems":[{"identityFullPinYin":"feihuiyuan","level":"0"}]});
    assert_eq!(parse(vec![template.clone()]).active, None);
    let result = parse(vec![none.clone(), template]);
    assert_eq!(result.active, Some(false));
    assert_eq!(result.level, None);
    assert_eq!(result.expires_at, None);
    assert_eq!(
        parse(vec![none, card("tianlaivip", "future_type", "20260916")]).active,
        None
    );
    assert_eq!(
        parse(vec![
            card("baijinhuiyuan", "baijinhuiyuan", "20260916"),
            card("tianlaivip", "future_type", "20260916")
        ])
        .active,
        Some(true)
    );
}

#[test]
fn effective_pending_and_unactivated_subscriptions_preserve_distinct_states() {
    let mut c = card("baijinhuiyuan", "baijinhuiyuan", "20261001");
    c["identityItems"][0]["memberItems"]
        .as_array_mut()
        .unwrap()
        .push(json!({"notInForce":true,"validTime":"20270101","payType":"01"}));
    c["notInForceItems"] =
        json!([{"name":"Later","notInForceDay":"30","identityFullPinYin":"baijinhuiyuan"}]);
    c["notActiveItems"] =
        json!([{"name":"Activate later","activeNum":2,"serviceId":"do-not-export"}]);
    let result = parse(vec![c]);
    assert_eq!(result.active, Some(true));
    assert_eq!(result.expires_at.as_deref(), Some("2026-10-01"));
    let cards = &result.extensions["cards"];
    assert_eq!(cards[0]["subscriptions"][0]["state"], "active");
    assert_eq!(cards[0]["subscriptions"][1]["state"], "pending");
    assert_eq!(cards[0]["pending"][0]["days"], 30);
    assert_eq!(cards[0]["pending"][1]["state"], "awaiting_activation");
    assert!(
        !serde_json::to_string(&result)
            .unwrap()
            .contains("do-not-export")
    );
}

#[test]
fn membership_expiry_preserves_calendar_precision_and_monthly_sentinels() {
    for (raw, expected) in [
        ("20280229", Some("2028-02-29")),
        ("202609152359", Some("2026-09-15T23:59")),
        ("20260915235958", Some("2026-09-15T23:59:58")),
        ("20991231", None),
    ] {
        let result = parse(vec![card("baijinhuiyuan", "baijinhuiyuan", raw)]);
        assert_eq!(result.expires_at.as_deref(), expected);
        assert!(result.annual_count.is_none());
    }
    for pay in ["00", "02"] {
        let mut c = card("baijinhuiyuan", "baijinhuiyuan", "20260915");
        c["identityItems"][0]["memberItems"][0]["payType"] = json!(pay);
        assert_eq!(parse(vec![c]).expires_at, None);
    }
    let multiple = parse(vec![
        card("baijinhuiyuan", "baijinhuiyuan", "20261001"),
        card("tianlaivip", "tianlaivip", "20261101"),
    ]);
    assert_eq!(multiple.active, Some(true));
    assert!(multiple.expires_at.is_none());
    assert_eq!(
        multiple.extensions["cards"][1]["subscriptions"][0]["expires_at"],
        "2026-11-01"
    );
    for raw in [
        "20260229",
        "20260001",
        "20261301",
        "20260431",
        "20260100",
        "00000101",
        "202609152400",
        "20260915235960",
        "bad-secret",
        "20260915Z",
    ] {
        let error = parse_membership(
            json!({"memberCards":[card("baijinhuiyuan", "baijinhuiyuan", raw)]}),
            "111",
        )
        .unwrap_err();
        assert!(!format!("{error:?}").contains(raw));
    }
}

#[test]
fn omitted_effective_flags_and_pending_only_cards_do_not_assert_active_membership() {
    let mut c = card("baijinhuiyuan", "baijinhuiyuan", "20261001");
    c.as_object_mut()
        .unwrap()
        .remove("memberIdentityRightsItem");
    c["identityItems"][0]["memberItems"][0]
        .as_object_mut()
        .unwrap()
        .remove("notInForce");
    assert_eq!(parse(vec![c.clone()]).active, None);
    c["identityItems"][0]["memberItems"][0]["notInForce"] = json!(true);
    assert_eq!(parse(vec![c.clone()]).active, None);
    c["identityItems"][0]["memberItems"][0]["notInForce"] = json!(0);
    assert_eq!(parse(vec![c]).active, Some(true));
}

#[test]
fn malformed_oversized_duplicate_and_contradictory_membership_data_are_rejected() {
    let valid = card("baijinhuiyuan", "baijinhuiyuan", "20261001");
    for body in [
        json!({}),
        json!({"memberCards":[]}),
        json!({"memberCards":[valid.clone(),valid.clone()]}),
        json!({"memberCards":vec![valid.clone();17]}),
    ] {
        assert!(parse_membership(body, "111").is_err());
    }
    for (field, value) in [
        ("notInForce", json!(2)),
        ("notInForce", json!("false")),
        ("payType", json!("secret\n")),
        ("name", json!("x".repeat(513))),
    ] {
        let mut c = valid.clone();
        c["identityItems"][0]["memberItems"][0][field] = value;
        assert!(parse_membership(json!({"memberCards":[c]}), "111").is_err());
    }
    let mut c = valid.clone();
    c["memberIdentityRightsItem"]["identityFullPinYin"] = json!("feihuiyuan");
    assert!(parse_membership(json!({"memberCards":[c]}), "111").is_err());
    let mut c = valid;
    c["notInForceItems"] = json!([{"notInForceDay":-1}]);
    assert!(parse_membership(json!({"memberCards":[c]}), "111").is_err());
}

#[test]
fn member_icons_have_bounded_public_fields_and_no_action_urls() {
    let icons=parse_icons(json!([{"memberTypeName":"Member","iconUrl":"https://d.musicapp.migu.cn/icon.png","actionUrl":"secret-checkout"}])).unwrap();
    assert_eq!(
        icons[0].icon_url.as_deref(),
        Some("https://d.musicapp.migu.cn/icon.png")
    );
    assert!(
        !serde_json::to_string(&icons)
            .unwrap()
            .contains("secret-checkout")
    );
    assert!(parse_icons(json!([])).unwrap().is_empty());
    for url in [
        "http://d.musicapp.migu.cn/a",
        "https://user:secret@d.musicapp.migu.cn/a",
        "https://evil.invalid/a",
        "https://d.musicapp.migu.cn:8443/a",
    ] {
        assert!(parse_icons(json!([{"iconUrl":url}])).is_err());
    }
}

#[test]
fn additional_media_identities_do_not_supply_music_membership_flags_and_unknown_subscriptions_suppress_summary_expiry()
 {
    let identities = parse_media_identities(json!({"mediaMemberIdentities":[{"identityFullPinYin":"video_bundle","name":"Other media","payType":"01","validTime":"202610011230","desc":"not-exported"}]})).unwrap();
    let value = serde_json::to_value(identities).unwrap();
    assert_eq!(value[0]["expires_at"], "2026-10-01T12:30");
    assert!(value[0].get("active").is_none());
    assert!(!value.to_string().contains("not-exported"));
    assert!(
        parse_media_identities(json!({"mediaMemberIdentities":[]}))
            .unwrap()
            .is_empty()
    );
    assert!(parse_media_identities(json!({})).is_err());
    assert!(parse_media_identities(json!({"mediaMemberIdentities":[{"identityFullPinYin":"video_bundle","validTime":"20261301"}]})).is_err());
    let mut c = card("baijinhuiyuan", "baijinhuiyuan", "20261001");
    c["identityItems"][0]["memberItems"]
        .as_array_mut()
        .unwrap()
        .push(json!({"validTime":"20270101","payType":"01"}));
    assert!(parse(vec![c]).expires_at.is_none());
}
