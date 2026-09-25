use super::account_download::NativeAuthorization;
use super::*;
use crate::credential::error;

pub(crate) const ORDER_PATH: &str = "/MIGUM2.0/v1.0/user/mySongSorts.do";

// Keep the original private DTO values, without exporting these write fields
// through the normalized public Track model.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct NativePlaylistSong {
    pub content_id: String,
    pub song_id: String,
    pub name: String,
    pub singer: String,
}

pub(crate) fn order_song(value: &serde_json::Value) -> Option<NativePlaylistSong> {
    fn identity(value: &serde_json::Value, key: &str) -> Option<String> {
        let value = value.get(key)?.as_str()?;
        (canonical_platform_id(value) == Some(value)).then(|| value.to_owned())
    }
    fn text(value: &str) -> Option<&str> {
        (!value.trim().is_empty() && value.len() <= 2048 && !value.chars().any(char::is_control))
            .then_some(value)
    }
    if value.get("resourceType")?.as_str()? != "2" {
        return None;
    }
    let singer = match value.get("singerName") {
        Some(serde_json::Value::String(singer)) if !singer.is_empty() => text(singer)?.to_owned(),
        None | Some(serde_json::Value::Null | serde_json::Value::String(_)) => {
            let singers = value.get("singerList")?.as_array()?;
            if singers.is_empty() || singers.len() > 128 {
                return None;
            }
            let names = singers
                .iter()
                .map(|singer| text(singer.get("name")?.as_str()?))
                .collect::<Option<Vec<_>>>()?;
            // Official Song.initSinger concatenates singerList names with '|'.
            let joined = names.join("|");
            text(&joined)?.to_owned()
        }
        _ => return None,
    };
    Some(NativePlaylistSong {
        content_id: identity(value, "contentId")?,
        song_id: identity(value, "songId")?,
        name: text(value.get("songName")?.as_str()?)?.to_owned(),
        singer,
    })
}

impl MiguClient {
    pub(crate) async fn move_native_playlist_song(
        &self,
        auth: &NativeAuthorization,
        id: &str,
        song: &NativePlaylistSong,
        from: usize,
        to: usize,
    ) -> Result<()> {
        let old = (from + 1).to_string();
        let new = (to + 1).to_string();
        let value = self
            .native_get(
                "app.u.nf.migu.cn",
                ORDER_PATH,
                &auth.token,
                Some(&auth.uid),
                vec![
                    ("musicList", id),
                    ("songName", &song.name),
                    ("songId", &song.song_id),
                    ("contentId", &song.content_id),
                    ("singer", &song.singer),
                    ("newPosition", &new),
                    ("oldPostion", &old),
                ],
                false,
            )
            .await?;
        let code = value.get("code").and_then(serde_json::Value::as_str);
        if code != Some("000000") {
            let mut failure = error(
                if code.is_some_and(|code| {
                    matches!(
                        code,
                        "200000" | "200010" | "200013" | "200004" | "220000" | "290001"
                    )
                }) {
                    ErrorCode::PermissionDenied
                } else {
                    ErrorCode::UpstreamError
                },
                "Migu native playlist movement was not acknowledged",
            );
            if let Some(code) = code
                .filter(|code| code.len() == 6 && code.bytes().all(|byte| byte.is_ascii_digit()))
            {
                failure = failure.with_details(json!({"upstream_code":code}));
            }
            return Err(failure);
        }
        // The official Object callback does not promise an updated song list.
        // Only the provider's full independent readback confirms the movement.
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_playlist_order_uses_original_dto_names_and_singer_delimiters() {
        let mut value = json!({"resourceType":"2","contentId":"123","songId":"456","songName":"  原始歌名  ",
            "singerList":[{"name":" A "},{"name":"B & C"}]});
        let song = order_song(&value).unwrap();
        assert_eq!(song.name, "  原始歌名  ");
        assert_eq!(song.singer, " A |B & C");
        value["singerName"] = json!("原始合并名");
        assert_eq!(order_song(&value).unwrap().singer, "原始合并名");
        value["songId"] = json!(" 456");
        assert!(order_song(&value).is_none());
        value["songId"] = json!("456");
        value["singerName"] = json!(null);
        value["singerList"][0]["name"] = json!(null);
        assert!(order_song(&value).is_none());
    }
}
