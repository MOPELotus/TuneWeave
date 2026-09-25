//! Bounded container/framing checks, not a PCM decoder or playback acceptance.
use super::*;

pub(super) fn inspect(
    bytes: &[u8],
    declared: &str,
    control: &Control,
) -> Result<(&'static str, &'static str)> {
    match declared {
        "mp3" if mp3(bytes, control).is_some() => Ok(("audio/mpeg", "mp3")),
        "aac" if adts(bytes, control).is_some() => Ok(("audio/aac", "aac")),
        "aac" if mp4(bytes, control).is_some() => Ok(("audio/mp4", "m4a")),
        "mmp4" if super::dtsx::inspect(bytes, control).is_some() => Ok(("audio/mp4", "mp4")),
        "flac" | "mflac" if flac(bytes, control).is_some() => Ok(("audio/flac", "flac")),
        _ => Err(invalid()),
    }
}
pub(super) fn validate_preview_duration(
    bytes: &[u8],
    window: &TrialWindow,
    control: &Control,
) -> Result<()> {
    let duration = mp3(bytes, control).ok_or_else(invalid)?;
    // Audition metadata is expressed in whole seconds. Account for its coarse
    // boundary and MP3 frame padding, while rejecting full or truncated media.
    if window.start_ms >= window.end_ms || duration.abs_diff(window.end_ms - window.start_ms) > 1500
    {
        return Err(invalid());
    }
    Ok(())
}
fn without_id3(bytes: &[u8]) -> Option<&[u8]> {
    if !bytes.starts_with(b"ID3") {
        return Some(bytes);
    }
    let header = bytes.get(..10)?;
    if !(2..=4).contains(&header[3]) || header[4] == 255 || header[6..].iter().any(|b| b & 128 != 0)
    {
        return None;
    }
    let size = header[6..]
        .iter()
        .fold(0_usize, |v, b| (v << 7) | usize::from(*b));
    if size > 16 * 1024 * 1024 {
        return None;
    }
    let footer = if header[3] == 4 && header[5] & 16 != 0 {
        10
    } else {
        0
    };
    bytes.get(10 + size + footer..)
}
fn mp3(bytes: &[u8], control: &Control) -> Option<u64> {
    let mut data = without_id3(bytes)?;
    let mut frames = 0;
    let mut signature = None;
    let mut samples = 0_u64;
    let mut sample_rate = 0_u64;
    while !data.is_empty() {
        control.check().ok()?;
        if data.len() == 128 && data.starts_with(b"TAG") {
            break;
        }
        let h = data.get(..4)?;
        let version = (h[1] >> 3) & 3;
        let rate_index = (h[2] >> 2) & 3;
        let bitrate_index = usize::from(h[2] >> 4);
        if h[0] != 255
            || h[1] & 0xe6 != 0xe2
            || version == 1
            || rate_index == 3
            || !(1..15).contains(&bitrate_index)
        {
            return None;
        }
        let current = (version, rate_index, h[3] >> 6 == 3);
        if signature.is_some_and(|s| s != current) {
            return None;
        }
        signature = Some(current);
        let table = if version == 3 {
            [
                0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320,
            ]
        } else {
            [0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160]
        };
        let rate = [44100, 48000, 32000][rate_index as usize]
            / match version {
                3 => 1,
                2 => 2,
                _ => 4,
            };
        let size = (if version == 3 { 144 } else { 72 }) * table[bitrate_index] * 1000 / rate
            + usize::from((h[2] >> 1) & 1);
        data = data.get(size..)?;
        frames += 1;
        samples += if version == 3 { 1152 } else { 576 };
        sample_rate = rate as u64;
    }
    (frames >= 2 && sample_rate > 0).then(|| samples * 1000 / sample_rate)
}
fn adts(bytes: &[u8], control: &Control) -> Option<()> {
    let mut data = without_id3(bytes)?;
    let mut frames = 0;
    let mut signature = None;
    while !data.is_empty() {
        control.check().ok()?;
        let h = data.get(..7)?;
        if h[0] != 255 || h[1] & 0xf6 != 0xf0 || (h[2] >> 2) & 15 > 12 {
            return None;
        }
        let channels = ((h[2] & 1) << 2) | (h[3] >> 6);
        if channels == 0 {
            return None;
        }
        let current = (h[2] & 0xfc, channels);
        if signature.is_some_and(|s| s != current) {
            return None;
        }
        signature = Some(current);
        let size =
            (usize::from(h[3] & 3) << 11) | (usize::from(h[4]) << 3) | usize::from(h[5] >> 5);
        let header = if h[1] & 1 == 0 { 9 } else { 7 };
        if size <= header {
            return None;
        }
        data = data.get(size..)?;
        frames += 1;
    }
    (frames >= 2).then_some(())
}
fn word(bytes: &[u8]) -> Option<usize> {
    Some(u32::from_be_bytes(bytes.get(..4)?.try_into().ok()?) as usize)
}
fn boxes(mut data: &[u8]) -> Option<Vec<([u8; 4], &[u8])>> {
    let mut out = Vec::new();
    while !data.is_empty() {
        if out.len() >= 16384 {
            return None;
        }
        let mut size = word(data)?;
        let kind = data.get(4..8)?.try_into().ok()?;
        let header = if size == 1 {
            size = usize::try_from(u64::from_be_bytes(data.get(8..16)?.try_into().ok()?)).ok()?;
            16
        } else {
            8
        };
        // Zero-size boxes cannot independently prove a complete transfer.
        if size < header {
            return None;
        }
        out.push((kind, data.get(header..size)?));
        data = data.get(size..)?;
    }
    Some(out)
}
fn child<'a>(data: &'a [u8], kind: &[u8; 4]) -> Option<&'a [u8]> {
    let mut found = None;
    for (k, b) in boxes(data)? {
        if &k == kind {
            if found.is_some() {
                return None;
            }
            found = Some(b);
        }
    }
    found
}
fn mp4(bytes: &[u8], control: &Control) -> Option<()> {
    let top = boxes(bytes)?;
    if top.first()?.0 != *b"ftyp" || top.first()?.1.len() < 8 {
        return None;
    }
    let moov = child(bytes, b"moov")?;
    let trak = child(moov, b"trak")?;
    let mdia = child(trak, b"mdia")?;
    if child(mdia, b"hdlr")?.get(8..12)? != b"soun" {
        return None;
    }
    let stbl = child(child(mdia, b"minf")?, b"stbl")?;
    let stsd = child(stbl, b"stsd")?;
    if stsd.get(..4)? != [0, 0, 0, 0] || word(stsd.get(4..)?)? != 1 {
        return None;
    }
    let entries = boxes(stsd.get(8..)?)?;
    if entries.len() != 1 || entries[0].0 != *b"mp4a" {
        return None;
    }
    let sample = entries[0].1;
    if sample.get(8..10)? != [0, 0] {
        return None;
    }
    let extra = sample.get(28..)?;
    if child(extra, b"esds")?.len() < 8 || boxes(extra)?.iter().any(|(k, _)| k == b"sinf") {
        return None;
    }
    let sizes = child(stbl, b"stsz")?;
    let fixed = word(sizes.get(4..)?)?;
    let count = word(sizes.get(8..)?)?;
    if count == 0 || count > 10_000_000 {
        return None;
    }
    let total = if fixed != 0 {
        if sizes.len() != 12 {
            return None;
        }
        fixed.checked_mul(count)?
    } else {
        if sizes.len() != 12 + count.checked_mul(4)? {
            return None;
        }
        let mut total = 0_usize;
        for size in sizes[12..].chunks_exact(4) {
            control.check().ok()?;
            let n = word(size)?;
            if n == 0 {
                return None;
            }
            total = total.checked_add(n)?;
        }
        total
    };
    let mut media_size = 0_usize;
    for (k, b) in top {
        if k == *b"mdat" {
            media_size = media_size.checked_add(b.len())?;
        }
    }
    (total > 0 && total == media_size).then_some(())
}

fn crc8(bytes: &[u8]) -> u8 {
    let mut crc = 0_u8;
    for byte in bytes {
        crc ^= byte;
        for _ in 0..8 {
            crc = (crc << 1) ^ if crc & 0x80 != 0 { 7 } else { 0 };
        }
    }
    crc
}
fn crc16(crc: u16, byte: u8) -> u16 {
    let mut crc = crc ^ (u16::from(byte) << 8);
    for _ in 0..8 {
        crc = (crc << 1) ^ if crc & 0x8000 != 0 { 0x8005 } else { 0 };
    }
    crc
}
fn flac_header(bytes: &[u8]) -> Option<(usize, u64)> {
    let h = bytes.get(..5)?;
    if h[0] != 255 || h[1] & 0xfe != 0xf8 || h[3] & 1 != 0 || h[3] >> 4 > 10 {
        return None;
    }
    let block = h[2] >> 4;
    let rate = h[2] & 15;
    if block == 0 || rate == 15 {
        return None;
    }
    let leading = h[4].leading_ones() as usize;
    let number_len = if leading == 0 {
        1
    } else if (2..=7).contains(&leading) {
        leading
    } else {
        return None;
    };
    if bytes
        .get(5..4 + number_len)?
        .iter()
        .any(|b| b & 0xc0 != 0x80)
    {
        return None;
    }
    let mut end = 4 + number_len;
    let samples = match block {
        1 => 192,
        2..=5 => 576_u64 << (block - 2),
        6 => {
            end += 1;
            u64::from(*bytes.get(end - 1)?) + 1
        }
        7 => {
            end += 2;
            u64::from(u16::from_be_bytes(
                bytes.get(end - 2..end)?.try_into().ok()?,
            )) + 1
        }
        _ => 256_u64 << (block - 8),
    };
    end += match rate {
        12 => 1,
        13 | 14 => 2,
        _ => 0,
    };
    end += 1;
    (crc8(bytes.get(..end)?) == 0).then_some((end, samples))
}
fn flac(bytes: &[u8], control: &Control) -> Option<()> {
    if bytes.get(..4)? != b"fLaC" {
        return None;
    }
    let mut pos = 4;
    let mut total = 0;
    for index in 0..128 {
        let kind = *bytes.get(pos)?;
        let size = word(&[&[0], bytes.get(pos + 1..pos + 4)?].concat())?;
        let metadata = bytes.get(pos + 4..pos + 4 + size)?;
        if index == 0 {
            if kind & 127 != 0 || size != 34 {
                return None;
            }
            let info = u64::from_be_bytes(metadata.get(10..18)?.try_into().ok()?);
            if info >> 44 == 0 {
                return None;
            }
            total = info & 0xf_ffff_ffff;
        } else if kind & 127 == 0 || kind & 127 == 127 {
            return None;
        }
        pos += 4 + size;
        if kind & 128 != 0 {
            break;
        }
        if index == 127 {
            return None;
        }
    }
    let frames = bytes.get(pos..)?;
    let (mut header_size, mut samples) = flac_header(frames)?;
    let mut start = 0;
    let mut crc = 0;
    for i in 0..frames.len() {
        if i % 65536 == 0 {
            control.check().ok()?;
        }
        if i > start + header_size + 2
            && crc == 0
            && let Some((size, count)) = flac_header(&frames[i..])
        {
            start = i;
            header_size = size;
            samples = samples.checked_add(count)?;
        }
        crc = crc16(crc, frames[i]);
    }
    (crc == 0 && frames.len() > start + header_size + 2 && total > 0 && samples == total)
        .then_some(())
}
