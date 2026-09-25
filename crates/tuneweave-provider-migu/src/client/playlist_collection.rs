use super::*;

#[derive(Clone, Copy)]
pub(super) enum Kind {
    Playlist,
    Album(super::albums::AlbumKind),
}
impl Kind {
    pub(super) fn resource_type(self) -> &'static str {
        match self {
            Self::Playlist => "2021",
            Self::Album(kind) => kind.resource_type(),
        }
    }
    pub(super) fn state_path(self) -> &'static str {
        match self {
            Self::Playlist => "/pc/query-ops/2021",
            Self::Album(super::albums::AlbumKind::Ordinary) => "/pc/query-ops/2003",
            Self::Album(super::albums::AlbumKind::Digital) => "/pc/query-ops/5",
        }
    }
}

pub(super) fn state(rows: Vec<serde_json::Value>, id: &str, kind: Kind) -> Result<bool> {
    let [row] = rows.as_slice() else {
        return Err(migu_upstream_error(
            "Migu collection state is not uniquely identified",
        ));
    };
    for (key, expected) in [
        ("resourceId", id),
        ("resourceType", kind.resource_type()),
        ("opType", "03"),
        ("code", "000000"),
    ] {
        if row
            .get(key)
            .is_some_and(|value| value.as_str() != Some(expected))
        {
            return Err(migu_upstream_error(
                "Migu collection state has a different resource or operation",
            ));
        }
    }
    match row.get("isOP").and_then(serde_json::Value::as_str) {
        Some("00") => Ok(true),
        Some("01") => Ok(false),
        _ => Err(migu_upstream_error("Migu collection state is unknown")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn playlist_state(rows: Vec<serde_json::Value>, id: &str) -> Result<bool> {
        state(rows, id, Kind::Playlist)
    }

    #[test]
    fn collection_state_requires_a_unique_known_operation_and_checks_returned_identity() {
        for (flag, expected) in [("00", true), ("01", false)] {
            assert_eq!(
                playlist_state(vec![json!({"isOP":flag})], "77").unwrap(),
                expected
            );
            assert_eq!(
                playlist_state(
                    vec![
                        json!({"isOP":flag,"resourceId":"77","resourceType":"2021","opType":"03"})
                    ],
                    "77"
                )
                .unwrap(),
                expected
            );
        }
        for rows in [
            vec![],
            vec![json!(null)],
            vec![json!({})],
            vec![json!({"isOP":false})],
            vec![json!({"isOP":"00"}), json!({"isOP":"00"})],
            vec![json!({"isOP":"02"})],
            vec![json!({"isOP":"00","resourceId":"88"})],
            vec![json!({"isOP":"00","resourceType":"2003"})],
            vec![json!({"isOP":"00","opType":"08"})],
            vec![json!({"isOP":"00","code":"200000"})],
        ] {
            assert!(playlist_state(rows, "77").is_err());
        }
    }
}
