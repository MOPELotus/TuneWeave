use super::*;
use crate::client::library::{MAX_PAGES, PAGE_SIZE, Section};
use crate::credential::{MiguCredential, authentication_required, error, validate_uid};
use tuneweave_core::{ErrorCode, StoredAccountCredential};

impl MiguProvider {
    pub(super) async fn read_library_section(
        &self,
        section: Section,
        account: &str,
        current: &mut MiguCredential,
        stored: &mut Option<StoredAccountCredential>,
    ) -> Result<Vec<Playlist>> {
        let mut items = Vec::new();
        let mut seen = BTreeSet::new();
        let mut expected_total = None;
        for index in 1..=MAX_PAGES {
            let response = self
                .client
                .account_library_page(section, index, current.token(), current.user_id())
                .await?;
            self.accept_read(current, stored.as_ref(), current)?;
            self.verify_account_step(account, current, stored, &response.token)
                .await?;
            let page = response.data?;
            if let Some(total) = expected_total {
                if total != page.total {
                    return Err(migu_upstream_error(
                        "Migu library total changed between pages",
                    ));
                }
            } else {
                expected_total = Some(page.total);
            }
            let count = page.items.len();
            for item in &page.items {
                if !seen.insert(item.id.clone()) {
                    return Err(migu_upstream_error(
                        "Migu library repeated a playlist within one section",
                    ));
                }
            }
            items.extend(page.items);
            let consumed = items.len() as u64;
            let finished = match page.total {
                Some(total) => {
                    if consumed > total || (consumed < total && count != PAGE_SIZE) {
                        return Err(migu_upstream_error(
                            "Migu library page disagrees with its total",
                        ));
                    }
                    consumed == total
                }
                None => count < PAGE_SIZE,
            };
            if let Some(has_next) = page.has_next {
                if has_next && count != PAGE_SIZE || page.total.is_some() && has_next == finished {
                    return Err(migu_upstream_error(
                        "Migu library continuation is inconsistent",
                    ));
                }
                if !has_next {
                    return Ok(items);
                }
            }
            if finished {
                return Ok(items);
            }
        }
        Err(migu_upstream_error(
            "Migu library exceeded its complete-read page budget",
        ))
    }

    pub(super) async fn read_account_library(
        &self,
        uid: Option<&str>,
        request: &PageRequest,
        section: Option<Section>,
    ) -> Result<Page<Playlist>> {
        if !(1..=100).contains(&request.limit)
            || request.offset.checked_add(request.limit).is_none()
        {
            return Err(migu_invalid_request(
                "Migu account library pagination is invalid",
            ));
        }
        if let Some(uid) = uid {
            validate_uid(uid)?;
        }
        let account = request.account.as_deref().unwrap_or("default");
        let (mut current, mut stored) = self
            .selected(account)?
            .ok_or_else(authentication_required)?;
        if uid.is_some_and(|uid| uid != current.user_id()) {
            return Err(error(
                ErrorCode::PermissionDenied,
                "Migu library is available only for the selected account",
            ));
        }
        let original = current.clone();
        let result = async {
            self.verify_account_step(account, &mut current, &mut stored, original.token())
                .await?;
            let mut all = Vec::new();
            for selected in [Section::Created, Section::Saved] {
                if section.is_none_or(|wanted| wanted == selected) {
                    all.extend(
                        self.read_library_section(selected, account, &mut current, &mut stored)
                            .await?,
                    );
                }
            }
            self.accept_read(&current, stored.as_ref(), &current)?;
            let total = all.len() as u64;
            let items: Vec<_> = all
                .into_iter()
                .skip(request.offset as usize)
                .take(request.limit as usize)
                .collect();
            let end = u64::from(request.offset) + items.len() as u64;
            let has_more = end < total;
            Ok(Page {
                items,
                pagination: PageMeta {
                    limit: request.limit,
                    offset: request.offset,
                    total: Some(total),
                    has_more,
                    next_offset: has_more.then_some(end as u32),
                    extensions: Extensions::from([
                        ("backend".into(), json!("official_h5_account_playlists")),
                        ("upstream_page_size".into(), json!(PAGE_SIZE)),
                        ("complete_read".into(), json!(true)),
                        (
                            "library_section".into(),
                            json!(section.map_or("all", Section::name)),
                        ),
                    ]),
                },
            })
        }
        .await;
        self.finish_account_read(&original, &current, stored.as_ref(), result)
    }
}

#[cfg(test)]
mod tests;
