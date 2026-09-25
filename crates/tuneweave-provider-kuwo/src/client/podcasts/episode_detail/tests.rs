use super::*;
use crate::client::catalog::tests::{home, json_response, requests, setup};
use tuneweave_core::MusicProvider;

fn track() -> serde_json::Value {
    json!({"code":200,"msg":"success","data":{"musicrid":"MUSIC_64403133","rid":64403133,
        "name":"第二期","track":2,"artist":"主播","artistid":3194753,"album":"节目集","albumid":9240675,
        "duration":733,"releaseDate":"2022-06-20","isstar":1,"content_type":"0","online":1,
        "albuminfo":"这是专辑简介，不是本期简介","pic":"https://img1.kuwo.cn/star/albumcover/500/x.jpg"}})
}
fn show() -> serde_json::Value {
    json!({"albumid":"9240675","isstar":"1","content_type":"0","name":"节目集","mcnum":457})
}

#[tokio::test]
async fn anchor_episode_detail_preserves_distinct_episode_audio_and_album_refs() {
    let mut f = setup(vec![
        home(),
        json_response(&track()),
        json_response(&show()),
    ])
    .await;
    let e = f
        .provider
        .podcast_episode("episode:64403133", None)
        .await
        .unwrap();
    assert_eq!(e.resource_ref.to_string(), "kuwo:episode:64403133");
    assert_eq!(e.podcast_ref.unwrap().to_string(), "kuwo:anchor:9240675");
    assert_eq!(e.audio.unwrap().resource_ref.to_string(), "kuwo:64403133");
    assert_eq!(e.serial_number, Some(2));
    assert_eq!(e.duration_ms, Some(733000));
    assert!(e.description.is_empty());
    assert_eq!(e.purchased, None);
    assert_eq!(e.paid, None);
    let wires = requests(&mut f, 3).await;
    assert!(wires[1].starts_with("GET /api/www/music/musicInfo?mid=64403133&"));
    assert!(wires[2].starts_with("GET /basedata.s?type=get_album_info&id=9240675&"));
    assert!(!wires[2].to_lowercase().contains("cookie:"));
}

#[tokio::test]
async fn anchor_episode_stream_uses_fresh_platform_authorization_and_preserves_denials() {
    for permitted in [false, true] {
        let playback = if permitted {
            json!({"code":200,"msg":"success","data":{"url":"https://kw-bj.kuwo.cn/anchor/file.mp3"}})
        } else {
            json!({"code":-1,"msg":"permission denied","data":null})
        };
        let mut f = setup(vec![
            home(),
            json_response(&track()),
            json_response(&show()),
            json_response(&playback),
        ])
        .await;
        let result = f
            .provider
            .podcast_episode_stream("episode:64403133", &StreamRequest::default())
            .await;
        if permitted {
            let result = result.unwrap();
            assert_eq!(result.episode_ref.to_string(), "kuwo:episode:64403133");
            assert_eq!(result.audio_ref.to_string(), "kuwo:64403133");
            assert_eq!(result.stream.resolved_track, result.audio_ref);
            assert_eq!(result.stream.actual_quality, Quality::Standard);
        } else {
            assert_eq!(result.unwrap_err().code, ErrorCode::PermissionDenied);
        }
        let wires = requests(&mut f, 4).await;
        assert!(wires[3].starts_with("GET /api/v1/www/music/playUrl?"));
        assert!(wires[3].contains("mid=64403133"));
    }
}

#[tokio::test]
async fn anchor_episode_detail_rejects_music_mini_apps_and_foreign_album_without_playback() {
    for (field, value) in [
        ("isstar", json!(0)),
        ("content_type", json!(1)),
        ("albumid", json!(0)),
    ] {
        let mut t = track();
        t["data"][field] = value;
        let mut f = setup(vec![home(), json_response(&t)]).await;
        assert!(
            f.provider
                .podcast_episode_stream("episode:64403133", &StreamRequest::default())
                .await
                .is_err()
        );
        requests(&mut f, 2).await;
    }
    let mut s = show();
    s["albumid"] = json!("9240676");
    let mut f = setup(vec![home(), json_response(&track()), json_response(&s)]).await;
    assert!(
        f.provider
            .podcast_episode("episode:64403133", None)
            .await
            .is_err()
    );
    requests(&mut f, 3).await;
}

#[tokio::test]
async fn anchor_episode_detail_and_stream_reject_account_and_invalid_options_before_io() {
    let mut f = setup(vec![]).await;
    for id in [
        "64403133",
        "anchor:64403133",
        "episode:0",
        "episode:01",
        "episode:1/2",
    ] {
        assert_eq!(
            f.provider.podcast_episode(id, None).await.unwrap_err().code,
            ErrorCode::InvalidRequest
        );
    }
    assert_eq!(
        f.provider
            .podcast_episode("episode:64403133", Some("default"))
            .await
            .unwrap_err()
            .code,
        ErrorCode::InvalidRequest
    );
    let req = StreamRequest {
        variant: StreamVariant::SingAlong,
        ..StreamRequest::default()
    };
    assert_eq!(
        f.provider
            .podcast_episode_stream("episode:64403133", &req)
            .await
            .unwrap_err()
            .code,
        ErrorCode::InvalidRequest
    );
    let c = crate::client::native::tests::credential_fixture("42", "private-episode-session")
        .caller()
        .unwrap();
    let caller = f.provider.with_caller_credential(&c).unwrap();
    assert_eq!(
        caller
            .podcast_episode("episode:64403133", None)
            .await
            .unwrap_err()
            .code,
        ErrorCode::CapabilityNotSupported
    );
    requests(&mut f, 0).await;
}

#[tokio::test]
#[ignore = "real anonymous episode metadata and playback URL authorization only; no media bytes"]
async fn live_anchor_episode_detail_and_playback_authorization() {
    let client = KuwoClient::new(&KuwoConfig::default()).unwrap();
    let e = client
        .podcast_episode("episode:64403133", None)
        .await
        .unwrap();
    assert_eq!(e.podcast_ref.unwrap().id(), "anchor:9240675");
    assert_eq!(e.audio.unwrap().id, "64403133");
    let stream = client
        .podcast_episode_stream("episode:64403133", &StreamRequest::default())
        .await
        .unwrap();
    assert_eq!(stream.episode_ref.id(), "episode:64403133");
    assert_eq!(stream.audio_ref.id(), "64403133");
}
