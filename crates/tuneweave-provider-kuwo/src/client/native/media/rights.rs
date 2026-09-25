use super::*;

pub(super) struct Grant {
    pub(super) spec: Spec,
    pub(super) timestamp: u64,
    pub(super) token: String,
    pub(super) bc_token: Option<String>,
    pub(super) play_pay: String,
    pub(super) download_pay: String,
}
pub(super) enum Decision {
    Granted(Grant),
    Preview,
    Denied(Outcome),
}

impl KuwoClient {
    pub(super) async fn fetch_native_media_rights(
        &self,
        input: &KuwoNativeSessionInput,
        id: &str,
        choices: &[Spec],
        action: Action,
        sing_along: bool,
    ) -> Result<Decision> {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| invalid())?
            .as_millis()
            .to_string();
        let query = {
            let mut query = url::form_urlencoded::Serializer::new(String::new());
            query.extend_pairs([
                ("newver", "2"),
                ("clienttimestamp", timestamp.as_str()),
                ("uid", input.user_id()),
                ("sid", input.session_id()),
                ("from", "ar"),
                ("deviceid", input.device_user()),
                ("ver", CLIENT_VERSION),
                ("src", CLIENT_SOURCE),
                ("appuid", input.device_id()),
                ("allpay", "0"),
                ("notrace", "1"),
                ("oaid", ""),
                ("vipMode", "0"),
                ("op", "query"),
                ("action", action.rights()),
                ("signver", "new"),
                ("filter", "no"),
                ("apiversion", "4"),
                ("local", "0"),
                (
                    "quality",
                    if sing_along {
                        BCMS.tag
                    } else {
                        choices.first().ok_or_else(invalid)?.tag
                    },
                ),
                ("preload", "0"),
                ("ids", id),
            ]);
            if let Some(context) = &input.context {
                query.append_pair("android_id", &context.android_id);
            }
            query.finish()
        };
        let target = format!("{}?{query}", self.native_target(RIGHTS_HOST, RIGHTS_PATH));
        self.native_get(
            RIGHTS_HOST,
            RIGHTS_PATH,
            "native_account_media_rights",
            target,
            |bytes| {
                if sing_along {
                    let accompaniment = match parse(bytes, input, id, &[BCMS], action)? {
                        Decision::Granted(grant) => grant,
                        Decision::Denied(outcome) => return Ok(Decision::Denied(outcome)),
                        Decision::Preview => return Ok(denied(None)),
                    };
                    match parse(bytes, input, id, choices, action)? {
                        Decision::Granted(mut main) => {
                            main.bc_token = Some(accompaniment.token);
                            Ok(Decision::Granted(main))
                        }
                        Decision::Denied(outcome) => Ok(Decision::Denied(outcome)),
                        Decision::Preview => Ok(denied(None)),
                    }
                } else {
                    parse(bytes, input, id, choices, action)
                }
            },
        )
        .await
    }
}

#[derive(Deserialize)]
struct Envelope {
    result: String,
    #[serde(default, deserialize_with = "deserialize_code")]
    errorcode: Option<String>,
    #[serde(default, deserialize_with = "deserialize_code")]
    uid: Option<String>,
    #[serde(deserialize_with = "number")]
    timestamp: u64,
    songs: Vec<Song>,
}
#[derive(Deserialize)]
struct Song {
    #[serde(deserialize_with = "identifier")]
    id: String,
    audio: Vec<Audio>,
    token: Tokens,
    #[serde(rename = "payInfo")]
    pay_info: PayInfo,
}
struct Tokens(BTreeMap<String, String>);
impl<'de> Deserialize<'de> for Tokens {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        struct Unique;
        impl<'de> Visitor<'de> for Unique {
            type Value = Tokens;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a bounded unique quality token object")
            }
            fn visit_map<M: MapAccess<'de>>(
                self,
                mut map: M,
            ) -> std::result::Result<Tokens, M::Error> {
                let mut values = BTreeMap::new();
                while let Some((key, value)) = map.next_entry::<String, String>()? {
                    if values.len() >= 32
                        || key.is_empty()
                        || key.len() > 16
                        || !key
                            .bytes()
                            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
                        || value.len() > 4096
                        || value.bytes().any(|b| b.is_ascii_control())
                        || values.insert(key, value).is_some()
                    {
                        return Err(serde::de::Error::custom("invalid quality token"));
                    }
                }
                Ok(Tokens(values))
            }
        }
        d.deserialize_map(Unique)
    }
}
#[derive(Deserialize)]
struct Audio {
    quality: String,
    #[serde(deserialize_with = "number")]
    br: u64,
    fmt: String,
    policy: String,
    #[serde(deserialize_with = "number")]
    st: u64,
    cost: Option<f64>,
    price: Option<f64>,
    #[serde(default, deserialize_with = "optional_flag")]
    avaliable: Option<bool>,
}
#[derive(Deserialize)]
struct PayInfo {
    nplay: String,
    ndown: String,
    #[serde(
        default,
        rename = "cannotOnlinePlay",
        deserialize_with = "optional_flag"
    )]
    cannot_play: Option<bool>,
    #[serde(default, rename = "cannotDownload", deserialize_with = "optional_flag")]
    cannot_download: Option<bool>,
}
pub(super) fn number<'de, D: serde::Deserializer<'de>>(d: D) -> std::result::Result<u64, D::Error> {
    let value =
        deserialize_code(d)?.ok_or_else(|| serde::de::Error::custom("missing media number"))?;
    value
        .parse::<u64>()
        .ok()
        .filter(|n| n.to_string() == value)
        .ok_or_else(|| serde::de::Error::custom("invalid media number"))
}
fn identifier<'de, D: serde::Deserializer<'de>>(d: D) -> std::result::Result<String, D::Error> {
    let value = number(d)?;
    if value == 0 || value > i64::MAX as u64 {
        return Err(serde::de::Error::custom("invalid media ID"));
    }
    Ok(value.to_string())
}
fn optional_flag<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> std::result::Result<Option<bool>, D::Error> {
    deserialize_code(d)?
        .map(|value| match value.as_str() {
            "0" => Ok(false),
            "1" => Ok(true),
            _ => Err(serde::de::Error::custom("invalid media flag")),
        })
        .transpose()
}
fn pay_bits(value: &str) -> bool {
    !value.is_empty() && value.len() <= 32 && value.bytes().all(|b| matches!(b, b'0' | b'1'))
}
fn denied(code: Option<i64>) -> Decision {
    Decision::Denied(Outcome::Denied {
        code,
        message: "Kuwo did not authorize the requested full account media",
    })
}

fn parse(
    bytes: &[u8],
    input: &KuwoNativeSessionInput,
    id: &str,
    choices: &[Spec],
    action: Action,
) -> Result<Decision> {
    let body: Envelope = serde_json::from_slice(bytes).map_err(|_| invalid())?;
    if body.result != "ok"
        || body.errorcode.as_deref().is_some_and(|v| v != "0")
        || body.uid.as_deref().is_some_and(|v| v != input.user_id())
        || body.timestamp == 0
        || body.timestamp > 253_402_300_799
        || body.songs.len() != 1
    {
        return Err(invalid());
    }
    let song = body.songs.into_iter().next().ok_or_else(invalid)?;
    if song.id != id
        || song.audio.is_empty()
        || song.audio.len() > 128
        || !pay_bits(&song.pay_info.nplay)
        || !pay_bits(&song.pay_info.ndown)
    {
        return Err(invalid());
    }
    // No absent or unknown status defaults to FREE. Keep every requested
    // format's policies together: one purchased policy can satisfy the group.
    for row in &song.audio {
        if row.quality.is_empty()
            || row.quality.len() > 16
            || row.fmt.is_empty()
            || row.fmt.len() > 32
            || row.policy.len() > 32
            || row.quality.chars().any(char::is_control)
            || row.fmt.chars().any(char::is_control)
            || row.policy.chars().any(char::is_control)
            || row.br == 0
            || row.br > 100_000
            || row
                .cost
                .into_iter()
                .chain(row.price)
                .any(|v| !v.is_finite() || v.abs() > 1_000_000_000.)
        {
            return Err(invalid());
        }
    }
    if match action {
        Action::Play => song.pay_info.cannot_play == Some(true),
        Action::Download => song.pay_info.cannot_download == Some(true),
    } {
        return Ok(denied(None));
    }
    let mut last_status = None;
    for &spec in choices {
        let matching: Vec<_> = song
            .audio
            .iter()
            .filter(|row| {
                row.quality == spec.tag
                    && row.br == u64::from(spec.selector)
                    && row.fmt.eq_ignore_ascii_case(spec.rights_format)
            })
            .collect();
        if matching.is_empty() {
            continue;
        }
        let mut allowed = false;
        for row in matching {
            if row.avaliable == Some(false) {
                continue;
            }
            if !matches!(row.policy.as_str(), "" | "vip" | "song" | "album")
                || !matches!(row.st, 0 | 102 | 103 | 104 | 107 | 201 | 502 | 1000)
            {
                return Err(invalid());
            }
            if row.st != 0 {
                last_status = Some(row.st as i64);
                continue;
            }
            allowed |= match row.policy.as_str() {
                "" => row.price == Some(0.),
                "vip" => row.cost.is_some_and(|cost| cost <= 0.),
                "song" | "album" => true,
                _ => false,
            };
        }
        if allowed {
            let token = song
                .token
                .0
                .get(spec.tag)
                .filter(|v| {
                    !v.is_empty() && v.is_ascii() && !v.bytes().any(|b| b.is_ascii_whitespace())
                })
                .ok_or_else(invalid)?;
            if echoes_secret(token, input.session_id()) {
                return Err(invalid());
            }
            return Ok(Decision::Granted(Grant {
                spec,
                timestamp: body.timestamp,
                token: token.clone(),
                bc_token: None,
                play_pay: song.pay_info.nplay,
                download_pay: song.pay_info.ndown,
            }));
        }
    }
    // A known payment restriction only permits asking the separate audition
    // service. It never grants full playback or proves a preview exists. The
    // audition response must independently authorize this exact track/window.
    if action == Action::Play
        && choices.contains(&STANDARD)
        && song.pay_info.cannot_play == Some(false)
    {
        let standard: Vec<_> = song
            .audio
            .iter()
            .filter(|row| {
                row.quality == STANDARD.tag
                    && row.br == u64::from(STANDARD.selector)
                    && row.fmt.eq_ignore_ascii_case(STANDARD.rights_format)
            })
            .collect();
        if !standard.is_empty()
            && standard.iter().all(|row| {
                row.avaliable == Some(true)
                    && matches!(
                        (row.policy.as_str(), row.st),
                        ("vip", 102) | ("song", 103) | ("album", 104)
                    )
            })
        {
            return Ok(Decision::Preview);
        }
    }
    Ok(denied(last_status))
}
