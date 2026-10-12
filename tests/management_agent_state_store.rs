#[path = "../src/management_agent/state_store.rs"]
mod state_store;

#[cfg(windows)]
mod windows_tests {
    use super::state_store;
    use std::{fs, os::windows::fs::OpenOptionsExt};

    fn fixture() -> std::path::PathBuf {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "rustdesk-state-store-{}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&path).unwrap();
        path
    }

    #[test]
    fn repeated_atomic_publication_preserves_identity_and_hidden_destination() {
        use std::os::windows::fs::MetadataExt;
        let root = fixture();
        let path = root.join("agent.json");
        for generation in 1..=5 {
            let bytes = format!(r#"{{"installation_id":"original","generation":{generation}}}"#);
            state_store::publish(&path, bytes.as_bytes()).unwrap();
            assert_eq!(fs::read(&path).unwrap(), bytes.as_bytes());
            assert_eq!(fs::metadata(&path).unwrap().file_attributes() & 6, 6);
        }
        assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn concurrent_readers_never_observe_missing_or_partial_identity() {
        use std::sync::{
            atomic::{AtomicBool, Ordering},
            Arc,
        };
        let root = fixture();
        let path = root.join("agent.json");
        let before = br#"{"identity":"original","generation":0}"#;
        let after = br#"{"identity":"original","generation":1}"#;
        state_store::publish(&path, before).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let reader_stop = stop.clone();
        let reader_path = path.clone();
        let reader = std::thread::spawn(move || {
            let mut reads = 0;
            while !reader_stop.load(Ordering::SeqCst) || reads == 0 {
                let bytes = match fs::read(&reader_path) {
                    Ok(bytes) => bytes,
                    Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => continue,
                    Err(error) => panic!("committed state unavailable: {error}"),
                };
                assert!(
                    bytes == before || bytes == after,
                    "partial state: {bytes:?}"
                );
                reads += 1;
            }
            reads
        });
        for i in 0..200 {
            if let Err(error) = state_store::publish(&path, if i % 2 == 0 { after } else { before })
            {
                assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
                assert!(
                    path.exists(),
                    "locked publication must preserve committed state"
                );
            }
        }
        stop.store(true, Ordering::SeqCst);
        assert!(reader.join().unwrap() > 0);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn locked_destination_reports_failure_without_losing_previous_or_staging() {
        let root = fixture();
        let path = root.join("agent.json");
        state_store::publish(&path, b"old identity").unwrap();
        let lock = fs::OpenOptions::new()
            .read(true)
            .share_mode(1)
            .open(&path)
            .unwrap();
        let error = state_store::publish(&path, b"new state").unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
        assert_eq!(fs::read(&path).unwrap(), b"old identity");
        assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
        drop(lock);
        state_store::publish(&path, b"retry state").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"retry state");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn uncommitted_existing_staging_is_not_overwritten_or_promoted() {
        let root = fixture();
        let path = root.join("agent.json");
        state_store::publish(&path, b"committed identity").unwrap();
        let pending = path.with_extension(format!("tmp-{}", std::process::id()));
        fs::write(&pending, b"uncommitted foreign state").unwrap();
        let error = state_store::publish(&path, b"next state").unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read(&path).unwrap(), b"committed identity");
        assert_eq!(fs::read(&pending).unwrap(), b"uncommitted foreign state");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn killed_writer_keeps_a_complete_committed_identity_without_promoting_staging() {
        use std::{
            process::{Command, Stdio},
            time::{Duration, Instant},
        };
        let first = br#"{"identity":"original","generation":0}"#;
        let second = br#"{"identity":"original","generation":1}"#;
        if let Some(root) = std::env::var_os("RUSTDESK_STATE_CRASH_FIXTURE") {
            let root = std::path::PathBuf::from(root);
            assert!(root.starts_with(std::env::temp_dir()));
            assert!(root
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("rustdesk-state-store-"));
            let path = root.join("agent.json");
            loop {
                state_store::publish(&path, first).unwrap();
                state_store::publish(&path, second).unwrap();
                fs::write(root.join("writer-ready"), b"published").unwrap();
            }
        }
        let root = fixture();
        let path = root.join("agent.json");
        state_store::publish(&path, first).unwrap();
        for _ in 0..8 {
            let ready = root.join("writer-ready");
            if ready.exists() {
                fs::remove_file(&ready).unwrap();
            }
            let mut child = Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "windows_tests::killed_writer_keeps_a_complete_committed_identity_without_promoting_staging",
                    "--nocapture",
                ])
                .env("RUSTDESK_STATE_CRASH_FIXTURE", &root)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap();
            let deadline = Instant::now() + Duration::from_secs(10);
            while !ready.exists() && Instant::now() < deadline {
                assert!(
                    child.try_wait().unwrap().is_none(),
                    "writer exited before evidence"
                );
                std::thread::sleep(Duration::from_millis(5));
            }
            if !ready.exists() {
                child.kill().unwrap();
                child.wait().unwrap();
                panic!("writer did not publish");
            }
            child.kill().unwrap();
            assert!(!child.wait().unwrap().success());
            let saved = fs::read(&path).expect("process death must not remove committed state");
            assert!(
                saved == first || saved == second,
                "partial committed identity: {saved:?}"
            );
            let orphan = path.with_extension(format!("tmp-{}", child.id()));
            if orphan.exists() {
                let before = fs::read(&path).unwrap();
                assert_eq!(before, saved, "orphan staging was promoted");
                fs::remove_file(&orphan).unwrap();
            }
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn invalid_parent_and_destination_preserve_old_data() {
        let root = fixture();
        let path = root.join("agent.json");
        state_store::publish(&path, b"old").unwrap();
        let blocked = root.join("not-a-directory");
        fs::write(&blocked, b"unrelated").unwrap();
        assert!(state_store::publish(&blocked.join("agent.json"), b"new").is_err());
        assert_eq!(fs::read(&path).unwrap(), b"old");
        assert_eq!(fs::read(&blocked).unwrap(), b"unrelated");
        let directory = root.join("directory.json");
        fs::create_dir(&directory).unwrap();
        assert!(state_store::publish(&directory, b"new").is_err());
        assert!(directory.is_dir());
        fs::remove_dir_all(root).unwrap();
    }
}
