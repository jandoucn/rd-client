use super::{
    handoff::{Handoff, ObservedProcess},
    install_guard::InstallGuard,
    install_plan::InstallPlan, maintenance,
    package_gate::{self, LockedPackage},
    resume_task, state_store,
    transaction::{Phase, Transaction},
    workspace,
};
use sha2::{Digest, Sha256};
use std::{io::Read, path::Path};

pub fn wait_for_release(root: &Path) -> Result<(), String> {
    // The parent still verifies backups and the registered task after readiness.
    // Takeover separately enforces the signed handoff's 300-second lifetime.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(240);
    loop {
        match std::fs::symlink_metadata(root.join("helper-release.json")) {
            Ok(_) => return workspace::verify_file(&root.join("helper-release.json")),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(format!("helper release observation failed: {error}")),
        }
        if std::time::Instant::now() >= deadline {
            return Err("helper release timed out before authorization".into());
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}

fn bounded_read(path: &Path, limit: u64) -> Result<Vec<u8>, String> {
    workspace::verify_file(path)?;
    let file = std::fs::File::open(path).map_err(|e| format!("helper record open: {e}"))?;
    let mut bytes = Vec::new();
    file.take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("helper record read: {e}"))?;
    if bytes.len() as u64 > limit {
        return Err("helper record too large".into());
    }
    Ok(bytes)
}

pub fn prepare_ready(root: &Path) -> Result<(), String> {
    workspace::verify(root)?;
    let plan_bytes = bounded_read(&root.join("prepared.json"), 65536)?;
    let plan = InstallPlan::decode(&plan_bytes)?;
    let transaction = Transaction::decode(&bounded_read(&root.join("transaction.json"), 65536)?)?;
    plan.bind_transaction(&transaction)?;
    if transaction.phase != Phase::PackageVerified {
        return Err("helper prepare phase not package_verified".into());
    }
    for artifact in [
        &plan.package,
        &plan.client_backup,
        &plan.agent_backup,
        &plan.identity_backup,
        &plan.helper,
    ] {
        workspace::verify_file(&root.join(&artifact.relative_path))?;
    }
    plan.verify_assets(root)?;
    let binding = Handoff::decode(&bounded_read(&root.join("handoff.json"), 4096)?)?;
    if binding.transaction_id != plan.transaction_id
        || binding.plan_sha256 != hex::encode(Sha256::digest(&plan_bytes))
    {
        return Err("helper preparation handoff binding mismatch".into());
    }
    let current = ObservedProcess::open(std::process::id())?;
    if current.owner() != &binding.helper {
        return Err("helper process identity does not match prepared owner".into());
    }
    let executable = std::env::current_exe().map_err(|e| e.to_string())?;
    if executable != root.join(&plan.helper.relative_path) {
        return Err("helper executable outside prepared runtime".into());
    }
    let parent = ObservedProcess::open(binding.parent.pid)?;
    if parent.owner() != &binding.parent || !parent.is_alive()? {
        return Err("helper preparation parent identity unavailable".into());
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| "helper clock before epoch".to_owned())?
        .as_secs();
    let receipt = binding.ready_receipt(now)?;
    let path = root.join("helper-ready.json");
    if path.exists() {
        let existing: super::handoff::ReadyReceipt =
            serde_json::from_slice(&bounded_read(&path, 4096)?)
                .map_err(|_| "helper ready record malformed".to_owned())?;
        binding.verify_ready(&existing, now)?;
        return Ok(());
    }
    state_store::publish(
        &path,
        &serde_json::to_vec(&receipt).map_err(|e| e.to_string())?,
    )
    .map_err(|e| format!("helper readiness persistence: {e}"))?;
    workspace::verify_file(&path)
}

pub struct OwnedInstallation {
    _guard: InstallGuard,
    _package: LockedPackage,
    transaction: Transaction,
    root: std::path::PathBuf,
    state_directory: std::path::PathBuf,
    intent_path: std::path::PathBuf,
    plan_sha256: String,
    nonce: String,
    evidence: super::transaction::Evidence,
}

impl OwnedInstallation {
    pub fn finalize(&self) -> Result<(), String> {
        if !matches!(self.transaction.phase, Phase::Committed | Phase::RolledBack) {
            return Err("installation is not verified terminal".into());
        }
        let transaction = Transaction::decode(&bounded_read(&self.root.join("transaction.json"), 65536)?)?;
        if transaction != self.transaction {
            return Err("installation finalization transaction changed".into());
        }
        let bytes = bounded_read(&self.root.join("prepared.json"), 65536)?;
        if hex::encode(Sha256::digest(&bytes)) != self.plan_sha256 {
            return Err("installation finalization plan changed".into());
        }
        let plan = InstallPlan::decode(&bytes)?;
        plan.bind_transaction(&transaction)?;
        let result: serde_json::Value = serde_json::from_slice(&bounded_read(&self.root.join("execution-result.json"), 65536)?)
            .map_err(|_| "installation execution result malformed")?;
        if result["schema"] != 1 || result["transaction_id"] != plan.transaction_id
            || result["command_id"] != serde_json::to_value(&plan.command_id).map_err(|e| e.to_string())?
            || result["phase"] != serde_json::to_value(transaction.phase).map_err(|e| e.to_string())?
            || result["reboot_required"] != false {
            return Err("installation finalization result binding mismatch".into());
        }
        let error = result.get("error").ok_or("installation finalization result incomplete")?;
        if !(error.is_null() || error.is_string())
            || (transaction.phase == Phase::Committed && !error.is_null()) {
            return Err("installation finalization result inconsistent".into());
        }
        let helper = self.root.join(&plan.helper.relative_path);
        resume_task::verify(&resume_task::read_back(&plan.transaction_id)?, &helper, &plan.transaction_id)?;
        maintenance::release_owned(&self.state_directory, &plan.installation_id,
            &plan.transaction_id, &self.plan_sha256, &self.nonce)?;
        resume_task::remove(&plan.transaction_id)
    }

    pub fn transaction(&self) -> &Transaction {
        &self.transaction
    }

    pub fn execute(&mut self, backend: &mut impl super::install_execution::Backend) -> Result<super::install_execution::ExecutionResult, String> {
        let bytes = bounded_read(&self.root.join("prepared.json"), 65536)?;
        if hex::encode(Sha256::digest(&bytes)) != self.plan_sha256 {
            return Err("owned installation plan changed".into());
        }
        let plan = InstallPlan::decode(&bytes)?;
        maintenance::verify(&self.state_directory, &self.intent_path, &plan.installation_id,
            &plan.transaction_id, &self.plan_sha256, &self.nonce)?;
        let path = self.root.join("transaction.json");
        plan.authorize_stop(&self.root, &self.root.join("prepared.json"), &path, &self.transaction, &self.evidence)?;
        super::native_install::remote_stop_preflight(&plan)?;
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| "helper clock before epoch")?.as_secs();
        let result = super::install_execution::execute(&path, &mut self.transaction, &self.evidence, backend, now)?;
        let record = serde_json::json!({"schema":1,"transaction_id":plan.transaction_id,
            "command_id":plan.command_id,"phase":result.phase,"error":result.error,
            "reboot_required":result.reboot_required});
        state_store::publish(&self.root.join("execution-result.json"),
            &serde_json::to_vec(&record).map_err(|e| e.to_string())?)
            .map_err(|e| format!("helper execution result persistence: {e}"))?;
        Ok(result)
    }

    pub fn resume(&mut self, backend: &mut impl super::install_execution::Backend) -> Result<super::install_execution::ExecutionResult, String> {
        let bytes = bounded_read(&self.root.join("prepared.json"), 65536)?;
        if hex::encode(Sha256::digest(&bytes)) != self.plan_sha256 {
            return Err("owned installation plan changed".into());
        }
        let plan = InstallPlan::decode(&bytes)?;
        maintenance::verify(&self.state_directory, &self.intent_path, &plan.installation_id,
            &plan.transaction_id, &self.plan_sha256, &self.nonce)?;
        if matches!(self.transaction.phase, Phase::Committed | Phase::RolledBack | Phase::RollbackFailed) {
            let transaction = Transaction::decode(&bounded_read(&self.root.join("transaction.json"), 65536)?)?;
            if transaction != self.transaction {
                return Err("installation resume snapshot mismatch".into());
            }
            plan.bind_transaction(&transaction)?;
            let path = self.root.join("execution-result.json");
            match std::fs::symlink_metadata(&path) {
                Ok(_) => {
                    let result: serde_json::Value = serde_json::from_slice(&bounded_read(&path, 65536)?)
                        .map_err(|_| "installation terminal result malformed")?;
                    let command = serde_json::to_value(&plan.command_id).map_err(|e| e.to_string())?;
                    let phase = serde_json::to_value(transaction.phase).map_err(|e| e.to_string())?;
                    if result.get("schema") != Some(&serde_json::json!(1))
                        || result.get("transaction_id").and_then(|value| value.as_str()) != Some(plan.transaction_id.as_str())
                        || result.get("command_id") != Some(&command)
                        || result.get("phase") != Some(&phase)
                        || result.get("reboot_required").and_then(|value| value.as_bool()) != Some(false) {
                        return Err("installation terminal result binding mismatch".into());
                    }
                    let error = result.get("error").ok_or("installation terminal result incomplete")?;
                    if !(error.is_null() || error.is_string())
                        || (transaction.phase == Phase::Committed && !error.is_null()) {
                        return Err("installation terminal result inconsistent".into());
                    }
                    return Ok(super::install_execution::ExecutionResult {
                        phase: transaction.phase,
                        error: error.as_str().map(str::to_owned),
                        reboot_required: false,
                    });
                }
                // A crash can persist the terminal phase before its result receipt.
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(format!("installation terminal result observation failed: {error}")),
            }
        }
        if self.transaction.phase == Phase::StoppingServices {
            super::native_install::remote_stop_preflight(&plan)?;
        }
        let outcome = if self.transaction.phase == Phase::Applying {
            super::installer_process::read_recorded(&self.root, "install", &plan.transaction_id, &plan.package.sha256)?
        } else { None };
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| "helper clock before epoch")?.as_secs();
        let result = super::install_execution::resume(&self.root.join("transaction.json"),
            &mut self.transaction, &self.evidence, backend, outcome, now)?;
        let record = serde_json::json!({"schema":1,"transaction_id":plan.transaction_id,
            "command_id":plan.command_id,"phase":result.phase,"error":result.error,
            "reboot_required":result.reboot_required});
        state_store::publish(&self.root.join("execution-result.json"),
            &serde_json::to_vec(&record).map_err(|e| e.to_string())?)
            .map_err(|e| format!("helper resumed result persistence: {e}"))?;
        Ok(result)
    }
}

#[cfg(test)]
pub fn resume_with_fixture_key(root: &Path, key_id: &str, public: &[u8]) -> Result<OwnedInstallation, String> {
    resume_checked(root, Some((key_id, public)))
}

pub fn resume_installation(root: &Path) -> Result<OwnedInstallation, String> {
    resume_checked(root, None)
}

fn resume_checked(root: &Path, fixture_key: Option<(&str, &[u8])>) -> Result<OwnedInstallation, String> {
    workspace::verify(root)?;
    let bytes = bounded_read(&root.join("prepared.json"), 65536)?;
    let plan = InstallPlan::decode(&bytes)?;
    let binding = Handoff::decode(&bounded_read(&root.join("handoff.json"), 4096)?)?;
    if binding.transaction_id != plan.transaction_id || binding.plan_sha256 != hex::encode(Sha256::digest(&bytes)) {
        return Err("helper resume preparation binding mismatch".into());
    }
    if std::env::current_exe().map_err(|e| e.to_string())? != root.join(&plan.helper.relative_path) {
        return Err("resume executable outside prepared runtime".into());
    }
    if ObservedProcess::owner_alive(&binding.helper)? {
        return Err("original helper still running".into());
    }
    let Some(guard) = InstallGuard::try_acquire().map_err(|e| format!("helper resume guard: {e}"))? else {
        return Err("helper resume guard busy".into());
    };
    let transaction = Transaction::decode(&bounded_read(&root.join("transaction.json"), 65536)?)?;
    plan.bind_transaction(&transaction)?;
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| "helper clock before epoch")?.as_secs();
    if matches!(transaction.recovery_step(false, now), super::transaction::RecoveryStep::ManualReconciliation | super::transaction::RecoveryStep::Prepare) {
        return Err("transaction not ready for automatic resume".into());
    }
    let ready: super::handoff::ReadyReceipt = serde_json::from_slice(&bounded_read(&root.join("helper-ready.json"), 4096)?)
        .map_err(|_| "helper readiness malformed")?;
    let release: super::handoff::ReleaseReceipt = serde_json::from_slice(&bounded_read(&root.join("helper-release.json"), 4096)?)
        .map_err(|_| "helper release malformed")?;
    // Validate the historical handoff at its release time, not at reboot time.
    if release.released_at > now { return Err("helper release from future".into()); }
    binding.verify_release(&ready, &release, &binding.helper, release.released_at)?;
    plan.verify_assets(root)?;
    for artifact in [&plan.package, &plan.client_backup, &plan.agent_backup, &plan.identity_backup, &plan.helper] {
        workspace::verify_file(&root.join(&artifact.relative_path))?;
    }
    let manifest: hbb_common::UpdateManifest = serde_json::from_slice(&bounded_read(&root.join("approved-manifest.json"), 262144)?)
        .map_err(|_| "installation approved manifest malformed")?;
    let target = match fixture_key {
        Some((key, public)) => package_gate::verify_with_key(&manifest, &plan.target_key, plan.original_build_seq, key, public)?,
        None => package_gate::verify(&manifest, &plan.target_key, plan.original_build_seq)?,
    };
    let (version, build, _) = base::update::update_target_release(&manifest, target).map_err(|e| e.to_string())?;
    if version != plan.target_version || build != plan.target_build_seq || target.size != plan.package.size || target.sha256 != plan.package.sha256 {
        return Err("installation approved package differs from prepared target".into());
    }
    let package = package_gate::lock_verified_file(&root.join(&plan.package.relative_path), target)?;
    let helper = root.join(&plan.helper.relative_path);
    let definition = String::from_utf8(bounded_read(&root.join("resume-definition.xml"), 16384)?)
        .map_err(|_| "resume task definition encoding invalid")?;
    resume_task::verify(&definition, &helper, &plan.transaction_id)?;
    resume_task::verify(&resume_task::read_back(&plan.transaction_id)?, &helper, &plan.transaction_id)?;
    let state_directory = std::env::var_os("ProgramData").map(std::path::PathBuf::from)
        .unwrap_or_else(|| r"C:\ProgramData".into()).join("RustDesk/ManagementAgent/state");
    let intent_path = hbb_common::config::Config2::file();
    #[cfg(test)]
    let (state_directory, intent_path) = if fixture_key.is_some() {
        (root.join("state"), root.join("intent.toml"))
    } else { (state_directory, intent_path) };
    maintenance::verify(&state_directory, &intent_path, &plan.installation_id,
        &plan.transaction_id, &binding.plan_sha256, &binding.nonce)?;
    let evidence = super::transaction::Evidence {
        package_verified:true,backups_verified:true,helper_verified:true,resume_task_verified:true,
        handoff_verified:true,maintenance_owned:true,services_stopped:false,target_verified:false,
        identity_unchanged:false,client_ready:false,agent_ready:false,rollback_verified:false,locally_blocked:false,
    };
    Ok(OwnedInstallation { _guard:guard,_package:package,transaction,root:root.into(),state_directory,
        intent_path,plan_sha256:binding.plan_sha256,nonce:binding.nonce,evidence })
}

pub fn take_over(root: &Path) -> Result<OwnedInstallation, String> {
    take_over_checked(root, None, false)
}

#[cfg(test)]
pub fn take_over_with_fixture_key(
    root: &Path,
    key_id: &str,
    public: &[u8],
) -> Result<OwnedInstallation, String> {
    take_over_checked(root, Some((key_id, public)), false)
}

#[cfg(test)]
pub fn take_over_native_fixture(root: &Path, public: &[u8]) -> Result<OwnedInstallation, String> {
    take_over_checked(root, Some(("fixture-key", public)), true)
}

fn take_over_checked(
    root: &Path,
    fixture_key: Option<(&str, &[u8])>,
    native_fixture: bool,
) -> Result<OwnedInstallation, String> {
    workspace::verify(root)?;
    let bytes = bounded_read(&root.join("prepared.json"), 65536)?;
    let plan = InstallPlan::decode(&bytes)?;
    let binding = Handoff::decode(&bounded_read(&root.join("handoff.json"), 4096)?)?;
    if binding.transaction_id != plan.transaction_id
        || binding.plan_sha256 != hex::encode(Sha256::digest(&bytes))
    {
        return Err("helper takeover preparation binding mismatch".into());
    }
    let current = ObservedProcess::open(std::process::id())?;
    if current.owner() != &binding.helper
        || std::env::current_exe().map_err(|e| e.to_string())?
            != root.join(&plan.helper.relative_path)
    {
        return Err("helper takeover process identity mismatch".into());
    }
    let ready: super::handoff::ReadyReceipt =
        serde_json::from_slice(&bounded_read(&root.join("helper-ready.json"), 4096)?)
            .map_err(|_| "helper readiness malformed".to_owned())?;
    let released: super::handoff::ReleaseReceipt =
        serde_json::from_slice(&bounded_read(&root.join("helper-release.json"), 4096)?)
            .map_err(|_| "helper release malformed".to_owned())?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| "helper clock before epoch".to_owned())?
        .as_secs();
    binding.verify_release(&ready, &released, current.owner(), now)?;
    let Some(guard) =
        InstallGuard::try_acquire().map_err(|e| format!("helper install guard: {e}"))?
    else {
        return Err("helper install guard busy".into());
    };
    let transaction_path = root.join("transaction.json");
    let transaction = Transaction::decode(&bounded_read(&transaction_path, 65536)?)?;
    let manifest: hbb_common::UpdateManifest =
        serde_json::from_slice(&bounded_read(&root.join("approved-manifest.json"), 262144)?)
            .map_err(|_| "installation approved manifest malformed".to_owned())?;
    let target = match fixture_key {
        Some((key, public)) => package_gate::verify_with_key(
            &manifest,
            &plan.target_key,
            plan.original_build_seq,
            key,
            public,
        )?,
        None => package_gate::verify(&manifest, &plan.target_key, plan.original_build_seq)?,
    };
    let (version, build, _) =
        base::update::update_target_release(&manifest, target).map_err(|e| e.to_string())?;
    if version != plan.target_version
        || build != plan.target_build_seq
        || target.size != plan.package.size
        || target.sha256.to_ascii_lowercase() != plan.package.sha256
    {
        return Err("installation approved package differs from prepared target".into());
    }
    let package =
        package_gate::lock_verified_file(&root.join(&plan.package.relative_path), target)?;
    let helper_path = root.join(&plan.helper.relative_path);
    let definition = String::from_utf8(bounded_read(&root.join("resume-definition.xml"), 16384)?)
        .map_err(|_| "resume task definition encoding invalid".to_owned())?;
    resume_task::verify(&definition, &helper_path, &plan.transaction_id)?;
    let registered = resume_task::read_back(&plan.transaction_id)?;
    resume_task::verify(&registered, &helper_path, &plan.transaction_id)?;
    let state_directory = std::env::var_os("ProgramData")
        .map(std::path::PathBuf::from).unwrap_or_else(|| r"C:\ProgramData".into())
        .join("RustDesk/ManagementAgent/state");
    let intent_path = hbb_common::config::Config2::file();
    #[cfg(test)]
    let (state_directory, intent_path) = if fixture_key.is_some() && !native_fixture {
        (root.join("state"), root.join("intent.toml"))
    } else { (state_directory, intent_path) };
    maintenance::verify(&state_directory, &intent_path, &plan.installation_id,
        &plan.transaction_id, &binding.plan_sha256, &binding.nonce)?;
    let evidence = super::transaction::Evidence {
        package_verified: true,
        backups_verified: true,
        helper_verified: true,
        resume_task_verified: true,
        handoff_verified: true,
        maintenance_owned: true,
        services_stopped: false,
        target_verified: false,
        identity_unchanged: false,
        client_ready: false,
        agent_ready: false,
        rollback_verified: false,
        locally_blocked: false,
    };
    for artifact in [
        &plan.package,
        &plan.client_backup,
        &plan.agent_backup,
        &plan.identity_backup,
        &plan.helper,
    ] {
        workspace::verify_file(&root.join(&artifact.relative_path))?;
    }
    plan.authorize_stop(
        root,
        &root.join("prepared.json"),
        &transaction_path,
        &transaction,
        &evidence,
    )?;
    Ok(OwnedInstallation {
        _guard: guard,
        _package: package,
        transaction,
        root: root.into(),
        state_directory,
        intent_path,
        plan_sha256: binding.plan_sha256,
        nonce: binding.nonce,
        evidence,
    })
}
