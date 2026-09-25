//! Account library mutations; acknowledgements never replace complete readback.
use super::*;
use crate::client::{
    decrypt_device_registration_response, encrypt_device_profile, rsa_pkcs1_v15_encrypt_for_client,
};
#[cfg(not(test))]
use rand::{TryRng, rngs::SysRng};

mod concept;

pub(crate) struct Collection<'a> {
    pub(crate) user_id: u64,
    pub(crate) list_id: u64,
    pub(crate) gid: &'a str,
}
pub(crate) struct ListEdit {
    pub(crate) name: String,
    pub(crate) intro: String,
    pub(crate) tags: String,
    pub(crate) private: bool,
    pub(crate) sort: u64,
    pub(crate) total_ver: u64,
}
pub(crate) enum ListWrite<'a> {
    Add {
        name: &'a str,
        private: bool,
        source: Option<Collection<'a>>,
    },
    Modify {
        list_id: u64,
        edit: &'a ListEdit,
    },
    Visibility {
        list_id: u64,
        name: &'a str,
        private: bool,
        sort: u64,
        total_ver: u64,
    },
    Delete {
        list_id: u64,
        kind: u8,
        total_ver: u64,
    },
}
pub(crate) struct ListAck {
    pub(crate) list_id: Option<u64>,
    pub(crate) total_ver: Option<u64>,
    pub(crate) previous_ver: Option<u64>,
    pub(crate) list_count: Option<u64>,
    pub(crate) gid: Option<String>,
}
#[derive(Deserialize)]
struct WireInfo {
    userid: Option<Number>,
    listid: Option<Number>,
    #[serde(rename = "type")]
    kind: Option<Number>,
    global_collection_id: Option<String>,
    code: Option<Number>,
}
#[derive(Deserialize)]
struct WireAck {
    userid: Option<Number>,
    listid: Option<Number>,
    #[serde(rename = "type")]
    kind: Option<Number>,
    total_ver: Option<Number>,
    pre_total_ver: Option<Number>,
    list_count: Option<Number>,
    global_collection_id: Option<String>,
    code: Option<Number>,
    info: Option<WireInfo>,
}
#[derive(Deserialize)]
struct StandardDeleteAck {
    userid: Number,
    total_ver: Number,
    pre_total_ver: Number,
    list_count: Number,
}
#[derive(Serialize)]
struct StandardSingleDeleteRequest {
    listid: u64,
    total_ver: u64,
    #[serde(rename = "type")]
    kind: u8,
}
fn consistent<T: PartialEq>(a: Option<T>, b: Option<T>) -> Result<Option<T>> {
    if a.as_ref().zip(b.as_ref()).is_some_and(|(a, b)| a != b) {
        return Err(identity_conflict());
    }
    Ok(a.or(b))
}
#[derive(Clone, Copy)]
pub(super) enum AckPolicy {
    StandardChange,
    LegacyZero,
    // Concept cover success is status plus pic, checked by its dedicated parser
    // and full library readback; its UI does not interpret either nested code.
    ConceptCover,
}
pub(super) fn acknowledge(
    bytes: &[u8],
    uid: &str,
    kind: u8,
    expected: Option<u64>,
    policy: AckPolicy,
) -> Result<ListAck> {
    let wire: WireAck = data(bytes)?;
    // Official Standard add_list/modify_list use one, unlike the legacy
    // zero-code parser. Do not infer the success code for another endpoint.
    let item_code = wire.info.as_ref().and_then(|v| v.code).map(|v| v.0);
    let accepted = match policy {
        AckPolicy::StandardChange => item_code == Some(1),
        AckPolicy::LegacyZero => item_code.is_none_or(|v| v == 0),
        AckPolicy::ConceptCover => true,
    };
    if (!matches!(policy, AckPolicy::ConceptCover) && wire.code.is_some_and(|v| v.0 != 0))
        || !accepted
    {
        return Err(error(
            ErrorCode::UpstreamError,
            "KuGou playlist mutation was not acknowledged",
        ));
    }
    let info = wire.info.unwrap_or(WireInfo {
        userid: None,
        listid: None,
        kind: None,
        global_collection_id: None,
        code: None,
    });
    let owner = consistent(wire.userid, info.userid)?;
    let list_kind = consistent(wire.kind, info.kind)?;
    if owner.is_some_and(|v| v.0.to_string() != uid)
        || list_kind.is_some_and(|v| v.0 != u64::from(kind))
    {
        return Err(identity_conflict());
    }
    let list_id = consistent(wire.listid, info.listid)?
        .map(positive)
        .transpose()?;
    if expected.zip(list_id).is_some_and(|(a, b)| a != b) {
        return Err(identity_conflict());
    }
    if expected.is_none() && list_id.is_none() {
        return Err(malformed());
    }
    Ok(ListAck {
        list_id,
        total_ver: wire.total_ver.map(|v| v.0),
        previous_ver: wire.pre_total_ver.map(|v| v.0),
        list_count: wire.list_count.map(|v| v.0),
        gid: gid(consistent(
            wire.global_collection_id,
            info.global_collection_id,
        )?)?,
    })
}
fn acknowledge_standard_delete(
    bytes: &[u8],
    seed: &str,
    uid: &str,
    expected: u64,
) -> Result<ListAck> {
    if expected == 0 {
        return Err(malformed());
    }
    // The Standard single-delete consumer reads these fields directly from data and
    // binds its result to the active UID. It does not consume the batch-only info array.
    let plaintext = decrypt_device_registration_response(bytes, seed).map_err(|_| malformed())?;
    let wire: StandardDeleteAck = data(&plaintext)?;
    if wire.userid.0.to_string() != uid {
        return Err(identity_conflict());
    }
    Ok(ListAck {
        list_id: Some(expected),
        total_ver: Some(wire.total_ver.0),
        previous_ver: Some(wire.pre_total_ver.0),
        list_count: Some(wire.list_count.0),
        gid: None,
    })
}
fn checked_text(value: &str, limit: usize, allow_empty: bool) -> Result<()> {
    if text(Some(value.to_owned()), limit)?.is_none() && !allow_empty {
        return Err(malformed());
    }
    Ok(())
}
pub(super) fn token_fields(
    client: KugouLoginClient,
    token: &str,
    seed: &str,
) -> Result<(String, String)> {
    if seed.len() != 6 || !seed.bytes().all(|b| b.is_ascii_alphanumeric()) {
        return Err(malformed());
    }
    let encstr = encrypt_device_profile(&crypto::encode(&json!({"token":token}))?, seed)?;
    let enckey = rsa_pkcs1_v15_encrypt_for_client(client, seed.as_bytes())?.to_ascii_uppercase();
    Ok((enckey, encstr))
}
#[cfg(test)]
pub(in crate::account) fn random_seed() -> Result<String> {
    Ok(TEST_RANDOM_SEED.to_owned())
}
#[cfg(not(test))]
pub(in crate::account) fn random_seed() -> Result<String> {
    const CHARS: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ";
    let mut out = String::with_capacity(6);
    while out.len() < 6 {
        let mut byte = [0];
        SysRng.try_fill_bytes(&mut byte).map_err(|_| {
            error(
                ErrorCode::InternalError,
                "KuGou playlist encryption randomness failed",
            )
        })?;
        if byte[0] < 248 {
            out.push(char::from(CHARS[usize::from(byte[0]) % CHARS.len()]));
        }
    }
    Ok(out)
}
#[cfg(test)]
pub(crate) const TEST_RANDOM_SEED: &str = "a1B2c3";
impl KugouClient {
    pub(crate) async fn native_write_list(
        &self,
        session: &NativeSession,
        operation: ListWrite<'_>,
    ) -> Result<ListAck> {
        validate_session(session)?;
        if session.client == KugouLoginClient::Concept
            && let ListWrite::Modify { list_id, edit } = &operation
        {
            return self
                .native_modify_concept_list(session, *list_id, edit)
                .await;
        }
        if session.client == KugouLoginClient::Concept
            && matches!(&operation, ListWrite::Add { source: None, .. })
        {
            return Err(error(
                ErrorCode::CapabilityNotSupported,
                "KuGou Concept creation supports only platform-default visibility through its dedicated protocol",
            ));
        }
        if session.client == KugouLoginClient::Concept
            && matches!(
                &operation,
                ListWrite::Add {
                    source: Some(_),
                    ..
                }
            )
        {
            // Concept's type1 creation and subsequent source-song synchronization
            // are not the generic v5 membership operation.
            return Err(error(
                ErrorCode::CapabilityNotSupported,
                "KuGou Concept does not support creating a new playlist subscription",
            ));
        }
        if session.client == KugouLoginClient::Concept
            && let ListWrite::Delete {
                list_id,
                kind,
                total_ver,
            } = &operation
        {
            return self
                .native_delete_concept_list(session, *list_id, *kind, *total_ver)
                .await;
        }
        let uid = session.user_id.parse::<u64>().map_err(|_| malformed())?;
        let (endpoint, body, kind, expected, standard_seed) = match operation {
            ListWrite::Add {
                name,
                private,
                source,
            } => {
                checked_text(name, 1024, false)?;
                let kind = u8::from(source.is_some());
                let (source_uid, source_id, source_gid) = source
                    .map(|s| (s.user_id, s.list_id, s.gid))
                    .unwrap_or((0, 0, ""));
                if kind == 1 && (gid(Some(source_gid.to_owned()))?.is_none() || private) {
                    return Err(malformed());
                }
                (
                    if kind == 0 {
                        Endpoint::ListCreate
                    } else {
                        Endpoint::ListCollect
                    },
                    crypto::encode(
                        &json!({"userid":uid,"token":session.token,"total_ver":0,"name":name,"type":kind,"source":1,
                    "is_pri":u8::from(private),"list_create_userid":source_uid,"list_create_listid":source_id,
                    "list_create_gid":source_gid,"from_shupinmv":0}),
                    )?,
                    kind,
                    None,
                    None,
                )
            }
            ListWrite::Modify { list_id, edit } => {
                if list_id == 0 {
                    return Err(malformed());
                }
                checked_text(&edit.name, 1024, false)?;
                checked_text(&edit.intro, 16384, true)?;
                checked_text(&edit.tags, 4096, true)?;
                let (enckey, encstr) =
                    token_fields(session.client, &session.token, &random_seed()?)?;
                (
                    Endpoint::ListModify,
                    crypto::encode(
                        &json!({"userid":uid,"listid":list_id,"total_ver":edit.total_ver,"type":0,
                    "name":edit.name,"intro":edit.intro,"tags":edit.tags,"is_pri":u8::from(edit.private),
                    "sort":edit.sort,"enckey":enckey,"encstr":encstr}),
                    )?,
                    0,
                    Some(list_id),
                    None,
                )
            }
            ListWrite::Visibility {
                list_id,
                name,
                private,
                sort,
                total_ver,
            } => {
                if session.client != KugouLoginClient::Standard {
                    return Err(error(
                        ErrorCode::CapabilityNotSupported,
                        "KuGou playlist visibility requires a Standard native credential",
                    ));
                }
                if list_id == 0 {
                    return Err(malformed());
                }
                checked_text(name, 1024, false)?;
                let (enckey, encstr) =
                    token_fields(session.client, &session.token, &random_seed()?)?;
                // Standard CloudMusicSetLMRenameRequestor.f() uses modify type 2.
                // It omits absent name/pic overrides and list_create_gid, and does
                // not send intro/tags. The provider excludes collaborative lists.
                (
                    Endpoint::ListModify,
                    crypto::encode(&json!({"userid":uid,"listid":list_id,"total_ver":total_ver,
                        "type":0,"name":name,"sort":sort,"is_pri":u8::from(private),
                        "is_mutual":0,"support_pub":1,"enckey":enckey,"encstr":encstr}))?,
                    0,
                    Some(list_id),
                    None,
                )
            }
            ListWrite::Delete {
                list_id,
                kind,
                total_ver,
            } => {
                if list_id == 0 || kind > 1 {
                    return Err(malformed());
                }
                if session.client == KugouLoginClient::Standard {
                    let seed = random_seed()?;
                    let plaintext = crypto::encode(&StandardSingleDeleteRequest {
                        listid: list_id,
                        total_ver,
                        kind,
                    })?;
                    (
                        Endpoint::ListDeleteStandard,
                        encrypt_device_profile(&plaintext, &seed)?.into_bytes(),
                        kind,
                        Some(list_id),
                        Some(seed),
                    )
                } else {
                    (
                        Endpoint::ListDelete,
                        crypto::encode(
                            &json!({"userid":uid,"token":session.token,"listid":list_id,"type":kind,"total_ver":0}),
                        )?,
                        kind,
                        Some(list_id),
                        None,
                    )
                }
            }
        };
        let policy = if session.client == KugouLoginClient::Standard
            && matches!(
                endpoint,
                Endpoint::ListCreate | Endpoint::ListCollect | Endpoint::ListModify
            ) {
            AckPolicy::StandardChange
        } else {
            AckPolicy::LegacyZero
        };
        if let Some(seed) = standard_seed {
            self.native_post_standard_delete(session, now_ms()? / 1000, &seed, body, |bytes| {
                acknowledge_standard_delete(bytes, &seed, &session.user_id, expected.unwrap_or(0))
            })
            .await
        } else {
            self.native_post(endpoint, session, now_ms()? / 1000, body, |bytes| {
                acknowledge(bytes, &session.user_id, kind, expected, policy)
            })
            .await
        }
    }
}

#[cfg(test)]
mod tests;
