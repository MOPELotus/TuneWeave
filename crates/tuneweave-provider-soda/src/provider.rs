use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use serde_json::json;
use tuneweave_core::{
    AccountCredentialStore, AccountProfile, Album, Artist, ArtistOverview, AudioContent, AuthState,
    CALLER_CREDENTIAL_HEADER, CallerCredential, Capability, CredentialImportRequest,
    CredentialMode, DigitalAlbum, ErrorCode, Extensions, ImportedCredential, Lyrics, LyricsRequest,
    MediaDownload, MediaStream, MembershipSummary, MusicProvider, Page, PageMeta, PageRequest,
    Platform, Playlist, PlaylistPlayableItem, ProviderAuthResult, ProviderCredential,
    ProviderLogoutResult, ProviderQrPoll, ProviderQrStart, Quality, Result, SearchItem, SearchKind,
    SearchQuery, SearchSuggestionClient, SearchSuggestionList, SearchSuggestionRequest,
    SearchVariant, StoredAccountCredential, StreamRequest, StreamVariant, SubscriptionResult,
    Track, TrackAvailability, TrackAvailabilityRequest, TrialWindow, TuneWeaveError, UserProfile,
    UserProfileBackend,
};

use crate::{
    account::SodaAccount,
    client::{
        MAX_UPSTREAM_PLAYLIST_PAGES, SodaAccountTrack, SodaCatalogKind, SodaClient, SodaConfig,
        SodaPlayback, UPSTREAM_PLAYLIST_PAGE_SIZE, UPSTREAM_SEARCH_PAGE_SIZE,
    },
    identity::SodaTrackIdentity,
    library::{LibraryPagination, LibrarySection},
    login::{SodaCredential, SodaQrPollOutcome, SodaQrTransactions},
};

mod account_albums;
mod account_artists;
mod account_digital_albums;
mod account_following_artists;
mod account_playlists;
mod account_search;
mod account_suggestions;
mod album_collection_source;
mod album_collections;
mod artist_catalog;
mod artist_collection;
mod auth_generation;
mod favorites;
mod library_operations;
mod membership;
mod playlist_write;
mod purchased_album_source;
mod session_revocation;

const MAX_UPSTREAM_PAGES_PER_SEARCH: u32 = 6;
const SODA_CREDENTIAL_KIND: &str = "soda_cookie_v1";

// Unless explicitly completed, an account operation must not leave an update for a later response.
struct PendingCredentialUpdate {
    response: Arc<Mutex<Option<ProviderCredential>>>,
    successful: bool,
}

impl PendingCredentialUpdate {
    fn new(response: Arc<Mutex<Option<ProviderCredential>>>) -> Self {
        Self {
            response,
            successful: false,
        }
    }
    fn complete(&mut self) {
        self.successful = true;
    }
}

impl Drop for PendingCredentialUpdate {
    fn drop(&mut self) {
        if !self.successful
            && let Ok(mut response) = self.response.lock()
        {
            *response = None;
        }
    }
}

#[derive(Clone)]
pub struct SodaProvider {
    client: SodaClient,
    credential_store: Option<Arc<dyn AccountCredentialStore>>,
    caller_credential: Option<Arc<Mutex<SodaCredential>>>,
    response_credential: Arc<Mutex<Option<ProviderCredential>>>,
    qr_transactions: SodaQrTransactions,
    auth_transactions: crate::authentication::AuthTransactions,
}

impl fmt::Debug for SodaProvider {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SodaProvider")
            .field(
                "credential_store_configured",
                &self.credential_store.is_some(),
            )
            .field(
                "caller_credential_configured",
                &self.caller_credential.is_some(),
            )
            .finish_non_exhaustive()
    }
}

impl SodaProvider {
    pub fn new(config: SodaConfig) -> Result<Self> {
        let credential_store = config.credential_store.clone();
        Ok(Self {
            client: SodaClient::new(&config)?,
            credential_store,
            caller_credential: None,
            response_credential: Arc::default(),
            qr_transactions: SodaQrTransactions::default(),
            auth_transactions: crate::authentication::AuthTransactions::default(),
        })
    }

    #[must_use]
    pub fn from_client(client: SodaClient) -> Self {
        Self {
            client,
            credential_store: None,
            caller_credential: None,
            response_credential: Arc::default(),
            qr_transactions: SodaQrTransactions::default(),
            auth_transactions: crate::authentication::AuthTransactions::default(),
        }
    }

    fn caller_credential_scope(&self, credential: &ProviderCredential) -> Result<Self> {
        Ok(Self {
            client: self.client.clone(),
            credential_store: None,
            caller_credential: Some(Arc::new(Mutex::new(parse_soda_caller_credential(
                credential,
            )?))),
            response_credential: Arc::default(),
            qr_transactions: self.qr_transactions.clone(),
            auth_transactions: self.auth_transactions.clone(),
        })
    }

    async fn finish_authentication(
        &self,
        account: &str,
        credential: &SodaCredential,
        mode: CredentialMode,
        deadline: Option<std::time::Instant>,
    ) -> Result<ProviderAuthResult> {
        let deadline = deadline
            .unwrap_or_else(|| std::time::Instant::now() + std::time::Duration::from_secs(45));
        let lease = self.authentication_lease(Some(account), mode, deadline)?;
        self.finish_authentication_with_lease(credential, &lease)
            .await
    }

    async fn finish_authentication_with_lease(
        &self,
        credential: &SodaCredential,
        lease: &crate::authentication::AuthLease,
    ) -> Result<ProviderAuthResult> {
        let account = lease
            .account
            .as_deref()
            .ok_or_else(soda_credential_lock_error)?;
        let verified = lease.wait(self.client.account(account, credential)).await?;
        self.save_authenticated(account, verified, lease)
    }

    fn save_authenticated(
        &self,
        account: &str,
        verified: SodaAccount,
        lease: &crate::authentication::AuthLease,
    ) -> Result<ProviderAuthResult> {
        let mode = lease.mode;
        validate_soda_login_account(account, mode)?;
        if verified.credential.user_id() != verified.profile.user_id.as_deref()
            || verified.credential.user_id().is_none()
            || !verified.profile.authenticated
        {
            return Err(soda_upstream_error(
                "Soda login omitted a verified account identity",
            ));
        }
        let secret = verified.credential.serialize()?;
        let caller_credential = mode
            .returns_to_caller()
            .then(|| ProviderCredential::new(Platform::Soda, SODA_CREDENTIAL_KIND, &secret, None))
            .transpose()?;
        lease.publish(&StoredAccountCredential::new(
            Platform::Soda,
            account,
            SODA_CREDENTIAL_KIND,
            &secret,
        )?)?;
        let mut profile = verified.profile;
        profile
            .extensions
            .insert("credential_kind".to_owned(), json!(SODA_CREDENTIAL_KIND));
        Ok(ProviderAuthResult {
            profile,
            credential: caller_credential,
        })
    }

    fn selected_credential(
        &self,
        account: &str,
    ) -> Result<Option<(SodaCredential, Option<StoredAccountCredential>)>> {
        validate_soda_login_account(account, CredentialMode::Server)?;
        if let Some(credential) = &self.caller_credential {
            if account != "default" {
                return Err(soda_invalid_request(
                    "caller-managed Soda requests cannot select server account aliases",
                ));
            }
            return Ok(Some((
                credential
                    .lock()
                    .map_err(|_| soda_credential_lock_error())?
                    .clone(),
                None,
            )));
        }
        let Some(store) = &self.credential_store else {
            return Ok(None);
        };
        let Some(stored) = store
            .load_platform(Platform::Soda)?
            .into_iter()
            .find(|credential| credential.account == account)
        else {
            return Ok(None);
        };
        if stored.kind != SODA_CREDENTIAL_KIND {
            return Err(TuneWeaveError::new(
                ErrorCode::InternalError,
                "stored Soda credential kind is invalid",
            )
            .with_platform(Platform::Soda));
        }
        Ok(Some((
            SodaCredential::parse(stored.secret())?,
            Some(stored),
        )))
    }

    fn accept_account_read(
        &self,
        source: &SodaCredential,
        stored: Option<&StoredAccountCredential>,
        refreshed: &SodaCredential,
    ) -> Result<()> {
        if let Some(stored) = stored {
            let replacement = StoredAccountCredential::new(
                Platform::Soda,
                &stored.account,
                SODA_CREDENTIAL_KIND,
                refreshed.serialize()?,
            )?;
            let store = self
                .credential_store
                .as_ref()
                .ok_or_else(soda_credential_lock_error)?;
            if !store.compare_exchange(stored, Some(&replacement))? {
                return Err(soda_session_changed());
            }
        } else {
            let caller = self
                .caller_credential
                .as_ref()
                .ok_or_else(soda_credential_lock_error)?;
            let mut credential = caller.lock().map_err(|_| soda_credential_lock_error())?;
            if &*credential != source {
                return Err(soda_session_changed());
            }
            if source == refreshed {
                return Ok(());
            }
            let issued = ProviderCredential::new(
                Platform::Soda,
                SODA_CREDENTIAL_KIND,
                refreshed.serialize()?,
                None,
            )?;
            let mut response = self
                .response_credential
                .lock()
                .map_err(|_| soda_credential_lock_error())?;
            *credential = refreshed.clone();
            *response = Some(issued);
        }
        Ok(())
    }

    fn advance_library_credential(
        &self,
        source: &mut SodaCredential,
        stored: &mut Option<StoredAccountCredential>,
        refreshed: SodaCredential,
    ) -> Result<()> {
        self.accept_account_read(source, stored.as_ref(), &refreshed)?;
        if let Some(previous) = stored {
            *previous = StoredAccountCredential::new(
                Platform::Soda,
                &previous.account,
                SODA_CREDENTIAL_KIND,
                refreshed.serialize()?,
            )?;
        }
        *source = refreshed;
        Ok(())
    }

    async fn verified_account_source(
        &self,
        requested_user: Option<&str>,
        account: Option<&str>,
    ) -> Result<(SodaCredential, Option<StoredAccountCredential>)> {
        let (mut source, mut stored) = self.collection_source(requested_user, account)?;
        self.verify_collection_source(requested_user, account, &mut source, &mut stored)
            .await?;
        Ok((source, stored))
    }

    fn collection_source(
        &self,
        requested_user: Option<&str>,
        account: Option<&str>,
    ) -> Result<(SodaCredential, Option<StoredAccountCredential>)> {
        if requested_user.is_some_and(|id| {
            id.is_empty()
                || id.len() > 32
                || id.starts_with('0')
                || !id.bytes().all(|c| c.is_ascii_digit())
        }) {
            return Err(soda_invalid_request(
                "Soda user ID must be a canonical positive decimal",
            ));
        }
        let alias = account.unwrap_or("default");
        let selected = self
            .selected_credential(alias)?
            .ok_or_else(soda_authentication_required)?;
        if requested_user.is_some_and(|id| selected.0.user_id().is_some_and(|owner| owner != id)) {
            return Err(TuneWeaveError::new(
                ErrorCode::PermissionDenied,
                "Soda account collections are available only for the selected account",
            )
            .with_platform(Platform::Soda));
        }
        Ok(selected)
    }

    async fn verify_collection_source(
        &self,
        requested_user: Option<&str>,
        account: Option<&str>,
        source: &mut SodaCredential,
        stored: &mut Option<StoredAccountCredential>,
    ) -> Result<()> {
        let alias = account.unwrap_or("default");
        let mismatch = || {
            TuneWeaveError::new(
                ErrorCode::PermissionDenied,
                "Soda account collections are available only for the selected account",
            )
            .with_platform(Platform::Soda)
        };
        self.ensure_account_snapshot_current(Some(alias), source)?;
        let verified = self.client.account(alias, source).await;
        self.ensure_account_snapshot_current(Some(alias), source)?;
        let verified = verified?;
        if requested_user.is_some_and(|id| verified.credential.user_id() != Some(id)) {
            return Err(mismatch());
        }
        self.advance_library_credential(source, stored, verified.credential)
    }

    async fn read_library_section(
        &self,
        section: LibrarySection,
        source: &mut SodaCredential,
        stored: &mut Option<StoredAccountCredential>,
    ) -> Result<Vec<Playlist>> {
        Ok(self
            .read_library_section_snapshot(section, source, stored)
            .await?
            .items)
    }

    async fn read_library_section_snapshot(
        &self,
        section: LibrarySection,
        source: &mut SodaCredential,
        stored: &mut Option<StoredAccountCredential>,
    ) -> Result<library_operations::LibraryPlaylistSnapshot> {
        let mut items = Vec::new();
        let mut pagination = LibraryPagination::default();
        let mut cursor = "0".to_owned();
        loop {
            let page = self.client.library_page(section, &cursor, source).await;
            self.ensure_account_snapshot_current(
                stored.as_ref().map(|s| s.account.as_str()),
                source,
            )?;
            let page = page?;
            let next = pagination.accept(&cursor, &page)?;
            self.advance_library_credential(source, stored, page.credential)?;
            items.extend(page.items);
            let Some(next) = next else { break };
            cursor = next;
        }
        Ok(library_operations::LibraryPlaylistSnapshot {
            items,
            absence_proven: pagination.absence_is_proven(),
        })
    }

    async fn account_track_snapshot(
        &self,
        identity: &SodaTrackIdentity,
        account: Option<&str>,
    ) -> Result<Option<SodaAccountTrack>> {
        if account.is_none() && self.caller_credential.is_none() {
            return Ok(None);
        }
        let account = account.unwrap_or("default");
        let (mut source, mut stored) = self
            .selected_credential(account)?
            .ok_or_else(soda_authentication_required)?;
        let verified = self.client.account(account, &source).await;
        self.ensure_account_snapshot_current(Some(account), &source)?;
        let verified = verified?;
        self.advance_library_credential(&mut source, &mut stored, verified.credential)?;
        let snapshot = self.client.account_track(identity, &source).await;
        self.ensure_account_snapshot_current(Some(account), &source)?;
        let snapshot = snapshot?;
        self.advance_library_credential(&mut source, &mut stored, snapshot.credential.clone())?;
        Ok(Some(snapshot))
    }

    async fn media_playback(
        &self,
        identity: &SodaTrackIdentity,
        request: &StreamRequest,
    ) -> Result<(SodaPlayback, Option<SodaCredential>)> {
        let bitrate = requested_media_bitrate(request);
        if let Some(snapshot) = self
            .account_track_snapshot(identity, request.account.as_deref())
            .await?
        {
            let playback = snapshot.playback(&self.client, bitrate).await;
            self.ensure_account_snapshot_current(request.account.as_deref(), &snapshot.credential)?;
            return Ok((playback?, Some(snapshot.credential)));
        }
        Ok((self.client.playback(identity, bitrate).await?, None))
    }

    fn ensure_account_snapshot_current(
        &self,
        account: Option<&str>,
        snapshot: &SodaCredential,
    ) -> Result<()> {
        let current = self.selected_credential(account.unwrap_or("default"))?;
        if current
            .as_ref()
            .is_none_or(|(current, _)| current != snapshot)
        {
            self.discard_response_credential_after_error(ErrorCode::Conflict)?;
            return Err(soda_session_changed());
        }
        Ok(())
    }

    fn media_location(
        &self,
        identity: &SodaTrackIdentity,
        playback: &SodaPlayback,
        request: &StreamRequest,
        credential: Option<&SodaCredential>,
    ) -> Result<(String, BTreeMap<String, String>)> {
        let mut url = local_content_url(identity, playback);
        let mut headers = BTreeMap::new();
        if self.caller_credential.is_some() {
            let credential = credential.ok_or_else(soda_credential_lock_error)?;
            let issued = CallerCredential::issue(&ProviderCredential::new(
                Platform::Soda,
                SODA_CREDENTIAL_KIND,
                credential.serialize()?,
                None,
            )?)?;
            headers.insert(CALLER_CREDENTIAL_HEADER.to_owned(), issued.value);
        } else if let Some(account) = request.account.as_deref() {
            let query = url::form_urlencoded::Serializer::new(String::new())
                .append_pair("account", account)
                .finish();
            url.push('&');
            url.push_str(&query);
        }
        Ok((url, headers))
    }

    fn stored_credential(&self, account: &str) -> Result<Option<StoredAccountCredential>> {
        let Some(store) = &self.credential_store else {
            return Ok(None);
        };
        Ok(store
            .load_platform(Platform::Soda)?
            .into_iter()
            .find(|stored| stored.account == account))
    }

    fn validate_session_operation(
        &self,
        account: &str,
        source: Option<&ProviderCredential>,
        mode: CredentialMode,
    ) -> Result<()> {
        validate_soda_login_account(account, mode)?;
        if self.caller_credential.is_some() {
            return Err(soda_invalid_request(
                "use the base Soda provider and supply the caller credential explicitly for session ownership operations",
            ));
        }
        if (source.is_some() && mode == CredentialMode::Server)
            || (source.is_none() && mode == CredentialMode::Client)
        {
            return Err(soda_invalid_request(
                "Soda session credential source does not match its ownership mode",
            ));
        }
        Ok(())
    }

    fn session_source(
        &self,
        account: &str,
        source: Option<&ProviderCredential>,
        mode: CredentialMode,
    ) -> Result<(SodaCredential, Option<StoredAccountCredential>)> {
        self.validate_session_operation(account, source, mode)?;
        if let Some(source) = source {
            let caller = parse_soda_caller_credential(source)?;
            if mode == CredentialMode::Client {
                return Ok((caller, None));
            }
            let (server, stored) = self
                .selected_credential(account)?
                .ok_or_else(soda_authentication_required)?;
            if !server.same_login(&caller) {
                return Err(soda_session_changed());
            }
            return Ok((server, stored));
        }
        self.selected_credential(account)?
            .ok_or_else(soda_authentication_required)
    }
}

impl SodaProvider {
    async fn qr_poll_result(
        &self,
        provider_transaction_id: &str,
        outcome: SodaQrPollOutcome,
        access: &mut crate::login::SodaQrAccess,
    ) -> Result<ProviderQrPoll> {
        match outcome {
            SodaQrPollOutcome::Waiting => Ok(ProviderQrPoll {
                verification: None,
                state: AuthState::Waiting,
                message: Some("waiting for Soda QR scan".to_owned()),
                profile: None,
                credential: None,
            }),
            SodaQrPollOutcome::Scanned => Ok(ProviderQrPoll {
                verification: None,
                state: AuthState::Scanned,
                message: Some("Soda QR scanned; waiting for confirmation".to_owned()),
                profile: None,
                credential: None,
            }),
            SodaQrPollOutcome::AdditionalVerificationRequired => Ok(ProviderQrPoll {
                verification: Some(
                    self.qr_transactions
                        .verification(provider_transaction_id)
                        .await?,
                ),
                state: AuthState::VerificationRequired,
                message: Some(
                    "Soda QR confirmed; additional account verification is required".to_owned(),
                ),
                profile: None,
                credential: None,
            }),
            SodaQrPollOutcome::Expired => Ok(ProviderQrPoll {
                verification: None,
                state: AuthState::Expired,
                message: Some("Soda QR login expired".to_owned()),
                profile: None,
                credential: None,
            }),
            SodaQrPollOutcome::Failed { code } => Ok(ProviderQrPoll {
                verification: None,
                state: AuthState::Failed,
                message: Some(format!("Soda QR login failed ({code})")),
                profile: None,
                credential: None,
            }),
            SodaQrPollOutcome::Confirmed(credential) => {
                let result = self
                    .finish_authentication_with_lease(
                        &credential,
                        access
                            .authentication
                            .as_ref()
                            .ok_or_else(soda_credential_lock_error)?,
                    )
                    .await?;
                access.finished = true;
                Ok(ProviderQrPoll {
                    verification: None,
                    state: AuthState::Confirmed,
                    message: Some("Soda account authenticated".to_owned()),
                    profile: Some(result.profile),
                    credential: result.credential,
                })
            }
        }
    }
}

fn soda_credential_lock_error() -> TuneWeaveError {
    TuneWeaveError::new(
        ErrorCode::InternalError,
        "Soda credential state is unavailable",
    )
    .with_platform(Platform::Soda)
}

fn soda_session_changed() -> TuneWeaveError {
    TuneWeaveError::new(
        ErrorCode::Conflict,
        "Soda session changed while the request was in flight",
    )
    .with_platform(Platform::Soda)
}

fn soda_authentication_required() -> TuneWeaveError {
    TuneWeaveError::new(
        ErrorCode::AuthenticationRequired,
        "the selected Soda account session is unavailable",
    )
    .with_platform(Platform::Soda)
}

#[async_trait]
impl MusicProvider for SodaProvider {
    fn platform(&self) -> Platform {
        Platform::Soda
    }

    fn name(&self) -> &'static str {
        "Soda Music"
    }

    fn with_caller_credential(
        &self,
        credential: &ProviderCredential,
    ) -> Result<Arc<dyn MusicProvider>> {
        Ok(Arc::new(self.caller_credential_scope(credential)?))
    }

    fn capabilities(&self) -> BTreeSet<Capability> {
        BTreeSet::from([
            Capability::QrLogin,
            Capability::QrLoginVerification,
            Capability::CredentialImport,
            Capability::SessionManagement,
            Capability::SessionRevocation,
            Capability::CallerManagedCredentials,
            Capability::AccountProfile,
            Capability::UserProfileModern,
            Capability::UserMembership,
            Capability::UserMembershipClientInfo,
            Capability::AccountPlaylists,
            Capability::PlaylistWrite,
            Capability::PlaylistVisibilityWrite,
            Capability::AccountAlbums,
            Capability::AccountDigitalAlbums,
            Capability::AccountFollowingArtists,
            Capability::AlbumSubscriptionWrite,
            Capability::Favorites,
            Capability::TrackSubscriptionWrite,
            Capability::AlbumDetail,
            Capability::ArtistDetail,
            Capability::ArtistOverview,
            Capability::ArtistTracks,
            Capability::ArtistAlbums,
            Capability::ArtistSubscriptionWrite,
            Capability::AudioDownload,
            Capability::AudioStream,
            Capability::PlaylistRead,
            Capability::PlaylistSubscriptionWrite,
            Capability::SearchTracks,
            Capability::SearchAlbums,
            Capability::SearchPlaylists,
            Capability::SearchArtists,
            Capability::SearchSuggestions,
            Capability::TrackDetail,
            Capability::Lyrics,
            Capability::TrackAvailability,
        ])
    }

    fn take_response_credential(&self) -> Result<Option<ProviderCredential>> {
        Ok(self
            .response_credential
            .lock()
            .map_err(|_| soda_credential_lock_error())?
            .take())
    }

    async fn import_credential(
        &self,
        request: &CredentialImportRequest,
        mode: CredentialMode,
    ) -> Result<ProviderAuthResult> {
        validate_soda_login_account(&request.account, mode)?;
        let ImportedCredential::Cookie { value } = &request.credential;
        let credential = SodaCredential::import_cookie_header(value)?;
        self.finish_authentication(&request.account, &credential, mode, None)
            .await
    }

    async fn refresh_session(&self, account: &str) -> Result<AccountProfile> {
        Ok(self
            .refresh_session_with_ownership(account, None, CredentialMode::Server)
            .await?
            .profile)
    }

    async fn refresh_session_with_ownership(
        &self,
        account: &str,
        source_credential: Option<&ProviderCredential>,
        mode: CredentialMode,
    ) -> Result<ProviderAuthResult> {
        let (source, stored) = self.session_source(account, source_credential, mode)?;
        // The account endpoint revalidates identity and can rotate cookies. It does not
        // promise to extend the upstream expiry or revive an already expired session.
        let result = self.client.account(account, &source).await;
        if let Some(expected) = &stored {
            if self.stored_credential(account)?.as_ref() != Some(expected) {
                return Err(soda_session_changed());
            }
        }
        let verified = result?;
        let secret = verified.credential.serialize()?;
        let credential = mode
            .returns_to_caller()
            .then(|| ProviderCredential::new(Platform::Soda, SODA_CREDENTIAL_KIND, &secret, None))
            .transpose()?;
        if mode.persists_on_server() {
            let stored = stored.as_ref().ok_or_else(soda_credential_lock_error)?;
            let replacement = StoredAccountCredential::new(
                Platform::Soda,
                account,
                SODA_CREDENTIAL_KIND,
                &secret,
            )?;
            let store = self
                .credential_store
                .as_ref()
                .ok_or_else(soda_credential_lock_error)?;
            // Check even an unchanged reply: a concurrent logout must prevent exporting
            // the old server credential through an explicitly requested both response.
            if !store.compare_exchange(stored, Some(&replacement))? {
                return Err(soda_session_changed());
            }
        }
        let mut profile = verified.profile;
        profile
            .extensions
            .insert("credential_kind".to_owned(), json!(SODA_CREDENTIAL_KIND));
        profile
            .extensions
            .insert("refreshed".to_owned(), json!(verified.credential != source));
        profile
            .extensions
            .insert("refresh_method".to_owned(), json!("account_revalidation"));
        Ok(ProviderAuthResult {
            profile,
            credential,
        })
    }

    async fn logout(&self, account: &str) -> Result<bool> {
        Ok(self
            .logout_with_ownership(account, None, CredentialMode::Server)
            .await?
            .removed)
    }

    async fn logout_with_ownership(
        &self,
        account: &str,
        source_credential: Option<&ProviderCredential>,
        mode: CredentialMode,
    ) -> Result<ProviderLogoutResult> {
        self.validate_session_operation(account, source_credential, mode)?;
        if source_credential.is_none() && mode != CredentialMode::Server {
            return Err(soda_invalid_request(
                "caller-managed Soda logout requires a caller credential",
            ));
        }
        let caller = source_credential
            .map(parse_soda_caller_credential)
            .transpose()?;
        let stored = if mode.persists_on_server() {
            self.stored_credential(account)?
        } else {
            None
        };
        if let (Some(caller), Some(stored)) = (&caller, &stored) {
            if stored.kind != SODA_CREDENTIAL_KIND
                || !SodaCredential::parse(stored.secret())?.same_login(caller)
            {
                return Err(soda_session_changed());
            }
        }
        // This operation removes TuneWeave's selected alias only. Upstream revocation
        // is a separate explicit operation; caller-managed copies are discarded locally.
        let removed = if mode.persists_on_server() {
            let (removed, cancelled) = self.auth_transactions.cancel_server(account, || {
                if self.stored_credential(account)? != stored {
                    return Err(soda_session_changed());
                }
                let removed = if let Some(stored) = &stored {
                    let store = self
                        .credential_store
                        .as_ref()
                        .ok_or_else(soda_credential_lock_error)?;
                    if !store.compare_exchange(stored, None)? {
                        return Err(soda_session_changed());
                    }
                    true
                } else {
                    false
                };
                let cancelled = self.qr_transactions.cancel_server(account)?;
                Ok((removed, cancelled))
            })?;
            drop(cancelled);
            removed
        } else {
            false
        };
        Ok(ProviderLogoutResult {
            removed,
            caller_credential_discard_required: caller.is_some(),
        })
    }

    async fn session_profile(&self, account: &str) -> Result<AccountProfile> {
        let empty = || AccountProfile {
            platform: Platform::Soda,
            account: account.to_owned(),
            user_id: None,
            nickname: None,
            avatar_url: None,
            authenticated: false,
            extensions: BTreeMap::new(),
        };
        let Some((source, stored)) = self.selected_credential(account)? else {
            return Ok(empty());
        };
        let result = self.client.account(account, &source).await;
        self.ensure_account_snapshot_current(Some(account), &source)?;
        let verified = match result {
            Ok(verified) => verified,
            Err(error) if error.code == ErrorCode::AuthenticationRequired => return Ok(empty()),
            Err(error) => return Err(error),
        };
        self.accept_account_read(&source, stored.as_ref(), &verified.credential)?;
        Ok(verified.profile)
    }

    async fn revoke_session_with_ownership(
        &self,
        account: &str,
        source: Option<&ProviderCredential>,
        mode: CredentialMode,
    ) -> Result<tuneweave_core::ProviderSessionRevocationResult> {
        self.revoke_owned_session(account, source, mode, std::time::Duration::from_secs(45))
            .await
    }

    async fn user_membership(
        &self,
        id: Option<&str>,
        account: Option<&str>,
    ) -> Result<MembershipSummary> {
        self.read_membership(id, account, std::time::Duration::from_secs(45))
            .await
    }

    async fn user_membership_client_info(
        &self,
        id: Option<&str>,
        account: Option<&str>,
    ) -> Result<MembershipSummary> {
        self.user_membership(id, account).await
    }

    async fn account_playlists(&self, request: &PageRequest) -> Result<Page<Playlist>> {
        self.read_library_playlists(request, std::time::Duration::from_secs(45))
            .await
    }

    async fn create_playlist(
        &self,
        request: &tuneweave_core::PlaylistCreateRequest,
    ) -> Result<tuneweave_core::PlaylistMutationResult> {
        self.create_owned_playlist(request).await
    }

    async fn update_playlist(
        &self,
        id: &str,
        request: &tuneweave_core::PlaylistUpdateRequest,
    ) -> Result<tuneweave_core::PlaylistMutationResult> {
        SodaProvider::update_playlist_metadata(self, id, request).await
    }

    async fn update_playlist_visibility(
        &self,
        id: &str,
        request: &tuneweave_core::PlaylistVisibilityUpdateRequest,
    ) -> Result<tuneweave_core::PlaylistMutationResult> {
        SodaProvider::update_playlist_visibility(self, id, request).await
    }

    async fn delete_playlists(
        &self,
        request: &tuneweave_core::PlaylistDeleteRequest,
    ) -> Result<tuneweave_core::PlaylistDeleteResult> {
        SodaProvider::delete_owned_playlists(self, request).await
    }

    async fn mutate_playlist_items(
        &self,
        id: &str,
        action: tuneweave_core::PlaylistItemMutationAction,
        request: &tuneweave_core::PlaylistItemMutationRequest,
    ) -> Result<tuneweave_core::PlaylistItemMutationResult> {
        SodaProvider::mutate_owned_playlist_items(self, id, action, request).await
    }

    async fn reorder_playlist_tracks(
        &self,
        id: &str,
        request: &tuneweave_core::PlaylistTrackOrderRequest,
    ) -> Result<tuneweave_core::PlaylistTrackOrderResult> {
        SodaProvider::reorder_owned_playlist_tracks(self, id, request).await
    }

    async fn account_albums(&self, request: &PageRequest) -> Result<Page<Album>> {
        self.read_album_collections(None, request).await
    }

    async fn account_digital_albums(&self, request: &PageRequest) -> Result<Page<DigitalAlbum>> {
        self.read_account_digital_albums(request).await
    }

    async fn account_following_artists(&self, request: &PageRequest) -> Result<Page<Artist>> {
        self.read_account_following_artists(request).await
    }

    async fn user_favorite_albums(
        &self,
        user_id: &str,
        request: &PageRequest,
    ) -> Result<Page<Album>> {
        self.read_album_collections(Some(user_id), request).await
    }

    async fn set_album_subscription(
        &self,
        id: &str,
        subscribed: bool,
        account: Option<&str>,
    ) -> Result<SubscriptionResult> {
        self.change_album_collection(id, subscribed, account).await
    }

    async fn set_artist_subscription(
        &self,
        id: &str,
        subscribed: bool,
        account: Option<&str>,
    ) -> Result<SubscriptionResult> {
        self.change_artist_collection(id, subscribed, account).await
    }

    async fn set_album_subscriptions(
        &self,
        ids: &[String],
        subscribed: bool,
        account: Option<&str>,
    ) -> Result<Vec<SubscriptionResult>> {
        self.change_album_collections(ids, subscribed, account)
            .await
    }

    async fn favorite_playlist(&self, account: Option<&str>) -> Result<Playlist> {
        Ok(self.read_favorite_snapshot(None, account).await?.playlist)
    }

    async fn favorite_tracks(&self, request: &PageRequest) -> Result<Page<Track>> {
        validate_playlist_page(request)?;
        Ok(self
            .read_favorite_snapshot(None, request.account.as_deref())
            .await?
            .into_page(request))
    }

    async fn user_favorite_playlist(
        &self,
        user_id: &str,
        account: Option<&str>,
    ) -> Result<Playlist> {
        Ok(self
            .read_favorite_snapshot(Some(user_id), account)
            .await?
            .playlist)
    }

    async fn user_favorite_tracks(
        &self,
        user_id: &str,
        request: &PageRequest,
    ) -> Result<Page<Track>> {
        validate_playlist_page(request)?;
        Ok(self
            .read_favorite_snapshot(Some(user_id), request.account.as_deref())
            .await?
            .into_page(request))
    }

    async fn set_track_subscription(
        &self,
        id: &str,
        subscribed: bool,
        account: Option<&str>,
    ) -> Result<SubscriptionResult> {
        self.set_favorite_track(id, subscribed, account).await
    }

    async fn set_playlist_subscription(
        &self,
        id: &str,
        subscribed: bool,
        account: Option<&str>,
    ) -> Result<SubscriptionResult> {
        self.change_playlist_collection(id, subscribed, account, std::time::Duration::from_secs(45))
            .await
    }

    async fn user_profile(
        &self,
        id: &str,
        backend: UserProfileBackend,
        account: Option<&str>,
    ) -> Result<UserProfile> {
        if backend != UserProfileBackend::Modern {
            return Err(TuneWeaveError::unsupported(
                Platform::Soda,
                Capability::UserProfileLegacy,
            ));
        }
        let account = account
            .or_else(|| self.caller_credential.as_ref().map(|_| "default"))
            .ok_or_else(|| {
                TuneWeaveError::new(
                    ErrorCode::AuthenticationRequired,
                    "Soda user profiles require the selected account session",
                )
                .with_platform(Platform::Soda)
            })?;
        let profile = self.session_profile(account).await?;
        if !profile.authenticated {
            return Err(TuneWeaveError::new(
                ErrorCode::AuthenticationRequired,
                "Soda account is not authenticated",
            )
            .with_platform(Platform::Soda));
        }
        if profile.user_id.as_deref() != Some(id) {
            return Err(TuneWeaveError::new(
                ErrorCode::PermissionDenied,
                "Soda account profile cannot resolve a different user",
            )
            .with_platform(Platform::Soda));
        }
        crate::account::into_user_profile(profile)
    }

    async fn start_qr_login(&self, login_type: Option<&str>) -> Result<ProviderQrStart> {
        self.start_qr_login_with_mode(login_type, CredentialMode::Server)
            .await
    }

    async fn start_qr_login_with_mode(
        &self,
        login_type: Option<&str>,
        mode: CredentialMode,
    ) -> Result<ProviderQrStart> {
        self.start_bound_qr(login_type, None, mode, None).await
    }

    async fn start_qr_login_for_account(
        &self,
        login_type: Option<&str>,
        account: &str,
        mode: CredentialMode,
    ) -> Result<ProviderQrStart> {
        self.start_bound_qr(login_type, Some(account), mode, None)
            .await
    }

    async fn start_qr_login_for_account_with_context(
        &self,
        login_type: Option<&str>,
        account: &str,
        mode: CredentialMode,
        client_context: Option<serde_json::Value>,
    ) -> Result<ProviderQrStart> {
        self.start_bound_qr(login_type, Some(account), mode, client_context)
            .await
    }

    async fn poll_qr_login(
        &self,
        provider_transaction_id: &str,
        account: &str,
    ) -> Result<ProviderQrPoll> {
        self.poll_qr_login_with_mode(provider_transaction_id, account, CredentialMode::Server)
            .await
    }

    async fn poll_qr_login_with_mode(
        &self,
        provider_transaction_id: &str,
        account: &str,
        mode: CredentialMode,
    ) -> Result<ProviderQrPoll> {
        self.continue_bound_qr(provider_transaction_id, account, mode, None)
            .await
    }

    async fn verify_qr_login(
        &self,
        provider_transaction_id: &str,
        account: &str,
        mode: CredentialMode,
        action: &tuneweave_core::QrVerificationAction,
    ) -> Result<ProviderQrPoll> {
        self.continue_bound_qr(provider_transaction_id, account, mode, Some(action))
            .await
    }

    async fn search_suggestions(
        &self,
        request: &SearchSuggestionRequest,
    ) -> Result<SearchSuggestionList> {
        if request.client == SearchSuggestionClient::Web {
            return Err(TuneWeaveError::new(
                ErrorCode::CapabilityNotSupported,
                "Soda suggestions support PC and Android Mobile clients; Web is unsupported",
            )
            .with_platform(Platform::Soda));
        }
        crate::client::validate_suggestion_query(&request.query)?;
        if request.client == SearchSuggestionClient::Mobile {
            if request.account.is_some() || self.caller_credential.is_some() {
                return self
                    .read_account_suggestions(request, std::time::Duration::from_secs(45))
                    .await;
            }
            return self.client.mobile_search_suggestions(&request.query).await;
        }
        if request.account.is_some() || self.caller_credential.is_some() {
            return self
                .read_account_suggestions(request, std::time::Duration::from_secs(45))
                .await;
        }
        self.client.pc_search_suggestions(&request.query).await
    }

    async fn search(&self, query: &SearchQuery) -> Result<Page<Track>> {
        if query.account.is_some() || self.caller_credential.is_some() {
            if query.kind != SearchKind::Track {
                return Err(TuneWeaveError::unsupported(
                    Platform::Soda,
                    Capability::SearchTracks,
                ));
            }
            let page = self
                .read_account_search(query, std::time::Duration::from_secs(45))
                .await?;
            let tracks = page
                .items
                .into_iter()
                .map(|item| match item {
                    SearchItem::Track(track) => Ok(track),
                    _ => Err(soda_upstream_error(
                        "Soda track search returned another media type",
                    )),
                })
                .collect::<Result<Vec<_>>>()?;
            return Ok(Page {
                items: tracks,
                pagination: page.pagination,
            });
        }
        validate_search_query(query)?;
        collect_search_pages(
            query,
            "official_android_track_search",
            |cursor| async move {
                let page = self
                    .client
                    .search_tracks_page(query.query.trim(), cursor)
                    .await?;
                Ok(SearchPhysicalPage {
                    items: page.tracks,
                    next_cursor: page.next_cursor,
                    has_more: page.has_more,
                })
            },
        )
        .await
    }

    async fn search_catalog(&self, query: &SearchQuery) -> Result<Page<SearchItem>> {
        if query.account.is_some() || self.caller_credential.is_some() {
            return self
                .read_account_search(query, std::time::Duration::from_secs(45))
                .await;
        }
        let kind = match query.kind {
            SearchKind::Track => {
                let page = self.search(query).await?;
                return Ok(Page {
                    items: page.items.into_iter().map(SearchItem::Track).collect(),
                    pagination: page.pagination,
                });
            }
            SearchKind::Album => SodaCatalogKind::Album,
            SearchKind::Playlist => SodaCatalogKind::Playlist,
            SearchKind::Artist => SodaCatalogKind::Artist,
            _ => {
                return Err(TuneWeaveError::new(
                    ErrorCode::CapabilityNotSupported,
                    "Soda does not support this catalogue search kind",
                )
                .with_platform(Platform::Soda));
            }
        };
        validate_search_options(query)?;
        collect_search_pages(query, kind.backend(), |cursor| async move {
            let page = self
                .client
                .search_catalog_page(kind, query.query.trim(), cursor)
                .await?;
            Ok(SearchPhysicalPage {
                items: page.items,
                next_cursor: page.next_cursor,
                has_more: page.has_more,
            })
        })
        .await
    }

    async fn track(&self, id: &str, account: Option<&str>) -> Result<Track> {
        let identity = self.client.resolve_track_identity(id).await?;
        if let Some(snapshot) = self.account_track_snapshot(&identity, account).await? {
            return Ok(snapshot.into_track());
        }
        self.client.track_detail(&identity).await
    }

    async fn artist(&self, id: &str, account: Option<&str>) -> Result<Artist> {
        if let Some(page) = self
            .read_account_artist(id, account, std::time::Duration::from_secs(60))
            .await?
        {
            return Ok(page.artist);
        }
        validate_public_artist(id, account, self.caller_credential.is_some())?;
        Ok(self.client.artist_page(id).await?.artist)
    }

    async fn artist_overview(&self, id: &str, account: Option<&str>) -> Result<ArtistOverview> {
        if let Some(page) = self
            .read_account_artist(id, account, std::time::Duration::from_secs(60))
            .await?
        {
            return Ok(page);
        }
        validate_public_artist(id, account, self.caller_credential.is_some())?;
        let page = self.client.artist_page(id).await?;
        let count = page.artist.track_count.ok_or_else(|| {
            soda_upstream_error(
                "Soda artist overview cannot establish whether its track preview is complete",
            )
        })?;
        Ok(ArtistOverview {
            has_more_tracks: count > page.featured_tracks.len() as u64,
            artist: page.artist,
            featured_tracks: page.featured_tracks,
            extensions: Extensions::from([(
                "backend".to_owned(),
                json!("official_web_artist_share"),
            )]),
        })
    }

    async fn artist_tracks(
        &self,
        id: &str,
        request: &tuneweave_core::ArtistTrackListRequest,
    ) -> Result<Page<Track>> {
        self.read_artist_tracks(id, request).await
    }

    async fn artist_albums(&self, id: &str, request: &PageRequest) -> Result<Page<Album>> {
        self.read_artist_albums(id, request).await
    }

    async fn lyrics(&self, id: &str, account: Option<&str>) -> Result<Lyrics> {
        let identity = self.client.resolve_track_identity(id).await?;
        if let Some(snapshot) = self.account_track_snapshot(&identity, account).await? {
            return snapshot.lyrics();
        }
        self.client.lyrics(&identity).await
    }

    async fn lyrics_with_options(&self, id: &str, request: &LyricsRequest) -> Result<Lyrics> {
        validate_lyrics_request(request.song_type, request.singing_annotations)?;
        self.lyrics(id, request.account.as_deref()).await
    }

    async fn track_availability(
        &self,
        id: &str,
        request: &TrackAvailabilityRequest,
    ) -> Result<TrackAvailability> {
        validate_availability_request(request)?;
        let identity = self.client.resolve_track_identity(id).await?;
        if let Some(snapshot) = self
            .account_track_snapshot(&identity, request.account.as_deref())
            .await?
        {
            let result = snapshot.availability(&self.client, request).await;
            self.ensure_account_snapshot_current(request.account.as_deref(), &snapshot.credential)?;
            return result;
        }
        self.client.track_availability(&identity, request).await
    }

    async fn stream(&self, track: &Track, request: &StreamRequest) -> Result<MediaStream> {
        let identity = canonical_media_identity(track)?;
        validate_media_request(request)?;
        let (playback, credential) = self.media_playback(&identity, request).await?;
        let (url, headers) =
            self.media_location(&identity, &playback, request, credential.as_ref())?;
        Ok(MediaStream {
            url,
            backup_urls: Vec::new(),
            headers,
            expires_at: None,
            format: Some(delivery_format(&playback).to_owned()),
            codec: Some(playback.codec.clone()),
            bitrate: Some(playback.bitrate),
            size: playback.size,
            duration_ms: Some(playback.duration_ms),
            requested_quality: request.quality,
            actual_quality: playback.quality,
            trial: playback.preview.then(|| TrialWindow {
                start_ms: playback.preview_start_ms.unwrap_or_default(),
                end_ms: playback
                    .preview_start_ms
                    .unwrap_or_default()
                    .saturating_add(playback.preview_duration_ms.unwrap_or(playback.duration_ms)),
            }),
            origin_track: Some(track.resource_ref.clone()),
            resolved_track: track.resource_ref.clone(),
            resolved_platform: Platform::Soda,
            match_score: Some(1.0),
            attempts: Vec::new(),
        })
    }

    async fn audio_content(&self, track: &Track, request: &StreamRequest) -> Result<AudioContent> {
        let identity = canonical_media_identity(track)?;
        validate_media_request(request)?;
        if let Some(snapshot) = self
            .account_track_snapshot(&identity, request.account.as_deref())
            .await?
        {
            let result = snapshot
                .audio_content(&self.client, requested_media_bitrate(request))
                .await;
            self.ensure_account_snapshot_current(request.account.as_deref(), &snapshot.credential)?;
            return result;
        }
        self.client
            .audio_content(&identity, requested_media_bitrate(request))
            .await
    }

    async fn download(&self, track: &Track, request: &StreamRequest) -> Result<MediaDownload> {
        let identity = canonical_media_identity(track)?;
        validate_media_request(request)?;
        let (playback, credential) = self.media_playback(&identity, request).await?;
        let (url, headers) =
            self.media_location(&identity, &playback, request, credential.as_ref())?;
        let available = !playback.preview;
        let mut extensions = Extensions::new();
        extensions.insert(
            "backend".to_owned(),
            json!(if credential.is_some() {
                "official_pc_track_v2"
            } else {
                "official_seo_track"
            }),
        );
        extensions.insert("local_delivery".to_owned(), json!(true));
        extensions.insert("encrypted_upstream".to_owned(), json!(playback.encrypted));
        extensions.insert("preview_url_withheld".to_owned(), json!(!available));
        Ok(MediaDownload {
            track_ref: track.resource_ref.clone(),
            platform: Platform::Soda,
            available,
            url: available.then_some(url),
            headers: if available { headers } else { BTreeMap::new() },
            expires_at: None,
            format: Some(delivery_format(&playback).to_owned()),
            codec: Some(playback.codec),
            bitrate: Some(playback.bitrate),
            size: available.then_some(playback.size).flatten(),
            duration_ms: Some(playback.duration_ms),
            requested_quality: request.quality,
            actual_quality: playback.quality,
            platform_code: Some(playback.platform_code),
            fee: None,
            message: (!available).then(|| {
                "Soda only authorized a preview; a full download is unavailable".to_owned()
            }),
            extensions,
        })
    }

    async fn album(&self, id: &str, account: Option<&str>) -> Result<Album> {
        let album_id = parse_album_id(id)?;
        if let Some(snapshot) = self.read_account_album(album_id, account).await? {
            return Ok(snapshot.page.album);
        }
        Ok(self.client.album_page(album_id).await?.album)
    }

    async fn album_tracks(&self, id: &str, request: &PageRequest) -> Result<Page<Track>> {
        let album_id = parse_album_id(id)?;
        if !(1..=100).contains(&request.limit)
            || request.offset.checked_add(request.limit).is_none()
        {
            return Err(soda_invalid_request(
                "Soda album window requires limit1-100 and a bounded offset",
            ));
        }
        if let Some(snapshot) = self
            .read_account_album(album_id, request.account.as_deref())
            .await?
        {
            return Ok(snapshot.into_page(request));
        }
        validate_album_page(request)?;
        let page = self.client.album_page(album_id).await?;
        Ok(soda_album_track_page(page.tracks, request))
    }

    async fn playlist(&self, id: &str, account: Option<&str>) -> Result<Playlist> {
        let playlist_id = parse_playlist_id(id)?;
        if let Some(snapshot) = self.read_account_playlist(playlist_id, account).await? {
            return Ok(snapshot.playlist);
        }
        Ok(self.client.playlist_page(playlist_id, 0, 1).await?.playlist)
    }

    async fn playlist_tracks(&self, id: &str, request: &PageRequest) -> Result<Page<Track>> {
        let playlist_id = parse_playlist_id(id)?;
        validate_playlist_page(request)?;
        if let Some(snapshot) = self
            .read_account_playlist(playlist_id, request.account.as_deref())
            .await?
        {
            return Ok(snapshot.into_page(request));
        }
        let requested = usize::try_from(request.limit)
            .map_err(|_| soda_invalid_request("Soda playlist limit is too large"))?;
        let requested_start = u64::from(request.offset);
        let requested_end = requested_start
            .checked_add(u64::from(request.limit))
            .ok_or_else(|| soda_invalid_request("Soda playlist window overflowed"))?;
        let mut cursor = 0_u64;
        let mut seen_cursors = BTreeSet::new();
        let mut snapshot = None;
        let mut visible_position = 0_u64;
        let mut tracks = Vec::with_capacity(requested);
        let mut pages_fetched = 0_u32;

        loop {
            if pages_fetched >= MAX_UPSTREAM_PLAYLIST_PAGES {
                return Err(soda_upstream_error(
                    "Soda playlist exceeded the bounded upstream page count",
                ));
            }
            if !seen_cursors.insert(cursor) {
                return Err(soda_upstream_error(
                    "Soda playlist repeated an upstream cursor",
                ));
            }
            let page = self
                .client
                .playlist_page(playlist_id, cursor, UPSTREAM_PLAYLIST_PAGE_SIZE)
                .await?;
            pages_fetched = pages_fetched.saturating_add(1);
            let current_snapshot = (page.total, page.raw_total, page.updated_at);
            if let Some(expected) = snapshot {
                if current_snapshot != expected {
                    return Err(soda_upstream_error(
                        "Soda playlist changed during pagination",
                    ));
                }
            } else {
                snapshot = Some(current_snapshot);
                if requested_start >= page.total {
                    return Ok(soda_playlist_page(
                        Vec::new(),
                        request,
                        page.total,
                        pages_fetched,
                        cursor,
                        page.raw_total,
                    ));
                }
            }

            for mut track in page.tracks {
                if visible_position >= requested_start && tracks.len() < requested {
                    track
                        .extensions
                        .insert("playlist_position".to_owned(), json!(visible_position));
                    tracks.push(track);
                }
                visible_position = visible_position.checked_add(1).ok_or_else(|| {
                    soda_upstream_error("Soda playlist visible position overflowed")
                })?;
            }
            if tracks.len() == requested || visible_position >= page.total {
                break;
            }
            if !page.has_more {
                return Err(soda_upstream_error(
                    "Soda playlist pagination ended before its visible track total",
                ));
            }
            cursor = page
                .next_cursor
                .ok_or_else(|| soda_upstream_error("Soda playlist continuation cursor was lost"))?;
        }

        let (total, raw_total, _) = snapshot
            .ok_or_else(|| soda_upstream_error("Soda playlist snapshot was not established"))?;
        if visible_position < total.min(requested_end) {
            return Err(soda_upstream_error(
                "Soda playlist pagination ended before the requested window",
            ));
        }
        Ok(soda_playlist_page(
            tracks,
            request,
            total,
            pages_fetched,
            cursor,
            raw_total,
        ))
    }

    async fn playlist_source(
        &self,
        id: &str,
        source_type: &str,
        account: Option<&str>,
    ) -> Result<Playlist> {
        match source_type {
            "playlist" => self.playlist(id, account).await,
            "favorite_tracks" => self.user_favorite_playlist(id, account).await,
            "album" => Ok(album_as_playlist(self.album(id, account).await?)),
            "collected_albums" => Ok(self
                .read_album_collection_source(id, account)
                .await?
                .playlist),
            "purchased_albums" => Ok(self
                .read_purchased_album_source(id, account)
                .await?
                .playlist),
            _ => Err(unsupported_soda_playlist_source_type(source_type)),
        }
    }

    async fn playlist_source_items(
        &self,
        id: &str,
        source_type: &str,
        request: &PageRequest,
    ) -> Result<Page<PlaylistPlayableItem>> {
        let page = match source_type {
            "playlist" => self.playlist_tracks(id, request).await?,
            "favorite_tracks" => self.user_favorite_tracks(id, request).await?,
            "album" => self.album_tracks(id, request).await?,
            "collected_albums" => self.album_collection_source_items(id, request).await?,
            "purchased_albums" => self.purchased_album_source_items(id, request).await?,
            _ => return Err(unsupported_soda_playlist_source_type(source_type)),
        };
        Ok(Page {
            items: page
                .items
                .into_iter()
                .map(PlaylistPlayableItem::Track)
                .collect(),
            pagination: page.pagination,
        })
    }
}

fn soda_album_track_page(tracks: Vec<Track>, request: &PageRequest) -> Page<Track> {
    let total = u64::try_from(tracks.len()).unwrap_or(u64::MAX);
    let start = usize::try_from(request.offset)
        .unwrap_or(usize::MAX)
        .min(tracks.len());
    let items = tracks
        .into_iter()
        .skip(start)
        .take(usize::try_from(request.limit).unwrap_or(usize::MAX))
        .collect::<Vec<_>>();
    let returned = u32::try_from(items.len()).unwrap_or(u32::MAX);
    let consumed = request.offset.saturating_add(returned);
    let has_more = u64::from(consumed) < total;
    Page {
        items,
        pagination: PageMeta {
            limit: request.limit,
            offset: request.offset,
            total: Some(total),
            next_offset: (has_more && returned > 0).then_some(consumed),
            has_more,
            extensions: Extensions::from([
                ("backend".to_owned(), json!("official_web_album_share")),
                ("complete_snapshot".to_owned(), json!(true)),
            ]),
        },
    }
}

fn album_as_playlist(album: Album) -> Playlist {
    let creator = album.artists.first().cloned();
    let mut extensions = album.extensions;
    extensions.insert("source_type".to_owned(), json!("album"));
    extensions.insert("album_artists".to_owned(), json!(album.artists));
    if let Some(value) = album.published_at {
        extensions.insert("published_at".to_owned(), json!(value));
    }
    if let Some(value) = album.company {
        extensions.insert("company".to_owned(), json!(value));
    }
    if let Some(value) = album.kind {
        extensions.insert("album_kind".to_owned(), json!(value));
    }
    Playlist {
        resource_ref: album.resource_ref,
        platform: album.platform,
        id: album.id,
        name: album.name,
        description: album.description,
        cover_url: album.cover_url,
        creator,
        track_count: album.track_count,
        tags: Vec::new(),
        subscribed: None,
        created_at: None,
        updated_at: None,
        extensions,
    }
}

fn soda_playlist_page(
    tracks: Vec<Track>,
    request: &PageRequest,
    total: u64,
    pages_fetched: u32,
    final_cursor: u64,
    raw_total: Option<u64>,
) -> Page<Track> {
    let returned = u32::try_from(tracks.len()).unwrap_or(u32::MAX);
    let consumed = request.offset.saturating_add(returned);
    let has_more = u64::from(consumed) < total;
    let mut extensions = Extensions::new();
    extensions.insert("backend".to_owned(), json!("official_pc_playlist_detail"));
    extensions.insert(
        "upstream_page_size".to_owned(),
        json!(UPSTREAM_PLAYLIST_PAGE_SIZE),
    );
    extensions.insert("upstream_pages_fetched".to_owned(), json!(pages_fetched));
    extensions.insert("upstream_final_cursor".to_owned(), json!(final_cursor));
    if let Some(value) = raw_total {
        extensions.insert("upstream_raw_resource_count".to_owned(), json!(value));
    }
    Page {
        items: tracks,
        pagination: PageMeta {
            limit: request.limit,
            offset: request.offset,
            total: Some(total),
            next_offset: (has_more && returned > 0).then_some(consumed),
            has_more,
            extensions,
        },
    }
}

fn parse_playlist_id(id: &str) -> Result<&str> {
    let parsed = id.parse::<u64>().map_err(|_| {
        soda_invalid_request("Soda playlist ID must be a canonical positive integer")
    })?;
    if parsed == 0 || parsed.to_string() != id {
        return Err(soda_invalid_request(
            "Soda playlist ID must be a canonical positive integer",
        ));
    }
    Ok(id)
}

fn parse_album_id(id: &str) -> Result<&str> {
    let parsed = id
        .parse::<u64>()
        .map_err(|_| soda_invalid_request("Soda album ID must be a canonical positive integer"))?;
    if parsed == 0 || parsed.to_string() != id {
        return Err(soda_invalid_request(
            "Soda album ID must be a canonical positive integer",
        ));
    }
    Ok(id)
}

fn validate_playlist_page(request: &PageRequest) -> Result<()> {
    if !(1..=100).contains(&request.limit) {
        return Err(soda_invalid_request(
            "Soda playlist limit must be between 1 and 100",
        ));
    }
    Ok(())
}

fn validate_album_page(request: &PageRequest) -> Result<()> {
    if request.account.is_some() {
        return Err(soda_invalid_request(
            "Soda public albums do not accept an account",
        ));
    }
    if !(1..=100).contains(&request.limit) {
        return Err(soda_invalid_request(
            "Soda album limit must be between 1 and 100",
        ));
    }
    Ok(())
}

fn unsupported_soda_playlist_source_type(source_type: &str) -> TuneWeaveError {
    TuneWeaveError::new(
        tuneweave_core::ErrorCode::CapabilityNotSupported,
        format!("Soda does not support playlist source type {source_type}"),
    )
    .with_platform(Platform::Soda)
    .with_details(json!({ "source_type": source_type }))
}

fn canonical_media_identity(track: &Track) -> Result<SodaTrackIdentity> {
    if track.platform != Platform::Soda
        || track.resource_ref.platform() != Platform::Soda
        || track.id != track.resource_ref.id()
    {
        return Err(soda_invalid_request(
            "Soda media resolution requires a canonical Soda track",
        ));
    }
    SodaTrackIdentity::parse(track.resource_ref.id())
}

fn validate_media_request(request: &StreamRequest) -> Result<()> {
    if request.variant != StreamVariant::Default {
        return Err(soda_invalid_request(
            "Soda media only supports the default stream variant",
        ));
    }
    if request.immersive_type.is_some() {
        return Err(soda_invalid_request(
            "Soda media does not accept immersive_type",
        ));
    }
    if request
        .bitrate
        .is_some_and(|bitrate| bitrate == 0 || bitrate > 10_000_000)
    {
        return Err(soda_invalid_request(
            "Soda media bitrate must be between 1 and 10000000",
        ));
    }
    if matches!(
        request.quality,
        Quality::Dtsx
            | Quality::Surround
            | Quality::Dolby
            | Quality::Master
            | Quality::Vivid
            | Quality::Vinyl
    ) {
        return Err(soda_invalid_request(
            "Soda media does not support the requested quality class",
        ));
    }
    Ok(())
}

fn requested_media_bitrate(request: &StreamRequest) -> u64 {
    request.bitrate.unwrap_or(match request.quality {
        Quality::Auto | Quality::Spatial => 10_000_000,
        Quality::Low => 96_000,
        Quality::Standard => 192_000,
        Quality::Higher | Quality::High => 500_000,
        Quality::Lossless => 2_000_000,
        Quality::Hires => 5_000_000,
        Quality::Dtsx
        | Quality::Surround
        | Quality::Dolby
        | Quality::Master
        | Quality::Vivid
        | Quality::Vinyl => 10_000_000,
    })
}

fn local_content_url(identity: &SodaTrackIdentity, playback: &SodaPlayback) -> String {
    format!(
        "/v1/tracks/soda:{}/stream/content?quality={}&bitrate={}",
        identity.id(),
        quality_parameter(playback.quality),
        playback.bitrate
    )
}

const fn quality_parameter(quality: Quality) -> &'static str {
    match quality {
        Quality::Auto => "auto",
        Quality::Low => "low",
        Quality::Standard => "standard",
        Quality::Higher => "higher",
        Quality::High => "high",
        Quality::Lossless => "lossless",
        Quality::Hires => "hires",
        Quality::Dtsx => "dtsx",
        Quality::Surround => "surround",
        Quality::Spatial => "spatial",
        Quality::Dolby => "dolby",
        Quality::Master => "master",
        Quality::Vivid => "vivid",
        Quality::Vinyl => "vinyl",
    }
}

fn delivery_format(playback: &SodaPlayback) -> &'static str {
    if playback.codec.eq_ignore_ascii_case("flac") {
        "flac"
    } else {
        "m4a"
    }
}

struct SearchPhysicalPage<T> {
    items: Vec<T>,
    next_cursor: Option<u32>,
    has_more: bool,
}

async fn collect_search_pages<T, F, Fut>(
    query: &SearchQuery,
    backend: &'static str,
    mut fetch: F,
) -> Result<Page<T>>
where
    F: FnMut(u32) -> Fut,
    Fut: std::future::Future<Output = Result<SearchPhysicalPage<T>>>,
{
    let start_cursor = query.offset / UPSTREAM_SEARCH_PAGE_SIZE * UPSTREAM_SEARCH_PAGE_SIZE;
    let skip = usize::try_from(query.offset % UPSTREAM_SEARCH_PAGE_SIZE)
        .map_err(|_| soda_invalid_request("Soda search offset is too large"))?;
    let requested = usize::try_from(query.limit)
        .map_err(|_| soda_invalid_request("Soda search limit is too large"))?;
    let needed = skip.saturating_add(requested);
    let mut cursor = start_cursor;
    let mut buffer = Vec::with_capacity(needed);
    let mut pages_fetched = 0_u32;
    let mut upstream_has_more = false;
    let mut upstream_next_cursor = None;

    while buffer.len() < needed {
        if pages_fetched >= MAX_UPSTREAM_PAGES_PER_SEARCH {
            return Err(soda_upstream_error(
                "Soda search exceeded the bounded upstream page count",
            ));
        }
        let page = fetch(cursor).await?;
        if page.items.len() > UPSTREAM_SEARCH_PAGE_SIZE as usize
            || (page.has_more
                && (page.items.is_empty() || !page.next_cursor.is_some_and(|next| next > cursor)))
        {
            return Err(soda_upstream_error(
                "Soda search returned an invalid page or continuation",
            ));
        }
        pages_fetched = pages_fetched.saturating_add(1);
        buffer.extend(page.items);
        upstream_has_more = page.has_more;
        upstream_next_cursor = page.next_cursor;
        if !page.has_more {
            break;
        }
        cursor = page
            .next_cursor
            .ok_or_else(|| soda_upstream_error("Soda search continuation cursor was lost"))?;
    }

    let buffered_after_skip = buffer.len().saturating_sub(skip);
    let mut items = buffer.into_iter().skip(skip).collect::<Vec<_>>();
    items.truncate(requested);
    let returned = u32::try_from(items.len()).unwrap_or(u32::MAX);
    let consumed = query.offset.saturating_add(returned);
    let has_buffered_more = buffered_after_skip > items.len();
    let has_more = has_buffered_more || upstream_has_more;
    let mut extensions = Extensions::new();
    extensions.insert("backend".to_owned(), json!(backend));
    extensions.insert(
        "upstream_page_size".to_owned(),
        json!(UPSTREAM_SEARCH_PAGE_SIZE),
    );
    extensions.insert("upstream_pages_fetched".to_owned(), json!(pages_fetched));
    extensions.insert("upstream_cursor_start".to_owned(), json!(start_cursor));
    if let Some(next_cursor) = upstream_next_cursor {
        extensions.insert("upstream_next_cursor".to_owned(), json!(next_cursor));
    }
    extensions.insert("anonymous_device_required".to_owned(), json!(false));
    extensions.insert("request_signature_required".to_owned(), json!(false));

    Ok(Page {
        items,
        pagination: PageMeta {
            limit: query.limit,
            offset: query.offset,
            total: None,
            next_offset: (has_more && returned > 0).then_some(consumed),
            has_more,
            extensions,
        },
    })
}

fn validate_search_query(query: &SearchQuery) -> Result<()> {
    if query.kind != SearchKind::Track {
        return Err(TuneWeaveError::unsupported(
            Platform::Soda,
            Capability::SearchTracks,
        ));
    }
    validate_search_options(query)
}

fn validate_public_artist(id: &str, account: Option<&str>, caller_managed: bool) -> Result<()> {
    if id.is_empty()
        || id.len() > 64
        || id.starts_with('0')
        || !id.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(soda_invalid_request(
            "Soda artist identity must be a canonical positive decimal",
        ));
    }
    if account.is_some() || caller_managed {
        return Err(soda_invalid_request(
            "Soda public artist pages do not accept account credentials",
        ));
    }
    Ok(())
}

fn validate_search_options(query: &SearchQuery) -> Result<()> {
    if query.variant != SearchVariant::Default {
        return Err(soda_invalid_request(
            "Soda public search supports only the default variant",
        ));
    }
    let text = query.query.trim();
    if text.is_empty() || text.len() > 500 {
        return Err(soda_invalid_request(
            "Soda search query must contain between 1 and 500 bytes",
        ));
    }
    if !(1..=100).contains(&query.limit) {
        return Err(soda_invalid_request(
            "Soda search limit must be between 1 and 100",
        ));
    }
    if query.offset.checked_add(query.limit).is_none() {
        return Err(soda_invalid_request(
            "Soda search offset and limit exceed the supported range",
        ));
    }
    if query.account.is_some() {
        return Err(soda_invalid_request(
            "Soda public search does not accept an account",
        ));
    }
    if query.search_id.is_some()
        || query.highlight
        || !query.selectors.is_empty()
        || query.video_filters.is_some()
    {
        return Err(soda_invalid_request(
            "Soda public search does not accept provider-specific search state",
        ));
    }
    Ok(())
}

fn validate_lyrics_request(song_type: Option<i64>, singing_annotations: bool) -> Result<()> {
    if song_type.is_some() || singing_annotations {
        return Err(soda_invalid_request(
            "Soda lyrics do not accept song_type or singing annotations",
        ));
    }
    Ok(())
}

fn validate_availability_request(request: &TrackAvailabilityRequest) -> Result<()> {
    if request.bitrate == 0 || request.bitrate > 10_000_000 {
        return Err(soda_invalid_request(
            "Soda availability bitrate must be between 1 and 10000000",
        ));
    }
    Ok(())
}

fn validate_soda_login_account(account: &str, mode: CredentialMode) -> Result<()> {
    if account != account.trim() {
        return Err(soda_invalid_request(
            "Soda account alias cannot contain surrounding whitespace",
        ));
    }
    let account = account.trim();
    if account.is_empty() {
        return Err(soda_invalid_request("Soda account alias cannot be empty"));
    }
    if account.len() > 64 {
        return Err(soda_invalid_request(
            "Soda account alias cannot exceed 64 bytes",
        ));
    }
    if mode == CredentialMode::Client && account != "default" {
        return Err(soda_invalid_request(
            "client credential mode does not accept a server account alias",
        ));
    }
    Ok(())
}

fn parse_soda_caller_credential(credential: &ProviderCredential) -> Result<SodaCredential> {
    if credential.platform != Platform::Soda {
        return Err(soda_invalid_request(
            "caller credential platform does not match Soda",
        ));
    }
    if credential.kind != SODA_CREDENTIAL_KIND {
        return Err(soda_invalid_request(
            "caller credential kind is not supported by Soda",
        ));
    }
    if credential.expires_at.is_some() {
        return Err(soda_invalid_request(
            "caller Soda credential expiry does not match its payload",
        ));
    }
    SodaCredential::parse(credential.secret())
        .map_err(|_| soda_invalid_request("caller Soda credential payload is malformed or invalid"))
}

fn soda_invalid_request(message: impl Into<String>) -> TuneWeaveError {
    TuneWeaveError::invalid_request(message).with_platform(Platform::Soda)
}

fn soda_upstream_error(message: impl Into<String>) -> TuneWeaveError {
    TuneWeaveError::new(tuneweave_core::ErrorCode::UpstreamError, message)
        .with_platform(Platform::Soda)
}

#[cfg(test)]
mod tests {
    use super::*;

    mod account_albums;
    mod account_artist_catalog;
    mod account_artists;
    mod account_digital_albums;
    mod account_following_artists;
    mod account_playlists;
    mod account_search;
    mod account_suggestions;
    mod album_collection_source;
    mod album_collections;
    mod artist_catalog;
    mod artist_collection;
    mod auth_generation;
    mod favorites;
    mod library_lifecycle;
    mod lyrics;
    mod membership;
    mod plaintext_media;
    mod playlist_create;
    mod playlist_delete;
    mod playlist_item_mutation;
    mod playlist_metadata;
    mod playlist_track_order;
    mod playlist_visibility;
    mod purchased_album_source;
    mod secondary_encryption;
    mod session_revocation;
    mod suggestions;

    fn test_soda_credential() -> SodaCredential {
        SodaCredential::test_credential("session-secret")
    }

    struct SessionFixture {
        provider: SodaProvider,
        store: Arc<tuneweave_core::FileAccountCredentialStore>,
        root: std::path::PathBuf,
    }

    impl SessionFixture {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!("soda-session-{}", rand::random::<u64>()));
            let store = Arc::new(tuneweave_core::FileAccountCredentialStore::new(&root));
            let provider = SodaProvider::new(SodaConfig {
                credential_store: Some(store.clone()),
                ..SodaConfig::default()
            })
            .unwrap();
            Self {
                provider,
                store,
                root,
            }
        }
        fn put(&self, account: &str, credential: &SodaCredential) {
            self.store
                .put(
                    &StoredAccountCredential::new(
                        Platform::Soda,
                        account,
                        SODA_CREDENTIAL_KIND,
                        credential.serialize().unwrap(),
                    )
                    .unwrap(),
                )
                .unwrap();
        }
        fn stored(&self, account: &str) -> Option<StoredAccountCredential> {
            self.store
                .load_platform(Platform::Soda)
                .unwrap()
                .into_iter()
                .find(|stored| stored.account == account)
        }
    }
    impl Drop for SessionFixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }
    fn account_reply(id: &str, cookie: Option<&str>) -> String {
        crate::test_http::json(
            &json!({"status_code":0,"my_info":{"id":id,"nickname":"listener"}}).to_string(),
            cookie,
        )
    }
    fn caller_from(credential: &SodaCredential) -> ProviderCredential {
        ProviderCredential::new(
            Platform::Soda,
            SODA_CREDENTIAL_KIND,
            credential.serialize().unwrap(),
            None,
        )
        .unwrap()
    }

    #[tokio::test]
    async fn membership_uses_selected_identity_and_rotates_only_its_credential() {
        let mut fixture = SessionFixture::new();
        let source = test_soda_credential().bind_user("123456").unwrap();
        fixture.put("personal", &source);
        let other = SodaCredential::test_credential("other-secret")
            .bind_user("654321")
            .unwrap();
        fixture.put("other", &other);
        let (origin, server) = crate::test_http::serve(vec![
            crate::test_http::json(
                r#"{"status_code":0,"my_info":{"id":"123456","is_vip":true,"vip_stage":"trial"}}"#,
                Some("sessionid_ss=member-server"),
            ),
            crate::test_http::json(
                &crate::client::membership::tests::body(
                    json!({"is_membership":true,"membership_type":"trial"}),
                )
                .to_string(),
                None,
            ),
            account_reply("123456", None),
            crate::test_http::json(
                r#"{"status_code":0,"my_info":{"id":"123456","is_vip":false}}"#,
                Some("sessionid_ss=member-client"),
            ),
            crate::test_http::json(
                &crate::client::membership::tests::body(json!({"is_membership":false})).to_string(),
                None,
            ),
            account_reply("123456", None),
        ])
        .await;
        fixture.provider.client = fixture
            .provider
            .client
            .clone()
            .with_auth_test_origin(origin);
        assert_eq!(
            fixture
                .provider
                .user_membership(Some("654321"), Some("personal"))
                .await
                .unwrap_err()
                .code,
            ErrorCode::PermissionDenied
        );
        assert_eq!(
            fixture
                .provider
                .user_membership(None, Some("missing"))
                .await
                .unwrap_err()
                .code,
            ErrorCode::AuthenticationRequired
        );
        let member = fixture
            .provider
            .user_membership(None, Some("personal"))
            .await
            .unwrap();
        assert_eq!(member.active, Some(true));
        assert_eq!(member.user_ref.unwrap().id(), "123456");
        assert_eq!(member.extensions["vip_stage"], "trial");
        assert!(member.expires_at.is_none());
        let stored = fixture.stored("personal").unwrap();
        assert!(
            SodaCredential::parse(stored.secret())
                .unwrap()
                .cookie_header()
                .unwrap()
                .contains("member-server")
        );
        assert!(
            fixture
                .provider
                .take_response_credential()
                .unwrap()
                .is_none()
        );
        let caller = fixture
            .provider
            .caller_credential_scope(&caller_from(&source))
            .unwrap();
        assert_eq!(
            caller
                .user_membership_client_info(Some("123456"), None)
                .await
                .unwrap()
                .active,
            Some(false)
        );
        let rotated = caller.take_response_credential().unwrap().unwrap();
        assert!(rotated.secret().contains("member-client"));
        assert_eq!(fixture.stored("personal").unwrap(), stored);
        assert_eq!(
            fixture.stored("other").unwrap().secret(),
            other.serialize().unwrap()
        );
        let requests = server.await.unwrap();
        assert_eq!(requests.len(), 6);
        assert!(
            requests
                .iter()
                .enumerate()
                .all(|(i, request)| request.starts_with(if i % 3 == 1 {
                    "POST /luna/pc/commerce/v2/commerce_info?"
                } else {
                    "GET /luna/pc/me?aid=386088&app_name=luna_pc"
                }))
        );
        assert!(
            requests
                .iter()
                .all(|request| !request.contains("other-secret"))
        );
    }

    #[tokio::test]
    async fn account_library_pages_created_then_saved_and_propagates_each_cookie_update() {
        for caller_managed in [false, true] {
            let mut fixture = SessionFixture::new();
            let source = test_soda_credential().bind_user("123456").unwrap();
            fixture.put("personal", &source);
            let other = SodaCredential::test_credential("other-secret")
                .bind_user("654321")
                .unwrap();
            fixture.put("other", &other);
            let (origin, server) = crate::test_http::serve(vec![
                account_reply("123456", Some("sessionid_ss=verified")),
                crate::test_http::json(r#"{"playlists":[{"id":"11","title":"created one","owner":{"id":"123456"}}],"has_more":true,"next_cursor":"c1","total_num":2}"#, Some("sessionid_ss=created-page")),
                crate::test_http::json(r#"{"playlists":[{"id":"12","title":"created two"}],"has_more":false,"total_num":2}"#, None),
                crate::test_http::json(r#"{"mixed_collections":[{"item_type":"album"},{"item_type":"playlist","playlist":{"id":"21","title":"saved one","owner":{"id":"654321"}}}],"has_more":true,"next_cursor":"s1","total_num":3}"#, Some("sessionid_ss=saved-page")),
                crate::test_http::json(r#"{"mixed_collections":[{"item_type":"playlist","playlist":{"id":"22","title":"saved two"}}],"has_more":false,"total_num":3}"#, None),
            ]).await;
            fixture.provider.client = fixture
                .provider
                .client
                .clone()
                .with_auth_test_origin(origin);
            let provider = if caller_managed {
                fixture
                    .provider
                    .caller_credential_scope(&caller_from(&source))
                    .unwrap()
            } else {
                fixture.provider.clone()
            };
            let page = provider
                .account_playlists(&PageRequest {
                    limit: 2,
                    offset: 1,
                    account: Some(
                        if caller_managed {
                            "default"
                        } else {
                            "personal"
                        }
                        .to_owned(),
                    ),
                })
                .await
                .unwrap();
            assert_eq!(
                page.items
                    .iter()
                    .map(|item| item.id.as_str())
                    .collect::<Vec<_>>(),
                ["12", "21"]
            );
            assert_eq!(page.items[0].subscribed, Some(false));
            assert_eq!(page.items[1].subscribed, Some(true));
            assert_eq!(page.items[1].extensions["owner_id"], "654321");
            assert_eq!(page.items[1].extensions["source_user_id"], "123456");
            assert_eq!(page.pagination.total, Some(4));
            assert_eq!(page.pagination.next_offset, Some(3));
            assert!(page.pagination.has_more);
            if caller_managed {
                assert!(
                    provider
                        .take_response_credential()
                        .unwrap()
                        .unwrap()
                        .secret()
                        .contains("saved-page")
                );
                assert_eq!(
                    fixture.stored("personal").unwrap().secret(),
                    source.serialize().unwrap()
                );
            } else {
                assert!(provider.take_response_credential().unwrap().is_none());
                assert!(
                    fixture
                        .stored("personal")
                        .unwrap()
                        .secret()
                        .contains("saved-page")
                );
            }
            assert_eq!(
                fixture.stored("other").unwrap().secret(),
                other.serialize().unwrap()
            );
            let requests = server.await.unwrap();
            assert_eq!(requests.len(), 5);
            for (request, cookie) in requests.iter().zip([
                "session-secret",
                "verified",
                "created-page",
                "created-page",
                "saved-page",
            ]) {
                assert!(request.contains(&format!("cookie: sessionid_ss={cookie}\r\n")));
                assert!(!request.contains("other-secret"));
            }
            assert!(requests[1].starts_with("GET /luna/pc/me/playlist?"));
            assert!(requests[2].contains("cursor=c1&count=50"));
            assert!(requests[3].starts_with("GET /luna/pc/me/collection/mixed?"));
            assert!(requests[4].contains("cursor=s1&count=500"));
            assert!(
                !requests
                    .iter()
                    .any(|request| request.contains("/user/playlist"))
            );
            let device = provider.client.login_device().unwrap();
            for request in &requests[1..] {
                assert!(request.contains("iid=&"));
                assert!(!request.contains("install_id="));
                assert!(request.contains(&format!("device_id={}", device.device_id)));
            }
        }
    }

    #[tokio::test]
    async fn account_media_metadata_uses_the_selected_source_and_preserves_preview_rights() {
        let id = "7304719759323564095";
        for caller_managed in [false, true] {
            for (operation, preview) in [(0, false), (1, false), (2, false), (2, true)] {
                let mut fixture = SessionFixture::new();
                let source = test_soda_credential().bind_user("123456").unwrap();
                fixture.put("personal", &source);
                let other = SodaCredential::test_credential("other-secret")
                    .bind_user("654321")
                    .unwrap();
                fixture.put("other", &other);
                let body =
                    String::from_utf8(crate::client::test_account_track_fixture(preview)).unwrap();
                let (origin, server) = crate::test_http::serve(vec![
                    account_reply("123456", Some("sessionid_ss=verified")),
                    crate::test_http::json(&body, Some("sessionid_ss=metadata")),
                ])
                .await;
                fixture.provider.client = fixture
                    .provider
                    .client
                    .clone()
                    .with_auth_test_origin(origin);
                let provider = if caller_managed {
                    fixture
                        .provider
                        .caller_credential_scope(&caller_from(&source))
                        .unwrap()
                } else {
                    fixture.provider.clone()
                };
                let account = if caller_managed {
                    None
                } else {
                    Some("personal")
                };
                let value = match operation {
                    0 => serde_json::to_value(provider.track(id, account).await.unwrap()).unwrap(),
                    1 => serde_json::to_value(
                        provider
                            .lyrics_with_options(
                                id,
                                &LyricsRequest {
                                    account: account.map(str::to_owned),
                                    ..LyricsRequest::default()
                                },
                            )
                            .await
                            .unwrap(),
                    )
                    .unwrap(),
                    _ => serde_json::to_value(
                        provider
                            .track_availability(
                                id,
                                &TrackAvailabilityRequest {
                                    account: account.map(str::to_owned),
                                    bitrate: 200_000,
                                },
                            )
                            .await
                            .unwrap(),
                    )
                    .unwrap(),
                };
                assert_eq!(value["extensions"]["backend"], "official_pc_track_v2");
                if operation == 2 {
                    assert_eq!(value["playable"], !preview);
                    assert_eq!(value["extensions"]["preview_available"], preview);
                }
                for secret in [
                    "session-secret",
                    "url_player_info",
                    "video_model",
                    "spade_a",
                    "other-secret",
                ] {
                    assert!(!value.to_string().contains(secret));
                }
                if caller_managed {
                    assert!(
                        provider
                            .take_response_credential()
                            .unwrap()
                            .unwrap()
                            .secret()
                            .contains("metadata")
                    );
                    assert_eq!(
                        fixture.stored("personal").unwrap().secret(),
                        source.serialize().unwrap()
                    );
                } else {
                    assert!(provider.take_response_credential().unwrap().is_none());
                    assert!(
                        fixture
                            .stored("personal")
                            .unwrap()
                            .secret()
                            .contains("metadata")
                    );
                }
                assert_eq!(
                    fixture.stored("other").unwrap().secret(),
                    other.serialize().unwrap()
                );
                let requests = server.await.unwrap();
                assert_eq!(requests.len(), 2);
                assert!(requests[0].contains("cookie: sessionid_ss=session-secret\r\n"));
                assert!(requests[1].starts_with("POST /luna/pc/track_v2?"));
                assert!(requests[1].contains("cookie: sessionid_ss=verified\r\n"));
                let body: serde_json::Value =
                    serde_json::from_str(requests[1].split_once("\r\n\r\n").unwrap().1).unwrap();
                assert_eq!(
                    body,
                    json!({"track_id":id,"media_type":"track","queue_type":"search_one_track","scene_name":"search"})
                );
                let device = provider.client.login_device().unwrap();
                assert!(requests[1].contains(&format!("device_id={}", device.device_id)));
                assert!(requests[1].contains(&format!("iid={}", device.install_id)));
                assert!(
                    !requests
                        .iter()
                        .any(|request| request.contains("/h5/seo_track"))
                );
            }
        }
    }

    #[tokio::test]
    async fn account_media_failure_keeps_valid_prior_rotations_and_never_falls_back_to_anonymous() {
        let id = "7304719759323564095";
        let mut malformed_lyric: serde_json::Value =
            serde_json::from_slice(&crate::client::test_account_track_fixture(false)).unwrap();
        malformed_lyric["lyric"]["content"] = json!("[0,1000]<0,500,0>test\u{0}");
        for (reply, lyrics, code, final_cookie) in [
            (crate::test_http::json(r#"{"status_code":1000016}"#, Some("sessionid_ss=poison")), false, ErrorCode::AuthenticationRequired, "verified"),
            (crate::test_http::json(r#"{"status_code":0,"track":{"id":"999","name":"wrong"}}"#, Some("sessionid_ss=poison")), false, ErrorCode::UpstreamError, "verified"),
            ("HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_owned(), false, ErrorCode::UpstreamError, "verified"),
            ("HTTP/1.1 302 Found\r\nLocation: https://example.invalid/media\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_owned(), false, ErrorCode::UpstreamError, "verified"),
            (crate::test_http::json(&malformed_lyric.to_string(), Some("sessionid_ss=metadata")), true, ErrorCode::UpstreamError, "metadata"),
        ] {
            let mut fixture = SessionFixture::new();
            let source = test_soda_credential().bind_user("123456").unwrap();
            let (origin, server) = crate::test_http::serve(vec![account_reply("123456", Some("sessionid_ss=verified")), reply]).await;
            fixture.provider.client = fixture.provider.client.clone().with_auth_test_origin(origin);
            let caller = fixture.provider.caller_credential_scope(&caller_from(&source)).unwrap();
            let error = if lyrics { caller.lyrics(id, None).await.unwrap_err() } else { caller.track(id, None).await.unwrap_err() };
            assert_eq!(error.code, code);
            let rotated = caller.take_response_credential().unwrap().unwrap();
            assert_eq!(parse_soda_caller_credential(&rotated).unwrap().cookie_header().unwrap(), format!("sessionid_ss={final_cookie}"));
            let requests = server.await.unwrap();
            assert_eq!(requests.len(), 2);
            assert!(!requests.iter().any(|request| request.contains("/h5/seo_track")));
        }
    }

    #[tokio::test]
    async fn account_media_late_metadata_cannot_cross_server_or_caller_login_generations() {
        for caller_managed in [false, true] {
            let mut fixture = SessionFixture::new();
            let source = test_soda_credential().bind_user("123456").unwrap();
            fixture.put("personal", &source);
            let body = String::from_utf8(crate::client::test_account_track_fixture(false)).unwrap();
            let paused = crate::test_http::serve_paused_at(
                vec![
                    account_reply("123456", None),
                    crate::test_http::json(&body, None),
                ],
                1,
            )
            .await;
            fixture.provider.client = fixture
                .provider
                .client
                .clone()
                .with_auth_test_origin(paused.origin);
            let provider = if caller_managed {
                fixture
                    .provider
                    .caller_credential_scope(&caller_from(&source))
                    .unwrap()
            } else {
                fixture.provider.clone()
            };
            let task_provider = provider.clone();
            let pending = tokio::spawn(async move {
                task_provider
                    .track(
                        "7304719759323564095",
                        if caller_managed {
                            None
                        } else {
                            Some("personal")
                        },
                    )
                    .await
            });
            paused.arrived.await.unwrap();
            let replacement = SodaCredential::test_credential("relogin")
                .bind_user("123456")
                .unwrap();
            if caller_managed {
                *provider.caller_credential.as_ref().unwrap().lock().unwrap() = replacement.clone();
            } else {
                fixture.put("personal", &replacement);
            }
            paused.release.send(()).unwrap();
            assert_eq!(
                pending.await.unwrap().unwrap_err().code,
                ErrorCode::Conflict
            );
            assert!(provider.take_response_credential().unwrap().is_none());
            if caller_managed {
                assert_eq!(
                    *provider.caller_credential.as_ref().unwrap().lock().unwrap(),
                    replacement
                );
            } else {
                assert_eq!(
                    fixture.stored("personal").unwrap().secret(),
                    replacement.serialize().unwrap()
                );
            }
            assert_eq!(paused.requests.await.unwrap().len(), 2);
        }
    }

    #[tokio::test]
    async fn playlist_collection_writes_verify_all_saved_pages_and_rotate_only_the_selected_source()
    {
        for caller_managed in [false, true] {
            for subscribed in [false, true] {
                let mut fixture = SessionFixture::new();
                let source = test_soda_credential().bind_user("123456").unwrap();
                fixture.put("personal", &source);
                let other = SodaCredential::test_credential("other-secret")
                    .bind_user("654321")
                    .unwrap();
                fixture.put("other", &other);
                let final_id = if subscribed { "21" } else { "22" };
                let (origin, server) = crate::test_http::serve(vec![
                    account_reply("123456", Some("sessionid_ss=verified")),
                    crate::test_http::json(r#"{"status_info":{"status_code":0}}"#, Some("sessionid_ss=written")),
                    crate::test_http::json(r#"{"mixed_collections":[{"item_type":"album"},{"item_type":"playlist","playlist":{"id":"11","title":"other"}}],"has_more":true,"next_cursor":"next","total_num":3}"#, Some("sessionid_ss=page-one")),
                    crate::test_http::json(&json!({"mixed_collections":[{"item_type":"playlist","playlist":{"id":final_id,"title":"last"}}],"has_more":false,"total_num":3}).to_string(), Some("sessionid_ss=page-two")),
                ]).await;
                fixture.provider.client = fixture
                    .provider
                    .client
                    .clone()
                    .with_auth_test_origin(origin);
                let provider = if caller_managed {
                    fixture
                        .provider
                        .caller_credential_scope(&caller_from(&source))
                        .unwrap()
                } else {
                    fixture.provider.clone()
                };
                let account = if caller_managed {
                    "default"
                } else {
                    "personal"
                };
                let result = provider
                    .set_playlist_subscription("21", subscribed, Some(account))
                    .await
                    .unwrap();
                assert_eq!(result.resource_ref.to_string(), "soda:21");
                assert_eq!(result.subscribed, subscribed);
                assert_eq!(result.extensions["source_user_id"], "123456");
                assert_eq!(
                    result.extensions["verified_by"],
                    "complete_saved_library_readback"
                );
                if caller_managed {
                    assert!(
                        provider
                            .take_response_credential()
                            .unwrap()
                            .unwrap()
                            .secret()
                            .contains("page-two")
                    );
                    assert_eq!(
                        fixture.stored("personal").unwrap().secret(),
                        source.serialize().unwrap()
                    );
                } else {
                    assert!(provider.take_response_credential().unwrap().is_none());
                    assert!(
                        fixture
                            .stored("personal")
                            .unwrap()
                            .secret()
                            .contains("page-two")
                    );
                }
                assert_eq!(
                    fixture.stored("other").unwrap().secret(),
                    other.serialize().unwrap()
                );
                let requests = server.await.unwrap();
                assert_eq!(requests.len(), 4);
                assert_eq!(
                    requests
                        .iter()
                        .filter(|request| request.starts_with("POST "))
                        .count(),
                    1
                );
                let path = if subscribed {
                    "/luna/pc/me/collection/playlist?"
                } else {
                    "/luna/pc/me/collection/playlist/delete?"
                };
                assert!(requests[1].starts_with(&format!("POST {path}")));
                let body: serde_json::Value =
                    serde_json::from_str(requests[1].split_once("\r\n\r\n").unwrap().1).unwrap();
                assert_eq!(body, json!({"playlist_ids":["21"]}));
                assert!(requests[2].starts_with("GET /luna/pc/me/collection/mixed?"));
                assert!(requests[3].contains("cursor=next&count=500"));
                for (request, cookie) in
                    requests
                        .iter()
                        .zip(["session-secret", "verified", "written", "page-one"])
                {
                    assert!(request.contains(&format!("cookie: sessionid_ss={cookie}\r\n")));
                    assert!(!request.contains("other-secret"));
                }
                let device = provider.client.login_device().unwrap();
                for request in &requests[1..] {
                    assert!(request.contains("iid=&"));
                    assert!(!request.contains("install_id="));
                    assert!(request.contains(&format!("device_id={}", device.device_id)));
                }
            }
        }
    }

    #[tokio::test]
    async fn playlist_collection_failure_never_retries_or_reports_unverified_success() {
        let good_write =
            crate::test_http::json(r#"{"status_code":0}"#, Some("sessionid_ss=written"));
        let good_first_page = crate::test_http::json(
            r#"{"mixed_collections":[{"item_type":"playlist","playlist":{"id":"21","title":"target"}}],"has_more":true,"next_cursor":"next","total_num":2}"#,
            Some("sessionid_ss=page-one"),
        );
        for (following, code, cookie) in [
            (vec![crate::test_http::json(r#"{}"#, Some("sessionid_ss=bad-write"))], ErrorCode::UpstreamError, "verified"),
            (vec![crate::test_http::json(r#"{"status_code":1000016}"#, Some("sessionid_ss=bad-write"))], ErrorCode::AuthenticationRequired, "verified"),
            (vec!["HTTP/1.1 429 Too Many Requests\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_owned()], ErrorCode::RateLimited, "verified"),
            (vec!["HTTP/1.1 302 Found\r\nLocation: https://example.invalid/write\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_owned()], ErrorCode::UpstreamError, "verified"),
            (vec![good_write.clone(), crate::test_http::json(r#"{"mixed_collections":[],"has_more":false,"total_num":0}"#, Some("sessionid_ss=readback"))], ErrorCode::UpstreamError, "readback"),
            (vec![good_write.clone(), crate::test_http::json(r#"{"mixed_collections":[]}"#, Some("sessionid_ss=bad-page"))], ErrorCode::UpstreamError, "written"),
            (vec![good_write.clone(), good_first_page.clone(), crate::test_http::json(r#"{"mixed_collections":[],"has_more":false,"total_num":2}"#, Some("sessionid_ss=bad-page"))], ErrorCode::UpstreamError, "page-one"),
            (vec![good_write, good_first_page, crate::test_http::json(r#"{"status_code":1000016}"#, None)], ErrorCode::AuthenticationRequired, "page-one"),
        ] {
            let mut fixture = SessionFixture::new();
            let source = test_soda_credential().bind_user("123456").unwrap();
            fixture.put("personal", &source);
            let mut responses = vec![account_reply("123456", Some("sessionid_ss=verified"))];
            responses.extend(following);
            let expected_requests = responses.len();
            let (origin, server) = crate::test_http::serve(responses).await;
            fixture.provider.client = fixture.provider.client.clone().with_auth_test_origin(origin);
            let caller = fixture.provider.caller_credential_scope(&caller_from(&source)).unwrap();
            let error = caller.set_playlist_subscription("21", true, None).await.unwrap_err();
            assert_eq!(error.code, code);
            assert!(!error.retryable);
            assert_eq!(error.details["write_outcome"], "unconfirmed");
            let rotated = caller.take_response_credential().unwrap();
            if code == ErrorCode::AuthenticationRequired {
                assert!(rotated.is_none());
            } else {
                assert_eq!(parse_soda_caller_credential(&rotated.unwrap()).unwrap().cookie_header().unwrap(), format!("sessionid_ss={cookie}"));
            }
            let credential = caller.selected_credential("default").unwrap().unwrap().0;
            assert_eq!(credential.cookie_header().unwrap(), format!("sessionid_ss={cookie}"));
            assert_eq!(fixture.stored("personal").unwrap().secret(), source.serialize().unwrap());
            let requests = server.await.unwrap();
            assert_eq!(requests.len(), expected_requests);
            assert_eq!(requests.iter().filter(|request| request.starts_with("POST ")).count(), 1);
        }
    }

    #[tokio::test]
    async fn playlist_collection_rejects_invalid_sources_and_identity_before_writing() {
        let mut fixture = SessionFixture::new();
        for id in ["", "0", "01", "-1", "1/2", "12?x=1", " 12", &"1".repeat(65)] {
            assert_eq!(
                fixture
                    .provider
                    .set_playlist_subscription(id, true, None)
                    .await
                    .unwrap_err()
                    .code,
                ErrorCode::InvalidRequest
            );
        }
        assert_eq!(
            fixture
                .provider
                .set_playlist_subscription("21", true, Some("missing"))
                .await
                .unwrap_err()
                .code,
            ErrorCode::AuthenticationRequired
        );
        let source = test_soda_credential().bind_user("123456").unwrap();
        fixture.put("personal", &source);
        let caller = fixture
            .provider
            .caller_credential_scope(&caller_from(&source))
            .unwrap();
        assert_eq!(
            caller
                .set_playlist_subscription("21", true, Some("personal"))
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
        let (origin, server) = crate::test_http::serve(vec![account_reply(
            "654321",
            Some("sessionid_ss=wrong-account"),
        )])
        .await;
        fixture.provider.client = fixture
            .provider
            .client
            .clone()
            .with_auth_test_origin(origin);
        assert_eq!(
            fixture
                .provider
                .set_playlist_subscription("21", true, Some("personal"))
                .await
                .unwrap_err()
                .code,
            ErrorCode::UpstreamError
        );
        assert_eq!(server.await.unwrap().len(), 1);
        assert_eq!(
            fixture.stored("personal").unwrap().secret(),
            source.serialize().unwrap()
        );
    }

    #[tokio::test]
    async fn playlist_collection_late_write_or_readback_cannot_revive_a_replaced_session() {
        for pause_at in [1, 2] {
            for replace in [false, true] {
                let mut fixture = SessionFixture::new();
                let source = test_soda_credential().bind_user("123456").unwrap();
                fixture.put("personal", &source);
                let mut responses = vec![
                    account_reply("123456", None),
                    crate::test_http::json(r#"{"status_code":0}"#, None),
                ];
                if pause_at == 2 {
                    responses.push(crate::test_http::json(r#"{"mixed_collections":[{"item_type":"playlist","playlist":{"id":"21","title":"target"}}],"has_more":false,"total_num":1}"#, None));
                }
                let paused = crate::test_http::serve_paused_at(responses, pause_at).await;
                fixture.provider.client = fixture
                    .provider
                    .client
                    .clone()
                    .with_auth_test_origin(paused.origin);
                let task_provider = fixture.provider.clone();
                let pending = tokio::spawn(async move {
                    task_provider
                        .set_playlist_subscription("21", true, Some("personal"))
                        .await
                });
                paused.arrived.await.unwrap();
                let replacement = SodaCredential::test_credential("new-login")
                    .bind_user("123456")
                    .unwrap();
                if replace {
                    fixture.put("personal", &replacement);
                } else {
                    fixture.store.remove(Platform::Soda, "personal").unwrap();
                }
                paused.release.send(()).unwrap();
                let error = pending.await.unwrap().unwrap_err();
                assert_eq!(error.code, ErrorCode::Conflict);
                assert_eq!(error.details["write_outcome"], "unconfirmed");
                if replace {
                    assert_eq!(
                        fixture.stored("personal").unwrap().secret(),
                        replacement.serialize().unwrap()
                    );
                } else {
                    assert!(fixture.stored("personal").is_none());
                }
                assert_eq!(paused.requests.await.unwrap().len(), pause_at + 1);
            }
        }
    }

    #[tokio::test]
    async fn library_failure_does_not_accept_bad_cookies_or_discard_an_earlier_update() {
        for (reply, code) in [
            (
                crate::test_http::json(r#"{"status_code":99}"#, Some("sessionid_ss=invalid-page")),
                ErrorCode::UpstreamError,
            ),
            (
                crate::test_http::json(
                    r#"{"mixed_collections":[],"total_num":0}"#,
                    Some("sessionid_ss=; Max-Age=0"),
                ),
                ErrorCode::AuthenticationRequired,
            ),
            (
                crate::test_http::json(
                    r#"{"mixed_collections":[],"total_num":1}"#,
                    Some("sessionid_ss=invalid-page"),
                ),
                ErrorCode::UpstreamError,
            ),
        ] {
            let mut fixture = SessionFixture::new();
            let source = test_soda_credential().bind_user("123456").unwrap();
            fixture.put("personal", &source);
            let (origin, server) = crate::test_http::serve(vec![
                account_reply("123456", None),
                crate::test_http::json(
                    r#"{"playlists":[],"total_num":0}"#,
                    Some("sessionid_ss=accepted-update"),
                ),
                reply,
            ])
            .await;
            fixture.provider.client = fixture
                .provider
                .client
                .clone()
                .with_auth_test_origin(origin);
            let caller = fixture
                .provider
                .caller_credential_scope(&caller_from(&source))
                .unwrap();
            assert_eq!(
                caller
                    .account_playlists(&PageRequest::new(30, 0))
                    .await
                    .unwrap_err()
                    .code,
                code
            );
            let update = caller.take_response_credential().unwrap();
            if code == ErrorCode::AuthenticationRequired {
                assert!(update.is_none());
            } else {
                let update = update.unwrap();
                assert!(update.secret().contains("accepted-update"));
                assert!(!update.secret().contains("invalid-page"));
            }
            assert!(
                caller
                    .selected_credential("default")
                    .unwrap()
                    .unwrap()
                    .0
                    .cookie_header()
                    .unwrap()
                    .contains("accepted-update")
            );
            assert_eq!(
                fixture.stored("personal").unwrap().secret(),
                source.serialize().unwrap()
            );
            assert_eq!(server.await.unwrap().len(), 3);
        }
    }

    #[tokio::test]
    async fn library_rejects_delayed_identity_reply_after_logout_or_replacement() {
        for replace in [false, true] {
            let mut fixture = SessionFixture::new();
            let source = test_soda_credential().bind_user("123456").unwrap();
            fixture.put("personal", &source);
            let server = crate::test_http::serve_paused(account_reply("123456", None)).await;
            fixture.provider.client = fixture
                .provider
                .client
                .clone()
                .with_auth_test_origin(server.origin);
            let provider = fixture.provider.clone();
            let read = tokio::spawn(async move {
                provider
                    .account_playlists(&PageRequest {
                        limit: 30,
                        offset: 0,
                        account: Some("personal".to_owned()),
                    })
                    .await
            });
            server.arrived.await.unwrap();
            if replace {
                fixture.put(
                    "personal",
                    &SodaCredential::test_credential("new-login")
                        .bind_user("123456")
                        .unwrap(),
                );
            } else {
                fixture.provider.logout("personal").await.unwrap();
            }
            server.release.send(()).unwrap();
            assert_eq!(read.await.unwrap().unwrap_err().code, ErrorCode::Conflict);
            assert_eq!(server.requests.await.unwrap().len(), 1);
            if replace {
                assert!(
                    fixture
                        .stored("personal")
                        .unwrap()
                        .secret()
                        .contains("new-login")
                );
            } else {
                assert!(fixture.stored("personal").is_none());
            }
        }
    }

    #[tokio::test]
    async fn credential_import_verifies_identity_and_respects_all_three_ownership_modes() {
        let mut fixture = SessionFixture::new();
        let (origin, server) = crate::test_http::serve(vec![
            account_reply("123456", None),
            account_reply("123456", Some("sessionid_ss=rotated-import")),
            account_reply("654321", None),
        ])
        .await;
        fixture.provider.client = fixture
            .provider
            .client
            .clone()
            .with_auth_test_origin(origin);
        let request = |account: &str| CredentialImportRequest {
            account: account.to_owned(),
            credential: ImportedCredential::Cookie {
                value: "sessionid_ss=import-secret".to_owned(),
            },
        };
        let server_only = fixture
            .provider
            .import_credential(&request("personal"), CredentialMode::Server)
            .await
            .unwrap();
        assert_eq!(server_only.profile.user_id.as_deref(), Some("123456"));
        assert!(server_only.credential.is_none());
        let original = fixture.stored("personal").unwrap();
        let both = fixture
            .provider
            .import_credential(&request("shared"), CredentialMode::Both)
            .await
            .unwrap();
        let shared = fixture.stored("shared").unwrap();
        assert_eq!(shared.secret(), both.credential.as_ref().unwrap().secret());
        assert!(shared.secret().contains("rotated-import"));
        let client = fixture
            .provider
            .import_credential(&request("default"), CredentialMode::Client)
            .await
            .unwrap();
        assert_eq!(client.profile.user_id.as_deref(), Some("654321"));
        assert!(client.credential.is_some());
        assert!(fixture.stored("default").is_none());
        assert_eq!(fixture.stored("personal").unwrap(), original);
        assert_eq!(
            fixture.store.load_platform(Platform::Soda).unwrap().len(),
            2
        );
        let requests = server.await.unwrap();
        assert_eq!(requests.len(), 3);
        assert!(requests.iter().all(|request| {
            request.starts_with("GET /luna/pc/me?aid=386088&app_name=luna_pc")
                && request.contains("sessionid_ss=import-secret")
        }));
    }

    #[tokio::test]
    async fn failed_import_keeps_the_existing_account_and_hides_cookie_values() {
        let mut fixture = SessionFixture::new();
        fixture.put(
            "personal",
            &test_soda_credential().bind_user("123456").unwrap(),
        );
        let original = fixture.stored("personal").unwrap();
        let (origin, server) = crate::test_http::serve(vec![
            crate::test_http::json(r#"{"status_code":1000016}"#, Some("sessionid_ss=poison")),
            crate::test_http::json(r#"{"status_code":0,"my_info":{}}"#, None),
        ])
        .await;
        fixture.provider.client = fixture
            .provider
            .client
            .clone()
            .with_auth_test_origin(origin);
        for value in [
            "sessionid_ss=private; sessionid_ss=other",
            "sessionid_ss=private",
            "sessionid_ss=private",
        ] {
            let error = fixture
                .provider
                .import_credential(
                    &CredentialImportRequest {
                        account: "personal".to_owned(),
                        credential: ImportedCredential::Cookie {
                            value: value.to_owned(),
                        },
                    },
                    CredentialMode::Both,
                )
                .await
                .unwrap_err();
            assert!(!error.to_string().contains("private"));
            assert!(!error.to_string().contains("poison"));
            assert_eq!(fixture.stored("personal").unwrap(), original);
        }
        assert_eq!(server.await.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn explicit_refresh_syncs_the_same_login_and_never_falls_back_from_client_to_server() {
        let mut fixture = SessionFixture::new();
        let source = test_soda_credential().bind_user("123456").unwrap();
        fixture.put("personal", &source);
        let (origin, server) = crate::test_http::serve(vec![
            account_reply("123456", Some("sessionid_ss=server-rotated")),
            account_reply("123456", Some("sessionid_ss=client-rotated")),
            account_reply("123456", None),
        ])
        .await;
        fixture.provider.client = fixture
            .provider
            .client
            .clone()
            .with_auth_test_origin(origin);
        let both = fixture
            .provider
            .refresh_session_with_ownership("personal", None, CredentialMode::Both)
            .await
            .unwrap();
        let stored = fixture.stored("personal").unwrap();
        assert_eq!(stored.secret(), both.credential.as_ref().unwrap().secret());
        assert_eq!(
            both.profile.extensions["refresh_method"],
            "account_revalidation"
        );
        assert_eq!(both.profile.extensions["refreshed"], true);
        let client = fixture
            .provider
            .refresh_session_with_ownership(
                "default",
                both.credential.as_ref(),
                CredentialMode::Client,
            )
            .await
            .unwrap();
        assert_eq!(fixture.stored("personal").unwrap(), stored);
        assert!(
            client
                .credential
                .as_ref()
                .unwrap()
                .secret()
                .contains("client-rotated")
        );
        let synced = fixture
            .provider
            .refresh_session_with_ownership(
                "personal",
                client.credential.as_ref(),
                CredentialMode::Both,
            )
            .await
            .unwrap();
        assert_eq!(synced.profile.extensions["refreshed"], false);
        assert_eq!(
            fixture.stored("personal").unwrap().secret(),
            synced.credential.as_ref().unwrap().secret()
        );
        assert!(
            fixture
                .provider
                .refresh_session_with_ownership("personal", None, CredentialMode::Client)
                .await
                .is_err()
        );
        assert!(
            fixture
                .provider
                .refresh_session_with_ownership(
                    "personal",
                    both.credential.as_ref(),
                    CredentialMode::Server
                )
                .await
                .is_err()
        );
        let unrelated = caller_from(&test_soda_credential().bind_user("123456").unwrap());
        assert_eq!(
            fixture
                .provider
                .refresh_session_with_ownership("personal", Some(&unrelated), CredentialMode::Both)
                .await
                .unwrap_err()
                .code,
            ErrorCode::Conflict
        );
        let requests = server.await.unwrap();
        assert!(requests[0].contains("sessionid_ss=session-secret"));
        assert!(requests[1].contains("sessionid_ss=server-rotated"));
        assert!(requests[2].contains("sessionid_ss=server-rotated"));
    }

    #[tokio::test]
    async fn invalidated_or_changed_identity_refresh_cannot_replace_a_session() {
        let mut fixture = SessionFixture::new();
        fixture.put(
            "personal",
            &test_soda_credential().bind_user("123456").unwrap(),
        );
        let original = fixture.stored("personal").unwrap();
        let (origin, server) = crate::test_http::serve(vec![
            crate::test_http::json(r#"{"status_code":1000016}"#, Some("sessionid_ss=poison")),
            account_reply("654321", Some("sessionid_ss=wrong-user")),
        ])
        .await;
        fixture.provider.client = fixture
            .provider
            .client
            .clone()
            .with_auth_test_origin(origin);
        for _ in 0..2 {
            assert!(
                fixture
                    .provider
                    .refresh_session_with_ownership("personal", None, CredentialMode::Both)
                    .await
                    .is_err()
            );
            assert_eq!(fixture.stored("personal").unwrap(), original);
        }
        assert_eq!(server.await.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn even_unchanged_refresh_replies_cannot_export_after_logout_or_relogin() {
        for relogin in [false, true] {
            let mut fixture = SessionFixture::new();
            fixture.put(
                "personal",
                &test_soda_credential().bind_user("123456").unwrap(),
            );
            let server = crate::test_http::serve_paused(account_reply("123456", None)).await;
            fixture.provider.client = fixture
                .provider
                .client
                .clone()
                .with_auth_test_origin(server.origin);
            let provider = fixture.provider.clone();
            let refresh = tokio::spawn(async move {
                provider
                    .refresh_session_with_ownership("personal", None, CredentialMode::Both)
                    .await
            });
            server.arrived.await.unwrap();
            if relogin {
                fixture.put(
                    "personal",
                    &test_soda_credential().bind_user("123456").unwrap(),
                );
            } else {
                assert!(fixture.provider.logout("personal").await.unwrap());
            }
            let after = fixture.stored("personal");
            server.release.send(()).unwrap();
            assert_eq!(
                refresh.await.unwrap().unwrap_err().code,
                ErrorCode::Conflict
            );
            assert_eq!(fixture.stored("personal"), after);
            assert_eq!(server.requests.await.unwrap().len(), 1);
        }
    }

    #[tokio::test]
    async fn local_logout_is_idempotent_and_only_removes_the_selected_login_generation() {
        let fixture = SessionFixture::new();
        let source = test_soda_credential().bind_user("123456").unwrap();
        let caller = caller_from(&source);
        fixture.put("personal", &source);
        fixture.put(
            "other",
            &test_soda_credential().bind_user("654321").unwrap(),
        );
        let other = fixture.stored("other").unwrap();
        let client = fixture
            .provider
            .logout_with_ownership("default", Some(&caller), CredentialMode::Client)
            .await
            .unwrap();
        assert!(!client.removed && client.caller_credential_discard_required);
        assert!(fixture.stored("personal").is_some());
        assert_eq!(
            fixture
                .provider
                .logout_with_ownership("other", Some(&caller), CredentialMode::Both)
                .await
                .unwrap_err()
                .code,
            ErrorCode::Conflict
        );
        let both = fixture
            .provider
            .logout_with_ownership("personal", Some(&caller), CredentialMode::Both)
            .await
            .unwrap();
        assert!(both.removed && both.caller_credential_discard_required);
        assert!(
            !fixture
                .provider
                .logout_with_ownership("personal", Some(&caller), CredentialMode::Both)
                .await
                .unwrap()
                .removed
        );
        fixture.put(
            "personal",
            &test_soda_credential().bind_user("123456").unwrap(),
        );
        assert_eq!(
            fixture
                .provider
                .logout_with_ownership("personal", Some(&caller), CredentialMode::Both)
                .await
                .unwrap_err()
                .code,
            ErrorCode::Conflict
        );
        assert_eq!(fixture.stored("other").unwrap(), other);
        assert!(fixture.provider.logout("personal").await.unwrap());
        assert!(!fixture.provider.logout("personal").await.unwrap());
    }

    #[test]
    fn provider_advertises_current_public_and_auth_capabilities() {
        let provider = SodaProvider::new(SodaConfig::default()).expect("Soda provider");
        assert_eq!(provider.platform(), Platform::Soda);
        assert_eq!(provider.name(), "Soda Music");
        assert_eq!(
            provider.capabilities(),
            BTreeSet::from([
                Capability::QrLogin,
                Capability::QrLoginVerification,
                Capability::CredentialImport,
                Capability::SessionManagement,
                Capability::SessionRevocation,
                Capability::CallerManagedCredentials,
                Capability::AccountProfile,
                Capability::UserProfileModern,
                Capability::UserMembership,
                Capability::UserMembershipClientInfo,
                Capability::AccountPlaylists,
                Capability::PlaylistWrite,
                Capability::PlaylistVisibilityWrite,
                Capability::AccountAlbums,
                Capability::AccountDigitalAlbums,
                Capability::AccountFollowingArtists,
                Capability::AlbumSubscriptionWrite,
                Capability::Favorites,
                Capability::TrackSubscriptionWrite,
                Capability::AlbumDetail,
                Capability::ArtistDetail,
                Capability::ArtistOverview,
                Capability::ArtistTracks,
                Capability::ArtistAlbums,
                Capability::ArtistSubscriptionWrite,
                Capability::AudioDownload,
                Capability::AudioStream,
                Capability::PlaylistRead,
                Capability::PlaylistSubscriptionWrite,
                Capability::SearchTracks,
                Capability::SearchAlbums,
                Capability::SearchPlaylists,
                Capability::SearchArtists,
                Capability::SearchSuggestions,
                Capability::TrackDetail,
                Capability::Lyrics,
                Capability::TrackAvailability,
            ])
        );
        assert!(provider.supports(Capability::TrackDetail));
        assert!(provider.supports(Capability::Lyrics));
        assert!(provider.supports(Capability::TrackAvailability));
        assert!(provider.supports(Capability::AudioStream));
        assert!(provider.supports(Capability::AudioDownload));
        assert!(provider.supports(Capability::PlaylistRead));
        assert!(provider.supports(Capability::AlbumDetail));
        assert!(provider.supports(Capability::ArtistSubscriptionWrite));
        assert!(provider.supports(Capability::AccountFollowingArtists));
        assert!(provider.supports(Capability::AccountDigitalAlbums));
    }

    #[tokio::test]
    async fn authentication_ownership_is_atomic_and_caller_credentials_are_strict() {
        let root = std::env::temp_dir().join(format!(
            "tuneweave-soda-provider-credential-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ));
        let store = Arc::new(tuneweave_core::FileAccountCredentialStore::new(&root));
        let mut provider = SodaProvider::new(SodaConfig {
            credential_store: Some(store.clone()),
            ..SodaConfig::default()
        })
        .expect("Soda provider");
        let (origin, server) = crate::test_http::serve(vec![crate::test_http::json(
            r#"{"status_code":0,"my_info":{"id":"123456","nickname":"listener"}}"#,
            None,
        )])
        .await;
        provider.client = provider.client.with_auth_test_origin(origin);
        let credential = test_soda_credential();

        let result = provider
            .finish_authentication("personal", &credential, CredentialMode::Both, None)
            .await
            .expect("persist and return credential");
        assert!(result.profile.authenticated);
        assert_eq!(result.profile.account, "personal");
        assert_eq!(result.profile.user_id.as_deref(), Some("123456"));
        let caller = result.credential.expect("caller credential");
        assert_eq!(caller.platform, Platform::Soda);
        assert_eq!(caller.kind, SODA_CREDENTIAL_KIND);
        assert!(caller.expires_at.is_none());
        assert_eq!(
            store
                .load_platform(Platform::Soda)
                .expect("load Soda credentials")
                .len(),
            1
        );
        let scoped = provider
            .caller_credential_scope(&caller)
            .expect("caller credential scope");
        assert!(scoped.caller_credential.is_some());
        assert!(scoped.credential_store.is_none());

        let wrong_platform =
            ProviderCredential::new(Platform::Qq, SODA_CREDENTIAL_KIND, caller.secret(), None)
                .expect("foreign credential");
        assert!(provider.caller_credential_scope(&wrong_platform).is_err());
        assert!(
            provider
                .finish_authentication("named", &credential, CredentialMode::Client, None)
                .await
                .is_err()
        );
        assert_eq!(
            store.load_platform(Platform::Soda).unwrap()[0].secret(),
            caller.secret()
        );
        let requests = server.await.unwrap();
        assert!(requests[0].starts_with("GET /luna/pc/me?aid=386088&app_name=luna_pc"));
        assert!(requests[0].contains("sessionid_ss=session-secret"));
        std::fs::remove_dir_all(root).expect("remove credential directory");
    }

    #[tokio::test]
    async fn invalid_session_never_issues_or_persists_a_credential() {
        let (origin, server) = crate::test_http::serve(vec![crate::test_http::json(
            r#"{"status_code":1000016}"#,
            None,
        )])
        .await;
        let provider =
            SodaProvider::from_client(SodaClient::test_client().with_auth_test_origin(origin));
        let error = provider
            .finish_authentication(
                "default",
                &test_soda_credential(),
                CredentialMode::Client,
                None,
            )
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::AuthenticationRequired);
        assert!(provider.take_response_credential().unwrap().is_none());
        server.await.unwrap();
    }

    #[tokio::test]
    async fn expired_qr_does_not_persist_a_successful_account_lookup() {
        let root = std::env::temp_dir().join(format!("soda-expired-qr-{}", rand::random::<u64>()));
        let store = Arc::new(tuneweave_core::FileAccountCredentialStore::new(&root));
        let mut provider = SodaProvider::new(SodaConfig {
            credential_store: Some(store.clone()),
            ..SodaConfig::default()
        })
        .unwrap();
        let (origin, server) = crate::test_http::serve(vec![crate::test_http::json(
            r#"{"status_code":0,"my_info":{"id":"123456","nickname":"listener"}}"#,
            Some("sessionid_ss=late-session"),
        )])
        .await;
        provider.client = provider.client.with_auth_test_origin(origin);
        let result = provider
            .finish_authentication(
                "personal",
                &test_soda_credential(),
                CredentialMode::Both,
                Some(std::time::Instant::now()),
            )
            .await;
        assert_eq!(result.unwrap_err().code, ErrorCode::InvalidRequest);
        assert!(store.load_platform(Platform::Soda).unwrap().is_empty());
        assert!(provider.take_response_credential().unwrap().is_none());
        server.abort();
        assert!(server.await.unwrap_err().is_cancelled());
        if root.exists() {
            std::fs::remove_dir_all(root).unwrap();
        }
    }

    #[tokio::test]
    async fn concurrent_qr_confirmation_is_verified_and_consumed_once() {
        let create = crate::test_http::json(
            r#"{"message":"success","data":{"token":"01234567890123456789012345678901234","qrcode":"iVBORw0KGgo="}}"#,
            None,
        );
        let poll = crate::test_http::json(
            r#"{"data":{"status":"confirmed","error_code":0}}"#,
            Some("sessionid_ss=qr-session; Path=/; HttpOnly"),
        );
        let account = crate::test_http::json(
            r#"{"status_code":0,"my_info":{"id":"123456","nickname":"listener"}}"#,
            None,
        );
        let (origin, server) = crate::test_http::serve(vec![create, poll, account]).await;
        let provider =
            SodaProvider::from_client(SodaClient::test_client().with_auth_test_origin(origin));
        let start = provider
            .start_qr_login_with_mode(None, CredentialMode::Client)
            .await
            .unwrap();
        let (first, second) = tokio::join!(
            provider.poll_qr_login_with_mode(
                &start.provider_transaction_id,
                "default",
                CredentialMode::Client
            ),
            provider.poll_qr_login_with_mode(
                &start.provider_transaction_id,
                "default",
                CredentialMode::Client
            ),
        );
        assert_eq!(usize::from(first.is_ok()) + usize::from(second.is_ok()), 1);
        let confirmed = first.or(second).unwrap();
        assert_eq!(confirmed.state, AuthState::Confirmed);
        assert_eq!(
            confirmed.profile.unwrap().user_id.as_deref(),
            Some("123456")
        );
        assert!(confirmed.credential.is_some());
        assert!(
            provider
                .poll_qr_login_with_mode(
                    &start.provider_transaction_id,
                    "default",
                    CredentialMode::Client
                )
                .await
                .is_err()
        );
        let requests = server.await.unwrap();
        assert_eq!(requests.len(), 3);
        assert!(requests[1].starts_with("POST /passport/web/check_qrconnect/"));
        assert!(!requests[1].contains("bd-ticket-guard"));
    }

    #[tokio::test]
    async fn failed_qr_responses_cannot_authenticate_using_response_cookies() {
        let create = crate::test_http::json(
            r#"{"message":"success","data":{"token":"01234567890123456789012345678901234","qrcode":"iVBORw0KGgo="}}"#,
            None,
        );
        let failure = crate::test_http::json(
            r#"{"data":{"status":"failed","error_code":6}}"#,
            Some("sessionid_ss=failed-session"),
        );
        let (origin, server) = crate::test_http::serve(vec![create, failure]).await;
        let provider =
            SodaProvider::from_client(SodaClient::test_client().with_auth_test_origin(origin));
        let start = provider
            .start_qr_login_with_mode(None, CredentialMode::Client)
            .await
            .unwrap();
        let failed = provider
            .poll_qr_login_with_mode(
                &start.provider_transaction_id,
                "default",
                CredentialMode::Client,
            )
            .await
            .unwrap();
        assert_eq!(failed.state, AuthState::Failed);
        assert!(failed.profile.is_none());
        assert!(failed.credential.is_none());
        server.await.unwrap();
    }

    #[tokio::test]
    async fn qr_sms_verification_keeps_the_original_transaction_and_requires_identity_confirmation()
    {
        use tuneweave_core::QrVerificationAction;
        let create = crate::test_http::json(
            r#"{"message":"success","data":{"token":"01234567890123456789012345678901234","qrcode":"iVBORw0KGgo="}}"#,
            None,
        );
        let mfa = String::from_utf8(crate::mfa::tests::sms_fixture()).unwrap();
        let (origin, server) = crate::test_http::serve(vec![
            create,
            crate::test_http::json(&mfa, Some("passport_mfa_token=temporary-mfa-secret")),
            crate::test_http::json(r#"{"message":"success","data":{"retry_time":60}}"#, None),
            crate::test_http::json(
                r#"{"message":"success","data":{"ticket":"ticket-secret"}}"#,
                None,
            ),
            crate::test_http::json(
                r#"{"data":{"status":"confirmed","error_code":0}}"#,
                Some("sessionid_ss=confirmed-session"),
            ),
            crate::test_http::json(
                r#"{"status_code":0,"my_info":{"id":"10001234","nickname":"listener"}}"#,
                None,
            ),
        ])
        .await;
        let provider =
            SodaProvider::from_client(SodaClient::test_client().with_auth_test_origin(origin));
        let start = provider
            .start_qr_login_with_mode(None, CredentialMode::Client)
            .await
            .unwrap();
        let id = &start.provider_transaction_id;
        assert!(
            provider
                .poll_qr_login_with_mode(id, "default", CredentialMode::Server)
                .await
                .is_err()
        );
        let poll = provider
            .poll_qr_login_with_mode(id, "default", CredentialMode::Client)
            .await
            .unwrap();
        assert_eq!(poll.state, AuthState::VerificationRequired);
        let public = serde_json::to_string(&poll).unwrap();
        for secret in [
            "encrypted-user-secret",
            "verify-token-secret",
            "temporary-mfa-secret",
            "13800138000",
        ] {
            assert!(!public.contains(secret));
        }
        let sent = provider
            .verify_qr_login(
                id,
                "default",
                CredentialMode::Client,
                &QrVerificationAction::SendSms,
            )
            .await
            .unwrap();
        assert_eq!(sent.state, AuthState::VerificationRequired);
        assert!(sent.verification.unwrap().resend_after_secs.unwrap() >= 59);
        assert_eq!(
            provider
                .verify_qr_login(
                    id,
                    "default",
                    CredentialMode::Client,
                    &QrVerificationAction::SendSms
                )
                .await
                .unwrap_err()
                .code,
            ErrorCode::RateLimited
        );
        let submit = QrVerificationAction::SubmitSms {
            code: "864209".to_owned(),
        };
        assert!(
            provider
                .verify_qr_login(id, "other-account", CredentialMode::Client, &submit)
                .await
                .is_err()
        );
        let verified = provider
            .verify_qr_login(id, "default", CredentialMode::Client, &submit)
            .await
            .unwrap();
        assert_eq!(verified.state, AuthState::Scanned);
        assert!(verified.profile.is_none());
        assert!(verified.credential.is_none());
        let confirmed = provider
            .poll_qr_login_with_mode(id, "default", CredentialMode::Client)
            .await
            .unwrap();
        assert_eq!(confirmed.state, AuthState::Confirmed);
        assert_eq!(
            confirmed.profile.unwrap().user_id.as_deref(),
            Some("10001234")
        );
        let secret = confirmed.credential.unwrap().into_secret();
        assert!(!secret.contains("temporary-mfa-secret"));
        assert!(!secret.contains("verify-token-secret"));
        let requests = server.await.unwrap();
        assert_eq!(requests.len(), 6);
        assert!(requests[2].starts_with("POST /passport/web/send_code/"));
        assert!(requests[3].starts_with("POST /passport/web/validate_code/"));
        assert!(requests[3].contains("code=383634323039"));
        assert!(requests[4].contains("std_verify_flow_id=flow-secret"));
        assert!(requests[4].contains("std_verify_token=verify-token-secret"));
        assert!(requests[5].starts_with("GET /luna/pc/me?aid=386088"));
    }

    #[tokio::test]
    async fn user_profile_maps_the_authenticated_identity_and_rejects_other_users() {
        let response = crate::test_http::json(
            r#"{"status_code":0,"my_info":{"id":"123456","nickname":"listener"}}"#,
            None,
        );
        let (origin, server) = crate::test_http::serve(vec![response.clone(), response]).await;
        let provider =
            SodaProvider::from_client(SodaClient::test_client().with_auth_test_origin(origin));
        let credential = ProviderCredential::new(
            Platform::Soda,
            SODA_CREDENTIAL_KIND,
            test_soda_credential().serialize().unwrap(),
            None,
        )
        .unwrap();
        let caller = provider.caller_credential_scope(&credential).unwrap();
        let profile = caller
            .user_profile("123456", UserProfileBackend::Modern, None)
            .await
            .unwrap();
        assert_eq!(profile.user.name, "listener");
        assert_eq!(profile.user.resource_ref.platform(), Platform::Soda);
        assert_eq!(profile.user.id, "123456");
        assert!(profile.level.is_none());
        assert_eq!(
            caller
                .user_profile("654321", UserProfileBackend::Modern, None)
                .await
                .unwrap_err()
                .code,
            ErrorCode::PermissionDenied
        );
        server.await.unwrap();
    }

    #[tokio::test]
    async fn caller_profile_returns_rotated_credentials_without_server_fallback() {
        let (origin, server) = crate::test_http::serve(vec![crate::test_http::json(
            r#"{"status_code":0,"my_info":{"id":"123456"}}"#,
            Some("sessionid_ss=rotated-session; Path=/; Secure; HttpOnly"),
        )])
        .await;
        let provider =
            SodaProvider::from_client(SodaClient::test_client().with_auth_test_origin(origin));
        let credential = ProviderCredential::new(
            Platform::Soda,
            SODA_CREDENTIAL_KIND,
            test_soda_credential().serialize().unwrap(),
            None,
        )
        .unwrap();
        let caller = provider.caller_credential_scope(&credential).unwrap();
        assert!(caller.session_profile("personal").await.is_err());
        let profile = caller.session_profile("default").await.unwrap();
        assert_eq!(profile.user_id.as_deref(), Some("123456"));
        let refreshed = caller.take_response_credential().unwrap().unwrap();
        assert!(refreshed.secret().contains("rotated-session"));
        assert!(
            !serde_json::to_string(&profile)
                .unwrap()
                .contains("rotated-session")
        );
        assert!(provider.take_response_credential().unwrap().is_none());
        assert!(caller.take_response_credential().unwrap().is_none());
        server.await.unwrap();
    }

    #[test]
    fn delayed_account_reads_cannot_replace_relogin_or_resurrect_logout() {
        let root = std::env::temp_dir().join(format!("soda-generation-{}", rand::random::<u64>()));
        let store = Arc::new(tuneweave_core::FileAccountCredentialStore::new(&root));
        let provider = SodaProvider::new(SodaConfig {
            credential_store: Some(store.clone()),
            ..SodaConfig::default()
        })
        .unwrap();
        let source = test_soda_credential().bind_user("123456").unwrap();
        let stored = StoredAccountCredential::new(
            Platform::Soda,
            "personal",
            SODA_CREDENTIAL_KIND,
            source.serialize().unwrap(),
        )
        .unwrap();
        store.put(&stored).unwrap();
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            reqwest::header::SET_COOKIE,
            "sessionid_ss=rotated-session".parse().unwrap(),
        );
        let refreshed = source.with_response_cookies(&headers).unwrap();
        let relogin = StoredAccountCredential::new(
            Platform::Soda,
            "personal",
            SODA_CREDENTIAL_KIND,
            test_soda_credential()
                .bind_user("123456")
                .unwrap()
                .serialize()
                .unwrap(),
        )
        .unwrap();
        assert_ne!(
            relogin.secret(),
            stored.secret(),
            "fresh login changes generation even with identical cookies"
        );
        store.put(&relogin).unwrap();
        assert_eq!(
            provider
                .accept_account_read(&source, Some(&stored), &refreshed)
                .unwrap_err()
                .code,
            ErrorCode::Conflict
        );
        assert_eq!(store.load_platform(Platform::Soda).unwrap(), vec![relogin]);
        store.remove(Platform::Soda, "personal").unwrap();
        assert_eq!(
            provider
                .accept_account_read(&source, Some(&stored), &refreshed)
                .unwrap_err()
                .code,
            ErrorCode::Conflict
        );
        assert!(store.load_platform(Platform::Soda).unwrap().is_empty());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn public_search_rejects_accounts_foreign_options_and_unbounded_inputs() {
        assert!(validate_search_query(&SearchQuery::tracks("落了白", 20, 0)).is_ok());

        let mut query = SearchQuery::tracks("落了白", 20, 0);
        query.account = Some("default".to_owned());
        assert!(validate_search_query(&query).is_err());

        let mut query = SearchQuery::tracks("落了白", 20, 0);
        query.variant = SearchVariant::Legacy;
        assert!(validate_search_query(&query).is_err());

        let mut query = SearchQuery::tracks("落了白", 20, 0);
        query.highlight = true;
        assert!(validate_search_query(&query).is_err());

        assert!(validate_search_query(&SearchQuery::tracks("", 20, 0)).is_err());
        assert!(validate_search_query(&SearchQuery::tracks("落了白", 101, 0)).is_err());
        assert!(validate_search_query(&SearchQuery::tracks("query", 20, u32::MAX)).is_err());
    }

    fn catalogue_fixture(kind: SearchKind, start: u32, total: u32) -> String {
        let end = start.saturating_add(20).min(total);
        let data: Vec<_> = (start..end).map(|index| {
            let id = (1000 + index).to_string();
            let entity = match kind {
                SearchKind::Album => json!({"album":{"id":id,"name":format!("Album {index}"),"count_tracks":5}}),
                SearchKind::Playlist => json!({"playlist":{"id":id,"title":format!("Playlist {index}"),"count_tracks":5}}),
                SearchKind::Artist => json!({"artist":{"id":id,"name":format!("Artist {index}"),"count_tracks":5}}),
                _ => panic!("unexpected fixture kind"),
            };
            json!({"entity":entity})
        }).collect();
        crate::test_http::json(&json!({"status_info":{},"result_groups":[{"id":match kind { SearchKind::Album => "albums", SearchKind::Playlist => "playlists", SearchKind::Artist => "artists", _ => panic!("unexpected fixture kind") },"data":data,"has_more":end<total,"next_cursor":end.to_string()}]}).to_string(), None)
    }

    #[tokio::test]
    async fn catalogue_search_paginates_physical_pages_without_losing_types_or_offsets() {
        for kind in [SearchKind::Album, SearchKind::Playlist, SearchKind::Artist] {
            for (offset, limit, starts, expected_count, next_offset) in [
                (17, 25, vec![0, 20, 40], 25, Some(42)),
                (42, 25, vec![40], 13, None),
                (0, 5, vec![0], 5, Some(5)),
                (50, 100, vec![40], 5, None),
                (60, 30, vec![60], 0, None),
            ] {
                let (origin, server) = crate::test_http::serve(
                    starts
                        .iter()
                        .map(|start| catalogue_fixture(kind, *start, 55))
                        .collect(),
                )
                .await;
                let client = SodaClient::test_client().with_auth_test_origin(origin);
                let provider = SodaProvider::from_client(client);
                let mut query = SearchQuery::tracks("天空 & moon", limit, offset);
                query.kind = kind;
                let page = provider.search_catalog(&query).await.unwrap();
                assert_eq!(page.items.len(), expected_count);
                assert_eq!(page.pagination.offset, offset);
                assert_eq!(page.pagination.limit, limit);
                assert_eq!(page.pagination.next_offset, next_offset);
                assert_eq!(page.pagination.has_more, next_offset.is_some());
                assert!(page.pagination.total.is_none());
                assert_eq!(
                    page.pagination.extensions["upstream_pages_fetched"],
                    starts.len()
                );
                let backend = match kind {
                    SearchKind::Album => "official_android_album_search",
                    SearchKind::Playlist => "official_android_playlist_search",
                    SearchKind::Artist => "official_android_artist_search",
                    _ => panic!("unexpected kind"),
                };
                assert_eq!(page.pagination.extensions["backend"], backend);
                for (index, item) in page.items.iter().enumerate() {
                    let expected = (1000 + offset + index as u32).to_string();
                    match (kind, item) {
                        (SearchKind::Album, SearchItem::Album(album)) => {
                            assert_eq!(album.id, expected)
                        }
                        (SearchKind::Playlist, SearchItem::Playlist(playlist)) => {
                            assert_eq!(playlist.id, expected)
                        }
                        (SearchKind::Artist, SearchItem::Artist(artist)) => {
                            assert_eq!(artist.id, expected)
                        }
                        _ => panic!("catalogue changed resource type"),
                    }
                }
                let requests = server.await.unwrap();
                assert_eq!(requests.len(), starts.len());
                for (request, start) in requests.iter().zip(starts) {
                    let path = request
                        .lines()
                        .next()
                        .unwrap()
                        .split_whitespace()
                        .nth(1)
                        .unwrap();
                    let url = url::Url::parse(&format!("http://test{path}")).unwrap();
                    assert_eq!(
                        url.path(),
                        match kind {
                            SearchKind::Album => "/luna/search/album",
                            SearchKind::Playlist => "/luna/search/playlist",
                            SearchKind::Artist => "/luna/search/artist",
                            _ => panic!("unexpected kind"),
                        }
                    );
                    let params: BTreeMap<_, _> = url.query_pairs().collect();
                    assert_eq!(params["q"], "天空 & moon");
                    assert_eq!(params["cursor"], start.to_string());
                    assert_eq!(params["count"], "20");
                    assert_eq!(params["device_platform"], "android");
                    assert!(!params.contains_key("iid"));
                    assert!(!request.to_ascii_lowercase().contains("cookie:"));
                }
            }
        }
    }

    #[tokio::test]
    async fn public_catalogue_rejects_unsupported_sources_before_network_requests() {
        let fixture = SessionFixture::new();
        let caller = fixture
            .provider
            .caller_credential_scope(&caller_from(&test_soda_credential()))
            .unwrap();
        for kind in [
            SearchKind::Track,
            SearchKind::Album,
            SearchKind::Playlist,
            SearchKind::Artist,
        ] {
            let mut query = SearchQuery::tracks("query", 20, 0);
            query.kind = kind;
            assert_eq!(
                caller.search_catalog(&query).await.unwrap_err().code,
                ErrorCode::AuthenticationRequired
            );
            query.account = Some("personal".to_owned());
            assert_eq!(
                fixture
                    .provider
                    .search_catalog(&query)
                    .await
                    .unwrap_err()
                    .code,
                ErrorCode::AuthenticationRequired
            );
            query.account = None;
            query.variant = SearchVariant::Cloud;
            assert_eq!(
                fixture
                    .provider
                    .search_catalog(&query)
                    .await
                    .unwrap_err()
                    .code,
                ErrorCode::InvalidRequest
            );
        }
        let mut query = SearchQuery::tracks("query", 20, 0);
        query.kind = SearchKind::User;
        assert_eq!(
            fixture
                .provider
                .search_catalog(&query)
                .await
                .unwrap_err()
                .code,
            ErrorCode::CapabilityNotSupported
        );
    }

    #[tokio::test]
    async fn catalogue_search_failure_on_later_page_is_not_returned_as_partial_success() {
        let (origin, server) = crate::test_http::serve(vec![catalogue_fixture(SearchKind::Album, 0, 55), crate::test_http::json(r#"{"status_info":{},"result_groups":[{"id":"albums","has_more":true,"next_cursor":"20","data":[{"entity":{"album":{"id":"1001","name":"repeated"}}}]}]}"#, None)]).await;
        let provider =
            SodaProvider::from_client(SodaClient::test_client().with_auth_test_origin(origin));
        let mut query = SearchQuery::tracks("query", 25, 0);
        query.kind = SearchKind::Album;
        assert_eq!(
            provider.search_catalog(&query).await.unwrap_err().code,
            ErrorCode::UpstreamError
        );
        assert_eq!(server.await.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn shared_search_pagination_enforces_budget_and_cursor_progress() {
        let query = SearchQuery::tracks("query", 100, 19);
        let mut calls = 0;
        let error = collect_search_pages(&query, "test", |cursor| {
            calls += 1;
            std::future::ready(Ok(SearchPhysicalPage {
                items: vec![cursor],
                next_cursor: Some(cursor + 1),
                has_more: true,
            }))
        })
        .await
        .unwrap_err();
        assert_eq!(error.code, ErrorCode::UpstreamError);
        assert_eq!(calls, 6);
        assert!(
            collect_search_pages(&query, "test", |cursor| std::future::ready(Ok(
                SearchPhysicalPage {
                    items: vec![cursor],
                    next_cursor: Some(cursor),
                    has_more: true
                }
            )))
            .await
            .is_err()
        );
        assert!(
            collect_search_pages(&query, "test", |cursor| std::future::ready(Ok(
                SearchPhysicalPage {
                    items: Vec::<u32>::new(),
                    next_cursor: Some(cursor + 20),
                    has_more: true
                }
            )))
            .await
            .is_err()
        );
    }

    #[tokio::test]
    #[ignore = "requires the official public Soda search service; no account is used"]
    async fn live_public_catalogue_search_returns_typed_cross_page_results() {
        let provider = SodaProvider::new(SodaConfig::default()).unwrap();
        for kind in [SearchKind::Album, SearchKind::Playlist, SearchKind::Artist] {
            let mut query = SearchQuery::tracks("周杰伦", 25, 17);
            query.kind = kind;
            let page = provider.search_catalog(&query).await.unwrap();
            assert!(!page.items.is_empty());
            assert!(page.items.len() <= 25);
            assert_eq!(page.pagination.offset, 17);
            assert!(
                page.pagination.extensions["upstream_pages_fetched"]
                    .as_u64()
                    .unwrap()
                    > 1
            );
            for item in page.items {
                match (kind, item) {
                    (SearchKind::Album, SearchItem::Album(album)) => {
                        assert_eq!(album.resource_ref.platform(), Platform::Soda);
                        assert!(!album.name.is_empty());
                        assert_eq!(album.extensions["backend"], "official_android_album_search");
                    }
                    (SearchKind::Playlist, SearchItem::Playlist(playlist)) => {
                        assert_eq!(playlist.resource_ref.platform(), Platform::Soda);
                        assert!(!playlist.name.is_empty());
                        assert_eq!(
                            playlist.extensions["backend"],
                            "official_android_playlist_search"
                        );
                    }
                    (SearchKind::Artist, SearchItem::Artist(artist)) => {
                        assert_eq!(artist.resource_ref.platform(), Platform::Soda);
                        assert!(!artist.name.is_empty());
                        assert_eq!(
                            artist.extensions["backend"],
                            "official_android_artist_search"
                        );
                    }
                    _ => panic!("public search changed the requested resource type"),
                }
            }
        }
    }

    #[tokio::test]
    async fn account_track_reads_require_available_credentials_before_network_access() {
        let provider = SodaProvider::new(SodaConfig::default()).expect("Soda provider");
        let error = provider
            .track("7304719759323564095", Some("default"))
            .await
            .expect_err("Soda detail requires the selected session");
        assert_eq!(
            error.code,
            tuneweave_core::ErrorCode::AuthenticationRequired
        );
        assert_eq!(error.platform, Some(Platform::Soda));

        let error = provider
            .lyrics("7304719759323564095", Some("default"))
            .await
            .expect_err("Soda lyrics require the selected session");
        assert_eq!(
            error.code,
            tuneweave_core::ErrorCode::AuthenticationRequired
        );

        let request = LyricsRequest {
            singing_annotations: true,
            ..LyricsRequest::default()
        };
        let error = provider
            .lyrics_with_options("7304719759323564095", &request)
            .await
            .expect_err("Soda lyrics must reject foreign lyric controls");
        assert_eq!(error.code, tuneweave_core::ErrorCode::InvalidRequest);

        let availability = TrackAvailabilityRequest {
            bitrate: 200_000,
            account: Some("default".to_owned()),
        };
        let error = provider
            .track_availability("7304719759323564095", &availability)
            .await
            .expect_err("Soda availability requires the selected session");
        assert_eq!(
            error.code,
            tuneweave_core::ErrorCode::AuthenticationRequired
        );
        let error = provider
            .track_availability("7304719759323564095", &TrackAvailabilityRequest::new(0))
            .await
            .expect_err("Soda availability must reject zero bitrate");
        assert_eq!(error.code, tuneweave_core::ErrorCode::InvalidRequest);
    }

    #[tokio::test]
    async fn artist_overview_preserves_partial_previews_and_rejects_unknown_completeness() {
        let reply = |count: Option<u64>| {
            let mut artist = json!({"id":"123","name":"Artist"});
            if let Some(count) = count {
                artist["count_tracks"] = json!(count);
            }
            let body = format!(
                "<script>window._ROUTER_DATA = {};</script>",
                json!({"loaderData":{"artist_page":{"artistInfo":artist,"trackList":[{"id":"456","name":"Featured","duration":1000,"artists":[{"id":"123","name":"Artist"}]}]}}})
            );
            format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
        };
        let (origin, server) = crate::test_http::serve(vec![
            reply(Some(9)),
            reply(Some(1)),
            reply(None),
            reply(None),
        ])
        .await;
        let provider =
            SodaProvider::from_client(SodaClient::test_client().with_auth_test_origin(origin));
        let partial = provider.artist_overview("123", None).await.unwrap();
        assert!(partial.has_more_tracks);
        assert_eq!(partial.featured_tracks.len(), 1);
        assert_eq!(partial.artist.track_count, Some(9));
        assert!(
            !provider
                .artist_overview("123", None)
                .await
                .unwrap()
                .has_more_tracks
        );
        assert!(
            provider
                .artist("123", None)
                .await
                .unwrap()
                .track_count
                .is_none()
        );
        assert_eq!(
            provider
                .artist_overview("123", None)
                .await
                .unwrap_err()
                .code,
            ErrorCode::UpstreamError
        );
        for request in server.await.unwrap() {
            assert!(request.starts_with("GET /share/artist?artist_id=123 "));
            assert!(!request.to_ascii_lowercase().contains("cookie:"));
        }
    }

    #[tokio::test]
    async fn artist_pages_reject_missing_or_conflicting_account_scopes_and_invalid_ids_without_network_access()
     {
        let provider = SodaProvider::new(SodaConfig::default()).unwrap();
        for id in ["", "0", "0123", "123?other=x", "123/456"] {
            assert_eq!(
                provider.artist(id, None).await.unwrap_err().code,
                ErrorCode::InvalidRequest
            );
        }
        assert_eq!(
            provider
                .artist("123", Some("personal"))
                .await
                .unwrap_err()
                .code,
            ErrorCode::AuthenticationRequired
        );
        let caller = provider
            .caller_credential_scope(&caller_from(&test_soda_credential()))
            .unwrap();
        assert_eq!(
            caller
                .artist_overview("123", Some("personal"))
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
    }

    #[tokio::test]
    #[ignore = "requires the official public Soda artist share service; no account is used"]
    async fn live_public_artist_overview_keeps_the_official_preview_distinct_from_the_catalogue() {
        let provider = SodaProvider::new(SodaConfig::default()).unwrap();
        let page = provider
            .artist_overview("6681166129722824706", None)
            .await
            .unwrap();
        assert_eq!(page.artist.id, "6681166129722824706");
        assert!(!page.artist.name.is_empty());
        assert!(!page.artist.description.is_empty());
        assert!(!page.featured_tracks.is_empty());
        assert_eq!(
            page.has_more_tracks,
            page.artist.track_count.unwrap() > page.featured_tracks.len() as u64
        );
    }

    #[tokio::test]
    async fn account_media_delivery_preserves_sources_rotations_and_full_or_preview_rights() {
        let id = "7304719759323564095";
        let audio = crate::media::plaintext::tests::AAC;
        for caller_managed in [false, true] {
            for preview in [false, true] {
                for operation in 0..3 {
                    let mut fixture = SessionFixture::new();
                    let source = test_soda_credential().bind_user("123456").unwrap();
                    fixture.put("personal", &source);
                    let other = SodaCredential::test_credential("other-secret")
                        .bind_user("654321")
                        .unwrap();
                    fixture.put("other", &other);
                    let mut body: serde_json::Value =
                        serde_json::from_slice(&crate::client::test_account_track_fixture(preview))
                            .unwrap();
                    if operation == 2 {
                        let mut model: serde_json::Value = serde_json::from_str(
                            body["track_player"]["video_model"].as_str().unwrap(),
                        )
                        .unwrap();
                        for variant in model["video_list"].as_array_mut().unwrap() {
                            variant["video_meta"]["size"] = json!(audio.len());
                            variant["encrypt_info"] = json!({"encrypt":false});
                        }
                        body["track_player"]["video_model"] = json!(model.to_string());
                    }
                    let mut replies = vec![
                        account_reply("123456", Some("sessionid_ss=verified")),
                        crate::test_http::json(
                            &body.to_string(),
                            Some("sessionid_ss=media-current"),
                        ),
                    ];
                    let mut replies = replies
                        .drain(..)
                        .map(String::into_bytes)
                        .collect::<Vec<_>>();
                    if operation == 2 {
                        replies.push(crate::media::plaintext::tests::reply(audio));
                    }
                    let (origin, server) = crate::test_http::serve_bytes(replies).await;
                    fixture.provider.client = fixture
                        .provider
                        .client
                        .clone()
                        .with_auth_test_origin(origin);
                    let provider = if caller_managed {
                        fixture
                            .provider
                            .caller_credential_scope(&caller_from(&source))
                            .unwrap()
                    } else {
                        fixture.provider.clone()
                    };
                    let request = StreamRequest {
                        account: (!caller_managed).then(|| "personal".to_owned()),
                        ..StreamRequest::default()
                    };
                    let track = Track::new(
                        tuneweave_core::ResourceRef::new(Platform::Soda, id).unwrap(),
                        "test",
                    );
                    let (url, headers) = match operation {
                        0 => {
                            let stream = provider.stream(&track, &request).await.unwrap();
                            assert!(!format!("{stream:?}").contains("twc1_"));
                            assert_eq!(stream.trial.is_some(), preview);
                            assert_eq!(
                                stream.duration_ms,
                                Some(if preview { 60001 } else { 180822 })
                            );
                            assert_eq!(stream.format.as_deref(), Some("m4a"));
                            (Some(stream.url), stream.headers)
                        }
                        1 => {
                            let download = provider.download(&track, &request).await.unwrap();
                            assert!(!format!("{download:?}").contains("twc1_"));
                            assert_eq!(download.available, !preview);
                            assert_eq!(download.extensions["backend"], "official_pc_track_v2");
                            assert_eq!(download.extensions["preview_url_withheld"], preview);
                            assert_eq!(download.url.is_none(), preview);
                            (download.url, download.headers)
                        }
                        _ => {
                            let content = provider.audio_content(&track, &request).await.unwrap();
                            assert_eq!(content.bytes, audio);
                            assert_eq!(content.content_type, "audio/mp4");
                            (None, BTreeMap::new())
                        }
                    };
                    if let Some(url) = url {
                        assert!(url.starts_with(&format!("/v1/tracks/soda:{id}/stream/content?")));
                        assert!(
                            !url.contains("twc1")
                                && !url.contains("cookie")
                                && !url.contains("secret")
                        );
                        assert_eq!(url.contains("account=personal"), !caller_managed);
                        if caller_managed {
                            assert!(!url.contains("account="));
                            assert_eq!(headers.len(), 1);
                            let credential =
                                CallerCredential::parse(&headers[CALLER_CREDENTIAL_HEADER])
                                    .unwrap();
                            assert_eq!(
                                parse_soda_caller_credential(&credential)
                                    .unwrap()
                                    .cookie_header()
                                    .unwrap(),
                                "sessionid_ss=media-current"
                            );
                        } else {
                            assert!(headers.is_empty());
                        }
                    } else {
                        assert!(headers.is_empty());
                    }
                    if caller_managed {
                        let rotated = provider.take_response_credential().unwrap().unwrap();
                        assert_eq!(
                            parse_soda_caller_credential(&rotated)
                                .unwrap()
                                .cookie_header()
                                .unwrap(),
                            "sessionid_ss=media-current"
                        );
                        assert_eq!(
                            fixture.stored("personal").unwrap().secret(),
                            source.serialize().unwrap()
                        );
                    } else {
                        assert!(provider.take_response_credential().unwrap().is_none());
                        assert!(
                            fixture
                                .stored("personal")
                                .unwrap()
                                .secret()
                                .contains("media-current")
                        );
                    }
                    assert_eq!(
                        fixture.stored("other").unwrap().secret(),
                        other.serialize().unwrap()
                    );
                    let requests = server.await.unwrap();
                    assert_eq!(requests.len(), if operation == 2 { 3 } else { 2 });
                    assert!(requests[0].starts_with("GET /luna/pc/me?"));
                    assert!(requests[0].contains("cookie: sessionid_ss=session-secret\r\n"));
                    assert!(requests[1].starts_with("POST /luna/pc/track_v2?"));
                    assert!(requests[1].contains("cookie: sessionid_ss=verified\r\n"));
                    if operation == 2 {
                        assert!(requests[2].starts_with("GET /media/"));
                        assert!(!requests[2].to_ascii_lowercase().contains("cookie:"));
                        assert!(
                            !requests[2].contains("twc1") && !requests[2].contains("sessionid")
                        );
                    }
                }
            }
        }
    }

    fn secondary_media_replies(preview: bool, operation: u8) -> Vec<Vec<u8>> {
        let (body, mut info) = crate::client::test_secondary_fixture(preview);
        let audio = crate::media::plaintext::tests::AAC;
        if operation == 2 {
            for variant in info["Result"]["Data"]["PlayInfoList"]
                .as_array_mut()
                .unwrap()
            {
                variant["Size"] = json!(audio.len());
            }
        }
        let mut replies = vec![
            account_reply("123456", Some("sessionid_ss=verified")),
            crate::test_http::json(&body.to_string(), Some("sessionid_ss=media-current")),
            crate::test_http::json(&info.to_string(), Some("sessionid_ss=info-must-not-rotate")),
        ];
        let mut replies = replies
            .drain(..)
            .map(String::into_bytes)
            .collect::<Vec<_>>();
        if operation == 2 {
            replies.push(crate::media::plaintext::tests::reply(audio));
        }
        replies
    }

    async fn secondary_media_result(
        provider: &SodaProvider,
        operation: u8,
        account: Option<&str>,
    ) -> Result<serde_json::Value> {
        let track = Track::new(
            tuneweave_core::ResourceRef::new(Platform::Soda, "7304719759323564095").unwrap(),
            "test",
        );
        let request = StreamRequest {
            account: account.map(str::to_owned),
            ..StreamRequest::default()
        };
        Ok(match operation {
            0 => serde_json::to_value(provider.stream(&track, &request).await?).unwrap(),
            1 => serde_json::to_value(provider.download(&track, &request).await?).unwrap(),
            2 => {
                let content = provider.audio_content(&track, &request).await?;
                json!({"bytes":content.bytes,"content_type":content.content_type})
            }
            _ => serde_json::to_value(
                provider
                    .track_availability(
                        &track.id,
                        &TrackAvailabilityRequest {
                            account: account.map(str::to_owned),
                            bitrate: 200000,
                        },
                    )
                    .await?,
            )
            .unwrap(),
        })
    }

    #[tokio::test]
    async fn secondary_account_media_uses_selected_credentials_and_no_cookies_on_player_info_or_cdn()
     {
        for caller_managed in [false, true] {
            for preview in [false, true] {
                for operation in 0..4 {
                    let mut fixture = SessionFixture::new();
                    let source = test_soda_credential().bind_user("123456").unwrap();
                    fixture.put("personal", &source);
                    let other = SodaCredential::test_credential("other-secret")
                        .bind_user("654321")
                        .unwrap();
                    fixture.put("other", &other);
                    let (origin, server) =
                        crate::test_http::serve_bytes(secondary_media_replies(preview, operation))
                            .await;
                    fixture.provider.client = fixture
                        .provider
                        .client
                        .clone()
                        .with_auth_test_origin(origin);
                    let provider = if caller_managed {
                        fixture
                            .provider
                            .caller_credential_scope(&caller_from(&source))
                            .unwrap()
                    } else {
                        fixture.provider.clone()
                    };
                    let result = secondary_media_result(
                        &provider,
                        operation,
                        if caller_managed {
                            None
                        } else {
                            Some("personal")
                        },
                    )
                    .await
                    .unwrap();
                    match operation {
                        0 => assert_eq!(result["trial"].is_object(), preview),
                        1 => {
                            assert_eq!(result["available"], !preview);
                            assert_eq!(result["url"].is_null(), preview);
                        }
                        2 => {
                            assert_eq!(result["bytes"], json!(crate::media::plaintext::tests::AAC));
                            assert_eq!(result["content_type"], "audio/mp4");
                        }
                        _ => {
                            assert_eq!(result["playable"], !preview);
                            assert_eq!(result["extensions"]["preview_available"], preview);
                            assert_eq!(result["extensions"]["backend"], "official_pc_track_v2");
                            assert_eq!(
                                result["extensions"]["media_expires_at_epoch_seconds"],
                                1785336862_u64 + 3600
                            );
                        }
                    }
                    if operation < 2 && result["url"].is_string() {
                        let url = result["url"].as_str().unwrap();
                        assert!(url.starts_with("/v1/tracks/soda:"));
                        assert_eq!(url.contains("account=personal"), !caller_managed);
                        if caller_managed {
                            let token = result["headers"][CALLER_CREDENTIAL_HEADER]
                                .as_str()
                                .unwrap();
                            let credential = CallerCredential::parse(token).unwrap();
                            assert_eq!(
                                parse_soda_caller_credential(&credential)
                                    .unwrap()
                                    .cookie_header()
                                    .unwrap(),
                                "sessionid_ss=media-current"
                            );
                        }
                    }
                    for secret in [
                        "player-secret",
                        "info-must-not-rotate",
                        "cdn-must-not-rotate",
                        "video_model",
                        "url_player_info",
                    ] {
                        assert!(!result.to_string().contains(secret));
                    }
                    if caller_managed {
                        let rotated = provider.take_response_credential().unwrap().unwrap();
                        assert_eq!(
                            parse_soda_caller_credential(&rotated)
                                .unwrap()
                                .cookie_header()
                                .unwrap(),
                            "sessionid_ss=media-current"
                        );
                        assert_eq!(
                            fixture.stored("personal").unwrap().secret(),
                            source.serialize().unwrap()
                        );
                    } else {
                        assert!(provider.take_response_credential().unwrap().is_none());
                        assert!(
                            fixture
                                .stored("personal")
                                .unwrap()
                                .secret()
                                .contains("media-current")
                        );
                    }
                    assert_eq!(
                        fixture.stored("other").unwrap().secret(),
                        other.serialize().unwrap()
                    );
                    let requests = server.await.unwrap();
                    assert_eq!(requests.len(), if operation == 2 { 4 } else { 3 });
                    assert!(requests[0].contains("cookie: sessionid_ss=session-secret\r\n"));
                    assert!(requests[1].contains("cookie: sessionid_ss=verified\r\n"));
                    assert!(
                        requests[2].starts_with("GET /?Action=GetPlayInfo&token=player-secret ")
                    );
                    for request in &requests[2..] {
                        assert!(
                            !request.to_ascii_lowercase().contains("cookie:")
                                && !request.contains("twc1_")
                        );
                    }
                }
            }
        }
    }

    #[tokio::test]
    async fn late_secondary_or_audio_responses_cannot_export_after_logout_or_relogin() {
        for caller_managed in [false, true] {
            for relogin in [false, true] {
                for (operation, pause_at) in [(0, 2), (1, 2), (2, 3), (3, 2)] {
                    if caller_managed && !relogin {
                        continue;
                    } // Caller scope replacement models caller-side logout/relogin ownership.
                    let mut fixture = SessionFixture::new();
                    let source = test_soda_credential().bind_user("123456").unwrap();
                    fixture.put("personal", &source);
                    let paused = crate::test_http::serve_bytes_paused_at(
                        secondary_media_replies(false, operation),
                        pause_at,
                    )
                    .await;
                    fixture.provider.client = fixture
                        .provider
                        .client
                        .clone()
                        .with_auth_test_origin(paused.origin);
                    let provider = if caller_managed {
                        fixture
                            .provider
                            .caller_credential_scope(&caller_from(&source))
                            .unwrap()
                    } else {
                        fixture.provider.clone()
                    };
                    let task_provider = provider.clone();
                    let task = tokio::spawn(async move {
                        secondary_media_result(
                            &task_provider,
                            operation,
                            if caller_managed {
                                None
                            } else {
                                Some("personal")
                            },
                        )
                        .await
                    });
                    paused.arrived.await.unwrap();
                    let replacement = SodaCredential::test_credential("replacement-secret")
                        .bind_user("123456")
                        .unwrap();
                    if caller_managed {
                        *provider.caller_credential.as_ref().unwrap().lock().unwrap() =
                            replacement.clone();
                    } else if relogin {
                        fixture.put("personal", &replacement);
                    } else {
                        fixture.store.remove(Platform::Soda, "personal").unwrap();
                    }
                    paused.release.send(()).unwrap();
                    let error = task.await.unwrap().unwrap_err();
                    assert_eq!(error.code, ErrorCode::Conflict);
                    assert!(provider.take_response_credential().unwrap().is_none());
                    if caller_managed {
                        assert_eq!(
                            *provider.caller_credential.as_ref().unwrap().lock().unwrap(),
                            replacement
                        );
                    } else if relogin {
                        assert_eq!(
                            fixture.stored("personal").unwrap().secret(),
                            replacement.serialize().unwrap()
                        );
                    } else {
                        assert!(fixture.stored("personal").is_none());
                    }
                    assert_eq!(
                        paused.requests.await.unwrap().len(),
                        if operation == 2 { 4 } else { 3 }
                    );
                }
            }
        }
    }

    #[tokio::test]
    async fn account_media_delivery_rejects_untrusted_authorizations_before_any_download() {
        let id = "7304719759323564095";
        for mutation in 0..7 {
            for operation in 0..3 {
                let mut fixture = SessionFixture::new();
                let source = test_soda_credential().bind_user("123456").unwrap();
                let mut body: serde_json::Value =
                    serde_json::from_slice(&crate::client::test_account_track_fixture(false))
                        .unwrap();
                let mut model: serde_json::Value =
                    serde_json::from_str(body["track_player"]["video_model"].as_str().unwrap())
                        .unwrap();
                match mutation {
                    0 => model["video_id"] = json!("wrong-media"),
                    1 => {
                        model["video_list"][0]["main_url"] =
                            json!("https://example.invalid/private?secret=must-not-leak")
                    }
                    2 => model["url_expire"] = json!(1),
                    3 => model["video_duration"] = json!(60.0),
                    4 => model["video_list"][0]["encrypt_info"]["kid"] = json!("unsupported"),
                    5 => {
                        model["video_list"][0]["video_meta"]["size"] = json!(1024 * 1024 * 1024_u64)
                    }
                    _ => {}
                }
                body["track_player"]["video_model"] = json!(model.to_string());
                if mutation == 6 {
                    body.as_object_mut().unwrap().remove("track_player");
                }
                let (origin, server) = crate::test_http::serve(vec![
                    account_reply("123456", Some("sessionid_ss=verified")),
                    crate::test_http::json(&body.to_string(), Some("sessionid_ss=media-metadata")),
                ])
                .await;
                fixture.provider.client = fixture
                    .provider
                    .client
                    .clone()
                    .with_auth_test_origin(origin);
                let caller = fixture
                    .provider
                    .caller_credential_scope(&caller_from(&source))
                    .unwrap();
                let track = Track::new(
                    tuneweave_core::ResourceRef::new(Platform::Soda, id).unwrap(),
                    "test",
                );
                let request = StreamRequest::default();
                let error = match operation {
                    0 => caller.stream(&track, &request).await.unwrap_err(),
                    1 => caller.download(&track, &request).await.unwrap_err(),
                    _ => caller.audio_content(&track, &request).await.unwrap_err(),
                };
                assert_eq!(
                    error.code,
                    if mutation == 6 {
                        ErrorCode::PermissionDenied
                    } else {
                        ErrorCode::UpstreamError
                    }
                );
                for secret in [
                    "must-not-leak",
                    "session-secret",
                    "spade_a",
                    "private-spade",
                ] {
                    assert!(!format!("{error:?}").contains(secret));
                }
                // Account metadata is valid even when the later player authorization is unusable.
                assert!(
                    caller
                        .take_response_credential()
                        .unwrap()
                        .unwrap()
                        .secret()
                        .contains("media-metadata")
                );
                let requests = server.await.unwrap();
                assert_eq!(requests.len(), 2);
                assert!(!requests.iter().any(
                    |request| request.contains("/media/") || request.contains("/h5/seo_track")
                ));
            }
        }
    }

    #[tokio::test]
    async fn account_media_delivery_rejects_missing_and_mismatched_accounts_before_network() {
        let fixture = SessionFixture::new();
        let source = test_soda_credential().bind_user("123456").unwrap();
        let caller = fixture
            .provider
            .caller_credential_scope(&caller_from(&source))
            .unwrap();
        let track = Track::new(
            tuneweave_core::ResourceRef::new(Platform::Soda, "7304719759323564095").unwrap(),
            "test",
        );
        for (provider, account, code) in [
            (
                &fixture.provider,
                "missing",
                ErrorCode::AuthenticationRequired,
            ),
            (&caller, "personal", ErrorCode::InvalidRequest),
        ] {
            let request = StreamRequest {
                account: Some(account.to_owned()),
                ..StreamRequest::default()
            };
            assert_eq!(
                provider.stream(&track, &request).await.unwrap_err().code,
                code
            );
            assert_eq!(
                provider.download(&track, &request).await.unwrap_err().code,
                code
            );
            assert_eq!(
                provider
                    .audio_content(&track, &request)
                    .await
                    .unwrap_err()
                    .code,
                code
            );
        }
    }

    #[test]
    fn public_media_request_maps_quality_and_builds_only_local_delivery_urls() {
        let request = StreamRequest {
            quality: Quality::Lossless,
            ..StreamRequest::default()
        };
        assert!(validate_media_request(&request).is_ok());
        assert_eq!(requested_media_bitrate(&request), 2_000_000);

        let request = StreamRequest {
            quality: Quality::Low,
            bitrate: Some(123_456),
            ..StreamRequest::default()
        };
        assert_eq!(requested_media_bitrate(&request), 123_456);

        let identity = SodaTrackIdentity::parse("7304719759323564095").expect("Soda identity");
        let playback = SodaPlayback {
            preview: false,
            preview_start_ms: None,
            preview_duration_ms: None,
            duration_ms: 180_822,
            bitrate: 132_424,
            quality: Quality::Standard,
            codec: "aac".to_owned(),
            size: Some(3_000_000),
            platform_code: 10,
            encrypted: true,
        };
        assert_eq!(
            local_content_url(&identity, &playback),
            "/v1/tracks/soda:7304719759323564095/stream/content?quality=standard&bitrate=132424"
        );
        assert!(!local_content_url(&identity, &playback).contains("http"));

        for quality in [
            Quality::Dtsx,
            Quality::Surround,
            Quality::Dolby,
            Quality::Master,
            Quality::Vinyl,
        ] {
            let request = StreamRequest {
                quality,
                ..StreamRequest::default()
            };
            assert!(validate_media_request(&request).is_err());
            assert!(
                validate_media_request(&StreamRequest {
                    bitrate: Some(320_000),
                    ..request
                })
                .is_err()
            );
        }
        let account = StreamRequest {
            account: Some("default".to_owned()),
            ..StreamRequest::default()
        };
        assert!(validate_media_request(&account).is_ok());
    }

    #[test]
    fn playlists_require_canonical_ids_and_bounded_pages_for_any_source() {
        assert_eq!(
            parse_playlist_id("7200303561195061287").expect("playlist ID"),
            "7200303561195061287"
        );
        for value in ["", "0", "01", "-1", "playlist:1", "1/path", " 1"] {
            assert!(parse_playlist_id(value).is_err(), "{value:?} must fail");
        }

        assert!(validate_playlist_page(&PageRequest::new(100, 0)).is_ok());
        assert!(validate_playlist_page(&PageRequest::new(0, 0)).is_err());
        assert!(validate_playlist_page(&PageRequest::new(101, 0)).is_err());
        let account = PageRequest {
            account: Some("default".to_owned()),
            ..PageRequest::new(20, 0)
        };
        assert!(validate_playlist_page(&account).is_ok());

        let page = soda_playlist_page(Vec::new(), &PageRequest::new(20, 398), 398, 1, 0, Some(506));
        assert_eq!(page.pagination.total, Some(398));
        assert!(!page.pagination.has_more);
        assert_eq!(page.pagination.next_offset, None);
        assert_eq!(
            page.pagination.extensions["upstream_raw_resource_count"],
            506
        );
    }

    #[test]
    fn public_albums_require_canonical_ids_and_support_album_import_pages() {
        assert_eq!(
            parse_album_id("7528799183039825936").expect("album ID"),
            "7528799183039825936"
        );
        for value in ["", "0", "01", "-1", "album:1", "1/path", " 1"] {
            assert!(parse_album_id(value).is_err(), "{value:?} must fail");
        }

        assert!(validate_album_page(&PageRequest::new(100, 0)).is_ok());
        assert!(validate_album_page(&PageRequest::new(0, 0)).is_err());
        assert!(validate_album_page(&PageRequest::new(101, 0)).is_err());
        let account = PageRequest {
            account: Some("default".to_owned()),
            ..PageRequest::new(20, 0)
        };
        assert!(validate_album_page(&account).is_err());

        let tracks = (0..3)
            .map(|index| {
                Track::new(
                    tuneweave_core::ResourceRef::new(
                        Platform::Soda,
                        format!("75287991830398259{index:02}"),
                    )
                    .expect("track ref"),
                    format!("Track {index}"),
                )
            })
            .collect();
        let page = soda_album_track_page(tracks, &PageRequest::new(2, 1));
        assert_eq!(page.items.len(), 2);
        assert_eq!(page.pagination.total, Some(3));
        assert!(!page.pagination.has_more);
        assert_eq!(page.pagination.next_offset, None);
        assert_eq!(
            page.pagination.extensions["backend"],
            "official_web_album_share"
        );

        let error = unsupported_soda_playlist_source_type("unknown_collection");
        assert_eq!(
            error.code,
            tuneweave_core::ErrorCode::CapabilityNotSupported
        );
        assert_eq!(error.details["source_type"], "unknown_collection");
    }
}
