use std::{fs::File, io::Read, path::{Path, PathBuf}};
use sha2::{Digest, Sha256};
use super::{package_gate, rescue_assets, workspace};

pub struct Evidence {
    pub installation_id: String,
    pub target: hbb_common::UpdateTarget,
    pub package: PathBuf,
    pub locks: Vec<File>,
    pub package_lock: package_gate::LockedPackage,
}

fn read_locked(path: &Path, locks: &mut Vec<File>) -> Result<Vec<u8>, String> {
    let mut file = super::purge_discovery::hold(path)?;
    let mut bytes = Vec::new();
    (&mut file).take(262145).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    if bytes.len() > 262144 { return Err("purge evidence too large".into()); }
    locks.push(file);
    Ok(bytes)
}

pub(super) fn installed() -> Result<Evidence, String> {
    resolve(&super::system_dirs::get().agent(), &[
        super::system_dirs::get().local_service_roaming().join(r"RustDesk\config\RustDesk.toml"),
        super::system_dirs::get().system_profile_roaming().join(r"RustDesk\config\RustDesk.toml"),
    ], None)
}

fn resolve(agent: &Path, candidates: &[PathBuf; 2], public: Option<&[u8]>) -> Result<Evidence, String> {
    let mut selected = None;
    for path in candidates {
        match std::fs::symlink_metadata(path) {
            Err(error) if matches!(error.raw_os_error(), Some(2 | 3)) => continue,
            Err(error) => return Err(format!("purge fixed identity query unknown: {error}")),
            Ok(_) => {},
        }
        // Every present profile must be trusted; do not silently skip a foreign
        // identity or an unreadable profile in favor of the other one.
        let evidence = verify(agent, path, public)?;
        if selected.is_some() { return Err("purge fixed identity ambiguous".into()); }
        selected = Some(evidence);
    }
    selected.ok_or_else(|| "purge fixed identity absent".into())
}

#[cfg(test)]
pub(super) fn resolve_fixture(agent: &Path, candidates: &[PathBuf; 2], public: &[u8]) -> Result<Evidence, String> {
    resolve(agent, candidates, Some(public))
}

#[cfg(test)]
pub(super) fn fixture(agent: &Path, identity: &Path, public: Option<&[u8]>) -> Result<Evidence, String> {
    verify(agent, identity, public)
}

fn verify(agent: &Path, identity: &Path, public: Option<&[u8]>) -> Result<Evidence, String> {
    let mut locks = Vec::new();
    let state_path = agent.join("state/agent.json");
    workspace::verify_restore_target(&state_path, false)?;
    let state: serde_json::Value = serde_json::from_slice(&read_locked(&state_path, &mut locks)?)
        .map_err(|_| "purge state malformed")?;
    let key = match state["edition"].as_str() {
        Some("standard") => "windows-x86_64-msi-standard",
        Some("sos") => "windows-x86_64-msi-sos",
        _ => return Err("purge edition invalid".into()),
    };
    if state["package_kind"] != "msi" || state["target_key"] != key { return Err("purge package kind/edition mismatch".into()); }
    let installation = state["agent_installation_id"].as_str().ok_or("purge installation missing")?;
    let transactions = agent.join("transactions");
    match std::fs::read_dir(&transactions) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {},
        Err(e) => return Err(format!("purge transactions unknown: {e}")),
        Ok(entries) => for (index, entry) in entries.enumerate() {
            if index >= 1024 { return Err("purge transaction limit".into()); }
            let path = entry.map_err(|e| e.to_string())?.path().join("transaction.json");
            workspace::verify_file(&path)?;
            let transaction = super::transaction::Transaction::decode(&read_locked(&path, &mut locks)?)?;
            if !matches!(transaction.phase, super::transaction::Phase::Committed | super::transaction::Phase::RolledBack) {
                return Err("purge active transaction".into());
            }
        },
    }
    workspace::verify_restore_target(identity, true)?;
    let current_identity = read_locked(identity, &mut locks)?;
    let root = agent.join("rescue/current");
    let bundle: serde_json::Value = serde_json::from_slice(&read_locked(&root.join("bundle.json"), &mut locks)?)
        .map_err(|_| "purge bundle malformed")?;
    let raw: serde_json::Value = serde_json::from_slice(&read_locked(&root.join("approved-manifest.json"), &mut locks)?)
        .map_err(|_| "purge manifest malformed")?;
    let verified = rescue_assets::verify_readonly_status(&root, installation, key, public)?;
    if bundle["identity_sha256"] != hex::encode(Sha256::digest(&current_identity)) { return Err("purge current identity mismatch".into()); }
    if package_gate::package_reference(&raw, key)? != verified["source_reference"].as_str().ok_or("purge reference missing")? {
        return Err("purge approved reference mismatch".into());
    }
    let manifest = serde_json::from_value(raw).map_err(|_| "purge manifest invalid")?;
    let build = verified["build_seq"].as_u64().ok_or("purge build missing")?;
    let target = signed_target(&manifest, key, build, public)?.clone();
    let package = root.join(package_gate::cache_name(&target.primary, "msi")?);
    let package_lock = package_gate::lock_verified_file(&package, &target)?;
    Ok(Evidence { installation_id: installation.to_owned(), target, package, locks, package_lock })
}

fn signed_target<'a>(manifest: &'a hbb_common::UpdateManifest, key: &str, build: u64, public: Option<&[u8]>) -> Result<&'a hbb_common::UpdateTarget, String> {
    #[cfg(test)]
    if let Some(public) = public { return package_gate::verify_with_key(manifest, key, build, "fixture-key", public); }
    let _ = public;
    package_gate::verify(manifest, key, build)
}
