use super::*;
use crate::{device::KugouDeviceIdentity, signing::concept_signature};
use serde_json::Value;
use std::collections::BTreeMap;

fn vectors() -> Value {
    serde_json::from_str(include_str!("test_vectors.json")).unwrap()
}

#[test]
fn native_encryption_matches_independent_rsa_and_openssl_vectors() {
    let v = vectors();
    let cipher = ExchangeCipher {
        seed: v["seed"].as_str().unwrap().to_owned(),
    };
    for (client, name) in [
        (KugouLoginClient::Standard, "standard"),
        (KugouLoginClient::Concept, "concept"),
    ] {
        let session = NativeSession {
            client,
            device: KugouDeviceIdentity {
                guid: v["guid"].as_str().unwrap().to_owned(),
                mid: "unused".to_owned(),
                dfid: None,
            },
            user_id: "123456789".to_owned(),
            token: v["token"].as_str().unwrap().to_owned(),
            vip_token: None,
            t1: Some("previous-device-token".to_owned()),
        };
        assert_eq!(
            cipher.pk(client, 1700000000123).unwrap(),
            v["cases"][name]["pk"]
        );
        assert_eq!(
            profile_p(client, &session.token, 1700000000).unwrap(),
            v["cases"][name]["profile_p"]
        );
        assert_eq!(p3(&session, 1700000000).unwrap(), v["cases"][name]["p3"]);
        if client == KugouLoginClient::Concept {
            let (t1, t2) = concept_fingerprints(&session, 1700000000123).unwrap();
            assert_eq!(t1, v["t1"]);
            assert_eq!(t2, v["t2"]);
        }
    }
    assert_eq!(cipher.encrypt(b"{}").unwrap(), v["params"]);
    assert_eq!(
        cipher.password_pk("1700000000628").unwrap(),
        v["native_password_pk"]
    );
    assert_eq!(
        cipher.decrypt(v["secu_params"].as_str().unwrap()).unwrap(),
        v["secu_plain"].as_str().unwrap().as_bytes()
    );
}

#[test]
fn raw_rsa_never_truncates_or_uses_a_native_key_for_web() {
    for client in [KugouLoginClient::Standard, KugouLoginClient::Concept] {
        assert_eq!(rsa(client, &[b'a'; 128]).unwrap().len(), 256);
        assert!(rsa(client, &[b'a'; 129]).is_err());
        assert!(rsa(client, &[]).is_err());
        assert!(rsa(client, &[255; 128]).is_err());
        assert!(profile_p(client, &"long-secret".repeat(30), 1700000000).is_err());
    }
    assert_eq!(
        rsa(KugouLoginClient::Web, b"{}").unwrap_err().code,
        ErrorCode::CapabilityNotSupported
    );
}

#[test]
fn encrypted_responses_are_bounded_and_require_valid_padding() {
    let cipher = ExchangeCipher {
        seed: "0123456789ABCDEF0123456789ABCDEF".to_owned(),
    };
    for ciphertext in [
        String::new(),
        "0".to_owned(),
        "00".repeat(16),
        "gg".repeat(16),
        "00".repeat(65_537),
    ] {
        let error = cipher.decrypt(&ciphertext).unwrap_err();
        assert_eq!(error.code, ErrorCode::UpstreamError);
        assert!(!format!("{error:?}").contains(&cipher.seed));
    }
    let first = ExchangeCipher::random().unwrap();
    let second = ExchangeCipher::random().unwrap();
    assert_ne!(first.seed, second.seed);
    assert!(!format!("{first:?}").contains(&first.seed));
}

#[test]
fn concept_signature_matches_independent_exact_body_vector() {
    let v = vectors();
    let p = v["concept_sign"]["params"]
        .as_object()
        .unwrap()
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str().unwrap().to_owned()))
        .collect::<BTreeMap<_, _>>();
    let body = v["concept_sign"]["body"].as_str().unwrap().as_bytes();
    assert_eq!(concept_signature(&p, body), v["concept_sign"]["signature"]);
    assert_ne!(concept_signature(&p, b"{}"), v["concept_sign"]["signature"]);
}
