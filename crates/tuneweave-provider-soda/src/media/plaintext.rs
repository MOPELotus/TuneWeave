//! Container and frame validation. This does not decode compressed samples to PCM.
use super::*;

mod codec;
mod flac;
mod groups;
#[cfg(test)]
pub(crate) mod tests;

#[track_caller]
fn invalid() -> TuneWeaveError {
    #[cfg(debug_assertions)]
    if std::env::var_os("TUNEWEAVE_SODA_MEDIA_DIAGNOSTICS").is_some() {
        eprintln!(
            "DIAGNOSTIC soda_plaintext_invalid line={}",
            std::panic::Location::caller().line()
        );
    }
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
    let edit_list = parse_audio_edit_list(bytes, track)?;
    let movie_scale = edit_list
        .map(|_| {
            let header = exactly_one(&movie, b"mvhd", "Soda movie requires one mvhd atom")?;
            movie_timescale(bytes, header)
        })
        .transpose()?;
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
    let sample_rate_fixed = read_u32(entry_data, 24)?;
    let sample_rate = sample_rate_fixed >> 16;
    if !(1..=8).contains(&channels) || sample_rate == 0 {
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
        AudioTimingContext {
            track,
            mdhd: child(bytes, mdia, b"mdhd")?,
            stts: exactly_one(
                &table,
                b"stts",
                "Soda plaintext audio requires one stts box",
            )?,
            samples: sizes.len(),
            composition_offsets_present: table.iter().any(|atom| atom.kind == *b"ctts"),
            format,
            sample_rate_fixed,
            edit_list,
            movie_scale,
        },
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

#[derive(Clone, Copy)]
struct AudioEditList {
    segment_duration: u64,
    media_time: u64,
}

fn parse_audio_edit_list(bytes: &[u8], track: BoxHeader) -> Result<Option<AudioEditList>> {
    let track_atoms = parse_boxes(bytes, track.payload_start(), track.end())?;
    let Some(edit) = optional_one(
        &track_atoms,
        b"edts",
        "Soda audio has duplicate edit list atoms",
    )?
    else {
        return Ok(None);
    };
    let edit_atoms = parse_boxes(bytes, edit.payload_start(), edit.end())?;
    if edit_atoms.len() != 1 || edit_atoms[0].kind != *b"elst" {
        return Err(invalid());
    }
    let data = payload(bytes, edit_atoms[0])?;
    if data.len() < 8 || data[1..4] != [0; 3] || read_u32(data, 4)? != 1 {
        return Err(invalid());
    }
    let (segment_duration, media_time, rate_integer, rate_fraction) = match data[0] {
        0 if data.len() == 20 => (
            u64::from(read_u32(data, 8)?),
            i64::from(i32::from_be_bytes(data[12..16].try_into().unwrap())),
            i16::from_be_bytes(data[16..18].try_into().unwrap()),
            i16::from_be_bytes(data[18..20].try_into().unwrap()),
        ),
        1 if data.len() == 28 => (
            read_u64(data, 8)?,
            i64::from_be_bytes(data[16..24].try_into().unwrap()),
            i16::from_be_bytes(data[24..26].try_into().unwrap()),
            i16::from_be_bytes(data[26..28].try_into().unwrap()),
        ),
        _ => return Err(invalid()),
    };
    if segment_duration == 0 || media_time < 0 || rate_integer != 1 || rate_fraction != 0 {
        return Err(invalid());
    }
    Ok(Some(AudioEditList {
        segment_duration,
        media_time: u64::try_from(media_time).map_err(|_| invalid())?,
    }))
}

fn movie_timescale(bytes: &[u8], header: BoxHeader) -> Result<u32> {
    let data = payload(bytes, header)?;
    let scale = match data.first() {
        Some(0) if data.len() >= 20 => read_u32(data, 12)?,
        Some(1) if data.len() >= 32 => read_u32(data, 20)?,
        _ => return Err(invalid()),
    };
    if data[1..4] != [0; 3] || scale == 0 {
        return Err(invalid());
    }
    Ok(scale)
}

#[derive(Clone, Copy)]
struct AudioTimingContext {
    track: BoxHeader,
    mdhd: BoxHeader,
    stts: BoxHeader,
    samples: usize,
    composition_offsets_present: bool,
    format: SodaAudioFormat,
    sample_rate_fixed: u32,
    edit_list: Option<AudioEditList>,
    movie_scale: Option<u32>,
}

fn validate_timing(bytes: &[u8], timing: AudioTimingContext) -> Result<()> {
    let AudioTimingContext {
        track: _track,
        mdhd,
        stts,
        samples,
        composition_offsets_present,
        format,
        sample_rate_fixed,
        edit_list,
        movie_scale,
    } = timing;
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
    let timing_matches = if let Some(edit) = edit_list {
        let movie_scale = u128::from(movie_scale.unwrap_or_default());
        let movie_ticks = u128::from(edit.segment_duration) * u128::from(scale);
        let media_scale = movie_scale;
        let segment_duration = (media_scale != 0)
            .then_some((movie_ticks + media_scale / 2) / media_scale)
            .and_then(|value| u64::try_from(value).ok());
        let sample_rate = u128::from(sample_rate_fixed >> 16);
        let trim_numerator = u128::from(edit.media_time) * sample_rate;
        let media_timescale = u128::from(scale);
        let trim_samples = (media_timescale != 0 && trim_numerator % media_timescale == 0)
            .then_some(trim_numerator / media_timescale);
        let bounded_aac_edit = format == SodaAudioFormat::Aac
            && !composition_offsets_present
            && sample_rate_fixed & 0xffff == 0
            && trim_samples.is_some_and(|samples| (1..=2112).contains(&samples));
        // AAC encoders can express priming in either duration bookkeeping layout:
        // some keep stts and mdhd equal and shorten elst by media_time; others keep
        // elst and mdhd equal while stts includes the leading priming samples.
        let presentation_matches = segment_duration.is_some_and(|segment| {
            (ticks == duration
                && duration
                    .checked_sub(edit.media_time)
                    .is_some_and(|visible| segment.abs_diff(visible) <= 1))
                || (ticks.checked_sub(duration) == Some(edit.media_time)
                    && segment.abs_diff(duration) <= 1)
        });
        if edit.media_time == 0 {
            ticks == duration
                && segment_duration.is_some_and(|segment| segment.abs_diff(duration) <= 1)
        } else {
            bounded_aac_edit && presentation_matches
        }
    } else {
        movie_scale.is_none() && ticks == duration
    };
    if total != samples as u64 || !timing_matches {
        #[cfg(debug_assertions)]
        if std::env::var_os("TUNEWEAVE_SODA_MEDIA_DIAGNOSTICS").is_some() {
            eprintln!(
                "DIAGNOSTIC soda_plaintext_timing_mismatch scale={} sample_entries={} stts_samples={} mdhd_duration={} stts_duration={} delta={} ctts_present={} edit_list={}",
                scale,
                samples,
                total,
                duration,
                ticks,
                ticks.abs_diff(duration),
                composition_offsets_present,
                diagnostic_edit_list(bytes, _track),
            );
        }
        return Err(invalid());
    }
    Ok(())
}

#[cfg(debug_assertions)]
fn diagnostic_edit_list(bytes: &[u8], track: BoxHeader) -> String {
    let Ok(track_atoms) = parse_boxes(bytes, track.payload_start(), track.end()) else {
        return "track_malformed".to_owned();
    };
    let edits = track_atoms
        .iter()
        .filter(|atom| atom.kind == *b"edts")
        .collect::<Vec<_>>();
    if edits.is_empty() {
        return "absent".to_owned();
    }
    if edits.len() != 1 {
        return "ambiguous".to_owned();
    }
    let Ok(edit_atoms) = parse_boxes(bytes, edits[0].payload_start(), edits[0].end()) else {
        return "malformed_edts".to_owned();
    };
    let lists = edit_atoms
        .iter()
        .filter(|atom| atom.kind == *b"elst")
        .collect::<Vec<_>>();
    if lists.len() != 1 {
        return "missing_or_ambiguous_elst".to_owned();
    }
    let Ok(data) = payload(bytes, *lists[0]) else {
        return "malformed_elst".to_owned();
    };
    if data.len() < 8 || data[1..4] != [0; 3] {
        return "invalid_elst_header".to_owned();
    }
    let Ok(count) = read_u32(data, 4) else {
        return "invalid_elst_count".to_owned();
    };
    let first = match data[0] {
        0 if data.len() == 8 + count as usize * 12 => (|| {
            Some((
                u64::from(read_u32(data, 8).ok()?),
                i64::from(i32::from_be_bytes(data[12..16].try_into().ok()?)),
                i32::from(i16::from_be_bytes(data[16..18].try_into().ok()?)),
                i32::from(i16::from_be_bytes(data[18..20].try_into().ok()?)),
            ))
        })(),
        1 if data.len() == 8 + count as usize * 20 => (|| {
            Some((
                read_u64(data, 8).ok()?,
                i64::from_be_bytes(data[16..24].try_into().ok()?),
                i32::from(i16::from_be_bytes(data[24..26].try_into().ok()?)),
                i32::from(i16::from_be_bytes(data[26..28].try_into().ok()?)),
            ))
        })(),
        _ => None,
    };
    let Some((segment_duration, media_time, rate_integer, rate_fraction)) = first else {
        return "invalid_elst_layout".to_owned();
    };
    format!(
        "version={};count={count};first_segment={segment_duration};first_media_time={media_time};first_rate={rate_integer}.{rate_fraction}",
        data[0]
    )
}
