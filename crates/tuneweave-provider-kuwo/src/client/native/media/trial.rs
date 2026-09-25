//! Independent authorization for the ordinary 30-second MP3 audition branch.
use super::*;

pub(in crate::client::native) const PATH: &str = "/audi.tion";
const HOST: &str = "musicpay30.kuwo.cn";

impl KuwoClient {
    pub(super) async fn fetch_native_trial(
        &self,
        input: &KuwoNativeSessionInput,
        id: &str,
    ) -> Result<Outcome> {
        let query = {
            let mut query = url::form_urlencoded::Serializer::new(String::new());
            query.extend_pairs([
                ("op", "query"),
                ("user", input.device_user()),
                ("prod", "kwplayer_ar_12.2.2.0"),
                ("corp", "kuwo"),
                ("newver", "3"),
                ("vipver", CLIENT_VERSION),
                ("source", CLIENT_SOURCE),
                ("p2p", "1"),
                ("approval", "false"),
                ("loginUid", input.user_id()),
                ("loginSid", input.session_id()),
                ("appuid", input.device_id()),
                ("allpay", "0"),
                ("notrace", "1"),
                ("oaid", ""),
                ("vipMode", "0"),
                ("ids", id),
            ]);
            if let Some(context) = &input.context {
                query.append_pair("android_id", &context.android_id);
            }
            query.finish()
        };
        let target = format!("{}?{query}", self.native_target(HOST, PATH));
        self.native_get(HOST, PATH, "native_account_trial", target, |bytes| {
            parse(bytes, input, id)
        })
        .await
    }
}

#[derive(Deserialize)]
struct Envelope {
    #[serde(deserialize_with = "rights::number")]
    code: u64,
    result: Option<String>,
    songs: Option<Vec<Song>>,
}
#[derive(Deserialize)]
struct Song {
    #[serde(deserialize_with = "rights::number")]
    id: u64,
    #[serde(deserialize_with = "rights::number")]
    duration: u64,
    #[serde(deserialize_with = "rights::number")]
    start: u64,
    #[serde(deserialize_with = "rights::number")]
    end: u64,
    #[serde(deserialize_with = "rights::number")]
    br: u64,
    format: String,
    url: Option<String>,
    https: String,
    ekey: Option<String>,
}

fn denied() -> Outcome {
    Outcome::Denied {
        code: Some(200),
        message: "Kuwo did not authorize an audio preview for this account",
    }
}
fn parse(bytes: &[u8], input: &KuwoNativeSessionInput, id: &str) -> Result<Outcome> {
    let body: Envelope = serde_json::from_slice(bytes).map_err(|_| invalid())?;
    // 407 is the native audition service's location restriction. Unknown
    // business failures are not evidence of an expired account session.
    if body.code == 407 {
        return Ok(Outcome::Denied {
            code: Some(407),
            message: "Kuwo restricted this audio preview in the current location",
        });
    }
    if body.code != 200 || body.result.as_deref() != Some("ok") {
        return Err(invalid());
    }
    let songs = body.songs.ok_or_else(invalid)?;
    if songs.len() > 1 {
        return Err(invalid());
    }
    let Some(song) = songs.into_iter().next() else {
        return Ok(denied());
    };
    if song.id.to_string() != id
        || song.duration == 0
        || song.duration > 86_400
        || song.start >= song.end
        || song.end > song.duration
        || song.end - song.start > 30
        || song.br != 128
        || !song.format.eq_ignore_ascii_case("mp3")
        || song.ekey.as_deref().is_some_and(|s| !s.is_empty())
    {
        return Err(invalid());
    }
    // Use the separately supplied HTTPS location. Neither the HTTP URL nor
    // the different vehicle CDN branch can be promoted into a playback URL.
    if let Some(value) = song.url.as_deref().filter(|s| !s.is_empty()) {
        response::validate_url(value, input, STANDARD)?;
    }
    let url = response::validate_url(&song.https, input, STANDARD)?;
    if url.scheme() != "https" {
        return Err(invalid());
    }
    Ok(Outcome::Allowed {
        url: url.to_string(),
        backups: Vec::new(),
        format: "mp3",
        bitrate: Some(128_000),
        quality: Quality::Standard,
        duration_ms: (song.end - song.start) * 1000,
        key: None,
        trial: Some(TrialWindow {
            start_ms: song.start * 1000,
            end_ms: song.end * 1000,
        }),
    })
}

#[cfg(test)]
pub(crate) mod tests;
