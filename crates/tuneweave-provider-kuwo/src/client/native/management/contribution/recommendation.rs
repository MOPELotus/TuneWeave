//! The recommendation form uses text validation and its own metadata-save API.
use super::*;
use crate::client::native::profile;

#[cfg(test)]
pub(crate) mod tests;

pub(super) struct Author(String);
impl std::fmt::Debug for Author {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Author([redacted])")
    }
}

pub(super) fn validate(value: Option<&str>) -> Result<()> {
    if value.is_some_and(|v| {
        v.is_empty()
            || v.encode_utf16().count() > 5
            || v.chars().any(char::is_control)
            || v.trim_matches(|c| (c as u32) <= 32) != v
    }) {
        return Err(kuwo_invalid_request(
            "Kuwo recommendation must contain 1 to 5 Java UTF-16 units without surrounding whitespace or control characters",
        ));
    }
    Ok(())
}

impl KuwoClient {
    pub(super) async fn prepare_recommended_submission(
        &self,
        input: &KuwoNativeSessionInput,
        id: &str,
        metadata: &Metadata,
        recommendation: &str,
        check: &mut (impl FnMut() -> Result<()> + Send),
    ) -> Result<Author> {
        check()?;
        let author = self.submission_author(input).await;
        check()?;
        let author = author?;
        // RecommendEditorView's ID field is zero-initialized; no setter call
        // exists in the pinned APK. It validates intro text separately from the
        // selected playlist's name/description request on submit.
        let result = self
            .check_submission_text(input, "0", &[("intro", recommendation)])
            .await;
        check()?;
        result?;
        let result = self
            .check_submission_text(
                input,
                id,
                &[("name", &metadata.name), ("intro", &metadata.description)],
            )
            .await;
        check()?;
        result?;
        Ok(author)
    }

    async fn submission_author(&self, input: &KuwoNativeSessionInput) -> Result<Author> {
        let key = self.native_response_key()?;
        let target = format!(
            "{}?f=ar&q={}",
            self.native_target(EXCHANGE_HOST, profile::PATH),
            codec::seal_query(profile::query(input, &key).as_bytes())?
        );
        self.native_get_with_metadata(
            EXCHANGE_HOST,
            profile::PATH,
            "native_submission_author",
            target,
            Some(session_metadata(input)?),
            |body| parse_author(&codec::open_response(body, &key)?, input),
        )
        .await
    }

    async fn check_submission_text(
        &self,
        input: &KuwoNativeSessionInput,
        id: &str,
        fields: &[(&str, &str)],
    ) -> Result<()> {
        let query = {
            let mut q = url::form_urlencoded::Serializer::new(String::new());
            q.extend_pairs([
                ("op", "verifykeyword"),
                ("encode", "utf-8"),
                ("plat", "ar"),
                ("uid", input.user_id()),
            ]);
            q.finish()
        };
        let payload = json!({"playlistid":id.parse::<i64>().map_err(|_|invalid())?,
            "list":fields.iter().map(|(kind,content)|json!({"type":kind,"content":content})).collect::<Vec<_>>()});
        let bytes = self
            .recommendation_post("native_submission_text_check", query, payload, None)
            .await?;
        parse_keyword(&bytes, id, fields)
    }

    pub(super) async fn save_recommended_submission_metadata(
        &self,
        input: &KuwoNativeSessionInput,
        id: &str,
        metadata: &Metadata,
        playlist: &Playlist,
        author: &Author,
        dispatched: &mut bool,
    ) -> Result<()> {
        let query = {
            let mut q = url::form_urlencoded::Serializer::new(String::new());
            q.extend_pairs([
                ("op", "updatelistinfo"),
                ("encode", "utf-8"),
                ("pid", id),
                ("uid", input.user_id()),
                ("sid", input.session_id()),
            ]);
            q.finish()
        };
        let payload = json!({"name":metadata.name,"info":metadata.description,"tag":metadata.tags.join(","),
            "pic":playlist.cover_url,"ispub":true,"uname":author.0});
        let bytes = self
            .recommendation_post(
                "native_submission_metadata_save",
                query,
                payload,
                Some(dispatched),
            )
            .await?;
        let ack: SaveAck = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
        if !ack.opret.eq_ignore_ascii_case("ok") {
            return Err(invalid());
        }
        Ok(())
    }

    async fn recommendation_post(
        &self,
        operation: &'static str,
        query: String,
        payload: serde_json::Value,
        dispatched: Option<&mut bool>,
    ) -> Result<Vec<u8>> {
        let body = serde_json::to_vec(&payload).map_err(|_| invalid())?;
        let request = self
            .http
            .post(format!(
                "{}?{query}",
                self.native_target(HOST, library::OWNED_PATH)
            ))
            .header(ACCEPT, "application/json")
            .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(body);
        let started = Instant::now();
        let mut status = None;
        if let Some(dispatched) = dispatched {
            *dispatched = true;
        }
        let result = async {
            let response = request
                .send()
                .await
                .map_err(|e| kuwo_network_error(e).retryable(false))?;
            status = Some(response.status());
            // The official anonymous keyword endpoint returns JSON as text/html.
            // JSON parsing remains mandatory; HTML documents never count as ACKs.
            read_response(response, true, false, ACK_LIMIT).await
        }
        .await;
        self.log_upstream_request(
            operation,
            HOST,
            library::OWNED_PATH,
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
struct AuthorEnvelope {
    #[serde(default, deserialize_with = "deserialize_code")]
    status: Option<String>,
    info: Vec<AuthorFields>,
}
#[derive(Deserialize)]
struct AuthorFields {
    #[serde(rename = "UID", deserialize_with = "deserialize_uid")]
    uid: String,
    #[serde(rename = "NAME")]
    name: Option<String>,
}
fn parse_author(bytes: &[u8], input: &KuwoNativeSessionInput) -> Result<Author> {
    let envelope: AuthorEnvelope = serde_json::from_slice(bytes).map_err(|_| invalid())?;
    if envelope.status.as_deref() != Some("200") || envelope.info.len() != 1 {
        return Err(invalid());
    }
    let value = envelope.info.into_iter().next().ok_or_else(invalid)?;
    let name = value.name.ok_or_else(invalid)?;
    if value.uid != input.user_id()
        || name.trim().is_empty()
        || name.len() > 1024
        || name.chars().any(char::is_control)
        || echoes_secret(&name, input.session_id())
    {
        return Err(invalid());
    }
    Ok(Author(name))
}
#[derive(Deserialize)]
struct KeywordAck {
    result: String,
    #[serde(deserialize_with = "deserialize_code")]
    playlistid: Option<String>,
    list: Vec<KeywordField>,
}
#[derive(Deserialize)]
struct KeywordField {
    #[serde(rename = "type")]
    kind: String,
    content: String,
    issensitive: bool,
}
fn parse_keyword(bytes: &[u8], id: &str, fields: &[(&str, &str)]) -> Result<()> {
    let ack: KeywordAck = serde_json::from_slice(bytes).map_err(|_| invalid())?;
    if !ack.result.eq_ignore_ascii_case("ok")
        || ack.playlistid.as_deref() != Some(id)
        || ack.list.len() != fields.len()
    {
        return Err(invalid());
    }
    let mut seen = std::collections::BTreeSet::new();
    for row in &ack.list {
        if !seen.insert(&row.kind)
            || !fields
                .iter()
                .any(|(kind, value)| *kind == row.kind && *value == row.content)
        {
            return Err(invalid());
        }
    }
    if ack.list.iter().any(|r| r.issensitive) {
        return Err(TuneWeaveError::new(
            ErrorCode::PermissionDenied,
            "Kuwo rejected submission text",
        )
        .with_platform(Platform::Kuwo));
    }
    Ok(())
}
#[derive(Deserialize)]
struct SaveAck {
    opret: String,
}
fn invalid() -> TuneWeaveError {
    kuwo_upstream_error("Kuwo recommendation submission response is invalid or incomplete")
}
