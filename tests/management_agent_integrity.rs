#[path = "../src/management_agent/integrity.rs"]
mod integrity;

use sha2::{Digest, Sha256};

#[test]
fn inventory_is_not_verified_without_an_approved_reference() {
    let root = std::env::temp_dir().join(format!("agent-integrity-inventory-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("client.exe");
    std::fs::write(&path, b"installed-client").unwrap();
    let report = integrity::inspect(&path, None, None);
    assert_eq!(report.status, "unverified");
    assert_eq!(report.sha256, Some(hex::encode(Sha256::digest(b"installed-client"))));
    assert_eq!(std::fs::read(&path).unwrap(), b"installed-client");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn trusted_digest_detects_damage_and_does_not_modify_files() {
    let root = std::env::temp_dir().join(format!("agent-integrity-damage-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("agent.exe");
    let digest = hex::encode(Sha256::digest(b"approved-agent"));
    std::fs::write(&path, b"approved-agent").unwrap();
    assert_eq!(integrity::inspect(&path, Some(&digest), Some(14)).status, "verified");
    std::fs::write(&path, b"modified-agent").unwrap();
    assert_eq!(integrity::inspect(&path, Some(&digest), Some(14)).status, "hash_mismatch");
    assert_eq!(integrity::inspect(&path, Some(&digest), Some(1)).status, "size_mismatch");
    assert_eq!(integrity::inspect(&path, Some("bad"), None).status, "invalid_reference");
    assert_eq!(std::fs::read(&path).unwrap(), b"modified-agent");
    std::fs::remove_file(&path).unwrap();
    assert_eq!(integrity::inspect(&path, Some(&digest), None).status, "missing");
    assert!(!path.exists());
    assert_eq!(integrity::inspect(&root, None, None).status, "not_regular_file");
    std::fs::remove_dir_all(root).unwrap();
}

#[cfg(windows)]
#[test]
fn unreadable_locked_file_is_reported_without_modification() {
    use std::os::windows::fs::OpenOptionsExt;
    let root = std::env::temp_dir().join(format!("agent-integrity-lock-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("client.exe");
    std::fs::write(&path, b"original").unwrap();
    let owner = std::fs::OpenOptions::new().read(true).write(true).share_mode(0).open(&path).unwrap();
    assert_eq!(integrity::inspect(&path, None, None).status, "read_failed");
    drop(owner);
    assert_eq!(std::fs::read(&path).unwrap(), b"original");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn relative_paths_are_refused_and_empty_file_hash_is_supported() {
    assert_eq!(integrity::inspect(std::path::Path::new("client.exe"), None, None).status, "unsafe_path");
    let root = std::env::temp_dir().join(format!("agent-integrity-empty-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("empty.exe");
    std::fs::write(&path, b"").unwrap();
    let hash = hex::encode(Sha256::digest(b""));
    assert_eq!(integrity::inspect(&path, Some(&hash), Some(0)).status, "verified");
    assert_eq!(integrity::inspect(&path, None, Some(0)).status, "invalid_reference");
    assert_eq!(std::fs::metadata(&path).unwrap().len(), 0);
    std::fs::remove_dir_all(root).unwrap();
}

#[cfg(windows)]
#[test]
fn junction_ancestor_is_refused_without_reading_the_target() {
    let root = std::env::temp_dir().join(format!("agent-integrity-junction-{}", std::process::id()));
    let real = root.join("real");
    let link = root.join("link");
    std::fs::create_dir_all(&real).unwrap();
    std::fs::write(real.join("client.exe"), b"keep-original").unwrap();
    let status = std::process::Command::new("cmd.exe")
        .args(["/d", "/c", "mklink", "/J"])
        .arg(&link).arg(&real).status().unwrap();
    assert!(status.success(), "junction fixture must actually be created");
    let report = integrity::inspect(&link.join("client.exe"), None, None);
    assert_eq!(report.status, "unsafe_path");
    assert_eq!(report.sha256, None);
    assert_eq!(std::fs::read(real.join("client.exe")).unwrap(), b"keep-original");
    std::fs::remove_dir(link).unwrap();
    std::fs::remove_dir_all(root).unwrap();
}
