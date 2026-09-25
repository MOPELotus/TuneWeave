//! Complete SQ and ZQ24 FLAC validation for authorized ordinary/MG3D content.
//! SQ accepts 16-bit mono/stereo at 44.1/48 kHz; ZQ24 accepts 24-bit
//! mono/stereo at 44.1/48/88.2/96/176.4/192 kHz. No media is transcoded.
use super::*;
use claxon::frame::FrameReader;
use std::io::Cursor;

struct Info {
    rate: u64,
    channels: u32,
    bits: u32,
    samples: u64,
    max_block: u32,
    md5: [u8; 16],
}

fn invalid() -> tuneweave_core::TuneWeaveError {
    migu_upstream_error("Migu content is not the authorized complete FLAC rendition")
}

fn metadata(bytes: &[u8], duration_ms: u64) -> Result<(Info, usize)> {
    // Parse only bounded native FLAC metadata; do not let arbitrary tags cause
    // decoder allocations. STREAMINFO must be first and appear exactly once.
    if bytes.len() as u64 > MAX_CONTENT
        || bytes.get(..8) != Some(b"fLaC\0\0\0\x22") && bytes.get(..8) != Some(b"fLaC\x80\0\0\x22")
    {
        return Err(invalid());
    }
    let data = bytes.get(8..42).ok_or_else(invalid)?;
    let packed = u64::from_be_bytes(data[10..18].try_into().map_err(|_| invalid())?);
    let rate = packed >> 44;
    let channels = ((packed >> 41) & 7) as u32 + 1;
    let bits = ((packed >> 36) & 31) as u32 + 1;
    let samples = packed & 0xf_ffff_ffff;
    let min_block = u16::from_be_bytes([data[0], data[1]]);
    let max_block = u32::from(u16::from_be_bytes([data[2], data[3]]));
    let md5 = data[18..34].try_into().map_err(|_| invalid())?;
    if !(matches!(rate, 44_100 | 48_000)
        || bits == 24 && matches!(rate, 88_200 | 96_000 | 176_400 | 192_000))
        || !(1..=2).contains(&channels)
        || !matches!(bits, 16 | 24)
        || samples == 0
        || samples > rate * 3600
        || duration_ms == 0
        || (samples * 1000 / rate).abs_diff(duration_ms) > 1500
        || min_block < 16
        || u32::from(min_block) > max_block
        || max_block > 8192
        || md5 == [0; 16]
    {
        return Err(invalid());
    }
    let mut at = 4;
    for index in 0..128 {
        let header = bytes.get(at..at + 4).ok_or_else(invalid)?;
        let kind = header[0] & 127;
        if kind == 127 || index > 0 && kind == 0 {
            return Err(invalid());
        }
        let length =
            (usize::from(header[1]) << 16) | (usize::from(header[2]) << 8) | usize::from(header[3]);
        at = at.checked_add(4 + length).ok_or_else(invalid)?;
        if at > 1024 * 1024 || at > bytes.len() {
            return Err(invalid());
        }
        if header[0] & 128 != 0 {
            return Ok((
                Info {
                    rate,
                    channels,
                    bits,
                    samples,
                    max_block,
                    md5,
                },
                at,
            ));
        }
    }
    Err(invalid())
}

/// Decode every frame and verify the canonical interleaved PCM MD5. Session
/// checks/yields occur between bounded frames, so cancellation or replacement
/// cannot publish bytes after the original account generation was abandoned.
pub(crate) async fn inspect(
    bytes: &[u8],
    duration_ms: u64,
    expected_bits: u32,
    mut check: impl FnMut() -> Result<()>,
) -> Result<()> {
    check()?;
    let (info, offset) = metadata(bytes, duration_ms)?;
    if info.bits != expected_bits || !matches!(expected_bits, 16 | 24) {
        return Err(invalid());
    }
    let mut cursor = Cursor::new(&bytes[offset..]);
    let mut buffer = Vec::new();
    let bytes_per_sample = (info.bits as usize).div_ceil(8);
    let mut pcm =
        Vec::with_capacity(info.max_block as usize * info.channels as usize * bytes_per_sample);
    let mut digest = Md5::new();
    let mut samples = 0_u64;
    let mut frames = 0_u64;
    let mut strategy = None;
    let mut short_block = false;
    while cursor.position() < cursor.get_ref().len() as u64 {
        tokio::task::yield_now().await;
        check()?;
        let at = offset + cursor.position() as usize;
        let header = bytes.get(at..at + 4).ok_or_else(invalid)?;
        let variable = header[1] & 1 != 0;
        let rate_code = header[2] & 15;
        let rate_matches = rate_code == 0
            || rate_code == 1 && info.rate == 88_200
            || rate_code == 2 && info.rate == 176_400
            || rate_code == 3 && info.rate == 192_000
            || rate_code == 9 && info.rate == 44_100
            || rate_code == 10 && info.rate == 48_000
            || rate_code == 11 && info.rate == 96_000;
        let channel_code = header[3] >> 4;
        // Claxon validates CRC8/CRC16, but does not compare header rate/bps
        // against STREAMINFO. Narrow these before decoding. Inherited bps and
        // extended-rate encodings are outside this supported subset.
        let expected_frame_bits = match info.bits {
            16 => 8,
            24 => 12,
            _ => return Err(invalid()),
        };
        if header[0] != 255
            || header[1] & 0xfe != 0xf8
            || header[3] & 15 != expected_frame_bits
            || !rate_matches
            || !(channel_code == info.channels as u8 - 1
                || info.channels == 2 && (8..=10).contains(&channel_code))
            || strategy.is_some_and(|prior| prior != variable)
            || short_block
        {
            return Err(invalid());
        }
        strategy = Some(variable);
        let block = FrameReader::new(&mut cursor)
            .read_next_or_eof(buffer)
            .map_err(|_| invalid())?
            .ok_or_else(invalid)?;
        let duration = block.duration();
        if block.channels() != info.channels
            || duration == 0
            || duration > info.max_block
            || block.time()
                != if variable {
                    samples
                } else {
                    u64::from(duration) * frames
                }
            || samples + u64::from(duration) > info.samples
        {
            return Err(invalid());
        }
        short_block = !variable && duration < info.max_block;
        pcm.clear();
        for index in 0..duration {
            for channel in 0..info.channels {
                let sample = block.sample(channel, index);
                let limit = 1_i32 << (info.bits - 1);
                if !(-limit..limit).contains(&sample) {
                    return Err(invalid());
                }
                pcm.extend_from_slice(&sample.to_le_bytes()[..bytes_per_sample]);
            }
        }
        digest.update(&pcm);
        samples += u64::from(duration);
        frames += 1;
        buffer = block.into_buffer();
    }
    let actual_md5: [u8; 16] = digest.finalize().into();
    if samples != info.samples || actual_md5 != info.md5 {
        return Err(invalid());
    }
    check()
}

#[cfg(test)]
mod tests;
