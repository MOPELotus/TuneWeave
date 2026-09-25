use super::*;
use base64::engine::general_purpose::STANDARD;

const SEED: [u8; 8] = [0x69, 0x56, 0x46, 0x38, 0x2b, 0x20, 0x15, 0x0b];
pub(crate) struct Key(Vec<u8>);
impl std::fmt::Debug for Key {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("KuwoMediaKey { .. }")
    }
}
impl Key {
    pub(crate) fn parse(encoded: &str, input: &KuwoNativeSessionInput) -> Result<Self> {
        let outer = super::super::super::codec::open_media_key(encoded.as_bytes())
            .map_err(|_| invalid())?;
        let outer = std::str::from_utf8(&outer).map_err(|_| invalid())?;
        let inner = outer.replace(input.device_user(), "");
        Self::inner(&inner)
    }
    fn inner(encoded: &str) -> Result<Self> {
        if encoded.len() > 8192 {
            return Err(invalid());
        }
        let raw = STANDARD.decode(encoded).map_err(|_| invalid())?;
        if raw.len() < 24 {
            return Err(invalid());
        }
        let mut tea_key = [0; 16];
        for i in 0..8 {
            tea_key[i * 2] = SEED[i];
            tea_key[i * 2 + 1] = raw[i];
        }
        let mut result = raw[..8].to_vec();
        result.extend(tea2(&raw[8..], &tea_key)?);
        if result.len() > 4096 {
            return Err(invalid());
        }
        // The native branch silently skips later segments for these lengths.
        if (301..512).contains(&result.len()) {
            return Err(unsupported(
                "Kuwo media key length cannot cover complete audio",
            ));
        }
        Ok(Self(result))
    }
    pub(super) fn transform(&self, offset: u64, data: &mut [u8]) -> Result<()> {
        offset.checked_add(data.len() as u64).ok_or_else(invalid)?;
        let key = &self.0;
        let n = key.len();
        if n <= 300 {
            for (i, byte) in data.iter_mut().enumerate() {
                let mut position = offset + i as u64;
                if position > 32767 {
                    position %= 32767;
                }
                let index = ((position * position + 71214) % n as u64) as usize;
                let shift = (index & 7) ^ 4;
                *byte ^=
                    ((u16::from(key[index]) << shift) | (u16::from(key[index]) >> shift)) as u8;
            }
            return Ok(());
        }
        let mut initial: Vec<u8> = (0..n).map(|i| i as u8).collect();
        let mut j = 0;
        for i in 0..n {
            j = (j + usize::from(initial[i]) + usize::from(key[i])) % n;
            initial.swap(i, j);
        }
        let mut hash = 1_u32;
        for &byte in key {
            if byte == 0 {
                continue;
            }
            let next = hash.wrapping_mul(u32::from(byte));
            if next <= hash {
                break;
            }
            hash = next;
        }
        let mut done = 0;
        while done < data.len() {
            let position = offset + done as u64;
            if position < 128 {
                let denominator = (position + 1) as f64 * f64::from(key[position as usize % n]);
                let index = (f64::from(hash) / denominator * 100.0) as u64 % n as u64;
                data[done] ^= key[index as usize];
                done += 1;
                continue;
            }
            let segment = position / 5120;
            let within = (position % 5120) as usize;
            let take = (5120 - within).min(data.len() - done);
            let denominator = (segment + 1) as f64 * f64::from(key[(segment & 511) as usize]);
            let discard =
                (((f64::from(hash) / denominator * 100.0) as i64 & 511) as usize) + within;
            let mut state = initial.clone();
            let (mut i, mut j) = (0, 0);
            for step in 0..discard + take {
                i = (i + 1) % n;
                j = (j + usize::from(state[i])) % n;
                state.swap(i, j);
                if step >= discard {
                    data[done + step - discard] ^=
                        state[(usize::from(state[i]) + usize::from(state[j])) % n];
                }
            }
            done += take;
        }
        Ok(())
    }
}

fn tea2(cipher: &[u8], key: &[u8; 16]) -> Result<Vec<u8>> {
    if cipher.len() < 16 || cipher.len() % 8 != 0 {
        return Err(invalid());
    }
    let mut previous_cipher = [0; 8];
    let mut previous_inner = [0; 8];
    let mut plain = Vec::with_capacity(cipher.len());
    for block in cipher.chunks_exact(8) {
        let mut current = [0; 8];
        for i in 0..8 {
            current[i] = block[i] ^ previous_inner[i];
        }
        let inner = tea(current, key);
        for i in 0..8 {
            plain.push(inner[i] ^ previous_cipher[i]);
        }
        previous_cipher.copy_from_slice(block);
        previous_inner = inner;
    }
    let start = usize::from(plain[0] & 7) + 3;
    if plain.len() < start + 7 || plain[plain.len() - 7..].iter().any(|b| *b != 0) {
        return Err(invalid());
    }
    Ok(plain[start..plain.len() - 7].to_vec())
}
fn tea(block: [u8; 8], key: &[u8; 16]) -> [u8; 8] {
    let word = |b: &[u8]| u32::from_be_bytes(b.try_into().expect("fixed TEA word"));
    let (mut a, mut b) = (word(&block[..4]), word(&block[4..]));
    let k = [
        word(&key[..4]),
        word(&key[4..8]),
        word(&key[8..12]),
        word(&key[12..]),
    ];
    let delta = 0x9e3779b9_u32;
    let mut sum = delta.wrapping_mul(16);
    for _ in 0..16 {
        b = b.wrapping_sub(
            (a << 4).wrapping_add(k[2]) ^ a.wrapping_add(sum) ^ (a >> 5).wrapping_add(k[3]),
        );
        a = a.wrapping_sub(
            (b << 4).wrapping_add(k[0]) ^ b.wrapping_add(sum) ^ (b >> 5).wrapping_add(k[1]),
        );
        sum = sum.wrapping_sub(delta);
    }
    let mut out = [0; 8];
    out[..4].copy_from_slice(&a.to_be_bytes());
    out[4..].copy_from_slice(&b.to_be_bytes());
    out
}

#[cfg(test)]
mod tests;
