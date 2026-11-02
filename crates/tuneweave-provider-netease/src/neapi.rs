//! NEAPI wire containers and bounded, non-executing configuration decoding.
use aws_lc_rs::aead::{Aad, CHACHA20_POLY1305, LessSafeKey, Nonce, UnboundKey};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use num_bigint::BigUint;
use serde_json::Value;
use std::{collections::BTreeMap, io::Read};
use tuneweave_core::{ErrorCode, Platform, Result, TuneWeaveError};

const MAX_BYTES: usize = 8 * 1024 * 1024;
const CODE_MASK: [u32; 8] = [
    0x94b4f825, 0x977c6795, 0x327de32b, 0x16da5ba1, 0xcf55a2eb, 0x080bb445, 0x021af055, 0xfc56b8bd,
];
const TEXT_MASK: [u8; 16] = [
    0xae, 0xfd, 0xa2, 0xd1, 0x83, 0x2b, 0x51, 0x8a, 0x77, 0x41, 0xc3, 0x25, 0xfb, 0x26, 0x87, 0xf0,
];

#[derive(Clone)]
pub(crate) struct Configuration {
    pub version: u16,
    pub encrypt_key: [u8; 32],
    pub decrypt_key: [u8; 32],
}
fn invalid() -> TuneWeaveError {
    TuneWeaveError::new(
        ErrorCode::UpstreamError,
        "NetEase NEAPI returned invalid configuration or encrypted data",
    )
    .with_platform(Platform::Netease)
}
fn inflate(data: &[u8]) -> Result<Vec<u8>> {
    let mut reader = zstd::stream::read::Decoder::new(data).map_err(|_| invalid())?;
    reader.window_log_max(23).map_err(|_| invalid())?;
    let mut out = Vec::new();
    reader
        .take((MAX_BYTES + 1) as u64)
        .read_to_end(&mut out)
        .map_err(|_| invalid())?;
    if out.len() > MAX_BYTES {
        return Err(invalid());
    }
    Ok(out)
}
pub(crate) fn encrypt(data: &[u8], config: &Configuration) -> Result<String> {
    if data.len() > MAX_BYTES {
        return Err(invalid());
    }
    let nonce = rand::random::<[u8; 12]>();
    let associated = rand::random::<[u8; 16]>();
    let key = LessSafeKey::new(
        UnboundKey::new(&CHACHA20_POLY1305, &config.encrypt_key).map_err(|_| invalid())?,
    );
    let mut sealed = zstd::stream::encode_all(data, 3).map_err(|_| invalid())?;
    key.seal_in_place_append_tag(
        Nonce::assume_unique_for_key(nonce),
        Aad::from(associated),
        &mut sealed,
    )
    .map_err(|_| invalid())?;
    let mut output = Vec::with_capacity(41 + sealed.len());
    output.extend_from_slice(b"CSJM");
    output.extend_from_slice(&config.version.to_be_bytes());
    output.extend_from_slice(&12_u16.to_le_bytes());
    output.extend_from_slice(&nonce);
    output.push(16);
    output.extend_from_slice(&associated);
    output.extend_from_slice(&(sealed.len() as u32).to_le_bytes());
    output.extend_from_slice(&sealed);
    Ok(BASE64.encode(output))
}
pub(crate) fn decrypt(data: &[u8], config: &Configuration) -> Result<Value> {
    if data.len() > MAX_BYTES {
        return Err(invalid());
    }
    let text = std::str::from_utf8(data).map_err(|_| invalid())?.trim();
    let raw = BASE64.decode(text).map_err(|_| invalid())?;
    let mut r = Reader::new(&raw);
    if r.take(4)? != b"CSJM"
        || u16::from_be_bytes(r.take(2)?.try_into().map_err(|_| invalid())?) != config.version
        || r.u16()? != 12
    {
        return Err(invalid());
    }
    let nonce: [u8; 12] = r.take(12)?.try_into().map_err(|_| invalid())?;
    if r.byte()? != 16 {
        return Err(invalid());
    }
    let associated = r.take(16)?;
    let length = r.u32()? as usize;
    if length < 16 || length != raw.len().saturating_sub(r.pos) {
        return Err(invalid());
    }
    let mut sealed = r.take(length)?.to_vec();
    let key = LessSafeKey::new(
        UnboundKey::new(&CHACHA20_POLY1305, &config.decrypt_key).map_err(|_| invalid())?,
    );
    let compressed = key
        .open_in_place(
            Nonce::assume_unique_for_key(nonce),
            Aad::from(associated),
            &mut sealed,
        )
        .map_err(|_| invalid())?;
    let decoded = inflate(&complete_streaming_frame(compressed.to_vec())?)?;
    // Require exactly one JSON document; never silently discard trailing data.
    serde_json::from_slice(&decoded).map_err(|_| invalid())
}

pub(crate) fn decode_configuration(raw: &[u8]) -> Result<Configuration> {
    if raw.len() < 68 || raw.len() > MAX_BYTES {
        return Err(invalid());
    }
    let modulus = BigUint::parse_bytes(b"e4e79c1cb27a9fab7d2c740e99e923ec51c009169ec3a380c60404e01b060f0160cdc631d9fb7fe53b228a5ff11bee42b965d28c6c2036193f4ccdc7ed853d11",16).ok_or_else(invalid)?;
    let cipher = BigUint::from_bytes_be(&raw[..64]);
    if cipher >= modulus {
        return Err(invalid());
    }
    let decoded = cipher
        .modpow(&BigUint::from(65_537_u32), &modulus)
        .to_bytes_be();
    let mut padded = vec![0; 64_usize.checked_sub(decoded.len()).ok_or_else(invalid)?];
    padded.extend_from_slice(&decoded);
    if padded[..2] != [0, 1] {
        return Err(invalid());
    }
    let end = padded[2..]
        .iter()
        .position(|b| *b == 0)
        .map(|i| i + 2)
        .ok_or_else(invalid)?;
    if end < 10 || padded[2..end].iter().any(|b| *b != 255) || end == 63 {
        return Err(invalid());
    }
    let mut r = Reader::new(&raw[64..]);
    let size = r.u32()? as usize;
    if size != raw.len() - 68 {
        return Err(invalid());
    }
    let compressed = rc4(&padded[end + 1..], r.take(size)?);
    let decoded = inflate(&complete_streaming_frame(compressed)?)?;
    configuration_from_container(&decoded)
}
// The live NEAPI service emits complete Zstd blocks without an end
// marker. Only the exact streaming, no-dictionary/no-checksum frame shape may
// gain an empty final block. A partial block is always an error.
fn complete_streaming_frame(mut frame: Vec<u8>) -> Result<Vec<u8>> {
    if frame.len() < 6 || frame[..4] != [0x28, 0xb5, 0x2f, 0xfd] || frame[4] != 0 {
        return Ok(frame);
    }
    let mut pos = 6_usize;
    while pos < frame.len() {
        let header = frame.get(pos..pos + 3).ok_or_else(invalid)?;
        let header = u32::from_le_bytes([header[0], header[1], header[2], 0]);
        let kind = (header >> 1) & 3;
        if kind == 3 {
            return Err(invalid());
        }
        let len = if kind == 1 { 1 } else { (header >> 3) as usize };
        pos = pos.checked_add(3 + len).ok_or_else(invalid)?;
        if pos > frame.len() {
            return Err(invalid());
        }
        if header & 1 != 0 {
            return Ok(frame);
        }
    }
    if pos != frame.len() || pos == 6 {
        return Err(invalid());
    }
    frame.extend_from_slice(&[1, 0, 0]);
    Ok(frame)
}

fn rc4(key: &[u8], bytes: &[u8]) -> Vec<u8> {
    let mut state = std::array::from_fn::<_, 256, _>(|i| i as u8);
    let mut j = 0_usize;
    for i in 0..256 {
        j = (j + usize::from(state[i]) + usize::from(key[i % key.len()])) & 255;
        state.swap(i, j);
    }
    let (mut i, mut j) = (0_usize, 0_usize);
    bytes
        .iter()
        .map(|b| {
            i = (i + 1) & 255;
            j = (j + usize::from(state[i])) & 255;
            state.swap(i, j);
            b ^ state[(usize::from(state[i]) + usize::from(state[j])) & 255]
        })
        .collect()
}
struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}
impl<'a> Reader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self.pos.checked_add(n).ok_or_else(invalid)?;
        let result = self.data.get(self.pos..end).ok_or_else(invalid)?;
        self.pos = end;
        Ok(result)
    }
    fn byte(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_le_bytes(
            self.take(2)?.try_into().map_err(|_| invalid())?,
        ))
    }
    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(
            self.take(4)?.try_into().map_err(|_| invalid())?,
        ))
    }
    fn count(&mut self) -> Result<usize> {
        let n = self.u32()? as usize;
        if n > 65_536 {
            return Err(invalid());
        }
        Ok(n)
    }
    fn string(&mut self) -> Result<Option<String>> {
        let size = match self.byte()? {
            0 => return Ok(None),
            255 => self.u32()? as usize,
            n => usize::from(n),
        };
        let raw = self.take(size.checked_sub(1).ok_or_else(invalid)?)?;
        let text = raw
            .iter()
            .enumerate()
            .map(|(i, b)| b ^ TEXT_MASK[i % 16])
            .collect();
        Ok(Some(String::from_utf8(text).map_err(|_| invalid())?))
    }
}
#[derive(Clone)]
enum Constant {
    Integer(i64),
    Text(String),
    Other,
}
struct Proto {
    code: Vec<u32>,
    constants: Vec<Constant>,
    upvalues: Vec<(u8, u8)>,
    children: Vec<Proto>,
}
fn proto(r: &mut Reader<'_>, depth: usize) -> Result<Proto> {
    if depth > 16 {
        return Err(invalid());
    }
    r.string()?;
    r.take(11)?;
    let n = r.count()?;
    let mut code = Vec::with_capacity(n);
    for i in 0..n {
        code.push(r.u32()? ^ CODE_MASK[i % 8]);
    }
    let n = r.count()?;
    let mut constants = Vec::with_capacity(n);
    for _ in 0..n {
        constants.push(match r.byte()? {
            0 => Constant::Other,
            1 => {
                r.byte()?;
                Constant::Other
            }
            3 => {
                r.take(8)?;
                Constant::Other
            }
            19 => Constant::Integer(i64::from_le_bytes(
                r.take(8)?.try_into().map_err(|_| invalid())?,
            )),
            4 | 20 => r.string()?.map(Constant::Text).unwrap_or(Constant::Other),
            _ => return Err(invalid()),
        });
    }
    let n = r.count()?;
    let mut upvalues = Vec::with_capacity(n);
    for _ in 0..n {
        upvalues.push((r.byte()?, r.byte()?));
    }
    let n = r.count()?;
    let mut children = Vec::with_capacity(n);
    for _ in 0..n {
        children.push(proto(r, depth + 1)?);
    }
    let n = r.count()?;
    r.take(n * 4)?;
    let n = r.count()?;
    for _ in 0..n {
        r.string()?;
        r.take(8)?;
    }
    let n = r.count()?;
    for _ in 0..n {
        r.string()?;
    }
    Ok(Proto {
        code,
        constants,
        upvalues,
        children,
    })
}
fn configuration_from_container(data: &[u8]) -> Result<Configuration> {
    let mut r = Reader::new(data);
    if r.take(4)? != b"ECFG" {
        return Err(invalid());
    }
    let version = u16::from_be_bytes(r.take(2)?.try_into().map_err(|_| invalid())?);
    if version == 0 || r.take(4)? != b"\x7fELF" {
        return Err(invalid());
    }
    r.take(29)?;
    r.byte()?;
    let main = proto(&mut r, 0)?;
    if r.pos != data.len() {
        return Err(invalid());
    }
    let mut registers = BTreeMap::new();
    for ins in &main.code {
        if ins & 63 == 1 {
            registers.insert(
                (ins >> 6) & 255,
                main.constants
                    .get((ins >> 14) as usize)
                    .ok_or_else(invalid)?,
            );
        }
    }
    let child = main.children.first().ok_or_else(invalid)?;
    let mut direction = None;
    let mut offsets = BTreeMap::new();
    for ins in &child.code {
        match ins & 63 {
            31 => {
                let c = (ins >> 14) & 511;
                direction = if c & 256 != 0 {
                    match child.constants.get((c & 255) as usize) {
                        Some(Constant::Integer(n)) => Some(*n),
                        _ => None,
                    }
                } else {
                    None
                };
            }
            5 => {
                if let Some(d) = direction.take() {
                    let &(stack, index) = child
                        .upvalues
                        .get(((ins >> 23) & 511) as usize)
                        .ok_or_else(invalid)?;
                    if stack != 1 {
                        return Err(invalid());
                    }
                    if let Some(Constant::Integer(offset)) = registers.get(&u32::from(index)) {
                        offsets.insert(d, usize::try_from(*offset).map_err(|_| invalid())?);
                    }
                }
            }
            _ => {}
        }
    }
    let blob = main
        .constants
        .iter()
        .filter_map(|c| match c {
            Constant::Text(s) if s.len() >= 64 => BASE64.decode(s).ok(),
            _ => None,
        })
        .max_by_key(Vec::len)
        .ok_or_else(invalid)?;
    let key = |direction| -> Result<[u8; 32]> {
        let start = *offsets.get(&direction).ok_or_else(invalid)?;
        blob.get(start..start.checked_add(32).ok_or_else(invalid)?)
            .ok_or_else(invalid)?
            .try_into()
            .map_err(|_| invalid())
    };
    Ok(Configuration {
        version,
        encrypt_key: key(251)?,
        decrypt_key: key(503)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn configuration_frame_accepts_missing_end_marker_only_after_complete_blocks() {
        let frame = vec![0x28, 0xb5, 0x2f, 0xfd, 0, 0x50, 8, 0, 0, b'x'];
        let completed = complete_streaming_frame(frame.clone()).unwrap();
        assert_eq!(inflate(&completed).unwrap(), b"x");
        assert!(complete_streaming_frame(frame[..frame.len() - 1].to_vec()).is_err());
    }

    #[test]
    fn authenticated_container_round_trip_and_tamper_rejection() {
        let config = Configuration {
            version: 7,
            encrypt_key: [19; 32],
            decrypt_key: [19; 32],
        };
        let wire = encrypt(br#"{"code":200,"data":"test"}"#, &config).unwrap();
        assert_eq!(decrypt(wire.as_bytes(), &config).unwrap()["code"], 200);
        let mut raw = BASE64.decode(wire).unwrap();
        let last = raw.len() - 1;
        raw[last] ^= 1;
        assert!(decrypt(BASE64.encode(raw).as_bytes(), &config).is_err());
    }
    #[test]
    fn malformed_configuration_and_container_are_rejected_without_panics() {
        let config = Configuration {
            version: 7,
            encrypt_key: [19; 32],
            decrypt_key: [19; 32],
        };
        for n in 0..512 {
            let bytes = vec![0; n];
            assert!(decode_configuration(&bytes).is_err());
            assert!(decrypt(BASE64.encode(&bytes).as_bytes(), &config).is_err());
        }
        assert_eq!(hex::encode(rc4(b"Key", b"Plaintext")), "bbf316e8d940af0ad3");
    }
}
