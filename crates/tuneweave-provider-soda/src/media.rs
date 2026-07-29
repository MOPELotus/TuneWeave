use aes::{
    Aes128,
    cipher::{BlockEncrypt, KeyInit, generic_array::GenericArray},
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use tuneweave_core::{ErrorCode, Platform, Result, TuneWeaveError};

const MAX_MEDIA_BYTES: u64 = 10 * 1024 * 1024 * 1024;
const MAX_SAMPLE_COUNT: usize = 2_000_000;
const MAX_CHUNK_COUNT: usize = 2_000_000;
const MAX_SUBSAMPLES_PER_SAMPLE: usize = 4_096;
const MAX_BOXES_PER_LEVEL: usize = 1_000_000;
const MAX_SPADE_CHARS: usize = 16 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SodaAudioFormat {
    Aac,
    Flac,
    Alac,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DecryptedSodaMedia {
    pub format: SodaAudioFormat,
    pub sample_count: usize,
}

#[derive(Clone, Copy)]
struct BoxHeader {
    offset: usize,
    header_len: usize,
    size: usize,
    kind: [u8; 4],
}

impl BoxHeader {
    fn payload_start(self) -> usize {
        self.offset + self.header_len
    }

    fn end(self) -> usize {
        self.offset + self.size
    }
}

#[derive(Clone, Copy)]
struct SampleToChunk {
    first_chunk: u32,
    samples_per_chunk: u32,
    description_index: u32,
}

struct SampleEncryption {
    iv: [u8; 16],
    subsamples: Vec<SubsampleEncryption>,
}

#[derive(Clone, Copy)]
struct SubsampleEncryption {
    clear_bytes: u16,
    encrypted_bytes: u32,
}

struct EncryptedSampleEntry {
    header: BoxHeader,
    description_index: u32,
    original_format: SodaAudioFormat,
    original_fourcc: [u8; 4],
    per_sample_iv_size: usize,
    key_id: [u8; 16],
}

pub fn decrypt_cenc_audio_in_place(
    media: &mut [u8],
    spade_a: &str,
    expected_key_id: &str,
) -> Result<DecryptedSodaMedia> {
    let media_len = u64::try_from(media.len())
        .map_err(|_| media_error("Soda media is too large for this platform"))?;
    if media.is_empty() || media_len > MAX_MEDIA_BYTES {
        return Err(media_error(
            "Soda media size is outside the supported range",
        ));
    }

    let mut key = decode_spade_key(spade_a)?;
    let cipher = Aes128::new_from_slice(&key)
        .map_err(|_| media_error("Soda media authorization contained an invalid key"))?;
    key.fill(0);

    let top_level = parse_boxes(media, 0, media.len())?;
    let moov = exactly_one(&top_level, b"moov", "Soda media must contain one moov box")?;
    let media_payloads = top_level
        .iter()
        .filter(|header| header.kind == *b"mdat")
        .map(|header| header.payload_start()..header.end())
        .collect::<Vec<_>>();
    if media_payloads.is_empty() {
        return Err(media_error("Soda media omitted its mdat payload"));
    }

    let (stbl, sample_entry) = encrypted_audio_sample_table(media, moov)?;
    if decode_key_id(expected_key_id)? != sample_entry.key_id {
        return Err(media_error(
            "Soda media authorization does not match its encryption metadata",
        ));
    }
    let children = parse_boxes(media, stbl.payload_start(), stbl.end())?;
    let stsz = exactly_one(&children, b"stsz", "Soda media must contain one stsz box")?;
    let stsc = exactly_one(&children, b"stsc", "Soda media must contain one stsc box")?;
    let senc = exactly_one(&children, b"senc", "Soda media must contain one senc box")?;
    let stco = optional_one(
        &children,
        b"stco",
        "Soda media contains duplicate stco boxes",
    )?;
    let co64 = optional_one(
        &children,
        b"co64",
        "Soda media contains duplicate co64 boxes",
    )?;
    if stco.is_some() == co64.is_some() {
        return Err(media_error(
            "Soda media must contain exactly one chunk offset table",
        ));
    }

    let sample_sizes = parse_sample_sizes(media, stsz)?;
    let sample_count = sample_sizes.len();
    let sample_to_chunk = parse_sample_to_chunk(media, stsc)?;
    let chunk_offsets = match (stco, co64) {
        (Some(header), None) => parse_chunk_offsets_32(media, header)?,
        (None, Some(header)) => parse_chunk_offsets_64(media, header)?,
        _ => unreachable!("validated exactly one offset table"),
    };
    let encryption = parse_sample_encryption(
        media,
        senc,
        sample_entry.per_sample_iv_size,
        sample_sizes.len(),
    )?;
    let sample_ranges = map_sample_ranges(
        &sample_sizes,
        &sample_to_chunk,
        &chunk_offsets,
        sample_entry.description_index,
        &media_payloads,
    )?;

    for ((range, encryption), expected_size) in
        sample_ranges.into_iter().zip(encryption).zip(sample_sizes)
    {
        let sample = &mut media[range];
        if sample.len() != usize::try_from(expected_size).unwrap_or(usize::MAX) {
            return Err(media_error(
                "Soda media sample table changed during decoding",
            ));
        }
        decrypt_sample(&cipher, sample, &encryption)?;
    }

    media[sample_entry.header.offset + 4..sample_entry.header.offset + 8]
        .copy_from_slice(&sample_entry.original_fourcc);
    Ok(DecryptedSodaMedia {
        format: sample_entry.original_format,
        sample_count,
    })
}

fn decode_spade_key(value: &str) -> Result<[u8; 16]> {
    if value.is_empty()
        || value.len() > MAX_SPADE_CHARS
        || value.trim() != value
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'/' | b'='))
    {
        return Err(media_error("Soda media authorization is malformed"));
    }
    let mut encoded = STANDARD
        .decode(value)
        .map_err(|_| media_error("Soda media authorization is malformed"))?;
    if encoded.len() < 3 {
        return Err(media_error("Soda media authorization is malformed"));
    }
    let padding_marker = encoded[0] ^ encoded[1] ^ encoded[2];
    let padding_len = padding_marker
        .checked_sub(b'0')
        .map(usize::from)
        .ok_or_else(|| media_error("Soda media authorization is malformed"))?;
    let inner_end = encoded
        .len()
        .checked_sub(padding_len)
        .filter(|end| *end > 1)
        .ok_or_else(|| media_error("Soda media authorization is malformed"))?;
    let mut decoded = Vec::with_capacity(inner_end - 1);
    for index in 0..inner_end - 1 {
        let source = encoded[index + 1];
        let mask = match index {
            0 => 0xfa,
            1 => 0x55,
            _ => encoded[index - 1],
        };
        let offset = i32::try_from(index.count_ones()).unwrap_or(i32::MAX) + 21;
        let value = (i32::from(source ^ mask) - offset).rem_euclid(255);
        decoded.push(u8::try_from(value).unwrap_or_default());
    }
    encoded.fill(0);

    let skip = decoded
        .first()
        .copied()
        .and_then(decode_base36)
        .ok_or_else(|| media_error("Soda media authorization is malformed"))?;
    let decoded_message_len = inner_end
        .checked_sub(2)
        .ok_or_else(|| media_error("Soda media authorization is malformed"))?;
    let end = 1_usize
        .checked_add(decoded_message_len)
        .and_then(|value| value.checked_sub(skip))
        .filter(|end| *end <= decoded.len() && *end > 1)
        .ok_or_else(|| media_error("Soda media authorization is malformed"))?;
    let key_text = std::str::from_utf8(&decoded[1..end])
        .map_err(|_| media_error("Soda media authorization is malformed"))?;
    if key_text.len() != 32 || !key_text.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        decoded.fill(0);
        return Err(media_error(
            "Soda media authorization contained an invalid key",
        ));
    }
    let mut key = [0_u8; 16];
    hex::decode_to_slice(key_text, &mut key)
        .map_err(|_| media_error("Soda media authorization contained an invalid key"))?;
    decoded.fill(0);
    Ok(key)
}

fn decode_base36(value: u8) -> Option<usize> {
    match value {
        b'0'..=b'9' => Some(usize::from(value - b'0')),
        b'a'..=b'z' => Some(usize::from(value - b'a' + 10)),
        _ => None,
    }
}

fn decode_key_id(value: &str) -> Result<[u8; 16]> {
    if value.len() != 32 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(media_error("Soda media key identifier is malformed"));
    }
    let mut key_id = [0_u8; 16];
    hex::decode_to_slice(value, &mut key_id)
        .map_err(|_| media_error("Soda media key identifier is malformed"))?;
    Ok(key_id)
}

fn encrypted_audio_sample_table(
    media: &[u8],
    moov: BoxHeader,
) -> Result<(BoxHeader, EncryptedSampleEntry)> {
    let tracks = parse_boxes(media, moov.payload_start(), moov.end())?
        .into_iter()
        .filter(|header| header.kind == *b"trak")
        .collect::<Vec<_>>();
    let mut selected = None;
    for track in tracks {
        let track_children = parse_boxes(media, track.payload_start(), track.end())?;
        let Some(mdia) = optional_one(
            &track_children,
            b"mdia",
            "Soda media track contains duplicate mdia boxes",
        )?
        else {
            continue;
        };
        let media_children = parse_boxes(media, mdia.payload_start(), mdia.end())?;
        let Some(handler) = optional_one(
            &media_children,
            b"hdlr",
            "Soda media track contains duplicate hdlr boxes",
        )?
        else {
            continue;
        };
        if handler_type(media, handler)? != *b"soun" {
            continue;
        }
        let minf = exactly_one(
            &media_children,
            b"minf",
            "Soda audio track must contain one minf box",
        )?;
        let minf_children = parse_boxes(media, minf.payload_start(), minf.end())?;
        let stbl = exactly_one(
            &minf_children,
            b"stbl",
            "Soda audio track must contain one stbl box",
        )?;
        let stbl_children = parse_boxes(media, stbl.payload_start(), stbl.end())?;
        let stsd = exactly_one(
            &stbl_children,
            b"stsd",
            "Soda audio track must contain one stsd box",
        )?;
        let Some(entry) = encrypted_sample_entry(media, stsd)? else {
            continue;
        };
        if selected.replace((stbl, entry)).is_some() {
            return Err(media_error(
                "Soda media contains more than one encrypted audio track",
            ));
        }
    }
    selected.ok_or_else(|| media_error("Soda media omitted its encrypted audio track"))
}

fn handler_type(media: &[u8], header: BoxHeader) -> Result<[u8; 4]> {
    let payload = payload(media, header)?;
    if payload.len() < 12 {
        return Err(media_error("Soda media contains a truncated hdlr box"));
    }
    Ok(payload[8..12].try_into().expect("fixed four-byte slice"))
}

fn encrypted_sample_entry(media: &[u8], stsd: BoxHeader) -> Result<Option<EncryptedSampleEntry>> {
    let data = payload(media, stsd)?;
    if data.len() < 8 {
        return Err(media_error("Soda media contains a truncated stsd box"));
    }
    let entry_count = bounded_count(read_u32(data, 4)?, MAX_BOXES_PER_LEVEL, "stsd")?;
    let entries = parse_boxes(media, stsd.payload_start() + 8, stsd.end())?;
    if entries.len() != entry_count {
        return Err(media_error("Soda media stsd entry count is inconsistent"));
    }
    let mut encrypted = None;
    for (index, entry) in entries.into_iter().enumerate() {
        if entry.kind != *b"enca" {
            continue;
        }
        if encrypted.is_some() {
            return Err(media_error(
                "Soda media contains duplicate encrypted sample entries",
            ));
        }
        if entry.size < entry.header_len + 28 {
            return Err(media_error(
                "Soda media contains a truncated encrypted sample entry",
            ));
        }
        let children = parse_boxes(media, entry.payload_start() + 28, entry.end())?;
        let sinf = exactly_one(
            &children,
            b"sinf",
            "Soda encrypted sample entry must contain one sinf box",
        )?;
        let protection = parse_boxes(media, sinf.payload_start(), sinf.end())?;
        let frma = exactly_one(
            &protection,
            b"frma",
            "Soda encrypted sample entry must contain one frma box",
        )?;
        let frma_payload = payload(media, frma)?;
        if frma_payload.len() != 4 {
            return Err(media_error("Soda media contains an invalid frma box"));
        }
        let original_fourcc: [u8; 4] = frma_payload.try_into().expect("four-byte format");
        let original_format = match &original_fourcc {
            b"mp4a" => SodaAudioFormat::Aac,
            b"fLaC" => SodaAudioFormat::Flac,
            b"alac" => SodaAudioFormat::Alac,
            _ => return Err(media_error("Soda media uses an unsupported audio codec")),
        };
        let schm = exactly_one(
            &protection,
            b"schm",
            "Soda encrypted sample entry must contain one schm box",
        )?;
        validate_scheme(media, schm)?;
        let schi = exactly_one(
            &protection,
            b"schi",
            "Soda encrypted sample entry must contain one schi box",
        )?;
        let scheme_children = parse_boxes(media, schi.payload_start(), schi.end())?;
        let tenc = exactly_one(
            &scheme_children,
            b"tenc",
            "Soda encrypted sample entry must contain one tenc box",
        )?;
        let (per_sample_iv_size, key_id) = parse_tenc(media, tenc)?;
        encrypted = Some(EncryptedSampleEntry {
            header: entry,
            description_index: u32::try_from(index + 1)
                .map_err(|_| media_error("Soda media has too many sample descriptions"))?,
            original_format,
            original_fourcc,
            per_sample_iv_size,
            key_id,
        });
    }
    Ok(encrypted)
}

fn validate_scheme(media: &[u8], header: BoxHeader) -> Result<()> {
    let data = payload(media, header)?;
    if data.len() < 12
        || data[0] != 0
        || data[1..4] != [0, 0, 0]
        || data[4..8] != *b"cenc"
        || data[8..12] != [0, 1, 0, 0]
    {
        return Err(media_error(
            "Soda media uses an unsupported encryption scheme",
        ));
    }
    Ok(())
}

fn parse_tenc(media: &[u8], header: BoxHeader) -> Result<(usize, [u8; 16])> {
    let data = payload(media, header)?;
    if data.len() != 24 || data[0] != 0 || data[1..4] != [0, 0, 0] || data[6] != 1 {
        return Err(media_error("Soda media contains an unsupported tenc box"));
    }
    let iv_size = usize::from(data[7]);
    if !matches!(iv_size, 8 | 16) || data[8..24].iter().all(|byte| *byte == 0) {
        return Err(media_error("Soda media contains an invalid tenc box"));
    }
    Ok((
        iv_size,
        data[8..24].try_into().expect("sixteen-byte key identifier"),
    ))
}

fn parse_sample_sizes(media: &[u8], header: BoxHeader) -> Result<Vec<u32>> {
    let data = payload(media, header)?;
    if data.len() < 12 {
        return Err(media_error("Soda media contains a truncated stsz box"));
    }
    let fixed_size = read_u32(data, 4)?;
    let count = bounded_count(read_u32(data, 8)?, MAX_SAMPLE_COUNT, "stsz")?;
    if count == 0 {
        return Err(media_error("Soda media contains no audio samples"));
    }
    let expected = if fixed_size == 0 {
        12_usize
            .checked_add(
                count
                    .checked_mul(4)
                    .ok_or_else(|| media_error("Soda media stsz table is too large"))?,
            )
            .ok_or_else(|| media_error("Soda media stsz table is too large"))?
    } else {
        12
    };
    if data.len() != expected {
        return Err(media_error("Soda media stsz table length is inconsistent"));
    }
    if fixed_size != 0 {
        return Ok(vec![fixed_size; count]);
    }
    (0..count)
        .map(|index| {
            let size = read_u32(data, 12 + index * 4)?;
            if size == 0 {
                return Err(media_error("Soda media contains an empty audio sample"));
            }
            Ok(size)
        })
        .collect()
}

fn parse_sample_to_chunk(media: &[u8], header: BoxHeader) -> Result<Vec<SampleToChunk>> {
    let data = payload(media, header)?;
    if data.len() < 8 {
        return Err(media_error("Soda media contains a truncated stsc box"));
    }
    let count = bounded_count(read_u32(data, 4)?, MAX_CHUNK_COUNT, "stsc")?;
    let expected = 8_usize
        .checked_add(
            count
                .checked_mul(12)
                .ok_or_else(|| media_error("Soda media stsc table is too large"))?,
        )
        .ok_or_else(|| media_error("Soda media stsc table is too large"))?;
    if count == 0 || data.len() != expected {
        return Err(media_error("Soda media stsc table length is inconsistent"));
    }
    let mut entries = Vec::with_capacity(count);
    for index in 0..count {
        let offset = 8 + index * 12;
        let entry = SampleToChunk {
            first_chunk: read_u32(data, offset)?,
            samples_per_chunk: read_u32(data, offset + 4)?,
            description_index: read_u32(data, offset + 8)?,
        };
        if entry.first_chunk == 0
            || entry.samples_per_chunk == 0
            || entry.description_index == 0
            || (index == 0 && entry.first_chunk != 1)
            || entries
                .last()
                .is_some_and(|previous: &SampleToChunk| previous.first_chunk >= entry.first_chunk)
        {
            return Err(media_error("Soda media contains an invalid stsc mapping"));
        }
        entries.push(entry);
    }
    Ok(entries)
}

fn parse_chunk_offsets_32(media: &[u8], header: BoxHeader) -> Result<Vec<u64>> {
    parse_chunk_offsets(media, header, 4)
}

fn parse_chunk_offsets_64(media: &[u8], header: BoxHeader) -> Result<Vec<u64>> {
    parse_chunk_offsets(media, header, 8)
}

fn parse_chunk_offsets(media: &[u8], header: BoxHeader, width: usize) -> Result<Vec<u64>> {
    let data = payload(media, header)?;
    if data.len() < 8 {
        return Err(media_error(
            "Soda media contains a truncated chunk offset table",
        ));
    }
    let count = bounded_count(read_u32(data, 4)?, MAX_CHUNK_COUNT, "chunk offset")?;
    let expected = 8_usize
        .checked_add(
            count
                .checked_mul(width)
                .ok_or_else(|| media_error("Soda media chunk table is too large"))?,
        )
        .ok_or_else(|| media_error("Soda media chunk table is too large"))?;
    if count == 0 || data.len() != expected {
        return Err(media_error("Soda media chunk offset table is inconsistent"));
    }
    (0..count)
        .map(|index| {
            if width == 4 {
                read_u32(data, 8 + index * width).map(u64::from)
            } else {
                read_u64(data, 8 + index * width)
            }
        })
        .collect()
}

fn parse_sample_encryption(
    media: &[u8],
    header: BoxHeader,
    iv_size: usize,
    expected_count: usize,
) -> Result<Vec<SampleEncryption>> {
    let data = payload(media, header)?;
    if data.len() < 8 {
        return Err(media_error("Soda media contains a truncated senc box"));
    }
    let version = data[0];
    let flags = u32::from_be_bytes([0, data[1], data[2], data[3]]);
    if version != 0 || flags & !0x02 != 0 {
        return Err(media_error("Soda media contains an unsupported senc box"));
    }
    let count = bounded_count(read_u32(data, 4)?, MAX_SAMPLE_COUNT, "senc")?;
    if count != expected_count {
        return Err(media_error(
            "Soda media sample and encryption counts differ",
        ));
    }
    let has_subsamples = flags & 0x02 != 0;
    let mut cursor = 8_usize;
    let mut samples = Vec::with_capacity(count);
    for _ in 0..count {
        let iv_end = cursor
            .checked_add(iv_size)
            .filter(|end| *end <= data.len())
            .ok_or_else(|| media_error("Soda media contains a truncated sample IV"))?;
        let mut iv = [0_u8; 16];
        iv[..iv_size].copy_from_slice(&data[cursor..iv_end]);
        cursor = iv_end;
        let mut subsamples = Vec::new();
        if has_subsamples {
            let count_end = cursor
                .checked_add(2)
                .filter(|end| *end <= data.len())
                .ok_or_else(|| media_error("Soda media contains a truncated subsample table"))?;
            let subsample_count = usize::from(u16::from_be_bytes(
                data[cursor..count_end].try_into().expect("two-byte count"),
            ));
            cursor = count_end;
            if subsample_count > MAX_SUBSAMPLES_PER_SAMPLE {
                return Err(media_error(
                    "Soda media contains too many encrypted subsamples",
                ));
            }
            subsamples.reserve(subsample_count);
            for _ in 0..subsample_count {
                let end = cursor
                    .checked_add(6)
                    .filter(|end| *end <= data.len())
                    .ok_or_else(|| {
                        media_error("Soda media contains a truncated subsample entry")
                    })?;
                subsamples.push(SubsampleEncryption {
                    clear_bytes: u16::from_be_bytes(
                        data[cursor..cursor + 2]
                            .try_into()
                            .expect("two-byte clear length"),
                    ),
                    encrypted_bytes: u32::from_be_bytes(
                        data[cursor + 2..end]
                            .try_into()
                            .expect("four-byte encrypted length"),
                    ),
                });
                cursor = end;
            }
        }
        samples.push(SampleEncryption { iv, subsamples });
    }
    if cursor != data.len() {
        return Err(media_error("Soda media senc table has trailing data"));
    }
    Ok(samples)
}

fn map_sample_ranges(
    sizes: &[u32],
    mappings: &[SampleToChunk],
    chunk_offsets: &[u64],
    expected_description: u32,
    media_payloads: &[std::ops::Range<usize>],
) -> Result<Vec<std::ops::Range<usize>>> {
    let chunk_count = u32::try_from(chunk_offsets.len())
        .map_err(|_| media_error("Soda media contains too many chunks"))?;
    if mappings
        .iter()
        .any(|mapping| mapping.first_chunk > chunk_count)
    {
        return Err(media_error(
            "Soda media stsc mapping references a missing chunk",
        ));
    }
    let mut ranges = Vec::with_capacity(sizes.len());
    let mut sample_index = 0_usize;
    let mut mapping_index = 0_usize;
    for (chunk_zero_index, chunk_offset) in chunk_offsets.iter().copied().enumerate() {
        let chunk_number = u32::try_from(chunk_zero_index + 1)
            .map_err(|_| media_error("Soda media contains too many chunks"))?;
        while mapping_index + 1 < mappings.len()
            && chunk_number >= mappings[mapping_index + 1].first_chunk
        {
            mapping_index += 1;
        }
        let mapping = mappings[mapping_index];
        if mapping.description_index != expected_description {
            return Err(media_error(
                "Soda media switches to an unsupported sample description",
            ));
        }
        let mut offset = chunk_offset;
        for _ in 0..mapping.samples_per_chunk {
            let size = sizes
                .get(sample_index)
                .copied()
                .ok_or_else(|| media_error("Soda media chunk mapping exceeds its sample table"))?;
            let end = offset
                .checked_add(u64::from(size))
                .ok_or_else(|| media_error("Soda media sample offset overflowed"))?;
            let start = usize::try_from(offset)
                .map_err(|_| media_error("Soda media sample offset is unsupported"))?;
            let end = usize::try_from(end)
                .map_err(|_| media_error("Soda media sample offset is unsupported"))?;
            if !media_payloads
                .iter()
                .any(|payload| start >= payload.start && end <= payload.end)
            {
                return Err(media_error("Soda media sample lies outside mdat"));
            }
            if ranges
                .last()
                .is_some_and(|previous: &std::ops::Range<usize>| previous.end > start)
            {
                return Err(media_error("Soda media sample ranges overlap or regress"));
            }
            ranges.push(start..end);
            offset = u64::try_from(end)
                .map_err(|_| media_error("Soda media sample offset is unsupported"))?;
            sample_index += 1;
        }
    }
    if sample_index != sizes.len() {
        return Err(media_error(
            "Soda media chunk mapping omitted audio samples",
        ));
    }
    Ok(ranges)
}

fn decrypt_sample(cipher: &Aes128, sample: &mut [u8], encryption: &SampleEncryption) -> Result<()> {
    let mut stream = CencCtr::new(cipher, encryption.iv);
    if encryption.subsamples.is_empty() {
        stream.apply(sample);
        return Ok(());
    }
    let mut cursor = 0_usize;
    for subsample in &encryption.subsamples {
        cursor = cursor
            .checked_add(usize::from(subsample.clear_bytes))
            .filter(|cursor| *cursor <= sample.len())
            .ok_or_else(|| media_error("Soda media subsample clear range is invalid"))?;
        let end = cursor
            .checked_add(usize::try_from(subsample.encrypted_bytes).unwrap_or(usize::MAX))
            .filter(|end| *end <= sample.len())
            .ok_or_else(|| media_error("Soda media subsample encrypted range is invalid"))?;
        stream.apply(&mut sample[cursor..end]);
        cursor = end;
    }
    if cursor != sample.len() {
        return Err(media_error(
            "Soda media subsample table does not cover its sample",
        ));
    }
    Ok(())
}

struct CencCtr<'a> {
    cipher: &'a Aes128,
    counter: [u8; 16],
    keystream: [u8; 16],
    consumed: usize,
}

impl<'a> CencCtr<'a> {
    fn new(cipher: &'a Aes128, counter: [u8; 16]) -> Self {
        Self {
            cipher,
            counter,
            keystream: [0; 16],
            consumed: 16,
        }
    }

    fn apply(&mut self, bytes: &mut [u8]) {
        for byte in bytes {
            if self.consumed == self.keystream.len() {
                let mut block = GenericArray::clone_from_slice(&self.counter);
                self.cipher.encrypt_block(&mut block);
                self.keystream.copy_from_slice(&block);
                increment_counter(&mut self.counter);
                self.consumed = 0;
            }
            *byte ^= self.keystream[self.consumed];
            self.consumed += 1;
        }
    }
}

fn increment_counter(counter: &mut [u8; 16]) {
    for byte in counter.iter_mut().rev() {
        let (next, overflow) = byte.overflowing_add(1);
        *byte = next;
        if !overflow {
            break;
        }
    }
}

fn parse_boxes(media: &[u8], start: usize, end: usize) -> Result<Vec<BoxHeader>> {
    if start > end || end > media.len() {
        return Err(media_error("Soda media box boundary is invalid"));
    }
    let mut boxes = Vec::new();
    let mut cursor = start;
    while cursor < end {
        if boxes.len() >= MAX_BOXES_PER_LEVEL || end - cursor < 8 {
            return Err(media_error("Soda media box table is malformed"));
        }
        let short_size = read_u32(media, cursor)?;
        let kind: [u8; 4] = media[cursor + 4..cursor + 8]
            .try_into()
            .expect("four-byte box type");
        let (header_len, size) = match short_size {
            0 => (8, end - cursor),
            1 => {
                if end - cursor < 16 {
                    return Err(media_error("Soda media contains a truncated extended box"));
                }
                let size = usize::try_from(read_u64(media, cursor + 8)?)
                    .map_err(|_| media_error("Soda media box is too large for this platform"))?;
                (16, size)
            }
            value => (
                8,
                usize::try_from(value)
                    .map_err(|_| media_error("Soda media box is too large for this platform"))?,
            ),
        };
        let box_end = cursor
            .checked_add(size)
            .filter(|box_end| size >= header_len && *box_end <= end)
            .ok_or_else(|| media_error("Soda media box size is invalid"))?;
        boxes.push(BoxHeader {
            offset: cursor,
            header_len,
            size,
            kind,
        });
        cursor = box_end;
    }
    Ok(boxes)
}

fn exactly_one(boxes: &[BoxHeader], kind: &[u8; 4], message: &'static str) -> Result<BoxHeader> {
    optional_one(boxes, kind, message)?.ok_or_else(|| media_error(message))
}

fn optional_one(
    boxes: &[BoxHeader],
    kind: &[u8; 4],
    message: &'static str,
) -> Result<Option<BoxHeader>> {
    let mut matching = boxes.iter().copied().filter(|header| header.kind == *kind);
    let first = matching.next();
    if matching.next().is_some() {
        return Err(media_error(message));
    }
    Ok(first)
}

fn payload(media: &[u8], header: BoxHeader) -> Result<&[u8]> {
    media
        .get(header.payload_start()..header.end())
        .ok_or_else(|| media_error("Soda media box payload is out of bounds"))
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32> {
    let end = offset
        .checked_add(4)
        .ok_or_else(|| media_error("Soda media integer offset overflowed"))?;
    let value = bytes
        .get(offset..end)
        .ok_or_else(|| media_error("Soda media contains a truncated integer"))?;
    Ok(u32::from_be_bytes(
        value.try_into().expect("four-byte integer"),
    ))
}

fn read_u64(bytes: &[u8], offset: usize) -> Result<u64> {
    let end = offset
        .checked_add(8)
        .ok_or_else(|| media_error("Soda media integer offset overflowed"))?;
    let value = bytes
        .get(offset..end)
        .ok_or_else(|| media_error("Soda media contains a truncated integer"))?;
    Ok(u64::from_be_bytes(
        value.try_into().expect("eight-byte integer"),
    ))
}

fn bounded_count(value: u32, maximum: usize, table: &'static str) -> Result<usize> {
    let count = usize::try_from(value)
        .map_err(|_| media_error(format!("Soda media {table} count is unsupported")))?;
    if count > maximum {
        return Err(media_error(format!(
            "Soda media {table} count exceeds its bound"
        )));
    }
    Ok(count)
}

fn media_error(message: impl Into<String>) -> TuneWeaveError {
    TuneWeaveError::new(ErrorCode::UpstreamError, message).with_platform(Platform::Soda)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_KEY_HEX: &str = "00112233445566778899aabbccddeeff";
    const TEST_KID_HEX: &str = "42424242424242424242424242424242";

    #[test]
    fn spade_authorization_decodes_only_a_strict_aes_key() {
        let encoded = encode_test_spade(TEST_KEY_HEX);
        assert_eq!(decode_spade_key(&encoded).expect("valid key"), test_key());

        for malformed in ["", " not-base64", "AA==", "Zm9v", "Zm9v\n"] {
            assert!(decode_spade_key(malformed).is_err());
        }
    }

    #[test]
    fn cenc_audio_decrypts_chunk_mapped_samples_in_place() {
        let plaintext = [
            b"first-aac-sample".to_vec(),
            b"second-sample-with-more-data".to_vec(),
            b"third".to_vec(),
        ];
        let ivs = [
            [
                0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0, 0, 0, 0, 0, 0, 0, 0,
            ],
            [
                0x20, 0x21, 0x22, 0x23, 0x24, 0x25, 0x26, 0x27, 0, 0, 0, 0, 0, 0, 0, 0,
            ],
            [
                0x30, 0x31, 0x32, 0x33, 0x34, 0x35, 0x36, 0x37, 0, 0, 0, 0, 0, 0, 0, 0,
            ],
        ];
        let mut encrypted = plaintext.clone();
        let cipher = Aes128::new_from_slice(&test_key()).expect("AES key");
        for (sample, iv) in encrypted.iter_mut().zip(ivs) {
            CencCtr::new(&cipher, iv).apply(sample);
        }
        let (mut media, ranges) = fixture_mp4(&encrypted, &ivs);

        let result =
            decrypt_cenc_audio_in_place(&mut media, &encode_test_spade(TEST_KEY_HEX), TEST_KID_HEX)
                .expect("decrypt media");

        assert_eq!(result.format, SodaAudioFormat::Aac);
        assert_eq!(result.sample_count, plaintext.len());
        for (range, expected) in ranges.into_iter().zip(plaintext) {
            assert_eq!(&media[range], expected);
        }
        let enca = find_fourcc(&media, b"mp4a").expect("patched sample entry");
        assert!(enca > 0);
    }

    #[test]
    fn cenc_audio_rejects_inconsistent_sample_encryption_counts() {
        let plaintext = [b"one".to_vec(), b"two".to_vec(), b"three".to_vec()];
        let ivs = [
            [1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
            [2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
            [3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        ];
        let (mut media, _) = fixture_mp4(&plaintext, &ivs);
        let senc = find_fourcc(&media, b"senc").expect("senc");
        media[senc + 8..senc + 12].copy_from_slice(&2_u32.to_be_bytes());

        let error =
            decrypt_cenc_audio_in_place(&mut media, &encode_test_spade(TEST_KEY_HEX), TEST_KID_HEX)
                .expect_err("count mismatch must fail");
        assert_eq!(error.code, ErrorCode::UpstreamError);
        assert!(!error.message.contains(TEST_KEY_HEX));
    }

    #[test]
    fn cenc_subsamples_advance_the_counter_only_for_encrypted_bytes() {
        let plaintext = (0_u8..53).collect::<Vec<_>>();
        let mut encrypted = plaintext.clone();
        let cipher = Aes128::new_from_slice(&test_key()).expect("AES key");
        let iv = [
            0x51, 0x52, 0x53, 0x54, 0x55, 0x56, 0x57, 0x58, 0, 0, 0, 0, 0, 0, 0, 0,
        ];
        let mut encryptor = CencCtr::new(&cipher, iv);
        encryptor.apply(&mut encrypted[5..16]);
        encryptor.apply(&mut encrypted[19..39]);
        let metadata = SampleEncryption {
            iv,
            subsamples: vec![
                SubsampleEncryption {
                    clear_bytes: 5,
                    encrypted_bytes: 11,
                },
                SubsampleEncryption {
                    clear_bytes: 3,
                    encrypted_bytes: 20,
                },
                SubsampleEncryption {
                    clear_bytes: 14,
                    encrypted_bytes: 0,
                },
            ],
        };

        decrypt_sample(&cipher, &mut encrypted, &metadata).expect("decrypt subsamples");
        assert_eq!(encrypted, plaintext);

        let incomplete = SampleEncryption {
            iv,
            subsamples: vec![SubsampleEncryption {
                clear_bytes: 5,
                encrypted_bytes: 11,
            }],
        };
        assert!(decrypt_sample(&cipher, &mut encrypted, &incomplete).is_err());
    }

    #[test]
    fn cenc_audio_rejects_a_mismatched_key_identifier_before_mutation() {
        let samples = [b"one".to_vec(), b"two".to_vec(), b"three".to_vec()];
        let ivs = [
            [1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
            [2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
            [3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        ];
        let (mut media, _) = fixture_mp4(&samples, &ivs);
        let original = media.clone();

        let error = decrypt_cenc_audio_in_place(
            &mut media,
            &encode_test_spade(TEST_KEY_HEX),
            "11111111111111111111111111111111",
        )
        .expect_err("mismatched KID must fail");
        assert_eq!(error.code, ErrorCode::UpstreamError);
        assert_eq!(media, original);
    }

    #[test]
    #[ignore = "requires a local authorized Soda media fixture"]
    fn live_local_authorized_preview_decrypts_without_persisting_its_key() {
        let root = std::env::var_os("TUNEWEAVE_SODA_MEDIA_FIXTURE_DIR")
            .map(std::path::PathBuf::from)
            .expect("fixture directory");
        let mut media = std::fs::read(root.join("preview-smallest.mp4")).expect("encrypted media");
        let spade_a = std::fs::read_to_string(root.join("spade-a.txt"))
            .expect("authorization")
            .trim()
            .to_owned();
        let key_id = std::fs::read_to_string(root.join("kid.txt"))
            .expect("key identifier")
            .trim()
            .to_owned();

        let result =
            decrypt_cenc_audio_in_place(&mut media, &spade_a, &key_id).expect("decrypt preview");
        assert_eq!(result.format, SodaAudioFormat::Aac);
        assert!(result.sample_count > 1_000);
        std::fs::write(root.join("preview-decrypted.m4a"), media).expect("decrypted fixture");
    }

    fn fixture_mp4(
        samples: &[Vec<u8>; 3],
        ivs: &[[u8; 16]; 3],
    ) -> (Vec<u8>, Vec<std::ops::Range<usize>>) {
        let ftyp = make_box(*b"ftyp", b"isom\0\0\0\0isom");
        let placeholder = build_moov(samples, ivs, [0, 0]);
        let mdat_start = ftyp.len() + placeholder.len();
        let first_offset = mdat_start + 8;
        let padding = 5_usize;
        let second_offset = first_offset + samples[0].len() + padding;
        let moov = build_moov(
            samples,
            ivs,
            [
                u32::try_from(first_offset).expect("small fixture"),
                u32::try_from(second_offset).expect("small fixture"),
            ],
        );
        assert_eq!(placeholder.len(), moov.len());
        let mut mdat_payload = Vec::new();
        mdat_payload.extend_from_slice(&samples[0]);
        mdat_payload.extend_from_slice(&[0xaa; 5]);
        mdat_payload.extend_from_slice(&samples[1]);
        mdat_payload.extend_from_slice(&samples[2]);
        let mdat = make_box(*b"mdat", &mdat_payload);
        let mut media = Vec::new();
        media.extend_from_slice(&ftyp);
        media.extend_from_slice(&moov);
        media.extend_from_slice(&mdat);
        let ranges = vec![
            first_offset..first_offset + samples[0].len(),
            second_offset..second_offset + samples[1].len(),
            second_offset + samples[1].len()..second_offset + samples[1].len() + samples[2].len(),
        ];
        (media, ranges)
    }

    fn build_moov(samples: &[Vec<u8>; 3], ivs: &[[u8; 16]; 3], offsets: [u32; 2]) -> Vec<u8> {
        let frma = make_box(*b"frma", b"mp4a");
        let mut schm_payload = vec![0, 0, 0, 0];
        schm_payload.extend_from_slice(b"cenc");
        schm_payload.extend_from_slice(&[0, 1, 0, 0]);
        let schm = make_box(*b"schm", &schm_payload);
        let mut tenc_payload = vec![0, 0, 0, 0, 0, 0, 1, 8];
        tenc_payload.extend_from_slice(&[0x42; 16]);
        let tenc = make_box(*b"tenc", &tenc_payload);
        let schi = make_box(*b"schi", &tenc);
        let mut sinf_payload = frma;
        sinf_payload.extend_from_slice(&schm);
        sinf_payload.extend_from_slice(&schi);
        let sinf = make_box(*b"sinf", &sinf_payload);
        let mut entry_payload = vec![0; 28];
        entry_payload.extend_from_slice(&sinf);
        let enca = make_box(*b"enca", &entry_payload);
        let mut stsd_payload = vec![0, 0, 0, 0];
        stsd_payload.extend_from_slice(&1_u32.to_be_bytes());
        stsd_payload.extend_from_slice(&enca);
        let stsd = make_box(*b"stsd", &stsd_payload);

        let mut stsc_payload = vec![0, 0, 0, 0];
        stsc_payload.extend_from_slice(&2_u32.to_be_bytes());
        for entry in [[1_u32, 1, 1], [2, 2, 1]] {
            for value in entry {
                stsc_payload.extend_from_slice(&value.to_be_bytes());
            }
        }
        let stsc = make_box(*b"stsc", &stsc_payload);

        let mut stsz_payload = vec![0, 0, 0, 0];
        stsz_payload.extend_from_slice(&0_u32.to_be_bytes());
        stsz_payload.extend_from_slice(&3_u32.to_be_bytes());
        for sample in samples {
            stsz_payload.extend_from_slice(
                &u32::try_from(sample.len())
                    .expect("small sample")
                    .to_be_bytes(),
            );
        }
        let stsz = make_box(*b"stsz", &stsz_payload);

        let mut stco_payload = vec![0, 0, 0, 0];
        stco_payload.extend_from_slice(&2_u32.to_be_bytes());
        for offset in offsets {
            stco_payload.extend_from_slice(&offset.to_be_bytes());
        }
        let stco = make_box(*b"stco", &stco_payload);

        let mut senc_payload = vec![0, 0, 0, 0];
        senc_payload.extend_from_slice(&3_u32.to_be_bytes());
        for iv in ivs {
            senc_payload.extend_from_slice(&iv[..8]);
        }
        let senc = make_box(*b"senc", &senc_payload);

        let mut stbl_payload = Vec::new();
        for child in [stsd, stsc, stsz, stco, senc] {
            stbl_payload.extend_from_slice(&child);
        }
        let stbl = make_box(*b"stbl", &stbl_payload);
        let minf = make_box(*b"minf", &stbl);
        let mut hdlr_payload = vec![0; 8];
        hdlr_payload.extend_from_slice(b"soun");
        hdlr_payload.extend_from_slice(&[0; 12]);
        let hdlr = make_box(*b"hdlr", &hdlr_payload);
        let mut mdia_payload = hdlr;
        mdia_payload.extend_from_slice(&minf);
        let mdia = make_box(*b"mdia", &mdia_payload);
        let trak = make_box(*b"trak", &mdia);
        make_box(*b"moov", &trak)
    }

    fn make_box(kind: [u8; 4], payload: &[u8]) -> Vec<u8> {
        let size = u32::try_from(payload.len() + 8).expect("small fixture");
        let mut bytes = Vec::with_capacity(size as usize);
        bytes.extend_from_slice(&size.to_be_bytes());
        bytes.extend_from_slice(&kind);
        bytes.extend_from_slice(payload);
        bytes
    }

    fn encode_test_spade(hex_key: &str) -> String {
        let mut clear = Vec::with_capacity(hex_key.len() + 1);
        clear.push(b'0');
        clear.extend_from_slice(hex_key.as_bytes());
        let mut inner = Vec::with_capacity(clear.len());
        for (index, desired) in clear.into_iter().enumerate() {
            let mask = match index {
                0 => 0xfa,
                1 => 0x55,
                _ => inner[index - 2],
            };
            let offset = i32::try_from(index.count_ones()).unwrap_or(i32::MAX) + 21;
            let transformed = (i32::from(desired) + offset).rem_euclid(255);
            inner.push(u8::try_from(transformed).unwrap_or_default() ^ mask);
        }
        let first = b'0' ^ inner[0] ^ inner[1];
        let mut encoded = Vec::with_capacity(inner.len() + 1);
        encoded.push(first);
        encoded.extend_from_slice(&inner);
        STANDARD.encode(encoded)
    }

    fn test_key() -> [u8; 16] {
        let mut key = [0_u8; 16];
        hex::decode_to_slice(TEST_KEY_HEX, &mut key).expect("test key");
        key
    }

    fn find_fourcc(media: &[u8], kind: &[u8; 4]) -> Option<usize> {
        media.windows(4).position(|window| window == kind)
    }
}
