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

use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serde::Deserialize;
use tuneweave_core::{ErrorCode, Platform, Result, TuneWeaveError};
use url::Url;

const SIGNER_NODE_ENV: &str = "TUNEWEAVE_SODA_BDMS_NODE";
const SIGNER_ADDON_ENV: &str = "TUNEWEAVE_SODA_BDMS_ADDON";
const SIGNER_WINE_ENV: &str = "TUNEWEAVE_SODA_BDMS_WINE";
const SIGNER_PASSPORT_SDK_ENV: &str = "TUNEWEAVE_SODA_BDMS_PASSPORT_SDK";
const SIGNER_JSDOM_ENV: &str = "TUNEWEAVE_SODA_BDMS_JSDOM";
const SIGNER_STARTUP_TIMEOUT: Duration = Duration::from_secs(10);
const SIGNER_IO_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_SIGNER_MESSAGE_BYTES: usize = 262_144;
const WORKER: &str = include_str!("bdms_sign_worker.cjs");

#[derive(Clone)]
pub(crate) struct SodaBdmsSigner {
    channel: Arc<Mutex<SignerChannel>>,
    passport_available: bool,
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

impl SodaBdmsSigner {
    pub(crate) fn from_env() -> Result<Option<Self>> {
        let node = nonempty_env(SIGNER_NODE_ENV);
        let addon = nonempty_env(SIGNER_ADDON_ENV);
        let wine = nonempty_env(SIGNER_WINE_ENV);
        let passport_sdk = nonempty_env(SIGNER_PASSPORT_SDK_ENV);
        let jsdom = nonempty_env(SIGNER_JSDOM_ENV);
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
            channel: Arc::new(Mutex::new(SignerChannel {
                child,
                writer: stream,
                reader,
            })),
            passport_available,
        })
    }

    pub(crate) async fn sign(
        &self,
        device_id: String,
        url: String,
        headers: Vec<(String, String)>,
    ) -> Result<HeaderMap> {
        let channel = self.channel.clone();
        tokio::task::spawn_blocking(move || {
            let mut channel = channel.lock().map_err(|_| signer_runtime_error())?;
            channel.sign(&device_id, &url, &headers)
        })
        .await
        .map_err(|_| signer_runtime_error())?
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
        let channel = self.channel.clone();
        tokio::task::spawn_blocking(move || {
            let mut channel = channel.lock().map_err(|_| signer_runtime_error())?;
            channel.sign_passport(&method, &url, &headers, body.as_deref(), &cookies)
        })
        .await
        .map_err(|_| signer_runtime_error())?
    }
}

impl SignerChannel {
    fn sign(
        &mut self,
        device_id: &str,
        url: &str,
        headers: &[(String, String)],
    ) -> Result<HeaderMap> {
        let headers: Vec<_> = headers
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_str()))
            .collect();
        let request = serde_json::json!({
            "mode": "native",
            "device_id": device_id,
            "url": url,
            "headers": headers,
        });
        let response = self.exchange(request)?;
        if !response.ok || response.headers.len() != 2 {
            return Err(signer_runtime_error());
        }
        let mut output = HeaderMap::new();
        for (name, value) in response.headers {
            let name =
                HeaderName::from_bytes(name.as_bytes()).map_err(|_| signer_runtime_error())?;
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

    fn sign_passport(
        &mut self,
        method: &str,
        url: &str,
        headers: &[(String, String)],
        body: Option<&str>,
        cookies: &std::collections::BTreeMap<String, String>,
    ) -> Result<Url> {
        let header_pairs: Vec<_> = headers
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_str()))
            .collect();
        let response = self.exchange(serde_json::json!({
            "mode": "passport",
            "method": method,
            "url": url,
            "headers": header_pairs,
            "body": body,
            "cookies": cookies,
        }))?;
        if !response.ok {
            return Err(signer_runtime_error());
        }
        let signed = response
            .url
            .ok_or_else(signer_runtime_error)
            .and_then(|url| Url::parse(&url).map_err(|_| signer_runtime_error()))?;
        let original = Url::parse(url).map_err(|_| signer_runtime_error())?;
        if signed.origin() != original.origin()
            || signed.path() != original.path()
            || !signed.query_pairs().any(|(name, _)| name == "a_bogus")
        {
            return Err(signer_runtime_error());
        }
        Ok(signed)
    }

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
        "Soda BDMS signer requires both a Node executable and the official native addon",
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
        "Soda Qishui requests require the official local security runtime",
    )
    .with_platform(Platform::Soda)
}

fn signer_runtime_error() -> TuneWeaveError {
    TuneWeaveError::new(ErrorCode::UpstreamError, "Soda BDMS request signing failed")
        .with_platform(Platform::Soda)
        .retryable(true)
}

#[cfg(test)]
mod tests {
    use super::*;

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
