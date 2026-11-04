use super::*;
use tuneweave_core::{ArtistCatalogFilter, ArtistVideoTag};

impl QqProvider {
    pub(super) async fn read_filtered_artist_albums(
        &self,
        id: &str,
        request: &tuneweave_core::PageRequest,
        filter: &ArtistCatalogFilter,
    ) -> Result<Page<Album>> {
        self.validate_public_account(request.account.as_deref())?;
        if !(1..=100).contains(&request.limit) {
            return Err(TuneWeaveError::invalid_request(
                "QQ singer album page size must be between 1 and 100",
            )
            .with_platform(Platform::Qq));
        }
        validate_filter(filter, "album")?;
        let mut request_api = qq_singer_albums_request(id, request.limit, request.offset)?;
        if let Some(order) = filter.order {
            request_api.param["order"] = json!(order_code(order)?);
        }
        if let Some(kind) = filter.album_type {
            request_api.param["types"] = json!([kind]);
        }
        let response = self
            .client
            .request_android(&[request_api])
            .await?
            .into_iter()
            .next()
            .ok_or_else(|| qq_data_error("QQ singer album request returned no response"))?;
        map_qq_singer_albums(id.trim(), request.limit, request.offset, response)
    }
    pub(super) async fn read_filtered_artist_videos(
        &self,
        id: &str,
        request: &ArtistVideoListRequest,
        filter: &ArtistCatalogFilter,
    ) -> Result<Page<Video>> {
        self.validate_public_account(request.account.as_deref())?;
        if !(1..=100).contains(&request.limit) {
            return Err(TuneWeaveError::invalid_request(
                "QQ singer MV page size must be between 1 and 100",
            )
            .with_platform(Platform::Qq));
        }
        if request.cursor.is_some() {
            return Err(TuneWeaveError::invalid_request(
                "QQ singer MV list does not support cursors",
            )
            .with_platform(Platform::Qq));
        }
        validate_filter(filter, "video")?;
        let mut request_api = qq_singer_mvs_request(id, request.limit, request.offset)?;
        let order = match request
            .order
            .as_deref()
            .unwrap_or("hot")
            .to_ascii_lowercase()
            .as_str()
        {
            "hot" => 1,
            "time" | "latest" | "newest" => 0,
            _ => {
                return Err(TuneWeaveError::invalid_request(
                    "QQ singer MV order must be hot or time",
                )
                .with_platform(Platform::Qq));
            }
        };
        request_api.param["order"] =
            json!(filter.order.map(order_code).transpose()?.unwrap_or(order));
        if let Some(tag) = filter.video_tag_id {
            request_api.param["tagid"] = json!(tag);
        }
        let response = self
            .client
            .request_android(&[request_api])
            .await?
            .into_iter()
            .next()
            .ok_or_else(|| qq_data_error("QQ singer MV request returned no response"))?;
        map_qq_singer_mvs(
            id.trim(),
            request.limit,
            request.offset,
            request.kind,
            response,
        )
    }
    pub(super) async fn read_filtered_artist_tab(
        &self,
        id: &str,
        request: &ArtistHomepageTabRequest,
        filter: &ArtistCatalogFilter,
    ) -> Result<ArtistHomepageTab> {
        self.validate_public_account(request.account.as_deref())?;
        let mut request_api = qq_singer_homepage_tab_request(id, request)?;
        configure_tab(&mut request_api, request.kind, filter)?;
        let response = self
            .client
            .request_android(&[request_api])
            .await?
            .into_iter()
            .next()
            .ok_or_else(|| qq_data_error("QQ singer homepage tab request returned no response"))?;
        map_qq_singer_homepage_tab(id.trim(), request, response)
    }
    pub(super) async fn read_artist_video_tags(
        &self,
        id: &str,
        account: Option<&str>,
    ) -> Result<Vec<ArtistVideoTag>> {
        validate_qq_media_id(id.trim(), "singer MID")?;
        self.validate_public_account(account)?;
        let request = QqApiRequest::new(
            SINGER_MV_MODULE,
            "GetSingerMvTag",
            json!({"singermid":id.trim()}),
        );
        let response = self
            .client
            .request_android(&[request])
            .await?
            .into_iter()
            .next()
            .ok_or_else(|| qq_data_error("QQ singer MV tag response is missing"))?;
        parse_video_tags(&response.data)
    }
}
fn order_code(order: ArtistTrackOrder) -> Result<u8> {
    match order {
        ArtistTrackOrder::Hot => Ok(1),
        ArtistTrackOrder::Time => Ok(0),
        ArtistTrackOrder::PlatformDefault => Err(TuneWeaveError::invalid_request(
            "QQ artist order must be hot or time",
        )
        .with_platform(Platform::Qq)),
    }
}
fn validate_filter(filter: &ArtistCatalogFilter, target: &str) -> Result<()> {
    if filter.album_type.is_some_and(|v| !matches!(v, 0 | 1 | 11))
        || (target == "album" && (filter.video_tag_id.is_some() || filter.include_categories))
        || (target == "video" && (filter.album_type.is_some() || filter.include_categories))
    {
        return Err(TuneWeaveError::invalid_request(
            "QQ artist filter is incompatible with the selected catalogue",
        )
        .with_platform(Platform::Qq));
    }
    if let Some(order) = filter.order {
        order_code(order)?;
    }
    Ok(())
}
fn configure_tab(
    request: &mut QqApiRequest,
    kind: ArtistHomepageTabKind,
    filter: &ArtistCatalogFilter,
) -> Result<()> {
    if let Some(order) = filter.order {
        request.param["Order"] = json!(order_code(order)?);
    }
    match kind {
        ArtistHomepageTabKind::Album => {
            if filter.video_tag_id.is_some()
                || filter.album_type.is_some_and(|v| !matches!(v, 0 | 1 | 11))
            {
                return Err(TuneWeaveError::invalid_request(
                    "invalid QQ album tab filter",
                ));
            }
            if filter.include_categories || filter.album_type.is_some() {
                request.param["AlbumExtension"] = json!({"IsNeedFilterType":u8::from(filter.include_categories),"FilterType":filter.album_type.unwrap_or(0)});
            }
        }
        ArtistHomepageTabKind::Video => {
            if filter.album_type.is_some() {
                return Err(TuneWeaveError::invalid_request(
                    "invalid QQ video tab filter",
                ));
            }
            if filter.include_categories || filter.video_tag_id.is_some() {
                request.param["VideoExtension"] = json!({"IsNeedTagList":u8::from(filter.include_categories),"TagID":filter.video_tag_id.unwrap_or(0)});
            }
        }
        _ => {
            if filter.album_type.is_some()
                || filter.video_tag_id.is_some()
                || filter.include_categories
            {
                return Err(TuneWeaveError::invalid_request(
                    "QQ category filters require an album or video tab",
                ));
            }
        }
    }
    Ok(())
}
fn parse_video_tags(data: &Value) -> Result<Vec<ArtistVideoTag>> {
    let list = data
        .get("list")
        .and_then(Value::as_array)
        .ok_or_else(|| qq_data_error("QQ singer MV tags omitted their list"))?;
    if list.len() > 1_000 {
        return Err(qq_data_error("QQ singer MV tag list exceeds its limit"));
    }
    let mut ids = BTreeSet::new();
    list.iter()
        .map(|item| {
            let id = item
                .get("id")
                .and_then(Value::as_u64)
                .ok_or_else(|| qq_data_error("QQ singer MV tag identity is invalid"))?;
            let name = item
                .get("name")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty() && s.len() <= 512 && !s.chars().any(char::is_control))
                .ok_or_else(|| qq_data_error("QQ singer MV tag name is invalid"))?;
            Ok(ArtistVideoTag {
                id,
                name: name.to_owned(),
            })
        })
        .filter_map(|result| match result {
            Ok(tag) if ids.insert(tag.id) => Some(Ok(tag)),
            Ok(_) => None,
            Err(error) => Some(Err(error)),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    #[ignore = "requires live QQ access"]
    async fn live_artist_catalog_filters_and_tags() {
        let provider = QqProvider::new(QqConfig::default()).unwrap();
        let id = "0025NhlN2yWrP4";
        for order in [ArtistTrackOrder::Hot, ArtistTrackOrder::Time] {
            let mut tracks = ArtistTrackListRequest::new(2, 0);
            tracks.order = order;
            assert!(
                !provider
                    .artist_tracks(id, &tracks)
                    .await
                    .unwrap()
                    .items
                    .is_empty()
            );
            let filter = ArtistCatalogFilter {
                order: Some(order),
                album_type: Some(0),
                ..Default::default()
            };
            let page = provider
                .artist_albums_filtered(
                    id,
                    &tuneweave_core::PageRequest {
                        limit: 2,
                        offset: 0,
                        account: None,
                    },
                    &filter,
                )
                .await
                .unwrap();
            assert!(!page.items.is_empty());
        }
        let tags = provider.artist_video_tags(id, None).await.unwrap();
        assert!(!tags.is_empty());
        let filter = ArtistCatalogFilter {
            video_tag_id: Some(tags[0].id),
            ..Default::default()
        };
        let mut request = ArtistVideoListRequest::new(2, 0);
        request.order = Some("time".into());
        let videos = provider
            .artist_videos_filtered(id, &request, &filter)
            .await
            .unwrap();
        assert!(!videos.items.is_empty());
        for kind in [ArtistHomepageTabKind::Album, ArtistHomepageTabKind::Video] {
            let filter = ArtistCatalogFilter {
                order: Some(ArtistTrackOrder::Time),
                include_categories: true,
                ..Default::default()
            };
            provider
                .artist_homepage_tab_filtered(
                    id,
                    &ArtistHomepageTabRequest::new(kind, 1, 2),
                    &filter,
                )
                .await
                .unwrap();
        }
    }

    #[test]
    fn tab_filters_use_native_numeric_fields_and_reject_cross_catalogue_controls() {
        let req = ArtistHomepageTabRequest::new(ArtistHomepageTabKind::Album, 1, 10);
        let mut wire = qq_singer_homepage_tab_request("0025NhlN2yWrP4", &req).unwrap();
        let filter = ArtistCatalogFilter {
            order: Some(ArtistTrackOrder::Time),
            album_type: Some(11),
            include_categories: true,
            ..Default::default()
        };
        configure_tab(&mut wire, req.kind, &filter).unwrap();
        assert_eq!(wire.param["Order"], 0);
        assert_eq!(
            wire.param["AlbumExtension"],
            json!({"IsNeedFilterType":1,"FilterType":11})
        );
        assert!(configure_tab(&mut wire, ArtistHomepageTabKind::Video, &filter).is_err());
        assert!(
            parse_video_tags(&json!({"list":[{"id":0,"name":"全部"},{"id":0,"name":"重复"}]}))
                .is_ok()
        );
        assert_eq!(
            parse_video_tags(&json!({"list":[{"id":0,"name":"全部"}]})).unwrap()[0].id,
            0
        );
    }
}
