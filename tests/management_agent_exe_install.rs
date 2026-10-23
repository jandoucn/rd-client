#![cfg(windows)]
#[path = "../src/management_agent/bootstrap_update.rs"]
mod bootstrap_update;
#[path = "../src/management_agent/directory_backup.rs"]
mod directory_backup;
#[path = "../src/management_agent/exe_assets.rs"]
mod exe_assets;
#[path = "../src/management_agent/exe_install.rs"]
mod exe_install;
#[path = "../src/management_agent/exe_preparation.rs"]
mod exe_preparation;
#[path = "../src/management_agent/exe_runtime.rs"]
mod exe_runtime;
#[path = "../src/management_agent/exe_service.rs"]
mod exe_service;
#[path = "../src/management_agent/file_restore.rs"]
mod file_restore;
#[path = "../src/management_agent/handoff.rs"]
mod handoff;
#[path = "../src/management_agent/health.rs"]
mod health;
#[path = "../src/management_agent/health_windows.rs"]
mod health_windows;
#[path = "../src/management_agent/install_execution.rs"]
mod install_execution;
#[path = "../src/platform/windows_agent_guard.rs"]
mod install_guard;
#[path = "../src/management_agent/install_plan.rs"]
mod install_plan;
#[path = "../src/management_agent/journal.rs"]
mod journal;
#[path = "../src/management_agent/maintenance.rs"]
mod maintenance;
#[path = "../src/management_agent/package_gate.rs"]
mod package_gate;
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
use install_execution::Backend;
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

struct Fixture {
    root: PathBuf,
    manifest: UpdateManifest,
    public: sign::PublicKey,
}

#[test]
fn bootstrap_exe_transaction_syncs_existing_entry_and_restores_manifest_on_failure() {
    for fail in [false, true] {
        let f = Fixture::new();
        let boot = f.root.join("RecoveryBootstrap");
        workspace::create(&boot).unwrap();
        let original = std::fs::read(f.root.join("agent/agent.exe")).unwrap();
        let old_hash = hex::encode(Sha256::digest(&original));
        let manifest=serde_json::to_vec(&serde_json::json!({"schema":1,"agent_sha256":old_hash,"edition":"standard","package_kind":"exe","normal_install_path":r"C:\Program Files\RustDesk","installation_id":"a".repeat(32)})).unwrap();
        std::fs::write(boot.join("bootstrap.json"), &manifest).unwrap();
        std::fs::write(boot.join("rustdesk-recovery-bootstrap.exe"), &original).unwrap();
        std::fs::write(boot.join("budget.json"), b"budget-preserve").unwrap();
        f.prepare().unwrap();
        let (mut tx, evidence) = executable_transaction(&f);
        let mut backend = exe_install::ExeInstaller::open_fixture(&f.root, platform(fail)).unwrap();
        let now = tx.updated_at;
        let result = backend.execute(&mut tx, &evidence, now).unwrap();
        assert_eq!(
            result.phase,
            if fail {
                transaction::Phase::RolledBack
            } else {
                transaction::Phase::Committed
            }
        );
        let agent = std::fs::read(f.root.join("agent/agent.exe")).unwrap();
        let updated: serde_json::Value =
            serde_json::from_slice(&std::fs::read(boot.join("bootstrap.json")).unwrap()).unwrap();
        assert_eq!(
            std::fs::read(boot.join("rustdesk-recovery-bootstrap.exe")).unwrap(),
            agent
        );
        assert_eq!(updated["agent_sha256"], hex::encode(Sha256::digest(&agent)));
        assert_eq!(
            std::fs::read(boot.join("budget.json")).unwrap(),
            b"budget-preserve"
        );
        if fail {
            assert_eq!(
                std::fs::read(boot.join("bootstrap.json")).unwrap(),
                manifest
            );
            assert_eq!(agent, original);
        }
    }
}

#[test]
fn bootstrap_expected_runtime_state_changes_are_captured_at_stop_not_rejected_or_lost() {
    let f = Fixture::new();
    f.prepare().unwrap();
    let (mut tx, evidence) = executable_transaction(&f);
    let state = f.root.join("state/agent.json");
    let mut value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&state).unwrap()).unwrap();
    value["service_healthy_since"] = 123456.into();
    value["last_policy_check_at"] = "123456".into();
    std::fs::write(&state, serde_json::to_vec(&value).unwrap()).unwrap();
    let receipt = serde_json::json!({"schema":1,"commands":{},"events":[]});
    std::fs::write(
        f.root.join("state/commands-2.json"),
        serde_json::to_vec(&receipt).unwrap(),
    )
    .unwrap();
    let latest = std::fs::read(&state).unwrap();
    let mut backend = exe_install::ExeInstaller::open_fixture(&f.root, platform(true)).unwrap();
    let now = tx.updated_at;
    let result = backend.execute(&mut tx, &evidence, now).unwrap();
    assert_eq!(result.phase, transaction::Phase::RolledBack);
    assert_eq!(std::fs::read(&state).unwrap(), latest);
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(
            &std::fs::read(f.root.join("state/commands-2.json")).unwrap()
        )
        .unwrap(),
        receipt
    );
}

#[test]
fn bootstrap_plan_binding_rejects_manifest_tamper_and_preserves_budget() {
    let f = Fixture::new();
    let boot = f.root.join("RecoveryBootstrap");
    workspace::create(&boot).unwrap();
    let original = std::fs::read(f.root.join("agent/agent.exe")).unwrap();
    std::fs::write(boot.join("rustdesk-recovery-bootstrap.exe"), &original).unwrap();
    let manifest=serde_json::to_vec(&serde_json::json!({"schema":1,"agent_sha256":hex::encode(Sha256::digest(&original)),"edition":"standard","package_kind":"exe","normal_install_path":r"C:\Program Files\RustDesk"})).unwrap();
    std::fs::write(boot.join("bootstrap.json"), &manifest).unwrap();
    std::fs::write(boot.join("budget.json"), b"keep-budget").unwrap();
    f.prepare().unwrap();
    let metadata = f.tx().join("bootstrap-update.json");
    let mut v: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&metadata).unwrap()).unwrap();
    v["manifests"] = serde_json::Value::Null;
    std::fs::write(&metadata, serde_json::to_vec(&v).unwrap()).unwrap();
    assert!(exe_install::ExeInstaller::open_fixture(&f.root, platform(false)).is_err());
    assert_eq!(
        std::fs::read(boot.join("bootstrap.json")).unwrap(),
        manifest
    );
    assert_eq!(
        std::fs::read(boot.join("rustdesk-recovery-bootstrap.exe")).unwrap(),
        original
    );
    assert_eq!(
        std::fs::read(boot.join("budget.json")).unwrap(),
        b"keep-budget"
    );
}

#[test]
fn bootstrap_live_state_whitelist_never_allows_identity_config_or_unknown_drift() {
    for kind in ["identity", "policy", "unknown", "config", "badjournal"] {
        let f = Fixture::new();
        f.prepare().unwrap();
        let (mut tx, evidence) = executable_transaction(&f);
        if kind == "config" {
            std::fs::write(f.root.join("config/RustDesk.toml"), b"changed config").unwrap();
        } else if kind == "badjournal" {
            std::fs::write(f.root.join("state/commands-2.json"), b"malformed").unwrap();
        } else {
            let path = f.root.join("state/agent.json");
            let mut v: serde_json::Value =
                serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
            let key = match kind {
                "identity" => "agent_installation_id",
                "policy" => "management_agent_policy_revision",
                _ => "unknown_new_field",
            };
            v[key] = "changed".into();
            std::fs::write(path, serde_json::to_vec(&v).unwrap()).unwrap();
        }
        let before = f.old();
        let p = platform(false);
        let calls = p.calls.clone();
        let mut backend = exe_install::ExeInstaller::open_fixture(&f.root, p).unwrap();
        let now = tx.updated_at;
        assert!(backend.execute(&mut tx, &evidence, now).is_err());
        assert_eq!(f.old(), before);
        assert!(calls.lock().unwrap().iter().all(|c| c == "preflight"));
    }
}

#[test]
fn stop_window_latest_journal_survives_full_execute_failure() {
    struct WritesOnStop {
        root: PathBuf,
        first: bool,
        calls: Arc<Mutex<Vec<&'static str>>>,
    }
    impl exe_install::Platform for WritesOnStop {
        fn preflight(&mut self) -> Result<(), String> {
            self.calls.lock().unwrap().push("preflight");
            Ok(())
        }
        fn stop(&mut self) -> Result<(), String> {
            self.calls.lock().unwrap().push("stop");
            if self.first {
                self.first = false;
                std::fs::write(
                    self.root.join("state/commands-1.json"),
                    b"{\"commands\":{\"latest\":\"acknowledged\"}}",
                )
                .unwrap();
                let path = self.root.join("state/agent.json");
                let mut state: serde_json::Value =
                    serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
                state["last_sequence"] = 99.into();
                std::fs::write(path, serde_json::to_vec(&state).unwrap()).unwrap();
            }
            Ok(())
        }
        fn stopped(&mut self) -> Result<bool, String> {
            Ok(true)
        }
        fn start_and_validate(&mut self, _: u64) -> Result<(bool, bool), String> {
            std::fs::write(
                self.root.join("state/commands-1.json"),
                b"new-agent-corrupted-journal",
            )
            .unwrap();
            Err("new agent startup failed".into())
        }
        fn restore_services(&mut self, _: u64) -> Result<(bool, bool), String> {
            self.calls.lock().unwrap().push("restore");
            Ok((true, true))
        }
    }
    let f = Fixture::new();
    f.prepare().unwrap();
    let (mut tx, evidence) = executable_transaction(&f);
    let old = f.old();
    let calls = Arc::new(Mutex::new(vec![]));
    let mut backend = exe_install::ExeInstaller::open_fixture(
        &f.root,
        WritesOnStop {
            root: f.root.clone(),
            first: true,
            calls: calls.clone(),
        },
    )
    .unwrap();
    let now = tx.updated_at;
    let result = backend.execute(&mut tx, &evidence, now).unwrap();
    assert_eq!(result.phase, transaction::Phase::RolledBack);
    assert!(result.error.unwrap().contains("startup failed"));
    assert_eq!(
        std::fs::read(f.root.join("state/commands-1.json")).unwrap(),
        b"{\"commands\":{\"latest\":\"acknowledged\"}}"
    );
    let state: serde_json::Value =
        serde_json::from_slice(&std::fs::read(f.root.join("state/agent.json")).unwrap()).unwrap();
    assert_eq!(state["last_sequence"], 99);
    assert_eq!(
        std::fs::read(f.root.join("normal/RustDesk.exe")).unwrap(),
        old[0]
    );
    assert_eq!(
        calls
            .lock()
            .unwrap()
            .iter()
            .filter(|c| **c == "restore")
            .count(),
        1
    );
}

#[test]
fn stop_window_identity_or_config_change_refuses_without_rewriting_fresh_data() {
    struct Changes {
        root: PathBuf,
        identity: bool,
    }
    impl exe_install::Platform for Changes {
        fn preflight(&mut self) -> Result<(), String> {
            Ok(())
        }
        fn stop(&mut self) -> Result<(), String> {
            if self.identity {
                let path = self.root.join("state/agent.json");
                let mut state: serde_json::Value =
                    serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
                state["agent_installation_id"] = "c".repeat(32).into();
                std::fs::write(path, serde_json::to_vec(&state).unwrap()).unwrap();
            } else {
                std::fs::write(
                    self.root.join("config/RustDesk.toml"),
                    b"id='changed-during-stop'",
                )
                .unwrap();
            }
            Ok(())
        }
        fn stopped(&mut self) -> Result<bool, String> {
            Ok(true)
        }
        fn start_and_validate(&mut self, _: u64) -> Result<(bool, bool), String> {
            panic!("changed identity/config must not deploy")
        }
        fn restore_services(&mut self, _: u64) -> Result<(bool, bool), String> {
            panic!("changed identity/config must not start")
        }
    }
    for identity in [true, false] {
        let f = Fixture::new();
        f.prepare().unwrap();
        let old = f.old();
        let (mut tx, evidence) = executable_transaction(&f);
        let mut backend = exe_install::ExeInstaller::open_fixture(
            &f.root,
            Changes {
                root: f.root.clone(),
                identity,
            },
        )
        .unwrap();
        let now = tx.updated_at;
        let result = backend.execute(&mut tx, &evidence, now).unwrap();
        assert_eq!(result.phase, transaction::Phase::RollbackFailed);
        assert!(result.error.is_some());
        assert_eq!(
            std::fs::read(f.root.join("normal/RustDesk.exe")).unwrap(),
            old[0]
        );
        if identity {
            let state: serde_json::Value =
                serde_json::from_slice(&std::fs::read(f.root.join("state/agent.json")).unwrap())
                    .unwrap();
            assert_eq!(state["agent_installation_id"], "c".repeat(32));
        } else {
            assert_eq!(
                std::fs::read(f.root.join("config/RustDesk.toml")).unwrap(),
                b"id='changed-during-stop'"
            );
        }
    }
}

#[test]
fn native_exe_runtime_child() {
    if let Some(root) = std::env::var_os("EXE_RUNTIME_TEST_ROOT") {
        let public = STANDARD
            .decode(std::env::var("EXE_RUNTIME_PUBLIC").unwrap())
            .unwrap();
        exe_runtime::helper_native_fixture(Path::new(&root), &public).unwrap();
    }
}

#[test]
#[ignore = "requires clean EXE product baseline; never run on MSI"]
fn native_exe_runtime_real_layout() {
    base::lab_guard::require_lab_os("win10");
    let state = Path::new(r"C:\ProgramData\RustDesk\ManagementAgent\state");
    let initial: serde_json::Value =
        serde_json::from_slice(&std::fs::read(state.join("agent.json")).unwrap()).unwrap();
    assert_eq!(initial["package_kind"], "exe");
    assert_eq!(initial["edition"], "standard");
    let package = Path::new(r"C:\ProgramData\RustDesk\ManagementAgent\cache\lab-target0505.exe");
    let bytes = std::fs::read(package).unwrap();
    hbb_common::sodiumoxide::init().unwrap();
    let (public, secret) = sign::gen_keypair();
    let mut manifest = UpdateManifest {
        schema: 2,
        product: "rustdesk-yan".into(),
        edition: "multi".into(),
        channel: "stable".into(),
        ..Default::default()
    };
    let key = "windows-x86_64-exe-standard";
    let mut target = UpdateTarget {
        version: "1.5.3".into(),
        build_seq: 2026100505,
        source_commit: "lab-exe0505".into(),
        primary: "https://download.yan.life/rustdesk/stable/1.5.3/lab-target0505.exe".into(),
        size: bytes.len() as u64,
        sha256: hex::encode(Sha256::digest(bytes)),
        signature_key_id: "fixture-key".into(),
        ..Default::default()
    };
    target.signature = STANDARD.encode(sign::sign_detached(
        &base::update::update_signature_payload(&manifest, key, &target).unwrap(),
        &secret,
    ));
    manifest.targets.insert(key.into(), target);
    let id = uuid::Uuid::new_v4().simple().to_string();
    let install = initial["agent_installation_id"].as_str().unwrap();
    let request = exe_preparation::Request {
        transaction_id: &id,
        installation_id: install,
        edition: "standard",
        current_build: 2026100504,
        package,
        manifest: &manifest,
    };
    let plan = exe_preparation::prepare_native_fixture(&request, public.as_ref()).unwrap();
    let root = Path::new(r"C:\ProgramData\RustDesk\ManagementAgent\transactions").join(&id);
    println!("NATIVE_EXE_TRANSACTION={}", root.display());
    let mut child = exe_runtime::launch_native_fixture(&root, public.as_ref()).unwrap();
    assert!(
        child.wait().unwrap().success(),
        "{}",
        std::fs::read_to_string(root.join("exe-helper.stderr.log")).unwrap()
    );
    let result: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("execution-result.json")).unwrap())
            .unwrap();
    let fail = std::env::var("EXE_NATIVE_FAIL_NEW_AGENT").ok().as_deref() == Some("yes");
    assert_eq!(
        result["phase"],
        if fail { "rolled_back" } else { "committed" }
    );
    if fail {
        assert!(result["error"]
            .as_str()
            .unwrap()
            .contains("actually stopped"));
    }
    assert!(!state.join("maintenance.json").exists());
    assert!(resume_task::read_back(&plan.transaction_id).is_err());
    let current: serde_json::Value =
        serde_json::from_slice(&std::fs::read(state.join("agent.json")).unwrap()).unwrap();
    assert_eq!(current["agent_installation_id"], install);
    assert_eq!(current["package_kind"], "exe");
}

#[test]
fn runtime_real_scm_helper_success_and_new_agent_failure_restore_old_contract() {
    for fail in [false, true] {
        let s =
            ScmFixture::from_package(Path::new(r"C:\RustDeskAgentLab\exe-v219-scm-package.exe"));
        let plan = s.f.prepare().unwrap();
        let old = s.f.old();
        if fail {
            std::fs::write(s.f.root.join("fail-new-agent"), b"fail").unwrap();
        }
        let mut child = exe_runtime::launch_fixture(&s.f.tx(), s.f.public.as_ref()).unwrap();
        assert!(
            child.wait().unwrap().success(),
            "helper failed: {}",
            std::fs::read_to_string(s.f.tx().join("exe-helper.stderr.log")).unwrap()
        );
        let result: serde_json::Value =
            serde_json::from_slice(&std::fs::read(s.f.tx().join("execution-result.json")).unwrap())
                .unwrap();
        assert_eq!(
            result["phase"],
            if fail { "rolled_back" } else { "committed" }
        );
        if fail {
            assert_eq!(s.f.old(), old);
        } else {
            assert_eq!(
                std::fs::read_to_string(s.f.root.join("normal/version.txt")).unwrap(),
                "2026100504"
            );
        }
        assert!(!s.f.root.join("state/maintenance.json").exists());
        assert!(resume_task::read_back(&plan.transaction_id).is_err());
        for c in &s.contracts {
            assert_eq!(
                service_control::inspect(c).unwrap().state,
                windows_service::service::ServiceState::Running
            );
        }
    }
}

#[test]
fn p1_second_service_start_failure_restores_old_files_and_both_services() {
    p1_scm_case("fail-agent-start");
}

#[test]
fn p1_new_agent_corrupt_state_restores_stopped_journal_with_real_scm() {
    p1_scm_case("corrupt-agent-state");
}

#[test]
fn p1_new_agent_corrupt_state_cross_process_resume_uses_bound_snapshot() {
    let s = ScmFixture::from_package(Path::new(r"C:\RustDeskAgentLab\exe-v222-scm-package.exe"));
    let plan = s.f.prepare().unwrap();
    let old = s.f.old();
    std::fs::write(s.f.root.join("corrupt-agent-state"), b"yes").unwrap();
    std::env::set_var("EXE_CORRUPT_INTERRUPT", "yes");
    let launched = exe_runtime::launch_fixture(&s.f.tx(), s.f.public.as_ref());
    std::env::remove_var("EXE_CORRUPT_INTERRUPT");
    assert_eq!(launched.unwrap().wait().unwrap().code(), Some(80));
    assert!(serde_json::from_slice::<serde_json::Value>(
        &std::fs::read(s.f.root.join("state/agent.json")).unwrap()
    )
    .is_err());
    let output = resume_test_child(&s.f);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(s.f.old(), old);
    assert!(!s.f.root.join("state/maintenance.json").exists());
    assert!(resume_task::read_back(&plan.transaction_id).is_err());
    for c in &s.contracts {
        assert_eq!(
            service_control::inspect(c).unwrap().state,
            windows_service::service::ServiceState::Running
        );
    }
}

fn p1_scm_case(marker: &str) {
    let s = ScmFixture::from_package(Path::new(r"C:\RustDeskAgentLab\exe-v222-scm-package.exe"));
    let plan = s.f.prepare().unwrap();
    let old = s.f.old();
    std::fs::write(s.f.root.join(marker), b"inject").unwrap();
    let mut child = exe_runtime::launch_fixture(&s.f.tx(), s.f.public.as_ref()).unwrap();
    let status = child.wait().unwrap();
    let log = std::fs::read_to_string(s.f.tx().join("exe-helper.stderr.log")).unwrap();
    assert!(status.success(), "{marker}: {log}");
    let result: serde_json::Value =
        serde_json::from_slice(&std::fs::read(s.f.tx().join("execution-result.json")).unwrap())
            .unwrap();
    assert_eq!(result["phase"], "rolled_back");
    assert!(result["error"].is_string());
    assert_eq!(s.f.old(), old);
    assert!(!s.f.root.join("state/maintenance.json").exists());
    assert!(resume_task::read_back(&plan.transaction_id).is_err());
    assert!(s.f.root.join(format!("{marker}-observed")).exists());
    for c in &s.contracts {
        assert_eq!(
            service_control::inspect(c).unwrap().state,
            windows_service::service::ServiceState::Running
        );
    }
}

fn resume_test_child(f: &Fixture) -> std::process::Output {
    std::process::Command::new(f.tx().join("runtime/helper.exe"))
        .args([
            "--exact",
            "runtime_helper_owned_maintenance_commits_and_preflight_failure_keeps_journal",
            "--nocapture",
        ])
        .env("EXE_RUNTIME_TEST_ROOT", f.tx())
        .env("EXE_RUNTIME_RESUME_TEST", "yes")
        .output()
        .unwrap()
}

#[test]
fn p1_terminal_cleanup_missing_records_is_idempotent_without_backend() {
    for missing in ["marker", "task", "both"] {
        let f = Fixture::new();
        let plan = f.prepare().unwrap();
        std::env::set_var("EXE_TERMINAL_INTERRUPT", "yes");
        let launched = exe_runtime::launch_fixture(&f.tx(), f.public.as_ref());
        std::env::remove_var("EXE_TERMINAL_INTERRUPT");
        assert_eq!(launched.unwrap().wait().unwrap().code(), Some(78));
        if missing == "marker" || missing == "both" {
            std::fs::remove_file(f.root.join("state/maintenance.json")).unwrap();
        }
        if missing == "task" || missing == "both" {
            resume_task::remove(&plan.transaction_id).unwrap();
        }
        let tx = std::fs::read(f.tx().join("transaction.json")).unwrap();
        let receipt = std::fs::read(f.tx().join("execution-result.json")).unwrap();
        let target = std::fs::read(f.root.join("normal/rustdesk.exe")).unwrap();
        let state = std::fs::read(f.root.join("state/agent.json")).unwrap();
        for _ in 0..2 {
            let output = resume_test_child(&f);
            assert!(
                output.status.success(),
                "{missing}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        assert_eq!(std::fs::read(f.tx().join("transaction.json")).unwrap(), tx);
        assert_eq!(
            std::fs::read(f.tx().join("execution-result.json")).unwrap(),
            receipt
        );
        assert_eq!(
            std::fs::read(f.root.join("normal/rustdesk.exe")).unwrap(),
            target
        );
        assert_eq!(
            std::fs::read(f.root.join("state/agent.json")).unwrap(),
            state
        );
        assert!(!f.root.join("state/maintenance.json").exists());
        assert!(resume_task::read_back(&plan.transaction_id).is_err());
    }
}

#[test]
fn p1_terminal_cleanup_preserves_foreign_objects_and_missing_receipt() {
    for case in ["receipt", "marker", "task"] {
        let f = Fixture::new();
        let plan = f.prepare().unwrap();
        std::env::set_var("EXE_TERMINAL_INTERRUPT", "yes");
        let launched = exe_runtime::launch_fixture(&f.tx(), f.public.as_ref());
        std::env::remove_var("EXE_TERMINAL_INTERRUPT");
        assert_eq!(launched.unwrap().wait().unwrap().code(), Some(78));
        let marker = f.root.join("state/maintenance.json");
        let receipt = f.tx().join("execution-result.json");
        if case == "receipt" {
            std::fs::remove_file(&receipt).unwrap();
        }
        if case == "marker" {
            let mut value: serde_json::Value =
                serde_json::from_slice(&std::fs::read(&marker).unwrap()).unwrap();
            value["nonce"] = "f".repeat(64).into();
            std::fs::write(&marker, serde_json::to_vec(&value).unwrap()).unwrap();
        }
        if case == "task" {
            resume_task::remove(&plan.transaction_id).unwrap();
            let definition =
                resume_task::definition(&f.tx().join("runtime/foreign.exe"), &plan.transaction_id)
                    .unwrap()
                    .replace(
                        "resume-install --transaction-id",
                        "exe-transaction-resume --transaction-id",
                    );
            let dir = f.tx().join("foreign-task");
            workspace::create(&dir).unwrap();
            resume_task::register_definition(&dir, &plan.transaction_id, &definition).unwrap();
        }
        let before = std::fs::read(&marker).unwrap();
        let task = resume_task::read_back(&plan.transaction_id).unwrap();
        let tx = std::fs::read(f.tx().join("transaction.json")).unwrap();
        let output = resume_test_child(&f);
        assert!(!output.status.success());
        assert_eq!(std::fs::read(&marker).unwrap(), before);
        assert_eq!(resume_task::read_back(&plan.transaction_id).unwrap(), task);
        assert_eq!(std::fs::read(f.tx().join("transaction.json")).unwrap(), tx);
        resume_task::remove(&plan.transaction_id).unwrap();
    }
}

#[test]
fn p1_cleanup_process_exit_after_marker_release_retries_without_backend() {
    let f = Fixture::new();
    let plan = f.prepare().unwrap();
    std::env::set_var("EXE_CLEANUP_INTERRUPT", "yes");
    let launched = exe_runtime::launch_fixture(&f.tx(), f.public.as_ref());
    std::env::remove_var("EXE_CLEANUP_INTERRUPT");
    assert_eq!(launched.unwrap().wait().unwrap().code(), Some(79));
    assert!(!f.root.join("state/maintenance.json").exists());
    assert!(resume_task::read_back(&plan.transaction_id).is_ok());
    let bytes = std::fs::read(f.tx().join("execution-result.json")).unwrap();
    let output = resume_test_child(&f);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        std::fs::read(f.tx().join("execution-result.json")).unwrap(),
        bytes
    );
    assert!(resume_task::read_back(&plan.transaction_id).is_err());
}

#[test]
fn runtime_helper_owned_maintenance_commits_and_preflight_failure_keeps_journal() {
    if let Some(root) = std::env::var_os("EXE_RUNTIME_TEST_ROOT") {
        let root = PathBuf::from(root);
        let public = std::fs::read(root.parent().unwrap().join("fixture-public.bin")).unwrap();
        if std::env::var("EXE_RUNTIME_RESUME_TEST").ok().as_deref() == Some("yes") {
            exe_runtime::resume_fixture(&root, &public).unwrap();
        } else {
            exe_runtime::helper_fixture(&root, &public).unwrap();
        }
        return;
    }
    for changed in [false, true] {
        let f = Fixture::new();
        let plan = f.prepare().unwrap();
        let old = f.old();
        if changed {
            std::fs::write(
                f.root.join("state/commands-1.json"),
                b"new-journal-must-survive",
            )
            .unwrap();
        }
        let latest = f.old();
        let launched = exe_runtime::launch_fixture(&f.tx(), f.public.as_ref());
        if changed {
            assert!(launched.err().unwrap().contains("snapshot"));
            assert_eq!(f.old(), latest);
            assert!(!f.root.join("state/maintenance.json").exists());
        } else {
            let mut child = launched.unwrap();
            assert!(child.wait().unwrap().success());
            let result: serde_json::Value = serde_json::from_slice(
                &std::fs::read(f.tx().join("execution-result.json")).unwrap(),
            )
            .unwrap();
            assert_eq!(result["phase"], "committed");
            assert!(!f.root.join("state/maintenance.json").exists());
            assert!(resume_task::read_back(&plan.transaction_id).is_err());
            assert_eq!(
                std::fs::read(f.root.join("config/RustDesk.toml")).unwrap(),
                old[5]
            );
        }
    }
}

#[test]
fn runtime_interrupted_validation_resumes_old_files_and_terminal_cleanup() {
    let f = Fixture::new();
    let old = f.old();
    let plan = f.prepare().unwrap();
    std::env::set_var("EXE_RUNTIME_INTERRUPT_TEST", "yes");
    let launched = exe_runtime::launch_fixture(&f.tx(), f.public.as_ref());
    std::env::remove_var("EXE_RUNTIME_INTERRUPT_TEST");
    let mut child = launched.unwrap();
    assert_eq!(child.wait().unwrap().code(), Some(77));
    let tx =
        transaction::Transaction::decode(&std::fs::read(f.tx().join("transaction.json")).unwrap())
            .unwrap();
    assert_eq!(tx.phase, transaction::Phase::Validating);
    let helper = f.tx().join("runtime/helper.exe");
    let result = std::process::Command::new(helper)
        .args([
            "--exact",
            "runtime_helper_owned_maintenance_commits_and_preflight_failure_keeps_journal",
            "--nocapture",
        ])
        .env("EXE_RUNTIME_TEST_ROOT", f.tx())
        .env("EXE_RUNTIME_RESUME_TEST", "yes")
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(f.old(), old);
    assert!(!f.root.join("state/maintenance.json").exists());
    assert!(resume_task::read_back(&plan.transaction_id).is_err());
    let result: serde_json::Value =
        serde_json::from_slice(&std::fs::read(f.tx().join("execution-result.json")).unwrap())
            .unwrap();
    assert_eq!(result["phase"], "rolled_back");
}

#[test]
fn runtime_binding_rejects_rewritten_target_state_and_config_descriptors() {
    for (field, directory, file) in [
        ("target", "target", "rustdesk-management-agent.exe"),
        ("state", "state-backup", "commands-1.json"),
        ("config", "config-backup", "RustDesk.toml"),
    ] {
        let f = Fixture::new();
        f.prepare().unwrap();
        let old = f.old();
        let replacement = b"substituted-after-prepare";
        std::fs::write(f.tx().join(directory).join("files").join(file), replacement).unwrap();
        let manifest_path = f.tx().join(directory).join("manifest.json");
        let mut manifest: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&manifest_path).unwrap()).unwrap();
        for entry in manifest["files"].as_array_mut().unwrap() {
            if entry["path"] == file {
                entry["size"] = (replacement.len() as u64).into();
                entry["sha256"] = hex::encode(Sha256::digest(replacement)).into();
            }
        }
        std::fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        let assets_path = f.tx().join("exe-assets.json");
        let mut assets: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&assets_path).unwrap()).unwrap();
        assets[field] = serde_json::to_value(
            exe_preparation::describe(&f.tx(), &format!("{directory}/manifest.json")).unwrap(),
        )
        .unwrap();
        std::fs::write(&assets_path, serde_json::to_vec(&assets).unwrap()).unwrap();
        assert!(
            exe_install::ExeInstaller::open_fixture(&f.root, platform(false)).is_err(),
            "rewritten {field} descriptor must not replace plan-bound authority"
        );
        assert_eq!(f.old(), old);
    }
}

fn executable_transaction(f: &Fixture) -> (transaction::Transaction, transaction::Evidence) {
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
    let path = f.tx().join("transaction.json");
    let mut tx = transaction::Transaction::decode(&std::fs::read(&path).unwrap()).unwrap();
    for phase in [
        transaction::Phase::Prepared,
        transaction::Phase::StoppingServices,
    ] {
        tx.advance(phase, &evidence, tx.updated_at).unwrap();
        tx.save(&path).unwrap();
    }
    (tx, evidence)
}

#[test]
fn execute_preflight_changed_journal_retains_latest_bytes_without_rollback() {
    let f = Fixture::new();
    f.prepare().unwrap();
    let (mut tx, evidence) = executable_transaction(&f);
    let p = platform(false);
    let calls = p.calls.clone();
    let mut backend = exe_install::ExeInstaller::open_fixture(&f.root, p).unwrap();
    std::fs::write(
        f.root.join("state/commands-1.json"),
        b"{\"commands\":{\"new\":\"event-saved\"}}",
    )
    .unwrap();
    let mut state: serde_json::Value =
        serde_json::from_slice(&std::fs::read(f.root.join("state/agent.json")).unwrap()).unwrap();
    state["last_sequence"] = 42.into();
    std::fs::write(
        f.root.join("state/agent.json"),
        serde_json::to_vec(&state).unwrap(),
    )
    .unwrap();
    let latest = f.old();
    let transaction_before = std::fs::read(f.tx().join("transaction.json")).unwrap();
    let now = tx.updated_at;
    let result = backend.execute(&mut tx, &evidence, now);
    assert_eq!(
        f.old(),
        latest,
        "pre-stop refusal must preserve the live journal and state"
    );
    assert!(result.unwrap_err().contains("snapshot changed before stop"));
    assert_eq!(
        std::fs::read(f.tx().join("transaction.json")).unwrap(),
        transaction_before
    );
    assert_eq!(tx.phase, transaction::Phase::StoppingServices);
    assert!(!f.root.join("state/maintenance.json").exists());
    assert!(calls.lock().unwrap().iter().all(|c| c == "preflight"));
}

#[test]
fn execute_preflight_refusals_have_no_stop_or_restore_calls() {
    struct Refuse {
        calls: Arc<Mutex<Vec<&'static str>>>,
        error: &'static str,
    }
    impl exe_install::Platform for Refuse {
        fn preflight(&mut self) -> Result<(), String> {
            Err(self.error.into())
        }
        fn stop(&mut self) -> Result<(), String> {
            self.calls.lock().unwrap().push("stop");
            Ok(())
        }
        fn stopped(&mut self) -> Result<bool, String> {
            Ok(true)
        }
        fn start_and_validate(&mut self, _: u64) -> Result<(bool, bool), String> {
            self.calls.lock().unwrap().push("start");
            Ok((true, true))
        }
        fn restore_services(&mut self, _: u64) -> Result<(bool, bool), String> {
            self.calls.lock().unwrap().push("restore");
            Ok((true, true))
        }
    }
    for error in [
        "EXE activity unknown",
        "EXE activity active",
        "EXE SCM contract mismatch",
    ] {
        let f = Fixture::new();
        f.prepare().unwrap();
        let (mut tx, evidence) = executable_transaction(&f);
        let before = f.old();
        let bytes = std::fs::read(f.tx().join("transaction.json")).unwrap();
        let calls = Arc::new(Mutex::new(vec![]));
        let mut backend = exe_install::ExeInstaller::open_fixture(
            &f.root,
            Refuse {
                calls: calls.clone(),
                error,
            },
        )
        .unwrap();
        let now = tx.updated_at;
        assert_eq!(backend.execute(&mut tx, &evidence, now).unwrap_err(), error);
        assert!(calls.lock().unwrap().is_empty());
        assert_eq!(f.old(), before);
        assert_eq!(
            std::fs::read(f.tx().join("transaction.json")).unwrap(),
            bytes
        );
    }
}

#[test]
fn execute_preflight_optin_revocation_and_maintenance_preserve_records() {
    for marker in ["optin", "maintenance"] {
        let f = Fixture::new();
        f.prepare().unwrap();
        let (mut tx, evidence) = executable_transaction(&f);
        let p = platform(false);
        let calls = p.calls.clone();
        let mut backend = exe_install::ExeInstaller::open_fixture(&f.root, p).unwrap();
        if marker == "optin" {
            std::fs::remove_file(f.root.join("state/service-recovery-enabled")).unwrap();
        } else {
            std::fs::write(
                f.root.join("state/maintenance.json"),
                b"foreign-maintenance-preserve",
            )
            .unwrap();
        }
        let old = f.old();
        let before = std::fs::read(f.tx().join("transaction.json")).unwrap();
        let now = tx.updated_at;
        assert!(backend.execute(&mut tx, &evidence, now).is_err());
        assert!(calls.lock().unwrap().is_empty());
        assert_eq!(f.old(), old);
        assert_eq!(
            std::fs::read(f.tx().join("transaction.json")).unwrap(),
            before
        );
        if marker == "maintenance" {
            assert_eq!(
                std::fs::read(f.root.join("state/maintenance.json")).unwrap(),
                b"foreign-maintenance-preserve"
            );
        } else {
            assert!(!f.root.join("state/service-recovery-enabled").exists());
        }
    }
}

#[test]
fn execute_after_partial_stop_failure_still_rolls_back() {
    struct Partial {
        calls: Arc<Mutex<Vec<&'static str>>>,
        attempts: u32,
    }
    impl exe_install::Platform for Partial {
        fn preflight(&mut self) -> Result<(), String> {
            Ok(())
        }
        fn stop(&mut self) -> Result<(), String> {
            self.attempts += 1;
            self.calls.lock().unwrap().push("stop");
            if self.attempts == 1 {
                Err("partial stop failure".into())
            } else {
                Ok(())
            }
        }
        fn stopped(&mut self) -> Result<bool, String> {
            Ok(true)
        }
        fn start_and_validate(&mut self, _: u64) -> Result<(bool, bool), String> {
            panic!("apply must not start")
        }
        fn restore_services(&mut self, _: u64) -> Result<(bool, bool), String> {
            self.calls.lock().unwrap().push("restore");
            Ok((true, true))
        }
    }
    let f = Fixture::new();
    f.prepare().unwrap();
    let old = f.old();
    let (mut tx, evidence) = executable_transaction(&f);
    let calls = Arc::new(Mutex::new(vec![]));
    let mut backend = exe_install::ExeInstaller::open_fixture(
        &f.root,
        Partial {
            calls: calls.clone(),
            attempts: 0,
        },
    )
    .unwrap();
    let now = tx.updated_at;
    let result = backend.execute(&mut tx, &evidence, now).unwrap();
    assert_eq!(result.phase, transaction::Phase::RolledBack);
    assert!(result.error.unwrap().contains("partial stop failure"));
    assert_eq!(f.old(), old);
    assert_eq!(*calls.lock().unwrap(), vec!["stop", "stop", "restore"]);
}
impl Fixture {
    fn new() -> Self {
        Self::from_package(&Self::package())
    }
    fn from_package(package: &Path) -> Self {
        base::lab_guard::require_lab_os("win10");
        let root = Path::new(r"C:\RustDeskAgentLab")
            .join(format!("exe-tx-v218-{}", uuid::Uuid::new_v4().simple()));
        workspace::create(&root).unwrap();
        std::fs::copy(package, root.join("package.exe")).unwrap();
        for dir in ["normal", "agent", "state", "config"] {
            workspace::create(&root.join(dir)).unwrap();
        }
        for (name, data) in [
            ("normal/RustDesk.exe", b"old client".as_slice()),
            ("normal/old-only.txt", b"preserve old extra"),
            ("agent/agent.exe", b"old agent"),
            ("state/commands-1.json", b"{\"commands\":{}}"),
            ("config/RustDesk.toml", b"id='original'\nkey='fixture-only'"),
            (
                "config/RustDesk2.toml",
                b"[options]\nstop-service='N'\nfuture='keep'",
            ),
        ] {
            std::fs::write(root.join(name), data).unwrap();
        }
        std::fs::write(root.join("state/agent.json"),serde_json::to_vec(&serde_json::json!({"schema_version":1,"agent_installation_id":"a".repeat(32),"edition":"standard","package_kind":"exe","management_agent_enabled":true,"disabled":false})).unwrap()).unwrap();
        std::fs::write(root.join("state/service-recovery-enabled"), b"yes").unwrap();
        hbb_common::sodiumoxide::init().unwrap();
        let (public, secret) = sign::gen_keypair();
        std::fs::write(root.join("fixture-public.bin"), public.as_ref()).unwrap();
        let bytes = std::fs::read(package).unwrap();
        let mut manifest = UpdateManifest {
            schema: 2,
            product: "rustdesk-yan".into(),
            edition: "multi".into(),
            channel: "stable".into(),
            ..Default::default()
        };
        let key = "windows-x86_64-exe-standard";
        let mut target = UpdateTarget {
            version: "1.5.3".into(),
            build_seq: 2026100504,
            source_commit: "exe-tx-fixture".into(),
            primary: "https://download.yan.life/rustdesk/stable/1.5.3/exe-tx.exe".into(),
            size: bytes.len() as u64,
            sha256: hex::encode(Sha256::digest(bytes)),
            signature_key_id: "fixture-key".into(),
            ..Default::default()
        };
        target.signature = STANDARD.encode(sign::sign_detached(
            &base::update::update_signature_payload(&manifest, key, &target).unwrap(),
            &secret,
        ));
        manifest.targets.insert(key.into(), target);
        Self {
            root,
            manifest,
            public,
        }
    }
    fn package() -> PathBuf {
        PathBuf::from(r"C:\RustDeskAgentLab\exe-v215\fixture.exe")
    }
    fn prepare(&self) -> Result<install_plan::InstallPlan, String> {
        let package = self.root.join("package.exe");
        let name = self.root.file_name().unwrap().to_str().unwrap();
        let id = name.strip_prefix("exe-tx-v218-").unwrap();
        let request = exe_preparation::Request {
            transaction_id: id,
            installation_id: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            edition: "standard",
            current_build: 2026100502,
            package: &package,
            manifest: &self.manifest,
        };
        exe_preparation::prepare_fixture(&self.root, &request, self.public.as_ref())
    }
    fn tx(&self) -> PathBuf {
        self.root.join("transaction")
    }
    fn old(&self) -> Vec<Vec<u8>> {
        [
            "normal/RustDesk.exe",
            "normal/old-only.txt",
            "agent/agent.exe",
            "state/agent.json",
            "state/commands-1.json",
            "config/RustDesk.toml",
            "config/RustDesk2.toml",
        ]
        .iter()
        .map(|p| std::fs::read(self.root.join(p)).unwrap())
        .collect()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        if std::thread::panicking() {
            eprintln!("PRESERVED_EXE_FIXTURE={}", self.root.display());
        } else {
            std::fs::remove_dir_all(&self.root).unwrap();
        }
    }
}

#[test]
fn preparation_seals_signed_payload_and_complete_old_state_without_mutation() {
    let f = Fixture::new();
    let old = f.old();
    let plan = f.prepare().unwrap();
    assert_eq!(plan.trigger, install_plan::Trigger::LegacyMigration);
    assert_eq!(plan.target_key, "windows-x86_64-exe-standard");
    plan.validate().unwrap();
    plan.verify_assets(&f.tx()).unwrap();
    assert_eq!(f.old(), old);
    for name in [
        "client-backup/manifest.json",
        "state-backup/files/commands-1.json",
        "config-backup/files/RustDesk2.toml",
        "backups/agent.exe",
        "runtime/helper.exe",
    ] {
        assert!(f.tx().join(name).is_file(), "{name}");
    }
    assert!(f.prepare().unwrap_err().contains("exists"));
    assert_eq!(f.old(), old);
}

#[test]
fn preparation_rejects_msi_cross_edition_and_local_stop_before_staging() {
    let f = Fixture::new();
    let old = f.old();
    let state = f.root.join("state/agent.json");
    let original = std::fs::read(&state).unwrap();
    for (field, value) in [("package_kind", "msi"), ("edition", "sos")] {
        let mut s: serde_json::Value = serde_json::from_slice(&original).unwrap();
        s[field] = value.into();
        std::fs::write(&state, serde_json::to_vec(&s).unwrap()).unwrap();
        assert!(f.prepare().unwrap_err().contains("identity"));
        assert!(!f.tx().exists());
    }
    std::fs::write(state, original).unwrap();
    std::fs::write(f.root.join("state/disable-agent"), b"keep").unwrap();
    assert!(f.prepare().unwrap_err().contains("blocked"));
    assert_eq!(f.old(), old);
    assert_eq!(
        std::fs::read(f.root.join("state/disable-agent")).unwrap(),
        b"keep"
    );
}

#[derive(Clone)]
struct Platform {
    calls: Arc<Mutex<Vec<String>>>,
    fail_new: bool,
    stopped: bool,
}
impl exe_install::Platform for Platform {
    fn preflight(&mut self) -> Result<(), String> {
        self.calls.lock().unwrap().push("preflight".into());
        Ok(())
    }
    fn stop(&mut self) -> Result<(), String> {
        self.calls.lock().unwrap().push("stop".into());
        self.stopped = true;
        Ok(())
    }
    fn stopped(&mut self) -> Result<bool, String> {
        Ok(self.stopped)
    }
    fn start_and_validate(&mut self, build: u64) -> Result<(bool, bool), String> {
        self.calls.lock().unwrap().push(format!("new:{build}"));
        if self.fail_new {
            Err("fixture new health failure".into())
        } else {
            self.stopped = false;
            Ok((true, true))
        }
    }
    fn restore_services(&mut self, build: u64) -> Result<(bool, bool), String> {
        self.calls.lock().unwrap().push(format!("old:{build}"));
        self.stopped = false;
        Ok((true, true))
    }
}
fn platform(fail: bool) -> Platform {
    Platform {
        calls: Arc::new(Mutex::new(vec![])),
        fail_new: fail,
        stopped: false,
    }
}

#[test]
fn backend_refuses_apply_without_confirmed_stop_and_preserves_files() {
    let f = Fixture::new();
    f.prepare().unwrap();
    let before = f.old();
    let mut backend = exe_install::ExeInstaller::open_fixture(&f.root, platform(false)).unwrap();
    assert!(backend.apply_package().unwrap_err().contains("stopped"));
    assert_eq!(f.old(), before);
}

#[test]
fn backend_replaces_real_files_and_failed_health_restores_all_old_data() {
    let f = Fixture::new();
    f.prepare().unwrap();
    let before = f.old();
    let p = platform(true);
    let calls = p.calls.clone();
    let mut backend = exe_install::ExeInstaller::open_fixture(&f.root, p).unwrap();
    backend.stop_services().unwrap();
    assert_eq!(
        backend.apply_package().unwrap(),
        install_execution::InstallOutcome::Installed
    );
    assert_eq!(
        std::fs::read(f.root.join("agent/agent.exe")).unwrap(),
        b"fixture-agent-not-executed"
    );
    assert!(!f.root.join("normal/old-only.txt").exists());
    assert!(backend
        .validate_target()
        .unwrap_err()
        .contains("fixture new health failure"));
    std::fs::write(
        f.root.join("state/commands-1.json"),
        b"changed-by-new-agent",
    )
    .unwrap();
    let restored = backend.rollback_original().unwrap();
    assert!(
        restored.target_matches
            && restored.identity_unchanged
            && restored.client_ready
            && restored.agent_ready
    );
    assert_eq!(f.old(), before);
    assert!(calls.lock().unwrap().iter().any(|v| v == "old:2026100502"));
}

#[test]
fn bad_signature_and_changed_package_do_not_stage_or_change_old_data() {
    let mut f = Fixture::new();
    let old = f.old();
    let target = f
        .manifest
        .targets
        .get_mut("windows-x86_64-exe-standard")
        .unwrap();
    let signature = target.signature.clone();
    target.signature = STANDARD.encode([0u8; 64]);
    assert!(f.prepare().is_err());
    assert!(!f.tx().exists());
    assert_eq!(f.old(), old);
    f.manifest
        .targets
        .get_mut("windows-x86_64-exe-standard")
        .unwrap()
        .signature = signature;
    std::fs::write(f.root.join("package.exe"), b"tampered").unwrap();
    assert!(f.prepare().is_err());
    assert!(!f.tx().exists());
    assert_eq!(f.old(), old);
}

#[test]
fn damaged_backup_and_post_prepare_disable_block_before_stop() {
    let f = Fixture::new();
    f.prepare().unwrap();
    let old = f.old();
    let p = platform(false);
    let calls = p.calls.clone();
    std::fs::write(
        f.tx().join("state-backup/files/commands-1.json"),
        b"damaged",
    )
    .unwrap();
    assert!(exe_install::ExeInstaller::open_fixture(&f.root, p)
        .err()
        .unwrap()
        .contains("mismatch"));
    assert!(calls.lock().unwrap().is_empty());
    assert_eq!(f.old(), old);
    let g = Fixture::new();
    g.prepare().unwrap();
    let old = g.old();
    let p = platform(false);
    let calls = p.calls.clone();
    let mut backend = exe_install::ExeInstaller::open_fixture(&g.root, p).unwrap();
    std::fs::write(g.root.join("state/disable-recovery"), b"preserve").unwrap();
    assert!(backend.stop_services().unwrap_err().contains("blocked"));
    assert!(calls.lock().unwrap().is_empty());
    assert_eq!(g.old(), old);
}

#[test]
fn partial_replace_failure_rolls_back_original_directory_and_agent() {
    use std::os::windows::fs::OpenOptionsExt;
    let f = Fixture::new();
    f.prepare().unwrap();
    let old = f.old();
    let mut backend = exe_install::ExeInstaller::open_fixture(&f.root, platform(false)).unwrap();
    backend.stop_services().unwrap();
    let lock = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(f.root.join("agent/agent.exe"))
        .unwrap();
    assert!(backend.apply_package().unwrap_err().contains("publication"));
    assert_eq!(
        std::fs::read(f.root.join("agent/agent.exe")).unwrap(),
        b"old agent"
    );
    drop(lock);
    let restored = backend.rollback_original().unwrap();
    assert!(restored.target_matches && restored.agent_ready);
    assert_eq!(f.old(), old);
}

#[test]
fn successful_file_backend_keeps_identity_and_full_rollback_assets() {
    let f = Fixture::new();
    let old = f.old();
    f.prepare().unwrap();
    let mut backend = exe_install::ExeInstaller::open_fixture(&f.root, platform(false)).unwrap();
    backend.stop_services().unwrap();
    backend.apply_package().unwrap();
    let result = backend.validate_target().unwrap();
    assert!(
        result.target_matches
            && result.identity_unchanged
            && result.client_ready
            && result.agent_ready
    );
    assert_eq!(
        std::fs::read(f.root.join("config/RustDesk.toml")).unwrap(),
        old[5]
    );
    assert_eq!(
        std::fs::read(f.tx().join("client-backup/files/old-only.txt")).unwrap(),
        old[1]
    );
}

#[test]
fn incompatible_new_state_is_restored_without_requiring_new_schema_to_parse() {
    let f = Fixture::new();
    let old = f.old();
    f.prepare().unwrap();
    let mut backend = exe_install::ExeInstaller::open_fixture(&f.root, platform(true)).unwrap();
    backend.stop_services().unwrap();
    backend.apply_package().unwrap();
    std::fs::write(f.root.join("state/agent.json"), b"incompatible-new-state").unwrap();
    assert!(backend
        .validate_target()
        .unwrap_err()
        .contains("state malformed"));
    let restored = backend.rollback_original().unwrap();
    assert!(restored.agent_ready && restored.identity_unchanged);
    assert_eq!(f.old(), old);
}

#[test]
fn real_standard0504_payload_is_signed_prepared_replaced_and_recoverable() {
    let f = Fixture::from_package(Path::new(r"C:\RustDeskAgentLab\exe-v218-standard0504.exe"));
    let old = f.old();
    let plan = f.prepare().unwrap();
    assert_eq!(plan.target_build_seq, 2026100504);
    let mut backend = exe_install::ExeInstaller::open_fixture(&f.root, platform(true)).unwrap();
    backend.stop_services().unwrap();
    backend.apply_package().unwrap();
    assert_eq!(
        hex::encode(Sha256::digest(
            std::fs::read(f.root.join("agent/agent.exe")).unwrap()
        )),
        "5716185c0099d3c9c4964c28705b69d61e5475934388d57de35153809ef8c2b8"
    );
    assert_eq!(
        hex::encode(Sha256::digest(
            std::fs::read(f.root.join("normal/librustdesk.dll")).unwrap()
        )),
        "7d49190d41bd5f7b5f63b9c4a69ac82dace58dd1445f30fe67a442581e5f8984"
    );
    assert!(backend
        .validate_target()
        .unwrap_err()
        .contains("fixture new health failure"));
    let restored = backend.rollback_original().unwrap();
    assert!(
        restored.target_matches
            && restored.identity_unchanged
            && restored.client_ready
            && restored.agent_ready
    );
    assert_eq!(f.old(), old);
}

#[test]
fn preflight_failure_and_unconfirmed_stop_never_replace_files() {
    struct Deny {
        stop_attempts: usize,
        deny_preflight: bool,
    }
    impl exe_install::Platform for Deny {
        fn preflight(&mut self) -> Result<(), String> {
            if self.deny_preflight {
                Err("helper/task not ready".into())
            } else {
                Ok(())
            }
        }
        fn stop(&mut self) -> Result<(), String> {
            self.stop_attempts += 1;
            Ok(())
        }
        fn stopped(&mut self) -> Result<bool, String> {
            Ok(false)
        }
        fn start_and_validate(&mut self, _: u64) -> Result<(bool, bool), String> {
            panic!("no start")
        }
        fn restore_services(&mut self, _: u64) -> Result<(bool, bool), String> {
            panic!("no restore")
        }
    }
    for deny_preflight in [true, false] {
        let f = Fixture::new();
        let old = f.old();
        f.prepare().unwrap();
        let mut backend = exe_install::ExeInstaller::open_fixture(
            &f.root,
            Deny {
                stop_attempts: 0,
                deny_preflight,
            },
        )
        .unwrap();
        assert!(backend
            .stop_services()
            .unwrap_err()
            .contains(if deny_preflight {
                "helper/task"
            } else {
                "stopped"
            }));
        assert!(backend.apply_package().unwrap_err().contains("stopped"));
        assert_eq!(f.old(), old);
    }
}

#[test]
fn agent_first_start_schema_change_cannot_be_reported_ready() {
    struct Mutating {
        state: PathBuf,
        inner: Platform,
    }
    impl exe_install::Platform for Mutating {
        fn preflight(&mut self) -> Result<(), String> {
            Ok(())
        }
        fn stop(&mut self) -> Result<(), String> {
            self.inner.stop()
        }
        fn stopped(&mut self) -> Result<bool, String> {
            self.inner.stopped()
        }
        fn start_and_validate(&mut self, _: u64) -> Result<(bool, bool), String> {
            std::fs::write(&self.state, b"new-incompatible-schema").unwrap();
            Ok((true, true))
        }
        fn restore_services(&mut self, b: u64) -> Result<(bool, bool), String> {
            self.inner.restore_services(b)
        }
    }
    let f = Fixture::new();
    let old = f.old();
    f.prepare().unwrap();
    let mut backend = exe_install::ExeInstaller::open_fixture(
        &f.root,
        Mutating {
            state: f.root.join("state/agent.json"),
            inner: platform(false),
        },
    )
    .unwrap();
    backend.stop_services().unwrap();
    backend.apply_package().unwrap();
    assert!(backend
        .validate_target()
        .unwrap_err()
        .contains("state malformed"));
    assert!(backend.rollback_original().unwrap().agent_ready);
    assert_eq!(f.old(), old);
}

#[test]
fn independent_helper_requires_readiness_release_and_registered_task_before_file_stop() {
    use std::time::{Duration, Instant};
    if let Some(root) = std::env::var_os("EXE_V218_HELPER_ROOT") {
        let root = PathBuf::from(root);
        exe_preparation::fixture_layout(root.parent().unwrap()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(30);
        while !root.join("handoff.json").exists() {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(20));
        }
        let binding =
            handoff::Handoff::decode(&std::fs::read(root.join("handoff.json")).unwrap()).unwrap();
        let current = handoff::ObservedProcess::open(std::process::id()).unwrap();
        assert_eq!(current.owner(), &binding.helper);
        let parent = handoff::ObservedProcess::open(binding.parent.pid).unwrap();
        assert!(parent.is_alive().unwrap());
        assert_eq!(parent.owner(), &binding.parent);
        let now = || {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs()
        };
        state_store::publish(
            &root.join("helper-ready.json"),
            &serde_json::to_vec(&binding.ready_receipt(now()).unwrap()).unwrap(),
        )
        .unwrap();
        while !root.join("helper-release.json").exists() {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(20));
        }
        struct HelperPlatform {
            root: PathBuf,
            inner: Platform,
        }
        impl exe_install::Platform for HelperPlatform {
            fn preflight(&mut self) -> Result<(), String> {
                exe_install::verify_helper_before_stop(&self.root)
            }
            fn stop(&mut self) -> Result<(), String> {
                self.inner.stop()
            }
            fn stopped(&mut self) -> Result<bool, String> {
                self.inner.stopped()
            }
            fn start_and_validate(&mut self, b: u64) -> Result<(bool, bool), String> {
                self.inner.start_and_validate(b)
            }
            fn restore_services(&mut self, b: u64) -> Result<(bool, bool), String> {
                self.inner.restore_services(b)
            }
        }
        let fixture = root.parent().unwrap();
        let mut backend = exe_install::ExeInstaller::open_fixture(
            fixture,
            HelperPlatform {
                root: root.clone(),
                inner: platform(false),
            },
        )
        .unwrap();
        let mut tx = transaction::Transaction::decode(
            &std::fs::read(root.join("transaction.json")).unwrap(),
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
        let result = backend.execute(&mut tx, &evidence, now()).unwrap();
        assert_eq!(result.phase, transaction::Phase::Committed);
        assert!(result.error.is_none());
        state_store::publish(
            &root.join("fixture-completed.json"),
            b"{\"phase\":\"committed\"}",
        )
        .unwrap();
        return;
    }
    let f = Fixture::new();
    let old = f.old();
    let plan = f.prepare().unwrap();
    assert!(exe_install::verify_helper_before_stop(&f.tx())
        .unwrap_err()
        .contains("not authorized"));
    assert_eq!(f.old(), old);
    let helper = f.tx().join("runtime/helper.exe");
    let mut child = std::process::Command::new(&helper)
        .args([
            "--exact",
            "independent_helper_requires_readiness_release_and_registered_task_before_file_stop",
            "--nocapture",
        ])
        .env("EXE_V218_HELPER_ROOT", f.tx())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let now = || {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
    };
    let parent = handoff::ObservedProcess::open(std::process::id()).unwrap();
    let process = handoff::ObservedProcess::open(child.id()).unwrap();
    let digest = hex::encode(Sha256::digest(
        std::fs::read(f.tx().join("prepared.json")).unwrap(),
    ));
    let binding = handoff::Handoff::new(
        plan.transaction_id.clone(),
        digest,
        "e".repeat(64),
        parent.owner().clone(),
        process.owner().clone(),
        now(),
    )
    .unwrap();
    state_store::publish(
        &f.tx().join("handoff.json"),
        &serde_json::to_vec(&binding).unwrap(),
    )
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    while !f.tx().join("helper-ready.json").exists() {
        assert!(Instant::now() < deadline);
        assert!(child.try_wait().unwrap().is_none());
        std::thread::sleep(Duration::from_millis(20));
    }
    let ready: handoff::ReadyReceipt =
        serde_json::from_slice(&std::fs::read(f.tx().join("helper-ready.json")).unwrap()).unwrap();
    binding.verify_ready(&ready, now()).unwrap();
    assert_eq!(f.old(), old);
    assert!(!f.tx().join("fixture-completed.json").exists());
    let definition = resume_task::definition(&helper, &plan.transaction_id).unwrap();
    resume_task::register_definition(&f.tx(), &plan.transaction_id, &definition).unwrap();
    resume_task::verify(
        &resume_task::read_back(&plan.transaction_id).unwrap(),
        &helper,
        &plan.transaction_id,
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
    let mut tx =
        transaction::Transaction::decode(&std::fs::read(f.tx().join("transaction.json")).unwrap())
            .unwrap();
    for phase in [
        transaction::Phase::Prepared,
        transaction::Phase::StoppingServices,
    ] {
        tx.advance(phase, &evidence, now()).unwrap();
        tx.save(&f.tx().join("transaction.json")).unwrap();
    }
    state_store::publish(
        &f.tx().join("helper-release.json"),
        &serde_json::to_vec(&binding.release_receipt(&ready, now()).unwrap()).unwrap(),
    )
    .unwrap();
    let result = child.wait().unwrap();
    resume_task::remove(&plan.transaction_id).unwrap();
    assert!(result.success(), "helper failed");
    assert!(f.tx().join("fixture-completed.json").is_file());
    assert_eq!(
        transaction::Transaction::decode(&std::fs::read(f.tx().join("transaction.json")).unwrap())
            .unwrap()
            .phase,
        transaction::Phase::Committed
    );
    assert_eq!(
        std::fs::read(f.root.join("config/RustDesk.toml")).unwrap(),
        old[5]
    );
}

windows_service::define_windows_service!(exe_scm_entry, exe_scm_main);
struct FixtureControl {
    root: PathBuf,
    role: usize,
    send: std::sync::mpsc::Sender<()>,
}
unsafe extern "system" fn fixture_control_raw(
    control: u32,
    _: u32,
    _: *mut std::ffi::c_void,
    context: *mut std::ffi::c_void,
) -> u32 {
    let context = &*(context as *const FixtureControl);
    std::fs::write(
        context.root.join(format!("control-{}", context.role)),
        format!(
            "control={control} refuse={} root={}",
            context.root.join("refuse-stop").exists(),
            context.root.display()
        ),
    )
    .unwrap();
    match control {
        1 => {
            if context.send.send(()).is_ok() {
                0
            } else {
                1064
            }
        }
        4 => 0,
        _ => 120,
    }
}
#[repr(C)]
struct FixtureStatus {
    kind: u32,
    state: u32,
    accept: u32,
    error: u32,
    specific: u32,
    checkpoint: u32,
    wait: u32,
}
#[link(name = "advapi32")]
extern "system" {
    fn RegisterServiceCtrlHandlerExW(
        name: *const u16,
        handler: unsafe extern "system" fn(
            u32,
            u32,
            *mut std::ffi::c_void,
            *mut std::ffi::c_void,
        ) -> u32,
        context: *mut std::ffi::c_void,
    ) -> *mut std::ffi::c_void;
    fn SetServiceStatus(handle: *mut std::ffi::c_void, status: *const FixtureStatus) -> i32;
}
fn exe_scm_main(_: Vec<std::ffi::OsString>) {
    let executable = std::env::current_exe().unwrap();
    let root = executable.parent().unwrap().parent().unwrap().to_path_buf();
    let role = if executable.parent().unwrap().file_name().unwrap() == "normal" {
        0
    } else {
        1
    };
    let config: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("scm.json")).unwrap()).unwrap();
    let name = config["names"][role].as_str().unwrap().to_owned();
    let pipe = config["pipes"][role].as_str().unwrap().to_owned();
    std::fs::write(
        root.join(format!("started-{role}-{}", std::process::id())),
        executable.to_string_lossy().as_bytes(),
    )
    .unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    let mut context = Box::new(FixtureControl {
        root: root.clone(),
        role,
        send: tx,
    });
    let wide = name.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
    let status = unsafe {
        RegisterServiceCtrlHandlerExW(
            wide.as_ptr(),
            fixture_control_raw,
            (&mut *context as *mut FixtureControl).cast(),
        )
    };
    assert!(!status.is_null());
    let (send, receive) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        use hbb_common::tokio::{
            self,
            io::{AsyncReadExt, AsyncWriteExt},
        };
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async move {
            let mut listener = health_windows::bind(&pipe).unwrap();
            send.send(()).unwrap();
            let owner = handoff::ObservedProcess::open(std::process::id())
                .unwrap()
                .owner()
                .clone();
            while let Some(connection) = listener.next().await {
                let mut stream = connection.unwrap();
                let size = stream.read_u32_le().await.unwrap();
                assert!(size < 4096);
                let mut bytes = vec![0; size as usize];
                stream.read_exact(&mut bytes).await.unwrap();
                let request = health::Request::decode(&bytes).unwrap();
                let mut response = health::Response::new(
                    &request,
                    owner.pid,
                    owner.created_at,
                    "1.5.3".into(),
                    std::fs::read_to_string(root.join("normal/version.txt"))
                        .ok()
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(2026100502),
                    true,
                    false,
                )
                .unwrap();
                if role == 1
                    && root.join("corrupt-agent-state").exists()
                    && root.join("normal/version.txt").exists()
                {
                    std::fs::write(root.join("state/agent.json"), b"{new-incompatible-state")
                        .unwrap();
                    std::fs::write(
                        root.join("state/commands-1.json"),
                        b"new-agent-overwrote-journal",
                    )
                    .unwrap();
                    std::fs::write(
                        root.join("corrupt-agent-state-observed"),
                        b"actual new SCM agent wrote malformed JSON",
                    )
                    .unwrap();
                }
                if role == 1
                    && root.join("fail-new-agent").exists()
                    && root.join("normal/version.txt").exists()
                {
                    response.local_responsive = false;
                }
                response.no_active_sessions =
                    match std::fs::read_to_string(root.join("health-mode"))
                        .unwrap()
                        .as_str()
                    {
                        "idle" => Some(true),
                        "active" => Some(false),
                        _ => None,
                    };
                let bytes = serde_json::to_vec(&response).unwrap();
                stream.write_u32_le(bytes.len() as u32).await.unwrap();
                stream.write_all(&bytes).await.unwrap();
                stream.flush().await.unwrap();
            }
        });
    });
    receive
        .recv_timeout(std::time::Duration::from_secs(10))
        .unwrap();
    let mut value = FixtureStatus {
        kind: 0x10,
        state: 4,
        accept: 1,
        error: 0,
        specific: 0,
        checkpoint: 0,
        wait: 0,
    };
    assert_ne!(unsafe { SetServiceStatus(status, &value) }, 0);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(180);
    loop {
        match rx.recv_timeout(std::time::Duration::from_millis(20)) {
            Ok(()) => break,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                assert!(std::time::Instant::now() < deadline);
                let accept = if context.root.join("refuse-stop").exists() {
                    0
                } else {
                    1
                };
                if value.accept != accept {
                    value.accept = accept;
                    assert_ne!(unsafe { SetServiceStatus(status, &value) }, 0);
                }
            }
            Err(e) => panic!("service control channel: {e}"),
        }
    }
    value.state = 1;
    value.accept = 0;
    assert_ne!(unsafe { SetServiceStatus(status, &value) }, 0);
}

#[test]
fn scm_fixture_service_entry() {
    let executable = std::env::current_exe().unwrap();
    if !matches!(
        executable.file_name().unwrap().to_str(),
        Some("RustDesk.exe" | "rustdesk.exe" | "agent.exe")
    ) {
        return;
    }
    let root = executable.parent().unwrap().parent().unwrap();
    exe_preparation::fixture_layout(root).unwrap();
    if executable.file_name().unwrap() == "agent.exe"
        && root.join("normal/version.txt").exists()
        && root.join("fail-agent-start").exists()
    {
        std::fs::write(
            root.join("fail-agent-start-observed"),
            b"exited before service dispatcher / Running",
        )
        .unwrap();
        std::process::exit(91);
    }
    let config: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("scm.json")).unwrap()).unwrap();
    let role = if executable.parent().unwrap().file_name().unwrap() == "normal" {
        0
    } else {
        1
    };
    windows_service::service_dispatcher::start(
        config["names"][role].as_str().unwrap(),
        exe_scm_entry,
    )
    .unwrap();
}

struct ScmFixture {
    f: Fixture,
    contracts: [service_control::Contract; 2],
    pipes: [String; 2],
    handles: Vec<windows_service::service::Service>,
}
impl ScmFixture {
    fn new() -> Self {
        Self::from_package(&Fixture::package())
    }
    fn from_package(package: &Path) -> Self {
        use windows_service::{service::*, service_manager::*};
        let f = Fixture::from_package(package);
        let id = uuid::Uuid::new_v4().simple().to_string();
        let names = [
            format!("RustDeskAgentTestService-ExeMain-{id}"),
            format!("RustDeskAgentTestService-ExeAgent-{id}"),
        ];
        let pipes = [
            format!(r"\\.\pipe\RustDeskExeTest-{id}-main"),
            format!(r"\\.\pipe\RustDeskExeTest-{id}-agent"),
        ];
        std::fs::write(
            f.root.join("scm.json"),
            serde_json::to_vec(&serde_json::json!({"names":names,"pipes":pipes})).unwrap(),
        )
        .unwrap();
        std::fs::write(f.root.join("health-mode"), b"idle").unwrap();
        let contracts = std::array::from_fn(|i| service_control::Contract {
            name: names[i].clone(),
            executable: f.root.join(if i == 0 {
                "normal/RustDesk.exe"
            } else {
                "agent/agent.exe"
            }),
            arguments: vec![
                "--exact".into(),
                "scm_fixture_service_entry".into(),
                "--nocapture".into(),
            ],
            start_type: ServiceStartType::OnDemand,
        });
        let manager = ServiceManager::local_computer(
            None::<&str>,
            ServiceManagerAccess::CONNECT | ServiceManagerAccess::CREATE_SERVICE,
        )
        .unwrap();
        let mut handles = vec![];
        for c in &contracts {
            std::fs::copy(std::env::current_exe().unwrap(), &c.executable).unwrap();
            handles.push(
                manager
                    .create_service(
                        &ServiceInfo {
                            name: c.name.clone().into(),
                            display_name: c.name.clone().into(),
                            service_type: ServiceType::OWN_PROCESS,
                            start_type: c.start_type,
                            error_control: ServiceErrorControl::Normal,
                            executable_path: c.executable.clone(),
                            launch_arguments: c.arguments.iter().map(Into::into).collect(),
                            dependencies: vec![],
                            account_name: None,
                            account_password: None,
                        },
                        ServiceAccess::all(),
                    )
                    .unwrap(),
            );
            service_control::start(c, std::time::Duration::from_secs(10)).unwrap();
        }
        Self {
            f,
            contracts,
            pipes,
            handles,
        }
    }
    fn owners(&self) -> Vec<handoff::Owner> {
        self.contracts
            .iter()
            .map(|c| {
                handoff::ObservedProcess::open(service_control::inspect(c).unwrap().pid.unwrap())
                    .unwrap()
                    .owner()
                    .clone()
            })
            .collect()
    }
    fn adapter(&self, plan: &install_plan::InstallPlan) -> exe_service::FixedPlatformAdapter {
        exe_service::FixedPlatformAdapter::fixture(
            &self.f.root,
            plan,
            self.contracts.clone(),
            self.pipes.clone(),
            std::time::Duration::from_secs(2),
        )
        .unwrap()
    }
}
impl Drop for ScmFixture {
    fn drop(&mut self) {
        if self.f.root.join("refuse-stop").exists() {
            std::fs::remove_file(self.f.root.join("refuse-stop")).unwrap();
        }
        for (c, h) in self.contracts.iter().zip(&self.handles) {
            if let Err(e) = service_control::stop(c, std::time::Duration::from_secs(10)) {
                eprintln!(
                    "FIXTURE_CLEANUP_FAILED name={} root={} error={e}",
                    c.name,
                    self.f.root.display()
                );
            }
            if let Err(e) = h.delete() {
                eprintln!("FIXTURE_DELETE_FAILED {} {e:?}", c.name);
            }
        }
        self.handles.clear();
    }
}

#[test]
fn scm_unknown_active_and_changed_process_identity_refuse_stop_and_rollback() {
    use exe_install::Platform as _;
    let s = ScmFixture::new();
    let plan = s.f.prepare().unwrap();
    let old = s.f.old();
    let owners = s.owners();
    let (mut tx, evidence) = executable_transaction(&s.f);
    let transaction_before = std::fs::read(s.f.tx().join("transaction.json")).unwrap();
    for mode in ["unknown", "active"] {
        std::fs::write(s.f.root.join("health-mode"), mode).unwrap();
        let adapter = s.adapter(&plan);
        let mut backend = exe_install::ExeInstaller::open_fixture(&s.f.root, adapter).unwrap();
        assert!(backend.stop_services().unwrap_err().contains("activity"));
        assert!(backend.rollback_original().is_err());
        let now = tx.updated_at;
        assert!(backend
            .execute(&mut tx, &evidence, now)
            .unwrap_err()
            .contains("activity"));
        assert_eq!(
            std::fs::read(s.f.tx().join("transaction.json")).unwrap(),
            transaction_before
        );
        assert_eq!(s.owners(), owners);
        assert_eq!(s.f.old(), old);
    }
    std::fs::write(s.f.root.join("health-mode"), b"idle").unwrap();
    let mut adapter = s.adapter(&plan);
    adapter.preflight().unwrap();
    let mut wrong = owners[0].clone();
    wrong.created_at += 1;
    adapter.replace_observed_owner(0, wrong);
    assert!(adapter.stop().unwrap_err().contains("identity"));
    assert_eq!(s.owners(), owners);
    assert_eq!(s.f.old(), old);
}

#[test]
fn execute_copy_failure_runs_real_restore_instead_of_noop_rollback() {
    use std::os::windows::fs::OpenOptionsExt;
    struct Locked {
        agent: PathBuf,
        lock: Option<std::fs::File>,
        stops: u32,
        calls: Arc<Mutex<Vec<&'static str>>>,
    }
    impl exe_install::Platform for Locked {
        fn preflight(&mut self) -> Result<(), String> {
            Ok(())
        }
        fn stop(&mut self) -> Result<(), String> {
            self.stops += 1;
            self.calls.lock().unwrap().push("stop");
            if self.stops == 1 {
                self.lock = Some(
                    std::fs::OpenOptions::new()
                        .read(true)
                        .share_mode(1)
                        .open(&self.agent)
                        .unwrap(),
                );
            } else {
                self.lock.take();
            }
            Ok(())
        }
        fn stopped(&mut self) -> Result<bool, String> {
            Ok(true)
        }
        fn start_and_validate(&mut self, _: u64) -> Result<(bool, bool), String> {
            panic!("copy failed")
        }
        fn restore_services(&mut self, _: u64) -> Result<(bool, bool), String> {
            self.calls.lock().unwrap().push("restore");
            Ok((true, true))
        }
    }
    let f = Fixture::new();
    let old = f.old();
    f.prepare().unwrap();
    let (mut tx, evidence) = executable_transaction(&f);
    let calls = Arc::new(Mutex::new(vec![]));
    let mut backend = exe_install::ExeInstaller::open_fixture(
        &f.root,
        Locked {
            agent: f.root.join("agent/agent.exe"),
            lock: None,
            stops: 0,
            calls: calls.clone(),
        },
    )
    .unwrap();
    let now = tx.updated_at;
    let result = backend.execute(&mut tx, &evidence, now).unwrap();
    assert_eq!(result.phase, transaction::Phase::RolledBack);
    assert!(result.error.unwrap().contains("publication"));
    assert_eq!(f.old(), old);
    assert_eq!(*calls.lock().unwrap(), vec!["stop", "stop", "restore"]);
}

#[test]
fn execute_preflight_changed_client_preserves_new_content_and_transaction() {
    let f = Fixture::new();
    f.prepare().unwrap();
    let (mut tx, evidence) = executable_transaction(&f);
    let p = platform(false);
    let calls = p.calls.clone();
    let mut backend = exe_install::ExeInstaller::open_fixture(&f.root, p).unwrap();
    std::fs::write(
        f.root.join("normal/old-only.txt"),
        b"legitimate-new-content",
    )
    .unwrap();
    let old = f.old();
    let before = std::fs::read(f.tx().join("transaction.json")).unwrap();
    let now = tx.updated_at;
    assert!(backend
        .execute(&mut tx, &evidence, now)
        .unwrap_err()
        .contains("snapshot changed before stop"));
    assert_eq!(f.old(), old);
    assert_eq!(
        std::fs::read(f.tx().join("transaction.json")).unwrap(),
        before
    );
    assert!(calls.lock().unwrap().iter().all(|c| c == "preflight"));
}

#[test]
fn scm_stop_failure_never_copies_and_copy_failure_restores_old_services() {
    let s = ScmFixture::from_package(Path::new(r"C:\RustDeskAgentLab\exe-v218-standard0504.exe"));
    let plan = s.f.prepare().unwrap();
    let old = s.f.old();
    let owners = s.owners();
    std::fs::write(s.f.root.join("refuse-stop"), b"refuse").unwrap();
    for h in &s.handles {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while !h.query_status().unwrap().controls_accepted.is_empty() {
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }
    {
        let mut backend =
            exe_install::ExeInstaller::open_fixture(&s.f.root, s.adapter(&plan)).unwrap();
        let result = backend.stop_services();
        assert!(
            result.is_err(),
            "stop unexpectedly succeeded; refused-main={} refused-agent={}",
            s.f.root.join("refused-0").exists(),
            s.f.root.join("refused-1").exists()
        );
        assert!(backend.apply_package().unwrap_err().contains("stopped"));
        assert_eq!(s.f.old(), old);
        assert_eq!(s.owners(), owners);
    }
    std::fs::remove_file(s.f.root.join("refuse-stop")).unwrap();
    for h in &s.handles {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while h.query_status().unwrap().controls_accepted.is_empty() {
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }
    let mut backend = exe_install::ExeInstaller::open_fixture(&s.f.root, s.adapter(&plan)).unwrap();
    let stopped = backend.stop_services();
    assert!(
        stopped.is_ok(),
        "{stopped:?}; control={:?}/{:?}",
        std::fs::read_to_string(s.f.root.join("control-0")),
        std::fs::read_to_string(s.f.root.join("control-1"))
    );
    for c in &s.contracts {
        assert_eq!(
            service_control::inspect(c).unwrap().state,
            windows_service::service::ServiceState::Stopped
        );
    }
    use std::os::windows::fs::OpenOptionsExt;
    let lock = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(s.f.root.join("agent/agent.exe"))
        .unwrap();
    assert!(backend.apply_package().unwrap_err().contains("publication"));
    drop(lock);
    let restored = backend.rollback_original().unwrap();
    assert!(
        restored.target_matches
            && restored.identity_unchanged
            && restored.client_ready
            && restored.agent_ready
    );
    assert_eq!(s.f.old(), old);
    for c in &s.contracts {
        assert_eq!(
            service_control::inspect(c).unwrap().state,
            windows_service::service::ServiceState::Running
        );
    }
}

#[test]
fn scm_local_gates_cross_kind_and_foreign_contract_preserve_running_services() {
    use exe_install::Platform as _;
    let s = ScmFixture::new();
    let plan = s.f.prepare().unwrap();
    let old = s.f.old();
    let owners = s.owners();
    for marker in ["disable-agent", "disable-recovery", "maintenance.json"] {
        let mut adapter = s.adapter(&plan);
        std::fs::write(s.f.root.join("state").join(marker), b"keep").unwrap();
        assert!(adapter.preflight().unwrap_err().contains("blocked"));
        assert!(adapter.stop().is_err());
        assert_eq!(s.owners(), owners);
        assert_eq!(s.f.old(), old);
        std::fs::remove_file(s.f.root.join("state").join(marker)).unwrap();
    }
    let state = s.f.root.join("state/agent.json");
    let bytes = std::fs::read(&state).unwrap();
    for (field, value) in [("package_kind", "msi"), ("edition", "sos")] {
        let mut adapter = s.adapter(&plan);
        let mut data: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        data[field] = value.into();
        std::fs::write(&state, serde_json::to_vec(&data).unwrap()).unwrap();
        assert!(adapter.preflight().unwrap_err().contains("identity"));
        assert!(adapter.stop().is_err());
        assert_eq!(s.owners(), owners);
        std::fs::write(&state, &bytes).unwrap();
    }
    let mut contracts = s.contracts.clone();
    contracts[0].name = "RustDesk".into();
    assert!(exe_service::FixedPlatformAdapter::fixture(
        &s.f.root,
        &plan,
        contracts,
        s.pipes.clone(),
        std::time::Duration::from_secs(2)
    )
    .err()
    .unwrap()
    .contains("fixture"));
    let mut contracts = s.contracts.clone();
    contracts[0].arguments.push("--foreign".into());
    assert!(exe_service::FixedPlatformAdapter::fixture(
        &s.f.root,
        &plan,
        contracts,
        s.pipes.clone(),
        std::time::Duration::from_secs(2)
    )
    .err()
    .unwrap()
    .contains("contract"));
    assert_eq!(s.owners(), owners);
    let marker = s.f.root.join("state/service-recovery-enabled");
    let optin = std::fs::read(&marker).unwrap();
    std::fs::remove_file(&marker).unwrap();
    assert!(exe_service::FixedPlatformAdapter::fixture(
        &s.f.root,
        &plan,
        s.contracts.clone(),
        s.pipes.clone(),
        std::time::Duration::from_secs(2)
    )
    .is_err());
    std::fs::write(&marker, optin).unwrap();
    assert_eq!(s.owners(), owners);
    let held = install_guard::InstallGuard::try_acquire().unwrap().unwrap();
    let root = s.f.root.clone();
    let plan_copy = plan.clone();
    let contracts = s.contracts.clone();
    let pipes = s.pipes.clone();
    let error = std::thread::spawn(move || {
        exe_service::FixedPlatformAdapter::fixture(
            &root,
            &plan_copy,
            contracts,
            pipes,
            std::time::Duration::from_secs(2),
        )
        .err()
        .unwrap()
    })
    .join()
    .unwrap();
    assert!(error.contains("busy"));
    drop(held);
    assert_eq!(s.owners(), owners);
    let mut adapter = s.adapter(&plan);
    std::fs::write(
        s.f.root.join("config/RustDesk2.toml"),
        b"[options]\nstop-service='Y'",
    )
    .unwrap();
    assert!(adapter.preflight().unwrap_err().contains("stop intent"));
    assert!(adapter.stop().is_err());
    assert_eq!(s.owners(), owners);
    std::fs::write(s.f.root.join("config/RustDesk2.toml"), &old[6]).unwrap();
    assert_eq!(s.f.old(), old);
}

#[test]
fn scm_service_restart_between_preflight_and_stop_is_not_followed() {
    use exe_install::Platform as _;
    let s = ScmFixture::new();
    let plan = s.f.prepare().unwrap();
    let old = s.f.old();
    let mut adapter = s.adapter(&plan);
    adapter.preflight().unwrap();
    let owners = s.owners();
    service_control::stop(&s.contracts[0], std::time::Duration::from_secs(10)).unwrap();
    service_control::start(&s.contracts[0], std::time::Duration::from_secs(10)).unwrap();
    let restarted = s.owners();
    assert_ne!(owners[0], restarted[0]);
    assert!(adapter.stop().unwrap_err().contains("identity"));
    assert_eq!(s.owners(), restarted);
    assert_eq!(s.f.old(), old);
}
