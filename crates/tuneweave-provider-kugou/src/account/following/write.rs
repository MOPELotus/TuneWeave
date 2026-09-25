//! Standard SingerFollowDataModel.c/l, with its own AES-256/RSA envelope.
use super::*;
use aes::{
    Aes256,
    cipher::{BlockEncryptMut, KeyIvInit, block_padding::Pkcs7},
};
#[cfg(not(test))]
use rand::{TryRng, rngs::SysRng};

fn seed() -> Result<String> {
    #[cfg(test)]
    return Ok("0123456789abcdef".into());
    #[cfg(not(test))]
    {
        let mut bytes = [0; 8];
        SysRng.try_fill_bytes(&mut bytes).map_err(|_| {
            error(
                ErrorCode::InternalError,
                "KuGou artist encryption initialization failed",
            )
        })?;
        Ok(hex::encode(bytes))
    }
}

fn encrypted_params(plaintext: &[u8], seed: &str) -> Result<String> {
    // qo3.a.d: MD5(seed) is used as 32 ASCII key bytes, with its last 16
    // ASCII bytes as IV. It is not the six-character cloud-list cipher.
    let key = format!("{:x}", Md5::digest(seed));
    let mut buffer = plaintext.to_vec();
    buffer.resize(plaintext.len().checked_add(16).ok_or_else(malformed)?, 0);
    let bytes = cbc::Encryptor::<Aes256>::new_from_slices(key.as_bytes(), &key.as_bytes()[16..])
        .map_err(|_| malformed())?
        .encrypt_padded_mut::<Pkcs7>(&mut buffer, plaintext.len())
        .map_err(|_| malformed())?;
    Ok(hex::encode(bytes))
}

fn body(session: &NativeSession, singer_id: u64, seconds: u64, seed: &str) -> Result<Vec<u8>> {
    let portrait = crypto::encode(&json!({"clienttime":seconds,"key":seed}))?;
    let params = crypto::encode(&json!({"singerid":singer_id,"token":session.token}))?;
    crypto::encode(&json!({
        "plat":"0",
        "userid":session.user_id.parse::<u64>().map_err(|_| malformed())?,
        "singerid":singer_id,
        // SingerFollowDelegate.Q's default source when no UI-specific source exists.
        "source":7,
        "p":rsa_pkcs1_v15_encrypt_for_client(session.client,&portrait)?.to_ascii_uppercase(),
        "params":encrypted_params(&params,seed)?,
    }))
}

fn acknowledge(bytes: &[u8]) -> Result<()> {
    #[derive(Deserialize)]
    struct Ack {
        status: i64,
        error_code: Option<i64>,
    }
    // The UI accepts status=1 even without data. Neither a returned rank nor
    // a free-form message proves that the selected account now follows this id.
    let ack: Ack = serde_json::from_slice(bytes).map_err(|_| malformed())?;
    if ack.status != 1 || ack.error_code.is_some_and(|v| v != 0) {
        return Err(error(
            if ack.status == 0 && ack.error_code == Some(20017) {
                ErrorCode::AuthenticationRequired
            } else {
                ErrorCode::UpstreamError
            },
            "KuGou artist subscription was not acknowledged",
        )
        .with_details(json!({"platform_code":ack.error_code})));
    }
    Ok(())
}

impl KugouClient {
    pub(crate) async fn native_write_artist_subscription(
        &self,
        session: &NativeSession,
        singer_id: u64,
        subscribed: bool,
    ) -> Result<()> {
        validate_session(session)?;
        if session.client != KugouLoginClient::Standard {
            return Err(error(
                ErrorCode::CapabilityNotSupported,
                "KuGou artist subscriptions require a Standard native credential",
            ));
        }
        if singer_id == 0 || singer_id > i64::MAX as u64 {
            return Err(error(ErrorCode::InvalidRequest, "Invalid KuGou artist id"));
        }
        let seconds = now_ms()? / 1000;
        let body = body(session, singer_id, seconds, &seed()?)?;
        // ParamGenerator.w signs this exact body. It does not put token or userid
        // in the URL. Official common_uuid bit 45 selects the '-' sentinel.
        let mut params = BTreeMap::from([
            ("clienttime", seconds.to_string()),
            ("dfid", session.device.dfid().to_owned()),
            ("appid", session.client.appid().to_string()),
            ("mid", session.device.mid.clone()),
            ("uuid", "-".into()),
            ("clientver", session.client.clientver().to_string()),
        ]);
        params.insert("signature", android_signature(&params, &body));
        let path = if subscribed {
            "/followservice/v3/follow_singer"
        } else {
            "/followservice/v3/unfollow_singer"
        };
        let url = format!("https://{HOST}{path}");
        #[cfg(test)]
        let url = self
            .login_test_origin
            .as_ref()
            .map(|origin| origin.join(path).unwrap().to_string())
            .unwrap_or(url);
        let started = Instant::now();
        let mut status = None;
        let result = async {
            let response = self
                .http
                .post(url)
                .query(&params)
                .header(CONTENT_TYPE, "application/json;charset=utf-8")
                .body(body)
                .send()
                .await
                .map_err(network_error)?;
            status = Some(response.status());
            acknowledge(&read_response_with_limit(response, RESPONSE_LIMIT).await?)
        }
        .await;
        self.log_upstream_request(
            "native_artist_subscription",
            HOST,
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

#[cfg(test)]
mod tests;
