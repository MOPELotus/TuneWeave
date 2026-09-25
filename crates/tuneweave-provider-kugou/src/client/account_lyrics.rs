//! Account downloads use the selected native session, independently of public catalogue reads.
use super::*;
use crate::{KugouLoginClient, credential::NativeSession, signing};

impl KugouClient {
    pub(crate) async fn native_lyrics(
        &self,
        session: &NativeSession,
        track: Track,
        mut check: impl FnMut() -> Result<()>,
    ) -> Result<Lyrics> {
        let id = account_media::validate_track(&track)?;
        let hash = track
            .extensions
            .get("hash")
            .and_then(Value::as_str)
            .ok_or_else(|| kugou_upstream_error("KuGou lyric catalogue omitted its hash"))?;
        let duration = track
            .duration_ms
            .filter(|v| *v > 0)
            .ok_or_else(|| kugou_upstream_error("KuGou lyric catalogue omitted its duration"))?;
        let search = BTreeMap::from([
            ("album_audio_id", id.to_string()),
            ("duration", duration.to_string()),
            ("hash", hash.to_owned()),
            ("keyword", lyric_search_keyword(&track)),
            ("lrctxt", "1".into()),
            ("man", "no".into()),
        ]);
        let result = self
            .account_lyric_get(session, "/v1/search", search, parse_lyric_search_response)
            .await;
        check()?;
        let (candidate, diagnostics) = result?;
        let mut downloads = Vec::new();
        for format in [RequestedLyricFormat::Krc, RequestedLyricFormat::Lrc] {
            let query = BTreeMap::from([
                ("accesskey", candidate.accesskey.clone()),
                ("id", candidate.id.clone()),
                ("ver", "1".into()),
                ("client", "android".into()),
                ("charset", "utf8".into()),
                ("fmt", format.as_str().into()),
            ]);
            let result = self
                .account_lyric_get(session, "/download", query, |bytes| {
                    parse_lyric_download_response(bytes, &candidate, format)
                })
                .await;
            // Do not hide an account error behind another format or issue a request
            // after logout/relogin. An upstream LRC response to a KRC request is valid.
            check()?;
            downloads.push(result?);
        }
        let lrc = downloads.pop().expect("two lyric downloads");
        let krc = downloads.pop().expect("two lyric downloads");
        map_lyrics(id, candidate, diagnostics, Ok(krc), Ok(lrc))
    }

    async fn account_lyric_get<T>(
        &self,
        session: &NativeSession,
        path: &'static str,
        mut query: BTreeMap<&str, String>,
        parse: impl FnOnce(&[u8]) -> Result<T>,
    ) -> Result<T> {
        if !session.valid() || session.client == KugouLoginClient::Web {
            return Err(kugou_invalid_media_request(
                "KuGou lyrics require a native credential",
            ));
        }
        let seconds = crate::account::now_ms()? / 1000;
        query.insert("appid", session.client.appid().to_string());
        query.insert("clientver", session.client.clientver().to_string());
        let operation = match path {
            // The search wrapper clears default query fields; its device headers
            // still use the selected session. Authentication belongs to download.
            "/v1/search" => "native_lyric_search",
            "/download" => {
                query.extend([
                    ("clienttime", seconds.to_string()),
                    ("dfid", session.device.dfid().into()),
                    ("mid", session.device.mid.clone()),
                    ("uuid", "-".into()),
                    ("userid", session.user_id.clone()),
                    ("token", session.token.clone()),
                ]);
                "native_lyric_download"
            }
            _ => return Err(kugou_invalid_media_request("Unknown KuGou lyric operation")),
        };
        let signature = match session.client {
            KugouLoginClient::Concept => signing::concept_signature(&query, &[]),
            _ => signing::android_signature(&query, &[]),
        };
        query.insert("signature", signature);
        let target = format!("https://lyrics.kugou.com{path}");
        #[cfg(test)]
        let target = self
            .login_test_origin
            .as_ref()
            .map(|origin| origin.join(path).unwrap().to_string())
            .unwrap_or(target);
        let started = Instant::now();
        let mut status = None;
        let result = async {
            let response = self
                .http
                .get(target)
                .header("accept", "application/json")
                .header("user-agent", ANDROID_USER_AGENT)
                .header("dfid", session.device.dfid())
                .header("mid", &session.device.mid)
                .header("clienttime", seconds)
                .header("kg-rc", "1")
                .header("kg-rec", "1")
                .query(&query)
                .send()
                .await
                .map_err(crate::account::network_error)?;
            status = Some(response.status());
            let bytes =
                crate::account::read_response_with_limit(response, MAX_API_RESPONSE_BYTES as usize)
                    .await?;
            #[derive(Deserialize)]
            struct BusinessStatus {
                status: i64,
                errcode: Option<i64>,
                error_code: Option<i64>,
            }
            let business: BusinessStatus = serde_json::from_slice(&bytes)
                .map_err(|_| kugou_upstream_error("KuGou account lyric status is invalid"))?;
            let code = if path == "/v1/search" {
                business.errcode
            } else {
                business.error_code
            };
            let success = if path == "/v1/search" { 200 } else { 0 };
            if business.status != 200 || code != Some(success) {
                return Err(
                    kugou_upstream_error("KuGou account lyric request was rejected").with_details(
                        json!({"platform_status":business.status,"platform_code":code}),
                    ),
                );
            }
            parse(&bytes).map_err(|mut error| {
                // The public parser's upstream prose may echo an authenticated request.
                if let Some(details) = error.details.as_object_mut() {
                    details.remove("platform_message");
                }
                error
            })
        }
        .await;
        self.log_upstream_request(
            operation,
            "lyrics.kugou.com",
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
