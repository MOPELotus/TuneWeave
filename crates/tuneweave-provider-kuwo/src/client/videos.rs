use super::*;
use catalog::Unsigned;
use tuneweave_core::{CreatorSummary, Video, VideoDetail, VideoResourceKind};

mod mv_catalog;

#[derive(Serialize)]
struct Query<'a> {
    mid: &'a str,
    ip: &'static str,
    cip: &'static str,
    #[serde(rename = "httpsStatus")]
    https_status: u8,
    #[serde(rename = "reqId")]
    request_id: String,
    plat: &'static str,
    from: &'static str,
}

impl KuwoClient {
    pub(crate) async fn mv_detail(&self, id: &str) -> Result<VideoDetail> {
        for refresh in [false, true] {
            let response = self
                .signed_get(
                    KuwoSignedEndpoint::MvDetail,
                    &Query {
                        mid: id,
                        ip: "",
                        cip: "",
                        https_status: 1,
                        request_id: new_request_id(),
                        plat: "web_www",
                        from: "",
                    },
                    &format!("https://www.kuwo.cn/mvplay/{id}"),
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
                    return parse(&bytes, id);
                }
            }
        }
        Err(invalid())
    }
}

#[derive(Deserialize)]
struct Envelope {
    code: i64,
    data: Option<Detail>,
}
#[derive(Deserialize)]
struct Detail {
    #[serde(flatten)]
    track: KuwoTrackDetail,
    disable: Option<Unsigned>,
}

fn parse(bytes: &[u8], id: &str) -> Result<VideoDetail> {
    let root: Envelope = serde_json::from_slice(bytes).map_err(|_| invalid())?;
    if root.code == -1 && root.data.is_none() {
        return Err(missing());
    }
    if root.code != 200 {
        return Err(invalid().with_details(json!({"upstream_code":root.code})));
    }
    let data = root.data.ok_or_else(invalid)?;
    let detail = data.track;
    let rid = detail.rid.as_text().ok_or_else(invalid)?;
    if canonical_positive_decimal(&rid) != Some(id)
        || detail
            .musicrid
            .strip_prefix("MUSIC_")
            .and_then(canonical_positive_decimal)
            != Some(id)
    {
        return Err(invalid());
    }
    match number(&detail.hasmv)? {
        Some(0) => return Err(missing()),
        Some(1) => {}
        _ => return Err(invalid()),
    }
    if number(&detail.content_type)?.is_some_and(|n| n != 0)
        || !matches!(detail.ad_type.as_str(), "" | "0")
    {
        return Err(invalid());
    }
    let info = detail.mvpayinfo.ok_or_else(invalid)?;
    let vid = info.vid.as_text().ok_or_else(invalid)?;
    canonical_positive_decimal(&vid).ok_or_else(invalid)?;
    let title = catalog::text(&detail.name, 512, false)?;
    if title.is_empty() {
        return Err(invalid());
    }
    let creators = artists::credits(
        &detail.artist,
        &detail.artistid.as_text().ok_or_else(invalid)?,
    )?
    .into_iter()
    .map(|credit| CreatorSummary {
        resource_ref: credit.resource_ref,
        name: credit.name,
        avatar_url: None,
    })
    .collect();
    let mut rights = serde_json::Map::from_iter([("vid".into(), json!(vid))]);
    for (key, value) in [
        ("play", &info.play),
        ("down", &info.down),
        ("download", &info.download),
    ] {
        if let Some(code) = number(value)? {
            rights.insert(key.into(), json!(code));
        }
    }
    let mut extensions = Extensions::from([
        ("backend".into(), json!("current_web_mv_music_info")),
        ("kind".into(), json!("mv")),
        ("source_track_id".into(), json!(id)),
        ("mv_pay_info".into(), json!(rights)),
    ]);
    if let Some(online) = number(&detail.online)? {
        if online > 1 {
            return Err(invalid());
        }
        extensions.insert("online".into(), json!(online));
    }
    if let Some(disable) = data.disable {
        let disable = disable.value()?;
        if disable > 1 {
            return Err(invalid());
        }
        extensions.insert("disable".into(), json!(disable));
    }
    // These fields describe the associated song/album, not an MV publication date.
    if let Some(date) = catalog::date(&detail.release_date)? {
        extensions.insert("source_track_release_date".into(), json!(date));
    }
    let duration_ms = number(&detail.duration)?
        .map(|seconds| seconds.checked_mul(1000).ok_or_else(invalid))
        .transpose()?;
    Ok(VideoDetail {
        kind: VideoResourceKind::Mv,
        video: Video {
            resource_ref: kuwo_track_ref(id)?,
            platform: Platform::Kuwo,
            id: id.to_owned(),
            title,
            creators,
            description: String::new(),
            cover_url: [
                detail.pic.as_str(),
                detail.pic120.as_str(),
                detail.albumpic.as_str(),
            ]
            .into_iter()
            .find_map(normalize_official_image_url),
            duration_ms,
            published_at: None,
            play_count: number(&detail.mv_play_count)?,
            subscribed: None,
            extensions,
        },
        resolutions: vec![],
        extensions: Extensions::from([("backend".into(), json!("current_web_mv_music_info"))]),
    })
}

fn number(value: &FlexibleText) -> Result<Option<u64>> {
    match value {
        FlexibleText::Null => Ok(None),
        FlexibleText::Number(value) => value.as_u64().map(Some).ok_or_else(invalid),
        FlexibleText::String(value) => {
            let number = value.parse::<u64>().map_err(|_| invalid())?;
            if number.to_string() != *value {
                return Err(invalid());
            }
            Ok(Some(number))
        }
        FlexibleText::Boolean(_) => Err(invalid()),
    }
}
fn missing() -> TuneWeaveError {
    TuneWeaveError::new(ErrorCode::ResourceNotFound, "Kuwo MV was not found")
        .with_platform(Platform::Kuwo)
}
fn invalid() -> TuneWeaveError {
    kuwo_upstream_error("Kuwo MV returned invalid or incomplete metadata")
}

#[cfg(test)]
mod tests;
