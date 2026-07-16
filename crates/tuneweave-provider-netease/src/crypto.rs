use std::io::Read;

use aes::{
    Aes128, Aes256,
    cipher::{BlockDecryptMut, BlockEncryptMut, KeyInit, KeyIvInit, block_padding::Pkcs7},
};
use aws_lc_rs::{
    aead::{AES_128_GCM, Aad, LessSafeKey, Nonce, UnboundKey},
    agreement, hmac,
    rand::SystemRandom,
};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use flate2::read::GzDecoder;
use md5::{Digest, Md5};
use num_bigint::BigUint;
use rand::{RngExt, distr::Alphanumeric};
use serde_json::{Map, Value};
use url::form_urlencoded;

const EAPI_KEY: &[u8; 16] = b"e82ckenh8dichen8";
const EAPI_SEPARATOR: &str = "-36cd479b6b5-";
const LINUXAPI_KEY: &[u8; 16] = b"rFgB&h#%2?^eDg:Q";
const WEAPI_IV: &[u8; 16] = b"0102030405060708";
const WEAPI_PRESET_KEY: &[u8; 16] = b"0CoJUm6Qyw8W8jud";
const WEAPI_RSA_EXPONENT: &str = "010001";
const WEAPI_RSA_MODULUS: &str = concat!(
    "e0b509f6259df8642dbc35662901477df22677ec152b5ff68ace615bb7b725152b3ab17a876aea8",
    "a5aa76d2e417629ec4ee341f56135fccf695280104e0312ecbda92557c93870114af6c9d05c4f7f0",
    "c3685b7a46bee255932575cce10b424d813cfe4875d3e82047b97ddef52741d546b8e289dc6935b3",
    "ece0462db0a22b8e7"
);
const XEAPI_STATIC_KEY: [u8; 32] = [
    0xab, 0x1d, 0x5a, 0x43, 0x0f, 0x6b, 0xb0, 0x4a, 0x3f, 0x01, 0xe8, 0x1d, 0xdd, 0x72, 0xbd, 0x91,
    0x6d, 0x5c, 0xe5, 0x91, 0x24, 0x8a, 0xc1, 0x28, 0x71, 0x48, 0x06, 0xd7, 0xf8, 0xfb, 0x1b, 0x84,
];
const XEAPI_SIGN_KEY: &[u8] =
    b"mUHCwVNWJbunMqAHf5MImuirT6plvs6VSFW62MGHstFQxhBGdEoIhLItH3djc4+FB/OKty3+lL2rGeoFBpVe5g==";

pub(crate) struct WeapiPayload {
    pub params: String,
    pub enc_sec_key: String,
}

pub(crate) struct XeapiPayload {
    pub b: String,
    pub s: String,
    pub r: String,
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

pub(crate) fn encrypt_linuxapi(payload: &str) -> String {
    hex::encode_upper(encrypt_aes_128_ecb(LINUXAPI_KEY, payload.as_bytes()))
}

pub(crate) fn xeapi_sign(timestamp: &str, nonce: &str) -> String {
    let key = hmac::Key::new(hmac::HMAC_SHA256, XEAPI_SIGN_KEY);
    BASE64.encode(hmac::sign(&key, format!("{timestamp}{nonce}").as_bytes()))
}

pub(crate) fn decrypt_xeapi_public_key(encrypted: &str) -> Result<String, &'static str> {
    let ciphertext = BASE64
        .decode(encrypted)
        .map_err(|_| "XEAPI public key is not valid base64")?;
    let plaintext = decrypt_aes_256_ecb(&XEAPI_STATIC_KEY, &ciphertext)?;
    String::from_utf8(plaintext).map_err(|_| "XEAPI public key is not valid UTF-8")
}

pub(crate) fn build_xeapi_plaintext(uri: &str, data: &Map<String, Value>) -> String {
    let mut fields = Map::new();
    fields.insert(
        "body".to_owned(),
        Value::String(BASE64.encode(encode_form(data, true))),
    );

    let query = uri
        .split_once('?')
        .map_or("", |(_, query)| query)
        .trim_start_matches('?');
    let query = if query.is_empty() {
        "e_r=true".to_owned()
    } else {
        format!("{query}&e_r=true")
    };
    fields.insert("queryString".to_owned(), Value::String(query));
    Value::Object(fields).to_string()
}

pub(crate) fn encode_form(data: &Map<String, Value>, omit_encrypted_response: bool) -> String {
    let mut body = form_urlencoded::Serializer::new(String::new());
    for (name, value) in data {
        if !omit_encrypted_response || name != "e_r" {
            body.append_pair(name, &javascript_form_value(value, false));
        }
    }
    body.finish()
}

pub(crate) fn encrypt_xeapi(
    plaintext: &str,
    peer_public_key: &[u8; 32],
    key_version: &str,
    server_key: &str,
    session: Option<(&str, &str)>,
) -> Result<XeapiPayload, &'static str> {
    let dynamic_key = if let Some((_, key)) = session {
        key.as_bytes()
            .try_into()
            .map_err(|_| "XEAPI session key must contain exactly 16 bytes")?
    } else {
        let mut key = [0_u8; 16];
        rand::fill(&mut key);
        key
    };

    let first = encrypt_aes_256_ecb(&XEAPI_STATIC_KEY, plaintext.as_bytes());
    let transformed = xeapi_mid_transform(&first);
    let b = encrypt_aes_128_ecb(&dynamic_key, &transformed);
    let s = encrypt_xeapi_s(&dynamic_key, peer_public_key, server_key)?;
    let session_id = session.map_or("", |(id, _)| id);
    let r = encrypt_aes_256_ecb(
        &XEAPI_STATIC_KEY,
        format!("{key_version}|{session_id}").as_bytes(),
    );

    Ok(XeapiPayload {
        b: BASE64.encode(b),
        s: BASE64.encode(s),
        r: BASE64.encode(r),
    })
}

pub(crate) fn decrypt_eapi_response(body: &[u8]) -> Result<Vec<u8>, &'static str> {
    let plaintext = decrypt_aes_128_ecb(EAPI_KEY, body)?;
    if !plaintext.starts_with(&[0x1f, 0x8b]) {
        return Ok(plaintext);
    }

    let mut decoded = Vec::new();
    GzDecoder::new(plaintext.as_slice())
        .read_to_end(&mut decoded)
        .map_err(|_| "XEAPI gzip response could not be decompressed")?;
    Ok(decoded)
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

fn encrypt_aes_128_ecb(key: &[u8; 16], plaintext: &[u8]) -> Vec<u8> {
    let message_len = plaintext.len();
    let mut buffer = vec![0_u8; message_len + 16];
    buffer[..message_len].copy_from_slice(plaintext);
    ecb::Encryptor::<Aes128>::new(key.into())
        .encrypt_padded_mut::<Pkcs7>(&mut buffer, message_len)
        .expect("AES-128 ECB buffer always includes a full padding block")
        .to_vec()
}

fn encrypt_aes_256_ecb(key: &[u8; 32], plaintext: &[u8]) -> Vec<u8> {
    let message_len = plaintext.len();
    let mut buffer = vec![0_u8; message_len + 16];
    buffer[..message_len].copy_from_slice(plaintext);
    ecb::Encryptor::<Aes256>::new(key.into())
        .encrypt_padded_mut::<Pkcs7>(&mut buffer, message_len)
        .expect("AES-256 ECB buffer always includes a full padding block")
        .to_vec()
}

fn decrypt_aes_128_ecb(key: &[u8; 16], ciphertext: &[u8]) -> Result<Vec<u8>, &'static str> {
    let mut buffer = ciphertext.to_vec();
    ecb::Decryptor::<Aes128>::new(key.into())
        .decrypt_padded_mut::<Pkcs7>(&mut buffer)
        .map(Vec::from)
        .map_err(|_| "AES-128 ECB padding is invalid")
}

fn decrypt_aes_256_ecb(key: &[u8; 32], ciphertext: &[u8]) -> Result<Vec<u8>, &'static str> {
    let mut buffer = ciphertext.to_vec();
    ecb::Decryptor::<Aes256>::new(key.into())
        .decrypt_padded_mut::<Pkcs7>(&mut buffer)
        .map(Vec::from)
        .map_err(|_| "AES-256 ECB padding is invalid")
}

fn javascript_form_value(value: &Value, nested: bool) -> String {
    match value {
        Value::Null if nested => String::new(),
        Value::Null => "null".to_owned(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        Value::String(value) => value.clone(),
        Value::Array(values) => values
            .iter()
            .map(|value| javascript_form_value(value, true))
            .collect::<Vec<_>>()
            .join(","),
        Value::Object(_) => "[object Object]".to_owned(),
    }
}

fn xeapi_mid_transform(ciphertext: &[u8]) -> Vec<u8> {
    let mut random = [0_u8; 16];
    rand::fill(&mut random);
    let xored = ciphertext
        .iter()
        .enumerate()
        .map(|(index, byte)| byte ^ random[index & 0x0f])
        .collect::<Vec<_>>();
    let encoded = BASE64.encode(xored).into_bytes();
    let rotation = if encoded.is_empty() {
        0
    } else {
        usize::from(random[0] & 0x0f) % encoded.len()
    };
    let mut transformed = Vec::with_capacity(random.len() + encoded.len());
    transformed.extend_from_slice(&random);
    transformed.extend_from_slice(&encoded[rotation..]);
    transformed.extend_from_slice(&encoded[..rotation]);
    transformed
}

fn encrypt_xeapi_s(
    dynamic_key: &[u8; 16],
    peer_public_key: &[u8; 32],
    server_key: &str,
) -> Result<Vec<u8>, &'static str> {
    let rng = SystemRandom::new();
    let private = agreement::EphemeralPrivateKey::generate(&agreement::X25519, &rng)
        .map_err(|_| "could not generate an XEAPI X25519 key")?;
    let public = private
        .compute_public_key()
        .map_err(|_| "could not derive the XEAPI X25519 public key")?;
    let public = public.as_ref().to_vec();
    let peer = agreement::UnparsedPublicKey::new(&agreement::X25519, peer_public_key);
    let shared = agreement::agree_ephemeral(private, peer, (), |secret| {
        Ok::<Vec<u8>, ()>(secret.to_vec())
    })
    .map_err(|()| "XEAPI X25519 key agreement failed")?;

    let zero_key = hmac::Key::new(hmac::HMAC_SHA256, &[0_u8; 32]);
    let pseudorandom_key = hmac::sign(&zero_key, &shared);
    let expand_key = hmac::Key::new(hmac::HMAC_SHA256, pseudorandom_key.as_ref());
    let mut expand_input = public.clone();
    expand_input.push(1);
    let expanded = hmac::sign(&expand_key, &expand_input);
    let aes_key: [u8; 16] = expanded.as_ref()[..16]
        .try_into()
        .expect("HMAC-SHA256 always contains at least 16 bytes");

    let mut iv = [0_u8; 12];
    rand::fill(&mut iv);
    let key = LessSafeKey::new(
        UnboundKey::new(&AES_128_GCM, &aes_key)
            .map_err(|_| "could not construct the XEAPI AES-GCM key")?,
    );
    let mut encrypted = format!("{}|android|{server_key}", BASE64.encode(dynamic_key)).into_bytes();
    key.seal_in_place_append_tag(
        Nonce::assume_unique_for_key(iv),
        Aad::empty(),
        &mut encrypted,
    )
    .map_err(|_| "XEAPI AES-GCM encryption failed")?;

    let mut result = Vec::with_capacity(public.len() + iv.len() + encrypted.len());
    result.extend_from_slice(&public);
    result.extend_from_slice(&iv);
    result.extend_from_slice(&encrypted);
    Ok(result)
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
    use aws_lc_rs::rand::SystemRandom;
    use serde_json::json;

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

    #[test]
    fn linuxapi_matches_an_independent_node_crypto_vector() {
        let payload = r#"{"method":"POST","url":"https://music.163.com/api/search/get","params":{"s":"TuneWeave","type":1}}"#;
        assert_eq!(
            encrypt_linuxapi(payload),
            "A0D9583F4C5FF68DE851D2893A49DE98FAFB24399F27B4F7E74C64B6FC49A965CFA972FA5EA3D6247CD6247C8198CB8770AAB1AA0DA78F0EB88EF1E1C88A4724A360FB15E36B2ED38D2E9E88AAF65ED9F71EDCE87F69822DA5B7043C5C637BA0E702478ED4773052C6F605ED77B5C32B"
        );
    }

    #[test]
    fn xeapi_signature_matches_node_crypto() {
        assert_eq!(
            xeapi_sign("1784194692618", "1234567890123456"),
            "Zh6pLbEzvwChYLZOEuOGOZjDGsHAYQQ4x3IR9LQrFt0="
        );
    }

    #[test]
    fn decrypts_a_live_xeapi_public_key_fixture() {
        let plaintext = decrypt_xeapi_public_key(
            "+6CJ7usl0OXT5E0N3Avs5lqdT+JN68IhBAwnx0ogw1lAQj0tVum4/e3HnU7jOPrnSn1xbIBZYnSAOH9LFVKoFK5klogigXcVYKAFMep3gXpWBooe3pJYPr0x8UB10OWbBHaKK0FCGtLgqow4Mz+stz445lD3HWtwvq34g2h12Gzq8imI8qYq9+eQP75Ouz2LuLa37GfarCqFjSE41mw5Dw==",
        )
        .expect("decrypt public key");
        let value: Value = serde_json::from_str(&plaintext).expect("parse public key JSON");
        assert_eq!(value["version"], "1000000000000");
        assert_eq!(value["publicKey"].as_str().map(str::len), Some(44));
        assert!(value["sk"].as_str().is_some_and(|value| !value.is_empty()));
    }

    #[test]
    fn xeapi_plaintext_matches_url_search_params_semantics() {
        let data = json!({
            "array": ["a", null, 3],
            "e_r": false,
            "nested": { "ignored": true },
            "space": "Tune Weave"
        });
        let plaintext = build_xeapi_plaintext(
            "/api/search/get?existing=1",
            data.as_object().expect("object"),
        );
        let value: Value = serde_json::from_str(&plaintext).expect("parse plaintext");
        assert_eq!(value["queryString"], "existing=1&e_r=true");
        assert_eq!(
            String::from_utf8(
                BASE64
                    .decode(value["body"].as_str().expect("body"))
                    .expect("base64 body")
            )
            .expect("UTF-8 body"),
            "array=a%2C%2C3&nested=%5Bobject+Object%5D&space=Tune+Weave"
        );
    }

    #[test]
    fn xeapi_envelope_contains_all_three_encrypted_fields() {
        let peer_private =
            agreement::EphemeralPrivateKey::generate(&agreement::X25519, &SystemRandom::new())
                .expect("generate peer key");
        let peer_public: [u8; 32] = peer_private
            .compute_public_key()
            .expect("peer public key")
            .as_ref()
            .try_into()
            .expect("X25519 public key length");
        let encrypted = encrypt_xeapi(
            r#"{"body":"","queryString":"e_r=true"}"#,
            &peer_public,
            "1000000000000",
            "server-key",
            None,
        )
        .expect("encrypt XEAPI envelope");

        assert!(!BASE64.decode(encrypted.b).expect("B").is_empty());
        assert!(BASE64.decode(encrypted.s).expect("S").len() > 60);
        assert!(!BASE64.decode(encrypted.r).expect("R").is_empty());
    }

    #[test]
    fn xeapi_response_decrypts_plain_json() {
        let encrypted = encrypt_aes_128_ecb(EAPI_KEY, br#"{"code":200}"#);
        assert_eq!(
            decrypt_eapi_response(&encrypted).expect("decrypt response"),
            br#"{"code":200}"#
        );
    }
}
