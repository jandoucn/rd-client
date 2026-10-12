#[path = "../src/management_agent/journal.rs"]
mod journal;

use journal::{CommandJournal, ExecutionState};

#[test]
fn deferred_commands_survive_reload_and_remain_cancellable_or_expirable() {
    let directory = std::env::temp_dir().join(format!("rustdesk-journal-deferred-{}", std::process::id()));
    std::fs::create_dir(&directory).unwrap();
    let path = directory.join("journal.json");
    let mut log = CommandJournal::open(&path).unwrap();
    let id = "a".repeat(32);
    log.prepare(&id, &"b".repeat(64), 2000, 1000).unwrap();
    log.receipt(&id).unwrap();
    log.defer(&id, "active_sessions", 1001).unwrap();
    assert_eq!(log.pending_events().last().unwrap().payload["status"], "deferred");
    assert_eq!(log.pending_events().last().unwrap().payload["result"]["error_code"], "active_sessions");
    assert_eq!(log.operation_snapshot()["state"], "deferred");
    let before = std::fs::read(&path).unwrap();
    let mut log = CommandJournal::open(&path).unwrap();
    log.receipt(&id).unwrap();
    log.defer(&id, "active_sessions", 1002).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), before, "polling must not duplicate defer/receipt events");
    log.start(&id, 1003).unwrap();
    assert_eq!(log.state(&id), Some(ExecutionState::Running));
    assert!(log.defer(&id, "active_sessions", 1004).is_err());
    log.finish(&id, "succeeded", serde_json::json!({}), 1005).unwrap();
    for (id, status, now) in [("c".repeat(32), "cancelled", 1002), ("d".repeat(32), "expired", 2000)] {
        log.prepare(&id, &"e".repeat(64), 2000, 1000).unwrap();
        log.receipt(&id).unwrap();
        log.defer(&id, "activity_unknown", 1001).unwrap();
        if status == "expired" {
            assert_eq!(log.start(&id, now).unwrap_err(), "command expired");
            assert_eq!(log.state(&id), Some(ExecutionState::Expired));
        } else {
            log.abandon(&id, status, now).unwrap();
            assert_eq!(log.state(&id), Some(ExecutionState::Cancelled));
        }
        let before = std::fs::read(&path).unwrap();
        assert!(log.start(&id, now + 1).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn operation_snapshot_reports_running_and_interrupted_work_without_writing() {
    let directory = std::env::temp_dir().join(format!("rustdesk-journal-projection-{}", std::process::id()));
    std::fs::create_dir(&directory).unwrap();
    let path = directory.join("journal.json");
    let mut log = CommandJournal::open(&path).unwrap();
    assert_eq!(log.operation_snapshot()["state"], "idle");
    assert!(!path.exists());
    let id = "a".repeat(32);
    log.prepare(&id, &"b".repeat(64), 2000, 1000).unwrap();
    log.receipt(&id).unwrap();
    log.start(&id, 1001).unwrap();
    let before = std::fs::read(&path).unwrap();
    assert_eq!(log.operation_snapshot()["state"], "running");
    assert_eq!(log.operation_snapshot()["command_id"], id);
    assert_eq!(std::fs::read(&path).unwrap(), before);
    log.finish(&id, "reconciliation_required", serde_json::json!({"error_code":"interrupted_mutation"}), 1002).unwrap();
    let before = std::fs::read(&path).unwrap();
    let reloaded = CommandJournal::open(&path).unwrap();
    assert_eq!(reloaded.operation_snapshot()["state"], "reconciliation_required");
    assert_eq!(reloaded.operation_snapshot()["command_id"], id);
    assert_eq!(std::fs::read(&path).unwrap(), before);
    log.finish(&id, "failed", serde_json::json!({"error_code":"reconciled_failed"}), 1003).unwrap();
    assert_eq!(log.operation_snapshot()["state"], "idle");
    assert_eq!(log.state(&id), Some(ExecutionState::Failed));
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn outbox_capacity_refuses_start_before_losing_room_for_the_terminal_result() {
    use sha2::{Digest, Sha256};
    let directory =
        std::env::temp_dir().join(format!("rustdesk-journal-capacity-{}", std::process::id()));
    std::fs::create_dir(&directory).unwrap();
    let path = directory.join("journal.json");
    let mut log = CommandJournal::open(&path).unwrap();
    log.prepare(&"a".repeat(32), &"b".repeat(64), 2000, 1000)
        .unwrap();
    log.receipt(&"a".repeat(32)).unwrap();
    let mut snapshot: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    let event = snapshot["events"][0].clone();
    let events = (0..9999)
        .map(|index| {
            let mut event = event.clone();
            let id = format!("{index:032x}");
            event["event_id"] = id.clone().into();
            event["payload"]["event_id"] = id.into();
            event["digest"] = hex::encode(Sha256::digest(
                serde_json::to_vec(&event["payload"]).unwrap(),
            ))
            .into();
            event
        })
        .collect::<Vec<_>>();
    snapshot["events"] = events.into();
    std::fs::write(&path, serde_json::to_vec(&snapshot).unwrap()).unwrap();
    let before = std::fs::read(&path).unwrap();
    let mut log = CommandJournal::open(&path).unwrap();
    assert_eq!(
        log.start(&"a".repeat(32), 1001).unwrap_err(),
        "outbox lacks terminal result capacity"
    );
    assert_eq!(log.state(&"a".repeat(32)), Some(ExecutionState::Accepted));
    assert_eq!(std::fs::read(&path).unwrap(), before);
    assert!(log
        .prepare(&"c".repeat(32), &"d".repeat(64), 2000, 1001)
        .is_err());
    assert_eq!(log.state(&"c".repeat(32)), None);
    let acknowledged = log.pending_events()[0].clone();
    log.acknowledge(&acknowledged.event_id, &acknowledged.digest)
        .unwrap();
    log.start(&"a".repeat(32), 1002).unwrap();
    log.finish(
        &"a".repeat(32),
        "failed",
        serde_json::json!({"error_code":"fixture"}),
        1003,
    )
    .unwrap();
    assert_eq!(log.pending_events().len(), 10000);
    assert_eq!(log.state(&"a".repeat(32)), Some(ExecutionState::Failed));
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn claim_receipt_and_start_survive_reload_without_reexecution() {
    let directory =
        std::env::temp_dir().join(format!("rustdesk-journal-test-{}", std::process::id()));
    std::fs::create_dir(&directory).unwrap();
    let path = directory.join("journal.json");
    let mut log = CommandJournal::open(&path).unwrap();
    let id = "a".repeat(32);
    let digest = "b".repeat(64);
    log.prepare(&id, &digest, 2000, 1000).unwrap();
    assert_eq!(log.state(&id), Some(ExecutionState::ClaimPrepared));
    assert!(log.start(&id, 1001).is_err());
    log.receipt(&id).unwrap();
    let mut log = CommandJournal::open(&path).unwrap();
    log.start(&id, 1002).unwrap();
    let mut reloaded = CommandJournal::open(&path).unwrap();
    assert!(reloaded.start(&id, 1003).is_err());
    assert_eq!(reloaded.state(&id), Some(ExecutionState::Running));
    assert!(reloaded.prepare(&id, &"c".repeat(64), 2000, 1004).is_err());
    reloaded
        .finish(
            &id,
            "succeeded",
            serde_json::json!({"service":"running"}),
            1005,
        )
        .unwrap();
    let pending = reloaded.pending_events();
    assert_eq!(pending.len(), 3);
    let event = pending.last().unwrap().clone();
    assert_eq!(event.payload["status"], "succeeded");
    assert!(reloaded
        .finish(&id, "failed", serde_json::json!({}), 1006)
        .is_err());
    let mut reloaded = CommandJournal::open(&path).unwrap();
    assert_eq!(
        reloaded.pending_events().last().unwrap().event_id,
        event.event_id
    );
    assert!(reloaded
        .acknowledge(&event.event_id, &"0".repeat(64))
        .is_err());
    assert_eq!(reloaded.pending_events().len(), 3);
    reloaded
        .acknowledge(&event.event_id, &event.digest)
        .unwrap();
    assert_eq!(
        CommandJournal::open(&path).unwrap().pending_events().len(),
        2
    );
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn expiration_and_storage_failure_preserve_old_record() {
    let directory =
        std::env::temp_dir().join(format!("rustdesk-journal-failure-{}", std::process::id()));
    std::fs::create_dir(&directory).unwrap();
    let path = directory.join("journal.json");
    let id = "1".repeat(32);
    let mut log = CommandJournal::open(&path).unwrap();
    assert!(log.prepare(&id, &"2".repeat(64), 1000, 1000).is_err());
    log.prepare(&id, &"2".repeat(64), 1010, 1000).unwrap();
    log.receipt(&id).unwrap();
    assert!(log.start(&id, 1010).is_err());
    assert_eq!(log.state(&id), Some(ExecutionState::Expired));
    let before = std::fs::read(&path).unwrap();
    std::fs::create_dir(path.with_extension("pending")).unwrap();
    assert!(log
        .prepare(&"3".repeat(32), &"4".repeat(64), 2000, 1100)
        .is_err());
    assert_eq!(log.state(&"3".repeat(32)), None);
    assert_eq!(std::fs::read(&path).unwrap(), before);
    std::fs::remove_dir(path.with_extension("pending")).unwrap();
    std::fs::write(&path, b"{corrupt}").unwrap();
    assert!(CommandJournal::open(&path).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), b"{corrupt}");
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn different_commands_do_not_overlap_running_mutations_and_rejection_is_durable() {
    let directory =
        std::env::temp_dir().join(format!("rustdesk-journal-serial-{}", std::process::id()));
    std::fs::create_dir(&directory).unwrap();
    let path = directory.join("journal.json");
    let mut log = CommandJournal::open(&path).unwrap();
    for id in ["a".repeat(32), "b".repeat(32)] {
        log.prepare(&id, &"c".repeat(64), 2000, 1000).unwrap();
        log.receipt(&id).unwrap();
    }
    log.start(&"a".repeat(32), 1001).unwrap();
    assert!(log.start(&"b".repeat(32), 1002).is_err());
    log.reject(
        &"b".repeat(32),
        "unsupported",
        serde_json::json!({"error_code":"unsupported"}),
        1002,
    )
    .unwrap();
    assert_eq!(
        CommandJournal::open(&path).unwrap().state(&"b".repeat(32)),
        Some(ExecutionState::Unsupported)
    );
    assert!(log.start(&"b".repeat(32), 1003).is_err());
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn stale_writer_cannot_drop_new_receipts_or_events() {
    let directory =
        std::env::temp_dir().join(format!("rustdesk-journal-cas-{}", std::process::id()));
    std::fs::create_dir(&directory).unwrap();
    let path = directory.join("journal.json");
    let mut first = CommandJournal::open(&path).unwrap();
    let mut stale = CommandJournal::open(&path).unwrap();
    first
        .prepare(&"a".repeat(32), &"b".repeat(64), 2000, 1000)
        .unwrap();
    first.receipt(&"a".repeat(32)).unwrap();
    let before = std::fs::read(&path).unwrap();
    assert!(stale
        .prepare(&"c".repeat(32), &"d".repeat(64), 2000, 1001)
        .is_err());
    assert_eq!(std::fs::read(&path).unwrap(), before);
    assert_eq!(
        CommandJournal::open(&path).unwrap().pending_events().len(),
        1
    );
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn interrupted_staging_is_recovered_without_promoting_uncommitted_action() {
    let directory =
        std::env::temp_dir().join(format!("rustdesk-journal-interrupt-{}", std::process::id()));
    std::fs::create_dir(&directory).unwrap();
    let path = directory.join("journal.json");
    let id = "a".repeat(32);
    let mut log = CommandJournal::open(&path).unwrap();
    log.prepare(&id, &"b".repeat(64), 2000, 1000).unwrap();
    log.receipt(&id).unwrap();
    let before = std::fs::read(&path).unwrap();
    std::fs::write(path.with_extension("pending"), b"{partial-running-snapshot").unwrap();
    let mut restarted = CommandJournal::open(&path).unwrap();
    assert_eq!(restarted.state(&id), Some(ExecutionState::Accepted));
    restarted.start(&id, 1001).unwrap();
    assert!(!path.with_extension("pending").exists());
    assert_ne!(std::fs::read(&path).unwrap(), before);
    let mut restarted = CommandJournal::open(&path).unwrap();
    assert_eq!(restarted.state(&id), Some(ExecutionState::Running));
    std::fs::write(path.with_extension("pending"), b"{partial-result-snapshot").unwrap();
    assert!(restarted.start(&id, 1002).is_err());
    restarted
        .finish(
            &id,
            "reconciliation_required",
            serde_json::json!({"error_code":"interrupted"}),
            1002,
        )
        .unwrap();
    assert_eq!(
        CommandJournal::open(&path).unwrap().state(&id),
        Some(ExecutionState::ReconciliationRequired)
    );
    assert!(!path.with_extension("pending").exists());
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn cancellation_and_expiration_before_start_are_durable_and_do_not_block_next_command() {
    let directory =
        std::env::temp_dir().join(format!("rustdesk-journal-cancel-{}", std::process::id()));
    std::fs::create_dir(&directory).unwrap();
    let path = directory.join("journal.json");
    let mut log = CommandJournal::open(&path).unwrap();
    log.prepare(&"a".repeat(32), &"b".repeat(64), 2000, 1000)
        .unwrap();
    log.abandon(&"a".repeat(32), "cancelled", 1001).unwrap();
    assert_eq!(log.state(&"a".repeat(32)), Some(ExecutionState::Cancelled));
    assert!(log.start(&"a".repeat(32), 1002).is_err());
    log.prepare(&"c".repeat(32), &"d".repeat(64), 1010, 1000)
        .unwrap();
    log.receipt(&"c".repeat(32)).unwrap();
    log.abandon(&"c".repeat(32), "expired", 1010).unwrap();
    assert_eq!(log.state(&"c".repeat(32)), Some(ExecutionState::Expired));
    log.prepare(&"e".repeat(32), &"f".repeat(64), 2000, 1011)
        .unwrap();
    log.receipt(&"e".repeat(32)).unwrap();
    log.start(&"e".repeat(32), 1011).unwrap();
    assert!(log.abandon(&"e".repeat(32), "expired", 2001).is_err());
    assert_eq!(
        CommandJournal::open(&path).unwrap().state(&"e".repeat(32)),
        Some(ExecutionState::Running)
    );
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn retention_keeps_unacknowledged_results_and_active_operations() {
    let directory =
        std::env::temp_dir().join(format!("rustdesk-journal-retention-{}", std::process::id()));
    std::fs::create_dir(&directory).unwrap();
    let path = directory.join("journal.json");
    let mut log = CommandJournal::open(&path).unwrap();
    for id in ["a".repeat(32), "b".repeat(32)] {
        log.prepare(&id, &"c".repeat(64), 2000, 1000).unwrap();
        log.receipt(&id).unwrap();
        log.start(&id, 1001).unwrap();
        log.finish(
            &id,
            "succeeded",
            serde_json::json!({"service":"running"}),
            1002,
        )
        .unwrap();
    }
    let events = log
        .pending_events()
        .iter()
        .filter(|event| event.payload["command_id"] == "a".repeat(32))
        .cloned()
        .collect::<Vec<_>>();
    for event in events {
        log.acknowledge(&event.event_id, &event.digest).unwrap();
    }
    log.prepare(&"d".repeat(32), &"e".repeat(64), 2000, 1000)
        .unwrap();
    log.receipt(&"d".repeat(32)).unwrap();
    log.start(&"d".repeat(32), 1001).unwrap();
    let deadline = 1002 + 30 * 86400;
    assert_eq!(log.prune_confirmed(deadline - 1).unwrap(), 0);
    assert_eq!(log.prune_confirmed(deadline).unwrap(), 1);
    assert_eq!(log.state(&"a".repeat(32)), None);
    assert_eq!(log.state(&"b".repeat(32)), Some(ExecutionState::Succeeded));
    assert_eq!(log.state(&"d".repeat(32)), Some(ExecutionState::Running));
    assert_eq!(log.pending_events().len(), 5);
    assert!(log
        .prepare(&"a".repeat(32), &"c".repeat(64), 2000, deadline)
        .is_err());
    assert_eq!(
        CommandJournal::open(&path).unwrap().state(&"d".repeat(32)),
        Some(ExecutionState::Running)
    );
    std::fs::remove_dir_all(directory).unwrap();
}

#[cfg(windows)]
#[test]
fn exclusive_writer_lock_preserves_staging_and_committed_state() {
    use std::os::windows::fs::OpenOptionsExt;
    let directory =
        std::env::temp_dir().join(format!("rustdesk-journal-locked-{}", std::process::id()));
    std::fs::create_dir(&directory).unwrap();
    let path = directory.join("journal.json");
    let mut log = CommandJournal::open(&path).unwrap();
    log.prepare(&"a".repeat(32), &"b".repeat(64), 2000, 1000)
        .unwrap();
    let before = std::fs::read(&path).unwrap();
    let lock = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .share_mode(0)
        .open(path.with_extension("lock"))
        .unwrap();
    std::fs::write(path.with_extension("pending"), b"active writer staging").unwrap();
    assert!(log.receipt(&"a".repeat(32)).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), before);
    assert_eq!(
        std::fs::read(path.with_extension("pending")).unwrap(),
        b"active writer staging"
    );
    drop(lock);
    log.receipt(&"a".repeat(32)).unwrap();
    assert_eq!(log.state(&"a".repeat(32)), Some(ExecutionState::Accepted));
    std::fs::remove_dir_all(directory).unwrap();
}
