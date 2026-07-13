use aes::{
    Aes128,
    cipher::{BlockEncryptMut, KeyInit, block_padding::Pkcs7},
};
use md5::{Digest, Md5};

const EAPI_KEY: &[u8; 16] = b"e82ckenh8dichen8";
const EAPI_SEPARATOR: &str = "-36cd479b6b5-";

pub(crate) fn encrypt_eapi(path: &str, payload: &str) -> String {
    let signature_input = format!("nobody{path}use{payload}md5forencrypt");
    let digest = hex::encode(Md5::digest(signature_input.as_bytes()));
    let plaintext = format!("{path}{EAPI_SEPARATOR}{payload}{EAPI_SEPARATOR}{digest}");
    let message_len = plaintext.len();
    let mut buffer = vec![0_u8; message_len + 16];
    buffer[..message_len].copy_from_slice(plaintext.as_bytes());

    let encrypted = ecb::Encryptor::<Aes128>::new(EAPI_KEY.into())
        .encrypt_padded_mut::<Pkcs7>(&mut buffer, message_len)
        .expect("EAPI buffer always includes a full padding block");
    hex::encode_upper(encrypted)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eapi_matches_the_reference_vector() {
        let encrypted = encrypt_eapi("/api/search/get", r#"{"s":"TuneWeave","type":1}"#);
        assert_eq!(
            encrypted,
            "1AF0E93B0E3EA03CE4E7F1B6AD7BD32BC198D7B70109AB343E0FC0C4A8F27C961AEA0CDD5FDEE2E497B3C30EEE38B39444D766C3647F55440F112329AC735976A4DF6BF001F33530CCC68EF492BF93641AF244BE466D0F032BD26A7387DDBE1C92F7774DEF667924C4681FC2C427D4ED"
        );
    }
}
