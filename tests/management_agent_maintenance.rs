#![cfg(windows)]
#[path = "../src/management_agent/maintenance.rs"]
mod maintenance;
#[path = "../src/management_agent/workspace.rs"]
mod workspace;

fn fixture() -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!("agent-maintenance-{}-{}", std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
    workspace::create(&root).unwrap();
    std::fs::write(root.join("agent.json"), serde_json::to_vec(&serde_json::json!({
        "agent_installation_id":"a".repeat(32), "disabled":false,"management_agent_enabled":true
    })).unwrap()).unwrap();
    std::fs::write(root.join("intent.toml"), b"[options]\nstop-service='N'\n").unwrap();
    std::fs::write(root.join("maintenance.json"), serde_json::to_vec(&serde_json::json!({
        "schema":1,"installation_id":"a".repeat(32),"transaction_id":"b".repeat(32),
        "plan_sha256":"c".repeat(64),"nonce":"d".repeat(64)
    })).unwrap()).unwrap();
    root
}

fn check(root: &std::path::Path) -> Result<(), String> {
    maintenance::verify(root, &root.join("intent.toml"), &"a".repeat(32), &"b".repeat(32), &"c".repeat(64), &"d".repeat(64))
}

#[test]
fn claim_is_exclusive_and_protected_without_overwriting_existing_maintenance() {
    let root = fixture();
    let marker = root.join("maintenance.json");
    let before = std::fs::read(&marker).unwrap();
    assert!(maintenance::claim(&root, &"a".repeat(32), &"b".repeat(32), &"c".repeat(64), &"d".repeat(64)).is_err());
    assert_eq!(std::fs::read(&marker).unwrap(), before);
    std::fs::remove_file(&marker).unwrap();
    assert!(maintenance::claim(&root, "invalid", &"b".repeat(32), &"c".repeat(64), &"d".repeat(64)).is_err());
    assert!(!marker.exists());
    maintenance::claim(&root, &"a".repeat(32), &"b".repeat(32), &"c".repeat(64), &"d".repeat(64)).unwrap();
    check(&root).unwrap();
    workspace::verify_file(&marker).unwrap();
    let before = std::fs::read(&marker).unwrap();
    assert!(maintenance::claim(&root, &"a".repeat(32), &"b".repeat(32), &"c".repeat(64), &"d".repeat(64)).is_err());
    assert_eq!(std::fs::read(&marker).unwrap(), before);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn release_removes_only_exact_owned_marker_and_keeps_local_disable_and_state() {
    let root = fixture();
    let marker = root.join("maintenance.json");
    let before = std::fs::read(&marker).unwrap();
    let state = std::fs::read(root.join("agent.json")).unwrap();
    std::fs::write(root.join("disable-agent"), b"local-disable").unwrap();
    assert_eq!(maintenance::release_owned(&root, &"a".repeat(32), &"b".repeat(32), &"c".repeat(64), &"e".repeat(64)).unwrap_err(), "maintenance binding mismatch");
    assert_eq!(std::fs::read(&marker).unwrap(), before);
    maintenance::release_owned(&root, &"a".repeat(32), &"b".repeat(32), &"c".repeat(64), &"d".repeat(64)).unwrap();
    assert!(!marker.exists());
    assert_eq!(std::fs::read(root.join("disable-agent")).unwrap(), b"local-disable");
    assert_eq!(std::fs::read(root.join("agent.json")).unwrap(), state);
    assert!(maintenance::release_owned(&root, &"a".repeat(32), &"b".repeat(32), &"c".repeat(64), &"d".repeat(64)).is_err());
    assert!(!marker.exists());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn marker_is_bound_to_exact_preparation_and_never_adopted_or_removed() {
    let root = fixture();
    check(&root).unwrap();
    let path = root.join("maintenance.json");
    let original = std::fs::read(&path).unwrap();
    for key in ["installation_id", "transaction_id", "plan_sha256", "nonce", "schema"] {
        let mut value: serde_json::Value = serde_json::from_slice(&original).unwrap();
        value[key] = if key == "schema" { 2.into() } else { "other".into() };
        let bytes = serde_json::to_vec(&value).unwrap();
        std::fs::write(&path, &bytes).unwrap();
        assert_eq!(check(&root).unwrap_err(), "maintenance binding mismatch", "{key}");
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
    }
    std::fs::remove_file(&path).unwrap();
    assert!(check(&root).is_err());
    assert!(!path.exists());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn local_disables_policy_changes_and_unreadable_intent_refuse_takeover() {
    let root = fixture();
    let original = std::fs::read(root.join("maintenance.json")).unwrap();
    for marker in ["disable-agent", "disable-recovery"] {
        std::fs::write(root.join(marker), b"keep").unwrap();
        assert_eq!(check(&root).unwrap_err(), "installation locally blocked");
        assert_eq!(std::fs::read(root.join(marker)).unwrap(), b"keep");
        std::fs::remove_file(root.join(marker)).unwrap();
    }
    let agent = root.join("agent.json");
    let state = std::fs::read(&agent).unwrap();
    for (key, value) in [("disabled", true.into()), ("management_agent_enabled", false.into()), ("agent_installation_id", "other".into())] {
        let mut changed: serde_json::Value = serde_json::from_slice(&state).unwrap();
        changed[key] = value;
        let bytes = serde_json::to_vec(&changed).unwrap();
        std::fs::write(&agent, &bytes).unwrap();
        assert_eq!(check(&root).unwrap_err(), "installation state disabled or identity changed");
        assert_eq!(std::fs::read(&agent).unwrap(), bytes);
    }
    std::fs::write(&agent, &state).unwrap();
    for text in ["[options]\nstop-service='Y'", "[options]\nstop-service='unknown'", "invalid toml"] {
        std::fs::write(root.join("intent.toml"), text).unwrap();
        assert!(check(&root).is_err());
        assert_eq!(std::fs::read_to_string(root.join("intent.toml")).unwrap(), text);
    }
    std::fs::remove_file(root.join("intent.toml")).unwrap();
    assert!(check(&root).is_err());
    assert_eq!(std::fs::read(root.join("maintenance.json")).unwrap(), original);
    std::fs::remove_dir_all(root).unwrap();
}
