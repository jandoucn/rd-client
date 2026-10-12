use super::install_execution::InstallOutcome;
use std::{
    ffi::OsString,
    path::Path,
    process::Command,
    time::{Duration, Instant},
};

pub fn msi_arguments(package: &Path, log: &Path) -> Result<Vec<OsString>, String> {
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    if !package.is_absolute()
        || !log.is_absolute()
        || package.extension().and_then(|v| v.to_str()) != Some("msi")
        || package.to_string_lossy().contains('"')
        || log.to_string_lossy().contains('"')
    {
        return Err("installer package/log path invalid".into());
    }
    let native = |path: &Path| OsString::from_wide(&path.as_os_str().encode_wide()
        .map(|unit| if unit == u16::from(b'/') { u16::from(b'\\') } else { unit }).collect::<Vec<_>>());
    Ok(vec![
        "/i".into(),
        native(package),
        "/qn".into(),
        "/norestart".into(),
        "REBOOT=ReallySuppress".into(),
        "LAUNCH_TRAY_APP=N".into(),
        "RUSTDESK_AGENT_RECOVERY=1".into(),
        "/L*v".into(),
        native(log),
    ])
}

pub fn run(command: &mut Command, timeout: Duration) -> Result<InstallOutcome, String> {
    let mut child = command
        .spawn()
        .map_err(|e| format!("installer launch failed: {e}"))?;
    let process = match super::handoff::ObservedProcess::open(child.id()) {
        Ok(process) => Some(process),
        Err(error) => match child.try_wait() {
            Ok(Some(status)) => return Ok(classify(status.code().unwrap_or(-1))),
            _ => {
                eprintln!("installer process identity unavailable; reconciliation required; pid={}: {error}",child.id());
                return Ok(InstallOutcome::Pending {
                    pid: child.id(),
                    created_at: 0,
                });
            }
        },
    };
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Err(error) => {
                eprintln!("installer result wait failed; reconciliation required: {error}");
                return Ok(InstallOutcome::Pending {
                    pid: child.id(),
                    created_at: process.as_ref().map(|p| p.owner().created_at).unwrap_or(0),
                });
            }
            Ok(Some(status)) => return Ok(classify(status.code().unwrap_or(-1))),
            Ok(None) if Instant::now() >= deadline => {
                return Ok(InstallOutcome::Pending {
                    pid: child.id(),
                    created_at: process.as_ref().map(|p| p.owner().created_at).unwrap_or(0),
                })
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(100)),
        }
    }
}

fn classify(code: i32) -> InstallOutcome {
    match code {
        0 => InstallOutcome::Installed,
        3010 => InstallOutcome::RebootRequired,
        1618 => InstallOutcome::Busy,
        code => InstallOutcome::Failed(code),
    }
}

pub fn read_recorded(root: &Path, step: &str, transaction_id: &str, package_sha256: &str) -> Result<Option<InstallOutcome>, String> {
    use std::{io::Read, os::windows::fs::OpenOptionsExt};
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Receipt {
        schema: u32,
        transaction_id: String,
        package_sha256: String,
        step: String,
        outcome: Option<InstallOutcome>,
    }
    let token = |value: &str, length: usize| value.len() == length
        && value.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
    if !matches!(step, "install" | "rollback") || !token(transaction_id, 32) || !token(package_sha256, 64) {
        return Err("installer receipt binding invalid".into());
    }
    super::workspace::verify(root)?;
    let path = root.join(format!("{step}-result.json"));
    match std::fs::symlink_metadata(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("installer receipt unavailable: {error}")),
        Ok(_) => {}
    }
    let mut outcome = None;
    for (path, is_result) in [(root.join(format!("{step}-started.json")), false), (path, true)] {
        super::workspace::verify_file(&path)?;
        let mut bytes = Vec::new();
        std::fs::OpenOptions::new().read(true).share_mode(1).custom_flags(0x00200000)
            .open(path).map_err(|e| e.to_string())?.take(4097)
            .read_to_end(&mut bytes).map_err(|e| e.to_string())?;
        if bytes.len() > 4096 { return Err("installer receipt too large".into()); }
        let receipt: Receipt = serde_json::from_slice(&bytes).map_err(|_| "installer receipt malformed")?;
        if receipt.schema != 1 || receipt.transaction_id != transaction_id
            || receipt.package_sha256 != package_sha256 || receipt.step != step {
            return Err("installer receipt binding mismatch".into());
        }
        if !is_result && receipt.outcome.is_some() { return Err("installer intent malformed".into()); }
        if is_result {
            outcome = Some(receipt.outcome.ok_or("installer result absent; reconciliation required")?);
        }
    }
    Ok(outcome)
}

pub fn run_recorded(
    command: &mut Command,
    timeout: Duration,
    root: &Path,
    step: &str,
    transaction_id: &str,
    package_sha256: &str,
) -> Result<InstallOutcome, String> {
    use std::{io::{Read, Write}, os::windows::fs::OpenOptionsExt};
    #[derive(serde::Serialize, serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Receipt {
        schema: u32,
        transaction_id: String,
        package_sha256: String,
        step: String,
        outcome: Option<InstallOutcome>,
    }
    let token = |value: &str, length: usize| value.len() == length
        && value.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
    if !matches!(step, "install" | "rollback")
        || !token(transaction_id, 32) || !token(package_sha256, 64) {
        return Err("installer receipt binding invalid".into());
    }
    super::workspace::verify(root)?;
    let intent_path = root.join(format!("{step}-started.json"));
    let result_path = root.join(format!("{step}-result.json"));
    let load = |path: &Path| -> Result<Receipt, String> {
        super::workspace::verify_file(path)?;
        let mut bytes = Vec::new();
        std::fs::OpenOptions::new().read(true).share_mode(1).custom_flags(0x00200000)
            .open(path).map_err(|e| e.to_string())?.take(4097)
            .read_to_end(&mut bytes).map_err(|e| e.to_string())?;
        if bytes.len() > 4096 { return Err("installer receipt too large".into()); }
        let receipt: Receipt = serde_json::from_slice(&bytes).map_err(|_| "installer receipt malformed")?;
        if receipt.schema != 1 || receipt.transaction_id != transaction_id
            || receipt.package_sha256 != package_sha256 || receipt.step != step {
            return Err("installer receipt binding mismatch".into());
        }
        Ok(receipt)
    };
    match std::fs::symlink_metadata(&result_path) {
        Ok(_) => {
            let intent = load(&intent_path)?;
            if intent.outcome.is_some() { return Err("installer intent malformed".into()); }
            return load(&result_path)?.outcome.ok_or_else(|| "installer result absent; reconciliation required".into());
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(format!("installer receipt unavailable: {error}")),
    }
    let mut intent = match std::fs::OpenOptions::new().write(true).create_new(true).share_mode(0)
        .open(&intent_path) {
        Ok(intent) => intent,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            return Err("installer launch already recorded; reconciliation required".into());
        }
        Err(error) => return Err(format!("installer launch record unavailable: {error}")),
    };
    let mut receipt = Receipt { schema: 1, transaction_id: transaction_id.into(), package_sha256: package_sha256.into(), step: step.into(), outcome: None };
    intent.write_all(&serde_json::to_vec(&receipt).map_err(|e| e.to_string())?)
        .and_then(|_| intent.sync_all()).map_err(|e| format!("installer intent persistence: {e}"))?;
    drop(intent);
    super::workspace::verify_file(&intent_path)?;
    // Once launch is possible, any unknown outcome must preserve the intent.
    // Never infer that a missing receipt means the package has not run.
    let outcome = run(command, timeout).map_err(|e| format!("{e}; reconciliation required"))?;
    receipt.outcome = Some(outcome.clone());
    super::state_store::publish(&result_path, &serde_json::to_vec(&receipt).map_err(|e| e.to_string())?)
        .map_err(|e| format!("installer result persistence failed; reconciliation required: {e}"))?;
    super::workspace::verify_file(&result_path)?;
    Ok(outcome)
}
