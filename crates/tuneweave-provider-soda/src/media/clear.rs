//! Preserve chunk offsets while retiring metadata for the samples just decrypted.
use super::*;

pub(super) fn protection_boxes(
    bytes: &[u8],
    top: &[BoxHeader],
    moov: BoxHeader,
    table: &[BoxHeader],
    entry: &EncryptedSampleEntry,
) -> Result<Vec<BoxHeader>> {
    let movie = parse_boxes(bytes, moov.payload_start(), moov.end())?;
    if top.iter().any(|b| b.kind == *b"moof")
        || movie.iter().any(|b| b.kind == *b"mvex")
        || movie.iter().filter(|b| b.kind == *b"trak").count() != 1
    {
        return Err(media_error(
            "Soda clear audio requires one nonfragmented audio track",
        ));
    }
    let mut retired = vec![entry.protection];
    for item in table {
        match &item.kind {
            b"senc" => retired.push(*item),
            b"saiz" | b"saio" => {
                let data = payload(bytes, *item)?;
                if data.len() < 4
                    || data[1..3] != [0, 0]
                    || data[3] > 1
                    || (data[3] == 1
                        && (data.get(4..8) != Some(b"cenc") || data.get(8..12) != Some(&[0; 4])))
                {
                    return Err(media_error(
                        "Soda media contains unsupported auxiliary protection metadata",
                    ));
                }
                retired.push(*item);
            }
            // roll/prol affect decoder pre-roll and must survive. seig (and
            // unknown groups) can override the encryption key/clear state;
            // this decoder currently supports the single tenc key only.
            b"sgpd" | b"sbgp"
                if !matches!(payload(bytes, *item)?.get(4..8), Some(b"roll" | b"prol")) =>
            {
                return Err(media_error(
                    "Soda media uses unsupported sample encryption groups",
                ));
            }
            _ => {}
        }
    }
    // A single-track clear result no longer needs initialization data that can
    // trigger an encrypted-media event in downstream players.
    retired.extend(
        top.iter()
            .chain(&movie)
            .filter(|b| b.kind == *b"pssh")
            .copied(),
    );
    Ok(retired)
}

pub(super) fn retire_protection_boxes(bytes: &mut [u8], boxes: &[BoxHeader]) {
    for item in boxes {
        // A same-sized free box keeps every stco/co64 sample offset intact, for
        // moov-before-mdat and moov-after-mdat files and extended box headers.
        bytes[item.offset + 4..item.offset + 8].copy_from_slice(b"free");
        bytes[item.payload_start()..item.end()].fill(0);
    }
}
