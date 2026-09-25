use super::*;
use serde_json::Value;

pub(crate) fn sample() -> Value {
    serde_json::from_str(include_str!("audio-vector.json")).unwrap()
}
fn bytes(value: &Value, name: &str) -> Vec<u8> {
    base64::engine::general_purpose::STANDARD
        .decode(value[name].as_str().unwrap())
        .unwrap()
}
fn control() -> Control {
    Control {
        cancelled: AtomicBool::new(false),
        deadline: std::time::Instant::now() + Duration::from_secs(60),
    }
}
#[test]
fn sing_along_limiter_matches_native_peaks_and_retains_every_short_tail() {
    let fixture: Value = serde_json::from_str(include_str!("preset-vectors.json")).unwrap();
    for row in fixture["vectors"].as_array().unwrap() {
        let input = bytes(row, "input_i16le_base64");
        let expected = bytes(row, "output_f32le_base64");
        let frames = row["frames"].as_u64().unwrap() as usize;
        let mut mixer = mix::Mixer::new();
        let mut actual = Vec::new();
        let total = (frames + mix::LATENCY_FRAMES).div_ceil(mix::BLOCK_FRAMES) * mix::BLOCK_FRAMES;
        for frame in 0..total {
            let samples = std::array::from_fn::<_, 4, _>(|channel| {
                input
                    .get(frame * 8 + channel * 2..frame * 8 + channel * 2 + 2)
                    .map(|b| i16::from_le_bytes(b.try_into().unwrap()))
                    .unwrap_or(0)
            });
            let mixed = mixer.push(&samples);
            if frame >= mix::LATENCY_FRAMES && frame - mix::LATENCY_FRAMES < frames {
                actual.extend(mixed);
            }
        }
        assert_eq!(actual.len(), frames * 2, "{}", row["label"]);
        assert_eq!(expected.len(), actual.len() * 4);
        for (actual, expected) in actual.iter().zip(expected.chunks_exact(4)) {
            let expected = f32::from_le_bytes(expected.try_into().unwrap());
            assert!(
                (*actual - expected).abs() <= 1e-6,
                "{}: {actual} != {expected}",
                row["label"]
            );
            assert!(actual.is_finite() && actual.abs() <= 1.);
        }
    }
}
#[test]
fn sing_along_decodes_owned_vorbis_to_native_verified_stereo_wave() {
    let fixture = sample();
    let source = bytes(&fixture, "plain_base64");
    let wave = convert(&source, &control()).unwrap();
    let expected = bytes(&fixture, "official_stereo_f32le_base64");
    assert_eq!(wave.len(), 44 + 11_025 * 4);
    assert_eq!(&wave[..4], b"RIFF");
    assert_eq!(&wave[8..16], b"WAVEfmt ");
    assert_eq!(u16::from_le_bytes(wave[22..24].try_into().unwrap()), 2);
    assert_eq!(u32::from_le_bytes(wave[24..28].try_into().unwrap()), 44_100);
    assert_eq!(u16::from_le_bytes(wave[34..36].try_into().unwrap()), 16);
    for (actual, expected) in wave[44..].chunks_exact(2).zip(expected.chunks_exact(4)) {
        let actual = i16::from_le_bytes(actual.try_into().unwrap());
        let native = f32::from_le_bytes(expected.try_into().unwrap());
        let expected = (native * 32_768.).round().clamp(-32_768., 32_767.) as i16;
        assert!(
            (i32::from(actual) - i32::from(expected)).abs() <= 4,
            "independent native Vorbis and mixing vector: {actual} != {expected}"
        );
    }
}
fn page_starts(bytes: &[u8]) -> Vec<usize> {
    let mut starts = Vec::new();
    let mut pos = 0;
    while pos < bytes.len() {
        starts.push(pos);
        let count = usize::from(bytes[pos + 26]);
        pos += 27
            + count
            + bytes[pos + 27..pos + 27 + count]
                .iter()
                .map(|&n| usize::from(n))
                .sum::<usize>();
    }
    starts
}
fn checksum(bytes: &mut [u8], start: usize) {
    let count = usize::from(bytes[start + 26]);
    let size = 27
        + count
        + bytes[start + 27..start + 27 + count]
            .iter()
            .map(|&n| usize::from(n))
            .sum::<usize>();
    let value = ogg::crc(&bytes[start..start + size]);
    bytes[start + 22..start + 26].copy_from_slice(&value.to_le_bytes());
}
#[test]
fn sing_along_rejects_incomplete_or_mismatched_ogg_before_delivery() {
    let original = bytes(&sample(), "plain_base64");
    let starts = page_starts(&original);
    let last = *starts.last().unwrap();
    let payload = 27 + usize::from(original[26]);
    let mut bad = vec![original[..original.len() - 1].to_vec()];
    let mut corrupt = original.clone();
    corrupt[original.len() - 1] ^= 1;
    bad.push(corrupt);
    for channel in [1, 2, 6] {
        let mut value = original.clone();
        value[payload + 11] = channel;
        checksum(&mut value, 0);
        bad.push(value);
    }
    let mut rate = original.clone();
    rate[payload + 12..payload + 16].copy_from_slice(&48_000_u32.to_le_bytes());
    checksum(&mut rate, 0);
    bad.push(rate);
    let mut no_end = original.clone();
    no_end[last + 5] &= !4;
    checksum(&mut no_end, last);
    bad.push(no_end);
    let mut oversized = original.clone();
    oversized[last + 6..last + 14].copy_from_slice(&(MAX_FRAMES + 1).to_le_bytes());
    checksum(&mut oversized, last);
    bad.push(oversized);
    let mut invented_duration = original.clone();
    invented_duration[last + 6..last + 14].copy_from_slice(&80_000_u64.to_le_bytes());
    checksum(&mut invented_duration, last);
    bad.push(invented_duration);
    let mut wrong_serial = original.clone();
    wrong_serial[last + 14] ^= 1;
    checksum(&mut wrong_serial, last);
    bad.push(wrong_serial);
    let mut chained = original.clone();
    chained.extend(&original);
    bad.push(chained);
    for value in bad {
        assert!(convert(&value, &control()).is_err());
    }
}
#[test]
fn sing_along_respects_conversion_cancellation_before_decode() {
    let guard = control();
    guard.cancelled.store(true, Ordering::Relaxed);
    assert_eq!(
        convert(&bytes(&sample(), "plain_base64"), &guard)
            .unwrap_err()
            .code,
        ErrorCode::UpstreamTimeout
    );
}
