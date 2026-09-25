use super::*;

const FOLLOWING_ARTISTS_BUDGET: std::time::Duration = std::time::Duration::from_secs(45);
const FOLLOWING_ARTISTS_MAX_PAGES: usize = 128;
const FOLLOWING_ARTISTS_MAX_ITEMS: usize = 10_000;

impl SodaProvider {
    pub(super) async fn read_account_following_artists(
        &self,
        request: &PageRequest,
    ) -> Result<Page<Artist>> {
        if !(1..=100).contains(&request.limit)
            || request.offset > FOLLOWING_ARTISTS_MAX_ITEMS as u32
            || request.offset.checked_add(request.limit).is_none()
        {
            return Err(soda_invalid_request(
                "Soda following-artists page requires limit 1-100 and offset at most 10000",
            ));
        }

        let alias = request.account.as_deref().unwrap_or("default");
        let (mut source, mut stored) = self
            .selected_credential(alias)?
            .ok_or_else(soda_authentication_required)?;
        source.user_id().ok_or_else(soda_authentication_required)?;
        let mut sources = vec![source.clone()];
        let mut pending = PendingCredentialUpdate::new(self.response_credential.clone());

        let outcome = tokio::time::timeout(FOLLOWING_ARTISTS_BUDGET, async {
            let verified = self.client.account(alias, &source).await;
            self.ensure_account_snapshot_current(Some(alias), &source)?;
            let verified = verified?;
            self.advance_library_credential(&mut source, &mut stored, verified.credential)?;
            sources.push(source.clone());
            let user_id = source
                .user_id()
                .ok_or_else(soda_authentication_required)?
                .to_owned();

            let mut artists = Vec::new();
            let mut artist_ids = BTreeSet::new();
            let mut requested_cursors = BTreeSet::new();
            let mut cursor: Option<String> = None;
            let mut total_num = None;
            let mut upstream_pages = 0usize;
            let mut reached_end = false;

            for _ in 0..FOLLOWING_ARTISTS_MAX_PAGES {
                self.ensure_account_snapshot_current(Some(alias), &source)?;
                let response = self
                    .client
                    .account_artist_collection_page(cursor.as_deref(), &source)
                    .await;
                self.ensure_account_snapshot_current(Some(alias), &source)?;
                let page = response?;

                if total_num.is_some_and(|total| total != page.total_num)
                    || page.total_num > FOLLOWING_ARTISTS_MAX_ITEMS as u64
                {
                    return Err(soda_upstream_error(
                        "Soda following-artists total changed or exceeded its bounded snapshot",
                    ));
                }
                total_num = Some(page.total_num);
                self.advance_library_credential(&mut source, &mut stored, page.credential)?;
                sources.push(source.clone());
                upstream_pages += 1;

                for mut artist in page.artists {
                    if !artist_ids.insert(artist.id.clone()) {
                        return Err(soda_upstream_error(
                            "Soda following-artists pages repeated an artist",
                        ));
                    }
                    artist
                        .extensions
                        .insert("source_user_id".to_owned(), json!(user_id));
                    artist
                        .extensions
                        .insert("authenticated".to_owned(), json!(true));
                    artists.push(artist);
                }
                if artists.len() > FOLLOWING_ARTISTS_MAX_ITEMS
                    || artists.len() as u64 > page.total_num
                {
                    return Err(soda_upstream_error(
                        "Soda following-artists pages exceeded or contradicted their total",
                    ));
                }

                if !page.has_more {
                    if artists.len() as u64 != page.total_num {
                        return Err(soda_upstream_error(
                            "Soda following-artists list ended before its reported total",
                        ));
                    }
                    reached_end = true;
                    break;
                }

                let next = page.next_cursor.ok_or_else(|| {
                    soda_upstream_error("Soda following-artists page omitted its next cursor")
                })?;
                if cursor.as_deref() == Some(next.as_str())
                    || !requested_cursors.insert(next.clone())
                {
                    return Err(soda_upstream_error(
                        "Soda following-artists pagination repeated a cursor",
                    ));
                }
                cursor = Some(next);
            }

            if !reached_end {
                return Err(soda_upstream_error(
                    "Soda following-artists list exceeded its page budget",
                ));
            }

            for artist in &mut artists {
                artist.extensions.insert(
                    "backend".to_owned(),
                    json!("official_android_account_following_artists"),
                );
            }
            crate::account::reject_secrets(
                &serde_json::to_value(&artists).map_err(|_| {
                    soda_upstream_error("Soda following-artists metadata could not be encoded")
                })?,
                &sources,
            )?;
            self.ensure_account_snapshot_current(Some(alias), &source)?;

            let total = total_num.unwrap_or_default();
            let items = artists
                .into_iter()
                .skip(request.offset as usize)
                .take(request.limit as usize)
                .collect::<Vec<_>>();
            let consumed = request.offset as u64 + items.len() as u64;
            let has_more = consumed < total;
            let next_offset =
                (has_more && !items.is_empty()).then_some(request.offset + items.len() as u32);
            Ok(Page {
                items,
                pagination: PageMeta {
                    limit: request.limit,
                    offset: request.offset,
                    total: Some(total),
                    has_more,
                    next_offset,
                    extensions: Extensions::from([
                        (
                            "backend".to_owned(),
                            json!("official_android_account_following_artists"),
                        ),
                        ("source_user_id".to_owned(), json!(user_id)),
                        ("complete_snapshot".to_owned(), json!(true)),
                        ("upstream_pages_read".to_owned(), json!(upstream_pages)),
                    ]),
                },
            })
        })
        .await;

        self.ensure_account_snapshot_current(Some(alias), &source)?;
        let result = match outcome {
            Ok(result) => result,
            Err(_) => Err(TuneWeaveError::new(
                ErrorCode::UpstreamTimeout,
                "Soda following-artists read exceeded its total time budget",
            )
            .with_platform(Platform::Soda)
            .retryable(true)),
        };
        if let Err(error) = &result {
            self.discard_response_credential_after_error(error.code)?;
        }
        pending.complete();
        result
    }
}
