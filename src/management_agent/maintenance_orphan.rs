//! Maintenance markers whose transaction already ended.
//!
//! `state\maintenance.json` halts every local recovery (R1 to R4) while an installation owns
//! the machine. Its owner removes it at the end, except when the transaction ends in
//! `rollback_failed` (nothing resumes such a transaction) or the owner dies after its terminal
//! phase. Such a marker would keep the device unrepaired forever, so it is reported with its
//! transaction's phase and released after a grace period. R4 still refuses to retry a bundle
//! whose attempt did not commit, so a release cannot start a repair loop.
use super::{maintenance, transaction::Phase, transaction::Transaction};
use std::path::Path;

/// The owner may still be doing terminal cleanup, and the backend gets to see the halted state
/// before it clears.
pub const GRACE_SECONDS: u64 = 1800;

#[derive(Debug, PartialEq, Eq)]
pub enum Owner {
    /// The bound transaction is still in progress: the marker stays.
    Active,
    /// The bound transaction ended `age` seconds ago in `phase`.
    Ended { phase: &'static str, age: u64 },
}

/// Pure: a marker is orphaned once its transaction reached a terminal phase.
pub fn owner(phase: Phase, updated_at: u64, now: u64) -> Owner {
    let phase = match phase {
        Phase::Committed => "committed",
        Phase::RolledBack => "rolled_back",
        Phase::RollbackFailed => "rollback_failed",
        _ => return Owner::Active,
    };
    Owner::Ended {
        phase,
        age: now.saturating_sub(updated_at),
    }
}

fn token(value: &str) -> bool {
    value.len() == 32 && value.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn bound(state: &Path) -> Result<(maintenance::Binding, Option<Transaction>), String> {
    let binding = maintenance::binding(state)?;
    if !token(&binding.transaction_id) {
        return Err("maintenance record malformed".into());
    }
    let agent = state.parent().ok_or("maintenance path invalid")?;
    let transaction = maintenance::read_record(
        &agent
            .join("transactions")
            .join(&binding.transaction_id)
            .join("transaction.json"),
    )
    .ok()
    .and_then(|bytes| Transaction::decode(&bytes).ok())
    .filter(|transaction| transaction.id == binding.transaction_id);
    Ok((binding, transaction))
}

fn phase_name(phase: Phase) -> String {
    serde_json::to_value(phase)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_else(|| "unknown".into())
}

/// Status detail for a present marker, `None` without one. Lets the backend tell an update in
/// progress from a device halted by a leftover marker.
pub fn observe(state: &Path, now: u64) -> Option<serde_json::Value> {
    match std::fs::symlink_metadata(state.join("maintenance.json")) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return None,
        _ => {}
    }
    Some(match bound(state) {
        Ok((binding, Some(transaction))) => serde_json::json!({
            "transaction_id": binding.transaction_id,
            "phase": phase_name(transaction.phase),
            "age_seconds": now.saturating_sub(transaction.updated_at),
        }),
        Ok((binding, None)) => serde_json::json!({
            "transaction_id": binding.transaction_id, "phase": "unknown", "age_seconds": null
        }),
        Err(_) => serde_json::json!({"transaction_id": null, "phase": "unknown", "age_seconds": null}),
    })
}

fn windows_installer_busy() -> bool {
    #[link(name = "kernel32")]
    extern "system" {
        fn OpenMutexW(access: u32, inherit: i32, name: *const u16) -> *mut std::ffi::c_void;
        fn CloseHandle(handle: *mut std::ffi::c_void) -> i32;
    }
    let name = "Global\\_MSIExecute"
        .encode_utf16()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let handle = unsafe { OpenMutexW(0x0010_0000, 0, name.as_ptr()) };
    if handle.is_null() {
        // Present but not accessible still means an installation is running.
        return std::io::Error::last_os_error().raw_os_error() == Some(5);
    }
    unsafe { CloseHandle(handle) };
    true
}

/// The rescue status, first-install and reconnect workers wake on the same tick and hold the
/// install guard for milliseconds; a single attempt kept colliding with them on a real device and
/// never got the guard. Wait briefly instead; a guard held for longer still means busy.
pub fn install_guard_patiently() -> Result<Option<super::install_guard::InstallGuard>, String> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        match super::install_guard::InstallGuard::try_acquire() {
            Ok(Some(guard)) => return Ok(Some(guard)),
            Ok(None) if std::time::Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(200))
            }
            Ok(None) => return Ok(None),
            Err(_) => return Err("install guard unavailable".into()),
        }
    }
}

/// Releases a marker whose transaction ended at least `GRACE_SECONDS` ago and returns that
/// phase. Holds the installation guard and refuses while Windows Installer runs, so no owner is
/// mid-flight. Local gates (disable-agent, disable-recovery, user stop intent) are untouched and
/// keep applying on their own.
pub fn release(state: &Path, now: u64) -> Result<Option<&'static str>, String> {
    match std::fs::symlink_metadata(state.join("maintenance.json")) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("maintenance observation failed".into()),
        Ok(_) => {}
    }
    let Some(_guard) = install_guard_patiently()? else {
        return Err("installation busy".into());
    };
    if windows_installer_busy() {
        return Err("Windows Installer busy".into());
    }
    let (binding, Some(transaction)) = bound(state)? else {
        return Ok(None);
    };
    let Owner::Ended { phase, age } = owner(transaction.phase, transaction.updated_at, now) else {
        return Ok(None);
    };
    if age < GRACE_SECONDS {
        return Ok(None);
    }
    #[derive(serde::Deserialize)]
    struct State {
        agent_installation_id: String,
    }
    let current: State = serde_json::from_slice(&maintenance::read_record(&state.join("agent.json"))?)
        .map_err(|_| "installation state malformed")?;
    if current.agent_installation_id != binding.installation_id {
        return Err("maintenance installation mismatch".into());
    }
    maintenance::release_owned(
        state,
        &binding.installation_id,
        &binding.transaction_id,
        &binding.plan_sha256,
        &binding.nonce,
    )?;
    Ok(Some(phase))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_terminal_transactions_orphan_their_marker() {
        for phase in [
            Phase::Planned,
            Phase::PackageVerified,
            Phase::Prepared,
            Phase::StoppingServices,
            Phase::ServicesStopped,
            Phase::Applying,
            Phase::Validating,
            Phase::RollingBack,
        ] {
            assert_eq!(owner(phase, 100, 5000), Owner::Active, "{phase:?}");
        }
        assert_eq!(
            owner(Phase::RollbackFailed, 100, 5000),
            Owner::Ended { phase: "rollback_failed", age: 4900 }
        );
        assert_eq!(
            owner(Phase::Committed, 100, 50),
            Owner::Ended { phase: "committed", age: 0 }
        );
        assert_eq!(
            owner(Phase::RolledBack, 0, GRACE_SECONDS),
            Owner::Ended { phase: "rolled_back", age: GRACE_SECONDS }
        );
    }

    #[test]
    fn phases_are_reported_in_wire_form() {
        assert_eq!(phase_name(Phase::RollbackFailed), "rollback_failed");
        assert_eq!(phase_name(Phase::Applying), "applying");
    }

    #[test]
    fn transaction_ids_are_lowercase_hex() {
        assert!(token(&"a1".repeat(16)));
        assert!(!token(&"A1".repeat(16)));
        assert!(!token("..\\x"));
    }
}
