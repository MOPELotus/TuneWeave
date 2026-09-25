use super::*;
use base64::engine::general_purpose::STANDARD;

const UPLOAD_HOST: &str = "wapi.kuwo.cn";
const PATH: &str = "/openapi/v1/playlist/upload/playlistPic";
pub(super) struct Uploaded {
    pub url: String,
    pub thumbnail: Option<String>,
}

impl KuwoClient {
    pub(super) async fn upload_native_cover(
        &self,
        input: &KuwoNativeSessionInput,
        prepared: &image::Prepared,
        dispatched: &mut bool,
    ) -> Result<Uploaded> {
        let key = self.native_response_key()?;
        let query = query(input, &key)?;
        let request = self
            .http
            .post(format!("{}?{query}", self.native_target(UPLOAD_HOST, PATH)))
            .header(ACCEPT, "application/json")
            .header(CONTENT_TYPE, "application/json")
            .header("Cookies", session_metadata(input)?)
            .body(
                serde_json::to_vec(&json!({"params":STANDARD.encode(&prepared.jpeg)}))
                    .map_err(|_| invalid())?,
            );
        let started = Instant::now();
        let mut status = None;
        *dispatched = true;
        let result = async {
            let response = request
                .send()
                .await
                .map_err(|e| kuwo_network_error(e).retryable(false))?;
            status = Some(response.status());
            let bytes = read_response(response, false, false, ACK_LIMIT).await?;
            parse(&bytes, input, &key)
        }
        .await;
        self.log_upstream_request(
            "native_playlist_cover_upload",
            UPLOAD_HOST,
            PATH,
            status,
            started,
            0,
            false,
            &result,
        );
        result
    }
}
fn query(input: &KuwoNativeSessionInput, key: &[u8; 8]) -> Result<String> {
    let plain = serde_json::to_vec(&json!({"from":"android","dev_name":"TuneWeave",
        "dev_id":input.device_user(),"uid":input.user_id(),"sid":input.session_id(),
        "sx":std::str::from_utf8(key).map_err(|_|invalid())?,
        // Official width is the device display width. This SDK has no display.
        "params":{"fileExt":"jpg","width":0}}))
    .map_err(|_| invalid())?;
    let mut q = url::form_urlencoded::Serializer::new(String::new());
    q.extend_pairs([
        ("appkey", "q7idgad1fr5t"),
        ("apiVer", "1"),
        ("source", CLIENT_SOURCE),
        ("loginUid", input.user_id()),
        ("loginSid", input.session_id()),
        ("prod", "kwplayer_ar_12.2.2.0"),
        ("platform", "ar"),
        ("uid", input.device_id()),
        ("corp", "kuwo"),
        ("q36", device::FALLBACK_Q36),
        ("approval", "false"),
        ("vipver", CLIENT_VERSION),
        ("newver", "3"),
        ("allpay", "0"),
        ("notrace", "1"),
        ("oaid", ""),
        ("vipMode", "0"),
    ]);
    // Native encrypted q is appended as raw standard Base64, as by s2.Ea/y3.
    Ok(format!(
        "{}&q={}",
        q.finish(),
        codec::seal_image_query(&plain)?
    ))
}
#[derive(Deserialize)]
struct Envelope {
    #[serde(default, deserialize_with = "deserialize_code")]
    status: Option<String>,
    data: Option<String>,
}
#[derive(Deserialize)]
struct Pictures {
    #[serde(rename = "picUrl")]
    url: String,
    #[serde(rename = "picThumbUrl")]
    thumbnail: Option<String>,
}
fn parse(bytes: &[u8], input: &KuwoNativeSessionInput, key: &[u8; 8]) -> Result<Uploaded> {
    let response: Envelope = serde_json::from_slice(bytes).map_err(|_| invalid())?;
    if response.status.as_deref() != Some("200") {
        return Err(invalid());
    }
    let plain = codec::open_response(
        response.data.as_deref().ok_or_else(invalid)?.as_bytes(),
        key,
    )?;
    let text = std::str::from_utf8(&plain).map_err(|_| invalid())?;
    let picture = if text.starts_with("https://") || text.starts_with("http://") {
        Pictures {
            url: text.to_owned(),
            thumbnail: None,
        }
    } else {
        serde_json::from_slice::<Pictures>(&plain).map_err(|_| invalid())?
    };
    Ok(Uploaded {
        url: library::dto::picture(Some(picture.url), input)?.ok_or_else(invalid)?,
        thumbnail: library::dto::picture(picture.thumbnail, input)?,
    })
}
fn invalid() -> TuneWeaveError {
    kuwo_upstream_error("Kuwo cover upload response is invalid or rejected")
}
