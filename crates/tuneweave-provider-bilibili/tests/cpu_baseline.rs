use aws_lc_rs::{digest, hmac};

#[test]
fn baseline_cpu_crypto_known_vectors() {
    assert_eq!(
        hex::encode(digest::digest(&digest::SHA256, b"abc").as_ref()),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    let key = hmac::Key::new(hmac::HMAC_SHA256, &[0x0b; 20]);
    assert_eq!(
        hex::encode(hmac::sign(&key, b"Hi There").as_ref()),
        "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7"
    );
}
