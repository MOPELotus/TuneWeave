use super::*;

pub(super) struct ParsedLyricContent {
    pub content: String,
    pub line_count: usize,
}

// The official renderer selects the original format from its line header. The
// translation is a separate LRC track, including when the original is word timed.
pub(super) fn is_word_synced(raw: &str) -> bool {
    raw.lines()
        .find(|line| !line.trim().is_empty())
        .is_some_and(|line| {
            line.strip_prefix('[')
                .and_then(|line| line.split_once(']'))
                .is_some_and(|(header, _)| header.contains(','))
        })
}

pub(super) fn parse_text(raw: &str) -> Result<ParsedLyricContent> {
    if raw.trim().is_empty()
        || raw.len() > MAX_LYRIC_CONTENT_BYTES
        || raw
            .chars()
            .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
    {
        return Err(soda_upstream_error("Soda text lyrics are empty or invalid"));
    }
    let line_count = raw.lines().count();
    if line_count > MAX_LYRIC_LINES {
        return Err(soda_upstream_error(
            "Soda text lyrics exceeded the line count limit",
        ));
    }
    Ok(ParsedLyricContent {
        content: raw.replace("\r\n", "\n"),
        line_count,
    })
}

pub(super) fn parse_lrc(raw: &str) -> Result<ParsedLyricContent> {
    if raw.is_empty()
        || raw.len() > MAX_LYRIC_CONTENT_BYTES
        || raw
            .chars()
            .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
    {
        return Err(soda_upstream_error("Soda LRC content is empty or invalid"));
    }
    let mut content = String::with_capacity(raw.len());
    let mut line_count = 0;
    for (index, line) in raw.lines().enumerate() {
        if index >= MAX_LYRIC_LINES {
            return Err(soda_upstream_error(
                "Soda LRC exceeded the line count limit",
            ));
        }
        let line = line.trim_end_matches('\r');
        if line.trim().is_empty() {
            continue;
        }
        let (header, mut text) = line
            .strip_prefix('[')
            .and_then(|line| line.split_once(']'))
            .ok_or_else(|| soda_upstream_error("Soda LRC line is missing a timestamp"))?;
        if is_metadata(header) {
            if !text.is_empty() {
                return Err(soda_upstream_error("Soda LRC metadata line is invalid"));
            }
        } else {
            validate_timestamp(header)?;
            // A standard LRC line may associate the same text with several times.
            while let Some(next) = text.strip_prefix('[') {
                let Some((tag, rest)) = next.split_once(']') else {
                    break;
                };
                if !tag.as_bytes().first().is_some_and(u8::is_ascii_digit) {
                    break;
                }
                validate_timestamp(tag)?;
                text = rest;
            }
            line_count += 1;
        }
        if !content.is_empty() {
            content.push('\n');
        }
        // Preserve timestamp precision, metadata, repetitions and translated text.
        // In particular, do not reinterpret centiseconds as milliseconds.
        content.push_str(line);
    }
    if line_count == 0 {
        return Err(soda_upstream_error("Soda LRC did not contain timed lines"));
    }
    Ok(ParsedLyricContent {
        content,
        line_count,
    })
}

fn is_metadata(header: &str) -> bool {
    header.split_once(':').is_some_and(|(name, _)| {
        matches!(
            name,
            "ar" | "al" | "ti" | "au" | "by" | "re" | "ve" | "length" | "offset"
        )
    })
}

fn validate_timestamp(tag: &str) -> Result<()> {
    let valid = (|| {
        let (minutes, seconds) = tag.split_once(':')?;
        let (seconds, fraction) = seconds
            .split_once('.')
            .map_or((seconds, None), |(seconds, fraction)| {
                (seconds, Some(fraction))
            });
        if minutes.is_empty()
            || minutes.len() > 4
            || !minutes.bytes().all(|b| b.is_ascii_digit())
            || seconds.is_empty()
            || seconds.len() > 2
            || !seconds.bytes().all(|b| b.is_ascii_digit())
        {
            return None;
        }
        if minutes.parse::<u32>().ok()? >= 24 * 60 || seconds.parse::<u32>().ok()? >= 60 {
            return None;
        }
        if fraction.is_some_and(|digits| {
            !(1..=3).contains(&digits.len()) || !digits.bytes().all(|b| b.is_ascii_digit())
        }) {
            return None;
        }
        Some(())
    })()
    .is_some();
    if valid {
        Ok(())
    } else {
        Err(soda_upstream_error("Soda LRC timestamp is invalid"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn response(original: &str, translations: serde_json::Value) -> serde_json::Value {
        let mut value: serde_json::Value =
            serde_json::from_slice(&test_account_track_fixture(false)).unwrap();
        value["lyric"] = json!({"content":original,"translations":translations});
        value
    }

    fn parse(value: &serde_json::Value) -> Result<Lyrics> {
        parse_lyrics_response(
            &serde_json::to_vec(value).unwrap(),
            &SodaTrackIdentity::parse("7304719759323564095").unwrap(),
        )
    }

    #[test]
    fn explicit_text_lyrics_preserve_lines_without_inventing_timestamps() {
        let mut value = response(
            "First line\r\n\r\n[00:01.00]Literal text\n\t末行",
            json!(null),
        );
        value["lyric"]["type"] = json!("text");
        let lyrics = parse(&value).unwrap();
        assert_eq!(lyrics.format, "text");
        assert_eq!(
            lyrics.plain.as_deref(),
            Some("First line\n\n[00:01.00]Literal text\n\t末行")
        );
        assert!(lyrics.word_synced.is_none());
        assert!(lyrics.translated.is_none());
        assert!(lyrics.romanized.is_none());
        assert_eq!(lyrics.extensions["line_count"], 4);
        assert_eq!(lyrics.extensions["plain_derived_from_word_synced"], false);
        assert!(!lyrics.extensions.contains_key("line_time_unit"));
        assert!(!lyrics.extensions.contains_key("word_count"));
    }

    #[test]
    fn explicit_text_lyrics_keep_bounds_and_do_not_rescue_invalid_timed_lyrics() {
        for content in [
            " \r\n\t".to_owned(),
            "First\0second".to_owned(),
            "x".repeat(MAX_LYRIC_CONTENT_BYTES + 1),
            "x\n".repeat(MAX_LYRIC_LINES + 1),
        ] {
            let mut value = response(&content, json!(null));
            value["lyric"]["type"] = json!("text");
            assert!(parse(&value).is_err());
        }
        for kind in [None, Some("lrc"), Some("krc")] {
            let mut value = response("Untimed text", json!(null));
            if let Some(kind) = kind {
                value["lyric"]["type"] = json!(kind);
            }
            assert!(parse(&value).is_err(), "{kind:?}");
        }
    }

    #[test]
    fn word_timed_original_and_official_chinese_lrc_remain_separate() {
        let original = "[8212,2479]<0,1000,0>Hello\n[10692,2896]<0,1000,0>World";
        let translated = "[00:08.21]你好\r\n[00:10.69]世界";
        let lyrics = parse(&response(original, json!({"cn":translated}))).unwrap();
        assert_eq!(lyrics.format, "krc");
        assert_eq!(lyrics.word_synced.as_deref(), Some(original));
        assert_eq!(
            lyrics.plain.as_deref(),
            Some("[00:08.212]Hello\n[00:10.692]World")
        );
        assert_eq!(
            lyrics.translated.as_deref(),
            Some("[00:08.21]你好\n[00:10.69]世界")
        );
        assert_eq!(lyrics.extensions["translated_language"], "zh-CN");
        assert_eq!(lyrics.extensions["translated_format"], "lrc");
        assert!(lyrics.romanized.is_none());
    }

    #[test]
    fn android_chinese_language_translation_uses_the_official_lrc_consumer_path() {
        let mut value = response("[00:01.00]Original", json!(null));
        value["track"]["lyric"] = json!({
            "type": "lrc",
            "content": "[00:01.00]Original",
            "lang_translations": {
                "CHINESE": {
                    "type": "lrc",
                    "content": "[00:01.25]中文翻译",
                    "lang": "ZH-HANS-CN"
                },
                "UNSUPPORTED": { "content": "[00:01.25]Ignored" }
            }
        });

        let lyrics = parse(&value).unwrap();
        assert_eq!(lyrics.translated.as_deref(), Some("[00:01.25]中文翻译"));
        assert_eq!(lyrics.extensions["translated_format"], "lrc");
        assert_eq!(lyrics.extensions["translated_language"], "zh-Hans-CN");
        assert_eq!(lyrics.plain.as_deref(), Some("[00:01.00]Original"));
        assert!(lyrics.romanized.is_none());
    }

    #[test]
    fn pc_chinese_translation_keeps_precedence_over_the_android_language_map() {
        let mut value = response("[00:01.00]Original", json!({"cn":"[00:01.25]PC译文"}));
        value["track"]["lyric"] = json!({
            "content": "[00:01.00]Original",
            "lang_translations": { "CHINESE": { "content": "[00:01.25]Android译文" } }
        });

        let lyrics = parse(&value).unwrap();
        assert_eq!(lyrics.translated.as_deref(), Some("[00:01.25]PC译文"));
        assert_eq!(lyrics.extensions["translated_language"], "zh-CN");
    }

    #[test]
    fn unsupported_android_language_keys_do_not_become_chinese_translations() {
        let mut value = response("[00:01.00]Original", json!(null));
        value["track"]["lyric"] = json!({
            "content": "[00:01.00]Original",
            "lang_translations": { "UNSUPPORTED": { "content": "[00:01.25]Other language" } }
        });

        let lyrics = parse(&value).unwrap();
        assert!(lyrics.translated.is_none());
        assert!(!lyrics.extensions.contains_key("translated_language"));
    }

    #[test]
    fn android_track_lyric_can_supply_content_when_pc_lyric_is_absent() {
        let mut value = response("", json!(null));
        value["track"]["lyric"] = json!({
            "type": "lrc",
            "content": "[00:02.00]Android original",
            "lang_translations": { "CHINESE": { "content": "[00:02.25]Android翻译" } }
        });

        let lyrics = parse(&value).unwrap();
        assert_eq!(lyrics.plain.as_deref(), Some("[00:02.00]Android original"));
        assert_eq!(lyrics.translated.as_deref(), Some("[00:02.25]Android翻译"));
        assert_eq!(lyrics.extensions["translated_language"], "zh-Hans-CN");
    }

    #[test]
    fn instantaneous_krc_words_keep_their_text_and_original_timing() {
        let original = "[10692,2896]<0,348,0>I<348,0,0>'m<348,1000,0> here";
        let lyrics = parse(&response(original, json!({"cn":"[00:10.69]译文"}))).unwrap();
        assert_eq!(lyrics.word_synced.as_deref(), Some(original));
        assert_eq!(lyrics.plain.as_deref(), Some("[00:10.692]I'm here"));
        assert_eq!(lyrics.extensions["word_count"], 3);
    }

    #[test]
    fn ordinary_lrc_keeps_precision_metadata_repeated_times_and_translation() {
        let original = "[ar:Test]\n[offset:25]\n[00:01.25][00:05.250]Line\n[00:06]End";
        let lyrics = parse(&response(
            original,
            json!({"cn":"[00:01.25][00:05.250]译文\n[00:06]结束"}),
        ))
        .unwrap();
        assert_eq!(lyrics.format, "lrc");
        assert_eq!(lyrics.plain.as_deref(), Some(original));
        assert!(lyrics.word_synced.is_none());
        assert_eq!(lyrics.extensions["plain_derived_from_word_synced"], false);
        assert_eq!(lyrics.extensions["line_count"], 2);
        assert!(!lyrics.extensions.contains_key("word_count"));
        assert!(lyrics.translated.is_some());
    }

    #[test]
    fn translation_absence_and_unrelated_language_do_not_fabricate_a_track() {
        for translations in [
            json!(null),
            json!({}),
            json!({"cn":null}),
            json!({"cn":" \r\n"}),
            json!({"ja":"not a Chinese translation"}),
        ] {
            let lyrics = parse(&response("[00:01.50]Original", translations)).unwrap();
            assert_eq!(lyrics.plain.as_deref(), Some("[00:01.50]Original"));
            assert!(lyrics.translated.is_none());
            assert!(!lyrics.extensions.contains_key("translated_language"));
        }
    }

    #[test]
    fn nested_seo_lyrics_preserve_the_matching_translation_without_cross_merging() {
        let mut value = response("[00:01.00]Root", json!({"cn":"[00:01.00]根"}));
        value["seo_track"] = json!({"track":value["track"],"lyric":{"content":"[00:02.00]Nested","translations":{"cn":"[00:02.00]嵌套"}}});
        assert_eq!(
            parse(&value).unwrap().translated.as_deref(),
            Some("[00:01.00]根")
        );
        value["lyric"]["content"] = json!("");
        let lyrics = parse(&value).unwrap();
        assert_eq!(lyrics.plain.as_deref(), Some("[00:02.00]Nested"));
        assert_eq!(lyrics.translated.as_deref(), Some("[00:02.00]嵌套"));
    }

    #[test]
    fn invalid_lyric_formats_are_not_reported_as_lrc_success() {
        for content in [
            "plain text",
            "[00:60.00]bad seconds",
            "[00:01.x]bad fraction",
            "[00:01.0000]bad precision",
            "[00:01.00]bad\0",
            "[0,1000]missing word tags",
            "[ti:title only]",
        ] {
            assert!(parse(&response(content, json!(null))).is_err());
        }
        assert!(
            parse(&response(
                "[00:01.00]Original",
                json!({"cn":"invalid translation"})
            ))
            .is_err()
        );
    }
}
