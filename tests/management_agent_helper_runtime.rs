#![cfg(windows)]
#[path = "../src/management_agent/bootstrap_update.rs"] mod bootstrap_update;
#[path = "../src/management_agent/handoff.rs"]
mod handoff;
#[path = "../src/management_agent/helper_runtime.rs"]
mod helper_runtime;
#[path = "../src/platform/windows_agent_guard.rs"]
mod install_guard;
#[path = "../src/management_agent/install_plan.rs"]
mod install_plan;
#[path = "../src/management_agent/package_gate.rs"]
mod package_gate;
#[path = "../src/management_agent/resume_task.rs"]
mod resume_task;
#[path = "../src/management_agent/state_store.rs"]
mod state_store;
#[path = "../src/management_agent/transaction.rs"]
mod transaction;
#[path = "../src/management_agent/workspace.rs"]
mod workspace;
#[path = "../src/management_agent/maintenance.rs"]
mod maintenance;
#[path = "../src/management_agent/install_execution.rs"]
mod install_execution;
#[path = "../src/management_agent/installer_process.rs"]
mod installer_process;
#[path = "../src/management_agent/parent_handoff.rs"]
mod parent_handoff;
#[path = "../src/management_agent/native_install.rs"] mod native_install;
#[path = "../src/management_agent/msi_metadata.rs"] mod msi_metadata;
#[path = "../src/management_agent/directory_backup.rs"] mod directory_backup;
#[path = "../src/management_agent/file_restore.rs"] mod file_restore;
#[path = "../src/management_agent/service_control.rs"] mod service_control;
#[path = "../src/management_agent/health.rs"] mod health;
#[path = "../src/management_agent/health_windows.rs"] mod health_windows;

struct ExecutionFixture { calls: Vec<&'static str>, pending: bool, fail_validation: bool }

#[test]
fn production_release_wait_survives_slow_parent_verification_without_creating_authorization() {
    let root = std::env::temp_dir().join(format!("rustdesk-helper-release-delay-{}", std::process::id()));
    workspace::create(&root).unwrap();
    let writer_root = root.clone();
    let writer = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(31));
        assert!(!writer_root.join("helper-release.json").exists());
        state_store::publish(&writer_root.join("helper-release.json"), b"parent-authored-receipt").unwrap();
    });
    let started = std::time::Instant::now();
    let result = helper_runtime::wait_for_release(&root);
    writer.join().unwrap();
    let bytes = std::fs::read(root.join("helper-release.json")).unwrap();
    std::fs::remove_dir_all(root).unwrap();
    assert!(result.is_ok(), "parent verification may exceed 30 seconds: {result:?}");
    assert!(started.elapsed() < std::time::Duration::from_secs(40));
    assert_eq!(bytes, b"parent-authored-receipt");
}
impl install_execution::Backend for ExecutionFixture {
    fn stop_services(&mut self) -> Result<(), String> { self.calls.push("stop"); Ok(()) }
    fn apply_package(&mut self) -> Result<install_execution::InstallOutcome, String> {
        self.calls.push("apply");
        Ok(if self.pending { install_execution::InstallOutcome::Pending { pid: 0, created_at: 0 } }
            else { install_execution::InstallOutcome::Installed })
    }
    fn validate_target(&mut self) -> Result<install_execution::Validation, String> {
        self.calls.push("validate");
        if self.fail_validation { return Err("fixture target health failure".into()); }
        Ok(install_execution::Validation { target_matches:true, identity_unchanged:true, client_ready:true, agent_ready:true })
    }
    fn rollback_original(&mut self) -> Result<install_execution::Validation, String> {
        assert!(self.fail_validation, "successful fixture must not roll back");
        self.calls.push("rollback");
        Ok(install_execution::Validation { target_matches:true, identity_unchanged:true, client_ready:true, agent_ready:true })
    }
}

#[test]
fn helper_rejects_unprotected_or_incomplete_workspace_before_readiness() {
    let root = std::env::temp_dir().join(format!("rustdesk-helper-runtime-{}", std::process::id()));
    std::fs::create_dir(&root).unwrap();
    let before = std::fs::read_dir(&root).unwrap().count();
    assert!(helper_runtime::prepare_ready(&root)
        .unwrap_err()
        .contains("workspace"));
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), before);
    std::fs::remove_dir(&root).unwrap();
    workspace::create(&root).unwrap();
    assert!(helper_runtime::prepare_ready(&root).is_err());
    assert!(!root.join("helper-ready.json").exists());
    std::fs::remove_dir(&root).unwrap();
}

#[test]
fn helper_process_prepares_only_after_exact_assets_identity_and_parent_validation() {
    if let Some(root) = std::env::var_os("RUSTDESK_HELPER_RUNTIME_FIXTURE") {
        let root = std::path::PathBuf::from(root);
        assert!(root.starts_with(std::env::temp_dir()));
        assert!(root
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("rustdesk-helper-runtime-"));
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        if std::env::var("RUSTDESK_HELPER_RESUME").ok().as_deref() == Some("yes") {
            use hbb_common::base64::{engine::general_purpose::STANDARD, Engine as _};
            let public = STANDARD.decode(std::env::var("RUSTDESK_HELPER_FIXTURE_PUBLIC").unwrap()).unwrap();
            if let Ok(case) = std::env::var("RUSTDESK_CLEANUP_CASE") {
                let receipt = std::fs::read(root.join("execution-result.json")).ok();
                let transaction = std::fs::read(root.join("transaction.json")).unwrap();
                let identity = std::fs::read(root.join("state/agent.json")).unwrap();
                let marker_before = std::fs::read(root.join("state/maintenance.json")).ok();
                let refused = case.starts_with("refuse-");
                let resumed =
                    helper_runtime::resume_with_fixture_key(&root, "fixture-key", &public);
                if refused {
                    let error = resumed.err().expect("unsafe cleanup must be refused");
                    let expected = match case.as_str() {
                        "refuse-foreign-marker" => "maintenance binding mismatch",
                        "refuse-malformed" => "installation terminal result malformed",
                        "refuse-disabled" => "installation locally blocked",
                        "refuse-foreign-task" => "resume task Command mismatch",
                        _ => "maintenance",
                    };
                    assert!(
                        error.contains(expected)
                            || (expected == "maintenance"
                                && error.contains("workspace path unavailable")),
                        "{case}: {error}"
                    );
                    assert_eq!(
                        std::fs::read(root.join("state/maintenance.json")).ok(),
                        marker_before
                    );
                    if case == "refuse-disabled" {
                        assert_eq!(
                            std::fs::read(root.join("state/disable-agent")).unwrap(),
                            b"keep disabled"
                        );
                    }
                } else {
                    let mut owned = resumed.unwrap();
                    let mut backend = ExecutionFixture {
                        calls: vec![],
                        pending: false,
                        fail_validation: false,
                    };
                    owned.resume(&mut backend).unwrap();
                    assert!(backend.calls.is_empty());
                    owned.finalize().unwrap();
                    owned.finalize().unwrap();
                    assert!(!root.join("state/maintenance.json").exists());
                    assert!(resume_task::read_back(&owned.transaction().id).is_err());
                }
                assert_eq!(
                    std::fs::read(root.join("execution-result.json")).ok(),
                    receipt
                );
                assert_eq!(
                    std::fs::read(root.join("transaction.json")).unwrap(),
                    transaction
                );
                assert_eq!(
                    std::fs::read(root.join("state/agent.json")).unwrap(),
                    identity
                );
                std::fs::write(root.join("helper-resumed"), b"terminal result recovered").unwrap();
                return;
            }
            let mut owned = helper_runtime::resume_with_fixture_key(&root, "fixture-key", &public).unwrap();
            let before = std::fs::read(root.join("transaction.json")).unwrap();
            let mut backend = ExecutionFixture { calls: vec![], pending: false, fail_validation: false };
            if owned.transaction().phase == transaction::Phase::StoppingServices {
                let maintenance = std::fs::read(root.join("state/maintenance.json")).unwrap();
                assert_eq!(owned.resume(&mut backend).unwrap_err(), "installation activity unconfirmed");
                assert!(backend.calls.is_empty(), "resume refusal must not perform a rollback installation");
                assert_eq!(std::fs::read(root.join("transaction.json")).unwrap(), before);
                assert_eq!(std::fs::read(root.join("state/maintenance.json")).unwrap(), maintenance);
                assert!(!root.join("execution-result.json").exists());
                std::fs::write(root.join("helper-resumed"), b"activity refusal preserved").unwrap();
                return;
            }
            let pending = owned.transaction().phase == transaction::Phase::Applying;
            if owned.transaction().phase == transaction::Phase::RolledBack {
                let path = root.join("execution-result.json");
                let original = std::fs::read(&path).unwrap();
                let expected: serde_json::Value = serde_json::from_slice(&original).unwrap();
                assert!(expected["error"].as_str().unwrap().contains("fixture target health failure"));
                let result = owned.resume(&mut backend).unwrap();
                assert_eq!(result.phase, transaction::Phase::RolledBack);
                assert_eq!(result.error.as_deref(), expected["error"].as_str());
                assert!(backend.calls.is_empty());
                assert_eq!(std::fs::read(&path).unwrap(), original);
                assert_eq!(std::fs::read(root.join("transaction.json")).unwrap(), before);
                owned.finalize().unwrap();
                assert!(!root.join("state/maintenance.json").exists());
                assert!(resume_task::read_back(&owned.transaction().id).is_err());
                std::fs::write(root.join("helper-resumed"), b"terminal result recovered").unwrap();
                return;
            }
            if !pending {
                let path = root.join("execution-result.json");
                let original = std::fs::read(&path).unwrap();
                let original_value: serde_json::Value = serde_json::from_slice(&original).unwrap();
                let marker = std::fs::read(root.join("state/maintenance.json")).unwrap();
                for field in ["error", "command_id", "schema", "transaction_id", "phase", "reboot_required"] {
                    let mut incomplete = original_value.clone();
                    incomplete.as_object_mut().unwrap().remove(field);
                    let bytes = serde_json::to_vec(&incomplete).unwrap();
                    std::fs::write(&path, &bytes).unwrap();
                    assert!(owned.resume(&mut backend).is_err(), "terminal resume must reject missing {field}");
                    assert_eq!(std::fs::read(&path).unwrap(), bytes);
                    assert_eq!(std::fs::read(root.join("transaction.json")).unwrap(), before);
                    assert_eq!(std::fs::read(root.join("state/maintenance.json")).unwrap(), marker);
                    assert!(backend.calls.is_empty());
                }
                for (field, value) in [
                    ("schema", serde_json::json!(2)),
                    ("transaction_id", serde_json::json!("f".repeat(32))),
                    ("command_id", serde_json::json!("f".repeat(32))),
                    ("phase", serde_json::json!("rolled_back")),
                    ("reboot_required", serde_json::json!(true)),
                    ("error", serde_json::json!("contradictory success")),
                    ("error", serde_json::json!({"unexpected":true})),
                ] {
                    let mut inconsistent = original_value.clone();
                    inconsistent[field] = value;
                    let bytes = serde_json::to_vec(&inconsistent).unwrap();
                    std::fs::write(&path, &bytes).unwrap();
                    assert!(owned.resume(&mut backend).is_err(), "terminal resume must reject inconsistent {field}");
                    assert_eq!(std::fs::read(&path).unwrap(), bytes);
                    assert_eq!(std::fs::read(root.join("transaction.json")).unwrap(), before);
                    assert_eq!(std::fs::read(root.join("state/maintenance.json")).unwrap(), marker);
                    assert!(backend.calls.is_empty());
                }
                std::fs::write(&path, b"invalid json").unwrap();
                assert_eq!(owned.resume(&mut backend).unwrap_err(), "installation terminal result malformed");
                assert_eq!(std::fs::read(&path).unwrap(), b"invalid json");
                std::fs::write(&path, &original).unwrap();
                owned.resume(&mut backend).unwrap();
                assert_eq!(std::fs::read(&path).unwrap(), original, "valid terminal receipt must remain byte-for-byte unchanged");
                std::fs::remove_file(&path).unwrap();
                assert_eq!(owned.resume(&mut backend).unwrap().phase, transaction::Phase::Committed);
                let recovered: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
                assert_eq!(recovered, original_value, "absent receipt can be reconstructed after terminal phase persistence");
                assert!(backend.calls.is_empty());
            }
            if pending {
                assert!(owned.resume(&mut backend).unwrap_err().contains("reconciliation required"));
                assert!(backend.calls.is_empty());
                assert_eq!(std::fs::read(root.join("transaction.json")).unwrap(), before);
                let plan = install_plan::InstallPlan::decode(&std::fs::read(root.join("prepared.json")).unwrap()).unwrap();
                let receipt = serde_json::json!({"schema":1,"transaction_id":plan.transaction_id,
                    "package_sha256":plan.package.sha256,"step":"install","outcome":null});
                state_store::publish(&root.join("install-started.json"), &serde_json::to_vec(&receipt).unwrap()).unwrap();
                let mut completed = receipt;
                completed["outcome"] = serde_json::to_value(install_execution::InstallOutcome::Installed).unwrap();
                state_store::publish(&root.join("install-result.json"), &serde_json::to_vec(&completed).unwrap()).unwrap();
            }
            assert_eq!(owned.resume(&mut backend).unwrap().phase, transaction::Phase::Committed);
            if pending {
                assert_eq!(backend.calls, ["validate"], "confirmed install must only validate on resume");
            } else {
                assert!(backend.calls.is_empty(), "terminal resume must never reinstall");
                assert_eq!(std::fs::read(root.join("transaction.json")).unwrap(), before);
            }
            std::fs::write(root.join("helper-resumed"), b"terminal result recovered").unwrap();
            let result_path = root.join("execution-result.json");
            let result = std::fs::read(&result_path).unwrap();
            std::fs::write(&result_path, b"invalid result").unwrap();
            assert!(owned.finalize().is_err());
            assert!(root.join("state/maintenance.json").exists());
            let complete: serde_json::Value = serde_json::from_slice(&result).unwrap();
            let marker_before = std::fs::read(root.join("state/maintenance.json")).unwrap();
            for invalid in [
                {
                    let mut value = complete.clone();
                    value.as_object_mut().unwrap().remove("error");
                    value
                },
                {
                    let mut value = complete.clone();
                    value["error"] = "target validation failed".into();
                    value
                },
                {
                    let mut value = complete.clone();
                    value["error"] = serde_json::json!({"unexpected":true});
                    value
                },
            ] {
                let bytes = serde_json::to_vec(&invalid).unwrap();
                std::fs::write(&result_path, &bytes).unwrap();
                assert!(owned.finalize().is_err(), "incomplete or contradictory success must retain recovery ownership");
                assert_eq!(std::fs::read(&result_path).unwrap(), bytes);
                assert_eq!(std::fs::read(root.join("state/maintenance.json")).unwrap(), marker_before);
                assert!(resume_task::read_back(&owned.transaction().id).is_ok());
            }
            std::fs::write(&result_path, result).unwrap();
            owned.finalize().unwrap();
            assert!(!root.join("state/maintenance.json").exists());
            assert!(resume_task::read_back(&owned.transaction().id).is_err());
            return;
        }
        while !root.join("handoff.json").exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        helper_runtime::prepare_ready(&root).unwrap();
        let release_deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !root.join("helper-release.json").exists()
            && std::time::Instant::now() < release_deadline
        {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let takeover_deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            let public = std::env::var("RUSTDESK_HELPER_FIXTURE_PUBLIC").unwrap();
            use hbb_common::base64::{engine::general_purpose::STANDARD, Engine as _};
            let public = STANDARD.decode(public).unwrap();
            match helper_runtime::take_over_with_fixture_key(&root, "fixture-key", &public) {
                Ok(mut owned)
                    if std::env::var("RUSTDESK_HELPER_VALID_TASK").ok().as_deref()
                        == Some("yes") && matches!(std::env::var("RUSTDESK_HELPER_LOCAL_BLOCK").unwrap_or_default().as_str(), "" | "execute" | "pending" | "rollback" | "remote-idle-check") =>
                {
                    assert_eq!(
                        owned.transaction().phase,
                        transaction::Phase::StoppingServices
                    );
                    std::fs::write(root.join("helper-owned"), b"resume task verified").unwrap();
                    let mode = std::env::var("RUSTDESK_HELPER_LOCAL_BLOCK").unwrap_or_default();
                    if mode == "remote-idle-check" {
                        let mut backend = ExecutionFixture { calls: vec![], pending: false, fail_validation: false };
                        let before = std::fs::read(root.join("transaction.json")).unwrap();
                        let maintenance = std::fs::read(root.join("state/maintenance.json")).unwrap();
                        // This fixture deliberately requires an unavailable build. A
                        // live but differently versioned server cannot authorize stop.
                        assert_eq!(owned.execute(&mut backend).unwrap_err(), "installation activity unconfirmed");
                        assert!(backend.calls.is_empty(), "refusal must not stop, install, validate or roll back");
                        assert_eq!(std::fs::read(root.join("transaction.json")).unwrap(), before);
                        assert_eq!(std::fs::read(root.join("state/maintenance.json")).unwrap(), maintenance);
                        assert!(!root.join("execution-result.json").exists());
                        assert!(!root.join("install-started.json").exists());
                    }
                    if matches!(mode.as_str(), "execute" | "pending" | "rollback") {
                        let refused = helper_runtime::resume_with_fixture_key(&root, "fixture-key", &public);
                        assert_eq!(refused.err().unwrap(), "original helper still running");
                        let mut backend = ExecutionFixture { calls: vec![], pending: mode == "pending", fail_validation: mode == "rollback" };
                        let block = root.join("state/disable-agent");
                        assert!(owned.finalize().is_err(), "active transactions must retain maintenance");
                        std::fs::write(&block, b"stop-after-handoff").unwrap();
                        let before = std::fs::read(root.join("transaction.json")).unwrap();
                        assert!(owned.execute(&mut backend).unwrap_err().contains("locally blocked"));
                        assert!(backend.calls.is_empty());
                        assert_eq!(std::fs::read(root.join("transaction.json")).unwrap(), before);
                        std::fs::remove_file(block).unwrap();
                        let result = owned.execute(&mut backend).unwrap();
                        assert_eq!(result.phase, if mode == "pending" { transaction::Phase::Applying } else if mode == "rollback" { transaction::Phase::RolledBack } else { transaction::Phase::Committed });
                        let calls = if mode == "pending" { vec!["stop", "apply"] } else if mode == "rollback" { vec!["stop", "apply", "validate", "rollback"] } else { vec!["stop", "apply", "validate"] };
                        assert_eq!(backend.calls, calls);
                        let before = std::fs::read(root.join("transaction.json")).unwrap();
                        assert!(owned.execute(&mut backend).is_err());
                        assert_eq!(backend.calls, calls);
                        assert_eq!(std::fs::read(root.join("transaction.json")).unwrap(), before);
                    }
                    break;
                }
                Ok(_) => panic!("missing resume task must not authorize service stop"),
                Err(error) if error.contains("resume task readback failed") => {
                    std::fs::write(root.join("helper-refused"), b"resume task missing").unwrap();
                    break;
                }
                Err(error) if error == "installation locally blocked"
                    && std::env::var("RUSTDESK_HELPER_LOCAL_BLOCK").ok().as_deref() == Some("disable-agent") => {
                    std::fs::write(root.join("helper-refused"), error.as_bytes()).unwrap();
                    break;
                }
                Err(error) if error == "maintenance binding mismatch"
                    && std::env::var("RUSTDESK_HELPER_LOCAL_BLOCK").ok().as_deref() == Some("foreign-maintenance") => {
                    std::fs::write(root.join("helper-refused"), error.as_bytes()).unwrap();
                    break;
                }
                Err(error)
                    if error.contains("busy") && std::time::Instant::now() < takeover_deadline =>
                {
                    std::thread::sleep(std::time::Duration::from_millis(10))
                }
                Err(error) => panic!("helper takeover failed: {error}"),
            }
        }
        return;
    }
    run_helper_scenario(false, "");
}

#[test]
fn helper_takes_over_only_with_registered_bound_resume_task() {
    run_helper_scenario(true, "");
}

#[test]
fn valid_helper_and_resume_cannot_override_local_disable_or_foreign_maintenance() {
    run_helper_scenario(true, "disable-agent");
    run_helper_scenario(true, "foreign-maintenance");
}

#[test]
fn owned_helper_rechecks_local_blocks_and_executes_only_once_with_persisted_phases() {
    run_helper_scenario(true, "execute");
}

#[test]
fn interrupted_applying_helper_requires_bound_result_and_never_reinstalls() {
    run_helper_scenario(true, "pending");
}

#[test]
fn terminal_rollback_resume_preserves_original_error_without_backend_actions() {
    run_helper_scenario(true, "rollback");
}

#[test]
fn remote_helper_unknown_activity_refuses_before_any_backend_action_and_preserves_evidence() {
    run_helper_scenario(true, "remote-idle-check");
}

#[test]
fn production_parent_handoff_checks_gate_before_releasing_lock_to_actual_child() {
    run_helper_scenario(true, "parent-launch");
}

#[test]
fn terminal_cleanup_missing_maintenance_reenters_in_real_child() {
    run_helper_scenario(true, "cleanup-missing-marker");
}
#[test]
fn terminal_cleanup_absent_task_reenters_in_real_child() {
    run_helper_scenario(true, "cleanup-absent-task");
}
#[test]
fn terminal_cleanup_both_absent_is_idempotent_in_real_child() {
    run_helper_scenario(true, "cleanup-both-absent");
}
#[test]
fn terminal_cleanup_rollback_missing_marker_keeps_error_in_real_child() {
    run_helper_scenario(true, "cleanup-rollback-missing-marker");
}
#[test]
fn terminal_cleanup_refuses_foreign_and_untrusted_state() {
    for case in [
        "refuse-foreign-marker",
        "refuse-malformed",
        "refuse-missing-receipt",
        "refuse-nonterminal",
        "refuse-disabled",
        "refuse-foreign-task",
    ] {
        run_helper_scenario(true, &format!("cleanup-{case}"));
    }
}

fn run_helper_scenario(valid_task: bool, local_block: &str) {
    use install_plan::{Artifact, InstallPlan, OriginalService, Trigger};
    use sha2::{Digest, Sha256};
    let cleanup_case = local_block.strip_prefix("cleanup-");
    let local_block = if cleanup_case == Some("rollback-missing-marker") {
        "rollback"
    } else if cleanup_case.is_some() {
        "execute"
    } else {
        local_block
    };
    let root = std::env::temp_dir().join(format!(
        "rustdesk-helper-runtime-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    workspace::create(&root).unwrap();
    std::fs::create_dir(root.join("runtime")).unwrap();
    std::fs::create_dir(root.join("backups")).unwrap();
    let helper = root.join("runtime/helper.exe");
    std::fs::copy(std::env::current_exe().unwrap(), &helper).unwrap();
    let artifact = |relative: &str| {
        let bytes = std::fs::read(root.join(relative)).unwrap();
        Artifact {
            relative_path: relative.into(),
            size: bytes.len() as u64,
            sha256: hex::encode(Sha256::digest(bytes)),
        }
    };
    for path in [
        "package.msi",
        "backups/client.bin",
        "backups/agent.exe",
        "backups/identity.toml",
    ] {
        std::fs::write(root.join(path), path.as_bytes()).unwrap();
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let transaction_id = hex::encode(hbb_common::sodiumoxide::randombytes::randombytes(16));
    let mut plan=InstallPlan {
        schema:1,transaction_id:transaction_id.clone(),trigger:Trigger::LocalRecovery,command_id:None,installation_id:"b".repeat(32),identity_fingerprint:"c".repeat(64),target_key:"windows-x86_64-msi-standard".into(),original_version:"1.5.3".into(),original_build_seq:2026100502,target_version:"1.5.3".into(),target_build_seq:2026101101,normal_install_path:r"C:\Program Files\RustDesk".into(),package:artifact("package.msi"),client_backup:artifact("backups/client.bin"),agent_backup:artifact("backups/agent.exe"),identity_backup:artifact("backups/identity.toml"),helper:artifact("runtime/helper.exe"),helper_version:"1.5.3".into(),resume_task:format!("RustDeskManagementAgentResume-{transaction_id}"),created_at:now,
        services:vec![OriginalService{name:"RustDesk".into(),command:r#""C:\Program Files\RustDesk\RustDesk.exe" --service"#.into(),account:"LocalSystem".into(),start_type:"auto".into(),running:true},OriginalService{name:"RustDeskManagementAgent".into(),command:r#""C:\ProgramData\RustDesk\ManagementAgent\rustdesk-management-agent.exe" --service --edition standard --package-kind msi --normal-install-path "C:\Program Files\RustDesk""#.into(),account:"LocalSystem".into(),start_type:"auto".into(),running:true}],
    };
    if local_block == "remote-idle-check" {
        plan.trigger = Trigger::RemoteCommand;
        plan.command_id = Some(transaction_id.clone());
        plan.original_build_seq = 1;
    }
    let mut tx = transaction::Transaction::new(
        plan.transaction_id.clone(),
        plan.target_key.clone(),
        plan.package.sha256.clone(),
        plan.identity_fingerprint.clone(),
        now,
    )
    .unwrap();
    let evidence = transaction::Evidence {
        package_verified: true,
        backups_verified: false,
        helper_verified: false,
        resume_task_verified: false,
        handoff_verified: false,
        maintenance_owned: false,
        services_stopped: false,
        target_verified: false,
        identity_unchanged: false,
        client_ready: false,
        agent_ready: false,
        rollback_verified: false,
        locally_blocked: false,
    };
    tx.save(&root.join("transaction.json")).unwrap();
    tx.advance(transaction::Phase::PackageVerified, &evidence, now)
        .unwrap();
    tx.save(&root.join("transaction.json")).unwrap();
    plan.save_bound(&root.join("prepared.json"), &tx).unwrap();
    let parent = handoff::ObservedProcess::open(std::process::id()).unwrap();
    use hbb_common::{
        base64::{engine::general_purpose::STANDARD, Engine as _},
        sodiumoxide::crypto::sign,
        UpdateManifest, UpdateTarget,
    };
    hbb_common::sodiumoxide::init().unwrap();
    let (public, secret) = sign::gen_keypair();
    let mut manifest = UpdateManifest {
        schema: 2,
        product: "rustdesk-yan".into(),
        edition: "multi".into(),
        channel: "stable".into(),
        catalog_revision: 2026101101,
        ..Default::default()
    };
    let mut target = UpdateTarget {
        version: plan.target_version.clone(),
        build_seq: plan.target_build_seq,
        source_commit: "fixture-commit".into(),
        primary: "https://download.yan.life/rustdesk/stable/1.5.3/package.msi".into(),
        size: plan.package.size,
        sha256: plan.package.sha256.clone(),
        signature_key_id: "fixture-key".into(),
        ..Default::default()
    };
    target.signature = STANDARD.encode(sign::sign_detached(
        &base::update::update_signature_payload(&manifest, &plan.target_key, &target).unwrap(),
        &secret,
    ));
    manifest.targets.insert(plan.target_key.clone(), target);
    state_store::publish(
        &root.join("approved-manifest.json"),
        &serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    if local_block == "parent-launch" {
        std::fs::create_dir(root.join("state")).unwrap();
        std::fs::write(root.join("state/agent.json"), serde_json::to_vec(&serde_json::json!({
            "agent_installation_id":plan.installation_id,"disabled":false,"management_agent_enabled":true
        })).unwrap()).unwrap();
        std::fs::write(root.join("intent.toml"), b"[options]\nstop-service='N'\n").unwrap();
        std::fs::write(root.join("state/disable-agent"), b"preserve").unwrap();
        let before = std::fs::read(root.join("transaction.json")).unwrap();
        assert!(parent_handoff::launch_fixture(&root, public.as_ref()).unwrap_err().contains("blocked"));
        assert!(!root.join("handoff.json").exists());
        assert!(!root.join("state/maintenance.json").exists());
        assert_eq!(std::fs::read(root.join("transaction.json")).unwrap(), before);
        std::fs::remove_file(root.join("state/disable-agent")).unwrap();
        let mut child = parent_handoff::launch_fixture(&root, public.as_ref()).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        while child.try_wait().unwrap().is_none() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        if child.try_wait().unwrap().is_none() { child.kill().unwrap(); }
        let status = child.wait().unwrap();
        resume_task::remove(&transaction_id).unwrap();
        assert!(status.success(), "{}", std::fs::read_to_string(root.join("parent-helper.stderr.log")).unwrap());
        assert_eq!(std::fs::read(root.join("helper-owned")).unwrap(), b"resume task verified");
        assert_eq!(transaction::Transaction::decode(&std::fs::read(root.join("transaction.json")).unwrap()).unwrap().phase, transaction::Phase::StoppingServices);
        assert!(parent_handoff::launch_fixture(&root, public.as_ref()).is_err(), "no second launch of prepared transaction");
        std::fs::remove_dir_all(root).unwrap();
        return;
    }
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    let parent_guard = loop {
        if let Some(guard) = install_guard::InstallGuard::try_acquire().unwrap() {
            break guard;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "fixture lock contention did not end"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    };
    let mut child = std::process::Command::new(&helper)
        .args([
            "--exact",
            "helper_process_prepares_only_after_exact_assets_identity_and_parent_validation",
            "--nocapture",
        ])
        .env("RUSTDESK_HELPER_RUNTIME_FIXTURE", &root)
        .env("RUSTDESK_HELPER_FIXTURE_PUBLIC", STANDARD.encode(public))
        .env("RUSTDESK_HELPER_LOCAL_BLOCK", local_block)
        .env(
            "RUSTDESK_HELPER_VALID_TASK",
            if valid_task { "yes" } else { "" },
        )
        .stdout(std::process::Stdio::null())
        .stderr(std::fs::File::create(root.join("helper.stderr.log")).unwrap())
        .spawn()
        .unwrap();
    let process = handoff::ObservedProcess::open(child.id()).unwrap();
    let bytes = std::fs::read(root.join("prepared.json")).unwrap();
    let binding = handoff::Handoff::new(
        plan.transaction_id.clone(),
        hex::encode(Sha256::digest(&bytes)),
        "d".repeat(64),
        parent.owner().clone(),
        process.owner().clone(),
        now,
    )
    .unwrap();
    state_store::publish(
        &root.join("handoff.json"),
        &serde_json::to_vec(&binding).unwrap(),
    )
    .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    while !root.join("helper-ready.json").exists() && std::time::Instant::now() < deadline {
        assert!(child.try_wait().unwrap().is_none());
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let receipt: handoff::ReadyReceipt =
        serde_json::from_slice(&std::fs::read(root.join("helper-ready.json")).unwrap()).unwrap();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    binding.verify_ready(&receipt, now).unwrap();
    assert_eq!(
        transaction::Transaction::decode(&std::fs::read(root.join("transaction.json")).unwrap())
            .unwrap()
            .phase,
        transaction::Phase::PackageVerified,
        "readiness must not stop services or advance installation"
    );
    let mut prepared_evidence = evidence;
    prepared_evidence.backups_verified = true;
    prepared_evidence.helper_verified = true;
    prepared_evidence.resume_task_verified = true;
    prepared_evidence.handoff_verified = true;
    prepared_evidence.maintenance_owned = true;
    for phase in [
        transaction::Phase::Prepared,
        transaction::Phase::StoppingServices,
    ] {
        tx.advance(phase, &prepared_evidence, now).unwrap();
        tx.save(&root.join("transaction.json")).unwrap();
    }
    let released = binding.release_receipt(&receipt, now).unwrap();
    std::fs::create_dir(root.join("state")).unwrap();
    std::fs::write(root.join("state/agent.json"), serde_json::to_vec(&serde_json::json!({
        "agent_installation_id":plan.installation_id,"disabled":false,"management_agent_enabled":true
    })).unwrap()).unwrap();
    std::fs::write(root.join("intent.toml"), b"[options]\nstop-service='N'\n").unwrap();
    std::fs::write(root.join("state/maintenance.json"), serde_json::to_vec(&serde_json::json!({
        "schema":1,"installation_id":plan.installation_id,"transaction_id":plan.transaction_id,
        "plan_sha256":binding.plan_sha256,"nonce":binding.nonce
    })).unwrap()).unwrap();
    if local_block == "disable-agent" {
        std::fs::write(root.join("state/disable-agent"), b"preserve local stop").unwrap();
    } else if local_block == "foreign-maintenance" {
        let path = root.join("state/maintenance.json");
        let mut marker: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        marker["nonce"] = "e".repeat(64).into();
        std::fs::write(path, serde_json::to_vec(&marker).unwrap()).unwrap();
    }
    let maintenance_before = std::fs::read(root.join("state/maintenance.json")).unwrap();
    let definition = resume_task::definition(&helper, &plan.transaction_id).unwrap();
    state_store::publish(&root.join("resume-definition.xml"), definition.as_bytes()).unwrap();
    if valid_task {
        resume_task::register_definition(&root, &transaction_id, &definition).unwrap();
    }
    state_store::publish(
        &root.join("helper-release.json"),
        &serde_json::to_vec(&released).unwrap(),
    )
    .unwrap();
    assert!(
        helper_runtime::take_over(&root).is_err(),
        "parent process must not impersonate helper"
    );
    drop(parent_guard);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    while child.try_wait().unwrap().is_none() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    if child.try_wait().unwrap().is_none() {
        child.kill().unwrap();
    }
    let exit = child.wait().unwrap();
    if matches!(local_block, "execute" | "pending" | "rollback" | "remote-idle-check") && exit.success() {
        if let Some(case)=cleanup_case {
            if matches!(case,"missing-marker"|"rollback-missing-marker"|"both-absent"|"refuse-malformed"|"refuse-missing-receipt"|"refuse-nonterminal") {
                std::fs::remove_file(root.join("state/maintenance.json")).unwrap();
            }
            if matches!(case,"absent-task"|"both-absent") {resume_task::remove(&transaction_id).unwrap();}
            if case=="refuse-foreign-marker" {
                let path=root.join("state/maintenance.json");
                let mut marker:serde_json::Value=serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
                marker["nonce"]="e".repeat(64).into();
                std::fs::write(path,serde_json::to_vec(&marker).unwrap()).unwrap();
            }
            if case=="refuse-malformed" {std::fs::write(root.join("execution-result.json"),b"malformed receipt").unwrap();}
            if case=="refuse-missing-receipt" {std::fs::remove_file(root.join("execution-result.json")).unwrap();}
            if case=="refuse-nonterminal" {
                let mut tx=transaction::Transaction::decode(&std::fs::read(root.join("transaction.json")).unwrap()).unwrap();
                tx.phase=transaction::Phase::Applying;std::fs::write(root.join("transaction.json"),serde_json::to_vec(&tx).unwrap()).unwrap();
            }
            if case=="refuse-disabled" {std::fs::write(root.join("state/disable-agent"),b"keep disabled").unwrap();}
            if case=="refuse-foreign-task" {
                assert!(std::process::Command::new(r"C:\Windows\System32\schtasks.exe")
                    .args(["/Change","/TN",&format!("RustDeskManagementAgentResume-{transaction_id}"),"/TR",r"C:\Windows\System32\cmd.exe /c exit 0"]).status().unwrap().success());
            }
        }
        let mut command = std::process::Command::new(&helper);
        if let Some(case)=cleanup_case {command.env("RUSTDESK_CLEANUP_CASE",case);}
        let mut child = command
            .args(["--exact", "helper_process_prepares_only_after_exact_assets_identity_and_parent_validation", "--nocapture"])
            .env("RUSTDESK_HELPER_RUNTIME_FIXTURE", &root)
            .env("RUSTDESK_HELPER_FIXTURE_PUBLIC", STANDARD.encode(public))
            .env("RUSTDESK_HELPER_RESUME", "yes")
            .stdout(std::process::Stdio::null())
            .stderr(std::fs::File::create(root.join("resume.stderr.log")).unwrap())
            .spawn().unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        while child.try_wait().unwrap().is_none() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        if child.try_wait().unwrap().is_none() { child.kill().unwrap(); }
        let exit = child.wait().unwrap();
        if !exit.success() {
            let diagnostic = std::fs::read_to_string(root.join("resume.stderr.log")).unwrap();
            let cleanup = resume_task::remove(&transaction_id);
            panic!("resume failed: {diagnostic}; fixture task cleanup: {cleanup:?}");
        }
        let expected: &[u8] = if local_block == "remote-idle-check" { b"activity refusal preserved" } else { b"terminal result recovered" };
        assert_eq!(std::fs::read(root.join("helper-resumed")).unwrap(), expected);
        if let Some(case)=cleanup_case {
            if case.starts_with("refuse-") {
                let task = resume_task::read_back(&transaction_id).unwrap();
                if case == "refuse-foreign-task" {
                    assert!(task.contains("cmd.exe"));
                }
                resume_task::remove(&transaction_id).unwrap();
            }
        }
    }
    if valid_task && !matches!(local_block, "execute" | "pending" | "rollback") {
        resume_task::remove(&transaction_id).unwrap();
    }
    assert!(
        exit.success(),
        "helper exit {exit}: {}",
        std::fs::read_to_string(root.join("helper.stderr.log")).unwrap()
    );
    if !local_block.is_empty() && !matches!(local_block, "execute" | "pending" | "rollback" | "remote-idle-check") {
        let expected = if local_block == "disable-agent" { "installation locally blocked" } else { "maintenance binding mismatch" };
        assert_eq!(std::fs::read_to_string(root.join("helper-refused")).unwrap(), expected);
        assert!(!root.join("helper-owned").exists());
        assert_eq!(std::fs::read(root.join("state/maintenance.json")).unwrap(), maintenance_before);
        if local_block == "disable-agent" {
            assert_eq!(std::fs::read(root.join("state/disable-agent")).unwrap(), b"preserve local stop");
        }
    } else if valid_task {
        assert_eq!(
            std::fs::read(root.join("helper-owned")).unwrap(),
            b"resume task verified"
        );
    } else {
        assert_eq!(
            std::fs::read(root.join("helper-refused")).unwrap(),
            b"resume task missing"
        );
    }
    assert_eq!(
        transaction::Transaction::decode(&std::fs::read(root.join("transaction.json")).unwrap())
            .unwrap()
            .phase,
        if cleanup_case==Some("refuse-nonterminal") { transaction::Phase::Applying } else if matches!(local_block, "execute" | "pending") { transaction::Phase::Committed } else if local_block == "rollback" { transaction::Phase::RolledBack } else { transaction::Phase::StoppingServices },
        "takeover validation must not execute install or stop services"
    );
    std::fs::remove_dir_all(root).unwrap();
}
