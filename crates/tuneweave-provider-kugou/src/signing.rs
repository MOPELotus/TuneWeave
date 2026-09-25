use std::collections::BTreeMap;

use md5::{Digest, Md5};

// These constants belong to fixed public client protocols, not account credentials.
pub(crate) const ANDROID_SALT: &str = "OIlwieks28dk2k092lksi2UIkp";
const WEB_SALT: &str = "NVPh5oo715z5DIWAeQlhMDsWXXQV4hwt";
const CONCEPT_SALT: &str = "LnT6xpN3khm36zse0QzvmgTZ3waWdRSA";

pub(crate) fn concept_signature(parameters: &BTreeMap<&str, String>, body: &[u8]) -> String {
    sign(CONCEPT_SALT, parameters, body)
}

pub(crate) fn android_signature(parameters: &BTreeMap<&str, String>, body: &[u8]) -> String {
    sign(ANDROID_SALT, parameters, body)
}

pub(crate) fn web_signature(parameters: &BTreeMap<&str, String>, body: &[u8]) -> String {
    // The current official getInterFaceKguser/getInterFacePublic MD5 returns uppercase.
    sign(WEB_SALT, parameters, body).to_ascii_uppercase()
}

fn sign(salt: &str, parameters: &BTreeMap<&str, String>, body: &[u8]) -> String {
    let mut digest = Md5::new();
    digest.update(salt);
    for (key, value) in parameters {
        digest.update(key.as_bytes());
        digest.update(b"=");
        digest.update(value.as_bytes());
    }
    // Callers must send these exact bytes, not serialize their body a second time.
    digest.update(body);
    digest.update(salt);
    hex::encode(digest.finalize())
}
