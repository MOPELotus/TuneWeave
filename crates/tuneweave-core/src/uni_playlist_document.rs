use std::collections::BTreeSet;

use crate::{
    ErrorCode, Platform, Result, TuneWeaveError, UniPlaylistDocument, UniPlaylistDocumentItem,
    UniPlaylistDocumentSnapshot, UniPlaylistDocumentSnapshotExtensions,
};

pub const UNI_PLAYLIST_DOCUMENT_FORMAT: &str = "tuneweave_uni_playlist_v1";
const MAX_DOCUMENT_ITEMS: usize = 100_000;

impl UniPlaylistDocument {
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
        Quality, ResourceRef, UniPlaylistDocumentExtensions, UniPlaylistDocumentFormat,
        UniPlaylistDocumentItemExtensions, UniPlaylistDocumentSnapshotExtensions,
        UniPlaylistItemKind,
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
