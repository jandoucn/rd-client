use super::{local_events, reconciliation_candidate, Binding, LocalEvents};
use std::{os::windows::fs::OpenOptionsExt, path::PathBuf};

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        assert_eq!(
            std::env::var("RUSTDESK_R1_RECONCILE_TEST").as_deref(),
            Ok("scheduled")
        );
        assert_eq!(std::env::var("COMPUTERNAME").as_deref(), Ok("RDLAB-WIN11"));
        let base = PathBuf::from(std::env::var_os("RUSTDESK_LOCAL_EVENTS_ROOT").unwrap());
        assert!(base.is_absolute());
        let root = base.join(format!("r1-reconcile-{}", uuid::Uuid::new_v4()));
        super::super::workspace::create(&root).unwrap();
        Self(root)
    }

    fn path(&self) -> PathBuf {
        self.0.join("events.json")
    }

    fn binding(&self) -> Binding {
        Binding {
            client_id: "fixture-device".into(),
            client_uuid: "fixture-uuid".into(),
            agent_installation_id: "a".repeat(32),
            enrollment_generation: 1,
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn r1_reconcile_corrupt_store_returns_error_and_preserves_bytes() {
    let f = Fixture::new();
    let before = b"{broken local event store";
    std::fs::write(f.path(), before).unwrap();
    let result = reconciliation_candidate(&f.path());
    assert_eq!(std::fs::read(f.path()).unwrap(), before);
    assert_eq!(result, Err("local event Corrupt".into()));
}

#[test]
fn r1_reconcile_read_denied_returns_error_and_preserves_bytes() {
    let f = Fixture::new();
    LocalEvents::open(&f.path())
        .unwrap()
        .remember_binding(f.binding())
        .unwrap();
    let before = std::fs::read(f.path()).unwrap();
    let lock = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(f.path())
        .unwrap();
    let result = reconciliation_candidate(&f.path());
    drop(lock);
    assert_eq!(std::fs::read(f.path()).unwrap(), before);
    assert_eq!(result, Err("local event Storage".into()));
}

#[test]
fn r1_reconcile_no_active_operation_returns_none_without_writing() {
    let f = Fixture::new();
    assert_eq!(reconciliation_candidate(&f.path()), Ok(None));
    assert!(!f.path().exists());
    let mut log = LocalEvents::open(&f.path()).unwrap();
    let binding = f.binding();
    log.remember_binding(binding.clone()).unwrap();
    let before = std::fs::read(f.path()).unwrap();
    assert_eq!(reconciliation_candidate(&f.path()), Ok(None));
    assert_eq!(std::fs::read(f.path()).unwrap(), before);
    let operation = log.begin_r1(&binding, 100).unwrap();
    log.record_bound_at(
        &operation,
        local_events::Record {
            phase: local_events::Phase::Terminal,
            attempt_seq: 1,
            outcome: local_events::Outcome::Succeeded,
            path_key: local_events::PathKey::RustDeskService,
            error_code: None,
        },
        &binding,
        101,
    )
    .unwrap();
    let before = std::fs::read(f.path()).unwrap();
    assert_eq!(reconciliation_candidate(&f.path()), Ok(None));
    assert_eq!(std::fs::read(f.path()).unwrap(), before);
}

#[test]
fn r1_reconcile_active_operation_keeps_original_binding_and_bytes() {
    let f = Fixture::new();
    let binding = f.binding();
    let operation = LocalEvents::open(&f.path())
        .unwrap()
        .begin_r1(&binding, 100)
        .unwrap();
    let before = std::fs::read(f.path()).unwrap();
    assert_eq!(
        reconciliation_candidate(&f.path()),
        Ok(Some((operation, binding)))
    );
    assert_eq!(std::fs::read(f.path()).unwrap(), before);
}
