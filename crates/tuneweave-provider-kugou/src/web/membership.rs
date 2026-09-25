//! The website's Cookie-based membership is separate from native token protocols.
use super::*;
use crate::account::membership::{LIMIT, Number, code_number, date_text, identity_error, summary};
use reqwest::header::SET_COOKIE;
use tuneweave_core::MembershipSummary;

const MEMBER_HOST: &str = "vip.kugou.com";
const MEMBER_PATH: &str = "/recharge/roleinfo";

pub(crate) struct WebMembership {
    pub(crate) summary: MembershipSummary,
    pub(crate) candidate: WebSession,
}

impl WebSession {
    pub(crate) fn membership_secrets(&self) -> Result<Vec<String>> {
        Ok(vec![
            self.cookie.identity()?.token,
            self.cookie
                .header(crate::account::now_ms()? / 1000)?
                .to_str()
                .map_err(|_| malformed())?
                .to_owned(),
        ])
    }
}

#[derive(Deserialize)]
struct Status {
    errno: Option<i64>,
    error_code: Option<i64>,
}
#[derive(Deserialize, Serialize)]
struct Role {
    role: Number,
    #[serde(rename = "vipEndTime")]
    vip_end_time: Option<String>,
    #[serde(rename = "rawVipEndTime")]
    raw_vip_end_time: Option<String>,
    #[serde(rename = "vipRemains")]
    vip_remains: Option<Number>,
    #[serde(rename = "musicEndTime")]
    music_end_time: Option<String>,
    #[serde(rename = "musicUsed")]
    music_used: Option<Number>,
    producttype: Option<Number>,
    #[serde(rename = "autoChargeType")]
    auto_charge_type: Option<Number>,
    #[serde(rename = "autoChargeTime")]
    auto_charge_time: Option<String>,
    autostatus: Option<Number>,
}

impl KugouClient {
    pub(crate) async fn web_membership(&self, session: &WebSession) -> Result<WebMembership> {
        if !session.valid() {
            return Err(invalid());
        }
        let now = crate::account::now_ms()?;
        // A login-host-only cookie is not readable by the official VIP site.
        session.cookie.media_token(now / 1000)?;
        let cookie = session.cookie.header(now / 1000)?;
        let url = format!("https://{MEMBER_HOST}{MEMBER_PATH}");
        #[cfg(test)]
        let url = self
            .login_test_origin
            .as_ref()
            .map(|o| o.join(MEMBER_PATH).unwrap().to_string())
            .unwrap_or(url);
        let started = Instant::now();
        let mut status = None;
        let result = async {
            let response = self
                .http
                .get(url)
                .query(&[("n", now.to_string())])
                .header(COOKIE, cookie)
                .header("accept", "application/json")
                .header(REFERER, "https://vip.kugou.com/")
                .send()
                .await
                .map_err(crate::account::network_error)?;
            status = Some(response.status());
            let headers = response.headers().clone();
            let bytes = crate::account::read_response_with_limit(response, LIMIT).await?;
            let summary = parse(&bytes, &session.user_id)?;
            let mut candidate = session.clone();
            let mut has_login_cookie = false;
            if headers.get_all(SET_COOKIE).iter().count() > 64 {
                return Err(malformed());
            }
            for header in headers.get_all(SET_COOKIE) {
                let raw = header.to_str().map_err(|_| malformed())?;
                if raw.len() > 20_480 {
                    return Err(malformed());
                }
                if raw
                    .split_once('=')
                    .is_some_and(|(name, _)| name.trim() == "KuGoo")
                {
                    has_login_cookie = true;
                }
            }
            if has_login_cookie {
                let cookie = WebCookie::received(&headers, crate::account::now_ms()? / 1000)?;
                // Only explicitly shared domain/root cookies can be verified at the
                // independent login service. No host-only cookie is promoted.
                cookie.media_token(crate::account::now_ms()? / 1000)?;
                if cookie.identity()?.user_id != session.user_id {
                    return Err(identity_error());
                }
                candidate.cookie = cookie;
            }
            Ok(WebMembership { summary, candidate })
        }
        .await;
        self.log_upstream_request(
            "web_membership",
            MEMBER_HOST,
            MEMBER_PATH,
            status,
            started,
            0,
            false,
            &result,
        );
        result
    }
}

fn parse(bytes: &[u8], uid: &str) -> Result<MembershipSummary> {
    if bytes.len() > LIMIT {
        return Err(malformed());
    }
    let status: Status = serde_json::from_slice(bytes).map_err(|_| malformed())?;
    if status.errno.is_some_and(|v| v != 0) || status.error_code.is_some_and(|v| v != 0) {
        return Err(error(
            if status.errno == Some(105) && status.error_code == Some(20017) {
                ErrorCode::AuthenticationRequired
            } else {
                ErrorCode::UpstreamError
            },
            "KuGou Web membership request was rejected",
        )
        .with_details(
            serde_json::json!({"platform_code":status.error_code,"platform_errno":status.errno}),
        ));
    }
    let mut role: Role = serde_json::from_slice(bytes).map_err(|_| malformed())?;
    for n in [
        Some(role.role),
        role.producttype,
        role.auto_charge_type,
        role.autostatus,
    ]
    .into_iter()
    .flatten()
    {
        code_number(n)?;
    }
    for d in [
        &mut role.vip_end_time,
        &mut role.raw_vip_end_time,
        &mut role.music_end_time,
        &mut role.auto_charge_time,
    ] {
        date_text(d)?;
    }
    // Website role is a combined product code. Main VIP status is separate from
    // the music package; unrecognized future roles are not classified as inactive.
    let active = match role.role.0 {
        0 | 31 | 33 => Some(false),
        1 | 2 | 11 | 13 => Some(true),
        _ => None,
    };
    summary(
        uid,
        "web_roleinfo",
        active,
        None,
        role.vip_end_time.clone(),
        serde_json::to_value(role).map_err(|_| malformed())?,
    )
}

#[cfg(test)]
mod tests;
