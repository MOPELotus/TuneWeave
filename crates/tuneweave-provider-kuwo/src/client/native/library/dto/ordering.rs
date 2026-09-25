//! Bounded known directory fields used only to verify a library-order write.
use super::*;
use std::collections::BTreeMap;

pub(in crate::client::native) struct Snapshot {
    pub(in crate::client::native) positions: BTreeMap<String, Option<i32>>,
    pub(in crate::client::native) stable_rows: BTreeMap<Vec<u8>, usize>,
}
#[derive(Deserialize)]
struct Directory {
    plist: Vec<Row>,
}
#[derive(Deserialize, serde::Serialize)]
struct Row {
    #[serde(flatten)]
    known: Owned,
    #[serde(default, deserialize_with = "unsigned")]
    recommend: Option<u64>,
    // System lists can be present without a cloud ID. Keep this known flag as a
    // bounded scalar; it is never returned or used to construct a write.
    hidden: Option<serde_json::Value>,
}

pub(in crate::client::native) fn parse(
    bytes: &[u8],
    input: &KuwoNativeSessionInput,
) -> Result<Snapshot> {
    // Preserve all existing response, ownership, type, ID and ordinary/Favorite
    // validation. Excluded system rows still participate in the comparison below.
    super::parse(bytes, input, Section::Owned)?;
    all_owned_ids(bytes)?;
    let directory: Directory = serde_json::from_slice(bytes).map_err(|_| invalid())?;
    let mut positions = BTreeMap::new();
    let mut stable_rows = BTreeMap::new();
    for mut row in directory.plist {
        for (value, limit, multiline) in [
            (&row.known.title, 1024, false),
            (&row.known.info, 16384, true),
            (&row.known.pic, 2048, false),
        ] {
            text(value.clone(), limit, multiline, input)?;
        }
        if let Some(value) = &row.hidden {
            match value {
                serde_json::Value::Bool(_) => (),
                serde_json::Value::Number(n) if n.as_u64().is_some() => (),
                serde_json::Value::String(s) => {
                    text(Some(s.clone()), 32, false, input)?;
                }
                _ => return Err(invalid()),
            }
        }
        if row.known.kind == "GENERAL" {
            let id = row.known.id.as_ref().ok_or_else(invalid)?.clone();
            if positions.insert(id, row.known.turn).is_some() {
                return Err(invalid());
            }
            row.known.turn = None;
        }
        // Unknown fields, tokens and installation metadata are never copied.
        let key = serde_json::to_vec(&row).map_err(|_| invalid())?;
        *stable_rows.entry(key).or_default() += 1;
    }
    Ok(Snapshot {
        positions,
        stable_rows,
    })
}
