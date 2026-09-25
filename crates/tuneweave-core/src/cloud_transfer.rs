//! Client-direct uploads with provider-owned request planning and response parsing.
//! A transfer lives in memory and is bound to its original file and account.
use crate::{CloudUploadTicketRequest, Result, TuneWeaveError};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fmt};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CloudUploadStrategy {
    #[default]
    Auto,
    Chunked,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CloudUploadTransferRequest {
    pub file: CloudUploadTicketRequest,
    pub strategy: CloudUploadStrategy,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CloudUploadTransferState {
    Transferring,
    ReadyToPublish,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CloudUploadStepKind {
    Upload,
    Probe,
}

/// Execute exactly this request, without redirects or application credentials.
/// Upload the original file's [offset, offset + length) bytes; probes have no body.
#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct CloudUploadStep {
    pub step_id: String,
    pub kind: CloudUploadStepKind,
    pub offset: u64,
    pub length: u64,
    pub method: String,
    pub url: String,
    pub headers: BTreeMap<String, String>,
    pub retry_delay_ms: u64,
    pub attempts_remaining: u8,
}
impl fmt::Debug for CloudUploadStep {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CloudUploadStep")
            .field("kind", &self.kind)
            .field("offset", &self.offset)
            .field("length", &self.length)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct CloudUploadTransfer {
    pub transfer_id: String,
    pub state: CloudUploadTransferState,
    pub file_size: u64,
    pub offset: u64,
    pub expires_at: u64,
    pub upload_required: bool,
    pub step: Option<CloudUploadStep>,
}
impl fmt::Debug for CloudUploadTransfer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CloudUploadTransfer")
            .field("state", &self.state)
            .field("file_size", &self.file_size)
            .field("offset", &self.offset)
            .field("step", &self.step)
            .finish_non_exhaustive()
    }
}

/// The client reports the storage response verbatim, not a guessed next offset.
/// None status denotes a transport failure; headers/body must then be empty.
#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CloudUploadStepResponse {
    pub step_id: String,
    pub status: Option<u16>,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    #[serde(default)]
    pub body: String,
}
impl CloudUploadStepResponse {
    pub fn validate(&self) -> Result<()> {
        if self.step_id.is_empty()
            || self.step_id.len() > 128
            || self.status.is_some_and(|s| !(100..=599).contains(&s))
            || self.body.len() > 64 * 1024
            || self.headers.len() > 64
            || self
                .headers
                .iter()
                .map(|(k, v)| k.len() + v.len())
                .sum::<usize>()
                > 16 * 1024
            || (self.status.is_none() && (!self.body.is_empty() || !self.headers.is_empty()))
        {
            return Err(TuneWeaveError::invalid_request(
                "Invalid cloud upload response envelope",
            ));
        }
        Ok(())
    }
}
impl fmt::Debug for CloudUploadStepResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CloudUploadStepResponse")
            .field("status", &self.status)
            .finish_non_exhaustive()
    }
}

/// File/resource/account fields remain bound to the transfer, not supplied again.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CloudUploadPublishMetadata {
    pub song_name: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
}
