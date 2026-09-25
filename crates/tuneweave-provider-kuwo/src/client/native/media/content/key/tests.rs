use super::*;
use serde_json::Value;
fn hex(value: &str) -> Vec<u8> {
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|b| u8::from_str_radix(std::str::from_utf8(b).unwrap(), 16).unwrap())
        .collect()
}
#[test]
fn native_tea_and_key_envelopes_match_independent_vectors() {
    let vectors: Value = serde_json::from_str(include_str!("vectors.json")).unwrap();
    for v in vectors["vectors"].as_array().unwrap() {
        if v["kind"] == "tea2" {
            let key: [u8; 16] = hex(v["key_hex"].as_str().unwrap()).try_into().unwrap();
            let plain = tea2(&hex(v["cipher_hex"].as_str().unwrap()), &key).unwrap();
            assert_eq!(plain, hex(v["input_hex"].as_str().unwrap()));
        } else {
            let result = Key::inner(v["encoded"].as_str().unwrap());
            if v["length"] == 301 {
                assert_eq!(result.unwrap_err().code, ErrorCode::CapabilityNotSupported);
            } else {
                assert_eq!(result.unwrap().0, hex(v["key_hex"].as_str().unwrap()));
            }
        }
    }
    for v in vectors["invalid_vectors"].as_array().unwrap() {
        let key = std::array::from_fn(|i| i as u8);
        assert!(tea2(&hex(v["cipher_hex"].as_str().unwrap()), &key).is_err());
    }
    for value in ["", "abc", "AAAA", "not base64"] {
        assert!(Key::inner(value).is_err());
    }
    assert!(Key::inner(&"A".repeat(8193)).is_err());
    assert_eq!(format!("{:?}", Key(vec![42; 128])), "KuwoMediaKey { .. }");
}
#[test]
fn native_content_offsets_segments_zero_keys_and_chunking_match_independent_vectors() {
    let vectors: Value = serde_json::from_str(include_str!("content-vectors.json")).unwrap();
    for v in vectors["vectors"].as_array().unwrap() {
        let n = v["key_length"].as_u64().unwrap() as usize;
        // This length cannot be delivered as a full track (tested above).
        if n == 301 {
            continue;
        }
        let key = Key((0..n)
            .map(|i| {
                if v["key_formula"] == "zero" {
                    0
                } else {
                    ((i * 73 + 19) % 255 + 1) as u8
                }
            })
            .collect());
        let offset = v["offset"].as_u64().unwrap();
        let original: Vec<_> = (0..v["length"].as_u64().unwrap())
            .map(|i| (i * 29 + 11) as u8)
            .collect();
        let mut data = original.clone();
        key.transform(offset, &mut data).unwrap();
        assert_eq!(
            data,
            hex(v["actual_hex"].as_str().unwrap()),
            "n={n}, offset={offset}"
        );
        for (i, chunk) in data.chunks_mut(73).enumerate() {
            key.transform(offset + i as u64 * 73, chunk).unwrap();
        }
        assert_eq!(data, original);
        assert!(key.transform(u64::MAX, &mut [0; 2]).is_err());
    }
}
