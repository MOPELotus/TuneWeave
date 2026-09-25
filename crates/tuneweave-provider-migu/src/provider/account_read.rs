use super::session::session_changed;
use super::*;
use crate::client::account::AccountData;
use crate::credential::{KIND, MiguCredential, authentication_required, error};
use tuneweave_core::{ErrorCode, StoredAccountCredential};

impl MiguProvider {
    pub(super) fn accept_account_step(
        &self,
        current: &mut MiguCredential,
        stored: &mut Option<StoredAccountCredential>,
        token: String,
    ) -> Result<()> {
        let next = current.rotate(token)?;
        let replacement = stored
            .as_ref()
            .map(|old| {
                StoredAccountCredential::new(Platform::Migu, &old.account, KIND, next.serialize()?)
            })
            .transpose()?;
        self.accept_read(current, stored.as_ref(), &next)?;
        *current = next;
        *stored = replacement;
        Ok(())
    }

    pub(super) async fn verify_account_step(
        &self,
        account: &str,
        current: &mut MiguCredential,
        stored: &mut Option<StoredAccountCredential>,
        token: &str,
    ) -> Result<AccountProfile> {
        let read = self
            .client
            .account_profile(account, token, Some(current.user_id()))
            .await?;
        self.accept_account_step(current, stored, read.token)?;
        read.profile
    }

    pub(super) fn finish_account_read<T>(
        &self,
        original: &MiguCredential,
        current: &MiguCredential,
        stored: Option<&StoredAccountCredential>,
        result: Result<T>,
    ) -> Result<T> {
        match result {
            Ok(value) => Ok(value),
            Err(mut failure) => {
                let checked = if failure.code == ErrorCode::AuthenticationRequired {
                    if let Some(snapshot) = stored {
                        self.credential_store
                            .as_ref()
                            .ok_or_else(|| {
                                error(
                                    ErrorCode::InternalError,
                                    "Migu account store is unavailable",
                                )
                            })?
                            .compare_exchange(snapshot, None)
                            .and_then(|removed| {
                                if removed {
                                    Ok(())
                                } else {
                                    Err(session_changed())
                                }
                            })
                    } else {
                        self.accept_read(current, None, current)
                    }
                } else {
                    self.accept_read(current, stored, current)
                };
                if let Err(conflict) = checked {
                    self.discard_response_credential_after_error(conflict.code)?;
                    return Err(conflict);
                }
                self.discard_response_credential_after_error(failure.code)?;
                if self.caller_credential.is_some()
                    && current != original
                    && !matches!(
                        failure.code,
                        ErrorCode::AuthenticationRequired | ErrorCode::Conflict
                    )
                {
                    failure = failure.with_caller_credential_update(current.caller()?);
                }
                Err(failure)
            }
        }
    }
}

pub(super) struct Read<'a> {
    provider: &'a MiguProvider,
    alias: String,
    original: MiguCredential,
    pub(super) current: MiguCredential,
    stored: Option<StoredAccountCredential>,
    pub(super) secrets: Vec<String>,
    finished: bool,
}
impl<'a> Read<'a> {
    pub(super) fn new(provider: &'a MiguProvider, account: Option<&str>) -> Result<Self> {
        let alias = account.unwrap_or("default").to_owned();
        let (current, stored) = provider
            .selected(&alias)?
            .ok_or_else(authentication_required)?;
        Ok(Self {
            provider,
            alias,
            original: current.clone(),
            secrets: vec![current.token().to_owned()],
            current,
            stored,
            finished: false,
        })
    }
    pub(super) fn check(&self) -> Result<()> {
        self.provider
            .accept_read(&self.current, self.stored.as_ref(), &self.current)
    }
    async fn verify(&mut self, token: &str) -> Result<AccountProfile> {
        self.check()?;
        let result = self
            .provider
            .verify_account_step(&self.alias, &mut self.current, &mut self.stored, token)
            .await;
        self.check()?;
        self.secrets.push(self.current.token().to_owned());
        result
    }
    pub(super) async fn start(&mut self) -> Result<AccountProfile> {
        let token = self.current.token().to_owned();
        self.verify(&token).await
    }
    pub(super) async fn native_session(&mut self) -> Result<String> {
        self.check()?;
        let read = self
            .provider
            .client
            .account_profile(
                &self.alias,
                self.current.token(),
                Some(self.current.user_id()),
            )
            .await?;
        self.check()?;
        self.provider
            .accept_account_step(&mut self.current, &mut self.stored, read.token)?;
        self.secrets.push(self.current.token().to_owned());
        read.profile?;
        let session = read.native_session?;
        self.secrets.push(session.clone());
        Ok(session)
    }
    pub(super) async fn accept<T>(
        &mut self,
        response: Result<AccountData<T>>,
    ) -> Result<Result<T>> {
        self.check()?;
        let response = response?;
        self.secrets.push(response.token.clone());
        self.verify(&response.token).await?;
        Ok(response.data)
    }
    pub(super) async fn track(&mut self, id: &str) -> Result<Track> {
        self.check()?;
        let result = self.provider.client.track_detail(id).await;
        self.check()?;
        let mut track = result?;
        self.mark(&mut track.extensions);
        Ok(track)
    }
    pub(super) fn mark(&self, extensions: &mut Extensions) {
        extensions.insert("source_user_id".into(), json!(self.current.user_id()));
        extensions.insert("catalogue_scope".into(), json!("public"));
    }
    pub(super) fn finish<T>(&mut self, result: Result<T>) -> Result<T> {
        let result = self.check().and(result);
        let result = self.provider.finish_account_read(
            &self.original,
            &self.current,
            self.stored.as_ref(),
            result,
        );
        self.finished = true;
        result
    }
}
impl Drop for Read<'_> {
    fn drop(&mut self) {
        // An abandoned future cannot deliver a credential update. Verified stored
        // rotations remain valid; pending caller response state must not escape.
        if !self.finished
            && self.provider.caller_credential.is_some()
            && let Ok(mut response) = self.provider.response_credential.lock()
        {
            *response = None;
        }
    }
}
