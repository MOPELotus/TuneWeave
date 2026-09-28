//! Current native membership state, separate from profile and music authorization.
use super::*;
use chrono::{DateTime, SecondsFormat, Utc};
use std::collections::BTreeSet;
use tuneweave_core::{MembershipSummary, ProviderCredential, ResourceRef};

mod crypto;
mod key;
#[cfg(test)]
pub(crate) mod tests;
pub(super) const PATH: &str = "/vip/enc/user/vip";
const HOST: &str = "vip1.kuwo.cn";
const MAX_TIME: u64 = 253_402_300_799_999;

impl KuwoClient {
    /// Independently validates the credential before reading current music
    /// membership. This neither renews the session nor grants any media rights.
    pub async fn native_membership(
        &self,
        credential: &ProviderCredential,
    ) -> Result<MembershipSummary> {
        let input = credential::NativeCredential::parse(credential)?.input()?;
        validate_session_metadata(&input)?;
        self.validate_native_session(&input).await?;
        self.fetch_native_membership(&input).await
    }
    pub(crate) async fn fetch_native_membership(
        &self,
        input: &KuwoNativeSessionInput,
    ) -> Result<MembershipSummary> {
        let metadata = session_metadata(input)?;
        let query = {
            let mut query = url::form_urlencoded::Serializer::new(String::new());
            query.extend_pairs([
                ("op", "ui"),
                ("uid", input.user_id()),
                ("sid", input.session_id()),
                ("extend", "1"),
                ("showChezai", "1"),
                ("showLinqi", "1"),
                ("apiVersion", "6"),
                ("devid", input.device_id()),
                ("user", input.device_user()),
                ("source", CLIENT_SOURCE),
                ("platform", "ar"),
            ]);
            query.finish()
        };
        let target = format!("{}?{query}", self.native_target(HOST, PATH));
        self.native_get_with_metadata(
            HOST,
            PATH,
            "native_membership",
            target,
            Some(metadata),
            |wire| parse(&crypto::decode(wire)?, input),
        )
        .await
    }
}

#[derive(Deserialize)]
struct Envelope {
    meta: Meta,
    #[serde(default, deserialize_with = "number")]
    ctime: Option<u64>,
    data: Option<Membership>,
}
#[derive(Deserialize)]
struct Meta {
    #[serde(default, deserialize_with = "deserialize_code")]
    code: Option<String>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Membership {
    // Some deployments may echo the request identity. Never accept a conflicting
    // echo; absent identity is bound by independent validation and this request.
    #[serde(default, deserialize_with = "deserialize_code")]
    uid: Option<String>,
    #[serde(default, deserialize_with = "number")]
    vip_expire: Option<u64>,
    #[serde(default, deserialize_with = "number")]
    vipm_expire: Option<u64>,
    #[serde(default, deserialize_with = "number")]
    vip_luxury_expire: Option<u64>,
    #[serde(default, deserialize_with = "number")]
    svip_expire: Option<u64>,
    #[serde(default, deserialize_with = "number")]
    chezai_expire: Option<u64>,
    #[serde(default, deserialize_with = "number")]
    experience_expire: Option<u64>,
    #[serde(default, deserialize_with = "number")]
    vip_ad_expire: Option<u64>,
    #[serde(default, deserialize_with = "number")]
    vip3_expire: Option<u64>,
    #[serde(default, deserialize_with = "flag")]
    vipm_auto_pay_user: Option<bool>,
    #[serde(default, deserialize_with = "flag")]
    lux_auto_pay_user: Option<bool>,
    #[serde(default, deserialize_with = "flag")]
    svip_auto_pay_user: Option<bool>,
    #[serde(default, rename = "cheZaiAutoPayUser", deserialize_with = "flag")]
    che_zai_auto_pay_user: Option<bool>,
    #[serde(default, deserialize_with = "number")]
    is_year_user: Option<u64>,
    vip_tag: Option<String>,
    user_vip_type: Option<String>,
    vip_icon: Option<String>,
}

fn number<'de, D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Option<u64>, D::Error> {
    deserialize_code(d)?
        .filter(|value| !value.trim().is_empty())
        .map(|v| {
            v.parse::<u64>()
                .ok()
                .filter(|n| n.to_string() == v)
                .ok_or_else(|| serde::de::Error::custom("invalid membership number"))
        })
        .transpose()
}
fn flag<'de, D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Option<bool>, D::Error> {
    number(d)?
        .map(|v| match v {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(serde::de::Error::custom("invalid membership flag")),
        })
        .transpose()
}

#[derive(Serialize)]
struct Entry {
    kind: &'static str,
    active: Option<bool>,
    state: &'static str,
    expires_at_ms: Option<u64>,
    expires_at: Option<String>,
    auto_renew: Option<bool>,
}
fn entry(kind: &'static str, expiry: Option<u64>, renew: Option<bool>, now: u64) -> Result<Entry> {
    if expiry.is_some_and(|value| value > MAX_TIME) {
        return Err(invalid());
    }
    let state = match expiry {
        None => "unknown",
        Some(0) => "none",
        Some(n) if n > now => "active",
        Some(_) => "expired",
    };
    Ok(Entry {
        kind,
        active: expiry.map(|v| v > now),
        state,
        expires_at_ms: expiry.filter(|v| *v > 0),
        expires_at: expiry.filter(|v| *v > 0).map(timestamp).transpose()?,
        auto_renew: renew,
    })
}
fn timestamp(value: u64) -> Result<String> {
    let value = i64::try_from(value).map_err(|_| invalid())?;
    let date = DateTime::<Utc>::from_timestamp_millis(value).ok_or_else(invalid)?;
    Ok(date.to_rfc3339_opts(SecondsFormat::Millis, true))
}

fn parse(bytes: &[u8], input: &KuwoNativeSessionInput) -> Result<MembershipSummary> {
    let result = parse_inner(bytes, input);
    if result.is_err() {
        #[cfg(debug_assertions)]
        eprintln!(
            "DIAGNOSTIC kuwo_membership_shape={} fields={}",
            super::diagnostic_response_shape(bytes),
            diagnostic_field_states(bytes)
        );
    }
    result
}

/// Classify only fields consumed by this parser. Diagnostics never include
/// membership values, account identity, or response text.
#[cfg(debug_assertions)]
fn diagnostic_field_states(bytes: &[u8]) -> serde_json::Value {
    let Some(root) = serde_json::from_slice::<serde_json::Value>(bytes)
        .ok()
        .filter(serde_json::Value::is_object)
    else {
        return json!({"root": "invalid"});
    };
    let data = root.get("data").and_then(serde_json::Value::as_object);
    let numeric = [
        "vipExpire",
        "vipmExpire",
        "vipLuxuryExpire",
        "svipExpire",
        "chezaiExpire",
        "experienceExpire",
        "vipAdExpire",
        "vip3Expire",
        "isYearUser",
    ];
    let flags = [
        "vipmAutoPayUser",
        "luxAutoPayUser",
        "svipAutoPayUser",
        "cheZaiAutoPayUser",
    ];
    let classify = |value: Option<&serde_json::Value>, flag: bool| match value {
        None => "missing",
        Some(serde_json::Value::Null) => "null",
        Some(serde_json::Value::Bool(_)) => "boolean",
        Some(serde_json::Value::Number(n)) => {
            if n.as_u64().is_some_and(|v| !flag || v <= 1) {
                "valid_number"
            } else {
                "invalid_number"
            }
        }
        Some(serde_json::Value::String(s)) if s.trim().is_empty() => "empty",
        Some(serde_json::Value::String(s)) => {
            if s.parse::<u64>()
                .ok()
                .is_some_and(|v| v.to_string() == *s && (!flag || v <= 1))
            {
                "valid_number"
            } else {
                "invalid_number"
            }
        }
        Some(_) => "other",
    };
    let number_states = numeric
        .into_iter()
        .map(|key| {
            (
                key,
                classify(data.and_then(|object| object.get(key)), false),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let flag_states = flags
        .into_iter()
        .map(|key| (key, classify(data.and_then(|object| object.get(key)), true)))
        .collect::<BTreeMap<_, _>>();
    let code = root
        .get("meta")
        .and_then(|meta| meta.get("code"))
        .map(|value| match value {
            serde_json::Value::String(s) if s == "200" => "expected",
            serde_json::Value::Number(n) if n.as_u64() == Some(200) => "expected",
            serde_json::Value::String(_) | serde_json::Value::Number(_) => "unexpected",
            _ => "invalid",
        })
        .unwrap_or("missing");
    json!({"code": code, "numeric": number_states, "flags": flag_states})
}

fn parse_inner(bytes: &[u8], input: &KuwoNativeSessionInput) -> Result<MembershipSummary> {
    let has_music_expiry_field = serde_json::from_slice::<serde_json::Value>(bytes)
        .ok()
        .and_then(|value| {
            value
                .get("data")
                .and_then(|data| data.get("vipmExpire"))
                .cloned()
        })
        .is_some();
    if !has_music_expiry_field {
        return Err(membership_failure("music_expiry_missing"));
    }
    let body: Envelope = serde_json::from_slice(bytes).map_err(|_| membership_failure("schema"))?;
    if body.meta.code.as_deref() != Some("200") {
        return Err(membership_failure("response_guard"));
    }
    let now = body
        .ctime
        .filter(|value| *value > 0 && *value <= MAX_TIME)
        .ok_or_else(|| membership_failure("server_time"))?;
    let data = body
        .data
        .ok_or_else(|| membership_failure("membership_data_missing"))?;
    if data
        .uid
        .as_deref()
        .is_some_and(|uid| uid != input.user_id())
    {
        return Err(membership_failure("identity_mismatch"));
    }
    if [
        data.vip_expire,
        data.vipm_expire,
        data.vip_luxury_expire,
        data.svip_expire,
        data.chezai_expire,
        data.experience_expire,
        data.vip_ad_expire,
        data.vip3_expire,
    ]
    .into_iter()
    .flatten()
    .any(|value| value > MAX_TIME)
    {
        return Err(membership_failure("expiry_range"));
    }
    let entries = [
        entry("legacy_vip", data.vip_expire, None, now)?,
        entry("music", data.vipm_expire, data.vipm_auto_pay_user, now)?,
        entry(
            "luxury",
            data.vip_luxury_expire,
            data.lux_auto_pay_user,
            now,
        )?,
        entry("super", data.svip_expire, data.svip_auto_pay_user, now)?,
        entry("car", data.chezai_expire, data.che_zai_auto_pay_user, now)?,
        entry("experience", data.experience_expire, None, now)?,
        entry("ad", data.vip_ad_expire, None, now)?,
        entry("given", data.vip3_expire, None, now)?,
    ];
    // SpecialInfoMgr.F uses music/luxury/super/given membership for the music
    // account classification. Other product states remain separate entries.
    let music = [&entries[1], &entries[2], &entries[3], &entries[7]];
    let active = if music.iter().any(|v| v.active == Some(true)) {
        Some(true)
    } else if music.iter().all(|v| v.active == Some(false)) {
        Some(false)
    } else {
        None
    };
    // Different simultaneous products do not have one authoritative expiry.
    let expiries: BTreeSet<_> = music
        .iter()
        .filter(|v| v.active == Some(true))
        .filter_map(|v| v.expires_at.clone())
        .collect();
    let expires_at = if expiries.len() == 1 && music.iter().all(|v| v.active.is_some()) {
        expiries.into_iter().next()
    } else {
        None
    };
    if data.is_year_user.is_some_and(|v| v > i32::MAX as u64) {
        return Err(membership_failure("year_code_range"));
    }
    // Display-only metadata must never make valid membership expiry data fail.
    // Apply the same secret, length, and control checks, then omit rejected text.
    let tag = text(data.vip_tag, 1024, input).ok().flatten();
    let user_type = text(data.user_vip_type, 128, input).ok().flatten();
    let icon = text(data.vip_icon, 2048, input).ok().flatten();
    let icon_url = icon.filter(|value| {
        let Ok(url) = Url::parse(value) else {
            return false;
        };
        matches!(url.scheme(), "http" | "https")
            && value.trim() == value
            && !value.contains('\\')
            && url.username().is_empty()
            && url.password().is_none()
            && url.port().is_none()
            && url.query().is_none()
            && url.fragment().is_none()
            && url
                .host_str()
                .is_some_and(|host| host.ends_with(".kuwo.cn"))
    });
    Ok(MembershipSummary {
        user_ref: Some(ResourceRef::new(Platform::Kuwo, input.user_id()).map_err(|_| invalid())?),
        level: None,
        active,
        annual_count: None,
        expires_at,
        icon_url,
        extensions: BTreeMap::from([
            ("backend".into(), json!("native_vip_api6")),
            ("identity_binding".into(), json!("validated_native_session")),
            ("server_time_ms".into(), json!(now)),
            (
                "active_scope".into(),
                json!(["music", "luxury", "super", "given"]),
            ),
            (
                "memberships".into(),
                serde_json::to_value(entries).map_err(|_| invalid())?,
            ),
            ("annual_user_code".into(), json!(data.is_year_user)),
            ("vip_tag".into(), json!(tag)),
            ("user_vip_type".into(), json!(user_type)),
        ]),
    })
}
fn membership_failure(_stage: &'static str) -> TuneWeaveError {
    #[cfg(debug_assertions)]
    eprintln!("DIAGNOSTIC kuwo_membership_failure={_stage}");
    invalid()
}
fn text(
    value: Option<String>,
    max: usize,
    input: &KuwoNativeSessionInput,
) -> Result<Option<String>> {
    let Some(value) = value.filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    if value.len() > max
        || value.chars().any(char::is_control)
        || echoes_secret(&value, input.session_id())
    {
        return Err(invalid());
    }
    Ok(Some(value))
}
fn invalid() -> TuneWeaveError {
    kuwo_upstream_error("Kuwo native membership response is invalid")
}
