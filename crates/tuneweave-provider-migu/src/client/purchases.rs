use super::*;

const MAX_IDS: usize = 10_000;

pub(super) fn parse_ids(value: serde_json::Value) -> Result<Vec<String>> {
    let ids = value
        .as_array()
        .ok_or_else(|| migu_upstream_error("Migu purchased tracks omitted the complete ID list"))?;
    if ids.len() > MAX_IDS {
        return Err(migu_upstream_error(
            "Migu purchased tracks exceeded the complete-read budget",
        ));
    }
    ids.iter()
        .map(|value| {
            let id = value.as_str().ok_or_else(|| {
                migu_upstream_error("Migu purchased track identity is not a string")
            })?;
            if id.is_empty() || id.len() > 64 || !id.bytes().all(|b| b.is_ascii_alphanumeric()) {
                return Err(migu_upstream_error(
                    "Migu purchased track identity is invalid",
                ));
            }
            Ok(id.to_owned())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn purchases_ids_require_complete_typed_bounded_array_and_preserve_order() {
        assert_eq!(
            parse_ids(json!(["123", "ABC2", "123"])).unwrap(),
            ["123", "ABC2", "123"]
        );
        assert!(parse_ids(json!([])).unwrap().is_empty());
        assert_eq!(
            parse_ids(json!(vec!["123"; MAX_IDS])).unwrap().len(),
            MAX_IDS
        );
        for value in [
            json!(null),
            json!({"items":[]}),
            json!([123]),
            json!([null]),
            json!([{}]),
            json!([""]),
            json!(["migu:123"]),
            json!(["123 "]),
            json!(["a".repeat(65)]),
            json!(vec!["123"; MAX_IDS + 1]),
        ] {
            assert_eq!(parse_ids(value).unwrap_err().code, ErrorCode::UpstreamError);
        }
    }
}
