//! In-memory orchestration for client-direct NOS uploads. No audio or context is persisted.
use super::*;
use rand::{TryRng, rngs::SysRng};
use std::sync::{
    Mutex,
    atomic::{AtomicBool, Ordering},
};
use tuneweave_core::{
    CloudUploadPublishMetadata, CloudUploadStep, CloudUploadStepKind, CloudUploadStepResponse,
    CloudUploadStrategy, CloudUploadTransfer, CloudUploadTransferRequest, CloudUploadTransferState,
};

const CHUNK: u64 = 4 * 1024 * 1024;
const THRESHOLD: u64 = 200 * 1024 * 1024;
const TTL: Duration = Duration::from_secs(3600);
const CAPACITY: usize = 128;
const RETRIES: u8 = 2;

#[derive(Clone, Default)]
pub(super) struct Transfers(Arc<Mutex<BTreeMap<String, Entry>>>);

#[derive(Clone)]
struct Owner {
    alias: String,
    caller: bool,
    client: NeteaseClient,
}
impl Owner {
    fn capture(provider: &NeteaseProvider, account: Option<&str>) -> Result<Self> {
        let alias = normalize_account_label(account)?.to_owned();
        let client = provider.client_for(Some(&alias))?;
        require_authenticated_client(&client, "cloud upload transfer")?;
        Ok(Self {
            alias,
            caller: provider.cloud_caller,
            client,
        })
    }
    fn check(&self, provider: &NeteaseProvider, account: Option<&str>) -> Result<()> {
        if self.alias != normalize_account_label(account)? || self.caller != provider.cloud_caller {
            return Err(error(
                ErrorCode::PermissionDenied,
                "Cloud upload belongs to a different account source",
            ));
        }
        let current = provider
            .client_for(Some(&self.alias))
            .map_err(|_| changed())?;
        if self.caller {
            if self.client.configured_cookie() != current.configured_cookie() {
                return Err(error(
                    ErrorCode::PermissionDenied,
                    "Cloud upload belongs to a different caller credential",
                ));
            }
        } else if !self.client.same_upload_session(&current) {
            return Err(changed());
        }
        Ok(())
    }
}

#[derive(Clone)]
struct Entry {
    cancelled: Arc<AtomicBool>,
    publishing: bool,
    owner: Owner,
    file: CloudUploadTicketRequest,
    ticket: Option<CloudUploadTicket>,
    deadline: Instant,
    expires_at: u64,
    offset: u64,
    chunked: bool,
    chunk_size: u64,
    context: Option<String>,
    step: Option<CloudUploadStep>,
    attempted_end: u64,
    failures: u8,
}

struct Lease {
    transfers: Transfers,
    id: String,
    armed: bool,
}
impl Drop for Lease {
    fn drop(&mut self) {
        if self.armed
            && let Ok(mut entries) = self.transfers.0.lock()
        {
            entries.remove(&self.id);
        }
    }
}
fn error(code: ErrorCode, message: &'static str) -> TuneWeaveError {
    TuneWeaveError::new(code, message).with_platform(Platform::Netease)
}
fn invalid() -> TuneWeaveError {
    error(
        ErrorCode::InvalidRequest,
        "Invalid cloud upload transfer action",
    )
}
fn missing() -> TuneWeaveError {
    error(
        ErrorCode::ResourceNotFound,
        "Cloud upload transfer is missing, expired, or consumed",
    )
}
fn changed() -> TuneWeaveError {
    error(ErrorCode::Conflict, "Cloud upload account changed")
}
fn malformed() -> TuneWeaveError {
    error(
        ErrorCode::UpstreamError,
        "Cloud upload response did not confirm the planned operation",
    )
}
fn nonce() -> Result<String> {
    let mut bytes = [0; 32];
    SysRng.try_fill_bytes(&mut bytes).map_err(|_| {
        error(
            ErrorCode::InternalError,
            "Cannot create cloud upload handle",
        )
    })?;
    Ok(hex::encode(bytes))
}
fn prune(entries: &mut BTreeMap<String, Entry>) {
    entries.retain(|_, e| e.deadline > Instant::now());
}

impl Entry {
    fn view(&self, id: &str) -> Result<CloudUploadTransfer> {
        if self.publishing {
            return Err(error(
                ErrorCode::Conflict,
                "Cloud upload publication is in progress",
            ));
        }
        let ticket = self.ticket.as_ref().ok_or_else(|| {
            error(
                ErrorCode::Conflict,
                "Cloud upload allocation is in progress",
            )
        })?;
        Ok(CloudUploadTransfer {
            transfer_id: id.into(),
            state: if self.offset == self.file.file_size {
                CloudUploadTransferState::ReadyToPublish
            } else {
                CloudUploadTransferState::Transferring
            },
            file_size: self.file.file_size,
            offset: self.offset,
            expires_at: self.expires_at,
            upload_required: ticket.upload_required,
            step: self.step.clone(),
        })
    }

    fn plan(&mut self, kind: CloudUploadStepKind, delay: u64) -> Result<()> {
        let ticket = self.ticket.as_ref().ok_or_else(malformed)?;
        if self.offset == self.file.file_size {
            self.step = None;
            return Ok(());
        }
        let mut url = Url::parse(&ticket.upload_url).map_err(|_| malformed())?;
        // Tickets originate exclusively from the existing validated NOS allocation.
        if !matches!(url.scheme(), "http" | "https")
            || !url.host_str().is_some_and(|h| h.ends_with(".127.net"))
            || !url.username().is_empty()
            || url.password().is_some()
            || url.port().is_some()
            || url.fragment().is_some()
        {
            return Err(malformed());
        }
        // Discovery advertises HTTP; use the verified NOS HTTPS transport exclusively.
        url.set_scheme("https").map_err(|_| malformed())?;
        url.set_query(None);
        let length = if kind == CloudUploadStepKind::Probe {
            0
        } else if self.chunked {
            self.chunk_size.min(self.file.file_size - self.offset)
        } else {
            self.file.file_size
        };
        let mut headers = BTreeMap::from([(
            "x-nos-token".into(),
            ticket
                .upload_headers
                .get("x-nos-token")
                .ok_or_else(malformed)?
                .clone(),
        )]);
        if kind == CloudUploadStepKind::Probe {
            url.set_query(Some("uploadContext"));
            url.query_pairs_mut()
                .append_pair("version", "1.0")
                .append_pair("context", self.context.as_deref().ok_or_else(malformed)?);
        } else {
            let end = self.offset.checked_add(length).ok_or_else(malformed)?;
            self.attempted_end = end;
            url.query_pairs_mut()
                .append_pair("offset", &self.offset.to_string())
                .append_pair(
                    "complete",
                    if end == self.file.file_size {
                        "true"
                    } else {
                        "false"
                    },
                )
                .append_pair("version", "1.0");
            if let Some(context) = &self.context {
                url.query_pairs_mut().append_pair("context", context);
            }
            headers.insert("Content-Length".into(), length.to_string());
            headers.insert(
                "Content-Type".into(),
                ticket
                    .upload_headers
                    .get("Content-Type")
                    .ok_or_else(malformed)?
                    .clone(),
            );
            if !self.chunked {
                headers.insert(
                    "Content-MD5".into(),
                    ticket
                        .upload_headers
                        .get("Content-MD5")
                        .ok_or_else(malformed)?
                        .clone(),
                );
            }
        }
        if headers
            .values()
            .any(|v| v.len() > 8192 || reqwest::header::HeaderValue::from_str(v).is_err())
        {
            return Err(malformed());
        }
        self.step = Some(CloudUploadStep {
            step_id: nonce()?,
            kind,
            offset: self.offset,
            length,
            method: if kind == CloudUploadStepKind::Probe {
                "GET"
            } else {
                "POST"
            }
            .into(),
            url: url.into(),
            headers,
            retry_delay_ms: delay,
            attempts_remaining: RETRIES.saturating_sub(self.failures) + 1,
        });
        Ok(())
    }

    fn advance(&mut self, response: &CloudUploadStepResponse) -> Result<()> {
        let step = self.step.as_ref().ok_or_else(invalid)?.clone();
        if response.status == Some(413) && step.kind == CloudUploadStepKind::Upload {
            // An explicit rejection permits switching from one request to smaller slices.
            if !self.chunked {
                self.chunked = true;
                self.chunk_size = (step.length / 2).clamp(1, CHUNK);
            } else if step.length > 64 * 1024 {
                self.chunk_size = (step.length / 2).max(64 * 1024);
            } else {
                return Err(error(
                    ErrorCode::UpstreamError,
                    "Cloud upload storage rejected the minimum slice size",
                ));
            }
            return self.plan(CloudUploadStepKind::Upload, 0);
        }
        if response.status.is_none()
            || response
                .status
                .is_some_and(|s| matches!(s, 408 | 425 | 429 | 500..=599))
        {
            if self.failures >= RETRIES {
                return Err(error(
                    ErrorCode::UpstreamError,
                    "Cloud upload retry limit reached",
                ));
            }
            self.failures += 1;
            let delay = retry_delay(response, self.failures)?;
            // Probe the exact existing resource/context before resending uncertain bytes.
            return self.plan(
                if self.context.is_some() {
                    CloudUploadStepKind::Probe
                } else {
                    CloudUploadStepKind::Upload
                },
                delay,
            );
        }
        if response.status != Some(200) {
            return Err(error(
                ErrorCode::UpstreamError,
                "Cloud upload storage rejected the operation",
            ));
        }
        let ack = acknowledgement(response)?;
        if step.kind == CloudUploadStepKind::Upload {
            if ack.offset != self.attempted_end || ack.offset <= self.offset {
                return Err(malformed());
            }
        } else if ack.offset < self.offset || ack.offset > self.attempted_end {
            return Err(malformed());
        }
        if let Some(context) = ack.context {
            self.context = Some(context);
        }
        if ack.offset < self.file.file_size && self.context.is_none() {
            return Err(malformed());
        }
        self.offset = ack.offset;
        if step.kind == CloudUploadStepKind::Upload {
            self.failures = 0;
        }
        self.plan(CloudUploadStepKind::Upload, 0)
    }
}

struct Ack {
    offset: u64,
    context: Option<String>,
}
fn acknowledgement(response: &CloudUploadStepResponse) -> Result<Ack> {
    #[derive(Deserialize)]
    struct Body {
        offset: u64,
        context: Option<String>,
        #[serde(rename = "errCode")]
        err_code: Option<Value>,
        code: Option<Value>,
    }
    let body: Body = serde_json::from_str(&response.body).map_err(|_| malformed())?;
    if [body.err_code, body.code]
        .into_iter()
        .flatten()
        .any(|v| v != json!(0))
    {
        return Err(malformed());
    }
    let values: Vec<_> = response
        .headers
        .iter()
        .filter(|(k, _)| k.eq_ignore_ascii_case("x-nos-context"))
        .map(|(_, v)| v.as_str())
        .collect();
    if values.len() > 1
        || body
            .context
            .as_deref()
            .zip(values.first().copied())
            .is_some_and(|(a, b)| a != b)
    {
        return Err(malformed());
    }
    let context = body
        .context
        .or_else(|| values.first().map(|s| (*s).to_owned()));
    if context
        .as_ref()
        .is_some_and(|v| v.is_empty() || v.len() > 8192 || !v.bytes().all(|b| b.is_ascii_graphic()))
    {
        return Err(malformed());
    }
    Ok(Ack {
        offset: body.offset,
        context,
    })
}
fn retry_delay(response: &CloudUploadStepResponse, attempts: u8) -> Result<u64> {
    let mut delay = u64::from(attempts) * 500;
    for (key, value) in &response.headers {
        if key.eq_ignore_ascii_case("retry-after") {
            // Unknown/date hints are not guessed. A long explicit wait cannot outlive this retry policy.
            if let Ok(seconds) = value.parse::<u64>() {
                if seconds > 30 {
                    return Err(error(
                        ErrorCode::RateLimited,
                        "Cloud upload retry delay exceeds the transfer policy",
                    ));
                }
                delay = delay.max(seconds * 1000);
            }
        }
    }
    Ok(delay)
}

impl NeteaseProvider {
    pub(super) fn cancel_server_uploads(&self, account: &str) -> Result<()> {
        let mut entries = self
            .cloud_transfers
            .0
            .lock()
            .map_err(|_| account_store_error())?;
        entries.retain(|_, entry| {
            let matches = !entry.owner.caller && entry.owner.alias == account;
            if matches {
                entry.cancelled.store(true, Ordering::SeqCst);
            }
            !matches
        });
        Ok(())
    }
    pub(super) fn cancel_caller_uploads(&self, client: &NeteaseClient) -> Result<()> {
        let mut entries = self
            .cloud_transfers
            .0
            .lock()
            .map_err(|_| account_store_error())?;
        entries.retain(|_, entry| {
            let matches = entry.owner.caller
                && entry.owner.client.configured_cookie() == client.configured_cookie();
            if matches {
                entry.cancelled.store(true, Ordering::SeqCst);
            }
            !matches
        });
        Ok(())
    }
    pub(super) async fn begin_upload_transfer(
        &self,
        request: &CloudUploadTransferRequest,
    ) -> Result<CloudUploadTransfer> {
        validate_cloud_upload_ticket_request(&request.file)?;
        let owner = Owner::capture(self, request.file.account.as_deref())?;
        let id = nonce()?;
        let mut lease = Lease {
            transfers: self.cloud_transfers.clone(),
            id: id.clone(),
            armed: true,
        };
        {
            let mut entries = self
                .cloud_transfers
                .0
                .lock()
                .map_err(|_| account_store_error())?;
            prune(&mut entries);
            if entries.len() >= CAPACITY {
                return Err(error(
                    ErrorCode::RateLimited,
                    "Too many pending cloud upload transfers",
                ));
            }
            entries.insert(
                id.clone(),
                Entry {
                    cancelled: Arc::new(AtomicBool::new(false)),
                    publishing: false,
                    owner: owner.clone(),
                    file: request.file.clone(),
                    ticket: None,
                    deadline: Instant::now() + TTL,
                    expires_at: SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .map_err(|_| account_store_error())?
                        .as_secs()
                        + TTL.as_secs(),
                    offset: 0,
                    chunked: request.strategy == CloudUploadStrategy::Chunked
                        || request.file.file_size > THRESHOLD,
                    chunk_size: CHUNK,
                    context: None,
                    step: None,
                    attempted_end: 0,
                    failures: 0,
                },
            );
        }
        let ticket = self.cloud_upload_ticket(&request.file).await;
        owner.check(self, request.file.account.as_deref())?;
        let ticket = ticket?;
        let mut entries = self
            .cloud_transfers
            .0
            .lock()
            .map_err(|_| account_store_error())?;
        prune(&mut entries);
        let entry = entries.get_mut(&id).ok_or_else(missing)?;
        if !ticket.upload_required {
            entry.offset = entry.file.file_size;
        }
        entry.ticket = Some(ticket);
        entry.plan(CloudUploadStepKind::Upload, 0)?;
        let result = entry.view(&id)?;
        lease.armed = false;
        Ok(result)
    }

    pub(super) fn read_upload_transfer(
        &self,
        id: &str,
        account: Option<&str>,
    ) -> Result<CloudUploadTransfer> {
        let mut entries = self
            .cloud_transfers
            .0
            .lock()
            .map_err(|_| account_store_error())?;
        prune(&mut entries);
        let entry = entries.get(id).ok_or_else(missing)?;
        entry.owner.check(self, account)?;
        entry.view(id)
    }
    pub(super) fn cancel_upload_transfer(&self, id: &str, account: Option<&str>) -> Result<bool> {
        let mut entries = self
            .cloud_transfers
            .0
            .lock()
            .map_err(|_| account_store_error())?;
        prune(&mut entries);
        let entry = entries.get(id).ok_or_else(missing)?;
        entry.owner.check(self, account)?;
        entry.cancelled.store(true, Ordering::SeqCst);
        entries.remove(id);
        Ok(true)
    }
    pub(super) fn advance_upload_transfer(
        &self,
        id: &str,
        response: &CloudUploadStepResponse,
        account: Option<&str>,
    ) -> Result<CloudUploadTransfer> {
        response.validate()?;
        let mut entries = self
            .cloud_transfers
            .0
            .lock()
            .map_err(|_| account_store_error())?;
        prune(&mut entries);
        let entry = entries.get_mut(id).ok_or_else(missing)?;
        entry.owner.check(self, account)?;
        if entry
            .step
            .as_ref()
            .is_none_or(|step| step.step_id != response.step_id)
        {
            return Err(invalid());
        }
        match entry.advance(response).and_then(|()| entry.view(id)) {
            Ok(result) => Ok(result),
            Err(error) => {
                entries.remove(id);
                Err(error
                    .retryable(false)
                    .with_details(json!({"transfer_consumed":true,"published":false})))
            }
        }
    }
    pub(super) async fn publish_upload_transfer(
        &self,
        id: &str,
        metadata: &CloudUploadPublishMetadata,
        account: Option<&str>,
    ) -> Result<CloudUploadResult> {
        let (entry, request) = {
            let mut entries = self
                .cloud_transfers
                .0
                .lock()
                .map_err(|_| account_store_error())?;
            prune(&mut entries);
            let entry = entries.get_mut(id).ok_or_else(missing)?;
            entry.owner.check(self, account)?;
            if entry.publishing || entry.offset != entry.file.file_size || entry.step.is_some() {
                return Err(error(
                    ErrorCode::Conflict,
                    "Cloud upload has not finished transferring",
                ));
            }
            let ticket = entry.ticket.as_ref().ok_or_else(malformed)?;
            let request = CloudUploadCompleteRequest {
                provisional_track_id: ticket.provisional_track_id.clone().ok_or_else(malformed)?,
                resource_id: ticket.resource_id.clone(),
                md5: entry.file.md5.clone(),
                filename: entry.file.filename.clone(),
                bitrate: entry.file.bitrate,
                account: Some(entry.owner.alias.clone()),
                song_name: metadata.song_name.clone(),
                artist: metadata.artist.clone(),
                album: metadata.album.clone(),
            };
            validate_cloud_upload_complete_request(&request)?;
            entry.publishing = true;
            (entry.clone(), request)
        };
        let _lease = Lease {
            transfers: self.cloud_transfers.clone(),
            id: id.to_owned(),
            armed: true,
        };
        // Claim once before metadata writes. Errors/cancellation never automatically republish.
        let check = || {
            if entry.cancelled.load(Ordering::SeqCst) {
                return Err(error(ErrorCode::Conflict, "Cloud upload was cancelled"));
            }
            if entry.deadline <= Instant::now() {
                return Err(missing());
            }
            entry.owner.check(self, account)
        };
        let result = async {
            check()?;
            let descriptor = validate_cloud_upload_complete_request(&request)?;
            let client = &entry.owner.client;
            let response = client.request_eapi("/api/upload/cloud/info/v2", json!({
                "md5":descriptor.md5,"songid":descriptor.provisional_track_id,"filename":descriptor.filename,
                "song":descriptor.song_name,"album":descriptor.album,"artist":descriptor.artist,
                "bitrate":request.bitrate.to_string(),"resourceId":descriptor.resource_id
            })).await;
            check()?;
            let response = response?;
            ensure_account_access(client, &response.body, "cloud upload completion")?;
            let track_id = response.body.get("songId").and_then(json_scalar_string).unwrap_or_else(|| descriptor.provisional_track_id.to_owned());
            let published = client.request_eapi("/api/cloud/pub/v2", json!({"songid":track_id})).await;
            check()?;
            let published = published?;
            ensure_account_access(client, &published.body, "cloud upload publication")?;
            let uploaded = entry.ticket.as_ref().ok_or_else(malformed)?.upload_required;
            map_cloud_upload_result(track_id, Some(uploaded), Some(uploaded), response.body, published.body)
        }.await;
        result.map_err(|e| {
            e.retryable(false)
                .with_details(json!({"transfer_consumed":true,"publish_outcome":"unconfirmed"}))
        })
    }
}

#[cfg(test)]
mod tests;
