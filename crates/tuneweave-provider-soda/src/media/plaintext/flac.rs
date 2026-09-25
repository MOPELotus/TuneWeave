//! RFC 9639 metadata, subframe framing, numbering and CRC checks. No PCM decoding.
mod subframes;
#[cfg(test)]
mod tests;

#[derive(Clone, Copy)]
struct Info {
    rate: u32,
    channels: u8,
    bits: u8,
    total: u64,
    min_block: u32,
    max_block: u32,
}
struct Header {
    size: usize,
    count: u64,
    number: u64,
    variable: bool,
}

pub(super) fn inspect(bytes: &[u8]) -> Option<usize> {
    if bytes.get(..4)? != b"fLaC" {
        return None;
    }
    let mut pos = 4;
    let mut info = None;
    for index in 0..super::MAX_FLAC_METADATA_BLOCKS {
        let tag = *bytes.get(pos)?;
        let size = (usize::from(*bytes.get(pos + 1)?) << 16)
            | (usize::from(*bytes.get(pos + 2)?) << 8)
            | usize::from(*bytes.get(pos + 3)?);
        pos = pos.checked_add(4)?;
        let metadata = bytes.get(pos..pos.checked_add(size)?)?;
        if index == 0 {
            if tag & 127 != 0 || size != 34 {
                return None;
            }
            let min = u16::from_be_bytes(metadata[..2].try_into().ok()?);
            let max = u16::from_be_bytes(metadata[2..4].try_into().ok()?);
            let packed = u64::from_be_bytes(metadata[10..18].try_into().ok()?);
            let value = Info {
                rate: (packed >> 44) as u32,
                channels: ((packed >> 41) & 7) as u8 + 1,
                bits: ((packed >> 36) & 31) as u8 + 1,
                total: packed & 0xf_ffff_ffff,
                min_block: u32::from(min),
                max_block: u32::from(max),
            };
            if min < 16 || max < min || value.rate == 0 || value.bits < 4 {
                return None;
            }
            info = Some(value);
        } else if matches!(tag & 127, 0 | 127) {
            return None;
        }
        pos = pos.checked_add(size)?;
        if pos > super::MAX_FLAC_METADATA_BYTES {
            return None;
        }
        if tag & 128 != 0 {
            return frames(bytes.get(pos..)?, info?);
        }
    }
    None
}

fn frames(bytes: &[u8], info: Info) -> Option<usize> {
    let first = header(bytes, info)?;
    let (mut start, mut count, mut samples) = (0, 0, 0_u64);
    while start < bytes.len() {
        let remaining = &bytes[start..];
        let current = header(remaining, info)?;
        if current.variable != first.variable
            || current.number
                != if current.variable {
                    samples
                } else {
                    count as u64
                }
            || count >= super::MAX_SAMPLE_COUNT
        {
            return None;
        }
        let body = subframes::size(
            remaining.get(current.size..)?,
            current.count as usize,
            info.bits,
            remaining[3] >> 4,
        )?;
        let end = current.size.checked_add(body)?.checked_add(2)?;
        let frame = remaining.get(..end)?;
        let crc = frame.iter().fold(0_u16, |crc, b| {
            (crc << 8) ^ CRC16[usize::from((crc >> 8) as u8 ^ b)]
        });
        if crc != 0 {
            return None;
        }
        start = start.checked_add(end)?;
        // Only the final frame may be shorter than the fixed/advertised block size.
        if start < bytes.len()
            && (current.count < u64::from(info.min_block)
                || (!current.variable && current.count != first.count))
        {
            return None;
        }
        if !current.variable && current.count > first.count {
            return None;
        }
        samples = samples.checked_add(current.count)?;
        count += 1;
    }
    (info.total == 0 || samples == info.total).then_some(count)
}

fn header(bytes: &[u8], info: Info) -> Option<Header> {
    let h = bytes.get(..5)?;
    if h[0] != 255 || h[1] & 0xfe != 0xf8 || h[3] & 1 != 0 {
        return None;
    }
    let channels = match h[3] >> 4 {
        n @ 0..=7 => n + 1,
        8..=10 => 2,
        _ => return None,
    };
    let bits = match (h[3] >> 1) & 7 {
        0 => info.bits,
        1 => 8,
        2 => 12,
        4 => 16,
        5 => 20,
        6 => 24,
        7 => 32,
        _ => return None,
    };
    if channels != info.channels || bits != info.bits {
        return None;
    }
    let leading = h[4].leading_ones() as usize;
    let n = match leading {
        0 => 1,
        2..=7 => leading,
        _ => return None,
    };
    let mut number = if n == 1 {
        u64::from(h[4])
    } else {
        u64::from(h[4] & (0x7f >> n))
    };
    for b in bytes.get(5..4 + n)? {
        if b & 0xc0 != 0x80 {
            return None;
        }
        number = (number << 6) | u64::from(b & 63);
    }
    let minimum = [0, 0, 0x80, 0x800, 0x10000, 0x200000, 0x4000000, 0x80000000][n];
    let variable = h[1] & 1 != 0;
    if number < minimum || (!variable && number > 0x7fff_ffff) || number > 0xf_ffff_ffff {
        return None;
    }
    let mut pos = 4 + n;
    let count = match h[2] >> 4 {
        0 => return None,
        1 => 192,
        n @ 2..=5 => 576_u64 << (n - 2),
        6 => {
            let v = u64::from(*bytes.get(pos)?) + 1;
            pos += 1;
            v
        }
        7 => {
            let v = u64::from(u16::from_be_bytes(
                bytes.get(pos..pos + 2)?.try_into().ok()?,
            )) + 1;
            pos += 2;
            v
        }
        n => 256_u64 << (n - 8),
    };
    if count > u64::from(info.max_block) {
        return None;
    }
    let rate = match h[2] & 15 {
        0 => info.rate,
        n @ 1..=11 => [
            88200, 176400, 192000, 8000, 16000, 22050, 24000, 32000, 44100, 48000, 96000,
        ][usize::from(n - 1)],
        12 => {
            let v = u32::from(*bytes.get(pos)?) * 1000;
            pos += 1;
            v
        }
        n @ (13 | 14) => {
            let v = u32::from(u16::from_be_bytes(
                bytes.get(pos..pos + 2)?.try_into().ok()?,
            ));
            pos += 2;
            v * if n == 14 { 10 } else { 1 }
        }
        _ => return None,
    };
    if rate != info.rate || crc8(bytes.get(..pos + 1)?) != 0 {
        return None;
    }
    Some(Header {
        size: pos + 1,
        count,
        number,
        variable,
    })
}

fn crc8(bytes: &[u8]) -> u8 {
    let mut crc = 0;
    for b in bytes {
        crc ^= b;
        for _ in 0..8 {
            crc = (crc << 1) ^ if crc & 128 != 0 { 7 } else { 0 };
        }
    }
    crc
}
const CRC16: [u16; 256] = {
    let mut table = [0; 256];
    let mut i = 0;
    while i < 256 {
        let mut crc = (i as u16) << 8;
        let mut bit = 0;
        while bit < 8 {
            crc = (crc << 1) ^ if crc & 0x8000 != 0 { 0x8005 } else { 0 };
            bit += 1;
        }
        table[i] = crc;
        i += 1;
    }
    table
};
