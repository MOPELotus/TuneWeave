use super::*;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

fn file(size: u64) -> CloudUploadTicketRequest {
    CloudUploadTicketRequest::new("0123456789abcdef0123456789abcdef", size, "original.flac")
}
fn provider() -> NeteaseProvider {
    NeteaseProvider::new(NeteaseConfig {
        cookie: Some("MUSIC_U=private-account-cookie".into()),
        ..Default::default()
    })
    .unwrap()
}
fn seeded(
    provider: &NeteaseProvider,
    size: u64,
    chunked: bool,
    account: Option<&str>,
) -> CloudUploadTransfer {
    let id = nonce().unwrap();
    let mut entry = Entry {
        cancelled: Arc::new(AtomicBool::new(false)),
        publishing: false,
        owner: Owner::capture(provider, account).unwrap(),
        file: file(size),
        ticket: Some(CloudUploadTicket {
            upload_required: true,
            provisional_track_id: Some("123".into()),
            resource_id: "456".into(),
            upload_method: "POST".into(),
            upload_url: format!(
                "http://nosup-jd1.127.net/{CLOUD_UPLOAD_BUCKET}/private-key?offset=0&complete=true&version=1.0"
            ),
            upload_headers: BTreeMap::from([
                ("x-nos-token".into(), "private-upload-token".into()),
                ("Content-Type".into(), "audio/flac".into()),
                ("Content-MD5".into(), file(size).md5),
            ]),
            extensions: Extensions::new(),
        }),
        deadline: Instant::now() + TTL,
        expires_at: 1234567890,
        offset: 0,
        chunked,
        chunk_size: CHUNK,
        context: None,
        step: None,
        attempted_end: 0,
        failures: 0,
    };
    entry.plan(CloudUploadStepKind::Upload, 0).unwrap();
    let view = entry.view(&id).unwrap();
    provider.cloud_transfers.0.lock().unwrap().insert(id, entry);
    view
}
fn report(plan: &CloudUploadTransfer, status: Option<u16>, body: Value) -> CloudUploadStepResponse {
    CloudUploadStepResponse {
        step_id: plan.step.as_ref().unwrap().step_id.clone(),
        status,
        headers: BTreeMap::new(),
        body: if body.is_null() {
            String::new()
        } else {
            body.to_string()
        },
    }
}
fn advance(
    provider: &NeteaseProvider,
    plan: &CloudUploadTransfer,
    body: Value,
) -> CloudUploadTransfer {
    provider
        .advance_upload_transfer(&plan.transfer_id, &report(plan, Some(200), body), None)
        .unwrap()
}
fn query(plan: &CloudUploadTransfer) -> BTreeMap<String, String> {
    Url::parse(&plan.step.as_ref().unwrap().url)
        .unwrap()
        .query_pairs()
        .into_owned()
        .collect()
}

#[test]
fn single_request_413_becomes_a_normalized_chunk_plan_without_whole_file_md5() {
    let p = provider();
    let plan = seeded(&p, CHUNK * 2, false, None);
    assert_eq!(plan.step.as_ref().unwrap().length, CHUNK * 2);
    assert!(
        plan.step
            .as_ref()
            .unwrap()
            .headers
            .contains_key("Content-MD5")
    );
    assert!(plan.step.as_ref().unwrap().url.starts_with("https://"));
    let next = p
        .advance_upload_transfer(
            &plan.transfer_id,
            &report(&plan, Some(413), Value::Null),
            None,
        )
        .unwrap();
    assert_eq!(next.offset, 0);
    assert_eq!(next.step.as_ref().unwrap().length, CHUNK);
    assert_eq!(query(&next)["complete"], "false");
    assert!(
        !next
            .step
            .as_ref()
            .unwrap()
            .headers
            .contains_key("Content-MD5")
    );
    assert_ne!(
        plan.step.unwrap().step_id,
        next.step.as_ref().unwrap().step_id
    );
    assert!(!format!("{next:?}").contains("private-"));
}

#[test]
fn chunk_context_and_uncertain_partial_transfer_resume_through_a_probe() {
    let p = provider();
    let first = seeded(&p, CHUNK * 2 + 10, true, None);
    let second = advance(&p, &first, json!({"offset":CHUNK,"context":"ctx/+first"}));
    assert_eq!(second.offset, CHUNK);
    assert_eq!(query(&second)["context"], "ctx/+first");
    let probe = p
        .advance_upload_transfer(
            &second.transfer_id,
            &report(&second, Some(503), Value::Null),
            None,
        )
        .unwrap();
    assert_eq!(
        probe.step.as_ref().unwrap().kind,
        CloudUploadStepKind::Probe
    );
    assert_eq!(probe.step.as_ref().unwrap().method, "GET");
    assert_eq!(probe.step.as_ref().unwrap().length, 0);
    assert!(query(&probe).contains_key("uploadContext"));
    assert_eq!(query(&probe)["context"], "ctx/+first");
    assert_eq!(probe.step.as_ref().unwrap().retry_delay_ms, 500);
    let resumed = advance(&p, &probe, json!({"offset":CHUNK+2,"context":"ctx/+next"}));
    assert_eq!(resumed.offset, CHUNK + 2);
    assert_eq!(resumed.step.as_ref().unwrap().length, CHUNK);
    assert_eq!(query(&resumed)["context"], "ctx/+next");
    let last = advance(&p, &resumed, json!({"offset":CHUNK*2+2}));
    assert_eq!(last.step.as_ref().unwrap().length, 8);
    assert_eq!(query(&last)["complete"], "true");
    let done = advance(&p, &last, json!({"offset":CHUNK*2+10}));
    assert_eq!(done.state, CloudUploadTransferState::ReadyToPublish);
    assert!(done.step.is_none());
    assert_eq!(
        p.read_upload_transfer(&done.transfer_id, None).unwrap(),
        done
    );
}

#[test]
fn malformed_acknowledgements_never_guess_progress_or_enable_publication() {
    for (body, header) in [
        (json!({"offset":0,"context":"ctx"}), None),
        (json!({"offset":CHUNK+1,"context":"ctx"}), None),
        (json!({"offset":CHUNK}), None),
        (json!({"offset":CHUNK,"context":"ctx"}), Some("other")),
        (json!({"offset":CHUNK,"context":"ctx","errCode":500}), None),
        (json!({"offset":"4194304","context":"ctx"}), None),
        (json!({"offset":CHUNK,"context":"bad\ncontext"}), None),
    ] {
        let p = provider();
        let plan = seeded(&p, CHUNK * 2, true, None);
        let mut response = report(&plan, Some(200), body);
        if let Some(h) = header {
            response.headers.insert("x-nos-context".into(), h.into());
        }
        let error = p
            .advance_upload_transfer(&plan.transfer_id, &response, None)
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::UpstreamError);
        assert_eq!(error.details["transfer_consumed"], true);
        assert_eq!(
            p.read_upload_transfer(&plan.transfer_id, None)
                .unwrap_err()
                .code,
            ErrorCode::ResourceNotFound
        );
        assert!(!format!("{error:?}").contains("ctx"));
    }
    let p = provider();
    let plan = seeded(&p, 8, false, None);
    let mut duplicate = report(&plan, Some(200), Value::Null);
    duplicate.body = "{\"offset\":8,\"offset\":0}".into();
    assert!(
        p.advance_upload_transfer(&plan.transfer_id, &duplicate, None)
            .is_err()
    );
}

#[test]
fn header_context_is_supported_and_step_reports_are_bound_and_bounded() {
    let p = provider();
    let plan = seeded(&p, CHUNK + 1, true, None);
    let mut ack = report(&plan, Some(200), json!({"offset":CHUNK}));
    ack.headers
        .insert("X-Nos-Context".into(), "private-context".into());
    let mut invalid = ack.clone();
    invalid.step_id = "different-step".into();
    assert_eq!(
        p.advance_upload_transfer(&plan.transfer_id, &invalid, None)
            .unwrap_err()
            .code,
        ErrorCode::InvalidRequest
    );
    invalid = ack.clone();
    invalid.body = "x".repeat(65537);
    assert_eq!(
        p.advance_upload_transfer(&plan.transfer_id, &invalid, None)
            .unwrap_err()
            .code,
        ErrorCode::InvalidRequest
    );
    let next = p
        .advance_upload_transfer(&plan.transfer_id, &ack, None)
        .unwrap();
    assert_eq!(query(&next)["context"], "private-context");
    assert!(!format!("{ack:?}").contains("private-context"));
    assert_eq!(
        p.advance_upload_transfer(&plan.transfer_id, &ack, None)
            .unwrap_err()
            .code,
        ErrorCode::InvalidRequest
    );
    assert_eq!(
        p.read_upload_transfer(&plan.transfer_id, None).unwrap(),
        next
    );
}

#[test]
fn transient_errors_have_a_shared_finite_retry_budget_and_auth_errors_are_terminal() {
    for status in [None, Some(408), Some(425), Some(429), Some(503)] {
        let p = provider();
        let mut plan = seeded(&p, 8, false, None);
        for remaining in [2, 1] {
            plan = p
                .advance_upload_transfer(
                    &plan.transfer_id,
                    &report(&plan, status, Value::Null),
                    None,
                )
                .unwrap();
            assert_eq!(plan.offset, 0);
            assert_eq!(plan.step.as_ref().unwrap().attempts_remaining, remaining);
        }
        assert!(
            p.advance_upload_transfer(&plan.transfer_id, &report(&plan, status, Value::Null), None)
                .is_err()
        );
        assert!(p.cloud_transfers.0.lock().unwrap().is_empty());
    }
    for status in [201, 202, 204, 301, 400, 401, 403, 404] {
        let p = provider();
        let plan = seeded(&p, 8, false, None);
        assert!(
            p.advance_upload_transfer(
                &plan.transfer_id,
                &report(&plan, Some(status), Value::Null),
                None
            )
            .is_err()
        );
        assert!(p.cloud_transfers.0.lock().unwrap().is_empty());
    }
}

#[test]
fn transfer_owner_file_and_lifetime_cannot_be_replaced() {
    let p = provider();
    p.install_session("locker", "MUSIC_U=stored".into())
        .unwrap();
    let plan = seeded(&p, 8, false, Some("locker"));
    assert_eq!(
        p.read_upload_transfer(&plan.transfer_id, None)
            .unwrap_err()
            .code,
        ErrorCode::PermissionDenied
    );
    p.install_session("locker", "MUSIC_U=stored".into())
        .unwrap();
    assert_eq!(
        p.read_upload_transfer(&plan.transfer_id, Some("locker"))
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
    let material = ProviderCredential::new(
        Platform::Netease,
        NETEASE_CREDENTIAL_KIND,
        "MUSIC_U=caller",
        None,
    )
    .unwrap();
    let caller = p.caller_credential_scope(&material).unwrap();
    let plan = seeded(&caller, 8, false, None);
    let next_scope = p.caller_credential_scope(&material).unwrap();
    assert_eq!(
        next_scope
            .read_upload_transfer(&plan.transfer_id, None)
            .unwrap(),
        plan
    );
    assert_eq!(
        p.read_upload_transfer(&plan.transfer_id, None)
            .unwrap_err()
            .code,
        ErrorCode::PermissionDenied
    );
    let other = p
        .caller_credential_scope(
            &ProviderCredential::new(
                Platform::Netease,
                NETEASE_CREDENTIAL_KIND,
                "MUSIC_U=other",
                None,
            )
            .unwrap(),
        )
        .unwrap();
    assert_eq!(
        other
            .read_upload_transfer(&plan.transfer_id, None)
            .unwrap_err()
            .code,
        ErrorCode::PermissionDenied
    );
    next_scope
        .cancel_upload_transfer(&plan.transfer_id, None)
        .unwrap();
    assert!(
        caller
            .read_upload_transfer(&plan.transfer_id, None)
            .is_err()
    );
    let plan = seeded(&p, 8, false, None);
    p.cloud_transfers
        .0
        .lock()
        .unwrap()
        .get_mut(&plan.transfer_id)
        .unwrap()
        .deadline = Instant::now();
    assert_eq!(
        p.read_upload_transfer(&plan.transfer_id, None)
            .unwrap_err()
            .code,
        ErrorCode::ResourceNotFound
    );
}

async fn server(responses: Vec<Value>) -> (NeteaseProvider, tokio::task::JoinHandle<Vec<String>>) {
    server_gated(responses, None).await
}
async fn server_gated(
    responses: Vec<Value>,
    gate: Option<(Arc<tokio::sync::Notify>, Arc<tokio::sync::Notify>)>,
) -> (NeteaseProvider, tokio::task::JoinHandle<Vec<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let client = NeteaseClient::new(NeteaseConfig {
        base_url: origin.clone(),
        web_base_url: origin.clone(),
        cookie: Some("MUSIC_U=account".into()),
        ..Default::default()
    })
    .unwrap()
    .with_cloud_servers_test_url(format!("{origin}/lbs"));
    let handle = tokio::spawn(async move {
        let mut requests = Vec::new();
        for response in responses {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut raw = Vec::new();
            let mut buf = [0; 4096];
            loop {
                let read = stream.read(&mut buf).await.unwrap();
                assert_ne!(read, 0);
                raw.extend_from_slice(&buf[..read]);
                if let Some(end) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&raw[..end]);
                    let length = headers
                        .lines()
                        .find_map(|s| {
                            s.split_once(':')
                                .filter(|(k, _)| k.eq_ignore_ascii_case("content-length"))
                                .map(|(_, v)| v.trim().parse::<usize>().unwrap())
                        })
                        .unwrap_or(0);
                    if raw.len() >= end + 4 + length {
                        break;
                    }
                }
            }
            requests.push(String::from_utf8(raw).unwrap());
            if let Some((started, release)) = &gate {
                started.notify_one();
                release.notified().await;
            }
            let body = response.to_string();
            stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).as_bytes()).await.unwrap();
        }
        requests
    });
    (NeteaseProvider::from_client(client), handle)
}
fn allocation(upload: bool) -> Vec<Value> {
    vec![
        json!({"code":200,"needUpload":upload,"songId":123}),
        json!({"code":200,"result":{"objectKey":"private/object","token":"private-token","resourceId":456}}),
        json!({"upload":["http://nosup-jd1.127.net"]}),
    ]
}

#[tokio::test]
async fn allocated_transfer_publishes_only_after_confirmation_and_binds_original_metadata() {
    for caller in [false, true] {
        for upload in [false, true] {
            let mut replies = allocation(upload);
            replies.extend([json!({"code":200,"songId":123}), json!({"code":200})]);
            let (base, requests) = server(replies).await;
            let p = if caller {
                base.caller_credential_scope(
                    &ProviderCredential::new(
                        Platform::Netease,
                        NETEASE_CREDENTIAL_KIND,
                        "MUSIC_U=caller",
                        None,
                    )
                    .unwrap(),
                )
                .unwrap()
            } else {
                base
            };
            let mut plan = p
                .begin_cloud_upload_transfer(&CloudUploadTransferRequest {
                    file: file(8),
                    strategy: CloudUploadStrategy::Auto,
                })
                .await
                .unwrap();
            assert_eq!(plan.upload_required, upload);
            if upload {
                assert_eq!(
                    p.publish_cloud_upload_transfer(
                        &plan.transfer_id,
                        &CloudUploadPublishMetadata::default(),
                        None
                    )
                    .await
                    .unwrap_err()
                    .code,
                    ErrorCode::Conflict
                );
                plan = advance(&p, &plan, json!({"offset":8}));
            }
            let result = p
                .publish_cloud_upload_transfer(
                    &plan.transfer_id,
                    &CloudUploadPublishMetadata {
                        song_name: Some("Synthetic Song".into()),
                        ..Default::default()
                    },
                    None,
                )
                .await
                .unwrap();
            assert!(result.published);
            assert_eq!(result.uploaded, Some(upload));
            assert_eq!(result.track_ref.unwrap().id(), "123");
            assert!(p.read_upload_transfer(&plan.transfer_id, None).is_err());
            let requests = requests.await.unwrap();
            assert_eq!(requests.len(), 5);
            assert!(requests[0].starts_with("POST /eapi/cloud/upload/check"));
            assert!(requests[1].starts_with("POST /weapi/nos/token/alloc"));
            assert!(requests[2].starts_with("GET /lbs?"));
            assert!(requests[3].starts_with("POST /eapi/upload/cloud/info/v2"));
            assert!(requests[4].starts_with("POST /eapi/cloud/pub/v2"));
            let encoded = requests[3].split_once("\r\n\r\n").unwrap().1;
            let params = url::form_urlencoded::parse(encoded.as_bytes())
                .find(|(k, _)| k == "params")
                .unwrap()
                .1
                .into_owned();
            let plaintext = String::from_utf8(
                crate::crypto::decrypt_eapi_response(&hex::decode(params).unwrap()).unwrap(),
            )
            .unwrap();
            let info: Value =
                serde_json::from_str(plaintext.split("-36cd479b6b5-").nth(1).unwrap()).unwrap();
            assert_eq!(info["md5"], file(8).md5);
            assert_eq!(info["filename"], "original.flac");
            assert_eq!(info["resourceId"], "456");
            assert_eq!(info["songid"], "123");
            assert_eq!(info["song"], "Synthetic Song");
        }
    }
}

#[tokio::test]
async fn large_files_start_chunked_and_failed_publication_is_not_retried() {
    let mut replies = allocation(true);
    replies.push(json!({"code":500,"message":"synthetic failure"}));
    let (p, requests) = server(replies).await;
    let plan = p
        .begin_cloud_upload_transfer(&CloudUploadTransferRequest {
            file: file(THRESHOLD + 1),
            strategy: CloudUploadStrategy::Auto,
        })
        .await
        .unwrap();
    assert_eq!(plan.step.as_ref().unwrap().length, CHUNK);
    // Complete a small independent transfer; only one metadata dispatch is allowed on failure.
    let small = seeded(&p, 8, false, None);
    let done = advance(&p, &small, json!({"offset":8}));
    let failure = p
        .publish_cloud_upload_transfer(
            &done.transfer_id,
            &CloudUploadPublishMetadata::default(),
            None,
        )
        .await
        .unwrap_err();
    assert_eq!(failure.details["transfer_consumed"], true);
    assert!(
        p.publish_cloud_upload_transfer(
            &done.transfer_id,
            &CloudUploadPublishMetadata::default(),
            None
        )
        .await
        .is_err()
    );
    assert_eq!(requests.await.unwrap().len(), 4);
}

#[test]
fn probes_reject_backward_and_impossible_offsets_and_keep_the_retry_budget() {
    for offset in [CHUNK - 1, CHUNK * 2 + 1] {
        let p = provider();
        let first = seeded(&p, CHUNK * 3, true, None);
        let next = advance(&p, &first, json!({"offset":CHUNK,"context":"ctx"}));
        let probe = p
            .advance_upload_transfer(&next.transfer_id, &report(&next, None, Value::Null), None)
            .unwrap();
        let error = p
            .advance_upload_transfer(
                &probe.transfer_id,
                &report(&probe, Some(200), json!({"offset":offset})),
                None,
            )
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::UpstreamError);
        assert!(p.read_upload_transfer(&probe.transfer_id, None).is_err());
    }
    let p = provider();
    let first = seeded(&p, CHUNK * 3, true, None);
    let mut plan = advance(&p, &first, json!({"offset":CHUNK,"context":"ctx"}));
    for remaining in [2, 1] {
        let probe = p
            .advance_upload_transfer(&plan.transfer_id, &report(&plan, None, Value::Null), None)
            .unwrap();
        plan = advance(&p, &probe, json!({"offset":CHUNK}));
        assert_eq!(plan.offset, CHUNK);
        assert_eq!(plan.step.as_ref().unwrap().attempts_remaining, remaining);
    }
    assert!(
        p.advance_upload_transfer(&plan.transfer_id, &report(&plan, None, Value::Null), None)
            .is_err()
    );
}

#[test]
fn chunk_413_fallback_is_bounded_and_invalid_ticket_headers_are_rejected() {
    let p = provider();
    let mut plan = seeded(&p, CHUNK * 2, true, None);
    for size in [
        2 * 1024 * 1024,
        1024 * 1024,
        512 * 1024,
        256 * 1024,
        128 * 1024,
        64 * 1024,
    ] {
        plan = p
            .advance_upload_transfer(
                &plan.transfer_id,
                &report(&plan, Some(413), Value::Null),
                None,
            )
            .unwrap();
        assert_eq!(plan.step.as_ref().unwrap().length, size);
    }
    assert!(
        p.advance_upload_transfer(
            &plan.transfer_id,
            &report(&plan, Some(413), Value::Null),
            None
        )
        .is_err()
    );
    let plan = seeded(&p, 8, false, None);
    let mut entries = p.cloud_transfers.0.lock().unwrap();
    let entry = entries.get_mut(&plan.transfer_id).unwrap();
    entry
        .ticket
        .as_mut()
        .unwrap()
        .upload_headers
        .insert("x-nos-token".into(), "bad\r\nheader".into());
    assert!(entry.plan(CloudUploadStepKind::Upload, 0).is_err());
}

#[tokio::test]
async fn cancelled_replaced_and_logged_out_accounts_stop_in_flight_publication() {
    for action in ["cancel", "replace", "caller_logout"] {
        let started = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Notify::new());
        let (base, requests) = server_gated(
            vec![json!({"code":200,"songId":123})],
            Some((started.clone(), release.clone())),
        )
        .await;
        let p = if action == "caller_logout" {
            base.caller_credential_scope(
                &ProviderCredential::new(
                    Platform::Netease,
                    NETEASE_CREDENTIAL_KIND,
                    "MUSIC_U=caller",
                    None,
                )
                .unwrap(),
            )
            .unwrap()
        } else {
            base
        };
        let p = Arc::new(p);
        let first = seeded(&p, 8, false, None);
        let plan = advance(&p, &first, json!({"offset":8}));
        let worker_p = p.clone();
        let id = plan.transfer_id.clone();
        let worker = tokio::spawn(async move {
            worker_p
                .publish_upload_transfer(&id, &CloudUploadPublishMetadata::default(), None)
                .await
        });
        tokio::time::timeout(Duration::from_secs(5), started.notified())
            .await
            .unwrap();
        assert_eq!(
            p.read_upload_transfer(&plan.transfer_id, None)
                .unwrap_err()
                .code,
            ErrorCode::Conflict
        );
        assert_eq!(
            p.publish_upload_transfer(
                &plan.transfer_id,
                &CloudUploadPublishMetadata::default(),
                None
            )
            .await
            .unwrap_err()
            .code,
            ErrorCode::Conflict
        );
        match action {
            "cancel" => {
                p.cancel_upload_transfer(&plan.transfer_id, None).unwrap();
            }
            "replace" => {
                p.install_session("default", "MUSIC_U=replacement".into())
                    .unwrap();
            }
            _ => {
                p.cancel_caller_uploads(&p.client_for(None).unwrap())
                    .unwrap();
            }
        }
        release.notify_one();
        let error = worker.await.unwrap().unwrap_err();
        assert_eq!(error.code, ErrorCode::Conflict);
        assert_eq!(error.details["transfer_consumed"], true);
        assert_eq!(requests.await.unwrap().len(), 1);
        assert!(p.cloud_transfers.0.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn caller_logout_discards_only_matching_pending_uploads_even_when_remote_logout_fails() {
    let (p, requests) = server(vec![json!({"code":500})]).await;
    let credential = ProviderCredential::new(
        Platform::Netease,
        NETEASE_CREDENTIAL_KIND,
        "MUSIC_U=caller",
        None,
    )
    .unwrap();
    let caller = p.caller_credential_scope(&credential).unwrap();
    let plan = seeded(&caller, 8, false, None);
    let server_plan = seeded(&p, 8, false, None);
    assert!(
        p.logout_with_ownership("default", Some(&credential), CredentialMode::Client)
            .await
            .is_err()
    );
    assert!(
        caller
            .read_upload_transfer(&plan.transfer_id, None)
            .is_err()
    );
    assert!(
        p.read_upload_transfer(&server_plan.transfer_id, None)
            .is_ok()
    );
    assert_eq!(requests.await.unwrap().len(), 1);
}

#[tokio::test]
async fn server_logout_cancels_outstanding_tasks_before_awaiting_upstream() {
    let started = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let (p, requests) = server_gated(
        vec![json!({"code":500})],
        Some((started.clone(), release.clone())),
    )
    .await;
    let p = Arc::new(p);
    let plan = seeded(&p, 8, false, None);
    let worker_p = p.clone();
    let worker = tokio::spawn(async move { worker_p.logout_account("default").await });
    tokio::time::timeout(Duration::from_secs(5), started.notified())
        .await
        .unwrap();
    assert!(p.read_upload_transfer(&plan.transfer_id, None).is_err());
    release.notify_one();
    assert!(worker.await.unwrap().is_err());
    assert_eq!(requests.await.unwrap().len(), 1);
}
