use serde::Deserialize;
use sha2::{Digest, Sha256};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandEnvelope {
    pub schema: u32,
    pub id: String,
    #[serde(rename = "type")]
    pub command_type: String,
    pub type_version: u32,
    pub client_id: String,
    pub client_uuid: String,
    pub agent_installation_id: String,
    pub enrollment_generation: u64,
    pub created_at: u64,
    pub expires_at: u64,
    pub timeout_seconds: u64,
    pub preconditions: Preconditions,
    pub args: serde_json::Value,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Preconditions {
    pub active_sessions: String,
}

pub struct Delivery {
    pub id: String,
    pub digest: String,
    pub expires_at: u64,
}

pub fn cursor_advances(previous: &str, next: &str) -> bool {
    fn parse(cursor: &str) -> Option<(u64, &str)> {
        let (timestamp, id) = cursor.split_once(':')?;
        if timestamp.is_empty()
            || timestamp.len() > 12
            || !timestamp.bytes().all(|byte| byte.is_ascii_digit())
            || id.len() != 32
            || !id
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return None;
        }
        Some((timestamp.parse().ok()?, id))
    }
    let Some(next) = parse(next) else {
        return false;
    };
    previous.is_empty()
        || parse(previous)
            .map(|previous| next > previous)
            .unwrap_or(false)
}

pub fn delivery(
    wire: &serde_json::Value,
    id: &str,
    uuid: &str,
    installation: &str,
    generation: u64,
) -> Result<Delivery, String> {
    let mut payload = wire.clone();
    let object = payload
        .as_object_mut()
        .ok_or_else(|| "command envelope must be an object".to_owned())?;
    let digest = object
        .remove("digest")
        .and_then(|value| value.as_str().map(str::to_owned))
        .ok_or_else(|| "command digest absent".to_owned())?;
    let status = object
        .remove("status")
        .ok_or_else(|| "command status absent".to_owned())?;
    if ![
        "pending",
        "accepted",
        "running",
        "deferred",
        "reconciliation_required",
        "cancelled",
        "expired",
        "succeeded",
        "failed",
        "rolled_back",
        "rollback_failed",
        "unsupported",
        "rejected",
    ]
    .iter()
    .any(|value| status == *value)
    {
        return Err("command status invalid".into());
    }
    if !object
        .remove("receipt_persisted")
        .map(|value| value.is_boolean())
        .unwrap_or(false)
    {
        return Err("command receipt invalid".into());
    }
    let bytes = serde_json::to_vec(&payload).map_err(|e| e.to_string())?;
    if bytes.len() > 20480 || hex::encode(Sha256::digest(bytes)) != digest {
        return Err("command digest mismatch".into());
    }
    let command_id = payload["id"]
        .as_str()
        .ok_or_else(|| "command id invalid".to_owned())?;
    let expires_at = payload["expires_at"]
        .as_u64()
        .ok_or_else(|| "command expiry invalid".to_owned())?;
    let created_at = payload["created_at"]
        .as_u64()
        .ok_or_else(|| "command creation invalid".to_owned())?;
    if payload["client_id"] != id
        || payload["client_uuid"] != uuid
        || payload["agent_installation_id"] != installation
        || payload["enrollment_generation"] != generation
        || command_id.len() != 32
        || !command_id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        || expires_at <= created_at
        || expires_at.saturating_sub(created_at) > 86400
    {
        return Err("command delivery identity invalid".into());
    }
    Ok(Delivery {
        id: command_id.into(),
        digest,
        expires_at,
    })
}

pub fn may_repeat_running(wire: &serde_json::Value) -> bool {
    execution_rejection(wire).is_none()
        && matches!(
            wire["type"].as_str(),
            Some(
                "service.inspect"
                    | "package.verify"
                    | "diagnostics.collect"
                    | "network.probe"
                    | "diagnostics.logs"
                    | "system.query_uac"
                    | "system.disable_uac"
                    | "system.run"
            )
        )
}

pub fn require_service_started(started: bool) -> Result<(), &'static str> {
    if started { Ok(()) } else { Err("service_action_not_applied") }
}

pub fn activity_deferral(kind: &str, idle: Option<bool>) -> Option<&'static str> {
    if !matches!(kind, "service.start" | "service.restart" | "service.repair" | "recovery.reconcile" | "package.update" | "package.reinstall" | "agent.update") {
        return None;
    }
    match idle {
        Some(true) => None,
        Some(false) => Some("active_sessions"),
        None => Some("activity_unknown"),
    }
}

pub fn execution_rejection(wire: &serde_json::Value) -> Option<&'static str> {
    if wire["schema"] != 1 || wire["type_version"] != 1 {
        return Some("unsupported");
    }
    let supported = [
        "service.inspect",
        "service.start",
        "service.restart",
        "service.repair",
        "recovery.reconcile",
        "package.verify",
        "diagnostics.collect",
        "network.probe",
        "package.reinstall",
        "package.update",
        "agent.update",
        "diagnostics.logs",
        "system.query_uac",
        "system.disable_uac",
        "system.run",
    ];
    if !wire["type"].as_str().is_some_and(|value| supported.contains(&value)) {
        return Some("unsupported");
    }
    let Some(id) = wire["client_id"].as_str() else {
        return Some("rejected");
    };
    let Some(uuid) = wire["client_uuid"].as_str() else {
        return Some("rejected");
    };
    let Some(installation) = wire["agent_installation_id"].as_str() else {
        return Some("rejected");
    };
    let Some(generation) = wire["enrollment_generation"].as_u64() else {
        return Some("rejected");
    };
    command_envelope(wire, id, uuid, installation, generation)
        .err()
        .map(|_| "rejected")
}

#[cfg(test)]
mod execution_tests {
    use super::*;

    #[test]
    fn fixed_maintenance_commands_are_admitted_and_unknown_commands_are_refused() {
        for command_type in [
            "service.inspect",
            "service.start",
            "service.restart",
            "service.repair",
            "recovery.reconcile",
            "package.verify",
            "diagnostics.collect",
            "network.probe",
            "system.query_uac",
            "system.disable_uac",
        ] {
            assert_ne!(
                execution_rejection(&serde_json::json!({"schema":1,"type_version":1,"type":command_type})),
                Some("unsupported"),
                "{command_type}"
            );
        }
        assert_eq!(
            execution_rejection(&serde_json::json!({"schema":1,"type_version":1,"type":"cmd.execute"})),
            Some("unsupported")
        );
    }

    fn signed(command_type: &str, args: serde_json::Value) -> serde_json::Value {
        let payload = serde_json::json!({
            "schema": 1, "id": "0123456789abcdef0123456789abcdef", "type": command_type,
            "type_version": 1, "client_id": "device", "client_uuid": "uuid",
            "agent_installation_id": "installation", "enrollment_generation": 3,
            "created_at": 1000, "expires_at": 2000, "timeout_seconds": 60,
            "preconditions": {"active_sessions": "defer"}, "args": args,
        });
        let digest = hex::encode(Sha256::digest(serde_json::to_vec(&payload).unwrap()));
        let mut wire = payload;
        wire["digest"] = digest.into();
        wire["status"] = "pending".into();
        wire["receipt_persisted"] = false.into();
        wire
    }

    #[test]
    fn diagnostics_logs_is_admitted_read_only_with_strict_arguments() {
        let id = "0123456789abcdef0123456789abcdef";
        for args in [
            serde_json::json!({"source":"agent"}),
            serde_json::json!({"source":"msi-update"}),
            serde_json::json!({"source":"transaction","transaction_id":id}),
        ] {
            let wire = signed("diagnostics.logs", args.clone());
            assert_eq!(execution_rejection(&wire), None, "{args}");
            assert!(command_envelope(&wire, "device", "uuid", "installation", 3).is_ok());
            assert!(may_repeat_running(&wire), "{args}");
        }
        assert_eq!(activity_deferral("diagnostics.logs", Some(false)), None);
        for args in [
            serde_json::json!({}),
            serde_json::json!([]),
            serde_json::json!({"source":"agent","path":"C:\\Windows"}),
            serde_json::json!({"source":"transaction"}),
            serde_json::json!({"source":"agent","transaction_id":id}),
            serde_json::json!({"source":"C:\\Windows\\System32\\config\\SAM"}),
            serde_json::json!({"source":"transaction","transaction_id":"..\\..\\x"}),
        ] {
            let wire = signed("diagnostics.logs", args.clone());
            assert_eq!(execution_rejection(&wire), Some("rejected"), "{args}");
            assert!(!may_repeat_running(&wire), "{args}");
        }
    }
}

// Kept local because this file is also compiled standalone by integration tests;
// diagnostic_logs::parse_args applies the same contract before any path is built.
pub fn diagnostics_logs_args_valid(args: &serde_json::Value) -> bool {
    let Some(object) = args.as_object() else {
        return false;
    };
    if object
        .keys()
        .any(|key| !matches!(key.as_str(), "source" | "transaction_id"))
    {
        return false;
    }
    let transaction = object.get("transaction_id");
    match object.get("source").and_then(serde_json::Value::as_str) {
        Some("transaction") => transaction
            .and_then(serde_json::Value::as_str)
            .is_some_and(|id| {
                id.len() == 32
                    && id
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            }),
        Some(
            "agent" | "server" | "service" | "update" | "silent-install" | "install" | "msi-update"
            | "recovery-msi",
        ) => transaction.is_none(),
        _ => false,
    }
}

pub fn canonical_request(
    method: &str,
    path: &str,
    identity: (&str, &str),
    timestamp: i64,
    nonce: &str,
    command: &str,
    body: &[u8],
) -> String {
    let (id, uuid) = identity;
    format!("rustdesk-update-auth-v1\nmethod={}\npath={path}\nclient_id={id}\nclient_uuid={uuid}\ntimestamp={timestamp}\nnonce={nonce}\ncommand_id={command}\nbody_sha256={}\n", method.to_ascii_uppercase(), hex::encode(Sha256::digest(body)))
}

pub fn command_envelope(
    wire: &serde_json::Value,
    id: &str,
    uuid: &str,
    installation: &str,
    generation: u64,
) -> Result<CommandEnvelope, String> {
    let mut payload = wire.clone();
    let object = payload
        .as_object_mut()
        .ok_or_else(|| "command envelope must be an object".to_owned())?;
    let digest = object
        .remove("digest")
        .and_then(|value| value.as_str().map(str::to_owned))
        .ok_or_else(|| "command digest absent".to_owned())?;
    let status = object
        .remove("status")
        .ok_or_else(|| "command status absent".to_owned())?;
    if ![
        "pending",
        "accepted",
        "running",
        "deferred",
        "reconciliation_required",
        "cancelled",
        "expired",
        "succeeded",
        "failed",
        "rolled_back",
        "rollback_failed",
        "unsupported",
        "rejected",
    ]
    .iter()
    .any(|value| status == *value)
    {
        return Err("command status invalid".into());
    }
    if !object
        .remove("receipt_persisted")
        .map(|value| value.is_boolean())
        .unwrap_or(false)
    {
        return Err("command receipt invalid".into());
    }
    let bytes = serde_json::to_vec(&payload).map_err(|e| e.to_string())?;
    if bytes.len() > 16384 + 4096 || hex::encode(Sha256::digest(bytes)) != digest {
        return Err("command digest mismatch".into());
    }
    let envelope: CommandEnvelope =
        serde_json::from_value(payload).map_err(|_| "command fields invalid".to_owned())?;
    if envelope.schema != 1
        || envelope.type_version != 1
        || envelope.client_id != id
        || envelope.client_uuid != uuid
        || envelope.agent_installation_id != installation
        || envelope.enrollment_generation != generation
        || envelope.id.len() != 32
        || !envelope
            .id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        || envelope.expires_at <= envelope.created_at
        || envelope.expires_at.saturating_sub(envelope.created_at) > 86400
        || envelope.timeout_seconds == 0
        || envelope.timeout_seconds > 900
        || envelope.preconditions.active_sessions != "defer"
    {
        return Err("command identity or contract invalid".into());
    }
    if matches!(
        envelope.command_type.as_str(),
        "service.inspect"
            | "service.start"
            | "service.restart"
            | "service.repair"
            | "recovery.reconcile"
            | "package.verify"
            | "diagnostics.collect"
            | "network.probe"
            | "system.query_uac"
            | "system.disable_uac"
    ) {
        if !(envelope
            .args
            .as_object()
            .map(|v| v.is_empty())
            .unwrap_or(false)
            || envelope
                .args
                .as_array()
                .map(|v| v.is_empty())
                .unwrap_or(false))
        {
            return Err("command execution parameters unsupported".into());
        }
    } else if envelope.command_type == "system.run" {
        if super::script_run::parse_args(&envelope.args, envelope.timeout_seconds).is_err() {
            return Err("system.run execution parameters invalid".into());
        }
    } else if matches!(envelope.command_type.as_str(), "package.reinstall" | "package.update" | "agent.update") {
        let args = envelope.args.as_object().ok_or("package command arguments must be object")?;
        let reference = args.get("package_id").and_then(serde_json::Value::as_str).ok_or("package reference absent")?;
        if args.len() != 4 || reference.len() != 64
            || !reference.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            || !matches!(args.get("target_key").and_then(serde_json::Value::as_str), Some("windows-x86_64-msi-standard" | "windows-x86_64-msi-sos"))
            || !args.get("target_build_seq").and_then(serde_json::Value::as_u64).is_some_and(|build| build > 0)
            || args.get("allow_downgrade") != Some(&serde_json::Value::Bool(false)) {
            return Err("package command execution parameters invalid".into());
        }
    } else if envelope.command_type == "diagnostics.logs" {
        if !diagnostics_logs_args_valid(&envelope.args) {
            return Err("diagnostics.logs execution parameters invalid".into());
        }
    } else {
        return Err("command type unsupported by executor".into());
    }
    Ok(envelope)
}
