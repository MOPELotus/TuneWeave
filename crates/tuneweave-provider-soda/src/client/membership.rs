//! Official PC commerce v2 membership read; no offers, orders or payment actions.
use super::*;
use crate::login::SodaCredential;
use tuneweave_core::MembershipSummary;

pub(crate) const BACKEND: &str = "official_pc_commerce_membership_v2";
const PATH: &str = "/luna/pc/commerce/v2/commerce_info";
const MAX_BYTES: usize = 1024 * 1024;
const MAX_TIME: u64 = 4_102_444_800;

#[derive(Debug)]
pub(crate) struct SodaMembershipRead {
    pub summary: MembershipSummary,
    // Candidate only: the Provider independently checks its UID before saving it.
    pub credential: SodaCredential,
}

#[derive(Deserialize)]
struct Business {
    status_code: Option<i64>,
    status_info: Option<BusinessStatus>,
}
#[derive(Deserialize)]
struct BusinessStatus {
    status_code: Option<i64>,
}
#[derive(Deserialize)]
struct Envelope {
    status_info: Status,
    membership: Member,
}
#[derive(Deserialize)]
struct Status {
    now: u64,
    now_ts_ms: Option<u64>,
}
#[derive(Deserialize)]
struct Member {
    is_membership: Option<bool>,
    membership_type: Option<String>,
    expire_time: Option<u64>,
    is_paying_user: Option<bool>,
    is_about_to_expire: Option<bool>,
    in_grace_period: Option<bool>,
    last_membership_type: Option<String>,
}

fn invalid() -> TuneWeaveError {
    soda_upstream_error("Soda membership returned incomplete or invalid account data")
}
fn unauthenticated() -> TuneWeaveError {
    TuneWeaveError::new(
        ErrorCode::AuthenticationRequired,
        "Soda membership requires an authenticated account",
    )
    .with_platform(Platform::Soda)
}

impl SodaClient {
    pub(crate) async fn commerce_membership(
        &self,
        source: &SodaCredential,
    ) -> Result<SodaMembershipRead> {
        let user_id = source.user_id().ok_or_else(unauthenticated)?;
        let started = Instant::now();
        let mut status = None;
        let result = async {
            let device = self.login_device()?;
            let mut url = Url::parse("https://api.qishui.com").map_err(|_| invalid())?;
            url.set_path(PATH);
            url.query_pairs_mut()
                .append_pair("aid", SODA_APP_ID)
                .append_pair("app_name", "luna_pc")
                .append_pair("device_platform", "windows")
                .append_pair("channel", "official")
                .append_pair("version_name", "2.1.0")
                .append_pair("version_code", "20010000")
                .append_pair("device_id", &device.device_id)
                .append_pair("iid", &device.install_id)
                .append_pair("fp", &device.device_id);
            let mut response = self
                .login_request(reqwest::Method::POST, url)
                .header(reqwest::header::ACCEPT, "application/json")
                .header(reqwest::header::COOKIE, source.cookie_header()?)
                .json(&json!({"includes":["membership"]}))
                .send()
                .await
                .map_err(soda_network_error)?;
            status = Some(response.status());
            if response.status() == StatusCode::UNAUTHORIZED {
                return Err(unauthenticated());
            }
            if response.status() != StatusCode::OK {
                return Err(soda_http_error(response.status()));
            }
            if response.headers().contains_key("bdturing-verify") {
                return Err(TuneWeaveError::new(
                    ErrorCode::CapabilityNotSupported,
                    "Soda membership requires an additional platform verification challenge",
                )
                .with_platform(Platform::Soda));
            }
            if !response
                .headers()
                .get(CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.split(';').next())
                .is_some_and(|v| v.trim().eq_ignore_ascii_case("application/json"))
                || response
                    .content_length()
                    .is_some_and(|n| n > MAX_BYTES as u64)
            {
                return Err(invalid());
            }
            let headers = response.headers().clone();
            let mut body = Vec::new();
            while let Some(chunk) = response.chunk().await.map_err(soda_network_error)? {
                if body.len().saturating_add(chunk.len()) > MAX_BYTES {
                    return Err(invalid());
                }
                body.extend_from_slice(&chunk);
            }
            let summary = parse(&body, user_id)?;
            let credential = source.with_response_cookies(&headers)?;
            crate::account::reject_secrets(
                &serde_json::to_value(&summary).map_err(|_| invalid())?,
                &[source.clone(), credential.clone()],
            )?;
            Ok(SodaMembershipRead {
                summary,
                credential,
            })
        }
        .await;
        self.log_upstream_request(
            "account_membership",
            "api.qishui.com",
            PATH,
            status,
            started,
            &result,
        );
        result
    }
}

fn parse(bytes: &[u8], user_id: &str) -> Result<MembershipSummary> {
    let business: Business = serde_json::from_slice(bytes).map_err(|_| invalid())?;
    let nested = business.status_info.and_then(|v| v.status_code);
    if [business.status_code, nested].contains(&Some(1_000_016)) {
        return Err(unauthenticated());
    }
    if business.status_code != Some(0) || nested.is_some_and(|n| n != 0) {
        return Err(invalid());
    }
    let envelope: Envelope = serde_json::from_slice(bytes).map_err(|_| invalid())?;
    let status = envelope.status_info;
    if status.now == 0
        || status.now > MAX_TIME
        || status.now_ts_ms.is_some_and(|ms| ms / 1000 != status.now)
    {
        return Err(invalid());
    }
    let member = envelope.membership;
    if member.expire_time.is_some_and(|n| n > MAX_TIME) {
        return Err(invalid());
    }
    let kind = member_type(member.membership_type)?;
    let last = member_type(member.last_membership_type)?;
    if member.is_membership.is_none()
        && kind.is_none()
        && member.expire_time.is_none()
        && member.is_paying_user.is_none()
        && member.is_about_to_expire.is_none()
        && member.in_grace_period.is_none()
        && last.is_none()
    {
        return Err(invalid());
    }
    let mut extensions = Extensions::new();
    extensions.insert("backend".into(), json!(BACKEND));
    extensions.insert("source_user_id".into(), json!(user_id));
    extensions.insert("observed_at_epoch_seconds".into(), json!(status.now));
    if let Some(kind) = kind {
        // Preserve the existing stage field while naming the commerce field explicitly.
        extensions.insert("vip_stage".into(), json!(kind));
        extensions.insert("membership_type".into(), json!(kind));
    }
    if let Some(last) = last {
        extensions.insert("last_membership_type".into(), json!(last));
    }
    if let Some(expiry) = member.expire_time {
        extensions.insert("expires_at_epoch_seconds".into(), json!(expiry));
    }
    for (key, value) in [
        ("is_paying_user", member.is_paying_user),
        ("is_about_to_expire", member.is_about_to_expire),
        ("in_grace_period", member.in_grace_period),
    ] {
        if let Some(value) = value {
            extensions.insert(key.into(), json!(value));
        }
    }
    Ok(MembershipSummary {
        user_ref: Some(ResourceRef::new(Platform::Soda, user_id).map_err(|_| invalid())?),
        active: member.is_membership,
        expires_at: member
            .expire_time
            .filter(|v| *v > 0)
            .map(|v| unix_rfc3339(v).ok_or_else(invalid))
            .transpose()?,
        level: None,
        annual_count: None,
        icon_url: None,
        extensions,
    })
}
fn member_type(value: Option<String>) -> Result<Option<String>> {
    if value
        .as_ref()
        .is_some_and(|v| v.len() > 64 || v.chars().any(char::is_control))
    {
        return Err(invalid());
    }
    Ok(value.filter(|v| !v.trim().is_empty()))
}

#[cfg(test)]
pub(crate) mod tests;
