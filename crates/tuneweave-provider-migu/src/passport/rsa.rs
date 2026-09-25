use num_bigint::BigUint;
use rand::{TryRng, rngs::SysRng};
use tuneweave_core::{ErrorCode, Result};

use crate::credential::error;

/// The current passport service uses a legacy 1024-bit key. This protocol adapter
/// accepts bounded dynamic keys; it is not a general-purpose encryption API.
pub(crate) struct LoginPublicKey {
    modulus: BigUint,
    exponent: BigUint,
    bytes: usize,
}

impl std::fmt::Debug for LoginPublicKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LoginPublicKey")
            .field("bits", &self.modulus.bits())
            .finish_non_exhaustive()
    }
}

impl LoginPublicKey {
    pub(crate) fn parse(modulus: &str, exponent: &str) -> Result<Self> {
        let valid_hex = |s: &str, limit| {
            !s.is_empty() && s.len() <= limit && s.bytes().all(|b| b.is_ascii_hexdigit())
        };
        if !valid_hex(modulus, 1026) || !valid_hex(exponent, 8) {
            return Err(invalid_key());
        }
        let modulus = BigUint::parse_bytes(modulus.as_bytes(), 16).ok_or_else(invalid_key)?;
        let exponent = BigUint::parse_bytes(exponent.as_bytes(), 16).ok_or_else(invalid_key)?;
        let bits = modulus.bits();
        if !(1024..=4096).contains(&bits)
            || !modulus.bit(0)
            || !exponent.bit(0)
            || exponent < BigUint::from(3_u8)
            || exponent > BigUint::from(65_537_u32)
        {
            return Err(invalid_key());
        }
        Ok(Self {
            modulus,
            exponent,
            bytes: bits.div_ceil(8) as usize,
        })
    }

    pub(crate) fn encrypt(&self, text: &str) -> Result<String> {
        self.encrypt_with_entropy(text, |bytes| {
            SysRng.try_fill_bytes(bytes).map_err(|_| {
                error(
                    ErrorCode::InternalError,
                    "Migu login entropy is unavailable",
                )
            })
        })
    }

    fn encrypt_with_entropy(
        &self,
        text: &str,
        mut fill: impl FnMut(&mut [u8]) -> Result<()>,
    ) -> Result<String> {
        // Official JS encodes UTF-16 code units separately, including surrogate pairs.
        // Enforce the actual encoded byte limit before padding (RFC 8017 section 7.2.1).
        let message = encode_login_text(text, self.bytes - 11)?;
        let padding_len = self.bytes - message.len() - 3;
        let mut block = vec![0; self.bytes];
        block[1] = 2;
        let padding = &mut block[2..2 + padding_len];
        fill(padding)?;
        for byte in padding {
            // Rejection sampling keeps nonzero bytes uniform. Bound broken entropy sources.
            let mut attempts = 0;
            while *byte == 0 {
                if attempts == 128 {
                    return Err(error(
                        ErrorCode::InternalError,
                        "Migu login entropy failed to produce padding",
                    ));
                }
                fill(std::slice::from_mut(byte))?;
                attempts += 1;
            }
        }
        block[3 + padding_len..].copy_from_slice(&message);
        let encrypted = BigUint::from_bytes_be(&block).modpow(&self.exponent, &self.modulus);
        // Passport's wire format strips leading zero octets, then pads to an even hex length.
        Ok(hex::encode(encrypted.to_bytes_be()))
    }
}

fn encode_login_text(text: &str, max_bytes: usize) -> Result<Vec<u8>> {
    if text.len() > max_bytes {
        return Err(message_too_long());
    }
    let mut bytes = Vec::with_capacity(text.len());
    for unit in text.encode_utf16() {
        match unit {
            0..=0x7f => bytes.push(unit as u8),
            0x80..=0x7ff => {
                bytes.extend_from_slice(&[(0xc0 | (unit >> 6)) as u8, (0x80 | (unit & 0x3f)) as u8])
            }
            _ => bytes.extend_from_slice(&[
                (0xe0 | (unit >> 12)) as u8,
                (0x80 | ((unit >> 6) & 0x3f)) as u8,
                (0x80 | (unit & 0x3f)) as u8,
            ]),
        }
        if bytes.len() > max_bytes {
            return Err(message_too_long());
        }
    }
    Ok(bytes)
}
fn message_too_long() -> tuneweave_core::TuneWeaveError {
    error(
        ErrorCode::InvalidRequest,
        "Migu login field exceeds the current public key capacity",
    )
}
fn invalid_key() -> tuneweave_core::TuneWeaveError {
    error(
        ErrorCode::UpstreamError,
        "Migu passport returned an invalid or unsupported public key",
    )
}

#[cfg(test)]
pub(crate) mod tests;
