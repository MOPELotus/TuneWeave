use super::*;

pub(crate) fn vectors() -> serde_json::Value {
    serde_json::from_str(include_str!("test_vectors.json")).unwrap()
}
pub(crate) fn key_json(index: usize) -> serde_json::Value {
    let all = vectors();
    let v = &all["vectors"][index];
    serde_json::json!({"status":2000,"result":{"modulus":v["modulus"],"publicExponent":v["exponent"]}})
}
pub(crate) fn decrypt(index: usize, ciphertext: &str) -> Vec<u8> {
    let all = vectors();
    let v = &all["vectors"][index];
    let n = BigUint::parse_bytes(v["modulus"].as_str().unwrap().as_bytes(), 16).unwrap();
    let d = BigUint::parse_bytes(v["private_exponent"].as_str().unwrap().as_bytes(), 16).unwrap();
    let plain = BigUint::parse_bytes(ciphertext.as_bytes(), 16)
        .unwrap()
        .modpow(&d, &n)
        .to_bytes_be();
    let mut block = vec![0; 128 - plain.len()];
    block.extend(plain);
    assert_eq!(&block[..2], &[0, 2]);
    let separator = block[2..].iter().position(|b| *b == 0).unwrap() + 2;
    assert!(separator >= 10);
    block[separator + 1..].to_vec()
}

#[test]
fn encryption_matches_independent_vectors_and_official_short_hex_format() {
    for (index, v) in vectors()["vectors"].as_array().unwrap().iter().enumerate() {
        let key = LoginPublicKey::parse(
            v["modulus"].as_str().unwrap(),
            v["exponent"].as_str().unwrap(),
        )
        .unwrap();
        let text = v["text"].as_str().unwrap();
        let actual = key
            .encrypt_with_entropy(text, |bytes| {
                bytes.fill(0x42);
                Ok(())
            })
            .unwrap();
        assert_eq!(actual, v["ciphertext_hex"].as_str().unwrap());
        assert_eq!(actual.len(), 254);
        assert_eq!(hex::encode(decrypt(index, &actual)), v["message_hex"]);
        assert_eq!(
            decrypt(index, &key.encrypt(text).unwrap()),
            hex::decode(v["message_hex"].as_str().unwrap()).unwrap()
        );
    }
}

#[test]
fn padding_is_fresh_and_unicode_limits_use_encoded_bytes() {
    let v = vectors();
    let v = &v["vectors"][0];
    let key = LoginPublicKey::parse(v["modulus"].as_str().unwrap(), "010001").unwrap();
    let a = key.encrypt("same password").unwrap();
    let b = key.encrypt("same password").unwrap();
    assert_ne!(a, b);
    assert_eq!(decrypt(0, &a), b"same password");
    assert_eq!(decrypt(0, &b), b"same password");
    for text in [
        "a".repeat(117),
        "中".repeat(39),
        format!("{}abc", "😀".repeat(19)),
    ] {
        let cipher = key.encrypt(&text).unwrap();
        assert_eq!(decrypt(0, &cipher).len(), 117);
    }
    for text in ["a".repeat(118), "中".repeat(40), "😀".repeat(20)] {
        assert_eq!(
            key.encrypt(&text).unwrap_err().code,
            ErrorCode::InvalidRequest
        );
    }
    assert_eq!(
        encode_login_text("a\0é中😀", 117).unwrap(),
        [
            0x61, 0, 0xc3, 0xa9, 0xe4, 0xb8, 0xad, 0xed, 0xa0, 0xbd, 0xed, 0xb8, 0x80
        ]
    );
    let mut calls = 0;
    let cipher = key
        .encrypt_with_entropy("padding", |bytes| {
            calls += 1;
            bytes.fill(if calls == 1 { 0 } else { 7 });
            Ok(())
        })
        .unwrap();
    assert!(calls > 1);
    assert_eq!(decrypt(0, &cipher), b"padding");
    assert_eq!(
        key.encrypt_with_entropy("x", |bytes| {
            bytes.fill(0);
            Ok(())
        })
        .unwrap_err()
        .code,
        ErrorCode::InternalError
    );
    assert_eq!(
        key.encrypt_with_entropy("x", |_| Err(error(
            ErrorCode::InternalError,
            "entropy unavailable"
        )))
        .unwrap_err()
        .code,
        ErrorCode::InternalError
    );
}

#[test]
fn dynamic_key_validation_rejects_malformed_small_even_or_excessive_keys() {
    let all = vectors();
    let n = all["vectors"][0]["modulus"].as_str().unwrap();
    assert!(LoginPublicKey::parse(&format!("00{}", n.to_uppercase()), "010001").is_ok());
    for exponent in [
        "",
        "0",
        "1",
        "2",
        "10000",
        "10003",
        "+10001",
        "0x10001",
        "10001 ",
        "000010001",
    ] {
        assert!(LoginPublicKey::parse(n, exponent).is_err());
    }
    for modulus in [
        String::new(),
        "f".repeat(255),
        "f".repeat(1027),
        format!("{}0", &n[..n.len() - 1]),
        format!("+{n}"),
        format!("{n} "),
    ] {
        assert!(LoginPublicKey::parse(&modulus, "10001").is_err());
    }
    let key = LoginPublicKey::parse(&"f".repeat(512), "10001").unwrap();
    assert!(key.encrypt(&"x".repeat(245)).is_ok());
    assert!(key.encrypt(&"x".repeat(246)).is_err());
}
