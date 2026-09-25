// MPEG-4 DecoderConfigDescriptor and Apple's ALACSpecificConfig identify the
// codec; the mp4a sample-entry label alone also covers non-AAC audio.
fn descriptor(bytes: &[u8]) -> Option<(u8, &[u8], &[u8])> {
    let tag = *bytes.first()?;
    let mut length = 0_usize;
    for n in 1..=4 {
        let b = *bytes.get(n)?;
        length = (length << 7) | usize::from(b & 127);
        if b & 128 == 0 {
            let end = n.checked_add(1)?.checked_add(length)?;
            return Some((tag, bytes.get(n + 1..end)?, bytes.get(end..)?));
        }
    }
    None
}
fn unique(mut bytes: &[u8], tag: u8) -> Option<&[u8]> {
    let mut found = None;
    let mut count = 0;
    while !bytes.is_empty() {
        count += 1;
        if count > 32 {
            return None;
        }
        let (kind, data, rest) = descriptor(bytes)?;
        if kind == tag && found.replace(data).is_some() {
            return None;
        }
        bytes = rest;
    }
    found
}
pub(super) fn aac(bytes: &[u8]) -> bool {
    aac_config(bytes).is_some()
}
fn aac_config(bytes: &[u8]) -> Option<()> {
    if bytes.get(..4)? != [0; 4] {
        return None;
    }
    let (tag, es, rest) = descriptor(bytes.get(4..)?)?;
    if tag != 3 || !rest.is_empty() {
        return None;
    }
    let flags = *es.get(2)?;
    let mut pos = 3;
    if flags & 128 != 0 {
        pos += 2;
    }
    if flags & 64 != 0 {
        pos += 1 + usize::from(*es.get(pos)?);
    }
    if flags & 32 != 0 {
        pos += 2;
    }
    let decoder = unique(es.get(pos..)?, 4)?;
    if decoder.len() < 13 || decoder[1] >> 2 != 5 || decoder[1] & 1 != 1 {
        return None;
    }
    // MPEG-2 AAC Main/LC/SSR have dedicated OTIs; MPEG-4 audio needs its AOT.
    if matches!(decoder[0], 0x66..=0x68) {
        return Some(());
    }
    if decoder[0] != 0x40 {
        return None;
    }
    let config = unique(&decoder[13..], 5)?;
    let mut bits = Bits {
        bytes: config,
        pos: 0,
    };
    let mut object = bits.read(5)?;
    if object == 31 {
        object = 32 + bits.read(6)?;
    }
    if !matches!(object, 1..=6 | 17 | 19 | 20 | 22 | 23 | 29 | 39 | 42) {
        return None;
    }
    frequency(&mut bits)?;
    if bits.read(4)? > 7 {
        return None;
    }
    if matches!(object, 5 | 29) {
        frequency(&mut bits)?;
        if !matches!(bits.read(5)?, 1..=4 | 17 | 19 | 20 | 22 | 23) {
            return None;
        }
    }
    // At least one remaining codec configuration byte/bit must be present.
    bits.read(1)?;
    Some(())
}
fn frequency(bits: &mut Bits<'_>) -> Option<()> {
    match bits.read(4)? {
        0..=12 => Some(()),
        15 => (bits.read(24)? > 0).then_some(()),
        _ => None,
    }
}
struct Bits<'a> {
    bytes: &'a [u8],
    pos: usize,
}
impl Bits<'_> {
    fn read(&mut self, n: usize) -> Option<u32> {
        let mut value = 0;
        for _ in 0..n {
            value = (value << 1)
                | u32::from((*self.bytes.get(self.pos / 8)? >> (7 - self.pos % 8)) & 1);
            self.pos += 1;
        }
        Some(value)
    }
}

pub(super) fn alac(bytes: &[u8], channels: u16) -> bool {
    if bytes.len() != 28 || bytes[..4] != [0; 4] {
        return false;
    }
    let data = &bytes[4..];
    let frames = u32::from_be_bytes(data[..4].try_into().unwrap());
    let rate = u32::from_be_bytes(data[20..24].try_into().unwrap());
    frames > 0
        && data[4] == 0
        && matches!(data[5], 16 | 20 | 24 | 32)
        && u16::from(data[9]) == channels
        && rate > 0
        && data[6] > 0
        && data[7] > 0
        && data[8] <= 31
}
