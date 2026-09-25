use super::*;
use crate::client::native::management::{Mutation, Outcome, write_failure};
use tuneweave_core::{
    PlaylistCreateRequest, PlaylistDeleteRequest, PlaylistDeleteResult, PlaylistItemMutationAction,
    PlaylistItemMutationRequest, PlaylistItemMutationResult, PlaylistMutationResult,
    PlaylistUpdateRequest, PlaylistVisibilityUpdateRequest,
};

#[cfg(test)]
mod collected_sort_tests;
#[cfg(test)]
mod collection_tests;
#[cfg(test)]
mod cover_tests;
#[cfg(test)]
mod edit_tests;
#[cfg(test)]
mod favorite_tests;
#[cfg(test)]
mod item_tests;
#[cfg(test)]
mod library_sort_tests;
#[cfg(test)]
mod sorting_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod visibility_tests;

impl KuwoProvider {
    pub(in crate::provider) async fn update_native_playlist_cover(
        &self,
        id: &str,
        request: &tuneweave_core::ImageUploadRequest,
    ) -> Result<tuneweave_core::PlaylistCoverUpdateResult> {
        use crate::client::native::management::cover::{Progress, image};
        image::validate(id, request)?;
        let account = request.account.as_deref().unwrap_or("default");
        let selected = self
            .selected(account)?
            .ok_or_else(authentication_required)?;
        let input = selected.credential.input()?;
        crate::client::native::validate_session_metadata(&input)?;
        let prepared = image::prepare(request).await;
        let prepared = self.finish_selected(account, &selected, prepared)?;
        let identity = self.client.validate_native_session(&input).await;
        self.finish_selected(account, &selected, identity)?;
        let mut progress = Progress::default();
        let result = self
            .client
            .perform_native_cover(&input, id, &prepared, &mut progress, || {
                self.check_selection(account, &selected)
            })
            .await;
        self.finish_selected(account, &selected, result)
            .map_err(|e| progress.failure(e))
    }

    pub(in crate::provider) async fn reorder_native_collected_playlists(
        &self,
        request: &tuneweave_core::PlaylistOrderRequest,
    ) -> Result<tuneweave_core::PlaylistOrderResult> {
        match self
            .write_native_library(Mutation::CollectedSort(request))
            .await?
        {
            Outcome::LibraryOrder(value) => Ok(value),
            _ => unreachable!("collected playlist order mutation"),
        }
    }

    pub(in crate::provider) async fn set_native_playlist_subscription(
        &self,
        id: &str,
        subscribed: bool,
        account: Option<&str>,
    ) -> Result<tuneweave_core::SubscriptionResult> {
        match self
            .write_native_library(Mutation::Collection(id, subscribed, account))
            .await?
        {
            Outcome::Subscription(value) => Ok(value),
            _ => unreachable!("playlist collection mutation"),
        }
    }

    pub(in crate::provider) async fn reorder_native_library(
        &self,
        request: &tuneweave_core::PlaylistOrderRequest,
    ) -> Result<tuneweave_core::PlaylistOrderResult> {
        match self
            .write_native_library(Mutation::LibrarySort(request))
            .await?
        {
            Outcome::LibraryOrder(value) => Ok(value),
            _ => unreachable!("library order mutation"),
        }
    }

    pub(in crate::provider) async fn reorder_native_tracks(
        &self,
        id: &str,
        request: &tuneweave_core::PlaylistTrackOrderRequest,
    ) -> Result<tuneweave_core::PlaylistTrackOrderResult> {
        match self
            .write_native_library(Mutation::Sort(id, request))
            .await?
        {
            Outcome::TrackOrder(value) => Ok(*value),
            _ => unreachable!("track order mutation"),
        }
    }

    pub(in crate::provider) async fn set_native_track_subscription(
        &self,
        id: &str,
        subscribed: bool,
        account: Option<&str>,
    ) -> Result<tuneweave_core::SubscriptionResult> {
        use crate::client::native::management::{favorites, items};
        let (action, request) = favorites::request(id, subscribed, account)?;
        match self
            .write_native_library(Mutation::Items(items::Change {
                target: items::Target::Favorite,
                action,
                request: &request,
            }))
            .await?
        {
            Outcome::Items(value) => Ok(favorites::result(request.item_refs[0].clone(), *value)),
            _ => unreachable!("favorite item mutation"),
        }
    }
    pub(in crate::provider) async fn mutate_native_playlist_items(
        &self,
        id: &str,
        action: PlaylistItemMutationAction,
        request: &PlaylistItemMutationRequest,
    ) -> Result<PlaylistItemMutationResult> {
        match self
            .write_native_library(Mutation::Items(
                crate::client::native::management::items::Change {
                    target: crate::client::native::management::items::Target::Created(id),
                    action,
                    request,
                },
            ))
            .await?
        {
            Outcome::Items(value) => Ok(*value),
            _ => unreachable!("item mutation operation"),
        }
    }
    pub(in crate::provider) async fn update_native_playlist(
        &self,
        id: &str,
        request: &PlaylistUpdateRequest,
    ) -> Result<PlaylistMutationResult> {
        match self
            .write_native_library(Mutation::Update(id, request))
            .await?
        {
            Outcome::Playlist(value) => Ok(*value),
            Outcome::Deleted(_)
            | Outcome::Items(_)
            | Outcome::TrackOrder(_)
            | Outcome::LibraryOrder(_)
            | Outcome::Subscription(_) => {
                unreachable!("update operation")
            }
        }
    }
    pub(in crate::provider) async fn update_native_playlist_visibility(
        &self,
        id: &str,
        request: &PlaylistVisibilityUpdateRequest,
    ) -> Result<PlaylistMutationResult> {
        match self
            .write_native_library(Mutation::Visibility(id, request))
            .await?
        {
            Outcome::Playlist(value) => Ok(*value),
            Outcome::Deleted(_)
            | Outcome::Items(_)
            | Outcome::TrackOrder(_)
            | Outcome::LibraryOrder(_)
            | Outcome::Subscription(_) => {
                unreachable!("visibility operation")
            }
        }
    }
    pub(in crate::provider) async fn create_native_playlist(
        &self,
        request: &PlaylistCreateRequest,
    ) -> Result<PlaylistMutationResult> {
        match self.write_native_library(Mutation::Create(request)).await? {
            Outcome::Playlist(value) => Ok(*value),
            Outcome::Deleted(_)
            | Outcome::Items(_)
            | Outcome::TrackOrder(_)
            | Outcome::LibraryOrder(_)
            | Outcome::Subscription(_) => {
                unreachable!("create operation")
            }
        }
    }
    pub(in crate::provider) async fn delete_native_playlists(
        &self,
        request: &PlaylistDeleteRequest,
    ) -> Result<PlaylistDeleteResult> {
        match self.write_native_library(Mutation::Delete(request)).await? {
            Outcome::Deleted(value) => Ok(value),
            Outcome::Playlist(_)
            | Outcome::Items(_)
            | Outcome::TrackOrder(_)
            | Outcome::LibraryOrder(_)
            | Outcome::Subscription(_) => {
                unreachable!("delete operation")
            }
        }
    }
    async fn write_native_library(&self, mutation: Mutation<'_>) -> Result<Outcome> {
        mutation.validate()?;
        let account = mutation.account().unwrap_or("default").to_owned();
        let selected = self
            .selected(&account)?
            .ok_or_else(authentication_required)?;
        let input = selected.credential.input()?;
        crate::client::native::validate_session_metadata(&input)?;
        let result = self.client.validate_native_session(&input).await;
        self.finish_selected(&account, &selected, result)?;
        let mut dispatched = false;
        let result = self
            .client
            .perform_native_mutation(&input, mutation, &mut dispatched, || {
                self.check_selection(&account, &selected)
            })
            .await;
        self.finish_selected(&account, &selected, result)
            .map_err(|e| write_failure(e, dispatched))
    }
}
