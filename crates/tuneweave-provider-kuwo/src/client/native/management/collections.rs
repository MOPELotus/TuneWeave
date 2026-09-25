//! Playlist subscriptions use a business GET, independently confirmed by Saved reads.
use super::*;
use tuneweave_core::SubscriptionResult;

#[cfg(test)]
pub(crate) mod tests;

impl KuwoClient {
    /// Sets collection membership without changing the source playlist.
    /// This can send a write even though the upstream protocol uses GET.
    pub async fn native_set_playlist_subscription(
        &self,
        credential: &ProviderCredential,
        id: &str,
        subscribed: bool,
    ) -> Result<SubscriptionResult> {
        match self
            .native_mutation(credential, Mutation::Collection(id, subscribed, None))
            .await?
        {
            Outcome::Subscription(value) => Ok(value),
            _ => unreachable!("playlist collection mutation"),
        }
    }

    pub(super) async fn perform_native_collection(
        &self,
        input: &KuwoNativeSessionInput,
        id: &str,
        subscribed: bool,
        dispatched: &mut bool,
        check: &mut (impl FnMut() -> Result<()> + Send),
    ) -> Result<SubscriptionResult> {
        let (before, _) = self
            .native_library_items(input, Some(Section::Saved), check)
            .await?;
        if before.iter().any(|p| p.id == id) == subscribed {
            return collection_result(input, id, subscribed, false);
        }
        if subscribed && before.len() >= library::MAX_SAVED {
            return Err(kuwo_invalid_request(
                "Kuwo collection would exceed the complete directory read limit",
            ));
        }
        check()?;
        let ack = self
            .native_collection_write(input, id, subscribed, dispatched)
            .await;
        check()?;
        ack?;
        let (after, _) = self
            .native_library_items(input, Some(Section::Saved), check)
            .await?;
        if after.iter().any(|p| p.id == id) != subscribed
            || !before
                .iter()
                .filter(|p| p.id != id)
                .eq(after.iter().filter(|p| p.id != id))
        {
            return Err(unconfirmed());
        }
        collection_result(input, id, subscribed, true)
    }

    async fn native_collection_write(
        &self,
        input: &KuwoNativeSessionInput,
        id: &str,
        subscribed: bool,
        dispatched: &mut bool,
    ) -> Result<()> {
        let query = url::form_urlencoded::Serializer::new(String::new())
            .extend_pairs([
                ("op", "like"),
                ("type", "PLAYLIST"),
                ("bigid", "1"),
                ("act", if subscribed { "add" } else { "delete" }),
                ("uid", input.user_id()),
                ("sid", input.session_id()),
                ("sourceid", id),
            ])
            .finish();
        let request = self
            .http
            .get(format!(
                "{}?{query}",
                self.native_target(HOST, library::OWNED_PATH)
            ))
            .header(ACCEPT, "application/json")
            .header("Cookies", session_metadata(input)?);
        let started = Instant::now();
        let mut status = None;
        *dispatched = true;
        let result = async {
            let response = request
                .send()
                .await
                .map_err(|e| kuwo_network_error(e).retryable(false))?;
            status = Some(response.status());
            let bytes = read_response(response, false, false, ACK_LIMIT).await?;
            parse_collection_ack(&bytes, input, id, subscribed)
        }
        .await;
        self.log_upstream_request(
            "native_playlist_collection_write",
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
struct CollectionAck {
    opret: String,
    result: Option<String>,
    #[serde(default, deserialize_with = "deserialize_code")]
    errcode: Option<String>,
    #[serde(default, deserialize_with = "deserialize_code")]
    uid: Option<String>,
    #[serde(default, deserialize_with = "deserialize_code")]
    sourceid: Option<String>,
    #[serde(default, deserialize_with = "deserialize_code")]
    pid: Option<String>,
}
fn parse_collection_ack(
    bytes: &[u8],
    input: &KuwoNativeSessionInput,
    id: &str,
    subscribed: bool,
) -> Result<()> {
    let ack: CollectionAck = serde_json::from_slice(bytes).map_err(|_| invalid())?;
    if ![
        "ok",
        if subscribed {
            "collected"
        } else {
            "notcollected"
        },
    ]
    .contains(&ack.opret.as_str())
        || ack.result.as_deref().is_some_and(|v| v != "ok")
        || ack.errcode.as_deref().is_some_and(|v| v != "0")
        || ack.uid.as_deref().is_some_and(|v| v != input.user_id())
        || ack.sourceid.as_deref().is_some_and(|v| v != id)
        || ack.pid.as_deref().is_some_and(|v| v != id)
    {
        return Err(invalid());
    }
    Ok(())
}
fn collection_result(
    input: &KuwoNativeSessionInput,
    id: &str,
    subscribed: bool,
    changed: bool,
) -> Result<SubscriptionResult> {
    Ok(SubscriptionResult {
        resource_ref: ResourceRef::new(Platform::Kuwo, id).map_err(|_| invalid())?,
        subscribed,
        extensions: Extensions::from([
            ("backend".into(), json!("native_account_library")),
            ("library_owner_id".into(), json!(input.user_id())),
            ("library_section".into(), json!("collected")),
            ("confirmed".into(), json!(true)),
            ("changed".into(), json!(changed)),
            (
                "write_requests_dispatched".into(),
                json!(usize::from(changed)),
            ),
            ("atomic".into(), json!(false)),
            (
                "consistency".into(),
                json!(if changed {
                    "write_ack_and_complete_directory_readback"
                } else {
                    "complete_directory_read_no_write"
                }),
            ),
        ]),
    })
}
