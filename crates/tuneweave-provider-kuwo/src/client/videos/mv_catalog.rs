use super::*;
use std::collections::BTreeSet;
use tuneweave_core::{
    MusicVideoArea, MusicVideoCatalog, MusicVideoListRequest, MusicVideoType, Page, PageMeta,
    VideoCatalogOption, VideoTaxonomyKind, VideoTaxonomyRequest,
};

// The official Web MV page has these fixed groups, in this order. In particular,
// 首播 is a curated group, not proof of all MVs or chronological publication order.
const GROUPS: [(&str, &str); 9] = [
    ("236682871", "首播"),
    ("236682731", "华语"),
    ("236742444", "日韩"),
    ("236682773", "网络"),
    ("236682735", "欧美"),
    ("236742576", "现场"),
    ("236682777", "热舞"),
    ("236742508", "伤感"),
    ("236742578", "剧情"),
];
const PAGE_SIZE: u32 = 20;

fn group(request: &MusicVideoListRequest) -> Result<&str> {
    if request.account.is_some()
        || request.catalog != MusicVideoCatalog::Group
        || !matches!(request.area, None | Some(MusicVideoArea::All))
        || !matches!(request.video_type, None | Some(MusicVideoType::Mv))
        || request.order.is_some()
        || !(20..=100).contains(&request.limit)
        || request.limit % PAGE_SIZE != 0
        || request.offset % PAGE_SIZE != 0
        || request.offset.checked_add(request.limit).is_none()
    {
        return Err(kuwo_invalid_request(
            "Kuwo MV groups require anonymous catalog=group, no area/order filters, type=mv or omitted, limit 20/40/60/80/100 and offset divisible by 20",
        ));
    }
    request
        .group_id
        .as_deref()
        .filter(|id| GROUPS.iter().any(|(known, _)| id == known))
        .ok_or_else(|| kuwo_invalid_request("Kuwo MV group_id must identify an official MV group"))
}

impl KuwoClient {
    pub async fn video_taxonomy(
        &self,
        request: &VideoTaxonomyRequest,
    ) -> Result<Page<VideoCatalogOption>> {
        if request.account.is_some()
            || request.kind != VideoTaxonomyKind::Groups
            || !(1..=100).contains(&request.limit)
            || request.offset.checked_add(request.limit).is_none()
        {
            return Err(kuwo_invalid_request(
                "Kuwo MV taxonomy requires anonymous kind=groups and limit 1..100",
            ));
        }
        let items = GROUPS
            .iter()
            .skip(request.offset as usize)
            .take(request.limit as usize)
            .map(|(id, name)| VideoCatalogOption {
                id: (*id).into(),
                name: (*name).into(),
                url: None,
                selected: None,
                related_video_type: Some("mv".into()),
                extensions: Extensions::from([("source_type".into(), json!("mv_group"))]),
            })
            .collect::<Vec<_>>();
        let end = request.offset + items.len() as u32;
        let has_more = end < GROUPS.len() as u32;
        Ok(Page {
            items,
            pagination: PageMeta {
                limit: request.limit,
                offset: request.offset,
                total: Some(GROUPS.len() as u64),
                has_more,
                next_offset: has_more.then_some(end),
                extensions: Extensions::from([
                    ("backend".into(), json!("current_web_mv_groups")),
                    ("source".into(), json!("fixed_official_web_tags")),
                    ("default_group_id".into(), json!(GROUPS[0].0)),
                ]),
            },
        })
    }

    pub async fn music_videos(&self, request: &MusicVideoListRequest) -> Result<Page<Video>> {
        let id = group(request)?;
        let first = request.offset / PAGE_SIZE + 1;
        let mut items = Vec::with_capacity(request.limit as usize);
        let mut seen = BTreeSet::new();
        let mut total = None;
        let mut fetched = 0;
        for page in first..first + request.limit / PAGE_SIZE {
            let result = self.mv_group_page(id, page).await?;
            if total.is_some_and(|value| value != result.total) {
                return Err(kuwo_upstream_error(
                    "Kuwo MV group total changed during pagination",
                ));
            }
            total = Some(result.total);
            fetched += 1;
            for item in result.items {
                if !seen.insert(item.id.clone()) {
                    return Err(kuwo_upstream_error(
                        "Kuwo MV group repeated a resource identity",
                    ));
                }
                items.push(item);
            }
            if u64::from(page) * u64::from(PAGE_SIZE) >= result.total {
                break;
            }
        }
        // Upstream can omit entries from a physical page and still retain the original
        // total. Progress by consumed source slots, including successful empty pages.
        let end = request.offset + fetched * PAGE_SIZE;
        let has_more = total.is_some_and(|value| u64::from(end) < value);
        Ok(Page {
            items,
            pagination: PageMeta {
                limit: request.limit,
                offset: request.offset,
                total,
                has_more,
                next_offset: has_more.then_some(end),
                extensions: Extensions::from([
                    ("backend".into(), json!("current_web_mv_catalogue")),
                    ("kind".into(), json!("mv")),
                    ("group_id".into(), json!(id)),
                    ("order".into(), json!("platform_default")),
                    ("offset_scope".into(), json!("upstream_catalogue_slots")),
                    ("total_scope".into(), json!("upstream_catalogue_slots")),
                    ("includes_offline_metadata".into(), json!(true)),
                    ("upstream_page_size".into(), json!(PAGE_SIZE)),
                    ("upstream_pages_fetched".into(), json!(fetched)),
                ]),
            },
        })
    }

    async fn mv_group_page(&self, id: &str, page: u32) -> Result<GroupPage> {
        for refresh in [false, true] {
            let response = self
                .signed_get(
                    KuwoSignedEndpoint::MvCatalog,
                    &GroupQuery {
                        pid: id,
                        pn: page,
                        rn: PAGE_SIZE,
                        https_status: 1,
                        request_id: new_request_id(),
                        plat: "web_www",
                        from: "",
                    },
                    "https://www.kuwo.cn/mvs",
                    refresh,
                    u8::from(refresh),
                )
                .await?;
            match response {
                KuwoSignedResponse::SessionRejected if !refresh => continue,
                KuwoSignedResponse::SessionRejected => return Err(invalid()),
                KuwoSignedResponse::Body(bytes) => {
                    if !refresh && is_signed_session_rejection(&bytes) {
                        continue;
                    }
                    return parse_group(&bytes, id, page);
                }
            }
        }
        Err(invalid())
    }
}

#[derive(Serialize)]
struct GroupQuery<'a> {
    pid: &'a str,
    pn: u32,
    rn: u32,
    #[serde(rename = "httpsStatus")]
    https_status: u8,
    #[serde(rename = "reqId")]
    request_id: String,
    plat: &'static str,
    from: &'static str,
}

struct GroupPage {
    total: u64,
    items: Vec<Video>,
}

#[derive(Deserialize)]
struct GroupEnvelope {
    code: i64,
    data: Option<GroupData>,
}

#[derive(Deserialize)]
struct GroupData {
    total: Unsigned,
    mvlist: Vec<artists::Mv>,
}

fn parse_group(bytes: &[u8], id: &str, page: u32) -> Result<GroupPage> {
    let body: GroupEnvelope = serde_json::from_slice(bytes).map_err(|_| invalid())?;
    if body.code != 200 {
        return Err(invalid().with_details(json!({"upstream_code": body.code})));
    }
    let data = body.data.ok_or_else(invalid)?;
    let total = data.total.value()?;
    let start = u64::from(page - 1) * u64::from(PAGE_SIZE);
    if data.mvlist.len() as u64 > total.saturating_sub(start).min(u64::from(PAGE_SIZE)) {
        return Err(invalid());
    }
    let items = data
        .mvlist
        .into_iter()
        .map(|row| {
            let mut video = artists::map_mv(row, None)?;
            video
                .extensions
                .insert("backend".into(), json!("current_web_mv_catalogue"));
            video.extensions.insert("source_group_id".into(), json!(id));
            Ok(video)
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(GroupPage { total, items })
}

#[cfg(test)]
mod tests;
