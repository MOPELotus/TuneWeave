//! Installed-client membership protocols. Membership is not a media authorization.
use super::*;
use tuneweave_core::{Extensions, MembershipSummary, ResourceRef};

pub(crate) const LIMIT: usize = 1_048_576;
const STANDARD_PATH: &str = "/vipos/v1/vip_info/detail/query";
const CONCEPT_PATH: &str = "/v1/get_union_vip";

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(try_from = "Value", into = "u64")]
pub(crate) struct Number(pub(crate) u64);
impl TryFrom<Value> for Number {
    type Error = &'static str;
    fn try_from(value: Value) -> std::result::Result<Self, Self::Error> {
        let n = match value {
            Value::Number(n) => n.as_u64(),
            Value::String(s) => s.parse::<u64>().ok().filter(|n| n.to_string() == s),
            _ => None,
        };
        n.map(Self).ok_or("expected a canonical unsigned integer")
    }
}
impl From<Number> for u64 {
    fn from(n: Number) -> Self {
        n.0
    }
}

#[derive(Deserialize)]
struct Envelope {
    status: i64,
    errcode: Option<i64>,
    error_code: Option<i64>,
    data: Option<Box<serde_json::value::RawValue>>,
}

#[derive(Deserialize, Serialize)]
struct Product {
    busi_type: String,
    product_type: String,
    userid: Number,
    is_vip: Option<Number>,
    is_paid_vip: Option<Number>,
    y_type: Option<Number>,
    purchased_type: Option<Number>,
    vip_begin_time: Option<String>,
    vip_end_time: Option<String>,
    vip_clearday: Option<String>,
    paid_vip_expire_time: Option<String>,
}

#[derive(Deserialize, Serialize)]
struct UnionProduct {
    busi_type: String,
    product_type: String,
    vip_valid: Option<Number>,
    vip_end_time: Option<String>,
}

// Only reviewed fields are deserialized/serialized. Unknown upstream credentials,
// phone numbers and payment records never become public extensions.
#[derive(Deserialize, Serialize)]
struct Wire {
    userid: Number,
    vip_type: Number,
    is_vip: Option<Number>,
    y_type: Option<Number>,
    vip_begin_time: Option<String>,
    vip_end_time: Option<String>,
    vip_clearday: Option<String>,
    svip_end_time: Option<String>,
    s_vip_end_time: Option<String>,
    svip_level: Option<Number>,
    svip_score: Option<Number>,
    user_type: Option<Number>,
    user_y_type: Option<Number>,
    su_vip_begin_time: Option<String>,
    su_vip_end_time: Option<String>,
    su_vip_clearday: Option<String>,
    su_vip_y_endtime: Option<String>,
    m_type: Option<Number>,
    m_begin_time: Option<String>,
    m_end_time: Option<String>,
    m_clearday: Option<String>,
    m_reset_time: Option<String>,
    roam_type: Option<Number>,
    roam_begin_time: Option<String>,
    roam_end_time: Option<String>,
    listen_begin_time: Option<String>,
    listen_end_time: Option<String>,
    busi_vip: Option<Vec<Product>>,
    union_vipinfo: Option<UnionProduct>,
}

impl KugouClient {
    pub(crate) async fn native_membership(
        &self,
        session: &NativeSession,
    ) -> Result<MembershipSummary> {
        validate_session(session)?;
        let seconds = now_ms()? / 1000;
        let params = parameters(session, seconds)?;
        let (host, path) = match session.client {
            KugouLoginClient::Standard => (HOST, STANDARD_PATH),
            KugouLoginClient::Concept => ("kugouvip.kugou.com", CONCEPT_PATH),
            KugouLoginClient::Web => return Err(malformed()),
        };
        let url = format!("https://{host}{path}");
        #[cfg(test)]
        let url = self
            .login_test_origin
            .as_ref()
            .map(|o| o.join(path).unwrap().to_string())
            .unwrap_or(url);
        let started = Instant::now();
        let mut status = None;
        let result = async {
            let mut request = self
                .http
                .get(url)
                .query(&params)
                .header("accept", "application/json")
                .header(
                    "user-agent",
                    "Android15-1070-11083-46-0-DiscoveryDRADProtocol-wifi",
                )
                .header("mid", &session.device.mid)
                .header("dfid", session.device.dfid())
                .header("clienttime", seconds);
            if session.client == KugouLoginClient::Standard {
                request = request.header("KG-TID", "524");
            }
            let response = request.send().await.map_err(network_error)?;
            status = Some(response.status());
            parse(
                &read_response_with_limit(response, LIMIT).await?,
                session.client,
                &session.user_id,
            )
        }
        .await;
        self.log_upstream_request(
            "account_membership",
            host,
            path,
            status,
            started,
            0,
            false,
            &result,
        );
        result
    }
}

fn parameters(session: &NativeSession, seconds: u64) -> Result<BTreeMap<&'static str, String>> {
    validate_session(session)?;
    let mut p = BTreeMap::from([
        ("appid", session.client.appid().to_string()),
        ("clientver", session.client.clientver().to_string()),
        ("clienttime", seconds.to_string()),
        ("mid", session.device.mid.clone()),
        ("uuid", "-".into()),
        ("dfid", session.device.dfid().into()),
    ]);
    let signature = match session.client {
        KugouLoginClient::Standard => {
            p.extend([
                ("clientappid", session.client.appid().to_string()),
                ("kugouid", session.user_id.clone()),
                ("clienttoken", session.token.clone()),
                ("infotype", "1".into()),
                ("level_flag", "1".into()),
                ("tone_flag", "1".into()),
                ("tone_packet", "1".into()),
                ("nameplate_flag", "1".into()),
                ("fancy_flag", "1".into()),
            ]);
            android_signature(&p, b"")
        }
        KugouLoginClient::Concept => {
            p.extend([
                ("userid", session.user_id.clone()),
                ("token", session.token.clone()),
                ("busi_type", "concept".into()),
                ("opt_product_types", "dvip,qvip,wvip".into()),
            ]);
            concept_signature(&p, b"")
        }
        KugouLoginClient::Web => return Err(malformed()),
    };
    p.insert("signature", signature);
    Ok(p)
}

fn parse(bytes: &[u8], client: KugouLoginClient, uid: &str) -> Result<MembershipSummary> {
    if bytes.len() > LIMIT {
        return Err(malformed());
    }
    let envelope: Envelope = serde_json::from_slice(bytes).map_err(|_| malformed())?;
    let code = match client {
        KugouLoginClient::Standard if envelope.error_code.is_none() => envelope.errcode,
        KugouLoginClient::Concept if envelope.errcode.is_none() => envelope.error_code,
        _ => return Err(malformed()),
    };
    if envelope.status != 1 || code.is_some_and(|c| c != 0) {
        return Err(error(
            if envelope.status == 0 && code == Some(20017) {
                ErrorCode::AuthenticationRequired
            } else {
                ErrorCode::UpstreamError
            },
            "KuGou membership request was rejected",
        )
        .with_details(json!({"platform_code":code})));
    }
    let mut wire: Wire = serde_json::from_str(envelope.data.ok_or_else(malformed)?.get())
        .map_err(|_| malformed())?;
    if wire.userid.0.to_string() != uid {
        return Err(identity_error());
    }
    // The two installed clients have different product schemas. A cross-client
    // payload is not accepted as proof that the requested membership was returned.
    if (client == KugouLoginClient::Standard && wire.busi_vip.is_some())
        || (client == KugouLoginClient::Concept && wire.union_vipinfo.is_some())
    {
        return Err(malformed());
    }
    for n in [
        Some(wire.vip_type),
        wire.is_vip,
        wire.y_type,
        wire.svip_level,
        wire.svip_score,
        wire.user_type,
        wire.user_y_type,
        wire.m_type,
        wire.roam_type,
    ]
    .into_iter()
    .flatten()
    {
        code_number(n)?;
    }
    for date in [
        &mut wire.vip_begin_time,
        &mut wire.vip_end_time,
        &mut wire.vip_clearday,
        &mut wire.svip_end_time,
        &mut wire.s_vip_end_time,
        &mut wire.su_vip_begin_time,
        &mut wire.su_vip_end_time,
        &mut wire.su_vip_clearday,
        &mut wire.su_vip_y_endtime,
        &mut wire.m_begin_time,
        &mut wire.m_end_time,
        &mut wire.m_clearday,
        &mut wire.m_reset_time,
        &mut wire.roam_begin_time,
        &mut wire.roam_end_time,
        &mut wire.listen_begin_time,
        &mut wire.listen_end_time,
    ] {
        date_text(date)?;
    }
    if let Some(products) = &mut wire.busi_vip {
        if products.len() > 64 {
            return Err(malformed());
        }
        let mut seen = std::collections::BTreeSet::new();
        for product in products {
            if product.userid.0.to_string() != uid {
                return Err(identity_error());
            }
            identifier(&product.busi_type)?;
            identifier(&product.product_type)?;
            if !seen.insert((product.busi_type.clone(), product.product_type.clone())) {
                return Err(malformed());
            }
            for n in [
                product.is_vip,
                product.is_paid_vip,
                product.y_type,
                product.purchased_type,
            ]
            .into_iter()
            .flatten()
            {
                code_number(n)?;
            }
            for date in [
                &mut product.vip_begin_time,
                &mut product.vip_end_time,
                &mut product.vip_clearday,
                &mut product.paid_vip_expire_time,
            ] {
                date_text(date)?;
            }
        }
    }
    if let Some(p) = &mut wire.union_vipinfo {
        identifier(&p.busi_type)?;
        identifier(&p.product_type)?;
        if let Some(n) = p.vip_valid {
            code_number(n)?;
        }
        date_text(&mut p.vip_end_time)?;
    }
    let active = main_active(wire.vip_type.0);
    let expires = wire.vip_end_time.clone();
    let level = if client == KugouLoginClient::Standard {
        wire.svip_level.map(code_number).transpose()?
    } else {
        None
    };
    let backend = match client {
        KugouLoginClient::Standard => "standard_vip_detail_v3",
        KugouLoginClient::Concept => "concept_union_vip",
        _ => return Err(malformed()),
    };
    summary(
        uid,
        backend,
        active,
        level,
        expires,
        serde_json::to_value(wire).map_err(|_| malformed())?,
    )
}

// Both current installed clients exclude 0/5. Restrict positive interpretation
// to their documented ordinary/trial/luxury codes; future/sentinel codes stay unknown.
fn main_active(code: u64) -> Option<bool> {
    match code {
        0 | 5 => Some(false),
        1..=4 | 6 => Some(true),
        _ => None,
    }
}
pub(crate) fn code_number(n: Number) -> Result<u32> {
    u32::try_from(n.0).map_err(|_| malformed())
}
fn identifier(v: &str) -> Result<()> {
    if v.is_empty()
        || v.len() > 32
        || !v
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
    {
        return Err(malformed());
    }
    Ok(())
}
pub(crate) fn date_text(v: &mut Option<String>) -> Result<()> {
    if let Some(s) = v {
        if s.len() > 128 || s.chars().any(char::is_control) || s.trim() != s {
            return Err(malformed());
        }
        if s.is_empty() {
            *v = None;
        }
    }
    Ok(())
}
pub(crate) fn identity_error() -> TuneWeaveError {
    error(
        ErrorCode::AuthenticationRequired,
        "KuGou membership returned a different account",
    )
}
pub(crate) fn summary(
    uid: &str,
    backend: &'static str,
    active: Option<bool>,
    level: Option<u32>,
    expires_at: Option<String>,
    details: Value,
) -> Result<MembershipSummary> {
    Ok(MembershipSummary {
        user_ref: Some(ResourceRef::new(Platform::Kugou, uid).map_err(|_| malformed())?),
        level,
        active,
        annual_count: None,
        expires_at,
        icon_url: None,
        extensions: Extensions::from([
            ("backend".into(), json!(backend)),
            ("source_user_id".into(), json!(uid)),
            ("summary_scope".into(), json!("main_membership")),
            (
                "level_scope".into(),
                if backend == "standard_vip_detail_v3" {
                    json!("super_membership")
                } else {
                    Value::Null
                },
            ),
            ("date_format".into(), json!("upstream_text")),
            ("date_timezone".into(), Value::Null),
            ("membership_details".into(), details),
        ]),
    })
}

#[cfg(test)]
mod tests;
