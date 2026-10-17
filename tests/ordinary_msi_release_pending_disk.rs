#[cfg(t02_release_policy)]
#[test]
fn real_intent_without_outcome_blocks_and_preserves_bytes() {
    let root = std::env::temp_dir().join(format!("release-pending-{}-{}", std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
    assert_eq!(refuse_pending_at(&root), Ok(()));
    assert!(!root.exists(), "检查缺失根目录不得创建目录");
    let binding = super::Binding {
        transaction_id: "a".repeat(32), client_id: "fixture".into(), client_uuid: "uuid".into(),
        installation_id: "b".repeat(32), target_key: "windows-x86_64-msi-standard".into(),
        package_sha256: "c".repeat(64), from_version: "1.5.3".into(), from_build_seq: 2026100506,
        to_version: "1.5.3".into(), to_build_seq: 2026101501, source: "primary".into(), command_id: Some("command".into()),
    };
    let outbox = Outbox::fixture_named(&root, binding, "approved.msi").unwrap();
    let intent = fs::read(outbox.directory.join("intent.json")).unwrap();
    let result = refuse_pending_at(&root);
    assert_eq!(fs::read(outbox.directory.join("intent.json")).unwrap(), intent);
    assert!(!outbox.directory.join("outcome.json").exists());
    assert!(!outbox.directory.join("terminal.json").exists());
    assert_eq!(result, Err("ordinary prior installation requires reconciliation".into()));
    outbox.record_outcome(super::InstallerOutcome::Failed { exit_code: 1603, error_code: "msi_exit_1603".into() }).unwrap();
    outbox.reconcile(None).unwrap();
    assert_eq!(refuse_pending_at(&root), Err("ordinary prior installation requires reconciliation".into()));
    let (_, digest) = outbox.pending(Channel::Update).unwrap().unwrap();
    outbox.acknowledge(Channel::Update, &digest).unwrap();
    assert_eq!(refuse_pending_at(&root), Err("ordinary prior installation requires reconciliation".into()));
    outbox.acknowledge(Channel::Command, &digest).unwrap();
    assert_eq!(refuse_pending_at(&root), Ok(()));
    assert_eq!(fs::read(outbox.directory.join("intent.json")).unwrap(), intent);
    fs::remove_dir_all(root).unwrap();
}
