#![cfg(windows)]
#[path = "../src/management_agent/agent_restore.rs"]
mod agent_restore;
#[path = "../src/management_agent/bootstrap_update.rs"]
mod bootstrap_update;
#[path = "../src/management_agent/directory_backup.rs"]
mod directory_backup;
#[path = "../src/management_agent/file_restore.rs"]
mod file_restore;
#[path = "../src/management_agent/handoff.rs"]
mod handoff;
#[path = "../src/management_agent/health.rs"]
mod health;
#[path = "../src/management_agent/health_windows.rs"]
mod health_windows;
#[path = "../src/management_agent/helper_runtime.rs"]
mod helper_runtime;
#[path = "../src/management_agent/identity.rs"]
mod identity;
#[path = "../src/management_agent/install_execution.rs"]
mod install_execution;
#[path = "../src/platform/windows_agent_guard.rs"]
mod install_guard;
#[path = "../src/management_agent/install_plan.rs"]
mod install_plan;
#[path = "../src/management_agent/installer_process.rs"]
mod installer_process;
#[path = "../src/management_agent/local_events.rs"]
mod local_events;
#[path = "../src/management_agent/local_recovery_events.rs"]
mod local_recovery_events;
#[path = "../src/management_agent/local_repair.rs"]
mod local_repair;
#[path = "../src/management_agent/local_repair_plan.rs"]
mod local_repair_plan;
#[path = "../src/management_agent/local_repair_schedule.rs"]
mod local_repair_schedule;
#[path = "../src/management_agent/maintenance.rs"]
mod maintenance;
#[path = "../src/management_agent/msi_assets.rs"]
mod msi_assets;
#[path = "../src/management_agent/msi_metadata.rs"]
mod msi_metadata;
#[path = "../src/management_agent/native_install.rs"]
mod native_install;
#[path = "../src/management_agent/package_gate.rs"]
mod package_gate;
#[path = "../src/management_agent/purge_approval.rs"]
mod purge_approval;
#[path = "../src/management_agent/purge_discovery.rs"]
mod purge_discovery;
#[path = "../src/management_agent/purge_policy.rs"]
mod purge_policy;
#[path = "../src/management_agent/purge_preflight.rs"]
mod purge_preflight;
#[path = "../src/management_agent/purge_service.rs"]
mod purge_service;
#[path = "../src/management_agent/purge_handoff.rs"]
mod purge_handoff;
#[path = "../src/management_agent/parent_handoff.rs"]
mod parent_handoff;
#[path = "../src/management_agent/preparation.rs"]
mod preparation;
#[path = "../src/management_agent/recovery.rs"]
mod recovery;
#[path = "../src/management_agent/rescue_assets.rs"]
mod rescue_assets;
#[path = "../src/management_agent/rescue_recovery.rs"]
mod rescue_recovery;
#[path = "../src/management_agent/rescue_status.rs"]
mod rescue_status;
#[path = "../src/management_agent/resume_task.rs"]
mod resume_task;
#[path = "../src/management_agent/service_control.rs"]
mod service_control;
#[path = "../src/management_agent/state_store.rs"]
mod state_store;
#[path = "../src/management_agent/transaction.rs"]
mod transaction;
#[path = "../src/management_agent/workspace.rs"]
mod workspace;

use hbb_common::{
    base64::{engine::general_purpose::STANDARD, Engine as _},
    sodiumoxide::crypto::sign,
    UpdateManifest, UpdateTarget,
};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

struct Fixture {
    root: PathBuf,
    public: sign::PublicKey,
}

impl Fixture {
    fn new() -> Self {
        assert!(matches!(
            std::env::var("COMPUTERNAME").unwrap().as_str(),
            "RDLAB-WIN10" | "RDLAB-WIN11"
        ));
        let root =
            Path::new(r"C:\RustDeskAgentLab").join(format!("rescue-v200-{}", uuid::Uuid::new_v4()));
        workspace::create(&root).unwrap();
        for name in ["state", "transactions", "current", "bootstrap"] {
            workspace::create(&root.join(name)).unwrap();
        }
        let tx = root.join("transactions").join("a".repeat(32));
        workspace::create(&tx).unwrap();
        hbb_common::sodiumoxide::init().unwrap();
        let (public, secret) = sign::gen_keypair();
        let package = Path::new(r"C:\RustDeskAgentLab\agent-sos-v187.msi");
        let bytes = std::fs::read(package).unwrap();
        assert_eq!(
            hex::encode(Sha256::digest(&bytes)),
            "b912548e12248e8950abae312afba156ac4b9aab7ee07b8c0a72b988855813a9"
        );
        std::fs::write(tx.join("agent-sos-v187.msi"), &bytes).unwrap();
        let key = "windows-x86_64-msi-sos";
        let mut manifest = UpdateManifest {
            schema: 2,
            product: "rustdesk-yan".into(),
            edition: "multi".into(),
            channel: "stable".into(),
            ..Default::default()
        };
        let mut target = UpdateTarget {
            version: "1.5.3".into(),
            build_seq: 2026100502,
            source_commit: "fixture".into(),
            primary: "https://download.yan.life/rustdesk/stable/1.5.3/agent-sos-v187.msi".into(),
            size: bytes.len() as u64,
            sha256: hex::encode(Sha256::digest(&bytes)),
            signature_key_id: "fixture-key".into(),
            ..Default::default()
        };
        target.signature = STANDARD.encode(sign::sign_detached(
            &base::update::update_signature_payload(&manifest, key, &target).unwrap(),
            &secret,
        ));
        manifest.targets.insert(key.into(), target.clone());
        save(&tx.join("approved-manifest.json"), &manifest);
        let extracted = msi_assets::extract_fixture(
            &tx,
            &tx.join("agent-sos-v187.msi"),
            &manifest,
            key,
            public.as_ref(),
        )
        .unwrap();
        directory_backup::capture(&extracted.client, &root.join("current/client")).unwrap();
        std::fs::copy(&extracted.agent, root.join("current/agent.exe")).unwrap();
        std::fs::copy(
            &extracted.agent,
            root.join("bootstrap/rustdesk-recovery-bootstrap.exe"),
        )
        .unwrap();
        let agent_hash = hex::encode(Sha256::digest(std::fs::read(&extracted.agent).unwrap()));
        save(
            &root.join("bootstrap/bootstrap.json"),
            &serde_json::json!({"schema":1,"agent_sha256":agent_hash,"edition":"sos","package_kind":"msi","normal_install_path":r"C:\Program Files\RustDesk"}),
        );
        let (identity_public, identity_secret) = sign::gen_keypair();
        let identity = hbb_common::toml::to_string(&serde_json::json!({"id":"fixture-original-id", "key_pair":[identity_secret.as_ref(),identity_public.as_ref()]})).unwrap();
        std::fs::write(root.join("current/identity.toml"), &identity).unwrap();
        std::fs::write(
            root.join("current/RustDesk2.toml"),
            "[options]\nstop-service = 'N'\n",
        )
        .unwrap();
        std::fs::write(tx.join("identity.toml"), &identity).unwrap();
        let artifact = |name: &str, data: &[u8]| install_plan::Artifact {
            relative_path: name.into(),
            size: data.len() as u64,
            sha256: hex::encode(Sha256::digest(data)),
        };
        let identity_ref = artifact("identity.toml", identity.as_bytes());
        let plan = install_plan::InstallPlan {schema:1,transaction_id:"a".repeat(32),trigger:install_plan::Trigger::RemoteCommand,command_id:Some("b".repeat(32)),installation_id:"c".repeat(32),identity_fingerprint:identity_ref.sha256.clone(),target_key:key.into(),original_version:"1.5.3".into(),original_build_seq:2026100502,target_version:"1.5.3".into(),target_build_seq:2026100502,normal_install_path:r"C:\Program Files\RustDesk".into(),package:artifact("agent-sos-v187.msi",&bytes),client_backup:artifact("old-client.json",b"old"),agent_backup:artifact("old-agent.exe",b"old"),identity_backup:identity_ref,helper:artifact("helper.exe",b"old"),helper_version:"1.5.3".into(),resume_task:format!("RustDeskManagementAgentResume-{}","a".repeat(32)),created_at:100,services:vec![install_plan::OriginalService {name:"RustDesk".into(),command:r#""C:\Program Files\RustDesk\RustDesk.exe" --service"#.into(),account:"LocalSystem".into(),start_type:"auto".into(),running:true},install_plan::OriginalService {name:"RustDeskManagementAgent".into(),command:r#""C:\ProgramData\RustDesk\ManagementAgent\rustdesk-management-agent.exe" --service --edition sos --package-kind msi --normal-install-path "C:\Program Files\RustDesk""#.into(),account:"LocalSystem".into(),start_type:"auto".into(),running:true}]};
        plan.validate().unwrap();
        save(&tx.join("prepared.json"), &plan);
        let mut transaction = transaction::Transaction::new(
            plan.transaction_id.clone(),
            key.into(),
            target.sha256.clone(),
            plan.identity_fingerprint.clone(),
            100,
        )
        .unwrap();
        transaction.phase = transaction::Phase::Committed;
        transaction.updated_at = 101;
        save(&tx.join("transaction.json"), &transaction);
        save(
            &tx.join("execution-result.json"),
            &serde_json::json!({"schema":1,"transaction_id":plan.transaction_id,"command_id":plan.command_id,"phase":"committed","reboot_required":false,"error":null}),
        );
        save(
            &root.join("state/agent.json"),
            &serde_json::json!({"agent_installation_id":plan.installation_id,"target_key":key,"management_agent_enabled":true,"disabled":false}),
        );
        std::fs::write(root.join("state/service-recovery-enabled"), b"enabled").unwrap();
        Self { root, public }
    }
    fn prepare(&self) -> Result<serde_json::Value, String> {
        rescue_assets::prepare_fixture(&self.root, self.public.as_ref())
    }
    fn verify(&self) -> Result<serde_json::Value, String> {
        rescue_assets::verify_fixture(&self.root, self.public.as_ref())
    }
    fn tx(&self) -> PathBuf {
        self.root.join("transactions").join("a".repeat(32))
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.root).unwrap();
    }
}
fn save(path: &Path, value: &impl serde::Serialize) {
    std::fs::write(path, serde_json::to_vec(value).unwrap()).unwrap();
}

#[test]
fn purge_approval_signed_files_preserve_stop_markers_and_refuse_foreign_active_bad_signature() {
    purge_approval_case("baseline");
}

#[test]
fn purge_approval_tampered_signature_trusted_key_preserves_data() { purge_approval_case("signature"); }
#[test]
fn purge_approval_package_size_mismatch_preserves_data() { purge_approval_case("size"); }
#[test]
fn purge_approval_package_hash_mismatch_preserves_data() { purge_approval_case("hash"); }
#[test]
fn purge_approval_fixed_profile_selection_foreign_and_ambiguous() { purge_approval_case("profiles"); }

#[test]
fn purge_service_stopped_registration_does_not_override_live_process() {
    assert_eq!(purge_service::wait_stopped(std::time::Duration::ZERO, || Ok((true,true))).unwrap_err(), "purge service process exit not confirmed");
    assert_eq!(purge_service::wait_stopped(std::time::Duration::ZERO, || Err("process observation unavailable".into())).unwrap_err(), "process observation unavailable");
    purge_service::wait_stopped(std::time::Duration::ZERO, || Ok((true,false))).unwrap();
}

#[test]
fn purge_task_maintenance_after_wait_blocks_delete_and_preserves_fixture() {
    let root = Path::new(r"C:\RustDeskAgentLab").join(format!("purge-task-gate-{}",uuid::Uuid::new_v4()));
    workspace::create(&root).unwrap();
    let marker = root.join("maintenance.json");
    let task = root.join("fixed-task.xml");
    std::fs::write(&task,b"original task registration").unwrap();
    let context = purge_preflight::acquire_fixture(&marker).unwrap();
    context.recheck().unwrap();
    std::fs::write(&marker,b"maintenance arrived during stop wait").unwrap();
    assert_eq!(context.recheck().unwrap_err(),"purge execution maintenance present");
    assert_eq!(std::fs::read(&task).unwrap(),b"original task registration");
    assert_eq!(std::fs::read(&marker).unwrap(),b"maintenance arrived during stop wait");
    drop(context);std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn purge_approval_disabled_stop_pair_keeps_shared_recovery_refusal() { purge_approval_case("intent-pair"); }

#[test]
fn purge_approval_preflight_child() {
    if std::env::var_os("PURGE_PREFLIGHT_CHILD").is_none() { return; }
    assert_eq!(purge_preflight::acquire().err().unwrap(), "purge execution busy");
}

#[test]
fn purge_handoff_child() {
    let Some(root) = std::env::var_os("PURGE_HANDOFF_FIXTURE") else { return; };
    let id = std::env::var("PURGE_HANDOFF_ID").unwrap();
    let mode = std::env::var("PURGE_HANDOFF_MODE").unwrap();
    if mode == "publisher" || mode == "publisher-error" {
        let directory = purge_handoff::prepare_fixture(Path::new(&root), &id).unwrap();
        std::fs::write(directory.join("fixture-data"), b"identity source state").unwrap();
        std::fs::write(directory.join("fixture-registration"), b"Absent").unwrap();
        for _ in 0..if mode == "publisher" { 2 } else { 1 } { std::process::Command::new(directory.join("purge-helper.exe"))
            .args(["--exact", "purge_handoff_child", "--nocapture"])
            .env("PURGE_HANDOFF_MODE", if mode == "publisher" { "complete" } else { "error" })
            .stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null())
            .spawn().unwrap(); }
        println!("Pending");
        return;
    }
    let result = purge_handoff::open_fixture(Path::new(&root), &id);
    if mode == "hash" {
        assert_eq!(result.err().unwrap(), "purge helper hash mismatch");
    } else if mode == "foreign" {
        assert_eq!(result.err().unwrap(), "purge handoff plan identity mismatch");
    } else if mode == "error" {
        let mut checked = result.unwrap();
        checked.wait_parent(std::time::Duration::from_secs(10)).unwrap();
        assert_eq!(checked.execute_once(|| Err("fixture installer wait failed: original detail".into())).unwrap_err(), "fixture installer wait failed: original detail");
        let receipt_path = Path::new(&root).join(&id).join("result.json");
        let before = std::fs::read(&receipt_path).unwrap();
        let receipt: serde_json::Value = serde_json::from_slice(&before).unwrap();
        assert_eq!(receipt["error"], "fixture installer wait failed: original detail");
        assert!(checked.execute_once(|| panic!("failed terminal cannot execute again")).is_err());
        assert_eq!(std::fs::read(receipt_path).unwrap(), before);
        std::fs::write(Path::new(&root).join(&id).join("error-checked"), b"checked").unwrap();
    } else if mode == "serial" {
        let mut checked = result.unwrap();
        assert_eq!(checked.execute_once(|| panic!("duplicate side effect")).unwrap_err(), "purge handoff terminal exists");
    } else if mode == "complete" {
        let mut checked = result.unwrap();
        if let Err(error) = checked.pending() {
            assert_eq!(error, "purge handoff terminal exists");
            std::fs::write(Path::new(&root).join(&id).join(format!("child-{}.done",std::process::id())), b"refused").unwrap();
            return;
        }
        checked.wait_parent(std::time::Duration::from_secs(10)).unwrap();
        let outcome = checked.execute_once(|| {
        let context = purge_preflight::acquire().unwrap();
        context.recheck().unwrap();
        let data = Path::new(&root).join(&id).join("fixture-data");
        use std::os::windows::fs::OpenOptionsExt;
        let held = std::fs::OpenOptions::new().read(true).share_mode(1).open(&data).unwrap();
        assert_eq!(std::fs::remove_file(&data).unwrap_err().raw_os_error(), Some(32));
        // Only this cfg(test) branch simulates registry absence and cleanup.
        assert_eq!(std::fs::read(Path::new(&root).join(&id).join("fixture-registration")).unwrap(), b"Absent");
        drop(held);
        context.recheck().unwrap();
        std::fs::remove_file(&data).unwrap();
        assert!(!data.exists());
        std::fs::OpenOptions::new().write(true).create_new(true).open(Path::new(&root).join(&id).join("cleanup-once")).unwrap();
        Ok(purge_policy::Outcome::Complete)
        });
        match outcome {
            Ok(value) => { assert_eq!(value, purge_policy::Outcome::Complete); assert!(checked.pending().is_err()); },
            Err(error) => assert!(error.starts_with("purge handoff claim refused:") || error == "purge handoff terminal exists", "{error}"),
        }
        std::fs::write(Path::new(&root).join(&id).join(format!("child-{}.done",std::process::id())),
            if Path::new(&root).join(&id).join("result.json").exists() { &b"finished"[..] } else { &b"refused"[..] }).unwrap();
    } else {
        let mut checked = result.unwrap();
        let error = checked.wait_parent(std::time::Duration::from_millis(50)).unwrap_err();
        assert_eq!(error, "purge parent still alive; Pending");
        assert_eq!(checked.execute_once(|| panic!("live parent must not execute")).unwrap_err(), "purge parent still alive; Pending");
        checked.pending().unwrap();
    }
}

#[test]
fn purge_handoff_real_child_parent_alive_hash_foreign_preserve_files() {
    assert_eq!(std::env::var("COMPUTERNAME").unwrap(), "RDLAB-WIN11");
    let root = Path::new(r"C:\RustDeskAgentLab").join(format!("purge-handoff-{}", uuid::Uuid::new_v4()));
    workspace::create(&root).unwrap();
    for mode in ["parent", "hash", "foreign"] {
        let id = uuid::Uuid::new_v4().simple().to_string();
        let directory = purge_handoff::prepare_fixture(&root, &id).unwrap();
        let victim = directory.join("fixture-data");
        std::fs::write(&victim, b"identity source state").unwrap();
        let plan_path = directory.join("plan.json");
        if mode != "parent" {
            let mut plan: serde_json::Value = serde_json::from_slice(&std::fs::read(&plan_path).unwrap()).unwrap();
            if mode == "hash" { plan["helper_sha256"] = "0".repeat(64).into(); }
            else { plan["transaction_id"] = "f".repeat(32).into(); }
            save(&plan_path, &plan);
        }
        let before = std::fs::read(&plan_path).unwrap();
        let result = std::process::Command::new(directory.join("purge-helper.exe"))
            .args(["--exact", "purge_handoff_child", "--nocapture"])
            .env("PURGE_HANDOFF_FIXTURE", &root).env("PURGE_HANDOFF_ID", &id).env("PURGE_HANDOFF_MODE", mode)
            .output().unwrap();
        assert!(result.status.success(), "{mode}: {}", String::from_utf8_lossy(&result.stderr));
        assert!(String::from_utf8_lossy(&result.stdout).contains("1 passed"));
        assert_eq!(std::fs::read(&victim).unwrap(), b"identity source state");
        assert_eq!(std::fs::read(&plan_path).unwrap(), before);
        if mode == "parent" {
            assert!(!directory.join("result.json").exists());
            assert!(std::fs::read_dir(&directory).unwrap().any(|entry| entry.unwrap().file_name().to_string_lossy().starts_with("progress-")));
        } else { assert!(!directory.join("result.json").exists()); }
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn purge_handoff_parent_exit_then_helper_records_actual_file_cleanup() {
    assert_eq!(std::env::var("COMPUTERNAME").unwrap(), "RDLAB-WIN11");
    let root = Path::new(r"C:\RustDeskAgentLab").join(format!("purge-handoff-success-{}", uuid::Uuid::new_v4()));
    workspace::create(&root).unwrap();
    let id = uuid::Uuid::new_v4().simple().to_string();
    let parent = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "purge_handoff_child", "--nocapture"])
        .env("PURGE_HANDOFF_FIXTURE", &root).env("PURGE_HANDOFF_ID", &id).env("PURGE_HANDOFF_MODE", "publisher")
        .output().unwrap();
    assert!(parent.status.success(), "{}", String::from_utf8_lossy(&parent.stderr));
    assert!(String::from_utf8_lossy(&parent.stdout).contains("Pending"));
    let result = root.join(&id).join("result.json");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while !result.exists() && std::time::Instant::now() < deadline { std::thread::sleep(std::time::Duration::from_millis(50)); }
    let receipt: serde_json::Value = serde_json::from_slice(&std::fs::read(&result).unwrap()).unwrap();
    assert_eq!(receipt["outcome"], "Complete");
    assert_eq!(receipt["transaction_id"], id);
    assert!(!root.join(&id).join("fixture-data").exists());
    assert!(root.join(&id).join("cleanup-once").exists());
    let before = std::fs::read(&result).unwrap();
    let serial = std::process::Command::new(root.join(&id).join("purge-helper.exe"))
        .args(["--exact", "purge_handoff_child", "--nocapture"])
        .env("PURGE_HANDOFF_FIXTURE", &root).env("PURGE_HANDOFF_ID", &id).env("PURGE_HANDOFF_MODE", "serial")
        .output().unwrap();
    assert!(serial.status.success(), "{}", String::from_utf8_lossy(&serial.stderr));
    assert_eq!(std::fs::read(&result).unwrap(), before);
    let finished = || std::fs::read_dir(root.join(&id)).unwrap().filter(|entry|
        entry.as_ref().unwrap().file_name().to_string_lossy().ends_with(".done")).count();
    while finished() < 2 && std::time::Instant::now() < deadline { std::thread::sleep(std::time::Duration::from_millis(50)); }
    assert_eq!(finished(), 2, "both concurrent helpers must finish assertions");
    // Keep the external helper and audit receipt; it has no startup registration.
    println!("HANDOFF_COMPLETE_RECEIPT={}", result.display());
}

#[test]
fn purge_handoff_error_receipt_preserves_detail_and_forbids_later_complete() {
    assert_eq!(std::env::var("COMPUTERNAME").unwrap(), "RDLAB-WIN11");
    let root = Path::new(r"C:\RustDeskAgentLab").join(format!("purge-handoff-error-{}", uuid::Uuid::new_v4()));
    workspace::create(&root).unwrap();
    let id = uuid::Uuid::new_v4().simple().to_string();
    let parent = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "purge_handoff_child", "--nocapture"])
        .env("PURGE_HANDOFF_FIXTURE", &root).env("PURGE_HANDOFF_ID", &id).env("PURGE_HANDOFF_MODE", "publisher-error")
        .output().unwrap();
    assert!(parent.status.success(), "{}", String::from_utf8_lossy(&parent.stderr));
    let directory = root.join(&id);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while !directory.join("error-checked").exists() && std::time::Instant::now() < deadline { std::thread::sleep(std::time::Duration::from_millis(50)); }
    assert!(directory.join("error-checked").exists(), "child error-receipt assertions incomplete");
    let before = std::fs::read(directory.join("result.json")).unwrap();
    let serial = std::process::Command::new(directory.join("purge-helper.exe"))
        .args(["--exact", "purge_handoff_child", "--nocapture"])
        .env("PURGE_HANDOFF_FIXTURE", &root).env("PURGE_HANDOFF_ID", &id).env("PURGE_HANDOFF_MODE", "serial").output().unwrap();
    assert!(serial.status.success(), "{}", String::from_utf8_lossy(&serial.stderr));
    assert_eq!(std::fs::read(directory.join("result.json")).unwrap(), before);
    assert_eq!(std::fs::read(directory.join("fixture-data")).unwrap(), b"identity source state");
    println!("HANDOFF_ERROR_RECEIPT={}", directory.join("result.json").display());
}

#[test]
fn purge_approval_preflight_real_process_busy_then_release() {
    assert_eq!(std::env::var("COMPUTERNAME").unwrap(), "RDLAB-WIN11");
    let guard = purge_preflight::acquire().unwrap();
    let result = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "purge_approval_preflight_child", "--nocapture"])
        .env("PURGE_PREFLIGHT_CHILD", "1").output().unwrap();
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
    assert!(String::from_utf8_lossy(&result.stdout).contains("1 passed"));
    drop(guard);
    drop(purge_preflight::acquire().unwrap());
    println!("PURGE_PREFLIGHT=child_busy;after_release=acquired");
}

fn purge_approval_case(case: &str) {
    assert_eq!(std::env::var("COMPUTERNAME").unwrap(), "RDLAB-WIN11");
    let original = Path::new(r"C:\ProgramData\RustDesk\ManagementAgent\rescue\current");
    let bundle_bytes = std::fs::read(original.join("bundle.json")).unwrap();
    let bundle: serde_json::Value = serde_json::from_slice(&bundle_bytes).unwrap();
    let public = rescue_assets::fixed_source_public_fixture(&bundle).unwrap();
    let root = Path::new(r"C:\RustDeskAgentLab").join(format!("purge-approval-{}", uuid::Uuid::new_v4()));
    workspace::create(&root).unwrap();
    fn copy_tree(from: &Path, to: &Path) {
        workspace::create(to).unwrap();
        for entry in std::fs::read_dir(from).unwrap() {
            let entry = entry.unwrap();
            if entry.file_type().unwrap().is_dir() { copy_tree(&entry.path(), &to.join(entry.file_name())); }
            else { std::fs::copy(entry.path(), to.join(entry.file_name())).unwrap(); }
        }
    }
    workspace::create(&root.join("rescue")).unwrap();
    copy_tree(original, &root.join("rescue/current"));
    workspace::create(&root.join("state")).unwrap();
    workspace::create(&root.join("transactions")).unwrap();
    let state_path = root.join("state/agent.json");
    let state = serde_json::json!({"agent_installation_id":bundle["installation_id"],"target_key":bundle["target_key"],"edition":"sos","package_kind":"msi","management_agent_enabled":true,"disabled":false});
    save(&state_path, &state);
    let identity_path = root.join("identity.toml");
    let installed_identity = Path::new(r"C:\Windows\ServiceProfiles\LocalService\AppData\Roaming\RustDesk\config\RustDesk.toml");
    std::fs::copy(installed_identity, &identity_path).unwrap();
    let marker = root.join("state/disable-recovery");
    let intent = root.join("RustDesk2.toml");
    std::fs::write(&marker, b"explicit local disable").unwrap();
    std::fs::write(&intent, b"[options]\nstop-service='Y'\n").unwrap();
    let original_identity = std::fs::read(&identity_path).unwrap();
    let snapshot = || [std::fs::read(&state_path).unwrap(), std::fs::read(&identity_path).unwrap(),
        std::fs::read(&marker).unwrap(), std::fs::read(&intent).unwrap(), std::fs::read(root.join("rescue/current/bundle.json")).unwrap()];
    let before = snapshot();
    drop(purge_approval::fixture(&root, &identity_path, Some(&public)).unwrap());
    assert_eq!(snapshot(), before);
    if case == "profiles" {
        let second = root.join("second-profile.toml");
        drop(purge_approval::resolve_fixture(&root, &[identity_path.clone(), second.clone()], &public).unwrap());
        assert_eq!(snapshot(), before);
        std::fs::write(&second, &original_identity).unwrap();
        assert_eq!(purge_approval::resolve_fixture(&root, &[identity_path.clone(), second.clone()], &public).err().unwrap(), "purge fixed identity ambiguous");
        assert_eq!(std::fs::read(&second).unwrap(), original_identity);
        std::fs::write(&second, b"foreign profile").unwrap();
        assert_eq!(purge_approval::resolve_fixture(&root, &[second.clone(), identity_path.clone()], &public).err().unwrap(), "purge current identity mismatch");
        assert_eq!(std::fs::read(&second).unwrap(), b"foreign profile");
        assert_eq!(snapshot(), before);
        std::fs::remove_dir_all(&root).unwrap();
        println!("PURGE_PROFILES=single accepted;ambiguous refused;foreign refused;bytes preserved");
        return;
    }
    if case == "intent-pair" {
        let gate = || rescue_assets::recovery_intent_gate_fixture(&root,
            bundle["installation_id"].as_str().unwrap(), bundle["target_key"].as_str().unwrap());
        assert_eq!(gate().unwrap_err(), "first installation locally blocked");
        assert_eq!(snapshot(), before);
        println!("PURGE_INTENT=disabled;recognition=allowed;recovery=first installation locally blocked;bytes_preserved=true");
        std::fs::remove_file(&marker).unwrap();
        std::fs::write(root.join("state/service-recovery-enabled"), b"enabled").unwrap();
        workspace::create(&root.join("current")).unwrap();
        let config = root.join("current/RustDesk2.toml");
        std::fs::write(&config, b"[options]\nstop-service='Y'\n").unwrap();
        let config_before = std::fs::read(&config).unwrap();
        drop(purge_approval::fixture(&root, &identity_path, Some(&public)).unwrap());
        assert_eq!(gate().unwrap_err(), "first installation user stopped");
        assert_eq!(std::fs::read(&config).unwrap(), config_before);
        assert!(!marker.exists());
        assert_eq!(std::fs::read(&identity_path).unwrap(), original_identity);
        assert_eq!(std::fs::read(&state_path).unwrap(), before[0]);
        println!("PURGE_INTENT=stop;recognition=allowed;recovery=first installation user stopped;bytes_preserved=true");
        std::fs::remove_dir_all(&root).unwrap();
        return;
    }
    if case != "baseline" {
        let assets = root.join("rescue/current");
        let manifest_path = assets.join("approved-manifest.json");
        let manifest_bytes = std::fs::read(&manifest_path).unwrap();
        let mut manifest: serde_json::Value = serde_json::from_slice(&manifest_bytes).unwrap();
        let key = bundle["target_key"].as_str().unwrap();
        let package_path = assets.join(bundle["package"]["relative_path"].as_str().unwrap());
        if case == "signature" {
            let mut signature = STANDARD.decode(manifest["targets"][key]["signature"].as_str().unwrap()).unwrap();
            signature[0] ^= 1;
            manifest["targets"][key]["signature"] = STANDARD.encode(signature).into();
            save(&manifest_path, &manifest);
            // Exercise the trusted-key signature verifier directly as well: a
            // receipt reference failure must not masquerade as signature rejection.
            let parsed: UpdateManifest = serde_json::from_value(manifest).unwrap();
            let error = package_gate::verify_with_key(&parsed, key, 1, "fixture-key", &public).unwrap_err();
            assert_eq!(error, "install manifest signature verification failed");
        } else {
            use std::io::{Read, Seek, SeekFrom, Write};
            let mut file = std::fs::OpenOptions::new().read(true).write(true).open(&package_path).unwrap();
            if case == "size" { file.set_len(file.metadata().unwrap().len() - 1).unwrap(); }
            else { let mut byte = [0u8]; file.read_exact(&mut byte).unwrap(); file.seek(SeekFrom::Start(0)).unwrap(); byte[0] ^= 1; file.write_all(&byte).unwrap(); }
        }
        let before = snapshot();
        let manifest_before = std::fs::read(&manifest_path).unwrap();
        let package_before = Sha256::digest(std::fs::read(&package_path).unwrap());
        let error = purge_approval::fixture(&root, &identity_path, Some(&public)).err().unwrap();
        let expected = match case { "size" => "rescue verification size mismatch", "hash" => "rescue verification hash mismatch", _ => "rescue approved reference mismatch" };
        assert_eq!(error, expected);
        assert_eq!(snapshot(), before);
        assert_eq!(std::fs::read(&manifest_path).unwrap(), manifest_before);
        assert_eq!(Sha256::digest(std::fs::read(&package_path).unwrap()), package_before);
        println!("PURGE_CASE={case};error={error};data_preserved=true");
        std::fs::remove_dir_all(&root).unwrap();
        return;
    }
    assert!(purge_approval::fixture(&root, &identity_path, None).err().unwrap().contains("key"));
    assert_eq!(snapshot(), before);
    let mut foreign = state.clone(); foreign["agent_installation_id"] = "e".repeat(32).into();
    save(&state_path, &foreign);
    let before = snapshot();
    assert!(purge_approval::fixture(&root, &identity_path, Some(&public)).err().unwrap().contains("identity mismatch"));
    assert_eq!(snapshot(), before);
    save(&state_path, &state);
    std::fs::write(&identity_path, b"malformed").unwrap();
    let before = snapshot();
    assert_eq!(purge_approval::fixture(&root, &identity_path, Some(&public)).err().unwrap(), "purge current identity mismatch");
    assert_eq!(snapshot(), before);
    std::fs::write(&identity_path, original_identity).unwrap();
    let txdir = root.join("transactions").join("d".repeat(32)); workspace::create(&txdir).unwrap();
    let mut tx = transaction::Transaction::new("d".repeat(32), "windows-x86_64-msi-sos".into(), "a".repeat(64), "b".repeat(64), 100).unwrap();
    tx.phase = transaction::Phase::Applying;
    save(&txdir.join("transaction.json"), &tx);
    let before = snapshot();
    assert_eq!(purge_approval::fixture(&root, &identity_path, Some(&public)).err().unwrap(), "purge active transaction");
    assert_eq!(snapshot(), before);
    assert_eq!(std::fs::read(original.join("bundle.json")).unwrap(), bundle_bytes);
    std::fs::remove_dir_all(&root).unwrap();
    println!("PURGE_APPROVAL_FIXTURE=passed;local_stop_preserved;production_key_refused;fixture_removed");
}

#[test]
fn committed_signed_msi_creates_verified_bundle_without_replacing_original_data() {
    let f = Fixture::new();
    let original = std::fs::read(f.root.join("current/identity.toml")).unwrap();
    let report = f.prepare().unwrap();
    assert_eq!(report["rescue_readiness"], "degraded");
    for component in [
        "client",
        "offline_package",
        "credentials",
        "agent_backup",
        "bootstrap_backup",
    ] {
        assert_eq!(report[component], "verified", "{component}");
    }
    assert_eq!(report["recovery_entry"], "not_integrated");
    assert_eq!(report["source_transaction"], "a".repeat(32));
    let bundle = f.root.join("rescue/current");
    assert!(bundle.join("client/files/data/flutter_assets").is_dir());
    assert!(bundle.join("agent-sos-v187.msi").is_file());
    assert!(!bundle.join("identity.toml").exists());
    let protected = std::fs::read(bundle.join("identity.dpapi")).unwrap();
    assert_ne!(protected, original);
    assert!(!protected
        .windows(b"fixture-original-id".len())
        .any(|part| part == b"fixture-original-id"));
    assert_eq!(
        std::fs::read(f.root.join("current/identity.toml")).unwrap(),
        original
    );
    assert_eq!(f.verify().unwrap()["client"], "verified");
    let before = std::fs::read(bundle.join("bundle.json")).unwrap();
    for name in [
        "identity.dpapi",
        "config.dpapi",
        "agent.exe",
        "bootstrap.exe",
        "bootstrap.json",
        "approved-manifest.json",
        "client/manifest.json",
    ] {
        let path = bundle.join(name);
        let bytes = std::fs::read(&path).unwrap();
        std::fs::write(&path, b"damaged").unwrap();
        let error = f.verify().unwrap_err();
        assert!(
            error.contains("mismatch") || error.contains("malformed"),
            "{name}: {error}"
        );
        assert_eq!(std::fs::read(bundle.join("bundle.json")).unwrap(), before);
        std::fs::write(&path, bytes).unwrap();
    }
    assert!(f.prepare().is_ok());
    assert_eq!(std::fs::read(bundle.join("bundle.json")).unwrap(), before);
    std::fs::write(bundle.join("client/files/librustdesk.dll"), b"damaged").unwrap();
    assert!(f.verify().unwrap_err().contains("mismatch"));
    assert_eq!(std::fs::read(bundle.join("bundle.json")).unwrap(), before);
}

#[test]
fn local_gates_and_active_transactions_refuse_before_publication() {
    let f = Fixture::new();
    for marker in ["disable-agent", "disable-recovery", "maintenance.json"] {
        let file = f.root.join("state").join(marker);
        std::fs::write(&file, b"keep").unwrap();
        assert!(
            f.prepare().unwrap_err().contains("locally blocked"),
            "{marker}"
        );
        assert_eq!(std::fs::read(&file).unwrap(), b"keep");
        assert!(!f.root.join("rescue/current").exists());
        std::fs::remove_file(file).unwrap();
    }
    let config = f.root.join("current/RustDesk2.toml");
    let before = std::fs::read(&config).unwrap();
    std::fs::write(&config, "[options]\nstop-service = 'Y'\n").unwrap();
    assert!(f.prepare().unwrap_err().contains("user stopped"));
    assert_eq!(
        std::fs::read_to_string(&config).unwrap(),
        "[options]\nstop-service = 'Y'\n"
    );
    std::fs::write(&config, before).unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    let (release, wait) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        let _guard = install_guard::InstallGuard::try_acquire().unwrap().unwrap();
        tx.send(()).unwrap();
        wait.recv().unwrap();
    });
    rx.recv().unwrap();
    assert!(f.prepare().unwrap_err().contains("installation busy"));
    release.send(()).unwrap();
    worker.join().unwrap();
    assert!(!f.root.join("rescue/current").exists());
    let active = f.root.join("transactions").join("d".repeat(32));
    workspace::create(&active).unwrap();
    let bytes = std::fs::read(f.tx().join("transaction.json")).unwrap();
    let mut tx = transaction::Transaction::decode(&bytes).unwrap();
    tx.id = "d".repeat(32);
    tx.phase = transaction::Phase::Applying;
    save(&active.join("transaction.json"), &tx);
    assert!(f.prepare().unwrap_err().contains("active transaction"));
    assert_eq!(
        std::fs::read(f.tx().join("transaction.json")).unwrap(),
        bytes
    );
    assert!(!f.root.join("rescue/current").exists());
}

#[test]
fn reparse_and_untrusted_acl_sources_fail_without_publishing_or_changing_identity() {
    let f = Fixture::new();
    let identity = std::fs::read(f.root.join("current/identity.toml")).unwrap();
    let tx = f.tx();
    let original = tx.join("approved-manifest.original.json");
    std::fs::rename(tx.join("approved-manifest.json"), &original).unwrap();
    std::os::windows::fs::symlink_file(&original, tx.join("approved-manifest.json")).unwrap();
    assert!(f.prepare().unwrap_err().contains("reparse"));
    assert!(!f.root.join("rescue/current").exists());
    std::fs::remove_file(tx.join("approved-manifest.json")).unwrap();
    std::fs::rename(&original, tx.join("approved-manifest.json")).unwrap();
    let path = tx.join("approved-manifest.json");
    let status = std::process::Command::new(r"C:\Windows\System32\icacls.exe")
        .arg(&path)
        .args(["/grant", "*S-1-5-32-545:(W)"])
        .status()
        .unwrap();
    assert!(status.success());
    assert!(f.prepare().unwrap_err().contains("permission"));
    assert_eq!(
        std::fs::read(f.root.join("current/identity.toml")).unwrap(),
        identity
    );
    assert!(!f.root.join("rescue/current").exists());
}

#[test]
fn wrong_target_or_damaged_current_client_never_replaces_old_assets() {
    let f = Fixture::new();
    let path = f.tx().join("approved-manifest.json");
    let original = std::fs::read(&path).unwrap();
    let mut manifest: UpdateManifest = serde_json::from_slice(&original).unwrap();
    let target = manifest.targets.remove("windows-x86_64-msi-sos").unwrap();
    manifest
        .targets
        .insert("windows-x86_64-msi-standard".into(), target);
    save(&path, &manifest);
    assert!(f.prepare().unwrap_err().contains("target absent"));
    assert!(!f.root.join("rescue/current").exists());
    std::fs::write(&path, &original).unwrap();
    let dll = f.root.join("current/client/files/librustdesk.dll");
    std::fs::write(&dll, b"previous damaged client").unwrap();
    assert!(f.prepare().unwrap_err().contains("current client mismatch"));
    assert_eq!(std::fs::read(&dll).unwrap(), b"previous damaged client");
    assert!(!f.root.join("rescue/current").exists());
    assert!(f.root.join("rescue/staging/agent-sos-v187.msi").is_file());
    assert!(f.prepare().unwrap_err().contains("occupied"));
    assert_eq!(std::fs::read(&path).unwrap(), original);
}

#[test]
fn invalid_signature_and_changed_live_identity_preserve_sources() {
    let f = Fixture::new();
    let path = f.tx().join("approved-manifest.json");
    let original = std::fs::read(&path).unwrap();
    let mut manifest: UpdateManifest = serde_json::from_slice(&original).unwrap();
    manifest
        .targets
        .get_mut("windows-x86_64-msi-sos")
        .unwrap()
        .signature = STANDARD.encode([0u8; 64]);
    save(&path, &manifest);
    assert!(f.prepare().unwrap_err().contains("signature"));
    assert!(!f.root.join("rescue/current").exists());
    std::fs::write(&path, &original).unwrap();
    std::fs::write(
        f.root.join("current/identity.toml"),
        b"old identity must remain",
    )
    .unwrap();
    assert!(f.prepare().unwrap_err().contains("identity"));
    assert_eq!(
        std::fs::read(f.root.join("current/identity.toml")).unwrap(),
        b"old identity must remain"
    );
    assert_eq!(std::fs::read(&path).unwrap(), original);
    assert!(!f.root.join("rescue/current").exists());
}

#[test]
fn safe_inherited_service_state_acl_allows_rescue_without_rewriting_permissions() {
    let f = Fixture::new();
    let state = f.root.join("state");
    let before = std::fs::read(state.join("agent.json")).unwrap();
    assert!(
        std::process::Command::new(r"C:\Windows\System32\icacls.exe")
            .arg(&state)
            .args(["/inheritance:e"])
            .status()
            .unwrap()
            .success()
    );
    assert!(workspace::verify(&state)
        .unwrap_err()
        .contains("inheritance not protected"));
    assert!(
        std::process::Command::new(r"C:\Windows\System32\icacls.exe")
            .arg(&state)
            .args(["/grant", "*S-1-5-32-545:(OI)(CI)(RX)"])
            .status()
            .unwrap()
            .success()
    );
    let report = f.prepare().unwrap();
    assert_eq!(report["offline_package"], "verified");
    assert_eq!(std::fs::read(state.join("agent.json")).unwrap(), before);
    assert!(workspace::verify(&state)
        .unwrap_err()
        .contains("inheritance not protected"));
}

#[test]
fn inherited_state_with_non_admin_write_or_reparse_is_refused_and_preserved() {
    let f = Fixture::new();
    let state = f.root.join("state");
    let before = std::fs::read(state.join("agent.json")).unwrap();
    assert!(
        std::process::Command::new(r"C:\Windows\System32\icacls.exe")
            .arg(&state)
            .args(["/inheritance:e", "/grant", "*S-1-5-32-545:(DC)"])
            .status()
            .unwrap()
            .success()
    );
    assert!(f.prepare().unwrap_err().contains("non-admin write"));
    assert_eq!(std::fs::read(state.join("agent.json")).unwrap(), before);
    std::fs::rename(&state, f.root.join("state-original")).unwrap();
    std::os::windows::fs::symlink_dir(f.root.join("state-original"), &state).unwrap();
    assert!(f.prepare().unwrap_err().contains("reparse"));
    assert!(!f.root.join("rescue/current").exists());
    std::fs::remove_dir(&state).unwrap();
    std::fs::rename(f.root.join("state-original"), &state).unwrap();
    assert!(
        std::process::Command::new(r"C:\Windows\System32\icacls.exe")
            .arg(&state)
            .args(["/remove:g", "*S-1-5-32-545"])
            .status()
            .unwrap()
            .success()
    );
    assert!(
        std::process::Command::new(r"C:\Windows\System32\icacls.exe")
            .arg(&f.root)
            .args(["/grant", "*S-1-5-32-545:(DC)"])
            .status()
            .unwrap()
            .success()
    );
    assert!(f.prepare().unwrap_err().contains("non-admin write"));
    assert_eq!(std::fs::read(state.join("agent.json")).unwrap(), before);
}

#[test]
fn real_registered_msi_source_and_live_ready_are_checked_without_installed_writes() {
    let f = Fixture::new();
    let manifest: UpdateManifest =
        serde_json::from_slice(&std::fs::read(f.tx().join("approved-manifest.json")).unwrap())
            .unwrap();
    let package = f.tx().join("agent-sos-v187.msi");
    let target = package_gate::verify_with_key(
        &manifest,
        "windows-x86_64-msi-sos",
        2026100502,
        "fixture-key",
        f.public.as_ref(),
    )
    .unwrap();
    let _package_lock = package_gate::lock_verified_file(&package, target).unwrap();
    let msi = msi_metadata::inspect(&package).unwrap();
    let registered = msi_metadata::installed_source(&msi.product_code).unwrap();
    assert_eq!(registered.package_name, "agent-sos-v187.msi");
    assert_eq!(
        registered.package_name,
        package_gate::cache_name(&target.primary, "msi").unwrap()
    );
    let source = &registered.source;
    let _source_lock = package_gate::lock_verified_file(source, target).unwrap();
    let before = std::fs::read(source).unwrap();
    assert_eq!(before, std::fs::read(&package).unwrap());
    let report = rescue_assets::installed_source_fixture(
        &f.tx().join("agent-sos-v187.msi"),
        &manifest,
        f.public.as_ref(),
        2026100502,
    )
    .unwrap();
    assert_eq!(report["source"], source.to_str().unwrap());
    assert_eq!(report["product_code"], msi.product_code);
    assert_eq!(report["package_name"], registered.package_name);
    assert_eq!(report["sha256"], target.sha256);
    assert_eq!(report["ready"], true);
    assert_eq!(report["build_seq"], 2026100502);
    println!("PRODUCTION_SOURCE_CHECK={report}");
    assert!(rescue_assets::installed_source_fixture(
        &f.tx().join("agent-sos-v187.msi"),
        &manifest,
        f.public.as_ref(),
        2026100503
    )
    .unwrap_err()
    .contains("health unconfirmed"));
    assert!(msi_metadata::installed_source("not-a-product")
        .unwrap_err()
        .contains("product code invalid"));
    let missing = f.root.join("missing-source.msi");
    assert!(package_gate::lock_verified_file(&missing, target)
        .err()
        .unwrap()
        .contains("path unavailable"));
    assert!(!missing.exists());
    let link = f.root.join("linked-source.msi");
    std::os::windows::fs::symlink_file(source, &link).unwrap();
    assert!(msi_metadata::inspect(&link)
        .unwrap_err()
        .contains("reparse"));
    let error = package_gate::lock_verified_file(&link, target)
        .err()
        .unwrap();
    assert!(
        error.contains("symlink") || error.contains("reparse"),
        "{error}"
    );
    std::fs::remove_file(link).unwrap();
    assert_eq!(std::fs::read(source).unwrap(), before);
    assert_eq!(
        msi_metadata::installed_source(&msi.product_code)
            .unwrap()
            .source,
        *source
    );
    rescue_assets::installed_state_fixture().unwrap();
    println!("INSTALLED_STATE_READONLY_VALIDATED");
}

#[test]
fn copied_installed_directory_acl_allows_existing_safe_assets() {
    let f = Fixture::new();
    let original = std::fs::read(f.root.join("state/agent.json")).unwrap();
    let script=format!("$source=Get-Acl -LiteralPath 'C:\\ProgramData\\RustDesk\\ManagementAgent\\state'; $acl=Get-Acl -LiteralPath '{}'; $acl.SetSecurityDescriptorSddlForm($source.Sddl.Replace('D:AI','D:P').Replace(';ID;',';;').Replace('CIID','CI').Replace('IOID','IO')); Set-Acl -LiteralPath '{}' -AclObject $acl",f.root.join("state").display(),f.root.join("state").display());
    assert!(std::process::Command::new("powershell.exe")
        .args(["-NoProfile", "-Command", &script])
        .status()
        .unwrap()
        .success());
    let result = f.prepare();
    assert_eq!(
        std::fs::read(f.root.join("state/agent.json")).unwrap(),
        original
    );
    assert!(result.is_ok(), "{:?}", result.err());
}

#[test]
fn create_child_permission_cannot_write_rename_or_delete_existing_protected_assets() {
    assert_eq!(std::env::var("COMPUTERNAME").unwrap(), "RDLAB-WIN11");
    let root =
        Path::new(r"C:\RustDeskAgentLab").join(format!("rescue-v200-acl-{}", uuid::Uuid::new_v4()));
    workspace::create(&root).unwrap();
    let script=format!("$source=Get-Acl 'C:\\ProgramData\\RustDesk\\ManagementAgent'; $acl=Get-Acl '{}'; $acl.SetSecurityDescriptorSddlForm($source.Sddl.Replace('D:AI','D:P').Replace(';ID;',';;').Replace('CIID','CI').Replace('IOID','IO')); Set-Acl -LiteralPath '{}' -AclObject $acl",root.display(),root.display());
    assert!(std::process::Command::new("powershell.exe")
        .args(["-NoProfile", "-Command", &script])
        .status()
        .unwrap()
        .success());
    let child = root.join("protected");
    workspace::create(&child).unwrap();
    std::fs::write(child.join("identity.bin"), b"original bytes").unwrap();
    #[repr(C)]
    struct SidAndAttributes {
        sid: *mut std::ffi::c_void,
        attributes: u32,
    }
    #[link(name = "advapi32")]
    extern "system" {
        fn OpenProcessToken(
            process: *mut std::ffi::c_void,
            access: u32,
            token: *mut *mut std::ffi::c_void,
        ) -> i32;
        fn CreateWellKnownSid(
            kind: i32,
            domain: *const std::ffi::c_void,
            sid: *mut std::ffi::c_void,
            size: *mut u32,
        ) -> i32;
        fn CreateRestrictedToken(
            token: *mut std::ffi::c_void,
            flags: u32,
            disable_count: u32,
            disable: *const SidAndAttributes,
            privilege_count: u32,
            privileges: *const std::ffi::c_void,
            restrict_count: u32,
            restrict: *const SidAndAttributes,
            result: *mut *mut std::ffi::c_void,
        ) -> i32;
        fn ImpersonateLoggedOnUser(token: *mut std::ffi::c_void) -> i32;
        fn RevertToSelf() -> i32;
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn GetCurrentProcess() -> *mut std::ffi::c_void;
        fn CloseHandle(handle: *mut std::ffi::c_void) -> i32;
    }
    let mut token = std::ptr::null_mut();
    let mut restricted = std::ptr::null_mut();
    let mut sid = [0u64; 16];
    let mut size = 128u32;
    unsafe {
        assert_ne!(
            OpenProcessToken(GetCurrentProcess(), 0xf01ff, &mut token),
            0
        );
        assert_ne!(
            CreateWellKnownSid(27, std::ptr::null(), sid.as_mut_ptr().cast(), &mut size),
            0
        );
        let entry = SidAndAttributes {
            sid: sid.as_mut_ptr().cast(),
            attributes: 0,
        };
        assert_ne!(
            CreateRestrictedToken(
                token,
                1,
                0,
                std::ptr::null(),
                0,
                std::ptr::null(),
                1,
                &entry,
                &mut restricted
            ),
            0
        );
        assert_ne!(ImpersonateLoggedOnUser(restricted), 0);
    }
    let created = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(root.join("new-file.bin"));
    let write = std::fs::OpenOptions::new()
        .write(true)
        .open(child.join("identity.bin"));
    let rename = std::fs::rename(&child, root.join("renamed"));
    let delete = std::fs::remove_file(child.join("identity.bin"));
    unsafe {
        assert_ne!(RevertToSelf(), 0);
        CloseHandle(restricted);
        CloseHandle(token);
    }
    assert!(
        created.is_ok(),
        "restricted Users must really be able to create child: {:?}",
        created.err()
    );
    drop(created);
    for (name, error) in [
        ("write", write.err().unwrap()),
        ("rename", rename.unwrap_err()),
        ("delete", delete.unwrap_err()),
    ] {
        assert_eq!(
            error.kind(),
            std::io::ErrorKind::PermissionDenied,
            "{name}: {error}"
        );
    }
    assert_eq!(
        std::fs::read(child.join("identity.bin")).unwrap(),
        b"original bytes"
    );
    println!("ACL_REAL_IO=create_child_allowed;existing_write_rename_delete_denied");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn actual_installed_layout_prepares_complete_lab_bundle_and_decrypts_exact_originals() {
    let f = Fixture::new();
    let state: serde_json::Value = serde_json::from_slice(
        &std::fs::read(r"C:\ProgramData\RustDesk\ManagementAgent\state\agent.json").unwrap(),
    )
    .unwrap();
    assert_eq!(
        state["agent_installation_id"],
        "dcc73dd2fba22bf04c7df497053ae2df"
    );
    let identity = std::fs::read(
        r"C:\Windows\ServiceProfiles\LocalService\AppData\Roaming\RustDesk\config\RustDesk.toml",
    )
    .unwrap();
    std::fs::write(f.tx().join("identity.toml"), &identity).unwrap();
    let path = f.tx().join("prepared.json");
    let mut plan = install_plan::InstallPlan::decode(&std::fs::read(&path).unwrap()).unwrap();
    plan.installation_id = state["agent_installation_id"].as_str().unwrap().into();
    plan.identity_fingerprint = hex::encode(Sha256::digest(&identity));
    plan.identity_backup.size = identity.len() as u64;
    plan.identity_backup.sha256 = plan.identity_fingerprint.clone();
    save(&path, &plan);
    let mut tx =
        transaction::Transaction::decode(&std::fs::read(f.tx().join("transaction.json")).unwrap())
            .unwrap();
    tx.identity_fingerprint = plan.identity_fingerprint.clone();
    save(&f.tx().join("transaction.json"), &tx);
    let result = rescue_assets::prepare_installed_fixture(&f.root, f.public.as_ref()).unwrap();
    assert_eq!(result["installed_source_check"]["ready"], true);
    assert_eq!(result["client"], "verified");
    assert_eq!(result["credentials"], "verified");
    assert_eq!(
        std::fs::read(
            r"C:\Windows\ServiceProfiles\LocalService\AppData\Roaming\RustDesk\config\RustDesk.toml"
        )
        .unwrap(),
        identity
    );
    assert!(!f.root.join("rescue/current/identity.toml").exists());
    println!("INSTALLED_FULL_PREPARE={result};DPAPI_ORIGINAL_BYTES_EQUAL=true");
}

#[test]
fn native_transaction_child() {
    let Some(directory) = std::env::var_os("RUSTDESK_HELPER_RUNTIME_FIXTURE") else {
        return;
    };
    assert_eq!(std::env::var("COMPUTERNAME").unwrap(), "RDLAB-WIN11");
    let directory = PathBuf::from(directory);
    assert_eq!(
        directory.parent().unwrap(),
        Path::new(r"C:\ProgramData\RustDesk\ManagementAgent\transactions")
    );
    let public = STANDARD
        .decode(std::env::var("RUSTDESK_HELPER_FIXTURE_PUBLIC").unwrap())
        .unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(120);
    while !directory.join("handoff.json").exists() && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(100));
    }
    helper_runtime::prepare_ready(&directory).unwrap();
    helper_runtime::wait_for_release(&directory).unwrap();
    let mut owner = loop {
        match helper_runtime::take_over_native_fixture(&directory, &public) {
            Ok(owner) => break owner,
            Err(error) if error.contains("guard busy") && std::time::Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(100))
            }
            Err(error) => panic!("takeover: {error}"),
        }
    };
    let mut backend = native_install::NativeInstaller::open(&directory).unwrap();
    let result = owner.execute(&mut backend).unwrap();
    assert_eq!(
        result.phase,
        transaction::Phase::Committed,
        "{:?}",
        result.error
    );
    owner.finalize().unwrap();
    drop(owner);
    std::fs::write(
        directory.join("rescue-finalize-observed.json"),
        b"{\"finalized\":true,\"owner_released\":true}",
    )
    .unwrap();
    rescue_assets::after_commit_fixture(&directory, &public).unwrap();
}

use std::time::Duration;

#[test]
#[ignore = "Real MSI SYSTEM Win11 transaction; requires explicit snapshot and v205 runner"]
fn successful_native_transaction_publishes_fixed_rescue_after_finalize() {
    committed_rescue_case(false);
}

#[test]
#[ignore = "Actual same-build MSI committed healthy refresh and retained previous; v217"]
fn rotation_real_samebuild_msi_commit_refreshes_fixed_bundle() {
    committed_rescue_case(true);
}

fn committed_rescue_case(rotate: bool) {
    assert_eq!(std::env::var("COMPUTERNAME").unwrap(), "RDLAB-WIN11");
    if rotate {
        assert_eq!(std::env::var("RUSTDESK_ROTATION_LAB").unwrap(), "v217");
    } else {
        assert_eq!(std::env::var("RUSTDESK_RESCUE_E2E").unwrap(), "v205");
    }
    let bundle = Path::new(r"C:\ProgramData\RustDesk\ManagementAgent\rescue\current");
    let old = if rotate {
        Some(std::fs::read(bundle.join("bundle.json")).unwrap())
    } else {
        assert!(!bundle.exists(), "preserve existing bundle");
        None
    };
    let package = Path::new(r"C:\RustDeskAgentLab\agent-sos-v187.msi");
    let bytes = std::fs::read(package).unwrap();
    assert_eq!(
        hex::encode(Sha256::digest(&bytes)),
        "b912548e12248e8950abae312afba156ac4b9aab7ee07b8c0a72b988855813a9"
    );
    let state_path = Path::new(r"C:\ProgramData\RustDesk\ManagementAgent\state\agent.json");
    let before: serde_json::Value =
        serde_json::from_slice(&std::fs::read(state_path).unwrap()).unwrap();
    assert_eq!(
        before["agent_installation_id"],
        "dcc73dd2fba22bf04c7df497053ae2df"
    );
    hbb_common::sodiumoxide::init().unwrap();
    let (mut public, secret) = sign::gen_keypair();
    let key = "windows-x86_64-msi-sos";
    let mut manifest = UpdateManifest {
        schema: 2,
        product: "rustdesk-yan".into(),
        edition: "multi".into(),
        channel: "stable".into(),
        ..Default::default()
    };
    let mut target = UpdateTarget {
        version: "1.5.3".into(),
        build_seq: 2026100502,
        source_commit: "production-equivalent-rescue-v205".into(),
        primary: "https://download.yan.life/rustdesk/stable/1.5.3/agent-sos-v187.msi".into(),
        size: bytes.len() as u64,
        sha256: hex::encode(Sha256::digest(&bytes)),
        signature_key_id: "fixture-key".into(),
        ..Default::default()
    };
    target.signature = STANDARD.encode(sign::sign_detached(
        &base::update::update_signature_payload(&manifest, key, &target).unwrap(),
        &secret,
    ));
    manifest.targets.insert(key.into(), target);
    if let Some(old) = &old {
        let value: serde_json::Value = serde_json::from_slice(old).unwrap();
        public = sign::PublicKey::from_slice(
            &rescue_assets::fixed_source_public_fixture(&value).unwrap(),
        )
        .unwrap();
        manifest =
            serde_json::from_slice(&std::fs::read(bundle.join("approved-manifest.json")).unwrap())
                .unwrap();
    }
    let parent = Path::new(r"C:\ProgramData\RustDesk\ManagementAgent\transactions");
    if !parent.exists() {
        workspace::create(parent).unwrap();
    }
    workspace::verify(parent).unwrap();
    let id = uuid::Uuid::new_v4().simple().to_string();
    let command = uuid::Uuid::new_v4().simple().to_string();
    let directory = parent.join(&id);
    println!("RESCUE_E2E_TRANSACTION={}", directory.display());
    let request = preparation::Request {
        transaction_id: &id,
        command_id: &command,
        installation_id: before["agent_installation_id"].as_str().unwrap(),
        target_key: key,
        current_build: 2026100502,
        package,
        manifest: &manifest,
        rollback_package: package,
        rollback_manifest: &manifest,
    };
    assert!(
        preparation::prepare(&directory, &request).is_err(),
        "test key rejected by production"
    );
    assert!(!directory.exists());
    let plan = preparation::prepare_fixture(&directory, &request, public.as_ref()).unwrap();
    std::fs::write(directory.join("rescue-fixture-public.bin"), public.as_ref()).unwrap();
    let mut child = parent_handoff::launch_native_fixture(&directory, public.as_ref()).unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(900);
    while child.try_wait().unwrap().is_none() && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(250));
    }
    let status = child
        .try_wait()
        .unwrap()
        .expect("helper timed out; preserve transaction");
    assert!(
        status.success(),
        "{}",
        std::fs::read_to_string(directory.join("parent-helper.stderr.log")).unwrap()
    );
    assert_eq!(
        transaction::Transaction::decode(
            &std::fs::read(directory.join("transaction.json")).unwrap()
        )
        .unwrap()
        .phase,
        transaction::Phase::Committed
    );
    assert!(!state_path
        .parent()
        .unwrap()
        .join("maintenance.json")
        .exists());
    assert!(resume_task::read_back(&plan.transaction_id).is_err());
    assert!(bundle.join("bundle.json").is_file());
    let report: serde_json::Value =
        serde_json::from_slice(&std::fs::read(directory.join("rescue-result.json")).unwrap())
            .unwrap();
    assert_eq!(report["rescue_readiness"], "degraded");
    assert_eq!(report["recovery_entry"], "not_integrated");
    assert_eq!(report["source_transaction"], id);
    let verified = rescue_assets::verify_fixed_fixture(public.as_ref()).unwrap();
    assert_eq!(verified["credentials"], "verified");
    assert_eq!(std::fs::read(package).unwrap(), bytes);
    assert!(native_install::client_ready(
        Path::new(r"C:\Program Files\RustDesk\RustDesk.exe"),
        2026100502,
        true
    ));
    println!("RESCUE_FIXED_ROOT_COMMITTED={report}");
    if let Some(old) = old {
        assert_eq!(
            std::fs::read(bundle.parent().unwrap().join("previous/bundle.json")).unwrap(),
            old
        );
        let updated = std::fs::read(bundle.join("bundle.json")).unwrap();
        assert_ne!(updated, old);
        rescue_assets::after_commit_fixture(&directory, public.as_ref()).unwrap();
        assert_eq!(std::fs::read(bundle.join("bundle.json")).unwrap(), updated);
        let receipt: serde_json::Value =
            serde_json::from_slice(&std::fs::read(directory.join("rotation.json")).unwrap())
                .unwrap();
        assert_eq!(receipt["phase"], "published");
        println!(
            "ROTATION_FIXED_CURRENT={id};PREVIOUS_RETAINED=true;SAME_BUILD=true;RECEIPT={receipt}"
        );
    }
}

#[test]
#[ignore = "Readback of real fixed bundle created by v205 E2E"]
fn fixed_bundle_readback_is_verified_and_never_replaced() {
    assert_eq!(std::env::var("COMPUTERNAME").unwrap(), "RDLAB-WIN11");
    let root = Path::new(r"C:\ProgramData\RustDesk\ManagementAgent\rescue\current");
    let bytes = std::fs::read(root.join("bundle.json")).unwrap();
    let bundle: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let public = rescue_assets::fixed_source_public_fixture(&bundle).unwrap();
    let verified = rescue_assets::verify_fixed_fixture(&public).unwrap();
    assert_eq!(verified["rescue_readiness"], "degraded");
    if bundle["schema"] == 1 {
        let key_path = rescue_assets::fixture_source_key_path(
            Path::new(r"C:\ProgramData\RustDesk\ManagementAgent"),
            &bundle,
        )
        .unwrap();
        rescue_assets::after_commit_fixture(key_path.parent().unwrap(), &public).unwrap();
    } else {
        assert_eq!(bundle["source_kind"], "installed_msi");
        assert!(rescue_assets::after_commit_fixture(root, &public)
            .unwrap_err()
            .contains("outside fixed transactions"));
    }
    assert_eq!(std::fs::read(root.join("bundle.json")).unwrap(), bytes);
    let (wrong, _) = sign::gen_keypair();
    assert!(rescue_assets::verify_fixed_fixture(wrong.as_ref())
        .unwrap_err()
        .contains("signature"));
    assert_eq!(std::fs::read(root.join("bundle.json")).unwrap(), bytes);
    println!("FIXED_RESCUE_READBACK={verified};ORIGINAL_IDENTITY_BYTES_EQUAL=true;OLD_BUNDLE_PRESERVED=true");
}

#[test]
fn r3_stopped_absent_gate_rejects_disabled_running_unknown_and_processes() {
    for state in ["stopped", "absent"] {
        assert!(rescue_recovery::gate_fixture(state, false, false).is_ok());
    }
    for (state, disabled, processes) in [
        ("running", false, false),
        ("unknown", false, false),
        ("stopped", true, false),
        ("stopped", false, true),
    ] {
        assert!(rescue_recovery::gate_fixture(state, disabled, processes).is_err());
    }
}

#[test]
fn r3_periodic_startup_grace_precedes_any_service_or_bundle_action() {
    assert!(matches!(
        std::env::var("COMPUTERNAME").unwrap().as_str(),
        "RDLAB-WIN10" | "RDLAB-WIN11"
    ));
    for elapsed in [
        Duration::ZERO,
        Duration::from_secs(30),
        Duration::from_millis(179999),
    ] {
        assert_eq!(
            rescue_recovery::periodic_fixture(elapsed, &[0; 32]).unwrap(),
            rescue_recovery::Cycle::Deferred
        );
    }
}

#[test]
fn review1_rejected_or_deferred_rescue_still_dispatches_management_without_package_bypass() {
    for result in [
        Err("signature invalid".into()),
        Ok(rescue_recovery::Cycle::Deferred),
        Ok(rescue_recovery::Cycle::Recovered),
    ] {
        let route = rescue_recovery::routing(&result);
        let (operations, ops) = std::sync::mpsc::sync_channel(1);
        let (status, reported) = std::sync::mpsc::sync_channel(1);
        let mut packages = 0;
        if route.management {
            operations.try_send("poll").unwrap();
            status.try_send("status-policy").unwrap();
        }
        if route.legacy_package {
            packages += 1;
        }
        assert_eq!(ops.try_recv().ok(), Some("poll"));
        assert_eq!(reported.try_recv().ok(), Some("status-policy"));
        assert!(!route.legacy_service);
        assert_eq!(packages, 0);
    }
}

fn review_inventory() -> (PathBuf, directory_backup::DirectoryBackup) {
    let root = Path::new(r"C:\RustDeskAgentLab")
        .join(format!("rescue-v200-review-{}", uuid::Uuid::new_v4()));
    workspace::create(&root).unwrap();
    let source = root.join("normal");
    workspace::create(&source).unwrap();
    std::fs::write(source.join("RustDesk.exe"), b"approved A executable").unwrap();
    std::fs::write(source.join("librustdesk.dll"), b"approved A dll").unwrap();
    let inventory = directory_backup::capture(&source, &root.join("approved")).unwrap();
    (root, inventory)
}

#[test]
fn review2_present_newer_or_different_client_never_becomes_old_bundle_damage() {
    let (root, inventory) = review_inventory();
    let normal = root.join("normal");
    std::fs::write(normal.join("RustDesk.exe"), b"complete B executable").unwrap();
    std::fs::write(normal.join("librustdesk.dll"), b"complete B dll").unwrap();
    assert_eq!(
        rescue_assets::r3_classify_fixture(&inventory, &normal, false).unwrap(),
        false
    );
    std::fs::write(normal.join("extra.txt"), b"user data").unwrap();
    assert_eq!(
        rescue_assets::r3_classify_fixture(&inventory, &normal, false).unwrap(),
        false
    );
    assert_eq!(
        std::fs::read(normal.join("RustDesk.exe")).unwrap(),
        b"complete B executable"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn review3_exact_existing_fallback_is_reusable_without_changing_bytes() {
    let (root, inventory) = review_inventory();
    let path = root.join("normal");
    assert!(rescue_assets::r3_classify_fixture(&inventory, &path, true).unwrap());
    assert_eq!(
        std::fs::read(path.join("RustDesk.exe")).unwrap(),
        b"approved A executable"
    );
    std::fs::write(path.join("unknown.txt"), b"keep").unwrap();
    assert!(rescue_assets::r3_classify_fixture(&inventory, &path, true)
        .unwrap_err()
        .contains("mismatch"));
    assert_eq!(std::fs::read(path.join("unknown.txt")).unwrap(), b"keep");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn review4_existing_retry_budget_defers_hash_and_persists_failure_before_action() {
    use std::cell::Cell;
    let mut budget = recovery::RetryBudget {
        failures: 2,
        next_at: 500,
        healthy_since: None,
    };
    let calls = Cell::new(0);
    let saved = Cell::new(false);
    let result = rescue_recovery::budgeted(
        Duration::from_secs(180),
        &mut budget,
        499,
        |_| {
            saved.set(true);
            Ok(())
        },
        || {
            calls.set(calls.get() + 1);
            Err("bad signature".into())
        },
    );
    assert_eq!(result.unwrap(), rescue_recovery::Cycle::Deferred);
    assert_eq!(calls.get(), 0);
    assert!(!saved.get());
    let error = rescue_recovery::budgeted(
        Duration::from_secs(180),
        &mut budget,
        500,
        |next| {
            assert_eq!(next.next_at, 800);
            saved.set(true);
            Ok(())
        },
        || {
            assert!(saved.get());
            calls.set(calls.get() + 1);
            Err("bad signature".into())
        },
    )
    .unwrap_err();
    assert_eq!(error, "bad signature");
    assert_eq!(budget.failures, 3);
    assert_eq!(budget.next_at, 800);
    let result = rescue_recovery::budgeted(
        Duration::from_secs(200),
        &mut budget,
        530,
        |_| panic!("must not write"),
        || panic!("must not hash"),
    );
    assert_eq!(result.unwrap(), rescue_recovery::Cycle::Deferred);
    let previous = budget;
    let result = rescue_recovery::budgeted(
        Duration::from_secs(200),
        &mut budget,
        800,
        |_| Err("disk write denied".into()),
        || panic!("must not act without durable budget"),
    );
    assert_eq!(result.unwrap_err(), "disk write denied");
    assert_eq!(budget.failures, previous.failures);
    assert_eq!(budget.next_at, previous.next_at);
    let result = rescue_recovery::budgeted(
        Duration::from_secs(179),
        &mut budget,
        900,
        |_| panic!("startup grace must not write"),
        || panic!("startup grace must not act"),
    );
    assert_eq!(result.unwrap(), rescue_recovery::Cycle::Deferred);
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
struct ReviewObservedState {
    service_recovery_failures: u8,
    service_recovery_next_at: u64,
    service_healthy_since: Option<u64>,
}

impl ReviewObservedState {
    fn budget(&self) -> recovery::RetryBudget {
        recovery::RetryBudget {
            failures: self.service_recovery_failures,
            next_at: self.service_recovery_next_at,
            healthy_since: self.service_healthy_since,
        }
    }
    fn with_budget(b: recovery::RetryBudget) -> Self {
        Self {
            service_recovery_failures: b.failures,
            service_recovery_next_at: b.next_at,
            service_healthy_since: b.healthy_since,
        }
    }
    fn read(path: &Path) -> Self {
        serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
    }
    fn write(&self, path: &Path) -> std::io::Result<()> {
        state_store::publish(path, &serde_json::to_vec_pretty(self).unwrap())
    }
}

#[test]
fn review6_running_observation_survives_reload_resets_after_600_seconds_and_next_fault_starts_at_one(
) {
    assert!(matches!(
        std::env::var("COMPUTERNAME").unwrap().as_str(),
        "RDLAB-WIN10" | "RDLAB-WIN11"
    ));
    let root = Path::new(r"C:\RustDeskAgentLab")
        .join(format!("rescue-v200-persist-{}", uuid::Uuid::new_v4()));
    workspace::create(&root).unwrap();
    let path = root.join("agent.json");
    ReviewObservedState {
        service_recovery_failures: 4,
        service_recovery_next_at: 900,
        service_healthy_since: None,
    }
    .write(&path)
    .unwrap();
    for now in [1000, 1030, 1300, 1599, 1600] {
        let previous = ReviewObservedState::read(&path);
        let mut budget = previous.budget();
        budget.observe_health(true, now);
        let observed = ReviewObservedState::with_budget(budget);
        assert!(
            rescue_recovery::persist_before_package_skip(false, || observed.write(&path)).unwrap()
        );
        let persisted = ReviewObservedState::read(&path);
        assert_eq!(persisted, observed, "round {now}");
        if now < 1600 {
            assert_eq!(persisted.service_healthy_since, Some(1000));
            assert_eq!(persisted.service_recovery_failures, 4);
        } else {
            assert_eq!(persisted.service_healthy_since, None);
            assert_eq!(persisted.service_recovery_failures, 0);
            assert_eq!(persisted.service_recovery_next_at, 0);
        }
    }
    let mut budget = ReviewObservedState::read(&path).budget();
    budget.record_failure(1601);
    let next = ReviewObservedState::with_budget(budget);
    next.write(&path).unwrap();
    let persisted = ReviewObservedState::read(&path);
    assert_eq!(persisted.service_recovery_failures, 1);
    assert_eq!(persisted.service_recovery_next_at, 1631);
    println!("PERSISTED_HEALTH_ROUNDS=5;first=1000;reset=1600;next_fault_failures=1;next_at=1631");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn review6_observation_write_failure_retains_original_disk_and_observed_memory_without_package_bypass(
) {
    use std::os::windows::fs::OpenOptionsExt;
    let root = Path::new(r"C:\RustDeskAgentLab")
        .join(format!("rescue-v200-write-fail-{}", uuid::Uuid::new_v4()));
    workspace::create(&root).unwrap();
    let path = root.join("agent.json");
    let original = ReviewObservedState {
        service_recovery_failures: 3,
        service_recovery_next_at: 900,
        service_healthy_since: None,
    };
    original.write(&path).unwrap();
    let bytes = std::fs::read(&path).unwrap();
    let mut budget = original.budget();
    budget.observe_health(true, 1000);
    let observed = ReviewObservedState::with_budget(budget);
    let memory_before = observed.clone();
    let lock = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(&path)
        .unwrap();
    let error =
        rescue_recovery::persist_before_package_skip(false, || observed.write(&path)).unwrap_err();
    assert!(matches!(error.raw_os_error(), Some(5 | 32)), "{error}");
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    assert_eq!(ReviewObservedState::read(&path), original);
    assert_eq!(observed, memory_before);
    assert!(!path
        .with_extension(format!("tmp-{}", std::process::id()))
        .exists());
    assert!(!rescue_recovery::routing(&Err(error.to_string())).legacy_package);
    drop(lock);
    assert!(rescue_recovery::persist_before_package_skip(false, || observed.write(&path)).unwrap());
    assert_eq!(ReviewObservedState::read(&path), observed);
    assert!(
        !rescue_recovery::persist_before_package_skip(true, || panic!(
            "old path must remain untouched"
        ))
        .unwrap()
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn r4_local_trigger_cannot_encode_original_fallback_service_in_existing_plan() {
    assert!(matches!(
        std::env::var("COMPUTERNAME").unwrap().as_str(),
        "RDLAB-WIN10" | "RDLAB-WIN11"
    ));
    let fixture = Fixture::new();
    let path = fixture.tx().join("prepared.json");
    let bytes = std::fs::read(&path).unwrap();
    let mut plan = install_plan::InstallPlan::decode(&bytes).unwrap();
    plan.trigger = install_plan::Trigger::LocalRecovery;
    plan.command_id = None;
    plan.validate().unwrap();
    plan.services
        .iter_mut()
        .find(|service| service.name == "RustDesk")
        .unwrap()
        .command = r#""C:\ProgramData\RustDesk\Client\sos\RustDesk\RustDesk.exe" --service"#.into();
    assert_eq!(
        plan.validate().unwrap_err(),
        "installation original service target mismatch"
    );
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    println!("R4_BLOCKER=LocalRecovery trigger exists but original fallback service is rejected by plan contract");
}

fn local_plan_fixture() -> local_repair_plan::LocalRepairPlan {
    let asset = |name: &str| install_plan::Artifact {
        relative_path: name.into(),
        size: 4,
        sha256: "a".repeat(64),
    };
    local_repair_plan::LocalRepairPlan{schema:1,trigger:"local_recovery".into(),transaction_id:"a".repeat(32),installation_id:"c".repeat(32),target_key:"windows-x86_64-msi-sos".into(),build_seq:2026100502,version:"1.5.3".into(),original_normal:"missing".into(),fallback_path:r"C:\ProgramData\RustDesk\Client\sos\RustDesk".into(),fallback_command:r#""C:\ProgramData\RustDesk\Client\sos\RustDesk\RustDesk.exe" --service"#.into(),fallback_start:"auto".into(),agent_command:r#"C:\ProgramData\RustDesk\ManagementAgent\rustdesk-management-agent.exe --service --edition sos --package-kind msi --normal-install-path "C:\Program Files\RustDesk""#.into(),agent_start:"auto".into(),identity_path:r"C:\Windows\ServiceProfiles\LocalService\AppData\Roaming\RustDesk\config\RustDesk.toml".into(),product_code:"{71A6889B-6003-42DB-989F-B7AE68260DDB}".into(),source_transaction:"b".repeat(32),approval_source:None,total_loss:None,created_at:100,package:asset("agent-sos-v187.msi"),client_inventory:asset("client-inventory.json"),agent_backup:asset("agent-backup.exe"),identity_backup:asset("identity-backup.toml"),config_backup:asset("config-backup.toml"),bootstrap_backup:asset("bootstrap-backup.exe"),bootstrap_manifest:asset("bootstrap-backup.json"),helper:asset("runtime/helper.exe")}
}

fn freeze_v225_fixture() -> (
    Fixture,
    PathBuf,
    local_repair_plan::LocalRepairPlan,
    UpdateManifest,
    Vec<u8>,
    Vec<u8>,
) {
    let f = Fixture::new();
    let source = f.tx();
    let raw: serde_json::Value =
        serde_json::from_slice(&std::fs::read(source.join("approved-manifest.json")).unwrap())
            .unwrap();
    let mut sparse = raw.clone();
    sparse.as_object_mut().unwrap().remove("catalog_revision");
    let target = sparse["targets"]["windows-x86_64-msi-sos"]
        .as_object_mut()
        .unwrap();
    for name in ["mirrors", "source_tag"] {
        target.remove(name);
    }
    let bytes = serde_json::to_vec_pretty(&sparse).unwrap();
    std::fs::write(source.join("approved-manifest.json"), &bytes).unwrap();
    let manifest: UpdateManifest = serde_json::from_value(sparse.clone()).unwrap();
    package_gate::verify_with_key(
        &manifest,
        "windows-x86_64-msi-sos",
        2026100502,
        "fixture-key",
        f.public.as_ref(),
    )
    .unwrap();
    let reference = package_gate::package_reference(&sparse, "windows-x86_64-msi-sos").unwrap();
    assert_ne!(
        reference,
        package_gate::package_reference(
            &serde_json::to_value(&manifest).unwrap(),
            "windows-x86_64-msi-sos"
        )
        .unwrap()
    );
    let identity = std::fs::read(f.root.join("current/identity.toml")).unwrap();
    let ih = hex::encode(Sha256::digest(&identity));
    let mut plan = local_plan_fixture();
    plan.created_at = 200;
    plan.identity_backup.size = identity.len() as u64;
    plan.identity_backup.sha256 = ih.clone();
    let target = &manifest.targets["windows-x86_64-msi-sos"];
    plan.package = install_plan::Artifact {
        relative_path: "agent-sos-v187.msi".into(),
        size: target.size,
        sha256: target.sha256.clone(),
    };
    plan.source_transaction = String::new();
    let source_id = hex::encode(Sha256::digest(format!(
        "installed-msi-v1\n{}\n{reference}\n{ih}",
        plan.installation_id
    )));
    let receipt = serde_json::json!({"schema":1,"source_kind":"installed_msi","source_id":source_id,"installation_id":plan.installation_id,"target_key":plan.target_key,"source_reference":reference,"build_seq":plan.build_seq,"product_code":plan.product_code,"registered_source":source.join("agent-sos-v187.msi"),"package":plan.package,"identity_sha256":ih,"client_pid":1,"agent_pid":2,"observed_at":100});
    let receipt_bytes = serde_json::to_vec(&receipt).unwrap();
    std::fs::write(source.join("installation-receipt.json"), &receipt_bytes).unwrap();
    plan.approval_source = Some(local_repair_plan::ApprovalSource {
        source_kind: "installed_msi".into(),
        source_id,
        source_reference: reference,
        installation_receipt: install_plan::Artifact {
            relative_path: "installation-receipt.json".into(),
            size: receipt_bytes.len() as u64,
            sha256: hex::encode(Sha256::digest(&receipt_bytes)),
        },
    });
    let directory = f.root.join("freeze-execution");
    workspace::create(&directory).unwrap();
    std::fs::copy(
        source.join("agent-sos-v187.msi"),
        directory.join("agent-sos-v187.msi"),
    )
    .unwrap();
    std::fs::write(directory.join("identity-backup.toml"), &identity).unwrap();
    std::fs::write(directory.join("installation-receipt.json"), &receipt_bytes).unwrap();
    (f, directory, plan, manifest, bytes, identity)
}

#[test]
fn freeze_v225_sparse_signed_manifest_prepare_reopen_preserves_approved_bytes() {
    let (f, directory, plan, manifest, approved, identity) = freeze_v225_fixture();
    local_repair::freeze_source_fixture(&directory, &f.tx(), &manifest, &plan).unwrap();
    local_repair::persist_source_fixture(&directory, &plan, &identity, f.public.as_ref()).unwrap();
    let reopened = local_repair_plan::LocalRepairPlan::decode(
        &std::fs::read(directory.join("local-repair-plan.json")).unwrap(),
    )
    .unwrap();
    reopened
        .verify_approval_source(&directory, &directory, &identity, Some(f.public.as_ref()))
        .unwrap();
    assert_eq!(
        std::fs::read(directory.join("approved-manifest.json")).unwrap(),
        approved
    );
    assert_eq!(
        std::fs::read(f.tx().join("approved-manifest.json")).unwrap(),
        approved
    );
    assert_eq!(reopened.approval_source, plan.approval_source);
    assert_eq!(
        transaction::Transaction::decode(
            &std::fs::read(directory.join("transaction.json")).unwrap()
        )
        .unwrap()
        .phase,
        transaction::Phase::PackageVerified
    );
}

#[test]
fn freeze_v225_wrong_reference_rejected_before_frozen_or_transaction_write() {
    let (f, directory, mut plan, manifest, approved, identity) = freeze_v225_fixture();
    plan.approval_source.as_mut().unwrap().source_reference = "d".repeat(64);
    let result = local_repair::freeze_source_fixture(&directory, &f.tx(), &manifest, &plan);
    assert!(result.unwrap_err().contains("reference mismatch"));
    assert!(!directory.join("transaction.json").exists());
    assert!(!directory.join("approved-manifest.json").exists());
    assert_eq!(
        std::fs::read(f.tx().join("approved-manifest.json")).unwrap(),
        approved
    );
    assert_eq!(
        std::fs::read(directory.join("identity-backup.toml")).unwrap(),
        identity
    );
}

#[test]
fn freeze_v225_mixed_source_preflight_keeps_old_data_and_no_execution_record() {
    let (f, directory, mut plan, manifest, approved, identity) = freeze_v225_fixture();
    local_repair::freeze_source_fixture(&directory, &f.tx(), &manifest, &plan).unwrap();
    plan.source_transaction = "b".repeat(32);
    assert!(
        local_repair::persist_source_fixture(&directory, &plan, &identity, f.public.as_ref())
            .unwrap_err()
            .contains("mixed approval source")
    );
    assert!(!directory.join("transaction.json").exists());
    assert!(!directory.join("local-repair-plan.json").exists());
    assert_eq!(
        std::fs::read(f.tx().join("approved-manifest.json")).unwrap(),
        approved
    );
    assert_eq!(
        std::fs::read(directory.join("identity-backup.toml")).unwrap(),
        identity
    );
}

#[test]
fn freeze_v225_schema1_keeps_existing_typed_serialization() {
    let (f, directory, mut plan, manifest, _, _) = freeze_v225_fixture();
    plan.approval_source = None;
    plan.source_transaction = "b".repeat(32);
    local_repair::freeze_source_fixture(&directory, &f.tx(), &manifest, &plan).unwrap();
    assert_eq!(
        std::fs::read(directory.join("approved-manifest.json")).unwrap(),
        serde_json::to_vec(&manifest).unwrap()
    );
    assert!(!directory.join("transaction.json").exists());
}

#[test]
fn freeze_v225_invalid_receipt_preflight_does_not_persist_package_verified_transaction() {
    let (f, directory, plan, manifest, approved, identity) = freeze_v225_fixture();
    local_repair::freeze_source_fixture(&directory, &f.tx(), &manifest, &plan).unwrap();
    std::fs::write(directory.join("installation-receipt.json"), b"{}").unwrap();
    assert!(
        local_repair::persist_source_fixture(&directory, &plan, &identity, f.public.as_ref())
            .unwrap_err()
            .contains("receipt digest mismatch")
    );
    assert!(!directory.join("transaction.json").exists());
    assert!(!directory.join("local-repair-plan.json").exists());
    assert_eq!(
        std::fs::read(directory.join("installation-receipt.json")).unwrap(),
        b"{}"
    );
    assert_eq!(
        std::fs::read(f.tx().join("approved-manifest.json")).unwrap(),
        approved
    );
}

#[test]
fn source_v224_fixed_schema2_approval_exports_without_creating_execution_transaction() {
    assert_eq!(std::env::var("COMPUTERNAME").unwrap(), "RDLAB-WIN11");
    let root = Path::new(r"C:\ProgramData\RustDesk\ManagementAgent\rescue\current");
    let before = std::fs::read(root.join("bundle.json")).unwrap();
    let bundle: serde_json::Value = serde_json::from_slice(&before).unwrap();
    assert_eq!(bundle["schema"], 2);
    let public = rescue_assets::fixed_source_public_fixture(&bundle).unwrap();
    let (_, package, _, build, _, installation, source_tx, approval) =
        rescue_assets::local_repair_source(Some(&public)).unwrap();
    assert!(source_tx.is_empty());
    let approval = approval.unwrap();
    assert_eq!(approval.source_kind, "installed_msi");
    assert_eq!(approval.source_reference, bundle["source_reference"]);
    assert_eq!(build, bundle["build_seq"]);
    assert_eq!(installation, bundle["installation_id"]);
    assert_eq!(
        package,
        root.join(bundle["package"]["relative_path"].as_str().unwrap())
    );
    approval.installation_receipt.verify(root).unwrap();
    let wrong = sign::gen_keypair().0;
    assert!(rescue_assets::local_repair_source(Some(wrong.as_ref()))
        .unwrap_err()
        .contains("signature"));
    assert_eq!(std::fs::read(root.join("bundle.json")).unwrap(), before);
}

#[test]
fn source_v224_fixed_receipt_freezes_into_temp_execution_and_scheduler_keeps_source_separate() {
    assert_eq!(std::env::var("COMPUTERNAME").unwrap(), "RDLAB-WIN11");
    let root = Path::new(r"C:\ProgramData\RustDesk\ManagementAgent\rescue\current");
    let original = std::fs::read(root.join("bundle.json")).unwrap();
    let bundle: serde_json::Value = serde_json::from_slice(&original).unwrap();
    let public = rescue_assets::fixed_source_public_fixture(&bundle).unwrap();
    let (_, package, _, build, _, installation, source_tx, source) =
        rescue_assets::local_repair_source(Some(&public)).unwrap();
    assert!(source_tx.is_empty());
    let source = source.unwrap();
    let (scheduled_source, attempt) = local_repair_schedule::installed_approval_fixture().unwrap();
    assert_eq!(scheduled_source, source.source_id);
    assert_eq!(attempt.len(), 32);
    assert_ne!(attempt, source.source_id);
    let temporary =
        std::env::temp_dir().join(format!("local-source-v224-fixed-{}", uuid::Uuid::new_v4()));
    workspace::create(&temporary).unwrap();
    let identity = std::fs::read(hbb_common::config::Config::file()).unwrap();
    let identity_hash = hex::encode(Sha256::digest(&identity));
    let mut plan = local_plan_fixture();
    plan.transaction_id = uuid::Uuid::new_v4().simple().to_string();
    plan.installation_id = installation;
    plan.build_seq = build;
    plan.created_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    plan.source_transaction = String::new();
    plan.approval_source = Some(source);
    plan.package = serde_json::from_value(bundle["package"].clone()).unwrap();
    plan.identity_backup.size = identity.len() as u64;
    plan.identity_backup.sha256 = identity_hash.clone();
    std::fs::copy(package, temporary.join(&plan.package.relative_path)).unwrap();
    for name in ["installation-receipt.json", "approved-manifest.json"] {
        std::fs::copy(root.join(name), temporary.join(name)).unwrap();
    }
    std::fs::write(temporary.join("identity-backup.toml"), &identity).unwrap();
    let tx = transaction::Transaction::new(
        plan.transaction_id.clone(),
        plan.target_key.clone(),
        plan.package.sha256.clone(),
        identity_hash,
        plan.created_at,
    )
    .unwrap();
    save(&temporary.join("transaction.json"), &tx);
    let before = std::fs::read(temporary.join("transaction.json")).unwrap();
    plan.verify_approval_source(
        &temporary,
        Path::new(r"C:\ProgramData\RustDesk\ManagementAgent\state"),
        &identity,
        Some(&public),
    )
    .unwrap();
    assert_eq!(
        std::fs::read(temporary.join("transaction.json")).unwrap(),
        before
    );
    assert_eq!(tx.phase, transaction::Phase::Planned);
    assert!(plan
        .verify_approval_source(
            &temporary,
            Path::new(r"C:\ProgramData\RustDesk\ManagementAgent\state"),
            &identity,
            None
        )
        .unwrap_err()
        .contains("signing key"));
    assert_eq!(std::fs::read(root.join("bundle.json")).unwrap(), original);
    assert_eq!(
        std::fs::read(hbb_common::config::Config::file()).unwrap(),
        identity
    );
    std::fs::remove_dir_all(temporary).unwrap();
}

#[test]
fn localrepair_plan_records_real_missing_normal_and_fixed_fallback() {
    let plan = local_plan_fixture();
    plan.validate().unwrap();
    for change in 0..5 {
        let mut invalid = plan.clone();
        match change {
            0 => invalid.original_normal = "present".into(),
            1 => invalid.fallback_path = r"C:\evil".into(),
            2 => invalid.target_key = "windows-x86_64-msi-standard".into(),
            3 => invalid.fallback_command.push_str(" --evil"),
            _ => invalid.identity_path = r"C:\elsewhere.toml".into(),
        };
        assert!(invalid.validate().is_err());
    }
}

#[test]
fn autoschedule_decision_requires_missing_normal_optin_due_idle_and_no_prior_source() {
    use local_repair_schedule::{decide, Decision};
    assert_eq!(decide(false, true, true, true, None), Decision::Observe);
    for (allowed, due, idle) in [
        (false, true, true),
        (true, false, true),
        (true, true, false),
    ] {
        assert_eq!(decide(true, allowed, due, idle, None), Decision::Deferred);
    }
    assert_eq!(
        decide(true, true, true, true, Some("a".repeat(32))),
        Decision::Existing("a".repeat(32))
    );
    assert_eq!(decide(true, true, true, true, None), Decision::Prepare);
}

#[test]
fn rescuestatus_freshness_24h_boundary_and_clock_rollback_are_explicit() {
    assert_eq!(rescue_status::freshness(100, 100), "fresh");
    assert_eq!(rescue_status::freshness(100, 86499), "fresh");
    assert_eq!(rescue_status::freshness(100, 86500), "stale");
    assert_eq!(rescue_status::freshness(100, 99), "clock_rollback");
    assert_eq!(rescue_status::freshness(0, 100), "unknown");
}

#[test]
fn firstinstall_without_approved_sidecar_waits_without_creating_bundle_or_transaction() {
    let root = Path::new(r"C:\RustDeskAgentLab")
        .join(format!("firstinstall-v220-{}", uuid::Uuid::new_v4()));
    workspace::create(&root).unwrap();
    let value = rescue_assets::first_install::prepare_fixture(&root, &[0; 32]).unwrap();
    assert_eq!(value["state"], "unavailable");
    assert_eq!(value["error_code"], "waiting_approval");
    assert!(!root.join("rescue/current").exists());
    assert!(!root.join("transactions").exists());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn firstinstall_windows_installer_acl_accepts_system_writer_and_refuses_users_write() {
    let root = Path::new(r"C:\RustDeskAgentLab")
        .join(format!("firstinstall-acl-{}", uuid::Uuid::new_v4()));
    workspace::create(&root).unwrap();
    assert!(std::process::Command::new("icacls.exe")
        .arg(&root)
        .args([
            "/grant",
            "*S-1-5-80-956008885-3418522649-1831038044-1853292631-2271478464:(OI)(CI)(F)"
        ])
        .status()
        .unwrap()
        .success());
    rescue_assets::first_install::installed_acl_fixture(&root).unwrap();
    assert!(std::process::Command::new("icacls.exe")
        .arg(&root)
        .args(["/grant", "*S-1-5-32-545:(OI)(CI)(W)"])
        .status()
        .unwrap()
        .success());
    assert!(rescue_assets::first_install::installed_acl_fixture(&root)
        .unwrap_err()
        .contains("non-admin write"));
    std::fs::remove_dir_all(root).unwrap();
}

fn firstinstall_fixture() -> Fixture {
    let f = Fixture::new();
    let cache = f.root.join("release-cache");
    workspace::create(&cache).unwrap();
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(f.tx().join("approved-manifest.json")).unwrap())
            .unwrap();
    let package = cache.join("agent-sos-v187.msi");
    std::fs::copy(f.tx().join("agent-sos-v187.msi"), &package).unwrap();
    let metadata = msi_metadata::inspect(&package).unwrap();
    save(
        &cache.join("first-install.json"),
        &serde_json::json!({"schema":1,"product_code":metadata.product_code,"manifest":manifest}),
    );
    std::fs::remove_dir_all(f.root.join("transactions")).unwrap();
    f
}

#[test]
fn firstreview_incompatible_fallback_keeps_unsigned_route_and_original_identity() {
    let f = firstinstall_fixture();
    let fallback = f.root.join("fallback");
    directory_backup::capture(
        &f.root.join("current/client/files"),
        &f.root.join("fallback-copy"),
    )
    .unwrap();
    std::fs::rename(f.root.join("fallback-copy/files"), &fallback).unwrap();
    std::fs::write(fallback.join("librustdesk.dll"), b"previous approved A dll").unwrap();
    let old = directory_backup::capture(&fallback, &f.root.join("old-fallback-evidence")).unwrap();
    let identity = std::fs::read(f.root.join("current/identity.toml")).unwrap();
    let result = rescue_assets::first_install::prepare_fixture(&f.root, f.public.as_ref()).unwrap();
    assert_eq!(result["state"], "degraded");
    assert_eq!(result["blocked_reason"], "existing_fallback_incompatible");
    assert!(
        !f.root.join("rescue/current").exists(),
        "must not enable signed route over usable old fallback"
    );
    assert!(old.matches_directory(&fallback).unwrap());
    assert_eq!(
        std::fs::read(f.root.join("current/identity.toml")).unwrap(),
        identity
    );
    std::fs::rename(
        f.root.join("current/client/files"),
        f.root.join("normal-preserved"),
    )
    .unwrap();
    assert!(rescue_assets::r3_normal_missing(&f.root.join("current/client/files")).unwrap());
    assert!(!f.root.join("rescue/current").exists());
    assert!(old.matches_directory(&fallback).unwrap());
}

#[test]
fn firstreview_matching_fallback_publishes_and_final_change_blocks_without_overwrite() {
    for change in [false, true] {
        let f = firstinstall_fixture();
        let fallback = f.root.join("fallback");
        directory_backup::capture(
            &f.root.join("current/client/files"),
            &f.root.join("fallback-copy"),
        )
        .unwrap();
        std::fs::rename(f.root.join("fallback-copy/files"), &fallback).unwrap();
        let identity = std::fs::read(f.root.join("current/identity.toml")).unwrap();
        let result = rescue_assets::first_install::fallback_change_fixture(
            &f.root,
            f.public.as_ref(),
            change,
        )
        .unwrap();
        if change {
            assert_eq!(result["blocked_reason"], "existing_fallback_incompatible");
            assert!(!f.root.join("rescue/current").exists());
            assert_eq!(
                std::fs::read(fallback.join("librustdesk.dll")).unwrap(),
                b"changed before publication"
            );
        } else {
            assert_eq!(result["rescue_readiness"], "degraded");
            assert!(f.root.join("rescue/current").exists());
            let inventory = directory_backup::DirectoryBackup::decode(
                &std::fs::read(f.root.join("rescue/current/client/manifest.json")).unwrap(),
            )
            .unwrap();
            assert!(rescue_assets::r3_classify_fixture(&inventory, &fallback, true).unwrap());
        }
        assert_eq!(
            std::fs::read(f.root.join("current/identity.toml")).unwrap(),
            identity
        );
    }
}

#[test]
fn firstreview_fixture_source_missing_or_mixed_is_rejected() {
    let agent = Path::new(r"C:\ProgramData\RustDesk\ManagementAgent");
    for bundle in [
        serde_json::json!({}),
        serde_json::json!({"schema":1}),
        serde_json::json!({"schema":2,"source_transaction":"a".repeat(32)}),
        serde_json::json!({"schema":2,"source_kind":"installed_msi","source_transaction":null}),
    ] {
        assert!(rescue_assets::fixture_source_key_path(agent, &bundle)
            .unwrap_err()
            .contains("source"));
    }
    let installed = serde_json::json!({"schema":2,"source_kind":"installed_msi","installation_receipt":{"relative_path":"installation-receipt.json"}});
    assert_eq!(
        rescue_assets::fixture_source_key_path(agent, &installed).unwrap(),
        agent.join("release-cache/fixture-public.bin")
    );
    assert_eq!(
        rescue_assets::fixture_source_key_path(
            agent,
            &serde_json::json!({"schema":1,"source_transaction":"a".repeat(32)})
        )
        .unwrap(),
        agent
            .join("transactions")
            .join("a".repeat(32))
            .join("rescue-fixture-public.bin")
    );
}

#[test]
fn firstreview_unknown_occupied_fallback_refuses_and_status_preserves_route() {
    let (root, inventory) = review_inventory();
    let fallback = root.join("fallback");
    rescue_assets::first_install::fallback_compatibility_fixture(&root, &inventory).unwrap();
    std::fs::write(&fallback, b"unknown occupied object").unwrap();
    assert!(
        rescue_assets::first_install::fallback_compatibility_fixture(&root, &inventory)
            .unwrap_err()
            .contains("directory")
    );
    assert_eq!(
        std::fs::read(&fallback).unwrap(),
        b"unknown occupied object"
    );
    std::fs::remove_file(&fallback).unwrap();
    std::os::windows::fs::symlink_dir(root.join("normal"), &fallback).unwrap();
    assert!(
        rescue_assets::first_install::fallback_compatibility_fixture(&root, &inventory)
            .unwrap_err()
            .contains("reparse")
    );
    assert_eq!(
        std::fs::read(root.join("normal/RustDesk.exe")).unwrap(),
        b"approved A executable"
    );
    std::fs::remove_dir(&fallback).unwrap();
    let reports = root.join("first-install-status");
    workspace::create(&reports).unwrap();
    save(
        &reports.join("last-check.json"),
        &serde_json::json!({"state":"degraded","blocked_reason":"existing_fallback_incompatible"}),
    );
    let status = rescue_status::snapshot(&root, &"c".repeat(32), "windows-x86_64-msi-sos");
    assert_eq!(status["state"], "degraded");
    assert_eq!(status["legacy_route_preserved"], true);
    assert_eq!(status["components"]["client"]["state"], "unknown");
    assert_eq!(status["ready"], false);
    assert!(!root.join("rescue/current").exists());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn firstinstall_approved_installed_without_transaction_creates_independent_receipt_and_is_idempotent(
) {
    let f = firstinstall_fixture();
    let identity = std::fs::read(f.root.join("current/identity.toml")).unwrap();
    let result = rescue_assets::first_install::prepare_fixture(&f.root, f.public.as_ref()).unwrap();
    assert_eq!(result["rescue_readiness"], "degraded");
    assert_eq!(result["source_kind"], "installed_msi");
    assert!(result["source_transaction"].is_null());
    assert!(!f.root.join("transactions").exists());
    let current = f.root.join("rescue/current");
    let before = std::fs::read(current.join("bundle.json")).unwrap();
    let receipt: serde_json::Value =
        serde_json::from_slice(&std::fs::read(current.join("installation-receipt.json")).unwrap())
            .unwrap();
    assert_eq!(receipt["source_kind"], "installed_msi");
    assert_eq!(receipt["installation_id"], "c".repeat(32));
    assert!(receipt.get("phase").is_none());
    assert_eq!(f.verify().unwrap()["client"], "verified");
    rescue_assets::first_install::prepare_fixture(&f.root, f.public.as_ref()).unwrap();
    assert_eq!(std::fs::read(current.join("bundle.json")).unwrap(), before);
    assert_eq!(
        std::fs::read(f.root.join("current/identity.toml")).unwrap(),
        identity
    );
    assert!(
        rescue_assets::after_commit_fixture(&current, f.public.as_ref())
            .unwrap_err()
            .contains("outside fixed transactions")
    );
    let receipt_path = current.join("installation-receipt.json");
    let receipt_before = std::fs::read(&receipt_path).unwrap();
    std::fs::write(&receipt_path, b"{}").unwrap();
    assert!(f.verify().unwrap_err().contains("mismatch"));
    assert_eq!(std::fs::read(current.join("bundle.json")).unwrap(), before);
    std::fs::write(&receipt_path, receipt_before).unwrap();
    let state = std::fs::read(f.root.join("state/agent.json")).unwrap();
    let mut altered: serde_json::Value = serde_json::from_slice(&state).unwrap();
    altered["agent_installation_id"] = "d".repeat(32).into();
    save(&f.root.join("state/agent.json"), &altered);
    assert!(
        rescue_assets::first_install::prepare_fixture(&f.root, f.public.as_ref())
            .unwrap_err()
            .contains("identity mismatch")
    );
    assert_eq!(std::fs::read(current.join("bundle.json")).unwrap(), before);
    std::fs::write(f.root.join("state/agent.json"), state).unwrap();
}

#[test]
fn firstinstall_approval_and_local_gates_reject_before_writes() {
    let f = firstinstall_fixture();
    let cache = f.root.join("release-cache/first-install.json");
    let approved = std::fs::read(&cache).unwrap();
    let identity = std::fs::read(f.root.join("current/identity.toml")).unwrap();
    for marker in ["disable-agent", "disable-recovery", "maintenance.json"] {
        let path = f.root.join("state").join(marker);
        std::fs::write(&path, b"hold").unwrap();
        assert!(
            rescue_assets::first_install::prepare_fixture(&f.root, f.public.as_ref())
                .unwrap_err()
                .contains("blocked")
        );
        assert!(!f.root.join("rescue").exists());
        assert_eq!(std::fs::read(&path).unwrap(), b"hold");
        std::fs::remove_file(path).unwrap();
    }
    let mut invalid: serde_json::Value = serde_json::from_slice(&approved).unwrap();
    invalid["manifest"]["targets"]["windows-x86_64-msi-sos"]["signature"] =
        STANDARD.encode([0u8; 64]).into();
    save(&cache, &invalid);
    assert!(
        rescue_assets::first_install::prepare_fixture(&f.root, f.public.as_ref())
            .unwrap_err()
            .contains("signature")
    );
    assert!(!f.root.join("rescue").exists());
    std::fs::write(&cache, &approved).unwrap();
    assert!(
        rescue_assets::first_install::prepare_fixture(&f.root, &[0; 32])
            .unwrap_err()
            .contains("signature")
    );
    assert_eq!(std::fs::read(&cache).unwrap(), approved);
    assert_eq!(
        std::fs::read(f.root.join("current/identity.toml")).unwrap(),
        identity
    );
    assert!(!f.root.join("transactions").exists());
    let config = f.root.join("current/RustDesk2.toml");
    let saved_config = std::fs::read(&config).unwrap();
    std::fs::write(&config, b"[options]\nstop-service='Y'\n").unwrap();
    assert!(
        rescue_assets::first_install::prepare_fixture(&f.root, f.public.as_ref())
            .unwrap_err()
            .contains("user stopped")
    );
    assert!(!f.root.join("rescue").exists());
    std::fs::write(&config, saved_config).unwrap();
    let sidecar = f.root.join("release-cache/first-install.saved");
    std::fs::rename(&cache, &sidecar).unwrap();
    std::os::windows::fs::symlink_file(&sidecar, &cache).unwrap();
    assert!(
        rescue_assets::first_install::prepare_fixture(&f.root, f.public.as_ref())
            .unwrap_err()
            .contains("reparse")
    );
    assert!(!f.root.join("rescue").exists());
    std::fs::remove_file(&cache).unwrap();
    std::fs::rename(&sidecar, &cache).unwrap();
    workspace::create(&f.root.join("rescue")).unwrap();
    workspace::create(&f.root.join("rescue/previous")).unwrap();
    std::fs::write(f.root.join("rescue/previous/keep"), b"original").unwrap();
    assert!(
        rescue_assets::first_install::prepare_fixture(&f.root, f.public.as_ref())
            .unwrap_err()
            .contains("occupied")
    );
    assert_eq!(
        std::fs::read(f.root.join("rescue/previous/keep")).unwrap(),
        b"original"
    );
    assert!(!f.root.join("rescue/current").exists());
}

#[test]
fn firstinstall_bundle_can_be_retained_as_previous_by_committed_rotation() {
    let f = Fixture::new();
    let cache = f.root.join("release-cache");
    workspace::create(&cache).unwrap();
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(f.tx().join("approved-manifest.json")).unwrap())
            .unwrap();
    let package = cache.join("agent-sos-v187.msi");
    std::fs::copy(f.tx().join("agent-sos-v187.msi"), &package).unwrap();
    let metadata = msi_metadata::inspect(&package).unwrap();
    save(
        &cache.join("first-install.json"),
        &serde_json::json!({"schema":1,"product_code":metadata.product_code,"manifest":manifest}),
    );
    let saved = f.root.join("saved-committed-fixture");
    std::fs::rename(f.tx(), &saved).unwrap();
    std::fs::remove_dir(f.root.join("transactions")).unwrap();
    rescue_assets::first_install::prepare_fixture(&f.root, f.public.as_ref()).unwrap();
    let old = std::fs::read(f.root.join("rescue/current/bundle.json")).unwrap();
    workspace::create(&f.root.join("transactions")).unwrap();
    std::fs::rename(saved, f.tx()).unwrap();
    let result = f.prepare().unwrap();
    assert_eq!(result["source_kind"], "committed_transaction");
    assert_eq!(result["source_transaction"], "a".repeat(32));
    assert_eq!(
        std::fs::read(f.root.join("rescue/previous/bundle.json")).unwrap(),
        old
    );
    let receipt: serde_json::Value =
        serde_json::from_slice(&std::fs::read(f.tx().join("rotation.json")).unwrap()).unwrap();
    assert_eq!(receipt["phase"], "published");
    assert!(
        rescue_assets::first_install::prepare_fixture(&f.root, f.public.as_ref())
            .unwrap_err()
            .contains("preserved")
    );
    assert_eq!(
        std::fs::read(f.root.join("rescue/previous/bundle.json")).unwrap(),
        old
    );
}

#[test]
#[ignore = "Win11 clean ordinary SOS installation; automatic worker with fixture approval only"]
fn firstinstall_clean_installed_worker_prepares_signed_bundle_without_transaction() {
    use std::time::{Duration, Instant};
    assert_eq!(std::env::var("COMPUTERNAME").unwrap(), "RDLAB-WIN11");
    assert_eq!(std::env::var("RUSTDESK_FIRST_INSTALL_LAB").unwrap(), "v220");
    let agent = Path::new(r"C:\ProgramData\RustDesk\ManagementAgent");
    let current = agent.join("rescue/current");
    assert!(!current.exists());
    assert!(!agent.join("rescue/previous").exists());
    if std::env::var("RUSTDESK_FIRST_INSTALL_RESUME").as_deref() == Ok("v221") {
        let status: serde_json::Value = serde_json::from_slice(
            &std::fs::read(agent.join("first-install-status/last-check.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(
            status["error"],
            "rescue directory non-admin write permission refused"
        );
        let archive = agent.join("first-install-v220-acl-refused");
        assert!(!archive.exists());
        workspace::create(&archive).unwrap();
        for (from, to) in [
            ("rescue/first-install-staging", "staging"),
            ("release-cache", "release-cache"),
            ("first-install-status", "status"),
        ] {
            std::fs::rename(agent.join(from), archive.join(to)).unwrap();
        }
    }
    assert!(
        !agent.join("transactions").exists()
            || std::fs::read_dir(agent.join("transactions"))
                .unwrap()
                .next()
                .is_none()
    );
    let identity = hbb_common::config::Config::file();
    let original = std::fs::read(&identity).unwrap();
    let state_bytes = std::fs::read(agent.join("state/agent.json")).unwrap();
    let state: serde_json::Value = serde_json::from_slice(&state_bytes).unwrap();
    let installation = state["agent_installation_id"].as_str().unwrap();
    let key = state["target_key"].as_str().unwrap();
    assert_eq!(key, "windows-x86_64-msi-sos");
    let files = [
        agent.join("rustdesk-management-agent.exe"),
        PathBuf::from(r"C:\ProgramData\RustDesk\RecoveryBootstrap\rustdesk-recovery-bootstrap.exe"),
        PathBuf::from(r"C:\Program Files\RustDesk\librustdesk.dll"),
        PathBuf::from(r"C:\RustDeskAgentLab\agent-sos-v187.msi"),
    ];
    let hashes = files
        .iter()
        .map(|p| hex::encode(Sha256::digest(std::fs::read(p).unwrap())))
        .collect::<Vec<_>>();
    assert_eq!(
        hashes[3],
        "b912548e12248e8950abae312afba156ac4b9aab7ee07b8c0a72b988855813a9"
    );
    let package = msi_metadata::inspect(&files[3]).unwrap();
    let source = msi_metadata::installed_source(&package.product_code).unwrap();
    assert_eq!(source.source, files[3]);
    let (public, secret) = sign::gen_keypair();
    let bytes = std::fs::read(&files[3]).unwrap();
    let mut manifest = UpdateManifest {
        schema: 2,
        product: "rustdesk-yan".into(),
        edition: "multi".into(),
        channel: "stable".into(),
        ..Default::default()
    };
    let mut target = UpdateTarget {
        version: "1.5.3".into(),
        build_seq: 2026100502,
        source_commit: "fixture".into(),
        primary: "https://download.yan.life/rustdesk/stable/1.5.3/agent-sos-v187.msi".into(),
        size: bytes.len() as u64,
        sha256: hashes[3].clone(),
        signature_key_id: "fixture-key".into(),
        ..Default::default()
    };
    target.signature = STANDARD.encode(sign::sign_detached(
        &base::update::update_signature_payload(&manifest, key, &target).unwrap(),
        &secret,
    ));
    manifest.targets.insert(key.into(), target);
    let cache = agent.join("release-cache");
    assert!(!cache.exists());
    workspace::create(&cache).unwrap();
    save(
        &cache.join("first-install.json"),
        &serde_json::json!({"schema":1,"product_code":package.product_code,"manifest":manifest}),
    );
    // Production authority must refuse the same test sidecar before the test-key worker starts.
    let production = rescue_assets::first_install::start().unwrap();
    production.notify().unwrap();
    let status = agent.join("first-install-status/last-check.json");
    let deadline = Instant::now() + Duration::from_secs(30);
    while !status.exists() {
        assert!(Instant::now() < deadline, "production refusal timed out");
        std::thread::sleep(Duration::from_millis(100));
    }
    let refusal: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&status).unwrap()).unwrap();
    assert_eq!(refusal["state"], "unavailable");
    assert!(!current.exists());
    drop(production);
    let worker = rescue_assets::first_install::start_fixture(public.as_ref()).unwrap();
    worker.notify().unwrap();
    let verifier = rescue_status::start_fixture(public.as_ref()).unwrap();
    let deadline = Instant::now() + Duration::from_secs(900);
    let report = loop {
        assert!(
            Instant::now() < deadline,
            "automatic preparation timeout: {:?}",
            std::fs::read_to_string(&status)
        );
        if let Ok(bytes) = std::fs::read(&status) {
            if let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) {
                if value["error_code"] == "preparation_refused"
                    && value["error"] != refusal["error"]
                {
                    panic!("automatic worker refused: {value}");
                }
            }
        }
        verifier
            .notify(rescue_status::Tick {
                installation: installation.into(),
                target_key: key.into(),
            })
            .unwrap();
        let value = rescue_status::snapshot_fixture(
            &current,
            &agent.join("rescue-status"),
            installation,
            key,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
        );
        if value["state"] == "degraded" && value["components"]["client"]["state"] == "verified" {
            break value;
        }
        std::thread::sleep(Duration::from_secs(1));
    };
    let before = std::fs::read(current.join("bundle.json")).unwrap();
    let bundle: serde_json::Value = serde_json::from_slice(&before).unwrap();
    assert_eq!(bundle["source_kind"], "installed_msi");
    assert!(bundle.get("source_transaction").is_none());
    worker.notify().unwrap();
    drop(worker);
    let restarted = rescue_assets::first_install::start_fixture(public.as_ref()).unwrap();
    restarted.notify().unwrap();
    std::thread::sleep(Duration::from_secs(2));
    drop(restarted);
    drop(verifier);
    assert_eq!(std::fs::read(current.join("bundle.json")).unwrap(), before);
    assert_eq!(std::fs::read(&identity).unwrap(), original);
    let after: serde_json::Value =
        serde_json::from_slice(&std::fs::read(agent.join("state/agent.json")).unwrap()).unwrap();
    assert_eq!(after["agent_installation_id"], installation);
    for (path, hash) in files.iter().zip(&hashes) {
        assert_eq!(
            &hex::encode(Sha256::digest(std::fs::read(path).unwrap())),
            hash
        );
    }
    assert_eq!(
        msi_metadata::installed_source(&package.product_code)
            .unwrap()
            .source,
        source.source
    );
    assert!(
        !agent.join("transactions").exists()
            || std::fs::read_dir(agent.join("transactions"))
                .unwrap()
                .next()
                .is_none()
    );
    std::fs::write(cache.join("fixture-public.bin"), public.as_ref()).unwrap();
    println!(
        "FIRST_INSTALL_EVIDENCE {}",
        serde_json::json!({"status":report,"installation":installation,"bundle_sha256":hex::encode(Sha256::digest(&before)),"source":source.source,"hashes":hashes,"identity_sha256":hex::encode(Sha256::digest(&original)),"authority":"cfg(test) fixture-key only","transactions_created":false})
    );
}

fn rotation_io_root() -> PathBuf {
    assert!(matches!(
        std::env::var("COMPUTERNAME").unwrap().as_str(),
        "RDLAB-WIN10" | "RDLAB-WIN11"
    ));
    let root =
        Path::new(r"C:\RustDeskAgentLab").join(format!("rotation-v217-{}", uuid::Uuid::new_v4()));
    workspace::create(&root).unwrap();
    for name in ["rescue", "transaction", "rescue/current", "rescue/staging"] {
        workspace::create(&root.join(name)).unwrap();
    }
    std::fs::write(
        root.join("rescue/current/bundle.json"),
        b"old approved bundle",
    )
    .unwrap();
    std::fs::write(
        root.join("rescue/staging/bundle.json"),
        b"new approved bundle",
    )
    .unwrap();
    root
}
fn rotation_digest(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

#[test]
fn rotation_receipt_resumes_between_renames_and_repeated_call_is_idempotent() {
    let root = rotation_io_root();
    let rescue = root.join("rescue");
    let tx = root.join("transaction");
    let old = rotation_digest(b"old approved bundle");
    let new = rotation_digest(b"new approved bundle");
    let error = rescue_assets::rotation_io_fixture(
        &rescue,
        &tx,
        &old,
        &new,
        None,
        Some("after_current_move"),
    )
    .unwrap_err();
    assert!(error.contains("injected rotation interruption"), "{error}");
    assert!(!rescue.join("current").exists());
    assert_eq!(
        std::fs::read(rescue.join("previous/bundle.json")).unwrap(),
        b"old approved bundle"
    );
    rescue_assets::rotation_io_fixture(&rescue, &tx, &old, &new, None, None).unwrap();
    assert_eq!(
        std::fs::read(rescue.join("current/bundle.json")).unwrap(),
        b"new approved bundle"
    );
    assert_eq!(
        std::fs::read(rescue.join("previous/bundle.json")).unwrap(),
        b"old approved bundle"
    );
    let receipt = std::fs::read(tx.join("rotation.json")).unwrap();
    rescue_assets::rotation_io_fixture(&rescue, &tx, &old, &new, None, None).unwrap();
    assert_eq!(std::fs::read(tx.join("rotation.json")).unwrap(), receipt);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn agentrestore_gate_only_stopped_without_disabled_or_active_pid() {
    assert!(agent_restore::gate("stopped", false, false).is_ok());
    for (state, disabled, pid) in [
        ("running", false, true),
        ("stopped", true, false),
        ("stopped", false, true),
        ("unknown", false, false),
        ("absent", false, false),
    ] {
        assert!(agent_restore::gate(state, disabled, pid).is_err());
    }
}

#[test]
fn agentrestore_missing_publication_verifies_hash_and_never_replaces_bad_existing_file() {
    let root = Path::new(r"C:\RustDeskAgentLab")
        .join(format!("agentrestore-v218-{}", uuid::Uuid::new_v4()));
    workspace::create(&root).unwrap();
    let source = root.join("approved.exe");
    let destination = root.join("agent.exe");
    std::fs::write(&source, b"signed fixture bytes").unwrap();
    let hash = hex::encode(Sha256::digest(b"signed fixture bytes"));
    agent_restore::publish_fixture(&source, &destination, 20, &hash).unwrap();
    assert_eq!(
        std::fs::read(&destination).unwrap(),
        b"signed fixture bytes"
    );
    std::fs::write(&destination, b"preserve corrupt evidence").unwrap();
    assert!(
        agent_restore::publish_fixture(&source, &destination, 20, &hash)
            .unwrap_err()
            .contains("occupied")
    );
    assert_eq!(
        std::fs::read(&destination).unwrap(),
        b"preserve corrupt evidence"
    );
    std::fs::remove_file(&destination).unwrap();
    std::fs::write(&source, b"tampered source byte").unwrap();
    assert!(
        agent_restore::publish_fixture(&source, &destination, 20, &hash)
            .unwrap_err()
            .contains("integrity")
    );
    assert!(!destination.exists());
    assert_eq!(std::fs::read(&source).unwrap(), b"tampered source byte");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn agentrestore_trusted_fixed_source_rejects_key_identity_build_and_bootstrap_mismatch() {
    assert_eq!(std::env::var("COMPUTERNAME").unwrap(), "RDLAB-WIN11");
    let root = Path::new(r"C:\ProgramData\RustDesk\ManagementAgent\rescue\current");
    let original = std::fs::read(root.join("bundle.json")).unwrap();
    let bundle: serde_json::Value = serde_json::from_slice(&original).unwrap();
    let installation = bundle["installation_id"].as_str().unwrap();
    let key = bundle["target_key"].as_str().unwrap();
    let hash = bundle["agent"]["sha256"].as_str().unwrap();
    let build = bundle["build_seq"].as_u64().unwrap();
    let public = rescue_assets::fixed_source_public_fixture(&bundle).unwrap();
    let identity = std::fs::read(hbb_common::config::Config::file()).unwrap();
    rescue_assets::agent_backup_fixture(root, installation, key, &public, hash, build, &identity)
        .unwrap();
    assert!(rescue_assets::agent_backup_fixture(
        root,
        &"0".repeat(32),
        key,
        &public,
        hash,
        build,
        &identity
    )
    .unwrap_err()
    .contains("identity"));
    let wrong = sign::gen_keypair().0;
    assert!(rescue_assets::agent_backup_fixture(
        root,
        installation,
        key,
        wrong.as_ref(),
        hash,
        build,
        &identity
    )
    .unwrap_err()
    .contains("signature"));
    assert!(rescue_assets::agent_backup_fixture(
        root,
        installation,
        key,
        &public,
        hash,
        build + 1,
        &identity
    )
    .unwrap_err()
    .contains("build"));
    assert!(rescue_assets::agent_backup_fixture(
        root,
        installation,
        key,
        &public,
        &"0".repeat(64),
        build,
        &identity
    )
    .unwrap_err()
    .contains("approval"));
    assert!(rescue_assets::agent_backup_fixture(
        root,
        installation,
        key,
        &public,
        hash,
        build,
        b"different identity"
    )
    .unwrap_err()
    .contains("identity"));
    assert_eq!(std::fs::read(root.join("bundle.json")).unwrap(), original);
    assert_eq!(
        std::fs::read(hbb_common::config::Config::file()).unwrap(),
        identity
    );
}

#[test]
fn rotation_locked_old_and_cas_conflict_keep_old_and_new_data() {
    use std::os::windows::fs::OpenOptionsExt;
    let root = rotation_io_root();
    let rescue = root.join("rescue");
    let tx = root.join("transaction");
    let old = rotation_digest(b"old approved bundle");
    let new = rotation_digest(b"new approved bundle");
    let lock = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(1)
        .custom_flags(0x02000000)
        .open(rescue.join("current"))
        .unwrap();
    assert!(rescue_assets::rotation_io_fixture(&rescue, &tx, &old, &new, None, None).is_err());
    assert_eq!(
        std::fs::read(rescue.join("current/bundle.json")).unwrap(),
        b"old approved bundle"
    );
    assert_eq!(
        std::fs::read(rescue.join("staging/bundle.json")).unwrap(),
        b"new approved bundle"
    );
    drop(lock);
    std::fs::write(rescue.join("current/bundle.json"), b"third party bytes").unwrap();
    assert!(
        rescue_assets::rotation_io_fixture(&rescue, &tx, &old, &new, None, None)
            .unwrap_err()
            .contains("conflict")
    );
    assert_eq!(
        std::fs::read(rescue.join("current/bundle.json")).unwrap(),
        b"third party bytes"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn rotation_existing_previous_is_archived_not_deleted() {
    let root = rotation_io_root();
    let rescue = root.join("rescue");
    let tx = root.join("transaction");
    let old = rotation_digest(b"old approved bundle");
    let new = rotation_digest(b"new approved bundle");
    workspace::create(&rescue.join("previous")).unwrap();
    std::fs::write(
        rescue.join("previous/bundle.json"),
        b"oldest approved bundle",
    )
    .unwrap();
    let previous = rotation_digest(b"oldest approved bundle");
    rescue_assets::rotation_io_fixture(&rescue, &tx, &old, &new, Some(&previous), None).unwrap();
    assert_eq!(
        std::fs::read(tx.join("retired-rescue/bundle.json")).unwrap(),
        b"oldest approved bundle"
    );
    assert_eq!(
        std::fs::read(rescue.join("previous/bundle.json")).unwrap(),
        b"old approved bundle"
    );
    assert_eq!(
        std::fs::read(rescue.join("current/bundle.json")).unwrap(),
        b"new approved bundle"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn rotationreview_archive_missing_corrupt_or_foreign_refuses_later_phases_without_writes() {
    for phase in ["after_current_move", "after_publish_move", "published"] {
        for damage in ["missing", "changed", "foreign"] {
            let root = rotation_io_root();
            let rescue = root.join("rescue");
            let tx = root.join("transaction");
            let old = rotation_digest(b"old approved bundle");
            let new = rotation_digest(b"new approved bundle");
            let prior = if damage == "foreign" {
                None
            } else {
                workspace::create(&rescue.join("previous")).unwrap();
                std::fs::write(rescue.join("previous/bundle.json"), b"oldest").unwrap();
                Some(rotation_digest(b"oldest"))
            };
            let result = rescue_assets::rotation_io_fixture(
                &rescue,
                &tx,
                &old,
                &new,
                prior.as_deref(),
                if phase == "published" {
                    None
                } else {
                    Some(phase)
                },
            );
            if phase == "published" {
                result.unwrap();
            } else {
                assert!(result.unwrap_err().contains("interruption"));
            }
            let receipt = std::fs::read(tx.join("rotation.json")).unwrap();
            match damage {
                "missing" => {
                    std::fs::rename(tx.join("retired-rescue"), root.join("preserved-retired"))
                        .unwrap()
                }
                "changed" => {
                    std::fs::write(tx.join("retired-rescue/bundle.json"), b"corrupt archive")
                        .unwrap()
                }
                _ => {
                    workspace::create(&tx.join("retired-rescue")).unwrap();
                    std::fs::write(tx.join("retired-rescue/bundle.json"), b"foreign archive")
                        .unwrap();
                }
            }
            let result = rescue_assets::rotation_io_fixture(
                &rescue,
                &tx,
                &old,
                &new,
                prior.as_deref(),
                None,
            );
            assert!(
                result.is_err(),
                "phase={phase} damage={damage}: archive must still be checked"
            );
            assert_eq!(std::fs::read(tx.join("rotation.json")).unwrap(), receipt);
            assert_eq!(
                std::fs::read(rescue.join("previous/bundle.json")).unwrap(),
                b"old approved bundle"
            );
            if phase == "after_current_move" {
                assert!(!rescue.join("current").exists());
                assert_eq!(
                    std::fs::read(rescue.join("staging/bundle.json")).unwrap(),
                    b"new approved bundle"
                );
            } else {
                assert_eq!(
                    std::fs::read(rescue.join("current/bundle.json")).unwrap(),
                    b"new approved bundle"
                );
            }
            std::fs::remove_dir_all(root).unwrap();
        }
    }
}

#[test]
fn rotationreview_archive_payload_integrity_is_rechecked_with_unchanged_metadata() {
    for phase in ["after_current_move", "after_publish_move", "published"] {
        let root = rotation_io_root();
        let rescue = root.join("rescue");
        let tx = root.join("transaction");
        let old = rotation_digest(b"old approved bundle");
        let new = rotation_digest(b"new approved bundle");
        workspace::create(&rescue.join("previous")).unwrap();
        std::fs::write(rescue.join("previous/bundle.json"), b"oldest").unwrap();
        let prior = rotation_digest(b"oldest");
        for name in ["current", "previous", "staging"] {
            std::fs::write(rescue.join(name).join("payload.bin"), b"verified payload").unwrap();
        }
        let result = rescue_assets::rotation_payload_fixture(
            &rescue,
            &tx,
            &old,
            &new,
            Some(&prior),
            if phase == "published" {
                None
            } else {
                Some(phase)
            },
        );
        if phase == "published" {
            result.unwrap();
        } else {
            assert!(result.unwrap_err().contains("interruption"));
        }
        let receipt = std::fs::read(tx.join("rotation.json")).unwrap();
        std::fs::write(tx.join("retired-rescue/payload.bin"), b"corrupt bytes").unwrap();
        let error =
            rescue_assets::rotation_payload_fixture(&rescue, &tx, &old, &new, Some(&prior), None)
                .unwrap_err();
        assert!(error.contains("integrity"), "{error}");
        assert_eq!(std::fs::read(tx.join("rotation.json")).unwrap(), receipt);
        assert_eq!(
            std::fs::read(tx.join("retired-rescue/payload.bin")).unwrap(),
            b"corrupt bytes"
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn rotationtakeover_published_receipt_rejects_foreign_staging_without_writes() {
    let root = rotation_io_root();
    let rescue = root.join("rescue");
    let tx = root.join("transaction");
    let old = rotation_digest(b"old approved bundle");
    let new = rotation_digest(b"new approved bundle");
    rescue_assets::rotation_io_fixture(&rescue, &tx, &old, &new, None, None).unwrap();
    let receipt = std::fs::read(tx.join("rotation.json")).unwrap();
    workspace::create(&rescue.join("staging")).unwrap();
    std::fs::write(rescue.join("staging/bundle.json"), b"foreign staged bytes").unwrap();
    let result = rescue_assets::rotation_io_fixture(&rescue, &tx, &old, &new, None, None);
    assert_eq!(std::fs::read(tx.join("rotation.json")).unwrap(), receipt);
    assert_eq!(
        std::fs::read(rescue.join("current/bundle.json")).unwrap(),
        b"new approved bundle"
    );
    assert_eq!(
        std::fs::read(rescue.join("previous/bundle.json")).unwrap(),
        b"old approved bundle"
    );
    assert_eq!(
        std::fs::read(rescue.join("staging/bundle.json")).unwrap(),
        b"foreign staged bytes"
    );
    println!("ROTATION_POSTIMAGE_OLD_BYTES_PRESERVED=true");
    std::fs::remove_dir_all(root).unwrap();
    assert!(result.unwrap_err().contains("published image conflict"));
}

#[test]
fn rotationtakeover_unpublished_candidate_keeps_config_gate_and_complete_archive() {
    let f = Fixture::new();
    f.prepare().unwrap();
    let rescue = f.root.join("rescue");
    let old = std::fs::read(rescue.join("current/bundle.json")).unwrap();
    directory_backup::capture(&rescue.join("current"), &f.root.join("prior-copy")).unwrap();
    std::fs::rename(f.root.join("prior-copy/files"), rescue.join("previous")).unwrap();
    let tx = f.root.join("transactions").join("f".repeat(32));
    directory_backup::capture(&f.tx(), &f.root.join("takeover-tx-copy")).unwrap();
    std::fs::rename(f.root.join("takeover-tx-copy/files"), &tx).unwrap();
    for path in [&tx, &rescue.join("previous")] {
        let script = format!(
            "$acl=Get-Acl '{}';$acl.SetAccessRuleProtection($true,$true);Set-Acl '{}' $acl",
            path.display(),
            path.display()
        );
        assert!(std::process::Command::new("powershell.exe")
            .args(["-NoProfile", "-Command", &script])
            .status()
            .unwrap()
            .success());
    }
    let mut plan =
        install_plan::InstallPlan::decode(&std::fs::read(tx.join("prepared.json")).unwrap())
            .unwrap();
    plan.transaction_id = "f".repeat(32);
    plan.resume_task = format!("RustDeskManagementAgentResume-{}", plan.transaction_id);
    save(&tx.join("prepared.json"), &plan);
    let mut transaction =
        transaction::Transaction::decode(&std::fs::read(tx.join("transaction.json")).unwrap())
            .unwrap();
    transaction.id = plan.transaction_id.clone();
    save(&tx.join("transaction.json"), &transaction);
    let mut outcome: serde_json::Value =
        serde_json::from_slice(&std::fs::read(tx.join("execution-result.json")).unwrap()).unwrap();
    outcome["transaction_id"] = plan.transaction_id.clone().into();
    save(&tx.join("execution-result.json"), &outcome);
    assert!(rescue_assets::refresh_fault_fixture(
        &f.root,
        &tx,
        f.public.as_ref(),
        "after_current_move"
    )
    .unwrap_err()
    .contains("interruption"));
    let receipt = std::fs::read(tx.join("rotation.json")).unwrap();
    let staged = std::fs::read(rescue.join("staging/bundle.json")).unwrap();
    let config = f.root.join("current/RustDesk2.toml");
    let original_config = std::fs::read(&config).unwrap();
    let changed = b"[options]\nstop-service = 'N'\ncustom-option = 'later preference'\n";
    std::fs::write(&config, changed).unwrap();
    assert!(
        rescue_assets::refresh_fixture(&f.root, &tx, f.public.as_ref())
            .unwrap_err()
            .contains("configuration changed")
    );
    assert_eq!(std::fs::read(&config).unwrap(), changed);
    assert_eq!(std::fs::read(tx.join("rotation.json")).unwrap(), receipt);
    assert_eq!(
        std::fs::read(rescue.join("previous/bundle.json")).unwrap(),
        old
    );
    assert_eq!(
        std::fs::read(rescue.join("staging/bundle.json")).unwrap(),
        staged
    );
    assert!(!rescue.join("current").exists());
    std::fs::write(&config, original_config).unwrap();
    let archive = tx.join("retired-rescue");
    let package = archive.join("agent-sos-v187.msi");
    let bytes = std::fs::read(&package).unwrap();
    for published in [false, true] {
        let before = std::fs::read(tx.join("rotation.json")).unwrap();
        std::fs::write(&package, b"damaged archive MSI").unwrap();
        let error = rescue_assets::refresh_fixture(&f.root, &tx, f.public.as_ref()).unwrap_err();
        assert!(
            error.contains("mismatch") || error.contains("integrity"),
            "{error}"
        );
        assert_eq!(std::fs::read(tx.join("rotation.json")).unwrap(), before);
        assert_eq!(std::fs::read(archive.join("bundle.json")).unwrap(), old);
        assert_eq!(std::fs::read(&package).unwrap(), b"damaged archive MSI");
        assert_eq!(
            std::fs::read(rescue.join("previous/bundle.json")).unwrap(),
            old
        );
        assert_eq!(
            std::fs::read(rescue.join(if published {
                "current/bundle.json"
            } else {
                "staging/bundle.json"
            }))
            .unwrap(),
            staged
        );
        std::fs::write(&package, &bytes).unwrap();
        rescue_assets::refresh_fixture(&f.root, &tx, f.public.as_ref()).unwrap();
    }
    assert_eq!(std::fs::read(&package).unwrap(), bytes);
    assert_eq!(
        std::fs::read(rescue.join("previous/bundle.json")).unwrap(),
        old
    );
    println!("ROTATION_FULL_MSI_ARCHIVE=verified;unpublished_config_gate=refused;old_bytes_preserved=true");
}

#[test]
fn rotationreview_registered_msi_inside_moving_tree_is_refused_before_mutation() {
    let root = rotation_io_root();
    let rescue = root.join("rescue");
    let before = std::fs::read(rescue.join("current/bundle.json")).unwrap();
    for part in ["current", "previous", "staging"] {
        let source = rescue.join(part).join("agent-sos-v187.msi");
        assert!(
            rescue_assets::rotation_registered_fixture(&rescue, &source).is_err(),
            "{part}"
        );
    }
    rescue_assets::rotation_registered_fixture(
        &rescue,
        &root.join("transaction/agent-sos-v187.msi"),
    )
    .unwrap();
    let case_variant = PathBuf::from(
        rescue
            .join("./CURRENT/agent-sos-v187.msi")
            .to_string_lossy()
            .to_ascii_uppercase(),
    );
    assert!(rescue_assets::rotation_registered_fixture(&rescue, &case_variant).is_err());
    rescue_assets::rotation_registered_fixture(
        &rescue,
        &rescue.join("current-other/agent-sos-v187.msi"),
    )
    .unwrap();
    assert_eq!(
        std::fs::read(rescue.join("current/bundle.json")).unwrap(),
        before
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn rotationreview_published_postimage_reconciles_after_config_change_without_reverting_it() {
    let f = Fixture::new();
    f.prepare().unwrap();
    let rescue = f.root.join("rescue");
    let old = std::fs::read(rescue.join("current/bundle.json")).unwrap();
    let tx = f.root.join("transactions").join("f".repeat(32));
    directory_backup::capture(&f.tx(), &f.root.join("review-new-tx-copy")).unwrap();
    std::fs::rename(f.root.join("review-new-tx-copy/files"), &tx).unwrap();
    let script = format!(
        "$acl=Get-Acl '{}';$acl.SetAccessRuleProtection($true,$true);Set-Acl '{}' $acl",
        tx.display(),
        tx.display()
    );
    assert!(std::process::Command::new("powershell.exe")
        .args(["-NoProfile", "-Command", &script])
        .status()
        .unwrap()
        .success());
    let mut plan =
        install_plan::InstallPlan::decode(&std::fs::read(tx.join("prepared.json")).unwrap())
            .unwrap();
    plan.transaction_id = "f".repeat(32);
    plan.resume_task = format!("RustDeskManagementAgentResume-{}", plan.transaction_id);
    save(&tx.join("prepared.json"), &plan);
    let mut transaction =
        transaction::Transaction::decode(&std::fs::read(tx.join("transaction.json")).unwrap())
            .unwrap();
    transaction.id = plan.transaction_id.clone();
    save(&tx.join("transaction.json"), &transaction);
    let mut outcome: serde_json::Value =
        serde_json::from_slice(&std::fs::read(tx.join("execution-result.json")).unwrap()).unwrap();
    outcome["transaction_id"] = plan.transaction_id.clone().into();
    save(&tx.join("execution-result.json"), &outcome);
    assert!(rescue_assets::refresh_fault_fixture(
        &f.root,
        &tx,
        f.public.as_ref(),
        "after_publish_move"
    )
    .unwrap_err()
    .contains("interruption"));
    let receipt: serde_json::Value =
        serde_json::from_slice(&std::fs::read(tx.join("rotation.json")).unwrap()).unwrap();
    assert_eq!(receipt["phase"], "old_moved");
    let published = std::fs::read(rescue.join("current/bundle.json")).unwrap();
    assert_eq!(
        std::fs::read(rescue.join("previous/bundle.json")).unwrap(),
        old
    );
    let config = f.root.join("current/RustDesk2.toml");
    let new_config =
        b"[options]\nstop-service = 'N'\ncustom-option = 'new valid user preference'\n";
    std::fs::write(&config, new_config).unwrap();
    let unfinished = std::fs::read(tx.join("rotation.json")).unwrap();
    for marker in ["disable-agent", "disable-recovery", "maintenance.json"] {
        let path = f.root.join("state").join(marker);
        std::fs::write(&path, b"hold closure").unwrap();
        assert!(
            rescue_assets::refresh_fixture(&f.root, &tx, f.public.as_ref())
                .unwrap_err()
                .contains("blocked")
        );
        assert_eq!(std::fs::read(tx.join("rotation.json")).unwrap(), unfinished);
        assert_eq!(std::fs::read(&path).unwrap(), b"hold closure");
        std::fs::remove_file(path).unwrap();
    }
    std::fs::write(&config, b"[options]\nstop-service = 'Y'\n").unwrap();
    assert!(
        rescue_assets::refresh_fixture(&f.root, &tx, f.public.as_ref())
            .unwrap_err()
            .contains("user stopped")
    );
    assert_eq!(std::fs::read(tx.join("rotation.json")).unwrap(), unfinished);
    std::fs::write(&config, new_config).unwrap();
    let result = rescue_assets::refresh_fixture(&f.root, &tx, f.public.as_ref());
    assert!(
        result.is_ok(),
        "postimage receipt closure must accept ordinary Config2 change: {:?}",
        result.err()
    );
    assert_eq!(std::fs::read(&config).unwrap(), new_config);
    assert_eq!(
        std::fs::read(rescue.join("current/bundle.json")).unwrap(),
        published
    );
    assert_eq!(
        std::fs::read(rescue.join("previous/bundle.json")).unwrap(),
        old
    );
    let receipt: serde_json::Value =
        serde_json::from_slice(&std::fs::read(tx.join("rotation.json")).unwrap()).unwrap();
    assert_eq!(receipt["phase"], "published");
    std::fs::write(&config, b"[options]\nstop-service = 'Y'\n").unwrap();
    let receipt = std::fs::read(tx.join("rotation.json")).unwrap();
    assert!(
        rescue_assets::refresh_fixture(&f.root, &tx, f.public.as_ref())
            .unwrap_err()
            .contains("user stopped")
    );
    assert_eq!(std::fs::read(tx.join("rotation.json")).unwrap(), receipt);
}

#[test]
fn rotation_receipt_recovers_previous_retire_and_final_publish_crash_windows() {
    for fault in ["after_previous_move", "after_publish_move"] {
        let root = rotation_io_root();
        let rescue = root.join("rescue");
        let tx = root.join("transaction");
        let old = rotation_digest(b"old approved bundle");
        let new = rotation_digest(b"new approved bundle");
        workspace::create(&rescue.join("previous")).unwrap();
        std::fs::write(rescue.join("previous/bundle.json"), b"oldest").unwrap();
        let prior = rotation_digest(b"oldest");
        assert!(rescue_assets::rotation_io_fixture(
            &rescue,
            &tx,
            &old,
            &new,
            Some(&prior),
            Some(fault)
        )
        .unwrap_err()
        .contains("interruption"));
        rescue_assets::rotation_io_fixture(&rescue, &tx, &old, &new, Some(&prior), None).unwrap();
        assert_eq!(
            std::fs::read(rescue.join("current/bundle.json")).unwrap(),
            b"new approved bundle"
        );
        assert_eq!(
            std::fs::read(rescue.join("previous/bundle.json")).unwrap(),
            b"old approved bundle"
        );
        assert_eq!(
            std::fs::read(tx.join("retired-rescue/bundle.json")).unwrap(),
            b"oldest"
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn rotation_approved_committed_samebuild_refresh_preserves_old_and_invalid_inputs() {
    let f = Fixture::new();
    f.prepare().unwrap();
    let rescue = f.root.join("rescue");
    let old = std::fs::read(rescue.join("current/bundle.json")).unwrap();
    let tx = f.root.join("transactions").join("e".repeat(32));
    directory_backup::capture(&f.tx(), &f.root.join("new-tx-copy")).unwrap();
    std::fs::rename(f.root.join("new-tx-copy/files"), &tx).unwrap();
    // The copy inherits a safe ACL; make only the new fixture transaction itself protected.
    let script = format!(
        "$acl=Get-Acl '{}';$acl.SetAccessRuleProtection($true,$true);Set-Acl '{}' $acl",
        tx.display(),
        tx.display()
    );
    assert!(std::process::Command::new("powershell.exe")
        .args(["-NoProfile", "-Command", &script])
        .status()
        .unwrap()
        .success());
    let mut plan =
        install_plan::InstallPlan::decode(&std::fs::read(tx.join("prepared.json")).unwrap())
            .unwrap();
    plan.transaction_id = "e".repeat(32);
    plan.resume_task = format!("RustDeskManagementAgentResume-{}", plan.transaction_id);
    save(&tx.join("prepared.json"), &plan);
    let mut transaction =
        transaction::Transaction::decode(&std::fs::read(tx.join("transaction.json")).unwrap())
            .unwrap();
    transaction.id = plan.transaction_id.clone();
    save(&tx.join("transaction.json"), &transaction);
    let mut result: serde_json::Value =
        serde_json::from_slice(&std::fs::read(tx.join("execution-result.json")).unwrap()).unwrap();
    result["transaction_id"] = plan.transaction_id.clone().into();
    save(&tx.join("execution-result.json"), &result);
    let approved = std::fs::read(tx.join("approved-manifest.json")).unwrap();
    let mut bad: UpdateManifest = serde_json::from_slice(&approved).unwrap();
    bad.targets
        .get_mut("windows-x86_64-msi-sos")
        .unwrap()
        .signature = STANDARD.encode([0u8; 64]);
    save(&tx.join("approved-manifest.json"), &bad);
    assert!(
        rescue_assets::refresh_fixture(&f.root, &tx, f.public.as_ref())
            .unwrap_err()
            .contains("signature")
    );
    assert_eq!(
        std::fs::read(rescue.join("current/bundle.json")).unwrap(),
        old
    );
    assert!(!rescue.join("previous").exists());
    std::fs::write(tx.join("approved-manifest.json"), &approved).unwrap();
    let reports = f.root.join("reports");
    rescue_status::refresh_fixture(
        &rescue.join("current"),
        &reports,
        &"c".repeat(32),
        "windows-x86_64-msi-sos",
        1000,
        Some(f.public.as_ref()),
    )
    .unwrap();
    for marker in ["disable-agent", "disable-recovery", "maintenance.json"] {
        let path = f.root.join("state").join(marker);
        std::fs::write(&path, b"preserve").unwrap();
        assert!(
            rescue_assets::refresh_fixture(&f.root, &tx, f.public.as_ref())
                .unwrap_err()
                .contains("blocked")
        );
        assert_eq!(std::fs::read(&path).unwrap(), b"preserve");
        std::fs::remove_file(path).unwrap();
    }
    let report = rescue_assets::refresh_fixture(&f.root, &tx, f.public.as_ref()).unwrap();
    assert_eq!(report["source_transaction"], plan.transaction_id);
    assert_eq!(
        std::fs::read(rescue.join("previous/bundle.json")).unwrap(),
        old
    );
    let current = std::fs::read(rescue.join("current/bundle.json")).unwrap();
    assert_ne!(current, old);
    assert_eq!(
        rescue_status::snapshot_fixture(
            &rescue.join("current"),
            &reports,
            &"c".repeat(32),
            "windows-x86_64-msi-sos",
            1001
        )["freshness"],
        "source_changed"
    );
    rescue_assets::refresh_fixture(&f.root, &tx, f.public.as_ref()).unwrap();
    assert_eq!(
        std::fs::read(rescue.join("current/bundle.json")).unwrap(),
        current
    );
    assert_eq!(
        std::fs::read(f.tx().join("agent-sos-v187.msi")).unwrap(),
        std::fs::read(tx.join("agent-sos-v187.msi")).unwrap()
    );
}

#[test]
fn rescuestatus_full_verify_cache_read_missing_corrupt_signature_and_prior_evidence() {
    let f = Fixture::new();
    f.prepare().unwrap();
    let bundle = f.root.join("rescue/current");
    let reports = f.root.join("status-reports");
    let installation = "c".repeat(32);
    let key = "windows-x86_64-msi-sos";
    assert!(rescue_status::refresh_fixture(
        &bundle,
        &reports,
        &installation,
        key,
        1000,
        Some(f.public.as_ref())
    )
    .unwrap());
    let verified = std::fs::read(reports.join("verified.json")).unwrap();
    let previous_check = std::fs::read(reports.join("last-check.json")).unwrap();
    use std::os::windows::fs::OpenOptionsExt;
    let report_lock = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(reports.join("verified.json"))
        .unwrap();
    assert!(rescue_status::refresh_fixture(
        &bundle,
        &reports,
        &installation,
        key,
        1000 + 86400,
        Some(f.public.as_ref())
    )
    .is_err());
    assert_eq!(
        std::fs::read(reports.join("verified.json")).unwrap(),
        verified
    );
    assert_eq!(
        std::fs::read(reports.join("last-check.json")).unwrap(),
        previous_check
    );
    drop(report_lock);
    let value = rescue_status::snapshot_fixture(&bundle, &reports, &installation, key, 1001);
    assert_eq!(value["state"], "degraded");
    assert_eq!(value["freshness"], "fresh");
    assert_eq!(value["ready"], false);
    assert_eq!(value["components"]["offline_package"]["state"], "verified");
    let package = bundle.join("agent-sos-v187.msi");
    let lock = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(&package)
        .unwrap();
    for _ in 0..3 {
        assert_eq!(
            rescue_status::snapshot_fixture(&bundle, &reports, &installation, key, 1002)["state"],
            "degraded"
        );
    }
    assert!(
        !rescue_status::refresh_fixture(
            &bundle,
            &reports,
            &installation,
            key,
            1002,
            Some(f.public.as_ref())
        )
        .unwrap(),
        "fresh report must not rescan locked MSI"
    );
    drop(lock);
    assert_eq!(
        rescue_status::snapshot_fixture(&bundle, &reports, &installation, key, 1000 + 86400)
            ["freshness"],
        "stale"
    );
    assert_eq!(
        rescue_status::snapshot_fixture(&bundle, &reports, &installation, key, 999)["freshness"],
        "clock_rollback"
    );
    assert!(!rescue_status::refresh_fixture(
        &bundle,
        &reports,
        &installation,
        key,
        999,
        Some(f.public.as_ref())
    )
    .unwrap());
    assert_eq!(
        std::fs::read(reports.join("verified.json")).unwrap(),
        verified
    );
    std::fs::write(&package, b"corrupt").unwrap();
    assert!(rescue_status::refresh_fixture(
        &bundle,
        &reports,
        &installation,
        key,
        87400,
        Some(f.public.as_ref())
    )
    .unwrap());
    assert_eq!(
        rescue_status::snapshot_fixture(&bundle, &reports, &installation, key, 87401)["state"],
        "unavailable"
    );
    assert_eq!(
        std::fs::read(reports.join("verified.json")).unwrap(),
        verified
    );
    let bad_key = sign::gen_keypair().0;
    assert!(rescue_status::refresh_fixture(
        &bundle,
        &reports,
        &installation,
        key,
        173800,
        Some(bad_key.as_ref())
    )
    .unwrap());
    assert_eq!(
        rescue_status::snapshot_fixture(&bundle, &reports, &installation, key, 173801)
            ["error_code"],
        "signature_invalid"
    );
    let old_manifest = std::fs::read(bundle.join("approved-manifest.json")).unwrap();
    std::fs::write(bundle.join("approved-manifest.json"), b"{}").unwrap();
    assert_eq!(
        rescue_status::snapshot_fixture(&bundle, &reports, &installation, key, 173802)["freshness"],
        "source_changed"
    );
    assert!(rescue_status::refresh_fixture(
        &bundle,
        &reports,
        &installation,
        key,
        173802,
        Some(f.public.as_ref())
    )
    .unwrap());
    assert_eq!(
        std::fs::read(reports.join("verified.json")).unwrap(),
        verified
    );
    std::fs::write(bundle.join("approved-manifest.json"), old_manifest).unwrap();
    let missing = f.root.join("rescue/missing");
    assert_eq!(
        rescue_status::snapshot_fixture(&missing, &reports, &installation, key, 174000)["state"],
        "unavailable"
    );
    assert!(rescue_status::refresh_fixture(
        &missing,
        &reports,
        &installation,
        key,
        174000,
        Some(f.public.as_ref())
    )
    .unwrap());
    assert_eq!(
        std::fs::read(reports.join("verified.json")).unwrap(),
        verified
    );
    let partial = f.root.join("rescue/partial");
    workspace::create(&partial).unwrap();
    std::fs::write(partial.join("package.partial"), b"keep").unwrap();
    assert_eq!(
        rescue_status::snapshot_fixture(&partial, &reports, &installation, key, 174001)["state"],
        "unavailable"
    );
    assert_eq!(
        std::fs::read(partial.join("package.partial")).unwrap(),
        b"keep"
    );
}

#[test]
fn rescuestatus_real_fixed_bundle_production_key_rejects_fixture_key_full_verifies_readonly() {
    assert_eq!(std::env::var("COMPUTERNAME").unwrap(), "RDLAB-WIN11");
    let bundle = Path::new(r"C:\ProgramData\RustDesk\ManagementAgent\rescue\current");
    let original = std::fs::read(bundle.join("bundle.json")).unwrap();
    let value: serde_json::Value = serde_json::from_slice(&original).unwrap();
    let public = rescue_assets::fixed_source_public_fixture(&value).unwrap();
    let installation = value["installation_id"].as_str().unwrap();
    let key = value["target_key"].as_str().unwrap();
    let root = Path::new(r"C:\RustDeskAgentLab")
        .join(format!("readiness-v216-fixed-{}", uuid::Uuid::new_v4()));
    workspace::create(&root).unwrap();
    let reports = root.join("reports");
    assert!(
        rescue_status::refresh_fixture(bundle, &reports, installation, key, 1000, None).unwrap()
    );
    let rejected = rescue_status::snapshot_fixture(bundle, &reports, installation, key, 1001);
    assert_eq!(rejected["state"], "unavailable");
    assert_eq!(rejected["error_code"], "signature_invalid");
    assert!(!reports.join("verified.json").exists());
    assert!(rescue_status::refresh_fixture(
        bundle,
        &reports,
        installation,
        key,
        1002,
        Some(&public)
    )
    .unwrap());
    let verified: serde_json::Value =
        serde_json::from_slice(&std::fs::read(reports.join("verified.json")).unwrap()).unwrap();
    assert!(verified["assets"]["verified_bytes"].as_u64().unwrap() > 67_000_000);
    assert_eq!(
        rescue_status::snapshot_fixture(bundle, &reports, installation, key, 1003)["state"],
        "degraded"
    );
    if value["source_kind"] == "installed_msi" {
        // The automatic local-repair engine accepts installed_msi, so status must
        // not claim it is unsupported; it reports not_assessed because status
        // cannot confirm the full approval chain.
        assert_eq!(
            rescue_status::snapshot_fixture(bundle, &reports, installation, key, 1003)
                ["components"]["local_repair"]["state"],
            "not_assessed"
        );
    }
    assert_eq!(std::fs::read(bundle.join("bundle.json")).unwrap(), original);
    println!("READONLY_FIXED_BUNDLE=full_verified;production_key=signature_invalid;fixture_key=degraded;bytes={};entries={}",verified["assets"]["verified_bytes"],verified["assets"]["verified_entries"]);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn autoschedule_real_files_hold_terminal_active_and_partial_transactions_without_new_ids() {
    let root = Path::new(r"C:\RustDeskAgentLab")
        .join(format!("autoschedule-v213-{}", uuid::Uuid::new_v4()));
    workspace::create(&root).unwrap();
    let plan = local_plan_fixture();
    let directory = root.join(&plan.transaction_id);
    workspace::create(&directory).unwrap();
    save(&directory.join("local-repair-plan.json"), &plan);
    let mut tx = transaction::Transaction::new(
        plan.transaction_id.clone(),
        plan.target_key.clone(),
        plan.package.sha256.clone(),
        plan.identity_backup.sha256.clone(),
        plan.created_at,
    )
    .unwrap();
    for phase in [
        transaction::Phase::Committed,
        transaction::Phase::RolledBack,
        transaction::Phase::Applying,
    ] {
        tx.phase = phase;
        save(&directory.join("transaction.json"), &tx);
        let before = std::fs::read(directory.join("transaction.json")).unwrap();
        assert_eq!(
            local_repair_schedule::existing(
                &root,
                &plan.installation_id,
                &plan.source_transaction,
                &plan.target_key,
                &plan.package.sha256
            )
            .unwrap(),
            Some(plan.transaction_id.clone())
        );
        assert_eq!(
            std::fs::read(directory.join("transaction.json")).unwrap(),
            before
        );
    }
    std::fs::remove_file(directory.join("transaction.json")).unwrap();
    assert!(local_repair_schedule::existing(
        &root,
        &plan.installation_id,
        &plan.source_transaction,
        &plan.target_key,
        &plan.package.sha256
    )
    .is_err());
    assert!(directory.join("local-repair-plan.json").exists());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn capacity63_reserves_one_then_64_can_launch_64_cannot_create_65_and_65_is_rejected() {
    assert!(matches!(
        std::env::var("COMPUTERNAME").unwrap().as_str(),
        "RDLAB-WIN10" | "RDLAB-WIN11"
    ));
    let root =
        Path::new(r"C:\RustDeskAgentLab").join(format!("capacity-v215-{}", uuid::Uuid::new_v4()));
    workspace::create(&root).unwrap();
    let create = |n: u32| {
        let id = format!("{n:032x}");
        let dir = root.join(&id);
        workspace::create(&dir).unwrap();
        let mut tx = transaction::Transaction::new(
            id,
            "windows-x86_64-msi-sos".into(),
            "b".repeat(64),
            "c".repeat(64),
            100,
        )
        .unwrap();
        tx.phase = transaction::Phase::Committed;
        save(&dir.join("transaction.json"), &tx);
    };
    for n in 0..63 {
        create(n);
    }
    let before = (0..63)
        .map(|n| std::fs::read(root.join(format!("{n:032x}/transaction.json"))).unwrap())
        .collect::<Vec<_>>();
    let _lock = install_guard::InstallGuard::try_acquire().unwrap().unwrap();
    local_repair::capacity_fixture(&root, None, true).unwrap();
    create(63);
    local_repair::capacity_fixture(&root, Some(&root.join(format!("{:032x}", 63))), false).unwrap();
    let candidate = root.join(format!("{:032x}", 64));
    let result = local_repair::capacity_fixture(&root, None, true);
    assert!(
        result.is_err(),
        "64 existing transactions must reserve no additional slot"
    );
    assert!(result.unwrap_err().contains("capacity"));
    assert!(!candidate.exists());
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), 64);
    assert!(local_repair_schedule::existing(
        &root,
        &"a".repeat(32),
        &"b".repeat(32),
        "windows-x86_64-msi-sos",
        &"b".repeat(64)
    )
    .unwrap_err()
    .contains("capacity"));
    for (n, bytes) in before.iter().enumerate() {
        assert_eq!(
            &std::fs::read(root.join(format!("{n:032x}/transaction.json"))).unwrap(),
            bytes
        );
    }
    create(64);
    assert!(local_repair::capacity_fixture(&root, None, true)
        .unwrap_err()
        .contains("capacity"));
    assert!(
        local_repair::capacity_fixture(&root, Some(&candidate), false)
            .unwrap_err()
            .contains("capacity")
    );
    assert!(local_repair_schedule::existing(
        &root,
        &"a".repeat(32),
        &"b".repeat(32),
        "windows-x86_64-msi-sos",
        &"b".repeat(64)
    )
    .is_err());
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), 65);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn autoschedule_periodic_child() {
    let Some(root) = std::env::var_os("RUSTDESK_AUTOSCHEDULE_CHILD") else {
        return;
    };
    assert_eq!(std::env::var("COMPUTERNAME").unwrap(), "RDLAB-WIN11");
    let root = PathBuf::from(root);
    assert!([
        Path::new(r"C:\RustDeskAgentLab\autoschedule-v213-evidence"),
        Path::new(r"C:\RustDeskAgentLab\autofailure-v215-evidence")
    ]
    .contains(&root.as_path()));
    workspace::verify(&root).unwrap();
    let public = std::fs::read(root.join("public.bin")).unwrap();
    let worker = local_repair_schedule::start_fixture(public).unwrap();
    let started = std::time::Instant::now();
    let (_tx, rx) = std::sync::mpsc::channel::<()>();
    let mut iteration = 0;
    while started.elapsed() < Duration::from_secs(1000) && !root.join("stop").exists() {
        let _ = rx.recv_timeout(Duration::from_secs(1));
        iteration += 1;
        let begin = std::time::Instant::now();
        worker
            .notify(local_repair_schedule::Tick {
                elapsed: started.elapsed(),
                enabled: true,
                retry_at: 0,
            })
            .unwrap();
        let enqueue_us = begin.elapsed().as_micros();
        let result = worker.result();
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .create(true)
            .open(root.join("iterations.jsonl"))
            .unwrap();
        writeln!(file,"{}",serde_json::json!({"iteration":iteration,"elapsed_ms":started.elapsed().as_millis(),"enqueue_us":enqueue_us,"decision":result})).unwrap();
        file.sync_all().unwrap();
    }
}

#[test]
#[ignore = "Independent periodic R4 worker and actual offline MSI; v213 SYSTEM snapshot fixture"]
fn autoschedule_real_periodic_worker_repairs_once() {
    autoschedule_real_case(false);
}

#[test]
#[ignore = "Independent worker actual MSI validation rollback and retained-source no-repeat; v215"]
fn autofailure_periodic_validation_rollback_never_repeats_source() {
    autoschedule_real_case(true);
}

fn autoschedule_real_case(fail: bool) {
    assert_eq!(std::env::var("COMPUTERNAME").unwrap(), "RDLAB-WIN11");
    assert_eq!(
        std::env::var("RUSTDESK_AUTO_REPAIR_LAB").unwrap(),
        if fail { "v215" } else { "v213" }
    );
    let evidence = Path::new(if fail {
        r"C:\RustDeskAgentLab\autofailure-v215-evidence"
    } else {
        r"C:\RustDeskAgentLab\autoschedule-v213-evidence"
    });
    workspace::create(evidence).unwrap();
    let current = Path::new(r"C:\ProgramData\RustDesk\ManagementAgent\rescue\current");
    let preserved = current.with_file_name(if fail {
        "current.v215-owned-previous"
    } else {
        "current.v213-owned-previous"
    });
    assert!(!preserved.exists());
    let bundle_bytes = std::fs::read(current.join("bundle.json")).unwrap();
    let original_bundle: serde_json::Value = serde_json::from_slice(&bundle_bytes).unwrap();
    let source_root = Path::new(r"C:\ProgramData\RustDesk\ManagementAgent\transactions");
    let existing_ids = || -> Vec<String> {
        std::fs::read_dir(source_root)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect()
    };
    let before_ids = existing_ids();
    let identity = hbb_common::config::Config::file();
    let identity_bytes = std::fs::read(&identity).unwrap();
    let config = hbb_common::config::Config2::file();
    let config_bytes = std::fs::read(&config).unwrap();
    let normal = Path::new(r"C:\Program Files\RustDesk");
    let backup = normal.with_file_name(if fail {
        "RustDesk.autofailure-v215-owned-backup"
    } else {
        "RustDesk.autoschedule-v213-owned-backup"
    });
    assert!(!backup.exists());
    let normal_inventory =
        directory_backup::capture(normal, &evidence.join("normal-before")).unwrap();
    let fallback = Path::new(r"C:\ProgramData\RustDesk\Client\sos\RustDesk");
    let fallback_inventory =
        directory_backup::capture(fallback, &evidence.join("fallback-before")).unwrap();
    let contract = service_control::Contract {
        name: "RustDesk".into(),
        executable: normal.join("RustDesk.exe"),
        arguments: vec!["--service".into()],
        start_type: windows_service::service::ServiceStartType::AutoStart,
    };
    let fallback_contract = service_control::Contract {
        executable: fallback.join("RustDesk.exe"),
        ..contract.clone()
    };
    let before_pid = service_control::inspect(&contract).unwrap().pid;
    #[link(name = "kernel32")]
    extern "system" {
        fn CreateMutexW(a: *const std::ffi::c_void, o: i32, n: *const u16)
            -> *mut std::ffi::c_void;
        fn WaitForSingleObject(h: *mut std::ffi::c_void, t: u32) -> u32;
        fn ReleaseMutex(h: *mut std::ffi::c_void) -> i32;
        fn CloseHandle(h: *mut std::ffi::c_void) -> i32;
    }
    struct Guard(*mut std::ffi::c_void);
    impl Drop for Guard {
        fn drop(&mut self) {
            unsafe {
                ReleaseMutex(self.0);
                CloseHandle(self.0);
            }
        }
    }
    let name = "Global\\RustDeskManagementAgentInstall"
        .encode_utf16()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let handle = unsafe { CreateMutexW(std::ptr::null(), 0, name.as_ptr()) };
    assert!(!handle.is_null());
    assert_eq!(unsafe { WaitForSingleObject(handle, 0) }, 0);
    let _guard = Guard(handle);
    // Preserve the historical approval intact; the isolated approval has a distinct signed reference.
    let inventory = directory_backup::capture(current, &evidence.join("approval-copy")).unwrap();
    inventory.verify(&evidence.join("approval-copy")).unwrap();
    let prepared = evidence.join("approval-copy/files");
    let mut manifest: UpdateManifest =
        serde_json::from_slice(&std::fs::read(prepared.join("approved-manifest.json")).unwrap())
            .unwrap();
    hbb_common::sodiumoxide::init().unwrap();
    let (public, secret) = sign::gen_keypair();
    let key = "windows-x86_64-msi-sos";
    let mut target = manifest.targets[key].clone();
    target.source_commit = if fail {
        "automatic-local-repair-v215-rollback-test-approval"
    } else {
        "automatic-local-repair-v213-test-approval"
    }
    .into();
    target.signature = STANDARD.encode(sign::sign_detached(
        &base::update::update_signature_payload(&manifest, key, &target).unwrap(),
        &secret,
    ));
    manifest.targets.insert(key.into(), target);
    save(&prepared.join("approved-manifest.json"), &manifest);
    let mut bundle = original_bundle.clone();
    bundle["source_reference"] =
        package_gate::package_reference(&serde_json::to_value(&manifest).unwrap(), key)
            .unwrap()
            .into();
    save(&prepared.join("bundle.json"), &bundle);
    let protected_approval = evidence.join("protected-approval");
    workspace::create(&protected_approval).unwrap();
    for entry in std::fs::read_dir(&prepared).unwrap() {
        let entry = entry.unwrap();
        std::fs::rename(entry.path(), protected_approval.join(entry.file_name())).unwrap();
    }
    std::fs::rename(current, &preserved).unwrap();
    std::fs::rename(&protected_approval, current).unwrap();
    std::fs::write(evidence.join("public.bin"), public.as_ref()).unwrap();
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "autoschedule_periodic_child", "--nocapture"])
        .env("RUSTDESK_AUTOSCHEDULE_CHILD", evidence)
        .env(
            "RUSTDESK_LOCAL_REPAIR_FAIL_VALIDATION",
            if fail { "yes" } else { "no" },
        )
        .stdout(std::fs::File::create(evidence.join("child.stdout.log")).unwrap())
        .stderr(std::fs::File::create(evidence.join("child.stderr.log")).unwrap())
        .spawn()
        .unwrap();
    let rows = || -> Vec<serde_json::Value> {
        std::fs::read_to_string(evidence.join("iterations.jsonl"))
            .unwrap_or_default()
            .lines()
            .filter_map(|line| serde_json::from_str(line).ok())
            .collect()
    };
    let mut transaction = None;
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(
        || -> Result<(), String> {
            let deadline = std::time::Instant::now() + Duration::from_secs(20);
            while rows()
                .iter()
                .filter(|row| row["decision"] == "Observe")
                .count()
                < 3
            {
                if std::time::Instant::now() > deadline {
                    return Err("healthy worker observations missing".into());
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            assert_eq!(service_control::inspect(&contract)?.pid, before_pid);
            assert_eq!(existing_ids().len(), before_ids.len());
            let reply = health_windows::probe_blocking(
                r"\\.\pipe\RustDesk\management-health-v2",
                &contract.executable,
                2026100502,
                Duration::from_secs(3),
            )?;
            if reply.no_active_sessions != Some(true) {
                return Err("activity unknown".into());
            }
            service_control::stop(&contract, Duration::from_secs(60))?;
            std::fs::rename(normal, &backup).map_err(|e| e.to_string())?;
            if !rescue_recovery::recover_fixture(public.as_ref())? {
                return Err("R3 fixture fallback failed".into());
            }
            // From this point only the timer-driven worker may prepare/launch R4.
            let deadline = std::time::Instant::now() + Duration::from_secs(900);
            loop {
                let added = existing_ids()
                    .into_iter()
                    .filter(|id| !before_ids.contains(id))
                    .collect::<Vec<_>>();
                assert!(added.len() <= 1, "duplicate automatic transaction");
                if let Some(id) = added.first() {
                    let path = source_root.join(id);
                    transaction = Some(path.clone());
                    if let Ok(bytes) = std::fs::read(path.join("local-result.json")) {
                        let result: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
                        if result["phase"] != if fail { "rolled_back" } else { "committed" } {
                            return Err(format!("automatic repair ended: {result}"));
                        }
                        if fail {
                            assert!(result["error"]
                                .as_str()
                                .is_some_and(|error| !error.is_empty()));
                        }
                        if !Path::new(
                            r"C:\ProgramData\RustDesk\ManagementAgent\state\maintenance.json",
                        )
                        .exists()
                        {
                            break;
                        }
                    }
                }
                if std::time::Instant::now() > deadline {
                    return Err("automatic repair timed out; preserve transaction".into());
                }
                std::thread::sleep(Duration::from_millis(250));
            }
            let tx = transaction.as_ref().unwrap();
            let plan = local_repair_plan::LocalRepairPlan::decode(
                &std::fs::read(tx.join("local-repair-plan.json")).unwrap(),
            )
            .unwrap();
            assert_eq!(plan.trigger, "local_recovery");
            assert!(normal_inventory.matches_directory(normal)?);
            assert!(fallback_inventory.matches_directory(fallback)?);
            let ready = health_windows::probe_blocking(
                r"\\.\pipe\RustDesk\management-health-v1",
                if fail {
                    &fallback_contract.executable
                } else {
                    &contract.executable
                },
                2026100502,
                Duration::from_secs(3),
            )?;
            let plan_bytes = std::fs::read(tx.join("local-repair-plan.json")).unwrap();
            let tx_bytes = std::fs::read(tx.join("transaction.json")).unwrap();
            let result_bytes = std::fs::read(tx.join("local-result.json")).unwrap();
            let msi_bytes = std::fs::read(tx.join("install-result.json")).unwrap();
            let approved: serde_json::Value =
                serde_json::from_slice(&std::fs::read(tx.join("approved-manifest.json")).unwrap())
                    .unwrap();
            assert_eq!(
                package_gate::package_reference(&approved, key).unwrap(),
                bundle["source_reference"].as_str().unwrap()
            );
            let expected_id = hex::encode(Sha256::digest(format!(
                "local-repair-auto-v1\n{}\n{}\n{}\n{}\n{}",
                plan.installation_id,
                plan.source_transaction,
                bundle["source_reference"].as_str().unwrap(),
                plan.target_key,
                plan.package.sha256
            )))[..32]
                .to_owned();
            assert_eq!(expected_id, plan.transaction_id);
            let observed = rows().len();
            let deadline = std::time::Instant::now() + Duration::from_secs(20);
            while rows().len() < observed + 5 {
                if std::time::Instant::now() > deadline {
                    return Err("post-commit cycle observation missing".into());
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            assert_eq!(existing_ids().len(), before_ids.len() + 1);
            assert_eq!(std::fs::read(&identity).unwrap(), identity_bytes);
            assert_eq!(std::fs::read(&config).unwrap(), config_bytes);
            assert_eq!(
                local_repair_schedule::existing_approval_fixture(
                    source_root,
                    &plan.installation_id,
                    &plan.source_transaction,
                    &plan.target_key,
                    &plan.package.sha256,
                    bundle["source_reference"].as_str().unwrap()
                )
                .unwrap(),
                Some(plan.transaction_id.clone())
            );
            if fail {
                // Installed normal is retained by product rollback. Move only these new test-owned bytes
                // aside to exercise same-source terminal lookup under another Missing observation.
                std::fs::rename(
                    normal,
                    evidence.join("rolledback-installed-normal-evidence"),
                )
                .map_err(|e| e.to_string())?;
                let start = rows().last().unwrap()["iteration"].as_u64().unwrap();
                let deadline = std::time::Instant::now() + Duration::from_secs(60);
                let expected = format!("Existing(\"{}\")", plan.transaction_id);
                loop {
                    if rows()
                        .iter()
                        .filter(|row| {
                            row["iteration"].as_u64().unwrap() > start
                                && row["decision"] == expected
                        })
                        .count()
                        >= 5
                    {
                        break;
                    }
                    if std::time::Instant::now() > deadline {
                        return Err("same-source terminal did not settle to Existing".into());
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
                assert_eq!(existing_ids().len(), before_ids.len() + 1);
                assert_eq!(
                    std::fs::read(tx.join("transaction.json")).unwrap(),
                    tx_bytes
                );
                assert_eq!(
                    std::fs::read(tx.join("local-result.json")).unwrap(),
                    result_bytes
                );
                assert_eq!(
                    std::fs::read(tx.join("install-result.json")).unwrap(),
                    msi_bytes
                );
                assert_eq!(
                    std::fs::read(tx.join("local-repair-plan.json")).unwrap(),
                    plan_bytes
                );
                let still_ready = health_windows::probe_blocking(
                    r"\\.\pipe\RustDesk\management-health-v1",
                    &fallback_contract.executable,
                    2026100502,
                    Duration::from_secs(3),
                )?;
                assert_eq!(still_ready.pid, ready.pid);
                save(
                    &evidence.join("rollback-proof.json"),
                    &serde_json::json!({"transaction_id":plan.transaction_id,"expected_id":expected_id,"source_reference":bundle["source_reference"],"result":serde_json::from_slice::<serde_json::Value>(&result_bytes).unwrap(),"msi_result":serde_json::from_slice::<serde_json::Value>(&msi_bytes).unwrap(),"post_terminal_existing_cycles":5,"transactions_created":1,"fallback_pid":still_ready.pid,"fallback_ready":true,"terminal_unchanged":true}),
                );
                println!("AUTOMATIC_R4_ROLLEDBACK={};error_nonempty=true;fallback_ready=true;same_source_existing_cycles=5;no_repeat_msi=true",tx.display());
            }
            println!("AUTOMATIC_R4_TERMINAL={};phase={};transactions_created=1;iterations={};health_build={};pid={};identity_config_equal=true",tx.display(),if fail{"RolledBack"}else{"Committed"},rows().len(),ready.build_seq,ready.pid);
            Ok(())
        },
    ));
    std::fs::write(evidence.join("stop"), b"stop").unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(950);
    while child.try_wait().unwrap().is_none() && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(
        child.try_wait().unwrap().is_some(),
        "worker process still owns repair"
    );
    if let Some(tx) = &transaction {
        let tx =
            transaction::Transaction::decode(&std::fs::read(tx.join("transaction.json")).unwrap())
                .unwrap();
        assert!(
            matches!(
                tx.phase,
                transaction::Phase::Committed | transaction::Phase::RolledBack
            ),
            "nonterminal preserve scene"
        );
    }
    if (fail || !matches!(result, Ok(Ok(())))) && backup.exists() {
        let manager = windows_service::service_manager::ServiceManager::local_computer(
            None::<&str>,
            windows_service::service_manager::ServiceManagerAccess::CONNECT,
        )
        .unwrap();
        let svc = manager
            .open_service(
                "RustDesk",
                windows_service::service::ServiceAccess::QUERY_CONFIG
                    | windows_service::service::ServiceAccess::QUERY_STATUS,
            )
            .unwrap();
        let cmd = svc
            .query_config()
            .unwrap()
            .executable_path
            .to_string_lossy()
            .into_owned();
        if svc.query_status().unwrap().current_state
            == windows_service::service::ServiceState::Running
        {
            service_control::stop(
                if fallback_contract.matches_command(&cmd) {
                    &fallback_contract
                } else {
                    &contract
                },
                Duration::from_secs(60),
            )
            .unwrap();
        }
        if normal.exists() {
            std::fs::rename(normal, evidence.join("failed-normal")).unwrap();
        }
        std::fs::rename(&backup, normal).unwrap();
        assert!(std::process::Command::new(r"C:\Windows\System32\sc.exe")
            .args([
                "config",
                "RustDesk",
                "binPath=",
                r#""C:\Program Files\RustDesk\RustDesk.exe" --service"#
            ])
            .status()
            .unwrap()
            .success());
        service_control::start(&contract, Duration::from_secs(60)).unwrap();
    }
    std::fs::rename(current, evidence.join("test-approved-bundle")).unwrap();
    std::fs::rename(&preserved, current).unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(60);
    while !native_install::client_ready(&contract.executable, 2026100502, true)
        && std::time::Instant::now() < deadline
    {
        std::thread::sleep(Duration::from_millis(250));
    }
    assert!(native_install::client_ready(
        &contract.executable,
        2026100502,
        true
    ));
    assert_eq!(
        std::fs::read(current.join("bundle.json")).unwrap(),
        bundle_bytes
    );
    assert_eq!(std::fs::read(&identity).unwrap(), identity_bytes);
    assert_eq!(std::fs::read(&config).unwrap(), config_bytes);
    println!(
        "AUTO_R4_HEALTHY_RESTORED=true;original_bundle_retained=true;original_backup={}",
        backup.display()
    );
    match result {
        Ok(Ok(())) => {}
        Ok(Err(error)) => panic!("auto repair failed after restoration: {error}"),
        Err(error) => std::panic::resume_unwind(error),
    }
}

#[test]
fn localrepair_blocked_evidence_and_failed_persistence_never_stop_services() {
    use install_execution::{Backend, InstallOutcome, Validation};
    use std::os::windows::fs::OpenOptionsExt;
    struct RefuseActions;
    impl Backend for RefuseActions {
        fn stop_services(&mut self) -> Result<(), String> {
            panic!("stop before durable authorization")
        }
        fn apply_package(&mut self) -> Result<InstallOutcome, String> {
            panic!("install before authorization")
        }
        fn validate_target(&mut self) -> Result<Validation, String> {
            panic!("validate before install")
        }
        fn rollback_original(&mut self) -> Result<Validation, String> {
            panic!("rollback before mutation")
        }
    }
    let root = Path::new(r"C:\RustDeskAgentLab")
        .join(format!("localrepair-v211-gates-{}", uuid::Uuid::new_v4()));
    workspace::create(&root).unwrap();
    let path = root.join("transaction.json");
    let mut tx = transaction::Transaction::new(
        "a".repeat(32),
        "windows-x86_64-msi-sos".into(),
        "b".repeat(64),
        "c".repeat(64),
        100,
    )
    .unwrap();
    tx.save(&path).unwrap();
    let mut proof = transaction::Evidence {
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
    for next in [
        transaction::Phase::PackageVerified,
        transaction::Phase::Prepared,
        transaction::Phase::StoppingServices,
    ] {
        tx.advance(next, &proof, 100).unwrap();
        tx.save(&path).unwrap();
    }
    let original = std::fs::read(&path).unwrap();
    let mut backend = RefuseActions;
    proof.locally_blocked = true;
    assert_eq!(
        local_repair::execute(&path, &mut tx, &proof, &mut backend, 100).unwrap_err(),
        "installation execution stop authorization unavailable"
    );
    assert_eq!(std::fs::read(&path).unwrap(), original);
    assert_eq!(tx.phase, transaction::Phase::StoppingServices);
    proof.locally_blocked = false;
    proof.resume_task_verified = false;
    assert!(
        local_repair::execute(&path, &mut tx, &proof, &mut backend, 100)
            .unwrap_err()
            .contains("authorization")
    );
    assert_eq!(std::fs::read(&path).unwrap(), original);
    proof.resume_task_verified = true;
    let lock = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(&path)
        .unwrap();
    let result = local_repair::execute(&path, &mut tx, &proof, &mut backend, 100);
    assert!(result.is_err());
    assert_eq!(std::fs::read(&path).unwrap(), original);
    assert_eq!(tx.phase, transaction::Phase::StoppingServices);
    drop(lock);
    std::fs::remove_dir_all(root).unwrap();
}

struct LocalReviewFixture {
    root: PathBuf,
    plan: local_repair_plan::LocalRepairPlan,
    binding: handoff::Handoff,
    tx: transaction::Transaction,
}
impl LocalReviewFixture {
    fn new(phase: transaction::Phase) -> Self {
        assert!(matches!(
            std::env::var("COMPUTERNAME").unwrap().as_str(),
            "RDLAB-WIN10" | "RDLAB-WIN11"
        ));
        let root = Path::new(r"C:\RustDeskAgentLab")
            .join(format!("localrepair-review-v212-{}", uuid::Uuid::new_v4()));
        workspace::create(&root).unwrap();
        workspace::create(&root.join("state")).unwrap();
        let plan = local_plan_fixture();
        let digest = hex::encode(Sha256::digest(serde_json::to_vec(&plan).unwrap()));
        let binding = handoff::Handoff::new(
            plan.transaction_id.clone(),
            digest,
            "d".repeat(64),
            handoff::Owner {
                pid: 10,
                created_at: 100,
            },
            handoff::Owner {
                pid: 11,
                created_at: 101,
            },
            100,
        )
        .unwrap();
        save(
            &root.join("state/agent.json"),
            &serde_json::json!({"agent_installation_id":plan.installation_id,"management_agent_enabled":true,"disabled":false}),
        );
        std::fs::write(root.join("intent.toml"), "[options]\nstop-service='N'\n").unwrap();
        std::fs::write(root.join("state/service-recovery-enabled"), b"enabled").unwrap();
        maintenance::claim(
            &root.join("state"),
            &plan.installation_id,
            &plan.transaction_id,
            &binding.plan_sha256,
            &binding.nonce,
        )
        .unwrap();
        let mut tx = transaction::Transaction::new(
            plan.transaction_id.clone(),
            plan.target_key.clone(),
            plan.package.sha256.clone(),
            plan.identity_backup.sha256.clone(),
            100,
        )
        .unwrap();
        tx.phase = phase;
        tx.revision = 4;
        save(&root.join("transaction.json"), &tx);
        save(&root.join("local-repair-plan.json"), &plan);
        std::fs::write(root.join("old-fallback"), b"known usable fallback").unwrap();
        Self {
            root,
            plan,
            binding,
            tx,
        }
    }
    fn dispatch(
        &mut self,
        backend: &mut LocalReviewBackend,
        time: u64,
        resume: bool,
        activity: impl FnOnce() -> Result<(), String>,
    ) -> Result<install_execution::ExecutionResult, String> {
        local_repair::dispatch_fixture(
            &self.root.join("transaction.json"),
            &mut self.tx,
            backend,
            time,
            resume,
            Some(install_execution::InstallOutcome::Installed),
            &self.root.join("state"),
            &self.root.join("intent.toml"),
            &self.plan,
            &self.binding,
            activity,
        )
    }
    fn bytes(&self) -> Vec<Vec<u8>> {
        [
            "transaction.json",
            "state/maintenance.json",
            "state/agent.json",
            "intent.toml",
            "old-fallback",
        ]
        .iter()
        .map(|p| std::fs::read(self.root.join(p)).unwrap())
        .collect()
    }
}
impl Drop for LocalReviewFixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.root).unwrap();
    }
}
#[derive(Default)]
struct LocalReviewBackend {
    calls: Vec<&'static str>,
}
impl install_execution::Backend for LocalReviewBackend {
    fn stop_services(&mut self) -> Result<(), String> {
        self.calls.push("stop");
        Err("fixture idle rejected inside backend".into())
    }
    fn apply_package(&mut self) -> Result<install_execution::InstallOutcome, String> {
        self.calls.push("apply");
        Ok(install_execution::InstallOutcome::Installed)
    }
    fn validate_target(&mut self) -> Result<install_execution::Validation, String> {
        self.calls.push("validate");
        Ok(install_execution::Validation {
            target_matches: true,
            identity_unchanged: true,
            client_ready: true,
            agent_ready: true,
        })
    }
    fn rollback_original(&mut self) -> Result<install_execution::Validation, String> {
        self.calls.push("rollback");
        Ok(install_execution::Validation {
            target_matches: true,
            identity_unchanged: true,
            client_ready: true,
            agent_ready: true,
        })
    }
}

#[test]
fn reviewr4_active_unknown_preflight_execute_and_resume_stop_have_zero_backend_calls() {
    for resume in [false, true] {
        for reason in ["active sessions", "unknown sessions"] {
            let mut f = LocalReviewFixture::new(transaction::Phase::StoppingServices);
            let before = f.bytes();
            let old = f.tx.clone();
            let mut backend = LocalReviewBackend::default();
            let result = f.dispatch(&mut backend, 110, resume, || Err(reason.into()));
            assert!(
                result.is_err(),
                "preflight rejected but executor performed recovery"
            );
            assert_eq!(result.unwrap_err(), reason);
            assert!(backend.calls.is_empty(), "{:?}", backend.calls);
            assert_eq!(f.tx, old);
            assert_eq!(f.bytes(), before);
        }
    }
}

#[test]
fn reviewr4_removed_optin_rejects_takeover_without_rollback_or_record_changes() {
    for resume in [false, true] {
        let mut f = LocalReviewFixture::new(transaction::Phase::StoppingServices);
        std::fs::remove_file(f.root.join("state/service-recovery-enabled")).unwrap();
        let before = f.bytes();
        let old = f.tx.clone();
        let mut backend = LocalReviewBackend::default();
        let result = f.dispatch(&mut backend, 110, resume, || Ok(()));
        assert!(result.is_err(), "revocation entered executor");
        assert!(result.unwrap_err().contains("opt-in"));
        assert!(backend.calls.is_empty());
        assert_eq!(f.tx, old);
        assert_eq!(f.bytes(), before);
        assert!(!f.root.join("state/service-recovery-enabled").exists());
    }
}

#[test]
fn reviewr4_resume_authority_rejects_expired_future_and_attempt_limit_before_backend() {
    for case in 0..3 {
        let mut f = LocalReviewFixture::new(transaction::Phase::StoppingServices);
        let time = match case {
            0 => 86500,
            1 => 99,
            _ => {
                f.tx.attempts = 3;
                save(&f.root.join("transaction.json"), &f.tx);
                110
            }
        };
        let before = f.bytes();
        let old = f.tx.clone();
        let mut backend = LocalReviewBackend::default();
        let result = f.dispatch(&mut backend, time, true, || Ok(()));
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("manual reconciliation"));
        assert!(backend.calls.is_empty());
        assert_eq!(f.tx, old);
        assert_eq!(f.bytes(), before);
    }
}

#[test]
fn reviewr4_already_applying_revocation_allows_known_installer_validation_not_new_install() {
    let mut f = LocalReviewFixture::new(transaction::Phase::Applying);
    std::fs::remove_file(f.root.join("state/service-recovery-enabled")).unwrap();
    let old = std::fs::read(f.root.join("state/maintenance.json")).unwrap();
    let mut backend = LocalReviewBackend::default();
    let result = f
        .dispatch(&mut backend, 110, true, || {
            panic!("no stop preflight after durable Applying")
        })
        .unwrap();
    assert_eq!(result.phase, transaction::Phase::Committed);
    assert_eq!(backend.calls, ["validate"]);
    assert_eq!(
        std::fs::read(f.root.join("state/maintenance.json")).unwrap(),
        old
    );
    assert!(!f.root.join("state/service-recovery-enabled").exists());
}

#[test]
fn reviewr4_services_already_stopped_can_restore_anchor_after_optin_revocation() {
    let mut f = LocalReviewFixture::new(transaction::Phase::ServicesStopped);
    std::fs::remove_file(f.root.join("state/service-recovery-enabled")).unwrap();
    let mut backend = LocalReviewBackend::default();
    let maintenance = std::fs::read(f.root.join("state/maintenance.json")).unwrap();
    let result = f
        .dispatch(&mut backend, 110, true, || {
            panic!("new-stop preflight must not prevent cleanup of stopped services")
        })
        .unwrap();
    assert_eq!(result.phase, transaction::Phase::RolledBack);
    assert_eq!(backend.calls, ["rollback"]);
    assert!(result.error.unwrap().contains("interrupted before apply"));
    assert_eq!(
        std::fs::read(f.root.join("state/maintenance.json")).unwrap(),
        maintenance
    );
    assert!(!f.root.join("state/service-recovery-enabled").exists());
}

#[test]
fn reviewr4_real_health_preflight_rejection_preserves_actual_service_pid_and_identity() {
    assert_eq!(std::env::var("COMPUTERNAME").unwrap(), "RDLAB-WIN11");
    let normal = service_control::Contract {
        name: "RustDesk".into(),
        executable: r"C:\Program Files\RustDesk\RustDesk.exe".into(),
        arguments: vec!["--service".into()],
        start_type: windows_service::service::ServiceStartType::AutoStart,
    };
    let before = service_control::inspect(&normal).unwrap();
    let identity = hbb_common::config::Config::file();
    let identity_bytes = std::fs::read(&identity).unwrap();
    let mut f = LocalReviewFixture::new(transaction::Phase::StoppingServices);
    let bytes = f.bytes();
    let mut backend = LocalReviewBackend::default();
    let result = f.dispatch(&mut backend, 110, false, || {
        health_windows::probe_blocking(
            r"\\.\pipe\RustDesk\management-health-v2",
            &normal.executable,
            2026100503,
            Duration::from_secs(3),
        )
        .map(|_| ())
        .map_err(|error| format!("real activity unavailable: {error}"))
    });
    assert!(result.unwrap_err().contains("real activity unavailable"));
    assert!(backend.calls.is_empty());
    assert_eq!(f.bytes(), bytes);
    let after = service_control::inspect(&normal).unwrap();
    assert_eq!(after.pid, before.pid);
    assert_eq!(after.state, before.state);
    assert_eq!(std::fs::read(&identity).unwrap(), identity_bytes);
    println!("REAL_HEALTH_REJECTION=zero_backend;service_pid_unchanged=true;original_identity_unchanged=true");
}

#[test]
fn localrepair_execution_persists_success_rollback_and_pending_without_faking_old_normal() {
    use install_execution::{Backend, InstallOutcome, Validation};
    struct Fake {
        outcome: InstallOutcome,
        valid: bool,
        rollbacks: usize,
    }
    impl Backend for Fake {
        fn stop_services(&mut self) -> Result<(), String> {
            Ok(())
        }
        fn apply_package(&mut self) -> Result<InstallOutcome, String> {
            Ok(self.outcome.clone())
        }
        fn validate_target(&mut self) -> Result<Validation, String> {
            Ok(Validation {
                target_matches: self.valid,
                identity_unchanged: true,
                client_ready: true,
                agent_ready: true,
            })
        }
        fn rollback_original(&mut self) -> Result<Validation, String> {
            self.rollbacks += 1;
            Ok(Validation {
                target_matches: true,
                identity_unchanged: true,
                client_ready: true,
                agent_ready: true,
            })
        }
    }
    for (outcome, valid, phase, rollbacks) in [
        (
            InstallOutcome::Installed,
            true,
            transaction::Phase::Committed,
            0,
        ),
        (
            InstallOutcome::Installed,
            false,
            transaction::Phase::RolledBack,
            1,
        ),
        (
            InstallOutcome::Pending {
                pid: 42,
                created_at: 99,
            },
            false,
            transaction::Phase::Applying,
            0,
        ),
    ] {
        let root = Path::new(r"C:\RustDeskAgentLab")
            .join(format!("localrepair-v211-{}", uuid::Uuid::new_v4()));
        workspace::create(&root).unwrap();
        let path = root.join("transaction.json");
        let mut tx = transaction::Transaction::new(
            "a".repeat(32),
            "windows-x86_64-msi-sos".into(),
            "b".repeat(64),
            "c".repeat(64),
            100,
        )
        .unwrap();
        tx.save(&path).unwrap();
        let proof = transaction::Evidence {
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
        for next in [
            transaction::Phase::PackageVerified,
            transaction::Phase::Prepared,
            transaction::Phase::StoppingServices,
        ] {
            tx.advance(next, &proof, 100).unwrap();
            tx.save(&path).unwrap();
        }
        std::fs::write(root.join("fallback-anchor"), b"old usable fallback").unwrap();
        let mut backend = Fake {
            outcome,
            valid,
            rollbacks: 0,
        };
        let result = local_repair::execute(&path, &mut tx, &proof, &mut backend, 100).unwrap();
        assert_eq!(result.phase, phase);
        assert_eq!(backend.rollbacks, rollbacks);
        assert_eq!(
            transaction::Transaction::decode(&std::fs::read(&path).unwrap())
                .unwrap()
                .phase,
            phase
        );
        if phase != transaction::Phase::Committed {
            assert!(result.error.is_some());
        }
        assert_eq!(
            std::fs::read(root.join("fallback-anchor")).unwrap(),
            b"old usable fallback"
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn localrepair_native_child() {
    let Ok(id) = std::env::var("RUSTDESK_LOCAL_REPAIR_CHILD") else {
        return;
    };
    assert_eq!(std::env::var("COMPUTERNAME").unwrap(), "RDLAB-WIN11");
    let public = STANDARD
        .decode(std::env::var("RUSTDESK_LOCAL_REPAIR_PUBLIC").unwrap())
        .unwrap();
    let fail = std::env::var("RUSTDESK_LOCAL_REPAIR_FAIL_VALIDATION")
        .ok()
        .as_deref()
        == Some("yes");
    local_repair::run_fixture(&id, &public, fail).unwrap();
}

#[test]
#[ignore = "Real offline same-MSI local repair; explicit Win11 snapshot v211 runner"]
fn localrepair_real_offline_success() {
    localrepair_real(false);
}

#[test]
#[ignore = "Win11 MSI v211 only: original local-repair action plus durable R3/R4 event acceptance"]
fn localrepair_real_offline_success_with_local_events() {
    std::env::set_var("RUSTDESK_LOCAL_EVENTS_ACCEPTANCE", "1");
    localrepair_real(false);
    std::env::remove_var("RUSTDESK_LOCAL_EVENTS_ACCEPTANCE");
    std::env::remove_var("RUSTDESK_LOCAL_EVENTS_FIXTURE");
}

#[test]
#[ignore = "Win11 schema2 actual validation failure rollback with durable R3/R4 events"]
fn localrepair_real_validation_failure_with_local_events() {
    std::env::set_var("RUSTDESK_LOCAL_EVENTS_ACCEPTANCE", "1");
    localrepair_real(true);
    std::env::remove_var("RUSTDESK_LOCAL_EVENTS_ACCEPTANCE");
    std::env::remove_var("RUSTDESK_LOCAL_EVENTS_FIXTURE");
}

#[test]
#[ignore = "Real offline validation failure rollback to fallback; explicit Win11 v211 runner"]
fn localrepair_real_validation_failure_returns_to_fallback() {
    localrepair_real(true);
}

type LocalRepairAttempt = std::thread::Result<Result<(), String>>;

fn localrepair_preserve_attempt(
    evidence: &Path,
    result: &LocalRepairAttempt,
    stage: &str,
    child_pid: Option<u32>,
    child_exit: Option<i32>,
) -> Result<(), String> {
    use std::io::Write;
    workspace::verify(evidence)?;
    let (kind, error) = match result {
        Ok(Ok(())) => ("ok", None),
        Ok(Err(error)) => ("error", Some(error.as_str())),
        Err(payload) => (
            "panic",
            Some(
                payload
                    .downcast_ref::<String>()
                    .map(String::as_str)
                    .or_else(|| payload.downcast_ref::<&str>().copied())
                    .unwrap_or("non-string panic payload"),
            ),
        ),
    };
    let bytes=serde_json::to_vec(&serde_json::json!({"result_kind":kind,"error":error,"stage":stage,"child_pid":child_pid,"child_exit_code":child_exit})).map_err(|e|e.to_string())?;
    let path = evidence.join("attempt-error.json");
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::AlreadyExists {
                "original attempt evidence exists; not overwritten".into()
            } else {
                e.to_string()
            }
        })?;
    file.write_all(&bytes)
        .and_then(|_| file.sync_all())
        .map_err(|e| e.to_string())?;
    drop(file);
    workspace::verify_file(&path)
}

#[test]
fn localrepair_error_evidence_preserves_original_before_nonterminal_and_refuses_overwrite() {
    assert_eq!(std::env::var("COMPUTERNAME").unwrap(), "RDLAB-WIN10");
    let root = Path::new(r"C:\RustDeskAgentLab")
        .join(format!("r4-error-evidence-{}", uuid::Uuid::new_v4()));
    workspace::create(&root).unwrap();
    let first: LocalRepairAttempt = Ok(Err("original launch error\n原始错误".into()));
    localrepair_preserve_attempt(&root, &first, "launch", Some(123), Some(-1073741515)).unwrap();
    let path = root.join("attempt-error.json");
    assert!(
        path.exists(),
        "original error must be persisted before nonterminal panic"
    );
    workspace::verify_file(&path).unwrap();
    let before = std::fs::read(&path).unwrap();
    let record: serde_json::Value = serde_json::from_slice(&before).unwrap();
    assert_eq!(record["error"], "original launch error\n原始错误");
    assert_eq!(record["stage"], "launch");
    assert_eq!(record["child_pid"], 123);
    assert_eq!(record["child_exit_code"], -1073741515i64);
    let second: LocalRepairAttempt = Ok(Err("nonterminal PackageVerified".into()));
    assert!(
        localrepair_preserve_attempt(&root, &second, "nonterminal", None, None)
            .unwrap_err()
            .contains("exists")
    );
    assert_eq!(std::fs::read(&path).unwrap(), before);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn localrepair_error_evidence_captures_panic_and_keeps_unknown_child_null() {
    assert_eq!(std::env::var("COMPUTERNAME").unwrap(), "RDLAB-WIN10");
    for (label, result, kind, error) in [
        (
            "panic",
            Err(Box::new(String::from("original panic")) as Box<dyn std::any::Any + Send>),
            "panic",
            Some("original panic"),
        ),
        ("ok", Ok(Ok(())), "ok", None),
    ] {
        let root = Path::new(r"C:\RustDeskAgentLab")
            .join(format!("r4-error-{label}-{}", uuid::Uuid::new_v4()));
        workspace::create(&root).unwrap();
        localrepair_preserve_attempt(&root, &result, "prepare", None, None).unwrap();
        let path = root.join("attempt-error.json");
        assert!(path.exists(), "attempt must be recorded");
        let record: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(record["result_kind"], kind);
        assert_eq!(record["error"].as_str(), error);
        assert!(record["child_pid"].is_null());
        assert!(record["child_exit_code"].is_null());
        std::fs::remove_dir_all(root).unwrap();
    }
}

fn localrepair_real(fail: bool) {
    assert_eq!(std::env::var("COMPUTERNAME").unwrap(), "RDLAB-WIN11");
    assert_eq!(std::env::var("RUSTDESK_LOCAL_REPAIR_LAB").unwrap(), "v211");
    let label = if fail { "rollback" } else { "success" };
    let evidence = Path::new(r"C:\RustDeskAgentLab")
        .join(format!("localrepair-v211-{label}-{}", uuid::Uuid::new_v4()));
    workspace::create(&evidence).unwrap();
    if std::env::var("RUSTDESK_LOCAL_EVENTS_ACCEPTANCE").as_deref() == Ok("1") {
        let state: serde_json::Value = serde_json::from_slice(
            &std::fs::read(r"C:\ProgramData\RustDesk\ManagementAgent\state\agent.json").unwrap(),
        )
        .unwrap();
        let device = identity::load_existing().unwrap();
        let context = local_recovery_events::EventContext {
            path: evidence.join("events.json"),
            binding: local_events::Binding {
                client_id: device.id,
                client_uuid: device.uuid,
                agent_installation_id: state["agent_installation_id"].as_str().unwrap().into(),
                enrollment_generation: 1,
            },
        };
        local_events::LocalEvents::open(&context.path)
            .unwrap()
            .remember_binding(context.binding.clone())
            .unwrap();
        let path = evidence.join("event-fixture.json");
        state_store::publish(&path, &serde_json::to_vec(&context).unwrap()).unwrap();
        std::env::set_var("RUSTDESK_LOCAL_EVENTS_FIXTURE", path);
    }
    let normal = Path::new(r"C:\Program Files\RustDesk");
    let backup = normal.with_file_name(format!(
        "RustDesk.localrepair-v211-{label}-{}-owned-backup",
        uuid::Uuid::new_v4()
    ));
    assert!(!backup.exists());
    let normal_inventory =
        directory_backup::capture(normal, &evidence.join("original-client")).unwrap();
    let fallback = Path::new(r"C:\ProgramData\RustDesk\Client\sos\RustDesk");
    let mut fallback_inventory = if fallback.exists() {
        Some(directory_backup::capture(fallback, &evidence.join("fallback-before")).unwrap())
    } else {
        None
    };
    let identity = hbb_common::config::Config::file();
    let identity_bytes = std::fs::read(&identity).unwrap();
    let config = hbb_common::config::Config2::file();
    let config_bytes = std::fs::read(&config).unwrap();
    let bundle_root = Path::new(r"C:\ProgramData\RustDesk\ManagementAgent\rescue\current");
    let bundle_bytes = std::fs::read(bundle_root.join("bundle.json")).unwrap();
    let bundle: serde_json::Value = serde_json::from_slice(&bundle_bytes).unwrap();
    let public = rescue_assets::fixed_source_public_fixture(&bundle).unwrap();
    let original_source = std::fs::read(r"C:\RustDeskAgentLab\agent-sos-v187.msi").unwrap();
    if std::env::var("RUSTDESK_SCHEMA2_R4_LAB").as_deref() == Ok("v226") {
        assert_eq!(bundle["schema"], 2);
        assert_eq!(bundle["source_kind"], "installed_msi");
        assert!(bundle.get("source_transaction").is_none());
    }
    let approved_bytes = std::fs::read(bundle_root.join("approved-manifest.json")).unwrap();
    let receipt_bytes = std::fs::read(bundle_root.join("installation-receipt.json")).ok();
    save(
        &evidence.join("checkpoint.json"),
        &serde_json::json!({"stage":"before_normal_missing","normal":normal,"backup":backup,"bundle_sha256":hex::encode(Sha256::digest(&bundle_bytes)),"source_sha256":hex::encode(Sha256::digest(&original_source)),"identity_sha256":hex::encode(Sha256::digest(&identity_bytes)),"failure_injection":fail}),
    );
    println!("LOCAL_REPAIR_EVIDENCE={}", evidence.display());
    let main = service_control::Contract {
        name: "RustDesk".into(),
        executable: normal.join("RustDesk.exe"),
        arguments: vec!["--service".into()],
        start_type: windows_service::service::ServiceStartType::AutoStart,
    };
    let fallback_contract = service_control::Contract {
        executable: fallback.join("RustDesk.exe"),
        ..main.clone()
    };
    #[link(name = "kernel32")]
    extern "system" {
        fn CreateMutexW(a: *const std::ffi::c_void, o: i32, n: *const u16)
            -> *mut std::ffi::c_void;
        fn WaitForSingleObject(h: *mut std::ffi::c_void, t: u32) -> u32;
        fn ReleaseMutex(h: *mut std::ffi::c_void) -> i32;
        fn CloseHandle(h: *mut std::ffi::c_void) -> i32;
    }
    struct Guard(*mut std::ffi::c_void);
    impl Drop for Guard {
        fn drop(&mut self) {
            unsafe {
                ReleaseMutex(self.0);
                CloseHandle(self.0);
            }
        }
    }
    let name = "Global\\RustDeskManagementAgentInstall"
        .encode_utf16()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let handle = unsafe { CreateMutexW(std::ptr::null(), 0, name.as_ptr()) };
    assert!(!handle.is_null());
    assert_eq!(unsafe { WaitForSingleObject(handle, 0) }, 0);
    let _guard = Guard(handle);
    let mut transaction_path = None;
    let mut attempt_stage = "before_normal_missing";
    let mut child_pid = None;
    let mut child_exit = None;
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(
        || -> Result<(), String> {
            let activity = health_windows::probe_blocking(
                r"\\.\pipe\RustDesk\management-health-v2",
                &main.executable,
                2026100502,
                Duration::from_secs(3),
            )?;
            if activity.no_active_sessions != Some(true) {
                return Err("fixture active or unknown sessions".into());
            }
            assert!(local_repair::prepare_fixture(
                &uuid::Uuid::new_v4().simple().to_string(),
                &public
            )
            .unwrap_err()
            .contains("missing normal"));
            service_control::stop(&main, Duration::from_secs(60))?;
            std::fs::rename(normal, &backup).map_err(|e| e.to_string())?;
            if !rescue_recovery::recover_fixture(&public)? {
                return Err("fixture R3 failed".into());
            }
            if fallback_inventory.is_none() {
                fallback_inventory = Some(directory_backup::capture(
                    fallback,
                    &evidence.join("fallback-established"),
                )?);
            }
            let id = uuid::Uuid::new_v4().simple().to_string();
            assert!(
                local_repair::prepare(&id).is_err(),
                "production must reject temporary signature"
            );
            attempt_stage = "prepare";
            let directory = local_repair::prepare_fixture(&id, &public)?;
            transaction_path = Some(directory.clone());
            println!("LOCAL_REPAIR_TRANSACTION={}", directory.display());
            let plan = local_repair_plan::LocalRepairPlan::decode(
                &std::fs::read(directory.join("local-repair-plan.json")).unwrap(),
            )
            .unwrap();
            assert_eq!(plan.original_normal, "missing");
            assert_eq!(plan.fallback_path, fallback.to_str().unwrap());
            if bundle["schema"] == 2 {
                assert!(plan.source_transaction.is_empty());
                assert_eq!(
                    plan.approval_source.as_ref().unwrap().source_kind,
                    "installed_msi"
                );
                assert_eq!(
                    std::fs::read(directory.join("approved-manifest.json")).unwrap(),
                    approved_bytes
                );
                assert_eq!(
                    Some(std::fs::read(directory.join("installation-receipt.json")).unwrap()),
                    receipt_bytes
                );
            }
            save(
                &evidence.join("before-helper.json"),
                &serde_json::json!({"transaction_id":id,"transaction_path":directory,"source_kind":bundle["source_kind"],"plan":plan,"fallback_ready":true}),
            );
            if fail {
                std::env::set_var("RUSTDESK_LOCAL_REPAIR_FAIL_VALIDATION", "yes");
            } else {
                std::env::remove_var("RUSTDESK_LOCAL_REPAIR_FAIL_VALIDATION");
            }
            attempt_stage = "launch";
            let mut child = local_repair::launch_fixture(&directory, &public)?;
            child_pid = Some(child.id());
            attempt_stage = "helper_wait";
            save(
                &evidence.join("child-observation.json"),
                &serde_json::json!({"pid":child_pid,"exit_code":null,"source":"returned_child_handle"}),
            );
            std::env::remove_var("RUSTDESK_LOCAL_REPAIR_FAIL_VALIDATION");
            let deadline = std::time::Instant::now() + Duration::from_secs(900);
            while child.try_wait().map_err(|e| e.to_string())?.is_none()
                && std::time::Instant::now() < deadline
            {
                std::thread::sleep(Duration::from_millis(250));
            }
            let status = child
                .try_wait()
                .map_err(|e| e.to_string())?
                .ok_or("helper pending; preserve scene")?;
            child_exit = status.code();
            attempt_stage = "helper_exited";
            save(
                &evidence.join("child-observation.json"),
                &serde_json::json!({"pid":child_pid,"exit_code":child_exit,"source":"returned_child_handle"}),
            );
            if !status.success() {
                return Err(format!(
                    "local helper failed: {}",
                    std::fs::read_to_string(directory.join("child.stderr.log")).unwrap()
                ));
            }
            let tx = transaction::Transaction::decode(
                &std::fs::read(directory.join("transaction.json")).unwrap(),
            )
            .unwrap();
            assert_eq!(
                tx.phase,
                if fail {
                    transaction::Phase::RolledBack
                } else {
                    transaction::Phase::Committed
                }
            );
            if std::env::var("RUSTDESK_LOCAL_EVENTS_ACCEPTANCE").as_deref() == Ok("1") {
                let log = local_events::LocalEvents::open(&evidence.join("events.json")).unwrap();
                let events = log.pending_events();
                assert_eq!(events.len(), 4);
                for code in ["r3", "r4"] {
                    let rows = events
                        .iter()
                        .filter(|event| event.payload["result"]["code"] == code)
                        .collect::<Vec<_>>();
                    assert_eq!(rows.len(), 2);
                    assert_eq!(rows[0].payload["status"], "running");
                    assert_eq!(
                        rows[1].payload["status"],
                        if code == "r4" && fail {
                            "rolled_back"
                        } else {
                            "succeeded"
                        }
                    );
                    assert!(rows
                        .iter()
                        .all(|event| event.payload["command_id"].is_null()));
                }
                assert_eq!(events.last().unwrap().payload["result"]["operation_id"], id);
                let before = std::fs::read(evidence.join("events.json")).unwrap();
                local_recovery_events::report_repair_terminal(
                    &tx,
                    &plan.installation_id,
                    tx.updated_at + 1,
                )
                .unwrap();
                assert_eq!(std::fs::read(evidence.join("events.json")).unwrap(), before);
            }
            let outcome: serde_json::Value = serde_json::from_slice(
                &std::fs::read(directory.join("local-result.json")).unwrap(),
            )
            .unwrap();
            assert!(resume_task::read_back(&id).is_err());
            assert!(
                !Path::new(r"C:\ProgramData\RustDesk\ManagementAgent\state\maintenance.json")
                    .exists()
            );
            let expected = if fail {
                &fallback_contract.executable
            } else {
                &main.executable
            };
            let health = health_windows::probe_blocking(
                r"\\.\pipe\RustDesk\management-health-v1",
                expected,
                2026100502,
                Duration::from_secs(3),
            )?;
            assert!(fallback_inventory
                .as_ref()
                .unwrap()
                .matches_directory(fallback)?);
            assert_eq!(std::fs::read(&identity).unwrap(), identity_bytes);
            assert_eq!(std::fs::read(&config).unwrap(), config_bytes);
            if !fail {
                assert!(normal_inventory.matches_directory(normal)?);
            } else {
                assert!(
                    outcome["error"]
                        .as_str()
                        .is_some_and(|error| error.contains("target validation failed")),
                    "{outcome}"
                );
            }
            save(
                &evidence.join("result.json"),
                &serde_json::json!({"phase":tx.phase,"build":health.build_seq,"pid":health.pid,"identity_preserved":true,"config_preserved":true,"fallback_preserved":true,"offline":true,"result":outcome,"msi_result":serde_json::from_slice::<serde_json::Value>(&std::fs::read(directory.join("install-result.json")).unwrap()).unwrap()}),
            );
            println!("LOCAL_REPAIR_PHASE={:?};MSI_RESULT_VERIFIED=true;HEALTH_BUILD={};IDENTITY_CONFIG_PRESERVED=true",tx.phase,health.build_seq);
            Ok(())
        },
    ));
    if let Err(error) =
        localrepair_preserve_attempt(&evidence, &result, attempt_stage, child_pid, child_exit)
    {
        eprintln!("LOCAL_REPAIR_EVIDENCE_WRITE_ERROR={error}; original_result={result:?}");
    }
    let terminal = transaction_path.as_ref().map(|path| {
        transaction::Transaction::decode(&std::fs::read(path.join("transaction.json")).unwrap())
            .unwrap()
            .phase
    });
    if terminal.is_some_and(|phase| {
        !matches!(
            phase,
            transaction::Phase::Committed | transaction::Phase::RolledBack
        )
    }) {
        panic!("nonterminal transaction retained; no concurrent fixture restore: {terminal:?}");
    }
    if backup.exists() {
        if !matches!(result, Ok(Ok(()))) || fail {
            let manager = windows_service::service_manager::ServiceManager::local_computer(
                None::<&str>,
                windows_service::service_manager::ServiceManagerAccess::CONNECT,
            )
            .unwrap();
            let service = manager
                .open_service(
                    "RustDesk",
                    windows_service::service::ServiceAccess::QUERY_CONFIG
                        | windows_service::service::ServiceAccess::QUERY_STATUS,
                )
                .unwrap();
            let command = service
                .query_config()
                .unwrap()
                .executable_path
                .to_string_lossy()
                .into_owned();
            if service.query_status().unwrap().current_state
                == windows_service::service::ServiceState::Running
            {
                let contract = if fallback_contract.matches_command(&command) {
                    &fallback_contract
                } else {
                    &main
                };
                service_control::stop(contract, Duration::from_secs(60)).unwrap();
            }
            if normal.exists() {
                std::fs::rename(normal, evidence.join("installed-normal-evidence")).unwrap();
            }
            std::fs::rename(&backup, normal).unwrap();
            assert!(std::process::Command::new(r"C:\Windows\System32\sc.exe")
                .args([
                    "config",
                    "RustDesk",
                    "binPath=",
                    r#""C:\Program Files\RustDesk\RustDesk.exe" --service"#
                ])
                .status()
                .unwrap()
                .success());
            service_control::start(&main, Duration::from_secs(60)).unwrap();
        } else {
            println!("ORIGINAL_NORMAL_BACKUP_RETAINED={}", backup.display());
        }
    }
    let deadline = std::time::Instant::now() + Duration::from_secs(60);
    while !native_install::client_ready(&main.executable, 2026100502, true)
        && std::time::Instant::now() < deadline
    {
        std::thread::sleep(Duration::from_millis(250));
    }
    assert!(native_install::client_ready(
        &main.executable,
        2026100502,
        true
    ));
    assert_eq!(std::fs::read(&identity).unwrap(), identity_bytes);
    assert_eq!(std::fs::read(&config).unwrap(), config_bytes);
    assert_eq!(
        std::fs::read(bundle_root.join("bundle.json")).unwrap(),
        bundle_bytes
    );
    assert_eq!(
        std::fs::read(r"C:\RustDeskAgentLab\agent-sos-v187.msi").unwrap(),
        original_source
    );
    std::env::remove_var("RUSTDESK_LOCAL_REPAIR_FAIL_VALIDATION");
    std::env::remove_var("RUSTDESK_LOCAL_EVENTS_FIXTURE");
    save(
        &evidence.join("cleanup.json"),
        &serde_json::json!({"normal_ready":true,"identity_preserved":true,"config_preserved":true,"bundle_preserved":true,"source_retained":true,"transaction":transaction_path,"terminal":terminal,"original_backup":backup,"http_ack":"not_executed"}),
    );
    match result {
        Ok(Ok(())) => {}
        Ok(Err(error)) => panic!("local repair failed: {error}"),
        Err(error) => std::panic::resume_unwind(error),
    }
}

#[test]
#[ignore = "R4 capability red: offline real fallback running, preparation must preserve old usable client; v210 runner"]
fn r4_offline_local_package_preparation_from_live_fallback_capability() {
    assert_eq!(std::env::var("COMPUTERNAME").unwrap(), "RDLAB-WIN11");
    assert_eq!(std::env::var("RUSTDESK_RESCUE_E2E").unwrap(), "v210");
    let root = Path::new(r"C:\RustDeskAgentLab\r4-v210-evidence");
    workspace::create(root).unwrap();
    let bundle_root = Path::new(r"C:\ProgramData\RustDesk\ManagementAgent\rescue\current");
    let bundle_bytes = std::fs::read(bundle_root.join("bundle.json")).unwrap();
    let bundle: serde_json::Value = serde_json::from_slice(&bundle_bytes).unwrap();
    let public = rescue_assets::fixed_source_public_fixture(&bundle).unwrap();
    let manifest: UpdateManifest =
        serde_json::from_slice(&std::fs::read(bundle_root.join("approved-manifest.json")).unwrap())
            .unwrap();
    let key = "windows-x86_64-msi-sos";
    let target =
        package_gate::verify_with_key(&manifest, key, 2026100502, "fixture-key", &public).unwrap();
    assert!(package_gate::verify(&manifest, key, 2026100502).is_err());
    let package = bundle_root.join(package_gate::cache_name(&target.primary, "msi").unwrap());
    let _package_lock = package_gate::lock_verified_file(&package, target).unwrap();
    let normal = Path::new(r"C:\Program Files\RustDesk");
    let backup = Path::new(r"C:\Program Files\RustDesk.r4-v210-owned-backup");
    assert!(!backup.exists());
    let normal_inventory = directory_backup::capture(normal, &root.join("normal-before")).unwrap();
    let fallback = Path::new(r"C:\ProgramData\RustDesk\Client\sos\RustDesk");
    let fallback_inventory =
        directory_backup::capture(fallback, &root.join("fallback-before")).unwrap();
    let identity = hbb_common::config::Config::file();
    let identity_bytes = std::fs::read(&identity).unwrap();
    let state: serde_json::Value = serde_json::from_slice(
        &std::fs::read(r"C:\ProgramData\RustDesk\ManagementAgent\state\agent.json").unwrap(),
    )
    .unwrap();
    let normal_contract = service_control::Contract {
        name: "RustDesk".into(),
        executable: normal.join("RustDesk.exe"),
        arguments: vec!["--service".into()],
        start_type: windows_service::service::ServiceStartType::AutoStart,
    };
    let fallback_contract = service_control::Contract {
        executable: fallback.join("RustDesk.exe"),
        ..normal_contract.clone()
    };
    #[link(name = "kernel32")]
    extern "system" {
        fn CreateMutexW(a: *const std::ffi::c_void, o: i32, n: *const u16)
            -> *mut std::ffi::c_void;
        fn WaitForSingleObject(h: *mut std::ffi::c_void, t: u32) -> u32;
        fn ReleaseMutex(h: *mut std::ffi::c_void) -> i32;
        fn CloseHandle(h: *mut std::ffi::c_void) -> i32;
    }
    struct Guard(*mut std::ffi::c_void);
    impl Drop for Guard {
        fn drop(&mut self) {
            unsafe {
                ReleaseMutex(self.0);
                CloseHandle(self.0);
            }
        }
    }
    let name = "Global\\RustDeskManagementAgentInstall"
        .encode_utf16()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let handle = unsafe { CreateMutexW(std::ptr::null(), 0, name.as_ptr()) };
    assert!(!handle.is_null());
    assert_eq!(unsafe { WaitForSingleObject(handle, 0) }, 0);
    let _guard = Guard(handle);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(
        || -> Result<(), String> {
            let health = health_windows::probe_blocking(
                r"\\.\pipe\RustDesk\management-health-v2",
                &normal_contract.executable,
                2026100502,
                Duration::from_secs(3),
            )?;
            if health.no_active_sessions != Some(true) {
                return Err("active or unknown sessions".into());
            }
            service_control::stop(&normal_contract, Duration::from_secs(60))?;
            std::fs::rename(normal, backup).map_err(|e| e.to_string())?;
            if !rescue_recovery::recover_fixture(&public)? {
                return Err("fallback did not recover".into());
            }
            let ready = health_windows::probe_blocking(
                r"\\.\pipe\RustDesk\management-health-v2",
                &fallback_contract.executable,
                2026100502,
                Duration::from_secs(3),
            )?;
            if ready.no_active_sessions != Some(true) {
                return Err("fallback sessions unknown".into());
            }
            let id = uuid::Uuid::new_v4().simple().to_string();
            let command = uuid::Uuid::new_v4().simple().to_string();
            let transaction = root.join("candidate");
            let request = preparation::Request {
                transaction_id: &id,
                command_id: &command,
                installation_id: state["agent_installation_id"].as_str().unwrap(),
                target_key: key,
                current_build: 2026100502,
                package: &package,
                manifest: &manifest,
                rollback_package: &package,
                rollback_manifest: &manifest,
            };
            let prepared = preparation::prepare_fixture(&transaction, &request, &public);
            let error = prepared.as_ref().err().cloned();
            assert!(
                !transaction.exists(),
                "rejection must precede transaction publication"
            );
            assert_eq!(std::fs::read(&identity).unwrap(), identity_bytes);
            assert!(fallback_inventory.matches_directory(fallback)?);
            let after = health_windows::probe_blocking(
                r"\\.\pipe\RustDesk\management-health-v1",
                &fallback_contract.executable,
                2026100502,
                Duration::from_secs(3),
            )?;
            assert_eq!(after.pid, ready.pid);
            save(
                &root.join("blocker.json"),
                &serde_json::json!({"package_verified":true,"network_required":false,"normal_missing":!normal.exists(),"fallback_ready":true,"fallback_pid_unchanged":true,"identity_unchanged":true,"preparation_error":error,"transaction_created":transaction.exists()}),
            );
            println!(
                "R4_PREPARATION_ERROR={error:?};FALLBACK_STILL_READY=true;IDENTITY_UNCHANGED=true"
            );
            prepared
                .map(|_| ())
                .map_err(|error| format!("R4_CAPABILITY_BLOCKED: {error}"))
        },
    ));
    if backup.exists() {
        let manager = windows_service::service_manager::ServiceManager::local_computer(
            None::<&str>,
            windows_service::service_manager::ServiceManagerAccess::CONNECT,
        )
        .unwrap();
        let service = manager
            .open_service(
                "RustDesk",
                windows_service::service::ServiceAccess::QUERY_STATUS,
            )
            .unwrap();
        if service.query_status().unwrap().current_state
            == windows_service::service::ServiceState::Running
        {
            service_control::stop(&fallback_contract, Duration::from_secs(60)).unwrap();
        }
        assert!(!normal.exists());
        std::fs::rename(backup, normal).unwrap();
        assert!(std::process::Command::new(r"C:\Windows\System32\sc.exe")
            .args([
                "config",
                "RustDesk",
                "binPath=",
                r#""C:\Program Files\RustDesk\RustDesk.exe" --service"#
            ])
            .status()
            .unwrap()
            .success());
        service_control::start(&normal_contract, Duration::from_secs(60)).unwrap();
    }
    let deadline = std::time::Instant::now() + Duration::from_secs(60);
    while !native_install::client_ready(&normal_contract.executable, 2026100502, true)
        && std::time::Instant::now() < deadline
    {
        std::thread::sleep(Duration::from_millis(250));
    }
    assert!(native_install::client_ready(
        &normal_contract.executable,
        2026100502,
        true
    ));
    assert!(normal_inventory.matches_directory(normal).unwrap());
    assert!(fallback_inventory.matches_directory(fallback).unwrap());
    assert_eq!(std::fs::read(&identity).unwrap(), identity_bytes);
    assert_eq!(
        std::fs::read(bundle_root.join("bundle.json")).unwrap(),
        bundle_bytes
    );
    println!("R4_FIXTURE_ORIGINAL_RESTORED=true;MSI_NOT_EXECUTED=true");
    match result {
        Ok(Ok(())) => {}
        Ok(Err(error)) => panic!("{error}"),
        Err(error) => std::panic::resume_unwind(error),
    }
}

#[test]
fn review5_whole_missing_directory_is_distinct_from_reparse_or_unreadable() {
    let (root, inventory) = review_inventory();
    let missing = root.join("missing");
    assert!(rescue_assets::r3_classify_fixture(&inventory, &missing, false).unwrap());
    std::os::windows::fs::symlink_dir(root.join("normal"), &missing).unwrap();
    assert!(
        rescue_assets::r3_classify_fixture(&inventory, &missing, false)
            .unwrap_err()
            .contains("reparse")
    );
    std::fs::remove_dir(&missing).unwrap();
    assert_eq!(
        std::fs::read(root.join("normal/RustDesk.exe")).unwrap(),
        b"approved A executable"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn review5_non_directory_or_file_acl_error_is_not_missing() {
    let (root, inventory) = review_inventory();
    let path = root.join("normal");
    let file = path.join("RustDesk.exe");
    assert!(
        std::process::Command::new(r"C:\Windows\System32\icacls.exe")
            .arg(&file)
            .args(["/deny", "*S-1-5-18:(R)"])
            .status()
            .unwrap()
            .success()
    );
    let result = rescue_assets::r3_classify_fixture(&inventory, &path, false);
    assert!(
        result.is_err(),
        "unreadable or unsupported ACL must not classify as missing"
    );
    assert!(
        std::process::Command::new(r"C:\Windows\System32\icacls.exe")
            .arg(&file)
            .args(["/remove:d", "*S-1-5-18"])
            .status()
            .unwrap()
            .success()
    );
    assert_eq!(std::fs::read(&file).unwrap(), b"approved A executable");
    let not_directory = root.join("file-not-directory");
    std::fs::write(&not_directory, b"keep").unwrap();
    assert!(
        rescue_assets::r3_classify_fixture(&inventory, &not_directory, false)
            .unwrap_err()
            .contains("not directory")
    );
    assert_eq!(std::fs::read(&not_directory).unwrap(), b"keep");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
#[ignore = "Win11 fixed fallback restart and whole-directory fault; v208 runner"]
fn review_e2e_whole_directory_missing_and_existing_fallback_second_start() {
    assert_eq!(std::env::var("COMPUTERNAME").unwrap(), "RDLAB-WIN11");
    assert_eq!(std::env::var("RUSTDESK_RESCUE_E2E").unwrap(), "v208");
    let evidence = Path::new(r"C:\RustDeskAgentLab\r3-review-v208");
    workspace::create(evidence).unwrap();
    let bundle = Path::new(r"C:\ProgramData\RustDesk\ManagementAgent\rescue\current");
    let bundle_bytes = std::fs::read(bundle.join("bundle.json")).unwrap();
    let record: serde_json::Value = serde_json::from_slice(&bundle_bytes).unwrap();
    let public = rescue_assets::fixed_source_public_fixture(&record).unwrap();
    let normal = Path::new(r"C:\Program Files\RustDesk");
    let backup = Path::new(r"C:\Program Files\RustDesk.r3-v208-owned-backup");
    assert!(!backup.exists());
    let inventory =
        directory_backup::capture(normal, &evidence.join("original-inventory")).unwrap();
    let fallback = Path::new(r"C:\ProgramData\RustDesk\Client\sos\RustDesk");
    assert!(fallback.join("RustDesk.exe").is_file());
    let fallback_inventory =
        directory_backup::capture(fallback, &evidence.join("fallback-inventory")).unwrap();
    let identity = hbb_common::config::Config::file();
    let identity_bytes = std::fs::read(&identity).unwrap();
    let contract = service_control::Contract {
        name: "RustDesk".into(),
        executable: normal.join("RustDesk.exe"),
        arguments: vec!["--service".into()],
        start_type: windows_service::service::ServiceStartType::AutoStart,
    };
    let fallback_contract = service_control::Contract {
        executable: fallback.join("RustDesk.exe"),
        ..contract.clone()
    };
    #[link(name = "kernel32")]
    extern "system" {
        fn CreateMutexW(a: *const std::ffi::c_void, o: i32, n: *const u16)
            -> *mut std::ffi::c_void;
        fn WaitForSingleObject(h: *mut std::ffi::c_void, t: u32) -> u32;
        fn ReleaseMutex(h: *mut std::ffi::c_void) -> i32;
        fn CloseHandle(h: *mut std::ffi::c_void) -> i32;
    }
    struct Guard(*mut std::ffi::c_void);
    impl Drop for Guard {
        fn drop(&mut self) {
            unsafe {
                ReleaseMutex(self.0);
                CloseHandle(self.0);
            }
        }
    }
    let name = "Global\\RustDeskManagementAgentInstall"
        .encode_utf16()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let handle = unsafe { CreateMutexW(std::ptr::null(), 0, name.as_ptr()) };
    assert!(!handle.is_null());
    assert_eq!(unsafe { WaitForSingleObject(handle, 0) }, 0);
    let _guard = Guard(handle);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(
        || -> Result<(), String> {
            let response = health_windows::probe_blocking(
                r"\\.\pipe\RustDesk\management-health-v2",
                &contract.executable,
                2026100502,
                Duration::from_secs(3),
            )?;
            if response.no_active_sessions != Some(true) {
                return Err("active/unknown sessions".into());
            }
            service_control::stop(&contract, Duration::from_secs(60))?;
            std::fs::rename(normal, backup).map_err(|e| e.to_string())?;
            let mut budget = recovery::RetryBudget::default();
            let mut now = 1000;
            for cycle in 1..=2 {
                let result = rescue_recovery::budgeted(
                    Duration::from_secs(180),
                    &mut budget,
                    now,
                    |next| {
                        std::fs::write(evidence.join("budget.json"),serde_json::to_vec(&serde_json::json!({"failures":next.failures,"next_at":next.next_at})).unwrap()).map_err(|e|e.to_string())
                    },
                    || rescue_recovery::periodic_fixture(Duration::from_secs(180), &public),
                )?;
                assert_eq!(result, rescue_recovery::Cycle::Recovered);
                let health = health_windows::probe_blocking(
                    r"\\.\pipe\RustDesk\management-health-v1",
                    &fallback_contract.executable,
                    record["build_seq"].as_u64().unwrap(),
                    Duration::from_secs(3),
                )?;
                assert!(!normal.exists());
                assert!(fallback_inventory.matches_directory(fallback)?);
                assert_eq!(std::fs::read(&identity).unwrap(), identity_bytes);
                println!("REVIEW_R3_CYCLE={cycle};whole_normal_missing=true;existing_fallback_reused=true;pid={};build={}",health.pid,health.build_seq);
                if cycle == 1 {
                    service_control::stop(&fallback_contract, Duration::from_secs(60))?;
                    now = budget.next_at;
                }
            }
            Ok(())
        },
    ));
    if backup.exists() {
        let manager = windows_service::service_manager::ServiceManager::local_computer(
            None::<&str>,
            windows_service::service_manager::ServiceManagerAccess::CONNECT,
        )
        .unwrap();
        let service = manager
            .open_service(
                "RustDesk",
                windows_service::service::ServiceAccess::QUERY_STATUS,
            )
            .unwrap();
        if service.query_status().unwrap().current_state
            == windows_service::service::ServiceState::Running
        {
            service_control::stop(&fallback_contract, Duration::from_secs(60)).unwrap();
        }
        assert!(!normal.exists());
        std::fs::rename(backup, normal).unwrap();
        assert!(std::process::Command::new(r"C:\Windows\System32\sc.exe")
            .args([
                "config",
                "RustDesk",
                "binPath=",
                r#""C:\Program Files\RustDesk\RustDesk.exe" --service"#
            ])
            .status()
            .unwrap()
            .success());
        service_control::start(&contract, Duration::from_secs(60)).unwrap();
    }
    let deadline = std::time::Instant::now() + Duration::from_secs(60);
    while !native_install::client_ready(&contract.executable, 2026100502, true)
        && std::time::Instant::now() < deadline
    {
        std::thread::sleep(Duration::from_millis(250));
    }
    assert!(native_install::client_ready(
        &contract.executable,
        2026100502,
        true
    ));
    assert!(inventory.matches_directory(normal).unwrap());
    assert!(fallback_inventory.matches_directory(fallback).unwrap());
    assert_eq!(std::fs::read(&identity).unwrap(), identity_bytes);
    assert_eq!(
        std::fs::read(bundle.join("bundle.json")).unwrap(),
        bundle_bytes
    );
    println!("REVIEW_ORIGINAL_DIRECTORY_RESTORED=true;identity_and_fallback_unchanged=true");
    match result {
        Ok(Ok(())) => {}
        Ok(Err(error)) => panic!("E2E failed after restore: {error}"),
        Err(error) => std::panic::resume_unwind(error),
    }
}

#[test]
fn r3_invalid_bundle_identity_acl_or_occupied_destination_preserves_old_bytes() {
    let f = Fixture::new();
    f.prepare().unwrap();
    let bundle = f.root.join("rescue/current");
    let identity = f.root.join("current/identity.toml");
    let destination = f.root.join("fallback");
    let before = std::fs::read(bundle.join("bundle.json")).unwrap();
    let original = std::fs::read(&identity).unwrap();
    rescue_assets::r3_validate_fixture(&bundle, f.public.as_ref(), &identity, &destination)
        .unwrap();
    let (wrong, _) = sign::gen_keypair();
    assert!(
        rescue_assets::r3_validate_fixture(&bundle, wrong.as_ref(), &identity, &destination)
            .unwrap_err()
            .contains("signature")
    );
    let package = bundle.join("agent-sos-v187.msi");
    let bytes = std::fs::read(&package).unwrap();
    std::fs::write(&package, b"invalid package").unwrap();
    assert!(rescue_assets::r3_validate_fixture(
        &bundle,
        f.public.as_ref(),
        &identity,
        &destination
    )
    .unwrap_err()
    .contains("size mismatch"));
    std::fs::write(&package, &bytes).unwrap();
    let saved = identity.with_extension("saved");
    std::fs::rename(&identity, &saved).unwrap();
    assert!(rescue_assets::r3_validate_fixture(
        &bundle,
        f.public.as_ref(),
        &identity,
        &destination
    )
    .unwrap_err()
    .contains("unavailable"));
    std::fs::rename(&saved, &identity).unwrap();
    std::fs::write(&identity, b"different original identity").unwrap();
    assert!(rescue_assets::r3_validate_fixture(
        &bundle,
        f.public.as_ref(),
        &identity,
        &destination
    )
    .unwrap_err()
    .contains("identity differs"));
    assert_eq!(
        std::fs::read(&identity).unwrap(),
        b"different original identity"
    );
    std::fs::write(&identity, &original).unwrap();
    workspace::create(&destination).unwrap();
    std::fs::write(destination.join("keep"), b"old fallback").unwrap();
    assert!(rescue_assets::r3_validate_fixture(
        &bundle,
        f.public.as_ref(),
        &identity,
        &destination
    )
    .unwrap_err()
    .contains("occupied"));
    assert_eq!(
        std::fs::read(destination.join("keep")).unwrap(),
        b"old fallback"
    );
    assert!(
        std::process::Command::new(r"C:\Windows\System32\icacls.exe")
            .arg(&package)
            .args(["/grant", "*S-1-5-32-545:(W)"])
            .status()
            .unwrap()
            .success()
    );
    assert!(rescue_assets::r3_validate_fixture(
        &bundle,
        f.public.as_ref(),
        &identity,
        &destination
    )
    .unwrap_err()
    .contains("permission"));
    assert_eq!(std::fs::read(bundle.join("bundle.json")).unwrap(), before);
    assert_eq!(std::fs::read(&identity).unwrap(), original);
}

#[test]
#[ignore = "Real R3 Win11 fault, exact owned EXE backup and restoration; v206 runner only"]
fn r3_real_missing_exe_uses_service_fallback_and_restores_original() {
    assert_eq!(std::env::var("COMPUTERNAME").unwrap(), "RDLAB-WIN11");
    assert_eq!(std::env::var("RUSTDESK_RESCUE_E2E").unwrap(), "v206");
    let root = Path::new(r"C:\ProgramData\RustDesk\ManagementAgent\rescue\current");
    let bundle: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("bundle.json")).unwrap()).unwrap();
    let public = rescue_assets::fixed_source_public_fixture(&bundle).unwrap();
    let exe = Path::new(r"C:\Program Files\RustDesk\RustDesk.exe");
    let backup = exe.with_file_name("RustDesk.exe.r3-v206-owned-backup");
    assert!(!backup.exists());
    let original = std::fs::read(exe).unwrap();
    let identity = hbb_common::config::Config::file();
    let identity_bytes = std::fs::read(&identity).unwrap();
    let normal = service_control::Contract {
        name: "RustDesk".into(),
        executable: exe.into(),
        arguments: vec!["--service".into()],
        start_type: windows_service::service::ServiceStartType::AutoStart,
    };
    let observed = service_control::inspect(&normal).unwrap();
    // Hold the production mutex so the installed old agent cannot race this fault fixture.
    #[link(name = "kernel32")]
    extern "system" {
        fn CreateMutexW(
            attributes: *const std::ffi::c_void,
            owner: i32,
            name: *const u16,
        ) -> *mut std::ffi::c_void;
        fn WaitForSingleObject(handle: *mut std::ffi::c_void, timeout: u32) -> u32;
        fn ReleaseMutex(handle: *mut std::ffi::c_void) -> i32;
        fn CloseHandle(handle: *mut std::ffi::c_void) -> i32;
    }
    struct Guard(*mut std::ffi::c_void);
    impl Drop for Guard {
        fn drop(&mut self) {
            unsafe {
                ReleaseMutex(self.0);
                CloseHandle(self.0);
            }
        }
    }
    let name = "Global\\RustDeskManagementAgentInstall"
        .encode_utf16()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let handle = unsafe { CreateMutexW(std::ptr::null(), 0, name.as_ptr()) };
    assert!(!handle.is_null());
    assert_eq!(unsafe { WaitForSingleObject(handle, 0) }, 0);
    let _guard = Guard(handle);
    let result = (|| -> Result<(), String> {
        assert!(
            !rescue_recovery::recover_fixture(&public)?,
            "healthy normal path is inert"
        );
        assert_eq!(service_control::inspect(&normal)?.pid, observed.pid);
        let activity = health_windows::probe_blocking(
            r"\\.\pipe\RustDesk\management-health-v2",
            exe,
            2026100502,
            Duration::from_secs(3),
        )?;
        if activity.no_active_sessions != Some(true) {
            return Err("fixture active or unknown sessions".into());
        }
        service_control::stop(&normal, Duration::from_secs(60))?;
        std::fs::rename(exe, &backup).map_err(|e| e.to_string())?;
        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        while std::time::Instant::now() < deadline {
            let output = std::process::Command::new(r"C:\Windows\System32\tasklist.exe")
                .args(["/FI", "IMAGENAME eq RustDesk.exe", "/FO", "CSV", "/NH"])
                .output()
                .map_err(|e| e.to_string())?;
            if !String::from_utf8_lossy(&output.stdout)
                .to_ascii_lowercase()
                .contains("rustdesk.exe")
            {
                break;
            }
            std::thread::sleep(Duration::from_millis(250));
        }
        if !rescue_recovery::recover_fixture(&public)? {
            return Err("R3 did not recover".into());
        }
        let fallback = Path::new(r"C:\ProgramData\RustDesk\Client\sos\RustDesk\RustDesk.exe");
        let response = health_windows::probe_blocking(
            r"\\.\pipe\RustDesk\management-health-v1",
            fallback,
            2026100502,
            Duration::from_secs(3),
        )?;
        assert!(response.local_responsive);
        assert_eq!(std::fs::read(&identity).unwrap(), identity_bytes);
        assert!(!exe.exists());
        assert_eq!(std::fs::read(&backup).unwrap(), original);
        println!(
            "R3_FALLBACK_READY=degraded;pid={};build={};identity_unchanged=true",
            response.pid, response.build_seq
        );
        Ok(())
    })();
    // Restoration is owned by this test, never a snapshot rollback or a product repair claim.
    if backup.exists() {
        let manager = windows_service::service_manager::ServiceManager::local_computer(
            None::<&str>,
            windows_service::service_manager::ServiceManagerAccess::CONNECT,
        )
        .unwrap();
        let service = manager
            .open_service(
                "RustDesk",
                windows_service::service::ServiceAccess::QUERY_CONFIG
                    | windows_service::service::ServiceAccess::QUERY_STATUS,
            )
            .unwrap();
        let config = service.query_config().unwrap();
        if service.query_status().unwrap().current_state
            == windows_service::service::ServiceState::Running
        {
            let running = service_control::Contract {
                executable: if config
                    .executable_path
                    .to_string_lossy()
                    .contains("ProgramData")
                {
                    r"C:\ProgramData\RustDesk\Client\sos\RustDesk\RustDesk.exe".into()
                } else {
                    exe.into()
                },
                ..normal.clone()
            };
            service_control::stop(&running, Duration::from_secs(60)).unwrap();
        }
        assert!(!exe.exists());
        std::fs::rename(&backup, exe).unwrap();
        assert!(std::process::Command::new(r"C:\Windows\System32\sc.exe")
            .args([
                "config",
                "RustDesk",
                "binPath=",
                r#""C:\Program Files\RustDesk\RustDesk.exe" --service"#
            ])
            .status()
            .unwrap()
            .success());
        service_control::start(&normal, Duration::from_secs(60)).unwrap();
    } else if service_control::inspect(&normal).unwrap().state
        == windows_service::service::ServiceState::Stopped
    {
        service_control::start(&normal, Duration::from_secs(60)).unwrap();
    }
    let deadline = std::time::Instant::now() + Duration::from_secs(60);
    while !native_install::client_ready(exe, 2026100502, true)
        && std::time::Instant::now() < deadline
    {
        std::thread::sleep(Duration::from_millis(250));
    }
    assert!(native_install::client_ready(exe, 2026100502, true));
    assert_eq!(std::fs::read(exe).unwrap(), original);
    assert_eq!(std::fs::read(&identity).unwrap(), identity_bytes);
    println!("R3_ORIGINAL_RESTORED=true;health=ready;identity_unchanged=true");
    assert!(result.is_ok(), "R3 result after restoration: {result:?}");
}

#[test]
fn r3_periodic_process_child() {
    let Some(root) = std::env::var_os("RUSTDESK_R3_PERIODIC_FIXTURE") else {
        return;
    };
    assert_eq!(std::env::var("COMPUTERNAME").unwrap(), "RDLAB-WIN11");
    let root = PathBuf::from(root);
    assert_eq!(
        root,
        Path::new(r"C:\RustDeskAgentLab\r3-periodic-v207-retry1")
    );
    workspace::verify(&root).unwrap();
    let public = std::fs::read(root.join("public.bin")).unwrap();
    let started = std::time::Instant::now();
    let mut iteration = 0u64;
    let (_tx, rx) = std::sync::mpsc::channel::<()>();
    while started.elapsed() < Duration::from_secs(600) && !root.join("stop").exists() {
        let _ = rx.recv_timeout(Duration::from_secs(1));
        if root.join("stop").exists() {
            break;
        }
        iteration += 1;
        let outcome = rescue_recovery::periodic_fixture(started.elapsed(), &public);
        let row = serde_json::json!({"iteration":iteration,"elapsed_ms":started.elapsed().as_millis(),"outcome":match &outcome {Ok(value)=>format!("{value:?}"),Err(error)=>format!("Error: {error}")}});
        use std::io::Write;
        let mut log = std::fs::OpenOptions::new()
            .append(true)
            .create(true)
            .open(root.join("iterations.jsonl"))
            .unwrap();
        writeln!(log, "{row}").unwrap();
        log.sync_all().unwrap();
        if outcome == Ok(rescue_recovery::Cycle::Recovered) {
            break;
        }
    }
}

#[test]
#[ignore = "Independent SYSTEM periodic process, real fault and owned restoration; v207 runner"]
fn r3_automatic_periodic_process_honors_gates_recovers_and_restores() {
    assert_eq!(std::env::var("COMPUTERNAME").unwrap(), "RDLAB-WIN11");
    assert_eq!(std::env::var("RUSTDESK_RESCUE_E2E").unwrap(), "v207");
    let root = Path::new(r"C:\RustDeskAgentLab\r3-periodic-v207-retry1");
    workspace::create(root).unwrap();
    let bundle_path = Path::new(r"C:\ProgramData\RustDesk\ManagementAgent\rescue\current");
    let bundle_bytes = std::fs::read(bundle_path.join("bundle.json")).unwrap();
    let bundle: serde_json::Value = serde_json::from_slice(&bundle_bytes).unwrap();
    let public = rescue_assets::fixed_source_public_fixture(&bundle).unwrap();
    std::fs::write(root.join("public.bin"), &public).unwrap();
    let normal_exe = Path::new(r"C:\Program Files\RustDesk\RustDesk.exe");
    let backup = normal_exe.with_file_name("RustDesk.exe.r3-v207-owned-backup");
    assert!(!backup.exists());
    let original = std::fs::read(normal_exe).unwrap();
    let identity = hbb_common::config::Config::file();
    let identity_bytes = std::fs::read(&identity).unwrap();
    let intent = hbb_common::config::Config2::file();
    let intent_bytes = std::fs::read(&intent).unwrap();
    let state = Path::new(r"C:\ProgramData\RustDesk\ManagementAgent\state");
    for name in ["disable-agent", "disable-recovery", "maintenance.json"] {
        assert!(!state.join(name).exists());
    }
    let fallback = Path::new(r"C:\ProgramData\RustDesk\Client\sos\RustDesk");
    let old_fallback = fallback.with_file_name("RustDesk.v207-owned-previous");
    assert!(!old_fallback.exists());
    let staging = Path::new(r"C:\ProgramData\RustDesk\ManagementAgent\rescue\r3-staging");
    let old_staging = staging.with_file_name("r3-staging.v207-owned-previous");
    assert!(!old_staging.exists());
    let normal = service_control::Contract {
        name: "RustDesk".into(),
        executable: normal_exe.into(),
        arguments: vec!["--service".into()],
        start_type: windows_service::service::ServiceStartType::AutoStart,
    };
    let before_pid = service_control::inspect(&normal).unwrap().pid;
    #[link(name = "kernel32")]
    extern "system" {
        fn CreateMutexW(
            attributes: *const std::ffi::c_void,
            owner: i32,
            name: *const u16,
        ) -> *mut std::ffi::c_void;
        fn WaitForSingleObject(handle: *mut std::ffi::c_void, timeout: u32) -> u32;
        fn ReleaseMutex(handle: *mut std::ffi::c_void) -> i32;
        fn CloseHandle(handle: *mut std::ffi::c_void) -> i32;
    }
    struct Guard(*mut std::ffi::c_void);
    impl Drop for Guard {
        fn drop(&mut self) {
            unsafe {
                ReleaseMutex(self.0);
                CloseHandle(self.0);
            }
        }
    }
    let name = "Global\\RustDeskManagementAgentInstall"
        .encode_utf16()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let handle = unsafe { CreateMutexW(std::ptr::null(), 0, name.as_ptr()) };
    assert!(!handle.is_null());
    assert_eq!(unsafe { WaitForSingleObject(handle, 0) }, 0);
    let _guard = Guard(handle);
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "r3_periodic_process_child", "--nocapture"])
        .env("RUSTDESK_R3_PERIODIC_FIXTURE", root)
        .stdout(std::fs::File::create(root.join("child.stdout.txt")).unwrap())
        .stderr(std::fs::File::create(root.join("child.stderr.txt")).unwrap())
        .spawn()
        .unwrap();
    let rows = || -> Vec<serde_json::Value> {
        std::fs::read_to_string(root.join("iterations.jsonl"))
            .unwrap_or_default()
            .lines()
            .filter_map(|line| serde_json::from_str(line).ok())
            .collect()
    };
    let wait = |after: u64,
                outcome: &str,
                count: usize,
                timeout: u64|
     -> Result<Vec<serde_json::Value>, String> {
        let deadline = std::time::Instant::now() + Duration::from_secs(timeout);
        loop {
            let matching = rows()
                .into_iter()
                .filter(|row| {
                    row["iteration"].as_u64().unwrap() > after && row["outcome"] == outcome
                })
                .collect::<Vec<_>>();
            if matching.len() >= count {
                return Ok(matching);
            }
            if std::time::Instant::now() >= deadline {
                return Err(format!("periodic observation timed out for {outcome}"));
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    };
    let mut owned_markers = Vec::<PathBuf>::new();
    let mut changed_intent = false;
    let mut moved_fallback = false;
    let mut moved_staging = false;
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(
        || -> Result<(), String> {
            wait(0, "Deferred", 2, 10)?;
            let healthy = wait(0, "Unchanged", 3, 210)?;
            assert!(healthy[0]["elapsed_ms"].as_u64().unwrap() >= 180000);
            assert_eq!(service_control::inspect(&normal)?.pid, before_pid);
            println!(
                "AUTO_HEALTHY_ITERATIONS={};STARTUP_GRACE_REAL_MS={}",
                healthy.len(),
                healthy[0]["elapsed_ms"]
            );
            let activity = health_windows::probe_blocking(
                r"\\.\pipe\RustDesk\management-health-v2",
                normal_exe,
                2026100502,
                Duration::from_secs(3),
            )?;
            if activity.no_active_sessions != Some(true) {
                return Err("active or unknown sessions".into());
            }
            let stop_marker = state.join("disable-recovery");
            std::fs::write(&stop_marker, b"v207-owned").map_err(|e| e.to_string())?;
            owned_markers.push(stop_marker.clone());
            let last = rows().last().unwrap()["iteration"].as_u64().unwrap();
            wait(last, "Deferred", 3, 15)?;
            service_control::stop(&normal, Duration::from_secs(60))?;
            std::fs::rename(normal_exe, &backup).map_err(|e| e.to_string())?;
            if fallback.exists() {
                std::fs::rename(fallback, &old_fallback).map_err(|e| e.to_string())?;
                moved_fallback = true;
            }
            if staging.exists() {
                std::fs::rename(staging, &old_staging).map_err(|e| e.to_string())?;
                moved_staging = true;
            }
            for marker in ["disable-agent", "maintenance.json"] {
                let path = state.join(marker);
                std::fs::write(&path, b"v207-owned").map_err(|e| e.to_string())?;
                owned_markers.push(path.clone());
                std::fs::remove_file(&stop_marker).map_err(|e| e.to_string())?;
                let last = rows().last().unwrap()["iteration"].as_u64().unwrap();
                let blocked = wait(last, "Deferred", 3, 15)?;
                assert!(!fallback.exists());
                println!("AUTO_GATE={marker};blocked_iterations={}", blocked.len());
                std::fs::write(&stop_marker, b"v207-owned").map_err(|e| e.to_string())?;
                std::fs::remove_file(&path).map_err(|e| e.to_string())?;
            }
            let mut config: hbb_common::toml::Value =
                hbb_common::toml::from_str(std::str::from_utf8(&intent_bytes).unwrap()).unwrap();
            config
                .as_table_mut()
                .unwrap()
                .entry("options".to_owned())
                .or_insert_with(|| hbb_common::toml::Value::Table(Default::default()))
                .as_table_mut()
                .unwrap()
                .insert(
                    "stop-service".into(),
                    hbb_common::toml::Value::String("Y".into()),
                );
            std::fs::write(&intent, hbb_common::toml::to_string(&config).unwrap())
                .map_err(|e| e.to_string())?;
            changed_intent = true;
            std::fs::remove_file(&stop_marker).map_err(|e| e.to_string())?;
            let last = rows().last().unwrap()["iteration"].as_u64().unwrap();
            let blocked = wait(last, "Deferred", 3, 15)?;
            assert!(!fallback.exists());
            println!("AUTO_GATE=user-stop;blocked_iterations={}", blocked.len());
            std::fs::write(&intent, &intent_bytes).map_err(|e| e.to_string())?;
            changed_intent = false;
            let last = rows().last().unwrap()["iteration"].as_u64().unwrap();
            let recovered = wait(last, "Recovered", 1, 240)?;
            let response = health_windows::probe_blocking(
                r"\\.\pipe\RustDesk\management-health-v1",
                &fallback.join("RustDesk.exe"),
                bundle["build_seq"].as_u64().unwrap(),
                Duration::from_secs(3),
            )?;
            assert_eq!(std::fs::read(&identity).unwrap(), identity_bytes);
            println!("AUTO_RECOVERED_ITERATION={};PID={};BUILD={};SOURCE_TRANSACTION={};MODE=independent_test_process",recovered[0]["iteration"],response.pid,response.build_seq,bundle["source_transaction"]);
            Ok(())
        },
    ));
    std::fs::write(root.join("stop"), b"stop").unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(180);
    while child.try_wait().unwrap().is_none() && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(
        child.try_wait().unwrap().is_some(),
        "preserve running child; do not race restore"
    );
    // Restore only test-owned markers/config/EXE and original fixed service, retaining new fallback evidence.
    if changed_intent {
        std::fs::write(&intent, &intent_bytes).unwrap();
    }
    for path in owned_markers {
        if path.exists() {
            assert_eq!(std::fs::read(&path).unwrap(), b"v207-owned");
            std::fs::remove_file(path).unwrap();
        }
    }
    if backup.exists() {
        let manager = windows_service::service_manager::ServiceManager::local_computer(
            None::<&str>,
            windows_service::service_manager::ServiceManagerAccess::CONNECT,
        )
        .unwrap();
        let service = manager
            .open_service(
                "RustDesk",
                windows_service::service::ServiceAccess::QUERY_CONFIG
                    | windows_service::service::ServiceAccess::QUERY_STATUS,
            )
            .unwrap();
        if service.query_status().unwrap().current_state
            == windows_service::service::ServiceState::Running
        {
            service_control::stop(
                &service_control::Contract {
                    executable: fallback.join("RustDesk.exe"),
                    ..normal.clone()
                },
                Duration::from_secs(60),
            )
            .unwrap();
        }
        assert!(!normal_exe.exists());
        std::fs::rename(&backup, normal_exe).unwrap();
        assert!(std::process::Command::new(r"C:\Windows\System32\sc.exe")
            .args([
                "config",
                "RustDesk",
                "binPath=",
                r#""C:\Program Files\RustDesk\RustDesk.exe" --service"#
            ])
            .status()
            .unwrap()
            .success());
        service_control::start(&normal, Duration::from_secs(60)).unwrap();
    }
    if moved_fallback {
        if fallback.exists() {
            std::fs::rename(fallback, root.join("new-fallback-evidence")).unwrap();
        }
        std::fs::rename(&old_fallback, fallback).unwrap();
    }
    if moved_staging {
        if staging.exists() {
            std::fs::rename(staging, root.join("new-staging-evidence")).unwrap();
        }
        std::fs::rename(&old_staging, staging).unwrap();
    }
    let deadline = std::time::Instant::now() + Duration::from_secs(60);
    while !native_install::client_ready(normal_exe, 2026100502, true)
        && std::time::Instant::now() < deadline
    {
        std::thread::sleep(Duration::from_millis(250));
    }
    assert!(native_install::client_ready(normal_exe, 2026100502, true));
    assert_eq!(std::fs::read(normal_exe).unwrap(), original);
    assert_eq!(std::fs::read(&intent).unwrap(), intent_bytes);
    assert_eq!(std::fs::read(&identity).unwrap(), identity_bytes);
    assert_eq!(
        std::fs::read(bundle_path.join("bundle.json")).unwrap(),
        bundle_bytes
    );
    println!(
        "AUTO_ORIGINAL_RESTORED=true;iterations={};original_data_equal=true",
        rows().len()
    );
    match result {
        Ok(Ok(())) => {}
        Ok(Err(error)) => panic!("periodic failed after restore: {error}"),
        Err(error) => std::panic::resume_unwind(error),
    }
}
