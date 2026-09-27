use super::*;

// ffmpeg lavfi sine=frequency=440:sample_rate=44100:duration=1, stereo,
// -c:a aac -b:a 128k / -c:a alac / -c:a flac, -flags +bitexact -fflags +bitexact.
// All recordings are synthetic. No credentials or platform media are included.
pub(crate) const AAC: &[u8] = include_bytes!("synthetic-aac.m4a");
pub(crate) const ALAC: &[u8] = include_bytes!("synthetic-alac.m4a");
pub(crate) const FLAC: &[u8] = include_bytes!("synthetic-flac.flac");
pub(crate) fn reply(bytes: &[u8]) -> Vec<u8> {
    let mut response = format!("HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nSet-Cookie: sessionid_ss=cdn-must-not-rotate\r\nConnection: close\r\n\r\n", bytes.len()).into_bytes();
    response.extend_from_slice(bytes);
    response
}

#[test]
fn plaintext_media_accepts_complete_synthetic_aac_alac_and_flac_without_changing_bytes() {
    for (bytes, format, count) in [
        (AAC, SodaAudioFormat::Aac, 45),
        (ALAC, SodaAudioFormat::Alac, 11),
        (FLAC, SodaAudioFormat::Flac, 10),
    ] {
        let result = validate(bytes.to_vec(), format).unwrap();
        assert_eq!(result.bytes, bytes);
        assert_eq!(result.format, format);
        assert_eq!(result.sample_count, count);
        for other in [
            SodaAudioFormat::Aac,
            SodaAudioFormat::Alac,
            SodaAudioFormat::Flac,
        ]
        .into_iter()
        .filter(|f| *f != format)
        {
            assert!(validate(bytes.to_vec(), other).is_err());
        }
    }
}

fn locate(bytes: &[u8], path: &[&[u8; 4]]) -> BoxHeader {
    let mut boxes = parse_boxes(bytes, 0, bytes.len()).unwrap();
    for (i, kind) in path.iter().enumerate() {
        let b = exactly_one(&boxes, kind, "fixture child").unwrap();
        if i + 1 == path.len() {
            return b;
        }
        boxes = parse_boxes(bytes, b.payload_start(), b.end()).unwrap();
    }
    panic!("empty fixture path")
}
fn word(bytes: &mut [u8], pos: usize, value: u32) {
    bytes[pos..pos + 4].copy_from_slice(&value.to_be_bytes());
}

fn aac_with_priming_edit(trim_ticks: u32) -> Vec<u8> {
    let mut bytes = AAC.to_vec();
    let mdhd = locate(AAC, &[b"moov", b"trak", b"mdia", b"mdhd"]);
    let mvhd = locate(AAC, &[b"moov", b"mvhd"]);
    let stts = locate(AAC, &[b"moov", b"trak", b"mdia", b"minf", b"stbl", b"stts"]);
    let elst = locate(AAC, &[b"moov", b"trak", b"edts", b"elst"]);
    let media_scale = u128::from(read_u32(&bytes, mdhd.payload_start() + 12).unwrap());
    let movie_scale = u128::from(read_u32(&bytes, mvhd.payload_start() + 12).unwrap());
    let segment_duration = u128::from(read_u32(&bytes, elst.payload_start() + 8).unwrap());
    let visible_duration =
        u64::try_from((segment_duration * media_scale + movie_scale / 2) / movie_scale).unwrap();
    let entries = read_u32(&bytes, stts.payload_start() + 4).unwrap();
    let last_delta = stts.payload_start() + 8 + entries as usize * 8 - 4;
    let delta = read_u32(&bytes, last_delta).unwrap();
    let table = payload(&bytes, stts).unwrap();
    let stts_duration = table[8..]
        .chunks_exact(8)
        .take(entries as usize)
        .map(|row| u64::from(read_u32(row, 0).unwrap()) * u64::from(read_u32(row, 4).unwrap()))
        .sum::<u64>();
    let new_stts_duration = visible_duration + u64::from(trim_ticks);
    word(
        &mut bytes,
        last_delta,
        delta + u32::try_from(new_stts_duration - stts_duration).unwrap(),
    );
    word(
        &mut bytes,
        mdhd.payload_start() + 16,
        u32::try_from(visible_duration).unwrap(),
    );
    word(&mut bytes, elst.payload_start() + 12, trim_ticks);
    bytes
}

#[test]
fn plaintext_media_accepts_aac_with_a_bounded_explicit_priming_edit() {
    let bytes = aac_with_priming_edit(2_048);
    let result = validate(bytes.clone(), SodaAudioFormat::Aac).unwrap();
    assert_eq!(result.bytes, bytes);
    assert_eq!(result.sample_count, 45);

    let mut wrong_trim = aac_with_priming_edit(2_048);
    let edit = locate(&wrong_trim, &[b"moov", b"trak", b"edts", b"elst"]);
    word(&mut wrong_trim, edit.payload_start() + 12, 1_024);
    assert!(validate(wrong_trim, SodaAudioFormat::Aac).is_err());

    let mut invalid_rate = aac_with_priming_edit(2_048);
    let edit = locate(&invalid_rate, &[b"moov", b"trak", b"edts", b"elst"]);
    word(&mut invalid_rate, edit.payload_start() + 16, 0);
    assert!(validate(invalid_rate, SodaAudioFormat::Aac).is_err());

    assert!(validate(aac_with_priming_edit(4_096), SodaAudioFormat::Aac).is_err());

    let mut alac_with_edit = ALAC.to_vec();
    let edit = locate(&alac_with_edit, &[b"moov", b"trak", b"edts", b"elst"]);
    word(&mut alac_with_edit, edit.payload_start() + 12, 1_024);
    assert!(validate(alac_with_edit, SodaAudioFormat::Alac).is_err());
}

#[test]
fn plaintext_media_rejects_invalid_iso_tracks_codec_configs_tables_and_encryption() {
    let table = [b"moov", b"trak", b"mdia", b"minf", b"stbl"];
    let stbl = locate(AAC, &table);
    let children = parse_boxes(AAC, stbl.payload_start(), stbl.end()).unwrap();
    let get = |kind: &[u8; 4]| exactly_one(&children, kind, "fixture box").unwrap();
    for mutation in 0..23 {
        let mut bytes = AAC.to_vec();
        match mutation {
            0 => bytes = b"\0\0\0\x0cftypisom".to_vec(),
            1 => {
                bytes.pop();
            }
            2 => bytes = crate::client::player_info::encrypted_tests::AUDIO.to_vec(),
            3 => bytes[locate(AAC, &[b"moov", b"trak", b"mdia", b"hdlr"]).payload_start() + 8..]
                [..4]
                .copy_from_slice(b"vide"),
            4 => word(&mut bytes, get(b"stsz").payload_start() + 8, 0),
            5 => word(&mut bytes, get(b"stsc").payload_start() + 16, 2),
            6 => word(&mut bytes, get(b"stco").payload_start() + 8, 0),
            7 => word(&mut bytes, get(b"stco").payload_start() + 8, u32::MAX),
            8 => word(&mut bytes, get(b"stsz").payload_start() + 12, u32::MAX),
            9 => word(&mut bytes, get(b"stsd").payload_start() + 4, 2),
            10 => word(&mut bytes, get(b"stts").payload_start() + 12, 0),
            11 => word(
                &mut bytes,
                locate(AAC, &[b"moov", b"trak", b"mdia", b"mdhd"]).payload_start() + 16,
                1,
            ),
            12 => bytes[get(b"stsz").payload_start()] = 1,
            13 => {
                let moov = locate(AAC, &[b"moov"]);
                let track = locate(AAC, &[b"moov", b"trak"]);
                let duplicate = AAC[track.offset..track.end()].to_vec();
                bytes.splice(moov.end()..moov.end(), duplicate.iter().copied());
                word(
                    &mut bytes,
                    moov.offset,
                    (moov.size + duplicate.len()) as u32,
                );
            }
            14 => {
                let dref = locate(AAC, &[b"moov", b"trak", b"mdia", b"minf", b"dinf", b"dref"]);
                let url = parse_boxes(AAC, dref.payload_start() + 8, dref.end()).unwrap()[0];
                word(&mut bytes, url.payload_start(), 0);
            }
            n => {
                let stsd = get(b"stsd");
                let entry = parse_boxes(AAC, stsd.payload_start() + 8, stsd.end()).unwrap()[0];
                let children = parse_boxes(AAC, entry.payload_start() + 28, entry.end()).unwrap();
                let esds = exactly_one(&children, b"esds", "fixture esds").unwrap();
                match n {
                    15 => bytes[entry.offset + 4..entry.offset + 8].copy_from_slice(b"alac"),
                    16 => bytes[entry.offset + 4..entry.offset + 8].copy_from_slice(b"enca"),
                    17 => bytes[esds.offset + 4..esds.offset + 8].copy_from_slice(b"sinf"),
                    18 => bytes[esds.payload_start() + 4] = 0,
                    19 => {
                        // Same mp4a entry with MPEG Layer-3 OTI is not AAC.
                        let offset = AAC[esds.payload_start()..esds.end()]
                            .windows(2)
                            .position(|w| w == [0x40, 0x15])
                            .unwrap();
                        bytes[esds.payload_start() + offset] = 0x69;
                    }
                    20 => bytes[entry.payload_start() + 9] = 1,
                    21 => bytes[entry.payload_start() + 7] = 2,
                    _ => {
                        // MPEG-4 OTI is shared with non-AAC codecs: AOT 21 is TwinVQ.
                        let offset = AAC[esds.payload_start()..esds.end()]
                            .windows(5)
                            .position(|w| w == [0x12, 0x10, 0x56, 0xe5, 0])
                            .unwrap();
                        bytes[esds.payload_start() + offset] = (21 << 3) | 2;
                    }
                }
            }
        }
        assert!(
            validate(bytes, SodaAudioFormat::Aac).is_err(),
            "mutation {mutation}"
        );
    }
    let mut protected = crate::client::player_info::encrypted_tests::AUDIO.to_vec();
    let pos = protected.windows(4).position(|w| w == b"enca").unwrap();
    protected[pos..pos + 4].copy_from_slice(b"mp4a");
    assert!(validate(protected, SodaAudioFormat::Aac).is_err());
}

#[test]
fn plaintext_media_ignores_encryption_words_in_unrelated_metadata() {
    let mut bytes = AAC.to_vec();
    let data = b"enca encv sinf schm schi tenc senc saiz saio";
    bytes.extend_from_slice(&((8 + data.len()) as u32).to_be_bytes());
    bytes.extend_from_slice(b"free");
    bytes.extend_from_slice(data);
    assert_eq!(
        validate(bytes.clone(), SodaAudioFormat::Aac).unwrap().bytes,
        bytes
    );
}

#[test]
fn plaintext_media_accepts_preroll_but_rejects_encryption_or_invalid_sample_groups() {
    let path = [b"moov", b"trak", b"mdia", b"minf", b"stbl"];
    let stbl = locate(AAC, &path);
    let boxes = parse_boxes(AAC, stbl.payload_start(), stbl.end()).unwrap();
    let description = exactly_one(&boxes, b"sgpd", "roll description")
        .unwrap()
        .payload_start();
    let mapping = exactly_one(&boxes, b"sbgp", "roll mapping")
        .unwrap()
        .payload_start();
    let mut prol = AAC.to_vec();
    prol[description + 4..description + 8].copy_from_slice(b"prol");
    prol[mapping + 4..mapping + 8].copy_from_slice(b"prol");
    assert!(validate(prol, SodaAudioFormat::Aac).is_ok());
    for mutation in 0..10 {
        let mut bytes = AAC.to_vec();
        match mutation {
            0 => bytes[description + 4..description + 8].copy_from_slice(b"seig"),
            1 => bytes[mapping + 4..mapping + 8].copy_from_slice(b"seig"),
            2 => bytes[description + 4..description + 8].copy_from_slice(b"zzzz"),
            3 => bytes[description] = 3,
            4 => word(&mut bytes, description + 8, 1),
            5 => word(&mut bytes, description + 12, u32::MAX),
            6 => word(&mut bytes, mapping + 12, 46),
            7 => word(&mut bytes, mapping + 16, 2),
            8 => bytes[mapping + 3] = 1,
            _ => word(&mut bytes, mapping + 12, 0),
        }
        assert!(
            validate(bytes, SodaAudioFormat::Aac).is_err(),
            "mutation {mutation}"
        );
    }
}

#[test]
fn plaintext_media_rejects_invalid_alac_configuration() {
    let stsd = locate(
        ALAC,
        &[b"moov", b"trak", b"mdia", b"minf", b"stbl", b"stsd"],
    );
    let entry = parse_boxes(ALAC, stsd.payload_start() + 8, stsd.end()).unwrap()[0];
    let children = parse_boxes(ALAC, entry.payload_start() + 28, entry.end()).unwrap();
    let config = exactly_one(&children, b"alac", "ALAC config")
        .unwrap()
        .payload_start()
        + 4;
    for mutation in 0..6 {
        let mut bytes = ALAC.to_vec();
        match mutation {
            0 => word(&mut bytes, config, 0),
            1 => bytes[config + 4] = 1,
            2 => bytes[config + 5] = 0,
            3 => bytes[config + 9] = 1,
            4 => word(&mut bytes, config + 20, 0),
            _ => bytes[config - 1] = 1,
        }
        assert!(
            validate(bytes, SodaAudioFormat::Alac).is_err(),
            "mutation {mutation}"
        );
    }
}

#[test]
fn plaintext_media_rejects_truncated_flac_metadata_frames_and_crc_corruption() {
    for mutation in 0..10 {
        let mut bytes = FLAC.to_vec();
        match mutation {
            0 => bytes = b"fLaC".to_vec(),
            1 => {
                bytes.pop();
            }
            2 => bytes[4] = 1,
            3 => bytes[7] = 33,
            4 => bytes[8..12].fill(0),
            5 => bytes[18..26].fill(0),
            6 => bytes[25] ^= 1,
            7 => {
                let end = bytes.len() - 3;
                bytes[end] ^= 64;
            }
            8 => bytes.extend_from_slice(&[0]),
            _ => bytes.truncate(42),
        }
        assert!(
            validate(bytes, SodaAudioFormat::Flac).is_err(),
            "mutation {mutation}"
        );
    }
}
