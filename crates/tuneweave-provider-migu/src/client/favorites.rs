use super::*;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct StateResponse {
    is_infavors: Vec<State>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct State {
    content_id: String,
    is_infavor: String,
}

pub(super) fn state(fields: serde_json::Map<String, serde_json::Value>, id: &str) -> Result<bool> {
    let response: StateResponse = serde_json::from_value(serde_json::Value::Object(fields))
        .map_err(|_| migu_upstream_error("Migu favorite state response is invalid"))?;
    let [item] = response.is_infavors.as_slice() else {
        return Err(migu_upstream_error(
            "Migu favorite state is not uniquely identified",
        ));
    };
    if item.content_id != id {
        return Err(migu_upstream_error(
            "Migu favorite state belongs to another track",
        ));
    }
    match item.is_infavor.as_str() {
        "0" => Ok(false),
        "1" => Ok(true),
        _ => Err(migu_upstream_error("Migu favorite state is unknown")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_favorite_state_requires_one_matching_track_and_known_boolean_encoding() {
        for (flag, expected) in [("0", false), ("1", true)] {
            let fields = json!({"isInfavors":[{"contentId":"abc12","isInfavor":flag}]})
                .as_object()
                .unwrap()
                .clone();
            assert_eq!(state(fields, "abc12").unwrap(), expected);
        }
        for value in [
            json!({}),
            json!({"isInfavors":null}),
            json!({"isInfavors":[]}),
            json!({"isInfavors":[{"contentId":"other","isInfavor":"0"}]}),
            json!({"isInfavors":[{"contentId":"abc12","isInfavor":"0"},{"contentId":"abc12","isInfavor":"0"}]}),
            json!({"isInfavors":[{"contentId":"abc12","isInfavor":"2"}]}),
            json!({"isInfavors":[{"contentId":"abc12","isInfavor":false}]}),
            json!({"isInfavors":[{"contentId":"abc12"}]}),
        ] {
            assert!(state(value.as_object().unwrap().clone(), "abc12").is_err());
        }
    }
}
