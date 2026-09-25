use super::*;
use serde_json::Value;
use std::collections::BTreeMap;

#[test]
fn web_cipher_and_signature_match_independent_python_and_openssl_vectors() {
    let v: Value = serde_json::from_str(include_str!("test_vectors.json")).unwrap();
    let cipher = WebCipher {
        seed: v["seed"].as_str().unwrap().to_owned(),
    };
    assert_eq!(
        cipher.pk(v["milliseconds"].as_u64().unwrap()).unwrap(),
        v["pk"]
    );
    assert_eq!(rsa(b"\0").unwrap(), v["zero_rsa"]);
    for case in v["cases"].as_array().unwrap() {
        assert_eq!(
            cipher.token(case["token"].as_str().unwrap()).unwrap(),
            case["params"]
        );
    }
    let params: BTreeMap<_, _> = v["sign_params"]
        .as_object()
        .unwrap()
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str().unwrap().to_owned()))
        .collect();
    assert_eq!(crate::signing::web_signature(&params, b""), v["signature"]);
    assert_ne!(
        crate::signing::web_signature(&params, b"{}"),
        v["signature"]
    );
}

#[test]
fn web_random_keys_have_the_official_shape_and_payloads_are_never_truncated() {
    let a = WebCipher::random().unwrap();
    let b = WebCipher::random().unwrap();
    assert_eq!(a.seed.len(), 16);
    assert!(
        a.seed
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
    );
    assert_ne!(a.seed, b.seed);
    assert!(!format!("{a:?}").contains(&a.seed));
    for plain in [vec![], vec![b'a'; 129], vec![255; 128]] {
        assert!(rsa(&plain).is_err());
    }
    assert!(rsa(&[b'a'; 128]).is_ok());
    for token in ["", "null", "secret\n", "秘密"] {
        assert!(a.token(token).is_err());
    }
}
