use super::*;

const HOST: &str = "app.c.nf.migu.cn";
const PATH: &str = "/MIGUM3.0/v1.0/template/musiclistplaza-taglist/release";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PlaylistTag {
    pub tag_id: String,
    pub tag_name: String,
}

#[derive(Clone, Copy)]
pub(crate) enum TagChange<'a> {
    Add(&'a PlaylistTag),
    Remove(&'a PlaylistTag),
}

impl MiguClient {
    pub(crate) async fn playlist_tag_catalogue(&self) -> Result<Vec<PlaylistTag>> {
        let started = Instant::now();
        let mut status = None;
        let result = async {
            let response = self
                .http
                .get(self.catalog_endpoint(&format!("https://{HOST}{PATH}"))?)
                .query(&[("templateVersion", "1")])
                .header(ACCEPT, "application/json")
                .send()
                .await
                .map_err(migu_network_error)?;
            status = Some(response.status());
            let bytes = read_bounded_response(response, "Migu playlist tag catalogue").await?;
            parse_catalogue(&bytes)
        }
        .await;
        self.log_upstream_request(
            "playlist_tag_catalogue",
            HOST,
            PATH,
            status,
            started,
            &result,
        );
        result
    }
}

fn parse_catalogue(bytes: &[u8]) -> Result<Vec<PlaylistTag>> {
    #[derive(Deserialize)]
    struct Envelope {
        code: String,
        data: Vec<Group>,
    }
    #[derive(Deserialize)]
    struct Group {
        content: Vec<Entry>,
    }
    #[derive(Deserialize)]
    struct Entry {
        texts: Vec<String>,
    }
    let invalid = || migu_upstream_error("Migu playlist tag catalogue is invalid or ambiguous");
    let envelope: Envelope = serde_json::from_slice(bytes).map_err(|_| invalid())?;
    if envelope.code != "000000"
        || envelope.data.is_empty()
        || envelope.data.len() > 32
        || envelope
            .data
            .iter()
            .map(|group| group.content.len())
            .sum::<usize>()
            > 2048
    {
        return Err(invalid());
    }
    let mut names = BTreeMap::new();
    let mut ids = BTreeMap::new();
    for entry in envelope.data.into_iter().flat_map(|group| group.content) {
        if !(2..=16).contains(&entry.texts.len()) {
            return Err(invalid());
        }
        // Official LabelMainPageBean.Content reads title/id from texts[0]/[1].
        // Other presentation fields and action URLs do not authorize a tag ID.
        let tag_name = &entry.texts[0];
        let tag_id = &entry.texts[1];
        let numeric = tag_id.parse::<u64>().ok();
        if numeric.is_none_or(|value| value == 0 || value.to_string() != *tag_id)
            || tag_name.trim().is_empty()
            || tag_name.len() > 256
            || tag_name.chars().any(|c| c.is_control() || c == '|')
            || names.get(tag_name).is_some_and(|old| old != tag_id)
            || ids.get(tag_id).is_some_and(|old| old != tag_name)
        {
            return Err(invalid());
        }
        names.insert(tag_name.clone(), tag_id.clone());
        ids.insert(tag_id.clone(), tag_name.clone());
    }
    if names.is_empty() {
        return Err(invalid());
    }
    Ok(names
        .into_iter()
        .map(|(tag_name, tag_id)| PlaylistTag { tag_id, tag_name })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_playlist_tag_catalogue_deduplicates_only_identical_official_id_name_pairs() {
        let value = json!({"code":"000000","data":[
            {"content":[{"texts":["流行","1000001672","ignored display value"]},{"texts":["摇滚","1000001679"]}]},
            {"content":[{"texts":["流行","1000001672"]}]}
        ]});
        let tags = parse_catalogue(&serde_json::to_vec(&value).unwrap()).unwrap();
        assert_eq!(tags.len(), 2);
        assert!(
            tags.iter()
                .any(|tag| tag.tag_id == "1000001672" && tag.tag_name == "流行")
        );
        for other in [
            json!(["流行", "99"]),
            json!(["其他", "1000001672"]),
            json!(["危险|名称", "99"]),
            json!(["缺少ID"]),
            json!(["测试", "099"]),
        ] {
            let mut invalid = value.clone();
            invalid["data"][1]["content"][0]["texts"] = other;
            assert!(parse_catalogue(&serde_json::to_vec(&invalid).unwrap()).is_err());
        }
    }
}
