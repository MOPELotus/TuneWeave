use super::*;

#[test]
fn native_media_codec_matches_independent_arm64_vectors() {
    let vectors: serde_json::Value = serde_json::from_str(include_str!("vectors.json")).unwrap();
    for row in vectors["digest"].as_array().unwrap() {
        assert_eq!(
            digest(&unhex(row["data_hex"].as_str().unwrap())),
            row["digest"].as_str().unwrap()
        );
    }
    for row in vectors["cipher"].as_array().unwrap() {
        assert_eq!(
            encrypt(
                &unhex(row["input_hex"].as_str().unwrap()),
                &unhex(row["key_material_hex"].as_str().unwrap())
            ),
            row["actual"].as_str().unwrap()
        );
    }
    for row in vectors["queries"].as_array().unwrap() {
        let values = row["values"]
            .as_object()
            .unwrap()
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str().unwrap().to_owned()))
            .collect();
        assert_eq!(seal(values).unwrap(), row["q"].as_str().unwrap());
    }
    assert_eq!(percent(b"a +~!$'*-._/=&"), "a%20%2B%7E!$'*-._%2F%3D%26");
    assert_eq!(percent("音乐".as_bytes()), "%E9%9F%B3%E4%B9%90");
}
#[test]
fn native_media_codec_bounds_before_encryption_and_rejects_external_signature() {
    assert!(seal(BTreeMap::from([("kwsign", "injected".into())])).is_err());
    assert!(seal(BTreeMap::from([("sid", "x".repeat(MAX_PLAIN))])).is_err());
    assert!(seal(BTreeMap::from([("sid", "/".repeat(MAX_PLAIN / 2))])).is_err());
}
fn unhex(value: &str) -> Vec<u8> {
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|b| u8::from_str_radix(std::str::from_utf8(b).unwrap(), 16).unwrap())
        .collect()
}
