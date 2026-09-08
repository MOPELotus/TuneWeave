use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

use aws_lc_rs::digest::{SHA256, digest};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};
use tuneweave_core::{
    AccountCredentialStore, ErrorCode, Platform, Result, StoredAccountCredential, TuneWeaveError,
};

pub(crate) const CREDENTIAL_KIND: &str = "netease_nmtid_v1";
pub(crate) const ACCOUNT_PREFIX: &str = "__nmtid_";

#[derive(Clone, Deserialize, Serialize)]
struct Identity {
    binding: String,
    value: Option<String>,
}

/// Session ownership is separate from the server's cryptographic transport key cache.
/// Only actual server-issued identities are persisted; no synthetic probe requests are sent.
pub(crate) struct Identities {
    entries: Mutex<BTreeMap<String, Identity>>,
    store: Option<Arc<dyn AccountCredentialStore>>,
}

fn state_error() -> TuneWeaveError {
    TuneWeaveError::new(
        ErrorCode::InternalError,
        "NetEase session identity state is unavailable",
    )
    .with_platform(Platform::Netease)
}

fn hash(value: &str) -> String {
    URL_SAFE_NO_PAD.encode(digest(&SHA256, value.as_bytes()).as_ref())
}

fn identity_binding(cookie: Option<&str>) -> String {
    let user = cookie_value(cookie, "MUSIC_U").unwrap_or_default();
    let anonymous = if user.is_empty() {
        cookie_value(cookie, "MUSIC_A").unwrap_or_default()
    } else {
        ""
    };
    hash(&format!("{user}\n{anonymous}"))
}

pub(crate) fn cookie_value<'a>(cookie: Option<&'a str>, key: &str) -> Option<&'a str> {
    cookie
        .unwrap_or_default()
        .split(';')
        .filter_map(|part| part.trim().split_once('='))
        .filter(|(name, _)| *name == key)
        .map(|(_, value)| value)
        .next_back()
}

fn valid(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

impl Identities {
    pub(crate) fn new(store: Option<Arc<dyn AccountCredentialStore>>) -> Result<Self> {
        let mut entries = BTreeMap::new();
        if let Some(store) = &store {
            for record in store.load_platform(Platform::Netease)? {
                if record.kind != CREDENTIAL_KIND {
                    continue;
                }
                let identity: Identity =
                    serde_json::from_str(record.secret()).map_err(|_| state_error())?;
                if !record.account.starts_with(ACCOUNT_PREFIX)
                    || identity.binding.len() != 43
                    || identity.value.as_deref().is_none_or(|value| !valid(value))
                {
                    return Err(state_error());
                }
                entries.insert(record.account, identity);
            }
        }
        Ok(Self {
            entries: Mutex::new(entries),
            store,
        })
    }

    pub(crate) fn ephemeral() -> Self {
        Self {
            entries: Mutex::new(BTreeMap::new()),
            store: None,
        }
    }

    pub(crate) fn current(&self, scope: &str, cookie: Option<&str>) -> Result<Option<String>> {
        let supplied = cookie_value(cookie, "NMTID");
        if supplied.is_some_and(|value| !valid(value)) {
            return Err(TuneWeaveError::invalid_request("NetEase NMTID is invalid")
                .with_platform(Platform::Netease));
        }
        let key = format!("{ACCOUNT_PREFIX}{}", hash(scope));
        let binding = identity_binding(cookie);
        let mut entries = self.entries.lock().map_err(|_| state_error())?;
        if let Some(entry) = entries.get(&key)
            && entry.binding == binding
        {
            return Ok(supplied.map(str::to_owned).or_else(|| entry.value.clone()));
        }
        if entries.len() >= 4096 && !entries.contains_key(&key) {
            return Err(state_error());
        }
        let value = supplied.map(str::to_owned);
        entries.insert(
            key,
            Identity {
                binding,
                value: value.clone(),
            },
        );
        Ok(value)
    }

    pub(crate) fn observe(
        &self,
        scope: &str,
        cookie: Option<&str>,
        cookies: &[String],
    ) -> Result<()> {
        // An explicitly supplied identity belongs to the caller and is never silently replaced.
        if cookie_value(cookie, "NMTID").is_some() {
            return Ok(());
        }
        let value = cookies
            .iter()
            .filter_map(|line| {
                let pair = line.split(';').next()?;
                let (name, value) = pair.split_once('=')?;
                (name.trim() == "NMTID" && valid(value)).then_some(value)
            })
            .next_back();
        let Some(value) = value else {
            return Ok(());
        };
        let key = format!("{ACCOUNT_PREFIX}{}", hash(scope));
        let binding = identity_binding(cookie);
        let mut entries = self.entries.lock().map_err(|_| state_error())?;
        let Some(current) = entries.get(&key) else {
            return Ok(());
        };
        // Ignore a response belonging to a session replaced while this request was in flight.
        if current.binding != binding || current.value.as_deref() == Some(value) {
            return Ok(());
        }
        let identity = Identity {
            binding,
            value: Some(value.to_owned()),
        };
        if let Some(store) = &self.store {
            store.put(&StoredAccountCredential::new(
                Platform::Netease,
                &key,
                CREDENTIAL_KIND,
                serde_json::to_string(&identity).map_err(|_| state_error())?,
            )?)?;
        }
        entries.insert(key, identity);
        Ok(())
    }

    pub(crate) fn remove(&self, scope: &str) -> Result<()> {
        let key = format!("{ACCOUNT_PREFIX}{}", hash(scope));
        let mut entries = self.entries.lock().map_err(|_| state_error())?;
        if let Some(store) = &self.store {
            store.remove(Platform::Netease, &key)?;
        }
        entries.remove(&key);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identities_preserve_supplied_values_and_isolate_accounts_and_late_responses() {
        let state = Identities::ephemeral();
        assert_eq!(
            state.current("account:a", Some("MUSIC_U=one")).unwrap(),
            None
        );
        state
            .observe(
                "account:a",
                Some("MUSIC_U=one"),
                &["NMTID=server_one; Path=/".to_owned()],
            )
            .unwrap();
        assert_eq!(
            state
                .current("account:a", Some("MUSIC_U=one"))
                .unwrap()
                .as_deref(),
            Some("server_one")
        );
        assert_eq!(
            state.current("account:b", Some("MUSIC_U=one")).unwrap(),
            None
        );
        assert_eq!(
            state.current("account:a", Some("MUSIC_U=two")).unwrap(),
            None
        );
        state
            .observe("account:a", Some("MUSIC_U=one"), &["NMTID=late".to_owned()])
            .unwrap();
        assert_eq!(
            state.current("account:a", Some("MUSIC_U=two")).unwrap(),
            None
        );
        assert_eq!(
            state
                .current("account:a", Some("MUSIC_U=two; NMTID=caller"))
                .unwrap()
                .as_deref(),
            Some("caller")
        );
        assert!(
            state
                .current("account:a", Some("NMTID=bad\r\nheader"))
                .is_err()
        );
    }

    #[test]
    fn server_identities_survive_restart_and_logout_removes_only_the_selected_scope() {
        let root = std::env::temp_dir().join(format!("tuneweave-nmtid-{}", rand::random::<u64>()));
        let store: Arc<dyn AccountCredentialStore> =
            Arc::new(tuneweave_core::FileAccountCredentialStore::open(&root).unwrap());
        let state = Identities::new(Some(store.clone())).unwrap();
        state.current("anonymous", None).unwrap();
        state
            .observe("anonymous", None, &["NMTID=server_value".to_owned()])
            .unwrap();
        let restored = Identities::new(Some(store)).unwrap();
        assert_eq!(
            restored.current("anonymous", None).unwrap().as_deref(),
            Some("server_value")
        );
        restored.remove("anonymous").unwrap();
        assert_eq!(restored.current("anonymous", None).unwrap(), None);
        std::fs::remove_dir_all(root).unwrap();
    }
}
