use super::{invalid, tables::*};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use tuneweave_core::Result;

const QUERY_KEY: [u8; 8] = *b"kwks&@69";
const MAX_PLAIN: usize = 64 * 1024;
pub(super) const MAX_RESPONSE: usize = 256 * 1024;

fn permute(value: u64, positions: &[u8]) -> u64 {
    positions
        .iter()
        .enumerate()
        .fold(0, |out, (bit, position)| {
            out | (((value >> position) & 1) << bit)
        })
}

fn expand(value: u64, positions: &[i8]) -> u64 {
    positions
        .iter()
        .enumerate()
        .fold(0, |out, (bit, position)| {
            if *position < 0 {
                out
            } else {
                out | (((value >> *position as u8) & 1) << bit)
            }
        })
}

fn schedule(key: &[u8; 8]) -> [u64; 16] {
    let mut current = permute(u64::from_le_bytes(*key), &KEY_FIRST);
    let mut result = [0; 16];
    for (round, shift) in ROTATIONS.into_iter().enumerate() {
        // These are the platform's wire values, not standard DES rotation masks.
        let mask = if shift == 1 { 1_048_577 } else { 3_145_731 };
        current = ((current & !mask) >> shift) | ((current & mask) << (28 - shift));
        result[round] = expand(current, &KEY_SECOND);
    }
    result
}

fn block(bytes: [u8; 8], schedule: &[u64; 16]) -> [u8; 8] {
    let state = permute(u64::from_le_bytes(bytes), &INITIAL);
    let mut left = state & 0xffff_ffff;
    let mut right = state >> 32;
    for key in schedule {
        let expanded = expand(right, &EXPANSION) ^ key;
        let substituted = BOXES.iter().enumerate().fold(0, |value, (i, table)| {
            value | (u64::from(table[((expanded >> (i * 8)) & 63) as usize]) << (i * 4))
        });
        (left, right) = (right, left ^ permute(substituted, &PERMUTATION));
    }
    permute(right | (left << 32), &FINAL).to_le_bytes()
}

pub(super) fn seal_query(plain: &[u8]) -> Result<String> {
    if plain.len() > MAX_PLAIN {
        return Err(invalid());
    }
    Ok(STANDARD.encode(encrypt(plain, &QUERY_KEY)))
}

pub(super) fn seal_catalog_query(plain: &[u8]) -> Result<String> {
    if plain.len() > MAX_PLAIN {
        return Err(invalid());
    }
    Ok(STANDARD.encode(encrypt(plain, b"ylzsxkwm")))
}

pub(super) fn seal_image_query(plain: &[u8]) -> Result<String> {
    if plain.len() > MAX_PLAIN {
        return Err(invalid());
    }
    Ok(STANDARD.encode(encrypt(plain, b"rbi3azxp")))
}

fn encrypt(plain: &[u8], key: &[u8; 8]) -> Vec<u8> {
    let mut padded = plain.to_vec();
    padded.resize(plain.len() + 8 - plain.len() % 8, 0);
    let schedule = schedule(key);
    for bytes in padded.chunks_exact_mut(8) {
        let mut input = [0; 8];
        input.copy_from_slice(bytes);
        bytes.copy_from_slice(&block(input, &schedule));
    }
    padded
}

pub(super) fn open_response(encoded: &[u8], key: &[u8; 8]) -> Result<Vec<u8>> {
    if encoded.len() > MAX_RESPONSE || !key.iter().all(u8::is_ascii_digit) {
        return Err(invalid());
    }
    let mut cipher = decrypt(encoded, key)?;
    let padding = cipher.iter().rev().take_while(|byte| **byte == 0).count();
    if !(1..=8).contains(&padding) {
        return Err(invalid());
    }
    cipher.truncate(cipher.len() - padding);
    Ok(cipher)
}

/// The media envelope uses a separate public wire key and Java String.trim.
/// Keep the numeric response-key contract of the login protocol unchanged.
pub(super) fn open_media_key(encoded: &[u8]) -> Result<Vec<u8>> {
    if encoded.len() > 16 * 1024 {
        return Err(invalid());
    }
    let plain = decrypt(encoded, b"ylzsxkwm")?;
    let start = plain.iter().position(|b| *b > 32).ok_or_else(invalid)?;
    let end = plain.iter().rposition(|b| *b > 32).ok_or_else(invalid)? + 1;
    Ok(plain[start..end].to_vec())
}

fn decrypt(encoded: &[u8], key: &[u8; 8]) -> Result<Vec<u8>> {
    let mut cipher = STANDARD
        .decode(encoded.trim_ascii())
        .map_err(|_| invalid())?;
    if cipher.is_empty() || cipher.len() % 8 != 0 {
        return Err(invalid());
    }
    let mut schedule = schedule(key);
    schedule.reverse();
    for bytes in cipher.chunks_exact_mut(8) {
        let mut input = [0; 8];
        input.copy_from_slice(bytes);
        bytes.copy_from_slice(&block(input, &schedule));
    }
    Ok(cipher)
}

#[cfg(test)]
pub(super) fn fixture_response(plain: &[u8], key: &[u8; 8]) -> Vec<u8> {
    STANDARD.encode(encrypt(plain, key)).into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn independent_byte_and_boundary_vectors_match_the_wire_cipher() {
        let vectors: serde_json::Value =
            serde_json::from_str(include_str!("test_vectors.json")).unwrap();
        for vector in vectors["independent_reference"].as_array().unwrap() {
            let plain = STANDARD
                .decode(vector["plain_base64"].as_str().unwrap())
                .unwrap();
            let key: [u8; 8] = vector["key"]
                .as_str()
                .unwrap()
                .as_bytes()
                .try_into()
                .unwrap();
            let expected = vector["cipher_base64"].as_str().unwrap();
            assert_eq!(STANDARD.encode(encrypt(&plain, &key)), expected);
            assert_eq!(open_response(expected.as_bytes(), &key).unwrap(), plain);
        }
    }

    #[test]
    fn current_official_anonymous_response_decrypts_to_an_explicit_failure() {
        let vectors: serde_json::Value =
            serde_json::from_str(include_str!("test_vectors.json")).unwrap();
        let sample = &vectors["official_anonymous_error"];
        let key: [u8; 8] = sample["sx"]
            .as_str()
            .unwrap()
            .as_bytes()
            .try_into()
            .unwrap();
        let bytes =
            open_response(sample["cipher_base64"].as_str().unwrap().as_bytes(), &key).unwrap();
        let decoded: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(decoded, sample["expected"]);
        assert_eq!(decoded["result"], "fail");
    }

    #[test]
    fn malformed_encoding_wrong_keys_and_budget_overflow_are_rejected() {
        let key = *b"17894932";
        for body in [
            b"".as_slice(),
            b"eA==",
            b"not base64",
            b"{\"result\":\"succ\"}",
        ] {
            assert!(open_response(body, &key).is_err());
        }
        let cipher = fixture_response(b"{\"result\":\"succ\"}", &key);
        assert!(open_response(&cipher, b"00000000").is_err());
        assert!(open_response(&cipher, b"notakey!").is_err());
        assert!(open_response(&vec![b'A'; MAX_RESPONSE + 1], &key).is_err());
        assert!(seal_query(&vec![0; MAX_PLAIN + 1]).is_err());
        let no_padding = STANDARD.encode(block(*b"12345678", &schedule(&key)));
        assert!(open_response(no_padding.as_bytes(), &key).is_err());
    }
}
