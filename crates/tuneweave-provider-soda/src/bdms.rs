use std::{
    env,
    io::{BufReader, Read, Write},
    net::{TcpListener, TcpStream},
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

use reqwest::{
    Client,
    header::{CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue},
    redirect::Policy,
};
use serde::Deserialize;
use tuneweave_core::{ErrorCode, Platform, Result, TuneWeaveError};
use url::{Host, Url};

const SIGNER_NODE_ENV: &str = "TUNEWEAVE_SODA_BDMS_NODE";
const SIGNER_ADDON_ENV: &str = "TUNEWEAVE_SODA_BDMS_ADDON";
const SIGNER_WINE_ENV: &str = "TUNEWEAVE_SODA_BDMS_WINE";
const SIGNER_PASSPORT_SDK_ENV: &str = "TUNEWEAVE_SODA_BDMS_PASSPORT_SDK";
const SIGNER_JSDOM_ENV: &str = "TUNEWEAVE_SODA_BDMS_JSDOM";
const SIGNER_SERVICE_URL_ENV: &str = "TUNEWEAVE_SODA_BDMS_SERVICE_URL";
const SIGNER_SERVICE_TOKEN_ENV: &str = "TUNEWEAVE_SODA_BDMS_SERVICE_TOKEN";
const SIGNER_STARTUP_TIMEOUT: Duration = Duration::from_secs(10);
const SIGNER_IO_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_SIGNER_MESSAGE_BYTES: usize = 262_144;
const WORKER: &str = include_str!("bdms_sign_worker.cjs");

#[derive(Clone)]
pub(crate) struct SodaBdmsSigner {
    backend: SignerBackend,
    passport_available: bool,
}

#[derive(Clone)]
enum SignerBackend {
    Local(Arc<Mutex<SignerChannel>>),
    Remote(RemoteSigner),
}

#[derive(Clone)]
struct RemoteSigner {
    endpoint: Url,
    token: String,
    http: Client,
}

struct SignerChannel {
    child: Child,
    writer: TcpStream,
    reader: BufReader<TcpStream>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SignerResponse {
    ok: bool,
    #[serde(default)]
    headers: std::collections::BTreeMap<String, String>,
    #[serde(default)]
    url: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SignerHandshake {
    #[serde(rename = "type")]
    kind: String,
    token: String,
    passport_available: bool,
}

impl std::fmt::Debug for SodaBdmsSigner {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("SodaBdmsSigner(configured)")
    }
}

impl std::fmt::Debug for RemoteSigner {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("RemoteSigner(configured)")
    }
}

impl SodaBdmsSigner {
    pub(crate) fn from_env() -> Result<Option<Self>> {
        let node = nonempty_env(SIGNER_NODE_ENV);
        let addon = nonempty_env(SIGNER_ADDON_ENV);
        let wine = nonempty_env(SIGNER_WINE_ENV);
        let passport_sdk = nonempty_env(SIGNER_PASSPORT_SDK_ENV);
        let jsdom = nonempty_env(SIGNER_JSDOM_ENV);
        let service_url = nonempty_text_env(SIGNER_SERVICE_URL_ENV);
        let service_token = nonempty_text_env(SIGNER_SERVICE_TOKEN_ENV);
        let local_configured = node.is_some()
            || addon.is_some()
            || wine.is_some()
            || passport_sdk.is_some()
            || jsdom.is_some();
        if service_url.is_some() || service_token.is_some() {
            if local_configured || service_url.is_none() || service_token.is_none() {
                return Err(signer_configuration_error());
            }
            return Self::remote(service_url.as_deref().unwrap(), service_token.unwrap()).map(Some);
        }
        if node.is_none()
            && addon.is_none()
            && wine.is_none()
            && passport_sdk.is_none()
            && jsdom.is_none()
        {
            return Ok(None);
        }
        if passport_sdk.is_some() != jsdom.is_some() {
            return Err(signer_configuration_error());
        }
        let (Some(node), Some(addon)) = (node, addon) else {
            return Err(signer_configuration_error());
        };
        Self::spawn(node, addon, wine, passport_sdk, jsdom).map(Some)
    }

    fn remote(endpoint: &str, token: String) -> Result<Self> {
        let endpoint = validate_remote_endpoint(endpoint)?;
        if !(32..=512).contains(&token.len())
            || !token.is_ascii()
            || token.bytes().any(|byte| byte.is_ascii_control())
        {
            return Err(signer_configuration_error());
        }
        let http = Client::builder()
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(10))
            .redirect(Policy::none())
            .build()
            .map_err(|_| signer_configuration_error())?;
        Ok(Self {
            backend: SignerBackend::Remote(RemoteSigner {
                endpoint,
                token,
                http,
            }),
            passport_available: true,
        })
    }

    fn spawn(
        node: PathBuf,
        addon: PathBuf,
        wine: Option<PathBuf>,
        passport_sdk: Option<PathBuf>,
        jsdom: Option<PathBuf>,
    ) -> Result<Self> {
        let listener = TcpListener::bind(("127.0.0.1", 0)).map_err(|_| signer_start_error())?;
        listener
            .set_nonblocking(true)
            .map_err(|_| signer_start_error())?;
        let address = listener.local_addr().map_err(|_| signer_start_error())?;
        let token = hex::encode(rand::random::<[u8; 32]>());
        let mut command = if let Some(wine) = wine {
            let mut command = Command::new(wine);
            command.arg(node);
            command
        } else {
            Command::new(node)
        };
        let child_command = command
            .arg("-e")
            .arg(WORKER)
            .env(SIGNER_ADDON_ENV, addon)
            .env("TUNEWEAVE_SODA_BDMS_SOCKET", address.to_string())
            .env("TUNEWEAVE_SODA_BDMS_TOKEN", &token)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        if let (Some(sdk), Some(jsdom)) = (&passport_sdk, &jsdom) {
            child_command
                .env(SIGNER_PASSPORT_SDK_ENV, sdk)
                .env(SIGNER_JSDOM_ENV, jsdom);
        }
        let mut child = child_command.spawn().map_err(|_| signer_start_error())?;

        let startup = (|| {
            let started = Instant::now();
            let stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                    Err(_) => return Err(signer_start_error()),
                }
                if child
                    .try_wait()
                    .map_err(|_| signer_start_error())?
                    .is_some()
                    || started.elapsed() >= SIGNER_STARTUP_TIMEOUT
                {
                    return Err(signer_start_error());
                }
                thread::sleep(Duration::from_millis(20));
            };
            stream
                .set_read_timeout(Some(SIGNER_IO_TIMEOUT))
                .map_err(|_| signer_start_error())?;
            stream
                .set_write_timeout(Some(SIGNER_IO_TIMEOUT))
                .map_err(|_| signer_start_error())?;
            stream.set_nodelay(true).map_err(|_| signer_start_error())?;
            let mut reader = BufReader::new(stream.try_clone().map_err(|_| signer_start_error())?);
            let handshake = read_bounded_line(&mut reader)
                .and_then(|line| serde_json::from_slice::<SignerHandshake>(&line).ok())
                .ok_or_else(signer_start_error)?;
            if handshake.kind != "ready"
                || handshake.token != token
                || handshake.passport_available != passport_sdk.is_some()
            {
                return Err(signer_start_error());
            }
            Ok((stream, reader, handshake.passport_available))
        })();
        let (stream, reader, passport_available) = match startup {
            Ok(startup) => startup,
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error);
            }
        };
        if !passport_available && passport_sdk.is_some() {
            let _ = child.kill();
            let _ = child.wait();
            return Err(signer_start_error());
        }
        Ok(Self {
            backend: SignerBackend::Local(Arc::new(Mutex::new(SignerChannel {
                child,
                writer: stream,
                reader,
            }))),
            passport_available,
        })
    }

    pub(crate) async fn sign(
        &self,
        device_id: String,
        url: String,
        headers: Vec<(String, String)>,
    ) -> Result<HeaderMap> {
        let header_pairs: Vec<_> = headers
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_str()))
            .collect();
        let response = self
            .exchange(serde_json::json!({
                "mode": "native",
                "device_id": device_id,
                "url": url,
                "headers": header_pairs,
            }))
            .await?;
        map_native_response(response)
    }

    pub(crate) async fn sign_passport(
        &self,
        method: String,
        url: String,
        headers: Vec<(String, String)>,
        body: Option<String>,
        cookies: std::collections::BTreeMap<String, String>,
    ) -> Result<Url> {
        if !self.passport_available {
            return Err(signer_start_error());
        }
        let header_pairs: Vec<_> = headers
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_str()))
            .collect();
        let response = self
            .exchange(serde_json::json!({
                "mode": "passport",
                "method": method,
                "url": url,
                "headers": header_pairs,
                "body": body,
                "cookies": cookies,
            }))
            .await?;
        map_passport_response(response, &url)
    }

    async fn exchange(&self, request: serde_json::Value) -> Result<SignerResponse> {
        match &self.backend {
            SignerBackend::Local(channel) => {
                let channel = channel.clone();
                tokio::task::spawn_blocking(move || {
                    channel
                        .lock()
                        .map_err(|_| signer_runtime_error())?
                        .exchange(request)
                })
                .await
                .map_err(|_| signer_runtime_error())?
            }
            SignerBackend::Remote(remote) => remote.exchange(request).await,
        }
    }
}

impl SignerChannel {
    fn exchange(&mut self, request: serde_json::Value) -> Result<SignerResponse> {
        let request = serde_json::to_vec(&request).map_err(|_| signer_runtime_error())?;
        if request.len() > MAX_SIGNER_MESSAGE_BYTES {
            return Err(signer_runtime_error());
        }
        self.writer
            .write_all(&request)
            .and_then(|()| self.writer.write_all(b"\n"))
            .and_then(|()| self.writer.flush())
            .map_err(|_| signer_runtime_error())?;
        read_bounded_line(&mut self.reader)
            .ok_or_else(signer_runtime_error)
            .and_then(|line| {
                serde_json::from_slice::<SignerResponse>(&line).map_err(|_| signer_runtime_error())
            })
    }
}

impl RemoteSigner {
    async fn exchange(&self, request: serde_json::Value) -> Result<SignerResponse> {
        let request = serde_json::to_vec(&request).map_err(|_| signer_runtime_error())?;
        if request.len() > MAX_SIGNER_MESSAGE_BYTES {
            return Err(signer_runtime_error());
        }
        let mut response = self
            .http
            .post(self.endpoint.clone())
            .bearer_auth(&self.token)
            .header(CONTENT_TYPE, "application/json")
            .body(request)
            .send()
            .await
            .map_err(|_| signer_service_unavailable_error())?;
        if !response.status().is_success()
            || response
                .content_length()
                .is_some_and(|length| length > MAX_SIGNER_MESSAGE_BYTES as u64)
        {
            return Err(signer_service_unavailable_error());
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| signer_service_unavailable_error())?
        {
            if bytes.len().saturating_add(chunk.len()) > MAX_SIGNER_MESSAGE_BYTES {
                return Err(signer_runtime_error());
            }
            bytes.extend_from_slice(&chunk);
        }
        serde_json::from_slice(&bytes).map_err(|_| signer_runtime_error())
    }
}

fn map_native_response(response: SignerResponse) -> Result<HeaderMap> {
    if !response.ok || response.headers.len() != 2 || response.url.is_some() {
        return Err(signer_runtime_error());
    }
    let mut output = HeaderMap::new();
    for (name, value) in response.headers {
        let name = HeaderName::from_bytes(name.as_bytes()).map_err(|_| signer_runtime_error())?;
        if !matches!(name.as_str(), "x-helios" | "x-medusa") {
            return Err(signer_runtime_error());
        }
        let value = HeaderValue::from_str(&value).map_err(|_| signer_runtime_error())?;
        output.insert(name, value);
    }
    if !output.contains_key("x-helios") || !output.contains_key("x-medusa") {
        return Err(signer_runtime_error());
    }
    Ok(output)
}

fn map_passport_response(response: SignerResponse, original: &str) -> Result<Url> {
    if !response.ok || !response.headers.is_empty() {
        return Err(signer_runtime_error());
    }
    let signed = response
        .url
        .ok_or_else(signer_runtime_error)
        .and_then(|url| Url::parse(&url).map_err(|_| signer_runtime_error()))?;
    let original = Url::parse(original).map_err(|_| signer_runtime_error())?;
    if signed.origin() != original.origin()
        || signed.path() != original.path()
        || !signed.query_pairs().any(|(name, _)| name == "a_bogus")
    {
        return Err(signer_runtime_error());
    }
    Ok(signed)
}

impl Drop for SignerChannel {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

pub(crate) fn should_sign_url(url: &Url) -> bool {
    let Some(host) = url.host_str() else {
        return false;
    };
    let host = host.to_ascii_lowercase();
    let qishui_host = host == "qishui.com" || host.ends_with(".qishui.com");
    qishui_host
        && url.scheme() == "https"
        && !url.path().starts_with("/passport/")
        && !url.path().starts_with("/ttwid/")
}

pub(crate) fn is_passport_url(url: &Url) -> bool {
    let Some(host) = url.host_str() else {
        return false;
    };
    let host = host.to_ascii_lowercase();
    (host == "qishui.com" || host.ends_with(".qishui.com"))
        && url.scheme() == "https"
        && url.path().starts_with("/passport/")
}

fn nonempty_env(name: &str) -> Option<PathBuf> {
    env::var_os(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

fn nonempty_text_env(name: &str) -> Option<String> {
    env::var(name).ok().filter(|value| !value.is_empty())
}

fn validate_remote_endpoint(value: &str) -> Result<Url> {
    let mut url = Url::parse(value).map_err(|_| signer_configuration_error())?;
    if url.username() != ""
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.path() != "/"
    {
        return Err(signer_configuration_error());
    }
    let loopback = match url.host() {
        Some(Host::Ipv4(address)) => address.is_loopback(),
        Some(Host::Ipv6(address)) => address.is_loopback(),
        Some(Host::Domain(host)) => host.eq_ignore_ascii_case("localhost"),
        None => false,
    };
    if url.scheme() != "https" && !(url.scheme() == "http" && loopback) {
        return Err(signer_configuration_error());
    }
    url.set_path("/v1/sign");
    Ok(url)
}

fn read_bounded_line(reader: &mut impl Read) -> Option<Vec<u8>> {
    let mut output = Vec::with_capacity(256);
    let mut byte = [0; 1];
    loop {
        match reader.read(&mut byte) {
            Ok(0) => return None,
            Ok(_) if byte[0] == b'\n' => return Some(output),
            Ok(_) if output.len() < MAX_SIGNER_MESSAGE_BYTES => output.push(byte[0]),
            Ok(_) | Err(_) => return None,
        }
    }
}

fn signer_configuration_error() -> TuneWeaveError {
    TuneWeaveError::new(
        ErrorCode::InvalidRequest,
        "Soda BDMS signer needs a valid local runtime or HTTPS signing service URL and token",
    )
    .with_platform(Platform::Soda)
}

fn signer_start_error() -> TuneWeaveError {
    TuneWeaveError::new(
        ErrorCode::CapabilityNotSupported,
        "Soda BDMS security runtime could not be started",
    )
    .with_platform(Platform::Soda)
}

pub(crate) fn signer_required_error() -> TuneWeaveError {
    TuneWeaveError::new(
        ErrorCode::CapabilityNotSupported,
        "Soda Qishui requests require a running official signing service or local security runtime",
    )
    .with_platform(Platform::Soda)
}

fn signer_service_unavailable_error() -> TuneWeaveError {
    TuneWeaveError::new(
        ErrorCode::CapabilityNotSupported,
        "Soda signing service is not running or is unreachable",
    )
    .with_platform(Platform::Soda)
    .retryable(true)
}

fn signer_runtime_error() -> TuneWeaveError {
    TuneWeaveError::new(ErrorCode::UpstreamError, "Soda BDMS request signing failed")
        .with_platform(Platform::Soda)
        .retryable(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::BufRead;

    #[test]
    fn general_signer_matches_the_official_domain_gate() {
        for url in [
            "https://api.qishui.com/luna/pc/me",
            "https://bff-pc.qishui.com/light/invoke/v1",
        ] {
            assert!(should_sign_url(&Url::parse(url).unwrap()));
        }
        for url in [
            "https://api.qishui.com/passport/web/check_qrconnect/",
            "https://api.qishui.com/ttwid/check/",
            "https://qishui.com.evil.example/luna/pc/me",
            "http://api.qishui.com/luna/pc/me",
            "https://example.com/luna/pc/me",
        ] {
            assert!(!should_sign_url(&Url::parse(url).unwrap()));
        }
    }

    #[test]
    fn passport_signer_matches_official_passport_routes_only() {
        assert!(is_passport_url(
            &Url::parse("https://api.qishui.com/passport/web/check_qrconnect/").unwrap()
        ));
        assert!(!is_passport_url(
            &Url::parse("https://api.qishui.com/luna/pc/me").unwrap()
        ));
        assert!(!is_passport_url(
            &Url::parse("https://api.qishui.com.evil.example/passport/web/").unwrap()
        ));
    }

    #[test]
    fn remote_signer_requires_tls_except_for_loopback() {
        assert!(validate_remote_endpoint("https://signer.example").is_ok());
        assert!(validate_remote_endpoint("http://127.0.0.1:7833").is_ok());
        assert!(validate_remote_endpoint("http://[::1]:7833").is_ok());
        for endpoint in [
            "http://signer.example",
            "http://192.168.1.20:7833",
            "https://user@signer.example",
            "https://signer.example/path",
            "https://signer.example/?token=secret",
        ] {
            assert!(validate_remote_endpoint(endpoint).is_err(), "{endpoint}");
        }
    }

    #[tokio::test]
    async fn remote_signer_sends_authenticated_native_and_passport_requests() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("local mock signer listener");
        let address = listener.local_addr().expect("local mock address");
        let token = "a".repeat(64);
        let expected_token = token.clone();
        let mock = thread::spawn(move || {
            for _ in 0..2 {
                let (stream, _) = listener.accept().expect("mock request");
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .expect("mock read timeout");
                let mut reader = BufReader::new(stream);
                let mut request_line = String::new();
                reader.read_line(&mut request_line).expect("request line");
                let mut content_length = 0usize;
                let mut authenticated = false;
                let mut correct_route = false;
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).expect("request headers");
                    if line == "\r\n" || line.is_empty() {
                        break;
                    }
                    let lower = line.to_ascii_lowercase();
                    if let Some(value) = lower.strip_prefix("content-length:") {
                        content_length = value.trim().parse().expect("content length");
                    }
                    if let Some(value) = lower.strip_prefix("authorization:") {
                        authenticated = value.trim() == format!("bearer {expected_token}");
                    }
                    correct_route = correct_route || request_line.starts_with("POST /v1/sign ");
                }
                let mut body = vec![0; content_length];
                reader.read_exact(&mut body).expect("request body");
                let request: serde_json::Value =
                    serde_json::from_slice(&body).expect("sign request JSON");
                assert!(authenticated && correct_route);
                let response = match request["mode"].as_str() {
                    Some("native") => {
                        assert_eq!(request["device_id"], "7100000000000000001");
                        serde_json::json!({"ok":true,"headers":{"x-helios":"signed-h","x-medusa":"signed-m"}})
                    }
                    Some("passport") => {
                        assert_eq!(request["method"], "POST");
                        assert_eq!(request["cookies"]["sessionid_ss"], "synthetic");
                        serde_json::json!({"ok":true,"url":"https://api.qishui.com/passport/web/check_qrconnect/?aid=386088&a_bogus=signed"})
                    }
                    _ => panic!("unexpected signer mode"),
                };
                let body = serde_json::to_vec(&response).expect("response JSON");
                let mut stream = reader.into_inner();
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                    body.len()
                )
                .expect("response headers");
                stream.write_all(&body).expect("response body");
                stream.flush().expect("flush response");
            }
        });

        let signer = SodaBdmsSigner::remote(&format!("http://{address}"), token)
            .expect("loopback remote signer");
        let native = signer
            .sign(
                "7100000000000000001".to_owned(),
                "https://api.qishui.com/luna/pc/me?aid=386088".to_owned(),
                vec![("cookie".to_owned(), "sessionid_ss=synthetic".to_owned())],
            )
            .await
            .expect("native signatures");
        assert_eq!(native["x-helios"], "signed-h");
        assert_eq!(native["x-medusa"], "signed-m");
        let passport = signer
            .sign_passport(
                "POST".to_owned(),
                "https://api.qishui.com/passport/web/check_qrconnect/?aid=386088".to_owned(),
                Vec::new(),
                Some("token=synthetic".to_owned()),
                std::collections::BTreeMap::from([(
                    "sessionid_ss".to_owned(),
                    "synthetic".to_owned(),
                )]),
            )
            .await
            .expect("passport signature");
        assert!(passport.query_pairs().any(|(name, _)| name == "a_bogus"));
        mock.join().expect("mock service thread");
    }

    #[tokio::test]
    async fn configured_runtime_signs_synthetic_requests_without_network() {
        if env::var_os("TUNEWEAVE_SODA_BDMS_RUN_TEST").is_none() {
            return;
        }
        let signer = SodaBdmsSigner::from_env()
            .expect("configured signer runtime")
            .expect("signer is configured");
        let headers = vec![
            ("accept".to_owned(), "application/json".to_owned()),
            ("cookie".to_owned(), "sessionid_ss=synthetic".to_owned()),
            (
                "user-agent".to_owned(),
                "LunaPC/3.7.0(452316191)".to_owned(),
            ),
        ];
        let device_id = "7100000000000000001";
        let native = signer
            .sign(
                device_id.to_owned(),
                "https://api.qishui.com/luna/pc/me?aid=386088".to_owned(),
                headers.clone(),
            )
            .await
            .expect("native signatures");
        assert!(native.contains_key("x-helios"));
        assert!(native.contains_key("x-medusa"));

        let passport = signer
            .sign_passport(
                "POST".to_owned(),
                "https://api.qishui.com/passport/web/check_qrconnect/?aid=386088&is_frontier=true"
                    .to_owned(),
                vec![
                    (
                        "user-agent".to_owned(),
                        "SodaMusic/3.7.0 Windows synthetic probe".to_owned(),
                    ),
                    (
                        "x-ss-stub".to_owned(),
                        "0123456789ABCDEF0123456789ABCDEF".to_owned(),
                    ),
                ],
                Some("token=synthetic".to_owned()),
                Default::default(),
            )
            .await
            .expect("Passport a_bogus signature");
        assert!(passport.query_pairs().any(|(name, _)| name == "a_bogus"));

        let native_again = signer
            .sign(
                device_id.to_owned(),
                "https://api.qishui.com/luna/pc/me?aid=386088".to_owned(),
                headers,
            )
            .await
            .expect("native signatures after Passport signing");
        assert!(native_again.contains_key("x-helios"));
        assert!(native_again.contains_key("x-medusa"));
    }
}
