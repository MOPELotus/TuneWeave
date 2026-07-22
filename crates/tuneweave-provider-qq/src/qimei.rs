use aes::{
    Aes128,
    cipher::{BlockEncryptMut, KeyIvInit, block_padding::Pkcs7},
};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use md5::{Digest, Md5};
use num_bigint::BigUint;
use reqwest::Client;
use serde_json::{Value, json};
use tuneweave_core::{ErrorCode, Platform, Result, TuneWeaveError};

use crate::device::{QqDevice, unix_seconds_now};

const ENDPOINT: &str = "https://api.tencentmusic.com/tme/trpc/proxy";
const RSA_MODULUS: &str = concat!(
    "c4231830a2eb5fc2827170641e79d80fec51bda9a22e4b4ab37d1f205a4ae44d928cda25879f66",
    "a3429051663312a127faf8a246bdaaf63918417e90d7c95b5908aa6a2d0f852e4a6770294a548ac",
    "1c2fe8f1f252fb826f4ac86ab9a00e7ce47d002a56e7c4b51eb889acc60ca6adbc9f72e81f4d31",
    "b1dd7464805264530ab1d"
);
const RSA_MODULUS_BYTES: usize = 128;
const SECRET: &str = "ZdJqM15EeO2zWc08";
const APP_KEY: &str = "0AND0HD6FE4HY80F";
const CHANNEL_ID: &str = "10003505";
const PACKAGE_ID: &str = "com.tencent.qqmusic";
const HEX: &[u8; 16] = b"0123456789abcdef";

#[derive(Clone, Eq, PartialEq)]
pub(crate) struct QimeiIdentity {
    pub q16: String,
    pub q36: String,
}

pub(crate) async fn request_qimei(client: &Client, device: &QqDevice) -> Result<QimeiIdentity> {
    let timestamp = unix_seconds_now()?;
    let request = build_qimei_request(device, timestamp)?;
    let response = client
        .post(ENDPOINT)
        .header("Host", "api.tencentmusic.com")
        .header("method", "GetQimei")
        .header("service", "trpc.tme_datasvr.qimeiproxy.QimeiProxy")
        .header("appid", "qimei_qq_android")
        .header(
            "sign",
            md5_hex(&[
                "qimei_qq_androidpzAuCmaFAaFaHrdakPjLIEqKrGnSOOvH",
                &timestamp.to_string(),
            ]),
        )
        .header("user-agent", "QQMusic")
        .header("timestamp", timestamp.to_string())
        .json(&request)
        .send()
        .await
        .map_err(network_error)?;
    if !response.status().is_success() {
        return Err(TuneWeaveError::new(
            ErrorCode::UpstreamError,
            format!("QQ QIMEI service returned HTTP {}", response.status()),
        )
        .with_platform(Platform::Qq)
        .retryable(response.status().is_server_error()));
    }
    let response: Value = response.json().await.map_err(|error| {
        TuneWeaveError::new(
            ErrorCode::UpstreamError,
            format!("QQ QIMEI service returned invalid JSON: {error}"),
        )
        .with_platform(Platform::Qq)
    })?;
    let inner = response
        .get("data")
        .and_then(Value::as_str)
        .ok_or_else(|| qimei_data_error("response is missing its data envelope"))?;
    let inner: Value = serde_json::from_str(inner)
        .map_err(|error| qimei_data_error(format!("data envelope is invalid JSON: {error}")))?;
    let data = inner
        .get("data")
        .and_then(Value::as_object)
        .ok_or_else(|| qimei_data_error("data envelope is missing its identity object"))?;
    let q16 = data
        .get("q16")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| qimei_data_error("identity is missing q16"))?;
    let q36 = data
        .get("q36")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| qimei_data_error("identity is missing q36"))?;
    Ok(QimeiIdentity {
        q16: q16.to_owned(),
        q36: q36.to_owned(),
    })
}

fn build_qimei_request(device: &QqDevice, timestamp: u64) -> Result<Value> {
    let crypt_key = random_hex(16);
    let nonce = random_hex(16);
    let payload = serde_json::to_vec(&qimei_payload(device, timestamp)).map_err(|error| {
        TuneWeaveError::new(
            ErrorCode::InternalError,
            format!("failed to encode QQ QIMEI payload: {error}"),
        )
        .with_platform(Platform::Qq)
    })?;
    let encrypted_key = rsa_encrypt_pkcs1_v15(crypt_key.as_bytes())?;
    let encrypted_payload = aes_encrypt(crypt_key.as_bytes(), &payload);
    let key = BASE64.encode(encrypted_key);
    let params = BASE64.encode(encrypted_payload);
    let time_millis = timestamp.saturating_mul(1_000).to_string();
    let extra = format!("{{\"appKey\":\"{APP_KEY}\"}}");
    let sign = md5_hex(&[&key, &params, &time_millis, &nonce, SECRET, &extra]);
    Ok(json!({
        "app": 0,
        "os": 1,
        "qimeiParams": {
            "key": key,
            "params": params,
            "time": timestamp.to_string(),
            "nonce": nonce,
            "sign": sign,
            "extra": extra
        }
    }))
}

fn rsa_encrypt_pkcs1_v15(message: &[u8]) -> Result<Vec<u8>> {
    if message.len() > RSA_MODULUS_BYTES - 11 {
        return Err(TuneWeaveError::new(
            ErrorCode::InternalError,
            "QQ QIMEI RSA message exceeds the public-key capacity",
        )
        .with_platform(Platform::Qq));
    }
    let modulus = BigUint::parse_bytes(RSA_MODULUS.as_bytes(), 16).ok_or_else(|| {
        TuneWeaveError::new(ErrorCode::InternalError, "QQ QIMEI RSA modulus is invalid")
            .with_platform(Platform::Qq)
    })?;
    let mut encoded = Vec::with_capacity(RSA_MODULUS_BYTES);
    encoded.extend_from_slice(&[0, 2]);
    encoded.extend(
        (0..RSA_MODULUS_BYTES - message.len() - 3).map(|_| rand::random_range(1_u8..=u8::MAX)),
    );
    encoded.push(0);
    encoded.extend_from_slice(message);
    let encrypted = BigUint::from_bytes_be(&encoded)
        .modpow(&BigUint::from(65_537_u32), &modulus)
        .to_bytes_be();
    let mut padded = vec![0_u8; RSA_MODULUS_BYTES.saturating_sub(encrypted.len())];
    padded.extend_from_slice(&encrypted);
    Ok(padded)
}

fn qimei_payload(device: &QqDevice, timestamp: u64) -> Value {
    let random_uptime = rand::random_range(0_u64..=14_400);
    let uptime = timestamp.saturating_sub(random_uptime);
    let reserved = json!({
        "harmony": "0",
        "clone": "0",
        "containe": "",
        "oz": "UhYmelwouA+V2nPWbOvLTgN2/m8jwGB+yUB5v9tysQg=",
        "oo": "Xecjt+9S1+f8Pz2VLSxgpw==",
        "kelong": "0",
        "uptimes": format_utc_datetime(uptime),
        "multiUser": "0",
        "bod": "Xiaomi",
        "dv": "sagit",
        "firstLevel": "",
        "manufact": "Xiaomi",
        "name": "MI 6",
        "host": "se.infra",
        "kernel": device.proc_version
    });
    json!({
        "androidId": device.android_id,
        "platformId": 1,
        "appKey": APP_KEY,
        "appVersion": "14.9.0.8",
        "beaconIdSrc": random_beacon_id(timestamp),
        "brand": "Xiaomi",
        "channelId": CHANNEL_ID,
        "cid": "",
        "imei": device.imei,
        "imsi": "",
        "mac": "",
        "model": "MI 6",
        "networkType": "unknown",
        "oaid": "",
        "osVersion": "Android 10,level 29",
        "qimei": "",
        "qimei36": "",
        "sdkVersion": "1.2.13.6",
        "targetSdkVersion": "33",
        "audit": "",
        "userId": "{}",
        "packageId": PACKAGE_ID,
        "deviceType": "Phone",
        "sdkName": "",
        "reserved": serde_json::to_string(&reserved).expect("reserved QIMEI object is serializable")
    })
}

fn aes_encrypt(key: &[u8], plaintext: &[u8]) -> Vec<u8> {
    let key: &[u8; 16] = key
        .try_into()
        .expect("random QQ QIMEI key always contains 16 ASCII bytes");
    let message_len = plaintext.len();
    let mut buffer = vec![0_u8; message_len + 16];
    buffer[..message_len].copy_from_slice(plaintext);
    cbc::Encryptor::<Aes128>::new(key.into(), key.into())
        .encrypt_padded_mut::<Pkcs7>(&mut buffer, message_len)
        .expect("QQ QIMEI buffer always contains a full padding block")
        .to_vec()
}

fn random_beacon_id(timestamp: u64) -> String {
    let (year, month, _, _, _, _) = utc_components(timestamp);
    let rand1 = rand::random_range(100_000_u32..=999_999);
    let rand2 = rand::random_range(100_000_000_u32..=999_999_999);
    let mut result = String::new();
    for index in 1..=40 {
        use std::fmt::Write as _;
        if matches!(
            index,
            1 | 2 | 13 | 14 | 17 | 18 | 21 | 22 | 25 | 26 | 29 | 30 | 33 | 34 | 37 | 38
        ) {
            let _ = write!(result, "k{index}:{year:04}-{month:02}-01{rand1}.{rand2};");
        } else if index == 3 {
            result.push_str("k3:0000000000000000;");
        } else if index == 4 {
            let _ = write!(result, "k4:{};", random_nonzero_hex(16));
        } else {
            let _ = write!(result, "k{index}:{};", rand::random_range(0_u16..=9_999));
        }
    }
    result
}

fn format_utc_datetime(timestamp: u64) -> String {
    let (year, month, day, hour, minute, second) = utc_components(timestamp);
    format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}:{second:02}")
}

fn utc_components(timestamp: u64) -> (i64, i64, i64, u64, u64, u64) {
    let days = i64::try_from(timestamp / 86_400).unwrap_or(i64::MAX);
    let seconds = timestamp % 86_400;
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_part = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_part + 2) / 5 + 1;
    let month = month_part + if month_part < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    (
        year,
        month,
        day,
        seconds / 3_600,
        seconds % 3_600 / 60,
        seconds % 60,
    )
}

fn random_hex(length: usize) -> String {
    (0..length)
        .map(|_| char::from(HEX[rand::random_range(0..HEX.len())]))
        .collect()
}

fn random_nonzero_hex(length: usize) -> String {
    (0..length)
        .map(|_| char::from(HEX[rand::random_range(1..HEX.len())]))
        .collect()
}

fn md5_hex(parts: &[&str]) -> String {
    let mut digest = Md5::new();
    for part in parts {
        digest.update(part.as_bytes());
    }
    format!("{:x}", digest.finalize())
}

fn network_error(error: reqwest::Error) -> TuneWeaveError {
    let code = if error.is_timeout() {
        ErrorCode::UpstreamTimeout
    } else {
        ErrorCode::UpstreamError
    };
    TuneWeaveError::new(code, format!("QQ QIMEI request failed: {error}"))
        .with_platform(Platform::Qq)
        .retryable(true)
}

fn qimei_data_error(message: impl Into<String>) -> TuneWeaveError {
    TuneWeaveError::new(ErrorCode::UpstreamError, message)
        .with_platform(Platform::Qq)
        .retryable(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utc_conversion_covers_epoch_and_leap_day() {
        assert_eq!(format_utc_datetime(0), "1970-01-01 00:00:00");
        assert_eq!(format_utc_datetime(1_709_164_799), "2024-02-28 23:59:59");
        assert_eq!(format_utc_datetime(1_709_164_800), "2024-02-29 00:00:00");
    }

    #[test]
    fn beacon_contains_all_forty_fields() {
        let beacon = random_beacon_id(1_709_164_800);
        assert_eq!(beacon.matches(';').count(), 40);
        assert!(beacon.starts_with("k1:2024-02-01"));
        assert!(beacon.contains(";k3:0000000000000000;"));
    }

    #[test]
    fn request_builder_encrypts_private_device_fields() {
        let device = QqDevice::default();
        let request = build_qimei_request(&device, 1_709_164_800).expect("build request");
        let encoded = serde_json::to_string(&request).expect("serialize request");
        assert!(!encoded.contains(&device.imei));
        assert!(!encoded.contains(&device.android_id));
        assert_eq!(request["qimeiParams"]["time"], "1709164800");
    }
}
