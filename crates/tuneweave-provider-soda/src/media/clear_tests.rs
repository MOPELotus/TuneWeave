use super::*;
use sha1::{Digest, Sha1};

const SPADE: &str = "nL8T+Vu+EvdZvhL1X7kR90G6DvVfpSbcXaUm3luiJdyloA==";
const KID: &str = "42424242424242424242424242424242";
const AAC: &[u8] = include_bytes!("../client/player_info/synthetic-aac-cenc.mp4");

#[test]
fn cenc_output_is_clear_audio_with_independently_verified_packets_for_all_codecs() {
    // Self-generated sine waves, encrypted by FFmpeg using synthetic key
    // 00112233445566778899aabbccddeeff and KID 42*16. ALAC/FLAC: 440 Hz,
    // 44100 Hz, 0.25 s, bitexact. Digests come from independently encoded clear
    // files via ffmpeg -map 0:a -c copy -f data, not this decoder.
    for (codec, fixture, expected_bytes, expected_sha1) in [
        (
            SodaAudioFormat::Aac,
            AAC,
            14958,
            "ac001d65881454c4f6eebd3b43dd20440d50f441",
        ),
        (
            SodaAudioFormat::Alac,
            include_bytes!("synthetic-alac-cenc.mp4").as_slice(),
            4469,
            "53c536e1a88110324864461ee1af469f37de6566",
        ),
        (
            SodaAudioFormat::Flac,
            include_bytes!("synthetic-flac-cenc.mp4").as_slice(),
            3146,
            "043d660199817bfa11ecdebf07f6903617740194",
        ),
    ] {
        let mut in_place = fixture.to_vec();
        let pointer = in_place.as_ptr();
        decrypt_cenc_audio_in_place(&mut in_place, SPADE, KID).unwrap();
        assert_eq!(in_place.len(), fixture.len());
        assert_eq!(in_place.as_ptr(), pointer);
        let boxes = parse_boxes(&in_place, 0, in_place.len()).unwrap();
        let mdat = exactly_one(&boxes, b"mdat", "test mdat").unwrap();
        let packets = payload(&in_place, mdat).unwrap();
        assert_eq!(packets.len(), expected_bytes);
        assert_eq!(hex::encode(Sha1::digest(packets)), expected_sha1);
        for marker in [b"sinf", b"senc", b"saiz", b"saio", b"tenc"] {
            assert!(!in_place.windows(4).any(|v| v == marker));
        }
        let output = decrypt_cenc_audio(fixture.to_vec(), SPADE, KID).unwrap();
        assert_eq!(output.format, codec);
        // Production's independently implemented clear-container/frame parser
        // must accept what the CENC delivery API calls clear audio.
        let checked = plaintext::validate(output.bytes, codec).unwrap();
        if codec != SodaAudioFormat::Flac {
            assert_eq!(checked.bytes, in_place);
        }
        if let Some(root) = std::env::var_os("TUNEWEAVE_SODA_SYNTHETIC_OUTPUT_DIR") {
            let name = match codec {
                SodaAudioFormat::Aac => "clear-aac.m4a",
                SodaAudioFormat::Alac => "clear-alac.m4a",
                SodaAudioFormat::Flac => "clear-flac.flac",
            };
            std::fs::write(std::path::PathBuf::from(root).join(name), checked.bytes).unwrap();
        }
    }
}

#[test]
fn clear_delivery_preserves_preroll_and_rejects_key_override_groups_before_mutation() {
    let mut bytes = AAC.to_vec();
    let index = bytes.windows(4).position(|v| v == b"sgpd").unwrap();
    assert_eq!(&bytes[index + 8..index + 12], b"roll");
    bytes[index + 8..index + 12].copy_from_slice(b"seig");
    let original = bytes.clone();
    assert!(decrypt_cenc_audio_in_place(&mut bytes, SPADE, KID).is_err());
    assert_eq!(bytes, original);

    let mut bytes = AAC.to_vec();
    let offset = bytes.len();
    // Extended-size protection init data must keep its complete header/offset.
    bytes.extend_from_slice(&1_u32.to_be_bytes());
    bytes.extend_from_slice(b"pssh");
    bytes.extend_from_slice(&32_u64.to_be_bytes());
    bytes.extend_from_slice(&[42; 16]);
    decrypt_cenc_audio_in_place(&mut bytes, SPADE, KID).unwrap();
    assert_eq!(&bytes[offset + 4..offset + 8], b"free");
    assert_eq!(&bytes[offset + 8..offset + 16], &32_u64.to_be_bytes());
    assert_eq!(&bytes[offset + 16..], &[0; 16]);
    assert_eq!(&bytes[index + 8..index + 12], b"roll");
    assert!(plaintext::validate(bytes, SodaAudioFormat::Aac).is_ok());
}
