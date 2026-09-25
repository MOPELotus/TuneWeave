use super::account::read_session_body_limited;
use super::*;
use crate::credential::{error, validate_token};
use reqwest::header::{CONTENT_TYPE, HeaderValue};

const KEY: &[u8] = b"ccTWaprX2aWmTIgA";
// Public DER certificate prefix used by the official 8.9.1 signing library.
const CERTIFICATE_PREFIX: &[u8] = b"308202333082019ca00302010202044d";
const ROUND_CONSTANTS: [u32; 64] = [
    0xd76aa478, 0xe8c7b756, 0x242070db, 0xc1bdceee, 0xf57c0faf, 0x4787c62a, 0xa8304613, 0xfd469501,
    0x698098d8, 0x8b44f7af, 0xffff5bb1, 0x895cd7be, 0x6b901122, 0xfd987193, 0xa679438e, 0x49b40821,
    0xf61e2562, 0xc040b340, 0x265e5a51, 0xe9b6c7aa, 0xd62f105d, 0x02441453, 0xd8a1e681, 0xe7d3fbc8,
    0x21e1cde6, 0xc33707d6, 0xf4d50d87, 0x455a14ed, 0xa9e3e905, 0xfcefa3f8, 0x676f02d9, 0x8d2a4c8a,
    0xfffa3942, 0x8771f681, 0x6d9d6122, 0xfde5380c, 0xa4beea44, 0x4bdecfa9, 0xf6bb4b60, 0xbebfbc70,
    0x289b7ec6, 0xeaa127fa, 0xd4ef3085, 0x04881d05, 0xd9d4d039, 0xe6db99e5, 0x1fa27cf8, 0xc4ac5665,
    0xf4292244, 0x432aff97, 0xab9423a7, 0xfc93a039, 0x655b59c3, 0x8f0ccc92, 0xffeff47d, 0x85845dd1,
    0x6fa87e4f, 0xfe2ce6e0, 0xa3014314, 0x4e0811a1, 0xf7537e82, 0xbd3af235, 0x2ad7d2bb, 0xeb86d391,
];
const SHIFTS: [[u32; 4]; 4] = [
    [7, 12, 17, 22],
    [5, 9, 14, 20],
    [4, 11, 16, 23],
    [6, 10, 15, 21],
];

pub(super) fn timestamp() -> Result<String> {
    Ok(std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| error(ErrorCode::InternalError, "System clock is unavailable"))?
        .as_millis()
        .to_string())
}

pub(crate) fn encode(bytes: &[u8]) -> String {
    hex::encode_upper(
        bytes
            .iter()
            .enumerate()
            .map(|(i, byte)| byte.wrapping_add(KEY[i % KEY.len()]))
            .collect::<Vec<_>>(),
    )
}

fn decode(bytes: &[u8]) -> Result<Vec<u8>> {
    if bytes.is_empty() || bytes.len() % 2 != 0 || !bytes.iter().all(u8::is_ascii_hexdigit) {
        return Err(migu_upstream_error(
            "Migu native response encoding is invalid",
        ));
    }
    let mut bytes = hex::decode(bytes)
        .map_err(|_| migu_upstream_error("Migu native response encoding is invalid"))?;
    for (i, byte) in bytes.iter_mut().enumerate() {
        *byte = byte.wrapping_sub(KEY[i % KEY.len()]);
    }
    Ok(bytes)
}

pub(super) fn sign(bytes: &[u8]) -> String {
    let mut rolling = 0x7a_u8;
    let mut transformed = Vec::with_capacity(bytes.len() + 96);
    for byte in bytes.iter().chain(CERTIFICATE_PREFIX) {
        rolling = rolling.wrapping_add(byte.wrapping_mul(2)).wrapping_add(1);
        transformed.push(rolling);
    }
    // V005 has a swapped MD5 IV and no message-length suffix. Exact blocks
    // receive no extra padding block; partial blocks end in 0x80 and zeros.
    if transformed.len() % 64 != 0 {
        transformed.push(0x80);
        transformed.resize(transformed.len().div_ceil(64) * 64, 0);
    }
    let mut state = [0x67452301_u32, 0x10325476, 0x98badcfe, 0xefcdab89];
    for block in transformed.chunks_exact(64) {
        let mut words = [0_u32; 16];
        for (word, bytes) in words.iter_mut().zip(block.chunks_exact(4)) {
            *word = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        }
        let [mut a, mut b, mut c, mut d] = state;
        for i in 0..64 {
            let (f, g) = match i {
                0..16 => ((b & c) | (!b & d), i),
                16..32 => ((d & b) | (!d & c), (5 * i + 1) % 16),
                32..48 => (b ^ c ^ d, (3 * i + 5) % 16),
                _ => (c ^ (b | !d), (7 * i) % 16),
            };
            let next = b.wrapping_add(
                a.wrapping_add(f)
                    .wrapping_add(ROUND_CONSTANTS[i])
                    .wrapping_add(words[g])
                    .rotate_left(SHIFTS[i / 16][i % 4]),
            );
            (a, b, c, d) = (d, next, b, c);
        }
        for (word, value) in state.iter_mut().zip([a, b, c, d]) {
            *word = word.wrapping_add(value);
        }
    }
    hex::encode_upper(
        state
            .into_iter()
            .flat_map(u32::to_le_bytes)
            .collect::<Vec<_>>(),
    )
}

enum Request<'a> {
    Get {
        query: Vec<(&'a str, &'a str)>,
        encrypted_response: bool,
    },
    Form(Vec<(&'a str, &'a str)>),
    Json(&'a [u8]),
    Image {
        query: Vec<(&'a str, &'a str)>,
        data: &'a [u8],
    },
}

fn request_signature(url: &Url, ce: &str, timestamp: &str, body: &[u8]) -> String {
    // The official interceptor signs the encoded path/query, then CE,
    // timestamp and the exact emitted body. An absent query has no '?'.
    let mut input = url.path().as_bytes().to_vec();
    if let Some(query) = url.query().filter(|query| !query.is_empty()) {
        input.push(b'?');
        input.extend_from_slice(query.as_bytes());
    }
    input.extend_from_slice(ce.as_bytes());
    input.extend_from_slice(timestamp.as_bytes());
    input.extend_from_slice(body);
    sign(&input)
}

fn native_mv_device_ce(device: &str, uid: Option<&str>) -> String {
    let mut fields = url::form_urlencoded::Serializer::new(String::new());
    fields.append_pair("deviceId", device);
    if let Some(uid) = uid {
        fields.append_pair("uid", uid);
    }
    encode(fields.finish().as_bytes())
}

impl MiguClient {
    pub(super) async fn native_mv_grant(
        &self,
        content_id: &str,
        format: tuneweave_core::MiguNativeMvFormat,
        auth: Option<&super::account_download::NativeAuthorization>,
    ) -> Result<serde_json::Value> {
        const HOST: &str = "app.c.nf.migu.cn";
        const PATH: &str = "/strategy/mvplayinfo/by-priority/v1.1";
        let profile = match format {
            tuneweave_core::MiguNativeMvFormat::Auto | tuneweave_core::MiguNativeMvFormat::Sq => {
                "SQ"
            }
            tuneweave_core::MiguNativeMvFormat::Hq => "HQ",
            tuneweave_core::MiguNativeMvFormat::Pq => "PQ",
        };
        let mut url = self.catalog_endpoint(&format!("https://{HOST}{PATH}"))?;
        let mut query = vec![("contentId", content_id), ("formatType", profile)];
        if format == tuneweave_core::MiguNativeMvFormat::Auto {
            query.push(("canFallback", "true"));
        }
        query.sort_unstable_by(|a, b| a.0.cmp(b.0));
        let encoded = url::form_urlencoded::Serializer::new(String::new())
            .extend_pairs(query)
            .finish()
            .replace('+', "%20");
        url.set_query(Some(&encoded));

        let device = self.music_device.identity()?;
        let ce = native_mv_device_ce(&device, auth.map(|auth| auth.uid.as_str()));
        let token = auth
            .map(|auth| {
                validate_token(&auth.token)?;
                let mut header = HeaderValue::from_str(&auth.token)
                    .map_err(|_| migu_upstream_error("Migu native token is invalid"))?;
                header.set_sensitive(true);
                Ok::<_, TuneWeaveError>(header)
            })
            .transpose()?;
        let mut ce_header = HeaderValue::from_str(&ce)
            .map_err(|_| migu_upstream_error("Migu native device context is invalid"))?;
        ce_header.set_sensitive(true);
        let timestamp = timestamp()?;
        let signature = request_signature(&url, &ce, &timestamp, &[]);
        let started = Instant::now();
        let mut status = None;
        let result = async {
            let mut request = self
                .http
                .get(url)
                .header(ACCEPT, "application/json")
                .header("ce", ce_header)
                .header("signVersion", "V005")
                .header("timestamp", &timestamp)
                .header("logId", &timestamp)
                .header("sign", signature)
                .header("ua", "Android_migu")
                .header("version", "8.9.1")
                .header("channel", "014000D")
                .header("subchannel", "spyn")
                .header("mode", "android")
                .header("appId", "music")
                .header("os", "Android")
                .header("pkgName", "cmccwm.mobilemusic")
                .header("language", "Chinese")
                .header("verify", "verify")
                .header("recommendstatus", "0")
                .header("randomsessionkey", "000000");
            if let Some(token) = token {
                request = request.header("token", token);
            }
            let response = request.send().await.map_err(migu_network_error)?;
            status = Some(response.status());
            if response.status() == StatusCode::UNAUTHORIZED {
                return Err(error(
                    if auth.is_some() {
                        ErrorCode::PermissionDenied
                    } else {
                        ErrorCode::AuthenticationRequired
                    },
                    "Migu native MV grant requires authentication",
                ));
            }
            if response.status() == StatusCode::FORBIDDEN {
                return Err(error(
                    ErrorCode::PermissionDenied,
                    "Migu native MV grant was denied",
                ));
            }
            if !response.status().is_success() {
                return Err(migu_http_error(response.status()));
            }
            if !response
                .headers()
                .get(CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.split(';').next())
                .is_some_and(|v| v.trim().eq_ignore_ascii_case("application/json"))
            {
                return Err(migu_upstream_error(
                    "Migu native MV grant returned an invalid content type",
                ));
            }
            let body = read_session_body_limited(response, 1024 * 1024).await?;
            serde_json::from_slice(&body)
                .map_err(|_| migu_upstream_error("Migu native MV grant returned invalid JSON"))
        }
        .await;
        let log = result
            .as_ref()
            .map(|_| ())
            .map_err(|error| TuneWeaveError::new(error.code, "Migu native MV grant failed"));
        self.log_upstream_request("native_mv_grant", HOST, PATH, status, started, &log);
        result
    }

    pub(super) async fn native_get(
        &self,
        host: &'static str,
        path: &'static str,
        token: &str,
        uid: Option<&str>,
        query: Vec<(&str, &str)>,
        encrypted_response: bool,
    ) -> Result<serde_json::Value> {
        self.native_request(
            host,
            path,
            token,
            uid,
            Request::Get {
                query,
                encrypted_response,
            },
        )
        .await
    }

    pub(super) async fn native_form(
        &self,
        host: &'static str,
        path: &'static str,
        token: &str,
        uid: &str,
        fields: Vec<(&str, &str)>,
    ) -> Result<serde_json::Value> {
        self.native_request(host, path, token, Some(uid), Request::Form(fields))
            .await
    }

    pub(super) async fn native_image_post(
        &self,
        host: &'static str,
        path: &'static str,
        token: &str,
        uid: &str,
        query: Vec<(&str, &str)>,
        data: &[u8],
    ) -> Result<serde_json::Value> {
        self.native_request(host, path, token, Some(uid), Request::Image { query, data })
            .await
    }

    pub(super) async fn native_json(
        &self,
        host: &'static str,
        path: &'static str,
        token: &str,
        uid: &str,
        body: &[u8],
    ) -> Result<serde_json::Value> {
        self.native_request(host, path, token, Some(uid), Request::Json(body))
            .await
    }

    async fn native_request(
        &self,
        host: &'static str,
        path: &'static str,
        token: &str,
        uid: Option<&str>,
        request: Request<'_>,
    ) -> Result<serde_json::Value> {
        let operation = match &request {
            Request::Get { .. }
                if matches!(
                    path,
                    super::account_avatar::MODE_PATH
                        | super::account_avatar::AUDIT_PATH
                        | super::account_avatar::USAGE_PATH
                ) =>
            {
                "native_account_avatar"
            }
            Request::Get { .. } if path == super::account_avatar::PROFILE_PATH => {
                "native_user_profile"
            }
            Request::Get { .. } if path == super::playlist_order::ORDER_PATH => {
                "native_playlist_order"
            }
            Request::Get { .. } if path == super::following_artists::PATH => {
                "native_following_artists"
            }
            Request::Get { .. }
                if matches!(
                    path,
                    super::following_artists::FOLLOW_PATH | super::following_artists::UNFOLLOW_PATH
                ) =>
            {
                "native_artist_subscription"
            }
            Request::Get { .. } => "native_account_authorization",
            Request::Form(_) if path == super::account_avatar::CONVERT_PATH => {
                "native_account_avatar"
            }
            Request::Form(_) => "native_playlist_metadata",
            Request::Json(_) => "native_account_avatar",
            Request::Image { query, .. } if query.contains(&("type", "00")) => {
                "native_account_avatar"
            }
            Request::Image { .. } => "native_playlist_cover",
        };
        validate_token(token)?;
        let mut auth = HeaderValue::from_str(token)
            .map_err(|_| migu_upstream_error("Migu native token is invalid"))?;
        auth.set_sensitive(true);
        let started = Instant::now();
        let mut status = None;
        let result = async {
            let device = self.music_device.identity()?;
            let ce = {
                let mut fields = url::form_urlencoded::Serializer::new(String::new());
                fields.append_pair("deviceId", &device);
                if let Some(uid) = uid {
                    fields.append_pair("uid", uid);
                }
                encode(fields.finish().as_bytes())
            };
            let mut ce_header = HeaderValue::from_str(&ce)
                .map_err(|_| migu_upstream_error("Migu native context is invalid"))?;
            ce_header.set_sensitive(true);
            let mut url = self.catalog_endpoint(&format!("https://{host}{path}"))?;
            let (body, content_type, encrypted_response) = match request {
                Request::Get {
                    mut query,
                    encrypted_response,
                } => {
                    query.sort_unstable_by(|a, b| a.0.cmp(b.0));
                    if !query.is_empty() {
                        // OkHttp addQueryParameter uses %20 for spaces. Keep
                        // the exact encoded query used by the native signature.
                        let encoded = url::form_urlencoded::Serializer::new(String::new())
                            .extend_pairs(query)
                            .finish()
                            .replace('+', "%20");
                        url.set_query(Some(&encoded));
                    }
                    (None, None, encrypted_response)
                }
                Request::Form(fields) => {
                    let body = url::form_urlencoded::Serializer::new(String::new())
                        .extend_pairs(fields)
                        .finish();
                    (
                        Some(body.into_bytes()),
                        Some("application/x-www-form-urlencoded"),
                        false,
                    )
                }
                Request::Json(data) => (
                    Some(data.to_vec()),
                    Some("application/json;charset=utf-8"),
                    false,
                ),
                Request::Image { mut query, data } => {
                    query.sort_unstable_by(|a, b| a.0.cmp(b.0));
                    if !query.is_empty() {
                        url.query_pairs_mut().extend_pairs(query);
                    }
                    (Some(data.to_vec()), Some("image/jpeg"), false)
                }
            };
            let timestamp = timestamp()?;
            let signature =
                request_signature(&url, &ce, &timestamp, body.as_deref().unwrap_or_default());
            let builder = if let Some(body) = body {
                self.http
                    .post(url)
                    .header(
                        reqwest::header::CONTENT_TYPE,
                        content_type.expect("POST body has a content type"),
                    )
                    .body(body)
            } else {
                self.http.get(url)
            };
            let response = builder
                .header(ACCEPT, "application/json")
                .header("token", auth)
                .header("ce", ce_header)
                .header("signVersion", "V005")
                .header("timestamp", timestamp)
                .header("sign", signature)
                .header("ua", "Android_migu")
                .header("version", "8.9.1")
                .header("channel", "014000D")
                .header("subchannel", "spyn")
                .header("mode", "android")
                .header("appId", "music")
                .header("os", "Android")
                .header("pkgName", "cmccwm.mobilemusic")
                .header("language", "Chinese")
                .header("verify", "verify")
                .send()
                .await
                .map_err(|e| {
                    error(
                        if e.is_timeout() {
                            ErrorCode::UpstreamTimeout
                        } else {
                            ErrorCode::UpstreamError
                        },
                        "Migu native request failed",
                    )
                })?;
            status = Some(response.status());
            if response.status() == StatusCode::TOO_MANY_REQUESTS {
                return Err(error(
                    ErrorCode::RateLimited,
                    "Migu native request was rate limited",
                ));
            }
            if matches!(
                response.status(),
                StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN
            ) {
                return Err(error(
                    ErrorCode::PermissionDenied,
                    "Migu native authorization was denied",
                ));
            }
            if !response.status().is_success() {
                return Err(migu_upstream_error(
                    "Migu native request returned an unsuccessful HTTP status",
                ));
            }
            let bytes = read_session_body_limited(response, 1_048_576).await?;
            let bytes = if encrypted_response {
                decode(&bytes)?
            } else {
                bytes
            };
            serde_json::from_slice(&bytes)
                .map_err(|_| migu_upstream_error("Migu native response is invalid"))
        }
        .await;
        self.log_upstream_request(operation, host, path, status, started, &result);
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_playlist_order_query_signature_matches_official_library() {
        let url = Url::parse("https://app.u.nf.migu.cn/MIGUM2.0/v1.0/user/mySongSorts.do?contentId=51&musicList=77&newPosition=1&oldPostion=51&singer=Artist%20A%7CArtist%20B&songId=s51&songName=%20%20%E6%B5%8B%E8%AF%95%20A%20%26%20%2B%20%23%20%2F%20100%25%20%20").unwrap();
        assert_eq!(
            request_signature(&url, "fixture-ce", "1758588800000", b""),
            "0CAF8936E002674FB107994DCC0D16A6"
        );
    }

    #[test]
    fn native_playlist_metadata_post_signatures_match_official_streaming_library() {
        let body = "id=77&songflag=0&info=%E6%B5%8B%E8%AF%95+%26+x%3Dy+%2B+100%25%0Aline+2&title=Title+%23";
        for (path, body, expected) in [
            (
                "/MIGUM2.0/v1.0/user/updateMusicList.do",
                body.as_bytes(),
                "8543D522783AADD76043522CD7ED4F11",
            ),
            (
                "/MIGUM2.0/v1.0/user/updateMusicList.do",
                b"".as_slice(),
                "511A6C9488E0CA51A25781307846EDA1",
            ),
            (
                "/path?a=1",
                body.as_bytes(),
                "8FF821675ED491C5AAE70E53CBB1DD26",
            ),
        ] {
            let url = Url::parse(&format!("https://app.u.nf.migu.cn{path}")).unwrap();
            assert_eq!(
                request_signature(&url, "C7C8CABD", "1758588800000", body),
                expected
            );
        }
    }
    #[test]
    fn signatures_match_official_native_vectors_at_block_boundaries() {
        for (size, expected) in [
            (0, "330D403813C5F7041929D523A6CB22D9"),
            (1, "19C6532ABE2A363F4505ADD99529A4F1"),
            (23, "2C31513E90441FCFEC5D33547F256889"),
            (24, "53216BA6E834691E310A9612333C2308"),
            (31, "B0D965C8912105BDEFC4083B0F4BEFB1"),
            (32, "435ED1D5871E6924682774AE162B9731"),
            (33, "37B1BD1A555C917044E14A58A8EC4931"),
            (63, "25A8A5E8FD73A7752B35032389BD8068"),
            (64, "673F5B194FA57EEC77503FE2A3891D8F"),
            (95, "60A113493D99CB67C907C8A426361658"),
            (96, "85626270AB9945CEAC8D702C2D60DB6D"),
            (97, "05866CCC5BB022CD24C38D9651F340E9"),
            (1024, "3016A6F7702FFFB3037ED344F1F66D2C"),
        ] {
            let input = (0..size)
                .map(|i| ((i * 13 + 7) % 256) as u8)
                .collect::<Vec<_>>();
            assert_eq!(sign(&input), expected, "{size}");
        }
    }
    #[test]
    fn response_encoding_is_hex_plus_repeating_byte_key() {
        let raw = br#"{"code":"000000","data":{"userInfoItem":{"userId":"fixture"}}}"#;
        let expected = "DE85B7C6C5D594925491879D847997638F85B8B8D5D19492AD83CCE0B9BBB0AFC9D29DCBC6DD9492AD83CCE0B9BBB0A5859D76BDCAE8E6CDA4C679EAD1C6";
        assert_eq!(encode(raw), expected);
        assert_eq!(decode(expected.as_bytes()).unwrap(), raw);
        for input in [b"".as_slice(), b"abc", b"gg", b"{\"code\":\"000000\"}"] {
            assert!(decode(input).is_err());
        }
    }
}
