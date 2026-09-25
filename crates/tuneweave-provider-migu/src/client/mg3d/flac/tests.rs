use super::*;

pub(crate) const STEREO: &[u8] = include_bytes!("tests/synthetic-stereo.flac");
pub(crate) const STEREO_24: &[u8] = include_bytes!("tests/synthetic-stereo-24.flac");
const MONO: &[u8] = include_bytes!("tests/synthetic-mono.flac");
const STEREO_24_96K: &[u8] = include_bytes!("tests/synthetic-stereo-24-96k.flac");
const STEREO_24_88K2: &[u8] = include_bytes!("tests/synthetic-stereo-24-88k2.flac");
const STEREO_24_176K4: &[u8] = include_bytes!("tests/synthetic-stereo-24-176k4.flac");
const STEREO_24_192K: &[u8] = include_bytes!("tests/synthetic-stereo-24-192k.flac");

fn with_streaminfo_rate(bytes: &[u8], rate: u64) -> Vec<u8> {
    let mut changed = bytes.to_vec();
    let packed = u64::from_be_bytes(changed[18..26].try_into().unwrap());
    changed[18..26].copy_from_slice(&((packed & ((1_u64 << 44) - 1)) | (rate << 44)).to_be_bytes());
    changed
}

fn assert_fixed_block_short_tail(bytes: &[u8]) -> (u64, u32) {
    let (info, start) = metadata(bytes, 1000).unwrap();
    let mut cursor = Cursor::new(&bytes[start..]);
    let mut frame_index = 0_u64;
    let mut sample_offset = 0_u64;
    let mut final_duration = 0;
    while cursor.position() < cursor.get_ref().len() as u64 {
        let relative = cursor.position() as usize;
        let header = bytes.get(start + relative..start + relative + 4).unwrap();
        assert_eq!(header[1] & 1, 0, "fixture uses fixed-block strategy");
        let block = FrameReader::new(&mut cursor)
            .read_next_or_eof(Vec::new())
            .unwrap()
            .unwrap();
        let duration = block.duration();
        assert_eq!(
            block.time(),
            u64::from(duration) * frame_index,
            "Claxon reports current block size times encoded frame number"
        );
        assert_eq!(
            sample_offset,
            u64::from(info.max_block) * frame_index,
            "logical fixed-block offset uses the preceding full block size"
        );
        sample_offset += u64::from(duration);
        frame_index += 1;
        final_duration = duration;
        if duration < info.max_block {
            assert_eq!(cursor.position(), cursor.get_ref().len() as u64);
        }
    }
    assert_eq!(sample_offset, info.samples);
    assert!(final_duration < info.max_block);
    (sample_offset, final_duration)
}

#[tokio::test]
async fn mg3d_flac_decodes_complete_independently_encoded_pcm_with_short_final_frame() {
    for bytes in [STEREO, MONO] {
        inspect(bytes, 1000, 16, || Ok(())).await.unwrap();
    }
    assert_eq!(
        hex::encode(&STEREO[26..42]),
        "bd4abb9f846beb89774eeb2d2d8a1707"
    );
    // The oracle is FFmpeg, not this decoder or the MG3D implementation.
    assert_eq!(assert_fixed_block_short_tail(STEREO), (44_100, 2628));
    assert_eq!(assert_fixed_block_short_tail(MONO), (48_000, 1920));
}

#[tokio::test]
async fn mg3d_zq24_flac_decodes_24bit_streaminfo_and_canonical_pcm_md5() {
    inspect(STEREO_24, 1000, 24, || Ok(())).await.unwrap();
    assert_eq!(
        hex::encode(&STEREO_24[26..42]),
        "427c9955d855b94f82ccd651fff04373"
    );
    assert_eq!(assert_fixed_block_short_tail(STEREO_24), (44_100, 2628));
    let (_, start) = metadata(STEREO_24, 1000).unwrap();
    assert_eq!(STEREO_24[start + 3] & 15, 12);
    assert!(inspect(STEREO_24, 1000, 16, || Ok(())).await.is_err());
    assert!(inspect(STEREO, 1000, 24, || Ok(())).await.is_err());
}

#[tokio::test]
async fn mg3d_zq24_96k_decodes_complete_pcm_and_checks_duration_at_the_real_rate() {
    inspect(STEREO_24_96K, 1000, 24, || Ok(())).await.unwrap();
    let (info, start) = metadata(STEREO_24_96K, 1000).unwrap();
    assert_eq!(info.rate, 96_000);
    assert_eq!(info.bits, 24);
    assert_eq!(info.channels, 2);
    assert_eq!(hex::encode(info.md5), "4ab37a36b35fbfdadf32177a0b97e67f");
    assert_eq!(STEREO_24_96K[start + 2] & 15, 11);
    assert_eq!(assert_fixed_block_short_tail(STEREO_24_96K), (96_000, 5888));
    assert!(inspect(STEREO_24_96K, 1000, 16, || Ok(())).await.is_err());
    assert!(inspect(STEREO_24_96K, 4000, 24, || Ok(())).await.is_err());
}

#[tokio::test]
async fn mg3d_zq24_96k_never_accepts_rate_mismatch_corruption_or_other_profiles() {
    let (_, start) = metadata(STEREO_24_96K, 1000).unwrap();
    let mut wrong_header = STEREO_24_96K.to_vec();
    wrong_header[start + 2] = (wrong_header[start + 2] & 0xf0) | 10;
    let mut wrong_md5 = STEREO_24_96K.to_vec();
    wrong_md5[26] ^= 1;
    let mut bad_crc = STEREO_24_96K.to_vec();
    *bad_crc.last_mut().unwrap() ^= 1;
    for bytes in [
        wrong_header,
        wrong_md5,
        bad_crc,
        STEREO_24_96K[..STEREO_24_96K.len() - 1].to_vec(),
    ] {
        assert!(inspect(&bytes, 1000, 24, || Ok(())).await.is_err());
    }
    for rate in [32_000_u64, 64_000, 352_800] {
        let mut bytes = STEREO_24_96K.to_vec();
        let packed = u64::from_be_bytes(bytes[18..26].try_into().unwrap());
        let changed = (packed & ((1_u64 << 44) - 1)) | (rate << 44);
        bytes[18..26].copy_from_slice(&changed.to_be_bytes());
        assert!(metadata(&bytes, 1000).is_err());
    }
    // Even with a matching sample count/duration, SQ is not expanded to 96 kHz.
    let mut sq = STEREO_24_96K.to_vec();
    let packed = u64::from_be_bytes(sq[18..26].try_into().unwrap());
    sq[18..26].copy_from_slice(&((packed & !(31_u64 << 36)) | (15_u64 << 36)).to_be_bytes());
    assert!(metadata(&sq, 1000).is_err());
}

#[tokio::test]
async fn mg3d_zq24_high_rates_decode_complete_frames_pcm_md5_and_real_duration() {
    for (bytes, rate, code, tail, md5) in [
        (
            STEREO_24_88K2,
            88_200,
            1,
            6280,
            "decf6d806143788ba445674b3b8cb872",
        ),
        (
            STEREO_24_176K4,
            176_400,
            2,
            4368,
            "bdc173a1c6137630519f444b37edd859",
        ),
        (
            STEREO_24_192K,
            192_000,
            3,
            3584,
            "48c930ee24dfe31c7a1ec1fd64b8172a",
        ),
    ] {
        inspect(bytes, 1000, 24, || Ok(())).await.unwrap();
        let (info, start) = metadata(bytes, 1000).unwrap();
        assert_eq!((info.rate, info.samples), (rate, rate));
        assert_eq!((info.bits, info.channels, info.max_block), (24, 2, 8192));
        // Expected PCM MD5 and frame durations come from FFmpeg, independently
        // of the production decoder and its frame/header policy.
        assert_eq!(hex::encode(info.md5), md5);
        assert_eq!(bytes[start + 2] & 15, code);
        assert_eq!(bytes[start + 3] & 15, 12);
        assert_eq!(assert_fixed_block_short_tail(bytes), (rate, tail));
        assert!(inspect(bytes, 4000, 24, || Ok(())).await.is_err());
        assert!(inspect(bytes, 1000, 16, || Ok(())).await.is_err());
    }
}

#[tokio::test]
async fn mg3d_zq24_high_rates_reject_mismatched_frames_and_incomplete_pcm() {
    for original in [STEREO_24_88K2, STEREO_24_176K4, STEREO_24_192K] {
        let (info, _) = metadata(original, 1000).unwrap();
        for rate in [88_200, 96_000, 176_400, 192_000] {
            if rate != info.rate {
                // Only STREAMINFO changes; every audio-frame CRC remains valid.
                // An otherwise supported rate must still agree with each frame.
                let changed = with_streaminfo_rate(original, rate);
                assert!(metadata(&changed, 1000).is_ok());
                assert!(inspect(&changed, 1000, 24, || Ok(())).await.is_err());
            }
        }
        let mut wrong_md5 = original.to_vec();
        wrong_md5[26] ^= 1;
        let mut bad_crc = original.to_vec();
        *bad_crc.last_mut().unwrap() ^= 1;
        let mut missing_sample = original.to_vec();
        let packed = u64::from_be_bytes(missing_sample[18..26].try_into().unwrap());
        missing_sample[18..26].copy_from_slice(&(packed + 1).to_be_bytes());
        let mut trailing = original.to_vec();
        trailing.extend_from_slice(b"trailing");
        for bytes in [
            wrong_md5,
            bad_crc,
            missing_sample,
            trailing,
            original[..original.len() - 1].to_vec(),
        ] {
            assert!(inspect(&bytes, 1000, 24, || Ok(())).await.is_err());
        }
        for bits in [8_u64, 16, 20, 32] {
            let mut changed = original.to_vec();
            let packed = u64::from_be_bytes(changed[18..26].try_into().unwrap());
            changed[18..26]
                .copy_from_slice(&((packed & !(31_u64 << 36)) | ((bits - 1) << 36)).to_be_bytes());
            assert!(metadata(&changed, 1000).is_err());
        }
    }
}

#[tokio::test]
async fn mg3d_zq24_high_rates_preserve_session_checks_between_frames() {
    for bytes in [STEREO_24_88K2, STEREO_24_176K4, STEREO_24_192K] {
        let mut calls = 0;
        let result = inspect(bytes, 1000, 24, || {
            calls += 1;
            if calls == 4 {
                Err(error(ErrorCode::Conflict, "replaced"))
            } else {
                Ok(())
            }
        })
        .await;
        assert_eq!(result.unwrap_err().code, ErrorCode::Conflict);
        assert_eq!(calls, 4);
    }
}

#[tokio::test]
async fn mg3d_flac_rejects_truncation_trailing_bytes_crc_md5_and_incomplete_samples() {
    let (_, start) = metadata(STEREO, 1000).unwrap();
    let mut variants = vec![
        STEREO[..STEREO.len() - 1].to_vec(),
        STEREO[..start].to_vec(),
    ];
    let mut trailing = STEREO.to_vec();
    trailing.extend_from_slice(b"trailing");
    variants.push(trailing);
    let mut duplicate = STEREO.to_vec();
    duplicate.extend_from_slice(&STEREO[start..]);
    variants.push(duplicate);
    for at in [26, 41, start + 6, STEREO.len() - 1, 25] {
        let mut bad = STEREO.to_vec();
        bad[at] ^= 1;
        variants.push(bad);
    }
    let mut no_md5 = STEREO.to_vec();
    no_md5[26..42].fill(0);
    variants.push(no_md5);
    let mut wrong_rate = STEREO.to_vec();
    wrong_rate[start + 2] = (wrong_rate[start + 2] & 0xf0) | 10;
    variants.push(wrong_rate);
    let mut wrong_bits = STEREO.to_vec();
    wrong_bits[start + 3] = (wrong_bits[start + 3] & 0xf0) | 12;
    variants.push(wrong_bits);
    let mut wrong_channels = STEREO.to_vec();
    wrong_channels[start + 3] = 8;
    variants.push(wrong_channels);
    for (index, bad) in variants.into_iter().enumerate() {
        assert!(
            inspect(&bad, 1000, 16, || Ok(())).await.is_err(),
            "case {index}"
        );
    }
    for duration in [0, 60000] {
        assert!(inspect(STEREO, duration, 16, || Ok(())).await.is_err());
    }
}

#[tokio::test]
async fn mg3d_flac_checks_session_between_frames_and_returns_no_partial_success() {
    let mut calls = 0;
    let result = inspect(STEREO, 1000, 16, || {
        calls += 1;
        if calls == 4 {
            Err(error(ErrorCode::Conflict, "replaced"))
        } else {
            Ok(())
        }
    })
    .await;
    assert_eq!(result.unwrap_err().code, ErrorCode::Conflict);
    assert_eq!(calls, 4);
}

#[tokio::test]
async fn mg3d_flac_rejects_unbounded_metadata_or_unsupported_streaminfo_before_decoding() {
    for (at, value) in [(8, 0), (10, 255), (18, 0), (20, 0), (21, 255), (4, 127)] {
        let mut bytes = STEREO.to_vec();
        bytes[at] = value;
        assert!(inspect(&bytes, 1000, 16, || Ok(())).await.is_err(), "{at}");
    }
    let mut bytes = STEREO.to_vec();
    bytes[42..46].copy_from_slice(&[0x81, 0xff, 0xff, 0xff]);
    assert!(inspect(&bytes, 1000, 16, || Ok(())).await.is_err());
}
