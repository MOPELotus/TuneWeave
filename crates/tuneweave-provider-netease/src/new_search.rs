use super::*;

impl NeteaseProvider {
    pub(super) async fn search_modern(&self, query: &SearchQuery) -> Result<Page<SearchItem>> {
        let keyword = query.query.trim();
        if keyword.is_empty()
            || keyword.len() > 4096
            || !matches!(query.kind, SearchKind::Track | SearchKind::Lyric)
        {
            return Err(TuneWeaveError::invalid_request(
                "NetEase modern search requires a track or lyric query of 1 to 4096 bytes",
            )
            .with_platform(Platform::Netease));
        }
        let limit = query.limit.clamp(1, 100);
        let path = if query.kind == SearchKind::Lyric {
            "/api/search/resource/lyric"
        } else {
            "/api/search/song/list/page"
        };
        let mut data = json!({"keyword":keyword,"limit":limit,"offset":query.offset});
        if query.kind == SearchKind::Track {
            data["scene"] = json!("normal");
        }
        let client = self.client_for(query.account.as_deref())?;
        let response = client.request_weapi(path, data).await?;
        ensure_success(&response.body)?;
        map_modern_search(query, limit, path, response.body)
    }
}
fn map_modern_search(
    query: &SearchQuery,
    limit: u32,
    path: &str,
    body: Value,
) -> Result<Page<SearchItem>> {
    let data = body
        .get("data")
        .and_then(Value::as_object)
        .ok_or_else(|| modern_error("missing data"))?;
    let rows = data
        .get("resources")
        .and_then(Value::as_array)
        .ok_or_else(|| modern_error("missing resource list"))?;
    if rows.len() > limit as usize {
        return Err(modern_error("resource list exceeded its requested limit"));
    }
    let total = data.get("totalCount").and_then(json_u64);
    let upstream_more = data
        .get(if query.kind == SearchKind::Lyric {
            "more"
        } else {
            "hasMore"
        })
        .and_then(json_bool)
        .ok_or_else(|| modern_error("missing continuation flag"))?;
    let count = u32::try_from(rows.len()).map_err(|_| modern_error("resource count overflow"))?;
    let next = query
        .offset
        .checked_add(count)
        .ok_or_else(|| modern_error("offset overflow"))?;
    // The lyric service's `more` describes its UI card, not offset exhaustion:
    // live offset 0/2/4 returns distinct songs while `more` stays false.
    let has_more = if query.kind == SearchKind::Lyric {
        total.map_or(upstream_more, |total| u64::from(next) < total)
    } else {
        upstream_more
    };
    if has_more && count == 0 {
        return Err(modern_error("empty continuing resource page"));
    }
    let items = rows
        .iter()
        .map(|row| {
            let song = if query.kind == SearchKind::Lyric {
                row.get("song")
            } else {
                row.pointer("/baseInfo/simpleSongData")
            }
            .ok_or_else(|| modern_error("resource omitted its song"))?;
            let song: Song = parse_body(song.clone())?;
            let mut track = map_song(song, None)?;
            if query.kind == SearchKind::Lyric {
                for name in ["matchedLyric", "highlightText"] {
                    if let Some(value) = row.get(name).and_then(Value::as_str) {
                        track.extensions.insert(name.to_owned(), json!(value));
                    }
                }
            }
            Ok(SearchItem::Track(track))
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(Page {
        items,
        pagination: PageMeta {
            limit,
            offset: query.offset,
            total,
            has_more,
            next_offset: has_more.then_some(next),
            extensions: Extensions::from([
                ("variant".into(), json!("modern")),
                ("request_path".into(), json!(path)),
            ]),
        },
    })
}
fn modern_error(detail: &str) -> TuneWeaveError {
    TuneWeaveError::new(
        ErrorCode::UpstreamError,
        format!("NetEase modern search {detail}"),
    )
    .with_platform(Platform::Netease)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    #[ignore = "requires live NetEase access"]
    async fn live_modern_song_lyric_suggestions_and_banner() {
        let provider = NeteaseProvider::new(NeteaseConfig::default()).unwrap();
        for kind in [SearchKind::Track, SearchKind::Lyric] {
            let mut query = SearchQuery::tracks("晴天", 2, 0);
            query.kind = kind;
            query.variant = SearchVariant::Modern;
            let first = provider.search_catalog(&query).await.unwrap();
            assert_eq!(first.items.len(), 2);
            assert!(first.pagination.has_more);
            query.offset = first.pagination.next_offset.unwrap();
            let second = provider.search_catalog(&query).await.unwrap();
            assert_eq!(second.items.len(), 2);
        }
        let suggestions = provider
            .search_suggestions(&SearchSuggestionRequest {
                query: "周杰伦".into(),
                client: SearchSuggestionClient::Mobile,
                account: None,
            })
            .await
            .unwrap();
        assert!(!suggestions.suggestions.is_empty());
        let banners = provider
            .banners(&BannerListRequest::new(BannerClient::Android))
            .await
            .unwrap();
        assert!(!banners.is_empty());
    }
    #[test]
    fn malformed_modern_pages_do_not_become_successful_empty_results() {
        let query = SearchQuery::tracks("test", 2, 0);
        for data in [
            json!({}),
            json!({"resources":[],"hasMore":true}),
            json!({"resources":[{}],"hasMore":false}),
        ] {
            assert!(
                map_modern_search(
                    &query,
                    2,
                    "/api/search/song/list/page",
                    json!({"code":200,"data":data})
                )
                .is_err()
            );
        }
    }
}
