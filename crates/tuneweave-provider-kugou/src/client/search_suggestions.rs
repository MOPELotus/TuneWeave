//! Anonymous keyword suggestions displayed by the official desktop web header.
use super::*;
use serde::de::IgnoredAny;
use tuneweave_core::{
    SearchKind, SearchSuggestion, SearchSuggestionClient, SearchSuggestionList,
    SearchSuggestionRequest,
};

const HOST: &str = "searchtip.kugou.com";
const PATH: &str = "/getSearchTip";
const CALLBACK: &str = "tuneweaveKugouSearchTip";
const RESPONSE_LIMIT: usize = 128 * 1024;

impl KugouClient {
    /// Returns anonymous Web search words, with no playable resource or account state.
    pub async fn search_suggestions(
        &self,
        request: &SearchSuggestionRequest,
    ) -> Result<SearchSuggestionList> {
        if request.account.is_some() || request.client != SearchSuggestionClient::Web {
            return Err(kugou_invalid_media_request(
                "KuGou search suggestions require the anonymous Web client",
            ));
        }
        let query = request.query.trim();
        if query.is_empty()
            || request.query.len() > 512
            || request.query.chars().any(char::is_control)
        {
            return Err(kugou_invalid_media_request(
                "KuGou suggestions require a nonempty query of at most 512 bytes without controls",
            ));
        }
        // The header's htmlEncode runs before encodeURIComponent. Ampersands are
        // intentionally not changed by that producer; this is not an HTML renderer.
        let keyword = query
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('\'', "&#39;")
            .replace('"', "&quot;")
            .replace(' ', "&nbsp;");
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
                .get(url)
                .header(REFERER, WEB_REFERER)
                .header("user-agent", WEB_USER_AGENT)
                .query(&[
                    ("MusicTipCount", "5"),
                    ("MVTipCount", "2"),
                    ("albumcount", "2"),
                    ("keyword", keyword.as_str()),
                    ("callback", CALLBACK),
                ])
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
                    "KuGou search suggestions require additional verification",
                )
                .with_platform(Platform::Kugou));
            }
            let mime = response
                .headers()
                .get(CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.split(';').next())
                .map(str::trim);
            if !matches!(
                mime,
                Some("text/plain" | "application/javascript" | "text/javascript")
            ) || response
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
            parse(&bytes, query)
        }
        .await;
        self.log_upstream_request(
            "search_suggestions",
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

#[derive(Deserialize)]
struct Envelope<T> {
    status: i64,
    error_code: i64,
    #[serde(rename = "ErrorCode")]
    legacy_error_code: Option<i64>,
    data: T,
}

#[derive(Deserialize)]
struct Group {
    #[serde(rename = "RecordDatas")]
    rows: Vec<Row>,
    #[serde(rename = "RecordCount")]
    count: u32,
    #[serde(rename = "LableName")]
    label: String,
}

#[derive(Deserialize)]
struct Row {
    #[serde(rename = "HintInfo")]
    hint: String,
}

fn parse(bytes: &[u8], query: &str) -> Result<SearchSuggestionList> {
    if bytes.len() > RESPONSE_LIMIT {
        return Err(malformed());
    }
    let text = std::str::from_utf8(bytes).map_err(|_| malformed())?.trim();
    let text = text.strip_suffix(';').unwrap_or(text).trim_end();
    let json = text
        .strip_prefix(CALLBACK)
        .and_then(|s| s.strip_prefix('('))
        .and_then(|s| s.strip_suffix(')'))
        .ok_or_else(malformed)?;
    // Parse data only after the business envelope, without evaluating JSONP code.
    let envelope: Envelope<IgnoredAny> = serde_json::from_str(json).map_err(|_| malformed())?;
    if envelope.status != 1
        || envelope.error_code != 0
        || envelope.legacy_error_code.is_some_and(|code| code != 0)
    {
        return Err(
            kugou_upstream_error("KuGou search suggestions were rejected")
                .with_details(json!({"platform_code":envelope.error_code})),
        );
    }
    let envelope: Envelope<[Group; 3]> = serde_json::from_str(json).map_err(|_| malformed())?;
    let mut suggestions = Vec::new();
    for (group, (label, limit, kind)) in envelope.data.into_iter().zip([
        ("", 5, Some(SearchKind::Track)),
        ("MV", 2, Some(SearchKind::Mv)),
        ("专辑", 2, None),
    ]) {
        if group.label != label || group.rows.len() != group.count as usize || group.count > limit {
            return Err(malformed());
        }
        for row in group.rows {
            let keyword = row.hint.trim();
            if keyword.is_empty() || row.hint.len() > 1024 || row.hint.chars().any(char::is_control)
            {
                return Err(malformed());
            }
            // The producer requests album hints but its current header only consumes
            // song and MV groups. Names and shortcut metadata are not resource IDs.
            if let Some(kind) = kind {
                suggestions.push(SearchSuggestion {
                    keyword: keyword.to_owned(),
                    kind: Some(kind),
                    display_text: Some(row.hint),
                    icon_url: None,
                    resource: None,
                    extensions: Extensions::new(),
                });
            }
        }
    }
    Ok(SearchSuggestionList {
        query: query.to_owned(),
        client: SearchSuggestionClient::Web,
        suggestions,
        recommendations: Vec::new(),
        extensions: Extensions::from([
            ("backend".into(), json!("official_web_header_search_tip")),
            ("result_scope".into(), json!("song_and_mv_keywords")),
            ("order_scope".into(), json!("upstream_group_and_row_order")),
        ]),
    })
}

fn malformed() -> TuneWeaveError {
    kugou_upstream_error("KuGou search suggestions returned invalid data")
}

#[cfg(test)]
pub(crate) mod tests;
