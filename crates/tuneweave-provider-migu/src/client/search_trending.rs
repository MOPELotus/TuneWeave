//! Official PC search-box hot words; no resource identity or popularity is inferred.
use super::*;
use tuneweave_core::{SearchTrendingDetail, SearchTrendingEntry, SearchTrendingList};

pub(crate) const PATH: &str = "/pc/bmw/hot-search/hot-search/v1.0";
const MAX_RESPONSE: u64 = 256 * 1024;
const MAX_WORDS: usize = 200;

#[derive(Deserialize)]
struct Envelope {
    code: String,
    data: Data,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Data {
    hot_word_item_list: Vec<Word>,
}

#[derive(Deserialize)]
struct Word {
    word: Option<String>,
    // This field is only compared with the exact annual-report sentinel by the
    // official consumer. It is not the displayed/searchable keyword or an ID.
    value: Option<serde_json::Value>,
}

fn invalid() -> TuneWeaveError {
    migu_upstream_error("Migu PC search hot words have an invalid response")
}

fn parse(bytes: &[u8], detail: SearchTrendingDetail) -> Result<SearchTrendingList> {
    let reply: Envelope = serde_json::from_slice(bytes).map_err(|_| invalid())?;
    if reply.code != "000000" {
        return Err(migu_upstream_error("Migu PC hot-word request was rejected"));
    }
    // Unlike nonempty input completion, the official consumer directly filters
    // this array. Missing/null data or lists do not mean successful emptiness.
    if reply.data.hot_word_item_list.len() > MAX_WORDS {
        return Err(invalid());
    }
    let mut entries = Vec::new();
    for row in reply.data.hot_word_item_list {
        if row.value.as_ref().and_then(serde_json::Value::as_str) == Some("年度听歌报告") {
            continue;
        }
        let keyword = row.word.ok_or_else(invalid)?;
        if keyword.trim().is_empty()
            || keyword.len() > 2048
            || keyword.chars().any(char::is_control)
        {
            return Err(invalid());
        }
        // The displayed heading is deliberately not actionable in onSelect.
        if keyword == "猜你想搜" {
            continue;
        }
        entries.push(SearchTrendingEntry {
            rank: entries.len() as u32 + 1,
            keyword,
            description: None,
            score: None,
            icon_type: None,
            icon_url: None,
            target_url: None,
            extensions: Extensions::new(),
        });
    }
    Ok(SearchTrendingList {
        detail,
        entries,
        extensions: Extensions::from([
            ("backend".into(), json!("official_pc_search_box")),
            ("rank_scope".into(), json!("searchable_response_order")),
            ("metadata_scope".into(), json!("keywords_only")),
        ]),
    })
}

impl MiguClient {
    pub(crate) async fn pc_search_trending(
        &self,
        detail: SearchTrendingDetail,
    ) -> Result<SearchTrendingList> {
        let started = Instant::now();
        let mut status = None;
        let result = async {
            // vn() uses the SDK's relative production origin, app.c.nf.migu.cn;
            // the absolute app.u origin belongs to nonempty suggestions only.
            let url = self.catalog_endpoint(&format!("https://app.c.nf.migu.cn{PATH}"))?;
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
                read_bounded_response_with_limit(response, "Migu search hot words", MAX_RESPONSE)
                    .await?;
            parse(&bytes, detail)
        }
        .await;
        self.log_upstream_request(
            "search_trending",
            "app.c.nf.migu.cn",
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
