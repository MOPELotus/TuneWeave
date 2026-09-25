//! Official PC input completion: ordered search words, not resolved resources.
use super::*;
use tuneweave_core::{SearchKind, SearchSuggestion, SearchSuggestionClient, SearchSuggestionList};

pub(crate) const PATH: &str = "/pc/resource/content/tone_search_suggest/v1.0";
const MAX_RESPONSE: u64 = 1024 * 1024;
const MAX_SUGGESTIONS: usize = 200;

pub(crate) fn validate_query(query: &str) -> Result<&str> {
    let query = query.trim();
    if query.is_empty() || query.len() > 1024 || query.chars().any(char::is_control) {
        return Err(migu_invalid_request(
            "Migu PC search suggestions require a nonempty query of at most 1024 bytes",
        ));
    }
    // The official Ut producer only escapes '#' and '&'. Literal '+' and '%'
    // can be reinterpreted by its legacy query decoder; do not invent escaping.
    if query.contains(['+', '%']) {
        return Err(migu_invalid_request(
            "Migu PC search suggestions do not support literal plus or percent signs",
        ));
    }
    Ok(query)
}

#[derive(Deserialize)]
struct Envelope {
    code: String,
    data: Option<Data>,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Data {
    singer_list: Option<Vec<Word>>,
    song_list: Option<Vec<Word>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Word {
    song_name: Option<String>,
    singer_name: Option<String>,
}

fn invalid() -> TuneWeaveError {
    migu_upstream_error("Migu PC search suggestions have an invalid response")
}

fn parse(bytes: &[u8], query: &str) -> Result<SearchSuggestionList> {
    let reply: Envelope = serde_json::from_slice(bytes).map_err(|_| invalid())?;
    if reply.code != "000000" {
        return Err(migu_upstream_error(
            "Migu PC search suggestion request was rejected",
        ));
    }
    // The official consumer explicitly uses optional data/list access and || [].
    // This applies only after the exact successful business code.
    let data = reply.data.unwrap_or_default();
    let singers = data.singer_list.unwrap_or_default();
    let songs = data.song_list.unwrap_or_default();
    if singers.len().saturating_add(songs.len()) > MAX_SUGGESTIONS {
        return Err(invalid());
    }
    let mut suggestions = Vec::with_capacity(singers.len() + songs.len());
    for (rows, kind, section) in [
        (singers, SearchKind::Artist, "singerList"),
        (songs, SearchKind::Track, "songList"),
    ] {
        for row in rows {
            // Match the real selection consumer: songName || singerName. IDs,
            // highlight HTML, artwork and arbitrary response fields are unused.
            let keyword = row
                .song_name
                .filter(|s| !s.is_empty())
                .or(row.singer_name)
                .filter(|s| {
                    !s.trim().is_empty() && s.len() <= 2048 && !s.chars().any(char::is_control)
                })
                .ok_or_else(invalid)?;
            suggestions.push(SearchSuggestion {
                keyword,
                kind: Some(kind),
                display_text: None,
                icon_url: None,
                resource: None,
                extensions: Extensions::from([("source_section".into(), json!(section))]),
            });
        }
    }
    Ok(SearchSuggestionList {
        query: query.into(),
        client: SearchSuggestionClient::Pc,
        suggestions,
        recommendations: Vec::new(),
        extensions: Extensions::from([
            ("backend".into(), json!("official_pc_search_input")),
            ("resource_resolution".into(), json!("keyword_only")),
        ]),
    })
}

impl MiguClient {
    pub(crate) async fn pc_search_suggestions(&self, query: &str) -> Result<SearchSuggestionList> {
        let started = Instant::now();
        let mut status = None;
        let result = async {
            let mut url = self.catalog_endpoint(&format!("https://app.u.nf.migu.cn{PATH}"))?;
            // Exact Ut function from the PC producer, followed by browser URL
            // serialization. Using form-url-encoding here would encode '%' again
            // and would incorrectly change the platform's double-escape contract.
            let escaped = query.replace('#', "%2523").replace('&', "%2526");
            url.set_query(Some(&format!("text={escaped}")));
            let response = self
                .http
                .get(url)
                .header(ACCEPT, "application/json")
                .send()
                .await
                .map_err(migu_network_error)?;
            status = Some(response.status());
            if response.status().is_success()
                && !response
                    .headers()
                    .get(reqwest::header::CONTENT_TYPE)
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.split(';').next())
                    .is_some_and(|v| v.trim().eq_ignore_ascii_case("application/json"))
            {
                return Err(invalid());
            }
            let bytes =
                read_bounded_response_with_limit(response, "Migu search suggestions", MAX_RESPONSE)
                    .await?;
            parse(&bytes, query)
        }
        .await;
        self.log_upstream_request(
            "search_suggestions",
            "app.u.nf.migu.cn",
            PATH,
            status,
            started,
            &result,
        );
        result
    }
}

#[cfg(test)]
pub(crate) mod tests;
