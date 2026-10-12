use super::{install_plan::{InstallPlan, Trigger}, transaction::{Phase, Transaction}};
use sha2::{Digest, Sha256};

#[cfg(windows)]
pub fn read_terminal(root: &std::path::Path, wire: &serde_json::Value) -> Result<Option<&'static str>, String> {
    use std::{io::Read, os::windows::fs::OpenOptionsExt};
    super::workspace::verify(root)?;
    let read = |name: &str| -> Result<Vec<u8>, String> {
        let path = root.join(name);
        super::workspace::verify_file(&path)?;
        let mut bytes = Vec::new();
        std::fs::OpenOptions::new().read(true).share_mode(1).custom_flags(0x00200000)
            .open(path).map_err(|e| e.to_string())?.take(65537)
            .read_to_end(&mut bytes).map_err(|e| e.to_string())?;
        if bytes.len() > 65536 { return Err("command transaction record too large".into()); }
        Ok(bytes)
    };
    let binding: serde_json::Value = serde_json::from_slice(&read("command-binding.json")?)
        .map_err(|_| "command transaction binding malformed")?;
    let plan = InstallPlan::decode(&read("prepared.json")?)?;
    let transaction = Transaction::decode(&read("transaction.json")?)?;
    // Validate command identity even when the helper has not published a result.
    let mut nonterminal = transaction.clone();
    nonterminal.phase = Phase::Applying;
    terminal_status(wire, &binding, &plan, &nonterminal, &serde_json::Value::Null)?;
    match std::fs::symlink_metadata(root.join("execution-result.json")) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("command transaction result unavailable: {error}")),
        Ok(_) => {}
    }
    let result: serde_json::Value = serde_json::from_slice(&read("execution-result.json")?)
        .map_err(|_| "command transaction result malformed")?;
    terminal_status(wire, &binding, &plan, &transaction, &result)
}

pub fn terminal_status(
    wire: &serde_json::Value,
    binding: &serde_json::Value,
    plan: &InstallPlan,
    transaction: &Transaction,
    result: &serde_json::Value,
) -> Result<Option<&'static str>, String> {
    plan.bind_transaction(transaction)?;
    let digest = hex::encode(Sha256::digest(serde_json::to_vec(plan).map_err(|e| e.to_string())?));
    if binding["schema"] != 1 || binding["plan_sha256"] != digest
        || plan.trigger != Trigger::RemoteCommand || plan.command_id.as_deref() != wire["id"].as_str()
        || plan.transaction_id != wire["id"] || plan.installation_id != wire["agent_installation_id"]
        || plan.target_key != wire["args"]["target_key"] || wire["args"]["target_build_seq"] != plan.target_build_seq
        || !matches!(wire["type"].as_str(), Some("package.reinstall" | "package.update" | "agent.update"))
        || wire["args"]["allow_downgrade"] != false {
        return Err("command transaction plan binding mismatch".into());
    }
    for (saved, current) in [("command_id","id"),("command_digest","digest"),("command_type","type"),
        ("client_id","client_id"),("client_uuid","client_uuid"),("installation_id","agent_installation_id"),
        ("enrollment_generation","enrollment_generation")] {
        if binding[saved].is_null() || binding[saved] != wire[current] {
            return Err("command transaction receipt binding mismatch".into());
        }
    }
    if binding["package_id"].is_null() || binding["package_id"] != wire["args"]["package_id"] {
        return Err("command transaction package binding mismatch".into());
    }
    let terminal = match transaction.phase {
        Phase::Committed => "succeeded",
        Phase::RolledBack => "rolled_back",
        Phase::RollbackFailed => "rollback_failed",
        _ => return Ok(None),
    };
    if result.get("schema").and_then(serde_json::Value::as_u64).is_none()
        || ["transaction_id", "command_id", "phase"].iter()
            .any(|key| result.get(*key).and_then(serde_json::Value::as_str).is_none())
        || !result.get("error").is_some_and(|error| error.is_null() || error.is_string())
        || result.get("reboot_required").and_then(serde_json::Value::as_bool).is_none()
    {
        return Err("command transaction result malformed".into());
    }
    if result["schema"] != 1 || result["transaction_id"] != transaction.id
        || result["command_id"] != wire["id"] || result["reboot_required"] != false
        || result["phase"] != serde_json::to_value(transaction.phase).map_err(|e| e.to_string())?
        || (transaction.phase == Phase::Committed && !result["error"].is_null()) {
        return Err("command transaction result binding mismatch".into());
    }
    Ok(Some(terminal))
}
