use super::*;
use crate::neapi::{Configuration, decode_configuration, decrypt, encrypt};

pub(super) type ConfigurationCache =
    Arc<tokio::sync::Mutex<Option<(Configuration, Instant, Duration)>>>;
const MAX_RESPONSE_BYTES: usize = 16 * 1024 * 1024;

impl NeteaseClient {
    async fn neapi_configuration(
        &self,
        force: bool,
        previous_version: Option<u16>,
    ) -> Result<Configuration> {
        let mut cached = self.neapi_config.lock().await;
        if let Some((config, fetched, ttl)) = &*cached {
            // Concurrent callers share a refresh, including a key-rejection refresh.
            if (!force && fetched.elapsed() < *ttl)
                || (force && previous_version.is_some_and(|v| v != config.version))
            {
                return Ok(config.clone());
            }
        }
        let version = cached.as_ref().map_or(0, |(config, _, _)| config.version);
        let started = Instant::now();
        let mut status = None;
        let outcome = async {
            let response = self
                .apply_network_identity(
                    self.http
                        .get(format!("{}/api/gorilla/algorithm/record", self.base_url))
                        .query(&[
                            ("version", version.to_string()),
                            (
                                "behavior",
                                if version == 0 { "open" } else { "update" }.to_owned(),
                            ),
                        ]),
                )
                .send()
                .await
                .map_err(request_error)?;
            status = Some(response.status());
            let bytes = bounded_bytes(response).await?;
            let body: Value = serde_json::from_slice(&bytes).map_err(|_| neapi_error())?;
            if body["code"] != 200 {
                return Err(neapi_error());
            }
            let ttl_ms = body["data"]["circleTime"]
                .as_u64()
                .filter(|v| *v > 0)
                .ok_or_else(neapi_error)?;
            let ttl = Duration::from_millis(ttl_ms.min(86_400_000));
            let file = body["data"]["file"].as_str().unwrap_or_default();
            let config = if file.is_empty() {
                let old = cached
                    .as_ref()
                    .map(|(config, _, _)| config.clone())
                    .ok_or_else(neapi_error)?;
                if body["data"]["version"]
                    .as_u64()
                    .is_some_and(|v| v != u64::from(old.version))
                {
                    return Err(neapi_error());
                }
                old
            } else {
                let raw = BASE64.decode(file).map_err(|_| neapi_error())?;
                decode_configuration(&raw)?
            };
            *cached = Some((config.clone(), Instant::now(), ttl));
            Ok(config)
        }
        .await;
        self.log_validated_upstream_request(
            "neapi_configuration",
            self.base_upstream_host,
            "/api/gorilla/algorithm/record",
            status,
            started,
            &outcome,
        );
        outcome
    }

    /// Sends a NEAPI container using a lazily fetched, bounded-lifetime public configuration.
    /// Only explicit key-version rejection triggers one retry; network errors are not retried.
    pub async fn request_neapi(&self, path: &str, payload: Value) -> Result<NeteaseResponse> {
        if !path.starts_with("/api/")
            || path.contains(['?', '#'])
            || path.chars().any(char::is_control)
        {
            return Err(
                TuneWeaveError::invalid_request("NEAPI requires a plain /api/ path")
                    .with_platform(Platform::Netease),
            );
        }
        let data = payload
            .as_object()
            .ok_or_else(|| TuneWeaveError::invalid_request("NEAPI payload must be an object"))?;
        let plaintext = encode_form(data, true);
        let mut config = self.neapi_configuration(false, None).await?;
        let started = Instant::now();
        let mut status = None;
        let outcome = async {
            for attempt in 0..2 {
                let body = encrypt(plaintext.as_bytes(), &config)?;
                let response = self
                    .apply_network_identity(
                        self.http
                            .post(format!(
                                "{}/neapi/{}",
                                self.xeapi_base_url,
                                path.trim_start_matches("/api/")
                            ))
                            .header(
                                header::CONTENT_TYPE,
                                "application/x-www-form-urlencoded;charset=utf-8",
                            )
                            .header(header::USER_AGENT, XEAPI_USER_AGENT)
                            .header("X-ER", config.version)
                            .header(
                                header::COOKIE,
                                self.xeapi_cookie(&unix_time_seconds().to_string())?,
                            )
                            .body(body),
                    )
                    .send()
                    .await
                    .map_err(request_error)?;
                status = Some(response.status());
                let rejected = response
                    .headers()
                    .get("x-user-status")
                    .and_then(|v| v.to_str().ok())
                    .is_some_and(|v| matches!(v, "777" | "999"));
                let cookies = response
                    .headers()
                    .get_all(header::SET_COOKIE)
                    .iter()
                    .filter_map(|v| v.to_str().ok().map(str::to_owned))
                    .collect();
                let bytes = bounded_bytes(response).await?;
                if rejected {
                    if attempt == 1 {
                        return Err(neapi_error());
                    }
                    config = self.neapi_configuration(true, Some(config.version)).await?;
                    continue;
                }
                return Ok(NeteaseResponse {
                    status: status.ok_or_else(neapi_error)?,
                    body: decrypt(&bytes, &config)?,
                    cookies,
                });
            }
            Err(neapi_error())
        }
        .await;
        self.log_validated_upstream_request(
            "neapi_request",
            self.xeapi_upstream_host,
            "/neapi/{path}",
            status,
            started,
            &outcome,
        );
        outcome
    }
}
async fn bounded_bytes(mut response: reqwest::Response) -> Result<Vec<u8>> {
    if !response.status().is_success() {
        return Err(TuneWeaveError::new(
            ErrorCode::UpstreamError,
            "NetEase NEAPI returned an HTTP error",
        )
        .with_platform(Platform::Netease)
        .with_details(json!({"status":response.status().as_u16()})));
    }
    if response
        .content_length()
        .is_some_and(|n| n > MAX_RESPONSE_BYTES as u64)
    {
        return Err(neapi_error());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(request_error)? {
        if bytes.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
            return Err(neapi_error());
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}
fn neapi_error() -> TuneWeaveError {
    TuneWeaveError::new(
        ErrorCode::UpstreamError,
        "NetEase NEAPI configuration or response is invalid",
    )
    .with_platform(Platform::Netease)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    #[ignore = "requires live NetEase access"]
    async fn live_neapi_configuration_and_search() {
        let client = NeteaseClient::new(NeteaseConfig::default()).unwrap();
        client
            .neapi_configuration(false, None)
            .await
            .expect("NEAPI configuration decode");
        let response = client
            .request_neapi(
                "/api/search/get",
                json!({"s":"TuneWeave","type":1,"limit":1,"offset":0}),
            )
            .await
            .unwrap();
        assert_eq!(response.body["code"], 200);
    }
}
