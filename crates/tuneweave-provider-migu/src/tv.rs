use std::time::{SystemTime, UNIX_EPOCH};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use des::{
    TdesEde3,
    cipher::{BlockDecryptMut, BlockEncryptMut, KeyInit, block_padding::Pkcs7},
};
use md5::{Digest, Md5};
use serde::{Deserialize, Serialize};
use tuneweave_core::{ErrorCode, Platform, Result, TuneWeaveError};

const PROTOCOL_SECRET: &str = "9HkocpYLeG1LNi5m";

fn failure() -> TuneWeaveError {
    TuneWeaveError::new(
        ErrorCode::UpstreamError,
        "Migu TV returned an invalid protocol response",
    )
    .with_platform(Platform::Migu)
}

#[derive(Clone)]
pub(crate) struct Device {
    id: String,
    mac: String,
}

impl Default for Device {
    fn default() -> Self {
        Self {
            id: format!("{:016x}", rand::random::<u64>()),
            mac: hex::encode_upper(rand::random::<[u8; 6]>()),
        }
    }
}

#[derive(Serialize)]
struct Request<'a> {
    #[serde(rename = "contentId")]
    content_id: &'a str,
    #[serde(rename = "songId")]
    song_id: &'a str,
    #[serde(rename = "toneFlag")]
    tone: &'a str,
    imei: &'a str,
    imsi: &'a str,
    stbid: &'a str,
    mac: &'a str,
    apn: &'static str,
    channel: &'static str,
    ip: &'static str,
    ua: &'static str,
    osid: &'static str,
    protocolver: &'static str,
    version: &'static str,
    stbserial: &'static str,
    accountid: &'static str,
    hwlevel: &'static str,
    mobilephone: &'static str,
}

#[derive(Deserialize)]
struct Envelope {
    message: String,
    #[serde(default)]
    body: String,
    #[serde(default)]
    salt: String,
}

#[derive(Deserialize)]
struct Payload {
    #[serde(rename = "musicSongListVo")]
    song: Song,
}

#[derive(Deserialize)]
pub(crate) struct Song {
    #[serde(rename = "zySongId")]
    pub song_id: String,
    #[serde(default)]
    pub hq: Option<Rendition>,
    #[serde(default)]
    pub nq: Option<Rendition>,
    #[serde(default)]
    pub sq: Option<Rendition>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Rendition {
    pub content_id: String,
    pub copyright_id: String,
    #[serde(default)]
    pub url: String,
}

fn key(salt: &str) -> Result<[u8; 24]> {
    if salt.len() != 6 || !salt.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(failure());
    }
    let digest = hex::encode(Md5::digest(format!("{salt}{PROTOCOL_SECRET}").as_bytes()));
    let mut key = [0; 24];
    key.copy_from_slice(&digest.as_bytes()[..24]);
    Ok(key)
}

fn encrypt(bytes: &[u8], salt: &str) -> Result<String> {
    let mut buffer = vec![0; bytes.len() + 8];
    buffer[..bytes.len()].copy_from_slice(bytes);
    let encrypted = ecb::Encryptor::<TdesEde3>::new(&key(salt)?.into())
        .encrypt_padded_mut::<Pkcs7>(&mut buffer, bytes.len())
        .map_err(|_| failure())?;
    Ok(STANDARD.encode(encrypted))
}

fn decrypt(value: &str, salt: &str) -> Result<Vec<u8>> {
    let mut bytes = STANDARD.decode(value).map_err(|_| failure())?;
    let plain = ecb::Decryptor::<TdesEde3>::new(&key(salt)?.into())
        .decrypt_padded_mut::<Pkcs7>(&mut bytes)
        .map_err(|_| failure())?;
    Ok(plain.to_vec())
}

pub(crate) async fn song(
    http: &reqwest::Client,
    device: &Device,
    content_id: &str,
    song_id: &str,
    tone: &str,
) -> Result<Song> {
    let request = Request {
        content_id,
        song_id,
        tone,
        imei: &device.id,
        imsi: &device.id,
        stbid: &device.id,
        mac: &device.mac,
        apn: "wifi",
        channel: "014B702",
        ip: "|192.168.1.2",
        ua: "okhttp/3.12.0",
        osid: "Android-TV",
        protocolver: "2.0.0",
        version: "2.4.001",
        stbserial: "",
        accountid: "",
        hwlevel: "0",
        mobilephone: "",
    };
    let salt = format!("{:06}", rand::random_range(0..1_000_000));
    let data = encrypt(&serde_json::to_vec(&request).map_err(|_| failure())?, &salt)?;
    let time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| failure())?
        .as_secs()
        .to_string();
    let service = "tvMusicSongFileService";
    let version = "2.0";
    let token = hex::encode(Md5::digest(
        format!("{service}{time}{data}{salt}{version}{PROTOCOL_SECRET}").as_bytes(),
    ));
    let form = reqwest::multipart::Form::new()
        .text("service", service)
        .text("time", time)
        .text("data", data)
        .text("salt", salt)
        .text("version", version)
        .text("token", token);
    let mut response = http
        .post("https://tv.ising.migu.cn/do")
        .multipart(form)
        .send()
        .await
        .map_err(|_| failure())?;
    if !response.status().is_success() {
        return Err(failure());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| failure())? {
        if bytes.len().saturating_add(chunk.len()) > 2 * 1024 * 1024 {
            return Err(failure());
        }
        bytes.extend_from_slice(&chunk);
    }
    let envelope: Envelope = serde_json::from_slice(&bytes).map_err(|_| failure())?;
    if envelope.message != "success" {
        return Err(TuneWeaveError::new(
            ErrorCode::PermissionDenied,
            "Migu TV did not authorize media",
        )
        .with_platform(Platform::Migu));
    }
    let payload: Payload =
        serde_json::from_slice(&decrypt(&envelope.body, &envelope.salt)?).map_err(|_| failure())?;
    if payload.song.song_id != song_id {
        return Err(failure());
    }
    Ok(payload.song)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tv_cipher_roundtrip_rejects_invalid_salts_and_ciphertext() {
        let input = br#"{"songId":"123"}"#;
        let encrypted = encrypt(input, "123456").unwrap();
        // Independent .NET TripleDES ECB/PKCS7 vector, not generated by this implementation.
        assert_eq!(encrypted, "/WEB+zC4sYNZNcDDcz0kxcZQdqq8AiWQ");
        assert_eq!(decrypt(&encrypted, "123456").unwrap(), input);
        assert!(decrypt("not base64", "123456").is_err());
        assert!(decrypt(&encrypted, "bad").is_err());
        assert!(decrypt(&encrypted, "654321").is_err());
    }
}
