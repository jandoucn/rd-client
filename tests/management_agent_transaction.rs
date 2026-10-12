#[path = "../src/management_agent/transaction.rs"]
mod transaction;

use transaction::{Evidence, Phase, RecoveryStep, Transaction};

fn fixture() -> Transaction {
    Transaction::new(
        "a".repeat(32),
        "windows-x86_64-msi-standard".into(),
        "b".repeat(64),
        "c".repeat(64),
        1000,
    )
    .unwrap()
}

fn ready() -> Evidence {
    Evidence {
        package_verified: true,
        backups_verified: true,
        helper_verified: true,
        resume_task_verified: true,
        handoff_verified: true,
        maintenance_owned: true,
        services_stopped: true,
        target_verified: true,
        identity_unchanged: true,
        client_ready: true,
        agent_ready: true,
        rollback_verified: true,
        locally_blocked: false,
    }
}

#[test]
fn stop_stage_requires_protected_backups_helper_resume_and_handoff() {
    let mut transaction = fixture();
    let evidence = ready();
    transaction
        .advance(Phase::PackageVerified, &evidence, 1001)
        .unwrap();
    for index in 0..5 {
        let mut evidence = ready();
        match index {
            0 => evidence.backups_verified = false,
            1 => evidence.helper_verified = false,
            2 => evidence.resume_task_verified = false,
            3 => evidence.handoff_verified = false,
            _ => evidence.maintenance_owned = false,
        };
        let previous = transaction.clone();
        assert!(transaction
            .advance(Phase::Prepared, &evidence, 1002)
            .is_err());
        assert_eq!(transaction, previous);
    }
    transaction
        .advance(Phase::Prepared, &evidence, 1002)
        .unwrap();
    assert!(transaction
        .advance(Phase::Applying, &evidence, 1003)
        .is_err());
    let mut changed = evidence;
    changed.locally_blocked = true;
    assert!(transaction
        .advance(Phase::ServicesStopped, &changed, 1003)
        .is_err());
    assert_eq!(transaction.phase, Phase::Prepared);
}

#[test]
fn interrupted_apply_reconciles_and_terminal_upload_never_reinstalls() {
    let evidence = ready();
    let mut transaction = fixture();
    for phase in [
        Phase::PackageVerified,
        Phase::Prepared,
        Phase::StoppingServices,
        Phase::ServicesStopped,
        Phase::Applying,
    ] {
        transaction.advance(phase, &evidence, 1001).unwrap();
    }
    assert_eq!(
        transaction.recovery_step(false, 1002),
        RecoveryStep::Reconcile
    );
    assert_eq!(
        transaction.recovery_step(true, 1002),
        RecoveryStep::WaitForOwner
    );
    transaction
        .advance(Phase::Validating, &evidence, 1003)
        .unwrap();
    let mut failed = evidence;
    failed.agent_ready = false;
    assert!(transaction
        .advance(Phase::Committed, &failed, 1004)
        .is_err());
    assert_eq!(transaction.phase, Phase::Validating);
    transaction
        .advance(Phase::Committed, &evidence, 1004)
        .unwrap();
    assert_eq!(
        transaction.recovery_step(false, 1005),
        RecoveryStep::ReportOnly
    );
    assert!(transaction
        .advance(Phase::Applying, &evidence, 1006)
        .is_err());
}

#[test]
fn blocked_mid_replacement_still_allows_verified_rollback_not_false_success() {
    let mut transaction = fixture();
    let evidence = ready();
    for phase in [
        Phase::PackageVerified,
        Phase::Prepared,
        Phase::StoppingServices,
        Phase::ServicesStopped,
        Phase::Applying,
    ] {
        transaction.advance(phase, &evidence, 1001).unwrap();
    }
    let mut blocked = evidence;
    blocked.locally_blocked = true;
    blocked.rollback_verified = false;
    transaction
        .advance(Phase::RollingBack, &blocked, 1002)
        .unwrap();
    assert_eq!(
        transaction.recovery_step(false, 1003),
        RecoveryStep::Rollback
    );
    assert!(transaction
        .advance(Phase::RolledBack, &blocked, 1003)
        .is_err());
    assert_eq!(transaction.phase, Phase::RollingBack);
    transaction
        .advance(Phase::RollbackFailed, &blocked, 1003)
        .unwrap();
    assert_eq!(
        transaction.recovery_step(false, 1004),
        RecoveryStep::ManualReconciliation
    );
}

#[test]
fn schema_and_identity_cannot_be_silently_repaired_and_budget_is_conservative() {
    let mut transaction = fixture();
    assert_eq!(
        transaction.recovery_step(false, 999),
        RecoveryStep::ManualReconciliation
    );
    assert_eq!(
        transaction.recovery_step(false, 1000 + 86400),
        RecoveryStep::ManualReconciliation
    );
    let mut json = serde_json::to_value(&transaction).unwrap();
    json["schema"] = 2.into();
    assert!(Transaction::decode(&serde_json::to_vec(&json).unwrap()).is_err());
    json["schema"] = 1.into();
    json["target_key"] = "windows-x86_64-exe-other".into();
    assert!(Transaction::decode(&serde_json::to_vec(&json).unwrap()).is_err());
    transaction.attempts = 3;
    assert_eq!(
        transaction.recovery_step(false, 1001),
        RecoveryStep::ManualReconciliation
    );
}

#[test]
fn local_snapshot_reloads_exact_phase_and_failure_preserves_previous() {
    let directory =
        std::env::temp_dir().join(format!("rustdesk-transaction-test-{}", std::process::id()));
    std::fs::create_dir(&directory).unwrap();
    let path = directory.join("transaction.json");
    let transaction = fixture();
    transaction.save(&path).unwrap();
    let before = std::fs::read(&path).unwrap();
    let loaded = Transaction::decode(&before).unwrap();
    assert_eq!(loaded, transaction);
    assert!(transaction
        .save(&directory.join("missing/transaction.json"))
        .is_err());
    assert_eq!(std::fs::read(&path).unwrap(), before);
    std::fs::write(&path, b"corrupt old record").unwrap();
    assert!(transaction.save(&path).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), b"corrupt old record");
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn stale_writer_same_timestamp_and_skipped_phase_cannot_overwrite_active_transaction() {
    let directory =
        std::env::temp_dir().join(format!("rustdesk-transaction-cas-{}", std::process::id()));
    std::fs::create_dir(&directory).unwrap();
    let path = directory.join("transaction.json");
    let mut current = fixture();
    current.save(&path).unwrap();
    let mut stale = current.clone();
    let evidence = ready();
    current
        .advance(Phase::PackageVerified, &evidence, 1000)
        .unwrap();
    current.save(&path).unwrap();
    current.advance(Phase::Prepared, &evidence, 1000).unwrap();
    current.save(&path).unwrap();
    let before = std::fs::read(&path).unwrap();
    stale
        .advance(Phase::PackageVerified, &evidence, 1000)
        .unwrap();
    assert!(stale.save(&path).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), before);
    current
        .advance(Phase::StoppingServices, &evidence, 1001)
        .unwrap();
    current
        .advance(Phase::ServicesStopped, &evidence, 1001)
        .unwrap();
    current.advance(Phase::Applying, &evidence, 1002).unwrap();
    assert!(current.save(&path).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), before);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn fabricated_phase_or_rebound_package_cannot_publish_over_the_old_snapshot() {
    let directory = std::env::temp_dir().join(format!(
        "rustdesk-transaction-invalid-{}",
        std::process::id()
    ));
    std::fs::create_dir(&directory).unwrap();
    let path = directory.join("transaction.json");
    let planned = fixture();
    planned.save(&path).unwrap();
    let before = std::fs::read(&path).unwrap();
    let mut forged = planned.clone();
    forged.revision = 1;
    forged.phase = Phase::Applying;
    assert!(forged.save(&path).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), before);
    let mut changed = planned.clone();
    changed
        .advance(Phase::PackageVerified, &ready(), 1001)
        .unwrap();
    changed.package_sha256 = "d".repeat(64);
    assert!(changed.save(&path).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), before);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn committed_and_rolled_back_snapshots_reload_without_restarting_installation() {
    let directory = std::env::temp_dir().join(format!(
        "rustdesk-transaction-terminal-{}",
        std::process::id()
    ));
    std::fs::create_dir(&directory).unwrap();
    let evidence = ready();
    for rollback in [false, true] {
        let path = directory.join(if rollback {
            "rollback.json"
        } else {
            "commit.json"
        });
        let mut transaction = fixture();
        transaction.save(&path).unwrap();
        for phase in [
            Phase::PackageVerified,
            Phase::Prepared,
            Phase::StoppingServices,
            Phase::ServicesStopped,
            Phase::Applying,
        ] {
            transaction.advance(phase, &evidence, 1001).unwrap();
            transaction.save(&path).unwrap();
            assert_eq!(
                Transaction::decode(&std::fs::read(&path).unwrap())
                    .unwrap()
                    .phase,
                phase
            );
        }
        let phases = if rollback {
            [Phase::RollingBack, Phase::RolledBack]
        } else {
            [Phase::Validating, Phase::Committed]
        };
        for phase in phases {
            transaction.advance(phase, &evidence, 1002).unwrap();
            transaction.save(&path).unwrap();
        }
        let loaded = Transaction::decode(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(loaded, transaction);
        assert_eq!(loaded.recovery_step(false, 1003), RecoveryStep::ReportOnly);
        transaction.save(&path).unwrap();
        let before = std::fs::read(&path).unwrap();
        transaction.phase = Phase::Applying;
        transaction.revision += 1;
        assert!(transaction.save(&path).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }
    std::fs::remove_dir_all(directory).unwrap();
}

#[cfg(windows)]
#[test]
fn concurrent_writer_and_invalid_staging_leave_existing_phase_unchanged() {
    use std::os::windows::fs::OpenOptionsExt;
    let directory =
        std::env::temp_dir().join(format!("rustdesk-transaction-lock-{}", std::process::id()));
    std::fs::create_dir(&directory).unwrap();
    let path = directory.join("transaction.json");
    let mut transaction = fixture();
    transaction.save(&path).unwrap();
    let before = std::fs::read(&path).unwrap();
    transaction
        .advance(Phase::PackageVerified, &ready(), 1001)
        .unwrap();
    let lock = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .share_mode(0)
        .open(path.with_extension("lock"))
        .unwrap();
    assert!(transaction
        .save(&path)
        .unwrap_err()
        .contains("transaction writer unavailable"));
    assert_eq!(std::fs::read(&path).unwrap(), before);
    drop(lock);
    std::fs::create_dir(path.with_extension("pending")).unwrap();
    assert!(transaction.save(&path).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), before);
    std::fs::remove_dir(path.with_extension("pending")).unwrap();
    transaction.save(&path).unwrap();
    assert_eq!(
        Transaction::decode(&std::fs::read(&path).unwrap())
            .unwrap()
            .phase,
        Phase::PackageVerified
    );
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn service_stop_intent_is_persisted_before_any_stop_is_authorized() {
    let directory = std::env::temp_dir().join(format!(
        "rustdesk-transaction-stop-intent-{}",
        std::process::id()
    ));
    std::fs::create_dir(&directory).unwrap();
    let path = directory.join("transaction.json");
    let mut transaction = fixture();
    let mut evidence = ready();
    evidence.services_stopped = false;
    transaction.save(&path).unwrap();
    for phase in [Phase::PackageVerified, Phase::Prepared] {
        transaction.advance(phase, &evidence, 1001).unwrap();
        transaction.save(&path).unwrap();
    }
    assert!(!transaction.may_stop_services(&path, &evidence));
    transaction
        .advance(Phase::StoppingServices, &evidence, 1002)
        .unwrap();
    assert!(!transaction.may_stop_services(&path, &evidence));
    transaction.save(&path).unwrap();
    let mut reloaded = Transaction::decode(&std::fs::read(&path).unwrap()).unwrap();
    assert!(reloaded.may_stop_services(&path, &evidence));
    assert_eq!(reloaded.recovery_step(false, 1003), RecoveryStep::Reconcile);
    assert!(reloaded
        .advance(Phase::ServicesStopped, &evidence, 1003)
        .is_err());
    assert_eq!(reloaded.phase, Phase::StoppingServices);
    evidence.services_stopped = true;
    reloaded
        .advance(Phase::ServicesStopped, &evidence, 1003)
        .unwrap();
    reloaded.save(&path).unwrap();
    assert!(!reloaded.may_stop_services(&path, &evidence));
    evidence.locally_blocked = true;
    assert!(!transaction.may_stop_services(&path, &evidence));
    std::fs::remove_dir_all(directory).unwrap();
}
