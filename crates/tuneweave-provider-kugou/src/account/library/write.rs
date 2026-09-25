use super::*;
use tuneweave_core::Track;

mod concept;
mod standard;

pub(crate) const WRITE_BATCH_SIZE: usize = 300;

#[derive(Serialize)]
#[serde(transparent)]
pub(crate) struct TrackInput {
    wire: TrackWire,
}

#[derive(Serialize)]
#[serde(untagged)]
enum TrackWire {
    Catalogue(CatalogueTrackInput),
    Standard(standard::StandardTrackInput),
}

impl TrackInput {
    pub(crate) fn from_catalogue(track: &Track, expected: &str) -> Result<Self> {
        Ok(Self {
            wire: TrackWire::Catalogue(CatalogueTrackInput::from_catalogue(track, expected)?),
        })
    }

    pub(crate) fn from_standard_catalogue(track: &Track, expected: &str) -> Result<Self> {
        Ok(Self {
            wire: TrackWire::Standard(standard::StandardTrackInput::from_catalogue(
                track, expected,
            )?),
        })
    }

    fn mixsongid(&self) -> u64 {
        match &self.wire {
            TrackWire::Catalogue(row) => row.mixsongid,
            TrackWire::Standard(row) => row.mixsongid,
        }
    }
}

#[derive(Serialize)]
struct CatalogueTrackInput {
    number: u8,
    name: String,
    hash: String,
    size: u64,
    sort: u8,
    timelen: u8,
    bitrate: u8,
    album_id: u64,
    mixsongid: u64,
}
impl CatalogueTrackInput {
    fn from_catalogue(track: &Track, expected: &str) -> Result<Self> {
        if track.platform != Platform::Kugou
            || track.resource_ref.platform() != Platform::Kugou
            || track.id != expected
            || track.resource_ref.id() != expected
            || !valid_uid(expected)
        {
            return Err(malformed());
        }
        let asset = track
            .extensions
            .get("qualities")
            .and_then(|q| q.get("standard"))
            .ok_or_else(malformed)?;
        let hash = asset
            .get("hash")
            .and_then(Value::as_str)
            .filter(|v| v.len() == 32 && v.bytes().all(|b| b.is_ascii_hexdigit()))
            .ok_or_else(malformed)?;
        let title = text(Some(track.name.clone()), 8192)?.ok_or_else(malformed)?;
        let artists = track
            .artists
            .iter()
            .map(|a| a.name.as_str())
            .collect::<Vec<_>>()
            .join("、");
        let name = if artists.is_empty() {
            title
        } else {
            format!("{artists} - {title}")
        };
        let name = text(Some(name), 8192)?.ok_or_else(malformed)?;
        let album_id = track
            .album
            .as_ref()
            .and_then(|a| a.resource_ref.as_ref())
            .map(|r| {
                if r.platform() != Platform::Kugou || (r.id() != "0" && !valid_uid(r.id())) {
                    return Err(malformed());
                }
                r.id().parse().map_err(|_| malformed())
            })
            .transpose()?
            .unwrap_or(0);
        Ok(Self {
            number: 1,
            name,
            hash: hash.to_ascii_lowercase(),
            size: asset.get("size").and_then(Value::as_u64).unwrap_or(0),
            // Keep the existing reference-based mapping for callers outside the
            // independently audited Standard ordinary-playlist input slice.
            sort: 0,
            timelen: 0,
            bitrate: 0,
            album_id,
            mixsongid: expected.parse().map_err(|_| malformed())?,
        })
    }
}

pub(crate) enum Write<'a> {
    Add(&'a [TrackInput]),
    Remove(&'a [u64]),
}
pub(crate) struct WriteAck {
    pub(crate) version: Option<u64>,
    pub(crate) previous_version: Option<u64>,
    pub(crate) count: Option<u64>,
}
#[derive(Deserialize)]
struct WireAck {
    userid: Option<Number>,
    listid: Option<Number>,
    #[serde(rename = "type")]
    kind: Option<Number>,
    list_ver: Option<Number>,
    pre_list_ver: Option<Number>,
    count: Option<Number>,
    code: Option<Number>,
}

impl KugouClient {
    pub(crate) async fn native_write_tracks(
        &self,
        session: &NativeSession,
        list_id: u64,
        operation: Write<'_>,
    ) -> Result<WriteAck> {
        validate_session(session)?;
        if session.client == KugouLoginClient::Concept && matches!(&operation, Write::Remove(_)) {
            return Err(error(
                ErrorCode::CapabilityNotSupported,
                "KuGou Concept removal requires the versioned single-occurrence protocol",
            ));
        }
        if list_id == 0 {
            return Err(error(
                ErrorCode::InvalidRequest,
                "KuGou write list ID is invalid",
            ));
        }
        let mut body = json!({"userid":session.user_id.parse::<u64>().map_err(|_|malformed())?,
            "token":session.token,"listid":list_id,"type":0,"list_ver":0});
        let endpoint = match operation {
            Write::Add(rows) => {
                if rows.is_empty() || rows.len() > WRITE_BATCH_SIZE {
                    return Err(malformed());
                }
                let mut ids = BTreeSet::new();
                if rows.iter().any(|r| !ids.insert(r.mixsongid())) {
                    return Err(malformed());
                }
                if session.client != KugouLoginClient::Standard
                    && rows
                        .iter()
                        .any(|r| matches!(&r.wire, TrackWire::Standard(_)))
                {
                    return Err(error(
                        ErrorCode::CapabilityNotSupported,
                        "KuGou Standard track input requires its matching client profile",
                    ));
                }
                body["slow_upload"] = json!(1);
                body["scene"] = json!("false;null");
                body["data"] = json!(rows);
                match session.client {
                    KugouLoginClient::Concept => Endpoint::LibraryAddConcept,
                    _ => Endpoint::LibraryAdd,
                }
            }
            Write::Remove(ids) => {
                if ids.is_empty()
                    || ids.len() > WRITE_BATCH_SIZE
                    || ids.contains(&0)
                    || ids.iter().copied().collect::<BTreeSet<_>>().len() != ids.len()
                {
                    return Err(malformed());
                }
                body["data"] = json!(
                    ids.iter()
                        .map(|id| json!({"fileid":id}))
                        .collect::<Vec<_>>()
                );
                Endpoint::LibraryRemove
            }
        };
        let body = crypto::encode(&body)?;
        self.native_post(endpoint, session, now_ms()? / 1000, body, |bytes| {
            let ack: WireAck = data(bytes)?;
            if ack
                .userid
                .is_some_and(|v| v.0.to_string() != session.user_id)
                || ack.listid.is_some_and(|v| v.0 != list_id)
                || ack.kind.is_some_and(|v| v.0 != 0)
            {
                return Err(identity_conflict());
            }
            if ack.code.is_some_and(|v| v.0 != 0) {
                return Err(error(
                    ErrorCode::UpstreamError,
                    "KuGou track write was not fully acknowledged",
                ));
            }
            Ok(WriteAck {
                version: ack.list_ver.map(|v| v.0),
                previous_version: ack.pre_list_ver.map(|v| v.0),
                count: ack.count.map(|v| v.0),
            })
        })
        .await
    }
}

#[cfg(test)]
mod tests;
