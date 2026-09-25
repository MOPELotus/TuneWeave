//! Bounded decoding of the fixed official-client RSA response format.
use super::{invalid, key};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use num_bigint::BigUint;
use std::sync::OnceLock;
use tuneweave_core::Result;

const BLOCK: usize = 128;
const SEPARATOR: &[u8] = b"#PART#";
const MAX_BLOCKS: usize = 256;
const MAX_WIRE: usize = 64 * 1024;

fn parameters() -> &'static (BigUint, BigUint) {
    static KEY: OnceLock<(BigUint, BigUint)> = OnceLock::new();
    KEY.get_or_init(|| {
        (
            BigUint::parse_bytes(key::MODULUS.as_bytes(), 16).expect("fixed protocol modulus"),
            BigUint::parse_bytes(key::EXPONENT.as_bytes(), 16).expect("fixed protocol exponent"),
        )
    })
}

pub(super) fn decode(wire: &[u8]) -> Result<Vec<u8>> {
    if wire.is_empty() || wire.len() > MAX_WIRE {
        return Err(invalid());
    }
    // Android's Base64.DEFAULT accepts wrapping whitespace; it does not turn
    // an HTML response, JSON or a plaintext success into an encrypted response.
    let compact: Vec<_> = wire
        .iter()
        .copied()
        .filter(|b| !b.is_ascii_whitespace())
        .collect();
    let cipher = STANDARD.decode(compact).map_err(|_| invalid())?;
    let width = BLOCK + SEPARATOR.len();
    if cipher.len() < BLOCK || (cipher.len() + SEPARATOR.len()) % width != 0 {
        return Err(invalid());
    }
    let count = (cipher.len() + SEPARATOR.len()) / width;
    if count > MAX_BLOCKS {
        return Err(invalid());
    }
    // Validate the complete framing before doing any modular exponentiation.
    for chunk in cipher.chunks(width).take(count.saturating_sub(1)) {
        if &chunk[BLOCK..] != SEPARATOR {
            return Err(invalid());
        }
    }
    let (modulus, exponent) = parameters();
    let mut plain = Vec::with_capacity(count * 117);
    for chunk in cipher.chunks(width) {
        let value = BigUint::from_bytes_be(&chunk[..BLOCK]);
        if &value >= modulus {
            return Err(invalid());
        }
        // This shared key ships in the public client; it is not an account
        // secret. Authorization is independently checked before this request.
        let bytes = value.modpow(exponent, modulus).to_bytes_be();
        let mut padded = [0_u8; BLOCK];
        if bytes.len() > BLOCK {
            return Err(invalid());
        }
        padded[BLOCK - bytes.len()..].copy_from_slice(&bytes);
        if padded[..2] != [0, 2] {
            return Err(invalid());
        }
        let stop = padded[2..]
            .iter()
            .position(|b| *b == 0)
            .map(|n| n + 2)
            .ok_or_else(invalid)?;
        if stop < 10 || stop == BLOCK - 1 {
            return Err(invalid());
        }
        plain.extend_from_slice(&padded[stop + 1..]);
    }
    Ok(plain)
}

#[cfg(test)]
pub(crate) fn encrypt(plain: &[u8]) -> Vec<u8> {
    let (modulus, _) = parameters();
    let mut cipher = Vec::new();
    for (index, chunk) in plain.chunks(117).enumerate() {
        if index > 0 {
            cipher.extend_from_slice(SEPARATOR);
        }
        let mut padded = [0x35; BLOCK];
        padded[..2].copy_from_slice(&[0, 2]);
        padded[BLOCK - chunk.len() - 1] = 0;
        padded[BLOCK - chunk.len()..].copy_from_slice(chunk);
        let bytes = BigUint::from_bytes_be(&padded)
            .modpow(&BigUint::from(65537_u32), modulus)
            .to_bytes_be();
        cipher.extend(std::iter::repeat_n(0, BLOCK - bytes.len()));
        cipher.extend(bytes);
    }
    STANDARD.encode(cipher).into_bytes()
}
