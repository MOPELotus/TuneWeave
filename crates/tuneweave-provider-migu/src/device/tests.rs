use super::*;

fn directory() -> PathBuf {
    let path = std::env::temp_dir().join(format!("tuneweave-migu-device-{}", generate().unwrap()));
    fs::create_dir(&path).unwrap();
    path
}

#[test]
fn device_is_lazy_stable_private_and_separate_from_other_instances() {
    let dir = directory();
    let path = dir.join("music.json");
    let store = MusicDeviceStore::new(Some(path.clone()));
    assert!(!path.exists());
    let id = store.identity().unwrap();
    assert!(valid_id(&id));
    assert_eq!(store.identity().unwrap(), id);
    assert_eq!(
        MusicDeviceStore::new(Some(path.clone()))
            .identity()
            .unwrap(),
        id
    );
    assert_ne!(MusicDeviceStore::default().identity().unwrap(), id);
    assert!(!format!("{store:?}").contains(&id));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn independent_first_initializations_publish_one_complete_identity() {
    let dir = directory();
    let path = dir.join("nested/music.json");
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(12));
    let tasks = (0..12)
        .map(|_| {
            let path = path.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                MusicDeviceStore::new(Some(path)).identity().unwrap()
            })
        })
        .collect::<Vec<_>>();
    let ids = tasks
        .into_iter()
        .map(|t| t.join().unwrap())
        .collect::<Vec<_>>();
    assert!(ids.iter().all(|id| id == &ids[0]));
    assert_eq!(read(&path).unwrap().unwrap(), ids[0]);
    assert_eq!(fs::read_dir(path.parent().unwrap()).unwrap().count(), 1);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn corrupt_or_unbounded_device_state_never_silently_changes_identity() {
    let dir = directory();
    let path = dir.join("music.json");
    let id = generate().unwrap();
    for body in [
        "{".into(),
        serde_json::json!({"version":2,"device_id":id}).to_string(),
        serde_json::json!({"version":1,"device_id":id.to_lowercase()}).to_string(),
        serde_json::json!({"version":1,"device_id":id,"extra":true}).to_string(),
        " ".repeat(4097),
    ] {
        fs::write(&path, &body).unwrap();
        assert!(
            MusicDeviceStore::new(Some(path.clone()))
                .identity()
                .is_err()
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), body);
    }
    assert!(!valid_id("00000000-0000-4000-7000-000000000000"));
    assert!(!valid_id("00000000-0000-1000-8000-000000000000"));
    fs::remove_dir_all(dir).unwrap();
}
