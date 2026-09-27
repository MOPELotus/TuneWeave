use tuneweave_core::{SearchKind, SearchSuggestion, SearchSuggestionClient, SearchSuggestionList};

use super::*;
use crate::login::SodaCredential;

const ENDPOINT: &str = "https://api.qishui.com/luna/pc/sug";
const MOBILE_ENDPOINT: &str = "https://api.qishui.com/luna/sug";
const MAX_BYTES: usize = 128 * 1024;
const MAX_SUGGESTIONS: usize = 64;

#[derive(Deserialize)]
struct Envelope {
    status_code: Option<i64>,
    status_info: Status,
    // The official empty-result response omits sugs; explicit null is malformed.
    #[serde(default)]
    sugs: Vec<Suggestion>,
}

#[derive(Deserialize)]
struct Status {
    status_code: Option<i64>,
    now: u64,
    now_ts_ms: u64,
}

#[derive(Deserialize)]
struct Suggestion {
    suggestion: String,
    content_type: Option<String>,
}

impl SodaClient {
    pub(crate) async fn pc_search_suggestions(&self, query: &str) -> Result<SearchSuggestionList> {
        self.read_suggestions(query, None, SearchSuggestionClient::Pc)
            .await
            .map(|(list, _)| list)
    }

    pub(crate) async fn mobile_search_suggestions(
        &self,
        query: &str,
    ) -> Result<SearchSuggestionList> {
        self.read_suggestions(query, None, SearchSuggestionClient::Mobile)
            .await
            .map(|(list, _)| list)
    }

    pub(crate) async fn account_search_suggestions(
        &self,
        query: &str,
        credential: &SodaCredential,
        client: SearchSuggestionClient,
    ) -> Result<(SearchSuggestionList, SodaCredential)> {
        if client == SearchSuggestionClient::Web {
            return Err(TuneWeaveError::new(
                ErrorCode::CapabilityNotSupported,
                "Soda Web suggestions have no verified first-party client contract",
            )
            .with_platform(Platform::Soda));
        }
        let (list, updated) = self
            .read_suggestions(query, Some(credential), client)
            .await?;
        let updated = updated.ok_or_else(|| {
            TuneWeaveError::new(
                ErrorCode::InternalError,
                "Soda suggestion credential is missing",
            )
            .with_platform(Platform::Soda)
        })?;
        Ok((list, updated))
    }

    async fn read_suggestions(
        &self,
        query: &str,
        credential: Option<&SodaCredential>,
        client: SearchSuggestionClient,
    ) -> Result<(SearchSuggestionList, Option<SodaCredential>)> {
        let (endpoint, path, app_name, platform, version_name, version_code) = match client {
            SearchSuggestionClient::Pc => (
                ENDPOINT,
                "/luna/pc/sug",
                "luna_pc",
                "windows",
                "2.1.0",
                "20010000",
            ),
            SearchSuggestionClient::Mobile => (
                MOBILE_ENDPOINT,
                "/luna/sug",
                "luna",
                "android",
                "21.1.0",
                "100211030",
            ),
            SearchSuggestionClient::Web => {
                return Err(TuneWeaveError::new(
                    ErrorCode::CapabilityNotSupported,
                    "Soda Web suggestions have no verified first-party client contract",
                )
                .with_platform(Platform::Soda));
            }
        };
        let query = validate_suggestion_query(query)?;
        let user_id = credential
            .map(|c| c.user_id().ok_or_else(authentication_required))
            .transpose()?;
        let started = Instant::now();
        let mut status = None;
        let result = async {
            let mut url = Url::parse(endpoint)
                .map_err(|_| soda_upstream_error("Soda suggestion endpoint is invalid"))?;
            let mut id = rand::random::<[u8; 16]>();
            id[6] = (id[6] & 0x0f) | 0x40;
            id[8] = (id[8] & 0x3f) | 0x80;
            let id = hex::encode(id);
            let search_id = format!(
                "{}-{}-{}-{}-{}",
                &id[..8],
                &id[8..12],
                &id[12..16],
                &id[16..20],
                &id[20..]
            );
            url.query_pairs_mut()
                .append_pair("q", query)
                .append_pair("sug_scene", "main")
                .append_pair("sug_search_id", &search_id)
                .append_pair("aid", SODA_APP_ID)
                .append_pair("app_name", app_name)
                .append_pair("device_platform", platform)
                .append_pair("version_name", version_name)
                .append_pair("version_code", version_code);
            if client == SearchSuggestionClient::Pc {
                url.query_pairs_mut().append_pair("channel", "official");
            }
            if client == SearchSuggestionClient::Pc {
                let device = self.login_device()?;
                url.query_pairs_mut()
                    .append_pair("device_id", &device.device_id)
                    .append_pair("iid", &device.install_id)
                    .append_pair("fp", &device.device_id);
            }
            let mut request = self
                .login_request(reqwest::Method::GET, url)
                .header(reqwest::header::ACCEPT, "application/json");
            if let Some(credential) = credential {
                request = request.header(reqwest::header::COOKIE, credential.cookie_header()?);
            }
            if client == SearchSuggestionClient::Mobile {
                // The official Android interceptor always sends this account-state marker.
                // Anonymous provider calls stay anonymous even when stored aliases exist.
                request = request.header(
                    "x-luna-is-login",
                    if credential.is_some() { "1" } else { "0" },
                );
            }
            let mut response = self.send_login_request(request).await?;
            status = Some(response.status());
            if credential.is_some() && response.status() == StatusCode::UNAUTHORIZED {
                return Err(authentication_required());
            }
            if response.status() != StatusCode::OK {
                return Err(soda_http_error(response.status()));
            }
            if !response
                .headers()
                .get(CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .is_some_and(|v| {
                    v.split(';')
                        .next()
                        .unwrap_or_default()
                        .trim()
                        .eq_ignore_ascii_case("application/json")
                })
            {
                return Err(soda_upstream_error(
                    "Soda suggestions returned an unexpected content type",
                ));
            }
            if response
                .content_length()
                .is_some_and(|n| n > MAX_BYTES as u64)
            {
                return Err(soda_upstream_error(
                    "Soda suggestions exceeded the size limit",
                ));
            }
            let headers = response.headers().clone();
            let mut body = Vec::new();
            while let Some(chunk) = response.chunk().await.map_err(soda_network_error)? {
                if body.len().saturating_add(chunk.len()) > MAX_BYTES {
                    return Err(soda_upstream_error(
                        "Soda suggestions exceeded the size limit",
                    ));
                }
                body.extend_from_slice(&chunk);
            }
            if credential.is_some() {
                check_account_business_status(&body)?;
            }
            let mut list = parse(&body, query)?;
            list.client = client;
            if client == SearchSuggestionClient::Mobile {
                list.extensions.insert(
                    "backend".to_owned(),
                    json!(if credential.is_some() {
                        "official_android_account_sug"
                    } else {
                        "official_android_sug"
                    }),
                );
            }
            if let Some(user_id) = user_id {
                list.extensions.insert(
                    "backend".to_owned(),
                    json!(if client == SearchSuggestionClient::Pc {
                        "official_pc_account_sug"
                    } else {
                        "official_android_account_sug"
                    }),
                );
                list.extensions
                    .insert("authenticated".to_owned(), json!(true));
                list.extensions
                    .insert("source_user_id".to_owned(), json!(user_id));
            }
            let updated = credential
                .map(|c| c.with_response_cookies(&headers))
                .transpose()?;
            Ok((list, updated))
        }
        .await;
        self.log_upstream_request(
            if credential.is_some() {
                "account_search_suggestions"
            } else {
                "search_suggestions"
            },
            "api.qishui.com",
            path,
            status,
            started,
            &result,
        );
        result
    }
}

pub(crate) fn validate_suggestion_query(query: &str) -> Result<&str> {
    let query = query.trim();
    if !valid_text(query) {
        return Err(soda_invalid_request(
            "Soda suggestion query must be 1-1024 bytes without control characters",
        ));
    }
    Ok(query)
}

fn authentication_required() -> TuneWeaveError {
    TuneWeaveError::new(
        ErrorCode::AuthenticationRequired,
        "Soda suggestion session is not authenticated",
    )
    .with_platform(Platform::Soda)
}

fn check_account_business_status(body: &[u8]) -> Result<()> {
    #[derive(Deserialize)]
    struct Business {
        status_code: Option<i64>,
        status_info: Option<Info>,
    }
    #[derive(Deserialize)]
    struct Info {
        status_code: Option<i64>,
    }
    let response: Business = serde_json::from_slice(body)
        .map_err(|_| soda_upstream_error("Soda suggestions returned malformed data"))?;
    let codes = [
        response.status_code,
        response.status_info.and_then(|s| s.status_code),
    ];
    if codes.contains(&Some(1_000_016)) {
        return Err(authentication_required());
    }
    if codes.into_iter().flatten().any(|code| code != 0) {
        return Err(soda_upstream_error("Soda rejected the suggestion request"));
    }
    Ok(())
}

fn valid_text(value: &str) -> bool {
    !value.is_empty() && value.len() <= 1024 && !value.chars().any(char::is_control)
}

fn parse(body: &[u8], query: &str) -> Result<SearchSuggestionList> {
    let response: Envelope = serde_json::from_slice(body)
        .map_err(|_| soda_upstream_error("Soda suggestions returned malformed data"))?;
    if [response.status_code, response.status_info.status_code]
        .into_iter()
        .flatten()
        .any(|code| code != 0)
    {
        return Err(soda_upstream_error("Soda rejected the suggestion request"));
    }
    if response.status_info.now == 0
        || response.status_info.now_ts_ms / 1000 != response.status_info.now
        || response.sugs.len() > MAX_SUGGESTIONS
    {
        return Err(soda_upstream_error(
            "Soda suggestions returned invalid status or size",
        ));
    }
    let suggestions = response
        .sugs
        .into_iter()
        .map(|s| {
            let keyword = s.suggestion.trim();
            if !valid_text(keyword)
                || s.content_type
                    .as_ref()
                    .is_some_and(|v| v.len() > 64 || v.chars().any(char::is_control))
            {
                return Err(soda_upstream_error("Soda returned an invalid suggestion"));
            }
            let kind = match s.content_type.as_deref() {
                Some("track") => Some(SearchKind::Track),
                Some("album") => Some(SearchKind::Album),
                Some("artist") => Some(SearchKind::Artist),
                Some("playlist") => Some(SearchKind::Playlist),
                _ => None,
            };
            Ok(SearchSuggestion {
                keyword: keyword.to_owned(),
                kind,
                // Entity hints contain only IDs, not complete catalogue resources.
                display_text: None,
                icon_url: None,
                resource: None,
                extensions: Extensions::new(),
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(SearchSuggestionList {
        query: query.to_owned(),
        client: SearchSuggestionClient::Pc,
        suggestions,
        recommendations: Vec::new(),
        extensions: Extensions::from([
            ("backend".to_owned(), json!("official_pc_sug")),
            ("authenticated".to_owned(), json!(false)),
        ]),
    })
}

#[cfg(test)]
mod account_tests;
#[cfg(test)]
mod tests;
