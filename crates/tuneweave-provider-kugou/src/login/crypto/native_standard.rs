//! Standard Android 20.8.0 (20809) fingerprints. These differ from the older
//! Concept protocol, including the timestamp's dependency on the t2 ciphertext.

use super::{DESKTOP_MAC, DESKTOP_MODEL, Digest, Md5, Result, encrypt};

const TOKEN_KEY: &[u8; 32] = b"fd387891254e6cedc4019ca0061ea6d9";
const DEVICE_KEY: &[u8; 32] = b"ce1e88d78dff2132dbfbb91af0ea9ca7";

pub(crate) struct Fingerprint {
    pub t1: String,
    pub t2: String,
    pub timestamp: String,
}

impl Fingerprint {
    pub(crate) fn fresh_desktop(milliseconds: u64) -> Result<Self> {
        // The host has no Android ID, telephony identity or saved native device
        // token. Keep the native unavailable-ID fallbacks and identify our host.
        generate("", "", "", DESKTOP_MODEL, milliseconds)
    }
}

fn generate(
    previous_token: &str,
    android_id: &str,
    imei: &str,
    model: &str,
    milliseconds: u64,
) -> Result<Fingerprint> {
    let android_id = if android_id.is_empty() {
        String::new()
    } else {
        hex::encode(Md5::digest(android_id.as_bytes()))
    };
    let phone_id = hex::encode(Md5::digest(
        if imei.is_empty() { DESKTOP_MAC } else { imei }.as_bytes(),
    ));
    let t1 = encrypt(
        format!("{previous_token}|{milliseconds}").as_bytes(),
        TOKEN_KEY,
    )?;
    let t2 = encrypt(
        format!("{android_id}|{phone_id}|{DESKTOP_MAC}|{model}|{milliseconds}").as_bytes(),
        DEVICE_KEY,
    )?;
    let suffix = t2
        .bytes()
        .fold(0_u64, |hash, byte| (hash * 31 + u64::from(byte)) % 677);
    let timestamp = (milliseconds / 1000 * 1000)
        .checked_add(suffix)
        .ok_or_else(super::internal)?
        .to_string();
    Ok(Fingerprint { t1, t2, timestamp })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Deserialize)]
    struct Vector {
        previous_token: String,
        android_id: String,
        imei: String,
        model: String,
        milliseconds: u64,
        t1: String,
        t2: String,
        clienttime_ms: String,
    }

    #[test]
    fn standard_fingerprints_match_native_arm64_and_openssl_vectors() {
        // Native f4/f6/f7 and hex formatting ran under bounded ARM64 emulation;
        // OpenSSL supplied AES, with synthetic clock/device data and no accounts.
        let vectors: Vec<Vector> =
            serde_json::from_str(include_str!("native_standard_vectors.json")).unwrap();
        for v in vectors {
            let fingerprint = generate(
                &v.previous_token,
                &v.android_id,
                &v.imei,
                &v.model,
                v.milliseconds,
            )
            .unwrap();
            assert_eq!(fingerprint.t1, v.t1);
            assert_eq!(fingerprint.t2, v.t2);
            assert_eq!(fingerprint.timestamp, v.clienttime_ms);
        }
    }
}
