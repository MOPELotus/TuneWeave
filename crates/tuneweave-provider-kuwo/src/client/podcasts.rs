use super::*;
use catalog::Unsigned;
use tuneweave_core::{
    CreatorSummary, Page, PageMeta, Podcast, PodcastCatalog, PodcastCategory,
    PodcastCategoryRecommendation, PodcastCategoryRecommendations, PodcastListRequest,
    PodcastTaxonomy, PodcastTaxonomyKind, PodcastTaxonomyRequest,
};

mod episode_detail;
mod episodes;
mod taxonomy;
#[cfg(test)]
mod tests;

const ENDPOINT: &str = "https://mobilebasedata.kuwo.cn/basedata.s";

impl KuwoClient {
    /// Reads anonymous native anchor-album metadata. This does not authorize playback.
    pub async fn podcast(&self, id: &str, account: Option<&str>) -> Result<Podcast> {
        let album_id = request_album(id, account)?;
        let started = Instant::now();
        let mut status = None;
        let result = tokio::time::timeout(Duration::from_secs(30), async {
            // The official native producer is s2.X / get_album_info. Anonymous
            // metadata needs no account, Web session, or fabricated device identity.
            let response = self
                .http
                .get(self.web_target(ENDPOINT))
                .header(ACCEPT, "application/json")
                .query(&[
                    ("type", "get_album_info"),
                    ("id", album_id),
                    ("szb", "1"),
                    ("aapiver", "1"),
                ])
                .send()
                .await
                .map_err(kuwo_network_error)?;
            status = Some(response.status());
            let bytes = catalog::read_response(response).await?;
            parse(&bytes, album_id)
        })
        .await
        .unwrap_or_else(|_| {
            Err(TuneWeaveError::new(
                ErrorCode::UpstreamTimeout,
                "Kuwo podcast detail exceeded the total time budget",
            )
            .with_platform(Platform::Kuwo))
        });
        self.log_upstream_request(
            "podcast_detail",
            "mobilebasedata.kuwo.cn",
            "/basedata.s",
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
struct Detail {
    albumid: Unsigned,
    isstar: Unsigned,
    content_type: Unsigned,
    name: String,
    #[serde(default)]
    desc: String,
    big_pic: Option<String>,
    artist: Option<String>,
    artid: Option<Unsigned>,
    artpic: Option<String>,
    mcnum: Option<Unsigned>,
    coll_num: Option<Unsigned>,
    pnum: Option<Unsigned>,
    #[serde(rename = "payPolicy")]
    pay_policy: Option<Unsigned>,
}

fn parse(bytes: &[u8], requested: &str) -> Result<Podcast> {
    let raw: Detail = serde_json::from_slice(bytes).map_err(|_| invalid())?;
    if raw.albumid.id()? != requested || raw.isstar.value()? != 1 || raw.content_type.value()? != 0
    {
        return Err(invalid());
    }
    text(&raw.name, 2048, false)?;
    if raw.name.trim().is_empty() {
        return Err(invalid());
    }
    text(&raw.desc, 128 * 1024, true)?;
    let resource =
        ResourceRef::new(Platform::Kuwo, format!("anchor:{requested}")).map_err(|_| invalid())?;
    let mut podcast = Podcast::new(resource, raw.name);
    podcast.description = raw.desc;
    podcast.cover_url = image(raw.big_pic)?;
    if let Some(name) = raw.artist.filter(|value| !value.trim().is_empty()) {
        text(&name, 2048, false)?;
        podcast.creator = Some(CreatorSummary {
            resource_ref: raw
                .artid
                .map(|id| ResourceRef::new(Platform::Kuwo, id.id()?).map_err(|_| invalid()))
                .transpose()?,
            name,
            avatar_url: image(raw.artpic)?,
        });
    }
    podcast.episode_count = raw.mcnum.map(|n| n.value()).transpose()?;
    podcast.subscriber_count = raw.coll_num.map(|n| n.value()).transpose()?;
    podcast.play_count = raw.pnum.map(|n| n.value()).transpose()?;
    podcast
        .extensions
        .insert("backend".into(), json!("native_anchor_album"));
    podcast
        .extensions
        .insert("source_album_id".into(), json!(requested));
    if let Some(policy) = raw.pay_policy {
        podcast
            .extensions
            .insert("native_pay_policy".into(), json!(policy.value()?));
    }
    // payPolicy is not a caller entitlement or an album purchase receipt.
    // Do not copy UID, arbitrary URLs, rich HTML, or mini-app navigation blobs.
    Ok(podcast)
}

fn text(value: &str, max: usize, multiline: bool) -> Result<()> {
    if value.len() > max
        || value
            .chars()
            .any(|c| c.is_control() && !(multiline && matches!(c, '\n' | '\r' | '\t')))
    {
        return Err(invalid());
    }
    Ok(())
}

fn image(value: Option<String>) -> Result<Option<String>> {
    let Some(value) = value.filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    text(&value, 2048, false)?;
    let url = Url::parse(&value).map_err(|_| invalid())?;
    if !matches!(url.scheme(), "https" | "http")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || !url.host_str().is_some_and(|host| {
            [
                "img1.kuwo.cn",
                "img2.kuwo.cn",
                "img3.kuwo.cn",
                "img4.kuwo.cn",
                "kwimg1.kuwo.cn",
                "kwimg2.kuwo.cn",
                "kwimg3.kuwo.cn",
                "kwimg4.kuwo.cn",
            ]
            .contains(&host)
        })
        || !url.path().starts_with("/star/")
    {
        return Err(invalid());
    }
    Ok(Some(value))
}

fn invalid() -> TuneWeaveError {
    kuwo_upstream_error("Kuwo podcast detail returned an invalid response")
}

fn request_album<'a>(id: &'a str, account: Option<&str>) -> Result<&'a str> {
    id.strip_prefix("anchor:")
        .filter(|value| {
            value
                .parse::<u64>()
                .ok()
                .is_some_and(|n| n > 0 && n.to_string() == *value)
        })
        .filter(|_| account.is_none())
        .ok_or_else(|| {
            kuwo_invalid_request("Kuwo podcast requires anchor:<positive album ID> and no account")
        })
}
