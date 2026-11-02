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
    fn server(responses: Vec<(String, String)>) -> (String, std::thread::JoinHandle<Vec<String>>) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let task = std::thread::spawn(move || {
            let mut requests = Vec::new();
            for (headers, body) in responses {
                let deadline = Instant::now() + Duration::from_secs(5);
                let mut stream = loop {
                    match listener.accept() {
                        Ok((stream, _)) => break stream,
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            assert!(Instant::now() < deadline, "missing NEAPI request");
                            std::thread::sleep(Duration::from_millis(5));
                        }
                        Err(error) => panic!("NEAPI test accept failed: {error}"),
                    }
                };
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut bytes = Vec::new();
                loop {
                    let mut buffer = [0; 4096];
                    let count = stream.read(&mut buffer).unwrap();
                    assert!(count > 0);
                    bytes.extend_from_slice(&buffer[..count]);
                    assert!(bytes.len() < 16 * 1024);
                    if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                        let header = String::from_utf8_lossy(&bytes[..end]).to_ascii_lowercase();
                        let length = header
                            .lines()
                            .find_map(|line| {
                                line.strip_prefix("content-length:")
                                    .map(|value| value.trim().parse::<usize>().unwrap())
                            })
                            .unwrap_or(0);
                        if bytes.len() >= end + 4 + length {
                            break;
                        }
                    }
                }
                requests.push(String::from_utf8(bytes).unwrap());
                write!(stream, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n{headers}\r\n{body}", body.len()).unwrap();
            }
            requests
        });
        (origin, task)
    }
    fn configuration() -> Configuration {
        Configuration {
            version: 7,
            encrypt_key: [7; 32],
            decrypt_key: [7; 32],
        }
    }
    fn refresh() -> (String, String) {
        (
            String::new(),
            json!({"code":200,"data":{"version":7,"file":"","circleTime":60000}}).to_string(),
        )
    }
    async fn cached_client(origin: &str, ttl: Duration) -> NeteaseClient {
        let mut client = NeteaseClient::new(NeteaseConfig {
            base_url: origin.into(),
            xeapi_base_url: origin.into(),
            ..NeteaseConfig::default()
        })
        .unwrap();
        client.http = reqwest::Client::builder()
            .no_proxy()
            .retry(reqwest::retry::never())
            .build()
            .unwrap();
        *client.neapi_config.lock().await = Some((configuration(), Instant::now(), ttl));
        client
    }
    #[tokio::test]
    async fn neapi_expired_cache_and_explicit_rejection_refresh_then_retry_once() {
        let reply = encrypt(br#"{"code":200,"result":"ok"}"#, &configuration()).unwrap();
        let (origin, task) = server(vec![
            refresh(),
            ("X-User-Status: 777\r\n".into(), String::new()),
            refresh(),
            (String::new(), reply.clone()),
            (String::new(), reply),
        ]);
        let client = cached_client(&origin, Duration::ZERO).await;
        for _ in 0..2 {
            let result = client
                .request_neapi("/api/search/get", json!({"s":"query","limit":1}))
                .await
                .unwrap();
            assert_eq!(result.body["result"], "ok");
        }
        let requests = task.join().unwrap();
        assert_eq!(requests.len(), 5);
        assert!(
            requests[0].starts_with("GET /api/gorilla/algorithm/record?version=7&behavior=update ")
        );
        assert!(requests[1].starts_with("POST /neapi/search/get "));
        assert!(requests[2].starts_with("GET /api/gorilla/algorithm/record?"));
        assert!(requests[3].starts_with("POST /neapi/search/get "));
        assert!(requests[4].starts_with("POST /neapi/search/get "));
        assert!(!requests[1].contains("s=query"));
    }
    #[tokio::test]
    async fn neapi_repeated_key_rejection_stops_after_one_retry() {
        let (origin, task) = server(vec![
            ("X-User-Status: 999\r\n".into(), String::new()),
            refresh(),
            ("X-User-Status: 777\r\n".into(), String::new()),
        ]);
        let client = cached_client(&origin, Duration::from_secs(60)).await;
        assert!(
            client
                .request_neapi("/api/search/get", json!({"s":"query"}))
                .await
                .is_err()
        );
        assert_eq!(task.join().unwrap().len(), 3);
    }
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
