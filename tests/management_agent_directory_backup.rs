#[path = "../src/management_agent/directory_backup.rs"]
mod directory_backup;

fn root(name: &str) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!(
        "rustdesk-directory-backup-{}-{name}",
        std::process::id()
    ));
    std::fs::create_dir(&path).unwrap();
    path
}

#[test]
fn complete_directory_snapshot_restores_files_subdirectories_and_removes_new_version_files() {
    let root = root("roundtrip");
    let original = root.join("client");
    let backup = root.join("backup");
    std::fs::create_dir_all(original.join("data/empty")).unwrap();
    std::fs::write(original.join("RustDesk.exe"), b"old executable").unwrap();
    std::fs::write(original.join("data/identity.config"), b"original config").unwrap();
    let snapshot = directory_backup::capture(&original, &backup).unwrap();
    snapshot.verify(&backup).unwrap();
    assert!(snapshot.matches_directory(&original).unwrap());
    std::fs::write(original.join("RustDesk.exe"), b"new executable").unwrap();
    std::fs::write(original.join("new.dll"), b"new dependency").unwrap();
    assert!(!snapshot.matches_directory(&original).unwrap());
    std::fs::remove_dir(original.join("data/empty")).unwrap();
    snapshot.restore(&backup, &original).unwrap();
    assert_eq!(
        std::fs::read(original.join("RustDesk.exe")).unwrap(),
        b"old executable"
    );
    assert_eq!(
        std::fs::read(original.join("data/identity.config")).unwrap(),
        b"original config"
    );
    assert!(original.join("data/empty").is_dir());
    assert!(!original.join("new.dll").exists());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn backup_drift_or_unsafe_manifest_does_not_touch_existing_client() {
    let root = root("drift");
    let original = root.join("client");
    let backup = root.join("backup");
    std::fs::create_dir(&original).unwrap();
    std::fs::write(original.join("RustDesk.exe"), b"old executable").unwrap();
    let snapshot = directory_backup::capture(&original, &backup).unwrap();
    std::fs::write(backup.join("files/RustDesk.exe"), b"changed backup").unwrap();
    assert!(snapshot
        .restore(&backup, &original)
        .unwrap_err()
        .contains("hash"));
    assert_eq!(
        std::fs::read(original.join("RustDesk.exe")).unwrap(),
        b"old executable"
    );
    let mut json = serde_json::to_value(&snapshot).unwrap();
    json["files"][0]["path"] = "../escape.exe".into();
    assert!(
        directory_backup::DirectoryBackup::decode(&serde_json::to_vec(&json).unwrap()).is_err()
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn existing_backup_and_nested_backup_are_refused_without_overwriting_data() {
    let root = root("exists");
    let original = root.join("client");
    std::fs::create_dir(&original).unwrap();
    std::fs::write(original.join("RustDesk.exe"), b"old").unwrap();
    let backup = root.join("backup");
    std::fs::create_dir(&backup).unwrap();
    std::fs::write(backup.join("foreign"), b"keep").unwrap();
    assert!(directory_backup::capture(&original, &backup).is_err());
    assert_eq!(std::fs::read(backup.join("foreign")).unwrap(), b"keep");
    assert!(directory_backup::capture(&original, &original.join("backup")).is_err());
    assert!(!original.join("backup").exists());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn extra_backup_files_and_missing_files_are_not_accepted_as_complete_assets() {
    let root = root("incomplete");
    let original = root.join("client");
    let backup = root.join("backup");
    std::fs::create_dir(&original).unwrap();
    std::fs::write(original.join("RustDesk.exe"), b"old").unwrap();
    let snapshot = directory_backup::capture(&original, &backup).unwrap();
    std::fs::write(backup.join("files/extra.dll"), b"unexpected").unwrap();
    assert!(snapshot.verify(&backup).is_err());
    std::fs::remove_file(backup.join("files/extra.dll")).unwrap();
    std::fs::remove_file(backup.join("files/RustDesk.exe")).unwrap();
    assert!(snapshot.verify(&backup).is_err());
    assert_eq!(
        std::fs::read(original.join("RustDesk.exe")).unwrap(),
        b"old"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[cfg(windows)]
#[test]
fn junction_in_source_or_backup_destination_is_refused_without_following_it() {
    let root = root("junction");
    let original = root.join("client");
    let outside = root.join("outside");
    std::fs::create_dir(&original).unwrap();
    std::fs::create_dir(&outside).unwrap();
    std::fs::write(outside.join("sentinel"), b"keep").unwrap();
    let command = std::env::var_os("SystemRoot")
        .map(std::path::PathBuf::from)
        .unwrap()
        .join("System32/WindowsPowerShell/v1.0/powershell.exe");
    let link = original.join("linked");
    let status = std::process::Command::new(command)
        .args(["-NoProfile","-Command","$ErrorActionPreference='Stop'; New-Item -ItemType Junction -Path $env:RDLAB_LINK -Target $env:RDLAB_TARGET | Out-Null"])
        .env("RDLAB_LINK",&link)
        .env("RDLAB_TARGET",&outside)
        .output()
        .unwrap();
    assert!(
        status.status.success(),
        "junction creation: {} {}",
        String::from_utf8_lossy(&status.stdout),
        String::from_utf8_lossy(&status.stderr)
    );
    assert!(directory_backup::capture(&original, &root.join("backup"))
        .unwrap_err()
        .contains("reparse"));
    assert_eq!(std::fs::read(outside.join("sentinel")).unwrap(), b"keep");
    std::fs::remove_dir(&link).unwrap();
    std::fs::remove_dir_all(root).unwrap();
}
