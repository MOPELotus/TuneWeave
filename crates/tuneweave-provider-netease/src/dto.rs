use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub(crate) struct SearchEnvelope {
    pub result: SearchResult,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SearchResult {
    #[serde(default)]
    pub songs: Vec<Song>,
    #[serde(rename = "songCount", default)]
    pub song_count: u64,
}

#[derive(Debug, Deserialize)]
pub(crate) struct TrackEnvelope {
    #[serde(default)]
    pub songs: Vec<Song>,
    #[serde(default)]
    pub privileges: Vec<Privilege>,
}

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct Song {
    pub id: u64,
    pub name: String,
    #[serde(default, alias = "alias")]
    pub alia: Vec<String>,
    #[serde(default, alias = "artists")]
    pub ar: Vec<Artist>,
    #[serde(alias = "album")]
    pub al: Option<Album>,
    #[serde(alias = "duration")]
    pub dt: Option<u64>,
    #[serde(alias = "mvid")]
    pub mv: Option<u64>,
    pub fee: Option<i64>,
    #[serde(alias = "status")]
    pub st: Option<i64>,
    pub mark: Option<u64>,
    pub privilege: Option<Privilege>,
    #[serde(alias = "lMusic")]
    pub l: Option<AudioQuality>,
    #[serde(alias = "mMusic")]
    pub m: Option<AudioQuality>,
    #[serde(alias = "hMusic")]
    pub h: Option<AudioQuality>,
    #[serde(alias = "sqMusic")]
    pub sq: Option<AudioQuality>,
    #[serde(alias = "hrMusic")]
    pub hr: Option<AudioQuality>,
}

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct Artist {
    pub id: u64,
    pub name: String,
}

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct Album {
    pub id: u64,
    pub name: String,
    #[serde(rename = "picUrl")]
    pub pic_url: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct Privilege {
    pub id: u64,
    #[serde(default)]
    pub st: i64,
    #[serde(default)]
    pub fee: i64,
    #[serde(default)]
    pub pl: u64,
    #[serde(default)]
    pub maxbr: u64,
}

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct AudioQuality {
    #[serde(alias = "bitrate")]
    pub br: Option<u64>,
}
