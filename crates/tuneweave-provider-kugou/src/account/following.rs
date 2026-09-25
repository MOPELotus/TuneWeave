//! Standard x1/o0 full followed-singer synchronization, distinct from social follows.
use super::*;
use super::{cloud::Cipher, library::Number};
use std::collections::BTreeSet;
use tuneweave_core::{Artist, Extensions, ResourceRef};

mod write;

const FOLLOW_HOST: &str = "followservice.kugou.com";
const FOLLOW_PATH: &str = "/v1/get_singerlist";
const MAX_ARTISTS: usize = 10_000;
const MAX_BYTES: usize = 1_048_576;

pub(crate) struct FollowingSnapshot {
    pub(crate) version: u64,
    pub(crate) items: Vec<Artist>,
}

#[derive(Deserialize)]
struct WireEnvelope<T> {
    status: i64,
    error_code: Option<i64>,
    need_update: Option<Number>,
    version: Option<Number>,
    data: Option<T>,
}
#[derive(Deserialize)]
struct WireData {
    singerlist: Vec<WireArtist>,
}
#[derive(Deserialize)]
struct WireArtist {
    id: Number,
    name: String,
    img: Option<String>,
    fansnum: Option<Number>,
    followtime: Option<Number>,
    userid: Option<Number>,
    identity: Option<Number>,
}

fn parse(bytes: &[u8]) -> Result<FollowingSnapshot> {
    let status: WireEnvelope<IgnoredAny> =
        serde_json::from_slice(bytes).map_err(|_| malformed())?;
    if status.status != 1 || status.error_code.is_some_and(|c| c != 0) {
        return Err(error(
            if status.status == 0 && status.error_code == Some(20017) {
                ErrorCode::AuthenticationRequired
            } else {
                ErrorCode::UpstreamError
            },
            "KuGou followed-artist directory request was rejected",
        )
        .with_details(json!({"platform_code":status.error_code})));
    }
    if status.need_update != Some(Number(1)) {
        // need_update=0 means reuse the caller's existing version, not an empty
        // directory. This stateless reader always asks for a full version 0 sync.
        return Err(error(
            ErrorCode::UpstreamError,
            "KuGou followed-artist response did not contain a full snapshot",
        ));
    }
    let wire: WireEnvelope<WireData> = serde_json::from_slice(bytes).map_err(|_| malformed())?;
    let version = wire.version.ok_or_else(malformed)?.0;
    let rows = wire.data.ok_or_else(malformed)?.singerlist;
    if rows.len() > MAX_ARTISTS {
        return Err(malformed());
    }
    let mut seen = BTreeSet::new();
    let items = rows
        .into_iter()
        .map(|row| {
            if row.id.0 == 0 || !seen.insert(row.id.0) {
                return Err(malformed());
            }
            let id = row.id.0.to_string();
            let name = profile_text(Some(row.name), 1024)?.ok_or_else(malformed)?;
            let avatar_url = profile_text(row.img, 4096)?
                .map(|url| normalize_image_url(&url).ok_or_else(malformed))
                .transpose()?;
            let mut extensions = Extensions::from([
                ("backend".into(), json!("standard_followed_singers")),
                ("followed".into(), json!(true)),
            ]);
            for (key, value) in [
                ("fans_count", row.fansnum),
                ("follow_time", row.followtime),
                ("identity_code", row.identity),
            ] {
                if let Some(v) = value {
                    extensions.insert(key.into(), json!(v.0));
                }
            }
            // The linked account's userid is not the singer id or the directory owner.
            if let Some(user) = row.userid.filter(|n| n.0 > 0) {
                extensions.insert("linked_user_id".into(), json!(user.0.to_string()));
            }
            Ok(Artist {
                resource_ref: ResourceRef::new(Platform::Kugou, &id).map_err(|_| malformed())?,
                platform: Platform::Kugou,
                id,
                name,
                avatar_url,
                cover_url: None,
                aliases: vec![],
                description: String::new(),
                biography_sections: vec![],
                album_count: None,
                track_count: None,
                mv_count: None,
                video_count: None,
                identities: vec![],
                extensions,
            })
        })
        .collect::<Result<_>>()?;
    Ok(FollowingSnapshot { version, items })
}

impl KugouClient {
    pub(crate) async fn native_followed_artists(
        &self,
        session: &NativeSession,
    ) -> Result<FollowingSnapshot> {
        validate_session(session)?;
        if session.client != KugouLoginClient::Standard {
            return Err(error(
                ErrorCode::CapabilityNotSupported,
                "KuGou followed-artist directory requires a Standard native credential",
            ));
        }
        let cipher = Cipher::random()?;
        let seconds = now_ms()? / 1000;
        let params = BTreeMap::from([
            ("appid", session.client.appid().to_string()),
            ("clientver", session.client.clientver().to_string()),
            ("mid", session.device.mid.clone()),
            ("dfid", session.device.dfid().to_owned()),
            ("clienttime", seconds.to_string()),
            (
                "key",
                format!(
                    "{:x}",
                    Md5::digest(format!(
                        "{}{}{}{seconds}",
                        session.client.appid(),
                        ANDROID_SALT,
                        session.client.clientver()
                    ))
                ),
            ),
            ("p", cipher.portrait(session)?),
        ]);
        let body = cipher.encode(br#"{"version":0}"#)?;
        // X41 supplies the direct followservice host. Its HTTPS endpoint was
        // verified anonymously; never send this account protocol over HTTP.
        let url = format!("https://{FOLLOW_HOST}{FOLLOW_PATH}");
        #[cfg(test)]
        let url = self
            .login_test_origin
            .as_ref()
            .map(|origin| origin.join(FOLLOW_PATH).unwrap().to_string())
            .unwrap_or(url);
        let started = Instant::now();
        let mut status = None;
        let result = async {
            let response = self
                .http
                .post(url)
                .query(&params)
                .header(CONTENT_TYPE, "application/json;charset=utf-8")
                .body(body)
                .send()
                .await
                .map_err(network_error)?;
            status = Some(response.status());
            let bytes = read_response_with_types(
                response,
                MAX_BYTES,
                &["application/json", "application/octet-stream"],
            )
            .await?;
            parse(&cipher.decode(&bytes)?)
        }
        .await;
        self.log_upstream_request(
            "native_followed_artists",
            FOLLOW_HOST,
            FOLLOW_PATH,
            status,
            started,
            0,
            false,
            &result,
        );
        result
    }
}

#[cfg(test)]
pub(crate) mod tests;
