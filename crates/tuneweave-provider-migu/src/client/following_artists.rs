use super::account::profile_image;
use super::account_download::NativeAuthorization;
use super::account_media::reject_url_secret;
use super::*;
use tuneweave_core::Artist;

pub(crate) const PATH: &str = "/MIGUM2.0/v1.0/user/followingSingers.do";
pub(crate) const FOLLOW_PATH: &str = "/MIGUM2.0/v1.0/user/follow.do";
pub(crate) const UNFOLLOW_PATH: &str = "/MIGUM2.0/v1.0/user/unfollow.do";
pub(crate) const PAGE_SIZE: usize = 20;
pub(crate) const BACKEND: &str = "official_native_following_artists";

// MyFansBean -> UserFollowItem -> UserInfoItem. Ignore phone/contact data,
// account identifiers and untyped fields rather than exporting the raw DTO.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Envelope {
    code: String,
    follows_from_user: Option<Vec<Entry>>,
}

#[derive(Deserialize)]
struct Entry {
    user: Singer,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Singer {
    user_id: String,
    user_type: String,
    nick_name: String,
    small_icon: Option<String>,
    middle_icon: Option<String>,
    big_icon: Option<String>,
    icon: Option<String>,
}

fn invalid() -> TuneWeaveError {
    migu_upstream_error("Migu following artist response has invalid identity or structure")
}

fn parse(value: serde_json::Value) -> Result<Vec<Artist>> {
    let response: Envelope = serde_json::from_value(value).map_err(|_| invalid())?;
    if response.code != "000000" {
        return Err(migu_upstream_error(
            "Migu following artist request was not acknowledged",
        ));
    }
    // The official presenter treats a successful null/absent or empty list as
    // terminal. It continues on every nonempty page, even a short one. Its
    // totalCount field is not used as a singer-specific pagination promise.
    let entries = response.follows_from_user.unwrap_or_default();
    if entries.len() > PAGE_SIZE {
        return Err(invalid());
    }
    entries
        .into_iter()
        .map(|entry| {
            let singer = entry.user;
            if singer.user_type != "01"
                || singer.user_id.is_empty()
                || singer.user_id.len() > 64
                || singer.user_id.starts_with('0')
                || !singer.user_id.bytes().all(|byte| byte.is_ascii_digit())
                || singer.nick_name.trim().is_empty()
                || singer.nick_name.len() > 2048
                || singer.nick_name.chars().any(char::is_control)
            {
                return Err(invalid());
            }
            // This is the singer delegate's actual small/middle/big/icon order.
            let avatar = [
                singer.small_icon,
                singer.middle_icon,
                singer.big_icon,
                singer.icon,
            ]
            .into_iter()
            .flatten()
            .find(|value| !value.is_empty());
            Ok(Artist {
                resource_ref: ResourceRef::new(Platform::Migu, &singer.user_id)
                    .map_err(|_| invalid())?,
                platform: Platform::Migu,
                id: singer.user_id,
                name: singer.nick_name,
                aliases: vec![],
                description: String::new(),
                biography_sections: vec![],
                avatar_url: avatar.as_deref().and_then(profile_image),
                cover_url: None,
                album_count: None,
                track_count: None,
                mv_count: None,
                video_count: None,
                identities: vec![],
                extensions: Extensions::from([("backend".into(), json!(BACKEND))]),
            })
        })
        .collect()
}

impl MiguClient {
    pub(crate) async fn set_native_artist_subscription(
        &self,
        auth: &NativeAuthorization,
        id: &str,
        subscribed: bool,
    ) -> Result<()> {
        #[derive(Deserialize)]
        struct Acknowledgment {
            code: String,
            follow: String,
        }
        // MyFollowPresenter.loadFollow: 00 is an ordinary user, 01 a singer.
        // This action deliberately sends only the latter's type=1 contract.
        let value = self
            .native_get(
                "app.c.nf.migu.cn",
                if subscribed {
                    FOLLOW_PATH
                } else {
                    UNFOLLOW_PATH
                },
                &auth.token,
                Some(&auth.uid),
                vec![("userId", &auth.uid), ("followId", id), ("type", "1")],
                false,
            )
            .await?;
        let ack: Acknowledgment = serde_json::from_value(value).map_err(|_| invalid())?;
        // The official UI treats both 1 and 2 as followed. Its missing-value
        // default of 0 is not sufficient proof of a successful remote removal.
        if ack.code != "000000"
            || if subscribed {
                !matches!(ack.follow.as_str(), "1" | "2")
            } else {
                ack.follow != "0"
            }
        {
            return Err(migu_upstream_error(
                "Migu artist subscription state was not acknowledged",
            ));
        }
        Ok(())
    }

    pub(crate) async fn native_following_artists(
        &self,
        auth: &NativeAuthorization,
        page: u32,
    ) -> Result<Vec<Artist>> {
        let page = page.to_string();
        // MyFollowPresenter: ordinary users use userType=2; singers use 1.
        // BuildRequest.request() is GET. This producer has no encryption
        // interceptor: query and response are plain, with normal native signing.
        let value = self
            .native_get(
                "app.c.nf.migu.cn",
                PATH,
                &auth.token,
                Some(&auth.uid),
                vec![
                    ("pageNo", &page),
                    ("pageSize", "20"),
                    ("userId", &auth.uid),
                    ("userType", "1"),
                ],
                false,
            )
            .await?;
        parse(value)
    }
}

pub(crate) fn reject_secrets(artists: &[Artist], secrets: &[String]) -> Result<()> {
    for artist in artists {
        for text in [
            Some(artist.id.as_str()),
            Some(artist.name.as_str()),
            artist.avatar_url.as_deref(),
        ]
        .into_iter()
        .flatten()
        {
            if secrets
                .iter()
                .any(|secret| !secret.is_empty() && text.contains(secret))
            {
                return Err(migu_upstream_error(
                    "Migu following artist data contains authorization material",
                ));
            }
        }
        if let Some(url) = &artist.avatar_url {
            for secret in secrets {
                reject_url_secret(url, secret)?;
            }
        }
    }
    Ok(())
}
