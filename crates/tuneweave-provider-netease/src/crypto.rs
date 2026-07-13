use aes::{
    Aes128,
    cipher::{BlockEncryptMut, KeyInit, KeyIvInit, block_padding::Pkcs7},
};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use md5::{Digest, Md5};
use num_bigint::BigUint;
use rand::{RngExt, distr::Alphanumeric};

const EAPI_KEY: &[u8; 16] = b"e82ckenh8dichen8";
const EAPI_SEPARATOR: &str = "-36cd479b6b5-";
const WEAPI_IV: &[u8; 16] = b"0102030405060708";
const WEAPI_PRESET_KEY: &[u8; 16] = b"0CoJUm6Qyw8W8jud";
const WEAPI_RSA_EXPONENT: &str = "010001";
const WEAPI_RSA_MODULUS: &str = concat!(
    "e0b509f6259df8642dbc35662901477df22677ec152b5ff68ace615bb7b725152b3ab17a876aea8",
    "a5aa76d2e417629ec4ee341f56135fccf695280104e0312ecbda92557c93870114af6c9d05c4f7f0",
    "c3685b7a46bee255932575cce10b424d813cfe4875d3e82047b97ddef52741d546b8e289dc6935b3",
    "ece0462db0a22b8e7"
);

pub(crate) struct WeapiPayload {
    pub params: String,
    pub enc_sec_key: String,
}

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

pub(crate) fn encrypt_weapi(payload: &str) -> WeapiPayload {
    let secret = random_weapi_secret();
    encrypt_weapi_with_secret(payload, &secret)
}

fn encrypt_weapi_with_secret(payload: &str, secret: &[u8; 16]) -> WeapiPayload {
    let first = encrypt_cbc_base64(payload.as_bytes(), WEAPI_PRESET_KEY);
    let params = encrypt_cbc_base64(first.as_bytes(), secret);
    let reversed_secret = secret.iter().rev().copied().collect::<Vec<_>>();
    let message = BigUint::from_bytes_be(&reversed_secret);
    let exponent = BigUint::parse_bytes(WEAPI_RSA_EXPONENT.as_bytes(), 16)
        .expect("WeAPI RSA exponent is valid hex");
    let modulus = BigUint::parse_bytes(WEAPI_RSA_MODULUS.as_bytes(), 16)
        .expect("WeAPI RSA modulus is valid hex");
    let encrypted_secret = message.modpow(&exponent, &modulus);
    let enc_sec_key = format!("{:0>256}", encrypted_secret.to_str_radix(16));
    WeapiPayload {
        params,
        enc_sec_key,
    }
}

fn encrypt_cbc_base64(plaintext: &[u8], key: &[u8; 16]) -> String {
    let message_len = plaintext.len();
    let mut buffer = vec![0_u8; message_len + 16];
    buffer[..message_len].copy_from_slice(plaintext);
    let encrypted = cbc::Encryptor::<Aes128>::new(key.into(), WEAPI_IV.into())
        .encrypt_padded_mut::<Pkcs7>(&mut buffer, message_len)
        .expect("WeAPI buffer always includes a full padding block");
    BASE64.encode(encrypted)
}

fn random_weapi_secret() -> [u8; 16] {
    let mut secret = [0_u8; 16];
    for (destination, value) in secret.iter_mut().zip(rand::rng().sample_iter(Alphanumeric)) {
        *destination = value;
    }
    secret
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

    #[test]
    fn weapi_matches_an_independent_node_crypto_vector() {
        let encrypted = encrypt_weapi_with_secret(
            r#"{"ctcode":"86","cellphone":"13800138000","csrf_token":"","secrete":"music_middleuser_pclogin"}"#,
            b"abcdefghijklmnop",
        );
        assert_eq!(
            encrypted.params,
            "dRY+IOEoPopGN/lk1On+iuJZ+Bo6UEK/l3MDJIs5CbY8ju/bssfNWsEQirSNKrBAdEuhwmKwjOFWcpNK2Nlun7lS+YPD0r9JKfvk+AnECs2xBjcTiQz5LUShqePSPpWBkBPO4TubATNZP/nECuRElLYBr7aQxl3wR0itNNprNEdDSQfogHC5BZYHeOjer65m"
        );
        assert_eq!(
            encrypted.enc_sec_key,
            "d15a1683c992095d0c234c19966605c5c5964911268bbeda8cb8d08d834913e59d53b32358903a121b5fca784c1f5ae44951fd02524df58ecc98e52cc7cf8689b42c2e93ddf05b0592512d87f5960467e2f086c018849d76014d323500e30f13ef4cafbb0cf5a66731a3f1776c75ca35d0062dac70a3e33245afabcf47938487"
        );
    }
}
