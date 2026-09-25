//! The official web search box: a nonempty key selects suggestions, an empty key trends.
//! Unlike the signed catalogue, the /openapi search-box consumer sends no Secret header.
use super::*;
use tuneweave_core::{
    SearchSuggestion, SearchSuggestionClient, SearchSuggestionList, SearchSuggestionRequest,
    SearchTrendingEntry, SearchTrendingList, SearchTrendingRequest,
};

const ENDPOINT: &str = "https://www.kuwo.cn/openapi/v1/www/search/searchKey";
const PATH: &str = "/openapi/v1/www/search/searchKey";
const RESPONSE_LIMIT: u64 = 256 * 1024;
const MAX_ROWS: usize = 100;
const MAX_RECORD_BYTES: usize = 4096;
const BUDGET: Duration = Duration::from_secs(45);

#[derive(Serialize)]
struct Query<'a> {
    key: &'a str,
    #[serde(rename = "httpsStatus")]
    https_status: u8,
    #[serde(rename = "reqId")]
    request_id: String,
    plat: &'static str,
    from: &'static str,
}

#[derive(Deserialize)]
struct Envelope {
    code: i64,
    success: Option<bool>,
    data: Vec<String>,
}

impl KuwoClient {
    /// Anonymous suggestions from the official web search box. Only `Web` is supported.
    pub async fn search_suggestions(
        &self,
        request: &SearchSuggestionRequest,
    ) -> Result<SearchSuggestionList> {
        require_anonymous(request.account.as_deref())?;
        if request.client != SearchSuggestionClient::Web {
            return Err(kuwo_invalid_request(
                "Kuwo search suggestions support only the web client",
            ));
        }
        let query = request.query.trim();
        if !valid_keyword(query, 128) || request.query.chars().any(char::is_control) {
            return Err(kuwo_invalid_request(
                "Kuwo suggestions require a nonempty query of at most 128 UTF-16 units without control characters",
            ));
        }
        let rows = self.search_keys(query, "search_suggestions").await?;
        let suggestions = rows
            .iter()
            .map(|row| suggestion(row))
            .collect::<Result<_>>()?;
        Ok(SearchSuggestionList {
            query: query.to_owned(),
            client: SearchSuggestionClient::Web,
            suggestions,
            recommendations: Vec::new(),
            extensions: Extensions::from([
                ("backend".into(), json!("web_search_box")),
                ("order_scope".into(), json!("upstream_response_order")),
                ("result_scope".into(), json!("keyword_suggestions")),
                ("recommendations_available".into(), json!(false)),
            ]),
        })
    }

    /// Anonymous hot keywords, in the order displayed by the official web search box.
    /// Both detail modes expose the known keywords; the source supplies no scores or icons.
    pub async fn trending_searches(
        &self,
        request: &SearchTrendingRequest,
    ) -> Result<SearchTrendingList> {
        require_anonymous(request.account.as_deref())?;
        let rows = self.search_keys("", "search_trending").await?;
        let entries = rows
            .into_iter()
            .enumerate()
            .map(|(position, keyword)| {
                if !valid_keyword(&keyword, 512) {
                    return Err(invalid());
                }
                Ok(SearchTrendingEntry {
                    rank: position as u32 + 1,
                    keyword,
                    description: None,
                    score: None,
                    icon_type: None,
                    icon_url: None,
                    target_url: None,
                    extensions: Extensions::new(),
                })
            })
            .collect::<Result<_>>()?;
        Ok(SearchTrendingList {
            detail: request.detail,
            entries,
            extensions: Extensions::from([
                ("backend".into(), json!("web_search_box")),
                ("rank_scope".into(), json!("upstream_response_order")),
                ("metadata_scope".into(), json!("keywords_only")),
            ]),
        })
    }

    async fn search_keys(&self, key: &str, operation: &'static str) -> Result<Vec<String>> {
        let started = Instant::now();
        let mut status = None;
        let outcome = tokio::time::timeout(BUDGET, async {
            // Do not bootstrap, read or update the signed-web session. This endpoint is
            // independent of native login, browser accounts and response cookies.
            let response = self
                .http
                .get(self.web_target(ENDPOINT))
                .header(ACCEPT, "application/json")
                .header(REFERER, WEB_REFERER)
                .query(&Query {
                    key,
                    https_status: 1,
                    request_id: new_request_id(),
                    plat: "web_www",
                    from: "",
                })
                .send()
                .await
                .map_err(kuwo_network_error)?;
            status = Some(response.status());
            if !response.status().is_success() {
                return Err(kuwo_http_error(response.status()));
            }
            let mime = response
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.split(';').next());
            if !mime.is_some_and(|value| value.trim().eq_ignore_ascii_case("application/json")) {
                return Err(invalid());
            }
            let bytes =
                read_bounded_response_with_limit(response, "Kuwo search discovery", RESPONSE_LIMIT)
                    .await?;
            let body: Envelope = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
            if body.code != 200 || body.success == Some(false) || body.data.len() > MAX_ROWS {
                return Err(invalid());
            }
            Ok(body.data)
        })
        .await
        .map_err(|_| {
            TuneWeaveError::new(
                ErrorCode::UpstreamTimeout,
                "Kuwo search discovery exceeded the total time budget",
            )
            .with_platform(Platform::Kuwo)
        })
        .and_then(|result| result);
        self.log_upstream_request(
            operation,
            "www.kuwo.cn",
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

fn require_anonymous(account: Option<&str>) -> Result<()> {
    if account.is_some() {
        return Err(kuwo_invalid_request(
            "Kuwo public search discovery does not accept an account",
        ));
    }
    Ok(())
}

fn valid_keyword(value: &str, max_units: usize) -> bool {
    !value.trim().is_empty()
        && value.encode_utf16().count() <= max_units
        && !value.chars().any(char::is_control)
}

fn suggestion(row: &str) -> Result<SearchSuggestion> {
    if row.len() > MAX_RECORD_BYTES {
        return Err(invalid());
    }
    let mut fields = BTreeMap::new();
    for line in row.lines() {
        let (name, value) = line.split_once('=').ok_or_else(invalid)?;
        if name.is_empty()
            || name.len() > 32
            || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
            || value.chars().any(char::is_control)
            || fields.insert(name, value).is_some()
            || fields.len() > 32
        {
            return Err(invalid());
        }
    }
    let keyword = fields
        .get("RELWORD")
        .filter(|value| valid_keyword(value, 512))
        .ok_or_else(invalid)?;
    let mut extensions = Extensions::new();
    // The consumer only reads RELWORD. TYPE is not a proven SearchKind and SNUM/RNUM
    // are not advertised as resource totals or hotness scores.
    for (field, name) in [
        ("TYPE", "upstream_type"),
        ("SNUM", "upstream_snum"),
        ("RNUM", "upstream_rnum"),
    ] {
        if let Some(value) = fields.get(field) {
            if value.is_empty() || value.len() > 20 || !value.bytes().all(|b| b.is_ascii_digit()) {
                return Err(invalid());
            }
            let number = value.parse::<u64>().map_err(|_| invalid())?;
            extensions.insert(name.into(), json!(number));
        }
    }
    Ok(SearchSuggestion {
        keyword: (*keyword).to_owned(),
        kind: None,
        display_text: None,
        icon_url: None,
        resource: None,
        extensions,
    })
}

fn invalid() -> TuneWeaveError {
    kuwo_upstream_error("Kuwo search discovery returned an invalid response")
}
