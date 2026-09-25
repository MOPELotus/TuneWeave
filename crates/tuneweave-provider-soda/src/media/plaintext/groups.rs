//! Audio pre-roll groups are not CENC sample encryption groups (`seig`).
use super::*;

pub(super) fn validate(bytes: &[u8], table: &[BoxHeader], samples: usize) -> Result<()> {
    let mut descriptions = [None; 2];
    let mut mappings = [None; 2];
    for item in table {
        if !matches!(&item.kind, b"sgpd" | b"sbgp") {
            continue;
        }
        let data = payload(bytes, *item)?;
        if data.len() < 8 || data[1..4] != [0; 3] {
            return Err(invalid());
        }
        // Unknown groups, including seig, must not change the interpretation of
        // samples without being examined. roll/prol contain signed roll distances.
        let index = match &data[4..8] {
            b"roll" => 0,
            b"prol" => 1,
            _ => return Err(invalid()),
        };
        let slot = if item.kind == *b"sgpd" {
            &mut descriptions[index]
        } else {
            &mut mappings[index]
        };
        if slot.replace(data).is_some() {
            return Err(invalid());
        }
    }
    for (description, mapping) in descriptions.into_iter().zip(mappings) {
        let count = description.map(description_count).transpose()?.unwrap_or(0);
        let Some(data) = mapping else { continue };
        let pos = match data[0] {
            0 => 8,
            1 if read_u32(data, 8)? == 0 => 12,
            _ => return Err(invalid()),
        };
        let rows = bounded_count(read_u32(data, pos)?, MAX_SAMPLE_COUNT, "sbgp")?;
        if data.len() != pos + 4 + rows * 8 {
            return Err(invalid());
        }
        let mut covered = 0_u64;
        for row in data[pos + 4..].chunks_exact(8) {
            let n = read_u32(row, 0)?;
            if n == 0 || read_u32(row, 4)? > count {
                return Err(invalid());
            }
            covered += u64::from(n);
        }
        if covered > samples as u64 {
            return Err(invalid());
        }
    }
    Ok(())
}

fn description_count(data: &[u8]) -> Result<u32> {
    let (length, default, pos) = match data[0] {
        0 => (2, 0, 8),
        1 => (read_u32(data, 8)?, 0, 12),
        2 => (read_u32(data, 8)?, read_u32(data, 12)?, 16),
        _ => return Err(invalid()),
    };
    let count = read_u32(data, pos)?;
    bounded_count(count, MAX_SAMPLE_COUNT, "sgpd")?;
    if !matches!(length, 0 | 2) || default > count {
        return Err(invalid());
    }
    let mut pos = pos + 4;
    for _ in 0..count {
        if length == 0 {
            if read_u32(data, pos)? != 2 {
                return Err(invalid());
            }
            pos += 4;
        }
        data.get(pos..pos + 2).ok_or_else(invalid)?;
        pos += 2;
    }
    if pos != data.len() {
        return Err(invalid());
    }
    Ok(count)
}
