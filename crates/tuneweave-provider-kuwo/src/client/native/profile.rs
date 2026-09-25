//! Authenticated self-profile reads, independently bound to the native session.
use super::*;
use tuneweave_core::{ProviderCredential, ResourceRef, User, UserProfile};

#[cfg(test)]
pub(crate) mod tests;
pub(super) const PATH: &str = "/userinfo/lua_get_user_and_follow";

impl KuwoClient {
    /// Validates the native credential, then reads only that user's profile.
    /// This does not rotate the session, infer membership or grant music rights.
    pub async fn native_self_profile(
        &self,
        credential: &ProviderCredential,
    ) -> Result<UserProfile> {
        let input = credential::NativeCredential::parse(credential)?.input()?;
        validate_session_metadata(&input)?;
        self.validate_native_session(&input).await?;
        self.fetch_native_self_profile(&input).await
    }

    /// Provider callers validate identity and check their original selection at
    /// each network boundary. This method alone does not authenticate a session.
    pub(crate) async fn fetch_native_self_profile(
        &self,
        input: &KuwoNativeSessionInput,
    ) -> Result<UserProfile> {
        validate_session_metadata(input)?;
        let key = self.native_response_key()?;
        let plain = query(input, &key);
        let target = format!(
            "{}?f=ar&q={}",
            self.native_target(EXCHANGE_HOST, PATH),
            codec::seal_query(plain.as_bytes())?
        );
        // The official native transport adds this plural metadata header. It is
        // not a Web Cookie and must carry only the selected native identity.
        let metadata = session_metadata(input)?;
        self.native_get_with_metadata(
            EXCHANGE_HOST,
            PATH,
            "native_self_profile",
            target,
            Some(metadata),
            |body| parse(&codec::open_response(body, &key)?, input),
        )
        .await
    }
}

pub(in crate::client::native) fn query(input: &KuwoNativeSessionInput, key: &[u8; 8]) -> String {
    let mut query = url::form_urlencoded::Serializer::new(String::new());
    query.extend_pairs([
        ("uid", input.user_id()),
        ("tid", input.user_id()),
        ("src", CLIENT_SOURCE),
        ("version", CLIENT_VERSION),
        ("dev_id", input.device_id()),
        ("app_id", input.device_id()),
        ("user", input.device_user()),
        (
            "sx",
            std::str::from_utf8(key).expect("numeric protocol key"),
        ),
        ("from", "android"),
    ]);
    query.finish()
}

#[derive(Deserialize)]
struct Envelope {
    #[serde(default, deserialize_with = "deserialize_code")]
    status: Option<String>,
    info: Option<Vec<Profile>>,
}
// Deliberately do not deserialize NAME, recovery phone/email, security answers,
// QQ, or the raw object into public extensions. qf.a.k reads literal strings;
// there is no URL/HTML decoding step for these profile fields.
#[derive(Deserialize)]
struct Profile {
    #[serde(rename = "UID", deserialize_with = "deserialize_uid")]
    uid: String,
    #[serde(rename = "NICK_NAME")]
    nickname: Option<String>,
    #[serde(rename = "PIC")]
    picture: Option<String>,
    #[serde(rename = "SIGNATURE")]
    signature: Option<String>,
    #[serde(default, rename = "LEVEL", deserialize_with = "deserialize_code")]
    level: Option<String>,
    #[serde(rename = "BIRTHDAY")]
    birthday: Option<String>,
    #[serde(rename = "REGTM")]
    registered: Option<String>,
    #[serde(rename = "FIELD6")]
    background: Option<String>,
    #[serde(rename = "FIELD7")]
    background_id: Option<String>,
}

fn parse(bytes: &[u8], input: &KuwoNativeSessionInput) -> Result<UserProfile> {
    let body: Envelope = serde_json::from_slice(bytes).map_err(|_| invalid())?;
    if body.status.as_deref() != Some("200") {
        // Profile business codes are not proof that the native session expired.
        return Err(invalid());
    }
    let mut items = body.info.ok_or_else(invalid)?;
    if items.len() != 1 {
        return Err(invalid());
    }
    let value = items.pop().ok_or_else(invalid)?;
    if value.uid != input.user_id() {
        return Err(invalid());
    }
    let nickname = text(value.nickname, 1024, false, input)?;
    let signature = text(value.signature, 8192, true, input)?;
    let level = value
        .level
        .map(|level| {
            level
                .parse::<u32>()
                .ok()
                .filter(|n| n.to_string() == level)
                .ok_or_else(invalid)
        })
        .transpose()?;
    let background_id = text(value.background_id, 128, false, input)?;
    // FIELD7 is a preset ID, not a URL. The official client uses FIELD6 only
    // when FIELD7 is absent. No fabricated URL for a preset is exposed.
    let background = if background_id.is_none() {
        picture(value.background, input)?
    } else {
        None
    };
    Ok(UserProfile {
        user: User {
            resource_ref: ResourceRef::new(Platform::Kuwo, &value.uid).map_err(|_| invalid())?,
            platform: Platform::Kuwo,
            id: value.uid,
            name: nickname.unwrap_or_default(),
            avatar_url: picture(value.picture, input)?,
            signature,
            followed: None,
            mutual: None,
            extensions: Default::default(),
        },
        level,
        birthday: text(value.birthday, 64, false, input)?,
        // Keep upstream date text; do not invent a timezone or epoch unit.
        created_at: text(value.registered, 64, false, input)?,
        background_url: background,
        listened_track_count: None,
        playlist_count: None,
        playlist_subscriber_count: None,
        following_count: None,
        follower_count: None,
        event_count: None,
        description: None,
        public_listening_history: None,
        extensions: Default::default(),
    })
}

fn text(
    value: Option<String>,
    max: usize,
    multiline: bool,
    input: &KuwoNativeSessionInput,
) -> Result<Option<String>> {
    let Some(value) = value.filter(|v| !v.is_empty()) else {
        return Ok(None);
    };
    if value.len() > max
        || value
            .chars()
            .any(|c| c.is_control() && !(multiline && matches!(c, '\n' | '\r' | '\t')))
        || echoes_secret(&value, input.session_id())
    {
        return Err(invalid());
    }
    Ok(Some(value))
}

fn picture(value: Option<String>, input: &KuwoNativeSessionInput) -> Result<Option<String>> {
    let Some(value) = text(value, 2048, false, input)? else {
        return Ok(None);
    };
    let url = Url::parse(&value).map_err(|_| invalid())?;
    if value.trim() != value
        || value.contains('\\')
        || !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || !matches!(
            url.host_str(),
            Some("img1.kuwo.cn" | "img2.kuwo.cn" | "img3.kuwo.cn" | "img4.kuwo.cn")
        )
        || url.path() == "/"
    {
        return Err(invalid());
    }
    // Metadata only: retain the supplied scheme, never fetch or grant media rights.
    Ok(Some(value))
}

fn invalid() -> TuneWeaveError {
    kuwo_upstream_error("Kuwo native self profile response is invalid")
}
