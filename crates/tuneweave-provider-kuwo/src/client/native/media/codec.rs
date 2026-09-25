//! Fixed native-client query encoding. These protocol constants are public,
//! unrelated to the selected account's SID or its per-song authorization token.
use super::*;
use aes::{
    Aes128,
    cipher::{BlockEncrypt, KeyInit},
};
use base64::engine::general_purpose::URL_SAFE;

const MATERIAL: &[u8] = b"$Aivn&2@BZ*A$qsv";
const APK_PUBLIC_KEY_DIGEST: &str = "a26692b114b983a72a19db4d6ce652bf";
const MAX_PLAIN: usize = 32 * 1024;

pub(super) fn seal(mut values: BTreeMap<&str, String>) -> Result<String> {
    if values.len() > 64 || values.contains_key("kwsign") {
        return Err(kuwo_invalid_request("Kuwo media query is invalid"));
    }
    values.insert("kpk", APK_PUBLIC_KEY_DIGEST.into());
    values.insert("kwso", "kwplayer_ar".into());
    let size = values
        .iter()
        .try_fold(MATERIAL.len() + MEDIA_PATH.len(), |size, (key, value)| {
            size.checked_add(key.len())?.checked_add(value.len())
        })
        .filter(|size| *size <= MAX_PLAIN)
        .ok_or_else(|| kuwo_invalid_request("Kuwo media query exceeds its limit"))?;
    let mut signed = Vec::with_capacity(size);
    signed.extend_from_slice(MATERIAL);
    for (key, value) in &values {
        signed.extend_from_slice(key.as_bytes());
        signed.extend_from_slice(value.as_bytes());
    }
    signed.extend_from_slice(MEDIA_PATH.as_bytes());
    values.insert("kwsign", digest(&signed));
    let plain = values
        .iter()
        .map(|(key, value)| format!("{}={}", percent(key.as_bytes()), percent(value.as_bytes())))
        .collect::<Vec<_>>()
        .join("&");
    if plain.len() > MAX_PLAIN {
        return Err(kuwo_invalid_request("Kuwo media query exceeds its limit"));
    }
    Ok(encrypt(plain.as_bytes(), MATERIAL))
}

fn encrypt(plain: &[u8], material: &[u8]) -> String {
    if plain.is_empty() {
        return String::new();
    }
    let key = digest(material);
    let cipher = Aes128::new_from_slice(&key.as_bytes()[7..23]).expect("fixed protocol key length");
    let padding = 16 - plain.len() % 16;
    let mut data = plain.to_vec();
    data.resize(data.len() + padding, padding as u8);
    for block in data.chunks_exact_mut(16) {
        cipher.encrypt_block(block.into());
    }
    URL_SAFE.encode(data).replace('=', ".")
}

fn percent(bytes: &[u8]) -> String {
    const HEX: &[u8] = b"0123456789ABCDEF";
    let mut out = String::with_capacity(bytes.len());
    for &byte in bytes {
        if byte.is_ascii_alphanumeric() || b"!$'*-._".contains(&byte) {
            out.push(char::from(byte));
        } else {
            out.push('%');
            out.push(char::from(HEX[(byte >> 4) as usize]));
            out.push(char::from(HEX[(byte & 15) as usize]));
        }
    }
    out
}

/// The two nonzero seeds used by this protocol select the first MD5 round
/// constant for all 64 rounds. A conventional MD5 implementation is different.
fn digest(input: &[u8]) -> String {
    const SHIFTS: [u32; 16] = [7, 12, 17, 22, 5, 9, 14, 20, 4, 11, 16, 23, 6, 10, 15, 21];
    let mut bytes = b"k!^E6Ks1".to_vec();
    bytes.extend_from_slice(input);
    let bits = (bytes.len() as u64) * 8;
    bytes.push(0x80);
    while bytes.len() % 64 != 56 {
        bytes.push(0);
    }
    bytes.extend_from_slice(&bits.to_le_bytes());
    let mut state = [0x6745_2301_u32, 0xefcd_ab89, 0x98ba_dcfe, 0x1032_5476];
    for block in bytes.chunks_exact(64) {
        let mut words = [0_u32; 16];
        for (word, chunk) in words.iter_mut().zip(block.chunks_exact(4)) {
            *word = u32::from_le_bytes(chunk.try_into().expect("four bytes"));
        }
        let [mut a, mut b, mut c, mut d] = state;
        for i in 0..64 {
            let (f, g) = match i {
                0..16 => ((b & c) | (!b & d), i),
                16..32 => ((d & b) | (!d & c), (5 * i + 1) % 16),
                32..48 => (b ^ c ^ d, (3 * i + 5) % 16),
                _ => (c ^ (b | !d), (7 * i) % 16),
            };
            let next = b.wrapping_add(
                a.wrapping_add(f)
                    .wrapping_add(0xd76a_a478)
                    .wrapping_add(words[g])
                    .rotate_left(SHIFTS[(i / 16) * 4 + i % 4]),
            );
            a = d;
            d = c;
            c = b;
            b = next;
        }
        for (old, value) in state.iter_mut().zip([a, b, c, d]) {
            *old = old.wrapping_add(value);
        }
    }
    state
        .iter()
        .flat_map(|n| n.to_le_bytes())
        .map(|n| format!("{n:02x}"))
        .collect()
}

#[cfg(test)]
mod tests;
