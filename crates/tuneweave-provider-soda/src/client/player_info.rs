use super::*;

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct PlayerInfoEnvelope {
    response_metadata: PlayerInfoMetadata,
    result: Option<PlayerInfoResult>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct PlayerInfoMetadata {
    error: Option<serde_json::Value>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct PlayerInfoResult {
    data: Option<PlayerInfoData>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct PlayerInfoData {
    status: i64,
    #[serde(rename = "VideoID")]
    video_id: String,
    media_type: String,
    duration: f64,
    total_count: u64,
    play_info_list: Vec<PlayerInfoVariant>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct PlayerInfoVariant {
    main_play_url: String,
    #[serde(default)]
    backup_play_url: FlexibleStringList,
    bitrate: u64,
    size: u64,
    codec: String,
    format: String,
    quality: String,
    duration: f64,
    url_expire: u64,
    #[serde(default)]
    encryption_method: String,
    #[serde(default)]
    play_auth: String,
    #[serde(default, rename = "PlayAuthID")]
    play_auth_id: String,
}

impl SodaClient {
    pub(super) async fn player_model(
        &self,
        player: &SodaTrackPlayer,
        upstream_now: u64,
    ) -> Result<SodaVideoModel> {
        // A malformed direct model is an error, not a reason to use another authorization.
        if player
            .video_model
            .as_ref()
            .is_some_and(|value| !value.is_empty())
        {
            return decode_direct_player_model(player);
        }
        if upstream_now == 0 || player.media_id.trim().is_empty() {
            return Err(soda_upstream_error(
                "Soda player info has no verifiable identity or upstream time",
            ));
        }
        let url = validate_player_info_url(player.url_player_info.as_deref().unwrap_or_default())?;
        let started = Instant::now();
        let mut http_status = None;
        let result = async {
            // The returned signed URL authorizes this GET; account cookies never leave PC APIs.
            let response = self
                .login_request(reqwest::Method::GET, url)
                .header(reqwest::header::ACCEPT, "application/json")
                .send()
                .await
                .map_err(soda_network_error)?;
            http_status = Some(response.status());
            if response.status() != StatusCode::OK {
                let mut error = soda_media_http_error(response.status());
                error.message = format!("Soda player info returned HTTP {}", response.status());
                return Err(error);
            }
            if response.headers().get(CONTENT_TYPE).is_some_and(|value| {
                value.to_str().map_or(true, |value| {
                    !value
                        .split(';')
                        .next()
                        .unwrap_or_default()
                        .trim()
                        .eq_ignore_ascii_case("application/json")
                })
            }) {
                return Err(soda_upstream_error(
                    "Soda player info returned an unexpected content type",
                ));
            }
            let body = read_bounded_response(response, "Soda player info").await?;
            parse_player_info(&body, player, upstream_now)
        }
        .await;
        self.log_upstream_request(
            "player_info",
            "vod-luna.douyin.com",
            "/",
            http_status,
            started,
            &result,
        );
        result
    }
}

fn validate_player_info_url(value: &str) -> Result<Url> {
    let invalid = || soda_upstream_error("Soda returned an untrusted player info URL");
    if value.is_empty() || value.len() > 8192 || value.bytes().any(|byte| byte.is_ascii_control()) {
        return Err(invalid());
    }
    let url = Url::parse(value).map_err(|_| invalid())?;
    if url.scheme() != "https"
        || url.host_str() != Some("vod-luna.douyin.com")
        || !url.username().is_empty()
        || url.password().is_some()
        || !matches!(url.port(), None | Some(443))
        || url.fragment().is_some()
        || url.path() != "/"
        || url.query().is_none_or(str::is_empty)
    {
        return Err(invalid());
    }
    Ok(url)
}

fn parse_player_info(
    body: &[u8],
    player: &SodaTrackPlayer,
    upstream_now: u64,
) -> Result<SodaVideoModel> {
    let invalid =
        || soda_upstream_error("Soda player info returned inconsistent authorization metadata");
    let envelope: PlayerInfoEnvelope = serde_json::from_slice(body)
        .map_err(|_| soda_upstream_error("Soda player info returned malformed data"))?;
    if envelope.response_metadata.error.is_some() {
        return Err(soda_upstream_error(
            "Soda player info reported an upstream error",
        ));
    }
    let data = envelope
        .result
        .and_then(|result| result.data)
        .ok_or_else(invalid)?;
    let duration_ms = seconds_to_milliseconds(data.duration).ok_or_else(invalid)?;
    if data.status != 10
        || data.media_type != "audio"
        || data.video_id.is_empty()
        || data.video_id != player.media_id
        || data.play_info_list.is_empty()
        || data.play_info_list.len() > 16
        || data.total_count != data.play_info_list.len() as u64
        || upstream_now == 0
    {
        return Err(invalid());
    }
    let mut variants = Vec::with_capacity(data.play_info_list.len());
    let mut expires_at = u64::MAX;
    for variant in data.play_info_list {
        if seconds_to_milliseconds(variant.duration)
            .is_none_or(|value| value.abs_diff(duration_ms) > 2000)
            || variant.url_expire <= upstream_now
            || variant.url_expire > upstream_now.saturating_add(2 * 24 * 60 * 60)
        {
            return Err(invalid());
        }
        // The official PC player decodes PlayAuth with the same decodeSpade call
        // as direct models. The VOD schema defines PlayAuthID as the key ID.
        // Common validation still requires explicit CENC, a complete authorization
        // and a matching tenc KID when the content is decrypted.
        let encrypt_info = SodaMediaEncryption {
            encrypt: !variant.encryption_method.is_empty()
                || !variant.play_auth.is_empty()
                || !variant.play_auth_id.is_empty(),
            encryption_method: variant.encryption_method,
            spade_a: variant.play_auth,
            kid: variant.play_auth_id,
        };
        validate_media_url(&variant.main_play_url)?;
        let backups = variant
            .backup_play_url
            .into_vec()
            .into_iter()
            .filter(|url| !url.is_empty())
            .collect::<Vec<_>>();
        for url in &backups {
            validate_media_url(url)?;
        }
        expires_at = expires_at.min(variant.url_expire);
        variants.push(SodaVideoVariant {
            main_url: variant.main_play_url,
            backup_url: FlexibleStringList::Many(backups),
            video_meta: SodaVideoMeta {
                quality: variant.quality,
                vtype: variant.format,
                bitrate: variant.bitrate,
                real_bitrate: variant.bitrate,
                size: variant.size,
                codec_type: variant.codec,
                audio_sample_rate: String::new(),
            },
            encrypt_info,
        });
    }
    // Normalize the explicit secondary success and verified HTTPS URLs into the common
    // player representation. Missing sample rate remains unknown.
    Ok(SodaVideoModel {
        status: data.status,
        message: "success".to_owned(),
        video_id: data.video_id,
        enable_ssl: true,
        video_duration: data.duration,
        media_type: data.media_type,
        url_expire: expires_at,
        video_list: variants,
    })
}

#[cfg(test)]
pub(crate) mod encrypted_tests;

#[cfg(test)]
pub(crate) fn test_secondary_fixture(preview: bool) -> (serde_json::Value, serde_json::Value) {
    let mut body: serde_json::Value =
        serde_json::from_slice(&test_account_track_fixture(preview)).unwrap();
    let model: serde_json::Value =
        serde_json::from_str(body["track_player"]["video_model"].as_str().unwrap()).unwrap();
    body["track_player"]["video_model"] = json!("");
    body["track_player"]["url_player_info"] =
        json!("https://vod-luna.douyin.com/?Action=GetPlayInfo&token=player-secret");
    let now = body["status_info"]["now"].as_u64().unwrap();
    let variants: Vec<_> = model["video_list"].as_array().unwrap().iter().enumerate().map(|(index, variant)| {
        let meta = &variant["video_meta"];
        json!({
            "MainPlayUrl": variant["main_url"], "BackupPlayUrl":variant["backup_url"],
            "Bitrate":meta["bitrate"],"Size":meta["size"],"Codec":meta["codec_type"],
            "Format":meta["vtype"],"Quality":meta["quality"],"Duration":model["video_duration"],
            "UrlExpire":now+3600+index as u64*300,"EncryptionMethod":"","PlayAuth":"","PlayAuthID":"",
        })
    }).collect();
    let info = json!({"ResponseMetadata":{},"Result":{"Data":{
        "Status":10,"VideoID":body["track_player"]["media_id"],"MediaType":"audio",
        "Duration":model["video_duration"],"TotalCount":variants.len(),"PlayInfoList":variants,
    }}});
    (body, info)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn authorized(
        body: &serde_json::Value,
        info: &serde_json::Value,
    ) -> Result<ValidatedSodaMedia> {
        let envelope = parse_track_envelope(&serde_json::to_vec(body).unwrap())?;
        let player = envelope.track_player.as_ref().unwrap();
        let model = parse_player_info(
            &serde_json::to_vec(info).unwrap(),
            player,
            envelope.status_info.now,
        )?;
        validate_decoded_player_model(
            envelope.track.as_ref().unwrap(),
            player,
            model,
            200000,
            envelope.status_info.now,
        )
    }

    #[test]
    fn secondary_models_preserve_rights_quality_and_the_earliest_expiry() {
        for preview in [false, true] {
            let (mut body, mut info) = test_secondary_fixture(preview);
            let now = body["status_info"]["now"].as_u64().unwrap();
            // The selected variant is not necessarily the one with the earliest expiry.
            info["Result"]["Data"]["PlayInfoList"][2]["UrlExpire"] = json!(now + 1200);
            let media = authorized(&body, &info).unwrap();
            assert_eq!(media.preview, preview);
            assert_eq!(media.selected_bitrate, 132424);
            assert_eq!(media.expires_at, now + 1200);
            assert!(!media.encrypted);
            assert!(media.specs.iter().all(|spec| spec.sample_rate_hz.is_none()));
            body["track_player"]["expire_at"] = json!(now + 600);
            assert_eq!(authorized(&body, &info).unwrap().expires_at, now + 600);
        }
    }

    #[test]
    fn inconsistent_secondary_authorizations_never_become_playable() {
        for mutation in 0..23 {
            let (mut body, mut info) = test_secondary_fixture(false);
            let data = &mut info["Result"]["Data"];
            match mutation {
                0 => data["Status"] = json!(0),
                1 => data["VideoID"] = json!("other-media"),
                2 => data["MediaType"] = json!("video"),
                3 => data["Duration"] = json!(60.001),
                4 => data["TotalCount"] = json!(4),
                5 => data["PlayInfoList"] = json!([]),
                6 => {
                    data["PlayInfoList"][0]["MainPlayUrl"] =
                        json!("https://example.invalid/private-secret")
                }
                7 => {
                    data["PlayInfoList"][0]["BackupPlayUrl"] =
                        json!("http://127.0.0.1/private-secret")
                }
                8 => data["PlayInfoList"][0]["UrlExpire"] = json!(1),
                9 => data["PlayInfoList"][0]["UrlExpire"] = json!(u64::MAX),
                10 => data["PlayInfoList"][0]["Duration"] = json!(42),
                11 => data["PlayInfoList"][0]["Codec"] = json!("mp3"),
                12 => data["PlayInfoList"][0]["Size"] = json!(0),
                13 => data["PlayInfoList"][0]["Bitrate"] = json!(0),
                14 => data["PlayInfoList"][0]["Quality"] = json!("unknown"),
                15 => data["PlayInfoList"][0]["EncryptionMethod"] = json!("cenc-aes-ctr"),
                16 => data["PlayInfoList"][0]["PlayAuth"] = json!("private-secret"),
                17 => {
                    data["PlayInfoList"][0]["PlayAuthID"] =
                        json!("42424242424242424242424242424242")
                }
                18 => info["ResponseMetadata"]["Error"] = json!({"Message":"private-secret"}),
                19 => info["Result"] = json!({"CipherText":"private-secret"}),
                20 => body["status_info"]["now"] = json!(0),
                21 => data["PlayInfoList"] = json!(vec![data["PlayInfoList"][0].clone(); 17]),
                _ => {
                    data["PlayInfoList"][0]
                        .as_object_mut()
                        .unwrap()
                        .remove("UrlExpire");
                }
            }
            let error = match authorized(&body, &info) {
                Ok(_) => panic!("invalid secondary authorization accepted: {mutation}"),
                Err(error) => error,
            };
            assert_eq!(error.code, ErrorCode::UpstreamError);
            assert!(!format!("{error:?}").contains("private-secret"));
        }
    }

    #[test]
    fn player_info_urls_are_bounded_fixed_https_and_never_exposed_in_errors() {
        assert!(
            validate_player_info_url(
                "https://vod-luna.douyin.com/?Action=GetPlayInfo&token=private-secret"
            )
            .is_ok()
        );
        for url in [
            "",
            "https://vod-luna.douyin.com/",
            "http://vod-luna.douyin.com/?token=private-secret",
            "https://vod-luna.douyin.com.evil.invalid/?token=private-secret",
            "https://vod-luna.douyin.com:8443/?token=private-secret",
            "https://user@vod-luna.douyin.com/?token=private-secret",
            "https://vod-luna.douyin.com/?token=private-secret#fragment",
            "https://vod-luna.douyin.com/private-secret?query=1",
            "https://vod-luna.douyin.com/?token=private-secret\n",
        ] {
            let error = validate_player_info_url(url).unwrap_err();
            assert!(!error.to_string().contains("private-secret"));
        }
        assert!(
            validate_player_info_url(&format!(
                "https://vod-luna.douyin.com/?token={}",
                "s".repeat(8192)
            ))
            .is_err()
        );
    }

    #[tokio::test]
    async fn direct_models_take_precedence_and_invalid_direct_data_does_not_fall_back() {
        let mut body: serde_json::Value =
            serde_json::from_slice(&test_account_track_fixture(false)).unwrap();
        body["track_player"]["url_player_info"] = json!("https://example.invalid/private-secret");
        let envelope = parse_track_envelope(&serde_json::to_vec(&body).unwrap()).unwrap();
        let client = SodaClient::test_client()
            .with_auth_test_origin(Url::parse("http://127.0.0.1:9/").unwrap());
        let mut player = envelope.track_player.unwrap();
        assert!(
            client
                .player_model(&player, envelope.status_info.now)
                .await
                .is_ok()
        );
        player.video_model = Some("not-json".to_owned());
        let error = match client.player_model(&player, envelope.status_info.now).await {
            Ok(_) => panic!("malformed direct model accepted"),
            Err(error) => error,
        };
        assert!(error.message.contains("malformed player model"));
    }

    #[tokio::test]
    async fn absent_or_null_direct_models_use_secondary_but_offline_tracks_do_not_request_it() {
        for representation in 0..3 {
            let (mut body, info) = test_secondary_fixture(false);
            match representation {
                0 => {
                    body["track_player"]
                        .as_object_mut()
                        .unwrap()
                        .remove("video_model");
                }
                1 => body["track_player"]["video_model"] = serde_json::Value::Null,
                _ => {}
            }
            let (origin, server) =
                crate::test_http::serve(vec![crate::test_http::json(&info.to_string(), None)])
                    .await;
            let client = SodaClient::test_client().with_auth_test_origin(origin);
            let identity = SodaTrackIdentity::parse("7304719759323564095").unwrap();
            let availability = client
                .availability_body(
                    &serde_json::to_vec(&body).unwrap(),
                    &identity,
                    &TrackAvailabilityRequest::new(200000),
                )
                .await
                .unwrap();
            assert!(availability.playable);
            assert_eq!(server.await.unwrap().len(), 1);
        }
        let identity = SodaTrackIdentity::parse("7304719759323564095").unwrap();
        let client = SodaClient::test_client()
            .with_auth_test_origin(Url::parse("http://127.0.0.1:9/").unwrap());
        let (mut body, _) = test_secondary_fixture(false);
        body["track"]["state"]["offline"] = json!(true);
        let result = client
            .availability_body(
                &serde_json::to_vec(&body).unwrap(),
                &identity,
                &TrackAvailabilityRequest::new(200000),
            )
            .await
            .unwrap();
        assert!(!result.playable);
        assert_eq!(result.extensions["unavailable_reason"], "offline");
        body["track"]["state"]["offline"] = json!(false);
        body["track_player"] = serde_json::Value::Null;
        assert!(
            !client
                .availability_body(
                    &serde_json::to_vec(&body).unwrap(),
                    &identity,
                    &TrackAvailabilityRequest::new(200000)
                )
                .await
                .unwrap()
                .playable
        );
    }

    #[tokio::test]
    #[ignore = "calls official public Soda metadata and player info without account cookies"]
    async fn live_public_secondary_model_has_bound_media_identity_and_expiry() {
        let identity = SodaTrackIdentity::parse("7304719759323564095").unwrap();
        let client = SodaClient::test_client();
        let body = client
            .fetch_track_body(&identity, "Soda playback")
            .await
            .unwrap();
        let envelope = parse_track_envelope(&body).unwrap();
        let track = validate_media_track(&envelope, &identity, "Soda playback").unwrap();
        let mut player: SodaTrackPlayer = serde_json::from_value(
            serde_json::from_slice::<serde_json::Value>(&body).unwrap()["track_player"].clone(),
        )
        .unwrap();
        player.video_model = None;
        let model = client
            .player_model(&player, envelope.status_info.now)
            .await
            .unwrap();
        let media =
            validate_decoded_player_model(track, &player, model, 200000, envelope.status_info.now)
                .unwrap();
        assert!(!media.specs.is_empty());
        assert!(media.expires_at > envelope.status_info.now);
        assert!(media.expires_at <= envelope.status_info.now + 2 * 24 * 60 * 60);
        assert_eq!(media.preview, media.preview_duration_ms.is_some());
    }

    #[tokio::test]
    async fn secondary_http_failures_are_bounded_and_do_not_invalidate_the_account() {
        let (body, info) = test_secondary_fixture(false);
        let envelope = parse_track_envelope(&serde_json::to_vec(&body).unwrap()).unwrap();
        let player = envelope.track_player.unwrap();
        for (reply, code) in [
            ("HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_owned(), ErrorCode::PermissionDenied),
            ("HTTP/1.1 429 Too Many Requests\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_owned(), ErrorCode::RateLimited),
            ("HTTP/1.1 302 Found\r\nLocation: https://example.invalid/private-secret\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_owned(), ErrorCode::UpstreamError),
            (format!("HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", info.to_string().len(), info), ErrorCode::UpstreamError),
            (format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", MAX_API_RESPONSE_BYTES+1), ErrorCode::UpstreamError),
            (crate::test_http::json("malformed-private-secret", None), ErrorCode::UpstreamError),
        ] {
            let (origin, server) = crate::test_http::serve(vec![reply]).await;
            let client = SodaClient::test_client().with_auth_test_origin(origin);
            let error = match client.player_model(&player, envelope.status_info.now).await {
                Ok(_) => panic!("invalid player info response accepted"), Err(error) => error,
            };
            assert_eq!(error.code, code);
            assert!(!format!("{error:?}").contains("private-secret"));
            let requests = server.await.unwrap();
            assert_eq!(requests.len(), 1);
            assert!(requests[0].starts_with("GET /?Action=GetPlayInfo&token=player-secret "));
            assert!(!requests[0].to_ascii_lowercase().contains("cookie:"));
        }
    }
}
