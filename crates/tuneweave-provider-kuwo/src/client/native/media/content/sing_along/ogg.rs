//! Complete single-stream Ogg framing and bounded Vorbis headers before decoding.
use super::*;

pub(super) struct Source {
    pub(super) frames: u64,
    pub(super) serial: u32,
}
const MAX_PACKET: usize = 1024 * 1024;

pub(super) fn inspect(bytes: &[u8], control: &Control) -> Result<Source> {
    control.check()?;
    let result = inspect_inner(bytes, control);
    control.check()?;
    result.ok_or_else(invalid)
}
fn inspect_inner(mut bytes: &[u8], control: &Control) -> Option<Source> {
    let mut serial = None;
    let mut sequence = 0_u32;
    let mut packet = Vec::new();
    let mut packet_length = 0_usize;
    let mut packet_first = None;
    let mut packets = 0_usize;
    let mut frames = 0;
    let mut ended = false;
    while !bytes.is_empty() {
        control.check().ok()?;
        let header = bytes.get(..27)?;
        let flags = header[5];
        let continued = packet_length != 0;
        let count = usize::from(header[26]);
        if &header[..5] != b"OggS\0"
            || flags & !7 != 0
            || (flags & 1 != 0) != continued
            || (flags & 2 != 0) != serial.is_none()
            || count == 0
            || ended
        {
            return None;
        }
        let current = u32::from_le_bytes(header[14..18].try_into().ok()?);
        if serial.is_some_and(|s| s != current)
            || u32::from_le_bytes(header[18..22].try_into().ok()?) != sequence
        {
            return None;
        }
        serial = Some(current);
        sequence = sequence.checked_add(1)?;
        let segments = bytes.get(27..27 + count)?;
        let size = 27 + count + segments.iter().map(|&b| usize::from(b)).sum::<usize>();
        let page = bytes.get(..size)?;
        if u32::from_le_bytes(header[22..26].try_into().ok()?) != crc(page) {
            return None;
        }
        let mut payload = &page[27 + count..];
        let mut completed = 0;
        for &segment in segments {
            let length = usize::from(segment);
            let data = payload.get(..length)?;
            packet_length = packet_length.checked_add(length)?;
            if packet_length > MAX_PACKET {
                return None;
            }
            if packet_first.is_none() {
                packet_first = data.first().copied();
            }
            if packets < 3 {
                packet.extend_from_slice(data);
            }
            payload = payload.get(length..)?;
            if segment != 255 {
                match packets {
                    0 => identification(&packet)?,
                    1 => comment(&packet)?,
                    2 => setup_bounds(&packet)?,
                    _ if packet_first? & 1 == 0 => {}
                    _ => return None,
                }
                packets += 1;
                completed += 1;
                packet.clear();
                packet_length = 0;
                packet_first = None;
            }
        }
        let granule = u64::from_le_bytes(header[6..14].try_into().ok()?);
        if completed == 0 {
            if granule != u64::MAX {
                return None;
            }
        } else if packets <= 3 {
            if granule != 0 {
                return None;
            }
        } else {
            if granule < frames || granule > MAX_FRAMES {
                return None;
            }
            frames = granule;
        }
        ended = flags & 4 != 0;
        if ended && (packets < 5 || packet_length != 0 || frames == 0) {
            return None;
        }
        bytes = bytes.get(size..)?;
    }
    ended.then_some(Source {
        frames,
        serial: serial?,
    })
}
fn identification(packet: &[u8]) -> Option<()> {
    if packet.len() != 30
        || packet.get(..7)? != b"\x01vorbis"
        || packet[7..11] != [0; 4]
        || packet[11] != 4
        || u32::from_le_bytes(packet[12..16].try_into().ok()?) != SAMPLE_RATE
        || !(6..=13).contains(&(packet[28] & 15))
        || !(6..=13).contains(&(packet[28] >> 4))
        || packet[28] & 15 > packet[28] >> 4
        || packet[29] != 1
    {
        return None;
    }
    Some(())
}
fn comment(mut packet: &[u8]) -> Option<()> {
    if packet.get(..7)? != b"\x03vorbis" {
        return None;
    }
    packet = packet.get(7..)?;
    fn word(bytes: &mut &[u8]) -> Option<usize> {
        let value = u32::from_le_bytes(bytes.get(..4)?.try_into().ok()?) as usize;
        *bytes = bytes.get(4..)?;
        Some(value)
    }
    let vendor = word(&mut packet)?;
    if vendor > 65_536 {
        return None;
    }
    packet = packet.get(vendor..)?;
    let count = word(&mut packet)?;
    if count > 4096 {
        return None;
    }
    for _ in 0..count {
        let size = word(&mut packet)?;
        if size > 65_536 {
            return None;
        }
        packet = packet.get(size..)?;
    }
    (packet == [1]).then_some(())
}
// Cap codebook allocation before entering the decoder. A tiny ordered-codebook
// packet can otherwise declare far more entries than its own byte size suggests.
fn setup_bounds(packet: &[u8]) -> Option<()> {
    if packet.get(..7)? != b"\x05vorbis" {
        return None;
    }
    let mut bits = Bits {
        data: packet.get(7..)?,
        position: 0,
    };
    let books = bits.read(8)? + 1;
    let mut total_entries = 0_u32;
    let mut total_cells = 0_u32;
    for _ in 0..books {
        if bits.read(24)? != 0x56_43_42 {
            return None;
        }
        let dimensions = bits.read(16)?;
        let entries = bits.read(24)?;
        total_entries = total_entries.checked_add(entries)?;
        total_cells = total_cells.checked_add(entries.checked_mul(dimensions)?)?;
        if !(1..=64).contains(&dimensions)
            || !(1..=65_536).contains(&entries)
            || total_entries > 1_048_576
            || total_cells > 4_194_304
        {
            return None;
        }
        if bits.read(1)? != 0 {
            let mut length = bits.read(5)? + 1;
            let mut assigned = 0;
            while assigned < entries {
                if length > 32 {
                    return None;
                }
                let available = entries - assigned;
                let count = bits.read(32 - available.leading_zeros())?;
                if count > available {
                    return None;
                }
                assigned += count;
                length += 1;
            }
        } else if bits.read(1)? != 0 {
            for _ in 0..entries {
                if bits.read(1)? != 0 {
                    bits.skip(5)?;
                }
            }
        } else {
            bits.skip(entries as usize * 5)?;
        }
        match bits.read(4)? {
            0 => {}
            lookup @ (1 | 2) => {
                bits.skip(64)?;
                let width = bits.read(4)? + 1;
                bits.skip(1)?;
                let values = if lookup == 2 {
                    entries * dimensions
                } else {
                    lookup_values(entries, dimensions)
                };
                bits.skip(values as usize * width as usize)?;
            }
            _ => return None,
        }
    }
    Some(())
}
fn lookup_values(entries: u32, dimensions: u32) -> u32 {
    let (mut low, mut high) = (1_u32, entries);
    while low < high {
        let mid = low + (high - low).div_ceil(2);
        let mut product = 1_u64;
        for _ in 0..dimensions {
            product *= u64::from(mid);
            if product > u64::from(entries) {
                break;
            }
        }
        if product > u64::from(entries) {
            high = mid - 1;
        } else {
            low = mid;
        }
    }
    low
}
struct Bits<'a> {
    data: &'a [u8],
    position: usize,
}
impl Bits<'_> {
    fn read(&mut self, width: u32) -> Option<u32> {
        if width > 32 || self.position.checked_add(width as usize)? > self.data.len() * 8 {
            return None;
        }
        let mut value = 0;
        for i in 0..width {
            value |= u32::from((self.data[self.position / 8] >> (self.position % 8)) & 1) << i;
            self.position += 1;
        }
        Some(value)
    }
    fn skip(&mut self, width: usize) -> Option<()> {
        let end = self.position.checked_add(width)?;
        if end > self.data.len() * 8 {
            return None;
        }
        self.position = end;
        Some(())
    }
}
const fn crc_table() -> [u32; 256] {
    let mut table = [0; 256];
    let mut i = 0;
    while i < 256 {
        let mut value = (i as u32) << 24;
        let mut bit = 0;
        while bit < 8 {
            value = (value << 1)
                ^ if value & 0x8000_0000 != 0 {
                    0x04c1_1db7
                } else {
                    0
                };
            bit += 1;
        }
        table[i] = value;
        i += 1;
    }
    table
}
pub(super) fn crc(page: &[u8]) -> u32 {
    const TABLE: [u32; 256] = crc_table();
    page.iter().enumerate().fold(0, |crc, (i, &byte)| {
        let byte = if (22..26).contains(&i) { 0 } else { byte };
        (crc << 8) ^ TABLE[usize::from((crc >> 24) as u8 ^ byte)]
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sing_along_rejects_codebook_cells_that_overflow_across_books() {
        // The second book's individual multiplication fits in u32, while
        // adding the first book's already accepted cells overflows it.
        // This truncated hostile setup must be rejected before allocation.
        let mut payload = Vec::new();
        let mut bit = 0;
        let mut write = |value: u32, width: usize| {
            for i in 0..width {
                if bit % 8 == 0 {
                    payload.push(0);
                }
                payload[bit / 8] |= (((value >> i) & 1) as u8) << (bit % 8);
                bit += 1;
            }
        };
        write(1, 8); // Two codebooks.
        write(0x56_43_42, 24);
        write(64, 16);
        write(4096, 24);
        write(1, 1); // Ordered, all entries have a twelve-bit code.
        write(11, 5);
        write(4096, 13);
        write(0, 4); // No lookup table.
        write(0x56_43_42, 24);
        write(65_535, 16);
        write(65_535, 24);
        let mut packet = b"\x05vorbis".to_vec();
        packet.extend(payload);
        assert!(setup_bounds(&packet).is_none());
    }
}
