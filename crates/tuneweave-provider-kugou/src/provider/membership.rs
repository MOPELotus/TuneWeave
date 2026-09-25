use super::*;
use tuneweave_core::MembershipSummary;

// A dropped future cannot leave a response update for another request to consume.
struct Pending {
    response: Arc<Mutex<Option<ProviderCredential>>>,
    complete: bool,
}
impl Drop for Pending {
    fn drop(&mut self) {
        if !self.complete {
            if let Ok(mut value) = self.response.lock() {
                *value = None;
            }
        }
    }
}

impl KugouProvider {
    pub(super) async fn read_membership(
        &self,
        id: Option<&str>,
        account: Option<&str>,
        budget: std::time::Duration,
    ) -> Result<MembershipSummary> {
        let mut pending = Pending {
            response: self.response_credential.clone(),
            complete: false,
        };
        if id.is_some_and(|v| !crate::credential::valid_uid(v)) {
            return Err(kugou_invalid_request("KuGou membership user ID is invalid"));
        }
        let (mut current, mut stored) = self
            .selected(account.unwrap_or("default"))?
            .ok_or_else(session::authentication_required)?;
        if id.is_some_and(|uid| current.user_id() != uid) {
            return Err(TuneWeaveError::new(
                ErrorCode::PermissionDenied,
                "KuGou membership requires the selected account's user ID",
            )
            .with_platform(Platform::Kugou));
        }
        let mut seen = vec![current.clone()];
        let mut accepted = false;
        let operation = tokio::time::timeout(budget, async {
            let exchange = match &current {
                KugouCredential::Native(v) => self
                    .client
                    .exchange_native_token(&v.session)
                    .await
                    .and_then(|session| v.rotate(session))
                    .map(KugouCredential::Native),
                KugouCredential::Web(v) => self
                    .client
                    .refresh_web_session(&v.session)
                    .await
                    .and_then(|session| current.rotate_web(session)),
            };
            self.apply_read(&current, &mut stored, &current)?;
            let next = exchange?;
            seen.push(next.clone());
            self.apply_read(&current, &mut stored, &next)?;
            current = next;
            accepted = true;
            let summary = match &current {
                KugouCredential::Native(v) => {
                    let profile = self.client.native_profile(&v.session).await;
                    self.apply_read(&current, &mut stored, &current)?;
                    profile?;
                    let membership = self.client.native_membership(&v.session).await;
                    self.apply_read(&current, &mut stored, &current)?;
                    membership?
                }
                KugouCredential::Web(v) => {
                    let response = self.client.web_membership(&v.session).await;
                    self.apply_read(&current, &mut stored, &current)?;
                    let response = response?;
                    let candidate = current.rotate_web(response.candidate)?;
                    seen.push(candidate.clone());
                    let KugouCredential::Web(v) = &candidate else {
                        return Err(session::state_error());
                    };
                    // roleinfo has no verified UID; even an unchanged Cookie needs
                    // a fresh, independent same-user exchange before its data is released.
                    let verified = self.client.refresh_web_session(&v.session).await;
                    self.apply_read(&current, &mut stored, &current)?;
                    let next = current.rotate_web(verified?)?;
                    seen.push(next.clone());
                    reject_secrets(&response.summary, &seen)?;
                    self.apply_read(&current, &mut stored, &next)?;
                    current = next;
                    response.summary
                }
            };
            reject_secrets(&summary, &seen)?;
            self.apply_read(&current, &mut stored, &current)?;
            Ok(summary)
        })
        .await;
        let result = match operation {
            Ok(result) => result,
            Err(_) => {
                return Err(TuneWeaveError::new(
                    ErrorCode::UpstreamTimeout,
                    "KuGou membership exceeded its total time budget",
                )
                .with_platform(Platform::Kugou)
                .retryable(true));
            }
        };
        match result {
            Ok(summary) => {
                pending.complete = true;
                Ok(summary)
            }
            Err(error) => {
                let mut error = self.finish_read_error(
                    &current,
                    &mut stored,
                    error,
                    accepted,
                    self.caller_credential.is_some(),
                );
                // Store errors cannot attest the final ownership state. Other
                // non-authentication failures may return a previously verified update.
                if matches!(
                    error.code,
                    ErrorCode::InternalError
                        | ErrorCode::AuthenticationRequired
                        | ErrorCode::Conflict
                        | ErrorCode::UpstreamTimeout
                ) {
                    error.take_caller_credential_update();
                }
                Err(error)
            }
        }
    }
}

fn reject_secrets(summary: &MembershipSummary, seen: &[KugouCredential]) -> Result<()> {
    let mut secrets = Vec::new();
    for credential in seen {
        match credential {
            KugouCredential::Native(v) => {
                secrets.push(v.session.token.clone());
                secrets.extend(v.session.vip_token.iter().cloned());
                secrets.extend(v.session.t1.iter().cloned());
            }
            KugouCredential::Web(v) => secrets.extend(v.session.membership_secrets()?),
        }
        secrets.push(credential.caller()?.into_secret());
    }
    let originals = secrets.clone();
    secrets.extend(
        originals
            .iter()
            .map(|s| url::form_urlencoded::byte_serialize(s.as_bytes()).collect()),
    );
    fn visit(value: &serde_json::Value, secrets: &[String]) -> bool {
        match value {
            serde_json::Value::String(s) => secrets
                .iter()
                .any(|secret| !secret.is_empty() && s.contains(secret)),
            serde_json::Value::Array(v) => v.iter().any(|v| visit(v, secrets)),
            serde_json::Value::Object(v) => v.values().any(|v| visit(v, secrets)),
            _ => false,
        }
    }
    let value = serde_json::to_value(summary).map_err(|_| session::state_error())?;
    if visit(&value, &secrets) {
        return Err(TuneWeaveError::new(
            ErrorCode::UpstreamError,
            "KuGou membership contains private account material",
        )
        .with_platform(Platform::Kugou));
    }
    Ok(())
}

#[cfg(test)]
mod tests;
