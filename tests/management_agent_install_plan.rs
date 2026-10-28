// install_plan.rs 引用 `super::system_dirs`，该模块在生产 crate 里只在 windows 下编译。
#![cfg(windows)]
#[path = "../src/platform/windows_system_dirs.rs"]
#[allow(dead_code)]
mod system_dirs;
#[path = "../src/management_agent/install_plan.rs"]
mod install_plan;
#[cfg(windows)]
#[path = "../src/management_agent/state_store.rs"]
mod state_store;
#[path = "../src/management_agent/transaction.rs"]
mod transaction;

use install_plan::{Artifact, InstallPlan, OriginalService, Trigger};

fn fixture() -> InstallPlan {
    InstallPlan {
        schema: 1,
        transaction_id: "a".repeat(32),
        trigger: Trigger::LocalRecovery,
        command_id: None,
        installation_id: "b".repeat(32),
        identity_fingerprint: "c".repeat(64),
        target_key: "windows-x86_64-msi-standard".into(),
        original_version: "1.5.3".into(),
        original_build_seq: 2026100502,
        target_version: "1.5.3".into(),
        target_build_seq: 2026101101,
        normal_install_path: r"C:\Program Files\RustDesk".into(),
        package: Artifact { relative_path: "package.msi".into(), size: 1024, sha256: "d".repeat(64) },
        client_backup: Artifact { relative_path: "backups/client.bin".into(), size: 2048, sha256: "e".repeat(64) },
        agent_backup: Artifact { relative_path: "backups/agent.exe".into(), size: 512, sha256: "f".repeat(64) },
        identity_backup: Artifact { relative_path: "backups/identity.toml".into(), size: 256, sha256: "1".repeat(64) },
        helper: Artifact { relative_path: "runtime/helper.exe".into(), size: 4096, sha256: "2".repeat(64) },
        helper_version: "1.5.3".into(),
        resume_task: "RustDeskManagementAgentResume-".to_owned() + &"a".repeat(32),
        created_at: 1000,
        services: vec![
            OriginalService { name: "RustDesk".into(), command: r#""C:\Program Files\RustDesk\RustDesk.exe" --service"#.into(), account: "LocalSystem".into(), start_type: "auto".into(), running: true },
            OriginalService { name: "RustDeskManagementAgent".into(), command: r#""C:\ProgramData\RustDesk\ManagementAgent\rustdesk-management-agent.exe" --service --edition standard --package-kind msi --normal-install-path "C:\Program Files\RustDesk""#.into(), account: "LocalSystem".into(), start_type: "auto".into(), running: true },
        ],
    }
}

#[test]
fn plan_roundtrip_binds_package_identity_backups_and_resume_contract() {
    let plan = fixture();
    plan.validate().unwrap();
    let bytes = serde_json::to_vec(&plan).unwrap();
    assert_eq!(InstallPlan::decode(&bytes).unwrap(), plan);
    let mut remote = plan.clone();
    remote.trigger = Trigger::RemoteCommand;
    assert!(remote.validate().unwrap_err().contains("command"));
    remote.command_id = Some("3".repeat(32));
    remote.validate().unwrap();
}

#[test]
fn every_missing_or_unsafe_preparation_reference_is_refused_without_mutation() {
    for index in 0..14 {
        let mut plan = fixture();
        match index {
            0 => plan.package.size = 0,
            1 => plan.package.sha256.clear(),
            2 => plan.client_backup.relative_path = "../escape".into(),
            3 => plan.agent_backup.relative_path = r"C:\outside.exe".into(),
            4 => plan.identity_backup.relative_path = "backups/id:secret".into(),
            5 => plan.helper.relative_path = "runtime/../helper.exe".into(),
            6 => plan.resume_task = "another-task".into(),
            7 => plan.identity_fingerprint.clear(),
            8 => plan.target_key = "windows-x86_64-msi-sos".into(),
            9 => plan.package.relative_path = "package.exe".into(),
            10 => plan.target_build_seq = plan.original_build_seq - 1,
            11 => plan.services[0].account = "another-user".into(),
            12 => plan.services[1].command.push_str(" --extra"),
            _ => plan.services.push(plan.services[0].clone()),
        }
        let before = plan.clone();
        assert!(plan.validate().is_err(), "index={index}");
        assert_eq!(plan, before);
    }
}

#[test]
fn schema_unknown_fields_and_oversized_records_are_not_silently_accepted() {
    let mut value = serde_json::to_value(fixture()).unwrap();
    value["schema"] = 2.into();
    assert!(InstallPlan::decode(&serde_json::to_vec(&value).unwrap()).is_err());
    value["schema"] = 1.into();
    value["unsafe_command"] = "powershell.exe".into();
    assert!(InstallPlan::decode(&serde_json::to_vec(&value).unwrap()).is_err());
    assert!(InstallPlan::decode(&vec![b' '; 65537]).is_err());
}

#[test]
fn runtime_files_are_verified_by_actual_size_hash_and_no_reparse_paths() {
    let root = std::env::temp_dir().join(format!("rustdesk-install-plan-{}", std::process::id()));
    std::fs::create_dir(&root).unwrap();
    let bytes = b"trusted prepared file";
    std::fs::write(root.join("helper.exe"), bytes).unwrap();
    use sha2::{Digest, Sha256};
    let artifact = Artifact {
        relative_path: "helper.exe".into(),
        size: bytes.len() as u64,
        sha256: hex::encode(Sha256::digest(bytes)),
    };
    artifact.verify(&root).unwrap();
    std::fs::write(root.join("helper.exe"), b"changed").unwrap();
    assert!(artifact.verify(&root).unwrap_err().contains("size"));
    std::fs::write(root.join("helper.exe"), vec![b'x'; bytes.len()]).unwrap();
    assert!(artifact.verify(&root).unwrap_err().contains("hash"));
    assert!(artifact.verify(&root.join("missing")).is_err());
    assert_eq!(
        std::fs::read(root.join("helper.exe")).unwrap(),
        vec![b'x'; bytes.len()]
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn preparation_binding_rejects_another_transaction_target_package_or_identity() {
    let plan = fixture();
    let current = transaction::Transaction::new(
        plan.transaction_id.clone(),
        plan.target_key.clone(),
        plan.package.sha256.clone(),
        plan.identity_fingerprint.clone(),
        plan.created_at,
    )
    .unwrap();
    plan.bind_transaction(&current).unwrap();
    for index in 0..5 {
        let mut changed = current.clone();
        match index {
            0 => changed.id = "9".repeat(32),
            1 => changed.target_key = "windows-x86_64-exe-standard".into(),
            2 => changed.package_sha256 = "8".repeat(64),
            3 => changed.identity_fingerprint = "7".repeat(64),
            _ => changed.created_at += 1,
        }
        assert!(plan.bind_transaction(&changed).is_err(), "index={index}");
    }
}

#[cfg(windows)]
#[test]
fn protected_preparation_is_write_once_and_corruption_never_gets_reapproved() {
    let root = std::env::temp_dir().join(format!(
        "rustdesk-install-plan-persist-{}",
        std::process::id()
    ));
    std::fs::create_dir(&root).unwrap();
    let plan = fixture();
    let current = transaction::Transaction::new(
        plan.transaction_id.clone(),
        plan.target_key.clone(),
        plan.package.sha256.clone(),
        plan.identity_fingerprint.clone(),
        plan.created_at,
    )
    .unwrap();
    let path = root.join("prepared.json");
    plan.save_bound(&path, &current).unwrap();
    let original = std::fs::read(&path).unwrap();
    plan.save_bound(&path, &current).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), original);
    let mut changed = plan.clone();
    changed.helper.sha256 = "6".repeat(64);
    assert!(changed
        .save_bound(&path, &current)
        .unwrap_err()
        .contains("immutable"));
    assert_eq!(std::fs::read(&path).unwrap(), original);
    std::fs::write(&path, b"corrupt preparation").unwrap();
    assert!(plan.save_bound(&path, &current).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), b"corrupt preparation");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn all_four_package_targets_and_duplicate_asset_paths_are_checked() {
    for kind in ["msi", "exe"] {
        for edition in ["standard", "sos"] {
            let mut plan = fixture();
            plan.target_key = format!("windows-x86_64-{kind}-{edition}");
            plan.package.relative_path = format!("package.{kind}");
            plan.services[1].command = format!("\"C:\\ProgramData\\RustDesk\\ManagementAgent\\rustdesk-management-agent.exe\" --service --edition {edition} --package-kind {kind} --normal-install-path \"C:\\Program Files\\RustDesk\"");
            plan.validate().unwrap();
            plan.helper.relative_path = plan.agent_backup.relative_path.clone();
            assert!(plan.validate().unwrap_err().contains("overlap"));
        }
    }
}

#[test]
fn every_prepared_asset_must_match_before_helper_can_be_considered_ready() {
    use sha2::{Digest, Sha256};
    let root = std::env::temp_dir().join(format!(
        "rustdesk-install-plan-assets-{}",
        std::process::id()
    ));
    std::fs::create_dir(&root).unwrap();
    let mut plan = fixture();
    for artifact in [
        &mut plan.package,
        &mut plan.client_backup,
        &mut plan.agent_backup,
        &mut plan.identity_backup,
        &mut plan.helper,
    ] {
        let path = root.join(&artifact.relative_path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let bytes = artifact.relative_path.as_bytes();
        std::fs::write(&path, bytes).unwrap();
        artifact.size = bytes.len() as u64;
        artifact.sha256 = hex::encode(Sha256::digest(bytes));
    }
    plan.verify_assets(&root).unwrap();
    for artifact in [
        &plan.package,
        &plan.client_backup,
        &plan.agent_backup,
        &plan.identity_backup,
        &plan.helper,
    ] {
        let path = root.join(&artifact.relative_path);
        let previous = std::fs::read(&path).unwrap();
        std::fs::write(&path, vec![b'x'; previous.len()]).unwrap();
        assert!(plan.verify_assets(&root).unwrap_err().contains("hash"));
        assert_eq!(std::fs::read(&path).unwrap(), vec![b'x'; previous.len()]);
        std::fs::write(&path, previous).unwrap();
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[cfg(windows)]
#[test]
fn stop_permission_requires_matching_persisted_transaction_and_complete_preparation() {
    use sha2::{Digest, Sha256};
    let root =
        std::env::temp_dir().join(format!("rustdesk-install-plan-stop-{}", std::process::id()));
    std::fs::create_dir(&root).unwrap();
    let mut plan = fixture();
    for artifact in [
        &mut plan.package,
        &mut plan.client_backup,
        &mut plan.agent_backup,
        &mut plan.identity_backup,
        &mut plan.helper,
    ] {
        let path = root.join(&artifact.relative_path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let bytes = artifact.relative_path.as_bytes();
        std::fs::write(&path, bytes).unwrap();
        artifact.size = bytes.len() as u64;
        artifact.sha256 = hex::encode(Sha256::digest(bytes));
    }
    let tx_path = root.join("transaction.json");
    let plan_path = root.join("prepared.json");
    let mut tx = transaction::Transaction::new(
        plan.transaction_id.clone(),
        plan.target_key.clone(),
        plan.package.sha256.clone(),
        plan.identity_fingerprint.clone(),
        plan.created_at,
    )
    .unwrap();
    let evidence = transaction::Evidence {
        package_verified: true,
        backups_verified: true,
        helper_verified: true,
        resume_task_verified: true,
        handoff_verified: true,
        maintenance_owned: true,
        services_stopped: false,
        target_verified: false,
        identity_unchanged: false,
        client_ready: false,
        agent_ready: false,
        rollback_verified: false,
        locally_blocked: false,
    };
    tx.save(&tx_path).unwrap();
    plan.save_bound(&plan_path, &tx).unwrap();
    for phase in [
        transaction::Phase::PackageVerified,
        transaction::Phase::Prepared,
    ] {
        tx.advance(phase, &evidence, 1001).unwrap();
        tx.save(&tx_path).unwrap();
    }
    assert!(plan
        .authorize_stop(&root, &plan_path, &tx_path, &tx, &evidence)
        .is_err());
    tx.advance(transaction::Phase::StoppingServices, &evidence, 1002)
        .unwrap();
    assert!(plan
        .authorize_stop(&root, &plan_path, &tx_path, &tx, &evidence)
        .is_err());
    tx.save(&tx_path).unwrap();
    plan.authorize_stop(&root, &plan_path, &tx_path, &tx, &evidence)
        .unwrap();
    let previous = std::fs::read(&plan_path).unwrap();
    std::fs::write(&plan_path, b"damaged preparation").unwrap();
    assert!(plan
        .authorize_stop(&root, &plan_path, &tx_path, &tx, &evidence)
        .is_err());
    assert_eq!(std::fs::read(&plan_path).unwrap(), b"damaged preparation");
    std::fs::write(&plan_path, previous).unwrap();
    let helper = root.join(&plan.helper.relative_path);
    std::fs::write(&helper, b"damaged helper").unwrap();
    assert!(plan
        .authorize_stop(&root, &plan_path, &tx_path, &tx, &evidence)
        .is_err());
    assert_eq!(
        transaction::Transaction::decode(&std::fs::read(&tx_path).unwrap()).unwrap(),
        tx
    );
    std::fs::remove_dir_all(root).unwrap();
}
