use super::install_plan::{Artifact, InstallPlan};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

struct Binding {
    transaction_id: String,
    plan_sha256: String,
    edition: String,
    kind: String,
    normal_install_path: String,
    original_agent: Artifact,
    target_agent: Artifact,
}

fn binding(plan: &InstallPlan, target: &Artifact) -> Result<Binding, String> {
    plan.validate()?;
    Ok(Binding {
        transaction_id: plan.transaction_id.clone(),
        plan_sha256: hex::encode(Sha256::digest(serde_json::to_vec(plan).map_err(|e| e.to_string())?)),
        edition: if plan.target_key.ends_with("-sos") { "sos" } else { "standard" }.into(),
        kind: if plan.target_key.contains("-msi-") { "msi" } else { "exe" }.into(),
        normal_install_path: plan.normal_install_path.clone(),
        original_agent: plan.agent_backup.clone(),
        target_agent: target.clone(),
    })
}

fn destination() -> Result<PathBuf, String> {
    Ok(std::env::var_os("ProgramData").map(PathBuf::from).ok_or("ProgramData unavailable")?
        .join("RustDesk").join("RecoveryBootstrap"))
}

pub fn prepare(root: &Path, plan: &InstallPlan, target: &Artifact) -> Result<(), String> {
    prepare_bound(root, &destination()?, &binding(plan, target)?)
}

pub fn synchronize(root: &Path, plan: &InstallPlan, target: &Artifact, installing: bool) -> Result<(), String> {
    synchronize_bound(root, &destination()?, &binding(plan, target)?, installing)
}

pub fn verify_prepared(root: &Path, plan: &InstallPlan, target: &Artifact) -> Result<(), String> {
    prepared(root, &binding(plan, target)?)?;
    Ok(())
}

#[cfg(test)]
pub fn prepare_fixture(
    root: &Path,
    directory: &Path,
    plan: &InstallPlan,
    target: &Artifact,
) -> Result<(), String> {
    prepare_bound(root, directory, &binding(plan, target)?)
}
#[cfg(test)]
pub fn synchronize_fixture(
    root: &Path,
    directory: &Path,
    plan: &InstallPlan,
    target: &Artifact,
    installing: bool,
) -> Result<(), String> {
    synchronize_bound(root, directory, &binding(plan, target)?, installing)
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Prepared {
    schema: u32,
    transaction_id: String,
    plan_sha256: String,
    original_agent: Artifact,
    target_agent: Artifact,
    manifests: Option<(Artifact, Artifact)>,
}

fn read(path: &Path, limit: u64) -> Result<Vec<u8>, String> {
    use std::{io::Read, os::windows::fs::OpenOptionsExt};
    super::workspace::verify_file(path)?;
    let mut bytes = Vec::new();
    std::fs::OpenOptions::new().read(true).share_mode(1).open(path)
        .map_err(|e| e.to_string())?.take(limit + 1).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    if bytes.len() as u64 > limit { return Err("bootstrap update record too large".into()); }
    Ok(bytes)
}

fn save_asset(root: &Path, relative: &str, bytes: &[u8]) -> Result<Artifact, String> {
    use std::io::Write;
    let mut output = std::fs::OpenOptions::new().write(true).create_new(true)
        .open(root.join(relative)).map_err(|e| e.to_string())?;
    output.write_all(bytes).and_then(|_| output.sync_all()).map_err(|e| e.to_string())?;
    Ok(Artifact { relative_path: relative.into(), size: bytes.len() as u64,
        sha256: hex::encode(Sha256::digest(bytes)) })
}

fn prepare_bound(root: &Path, directory: &Path, binding: &Binding) -> Result<(), String> {
    super::workspace::verify(root)?;
    let exists = match std::fs::symlink_metadata(directory) {
        Ok(_) => true,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(error) => return Err(error.to_string()),
    };
    let mut manifests = None;
    if exists {
        super::workspace::verify(directory)?;
        binding.original_agent.verify(root)?;
        binding.target_agent.verify(root)?;
        super::workspace::verify_file(&root.join(&binding.original_agent.relative_path))?;
        super::workspace::verify_file(&root.join(&binding.target_agent.relative_path))?;
        let old_bytes = read(&directory.join("bootstrap.json"), 65536)?;
        let mut manifest: serde_json::Value = serde_json::from_slice(&old_bytes)
            .map_err(|_| "bootstrap update manifest malformed")?;
        if manifest["schema"] != 1 || manifest["agent_sha256"] != binding.original_agent.sha256
            || manifest["edition"] != binding.edition || manifest["package_kind"] != binding.kind
            || manifest["normal_install_path"] != binding.normal_install_path {
            return Err("bootstrap update original identity mismatch".into());
        }
        let _old = lock_entry(directory, &binding.original_agent)?;
        super::workspace::create(&root.join("bootstrap-backup"))?;
        let old = save_asset(root, "bootstrap-backup/original.json", &old_bytes)?;
        manifest["agent_sha256"] = binding.target_agent.sha256.clone().into();
        let new = save_asset(root, "bootstrap-backup/target.json", &serde_json::to_vec(&manifest).map_err(|e| e.to_string())?)?;
        manifests = Some((old, new));
    }
    let prepared = Prepared { schema: 1, transaction_id: binding.transaction_id.clone(),
        plan_sha256: binding.plan_sha256.clone(), original_agent: binding.original_agent.clone(),
        target_agent: binding.target_agent.clone(), manifests };
    save_asset(root, "bootstrap-update.json", &serde_json::to_vec(&prepared).map_err(|e| e.to_string())?)?;
    Ok(())
}

fn lock_entry(directory: &Path, artifact: &Artifact) -> Result<super::package_gate::LockedPackage, String> {
    let path = directory.join("rustdesk-recovery-bootstrap.exe");
    super::workspace::verify_file(&path)?;
    super::package_gate::lock_verified_file(&path, &hbb_common::UpdateTarget {
        size: artifact.size, sha256: artifact.sha256.clone(), ..Default::default()
    })
}

fn prepared(root: &Path, binding: &Binding) -> Result<Prepared, String> {
    super::workspace::verify(root)?;
    let value: serde_json::Value = serde_json::from_slice(&read(&root.join("bootstrap-update.json"), 65536)?)
        .map_err(|_| "bootstrap update preparation malformed")?;
    if value.get("manifests").is_none() {
        return Err("bootstrap update preparation malformed".into());
    }
    let record: Prepared = serde_json::from_value(value)
        .map_err(|_| "bootstrap update preparation malformed")?;
    if record.schema != 1 || record.transaction_id != binding.transaction_id
        || record.plan_sha256 != binding.plan_sha256 || record.original_agent != binding.original_agent
        || record.target_agent != binding.target_agent {
        return Err("bootstrap update preparation binding mismatch".into());
    }
    if let Some((old, new)) = &record.manifests {
        if old.relative_path != "bootstrap-backup/original.json"
            || new.relative_path != "bootstrap-backup/target.json" {
            return Err("bootstrap update manifest paths invalid".into());
        }
        for artifact in [old, new, &binding.original_agent, &binding.target_agent] {
            artifact.verify(root)?;
            super::workspace::verify_file(&root.join(&artifact.relative_path))?;
        }
        for (artifact, digest) in [(old, &binding.original_agent.sha256), (new, &binding.target_agent.sha256)] {
            let value: serde_json::Value = serde_json::from_slice(&read(&root.join(&artifact.relative_path), 65536)?)
                .map_err(|_| "bootstrap update prepared manifest malformed")?;
            if value["schema"] != 1 || value["agent_sha256"] != *digest
                || value["edition"] != binding.edition || value["package_kind"] != binding.kind
                || value["normal_install_path"] != binding.normal_install_path {
                return Err("bootstrap update prepared manifest binding mismatch".into());
            }
        }
    }
    Ok(record)
}

fn synchronize_bound(root: &Path, directory: &Path, binding: &Binding, installing: bool) -> Result<(), String> {
    let record = prepared(root, binding)?;
    let Some((old, new)) = record.manifests else {
        return match std::fs::symlink_metadata(directory) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            _ => Err("bootstrap appeared after preparation".into()),
        };
    };
    super::workspace::verify(directory)?;
    let path = directory.join("bootstrap.json");
    let bytes = read(&path, 65536)?;
    let digest = hex::encode(Sha256::digest(&bytes));
    if ![&old, &new].iter().any(|artifact| artifact.sha256 == digest && artifact.size == bytes.len() as u64) {
        return Err("bootstrap update current manifest drift".into());
    }
    // A crash between the two replacements may leave an old/new pair. Only
    // bytes already bound to this prepared transaction may be reconciled.
    let current = lock_entry(directory, &binding.original_agent)
        .or_else(|_| lock_entry(directory, &binding.target_agent))?;
    drop(current);
    let (agent, manifest) = if installing { (&binding.target_agent, &new) } else { (&binding.original_agent, &old) };
    super::file_restore::restore(root, agent, &directory.join("rustdesk-recovery-bootstrap.exe"))?;
    super::file_restore::restore(root, manifest, &path)?;
    let _verified = lock_entry(directory, agent)?;
    let actual = read(&path, 65536)?;
    if actual.len() as u64 != manifest.size || hex::encode(Sha256::digest(actual)) != manifest.sha256 {
        return Err("bootstrap update publication mismatch".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bootstrap_upgrade_and_rollback_keep_the_fixed_entry_and_budget() {
        let root = std::env::temp_dir().join(format!("agent-bootstrap-update-{}", std::process::id()));
        super::super::workspace::create(&root).unwrap();
        let directory = root.join("RecoveryBootstrap");
        super::super::workspace::create(&directory).unwrap();
        let artifact = |name: &str, bytes: &[u8]| {
            std::fs::write(root.join(name), bytes).unwrap();
            Artifact { relative_path: name.into(), size: bytes.len() as u64, sha256: hex::encode(Sha256::digest(bytes)) }
        };
        let binding = Binding { transaction_id: "a".repeat(32), plan_sha256: "b".repeat(64),
            edition: "standard".into(), kind: "msi".into(), normal_install_path: r"C:\Program Files\RustDesk".into(),
            original_agent: artifact("old.exe", b"old approved agent"),
            target_agent: artifact("new.exe", b"new approved agent"),
        };
        let manifest = serde_json::json!({"schema":1,"agent_sha256":binding.original_agent.sha256,
            "edition":"standard","package_kind":"msi","normal_install_path":binding.normal_install_path});
        let old_manifest = serde_json::to_vec(&manifest).unwrap();
        std::fs::write(directory.join("bootstrap.json"), &old_manifest).unwrap();
        std::fs::write(directory.join("rustdesk-recovery-bootstrap.exe"), b"old approved agent").unwrap();
        std::fs::write(directory.join("budget.json"), b"existing recovery budget").unwrap();
        std::fs::write(directory.join("task.xml"), b"unchanged fixed task").unwrap();
        prepare_bound(&root, &directory, &binding).unwrap();
        assert_eq!(std::fs::read(directory.join("bootstrap.json")).unwrap(), old_manifest);
        synchronize_bound(&root, &directory, &binding, true).unwrap();
        assert_eq!(std::fs::read(directory.join("rustdesk-recovery-bootstrap.exe")).unwrap(), b"new approved agent");
        let new_manifest: serde_json::Value = serde_json::from_slice(&std::fs::read(directory.join("bootstrap.json")).unwrap()).unwrap();
        assert_eq!(new_manifest["agent_sha256"], binding.target_agent.sha256);
        synchronize_bound(&root, &directory, &binding, true).unwrap();
        synchronize_bound(&root, &directory, &binding, false).unwrap();
        assert_eq!(std::fs::read(directory.join("rustdesk-recovery-bootstrap.exe")).unwrap(), b"old approved agent");
        assert_eq!(std::fs::read(directory.join("bootstrap.json")).unwrap(), old_manifest);
        assert_eq!(std::fs::read(directory.join("budget.json")).unwrap(), b"existing recovery budget");
        assert_eq!(std::fs::read(directory.join("task.xml")).unwrap(), b"unchanged fixed task");
        let record = std::fs::read(root.join("bootstrap-update.json")).unwrap();
        let mut incomplete: serde_json::Value = serde_json::from_slice(&record).unwrap();
        incomplete.as_object_mut().unwrap().remove("manifests");
        let incomplete_bytes = serde_json::to_vec(&incomplete).unwrap();
        std::fs::write(root.join("bootstrap-update.json"), &incomplete_bytes).unwrap();
        assert!(matches!(prepared(&root, &binding), Err(error) if error == "bootstrap update preparation malformed"),
            "an omitted manifests field must not mean bootstrap was absent");
        assert_eq!(std::fs::read(root.join("bootstrap-update.json")).unwrap(), incomplete_bytes);
        assert_eq!(std::fs::read(directory.join("bootstrap.json")).unwrap(), old_manifest);
        std::fs::write(root.join("bootstrap-update.json"), &record).unwrap();
        assert!(prepare_bound(&root, &directory, &binding).is_err());
        assert_eq!(std::fs::read(root.join("bootstrap-update.json")).unwrap(), record);

        use std::os::windows::fs::OpenOptionsExt;
        let locked = std::fs::OpenOptions::new().read(true).share_mode(1)
            .open(directory.join("rustdesk-recovery-bootstrap.exe")).unwrap();
        assert!(synchronize_bound(&root, &directory, &binding, true).unwrap_err().contains("publication"));
        assert_eq!(std::fs::read(directory.join("bootstrap.json")).unwrap(), old_manifest);
        assert_eq!(std::fs::read(directory.join("rustdesk-recovery-bootstrap.exe")).unwrap(), b"old approved agent");
        drop(locked);

        // Model a power loss after publishing the binary but before the manifest.
        std::fs::write(directory.join("rustdesk-recovery-bootstrap.exe"), b"new approved agent").unwrap();
        synchronize_bound(&root, &directory, &binding, false).unwrap();
        assert_eq!(std::fs::read(directory.join("rustdesk-recovery-bootstrap.exe")).unwrap(), b"old approved agent");
        std::fs::write(directory.join("rustdesk-recovery-bootstrap.exe"), b"foreign bytes").unwrap();
        assert!(synchronize_bound(&root, &directory, &binding, true).unwrap_err().contains("mismatch"));
        assert_eq!(std::fs::read(directory.join("rustdesk-recovery-bootstrap.exe")).unwrap(), b"foreign bytes");
        assert_eq!(std::fs::read(directory.join("bootstrap.json")).unwrap(), old_manifest);
        std::fs::write(directory.join("rustdesk-recovery-bootstrap.exe"), b"old approved agent").unwrap();
        std::fs::write(directory.join("bootstrap.json"), b"foreign manifest").unwrap();
        assert_eq!(synchronize_bound(&root, &directory, &binding, false).unwrap_err(), "bootstrap update current manifest drift");
        assert_eq!(std::fs::read(directory.join("bootstrap.json")).unwrap(), b"foreign manifest");
        assert_eq!(std::fs::read(root.join("bootstrap-update.json")).unwrap(), record);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn absent_bootstrap_is_not_created_and_late_foreign_directory_is_preserved() {
        let root = std::env::temp_dir().join(format!("agent-bootstrap-absent-{}", std::process::id()));
        super::super::workspace::create(&root).unwrap();
        let directory = root.join("RecoveryBootstrap");
        let artifact = Artifact { relative_path:"unused.exe".into(),size:1,sha256:"c".repeat(64) };
        let mut binding = Binding { transaction_id:"a".repeat(32),plan_sha256:"b".repeat(64),edition:"standard".into(),
            kind:"msi".into(),normal_install_path:r"C:\Program Files\RustDesk".into(),original_agent:artifact.clone(),target_agent:artifact };
        prepare_bound(&root, &directory, &binding).unwrap();
        synchronize_bound(&root, &directory, &binding, true).unwrap();
        synchronize_bound(&root, &directory, &binding, false).unwrap();
        assert!(!directory.exists());
        let record = std::fs::read(root.join("bootstrap-update.json")).unwrap();
        binding.plan_sha256 = "d".repeat(64);
        assert_eq!(synchronize_bound(&root, &directory, &binding, true).unwrap_err(),"bootstrap update preparation binding mismatch");
        binding.plan_sha256 = "b".repeat(64);
        std::fs::create_dir(&directory).unwrap();
        std::fs::write(directory.join("sentinel"),b"retain").unwrap();
        assert_eq!(synchronize_bound(&root, &directory, &binding, true).unwrap_err(),"bootstrap appeared after preparation");
        assert_eq!(std::fs::read(directory.join("sentinel")).unwrap(),b"retain");
        assert_eq!(std::fs::read(root.join("bootstrap-update.json")).unwrap(),record);
        std::fs::remove_dir_all(root).unwrap();
    }
}
