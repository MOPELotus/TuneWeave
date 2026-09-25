//! NetEase desktop listening-log wire format. Only protocol facts are retained;
//! no simulated UI navigation, membership, effects, or listening time is generated.
use std::io::Write;

use chacha20::{
    ChaCha20,
    cipher::{KeyIvInit, StreamCipher, StreamCipherSeek},
};
use flate2::{Compression, write::GzEncoder};
use num_bigint::BigUint;
use serde::{Deserialize, Serialize};
use tuneweave_core::{ErrorCode, Platform, Quality, Result, ScrobbleRequest, TuneWeaveError};

const MODULUS: [u8; 32] = [
    0xfd, 0x90, 0xbd, 0x46, 0x6f, 0xf9, 0xbc, 0x8a, 0x3f, 0xec, 0x2f, 0xbc, 0xf2, 0x63, 0xb9, 0x0d,
    0x5c, 0x56, 0x48, 0x79, 0xfa, 0x5d, 0x7a, 0xab, 0x89, 0xb3, 0x1c, 0x1d, 0x5c, 0xb4, 0x13, 0x9d,
];
const FRAME_LIMIT: usize = 0x8000;

// Deliberately no Debug: metadata carries the selected account's credentials.
#[derive(Serialize)]
pub(crate) struct LogIdentity<'a> {
    #[serde(rename = "MUSIC_U")]
    pub session: &'a str,
    #[serde(rename = "JSESSIONID-WYYY")]
    pub session_id: &'a str,
    #[serde(rename = "NMTID")]
    pub nmtid: &'a str,
    #[serde(rename = "WEVNSM")]
    pub namespace: &'static str,
    #[serde(rename = "WNMCID")]
    pub client_id: &'a str,
    #[serde(rename = "__csrf")]
    pub csrf: &'a str,
    #[serde(rename = "deviceId")]
    pub device_id: &'a str,
    pub appver: &'static str,
    pub channel: &'static str,
    pub os: &'static str,
    pub osver: &'static str,
}

#[derive(Serialize)]
struct ListeningEvent<'a> {
    id: &'a str,
    #[serde(rename = "type")]
    kind: &'static str,
    resource_time: f64,
    bitrate: f64,
    bitrate_level: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    time: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    realtime: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    end: Option<&'static str>,
}

pub(crate) fn level(quality: Quality) -> Result<&'static str> {
    match quality {
        Quality::Standard => Ok("standard"),
        Quality::Higher => Ok("higher"),
        Quality::High => Ok("exhigh"),
        Quality::Lossless => Ok("lossless"),
        Quality::Hires => Ok("hires"),
        Quality::Surround => Ok("jyeffect"),
        Quality::Spatial => Ok("sky"),
        Quality::Dolby => Ok("dolby"),
        Quality::Master => Ok("jymaster"),
        Quality::Vivid => Ok("vivid"),
        Quality::Auto | Quality::Low | Quality::Dtsx | Quality::Vinyl => Err(TuneWeaveError::invalid_request(
            "NetEase scrobble requires an actual supported quality; auto, low, DTS:X and vinyl are not supported",
        )
        .with_platform(Platform::Netease)),
    }
}

pub(crate) fn records(
    id: &str,
    request: &ScrobbleRequest,
    timestamp: u64,
) -> Result<[(&'static str, Vec<u8>); 2]> {
    request.validate()?;
    let quality = level(request.quality)?;
    let seconds = request.played_ms as f64 / 1000.0;
    let encode = |action: &'static str, completed: bool| -> Result<(&'static str, Vec<u8>)> {
        let event = ListeningEvent {
            id,
            kind: "song",
            resource_time: request.duration_ms as f64 / 1000.0,
            bitrate: request.bitrate as f64 / 1000.0,
            bitrate_level: quality,
            time: completed.then_some(seconds),
            realtime: completed.then_some(seconds),
            end: completed.then_some(if request.played_ms == request.duration_ms {
                "playend"
            } else {
                "interrupt"
            }),
        };
        let json = serde_json::to_string(&event).map_err(|_| encoding_error())?;
        Ok((
            action,
            format!("{timestamp}\u{1}{action}\u{1}{json}").into_bytes(),
        ))
    };
    Ok([encode("_plv", false)?, encode("_pld", true)?])
}

pub(crate) fn encrypt(metadata: &[u8], record: &[u8]) -> Result<Vec<u8>> {
    let secret = loop {
        let candidate: [u8; 32] = rand::random();
        if candidate < MODULUS && candidate != [0; 32] {
            break candidate;
        }
    };
    let mut nonce_id: [u8; 16] = rand::random();
    nonce_id[6] = (nonce_id[6] & 0x0f) | 0x40;
    nonce_id[8] = (nonce_id[8] & 0x3f) | 0x80;
    encode_frame(
        metadata,
        record,
        &secret,
        &nonce_id,
        u32::from(rand::random::<u16>()),
    )
}

// Listening records fit one frame. Enforce that bound instead of reproducing
// the reference's multi-frame reuse of the same ChaCha key/nonce/counter.
fn encode_frame(
    metadata: &[u8],
    record: &[u8],
    secret: &[u8; 32],
    nonce_id: &[u8; 16],
    sequence: u32,
) -> Result<Vec<u8>> {
    let header_len = u16::try_from(
        74usize
            .checked_add(metadata.len())
            .ok_or_else(encoding_error)?,
    )
    .map_err(|_| encoding_error())?;
    if record.len() > FRAME_LIMIT {
        return Err(encoding_error());
    }
    let mut compressor = GzEncoder::new(Vec::new(), Compression::default());
    compressor.write_all(record).map_err(|_| encoding_error())?;
    let mut compressed = compressor.finish().map_err(|_| encoding_error())?;
    if compressed.len() > FRAME_LIMIT {
        return Err(encoding_error());
    }
    let wrapped = BigUint::from_bytes_be(secret)
        .modpow(&BigUint::from(65537u32), &BigUint::from_bytes_be(&MODULUS))
        .to_bytes_be();
    let mut public_key = [0u8; 32];
    public_key[32 - wrapped.len()..].copy_from_slice(&wrapped);
    let mut encrypted_meta = metadata.to_vec();
    crypt(&public_key, nonce_id, &mut encrypted_meta)?;
    crypt(secret, nonce_id, &mut compressed)?;
    let mut output = Vec::with_capacity(usize::from(header_len) + 6 + compressed.len());
    output.extend_from_slice(b"NCBL");
    output.extend_from_slice(&3u32.to_le_bytes());
    output.extend_from_slice(&header_len.to_le_bytes());
    output.extend_from_slice(nonce_id);
    output.extend_from_slice(&public_key);
    output.extend_from_slice(&sequence.to_le_bytes());
    output.extend_from_slice(&sequence.to_le_bytes());
    output.extend_from_slice(&((compressed.len() + 6) as u32).to_le_bytes());
    output.extend_from_slice(&0x4343u16.to_le_bytes());
    output.extend_from_slice(&(encrypted_meta.len() as u16).to_le_bytes());
    output.extend_from_slice(&encrypted_meta);
    output.extend_from_slice(&(compressed.len() as u16).to_le_bytes());
    output.extend_from_slice(&sequence.to_le_bytes());
    output.extend_from_slice(&compressed);
    Ok(output)
}

fn crypt(key: &[u8; 32], nonce_id: &[u8; 16], bytes: &mut [u8]) -> Result<()> {
    let nonce: [u8; 12] = nonce_id[..12].try_into().map_err(|_| encoding_error())?;
    let counter = u32::from_le_bytes(nonce_id[12..].try_into().map_err(|_| encoding_error())?) >> 2;
    let mut cipher = ChaCha20::new(key.into(), (&nonce).into());
    cipher
        .try_seek(u64::from(counter) * 64)
        .map_err(|_| encoding_error())?;
    cipher
        .try_apply_keystream(bytes)
        .map_err(|_| encoding_error())
}

fn encoding_error() -> TuneWeaveError {
    TuneWeaveError::new(
        ErrorCode::InternalError,
        "cannot encode NetEase listening log",
    )
    .with_platform(Platform::Netease)
}

#[derive(Deserialize)]
pub(crate) struct UploadResponse {
    pub code: i64,
    #[serde(default)]
    pub data: Option<UploadFiles>,
}

#[derive(Deserialize)]
pub(crate) struct UploadFiles {
    #[serde(default)]
    pub successfiles: Vec<String>,
}

/// Only a receipt for this exact upload counts as acceptance.
pub(crate) fn check_receipt(response: UploadResponse, filename: &str) -> Result<()> {
    if response.code == 200
        && response
            .data
            .is_some_and(|data| data.successfiles.iter().any(|name| name == filename))
    {
        return Ok(());
    }
    let code = match response.code {
        301 | 401 => ErrorCode::AuthenticationRequired,
        403 => ErrorCode::PermissionDenied,
        429 => ErrorCode::RateLimited,
        _ => ErrorCode::UpstreamError,
    };
    Err(
        TuneWeaveError::new(code, "NetEase did not acknowledge the listening log upload")
            .with_platform(Platform::Netease)
            .with_details(serde_json::json!({"upstream_code": response.code})),
    )
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use serde_json::{Value, json};
    use std::{
        io::Read,
        net::TcpListener,
        thread,
        time::{Duration, Instant},
    };

    pub(crate) fn request() -> ScrobbleRequest {
        ScrobbleRequest {
            played_ms: 90123,
            duration_ms: 210456,
            bitrate: 320000,
            quality: Quality::High,
            account: None,
        }
    }

    #[test]
    fn scrobble_wire_preserves_units_and_does_not_invent_playback_metadata() {
        let reports = records("123", &request(), 1700000000).unwrap();
        let start = String::from_utf8(reports[0].1.clone()).unwrap();
        let end = String::from_utf8(reports[1].1.clone()).unwrap();
        assert!(start.starts_with("1700000000\u{1}_plv\u{1}"));
        assert!(end.starts_with("1700000000\u{1}_pld\u{1}"));
        let data: Value = serde_json::from_str(end.split('\u{1}').nth(2).unwrap()).unwrap();
        assert_eq!(
            data,
            json!({"id":"123","type":"song","resource_time":210.456,
            "bitrate":320.0,"bitrate_level":"exhigh","time":90.123,"realtime":90.123,"end":"interrupt"})
        );
        for (quality, expected) in [
            (Quality::Standard, "standard"),
            (Quality::Higher, "higher"),
            (Quality::High, "exhigh"),
            (Quality::Lossless, "lossless"),
            (Quality::Hires, "hires"),
            (Quality::Surround, "jyeffect"),
            (Quality::Spatial, "sky"),
            (Quality::Dolby, "dolby"),
            (Quality::Master, "jymaster"),
            (Quality::Vivid, "vivid"),
        ] {
            assert_eq!(level(quality).unwrap(), expected);
        }
        assert!(level(Quality::Low).is_err());
        assert!(level(Quality::Auto).is_err());
        let mut full = request();
        full.played_ms = full.duration_ms;
        assert!(
            String::from_utf8(records("123", &full, 0).unwrap()[1].1.clone())
                .unwrap()
                .contains("playend")
        );
    }

    #[test]
    fn scrobble_ncbl_header_and_payload_round_trip_with_bounded_frame() {
        let secret = [1u8; 32];
        let nonce = [2u8; 16];
        let metadata = br#"{"MUSIC_U":"test-only"}"#;
        let data = b"1700000000\x01_pld\x01{\"id\":\"123\"}";
        let packet = encode_frame(metadata, data, &secret, &nonce, 37).unwrap();
        assert_eq!(&packet[..8], b"NCBL\x03\0\0\0");
        let offset = u16::from_le_bytes(packet[8..10].try_into().unwrap()) as usize;
        assert_eq!(offset, 74 + metadata.len());
        assert_eq!(&packet[58..66], &[37, 0, 0, 0, 37, 0, 0, 0]);
        assert_eq!(
            u32::from_le_bytes(packet[66..70].try_into().unwrap()) as usize,
            packet.len() - offset
        );
        assert_eq!(&packet[70..72], &[0x43, 0x43]);
        let mut meta = packet[74..offset].to_vec();
        crypt(packet[26..58].try_into().unwrap(), &nonce, &mut meta).unwrap();
        assert_eq!(meta, metadata);
        let mut compressed = packet[offset + 6..].to_vec();
        assert_eq!(
            u16::from_le_bytes(packet[offset..offset + 2].try_into().unwrap()) as usize,
            compressed.len()
        );
        crypt(&secret, &nonce, &mut compressed).unwrap();
        let mut plaintext = Vec::new();
        flate2::read::GzDecoder::new(compressed.as_slice())
            .read_to_end(&mut plaintext)
            .unwrap();
        assert_eq!(plaintext, data);
        assert!(encode_frame(&vec![0; 65536], data, &secret, &nonce, 0).is_err());
        assert!(encode_frame(metadata, &vec![0; FRAME_LIMIT + 1], &secret, &nonce, 0).is_err());
        assert_ne!(
            encrypt(metadata, data).unwrap(),
            encrypt(metadata, data).unwrap()
        );
    }

    #[test]
    fn scrobble_cipher_matches_rfc8439_block_vector() {
        let key: [u8; 32] = std::array::from_fn(|i| i as u8);
        let nonce = [0, 0, 0, 9, 0, 0, 0, 0x4a, 0, 0, 0, 0, 4, 0, 0, 0];
        let mut block = [0u8; 64];
        crypt(&key, &nonce, &mut block).unwrap();
        assert_eq!(
            hex::encode(block),
            concat!(
                "10f1e7e4d13b5915500fdd1fa32071c4c7d1f4c733c068030422aa9ac3d46c4e",
                "d2826446079faa0914c2d705d98b02a2b5129cd1de164eb9cbd083e8a2503c4e"
            )
        );
    }

    #[test]
    fn scrobble_receipt_requires_exact_file_and_filters_upstream_text() {
        for value in [
            json!({"code":200}),
            json!({"code":200,"data":{"successfiles":["other"]}}),
            json!({"code":401,"message":"MUSIC_U=secret"}),
            json!({"code":429}),
            json!({"code":500}),
        ] {
            let error =
                check_receipt(serde_json::from_value(value).unwrap(), "expected").unwrap_err();
            assert!(!error.retryable);
            assert!(!error.to_string().contains("secret"));
        }
        check_receipt(
            serde_json::from_value(json!({"code":200,"data":{"successfiles":["expected"]}}))
                .unwrap(),
            "expected",
        )
        .unwrap();
    }

    // Captures decrypted account metadata, never real user credentials.
    pub(crate) fn mock_uploads(codes: Vec<u16>) -> (String, thread::JoinHandle<Vec<Value>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let handle = thread::spawn(move || {
            let mut captures = Vec::new();
            for code in codes {
                let deadline = Instant::now() + Duration::from_secs(10);
                let mut socket = loop {
                    match listener.accept() {
                        Ok((socket, _)) => break socket,
                        Err(error)
                            if error.kind() == std::io::ErrorKind::WouldBlock
                                && Instant::now() < deadline =>
                        {
                            thread::sleep(Duration::from_millis(5))
                        }
                        Err(error) => panic!("mock upload timed out: {error}"),
                    }
                };
                socket
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut raw = Vec::new();
                let (header_end, length) =
                    loop {
                        let mut block = [0u8; 4096];
                        let count = socket.read(&mut block).unwrap();
                        assert!(count > 0);
                        raw.extend_from_slice(&block[..count]);
                        assert!(raw.len() < 100000);
                        if let Some(end) = raw.windows(4).position(|v| v == b"\r\n\r\n") {
                            let head = String::from_utf8_lossy(&raw[..end]).to_ascii_lowercase();
                            assert!(head.starts_with(
                                "post /api/clientlog/encrypt/upload?multiupload=true "
                            ));
                            let length: usize = head
                                .lines()
                                .find_map(|line| line.strip_prefix("content-length:"))
                                .unwrap()
                                .trim()
                                .parse()
                                .unwrap();
                            break (end + 4, length);
                        }
                    };
                while raw.len() < header_end + length {
                    let mut block = [0u8; 4096];
                    let count = socket.read(&mut block).unwrap();
                    assert!(count > 0);
                    raw.extend_from_slice(&block[..count]);
                }
                let body = &raw[header_end..header_end + length];
                let packet_start = body.windows(4).position(|part| part == b"NCBL").unwrap();
                let disposition = String::from_utf8_lossy(&body[..packet_start]);
                let filename = disposition
                    .split("filename=\"")
                    .nth(1)
                    .unwrap()
                    .split('"')
                    .next()
                    .unwrap();
                let packet = &body[packet_start..];
                let meta_end = u16::from_le_bytes(packet[8..10].try_into().unwrap()) as usize;
                let mut meta = packet[74..meta_end].to_vec();
                crypt(
                    packet[26..58].try_into().unwrap(),
                    packet[10..26].try_into().unwrap(),
                    &mut meta,
                )
                .unwrap();
                captures.push(serde_json::from_slice(&meta).unwrap());
                let response = if code == 200 {
                    json!({"code":200,"data":{"successfiles":[filename]}})
                } else if code == 201 {
                    json!({"code":200,"data":{"successfiles":["wrong-file"]}})
                } else {
                    json!({"code":code,"message":"secret-echo-must-not-leak"})
                };
                let body = response.to_string();
                write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            }
            captures
        });
        (format!("http://{address}"), handle)
    }
}
