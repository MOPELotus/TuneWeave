use super::*;
use tuneweave_core::MembershipSummary;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Center {
    member_cards: Vec<Card>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Card {
    card_member_identity: Product,
    member_identity_rights_item: Option<CurrentIdentity>,
    identity_items: Option<Vec<Identity>>,
    not_in_force_items: Option<Vec<PendingIdentity>>,
    not_active_items: Option<Vec<PendingIdentity>>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Product {
    identity_full_pin_yin: String,
    name: Option<String>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CurrentIdentity {
    identity_full_pin_yin: String,
    identity_name: Option<String>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Identity {
    identity_full_pin_yin: String,
    name: Option<String>,
    member_items: Option<Vec<Member>>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Member {
    name: Option<String>,
    not_in_force: Option<Flag>,
    pay_type: Option<String>,
    valid_time: Option<String>,
}
#[derive(Deserialize)]
#[serde(untagged)]
enum Flag {
    Boolean(bool),
    Number(u8),
}
impl Flag {
    fn value(&self) -> Result<bool> {
        match self {
            Self::Boolean(value) => Ok(*value),
            Self::Number(0) => Ok(false),
            Self::Number(1) => Ok(true),
            _ => Err(invalid()),
        }
    }
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PendingIdentity {
    identity_full_pin_yin: Option<String>,
    name: Option<String>,
    identity_name: Option<String>,
    not_in_force_day: Option<serde_json::Value>,
    active_num: Option<serde_json::Value>,
}
#[derive(Serialize)]
struct Subscription {
    kind: String,
    name: Option<String>,
    state: &'static str,
    payment_kind: Option<String>,
    expires_at: Option<String>,
}
#[derive(Serialize)]
struct Pending {
    kind: Option<String>,
    name: Option<String>,
    state: &'static str,
    days: Option<u32>,
    count: Option<u32>,
}
#[derive(Serialize)]
struct CardSummary {
    product_kind: String,
    product_name: Option<String>,
    current_kind: Option<String>,
    current_name: Option<String>,
    active: Option<bool>,
    subscriptions: Vec<Subscription>,
    pending: Vec<Pending>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Icon {
    icon_url: Option<String>,
    member_type_name: Option<String>,
}
#[derive(Serialize)]
pub(crate) struct MemberIcon {
    pub icon_url: Option<String>,
    pub name: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct MediaIdentityList {
    media_member_identities: Vec<MediaIdentityInput>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct MediaIdentityInput {
    identity_full_pin_yin: String,
    name: Option<String>,
    pay_type: Option<String>,
    valid_time: Option<String>,
}
#[derive(Serialize)]
pub(crate) struct MediaIdentity {
    kind: String,
    name: Option<String>,
    payment_kind: Option<String>,
    expires_at: Option<String>,
}

fn invalid() -> TuneWeaveError {
    migu_upstream_error("Migu membership data is invalid")
}
fn text(value: &str, limit: usize) -> Result<()> {
    if value.trim().is_empty() || value.len() > limit || value.chars().any(char::is_control) {
        return Err(invalid());
    }
    Ok(())
}
fn optional_text(value: Option<String>) -> Result<Option<String>> {
    value
        .filter(|v| !v.is_empty())
        .map(|v| {
            text(&v, 512)?;
            Ok(v)
        })
        .transpose()
}
fn kind(value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 80
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_')
    {
        return Err(invalid());
    }
    Ok(())
}
fn active_kind(value: &str) -> Option<bool> {
    match value {
        "feihuiyuan" | "nomusic" | "nosuper" => Some(false),
        "tejihuiyuan"
        | "baijinhuiyuan"
        | "lianhehuiyuan"
        | "baijinhuiyuantiyanban"
        | "xiaoshibao"
        | "5gchangwanbao"
        | "baijinhuiyuanchangxianban"
        | "baijinhuiyuanchangtingban"
        | "baijinhuiyuanrika"
        | "cailingyinyuetequan"
        | "baijinhuiyuanchangtingtiyanban"
        | "xingzuomhuiyuan"
        | "baijinhuiyuanduanshiquanyi"
        | "mzone"
        | "chaojihuiyuan"
        | "zhoutongxueyingyuanbao"
        | "zhoutongxuexinghuiyingyuanbao"
        | "zhoutongxuexingyaoyingyuanbao"
        | "15"
        | "16"
        | "12"
        | "2"
        | "tianlaihuiyuan"
        | "tianlaichangtinghuiyuan"
        | "tianlaivip"
        | "tianlaivip_exp"
        | "tianlaisvip" => Some(true),
        _ => None,
    }
}
fn count(value: Option<serde_json::Value>) -> Result<Option<u32>> {
    value
        .map(|v| {
            let n = if let Some(n) = v.as_u64() {
                n
            } else if let Some(s) = v.as_str() {
                if s.is_empty() || s.len() > 6 || !s.bytes().all(|b| b.is_ascii_digit()) {
                    return Err(invalid());
                }
                s.parse().map_err(|_| invalid())?
            } else {
                return Err(invalid());
            };
            if n > 365_000 {
                return Err(invalid());
            }
            Ok(n as u32)
        })
        .transpose()
}

// The API supplies local calendar values, not a timezone-qualified instant. Keep
// their precision and never invent a UTC offset or an expiry for a monthly sentinel.
fn expiry(raw: Option<&str>, pay: Option<&str>) -> Result<Option<String>> {
    let Some(raw) = raw.filter(|v| !v.is_empty()) else {
        return Ok(None);
    };
    if !matches!(raw.len(), 8 | 12 | 14) || !raw.bytes().all(|b| b.is_ascii_digit()) {
        return Err(invalid());
    }
    let number = |start, end| raw[start..end].parse::<u32>().map_err(|_| invalid());
    let year = number(0, 4)?;
    let month = number(4, 6)?;
    let day = number(6, 8)?;
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return Err(invalid()),
    };
    if year < 1970 || day == 0 || day > days {
        return Err(invalid());
    }
    if raw.len() >= 12 && (number(8, 10)? > 23 || number(10, 12)? > 59) {
        return Err(invalid());
    }
    if raw.len() == 14 && number(12, 14)? > 59 {
        return Err(invalid());
    }
    if raw.starts_with("20991231") || matches!(pay, Some("00" | "02")) {
        return Ok(None);
    }
    let mut value = format!("{}-{}-{}", &raw[..4], &raw[4..6], &raw[6..8]);
    if raw.len() >= 12 {
        value.push_str(&format!("T{}:{}", &raw[8..10], &raw[10..12]));
    }
    if raw.len() == 14 {
        value.push_str(&format!(":{}", &raw[12..14]));
    }
    Ok(Some(value))
}

pub(super) fn parse_membership(value: serde_json::Value, uid: &str) -> Result<MembershipSummary> {
    let center: Center = serde_json::from_value(value).map_err(|_| invalid())?;
    if center.member_cards.is_empty() || center.member_cards.len() > 16 {
        return Err(invalid());
    }
    let mut cards = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    let mut account_states = Vec::new();
    let mut active_expiries = Vec::new();
    for card in center.member_cards {
        let product = card.card_member_identity;
        kind(&product.identity_full_pin_yin)?;
        if !seen.insert(product.identity_full_pin_yin.clone()) {
            return Err(invalid());
        }
        let mut output = CardSummary {
            product_kind: product.identity_full_pin_yin,
            product_name: optional_text(product.name)?,
            current_kind: None,
            current_name: None,
            active: None,
            subscriptions: vec![],
            pending: vec![],
        };
        // A product card, including its title, is not evidence of account membership.
        if let Some(current) = card.member_identity_rights_item {
            kind(&current.identity_full_pin_yin)?;
            output.active = active_kind(&current.identity_full_pin_yin);
            output.current_kind = Some(current.identity_full_pin_yin);
            output.current_name = optional_text(current.identity_name)?;
            account_states.push(output.active);
        }
        let identities = card.identity_items.unwrap_or_default();
        if identities.len() > 64 {
            return Err(invalid());
        }
        for identity in identities {
            kind(&identity.identity_full_pin_yin)?;
            let name = optional_text(identity.name)?;
            let members = identity.member_items.unwrap_or_default();
            if members.len() > 64 {
                return Err(invalid());
            }
            for member in members {
                if output.subscriptions.len() >= 256 {
                    return Err(invalid());
                }
                let pending = member.not_in_force.as_ref().map(Flag::value).transpose()?;
                let active = active_kind(&identity.identity_full_pin_yin);
                let state = match (pending, active) {
                    (Some(true), _) => "pending",
                    (Some(false), Some(true)) => "active",
                    (Some(false), Some(false)) => "inactive",
                    _ => "unknown",
                };
                if output.active == Some(false) && state == "active" {
                    return Err(invalid());
                }
                if let Some(pay) = &member.pay_type {
                    kind(pay)?;
                }
                let expires_at = expiry(member.valid_time.as_deref(), member.pay_type.as_deref())?;
                output.subscriptions.push(Subscription {
                    kind: identity.identity_full_pin_yin.clone(),
                    name: optional_text(member.name)?.or_else(|| name.clone()),
                    state,
                    payment_kind: member.pay_type,
                    expires_at,
                });
            }
        }
        for (items, state) in [
            (card.not_in_force_items, "pending"),
            (card.not_active_items, "awaiting_activation"),
        ] {
            let items = items.unwrap_or_default();
            if items.len() > 128 {
                return Err(invalid());
            }
            for item in items {
                if let Some(value) = &item.identity_full_pin_yin {
                    kind(value)?;
                }
                output.pending.push(Pending {
                    kind: item.identity_full_pin_yin,
                    name: optional_text(item.name)?.or(optional_text(item.identity_name)?),
                    state,
                    days: count(item.not_in_force_day)?,
                    count: count(item.active_num)?,
                });
            }
        }
        if output.current_kind.is_none() && !output.subscriptions.is_empty() {
            // Explicit effective subscriptions can identify an account even when the
            // card omits its current-rights descriptor. Missing flags remain unknown.
            let state = if output.subscriptions.iter().any(|s| s.state == "active") {
                Some(true)
            } else {
                None
            };
            output.active = state;
            account_states.push(state);
        }
        if output.active == Some(true) {
            let effective: Vec<_> = output
                .subscriptions
                .iter()
                .filter(|s| s.state == "active")
                .collect();
            active_expiries.push(
                if effective.len() == 1
                    && !output.subscriptions.iter().any(|s| s.state == "unknown")
                {
                    effective[0].expires_at.clone()
                } else {
                    None
                },
            );
        }
        cards.push(output);
    }
    let active = if account_states.contains(&Some(true)) {
        Some(true)
    } else if !account_states.is_empty() && account_states.iter().all(|s| *s == Some(false)) {
        Some(false)
    } else {
        None
    };
    let expires_at = if active_expiries.len() == 1 && !account_states.contains(&None) {
        active_expiries.pop().flatten()
    } else {
        None
    };
    Ok(MembershipSummary {
        user_ref: Some(ResourceRef::new(Platform::Migu, uid).map_err(|_| invalid())?),
        level: None,
        active,
        annual_count: None,
        expires_at,
        icon_url: None,
        extensions: Extensions::from([
            ("backend".into(), json!("official_member_center_v3")),
            ("expiry_format".into(), json!("platform_local_calendar")),
            (
                "cards".into(),
                serde_json::to_value(cards).map_err(|_| invalid())?,
            ),
        ]),
    })
}

pub(super) fn parse_icons(value: serde_json::Value) -> Result<Vec<MemberIcon>> {
    let icons: Vec<Icon> = serde_json::from_value(value).map_err(|_| invalid())?;
    if icons.len() > 32 {
        return Err(invalid());
    }
    icons
        .into_iter()
        .map(|icon| {
            let icon_url = icon
                .icon_url
                .filter(|v| !v.is_empty())
                .map(|v| super::account::profile_image(&v).ok_or_else(invalid))
                .transpose()?;
            Ok(MemberIcon {
                icon_url,
                name: optional_text(icon.member_type_name)?,
            })
        })
        .collect()
}

pub(super) fn parse_media_identities(value: serde_json::Value) -> Result<Vec<MediaIdentity>> {
    let data: MediaIdentityList = serde_json::from_value(value).map_err(|_| invalid())?;
    if data.media_member_identities.len() > 64 {
        return Err(invalid());
    }
    data.media_member_identities
        .into_iter()
        .map(|identity| {
            kind(&identity.identity_full_pin_yin)?;
            if let Some(pay) = &identity.pay_type {
                kind(pay)?;
            }
            Ok(MediaIdentity {
                kind: identity.identity_full_pin_yin,
                name: optional_text(identity.name)?,
                expires_at: expiry(identity.valid_time.as_deref(), identity.pay_type.as_deref())?,
                payment_kind: identity.pay_type,
            })
        })
        .collect()
}

#[cfg(test)]
pub(crate) mod tests;
