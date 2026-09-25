use super::*;

#[test]
fn mg3d_key_derivation_matches_official_native_vectors() {
    for (key, expected) in [
        (
            b"0123456789abcdef0123456789abcdef",
            b"CB4E917FFEB2B4A056445F4B3544495E",
        ),
        (
            b"FEDCBA9876543210FEDCBA9876543210",
            b"A21150B4815EE9DEF0586D838797F8AF",
        ),
        (
            b"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            b"2A63FBA9E2FFE8CC816D852EA10B0A4E",
        ),
    ] {
        assert_eq!(&derive(key), expected);
    }
}

#[test]
fn mg3d_chunk_offsets_match_native_subtraction_and_preserve_case() {
    // Native mg_stream_encode and mg_stream_decode_start independently produced
    // these derived bytes; fixtures contain no service media or account key.
    let key = *b"CB4E917FFEB2B4A056445F4B3544495E";
    let mut native = hex::decode("45444437493e373e4e4f4f4d3e4f425040464847484a5c4b5a4c4f4f5051575465646457695e575e6e6f6f6d5e6f627060666867686a7c6b7a6c6f6f7071777485").unwrap();
    decode(&mut native, &key, 31);
    assert_eq!(native, (0..65).collect::<Vec<u8>>());
    for offset in [0, 1, 15, 31, 32, 33, 4096] {
        for size in [1, 15, 16, 31, 32, 33, 63, 64, 65, 257, 4096] {
            let expected: Vec<u8> = (0..size)
                .map(|i| (i as u8).wrapping_mul(17).wrapping_add(3))
                .collect();
            let mut encoded: Vec<u8> = expected
                .iter()
                .enumerate()
                .map(|(i, v)| v.wrapping_add(key[(offset + i) % 32]))
                .collect();
            for (i, chunk) in encoded.chunks_mut(23).enumerate() {
                decode(chunk, &key, offset + i * 23);
            }
            assert_eq!(encoded, expected);
        }
    }
    assert_ne!(
        derive(b"abcdefabcdefabcdefabcdefabcdefab"),
        derive(b"ABCDEFABCDEFABCDEFABCDEFABCDEFAB")
    );
}

#[test]
fn mg3d_mp3_requires_complete_frames_consistent_stream_and_duration() {
    let mut frame = vec![0; 417];
    frame[..4].copy_from_slice(&[0xff, 0xfb, 0x90, 0]);
    let bytes = frame.repeat(40);
    inspect_mp3(&bytes, 1044, || Ok(())).unwrap();
    let mut tagged = b"ID3\x04\0\0\0\0\0\0".to_vec();
    tagged.extend_from_slice(&bytes);
    inspect_mp3(&tagged, 1044, || Ok(())).unwrap();
    for bad in [
        vec![],
        bytes[..bytes.len() - 1].to_vec(),
        b"ID3\x04\0\0\x80\0\0\0".to_vec(),
        b"fLaC".to_vec(),
        frame,
    ] {
        assert!(inspect_mp3(&bad, 1044, || Ok(())).is_err());
    }
    let mut changed = bytes.clone();
    changed[417 + 2] = 0x94;
    assert!(inspect_mp3(&changed, 1044, || Ok(())).is_err());
    assert!(inspect_mp3(&bytes, 60_000, || Ok(())).is_err());
    assert!(inspect_mp3(&bytes, 0, || Ok(())).is_err());
    assert_eq!(
        inspect_mp3(&bytes, 1044, || Err(error(ErrorCode::Conflict, "replaced")))
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
}
