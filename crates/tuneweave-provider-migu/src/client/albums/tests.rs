use super::*;

#[test]
fn ordinary_and_digital_album_metadata_preserve_distinct_ids_unknown_counts_and_rights() {
    let ordinary:AlbumMetadata=serde_json::from_value(json!({"resourceType":"2003","albumId":"77","title":" Album ","singer":"Artist","singerId":"12","summary":" Intro\nSecond ","publishDate":"2024-02-29","albumAliasName":"Alias","publishCompany":"Label","albumClass":"Live"})).unwrap();
    let album = map_album_metadata(ordinary, "test").unwrap();
    assert_eq!(album.id, "77");
    assert_eq!(album.name, "Album");
    assert_eq!(album.track_count, None);
    assert_eq!(album.description, "Intro\nSecond");
    assert_eq!(album.published_at.as_deref(), Some("2024-02-29"));
    assert_eq!(album.aliases, ["Alias"]);
    assert_eq!(
        album.artists[0].resource_ref.as_ref().unwrap().to_string(),
        "migu:12"
    );
    let digital:DigitalAlbumMetadata=serde_json::from_value(json!({"resourceType":"5","contentId":"77","itemId":"999","title":"Product","totalCount":"0","price":"3000","foreverListen":true,"isCallPayment":"1","isBeforeSaleEndDate":"1","songItems":[]})).unwrap();
    let product = map_digital_album_metadata(digital, "test").unwrap();
    assert_eq!(product.id, "77");
    assert_eq!(product.extensions["item_id"], "999");
    assert_eq!(product.track_count, Some(0));
    assert!(product.price.is_none());
    assert!(product.is_free.is_none());
    assert!(product.purchasable.is_none());
    assert!(product.purchased.is_none());
    assert!(product.sale_count.is_none());
    assert!(!product.extensions.contains_key("foreverListen"));
}

#[test]
fn album_count_and_publication_fields_reject_malformed_values() {
    for date in [
        "2023-02-29",
        "2024-02-30",
        "2024-00-01",
        "2024-01-00",
        "0000-01-01",
        "20240101",
        "2024-01-01T00:00:00Z",
        "中中中中中",
    ] {
        assert!(publication_date(Some(date)).is_err(), "{date}");
    }
    for date in ["2024-02-29", "2000-02-29", "1900-02-28"] {
        assert_eq!(publication_date(Some(date)).unwrap().as_deref(), Some(date));
    }
    assert!(publication_date(None).unwrap().is_none());
    assert!(publication_date(Some("")).unwrap().is_none());
    for count in ["-1", "1.5", "+2", "", " 2", "18446744073709551616"] {
        assert!(optional_count(Some(&FlexibleU64::String(count.to_owned()))).is_err());
    }
    assert_eq!(
        optional_count(Some(&FlexibleU64::String("12".to_owned()))).unwrap(),
        Some(12)
    );
    assert_eq!(optional_count(None).unwrap(), None);
    for (id, kind) in [("01", "2003"), ("77", "5")] {
        let value: AlbumMetadata =
            serde_json::from_value(json!({"resourceType":kind,"albumId":id,"title":"Album"}))
                .unwrap();
        assert!(map_album_metadata(value, "test").is_err());
    }
}

#[test]
fn album_images_allow_only_verified_official_paths_and_upgrade_file_service_https() {
    let suffix = format!("{}/{}/{}", "a".repeat(32), "b".repeat(32), "c".repeat(32));
    for prefix in ["file-down01", "file-down"] {
        let path = format!("/prod/file-service/{prefix}/{suffix}");
        let url = format!("http://d.musicapp.migu.cn{path}?signature=keep");
        assert_eq!(
            album_image_url(&url).unwrap(),
            url.replacen("http:", "https:", 1)
        );
        for url in [
            format!("http://evil.example{path}"),
            format!("http://user:pass@d.musicapp.migu.cn{path}"),
            format!("http://d.musicapp.migu.cn:1234{path}"),
            format!("http://d.musicapp.migu.cn{path}#fragment"),
            "http://d.musicapp.migu.cn/arbitrary".to_owned(),
            format!("https://d.musicapp.migu.cn/prod/file-service/{prefix}/short"),
        ] {
            assert!(album_image_url(&url).is_none(), "{url}");
        }
    }
    assert!(album_image_url("https://d.musicapp.migu.cn/data/oss/resource/cover.webp").is_some());
}
