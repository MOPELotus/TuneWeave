use super::dto::Number;
use super::*;

#[derive(Deserialize)]
pub(super) struct Audio {
    hash: Option<String>,
    hash_128: Option<String>,
    hash_320: Option<String>,
    hash_flac: Option<String>,
    hash_high: Option<String>,
    hash_super: Option<String>,
    #[serde(alias = "timelength")]
    duration: Option<Number>,
    #[serde(alias = "timelength_128")]
    duration_128: Option<Number>,
    #[serde(alias = "timelength_320")]
    duration_320: Option<Number>,
    #[serde(alias = "timelength_flac")]
    duration_flac: Option<Number>,
    #[serde(alias = "timelength_high")]
    duration_high: Option<Number>,
    #[serde(alias = "timelength_super")]
    duration_super: Option<Number>,
    filesize: Option<Number>,
    filesize_128: Option<Number>,
    filesize_320: Option<Number>,
    filesize_flac: Option<Number>,
    filesize_high: Option<Number>,
    filesize_super: Option<Number>,
    bitrate: Option<Number>,
    bitrate_high: Option<Number>,
    bitrate_flac: Option<Number>,
    bitrate_super: Option<Number>,
    extname: Option<String>,
    extname_super: Option<String>,
}
fn number(value: Option<Number>) -> Option<u64> {
    value.map(|v| v.0).filter(|v| *v > 0)
}
fn hash(value: Option<String>) -> Result<Option<String>> {
    value
        .filter(|v| !v.is_empty())
        .map(|v| {
            if v.len() != 32 || !v.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err(invalid());
            }
            Ok(v)
        })
        .transpose()
}
fn container(value: Option<String>) -> Result<Option<&'static str>> {
    match value.as_deref() {
        None | Some("") => Ok(None),
        Some("mp3") => Ok(Some("mp3")),
        Some("flac") => Ok(Some("flac")),
        Some("ape") => Ok(Some("ape")),
        Some("dff") => Ok(Some("dff")),
        _ => Err(invalid()),
    }
}
pub(super) fn map_audio(a: Audio, track: &mut Track) -> Result<()> {
    let primary = hash(a.hash)?;
    let standard = hash(a.hash_128)?.or(primary);
    let high = hash(a.hash_320)?;
    let lossless = hash(a.hash_flac)?;
    let hires = hash(a.hash_high)?;
    let master = hash(a.hash_super)?;
    let standard_duration = number(a.duration_128);
    let duration = number(a.duration).or(standard_duration);
    track.duration_ms = duration;
    let mut qualities = BTreeMap::new();
    // These are catalogue asset descriptions. Playback still performs fresh authorization.
    for (key, quality, h, size, bitrate, length, format) in [
        (
            "standard",
            Quality::Standard,
            standard.clone(),
            number(a.filesize_128).or(number(a.filesize)),
            number(a.bitrate),
            standard_duration.or(duration),
            container(a.extname)?,
        ),
        (
            "high",
            Quality::High,
            high,
            number(a.filesize_320),
            Some(320),
            number(a.duration_320),
            Some("mp3"),
        ),
        (
            "lossless",
            Quality::Lossless,
            lossless,
            number(a.filesize_flac),
            number(a.bitrate_flac),
            number(a.duration_flac),
            Some("flac"),
        ),
        (
            "hires",
            Quality::Hires,
            hires,
            number(a.filesize_high),
            number(a.bitrate_high),
            number(a.duration_high),
            Some("flac"),
        ),
        (
            "master",
            Quality::Master,
            master,
            number(a.filesize_super),
            number(a.bitrate_super),
            number(a.duration_super),
            container(a.extname_super)?,
        ),
    ] {
        if let Some(h) = h {
            let mut asset = json!({"hash":h,"size":size,"bitrate":bitrate,"duration_ms":length});
            if let Some(format) = format {
                asset["format"] = json!(format);
            }
            qualities.insert(key, asset);
            track.available_qualities.push(quality);
        }
    }
    if let Some(h) = standard {
        track.extensions.insert("hash".into(), json!(h));
    }
    if !qualities.is_empty() {
        track
            .extensions
            .insert("qualities".into(), json!(qualities));
    }
    Ok(())
}

fn invalid() -> TuneWeaveError {
    kugou_upstream_error("KuGou catalogue returned invalid audio asset metadata")
}
