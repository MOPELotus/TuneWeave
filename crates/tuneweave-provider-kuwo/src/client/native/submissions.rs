//! Official account submission records. Review and publication are separate states.
use super::*;
use sha1::{Digest, Sha1};
use tuneweave_core::{Page, PageMeta, PageRequest, PlaylistSubmission, PlaylistSubmissionStatus};

#[cfg(test)]
pub(crate) mod tests;

const HOST: &str = "wapi.kuwo.cn";
const PATH: &str = "/api/mobicase/playlist/userTgRecordList";
const PAGE_SIZE: usize = 6;

#[derive(Clone, Copy)]
pub(crate) struct Limits {
    pub(crate) pages: u32,
    pub(crate) response_bytes: usize,
    pub(crate) total_bytes: usize,
    pub(crate) budget: Duration,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            pages: 256,
            response_bytes: 1024 * 1024,
            total_bytes: 16 * 1024 * 1024,
            budget: Duration::from_secs(120),
        }
    }
}

impl KuwoClient {
    pub(crate) fn submission_limits(&self) -> Limits {
        #[cfg(test)]
        if let Some(limits) = self.native_submission_limits {
            return limits;
        }
        Limits::default()
    }

    /// Reads a verified account's complete submission records twice before slicing.
    /// Approved review records do not prove current publication or media rights.
    pub async fn native_playlist_submissions(
        &self,
        credential: &tuneweave_core::ProviderCredential,
        request: &PageRequest,
    ) -> Result<Page<PlaylistSubmission>> {
        library::validate_request(request)?;
        if request.account.as_deref().is_some_and(|v| v != "default") {
            return Err(kuwo_invalid_request(
                "Kuwo SDK credentials cannot select a stored account",
            ));
        }
        let input = credential::NativeCredential::parse(credential)?.input()?;
        validate_session_metadata(&input)?;
        tokio::time::timeout(self.submission_limits().budget, async {
            self.validate_native_session(&input).await?;
            self.fetch_native_submissions(&input, request, || Ok(()))
                .await
        })
        .await
        .map_err(|_| timeout())?
    }

    pub(crate) async fn fetch_native_submissions(
        &self,
        input: &KuwoNativeSessionInput,
        request: &PageRequest,
        mut check: impl FnMut() -> Result<()> + Send,
    ) -> Result<Page<PlaylistSubmission>> {
        let (second, snapshot, upstream_pages) = self
            .fetch_complete_native_submissions(input, &mut check)
            .await?;
        let total = second.len() as u64;
        let items: Vec<_> = second
            .into_iter()
            .skip(request.offset as usize)
            .take(request.limit as usize)
            .collect();
        let end = request.offset + items.len() as u32;
        let has_more = u64::from(end) < total;
        Ok(Page {
            items,
            pagination: PageMeta {
                limit: request.limit,
                offset: request.offset,
                total: Some(total),
                has_more,
                next_offset: has_more.then_some(end),
                extensions: Extensions::from([
                    ("backend".into(), json!("native_playlist_submissions")),
                    ("source_user_id".into(), json!(input.user_id())),
                    ("complete_read".into(), json!(true)),
                    ("consistency".into(), json!("two_complete_reads")),
                    ("source_snapshot_id".into(), json!(snapshot)),
                    ("upstream_page_size".into(), json!(PAGE_SIZE)),
                    ("upstream_pages_fetched".into(), json!(upstream_pages)),
                ]),
            },
        })
    }

    pub(crate) async fn fetch_complete_native_submissions(
        &self,
        input: &KuwoNativeSessionInput,
        mut check: impl FnMut() -> Result<()> + Send,
    ) -> Result<(Vec<PlaylistSubmission>, String, u32)> {
        let limits = self.submission_limits();
        let mut remaining = limits.total_bytes;
        let (first, first_pages) = self
            .submission_scan(input, limits, &mut remaining, &mut check)
            .await?;
        let (second, second_pages) = self
            .submission_scan(input, limits, &mut remaining, &mut check)
            .await?;
        if first != second || first_pages != second_pages {
            return Err(TuneWeaveError::new(
                ErrorCode::Conflict,
                "Kuwo submission records changed during reading",
            )
            .with_platform(Platform::Kuwo));
        }
        check()?;
        // Local content identity, not an upstream version or an atomic snapshot.
        let encoded = serde_json::to_vec(&(input.user_id(), &second)).map_err(|_| malformed())?;
        let mut snapshot = String::from("kuwo-submissions-");
        for byte in Sha1::digest(encoded) {
            write!(&mut snapshot, "{byte:02x}").expect("writing to a string cannot fail");
        }
        Ok((second, snapshot, first_pages + second_pages))
    }

    async fn submission_scan(
        &self,
        input: &KuwoNativeSessionInput,
        limits: Limits,
        remaining: &mut usize,
        check: &mut (impl FnMut() -> Result<()> + Send),
    ) -> Result<(Vec<PlaylistSubmission>, u32)> {
        let mut items = Vec::new();
        let mut full_pages = std::collections::BTreeSet::new();
        for pn in 1..=limits.pages {
            if *remaining == 0 {
                return Err(malformed());
            }
            check()?;
            let result = self
                .submission_page(input, pn, limits.response_bytes.min(*remaining))
                .await;
            check()?;
            let (page, bytes) = result?;
            *remaining = remaining.checked_sub(bytes).ok_or_else(malformed)?;
            let complete = page.len() < PAGE_SIZE;
            // Preserve repeated records; reject a repeated full physical page,
            // which cannot be distinguished from an upstream pagination loop.
            if !complete && !full_pages.insert(serde_json::to_vec(&page).map_err(|_| malformed())?)
            {
                return Err(malformed());
            }
            items.extend(page);
            if complete {
                return Ok((items, pn));
            }
        }
        Err(malformed())
    }

    async fn submission_page(
        &self,
        input: &KuwoNativeSessionInput,
        pn: u32,
        max_bytes: usize,
    ) -> Result<(Vec<PlaylistSubmission>, usize)> {
        let target = {
            let mut query = url::form_urlencoded::Serializer::new(String::new());
            query.extend_pairs([
                ("loginUid", input.user_id()),
                ("sid", input.session_id()),
                ("pn", &pn.to_string()),
                ("rn", "6"),
            ]);
            format!("{}?{}", self.native_target(HOST, PATH), query.finish())
        };
        let started = Instant::now();
        let mut status = None;
        let result = async {
            let response = self
                .http
                .get(target)
                .header(ACCEPT, "application/json")
                .send()
                .await
                .map_err(|e| kuwo_network_error(e).retryable(false))?;
            status = Some(response.status());
            let bytes = read_response(response, false, false, max_bytes).await?;
            Ok((parse(&bytes, input)?, bytes.len()))
        }
        .await;
        self.log_upstream_request(
            "native_playlist_submissions",
            HOST,
            PATH,
            status,
            started,
            0,
            false,
            &result,
        );
        result
    }
}

#[derive(Deserialize)]
struct Envelope {
    #[serde(default, deserialize_with = "deserialize_code")]
    code: Option<String>,
    data: Option<Data>,
}
#[derive(Deserialize)]
struct Data {
    list: Vec<Wire>,
}
#[derive(Deserialize)]
struct Wire {
    #[serde(deserialize_with = "deserialize_uid")]
    uid: String,
    #[serde(deserialize_with = "deserialize_code")]
    id: Option<String>,
    name: Option<String>,
    pic: Option<String>,
    #[serde(default, deserialize_with = "library::dto::unsigned")]
    total: Option<u64>,
    #[serde(default, deserialize_with = "library::dto::unsigned")]
    listencnt: Option<u64>,
    #[serde(default, deserialize_with = "library::dto::unsigned")]
    digest: Option<u64>,
    #[serde(default, deserialize_with = "deserialize_code")]
    status: Option<String>,
}

fn parse(bytes: &[u8], input: &KuwoNativeSessionInput) -> Result<Vec<PlaylistSubmission>> {
    let envelope: Envelope = serde_json::from_slice(bytes).map_err(|_| malformed())?;
    if envelope.code.as_deref() == Some("-1001") {
        return Err(authentication_required());
    }
    if envelope.code.as_deref().is_some_and(|code| code != "200") {
        return Err(malformed());
    }
    let data = envelope.data.ok_or_else(malformed)?;
    if data.list.len() > PAGE_SIZE {
        return Err(malformed());
    }
    data.list
        .into_iter()
        .map(|row| {
            if row.uid != input.user_id() {
                return Err(malformed());
            }
            let id = row.id.ok_or_else(malformed)?;
            playlist::validate_id(&id).map_err(|_| malformed())?;
            let code = row
                .status
                .map(|v| {
                    v.parse::<i32>()
                        .ok()
                        .filter(|n| n.to_string() == v)
                        .ok_or_else(malformed)
                })
                .transpose()?;
            let review_status = match code {
                Some(0) => PlaylistSubmissionStatus::Pending,
                Some(1) => PlaylistSubmissionStatus::Approved,
                Some(2) => PlaylistSubmissionStatus::Rejected,
                _ => PlaylistSubmissionStatus::Unknown,
            };
            Ok(PlaylistSubmission {
                playlist_ref: ResourceRef::new(Platform::Kuwo, id).map_err(|_| malformed())?,
                owner_id: row.uid,
                name: library::dto::text(row.name, 1024, false, input)?,
                cover_url: library::dto::picture(row.pic, input)?,
                track_count: row.total,
                play_count: row.listencnt,
                review_status,
                published: None,
                extensions: Extensions::from([
                    ("upstream_review_status".into(), json!(code)),
                    ("upstream_digest".into(), json!(row.digest)),
                ]),
            })
        })
        .collect()
}
fn malformed() -> TuneWeaveError {
    kuwo_upstream_error("Kuwo submission records are invalid or incomplete")
}
pub(crate) fn timeout() -> TuneWeaveError {
    TuneWeaveError::new(
        ErrorCode::UpstreamTimeout,
        "Kuwo submission reading exceeded its total deadline",
    )
    .with_platform(Platform::Kuwo)
    .retryable(false)
}
