#[path = "../src/management_agent/install_execution.rs"]
mod install_execution;
#[path = "../src/management_agent/transaction.rs"]
mod transaction;

use install_execution::{execute, Backend, InstallOutcome, Validation};
use transaction::{Evidence, Phase, Transaction};

struct Fixture {
    path: std::path::PathBuf,
    calls: Vec<&'static str>,
    install: InstallOutcome,
    validation: Validation,
    rollback: Validation,
    old_retained: bool,
    stopped: bool,
    fail_stop: bool,
    fail_install: bool,
    fail_validation: bool,
    fail_rollback: bool,
}

impl Fixture {
    fn phase(&self) -> Phase {
        Transaction::decode(&std::fs::read(&self.path).unwrap())
            .unwrap()
            .phase
    }
}

impl Backend for Fixture {
    fn stop_services(&mut self) -> Result<(), String> {
        assert_eq!(self.phase(), Phase::StoppingServices);
        self.calls.push("stop");
        if self.fail_stop {
            return Err("fixture stop refused".into());
        }
        self.stopped = true;
        Ok(())
    }
    fn apply_package(&mut self) -> Result<InstallOutcome, String> {
        assert_eq!(self.phase(), Phase::Applying);
        assert!(self.stopped);
        self.calls.push("install");
        if self.fail_install {
            return Err("fixture installation launch failed".into());
        }
        Ok(self.install.clone())
    }
    fn validate_target(&mut self) -> Result<Validation, String> {
        assert_eq!(self.phase(), Phase::Validating);
        self.calls.push("validate");
        if self.fail_validation {
            return Err("fixture target inspection failed".into());
        }
        Ok(self.validation.clone())
    }
    fn rollback_original(&mut self) -> Result<Validation, String> {
        assert_eq!(self.phase(), Phase::RollingBack);
        self.calls.push("rollback");
        if self.fail_rollback {
            return Err("fixture rollback refused".into());
        }
        self.old_retained = true;
        Ok(self.rollback.clone())
    }
}

fn valid() -> Validation {
    Validation {
        target_matches: true,
        identity_unchanged: true,
        client_ready: true,
        agent_ready: true,
    }
}
fn evidence() -> Evidence {
    Evidence {
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
    }
}
fn fixture(name: &str) -> (std::path::PathBuf, Transaction, Fixture) {
    let root = std::env::temp_dir().join(format!(
        "rustdesk-install-execution-{}-{name}",
        std::process::id()
    ));
    std::fs::create_dir(&root).unwrap();
    let path = root.join("transaction.json");
    let mut tx = Transaction::new(
        "a".repeat(32),
        "windows-x86_64-msi-standard".into(),
        "b".repeat(64),
        "c".repeat(64),
        1000,
    )
    .unwrap();
    tx.save(&path).unwrap();
    for phase in [
        Phase::PackageVerified,
        Phase::Prepared,
        Phase::StoppingServices,
    ] {
        tx.advance(phase, &evidence(), 1001).unwrap();
        tx.save(&path).unwrap();
    }
    let backend = Fixture {
        path,
        calls: vec![],
        install: InstallOutcome::Installed,
        validation: valid(),
        rollback: valid(),
        old_retained: false,
        stopped: false,
        fail_stop: false,
        fail_install: false,
        fail_validation: false,
        fail_rollback: false,
    };
    (root, tx, backend)
}

#[test]
fn success_is_committed_only_after_both_components_identity_and_version_validate() {
    let (root, mut tx, mut backend) = fixture("success");
    let result = execute(
        &backend.path.clone(),
        &mut tx,
        &evidence(),
        &mut backend,
        1002,
    )
    .unwrap();
    assert_eq!(result.phase, Phase::Committed);
    assert_eq!(tx.phase, Phase::Committed);
    assert_eq!(backend.calls, vec!["stop", "install", "validate"]);
    assert!(!backend.old_retained);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn every_validation_failure_rolls_back_and_does_not_claim_success() {
    for index in 0..4 {
        let (root, mut tx, mut backend) = fixture(&format!("validation-{index}"));
        match index {
            0 => backend.validation.target_matches = false,
            1 => backend.validation.identity_unchanged = false,
            2 => backend.validation.client_ready = false,
            _ => backend.validation.agent_ready = false,
        };
        let result = execute(
            &backend.path.clone(),
            &mut tx,
            &evidence(),
            &mut backend,
            1002,
        )
        .unwrap();
        assert_eq!(result.phase, Phase::RolledBack);
        assert_eq!(tx.phase, Phase::RolledBack);
        assert!(result.error.unwrap().contains("validation"));
        assert!(backend.old_retained);
        assert_eq!(
            backend.calls,
            vec!["stop", "install", "validate", "rollback"]
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn install_busy_and_failure_restore_original_without_committing_target() {
    for (name, outcome) in [
        ("busy", InstallOutcome::Busy),
        ("failed", InstallOutcome::Failed(1603)),
    ] {
        let (root, mut tx, mut backend) = fixture(name);
        backend.install = outcome;
        let result = execute(
            &backend.path.clone(),
            &mut tx,
            &evidence(),
            &mut backend,
            1002,
        )
        .unwrap();
        assert_eq!(result.phase, Phase::RolledBack);
        assert!(backend.old_retained);
        assert!(result.error.is_some());
        assert_eq!(backend.calls, vec!["stop", "install", "rollback"]);
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn reboot_required_keeps_validation_and_resume_without_false_success_or_forced_restart() {
    let (root, mut tx, mut backend) = fixture("reboot");
    backend.install = InstallOutcome::RebootRequired;
    backend.validation.client_ready = false;
    let result = execute(
        &backend.path.clone(),
        &mut tx,
        &evidence(),
        &mut backend,
        1002,
    )
    .unwrap();
    assert_eq!(result.phase, Phase::Validating);
    assert!(result.reboot_required);
    assert!(!backend.old_retained);
    assert_eq!(backend.calls, vec!["stop", "install", "validate"]);
    assert_eq!(backend.phase(), Phase::Validating);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn active_installer_timeout_keeps_applying_without_rollback_racing_the_installer() {
    let (root, mut tx, mut backend) = fixture("active-installer");
    backend.install = InstallOutcome::Pending {
        pid: 4321,
        created_at: 2000,
    };
    let result = execute(
        &backend.path.clone(),
        &mut tx,
        &evidence(),
        &mut backend,
        1002,
    )
    .unwrap();
    assert_eq!(result.phase, Phase::Applying);
    assert_eq!(backend.phase(), Phase::Applying);
    assert_eq!(backend.calls, vec!["stop", "install"]);
    assert!(result.error.unwrap().contains("4321"));
    assert!(
        !backend.old_retained,
        "must not race rollback against an active installer"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn failed_rollback_is_durable_and_preserves_failure_diagnostic() {
    let (root, mut tx, mut backend) = fixture("rollback-failed");
    backend.install = InstallOutcome::Failed(1603);
    backend.rollback.agent_ready = false;
    let result = execute(
        &backend.path.clone(),
        &mut tx,
        &evidence(),
        &mut backend,
        1002,
    )
    .unwrap();
    assert_eq!(result.phase, Phase::RollbackFailed);
    assert_eq!(backend.phase(), Phase::RollbackFailed);
    assert!(result.error.unwrap().contains("1603"));
    assert!(backend.old_retained);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn unpersisted_state_missing_evidence_or_storage_failure_prevents_first_side_effect() {
    for index in 0..3 {
        let (root, mut tx, mut backend) = fixture(&format!("gate-{index}"));
        let mut proof = evidence();
        match index {
            0 => tx.revision += 1,
            1 => proof.handoff_verified = false,
            _ => {
                std::fs::write(backend.path.with_extension("pending"), b"uncommitted").unwrap();
            }
        };
        let result = execute(&backend.path.clone(), &mut tx, &proof, &mut backend, 1002);
        assert!(result.is_err());
        assert!(backend.calls.is_empty());
        assert_eq!(backend.phase(), Phase::StoppingServices);
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn stop_failure_restores_original_without_attempting_install() {
    let (root, mut tx, mut backend) = fixture("stop-failed");
    backend.fail_stop = true;
    let result = execute(
        &backend.path.clone(),
        &mut tx,
        &evidence(),
        &mut backend,
        1002,
    )
    .unwrap();
    assert_eq!(result.phase, Phase::RolledBack);
    assert!(result.error.unwrap().contains("stop refused"));
    assert_eq!(backend.calls, vec!["stop", "rollback"]);
    assert!(backend.old_retained);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn every_backend_error_keeps_a_durable_failure_and_verified_original_or_rollback_failed() {
    for index in 0..3 {
        let (root, mut tx, mut backend) = fixture(&format!("backend-error-{index}"));
        match index {
            0 => backend.fail_install = true,
            1 => backend.fail_validation = true,
            _ => {
                backend.install = InstallOutcome::Failed(1603);
                backend.fail_rollback = true;
            }
        };
        let result = execute(
            &backend.path.clone(),
            &mut tx,
            &evidence(),
            &mut backend,
            1002,
        )
        .unwrap();
        assert_eq!(
            result.phase,
            if index == 2 {
                Phase::RollbackFailed
            } else {
                Phase::RolledBack
            }
        );
        assert!(result.error.is_some());
        if index == 2 {
            assert!(result.error.unwrap().contains("rollback refused"));
        } else {
            assert!(backend.old_retained);
        }
        assert_eq!(backend.phase(), result.phase);
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn interrupted_or_pending_install_is_never_reexecuted_without_confirmed_result() {
    let (root, mut tx, mut backend) = fixture("resume-pending");
    backend.install = InstallOutcome::Pending {
        pid: 4321,
        created_at: 2000,
    };
    execute(
        &backend.path.clone(),
        &mut tx,
        &evidence(),
        &mut backend,
        1002,
    )
    .unwrap();
    backend.calls.clear();
    assert!(install_execution::resume(
        &backend.path.clone(),
        &mut tx,
        &evidence(),
        &mut backend,
        None,
        1003
    )
    .unwrap_err()
    .contains("reconciliation"));
    assert!(backend.calls.is_empty());
    assert_eq!(backend.phase(), Phase::Applying);
    let result = install_execution::resume(
        &backend.path.clone(),
        &mut tx,
        &evidence(),
        &mut backend,
        Some(InstallOutcome::Installed),
        1003,
    )
    .unwrap();
    assert_eq!(result.phase, Phase::Committed);
    assert_eq!(backend.calls, vec!["validate"]);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn reboot_validation_and_terminal_reload_do_not_stop_or_reinstall_again() {
    let (root, mut tx, mut backend) = fixture("resume-reboot");
    backend.install = InstallOutcome::RebootRequired;
    execute(
        &backend.path.clone(),
        &mut tx,
        &evidence(),
        &mut backend,
        1002,
    )
    .unwrap();
    backend.calls.clear();
    let result = install_execution::resume(
        &backend.path.clone(),
        &mut tx,
        &evidence(),
        &mut backend,
        None,
        1003,
    )
    .unwrap();
    assert_eq!(result.phase, Phase::Committed);
    assert_eq!(backend.calls, vec!["validate"]);
    backend.calls.clear();
    let terminal = install_execution::resume(
        &backend.path.clone(),
        &mut tx,
        &evidence(),
        &mut backend,
        None,
        1004,
    )
    .unwrap();
    assert_eq!(terminal.phase, Phase::Committed);
    assert!(backend.calls.is_empty());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn interrupted_stop_and_rollback_require_verified_restoration_not_new_install() {
    let (root, mut tx, mut backend) = fixture("resume-stop");
    let result = install_execution::resume(
        &backend.path.clone(),
        &mut tx,
        &evidence(),
        &mut backend,
        None,
        1002,
    )
    .unwrap();
    assert_eq!(result.phase, Phase::RolledBack);
    assert_eq!(backend.calls, vec!["rollback"]);
    assert!(backend.old_retained);
    std::fs::remove_dir_all(root).unwrap();
}
