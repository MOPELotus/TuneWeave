//! The native login protocol uses raw RSA and AES-256, unlike device registration.

use aes::{
    Aes256,
    cipher::{BlockDecryptMut, BlockEncryptMut, KeyIvInit, block_padding::Pkcs7},
};
use md5::{Digest, Md5};
use num_bigint::BigUint;
use rand::{TryRng, rngs::SysRng};
use serde::Serialize;
use tuneweave_core::{ErrorCode, Platform, Result, TuneWeaveError};

use crate::{KugouLoginClient, credential::NativeSession};

const STANDARD_MODULUS: &str = "c8006ed03842d2628209bd314984ca5ed6cfe06e30c95f9d4704d9c49791d7a935ba950ecb0bc8ebf5f5994f0bac927a7eb151b3c1de343303fa539c83136eccfd7d7e511e2dbce18eaa9f784c9b50d443e75865979e0a5e216e46c684066a8d6b998580bbaa22d73f5790286bb14742e83244e44db6d707ffe162c5c7002d45";
const CONCEPT_MODULUS: &str = "c40a2d0da76511f3bb1cc2bbd3afbd8bea83b4d6b05b6c13eb8920c53f1af7679b32ba0d0edb843240ef1b836efed3ee240734c14c1399fd6594d16af22f52525d14d72e0155c6dcc8638d4f7bb94f3a0b1f4c29f991972f2a160a25eb0a9e724336be7f69bbd319ffab1c6dd8470b021dc434f3faba89f4a2a01b33bdbdd08b";
const STANDARD_KEY: &[u8; 32] = b"90b8382a1bb4ccdcf063102053fd75b8";
const CONCEPT_KEY: &[u8; 32] = b"c24f74ca2820225badc01946dba4fdf7";
const T1_KEY: &[u8; 32] = b"5e4ef500e9597fe004bd09a46d8add98";
const T2_KEY: &[u8; 32] = b"fd14b35e3f81af3817a20ae7adae7020";
// Explicit desktop identity when no Android MAC/IMEI/model is available.
pub(crate) const DESKTOP_MODEL: &str = "TuneWeave";
const DESKTOP_MAC: &str = "02:00:00:00:00:00";
const NO_IMEI_MD5: &str = "0f607264fc6318a92b9e13c65db7cd3c";

pub(crate) mod native_standard;

pub(crate) struct ExchangeCipher {
    seed: String,
}

impl std::fmt::Debug for ExchangeCipher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ExchangeCipher { seed: [redacted] }")
    }
}

impl ExchangeCipher {
    #[cfg(test)]
    pub(crate) fn for_test(seed: &str) -> Self {
        assert_eq!(seed.len(), 32);
        assert!(seed.bytes().all(|byte| byte.is_ascii_hexdigit()));
        Self {
            seed: seed.to_owned(),
        }
    }

    pub(crate) fn random() -> Result<Self> {
        let mut bytes = [0; 16];
        SysRng.try_fill_bytes(&mut bytes).map_err(|_| internal())?;
        Ok(Self {
            seed: hex::encode_upper(bytes),
        })
    }

    pub(crate) fn pk(&self, client: KugouLoginClient, milliseconds: u64) -> Result<String> {
        #[derive(Serialize)]
        struct Secret<'a> {
            clienttime_ms: u64,
            key: &'a str,
        }
        rsa(
            client,
            &encode(&Secret {
                clienttime_ms: milliseconds,
                key: &self.seed,
            })?,
        )
    }

    pub(crate) fn password_pk(&self, timestamp: &str) -> Result<String> {
        #[derive(Serialize)]
        struct Secret<'a> {
            clienttime_ms: &'a str,
            key: &'a str,
        }
        rsa(
            KugouLoginClient::Standard,
            &encode(&Secret {
                clienttime_ms: timestamp,
                key: &self.seed,
            })?,
        )
    }

    pub(crate) fn encrypt(&self, plain: &[u8]) -> Result<String> {
        encrypt(plain, &self.key())
    }

    pub(crate) fn decrypt(&self, ciphertext: &str) -> Result<Vec<u8>> {
        if ciphertext.is_empty() || ciphertext.len() > 131_072 || ciphertext.len() % 32 != 0 {
            return Err(malformed());
        }
        let mut bytes = hex::decode(ciphertext).map_err(|_| malformed())?;
        let key = self.key();
        let decryptor =
            cbc::Decryptor::<Aes256>::new_from_slices(&key, &key[16..]).map_err(|_| internal())?;
        decryptor
            .decrypt_padded_mut::<Pkcs7>(&mut bytes)
            .map(<[u8]>::to_vec)
            .map_err(|_| malformed())
    }

    fn key(&self) -> [u8; 32] {
        let hex = hex::encode(Md5::digest(self.seed.as_bytes()));
        let mut key = [0; 32];
        key.copy_from_slice(hex.as_bytes());
        key
    }
}

pub(crate) fn profile_p(client: KugouLoginClient, token: &str, seconds: u64) -> Result<String> {
    #[derive(Serialize)]
    struct Secret<'a> {
        token: &'a str,
        clienttime: u64,
    }
    rsa(
        client,
        &encode(&Secret {
            token,
            clienttime: seconds,
        })?,
    )
}

pub(crate) fn p3(session: &NativeSession, seconds: u64) -> Result<String> {
    #[derive(Serialize)]
    struct Secret<'a> {
        clienttime: u64,
        token: &'a str,
    }
    let key = match session.client {
        KugouLoginClient::Standard => STANDARD_KEY,
        KugouLoginClient::Concept => CONCEPT_KEY,
        KugouLoginClient::Web => return Err(unsupported()),
    };
    encrypt(
        &encode(&Secret {
            clienttime: seconds,
            token: &session.token,
        })?,
        key,
    )
}

pub(crate) fn concept_fingerprints(
    session: &NativeSession,
    milliseconds: u64,
) -> Result<(String, String)> {
    let t1 = format!("{}|{milliseconds}", session.t1.as_deref().unwrap_or(""));
    let t2 = format!(
        "{}|{NO_IMEI_MD5}|{DESKTOP_MAC}|{DESKTOP_MODEL}|{milliseconds}",
        session.device.guid
    );
    Ok((
        encrypt(t1.as_bytes(), T1_KEY)?,
        encrypt(t2.as_bytes(), T2_KEY)?,
    ))
}

pub(crate) fn encode(value: &impl Serialize) -> Result<Vec<u8>> {
    serde_json::to_vec(value).map_err(|_| internal())
}

fn encrypt(plain: &[u8], key: &[u8; 32]) -> Result<String> {
    let mut bytes = vec![0; plain.len() + 16];
    bytes[..plain.len()].copy_from_slice(plain);
    let encryptor =
        cbc::Encryptor::<Aes256>::new_from_slices(key, &key[16..]).map_err(|_| internal())?;
    Ok(hex::encode(
        encryptor
            .encrypt_padded_mut::<Pkcs7>(&mut bytes, plain.len())
            .map_err(|_| internal())?,
    ))
}

fn rsa(client: KugouLoginClient, plain: &[u8]) -> Result<String> {
    if plain.is_empty() || plain.len() > 128 {
        return Err(TuneWeaveError::new(
            ErrorCode::UpstreamError,
            "KuGou login RSA payload exceeds the protocol capacity",
        )
        .with_platform(Platform::Kugou));
    }
    let modulus = rsa_modulus(client)?;
    let modulus = BigUint::parse_bytes(modulus.as_bytes(), 16).ok_or_else(internal)?;
    let mut padded = [0; 128];
    padded[..plain.len()].copy_from_slice(plain);
    let number = BigUint::from_bytes_be(&padded);
    if number >= modulus {
        return Err(malformed());
    }
    let encrypted = number
        .modpow(&BigUint::from(65_537u32), &modulus)
        .to_bytes_be();
    let mut output = [0; 128];
    output[128 - encrypted.len()..].copy_from_slice(&encrypted);
    Ok(hex::encode_upper(output))
}

pub(crate) fn rsa_modulus(client: KugouLoginClient) -> Result<&'static str> {
    match client {
        KugouLoginClient::Standard => Ok(STANDARD_MODULUS),
        KugouLoginClient::Concept => Ok(CONCEPT_MODULUS),
        KugouLoginClient::Web => Err(unsupported()),
    }
}

fn unsupported() -> TuneWeaveError {
    TuneWeaveError::new(
        ErrorCode::CapabilityNotSupported,
        "KuGou Web authorization requires a separate cookie exchange",
    )
    .with_platform(Platform::Kugou)
}
fn internal() -> TuneWeaveError {
    TuneWeaveError::new(ErrorCode::InternalError, "KuGou login cryptography failed")
        .with_platform(Platform::Kugou)
}
fn malformed() -> TuneWeaveError {
    TuneWeaveError::new(
        ErrorCode::UpstreamError,
        "KuGou login encrypted response is invalid",
    )
    .with_platform(Platform::Kugou)
}

#[cfg(test)]
mod tests;
