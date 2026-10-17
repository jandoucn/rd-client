// Compiled inside the real agent module, not as a replacement state serializer.
#![cfg(windows)]
#[cfg(test)]
use super::*;

#[cfg(test)]
fn roundtrip(edition: Edition) {
    let root = std::env::temp_dir().join(format!(
        "legacy-state-red-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let paths = AgentPaths::from_program_data(&root, edition);
    std::fs::create_dir_all(&paths.state).unwrap();
    let path = paths.state.join("agent.json");
    // Fields below follow f79ed6 AgentState. Unknown keys are deliberate forward
    // compatibility probes, not claimed to be a captured customer's state.
    let old = serde_json::json!({
        "schema_version": 1, "edition": edition.name(), "package_kind": "exe",
        "target_key": edition.target_key_for(PackageKind::Exe), "manifest_url": MANIFEST_URL,
        "normal_install_path": "C:\\Program Files\\RustDesk", "disabled": false,
        "management_agent_enabled": true, "management_agent_policy_revision": 17,
        "legacy_unknown_object": {"keep": [null, true, 7, "opaque"]},
        "legacy_unknown_null": null, "legacy_unknown_number": 9007199254740993u64
    });
    let original = serde_json::to_vec_pretty(&old).unwrap();
    std::fs::write(&path, &original).unwrap();
    let backup = root.join("original-state.json");
    std::fs::write(&backup, &original).unwrap();
    let mut actual = old.clone();
    for attempt in 1..=3 {
        let mut state: AgentState = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert!(
            state.agent_installation_id.is_empty(),
            "this test must not invent an installation ID"
        );
        state.retry_stage = attempt;
        write_state(&path, &state).unwrap();
        actual = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(actual["retry_stage"], attempt);
    }
    assert_eq!(std::fs::read(&backup).unwrap(), original);
    // Preserve failed guest artifacts for triage; remove only after assertions pass.
    for key in [
        "legacy_unknown_object",
        "legacy_unknown_null",
        "legacy_unknown_number",
    ] {
        assert!(
            actual.get(key).is_some(),
            "unknown key lost: {key}; fixture={}",
            root.display()
        );
        assert_eq!(actual[key], old[key], "unknown value changed: {key}");
    }
    assert_eq!(actual["management_agent_policy_revision"], 17);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn legacy_unknown_fields_standard_real_write_state() {
    roundtrip(Edition::Standard);
}

#[test]
fn legacy_unknown_fields_sos_real_write_state() {
    roundtrip(Edition::Sos);
}

#[test]
fn legacy_known_fields_reject_wrong_types_and_duplicates() {
    let base = r#"{"edition":"standard","target_key":"windows-x86_64-exe-standard","manifest_url":"fixed","disabled":false,"opaque":{"disabled":false,"agent_installation_id":"fake"}}"#;
    let state: AgentState = serde_json::from_str(base).unwrap();
    assert_eq!(state.schema_version, 1);
    assert_eq!(state.package_kind, PackageKind::Msi);
    assert!(state.agent_installation_id.is_empty());
    assert!(!state.disabled);
    for bad in [
        base.replace("\"disabled\":false,", "\"disabled\":\"false\","),
        base.replacen(
            "\"disabled\":false,",
            "\"disabled\":true,\"disabled\":false,",
            1,
        ),
        base.replace("\"edition\":\"standard\"", "\"edition\":17"),
    ] {
        assert!(serde_json::from_str::<AgentState>(&bad).is_err());
    }
}

#[test]
fn legacy_bad_json_preserves_original_disk_bytes() {
    let root = std::env::temp_dir().join(format!("legacy-invalid-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("agent.json");
    let original = b"{\"edition\": \"standard\", broken";
    std::fs::write(&path, original).unwrap();
    let result = serde_json::from_slice::<AgentState>(&std::fs::read(&path).unwrap());
    assert!(result.unwrap_err().is_syntax());
    assert_eq!(std::fs::read(&path).unwrap(), original);
    std::fs::remove_dir_all(root).unwrap();
}
