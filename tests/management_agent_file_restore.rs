#![cfg(windows)]
#[path = "../src/management_agent/file_restore.rs"]
mod file_restore;
#[path = "../src/management_agent/install_plan.rs"]
mod install_plan;
#[path = "../src/management_agent/state_store.rs"]
mod state_store;
#[path = "../src/management_agent/transaction.rs"]
mod transaction;
#[path = "../src/management_agent/workspace.rs"]
mod workspace;

#[test]
fn installed_read_execute_acl_is_preserved_but_untrusted_write_is_refused() {
    use sha2::{Digest, Sha256};
    let root = std::env::temp_dir().join(format!("rustdesk-restore-readers-{}", std::process::id()));
    workspace::create(&root).unwrap();
    let target_dir = root.join("installed");
    std::fs::create_dir(&target_dir).unwrap();
    let target = target_dir.join("agent.exe");
    std::fs::write(&target, b"new").unwrap();
    std::fs::write(root.join("backup.exe"), b"old").unwrap();
    let backup = install_plan::Artifact {relative_path:"backup.exe".into(),size:3,sha256:hex::encode(Sha256::digest(b"old"))};
    let tool = std::path::PathBuf::from(std::env::var_os("SystemRoot").unwrap()).join("System32/icacls.exe");
    assert!(std::process::Command::new(&tool).arg(&target_dir).args(["/grant", "*S-1-5-32-545:(OI)(CI)RX"]).status().unwrap().success());
    assert!(workspace::verify_file(&target).is_err(), "backup directory policy stays strict");
    file_restore::restore(&root, &backup, &target).unwrap();
    assert_eq!(std::fs::read(&target).unwrap(), b"old");
    workspace::verify_restore_target(&target, false).unwrap();
    assert!(std::process::Command::new(&tool).arg(&target).args(["/grant", "*S-1-5-32-545:F"]).status().unwrap().success());
    let error = file_restore::restore(&root, &backup, &target).unwrap_err();
    assert!(error.contains("permission"));
    assert_eq!(std::fs::read(&target).unwrap(), b"old");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn verified_backup_restores_exact_bytes_and_preserves_destination_security() {
    use sha2::{Digest, Sha256};
    let root = std::env::temp_dir().join(format!("rustdesk-file-restore-{}", std::process::id()));
    workspace::create(&root).unwrap();
    let source = root.join("backup.exe");
    let destination = root.join("agent.exe");
    std::fs::write(&source, b"old trusted executable").unwrap();
    std::fs::write(&destination, b"new broken executable").unwrap();
    let original = install_plan::Artifact {
        relative_path: "backup.exe".into(),
        size: 22,
        sha256: hex::encode(Sha256::digest(b"old trusted executable")),
    };
    file_restore::restore(&root, &original, &destination).unwrap();
    assert_eq!(
        std::fs::read(&destination).unwrap(),
        b"old trusted executable"
    );
    workspace::verify_file(&destination).unwrap();
    let lock = std::fs::File::open(&destination).unwrap();
    drop(lock);
    std::fs::write(&source, b"changed").unwrap();
    assert!(file_restore::restore(&root, &original, &destination).is_err());
    assert_eq!(
        std::fs::read(&destination).unwrap(),
        b"old trusted executable"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn occupied_destination_refuses_replacement_and_keeps_previous_binary() {
    use sha2::{Digest, Sha256};
    use std::os::windows::fs::OpenOptionsExt;
    let root =
        std::env::temp_dir().join(format!("rustdesk-file-restore-lock-{}", std::process::id()));
    workspace::create(&root).unwrap();
    std::fs::write(root.join("backup.exe"), b"old").unwrap();
    let destination = root.join("agent.exe");
    std::fs::write(&destination, b"new").unwrap();
    let source = install_plan::Artifact {
        relative_path: "backup.exe".into(),
        size: 3,
        sha256: hex::encode(Sha256::digest(b"old")),
    };
    let lock = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(&destination)
        .unwrap();
    assert!(file_restore::restore(&root, &source, &destination).is_err());
    assert_eq!(std::fs::read(&destination).unwrap(), b"new");
    drop(lock);
    file_restore::restore(&root, &source, &destination).unwrap();
    assert_eq!(std::fs::read(&destination).unwrap(), b"old");
    std::fs::remove_dir_all(root).unwrap();
}
