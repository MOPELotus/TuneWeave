use aes::{
    Aes256,
    cipher::{BlockEncryptMut, KeyIvInit, block_padding::Pkcs7},
};
use md5::{Digest, Md5};
use num_bigint::BigUint;
use rand::{TryRng, rngs::SysRng};
use serde::Serialize;
use tuneweave_core::{ErrorCode, Result};

use super::{error, invalid};

const MODULUS: &str = "B1B1EC76A1BBDBF0D18E8CD9A87E53FA3881E2F004C67C9DDA2CA677DBEFA3D61DF8463FE12D84FF4B4699E02C9D41CAB917F5A8FB9E35580C4BDF97763A0420A476295D763EE10174E6F9EBF7DF8A77BA5B20CDA4EE705DEF5BBA3C88567B9656E52C9CD5CD95CA735FF2D25F762B133273EEEB7B4F3EA8B6DA29040F3B67CD";

pub(super) struct WebCipher {
    seed: String,
}
impl std::fmt::Debug for WebCipher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("WebCipher { seed: [redacted] }")
    }
}
impl WebCipher {
    #[cfg(test)]
    pub(crate) fn test_cipher() -> Self {
        Self {
            seed: "0123456789ABCDEF".to_owned(),
        }
    }
    pub(super) fn random() -> Result<Self> {
        const ALPHABET: &[u8] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ";
        let mut seed = String::with_capacity(16);
        // Rejection sampling avoids modulo bias; the official wire format is 16 characters.
        while seed.len() < 16 {
            let mut random = [0; 32];
            SysRng
                .try_fill_bytes(&mut random)
                .map_err(|_| error(ErrorCode::InternalError, "KuGou Web randomness unavailable"))?;
            for byte in random {
                if byte < 252 {
                    seed.push(char::from(ALPHABET[usize::from(byte % 36)]));
                }
                if seed.len() == 16 {
                    break;
                }
            }
        }
        Ok(Self { seed })
    }

    pub(super) fn pk(&self, milliseconds: u64) -> Result<String> {
        #[derive(Serialize)]
        struct Secret<'a> {
            clienttime_ms: u64,
            key: &'a str,
        }
        rsa(&crate::login::crypto::encode(&Secret {
            clienttime_ms: milliseconds,
            key: &self.seed,
        })?)
    }

    pub(super) fn token(&self, token: &str) -> Result<String> {
        if !crate::credential::valid_secret(token) {
            return Err(invalid());
        }
        #[derive(Serialize)]
        struct Secret<'a> {
            token: &'a str,
        }
        self.encrypt(&Secret { token })
    }

    pub(super) fn encrypt(&self, value: &impl Serialize) -> Result<String> {
        let plain = crate::login::crypto::encode(value)?;
        if plain.len() > 65_536 {
            return Err(invalid());
        }
        let key = hex::encode(Md5::digest(self.seed.as_bytes()));
        let mut buffer = vec![0; plain.len() + 16];
        buffer[..plain.len()].copy_from_slice(&plain);
        let cipher =
            cbc::Encryptor::<Aes256>::new_from_slices(key.as_bytes(), &key.as_bytes()[16..])
                .map_err(|_| invalid())?;
        Ok(hex::encode(
            cipher
                .encrypt_padded_mut::<Pkcs7>(&mut buffer, plain.len())
                .map_err(|_| invalid())?,
        ))
    }
}

fn rsa(plain: &[u8]) -> Result<String> {
    if plain.is_empty() || plain.len() > 128 || !plain.is_ascii() {
        return Err(invalid());
    }
    let modulus = BigUint::parse_bytes(MODULUS.as_bytes(), 16).ok_or_else(invalid)?;
    let mut padded = [0; 128];
    padded[..plain.len()].copy_from_slice(plain);
    let value = BigUint::from_bytes_be(&padded);
    if value >= modulus {
        return Err(invalid());
    }
    let hex = value
        .modpow(&BigUint::from(65_537u32), &modulus)
        .to_str_radix(16);
    // The official BigInt encoder emits lowercase groups of four hex digits, not fixed 128 bytes.
    Ok(format!(
        "{:0>width$}",
        hex,
        width = hex.len().div_ceil(4) * 4
    ))
}

#[cfg(test)]
mod tests;
