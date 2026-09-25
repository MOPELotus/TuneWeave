use super::*;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SessionRevokeBody {
    platform: String,
    account: Option<String>,
    credential_mode: Option<CredentialMode>,
}

pub(super) async fn auth_session_revoke(
    State(state): State<AppState>,
    headers: HeaderMap,
    payload: Result<Json<SessionRevokeBody>, JsonRejection>,
) -> Result<Response, ApiError> {
    caller_scope::mark_sensitive();
    // Header parsing can create provider scopes. No scope may publish credentials
    // when this request completes, including early failures or another platform.
    caller_scope::invalidate(None);
    let body = json_body(payload)?;
    let platform = parse_platform_parameter(&body.platform)?;
    let credentials = CallerCredentialSet::from_headers(&headers, &state)?;
    let source = credentials.single_credential_for(platform)?;
    let mode = body.credential_mode.unwrap_or(if source.is_some() {
        CredentialMode::Client
    } else {
        CredentialMode::Server
    });
    let account = logout_account_alias(body.account.as_deref(), source.is_some(), mode)?;
    record_auth_session_provider_access(platform, source);
    let provider = state.registry.require(platform)?;
    let started = Instant::now();
    let result = match provider
        .revoke_session_with_ownership(&account, source, mode)
        .await
    {
        Ok(result) => result,
        Err(mut error) => {
            // A revocation response must never reissue an authentication credential.
            let _ = error.take_caller_credential_update();
            log_auth_operation_failure(
                AuthOperation::SessionRevoke,
                platform,
                mode,
                started,
                &error,
            );
            return Err(error.into());
        }
    };
    log_auth_operation_success(
        AuthOperation::SessionRevoke,
        platform,
        mode,
        started,
        Some(result.removed),
        Some(result.caller_credential_discard_required),
    );
    Ok(auth_json_response(
        auth_api_response(result, platform, &account, mode),
        true,
    ))
}
