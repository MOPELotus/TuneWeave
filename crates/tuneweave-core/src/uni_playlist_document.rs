use std::collections::BTreeSet;

use serde::de::DeserializeOwned;

use crate::{
    ErrorCode, Extensions, Platform, Quality, Result, TuneWeaveError, UniPlaylist,
    UniPlaylistDocument, UniPlaylistDocumentExtensions, UniPlaylistDocumentFormat,
    UniPlaylistDocumentItem, UniPlaylistDocumentItemExtensions, UniPlaylistDocumentSnapshot,
    UniPlaylistDocumentSnapshotExtensions, UniPlaylistItem,
};

pub const UNI_PLAYLIST_DOCUMENT_FORMAT: &str = "tuneweave_uni_playlist_v1";
const MAX_DOCUMENT_ITEMS: usize = 100_000;

impl UniPlaylistDocument {
    pub fn from_server_snapshot(playlist: &UniPlaylist, items: &[UniPlaylistItem]) -> Result<Self> {
        if playlist.item_count != u64::try_from(items.len()).unwrap_or(u64::MAX) {
            return Err(TuneWeaveError::new(
                ErrorCode::InternalError,
                "stored Uni Playlist item count does not match its item sequence",
            ));
        }
        let document = Self {
            format: UniPlaylistDocumentFormat::V1,
            id: playlist.id.clone(),
            name: playlist.name.trim().to_owned(),
            description: playlist.description.trim().to_owned(),
            item_count: u64::try_from(items.len()).unwrap_or(u64::MAX),
            created_at_ms: playlist.created_at_ms,
            updated_at_ms: playlist.updated_at_ms,
            items: items
                .iter()
                .map(document_item_from_server)
                .collect::<Result<Vec<_>>>()?,
            extensions: UniPlaylistDocumentExtensions {
                duplicates_preserved: true,
            },
        };
        document.validate().map_err(|_| {
            TuneWeaveError::new(
                ErrorCode::InternalError,
                "stored Uni Playlist cannot be exported as a safe V1 document",
            )
        })?;
        Ok(document)
    }

    pub fn validate(&self) -> Result<()> {
        validate_document_id(&self.id, "Uni Playlist document id")?;
        validate_trimmed_text(&self.name, 200, false, "Uni Playlist document name")?;
        validate_trimmed_text(
            &self.description,
            4_000,
            true,
            "Uni Playlist document description",
        )?;
        if self.updated_at_ms < self.created_at_ms {
            return Err(TuneWeaveError::invalid_request(
                "Uni Playlist document updated_at_ms cannot precede created_at_ms",
            ));
        }
        if self.items.len() > MAX_DOCUMENT_ITEMS {
            return Err(TuneWeaveError::invalid_request(
                "Uni Playlist document cannot contain more than 100000 items",
            ));
        }
        let item_count = u64::try_from(self.items.len()).unwrap_or(u64::MAX);
        if self.item_count != item_count {
            return Err(TuneWeaveError::invalid_request(
                "Uni Playlist document item_count must match its item sequence",
            ));
        }
        if !self.extensions.duplicates_preserved {
            return Err(TuneWeaveError::invalid_request(
                "Uni Playlist document must preserve duplicate source entries",
            ));
        }
        let mut item_ids = BTreeSet::new();
        for (position, item) in self.items.iter().enumerate() {
            validate_document_item(item, position, self.updated_at_ms)?;
            if !item_ids.insert(item.id.as_str()) {
                return Err(TuneWeaveError::new(
                    ErrorCode::Conflict,
                    "Uni Playlist document item ids must be unique",
                ));
            }
        }
        Ok(())
    }
}

fn document_item_from_server(item: &UniPlaylistItem) -> Result<UniPlaylistDocumentItem> {
    let import_source_index = decode_extension(&item.extensions, "import_source_index")?;
    let import_source_ref = decode_extension(&item.extensions, "import_source_ref")?;
    let import_source_type = decode_extension(&item.extensions, "import_source_type")?;
    let imported_from_item_id = decode_extension(&item.extensions, "imported_from_item_id")?;
    let provenance_count = [
        import_source_index.is_some(),
        import_source_ref.is_some(),
        import_source_type.is_some(),
    ]
    .into_iter()
    .filter(|present| *present)
    .count();
    if !matches!(provenance_count, 0 | 3) {
        return Err(TuneWeaveError::new(
            ErrorCode::InternalError,
            "stored Uni Playlist contains incomplete import provenance",
        ));
    }
    Ok(UniPlaylistDocumentItem {
        id: item.id.clone(),
        position: item.position,
        kind: item.kind,
        source_ref: item.source_ref.clone(),
        snapshot: document_snapshot_from_server(&item.snapshot)?,
        added_at_ms: item.added_at_ms,
        extensions: UniPlaylistDocumentItemExtensions {
            import_source_index,
            import_source_ref,
            import_source_type,
            imported_from_item_id,
        },
    })
}

fn document_snapshot_from_server(
    snapshot: &crate::UniPlaylistItemSnapshot,
) -> Result<UniPlaylistDocumentSnapshot> {
    let available_qualities =
        decode_extension::<Vec<Quality>>(&snapshot.extensions, "available_qualities")?
            .unwrap_or_default();
    let cover_url = snapshot
        .cover_url
        .as_deref()
        .map(str::trim)
        .filter(|url| {
            url.len() <= 4_096 && url.starts_with("https://") && !url.chars().any(char::is_control)
        })
        .map(str::to_owned);
    Ok(UniPlaylistDocumentSnapshot {
        title: snapshot.title.trim().to_owned(),
        artists: snapshot
            .artists
            .iter()
            .map(|artist| artist.trim().to_owned())
            .collect(),
        album: normalized_optional_text(snapshot.album.as_deref()),
        duration_ms: snapshot.duration_ms,
        isrc: normalized_optional_text(snapshot.isrc.as_deref()).filter(|value| !value.is_empty()),
        cover_url,
        version_tags: snapshot
            .version_tags
            .iter()
            .map(|tag| tag.trim().to_owned())
            .collect(),
        extensions: UniPlaylistDocumentSnapshotExtensions {
            canonical_ref: decode_extension(&snapshot.extensions, "canonical_ref")?,
            playable: decode_extension(&snapshot.extensions, "playable")?,
            available_qualities,
            mv_ref: decode_extension(&snapshot.extensions, "mv_ref")?,
            video_kind: decode_extension(&snapshot.extensions, "video_kind")?,
            published_at: decode_extension::<String>(&snapshot.extensions, "published_at")?
                .and_then(|value| normalized_optional_text(Some(&value))),
            podcast_ref: decode_extension(&snapshot.extensions, "podcast_ref")?,
            audio_ref: decode_extension(&snapshot.extensions, "audio_ref")?,
            serial_number: decode_extension(&snapshot.extensions, "serial_number")?,
            description: decode_extension::<String>(&snapshot.extensions, "description")?
                .and_then(|value| normalized_optional_text(Some(&value))),
            category: decode_extension::<String>(&snapshot.extensions, "category")?
                .and_then(|value| normalized_optional_text(Some(&value))),
            region: decode_extension::<String>(&snapshot.extensions, "region")?
                .and_then(|value| normalized_optional_text(Some(&value))),
            current_program: decode_extension::<String>(&snapshot.extensions, "current_program")?
                .and_then(|value| normalized_optional_text(Some(&value))),
            has_direct_stream: decode_extension(&snapshot.extensions, "has_direct_stream")?,
        },
    })
}

fn decode_extension<T: DeserializeOwned>(extensions: &Extensions, key: &str) -> Result<Option<T>> {
    let Some(value) = extensions.get(key) else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    serde_json::from_value(value.clone())
        .map(Some)
        .map_err(|_| {
            TuneWeaveError::new(
                ErrorCode::InternalError,
                format!("stored Uni Playlist extension {key} has an invalid shape"),
            )
        })
}

fn normalized_optional_text(value: Option<&str>) -> Option<String> {
    value.map(str::trim).map(str::to_owned)
}

fn validate_document_item(
    item: &UniPlaylistDocumentItem,
    position: usize,
    updated_at_ms: u64,
) -> Result<()> {
    validate_document_id(&item.id, "Uni Playlist document item id")?;
    if item.position != u64::try_from(position).unwrap_or(u64::MAX) {
        return Err(TuneWeaveError::invalid_request(
            "Uni Playlist document item positions must be zero-based and contiguous",
        ));
    }
    if item.source_ref.platform() == Platform::Uni {
        return Err(TuneWeaveError::invalid_request(
            "Uni Playlist document items must reference external platform resources",
        ));
    }
    if item.added_at_ms > updated_at_ms {
        return Err(TuneWeaveError::invalid_request(
            "Uni Playlist document items cannot be newer than the document",
        ));
    }
    validate_document_snapshot(&item.snapshot)?;
    let provenance_count = [
        item.extensions.import_source_index.is_some(),
        item.extensions.import_source_ref.is_some(),
        item.extensions.import_source_type.is_some(),
    ]
    .into_iter()
    .filter(|present| *present)
    .count();
    if !matches!(provenance_count, 0 | 3) {
        return Err(TuneWeaveError::invalid_request(
            "Uni Playlist document import provenance must be complete or absent",
        ));
    }
    if let Some(source_type) = item.extensions.import_source_type.as_deref() {
        validate_source_type(source_type)?;
    }
    if let Some(item_id) = item.extensions.imported_from_item_id.as_deref() {
        validate_document_id(item_id, "imported Uni Playlist document item id")?;
    }
    Ok(())
}

fn validate_document_snapshot(snapshot: &UniPlaylistDocumentSnapshot) -> Result<()> {
    validate_trimmed_text(
        &snapshot.title,
        500,
        false,
        "Uni Playlist document item title",
    )?;
    if snapshot.artists.len() > 100 {
        return Err(TuneWeaveError::invalid_request(
            "Uni Playlist document item cannot contain more than 100 artists",
        ));
    }
    for artist in &snapshot.artists {
        validate_trimmed_text(artist, 200, false, "Uni Playlist document item artist")?;
    }
    validate_optional_text(
        snapshot.album.as_deref(),
        500,
        true,
        "Uni Playlist document item album",
    )?;
    validate_optional_text(
        snapshot.isrc.as_deref(),
        64,
        false,
        "Uni Playlist document item ISRC",
    )?;
    if let Some(cover_url) = snapshot.cover_url.as_deref()
        && (cover_url.len() > 4_096
            || !cover_url.starts_with("https://")
            || cover_url.chars().any(char::is_control))
    {
        return Err(TuneWeaveError::invalid_request(
            "Uni Playlist document cover_url must be a bounded HTTPS URL",
        ));
    }
    if snapshot.version_tags.len() > 100 {
        return Err(TuneWeaveError::invalid_request(
            "Uni Playlist document item cannot contain more than 100 version tags",
        ));
    }
    for tag in &snapshot.version_tags {
        validate_trimmed_text(tag, 200, false, "Uni Playlist document item version tag")?;
    }
    validate_snapshot_extensions(&snapshot.extensions)
}

fn validate_snapshot_extensions(extensions: &UniPlaylistDocumentSnapshotExtensions) -> Result<()> {
    for reference in [
        extensions.canonical_ref.as_ref(),
        extensions.mv_ref.as_ref(),
        extensions.podcast_ref.as_ref(),
        extensions.audio_ref.as_ref(),
    ]
    .into_iter()
    .flatten()
    {
        if reference.platform() == Platform::Uni {
            return Err(TuneWeaveError::invalid_request(
                "Uni Playlist document snapshot references must use external platforms",
            ));
        }
    }
    if extensions.available_qualities.len() > 32 {
        return Err(TuneWeaveError::invalid_request(
            "Uni Playlist document snapshot cannot contain more than 32 quality tiers",
        ));
    }
    validate_optional_text(
        extensions.published_at.as_deref(),
        128,
        true,
        "Uni Playlist document publication time",
    )?;
    validate_optional_text(
        extensions.description.as_deref(),
        4_000,
        true,
        "Uni Playlist document station description",
    )?;
    validate_optional_text(
        extensions.category.as_deref(),
        200,
        true,
        "Uni Playlist document station category",
    )?;
    validate_optional_text(
        extensions.region.as_deref(),
        200,
        true,
        "Uni Playlist document station region",
    )?;
    validate_optional_text(
        extensions.current_program.as_deref(),
        500,
        true,
        "Uni Playlist document station current program",
    )
}

fn validate_document_id(id: &str, field: &str) -> Result<()> {
    if !(16..=64).contains(&id.len())
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(TuneWeaveError::invalid_request(format!(
            "{field} must be 16 to 64 URL-safe ASCII characters"
        )));
    }
    Ok(())
}

fn validate_source_type(source_type: &str) -> Result<()> {
    if source_type.is_empty()
        || source_type.len() > 64
        || !source_type.bytes().enumerate().all(|(index, byte)| {
            byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || (byte == b'_' && index > 0 && index + 1 < source_type.len())
        })
    {
        return Err(TuneWeaveError::invalid_request(
            "Uni Playlist document import source type must use normalized snake_case ASCII",
        ));
    }
    Ok(())
}

fn validate_optional_text(
    value: Option<&str>,
    maximum: usize,
    empty_allowed: bool,
    field: &str,
) -> Result<()> {
    if let Some(value) = value {
        validate_trimmed_text(value, maximum, empty_allowed, field)?;
    }
    Ok(())
}

fn validate_trimmed_text(
    value: &str,
    maximum: usize,
    empty_allowed: bool,
    field: &str,
) -> Result<()> {
    if value.len() > maximum
        || value != value.trim()
        || (!empty_allowed && value.is_empty())
        || value.chars().any(char::is_control)
    {
        return Err(TuneWeaveError::invalid_request(format!(
            "{field} must be trimmed, bounded UTF-8 text"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::{
        Extensions, Quality, ResourceRef, UniPlaylist, UniPlaylistDocumentExtensions,
        UniPlaylistDocumentFormat, UniPlaylistDocumentItemExtensions,
        UniPlaylistDocumentSnapshotExtensions, UniPlaylistItem, UniPlaylistItemKind,
        UniPlaylistItemSnapshot,
    };

    fn sample_document() -> UniPlaylistDocument {
        UniPlaylistDocument {
            format: UniPlaylistDocumentFormat::V1,
            id: "pl_01abcdefghijklmnop".to_owned(),
            name: "跨平台收藏".to_owned(),
            description: "可在兼容客户端之间交换".to_owned(),
            item_count: 1,
            created_at_ms: 1_753_137_600_000,
            updated_at_ms: 1_753_137_600_100,
            items: vec![UniPlaylistDocumentItem {
                id: "item_01abcdefghijklmnop".to_owned(),
                position: 0,
                kind: UniPlaylistItemKind::Track,
                source_ref: ResourceRef::new(Platform::Netease, "185809").expect("source ref"),
                snapshot: UniPlaylistDocumentSnapshot {
                    title: "反方向的钟".to_owned(),
                    artists: vec!["周杰伦".to_owned()],
                    album: Some("Jay".to_owned()),
                    duration_ms: Some(258_000),
                    isrc: Some("TWK970000101".to_owned()),
                    cover_url: Some("https://example.test/cover.jpg".to_owned()),
                    version_tags: Vec::new(),
                    extensions: UniPlaylistDocumentSnapshotExtensions {
                        canonical_ref: Some(
                            ResourceRef::new(Platform::Netease, "185809").expect("canonical ref"),
                        ),
                        playable: Some(true),
                        available_qualities: vec![Quality::High],
                        ..Default::default()
                    },
                },
                added_at_ms: 1_753_137_600_100,
                extensions: UniPlaylistDocumentItemExtensions {
                    import_source_index: Some(0),
                    import_source_ref: Some(
                        ResourceRef::new(Platform::Netease, "3778678").expect("import source ref"),
                    ),
                    import_source_type: Some("playlist".to_owned()),
                    imported_from_item_id: None,
                },
            }],
            extensions: UniPlaylistDocumentExtensions {
                duplicates_preserved: true,
            },
        }
    }

    #[test]
    fn v1_document_round_trips_without_credential_or_transport_fields() {
        let document = sample_document();
        document.validate().expect("valid document");
        let encoded = serde_json::to_string(&document).expect("serialize document");
        assert!(encoded.contains(UNI_PLAYLIST_DOCUMENT_FORMAT));
        for forbidden in [
            "cookie",
            "token",
            "credential",
            "password",
            "stream_url",
            "headers",
        ] {
            assert!(!encoded.to_ascii_lowercase().contains(forbidden));
        }
        let decoded =
            serde_json::from_str::<UniPlaylistDocument>(&encoded).expect("deserialize document");
        assert_eq!(decoded, document);
    }

    #[test]
    fn server_snapshot_export_selects_only_safe_typed_fields() {
        let mut playlist = UniPlaylist::new(
            ResourceRef::new(Platform::Uni, "pl_01abcdefghijklmnop").expect("playlist ref"),
            "跨平台收藏",
            "安全导出",
            1_753_137_600_000,
        );
        playlist.item_count = 1;
        playlist.updated_at_ms = 1_753_137_600_100;
        playlist
            .extensions
            .insert("cookie".to_owned(), json!("MUSIC_U=secret"));
        let mut snapshot = UniPlaylistItemSnapshot::new("反方向的钟");
        snapshot.artists = vec!["周杰伦".to_owned()];
        snapshot.cover_url = Some("http://127.0.0.1/private-cover".to_owned());
        snapshot.extensions.insert(
            "canonical_ref".to_owned(),
            json!(ResourceRef::new(Platform::Netease, "185809").expect("canonical ref")),
        );
        snapshot
            .extensions
            .insert("playable".to_owned(), json!(true));
        snapshot
            .extensions
            .insert("cookie".to_owned(), json!("MUSIC_U=secret"));
        snapshot.extensions.insert(
            "stream_url".to_owned(),
            json!("https://media.example.test/temporary"),
        );
        let item = UniPlaylistItem {
            id: "item_01abcdefghijklmnop".to_owned(),
            position: 0,
            kind: UniPlaylistItemKind::Track,
            source_ref: ResourceRef::new(Platform::Netease, "185809").expect("source ref"),
            snapshot,
            added_at_ms: 1_753_137_600_100,
            extensions: Extensions::from([
                ("import_source_index".to_owned(), json!(0)),
                (
                    "import_source_ref".to_owned(),
                    json!(
                        ResourceRef::new(Platform::Netease, "3778678").expect("import source ref")
                    ),
                ),
                ("import_source_type".to_owned(), json!("playlist")),
                ("token".to_owned(), json!("secret")),
            ]),
        };

        let document =
            UniPlaylistDocument::from_server_snapshot(&playlist, &[item]).expect("safe export");
        assert_eq!(document.items[0].snapshot.cover_url, None);
        assert_eq!(
            document.items[0].snapshot.extensions.canonical_ref,
            Some(ResourceRef::new(Platform::Netease, "185809").expect("expected canonical ref"))
        );
        assert_eq!(
            document.items[0].extensions.import_source_type.as_deref(),
            Some("playlist")
        );
        let encoded = serde_json::to_string(&document).expect("serialize safe export");
        for forbidden in ["cookie", "token", "stream_url", "secret", "127.0.0.1"] {
            assert!(!encoded.to_ascii_lowercase().contains(forbidden));
        }

        let mut malformed = document.items[0].clone();
        malformed.snapshot.extensions.playable = None;
        let mut stored_snapshot = UniPlaylistItemSnapshot::new("反方向的钟");
        stored_snapshot
            .extensions
            .insert("playable".to_owned(), json!("yes"));
        let malformed_item = UniPlaylistItem {
            snapshot: stored_snapshot,
            extensions: Extensions::new(),
            ..UniPlaylistItem {
                id: malformed.id,
                position: malformed.position,
                kind: malformed.kind,
                source_ref: malformed.source_ref,
                snapshot: UniPlaylistItemSnapshot::new("unused"),
                added_at_ms: malformed.added_at_ms,
                extensions: Extensions::new(),
            }
        };
        assert_eq!(
            UniPlaylistDocument::from_server_snapshot(&playlist, &[malformed_item])
                .expect_err("reject malformed known extension")
                .code,
            ErrorCode::InternalError
        );
    }

    #[test]
    fn v1_document_rejects_identity_order_count_time_and_transport_drift() {
        let mut invalid = sample_document();
        invalid.item_count = 2;
        assert_eq!(
            invalid.validate().expect_err("reject count drift").code,
            ErrorCode::InvalidRequest
        );

        let mut invalid = sample_document();
        invalid.items[0].position = 1;
        assert_eq!(
            invalid.validate().expect_err("reject order drift").code,
            ErrorCode::InvalidRequest
        );

        let mut invalid = sample_document();
        invalid.items.push(invalid.items[0].clone());
        invalid.items[1].position = 1;
        invalid.item_count = 2;
        assert_eq!(
            invalid.validate().expect_err("reject duplicate id").code,
            ErrorCode::Conflict
        );

        let mut invalid = sample_document();
        invalid.items[0].source_ref =
            ResourceRef::new(Platform::Uni, "pl_02abcdefghijklmnop").expect("local source ref");
        assert_eq!(
            invalid.validate().expect_err("reject local source").code,
            ErrorCode::InvalidRequest
        );

        let mut invalid = sample_document();
        invalid.items[0].snapshot.cover_url = Some("http://127.0.0.1/secret".to_owned());
        assert_eq!(
            invalid
                .validate()
                .expect_err("reject unsafe cover URL")
                .code,
            ErrorCode::InvalidRequest
        );

        let mut invalid = sample_document();
        invalid.items[0].extensions.import_source_type = None;
        assert_eq!(
            invalid
                .validate()
                .expect_err("reject partial provenance")
                .code,
            ErrorCode::InvalidRequest
        );

        let mut invalid = sample_document();
        invalid.updated_at_ms = invalid.created_at_ms - 1;
        assert_eq!(
            invalid.validate().expect_err("reject time drift").code,
            ErrorCode::InvalidRequest
        );
    }

    #[test]
    fn v1_document_deserialization_rejects_unknown_sensitive_fields() {
        let mut encoded = serde_json::to_value(sample_document()).expect("serialize document");
        encoded["cookie"] = json!("SESSDATA=secret");
        assert!(serde_json::from_value::<UniPlaylistDocument>(encoded).is_err());

        let mut encoded = serde_json::to_value(sample_document()).expect("serialize document");
        encoded["items"][0]["snapshot"]["extensions"]["stream_url"] =
            json!("https://media.example.test/temporary");
        assert!(serde_json::from_value::<UniPlaylistDocument>(encoded).is_err());
    }
}
