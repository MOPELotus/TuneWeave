//! Explicit revocation uses independent identity reads, never a logout acknowledgement.
use super::*;
use crate::login::SodaCredential;

const REVOKE_PATH: &str = "/passport/web/logout/";
const PROFILE_PATH: &str = "/luna/pc/me";
const MAX_BYTES: usize = 1024 * 1024;

#[derive(Deserialize)]
struct IdentityEnvelope {
    status_code: i64,
    my_info: Option<Identity>,
}
#[derive(Deserialize)]
struct Identity {
    id: String,
}

fn invalid() -> TuneWeaveError {
    soda_upstream_error("Soda session verification returned invalid or conflicting identity data")
}

impl SodaClient {
    /// Does not accept Set-Cookie: deleting a Cookie is not proof of authentication failure.
    pub(crate) async fn revocation_session_authenticated(
        &self,
        credential: &SodaCredential,
    ) -> Result<bool> {
        let uid = credential.user_id().ok_or_else(invalid)?;
        let started = Instant::now();
        let mut status = None;
        let result = async {
            let device = self.login_device()?;
            let mut url = Url::parse("https://api.qishui.com").map_err(|_| invalid())?;
            url.set_path(PROFILE_PATH);
            url.query_pairs_mut()
                .append_pair("aid", SODA_APP_ID)
                .append_pair("app_name", "luna_pc")
                .append_pair("device_platform", "windows")
                .append_pair("channel", "official")
                .append_pair("version_name", "2.1.0")
                .append_pair("version_code", "20010000")
                .append_pair("device_id", &device.device_id)
                .append_pair("iid", &device.install_id)
                .append_pair("fp", &device.device_id);
            let mut response = self
                .send_login_request(
                    self.login_request(reqwest::Method::GET, url)
                        .header(reqwest::header::ACCEPT, "application/json")
                        .header(reqwest::header::COOKIE, credential.cookie_header()?),
                )
                .await?;
            status = Some(response.status());
            if response.headers().contains_key("bdturing-verify") {
                return Err(TuneWeaveError::new(
                    ErrorCode::CapabilityNotSupported,
                    "Soda session verification requires a platform challenge",
                )
                .with_platform(Platform::Soda));
            }
            if response.status() == StatusCode::UNAUTHORIZED {
                return Ok(false);
            }
            if response.status() != StatusCode::OK {
                return Err(soda_http_error(response.status()));
            }
            if !response
                .headers()
                .get(CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .is_some_and(|v| {
                    v.split(';')
                        .next()
                        .is_some_and(|v| v.trim().eq_ignore_ascii_case("application/json"))
                })
                || response
                    .content_length()
                    .is_some_and(|len| len > MAX_BYTES as u64)
            {
                return Err(invalid());
            }
            let mut body = Vec::new();
            while let Some(chunk) = response.chunk().await.map_err(soda_network_error)? {
                if body.len().saturating_add(chunk.len()) > MAX_BYTES {
                    return Err(invalid());
                }
                body.extend_from_slice(&chunk);
            }
            let identity: IdentityEnvelope =
                serde_json::from_slice(&body).map_err(|_| invalid())?;
            match (identity.status_code, identity.my_info) {
                (1_000_016, None) => Ok(false),
                (0, Some(identity)) if identity.id == uid => Ok(true),
                _ => Err(invalid()),
            }
        }
        .await;
        self.log_upstream_request(
            "session_revocation_identity",
            "api.qishui.com",
            PROFILE_PATH,
            status,
            started,
            &result,
        );
        result
    }

    pub(crate) async fn send_session_revocation(
        &self,
        credential: &SodaCredential,
        before_send: impl FnOnce() -> Result<()> + Send,
    ) -> Result<()> {
        credential.user_id().ok_or_else(invalid)?;
        let device = self.login_device()?;
        let mut url = Url::parse("https://api.qishui.com").map_err(|_| invalid())?;
        url.set_path(REVOKE_PATH);
        url.query_pairs_mut()
            .append_pair("need_redirect", "0")
            .append_pair("aid", SODA_APP_ID)
            .append_pair("device_id", &device.device_id)
            .append_pair("fp", &device.device_id);
        let request = self
            .login_request(reqwest::Method::GET, url)
            .header(reqwest::header::ACCEPT, "application/json")
            .header(reqwest::header::COOKIE, credential.cookie_header()?);
        before_send()?;
        let started = Instant::now();
        let mut status = None;
        let result = async {
            let response = self.send_login_request(request).await?;
            status = Some(response.status());
            // Neither a 200 response nor its body/Cookie proves invalidation. Drop
            // the body without buffering it; the Provider always independently probes.
            if response.status() != StatusCode::OK {
                return Err(soda_http_error(response.status()));
            }
            Ok(())
        }
        .await;
        self.log_upstream_request(
            "session_revocation_request",
            "api.qishui.com",
            REVOKE_PATH,
            status,
            started,
            &result,
        );
        result.map_err(|error| error.retryable(false))
    }
}

#[cfg(test)]
mod tests;
