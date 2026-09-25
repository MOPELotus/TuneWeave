//! Container and frame validation. This does not decode compressed samples to PCM.
use super::*;

mod codec;
mod flac;
mod groups;
#[cfg(test)]
pub(crate) mod tests;

fn invalid() -> TuneWeaveError {
    media_error("Soda plaintext audio contains invalid, encrypted or inconsistent media")
}

pub(crate) fn validate(bytes: Vec<u8>, format: SodaAudioFormat) -> Result<DecryptedSodaAudio> {
    if bytes.is_empty() || bytes.len() as u64 > MAX_MEDIA_BYTES {
        return Err(invalid());
    }
    let (sample_count, container) = if format == SodaAudioFormat::Flac {
        (
            flac::inspect(&bytes).ok_or_else(invalid)?,
            SodaAudioContainer::Flac,
        )
    } else {
        (iso_audio(&bytes, format)?, SodaAudioContainer::IsoBaseMedia)
    };
    Ok(DecryptedSodaAudio {
        bytes,
        format,
        container,
        sample_count,
    })
}

fn child(bytes: &[u8], parent: BoxHeader, kind: &[u8; 4]) -> Result<BoxHeader> {
    exactly_one(
        &parse_boxes(bytes, parent.payload_start(), parent.end())?,
        kind,
        "Soda plaintext audio omitted a unique required container box",
    )
}

fn iso_audio(bytes: &[u8], format: SodaAudioFormat) -> Result<usize> {
    let top = parse_boxes(bytes, 0, bytes.len())?;
    let ftyp = exactly_one(
        &top,
        b"ftyp",
        "Soda plaintext audio must contain one ftyp box",
    )?;
    let file_type = payload(bytes, ftyp)?;
    if top.first().is_none_or(|b| b.kind != *b"ftyp")
        || file_type.len() < 8
        || file_type.len() % 4 != 0
        || top.iter().any(|b| b.kind == *b"moof")
    {
        return Err(invalid());
    }
    let moov = exactly_one(
        &top,
        b"moov",
        "Soda plaintext audio must contain one moov box",
    )?;
    let movie = parse_boxes(bytes, moov.payload_start(), moov.end())?;
    if movie.iter().any(|b| b.kind == *b"mvex") {
        return Err(invalid());
    }
    // An audio delivery must not quietly include an unexamined second/video track.
    let track = exactly_one(
        &movie,
        b"trak",
        "Soda plaintext audio requires one audio track",
    )?;
    let mdia = child(bytes, track, b"mdia")?;
    if handler_type(bytes, child(bytes, mdia, b"hdlr")?)? != *b"soun" {
        return Err(invalid());
    }
    let stbl = child(bytes, child(bytes, mdia, b"minf")?, b"stbl")?;
    let table = parse_boxes(bytes, stbl.payload_start(), stbl.end())?;
    if table
        .iter()
        .any(|b| matches!(&b.kind, b"senc" | b"saiz" | b"saio"))
    {
        return Err(invalid());
    }
    let stsd = exactly_one(
        &table,
        b"stsd",
        "Soda plaintext audio requires one stsd box",
    )?;
    let data = payload(bytes, stsd)?;
    if data.len() < 8 || data[..4] != [0; 4] || read_u32(data, 4)? != 1 {
        return Err(invalid());
    }
    let entries = parse_boxes(bytes, stsd.payload_start() + 8, stsd.end())?;
    if entries.len() != 1 {
        return Err(invalid());
    }
    let entry = entries[0];
    let expected = if format == SodaAudioFormat::Aac {
        *b"mp4a"
    } else {
        *b"alac"
    };
    if entry.kind != expected {
        return Err(invalid());
    }
    let entry_data = payload(bytes, entry)?;
    if entry_data.len() < 28
        || entry_data[..6] != [0; 6]
        || entry_data[6..8] != [0, 1]
        || entry_data[8..16] != [0; 8]
    {
        return Err(invalid());
    }
    let channels = u16::from_be_bytes(entry_data[16..18].try_into().unwrap());
    if !(1..=8).contains(&channels) || read_u32(entry_data, 24)? == 0 {
        return Err(invalid());
    }
    let extra = parse_boxes(bytes, entry.payload_start() + 28, entry.end())?;
    if extra
        .iter()
        .any(|b| matches!(&b.kind, b"sinf" | b"schm" | b"schi" | b"tenc"))
    {
        return Err(invalid());
    }
    let kind = if format == SodaAudioFormat::Aac {
        b"esds"
    } else {
        b"alac"
    };
    let config = exactly_one(
        &extra,
        kind,
        "Soda plaintext audio omitted its unique codec configuration",
    )?;
    let config = payload(bytes, config)?;
    let valid = if format == SodaAudioFormat::Aac {
        codec::aac(config)
    } else {
        codec::alac(config, channels)
    };
    if !valid {
        return Err(invalid());
    }
    // Require an internal data reference; external media references must not be followed.
    let dinf = child(bytes, child(bytes, mdia, b"minf")?, b"dinf")?;
    let dref = child(bytes, dinf, b"dref")?;
    let data = payload(bytes, dref)?;
    if data.len() < 8 || data[..4] != [0; 4] || read_u32(data, 4)? != 1 {
        return Err(invalid());
    }
    let references = parse_boxes(bytes, dref.payload_start() + 8, dref.end())?;
    if references.len() != 1
        || references[0].kind != *b"url "
        || payload(bytes, references[0])? != [0, 0, 0, 1]
    {
        return Err(invalid());
    }
    let stsz = exactly_one(
        &table,
        b"stsz",
        "Soda plaintext audio requires one stsz box",
    )?;
    let stsc = exactly_one(
        &table,
        b"stsc",
        "Soda plaintext audio requires one stsc box",
    )?;
    if payload(bytes, stsz)?.get(..4) != Some(&[0; 4])
        || payload(bytes, stsc)?.get(..4) != Some(&[0; 4])
    {
        return Err(invalid());
    }
    let sizes = parse_sample_sizes(bytes, stsz)?;
    groups::validate(bytes, &table, sizes.len())?;
    let mappings = parse_sample_to_chunk(bytes, stsc)?;
    let offsets = match (
        optional_one(
            &table,
            b"stco",
            "Soda plaintext audio has duplicate offsets",
        )?,
        optional_one(
            &table,
            b"co64",
            "Soda plaintext audio has duplicate offsets",
        )?,
    ) {
        (Some(b), None) | (None, Some(b)) => {
            if payload(bytes, b)?.get(..4) != Some(&[0; 4]) {
                return Err(invalid());
            }
            if b.kind == *b"stco" {
                parse_chunk_offsets_32(bytes, b)?
            } else {
                parse_chunk_offsets_64(bytes, b)?
            }
        }
        _ => return Err(invalid()),
    };
    let payloads = top
        .iter()
        .filter(|b| b.kind == *b"mdat")
        .map(|b| b.payload_start()..b.end())
        .collect::<Vec<_>>();
    let ranges = map_sample_ranges(&sizes, &mappings, &offsets, 1, &payloads)?;
    validate_timing(
        bytes,
        child(bytes, mdia, b"mdhd")?,
        exactly_one(
            &table,
            b"stts",
            "Soda plaintext audio requires one stts box",
        )?,
        sizes.len(),
    )?;
    // Every chunk is inside mdat and exactly accounted for; no hidden extra payload.
    if payloads.is_empty()
        || payloads.iter().any(|p| p.is_empty())
        || ranges.iter().map(|r| r.len()).sum::<usize>()
            != payloads.iter().map(|r| r.len()).sum::<usize>()
    {
        return Err(invalid());
    }
    Ok(sizes.len())
}

fn validate_timing(bytes: &[u8], mdhd: BoxHeader, stts: BoxHeader, samples: usize) -> Result<()> {
    let data = payload(bytes, mdhd)?;
    let (scale, duration) = match data.first() {
        Some(0) if data.len() == 24 => (read_u32(data, 12)?, u64::from(read_u32(data, 16)?)),
        Some(1) if data.len() == 36 => (read_u32(data, 20)?, read_u64(data, 24)?),
        _ => return Err(invalid()),
    };
    if data[1..4] != [0; 3] || scale == 0 || duration == 0 {
        return Err(invalid());
    }
    let data = payload(bytes, stts)?;
    if data.len() < 8 || data[..4] != [0; 4] {
        return Err(invalid());
    }
    let count = bounded_count(read_u32(data, 4)?, MAX_SAMPLE_COUNT, "stts")?;
    if count == 0 || data.len() != 8 + count * 8 {
        return Err(invalid());
    }
    let (mut total, mut ticks) = (0_u64, 0_u64);
    for row in data[8..].chunks_exact(8) {
        let n = u64::from(read_u32(row, 0)?);
        let delta = u64::from(read_u32(row, 4)?);
        if n == 0 || delta == 0 {
            return Err(invalid());
        }
        total = total.checked_add(n).ok_or_else(invalid)?;
        ticks = ticks
            .checked_add(n.checked_mul(delta).ok_or_else(invalid)?)
            .ok_or_else(invalid)?;
    }
    if total != samples as u64 || ticks != duration {
        return Err(invalid());
    }
    Ok(())
}
