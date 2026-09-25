use std::collections::BTreeSet;

use super::*;
use tuneweave_core::{Artist, Extensions, Platform, ResourceRef};

const ENDPOINT: &str = "https://wapi.kuwo.cn/api/fmradio/bcsy_list";
const PATH: &str = "/api/fmradio/bcsy_list";
const MAX_ANCHORS: usize = 512;

#[derive(Deserialize)]
struct Envelope {
    code: i64,
    data: Option<Data>,
}

#[derive(Deserialize)]
struct Data {
    list: Vec<Anchor>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Anchor {
    id: FlexibleText,
    artist_name: String,
    head_pic: Option<String>,
    channel_id: FlexibleText,
    channel_name: Option<String>,
    priority: Option<FlexibleText>,
    status: Option<FlexibleText>,
}

pub(super) async fn read(client: &KuwoClient) -> Result<Vec<Artist>> {
    let started = Instant::now();
    let mut status = None;
    let result = async {
        let response = client
            .http
            .get(client.web_target(ENDPOINT))
            .header(ACCEPT, "application/json")
            .send()
            .await
            .map_err(kuwo_network_error)?;
        status = Some(response.status());
        let bytes = catalog::read_response(response).await?;
        parse(&bytes)
    }
    .await;
    client.log_upstream_request(
        "artist_catalog_baicheng_anchors",
        "wapi.kuwo.cn",
        PATH,
        status,
        started,
        0,
        false,
        &result,
    );
    result
}

fn parse(bytes: &[u8]) -> Result<Vec<Artist>> {
    let invalid = || kuwo_upstream_error("Kuwo anchor catalogue returned invalid metadata");
    let envelope: Envelope = serde_json::from_slice(bytes).map_err(|_| invalid())?;
    if envelope.code != 200 {
        return Err(invalid());
    }
    let data = envelope.data.ok_or_else(invalid)?;
    if data.list.len() > MAX_ANCHORS {
        return Err(invalid());
    }

    let mut seen = BTreeSet::new();
    let mut artists = Vec::with_capacity(data.list.len());
    for entry in data.list {
        let id = entry.id.as_text().ok_or_else(invalid)?;
        if canonical_positive_decimal(&id) != Some(id.as_str()) || !seen.insert(id.clone()) {
            return Err(invalid());
        }
        let channel_id = entry.channel_id.as_text().ok_or_else(invalid)?;
        let channel_number = channel_id
            .parse::<u64>()
            .ok()
            .filter(|n| n.to_string() == channel_id)
            .ok_or_else(invalid)?;
        let name = catalog::text(&entry.artist_name, 512, false)?;
        if name.is_empty() {
            return Err(invalid());
        }

        let mut extensions = Extensions::from([
            ("backend".into(), json!("native_bcsy")),
            ("catalog_scope".into(), json!("baicheng_sound_anchors")),
            ("source_type".into(), json!("anchor")),
            ("channel_id".into(), json!(channel_id)),
        ]);
        if channel_number > 0 {
            let channel_ref = ResourceRef::new(Platform::Kuwo, format!("fm:{channel_number}"))
                .map_err(|_| invalid())?;
            extensions.insert("radio_channel_ref".into(), json!(channel_ref.to_string()));
        }
        if let Some(channel_name) = entry.channel_name.as_deref() {
            catalog::optional_text(&mut extensions, "channel_name", Some(channel_name))?;
        }
        for (key, raw) in [
            ("source_priority", entry.priority),
            ("source_status", entry.status),
        ] {
            if let Some(raw) = raw.and_then(|value| value.as_text()) {
                catalog::optional_text(&mut extensions, key, Some(&raw))?;
            }
        }
        artists.push(Artist {
            resource_ref: ResourceRef::new(Platform::Kuwo, id.clone()).map_err(|_| invalid())?,
            platform: Platform::Kuwo,
            id,
            name,
            aliases: Vec::new(),
            description: String::new(),
            biography_sections: Vec::new(),
            avatar_url: entry.head_pic.as_deref().and_then(catalog::artist_image),
            cover_url: None,
            album_count: None,
            track_count: None,
            mv_count: None,
            video_count: None,
            identities: Vec::new(),
            extensions,
        });
    }
    Ok(artists)
}

#[cfg(test)]
#[path = "bcsy_tests.rs"]
mod tests;
