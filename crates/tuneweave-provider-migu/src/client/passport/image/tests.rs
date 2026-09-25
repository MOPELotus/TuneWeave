use super::*;

// A generated blank 2x2 JPEG; contains no service challenge or account data.
pub(crate) fn image_body(chinese: bool) -> serde_json::Value {
    json!({"status":2000,"result":{"graphtype":if chinese { "1" } else { "0" },
        "captchaurl":"data:image/jpeg;base64,/9j/4AAQSkZJRgABAQAAAQABAAD/2wBDAAgGBgcGBQgHBwcJCQgKDBQNDAsLDBkSEw8UHRofHh0aHBwgJC4nICIsIxwcKDcpLDAxNDQ0Hyc5PTgyPC4zNDL/2wBDAQkJCQwLDBgNDRgyIRwhMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjL/wAARCAACAAIDASIAAhEBAxEB/8QAHwAAAQUBAQEBAQEAAAAAAAAAAAECAwQFBgcICQoL/8QAtRAAAgEDAwIEAwUFBAQAAAF9AQIDAAQRBRIhMUEGE1FhByJxFDKBkaEII0KxwRVS0fAkM2JyggkKFhcYGRolJicoKSo0NTY3ODk6Q0RFRkdISUpTVFVWV1hZWmNkZWZnaGlqc3R1dnd4eXqDhIWGh4iJipKTlJWWl5iZmqKjpKWmp6ipqrKztLW2t7i5usLDxMXGx8jJytLT1NXW19jZ2uHi4+Tl5ufo6erx8vP09fb3+Pn6/8QAHwEAAwEBAQEBAQEBAQAAAAAAAAECAwQFBgcICQoL/8QAtREAAgECBAQDBAcFBAQAAQJ3AAECAxEEBSExBhJBUQdhcRMiMoEIFEKRobHBCSMzUvAVYnLRChYkNOEl8RcYGRomJygpKjU2Nzg5OkNERUZHSElKU1RVVldYWVpjZGVmZ2hpanN0dXZ3eHl6goOEhYaHiImKkpOUlZaXmJmaoqOkpaanqKmqsrO0tba3uLm6wsPExcbHyMnK0tPU1dbX2Nna4uPk5ebn6Onq8vP09fb3+Pn6/9oADAMBAAIRAxEAPwAooooA/9k="}})
}

#[test]
fn image_response_accepts_only_bounded_inline_jpeg_and_known_graph_types() {
    for chinese in [false, true] {
        let body = image_body(chinese);
        assert!(PassportImage::parse(body.clone()).is_ok());
        for url in [
            "https://untrusted.invalid/challenge",
            "data:image/svg+xml;base64,PHN2Zz4=",
            "data:image/jpeg;base64,AAAA",
            "data:image/jpeg;base64,!!!",
        ] {
            let mut bad = body.clone();
            bad["result"]["captchaurl"] = json!(url);
            assert!(PassportImage::parse(bad).is_err());
        }
        for kind in [json!(2), json!(null), json!(-1), json!(0.0), json!("01")] {
            let mut bad = body.clone();
            bad["result"]["graphtype"] = kind;
            assert!(PassportImage::parse(bad).is_err());
        }
        let raw = STANDARD
            .decode(
                body["result"]["captchaurl"]
                    .as_str()
                    .unwrap()
                    .split_once(',')
                    .unwrap()
                    .1,
            )
            .unwrap();
        for end in 0..raw.len() {
            assert!(!jpeg_envelope(&raw[..end]));
        }
        let mut oversized = raw;
        let frame = oversized
            .windows(2)
            .position(|v| v == [0xff, 0xc0])
            .unwrap();
        oversized[frame + 5..frame + 7].copy_from_slice(&2048_u16.to_be_bytes());
        assert!(!jpeg_envelope(&oversized));
        let mut bad = body;
        bad["result"]["captchaurl"] =
            json!(format!("data:image/jpeg;base64,{}", "A".repeat(45056)));
        assert!(PassportImage::parse(bad).is_err());
    }
}

#[test]
fn image_answer_validation_and_debug_do_not_expose_secrets() {
    for (chinese, valid, invalid) in [
        (
            false,
            vec!["0", "9", "42", "99"],
            vec!["00", "01", "-1", "100", "１２", " 1"],
        ),
        (
            true,
            vec!["汉字", "图形验证"],
            vec!["汉", "汉字图形码", "汉a", "汉 字"],
        ),
    ] {
        let image = PassportImage::parse(image_body(chinese)).unwrap();
        for answer in valid {
            assert!(image.validate_answer(answer).is_ok());
        }
        for answer in invalid {
            assert!(image.validate_answer(answer).is_err());
        }
    }
}
