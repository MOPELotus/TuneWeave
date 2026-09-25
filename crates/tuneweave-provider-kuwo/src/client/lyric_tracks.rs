//! Native optional lyric tracks. The native consumer marks the earlier row of
//! an equal-time pair as auxiliary only when the document declares `ml >= 1`.
use super::*;
use tuneweave_core::LyricsRequest;

#[cfg(test)]
mod tests;

const INFO: &str = "https://mobilebasedata.kuwo.cn/api/music/info";
const CONTENT: &str = "https://mlyric.kuwo.cn/mobi.s";
const APP: &str = "kwplayer_ar_12.2.2.0";
const SOURCE: &str = "kwplayer_ar_12.2.2.0_newpcguanwangmobile.apk";

impl KuwoClient {
    pub(crate) async fn lyrics_with_options(
        &self,
        music_id: &str,
        request: &LyricsRequest,
        devices: &KuwoNativeDeviceStore,
    ) -> Result<Lyrics> {
        crate::provider::parse_music_id(music_id)?;
        if request.account.is_some() || request.song_type.is_some() || request.singing_annotations {
            return Err(kuwo_invalid_request(
                "Kuwo public lyric options are invalid",
            ));
        }
        let mut lyrics = self.lyrics(music_id).await?;
        if request.translated || request.romanized {
            let context = async {
                let device = devices.initialize(self).await?;
                let track = self.track_detail(music_id).await?;
                // Validate all context before requesting any optional track.
                content_query(&track, &device, false)?;
                Ok::<_, TuneWeaveError>((device, track))
            }
            .await;
            match context {
                Ok((device, track)) => {
                    self.add_native_lyric_tracks(music_id, request, &mut lyrics, &device, &track)
                        .await
                }
                Err(error) => {
                    lyrics.extensions.insert(
                        "native_lyric_tracks".into(),
                        json!({"context":{"error_code":error.code}}),
                    );
                }
            }
        }
        Ok(lyrics)
    }

    async fn add_native_lyric_tracks(
        &self,
        music_id: &str,
        request: &LyricsRequest,
        lyrics: &mut Lyrics,
        device: &KuwoNativeDevice,
        track: &Track,
    ) {
        let metadata = self
            .native_lyric_request(INFO, &metadata_query(music_id, device))
            .await
            .and_then(|bytes| parse_metadata(&bytes, music_id));
        let mut diagnostics = serde_json::Map::new();
        let support = match metadata {
            Ok(support) => support,
            Err(error) => {
                diagnostics.insert("metadata".into(), json!({"error_code":error.code}));
                lyrics
                    .extensions
                    .insert("native_lyric_tracks".into(), diagnostics.into());
                return;
            }
        };
        // Separate calls preserve the official normal/roma request selectors.
        // Account sessions, stored cookies and native media rights are not used.
        for (requested, romanized, supported, field) in [
            (request.translated, false, support.translated, "translated"),
            (request.romanized, true, support.romanized, "romanized"),
        ] {
            if !requested {
                continue;
            }
            if supported == Some(false) {
                diagnostics.insert(field.into(), json!({"supported":false,"available":false}));
                continue;
            }
            if supported.is_none() {
                diagnostics.insert(field.into(), json!({"supported":null}));
                continue;
            }
            let query = match content_query(track, device, romanized) {
                Ok(query) => query,
                Err(error) => {
                    diagnostics.insert(
                        field.into(),
                        json!({"supported":true,"available":false,"error_code":error.code}),
                    );
                    continue;
                }
            };
            let result = self
                .native_lyric_request(CONTENT, &query)
                .await
                .and_then(|bytes| decode_content(&bytes))
                .and_then(|text| text.map(|text| auxiliary_lines(&text)).transpose())
                .map(Option::flatten);
            match result {
                Ok(text) => {
                    diagnostics.insert(
                        field.into(),
                        json!({"supported":true,"available":text.is_some(),"format":"lrc"}),
                    );
                    if romanized {
                        lyrics.romanized = text;
                    } else {
                        lyrics.translated = text;
                    }
                }
                Err(error) => {
                    diagnostics.insert(
                        field.into(),
                        json!({"supported":true,"available":false,"error_code":error.code}),
                    );
                }
            }
        }
        lyrics
            .extensions
            .insert("native_lyric_tracks".into(), diagnostics.into());
    }

    async fn native_lyric_request(&self, endpoint: &'static str, plain: &str) -> Result<Vec<u8>> {
        let encoded = native::seal_catalog_query(plain.as_bytes())?;
        let target = format!("{}?f=kuwo&q={encoded}", self.web_target(endpoint));
        let started = Instant::now();
        let mut status = None;
        let result = async {
            let response = self
                .http
                .get(target)
                .header(ACCEPT, "application/octet-stream, application/json")
                .send()
                .await
                .map_err(kuwo_network_error)?;
            status = Some(response.status());
            read_bounded_response_with_limit(
                response,
                "Kuwo native lyrics",
                MAX_LYRIC_RESPONSE_BYTES,
            )
            .await
        }
        .await;
        let (host, path) = if endpoint == INFO {
            ("mobilebasedata.kuwo.cn", "/api/music/info")
        } else {
            ("mlyric.kuwo.cn", "/mobi.s")
        };
        self.log_upstream_request(
            "native_lyric_track",
            host,
            path,
            status,
            started,
            0,
            false,
            &result,
        );
        result
    }
}

fn metadata_query(id: &str, device: &KuwoNativeDevice) -> String {
    format!(
        "source={SOURCE}&loginUid=0&loginSid=0&sid=0&prod={APP}&platform=ar&uid={}&corp=kuwo&approval=false&q36=f2ce3c2ef68ddfd1b2bea7ed00001f314716&vipver=12.2.2.0&newver=3&id={id}",
        device.app_uid()
    )
}

fn content_query(track: &Track, device: &KuwoNativeDevice, romanized: bool) -> Result<String> {
    if track.platform != Platform::Kuwo
        || track.resource_ref.platform() != Platform::Kuwo
        || track.resource_ref.id() != track.id
        || track.name.trim().is_empty()
    {
        return Err(invalid());
    }
    crate::provider::parse_music_id(&track.id)?;
    // The existing detail mapper converts upstream seconds to milliseconds.
    // s2.W4 supplies Music.duration * 1000; do not multiply Track.duration_ms again.
    let duration = track
        .duration_ms
        .filter(|value| *value > 0 && *value <= i32::MAX as u64)
        .ok_or_else(invalid)?
        .to_string();
    let artist = track
        .artists
        .iter()
        .map(|artist| artist.name.as_str())
        .collect::<Vec<_>>()
        .join("&");
    if artist.trim().is_empty() {
        return Err(invalid());
    }
    let mut query = url::form_urlencoded::Serializer::new(String::new());
    query.extend_pairs([
        ("user", device.device_user()),
        ("android_id", device.android_id()),
        ("prod", APP),
        ("corp", "kuwo"),
        ("newver", "3"),
        ("vipver", "12.2.2.0"),
        ("source", SOURCE),
        ("p2p", "1"),
        ("q36", "f2ce3c2ef68ddfd1b2bea7ed00001f314716"),
        ("approval", "false"),
        ("loginUid", "0"),
        ("loginSid", "0"),
        ("appuid", device.app_uid()),
        ("allpay", "0"),
        ("notrace", "1"),
        ("oaid", ""),
        ("vipMode", "0"),
        ("type", "lyric"),
        ("songname", track.name.as_str()),
        ("artist", artist.as_str()),
        ("filename", ""),
        ("duration", duration.as_str()),
        ("req", "2"),
        ("lrcx", "1"),
        ("rid", track.id.as_str()),
        ("encode", "utf8"),
    ]);
    if romanized {
        query.append_pair("trans_type", "roma");
    }
    Ok(query.finish())
}

#[derive(Deserialize)]
struct Metadata {
    code: FlexibleText,
    data: MetadataData,
}
#[derive(Deserialize)]
struct MetadataData {
    id: FlexibleText,
    hasdlrc: Option<u8>,
    hasdlrcx: Option<u8>,
    lrc_info: Option<TrackFlags>,
}
#[derive(Deserialize)]
struct TrackFlags {
    lrc_roma: Option<u8>,
    lrcx_roma: Option<u8>,
}
struct Support {
    translated: Option<bool>,
    romanized: Option<bool>,
}
fn flags(a: Option<u8>, b: Option<u8>) -> Result<Option<bool>> {
    if a.is_some_and(|value| value > 1) || b.is_some_and(|value| value > 1) {
        return Err(invalid());
    }
    Ok(if a == Some(1) || b == Some(1) {
        Some(true)
    } else if a == Some(0) && b == Some(0) {
        Some(false)
    } else {
        None
    })
}
fn parse_metadata(bytes: &[u8], id: &str) -> Result<Support> {
    let metadata: Metadata = serde_json::from_slice(bytes).map_err(|_| invalid())?;
    if metadata.code.as_i64() != Some(200) || metadata.data.id.as_text().as_deref() != Some(id) {
        return Err(invalid());
    }
    let data = metadata.data;
    let romanized = data
        .lrc_info
        .map(|info| flags(info.lrc_roma, info.lrcx_roma))
        .transpose()?
        .flatten();
    Ok(Support {
        translated: flags(data.hasdlrc, data.hasdlrcx)?,
        romanized,
    })
}

fn decode_content(bytes: &[u8]) -> Result<Option<String>> {
    if matches!(bytes, b"TP=none\r\n" | b"TP=none\r\n\r\n") {
        return Ok(None);
    }
    let (compressed, word_synced) =
        if let Some(body) = bytes.strip_prefix(b"TP=content\r\nlrcx=1\r\n\r\n") {
            (body, true)
        } else if let Some(body) = bytes.strip_prefix(b"TP=content\r\nlrcx=0\r\n\r\n") {
            (body, false)
        } else {
            return Err(invalid());
        };
    let mut decoder = flate2::Decompress::new(true);
    let mut decoded = Vec::new();
    let mut chunk = [0_u8; 8192];
    loop {
        let before_in = decoder.total_in();
        let before_out = decoder.total_out();
        let status = decoder
            .decompress(
                &compressed[before_in as usize..],
                &mut chunk,
                flate2::FlushDecompress::Finish,
            )
            .map_err(|_| invalid())?;
        let produced = (decoder.total_out() - before_out) as usize;
        if decoder.total_out() > MAX_LYRIC_DECOMPRESSED_BYTES {
            return Err(invalid());
        }
        decoded.extend_from_slice(&chunk[..produced]);
        if status == flate2::Status::StreamEnd {
            if decoder.total_in() != compressed.len() as u64 {
                return Err(invalid());
            }
            break;
        }
        if decoder.total_in() == before_in && produced == 0 {
            return Err(invalid());
        }
    }
    if word_synced {
        let encoded = std::str::from_utf8(&decoded).map_err(|_| invalid())?;
        decoded = BASE64_STANDARD
            .decode(encoded.trim())
            .map_err(|_| invalid())?;
        for (index, byte) in decoded.iter_mut().enumerate() {
            *byte ^= LYRIC_XOR_KEY[index % LYRIC_XOR_KEY.len()];
        }
    }
    let text = String::from_utf8(decoded)
        .map_err(|_| invalid())?
        .replace("&apos;", "'")
        .replace("&amp;", "&")
        .replace("&quot;", "\"");
    validate_lyric_text(&text)?;
    Ok(Some(text))
}

fn auxiliary_lines(text: &str) -> Result<Option<String>> {
    let plain = derive_plain_from_lrcx(text)?;
    let mut multilingual = None;
    let mut rows = Vec::new();
    for line in plain.lines().map(str::trim).filter(|line| !line.is_empty()) {
        if let Some(value) = line.strip_prefix("[ml:").and_then(|s| s.strip_suffix(']')) {
            let value = value.parse::<f32>().map_err(|_| invalid())?;
            if !value.is_finite()
                || multilingual.replace(value >= 1.0).is_some()
                || !rows.is_empty()
            {
                return Err(invalid());
            }
        } else if line.as_bytes().get(1).is_some_and(u8::is_ascii_digit) {
            let (time, value) = parse_line(line)?;
            if rows.last().is_some_and(|(previous, _)| *previous > time) || rows.len() >= 10_000 {
                return Err(invalid());
            }
            rows.push((time, value));
        }
    }
    // Absence of the official mixed-language declaration is not proof that the
    // document is an independent translated/romanized track.
    if multilingual != Some(true) || rows.is_empty() {
        return Err(invalid());
    }
    let mut output = String::new();
    let mut index = 0;
    while index < rows.len() {
        let (time, first) = rows[index];
        let mut end = index + 1;
        while end < rows.len() && rows[end].0 == time {
            end += 1;
        }
        if end - index > 2 {
            return Err(invalid());
        }
        if end - index == 2 && !first.is_empty() {
            if rows[index + 1].1.is_empty() || first == rows[index + 1].1 {
                return Err(invalid());
            }
            if !output.is_empty() {
                output.push('\n');
            }
            write!(
                &mut output,
                "[{:02}:{:02}.{:03}]{}",
                time / 60_000,
                time % 60_000 / 1_000,
                time % 1_000,
                first
            )
            .map_err(|_| invalid())?;
        }
        index = end;
    }
    Ok((!output.is_empty()).then_some(output))
}

fn parse_line(line: &str) -> Result<(u32, &str)> {
    let (stamp, value) = line
        .strip_prefix('[')
        .and_then(|s| s.split_once(']'))
        .ok_or_else(invalid)?;
    let (minutes, rest) = stamp.split_once(':').ok_or_else(invalid)?;
    let (seconds, fraction) = rest
        .split_once('.')
        .map_or((rest, None), |(s, f)| (s, Some(f)));
    let number = |text: &str, min: usize, max: usize| -> Result<u32> {
        if !(min..=max).contains(&text.len()) || !text.bytes().all(|b| b.is_ascii_digit()) {
            return Err(invalid());
        }
        text.parse().map_err(|_| invalid())
    };
    let minutes = number(minutes, 1, 2)?;
    let seconds = number(seconds, 1, 2)?;
    let millis = fraction
        .map(|fraction| {
            number(fraction, 1, 4).map(|value| {
                if fraction.len() <= 2 {
                    value * 10
                } else {
                    value
                }
            })
        })
        .transpose()?
        .unwrap_or(0);
    if seconds > 59 || millis > 999 || value.starts_with('[') || value.len() > 4096 {
        return Err(invalid());
    }
    Ok(((minutes * 60 + seconds) * 1000 + millis, value.trim()))
}
fn invalid() -> TuneWeaveError {
    kuwo_upstream_error("Kuwo native lyric track metadata or content was invalid")
}
