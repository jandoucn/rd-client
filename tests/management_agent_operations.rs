#![cfg(windows)]
#[path = "../src/management_agent/operations.rs"]
mod operations;
// operations.rs 里 `system.run` 的参数校验走 `super::script_run`，测试入口补同名路径。
#[path = "../src/management_agent/script_run.rs"]
mod script_run;

use operations::{canonical_request, command_envelope};

#[test]
fn disruptive_commands_require_confirmed_idle_but_read_only_commands_do_not() {
    for kind in ["service.restart", "service.repair", "recovery.reconcile", "package.update", "package.reinstall", "agent.update"] {
        assert_eq!(operations::activity_deferral(kind, None), Some("activity_unknown"));
        assert_eq!(operations::activity_deferral(kind, Some(false)), Some("active_sessions"));
        assert_eq!(operations::activity_deferral(kind, Some(true)), None);
    }
    for kind in ["service.inspect", "package.verify", "diagnostics.collect", "network.probe"] {
        assert_eq!(operations::activity_deferral(kind, None), None);
        assert_eq!(operations::activity_deferral(kind, Some(false)), None);
    }
}

fn maintenance_wire(kind: &str) -> serde_json::Value {
    use sha2::{Digest, Sha256};
    let mut wire = serde_json::json!({"schema":1,"id":"a".repeat(32),"type":kind,"type_version":1,
        "client_id":"device","client_uuid":"uuid","agent_installation_id":"b".repeat(32),"enrollment_generation":1,
        "created_at":1000,"expires_at":2000,"timeout_seconds":900,"preconditions":{"active_sessions":"defer"},"args":{}});
    wire["digest"] = hex::encode(Sha256::digest(serde_json::to_vec(&wire).unwrap())).into();
    wire["status"] = "running".into();
    wire["receipt_persisted"] = true.into();
    wire
}

#[test]
fn msi_package_commands_require_exact_approved_reference_and_forbid_extra_execution_inputs() {
    use sha2::{Digest, Sha256};
    let sign_wire = |mut value: serde_json::Value| {
        for key in ["digest", "status", "receipt_persisted"] {
            value.as_object_mut().unwrap().remove(key);
        }
        value["digest"] = hex::encode(Sha256::digest(serde_json::to_vec(&value).unwrap())).into();
        value["status"] = "accepted".into();
        value["receipt_persisted"] = true.into();
        value
    };
    for kind in ["package.reinstall", "package.update", "agent.update"] {
        let mut wire = maintenance_wire(kind);
        wire["args"] = serde_json::json!({"package_id":"c".repeat(64),
            "target_key":"windows-x86_64-msi-standard","target_build_seq":2026100503u64,"allow_downgrade":false});
        let wire = sign_wire(wire);
        assert_eq!(operations::execution_rejection(&wire), None, "{kind}");
        assert!(!operations::may_repeat_running(&wire));
        for (key, value) in [("package_id", "bad".into()), ("target_key", "windows-x86_64-exe-standard".into()),
            ("target_build_seq", 0.into()), ("allow_downgrade", true.into()), ("path", "C:\\arbitrary.msi".into())] {
            let mut invalid = wire.clone();
            invalid["args"][key] = value;
            assert_eq!(operations::execution_rejection(&sign_wire(invalid)), Some("rejected"), "{kind}/{key}");
        }
        for key in ["package_id", "target_key", "target_build_seq", "allow_downgrade"] {
            let mut invalid = wire.clone();
            invalid["args"].as_object_mut().unwrap().remove(key);
            assert_eq!(operations::execution_rejection(&sign_wire(invalid)), Some("rejected"), "missing {key}");
        }
    }
}

#[test]
fn interrupted_mutations_require_reconciliation_instead_of_repeat_execution() {
    for kind in ["service.start", "service.restart", "service.repair", "recovery.reconcile"] {
        let wire = maintenance_wire(kind);
        let before = wire.clone();
        assert_eq!(operations::execution_rejection(&wire), None, "{kind}");
        assert!(!operations::may_repeat_running(&wire), "must not repeat {kind}");
        assert_eq!(wire, before, "recovery classification must preserve the command");
    }
    for kind in ["service.inspect", "package.verify", "diagnostics.collect", "network.probe"] {
        let mut wire = maintenance_wire(kind);
        assert!(operations::may_repeat_running(&wire), "{kind}");
        wire["digest"] = "0".repeat(64).into();
        assert!(!operations::may_repeat_running(&wire), "invalid command must not repeat");
    }
}

#[test]
fn refused_service_action_cannot_be_reported_as_running() {
    assert_eq!(operations::require_service_started(false), Err("service_action_not_applied"));
    assert_eq!(operations::require_service_started(true), Ok(()));
}

#[test]
fn unsupported_delivery_remains_bound_and_can_be_rejected_without_execution() {
    use sha2::{Digest, Sha256};
    let mut payload = serde_json::json!({"schema":2,"id":"a".repeat(32),"type":"future.command","type_version":99,
        "client_id":"device","client_uuid":"uuid","agent_installation_id":"b".repeat(32),"enrollment_generation":1,
        "created_at":1000,"expires_at":2000,"timeout_seconds":900,"preconditions":{"active_sessions":"defer"},"args":{"future":true}});
    let digest = hex::encode(Sha256::digest(serde_json::to_vec(&payload).unwrap()));
    payload["digest"] = digest.clone().into();
    payload["status"] = "pending".into();
    payload["receipt_persisted"] = false.into();
    let delivery = operations::delivery(&payload, "device", "uuid", &"b".repeat(32), 1).unwrap();
    assert_eq!(delivery.id, "a".repeat(32));
    assert_eq!(delivery.digest, digest);
    assert_eq!(delivery.expires_at, 2000);
    assert_eq!(
        operations::execution_rejection(&payload),
        Some("unsupported")
    );
    assert!(command_envelope(&payload, "device", "uuid", &"b".repeat(32), 1).is_err());
    assert!(operations::delivery(&payload, "other", "uuid", &"b".repeat(32), 1).is_err());
    payload["digest"] = "0".repeat(64).into();
    assert!(operations::delivery(&payload, "device", "uuid", &"b".repeat(32), 1).is_err());
}

#[test]
fn unknown_execution_parameters_are_rejected_with_valid_delivery_identity() {
    use sha2::{Digest, Sha256};
    let mut payload = serde_json::json!({"schema":1,"id":"a".repeat(32),"type":"service.inspect","type_version":1,
        "client_id":"device","client_uuid":"uuid","agent_installation_id":"b".repeat(32),"enrollment_generation":1,
        "created_at":1000,"expires_at":2000,"timeout_seconds":900,"preconditions":{"active_sessions":"defer"},"args":{"path":"C:\\not-allowed"}});
    payload["digest"] = hex::encode(Sha256::digest(serde_json::to_vec(&payload).unwrap())).into();
    payload["status"] = "pending".into();
    payload["receipt_persisted"] = false.into();
    assert!(operations::delivery(&payload, "device", "uuid", &"b".repeat(32), 1).is_ok());
    assert_eq!(operations::execution_rejection(&payload), Some("rejected"));
}

#[test]
fn envelope_is_exactly_bound_and_unknown_execution_fields_are_refused() {
    let command = serde_json::json!({"schema":1,"id":"a".repeat(32),"type":"service.inspect","type_version":1,
        "client_id":"device","client_uuid":"uuid","agent_installation_id":"b".repeat(32),"enrollment_generation":1,
        "created_at":1000,"expires_at":2000,"timeout_seconds":900,"preconditions":{"active_sessions":"defer"},"args":[]});
    let mut wire = command.clone();
    use sha2::{Digest, Sha256};
    wire["digest"] = hex::encode(Sha256::digest(serde_json::to_vec(&command).unwrap())).into();
    wire["status"] = "pending".into();
    wire["receipt_persisted"] = false.into();
    assert!(command_envelope(&wire, "device", "uuid", &"b".repeat(32), 1).is_ok());
    assert!(command_envelope(&wire, "other", "uuid", &"b".repeat(32), 1).is_err());
    assert!(command_envelope(&wire, "device", "uuid", &"b".repeat(32), 2).is_err());
    wire["args"] = serde_json::json!({"path":"C:\\anything"});
    assert!(command_envelope(&wire, "device", "uuid", &"b".repeat(32), 1).is_err());
    wire["args"] = serde_json::json!([]);
    wire["force"] = true.into();
    assert!(command_envelope(&wire, "device", "uuid", &"b".repeat(32), 1).is_err());
}

#[test]
fn request_signature_binds_exact_path_command_and_bytes() {
    let canonical = canonical_request(
        "POST",
        "/rd/update/v1/agent/commands/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/claim",
        ("device", "uuid"),
        1000,
        "nonce",
        &"a".repeat(32),
        b"{}",
    );
    assert!(canonical.contains("method=POST\npath=/rd/update/v1/agent/commands/"));
    assert!(canonical.contains(&format!("command_id={}\n", "a".repeat(32))));
    assert!(canonical.ends_with(
        "body_sha256=44136fa355b3678a1146ad16f7e8649e94fb4fc21fe77e8310c060f61caaff8a\n"
    ));
    assert_ne!(
        canonical,
        canonical_request(
            "POST",
            "/wrong",
            ("device", "uuid"),
            1000,
            "nonce",
            &"a".repeat(32),
            b"{}"
        )
    );
}

#[test]
fn cursor_validation_requires_numeric_progress_and_rejects_loops_or_malformed_values() {
    assert!(operations::cursor_advances(
        "",
        "999:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
    ));
    assert!(operations::cursor_advances(
        "999:ffffffffffffffffffffffffffffffff",
        "1000:00000000000000000000000000000001"
    ));
    assert!(operations::cursor_advances(
        "1000:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "1000:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
    ));
    assert!(!operations::cursor_advances(
        "1000:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        "1000:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
    ));
    assert!(!operations::cursor_advances(
        "1000:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "1000:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
    ));
    for cursor in [
        "bad",
        "-1:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "1000:nothex",
        "1000:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
        "1000:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa:extra",
    ] {
        assert!(!operations::cursor_advances("", cursor));
    }
}
