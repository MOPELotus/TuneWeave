//! The initial anonymous search placeholder from the official desktop Web header.
use super::*;
use crate::device::KugouDevice;
use tuneweave_core::{SearchDefaultKeyword, SearchDefaultKeywordRequest, SearchKind};

const HOST: &str = "gateway.kugou.com";
const PATH: &str = "/ads.gateway/v1/search_no_focus_word";
// Match JSON.stringify's producer order and sign precisely the bytes sent below.
const BODY: &str = r#"{"userid":0,"plat":103,"m_type":0,"vip_type":0,"own_ads":{}}"#;
const RESPONSE_LIMIT: usize = 128 * 1024;

impl KugouClient {
    /// Returns the first anonymous Web placeholder, or ResourceNotFound for an empty rotation.
    pub async fn default_search_keyword(
        &self,
        request: &SearchDefaultKeywordRequest,
    ) -> Result<SearchDefaultKeyword> {
        if request.account.is_some() {
            return Err(kugou_invalid_media_request(
                "KuGou default search keywords require the anonymous Web client",
            ));
        }
        // No account device registration, credential lookup, or persistent cookies.
        let device = KugouDevice::default().identity().into_web();
        let now = SystemTime::now().duration_since(UNIX_EPOCH).map_err(|_| {
            TuneWeaveError::new(
                ErrorCode::InternalError,
                "KuGou default search keywords require a valid system clock",
            )
            .with_platform(Platform::Kugou)
        })?;
        let query = parameters(&device.mid, now);
        let url = format!("https://{HOST}{PATH}");
        #[cfg(test)]
        let url = self
            .login_test_origin
            .as_ref()
            .map(|origin| origin.join(PATH).unwrap().to_string())
            .unwrap_or(url);
        let started = Instant::now();
        let mut status = None;
        let outcome = async {
            let mut response = self
                .http
                .post(url)
                .header(REFERER, WEB_REFERER)
                .header("user-agent", WEB_USER_AGENT)
                // The producer passes a JSON string to jQuery without overriding its default MIME.
                .header(
                    CONTENT_TYPE,
                    "application/x-www-form-urlencoded; charset=UTF-8",
                )
                .query(&query)
                .body(BODY)
                .send()
                .await
                .map_err(kugou_network_error)?;
            status = Some(response.status());
            if !response.status().is_success() {
                return Err(kugou_http_error(response.status()));
            }
            if response.headers().contains_key("ssa-code") {
                return Err(TuneWeaveError::new(
                    ErrorCode::PermissionDenied,
                    "KuGou default search keywords require additional verification",
                )
                .with_platform(Platform::Kugou));
            }
            let mime = response
                .headers()
                .get(CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.split(';').next())
                .map(str::trim);
            if mime != Some("application/json")
                || response
                    .content_length()
                    .is_some_and(|n| n > RESPONSE_LIMIT as u64)
            {
                return Err(malformed());
            }
            let mut bytes = Vec::new();
            while let Some(chunk) = response.chunk().await.map_err(kugou_network_error)? {
                if bytes.len().saturating_add(chunk.len()) > RESPONSE_LIMIT {
                    return Err(malformed());
                }
                bytes.extend_from_slice(&chunk);
            }
            parse(&bytes)
        }
        .await;
        self.log_upstream_request(
            "search_default",
            HOST,
            PATH,
            status,
            started,
            0,
            false,
            &outcome,
        );
        outcome
    }
}

fn parameters(mid: &str, now: Duration) -> BTreeMap<&'static str, String> {
    let millis = now.as_millis();
    let mut query = BTreeMap::from([
        ("appid", "1014".into()),
        ("clientver", "1000".into()),
        // Header rounds seconds; infSign supplies its separate millisecond uuid.
        ("clienttime", ((millis + 500) / 1000).to_string()),
        ("mid", mid.into()),
        ("dfid", "-".into()),
        ("srcappid", "2919".into()),
        ("uuid", millis.to_string()),
    ]);
    // infSign's H5 MD5 is lowercase, unlike the legacy getInterFacePublic helper.
    let signature = crate::signing::web_signature(&query, BODY.as_bytes()).to_ascii_lowercase();
    query.insert("signature", signature);
    query
}

#[derive(Deserialize)]
struct Envelope {
    status: i64,
    error_code: i64,
}

#[derive(Deserialize)]
struct Payload {
    data: Ads,
}

#[derive(Deserialize)]
struct Ads {
    ads: Vec<Ad>,
}

#[derive(Deserialize)]
struct Ad {
    main_title: String,
    sub_title: Option<String>,
}

fn parse(bytes: &[u8]) -> Result<SearchDefaultKeyword> {
    if bytes.len() > RESPONSE_LIMIT {
        return Err(malformed());
    }
    let envelope: Envelope = serde_json::from_slice(bytes).map_err(|_| malformed())?;
    if envelope.status != 1 || envelope.error_code != 0 {
        return Err(
            kugou_upstream_error("KuGou default search keywords were rejected")
                .with_details(json!({"platform_code":envelope.error_code})),
        );
    }
    let payload: Payload = serde_json::from_slice(bytes).map_err(|_| malformed())?;
    let total = payload.data.ads.len();
    let row = payload.data.ads.into_iter().next().ok_or_else(|| {
        TuneWeaveError::new(
            ErrorCode::ResourceNotFound,
            "KuGou has no anonymous default search keyword",
        )
        .with_platform(Platform::Kugou)
    })?;
    if row.main_title.trim().is_empty()
        || row.main_title.len() > 1024
        || row.main_title.chars().any(char::is_control)
        || row
            .sub_title
            .as_ref()
            .is_some_and(|text| text.len() > 1024 || text.chars().any(char::is_control))
    {
        return Err(malformed());
    }
    let mut extensions = Extensions::from([
        (
            "backend".into(),
            json!("official_web_header_search_no_focus_word"),
        ),
        (
            "result_scope".into(),
            json!("initial_anonymous_placeholder"),
        ),
        ("rotation_total".into(), json!(total)),
    ]);
    if let Some(subtitle) = row.sub_title {
        extensions.insert("subtitle".into(), json!(subtitle));
    }
    // The first row is shown immediately; Enter/button search its main_title as a song.
    // Do not interpret ad links/IDs/images as resources, nor execute or decode title markup.
    Ok(SearchDefaultKeyword {
        keyword: row.main_title.clone(),
        display_text: row.main_title,
        kind: Some(SearchKind::Track),
        image_url: None,
        extensions,
    })
}

fn malformed() -> TuneWeaveError {
    kugou_upstream_error("KuGou default search keywords returned invalid data")
}

#[cfg(test)]
pub(crate) mod tests;
