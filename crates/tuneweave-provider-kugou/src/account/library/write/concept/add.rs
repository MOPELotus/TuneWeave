//! i0/g1 ordinary Concept items in one bounded request, with the observed version.
use super::*;
use crate::client::concept_album::ConceptAlbumTrack;

const PATH: &str = "/cloudlist.service/v4/add_song";
const MAX_ITEMS: usize = 100;

pub(crate) struct ConceptAddedTrack {
    pub(crate) file_id: u64,
    pub(crate) sort: u64,
}

pub(crate) struct ConceptAddAck {
    pub(crate) version: WriteAck,
    pub(crate) file_id: u64,
    pub(crate) sort: u64,
}
#[derive(Deserialize)]
struct AddReceipt {
    userid: Number,
    listid: Number,
    list_ver: Number,
    pre_list_ver: Number,
    count: Number,
    #[serde(rename = "type")]
    kind: Option<Number>,
    info: Vec<Item>,
}
#[derive(Deserialize)]
struct Item {
    fileid: Number,
    name: String,
    sort: Number,
    hash: String,
    album_id: Number,
    mixsongid: Number,
    code: Option<Number>,
    csong: Option<Number>,
}

#[cfg(test)]
fn acknowledge(
    bytes: &[u8],
    uid: &str,
    list_id: u64,
    version: u64,
    input: &ConceptAlbumTrack,
) -> Result<ConceptAddAck> {
    let (version, items) =
        acknowledge_many(bytes, uid, list_id, version, std::slice::from_ref(input))?;
    let item = items.into_iter().next().ok_or_else(malformed)?;
    Ok(ConceptAddAck {
        version,
        file_id: item.file_id,
        sort: item.sort,
    })
}

fn acknowledge_many(
    bytes: &[u8],
    uid: &str,
    list_id: u64,
    version: u64,
    inputs: &[ConceptAlbumTrack],
) -> Result<(WriteAck, Vec<ConceptAddedTrack>)> {
    let status: Response<IgnoredAny> = serde_json::from_slice(bytes).map_err(|_| malformed())?;
    if status.status != 1 || status.error_code.is_some_and(|v| v != 0) {
        let code = if status.status == 0 && status.error_code == Some(20017) {
            ErrorCode::AuthenticationRequired
        } else {
            ErrorCode::UpstreamError
        };
        return Err(error(code, "KuGou Concept track addition was rejected")
            .with_details(json!({"platform_code":status.error_code})));
    }
    let receipt = serde_json::from_slice::<Response<AddReceipt>>(bytes)
        .map_err(|_| malformed())?
        .data
        .ok_or_else(malformed)?;
    if receipt.userid.0.to_string() != uid
        || receipt.listid.0 != list_id
        || receipt.pre_list_ver.0 != version
        || receipt.list_ver.0 <= version
        || receipt.list_ver.0 > i32::MAX as u64
        || receipt.kind.is_some_and(|v| v.0 != 0)
    {
        return Err(identity_conflict());
    }
    if receipt.info.len() != inputs.len() {
        return Err(malformed());
    }
    let mut expected = inputs
        .iter()
        .map(|input| (input.mixsongid, input))
        .collect::<BTreeMap<_, _>>();
    if expected.len() != inputs.len() {
        return Err(malformed());
    }
    let mut files = BTreeSet::new();
    let mut ordinals = BTreeSet::new();
    let mut items = BTreeMap::new();
    for item in receipt.info {
        let input = expected.remove(&item.mixsongid.0).ok_or_else(malformed)?;
        if item.fileid.0 == 0
            || item.fileid.0 > i32::MAX as u64
            || !files.insert(item.fileid.0)
            || item.sort.0 > i32::MAX as u64
            || !ordinals.insert(item.sort.0)
            || text(Some(item.name), 8192)?.is_none()
            || !item.hash.eq_ignore_ascii_case(&input.hash)
            || item.album_id.0.to_string() != input.album_id
            || item.code.is_some_and(|v| v.0 != 0)
            || item.csong.is_some_and(|v| v.0 != 0)
        {
            return Err(malformed());
        }
        items.insert(
            input.mixsongid,
            ConceptAddedTrack {
                file_id: item.fileid.0,
                sort: item.sort.0,
            },
        );
    }
    // ACK array order is not an identity. Restore caller order only after
    // every returned item has matched its exact mix ID, hash and album.
    let ordered = inputs
        .iter()
        .map(|input| items.remove(&input.mixsongid).ok_or_else(malformed))
        .collect::<Result<Vec<_>>>()?;
    Ok((
        WriteAck {
            version: Some(receipt.list_ver.0),
            previous_version: Some(receipt.pre_list_ver.0),
            count: Some(receipt.count.0),
        },
        ordered,
    ))
}

impl KugouClient {
    pub(crate) async fn native_add_concept_track(
        &self,
        session: &NativeSession,
        list_id: u64,
        version: u64,
        input: &ConceptAlbumTrack,
    ) -> Result<ConceptAddAck> {
        let (version, items) = self
            .native_add_concept_tracks(session, list_id, version, std::slice::from_ref(input))
            .await?;
        let item = items.into_iter().next().ok_or_else(malformed)?;
        Ok(ConceptAddAck {
            version,
            file_id: item.file_id,
            sort: item.sort,
        })
    }

    pub(crate) async fn native_add_concept_tracks(
        &self,
        session: &NativeSession,
        list_id: u64,
        version: u64,
        inputs: &[ConceptAlbumTrack],
    ) -> Result<(WriteAck, Vec<ConceptAddedTrack>)> {
        validate_session(session)?;
        if session.client != KugouLoginClient::Concept {
            return Err(error(
                ErrorCode::CapabilityNotSupported,
                "KuGou versioned track addition requires a Concept credential",
            ));
        }
        if list_id == 0
            || list_id > i32::MAX as u64
            || version > i32::MAX as u64
            || inputs.is_empty()
            || inputs.len() > MAX_ITEMS
        {
            return Err(malformed());
        }
        let mut ids = BTreeSet::new();
        let mut hashes = BTreeSet::new();
        for input in inputs {
            if input.mixsongid == 0
                || input.mixsongid > i64::MAX as u64
                || !ids.insert(input.mixsongid)
                || !hashes.insert(input.hash.to_ascii_lowercase())
                || !valid_uid(&input.album_id)
                || input.size <= 0
                || input.timelen <= 0
                || input.bitrate <= 0
                || input.hash.len() != 32
                || !input.hash.bytes().all(|b| b.is_ascii_hexdigit())
                || !input.name.ends_with(".mp3")
                || text(Some(input.name.clone()), 8192)?.is_none()
            {
                return Err(malformed());
            }
        }
        // e.e's fresh-client insertion preference 0 resolves to mode 2.
        // i0 sends that mode in reverse wire order, retaining original sort=i.
        // <=100 stays within one official 300-item segment: no split/retry.
        let data = inputs
            .iter()
            .enumerate()
            .rev()
            .map(|(sort, input)| {
                json!({
                    "number":1,"name":input.name,"hash":input.hash.to_ascii_lowercase(),
                    "size":input.size,"sort":sort,"timelen":input.timelen,"bitrate":input.bitrate,
                    "album_id":input.album_id,"mixsongid":input.mixsongid
                })
            })
            .collect::<Vec<_>>();
        let body = crypto::encode(&json!({"userid":session.user_id,"token":session.token,
            "listid":list_id,"list_ver":version,"type":0,"data":data}))?;
        self.native_concept_plaintext_list(session, PATH, "concept_track_add", body, |bytes| {
            acknowledge_many(bytes, &session.user_id, list_id, version, inputs)
        })
        .await
    }
}

#[cfg(test)]
mod tests;
