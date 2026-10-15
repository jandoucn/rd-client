#![cfg(windows)]

#[path = "../src/management_agent/identity.rs"]
mod identity;
#[path = "../src/management_agent/local_event_logic.rs"]
mod local_event_runtime;
#[path = "../src/management_agent/local_events.rs"]
mod local_events;
#[path = "../src/management_agent/operations.rs"]
mod operations;
#[path = "../src/management_agent/state_store.rs"]
mod state_store;
#[path = "../src/management_agent/workspace.rs"]
mod workspace;

use local_events::{
    Action, ErrorCode, EventError, LocalEvents, Operation, Outcome, PathKey, Phase, Record,
    RecoveryCode, MAX_ERROR_INPUT, MAX_EVENTS, MAX_OPERATIONS, MAX_RECORD_BYTES, MAX_STORE_BYTES,
};
use sha2::{Digest, Sha256};
use std::{
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};

static SERIAL: AtomicUsize = AtomicUsize::new(0);

fn apply_r1_result(previous: &str, result: local_event_runtime::RecoveryResult) -> String {
    let mut error = previous.to_owned();
    match result.action {
        Ok(true) => error.clear(),
        Ok(false) => {}
        Err(action_error) => error = action_error,
    }
    local_event_runtime::format_recovery_error(&error, result.reporting)
}

#[test]
fn reporting_p2_unbound_failure_keeps_action_error() {
    let f = Fixture::new();
    let result = local_event_runtime::r1(
        &f.path(),
        None,
        1000,
        || panic!("offline old path"),
        || Err("service start: win32 5".into()),
    );
    assert_eq!(result.reporting, "unreported");
    assert_eq!(
        apply_r1_result("old", result),
        "service start: win32 5; local_events_unreported"
    );
    assert!(!f.path().exists());
}

#[test]
fn reporting_p2_capacity_failure_keeps_action_error_and_store() {
    let f = Fixture::new();
    let mut log = f.open();
    for index in 0..MAX_OPERATIONS {
        let op = operation(index, RecoveryCode::R1);
        begun(&mut log, &op);
        log.record(&op, record(Phase::Terminal, 1, Outcome::Succeeded))
            .unwrap();
        for e in log.pending_events().to_vec() {
            log.acknowledge(&e.event_id, &e.digest).unwrap();
        }
    }
    let before = f.bytes();
    let result = local_event_runtime::r1(
        &f.path(),
        Some(&binding(1)),
        1000,
        || Ok(local_event_runtime::Observation::Stopped),
        || Err("service start: win32 1058".into()),
    );
    assert_eq!(result.reporting, "degraded_capacity");
    assert_eq!(
        apply_r1_result("old", result),
        "service start: win32 1058; local_events_degraded_capacity"
    );
    assert_eq!(f.bytes(), before);
}

#[test]
fn reporting_p2_storage_failure_keeps_writer_busy_and_store() {
    use std::os::windows::fs::OpenOptionsExt;
    let f = Fixture::new();
    let mut log = f.open();
    log.remember_binding(binding(1)).unwrap();
    let before = f.bytes();
    let _lock = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .share_mode(0)
        .open(f.path().with_extension("lock"))
        .unwrap();
    let result = local_event_runtime::r1(
        &f.path(),
        Some(&binding(1)),
        1000,
        || Ok(local_event_runtime::Observation::Stopped),
        || panic!("storage failure must not execute"),
    );
    assert_eq!(result.action, Err("local event WriterBusy".into()));
    assert_eq!(
        apply_r1_result("old", result),
        "local event WriterBusy; local_events_degraded"
    );
    assert_eq!(f.bytes(), before);
}

#[test]
fn reporting_p2_success_clears_prior_failure_and_retains_missing_report_marker() {
    let f = Fixture::new();
    let result = local_event_runtime::r1(
        &f.path(),
        Some(&binding(1)),
        1000,
        || Ok(local_event_runtime::Observation::Stopped),
        || Ok(true),
    );
    assert_eq!(
        apply_r1_result("old failure; local_events_degraded", result),
        ""
    );
    let g = Fixture::new();
    let result = local_event_runtime::r1(
        &g.path(),
        None,
        1000,
        || panic!("offline old path"),
        || Ok(true),
    );
    assert_eq!(
        apply_r1_result("old failure", result),
        "local_events_unreported"
    );
}

#[test]
fn reporting_p2_repetition_and_untrusted_reporting_never_grow_or_erase_original() {
    use local_event_runtime::{format_recovery_error, RecoveryResult};
    let f = Fixture::new();
    let original = format!("旧错误 {}", "x".repeat(8192));
    let mut state = original.clone();
    for _ in 0..100 {
        let result = local_event_runtime::r1(
            &f.path(),
            None,
            1000,
            || panic!("offline old path"),
            || Ok(false),
        );
        state = apply_r1_result(&state, result);
        assert_eq!(state, format!("{original}; local_events_unreported"));
    }
    for reporting in ["degraded", "degraded_capacity", "unreported"] {
        state = apply_r1_result(
            &state,
            RecoveryResult {
                action: Ok(false),
                reporting,
            },
        );
        assert_eq!(state, format!("{original}; local_events_{reporting}"));
    }
    assert_eq!(
        format_recovery_error(&state, "arbitrary secret payload"),
        state
    );
    assert_eq!(format_recovery_error("", "pending"), "");
    assert_eq!(
        format_recovery_error("original; local_events_degraded inside", "degraded"),
        "original; local_events_degraded inside; local_events_degraded"
    );
    assert_eq!(
        format_recovery_error("local_events_unreported", "degraded"),
        "local_events_degraded"
    );
}

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        assert_eq!(
            std::env::var("RUSTDESK_LOCAL_EVENTS_VM").as_deref(),
            Ok("1"),
            "only the main agent's scheduled VM driver may execute these tests"
        );
        assert!(matches!(
            std::env::var("COMPUTERNAME").unwrap().as_str(),
            "RDLAB-WIN10" | "RDLAB-WIN11"
        ));
        let base = PathBuf::from(std::env::var_os("RUSTDESK_LOCAL_EVENTS_ROOT").unwrap());
        assert!(base.is_absolute());
        let root = base.join(format!(
            "case-{}-{}",
            std::process::id(),
            SERIAL.fetch_add(1, Ordering::SeqCst)
        ));
        workspace::create(&root).unwrap();
        Self(root)
    }

    fn path(&self) -> PathBuf {
        self.0.join("local-events.json")
    }
    fn open(&self) -> LocalEvents {
        LocalEvents::open(&self.path()).unwrap()
    }
    fn bytes(&self) -> Vec<u8> {
        std::fs::read(self.path()).unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

fn operation(index: usize, code: RecoveryCode) -> Operation {
    Operation {
        operation_id: format!("{index:032x}"),
        code,
        action: match code {
            RecoveryCode::R1 => Action::StartService,
            RecoveryCode::R3 => Action::RestoreFallback,
            RecoveryCode::R4 => Action::RepairInstallation,
        },
    }
}

fn record(phase: Phase, attempt_seq: u32, outcome: Outcome) -> Record {
    Record {
        phase,
        attempt_seq,
        outcome,
        path_key: PathKey::RustDeskService,
        error_code: None,
    }
}

fn begun(log: &mut LocalEvents, op: &Operation) {
    log.record(op, record(Phase::Started, 1, Outcome::Running))
        .unwrap();
}

fn assert_unchanged(
    f: &Fixture,
    log: &LocalEvents,
    disk: &[u8],
    memory: &[local_events::PendingEvent],
) {
    assert_eq!(f.bytes(), disk, "failed mutation must retain the preimage");
    assert_eq!(
        log.pending_events(),
        memory,
        "failed mutation must retain memory"
    );
    assert_eq!(
        f.open().pending_events(),
        memory,
        "restart must retain the preimage"
    );
}

#[test]
fn local_r1_r3_r4_persist_without_remote_command_and_reopen_for_replay() {
    let f = Fixture::new();
    let mut log = f.open();
    for (index, code) in [RecoveryCode::R1, RecoveryCode::R3, RecoveryCode::R4]
        .into_iter()
        .enumerate()
    {
        let op = operation(index, code);
        begun(&mut log, &op);
        log.record(&op, record(Phase::Terminal, 1, Outcome::Succeeded))
            .unwrap();
    }
    let pending = log.pending_events().to_vec();
    assert_eq!(pending.len(), 6);
    assert!(pending
        .iter()
        .all(|event| event.payload["command_id"].is_null()));
    drop(log);
    assert_eq!(f.open().pending_events(), pending);
}

#[test]
fn stable_id_binds_operation_phase_attempt_and_exact_retries_keep_bytes() {
    let f = Fixture::new();
    let op = operation(1, RecoveryCode::R1);
    let input = record(Phase::Started, 1, Outcome::Running);
    let mut log = f.open();
    let first = log.record(&op, input).unwrap();
    let disk = f.bytes();
    let expected = hex::encode(Sha256::digest(format!(
        "agent-local-event-v1:{}:started:1",
        op.operation_id
    )))[..32]
        .to_owned();
    assert_eq!(first.event_id, expected);
    assert_eq!(f.open().record(&op, input).unwrap(), first);
    assert_eq!(f.bytes(), disk);
    assert_eq!(log.record(&op, input).unwrap(), first);
    assert_eq!(f.bytes(), disk);
}

#[test]
fn attempt_sequence_and_phase_are_distinct_monotonic_events() {
    let f = Fixture::new();
    let op = operation(2, RecoveryCode::R3);
    let mut log = f.open();
    begun(&mut log, &op);
    log.record(&op, record(Phase::Progress, 1, Outcome::Running))
        .unwrap();
    log.record(&op, record(Phase::Progress, 2, Outcome::Running))
        .unwrap();
    log.record(&op, record(Phase::Terminal, 2, Outcome::Succeeded))
        .unwrap();
    let events = log.pending_events();
    let ids = events
        .iter()
        .map(|event| &event.event_id)
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(ids.len(), 4);
    for (index, event) in events.iter().enumerate() {
        assert_eq!(event.payload["sequence"], (index + 1) as u64);
    }
    assert_eq!(events[2].payload["result"]["attempt_seq"], 2);
}

#[test]
fn same_event_key_with_different_outcome_is_rejected_without_data_loss() {
    let f = Fixture::new();
    let op = operation(3, RecoveryCode::R4);
    let mut log = f.open();
    begun(&mut log, &op);
    log.record(&op, record(Phase::Terminal, 1, Outcome::Failed))
        .unwrap();
    let disk = f.bytes();
    let memory = log.pending_events().to_vec();
    assert_eq!(
        log.record(&op, record(Phase::Terminal, 1, Outcome::Succeeded)),
        Err(EventError::Conflict)
    );
    assert_unchanged(&f, &log, &disk, &memory);
}

#[test]
fn operation_identity_cannot_be_rebound_to_another_recovery_action() {
    let f = Fixture::new();
    let mut log = f.open();
    let op = operation(4, RecoveryCode::R1);
    begun(&mut log, &op);
    let disk = f.bytes();
    let memory = log.pending_events().to_vec();
    assert_eq!(
        log.record(
            &operation(4, RecoveryCode::R3),
            record(Phase::Started, 1, Outcome::Running)
        ),
        Err(EventError::Conflict)
    );
    assert_unchanged(&f, &log, &disk, &memory);
}

#[test]
fn invalid_sequence_and_phase_outcome_pairs_preserve_previous_state() {
    let f = Fixture::new();
    let op = operation(5, RecoveryCode::R1);
    let mut log = f.open();
    begun(&mut log, &op);
    log.record(&op, record(Phase::Progress, 2, Outcome::Running))
        .unwrap();
    let disk = f.bytes();
    let memory = log.pending_events().to_vec();
    for input in [
        record(Phase::Progress, 0, Outcome::Running),
        record(Phase::Progress, 1, Outcome::Running),
        record(Phase::Terminal, 2, Outcome::Running),
        record(Phase::Progress, 3, Outcome::Succeeded),
    ] {
        assert_eq!(log.record(&op, input), Err(EventError::InvalidRecord));
        assert_unchanged(&f, &log, &disk, &memory);
    }
}

#[test]
fn bounded_outbox_reserves_terminal_and_never_evicts_unacked_events() {
    let f = Fixture::new();
    let op = operation(6, RecoveryCode::R3);
    let mut log = f.open();
    begun(&mut log, &op);
    let mut refused = false;
    for attempt in 1..=MAX_EVENTS as u32 + 1 {
        let disk = f.bytes();
        let memory = log.pending_events().to_vec();
        match log.record(&op, record(Phase::Progress, attempt, Outcome::Running)) {
            Ok(_) => {}
            Err(error) => {
                assert_eq!(error, EventError::Full);
                assert_unchanged(&f, &log, &disk, &memory);
                refused = true;
                break;
            }
        }
    }
    assert!(refused);
    let before = log.pending_events().to_vec();
    assert_eq!(
        log.record(
            &operation(7, RecoveryCode::R1),
            record(Phase::Started, 1, Outcome::Running)
        ),
        Err(EventError::Full)
    );
    log.record(
        &op,
        record(Phase::Terminal, MAX_EVENTS as u32 + 1, Outcome::Failed),
    )
    .unwrap();
    assert_eq!(&log.pending_events()[..before.len()], before);
    assert!(log.pending_events().len() <= MAX_EVENTS);
    assert!(f.bytes().len() <= MAX_STORE_BYTES);
    assert_eq!(
        f.open().pending_events().last().unwrap().payload["status"],
        "failed"
    );
}

#[test]
fn wrong_unknown_and_duplicate_ack_are_rejected_and_preserve_pending() {
    let f = Fixture::new();
    let mut log = f.open();
    begun(&mut log, &operation(8, RecoveryCode::R4));
    let event = log.pending_events()[0].clone();
    let disk = f.bytes();
    let memory = log.pending_events().to_vec();
    assert_eq!(
        log.acknowledge(&event.event_id, &"0".repeat(64)),
        Err(EventError::AckDigest)
    );
    assert_eq!(
        log.acknowledge(&"f".repeat(32), &event.digest),
        Err(EventError::AckUnknown)
    );
    assert_unchanged(&f, &log, &disk, &memory);
    log.acknowledge(&event.event_id, &event.digest).unwrap();
    let disk = f.bytes();
    assert!(f.open().pending_events().is_empty());
    assert_eq!(
        log.acknowledge(&event.event_id, &event.digest),
        Err(EventError::AckUnknown)
    );
    assert_unchanged(&f, &log, &disk, &[]);
}

#[test]
fn lost_transport_ack_replays_same_bytes_without_recreating_the_event() {
    let f = Fixture::new();
    let mut log = f.open();
    begun(&mut log, &operation(9, RecoveryCode::R1));
    let first = log.pending_events()[0].clone();
    let disk = f.bytes();
    drop(log);
    let mut restarted = f.open();
    assert_eq!(restarted.pending_events()[0], first);
    assert_eq!(f.bytes(), disk);
    restarted
        .acknowledge(&first.event_id, &first.digest)
        .unwrap();
    assert!(f.open().pending_events().is_empty());
}

#[test]
fn stale_writer_record_and_ack_cannot_drop_another_writers_events() {
    let f = Fixture::new();
    let mut first = f.open();
    let op = operation(10, RecoveryCode::R3);
    begun(&mut first, &op);
    let mut stale = f.open();
    let prior = stale.pending_events().to_vec();
    first
        .record(&op, record(Phase::Progress, 1, Outcome::Running))
        .unwrap();
    let disk = f.bytes();
    assert_eq!(
        stale.acknowledge(&prior[0].event_id, &prior[0].digest),
        Err(EventError::StaleWriter)
    );
    assert_eq!(
        stale.record(&op, record(Phase::Terminal, 1, Outcome::Failed)),
        Err(EventError::StaleWriter)
    );
    assert_eq!(stale.pending_events(), prior);
    assert_eq!(f.bytes(), disk);
    assert_eq!(f.open().pending_events(), first.pending_events());
}

#[test]
fn writer_lock_failure_preserves_disk_memory_and_replay() {
    use std::os::windows::fs::OpenOptionsExt;
    let f = Fixture::new();
    let mut log = f.open();
    let op = operation(11, RecoveryCode::R4);
    begun(&mut log, &op);
    let disk = f.bytes();
    let memory = log.pending_events().to_vec();
    let lock = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .share_mode(0)
        .open(f.path().with_extension("lock"))
        .unwrap();
    assert_eq!(
        log.record(&op, record(Phase::Terminal, 1, Outcome::Failed)),
        Err(EventError::WriterBusy)
    );
    assert_eq!(
        log.acknowledge(&memory[0].event_id, &memory[0].digest),
        Err(EventError::WriterBusy)
    );
    assert_unchanged(&f, &log, &disk, &memory);
    drop(lock);
    log.record(&op, record(Phase::Terminal, 1, Outcome::Failed))
        .unwrap();
}

#[test]
fn disk_full_at_publish_preserves_preimage_for_record_and_ack() {
    let f = Fixture::new();
    let mut log = f.open();
    let op = operation(12, RecoveryCode::R3);
    begun(&mut log, &op);
    let disk = f.bytes();
    let memory = log.pending_events().to_vec();
    // Only the OS publish failure is injected; serialization/lock/CAS must be real.
    log.fail_next_publish_for_test(112);
    assert_eq!(
        log.record(&op, record(Phase::Terminal, 1, Outcome::Failed)),
        Err(EventError::DiskFull)
    );
    assert_unchanged(&f, &log, &disk, &memory);
    log.fail_next_publish_for_test(112);
    assert_eq!(
        log.acknowledge(&memory[0].event_id, &memory[0].digest),
        Err(EventError::DiskFull)
    );
    assert_unchanged(&f, &log, &disk, &memory);
    log.record(&op, record(Phase::Terminal, 1, Outcome::Failed))
        .unwrap();
}

#[test]
fn actual_state_store_staging_failure_retains_the_old_file() {
    let f = Fixture::new();
    let mut log = f.open();
    let op = operation(13, RecoveryCode::R1);
    begun(&mut log, &op);
    let disk = f.bytes();
    let memory = log.pending_events().to_vec();
    let staging = f
        .path()
        .with_extension(format!("tmp-{}", std::process::id()));
    std::fs::create_dir(&staging).unwrap();
    assert_eq!(
        log.record(&op, record(Phase::Terminal, 1, Outcome::Failed)),
        Err(EventError::Storage)
    );
    assert_unchanged(&f, &log, &disk, &memory);
    std::fs::remove_dir(staging).unwrap();
}

#[test]
fn running_and_terminal_receipts_survive_restart_and_ack_without_action_reexecution() {
    let f = Fixture::new();
    let op = operation(14, RecoveryCode::R4);
    let mut log = f.open();
    begun(&mut log, &op);
    let mut restarted = f.open();
    assert!(restarted.operation_seen(&op.operation_id));
    let disk = f.bytes();
    begun(&mut restarted, &op);
    assert_eq!(f.bytes(), disk);
    restarted
        .record(&op, record(Phase::Terminal, 1, Outcome::Succeeded))
        .unwrap();
    for event in restarted.pending_events().to_vec() {
        restarted
            .acknowledge(&event.event_id, &event.digest)
            .unwrap();
    }
    let mut restarted = f.open();
    assert!(restarted.operation_seen(&op.operation_id));
    assert!(restarted.pending_events().is_empty());
    let disk = f.bytes();
    // Idempotent observation is permitted; it grants no permission to execute an action.
    begun(&mut restarted, &op);
    assert_eq!(f.bytes(), disk);
    assert!(restarted.pending_events().is_empty());
    assert_eq!(
        restarted.record(&op, record(Phase::Started, 2, Outcome::Running)),
        Err(EventError::Terminal)
    );
    assert_eq!(f.bytes(), disk);
}

#[test]
fn receipt_capacity_refuses_new_operations_instead_of_forgetting_completed_actions() {
    let f = Fixture::new();
    let mut log = f.open();
    for index in 0..MAX_OPERATIONS {
        let op = operation(index, RecoveryCode::R1);
        begun(&mut log, &op);
        log.record(&op, record(Phase::Terminal, 1, Outcome::Succeeded))
            .unwrap();
        for event in log.pending_events().to_vec() {
            log.acknowledge(&event.event_id, &event.digest).unwrap();
        }
    }
    let disk = f.bytes();
    assert_eq!(
        log.record(
            &operation(MAX_OPERATIONS, RecoveryCode::R1),
            record(Phase::Started, 1, Outcome::Running)
        ),
        Err(EventError::Full)
    );
    assert_unchanged(&f, &log, &disk, &[]);
    assert!(f
        .open()
        .operation_seen(&operation(0, RecoveryCode::R1).operation_id));
}

#[test]
fn malformed_or_oversized_store_is_refused_without_reset() {
    let f = Fixture::new();
    for bytes in [b"{broken}".to_vec(), vec![b' '; MAX_STORE_BYTES + 1]] {
        state_store::publish(&f.path(), &bytes).unwrap();
        let expected = if bytes.len() > MAX_STORE_BYTES {
            EventError::Full
        } else {
            EventError::Corrupt
        };
        assert_eq!(LocalEvents::open(&f.path()).err(), Some(expected));
        assert_eq!(f.bytes(), bytes);
    }
}

#[test]
fn tampered_event_digest_is_rejected_without_repairing_or_discarding_it() {
    let f = Fixture::new();
    let mut log = f.open();
    begun(&mut log, &operation(15, RecoveryCode::R1));
    let mut data: serde_json::Value = serde_json::from_slice(&f.bytes()).unwrap();
    data["events"][0]["digest"] = "0".repeat(64).into();
    let bytes = serde_json::to_vec(&data).unwrap();
    state_store::publish(&f.path(), &bytes).unwrap();
    assert_eq!(
        LocalEvents::open(&f.path()).err(),
        Some(EventError::Corrupt)
    );
    assert_eq!(f.bytes(), bytes);
}

#[test]
fn wire_uses_existing_nullable_command_schema_and_only_bounded_record_fields() {
    let f = Fixture::new();
    let op = operation(16, RecoveryCode::R3);
    let mut log = f.open();
    begun(&mut log, &op);
    let mut input = record(Phase::Terminal, 1, Outcome::Failed);
    input.error_code = Some(ErrorCode::DiskFull);
    input.path_key = PathKey::RescueClient;
    let event = log.record(&op, input).unwrap();
    assert_eq!(
        event.payload,
        serde_json::json!({
            "schema": 1, "event_id": event.event_id, "command_id": null, "sequence": 2,
            "status": "failed", "result": {"operation_id": op.operation_id, "code": "r3",
            "action": "restore_fallback", "phase": "terminal", "attempt_seq": 1,
            "outcome": "failed", "path_key": "rescue_client", "error_code": "disk_full"}
        })
    );
    let bytes = serde_json::to_vec(&event.payload).unwrap();
    assert!(bytes.len() <= MAX_RECORD_BYTES);
    assert_eq!(event.digest, hex::encode(Sha256::digest(bytes)));
}

#[test]
fn arbitrary_payload_paths_errors_and_overlong_ids_are_rejected_or_redacted() {
    let f = Fixture::new();
    let mut log = f.open();
    begun(&mut log, &operation(17, RecoveryCode::R1));
    let disk = f.bytes();
    let memory = log.pending_events().to_vec();
    for id in [
        "../secret".to_owned(),
        "a".repeat(33),
        "A".repeat(32),
        "a".repeat(65536),
    ] {
        let mut op = operation(18, RecoveryCode::R1);
        op.operation_id = id;
        assert_eq!(
            log.record(&op, record(Phase::Started, 1, Outcome::Running)),
            Err(EventError::InvalidRecord)
        );
        assert_unchanged(&f, &log, &disk, &memory);
    }
    let safe = serde_json::to_value(record(Phase::Terminal, 1, Outcome::Failed)).unwrap();
    for (key, value) in [
        ("payload", serde_json::json!({"password":"secret"})),
        ("path_key", serde_json::json!("C:\\Users\\secret")),
        (
            "error_code",
            serde_json::json!("Authorization: Bearer secret"),
        ),
    ] {
        let mut raw = safe.clone();
        raw[key] = value;
        assert!(serde_json::from_value::<Record>(raw).is_err());
    }
    for text in [
        "Authorization: Bearer secret",
        "password=secret",
        "C:\\Users\\private\\config.toml",
    ] {
        assert_eq!(local_events::redact_error(text), Ok(ErrorCode::Redacted));
    }
    assert_eq!(
        local_events::redact_error(&"密".repeat(MAX_ERROR_INPUT)),
        Err(EventError::InvalidRecord)
    );
    assert_eq!(
        local_events::redact_error("disk_full"),
        Ok(ErrorCode::DiskFull)
    );
    assert_unchanged(&f, &log, &disk, &memory);
}

#[test]
fn non_workspace_or_lock_directory_is_rejected_without_overwriting_evidence() {
    let f = Fixture::new();
    let path = f.path();
    std::fs::create_dir(path.with_extension("lock")).unwrap();
    let result = LocalEvents::open(&path).and_then(|mut log| {
        log.record(
            &operation(19, RecoveryCode::R1),
            record(Phase::Started, 1, Outcome::Running),
        )
    });
    assert_eq!(result, Err(EventError::UnsafePath));
    assert!(!path.exists());
    assert!(path.with_extension("lock").is_dir());
    assert_eq!(
        LocalEvents::open(std::path::Path::new("relative.json")).err(),
        Some(EventError::UnsafePath)
    );
}

#[test]
fn recovery_action_mismatch_and_unstarted_terminal_do_not_create_a_receipt() {
    let f = Fixture::new();
    let mut log = f.open();
    let mut op = operation(20, RecoveryCode::R1);
    op.action = Action::RepairInstallation;
    assert_eq!(
        log.record(&op, record(Phase::Started, 1, Outcome::Running)),
        Err(EventError::InvalidRecord)
    );
    assert_eq!(
        log.record(
            &operation(21, RecoveryCode::R3),
            record(Phase::Terminal, 1, Outcome::Succeeded)
        ),
        Err(EventError::InvalidRecord)
    );
    assert!(!log.operation_seen(&op.operation_id));
    assert!(log.pending_events().is_empty());
    assert!(!f.path().exists());
}

#[test]
fn protected_workspace_is_required_and_reparse_store_is_not_followed() {
    let f = Fixture::new();
    let ordinary = f.0.join("unprotected");
    std::fs::create_dir(&ordinary).unwrap();
    assert_eq!(
        LocalEvents::open(&ordinary.join("local-events.json")).err(),
        Some(EventError::UnsafePath)
    );
    let target = f.0.join("unrelated.json");
    std::fs::write(&target, b"retain unrelated bytes").unwrap();
    std::os::windows::fs::symlink_file(&target, f.path()).unwrap();
    assert_eq!(
        LocalEvents::open(&f.path()).err(),
        Some(EventError::UnsafePath)
    );
    assert_eq!(std::fs::read(target).unwrap(), b"retain unrelated bytes");
    assert!(std::fs::symlink_metadata(f.path())
        .unwrap()
        .file_type()
        .is_symlink());
}

fn binding(generation: u64) -> local_events::Binding {
    local_events::Binding {
        client_id: "test-device".into(),
        client_uuid: "test-uuid".into(),
        agent_installation_id: "a".repeat(32),
        enrollment_generation: generation,
    }
}

#[test]
fn queued_events_keep_original_binding_after_verified_generation_changes() {
    let f = Fixture::new();
    let mut log = f.open();
    log.remember_binding(binding(1)).unwrap();
    let op = log.begin_r1(&binding(1), 1000).unwrap();
    let before = log.pending_events().to_vec();
    log.remember_binding(binding(2)).unwrap();
    let mut log = f.open();
    assert_eq!(log.pending_events(), before);
    assert_eq!(log.pending_events()[0].payload["enrollment_generation"], 1);
    log.record_bound_at(
        &op,
        record(Phase::Terminal, 1, Outcome::Succeeded),
        &binding(2),
        1001,
    )
    .unwrap_err();
    log.record_bound_at(
        &op,
        record(Phase::Terminal, 1, Outcome::Succeeded),
        &binding(1),
        1001,
    )
    .unwrap();
    assert_eq!(log.pending_events()[1].payload["enrollment_generation"], 1);
    assert_eq!(log.verified_binding(), Some(&binding(2)));
}

#[test]
fn prune_only_acked_terminal_older_than_thirty_days_and_ids_never_reuse() {
    let f = Fixture::new();
    let mut log = f.open();
    let op = log.begin_r1(&binding(1), 1000).unwrap();
    log.record_bound_at(
        &op,
        record(Phase::Terminal, 1, Outcome::Succeeded),
        &binding(1),
        1001,
    )
    .unwrap();
    assert_eq!(log.prune_confirmed(1001 + 30 * 86400).unwrap(), 0);
    for e in log.pending_events().to_vec() {
        log.acknowledge(&e.event_id, &e.digest).unwrap();
    }
    assert_eq!(log.prune_confirmed(1000 + 30 * 86400).unwrap(), 0);
    assert_eq!(log.prune_confirmed(999).unwrap(), 0);
    assert_eq!(log.prune_confirmed(1001 + 30 * 86400).unwrap(), 1);
    let next = f.open().begin_r1(&binding(1), 1001 + 30 * 86400).unwrap();
    assert_ne!(op.operation_id, next.operation_id);
    assert_eq!(f.open().active_r1().unwrap().0, next);
    assert_eq!(log.prune_confirmed(u64::MAX), Ok(0));
}

#[test]
fn crash_started_reuses_operation_and_rejects_binding_changes_without_overwrite() {
    let f = Fixture::new();
    let mut log = f.open();
    let op = log.begin_r1(&binding(1), 1000).unwrap();
    let bytes = f.bytes();
    assert_eq!(f.open().begin_r1(&binding(1), 1002).unwrap(), op);
    assert_eq!(f.bytes(), bytes);
    assert_eq!(log.begin_r1(&binding(2), 1002), Err(EventError::Conflict));
    assert_eq!(f.bytes(), bytes);
    let mut invalid = binding(1);
    invalid.enrollment_generation = 0;
    assert_eq!(
        log.remember_binding(invalid),
        Err(EventError::InvalidRecord)
    );
    assert_eq!(f.bytes(), bytes);
}

#[test]
fn r4_wire_means_repair_installation_not_agent_restore() {
    let value = serde_json::from_str::<Action>("\"repair_installation\"").unwrap();
    assert_eq!(
        serde_json::to_string(&value).unwrap(),
        "\"repair_installation\""
    );
    assert!(serde_json::from_str::<Action>("\"restore_agent\"").is_err());
}

#[test]
fn r1_started_is_durable_before_action_and_no_binding_retains_offline_start() {
    use local_event_runtime::{r1, Observation};
    let f = Fixture::new();
    let out = r1(
        &f.path(),
        Some(&binding(1)),
        1000,
        || Ok(Observation::Stopped),
        || {
            assert!(f.open().active_r1().is_some());
            assert_eq!(f.open().pending_events()[0].payload["status"], "running");
            Ok(true)
        },
    );
    assert_eq!(out.action, Ok(true));
    assert_eq!(out.reporting, "pending");
    assert!(f.open().active_r1().is_none());
    assert_eq!(f.open().pending_events().len(), 2);
    let g = Fixture::new();
    let mut called = false;
    let out = r1(
        &g.path(),
        None,
        1001,
        || Ok(Observation::Stopped),
        || {
            called = true;
            Ok(true)
        },
    );
    assert!(called);
    assert_eq!(out.action, Ok(true));
    assert_eq!(out.reporting, "unreported");
    assert!(!g.path().exists());
}

#[test]
fn r1_crash_unknown_does_not_reexecute_and_running_reconciles_original_operation() {
    use local_event_runtime::{r1, Observation};
    let f = Fixture::new();
    let op = f.open().begin_r1(&binding(1), 1000).unwrap();
    let before = f.bytes();
    let out = r1(
        &f.path(),
        Some(&binding(2)),
        1001,
        || Ok(Observation::Unknown),
        || panic!("unknown must not restart"),
    );
    assert!(out.action.is_err());
    assert_eq!(f.bytes(), before);
    let out = r1(
        &f.path(),
        Some(&binding(2)),
        1002,
        || Ok(Observation::Running),
        || panic!("running must only observe"),
    );
    assert_eq!(out.action, Ok(false));
    let log = f.open();
    assert_eq!(
        log.pending_events()[1].payload["result"]["operation_id"],
        op.operation_id
    );
    assert_eq!(log.pending_events()[1].payload["enrollment_generation"], 1);
}

#[test]
fn r1_writer_failure_prevents_side_effect_but_capacity_degrades_to_old_start() {
    use local_event_runtime::{r1, Observation};
    use std::os::windows::fs::OpenOptionsExt;
    let f = Fixture::new();
    let lock = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .share_mode(0)
        .open(f.path().with_extension("lock"))
        .unwrap();
    let out = r1(
        &f.path(),
        Some(&binding(1)),
        1000,
        || Ok(Observation::Stopped),
        || panic!("not persisted"),
    );
    assert!(out.action.is_err());
    assert!(!f.path().exists());
    drop(lock);
    let mut log = f.open();
    for index in 0..MAX_OPERATIONS {
        let op = operation(index, RecoveryCode::R1);
        begun(&mut log, &op);
        log.record(&op, record(Phase::Terminal, 1, Outcome::Succeeded))
            .unwrap();
        for e in log.pending_events().to_vec() {
            log.acknowledge(&e.event_id, &e.digest).unwrap();
        }
    }
    let before = f.bytes();
    let mut called = false;
    let out = r1(
        &f.path(),
        Some(&binding(1)),
        1001,
        || Ok(Observation::Stopped),
        || {
            called = true;
            Ok(true)
        },
    );
    assert!(called);
    assert_eq!(out.reporting, "degraded_capacity");
    assert_eq!(f.bytes(), before);
}

#[test]
fn uploader_never_rebinds_old_queue_and_ack_drop_replays_same_body() {
    let f = Fixture::new();
    let mut log = f.open();
    log.begin_r1(&binding(1), 1000).unwrap();
    let before = f.bytes();
    assert!(
        local_event_runtime::upload(&mut log, &binding(2), |_| panic!("must not rebind")).is_err()
    );
    assert_eq!(f.bytes(), before);
    let mut sent = None;
    assert!(local_event_runtime::upload(&mut log, &binding(1), |body| {
        sent = Some(body.clone());
        Err("response lost".into())
    })
    .is_err());
    assert_eq!(f.bytes(), before);
    local_event_runtime::upload(&mut log, &binding(1), |body| {
        assert_eq!(Some(body), sent.as_ref());
        Ok(serde_json::json!({"ok":true,"event_id":body["event_id"],"duplicate":true}))
    })
    .unwrap();
    assert!(f.open().pending_events().is_empty());
}

#[test]
fn signed_local_events_use_original_php_route_and_sqlite_without_rebinding() {
    use hbb_common::{
        base64::{engine::general_purpose::STANDARD, Engine as _},
        sodiumoxide::crypto::sign,
    };
    use std::{
        process::{Command, Stdio},
        time::{Duration, Instant},
    };
    let f = Fixture::new();
    hbb_common::sodiumoxide::init().unwrap();
    let api = PathBuf::from(std::env::var_os("LOCAL_EVENTS_API_FIXTURE").unwrap());
    let php = PathBuf::from(std::env::var_os("LOCAL_EVENTS_PHP").unwrap());
    let ini = PathBuf::from(std::env::var_os("LOCAL_EVENTS_PHP_INI").unwrap());
    let db = f.0.join("agent-status-test-local-events.db");
    let (public, secret) = sign::gen_keypair();
    let identity = identity::DeviceIdentity {
        id: "test-device".into(),
        uuid: "test-uuid".into(),
        public,
        secret,
    };
    let command = || {
        let mut cmd = Command::new(&php);
        cmd.arg("-c")
            .arg(&ini)
            .env("AGENT_FIXTURE_ROOT", &api)
            .env("AGENT_HTTP_FIXTURE_DB", &db)
            .env("RUSTDESK_DB_DRIVER", "sqlite")
            .env("RUSTDESK_ADMIN_PATH", "/agent-test")
            .env("RUSTDESK_INSTALL_CONFIG", f.0.join("absent.json"));
        cmd
    };
    let seed = r#"require getenv('AGENT_FIXTURE_ROOT').'/sqlite/lib.php';require getenv('AGENT_FIXTURE_ROOT').'/sqlite/agent_operations.php';$db=new PDO('sqlite:'.getenv('AGENT_HTTP_FIXTURE_DB'));configure_pdo($db);ensure_schema($db);$now=time();db_insert_ignore($db,'device_reports',['id'=>'test-device','uuid'=>'test-uuid','payload'=>'{}','last_seen'=>$now,'last_heartbeat'=>$now,'heartbeat_payload'=>'{}']);db_insert_ignore($db,'device_update_keys',['device_id'=>'test-device','uuid'=>'test-uuid','public_key'=>getenv('PK'),'first_seen_at'=>$now,'last_seen_at'=>$now]);agent_binding_authorize($db,['id'=>1,'address_book_scope'=>'all'],'test-device','test-uuid',str_repeat('a',32),true,$now);"#;
    let seeded = command()
        .args(["-r", seed])
        .env("PK", STANDARD.encode(identity.public.as_ref()))
        .output()
        .unwrap();
    assert!(
        seeded.status.success(),
        "{}",
        String::from_utf8_lossy(&seeded.stderr)
    );
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    struct Server(std::process::Child);
    impl Drop for Server {
        fn drop(&mut self) {
            self.0.kill().unwrap();
            self.0.wait().unwrap();
        }
    }
    let _server = Server(
        command()
            .args(["-S", &address.to_string()])
            .arg(api.join("tests/agent_status_http_router.php"))
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let deadline = Instant::now() + Duration::from_secs(15);
    while std::net::TcpStream::connect(address).is_err() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(25));
    }
    let url = format!("http://{address}");
    let mut log = f.open();
    let op = log.begin_r1(&binding(1), 1000).unwrap();
    log.record_bound_at(
        &op,
        record(Phase::Terminal, 1, Outcome::Succeeded),
        &binding(1),
        1001,
    )
    .unwrap();
    let before = f.bytes();
    let mut original = None;
    assert!(local_event_runtime::upload(&mut log, &binding(1), |body| {
        let reply = local_event_runtime::send_signed(&identity, &url, body)?;
        assert_eq!(reply["duplicate"], false);
        original = Some(body.clone());
        Err("ack deliberately dropped".into())
    })
    .is_err());
    assert_eq!(f.bytes(), before);
    let mut duplicates = 0;
    local_event_runtime::upload(&mut log, &binding(1), |body| {
        let reply = local_event_runtime::send_signed(&identity, &url, body)?;
        if reply["duplicate"] == true {
            duplicates += 1;
            assert_eq!(Some(body), original.as_ref());
        }
        Ok(reply)
    })
    .unwrap();
    assert_eq!(duplicates, 1);
    assert!(f.open().pending_events().is_empty());
    let queued = log.begin_r1(&binding(1), 1002).unwrap();
    let before = f.bytes();
    let changed=command().args(["-r",r#"$db=new PDO('sqlite:'.getenv('AGENT_HTTP_FIXTURE_DB'));$db->exec('UPDATE device_agent_bindings SET generation=2');"#]).output().unwrap();
    assert!(changed.status.success());
    let event = log.pending_events()[0].payload.clone();
    assert_eq!(
        local_event_runtime::send_signed(&identity, &url, &event).unwrap_err(),
        "local event HTTP 403"
    );
    assert!(
        local_event_runtime::upload(&mut log, &binding(2), |_| panic!(
            "old event must not be rewritten"
        ))
        .is_err()
    );
    assert_eq!(f.bytes(), before);
    assert_eq!(f.open().active_r1().unwrap().0, queued);
    let inspect=command().args(["-r",r#"$db=new PDO('sqlite:'.getenv('AGENT_HTTP_FIXTURE_DB'));echo json_encode(['events'=>$db->query('SELECT event_id,command_id,generation,sequence,status,payload,digest FROM device_agent_events ORDER BY sequence')->fetchAll(PDO::FETCH_ASSOC),'commands'=>$db->query('SELECT count(*) FROM device_agent_commands')->fetchColumn()]);"#]).output().unwrap();
    assert!(inspect.status.success());
    let data: serde_json::Value = serde_json::from_slice(&inspect.stdout).unwrap();
    assert_eq!(data["events"].as_array().unwrap().len(), 2);
    assert_eq!(data["commands"], 0);
    for (index, row) in data["events"].as_array().unwrap().iter().enumerate() {
        assert!(row["command_id"].is_null());
        assert_eq!(row["generation"], 1);
        assert_eq!(row["sequence"], index + 1);
        let text = row["payload"].as_str().unwrap();
        assert_eq!(row["digest"], hex::encode(Sha256::digest(text.as_bytes())));
        let body: serde_json::Value = serde_json::from_str(text).unwrap();
        assert_eq!(body["agent_installation_id"], "a".repeat(32));
        assert_eq!(body["enrollment_generation"], 1);
    }
}

#[test]
fn old_generation_backlog_does_not_starve_current_generation_uploads() {
    let f = Fixture::new();
    let mut log = f.open();
    let old = log.begin_r1(&binding(1), 1000).unwrap();
    for attempt in 1..=8 {
        log.record_bound_at(
            &old,
            record(Phase::Progress, attempt, Outcome::Running),
            &binding(1),
            1000,
        )
        .unwrap();
    }
    log.record_bound_at(
        &old,
        record(Phase::Terminal, 8, Outcome::Succeeded),
        &binding(1),
        1001,
    )
    .unwrap();
    log.begin_r1(&binding(2), 1002).unwrap();
    let mut sent = 0;
    let result = local_event_runtime::upload(&mut log, &binding(2), |body| {
        sent += 1;
        assert_eq!(body["enrollment_generation"], 2);
        Ok(serde_json::json!({"ok":true,"event_id":body["event_id"]}))
    });
    assert!(result.is_err(), "old generation remains unreported");
    assert_eq!(sent, 1);
    assert_eq!(f.open().pending_events().len(), 10);
}
