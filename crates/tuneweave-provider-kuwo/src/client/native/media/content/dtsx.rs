//! Bounded DTSX/MMP4 structure checks for original compressed-byte delivery.
//!
//! This deliberately is not a DTS-UHD decoder. It checks the container sample
//! map, the DTSX sample entry and each sample's FTOC sync/length/CRC framing.
//! A structural fixture is not evidence that a DTS-UHD bitstream is playable.
use super::*;

const MAX_BOXES: usize = 16_384;
const MAX_SAMPLES: usize = 500_000;
const DTSX_SAMPLE_RATE: u32 = 48_000;
const DTSX_FRAME_SAMPLES: u64 = 1_024;
const MAX_FTOC_BYTES: usize = 5_408;

#[derive(Clone, Copy)]
struct BoxView<'a> {
    kind: [u8; 4],
    payload: &'a [u8],
    payload_offset: usize,
    end: usize,
}

fn box_views<'a>(bytes: &'a [u8], base: usize) -> Option<Vec<BoxView<'a>>> {
    base.checked_add(bytes.len())?;
    let mut result = Vec::new();
    let mut offset = 0_usize;
    while offset < bytes.len() {
        if result.len() >= MAX_BOXES {
            return None;
        }
        let header = bytes.get(offset..)?;
        let size32 = usize::try_from(u32_at(header, 0)?).ok()?;
        let kind = header.get(4..8)?.try_into().ok()?;
        let (size, header_size) = match size32 {
            0 => return None,
            1 => (
                usize::try_from(u64::from_be_bytes(header.get(8..16)?.try_into().ok()?)).ok()?,
                16,
            ),
            size => (size, 8),
        };
        if size < header_size {
            return None;
        }
        let next = offset.checked_add(size)?;
        let payload_start = offset.checked_add(header_size)?;
        let payload = bytes.get(payload_start..next)?;
        result.push(BoxView {
            kind,
            payload,
            payload_offset: base.checked_add(payload_start)?,
            end: base.checked_add(next)?,
        });
        offset = next;
    }
    (offset == bytes.len()).then_some(result)
}

fn unique_box<'a>(boxes: &[BoxView<'a>], kind: &[u8; 4]) -> Option<BoxView<'a>> {
    let mut matches = boxes.iter().filter(|item| &item.kind == kind);
    let found = *matches.next()?;
    matches.next().is_none().then_some(found)
}

fn optional_box<'a>(boxes: &[BoxView<'a>], kind: &[u8; 4]) -> Option<Option<BoxView<'a>>> {
    let mut found = None;
    for item in boxes.iter().filter(|item| &item.kind == kind) {
        if found.is_some() {
            return None;
        }
        found = Some(*item);
    }
    Some(found)
}

fn version_zero(payload: &[u8]) -> Option<&[u8]> {
    (payload.get(..4)? == [0, 0, 0, 0]).then_some(&payload[4..])
}

fn u32_at(bytes: &[u8], start: usize) -> Option<u32> {
    Some(u32::from_be_bytes(
        bytes.get(start..start.checked_add(4)?)?.try_into().ok()?,
    ))
}

fn u16_at(bytes: &[u8], start: usize) -> Option<u16> {
    Some(u16::from_be_bytes(
        bytes.get(start..start.checked_add(2)?)?.try_into().ok()?,
    ))
}

struct Bits<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> Bits<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, position: 0 }
    }

    fn read(&mut self, count: usize) -> Option<u32> {
        if count > 32 {
            return None;
        }
        let mut value = 0_u32;
        for _ in 0..count {
            let byte = *self.bytes.get(self.position / 8)?;
            let bit = (byte >> (7 - (self.position % 8))) & 1;
            value = (value << 1) | u32::from(bit);
            self.position = self.position.checked_add(1)?;
        }
        Some(value)
    }

    fn align_zero(&mut self) -> Option<()> {
        while self.position % 8 != 0 {
            if self.read(1)? != 0 {
                return None;
            }
        }
        Some(())
    }
}

fn validate_udts(payload: &[u8]) -> Option<()> {
    let mut bits = Bits::new(payload);
    let decoder_profile = bits.read(6)?;
    let frame_duration = bits.read(2)?;
    let max_payload = bits.read(3)?;
    let presentations_code = bits.read(5)?;
    let channel_mask = bits.read(32)?;
    let base_sampling_frequency = bits.read(1)?;
    let sample_rate_mod = bits.read(2)?;
    let _representation_type = bits.read(3)?;
    let stream_index = bits.read(3)?;
    let expansion_box_present = bits.read(1)?;

    // Annex B maps NumPresentationsCode + 1. Annex F says one presentation
    // for Profile 2 but sets this code to 1. Accept only the two directly
    // plausible values and leave that disputed field uninterpreted.
    if decoder_profile != 0
        || frame_duration != 1
        || max_payload != 0
        || presentations_code > 1
        || channel_mask == 0
        || base_sampling_frequency != 1
        || sample_rate_mod != 0
        || stream_index != 0
        || expansion_box_present != 0
    {
        return None;
    }
    for _ in 0..=presentations_code {
        if bits.read(1)? != 0 {
            return None;
        }
    }
    bits.align_zero()?;
    (bits.position == payload.len().checked_mul(8)?).then_some(())
}

fn validate_sample_entry(stsd: BoxView<'_>) -> Option<()> {
    let payload = stsd.payload;
    let body = version_zero(payload)?;
    if body.len() < 4 || u32_at(body, 0)? != 1 {
        return None;
    }
    let entry_start = 8;
    let entries = box_views(
        &payload[entry_start..],
        stsd.payload_offset.checked_add(entry_start)?,
    )?;
    if entries.len() != 1 || entries[0].kind != *b"dtsx" {
        return None;
    }
    let entry = entries[0];
    let audio = entry.payload;
    if audio.len() < 28
        || u16_at(audio, 6)? != 1
        || u16_at(audio, 8)? != 0
        || !(1..=10).contains(&u16_at(audio, 16)?)
        || u16_at(audio, 18)? != 16
        || u32_at(audio, 24)? != DTSX_SAMPLE_RATE << 16
    {
        return None;
    }
    let child_start = 28;
    let children = box_views(
        &audio[child_start..],
        entry.payload_offset.checked_add(child_start)?,
    )?;
    if children
        .iter()
        .any(|item| !matches!(&item.kind, b"udts" | b"chan" | b"btrt"))
    {
        return None;
    }
    validate_udts(unique_box(&children, b"udts")?.payload)
}

fn sample_sizes(stsz: BoxView<'_>) -> Option<Vec<usize>> {
    let body = version_zero(stsz.payload)?;
    if body.len() < 8 {
        return None;
    }
    let fixed_size = usize::try_from(u32_at(body, 0)?).ok()?;
    let count = usize::try_from(u32_at(body, 4)?).ok()?;
    if count == 0 || count > MAX_SAMPLES {
        return None;
    }
    if fixed_size != 0 {
        return (body.len() == 8 && fixed_size != 0).then(|| vec![fixed_size; count]);
    }
    if body.len() != 8_usize.checked_add(count.checked_mul(4)?)? {
        return None;
    }
    let mut sizes = Vec::with_capacity(count);
    for index in 0..count {
        let size = usize::try_from(u32_at(body, 8 + index * 4)?).ok()?;
        if size == 0 {
            return None;
        }
        sizes.push(size);
    }
    Some(sizes)
}

#[derive(Clone, Copy)]
struct ChunkRule {
    first_chunk: usize,
    samples_per_chunk: usize,
}

fn chunk_rules(stsc: BoxView<'_>, chunk_count: usize) -> Option<Vec<ChunkRule>> {
    let body = version_zero(stsc.payload)?;
    if body.len() < 4 {
        return None;
    }
    let count = usize::try_from(u32_at(body, 0)?).ok()?;
    if count == 0
        || count > chunk_count
        || body.len() != 4_usize.checked_add(count.checked_mul(12)?)?
    {
        return None;
    }
    let mut rules = Vec::with_capacity(count);
    for index in 0..count {
        let offset = 4 + index * 12;
        let first_chunk = usize::try_from(u32_at(body, offset)?).ok()?;
        let samples_per_chunk = usize::try_from(u32_at(body, offset + 4)?).ok()?;
        let description = u32_at(body, offset + 8)?;
        if first_chunk == 0
            || first_chunk > chunk_count
            || samples_per_chunk == 0
            || description != 1
            || (index == 0 && first_chunk != 1)
            || rules
                .last()
                .is_some_and(|previous: &ChunkRule| previous.first_chunk >= first_chunk)
        {
            return None;
        }
        rules.push(ChunkRule {
            first_chunk,
            samples_per_chunk,
        });
    }
    Some(rules)
}

fn chunk_offsets(table: BoxView<'_>, wide: bool) -> Option<Vec<usize>> {
    let body = version_zero(table.payload)?;
    if body.len() < 4 {
        return None;
    }
    let count = usize::try_from(u32_at(body, 0)?).ok()?;
    let width = if wide { 8 } else { 4 };
    if count == 0
        || count > MAX_SAMPLES
        || body.len() != 4_usize.checked_add(count.checked_mul(width)?)?
    {
        return None;
    }
    let mut offsets = Vec::with_capacity(count);
    for index in 0..count {
        let position = 4 + index * width;
        let offset = if wide {
            usize::try_from(u64::from_be_bytes(
                body.get(position..position + 8)?.try_into().ok()?,
            ))
            .ok()?
        } else {
            usize::try_from(u32_at(body, position)?).ok()?
        };
        offsets.push(offset);
    }
    Some(offsets)
}

fn media_timing(mdhd: BoxView<'_>) -> Option<(u64, u64)> {
    let payload = mdhd.payload;
    if payload.len() < 4 || payload[1..4] != [0, 0, 0] {
        return None;
    }
    match payload[0] {
        0 if payload.len() == 24 => Some((
            u64::from(u32_at(payload, 12)?),
            u64::from(u32_at(payload, 16)?),
        )),
        1 if payload.len() == 36 => Some((
            u64::from(u32_at(payload, 20)?),
            u64::from_be_bytes(payload.get(24..32)?.try_into().ok()?),
        )),
        _ => None,
    }
}

fn validate_sample_times(
    stts: BoxView<'_>,
    count: usize,
    timescale: u64,
    duration: u64,
) -> Option<()> {
    let body = version_zero(stts.payload)?;
    if body.len() < 4 || timescale == 0 || timescale % u64::from(DTSX_SAMPLE_RATE) != 0 {
        return None;
    }
    let entries = usize::try_from(u32_at(body, 0)?).ok()?;
    if entries == 0
        || entries > count
        || body.len() != 4_usize.checked_add(entries.checked_mul(8)?)?
    {
        return None;
    }
    let expected_delta =
        (timescale / u64::from(DTSX_SAMPLE_RATE)).checked_mul(DTSX_FRAME_SAMPLES)?;
    let mut samples = 0_usize;
    let mut total_duration = 0_u64;
    for index in 0..entries {
        let offset = 4 + index * 8;
        let entry_samples = usize::try_from(u32_at(body, offset)?).ok()?;
        let delta = u64::from(u32_at(body, offset + 4)?);
        if entry_samples == 0 || delta != expected_delta {
            return None;
        }
        samples = samples.checked_add(entry_samples)?;
        total_duration = total_duration.checked_add(delta.checked_mul(entry_samples as u64)?)?;
    }
    (samples == count && total_duration == duration).then_some(())
}

fn sync_samples(stss: BoxView<'_>, count: usize) -> Option<Vec<bool>> {
    let body = version_zero(stss.payload)?;
    if body.len() < 4 {
        return None;
    }
    let entries = usize::try_from(u32_at(body, 0)?).ok()?;
    if entries == 0
        || entries > count
        || body.len() != 4_usize.checked_add(entries.checked_mul(4)?)?
    {
        return None;
    }
    let mut syncs = vec![false; count];
    let mut previous = 0;
    for index in 0..entries {
        let sample = usize::try_from(u32_at(body, 4 + index * 4)?).ok()?;
        if sample == 0 || sample > count || sample <= previous {
            return None;
        }
        syncs[sample - 1] = true;
        previous = sample;
    }
    Some(syncs)
}

fn ftoc_length(bits: &mut Bits<'_>) -> Option<usize> {
    let (width, base) = if bits.read(1)? == 0 {
        (5, 0)
    } else if bits.read(1)? == 0 {
        (8, 1 << 5)
    } else if bits.read(1)? == 0 {
        (10, (1 << 5) + (1 << 8))
    } else {
        (12, (1 << 5) + (1 << 8) + (1 << 10))
    };
    usize::try_from(base + bits.read(width)? + 1).ok()
}

fn dtsx_crc16(bytes: &[u8]) -> u16 {
    let mut crc = 0xffff_u16;
    for byte in bytes {
        crc ^= u16::from(*byte) << 8;
        for _ in 0..8 {
            crc = (crc << 1) ^ if crc & 0x8000 != 0 { 0x1021 } else { 0 };
        }
    }
    crc
}

fn validate_frame(sample: &[u8], full_channel_mix: &mut Option<bool>) -> Option<bool> {
    let sync = u32::from_be_bytes(sample.get(..4)?.try_into().ok()?);
    let is_sync = match sync {
        0x4041_1bf2 => true,
        0x71c4_42e8 => false,
        _ => return None,
    };
    let mut bits = Bits::new(sample);
    bits.position = 32;
    let length = ftoc_length(&mut bits)?;
    if !(8..=MAX_FTOC_BYTES).contains(&length) || length >= sample.len() {
        return None;
    }
    let full_channel = if is_sync {
        let value = bits.read(1)? != 0;
        *full_channel_mix = Some(value);
        value
    } else {
        (*full_channel_mix)?
    };
    let has_crc = is_sync || !full_channel;
    let protected_len = length.checked_sub(if has_crc { 2 } else { 0 })?;
    if bits.position > protected_len.checked_mul(8)? {
        return None;
    }
    if has_crc {
        let expected = u16_at(&sample[..length], length - 2)?;
        if dtsx_crc16(&sample[..length - 2]) != expected {
            return None;
        }
    }
    Some(is_sync)
}

fn validate_media_data(
    bytes: &[u8],
    mdats: &[BoxView<'_>],
    sizes: &[usize],
    offsets: &[usize],
    rules: &[ChunkRule],
    control: &Control,
) -> Option<Vec<bool>> {
    let total_mdat = mdats
        .iter()
        .try_fold(0_usize, |total, item| total.checked_add(item.payload.len()))?;
    let total_samples = sizes
        .iter()
        .try_fold(0_usize, |total, size| total.checked_add(*size))?;
    if total_mdat == 0 || total_samples != total_mdat {
        return None;
    }
    let ranges: Vec<_> = mdats
        .iter()
        .map(|item| (item.payload_offset, item.end))
        .collect();
    let mut chunk_ranges = Vec::with_capacity(offsets.len());
    let mut syncs = Vec::with_capacity(sizes.len());
    let mut sample_index = 0_usize;
    let mut rule_index = 0_usize;
    let mut full_channel_mix = None;

    for (chunk_index, chunk_offset) in offsets.iter().copied().enumerate() {
        control.check().ok()?;
        let chunk_number = chunk_index.checked_add(1)?;
        while rule_index + 1 < rules.len() && rules[rule_index + 1].first_chunk <= chunk_number {
            rule_index += 1;
        }
        let per_chunk = rules.get(rule_index)?.samples_per_chunk;
        let sample_end = sample_index.checked_add(per_chunk)?;
        if sample_end > sizes.len() {
            return None;
        }
        let chunk_size = sizes[sample_index..sample_end]
            .iter()
            .try_fold(0_usize, |total, size| total.checked_add(*size))?;
        let chunk_end = chunk_offset.checked_add(chunk_size)?;
        if !ranges
            .iter()
            .any(|(start, end)| chunk_offset >= *start && chunk_end <= *end)
        {
            return None;
        }
        chunk_ranges.push((chunk_offset, chunk_end));
        let mut position = chunk_offset;
        for sample_size in &sizes[sample_index..sample_end] {
            control.check().ok()?;
            let end = position.checked_add(*sample_size)?;
            let sample = bytes.get(position..end)?;
            syncs.push(validate_frame(sample, &mut full_channel_mix)?);
            position = end;
        }
        sample_index = sample_end;
    }
    if sample_index != sizes.len() || syncs.first() != Some(&true) {
        return None;
    }
    chunk_ranges.sort_unstable_by_key(|range| range.0);
    if chunk_ranges.windows(2).any(|pair| pair[0].1 > pair[1].0) {
        return None;
    }
    for (mdat_start, mdat_end) in &ranges {
        let mut cursor = *mdat_start;
        for (chunk_start, chunk_end) in chunk_ranges
            .iter()
            .copied()
            .filter(|(start, _)| start >= mdat_start && start < mdat_end)
        {
            if chunk_start != cursor {
                return None;
            }
            cursor = chunk_end;
        }
        if cursor != *mdat_end {
            return None;
        }
    }
    Some(syncs)
}

fn validate_sample_table(
    bytes: &[u8],
    stbl: BoxView<'_>,
    mdats: &[BoxView<'_>],
    timescale: u64,
    duration: u64,
    control: &Control,
) -> Option<()> {
    let boxes = box_views(stbl.payload, stbl.payload_offset)?;
    if boxes.iter().any(|item| {
        !matches!(
            &item.kind,
            b"stsd" | b"stts" | b"stsc" | b"stsz" | b"stco" | b"co64" | b"stss"
        )
    }) {
        return None;
    }
    let stsd = unique_box(&boxes, b"stsd")?;
    let stts = unique_box(&boxes, b"stts")?;
    let stsc = unique_box(&boxes, b"stsc")?;
    let stsz = unique_box(&boxes, b"stsz")?;
    validate_sample_entry(stsd)?;
    let sizes = sample_sizes(stsz)?;
    let stco = optional_box(&boxes, b"stco")?;
    let co64 = optional_box(&boxes, b"co64")?;
    let offsets = match (stco, co64) {
        (Some(table), None) => chunk_offsets(table, false)?,
        (None, Some(table)) => chunk_offsets(table, true)?,
        _ => return None,
    };
    let rules = chunk_rules(stsc, offsets.len())?;
    validate_sample_times(stts, sizes.len(), timescale, duration)?;
    let actual_syncs = validate_media_data(bytes, mdats, &sizes, &offsets, &rules, control)?;
    match optional_box(&boxes, b"stss")? {
        Some(stss) if sync_samples(stss, sizes.len())? != actual_syncs => return None,
        None if actual_syncs.iter().any(|sync| !sync) => return None,
        _ => {}
    }
    Some(())
}

pub(super) fn inspect(bytes: &[u8], control: &Control) -> Option<()> {
    let top = box_views(bytes, 0)?;
    if top.first()?.kind != *b"ftyp"
        || top.iter().any(|item| {
            !matches!(
                &item.kind,
                b"ftyp" | b"moov" | b"mdat" | b"free" | b"skip" | b"wide"
            )
        })
    {
        return None;
    }
    let ftyp = unique_box(&top, b"ftyp")?;
    if ftyp.payload.len() < 8 || (ftyp.payload.len() - 8) % 4 != 0 {
        return None;
    }
    let moov = unique_box(&top, b"moov")?;
    let mdats: Vec<_> = top
        .iter()
        .copied()
        .filter(|item| item.kind == *b"mdat")
        .collect();
    if mdats.is_empty() {
        return None;
    }
    let moov_children = box_views(moov.payload, moov.payload_offset)?;
    if moov_children.iter().any(|item| item.kind == *b"mvex") {
        return None;
    }
    let trak = unique_box(&moov_children, b"trak")?;
    let trak_children = box_views(trak.payload, trak.payload_offset)?;
    if trak_children.iter().any(|item| item.kind == *b"edts") {
        return None;
    }
    let mdia = unique_box(&trak_children, b"mdia")?;
    let mdia_children = box_views(mdia.payload, mdia.payload_offset)?;
    let hdlr = unique_box(&mdia_children, b"hdlr")?;
    if hdlr.payload.get(8..12)? != b"soun" {
        return None;
    }
    let (timescale, duration) = media_timing(unique_box(&mdia_children, b"mdhd")?)?;
    let minf = unique_box(&mdia_children, b"minf")?;
    let minf_children = box_views(minf.payload, minf.payload_offset)?;
    if unique_box(&minf_children, b"smhd")?.payload.len() != 8 {
        return None;
    }
    let dinf = unique_box(&minf_children, b"dinf")?;
    let dinf_children = box_views(dinf.payload, dinf.payload_offset)?;
    let dref = unique_box(&dinf_children, b"dref")?;
    let dref_body = version_zero(dref.payload)?;
    if dref_body.len() < 4 || u32_at(dref_body, 0)? != 1 {
        return None;
    }
    let entry_start = 8;
    let references = box_views(
        &dref.payload[entry_start..],
        dref.payload_offset.checked_add(entry_start)?,
    )?;
    if references.len() != 1
        || references[0].kind != *b"url "
        || references[0].payload != [0, 0, 0, 1]
    {
        return None;
    }
    let stbl = unique_box(&minf_children, b"stbl")?;
    validate_sample_table(bytes, stbl, &mdats, timescale, duration, control)
}

#[cfg(test)]
pub(crate) fn synthetic_fixture() -> Vec<u8> {
    fn box_bytes(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let mut result = Vec::with_capacity(payload.len() + 8);
        result.extend_from_slice(&u32::try_from(payload.len() + 8).unwrap().to_be_bytes());
        result.extend_from_slice(kind);
        result.extend_from_slice(payload);
        result
    }
    fn write_bits(bytes: &mut [u8], position: &mut usize, value: u32, count: usize) {
        for bit in (0..count).rev() {
            if ((value >> bit) & 1) != 0 {
                bytes[*position / 8] |= 1 << (7 - (*position % 8));
            }
            *position += 1;
        }
    }
    fn make_frame(sync: bool, full_channel_mix: bool) -> Vec<u8> {
        let mut frame = vec![0_u8; 40];
        frame[..4].copy_from_slice(
            &(if sync {
                0x4041_1bf2_u32
            } else {
                0x71c4_42e8_u32
            })
            .to_be_bytes(),
        );
        let mut position = 32;
        write_bits(&mut frame, &mut position, 0, 1); // short FTOC length code
        write_bits(&mut frame, &mut position, 31, 5); // 32-byte FTOC
        if sync {
            write_bits(&mut frame, &mut position, u32::from(full_channel_mix), 1);
        }
        if sync || !full_channel_mix {
            let crc = dtsx_crc16(&frame[..30]);
            frame[30..32].copy_from_slice(&crc.to_be_bytes());
        }
        frame
    }
    fn make_udts() -> Vec<u8> {
        let mut payload = vec![0_u8; 8];
        let mut position = 0;
        write_bits(&mut payload, &mut position, 0, 6); // decoder profile code
        write_bits(&mut payload, &mut position, 1, 2); // 1024 samples
        write_bits(&mut payload, &mut position, 0, 3); // max payload code
        write_bits(&mut payload, &mut position, 1, 5); // preserve Annex F value
        write_bits(&mut payload, &mut position, 3, 32); // stereo channel mask
        write_bits(&mut payload, &mut position, 1, 1); // 48 kHz base clock
        write_bits(&mut payload, &mut position, 0, 2); // unmodified sample rate
        write_bits(&mut payload, &mut position, 0, 3); // representation type
        write_bits(&mut payload, &mut position, 0, 3); // stream index
        write_bits(&mut payload, &mut position, 0, 1); // no expansion box
        write_bits(&mut payload, &mut position, 0, 2); // no presentation ID tags
        box_bytes(b"udts", &payload)
    }
    fn make_moov(mdat_payload_offset: u32) -> Vec<u8> {
        let mut sample_entry = vec![0_u8; 28];
        sample_entry[6..8].copy_from_slice(&1_u16.to_be_bytes());
        sample_entry[16..18].copy_from_slice(&2_u16.to_be_bytes());
        sample_entry[18..20].copy_from_slice(&16_u16.to_be_bytes());
        sample_entry[24..28].copy_from_slice(&(DTSX_SAMPLE_RATE << 16).to_be_bytes());
        sample_entry.extend_from_slice(&make_udts());
        let dtsx_entry = box_bytes(b"dtsx", &sample_entry);

        let mut stsd_body = vec![0_u8; 4];
        stsd_body.extend_from_slice(&1_u32.to_be_bytes());
        stsd_body.extend_from_slice(&dtsx_entry);
        let stsd = box_bytes(b"stsd", &stsd_body);

        let mut stts_body = vec![0_u8; 4];
        stts_body.extend_from_slice(&1_u32.to_be_bytes());
        stts_body.extend_from_slice(&2_u32.to_be_bytes());
        stts_body.extend_from_slice(&1024_u32.to_be_bytes());
        let stts = box_bytes(b"stts", &stts_body);

        let mut stsc_body = vec![0_u8; 4];
        stsc_body.extend_from_slice(&1_u32.to_be_bytes());
        stsc_body.extend_from_slice(&1_u32.to_be_bytes());
        stsc_body.extend_from_slice(&2_u32.to_be_bytes());
        stsc_body.extend_from_slice(&1_u32.to_be_bytes());
        let stsc = box_bytes(b"stsc", &stsc_body);

        let mut stsz_body = vec![0_u8; 4];
        stsz_body.extend_from_slice(&0_u32.to_be_bytes());
        stsz_body.extend_from_slice(&2_u32.to_be_bytes());
        stsz_body.extend_from_slice(&40_u32.to_be_bytes());
        stsz_body.extend_from_slice(&40_u32.to_be_bytes());
        let stsz = box_bytes(b"stsz", &stsz_body);

        let mut stco_body = vec![0_u8; 4];
        stco_body.extend_from_slice(&1_u32.to_be_bytes());
        stco_body.extend_from_slice(&mdat_payload_offset.to_be_bytes());
        let stco = box_bytes(b"stco", &stco_body);

        let mut stss_body = vec![0_u8; 4];
        stss_body.extend_from_slice(&1_u32.to_be_bytes());
        stss_body.extend_from_slice(&1_u32.to_be_bytes());
        let stss = box_bytes(b"stss", &stss_body);

        let mut stbl_body = Vec::new();
        for item in [stsd, stts, stsc, stsz, stco, stss] {
            stbl_body.extend_from_slice(&item);
        }
        let stbl = box_bytes(b"stbl", &stbl_body);

        let url_reference = vec![0, 0, 0, 1];
        let url = box_bytes(b"url ", &url_reference);
        let mut dref_body = vec![0_u8; 4];
        dref_body.extend_from_slice(&1_u32.to_be_bytes());
        dref_body.extend_from_slice(&url);
        let dref = box_bytes(b"dref", &dref_body);
        let dinf = box_bytes(b"dinf", &dref);
        let smhd_body = vec![0_u8; 8];
        let smhd = box_bytes(b"smhd", &smhd_body);
        let mut minf_body = Vec::new();
        minf_body.extend_from_slice(&smhd);
        minf_body.extend_from_slice(&dinf);
        minf_body.extend_from_slice(&stbl);
        let minf = box_bytes(b"minf", &minf_body);

        let mut mdhd_body = vec![0_u8; 4];
        mdhd_body.extend_from_slice(&0_u32.to_be_bytes());
        mdhd_body.extend_from_slice(&0_u32.to_be_bytes());
        mdhd_body.extend_from_slice(&DTSX_SAMPLE_RATE.to_be_bytes());
        mdhd_body.extend_from_slice(&2048_u32.to_be_bytes());
        mdhd_body.extend_from_slice(&0_u16.to_be_bytes());
        mdhd_body.extend_from_slice(&0_u16.to_be_bytes());
        let mdhd = box_bytes(b"mdhd", &mdhd_body);

        let mut hdlr_body = vec![0_u8; 4];
        hdlr_body.extend_from_slice(&0_u32.to_be_bytes());
        hdlr_body.extend_from_slice(b"soun");
        hdlr_body.extend_from_slice(&[0; 12]);
        let hdlr = box_bytes(b"hdlr", &hdlr_body);

        let mut mdia_body = Vec::new();
        mdia_body.extend_from_slice(&mdhd);
        mdia_body.extend_from_slice(&hdlr);
        mdia_body.extend_from_slice(&minf);
        let mdia = box_bytes(b"mdia", &mdia_body);
        let trak = box_bytes(b"trak", &mdia);

        let mut mvhd_body = vec![0_u8; 108];
        mvhd_body[12..16].copy_from_slice(&1000_u32.to_be_bytes());
        mvhd_body[16..20].copy_from_slice(&43_u32.to_be_bytes());
        mvhd_body[20..24].copy_from_slice(&0x0001_0000_u32.to_be_bytes());
        mvhd_body[24..26].copy_from_slice(&0x0100_u16.to_be_bytes());
        let mvhd = box_bytes(b"mvhd", &mvhd_body);
        let mut moov_body = Vec::new();
        moov_body.extend_from_slice(&mvhd);
        moov_body.extend_from_slice(&trak);
        box_bytes(b"moov", &moov_body)
    }

    let first = make_frame(true, true);
    let second = make_frame(false, true);
    let mut media = first;
    media.extend_from_slice(&second);
    let ftyp = box_bytes(b"ftyp", b"isom\0\0\0\0isom");
    let placeholder_moov = make_moov(0);
    let data_offset = u32::try_from(ftyp.len() + placeholder_moov.len() + 8).unwrap();
    let moov = make_moov(data_offset);
    debug_assert_eq!(placeholder_moov.len(), moov.len());
    let mdat = box_bytes(b"mdat", &media);
    let mut result = ftyp;
    result.extend_from_slice(&moov);
    result.extend_from_slice(&mdat);
    result
}
