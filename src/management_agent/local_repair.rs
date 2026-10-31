use super::{
    install_execution::{self, Backend, InstallOutcome, Validation},
    install_plan::Artifact,
    local_repair_plan::LocalRepairPlan,
    package_gate,
    service_control::{self, Contract},
    transaction::{Evidence, Phase, Transaction},
    workspace,
};
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use windows_service::{
    service::{ServiceAccess, ServiceStartType, ServiceState},
    service_manager::{ServiceManager, ServiceManagerAccess},
};

fn agent_root() -> PathBuf {
    crate::management_agent::system_dirs::get().agent()
}
fn normal() -> PathBuf {
    crate::management_agent::system_dirs::get().normal_install()
}

#[path = "local_repair_total_loss_runtime.rs"]
mod total_loss_runtime;

pub(super) fn prepare_total_loss(id: &str, public: Option<&[u8]>) -> Result<PathBuf, String> {
    total_loss_runtime::prepare(id, public)
}

pub fn execute(
    path: &Path,
    transaction: &mut super::transaction::Transaction,
    evidence: &super::transaction::Evidence,
    backend: &mut impl super::install_execution::Backend,
    now: u64,
) -> Result<super::install_execution::ExecutionResult, String> {
    install_execution::execute(path, transaction, evidence, backend, now)
}

fn dispatch(
    path: &Path,
    tx: &mut Transaction,
    proof: &Evidence,
    backend: &mut impl Backend,
    time: u64,
    resume: bool,
    outcome: Option<InstallOutcome>,
    preflight: impl FnOnce() -> Result<(), String>,
) -> Result<install_execution::ExecutionResult, String> {
    if resume {
        resume_authorized(tx, time)?;
    }
    if tx.phase == Phase::StoppingServices {
        preflight()?;
    }
    super::local_recovery_events::repair_dispatch(path, tx, proof, backend, time, resume, outcome)
}

fn resume_authorized(tx: &Transaction, time: u64) -> Result<(), String> {
    match tx.recovery_step(false, time) {
        super::transaction::RecoveryStep::Reconcile
        | super::transaction::RecoveryStep::Rollback => Ok(()),
        super::transaction::RecoveryStep::ReportOnly => {
            Err("local repair terminal evidence retained; no automatic re-execution".into())
        }
        _ => Err("local repair resume requires manual reconciliation".into()),
    }
}

#[cfg(test)]
pub fn event_dispatch_fixture(
    path: &Path,
    tx: &mut Transaction,
    proof: &Evidence,
    backend: &mut impl Backend,
    time: u64,
    resume: bool,
    outcome: Option<InstallOutcome>,
) -> Result<install_execution::ExecutionResult, String> {
    dispatch(path, tx, proof, backend, time, resume, outcome, || Ok(()))
}

fn require_optin(state: &Path) -> Result<(), String> {
    let marker = state.join("service-recovery-enabled");
    match std::fs::symlink_metadata(&marker) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err("local repair recovery opt-in revoked".into())
        }
        Err(_) => return Err("local repair recovery opt-in unreadable".into()),
        Ok(_) => {}
    }
    workspace::verify_restore_target(&marker, false)
        .map_err(|error| format!("local repair recovery opt-in invalid: {error}"))
}

fn preflight_owned(
    state: &Path,
    intent: &Path,
    plan: &LocalRepairPlan,
    binding: &super::handoff::Handoff,
    activity: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    super::maintenance::verify(
        state,
        intent,
        &plan.installation_id,
        &plan.transaction_id,
        &binding.plan_sha256,
        &binding.nonce,
    )?;
    require_optin(state)?;
    activity()
}

#[cfg(test)]
pub fn dispatch_fixture(
    path: &Path,
    tx: &mut Transaction,
    backend: &mut impl Backend,
    time: u64,
    resume: bool,
    outcome: Option<InstallOutcome>,
    state: &Path,
    intent: &Path,
    plan: &LocalRepairPlan,
    binding: &super::handoff::Handoff,
    activity: impl FnOnce() -> Result<(), String>,
) -> Result<install_execution::ExecutionResult, String> {
    plan.bind(tx)?;
    dispatch(
        path,
        tx,
        &evidence(),
        backend,
        time,
        resume,
        outcome,
        || preflight_owned(state, intent, plan, binding, activity),
    )
}

fn now() -> Result<u64, String> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|t| t.as_secs())
        .map_err(|_| "local repair clock invalid".into())
}
fn token(id: &str) -> bool {
    id.len() == 32
        && id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn root(id: &str) -> Result<PathBuf, String> {
    if !token(id) {
        return Err("local repair transaction ID invalid".into());
    }
    Ok(agent_root().join("transactions").join(id))
}
pub(super) fn capacity_admission(
    root: &Path,
    own: Option<&Path>,
    reserve: bool,
) -> Result<(), String> {
    workspace::verify(root)?;
    let mut count = 0usize;
    let mut self_seen = false;
    for entry in std::fs::read_dir(root).map_err(|e| e.to_string())? {
        let path = entry.map_err(|e| e.to_string())?.path();
        count += 1;
        if count > 64 || reserve && count >= 64 {
            return Err("local repair transaction capacity exceeded; preserve history".into());
        }
        if own == Some(path.as_path()) {
            self_seen = true;
        }
    }
    if own.is_some() && !self_seen {
        return Err("local repair own transaction absent".into());
    }
    Ok(())
}
#[cfg(test)]
pub fn capacity_fixture(root: &Path, own: Option<&Path>, reserve: bool) -> Result<(), String> {
    capacity_admission(root, own, reserve)
}
fn read(path: &Path, limit: u64) -> Result<Vec<u8>, String> {
    workspace::verify_file(path)?;
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .map_err(|e| e.to_string())?
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > limit {
        return Err("local repair record too large".into());
    }
    Ok(bytes)
}
fn save(path: &Path, value: &impl serde::Serialize) -> Result<(), String> {
    super::state_store::publish(path, &serde_json::to_vec(value).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())
}
fn copy(directory: &Path, source: &Path, name: &str, limit: u64) -> Result<Artifact, String> {
    use std::os::windows::fs::OpenOptionsExt;
    #[cfg(not(test))]
    let fixture_helper = false;
    #[cfg(test)]
    let fixture_helper = name == "runtime/helper.exe"
        && source == std::env::current_exe().map_err(|e| e.to_string())?;
    if fixture_helper {
        use std::os::windows::fs::MetadataExt;
        for ancestor in source.ancestors() {
            if std::fs::symlink_metadata(ancestor)
                .map_err(|e| e.to_string())?
                .file_attributes()
                & 0x400
                != 0
            {
                return Err("local repair helper reparse path refused".into());
            }
        }
    } else {
        workspace::verify_restore_target(
            source,
            name.contains("identity") || name.contains("config"),
        )
        .map_err(|error| format!("{name}: {error}"))?;
    }
    let mut input = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(source)
        .map_err(|e| e.to_string())?;
    let size = input.metadata().map_err(|e| e.to_string())?.len();
    if size == 0 || size > limit {
        return Err("local repair asset size invalid".into());
    }
    let mut output = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(directory.join(name))
        .map_err(|e| e.to_string())?;
    let mut hash = Sha256::new();
    let mut total = 0u64;
    let mut buffer = [0u8; 65536];
    loop {
        let count = input.read(&mut buffer).map_err(|e| e.to_string())?;
        if count == 0 {
            break;
        }
        total += count as u64;
        if total > size {
            return Err("local repair source changed".into());
        }
        output
            .write_all(&buffer[..count])
            .map_err(|e| e.to_string())?;
        hash.update(&buffer[..count]);
    }
    output.sync_all().map_err(|e| e.to_string())?;
    drop(output);
    if total != size {
        return Err("local repair source changed".into());
    }
    workspace::verify_file(&directory.join(name))?;
    Ok(Artifact {
        relative_path: name.into(),
        size,
        sha256: hex::encode(hash.finalize()),
    })
}
fn normal_missing() -> Result<(), String> {
    match std::fs::symlink_metadata(normal()) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err("local repair normal state unknown".into()),
        Ok(_) => Err("local repair requires explicit missing normal directory".into()),
    }
}
fn target<'a>(
    manifest: &'a hbb_common::UpdateManifest,
    key: &str,
    build: u64,
    public: Option<&[u8]>,
) -> Result<&'a hbb_common::UpdateTarget, String> {
    #[cfg(test)]
    if let Some(public) = public {
        return package_gate::verify_with_key(manifest, key, build, "fixture-key", public);
    }
    let _ = public;
    package_gate::verify(manifest, key, build)
}
fn start_type(value: &str) -> ServiceStartType {
    if value == "manual" {
        ServiceStartType::OnDemand
    } else {
        ServiceStartType::AutoStart
    }
}
fn main_contract(plan: &LocalRepairPlan, fallback: bool) -> Contract {
    Contract {
        name: "RustDesk".into(),
        executable: if fallback {
            PathBuf::from(plan.fallback_path.as_str())
        } else {
            normal()
        }
        .join("RustDesk.exe"),
        arguments: vec!["--service".into()],
        start_type: start_type(&plan.fallback_start),
    }
}
fn agent_contract(plan: &LocalRepairPlan) -> Contract {
    Contract {
        name: "RustDeskManagementAgent".into(),
        executable: agent_root().join("rustdesk-management-agent.exe"),
        arguments: vec![
            "--service".into(),
            "--edition".into(),
            if plan.target_key.ends_with("-sos") {
                "sos"
            } else {
                "standard"
            }
            .into(),
            "--package-kind".into(),
            "msi".into(),
            "--normal-install-path".into(),
            normal().to_string_lossy().into_owned(),
        ],
        start_type: ServiceStartType::AutoStart,
    }
}
fn snapshot_service(contract: &Contract) -> Result<String, String> {
    let obs = service_control::inspect(contract)?;
    if obs.state != ServiceState::Running {
        return Err("local repair requires running original services".into());
    }
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)
        .map_err(|e| e.to_string())?;
    let config = manager
        .open_service(&contract.name, ServiceAccess::QUERY_CONFIG)
        .map_err(|e| e.to_string())?
        .query_config()
        .map_err(|e| e.to_string())?;
    Ok(config.executable_path.to_string_lossy().into_owned())
}
fn ready(path: &Path, build: u64, idle: bool) -> Result<(), String> {
    let reply = super::health_windows::probe_blocking(
        if idle {
            r"\\.\pipe\RustDesk\management-health-v2"
        } else {
            r"\\.\pipe\RustDesk\management-health-v1"
        },
        path,
        build,
        Duration::from_secs(3),
    )?;
    if idle && reply.no_active_sessions != Some(true) {
        return Err("local repair sessions active or unknown".into());
    }
    Ok(())
}

pub fn prepare(id: &str) -> Result<PathBuf, String> {
    prepare_checked(id, None)
}
#[cfg(test)]
pub fn prepare_fixture(id: &str, public: &[u8]) -> Result<PathBuf, String> {
    prepare_checked(id, Some(public))
}

fn freeze_approval(
    directory: &Path,
    source_root: &Path,
    manifest: &hbb_common::UpdateManifest,
    approval: Option<&super::local_repair_plan::ApprovalSource>,
    key: &str,
) -> Result<(), String> {
    if let Some(approval) = approval {
        use std::io::Write;
        let bytes = super::rescue_assets::local_repair_approved_bytes(source_root, approval, key)?;
        let path = directory.join("approved-manifest.json");
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|e| e.to_string())?;
        file.write_all(&bytes)
            .and_then(|_| file.sync_all())
            .map_err(|e| e.to_string())?;
        drop(file);
        workspace::verify_file(&path)
    } else {
        save(&directory.join("approved-manifest.json"), manifest)
    }
}

fn persist_installed_plan(
    directory: &Path,
    plan: &LocalRepairPlan,
    state: &Path,
    identity: &[u8],
    public: Option<&[u8]>,
) -> Result<(), String> {
    plan.preflight_approval_source(directory, state, identity, public)?;
    save(&directory.join("local-repair-plan.json"), plan)?;
    let mut tx = Transaction::new(
        plan.transaction_id.clone(),
        plan.target_key.clone(),
        plan.package.sha256.clone(),
        plan.identity_backup.sha256.clone(),
        plan.created_at,
    )?;
    tx.save(&directory.join("transaction.json"))?;
    tx.advance(Phase::PackageVerified, &evidence(), now()?)?;
    tx.save(&directory.join("transaction.json"))?;
    plan.verify_approval_source(directory, state, identity, public)
}

#[cfg(test)]
pub fn freeze_source_fixture(
    directory: &Path,
    source_root: &Path,
    manifest: &hbb_common::UpdateManifest,
    plan: &LocalRepairPlan,
) -> Result<(), String> {
    freeze_approval(
        directory,
        source_root,
        manifest,
        plan.approval_source.as_ref(),
        &plan.target_key,
    )
}
#[cfg(test)]
pub fn persist_source_fixture(
    directory: &Path,
    plan: &LocalRepairPlan,
    identity: &[u8],
    public: &[u8],
) -> Result<(), String> {
    persist_installed_plan(directory, plan, directory, identity, Some(public))
}
fn prepare_checked(id: &str, public: Option<&[u8]>) -> Result<PathBuf, String> {
    let directory = root(id)?;
    normal_missing()?;
    let _guard = super::install_guard::InstallGuard::try_acquire()
        .map_err(|e| e.to_string())?
        .ok_or("local repair installation busy")?;
    let transactions = directory.parent().ok_or("local repair parent absent")?;
    match std::fs::symlink_metadata(transactions) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            workspace::create(transactions)?;
        }
        Err(e) => return Err(e.to_string()),
        Ok(_) => {}
    }
    capacity_admission(
        directory.parent().ok_or("local repair parent absent")?,
        None,
        true,
    )?;
    let (manifest, source, inventory, build, fallback, installation, source_tx, approval_source) =
        super::rescue_assets::local_repair_source(public)?;
    if !super::rescue_assets::r3_existing_fallback(&inventory, &fallback)? {
        return Err("local repair no verified usable fallback".into());
    }
    let key = if fallback.to_string_lossy().contains("\\sos\\") {
        "windows-x86_64-msi-sos"
    } else {
        "windows-x86_64-msi-standard"
    };
    let approved = target(&manifest, key, build, public)?;
    let (version, actual_build, _) =
        base::update::update_target_release(&manifest, approved).map_err(|e| e.to_string())?;
    if build != actual_build {
        return Err("local repair same-build required".into());
    }
    let _package = package_gate::lock_verified_file(&source, approved)?;
    let metadata = super::msi_metadata::inspect(&source)?;
    let registered = super::msi_metadata::registered_package(&metadata.product_code, &source)?;
    let name = package_gate::cache_name(&approved.primary, "msi")?;
    if registered.package_name() != name {
        return Err("local repair MSI basename differs from registered source".into());
    }
    let _registered = match &registered {
        super::msi_metadata::RegisteredPackage::Source(installed) => {
            Some(package_gate::lock_verified_file(&installed.source, approved)?)
        }
        // Removed registered source, or no registration at all (installed afresh below).
        super::msi_metadata::RegisteredPackage::PackageCode { .. }
        | super::msi_metadata::RegisteredPackage::NotInstalled { .. } => None,
    };
    let edition = if key.ends_with("sos") {
        "sos"
    } else {
        "standard"
    };
    let fallback_contract = Contract {
        name: "RustDesk".into(),
        executable: fallback.join("RustDesk.exe"),
        arguments: vec!["--service".into()],
        start_type: ServiceStartType::AutoStart,
    };
    let fallback_command = snapshot_service(&fallback_contract)?;
    let agent = Contract {
        name: "RustDeskManagementAgent".into(),
        executable: agent_root().join("rustdesk-management-agent.exe"),
        arguments: vec![
            "--service".into(),
            "--edition".into(),
            edition.into(),
            "--package-kind".into(),
            "msi".into(),
            "--normal-install-path".into(),
            normal().to_string_lossy().into_owned(),
        ],
        start_type: ServiceStartType::AutoStart,
    };
    let agent_command = snapshot_service(&agent)?;
    ready(&fallback_contract.executable, build, true)?;
    let configured = hbb_common::config::Config::file();
    let identity = [
        crate::management_agent::system_dirs::get()
            .local_service_roaming()
            .join(r"RustDesk\config\RustDesk.toml"),
        crate::management_agent::system_dirs::get()
            .system_profile_roaming()
            .join(r"RustDesk\config\RustDesk.toml"),
    ]
    .into_iter()
    .find(|path| {
        configured
            .to_string_lossy()
            .eq_ignore_ascii_case(&path.to_string_lossy())
    })
    .ok_or("local repair identity outside fixed profiles")?;
    let config = identity.with_file_name("RustDesk2.toml");
    if !hbb_common::config::Config2::file()
        .to_string_lossy()
        .eq_ignore_ascii_case(&config.to_string_lossy())
    {
        return Err("local repair config path mismatch".into());
    }
    workspace::verify(directory.parent().ok_or("local repair parent absent")?)?;
    workspace::create(&directory)?;
    workspace::create(&directory.join("runtime"))?;
    let package = copy(&directory, &source, &name, 2 * 1024 * 1024 * 1024)?;
    if let Some(approval) = &approval_source {
        let receipt = copy(
            &directory,
            &agent_root().join("rescue/current/installation-receipt.json"),
            "installation-receipt.json",
            65536,
        )?;
        if receipt != approval.installation_receipt {
            return Err("local repair source receipt changed; preparation retained".into());
        }
    }
    freeze_approval(
        &directory,
        &agent_root().join("rescue/current"),
        &manifest,
        approval_source.as_ref(),
        key,
    )?;
    let client_inventory = copy(
        &directory,
        agent_root()
            .join("rescue/current/client/manifest.json")
            .as_path(),
        "client-inventory.json",
        8 * 1024 * 1024,
    )?;
    let agent_backup = copy(
        &directory,
        &agent.executable,
        "agent-backup.exe",
        512 * 1024 * 1024,
    )?;
    let identity_backup = copy(&directory, &identity, "identity-backup.toml", 65536)?;
    let config_backup = copy(&directory, &config, "config-backup.toml", 65536)?;
    let bootstrap_backup = copy(
        &directory,
        &crate::management_agent::system_dirs::get()
            .recovery_bootstrap()
            .join("rustdesk-recovery-bootstrap.exe"),
        "bootstrap-backup.exe",
        512 * 1024 * 1024,
    )?;
    let bootstrap_manifest = copy(
        &directory,
        &crate::management_agent::system_dirs::get()
            .recovery_bootstrap()
            .join("bootstrap.json"),
        "bootstrap-backup.json",
        65536,
    )?;
    let approved_bundle: serde_json::Value = serde_json::from_slice(&read(
        agent_root().join("rescue/current/bundle.json").as_path(),
        65536,
    )?)
    .map_err(|_| "local repair bundle malformed")?;
    if approved_bundle["agent"]["sha256"] != agent_backup.sha256
        || approved_bundle["bootstrap"]["sha256"] != bootstrap_backup.sha256
    {
        return Err(
            "local repair running agent/bootstrap differs from approved same-build bundle".into(),
        );
    }
    let helper = copy(
        &directory,
        &std::env::current_exe().map_err(|e| e.to_string())?,
        "runtime/helper.exe",
        512 * 1024 * 1024,
    )?;
    let plan = LocalRepairPlan {
        schema: 1,
        trigger: "local_recovery".into(),
        transaction_id: id.into(),
        installation_id: installation,
        target_key: key.into(),
        build_seq: build,
        version: version.into(),
        original_normal: "missing".into(),
        fallback_path: fallback.to_string_lossy().into_owned(),
        fallback_command,
        fallback_start: "auto".into(),
        agent_command,
        agent_start: "auto".into(),
        identity_path: identity.to_string_lossy().into_owned(),
        product_code: metadata.product_code,
        source_transaction: source_tx,
        approval_source,
        total_loss: None,
        created_at: now()?,
        package,
        client_inventory,
        agent_backup,
        identity_backup,
        config_backup,
        bootstrap_backup,
        bootstrap_manifest,
        helper,
    };
    plan.validate()?;
    if plan.approval_source.is_some() {
        let identity_bytes = read(&directory.join("identity-backup.toml"), 65536)?;
        persist_installed_plan(
            &directory,
            &plan,
            &agent_root().join("state"),
            &identity_bytes,
            public,
        )?;
    } else {
        save(&directory.join("local-repair-plan.json"), &plan)?;
        let mut tx = Transaction::new(
            id.into(),
            key.into(),
            plan.package.sha256.clone(),
            plan.identity_backup.sha256.clone(),
            plan.created_at,
        )?;
        tx.save(&directory.join("transaction.json"))?;
        let proof = evidence();
        tx.advance(Phase::PackageVerified, &proof, now()?)?;
        tx.save(&directory.join("transaction.json"))?;
    }
    check(&directory, &plan, public)?;
    normal_missing()?;
    ready(&fallback_contract.executable, build, true)?;
    Ok(directory)
}

fn evidence() -> Evidence {
    Evidence {
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
    }
}
fn load(directory: &Path) -> Result<LocalRepairPlan, String> {
    workspace::verify(directory)?;
    let plan = LocalRepairPlan::decode(&read(&directory.join("local-repair-plan.json"), 65536)?)?;
    if root(&plan.transaction_id)? != directory {
        return Err("local repair root mismatch".into());
    }
    Ok(plan)
}
fn other_transactions(directory: &Path) -> Result<(), String> {
    other_transactions_checked(directory, None)
}

fn other_transactions_owned(
    directory: &Path,
    binding: &super::handoff::Handoff,
) -> Result<(), String> {
    other_transactions_checked(
        directory,
        Some(super::local_repair_plan::total_loss::CurrentExecution::OriginalHelper(binding)),
    )
}

pub(super) struct ResumeAuthorized<'a> {
    binding: &'a super::handoff::Handoff,
    owner: super::handoff::ObservedProcess,
}

impl ResumeAuthorized<'_> {
    pub(super) fn binding(&self) -> &super::handoff::Handoff {
        self.binding
    }
    pub(super) fn owner(&self) -> &super::handoff::Owner {
        self.owner.owner()
    }
    pub(super) fn alive(&self) -> Result<bool, String> {
        self.owner.is_alive()
    }
}

fn authorize_resume_competition<'a>(
    directory: &Path,
    plan: &LocalRepairPlan,
    binding: &'a super::handoff::Handoff,
    state: &Path,
    intent: &Path,
    task_check: impl FnOnce() -> Result<(), String>,
) -> Result<ResumeAuthorized<'a>, String> {
    workspace::verify(directory)?;
    let stored = read(&directory.join("local-repair-plan.json"), 65536)?;
    let tx = Transaction::decode(&read(&directory.join("transaction.json"), 65536)?)?;
    plan.bind(&tx)?;
    if serde_json::to_vec(plan).map_err(|e| e.to_string())?
        != serde_json::to_vec(&LocalRepairPlan::decode(&stored)?).map_err(|e| e.to_string())?
        || binding.plan_sha256 != hex::encode(Sha256::digest(&stored))
        || binding.transaction_id != tx.id
        || super::handoff::Handoff::decode(&read(&directory.join("handoff.json"), 4096)?)?
            != *binding
    {
        return Err("local repair resume competition binding mismatch".into());
    }
    let executable = std::env::current_exe().map_err(|e| e.to_string())?;
    if executable != directory.join(&plan.helper.relative_path) {
        return Err("local repair resume executable mismatch".into());
    }
    plan.helper.verify(directory)?;
    if super::handoff::ObservedProcess::owner_alive(&binding.helper)? {
        return Err("local repair previous helper still alive".into());
    }
    let ready = serde_json::from_slice(&read(&directory.join("local-ready.json"), 4096)?)
        .map_err(|_| "local repair resume ready malformed")?;
    let release: super::handoff::ReleaseReceipt =
        serde_json::from_slice(&read(&directory.join("local-release.json"), 4096)?)
            .map_err(|_| "local repair resume release malformed")?;
    if release.released_at > now()? {
        return Err("local repair resume future release".into());
    }
    binding.verify_release(&ready, &release, &binding.helper, release.released_at)?;
    resume_authorized(&tx, now()?)?;
    task_check()?;
    preflight_owned(state, intent, plan, binding, || Ok(()))?;
    let owner = super::handoff::ObservedProcess::open(std::process::id())?;
    if !owner.is_alive()? || owner.owner() == &binding.helper {
        return Err("local repair resume owner invalid".into());
    }
    Ok(ResumeAuthorized { binding, owner })
}

fn other_transactions_resumed(
    directory: &Path,
    authorized: &ResumeAuthorized<'_>,
) -> Result<(), String> {
    other_transactions_checked(
        directory,
        Some(super::local_repair_plan::total_loss::CurrentExecution::ResumeAuthorized(authorized)),
    )
}

fn other_transactions_checked(
    directory: &Path,
    current: Option<super::local_repair_plan::total_loss::CurrentExecution<'_>>,
) -> Result<(), String> {
    capacity_admission(
        directory.parent().ok_or("local repair parent missing")?,
        Some(directory),
        false,
    )?;
    for entry in std::fs::read_dir(directory.parent().ok_or("local repair parent missing")?)
        .map_err(|e| e.to_string())?
    {
        let path = entry.map_err(|e| e.to_string())?.path();
        if path == directory {
            continue;
        }
        workspace::verify(&path)?;
        let tx = Transaction::decode(&read(&path.join("transaction.json"), 65536)?)?;
        if !matches!(tx.phase, Phase::Committed | Phase::RolledBack) {
            if tx.phase == Phase::RollingBack {
                if let Ok(previous) =
                    super::local_repair_plan::total_loss::completed_preservation_for(
                        &path,
                        current.map(|context| (directory, context)),
                    )
                {
                    let next = LocalRepairPlan::decode(&read(
                        &directory.join("local-repair-plan.json"),
                        65536,
                    )?)?;
                    if previous.installation_id == next.installation_id
                        && previous.target_key == next.target_key
                        && previous.approval_id() != next.approval_id()
                    {
                        continue;
                    }
                }
            }
            return Err("local repair competing transaction".into());
        }
    }
    Ok(())
}
fn check(directory: &Path, plan: &LocalRepairPlan, public: Option<&[u8]>) -> Result<(), String> {
    plan.validate()?;
    if plan.total_loss.is_some() {
        return Err("total loss requires dedicated execution path".into());
    }
    if plan.approval_source.is_some() {
        workspace::verify_file(&directory.join("installation-receipt.json"))?;
        let identity = read(&directory.join(&plan.identity_backup.relative_path), 65536)?;
        plan.verify_approval_source(directory, &agent_root().join("state"), &identity, public)?;
    }
    for asset in plan.artifacts() {
        workspace::verify_file(&directory.join(&asset.relative_path))?;
        asset.verify(directory)?;
    }
    let manifest: hbb_common::UpdateManifest =
        serde_json::from_slice(&read(&directory.join("approved-manifest.json"), 262144)?)
            .map_err(|_| "local repair manifest malformed")?;
    let approved = target(&manifest, &plan.target_key, plan.build_seq, public)?;
    let (version, build, _) =
        base::update::update_target_release(&manifest, approved).map_err(|e| e.to_string())?;
    if version != plan.version
        || build != plan.build_seq
        || approved.size != plan.package.size
        || approved.sha256 != plan.package.sha256
        || package_gate::cache_name(&approved.primary, "msi")? != plan.package.relative_path
    {
        return Err("local repair signed target mismatch".into());
    }
    let _lock =
        package_gate::lock_verified_file(&directory.join(&plan.package.relative_path), approved)?;
    let metadata = super::msi_metadata::inspect(&directory.join(&plan.package.relative_path))?;
    if metadata.product_name != "RustDesk" || metadata.product_code != plan.product_code {
        return Err("local repair product mismatch".into());
    }
    let inventory = super::directory_backup::DirectoryBackup::decode(&read(
        &directory.join(&plan.client_inventory.relative_path),
        8 * 1024 * 1024,
    )?)?;
    if !super::rescue_assets::r3_existing_fallback(&inventory, Path::new(&plan.fallback_path))? {
        return Err("local repair fallback unavailable".into());
    }
    original_identity(directory, plan)?;
    Ok(())
}
fn original_identity(directory: &Path, plan: &LocalRepairPlan) -> Result<(), String> {
    use std::os::windows::fs::OpenOptionsExt;
    for (source, reference) in [
        (PathBuf::from(&plan.identity_path), &plan.identity_backup),
        (
            Path::new(&plan.identity_path).with_file_name("RustDesk2.toml"),
            &plan.config_backup,
        ),
    ] {
        workspace::verify_restore_target(&source, true)?;
        let mut bytes = Vec::new();
        std::fs::OpenOptions::new()
            .read(true)
            .share_mode(1)
            .open(source)
            .map_err(|e| e.to_string())?
            .take(65537)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        if bytes != read(&directory.join(&reference.relative_path), 65536)? {
            return Err("local repair original identity/config changed".into());
        }
    }
    Ok(())
}

fn definition(plan: &LocalRepairPlan, directory: &Path) -> Result<String, String> {
    Ok(super::resume_task::definition(
        &directory.join(&plan.helper.relative_path),
        &plan.transaction_id,
    )?
    .replace(
        "resume-install --transaction-id",
        "local-repair-resume --transaction-id",
    ))
}
fn verify_task(plan: &LocalRepairPlan, directory: &Path) -> Result<(), String> {
    let actual = super::resume_task::read_back(&plan.transaction_id)?;
    if !actual.contains("local-repair-resume --transaction-id") {
        return Err("local repair resume action mismatch".into());
    }
    super::resume_task::verify(
        &actual.replace(
            "local-repair-resume --transaction-id",
            "resume-install --transaction-id",
        ),
        &directory.join(&plan.helper.relative_path),
        &plan.transaction_id,
    )
}
pub fn launch(directory: &Path) -> Result<std::process::Child, String> {
    launch_checked(directory, None)
}
#[cfg(test)]
pub fn launch_fixture(directory: &Path, public: &[u8]) -> Result<std::process::Child, String> {
    launch_checked(directory, Some(public))
}
fn launch_checked(directory: &Path, public: Option<&[u8]>) -> Result<std::process::Child, String> {
    let plan = load(directory)?;
    if plan.total_loss.is_some() {
        return total_loss_runtime::launch(directory, public);
    }
    check(directory, &plan, public)?;
    normal_missing()?;
    require_optin(&agent_root().join("state"))?;
    if !super::rescue_assets::r3_cycle_allowed()? {
        return Err("local repair locally blocked".into());
    }
    let guard = super::install_guard::InstallGuard::try_acquire()
        .map_err(|e| e.to_string())?
        .ok_or("local repair installation busy")?;
    other_transactions(directory)?;
    if !super::rescue_assets::r3_cycle_allowed()? {
        return Err("local repair locally blocked".into());
    }
    let mut tx = Transaction::decode(&read(&directory.join("transaction.json"), 65536)?)?;
    plan.bind(&tx)?;
    if tx.phase != Phase::PackageVerified {
        return Err("local repair already launched".into());
    }
    ready(&main_contract(&plan, true).executable, plan.build_seq, true)?;
    let digest = hex::encode(Sha256::digest(read(
        &directory.join("local-repair-plan.json"),
        65536,
    )?));
    let nonce = hex::encode(hbb_common::sodiumoxide::randombytes::randombytes(32));
    super::maintenance::claim(
        &agent_root().join("state"),
        &plan.installation_id,
        &plan.transaction_id,
        &digest,
        &nonce,
    )?;
    let helper = directory.join(&plan.helper.relative_path);
    let mut command = std::process::Command::new(&helper);
    command.args([
        "local-repair-helper",
        "--transaction-id",
        &plan.transaction_id,
    ]);
    #[cfg(test)]
    if let Some(public) = public {
        use hbb_common::base64::{engine::general_purpose::STANDARD, Engine as _};
        command = std::process::Command::new(&helper);
        command
            .args(["--exact", "localrepair_native_child", "--nocapture"])
            .env("RUSTDESK_LOCAL_REPAIR_CHILD", &plan.transaction_id)
            .env("RUSTDESK_LOCAL_REPAIR_PUBLIC", STANDARD.encode(public));
    }
    command
        .stdout(
            std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(directory.join("child.stdout.log"))
                .map_err(|e| e.to_string())?,
        )
        .stderr(
            std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(directory.join("child.stderr.log"))
                .map_err(|e| e.to_string())?,
        );
    let mut child = command.spawn().map_err(|e| e.to_string())?;
    let parent = super::handoff::ObservedProcess::open(std::process::id())?;
    let process = super::handoff::ObservedProcess::open(child.id())?;
    let binding = super::handoff::Handoff::new(
        plan.transaction_id.clone(),
        digest,
        nonce,
        parent.owner().clone(),
        process.owner().clone(),
        now()?,
    )?;
    save(&directory.join("handoff.json"), &binding)?;
    let deadline = Instant::now() + Duration::from_secs(180);
    while !directory.join("local-ready.json").exists() {
        if child.try_wait().map_err(|e| e.to_string())?.is_some() {
            return Err("local repair helper exited before readiness".into());
        }
        if Instant::now() >= deadline {
            return Err("local repair readiness timed out".into());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let receipt: super::handoff::ReadyReceipt =
        serde_json::from_slice(&read(&directory.join("local-ready.json"), 4096)?)
            .map_err(|_| "local repair readiness malformed")?;
    binding.verify_ready(&receipt, now()?)?;
    super::resume_task::register_definition(
        directory,
        &plan.transaction_id,
        &definition(&plan, directory)?,
    )?;
    verify_task(&plan, directory)?;
    preflight_owned(
        &agent_root().join("state"),
        &hbb_common::config::Config2::file(),
        &plan,
        &binding,
        || {
            normal_missing()?;
            original_identity(directory, &plan)?;
            ready(&main_contract(&plan, true).executable, plan.build_seq, true)
        },
    )?;
    let proof = evidence();
    for phase in [Phase::Prepared, Phase::StoppingServices] {
        tx.advance(phase, &proof, now()?)?;
        tx.save(&directory.join("transaction.json"))?;
    }
    save(
        &directory.join("local-release.json"),
        &binding.release_receipt(&receipt, now()?)?,
    )?;
    drop(guard);
    Ok(child)
}

pub fn run_helper(id: &str, resume: bool) -> Result<(), String> {
    run_checked(id, None, resume, false)
}
#[cfg(test)]
pub fn run_fixture(id: &str, public: &[u8], fail_validation: bool) -> Result<(), String> {
    run_checked(id, Some(public), false, fail_validation)
}
fn run_checked(
    id: &str,
    public: Option<&[u8]>,
    resume: bool,
    fail_validation: bool,
) -> Result<(), String> {
    let directory = root(id)?;
    let plan = load(&directory)?;
    if plan.total_loss.is_some() {
        if std::env::current_exe().map_err(|e| e.to_string())?
            != directory.join(&plan.helper.relative_path)
        {
            return Err("total loss helper executable mismatch".into());
        }
        return total_loss_runtime::run(&directory, plan, public, resume, fail_validation);
    }
    if std::env::current_exe().map_err(|e| e.to_string())?
        != directory.join(&plan.helper.relative_path)
    {
        return Err("local repair helper executable mismatch".into());
    }
    check(&directory, &plan, public)?;
    let deadline = Instant::now() + Duration::from_secs(180);
    while !directory.join("handoff.json").exists() {
        if Instant::now() >= deadline {
            return Err("local repair handoff timeout".into());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let binding = super::handoff::Handoff::decode(&read(&directory.join("handoff.json"), 4096)?)?;
    if binding.transaction_id != id
        || binding.plan_sha256
            != hex::encode(Sha256::digest(read(
                &directory.join("local-repair-plan.json"),
                65536,
            )?))
    {
        return Err("local repair handoff binding mismatch".into());
    }
    let current = super::handoff::ObservedProcess::open(std::process::id())?;
    if !resume {
        if current.owner() != &binding.helper
            || !super::handoff::ObservedProcess::owner_alive(&binding.parent)?
        {
            return Err("local repair handoff process mismatch".into());
        }
        save(
            &directory.join("local-ready.json"),
            &binding.ready_receipt(now()?)?,
        )?;
        while !directory.join("local-release.json").exists() {
            if Instant::now() >= deadline {
                return Err("local repair release timeout".into());
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    } else if super::handoff::ObservedProcess::owner_alive(&binding.helper)? {
        return Err("local repair previous helper still alive".into());
    }
    let ready_receipt: super::handoff::ReadyReceipt =
        serde_json::from_slice(&read(&directory.join("local-ready.json"), 4096)?)
            .map_err(|_| "local repair ready malformed")?;
    let release: super::handoff::ReleaseReceipt =
        serde_json::from_slice(&read(&directory.join("local-release.json"), 4096)?)
            .map_err(|_| "local repair release malformed")?;
    binding.verify_release(
        &ready_receipt,
        &release,
        &binding.helper,
        if resume { release.released_at } else { now()? },
    )?;
    let _guard = loop {
        if let Some(guard) =
            super::install_guard::InstallGuard::try_acquire().map_err(|e| e.to_string())?
        {
            break guard;
        }
        if Instant::now() >= deadline {
            return Err("local repair guard busy".into());
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    let mut tx = Transaction::decode(&read(&directory.join("transaction.json"), 65536)?)?;
    plan.bind(&tx)?;
    if resume
        && matches!(
            tx.phase,
            Phase::Committed | Phase::RolledBack | Phase::RollbackFailed
        )
    {
        return finish_terminal_resume(
            &directory,
            &plan,
            &binding,
            &tx,
            &agent_root().join("state"),
            &hbb_common::config::Config2::file(),
            || {
                verify_task(&plan, &directory)?;
                super::resume_task::remove(id)
            },
        );
    }
    super::local_recovery_events::report_repair_terminal(&tx, &plan.installation_id, now()?)?;
    if resume {
        resume_authorized(&tx, now()?)?;
    }
    if resume {
        let authorized = authorize_resume_competition(
            &directory,
            &plan,
            &binding,
            &agent_root().join("state"),
            &hbb_common::config::Config2::file(),
            || verify_task(&plan, &directory),
        )?;
        other_transactions_resumed(&directory, &authorized)?;
    } else {
        other_transactions_owned(&directory, &binding)?;
    }
    if matches!(
        tx.phase,
        Phase::Committed | Phase::RolledBack | Phase::RollbackFailed
    ) {
        return Err("local repair terminal evidence retained; no automatic re-execution".into());
    }
    verify_task(&plan, &directory)?;
    super::maintenance::verify(
        &agent_root().join("state"),
        &hbb_common::config::Config2::file(),
        &plan.installation_id,
        id,
        &binding.plan_sha256,
        &binding.nonce,
    )?;
    check(&directory, &plan, public)?;
    let mut backend = LocalBackend {
        directory: directory.clone(),
        plan: plan.clone(),
        binding: binding.clone(),
        fail_validation,
    };
    let outcome = if resume && tx.phase == Phase::Applying {
        super::installer_process::read_recorded(&directory, "install", id, &plan.package.sha256)?
    } else {
        None
    };
    let proof = evidence();
    let result = dispatch(
        &directory.join("transaction.json"),
        &mut tx,
        &proof,
        &mut backend,
        now()?,
        resume,
        outcome,
        || {
            preflight_owned(
                &agent_root().join("state"),
                &hbb_common::config::Config2::file(),
                &plan,
                &binding,
                || {
                    normal_missing()?;
                    original_identity(&directory, &plan)?;
                    ready(&main_contract(&plan, true).executable, plan.build_seq, true)
                },
            )
        },
    );
    finish_execution(
        &directory,
        &plan,
        &binding,
        &agent_root().join("state"),
        result,
        || super::resume_task::remove(id),
    )
}

fn finish_terminal_resume(
    directory: &Path,
    plan: &LocalRepairPlan,
    binding: &super::handoff::Handoff,
    tx: &Transaction,
    state: &Path,
    intent: &Path,
    remove_task: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    plan.bind(tx)?;
    if Transaction::decode(&read(&directory.join("transaction.json"), 65536)?)? != *tx {
        return Err("local repair terminal transaction binding mismatch".into());
    }
    if !matches!(tx.phase, Phase::Committed | Phase::RolledBack) {
        return Err(
            "local repair terminal evidence retained; manual reconciliation required".into(),
        );
    }
    let stored_plan = read(&directory.join("local-repair-plan.json"), 65536)?;
    LocalRepairPlan::decode(&stored_plan)?.bind(tx)?;
    if serde_json::to_value(plan).map_err(|e| e.to_string())?
        != serde_json::from_slice::<serde_json::Value>(&stored_plan).map_err(|e| e.to_string())?
        || binding.transaction_id != tx.id
        || binding.plan_sha256 != hex::encode(Sha256::digest(&stored_plan))
        || super::handoff::Handoff::decode(&read(&directory.join("handoff.json"), 4096)?)?
            != *binding
    {
        return Err("local repair terminal handoff binding mismatch".into());
    }
    let ready = serde_json::from_slice(&read(&directory.join("local-ready.json"), 4096)?)
        .map_err(|_| "local repair terminal ready malformed")?;
    let release: super::handoff::ReleaseReceipt =
        serde_json::from_slice(&read(&directory.join("local-release.json"), 4096)?)
            .map_err(|_| "local repair terminal release malformed")?;
    binding.verify_release(&ready, &release, &binding.helper, release.released_at)?;
    super::maintenance::verify_terminal_cleanup(
        state,
        intent,
        &plan.installation_id,
        &tx.id,
        &binding.plan_sha256,
        &binding.nonce,
    )?;
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct ResultReceipt {
        schema: u32,
        transaction_id: String,
        phase: Phase,
        error: Option<String>,
        reboot_required: bool,
    }
    let result_path = directory.join("local-result.json");
    match std::fs::symlink_metadata(&result_path) {
        Ok(_) => {
            let result: ResultReceipt = serde_json::from_slice(&read(&result_path, 65536)?)
                .map_err(|_| "local repair result binding mismatch: malformed receipt")?;
            if result.schema != 1
                || result.transaction_id != tx.id
                || result.phase != tx.phase
                || result.reboot_required
                || (tx.phase == Phase::Committed && result.error.is_some())
                || (tx.phase == Phase::RolledBack
                    && result.error.as_deref().map_or(true, str::is_empty))
            {
                return Err("local repair result binding mismatch".into());
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            // Committed is persisted only after validation; a rollback cause is not in Transaction.
            if tx.phase != Phase::Committed {
                return Err("local repair terminal result evidence unavailable".into());
            }
            save(
                &result_path,
                &serde_json::json!({"schema":1,"transaction_id":tx.id,
                "phase":tx.phase,"error":null,"reboot_required":false}),
            )?;
        }
        Err(error) => return Err(error.to_string()),
    }
    super::local_recovery_events::report_repair_terminal_for_cleanup(
        tx,
        &plan.installation_id,
        now()?,
    )?;
    match std::fs::symlink_metadata(state.join("maintenance.json")) {
        Ok(_) => super::maintenance::release_owned(
            state,
            &plan.installation_id,
            &tx.id,
            &binding.plan_sha256,
            &binding.nonce,
        )?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.to_string()),
    }
    remove_task()
}

fn finish_execution(
    directory: &Path,
    plan: &LocalRepairPlan,
    binding: &super::handoff::Handoff,
    state: &Path,
    result: Result<install_execution::ExecutionResult, String>,
    remove_task: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    let result = result?;
    let id = &plan.transaction_id;
    save(
        &directory.join("local-result.json"),
        &serde_json::json!({"schema":1,"transaction_id":id,"phase":result.phase,"error":result.error,"reboot_required":result.reboot_required}),
    )?;
    if matches!(result.phase, Phase::Committed | Phase::RolledBack) {
        super::maintenance::release_owned(
            state,
            &plan.installation_id,
            id,
            &binding.plan_sha256,
            &binding.nonce,
        )?;
        remove_task()?;
        Ok(())
    } else {
        Err(format!(
            "local repair requires reconciliation: {:?}",
            result.phase
        ))
    }
}

struct LocalBackend {
    directory: PathBuf,
    plan: LocalRepairPlan,
    binding: super::handoff::Handoff,
    fail_validation: bool,
}
impl LocalBackend {
    fn gates(&self) -> Result<(), String> {
        super::maintenance::verify(
            &agent_root().join("state"),
            &hbb_common::config::Config2::file(),
            &self.plan.installation_id,
            &self.plan.transaction_id,
            &self.binding.plan_sha256,
            &self.binding.nonce,
        )
    }
    fn inventory(&self) -> Result<super::directory_backup::DirectoryBackup, String> {
        super::directory_backup::DirectoryBackup::decode(&read(
            &self
                .directory
                .join(&self.plan.client_inventory.relative_path),
            8 * 1024 * 1024,
        )?)
    }
    fn configure(&self, fallback: bool) -> Result<(), String> {
        let command = if fallback {
            self.plan.fallback_command.clone()
        } else {
            format!(r#""{}" --service"#, normal().join("RustDesk.exe").display())
        };
        let status = std::process::Command::new(
            crate::management_agent::system_dirs::get()
                .system32()
                .join("sc.exe"),
        )
        .args([
            "config",
            "RustDesk",
            "binPath=",
            &command,
            "start=",
            &self.plan.fallback_start,
            "obj=",
            "LocalSystem",
        ])
        .status()
        .map_err(|e| e.to_string())?;
        if !status.success() {
            return Err("local repair fixed service configuration failed".into());
        }
        Ok(())
    }
    fn verify_agent(&self) -> Result<(), String> {
        let _lock = package_gate::lock_verified_file(
            &agent_root().join("rustdesk-management-agent.exe"),
            &hbb_common::UpdateTarget {
                size: self.plan.agent_backup.size,
                sha256: self.plan.agent_backup.sha256.clone(),
                ..Default::default()
            },
        )?;
        let obs = service_control::inspect(&agent_contract(&self.plan))?;
        if obs.state != ServiceState::Running {
            return Err("local repair agent not running".into());
        }
        let _bootstrap = package_gate::lock_verified_file(
            &crate::management_agent::system_dirs::get()
                .recovery_bootstrap()
                .join("rustdesk-recovery-bootstrap.exe"),
            &hbb_common::UpdateTarget {
                size: self.plan.bootstrap_backup.size,
                sha256: self.plan.bootstrap_backup.sha256.clone(),
                ..Default::default()
            },
        )?;
        let actual: serde_json::Value = serde_json::from_slice(&read(
            &crate::management_agent::system_dirs::get()
                .recovery_bootstrap()
                .join("bootstrap.json"),
            65536,
        )?)
        .map_err(|_| "local repair bootstrap manifest invalid")?;
        let original: serde_json::Value = serde_json::from_slice(&read(
            &self
                .directory
                .join(&self.plan.bootstrap_manifest.relative_path),
            65536,
        )?)
        .map_err(|_| "local repair bootstrap backup invalid")?;
        if actual != original {
            return Err("local repair bootstrap manifest changed".into());
        }
        Ok(())
    }
    fn wait_ready(&self, fallback: bool) -> Result<(), String> {
        let path = main_contract(&self.plan, fallback).executable;
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            if ready(&path, self.plan.build_seq, false).is_ok() {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err("local repair client ready timeout".into());
            }
            std::thread::sleep(Duration::from_millis(250));
        }
    }
}
impl Backend for LocalBackend {
    fn stop_services(&mut self) -> Result<(), String> {
        service_control::stop(&main_contract(&self.plan, true), Duration::from_secs(60))?;
        service_control::stop(&agent_contract(&self.plan), Duration::from_secs(60))
    }
    fn apply_package(&mut self) -> Result<InstallOutcome, String> {
        self.gates()?;
        require_optin(&agent_root().join("state"))?;
        let package = self.directory.join(&self.plan.package.relative_path);
        let _lock = package_gate::lock_verified_file(
            &package,
            &hbb_common::UpdateTarget {
                size: self.plan.package.size,
                sha256: self.plan.package.sha256.clone(),
                ..Default::default()
            },
        )?;
        let mut command = std::process::Command::new(
            crate::management_agent::system_dirs::get()
                .system32()
                .join("msiexec.exe"),
        );
        command
            .args(super::installer_process::msi_arguments(
                &package,
                &self.directory.join("installer.log"),
            )?)
            .args(super::msi_metadata::install_properties(
                super::msi_metadata::product_installed(&self.plan.product_code)?,
            ));
        match super::installer_process::run_recorded(
            &mut command,
            Duration::from_secs(600),
            &self.directory,
            "install",
            &self.plan.transaction_id,
            &self.plan.package.sha256,
        ) {
            Ok(value) => Ok(value),
            Err(error) if self.directory.join("install-started.json").exists() => {
                hbb_common::log::warn!("local repair installer needs reconciliation: {error}");
                Ok(InstallOutcome::Pending {
                    pid: 0,
                    created_at: 0,
                })
            }
            Err(error) => Err(error),
        }
    }
    fn validate_target(&mut self) -> Result<Validation, String> {
        self.gates()?;
        self.configure(false)?;
        service_control::start(&agent_contract(&self.plan), Duration::from_secs(60))?;
        service_control::start(&main_contract(&self.plan, false), Duration::from_secs(60))?;
        self.wait_ready(false)?;
        self.verify_agent()?;
        original_identity(&self.directory, &self.plan)?;
        let registered = super::msi_metadata::registered_package(
            &self.plan.product_code,
            &self.directory.join(&self.plan.package.relative_path),
        )?;
        let _source = match &registered {
            super::msi_metadata::RegisteredPackage::Source(installed) => Some(
                package_gate::lock_verified_file(
                    &installed.source,
                    &hbb_common::UpdateTarget {
                        size: self.plan.package.size,
                        sha256: self.plan.package.sha256.clone(),
                        ..Default::default()
                    },
                )?,
            ),
            super::msi_metadata::RegisteredPackage::PackageCode { .. } => None,
            super::msi_metadata::RegisteredPackage::NotInstalled { .. } => {
                return Err("local repair product still not registered after install".into())
            }
        };
        let matches = self.inventory()?.matches_directory(&normal())?
            && registered.package_name() == self.plan.package.relative_path;
        #[cfg(test)]
        let matches = matches && !self.fail_validation;
        #[cfg(not(test))]
        let _ = self.fail_validation;
        Ok(Validation {
            target_matches: matches,
            identity_unchanged: true,
            client_ready: true,
            agent_ready: true,
        })
    }
    fn rollback_original(&mut self) -> Result<Validation, String> {
        self.gates()?;
        let inventory = self.inventory()?;
        if !super::rescue_assets::r3_existing_fallback(
            &inventory,
            Path::new(&self.plan.fallback_path),
        )? {
            return Err("local repair rollback anchor unavailable".into());
        }
        // Do not erase the newly created normal directory: it is transaction evidence, not the old usable anchor.
        let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)
            .map_err(|e| e.to_string())?;
        let service = manager
            .open_service(
                "RustDesk",
                ServiceAccess::QUERY_CONFIG | ServiceAccess::QUERY_STATUS,
            )
            .map_err(|e| e.to_string())?;
        let config = service.query_config().map_err(|e| e.to_string())?;
        let fallback = main_contract(&self.plan, true);
        let normal = main_contract(&self.plan, false);
        let current = if fallback.matches_command(&config.executable_path.to_string_lossy()) {
            fallback.clone()
        } else if normal.matches_command(&config.executable_path.to_string_lossy()) {
            normal
        } else {
            return Err("local repair rollback unknown service".into());
        };
        if service
            .query_status()
            .map_err(|e| e.to_string())?
            .current_state
            != ServiceState::Stopped
        {
            service_control::stop(&current, Duration::from_secs(60))?;
        }
        service_control::stop(&agent_contract(&self.plan), Duration::from_secs(60))?;
        super::file_restore::restore(
            &self.directory,
            &self.plan.agent_backup,
            &agent_root().join("rustdesk-management-agent.exe"),
        )?;
        original_identity(&self.directory, &self.plan)?;
        self.configure(true)?;
        service_control::start(&agent_contract(&self.plan), Duration::from_secs(60))?;
        service_control::start(&fallback, Duration::from_secs(60))?;
        self.wait_ready(true)?;
        self.verify_agent()?;
        Ok(Validation {
            target_matches: true,
            identity_unchanged: true,
            client_ready: true,
            agent_ready: true,
        })
    }
}

#[cfg(test)]
pub(super) mod r4_finalize_tests {
    use super::super::{
        handoff::{Handoff, Owner},
        local_events::{Binding, LocalEvents},
        local_recovery_events::{self, EventContext},
    };
    use super::*;
    use std::{cell::Cell, os::windows::fs::OpenOptionsExt};

    std::thread_local! {
        pub(crate) static BACKEND_CALLS: Cell<usize> = const { Cell::new(0) };
    }

    pub(crate) fn poll_save(path: &Path, value: &impl serde::Serialize) -> Result<(), String> {
        save(path, value)
    }

    pub(crate) fn poll_evidence() -> Evidence {
        evidence()
    }

    pub(crate) fn poll_dispatch(
        path: &Path,
        tx: &mut Transaction,
        proof: &Evidence,
        backend: &mut impl Backend,
        time: u64,
        resume: bool,
        outcome: Option<InstallOutcome>,
        preflight: impl FnOnce() -> Result<(), String>,
    ) -> Result<install_execution::ExecutionResult, String> {
        dispatch(path, tx, proof, backend, time, resume, outcome, preflight)
    }

    pub(crate) fn poll_finish_execution(
        directory: &Path,
        plan: &LocalRepairPlan,
        binding: &Handoff,
        state: &Path,
        result: Result<install_execution::ExecutionResult, String>,
        remove_task: impl FnOnce() -> Result<(), String>,
    ) -> Result<(), String> {
        finish_execution(directory, plan, binding, state, result, remove_task)
    }

    pub(crate) struct Fixture {
        pub(crate) root: PathBuf,
        pub(crate) state: PathBuf,
        pub(crate) plan: LocalRepairPlan,
        pub(crate) handoff: Handoff,
        pub(crate) context: EventContext,
    }

    impl Fixture {
        pub(crate) fn new() -> Self {
            base::lab_guard::require_lab();
            assert_eq!(
                std::env::var("LOCAL_EVENTS_R4_FINALIZE").as_deref(),
                Ok("scheduled")
            );
            let base = PathBuf::from(std::env::var_os("RUSTDESK_LOCAL_EVENTS_ROOT").unwrap());
            assert!(base.starts_with(r"C:\RustDeskAgentLab"));
            let root = base.join(format!("r4-finalize-{}", uuid::Uuid::new_v4()));
            workspace::create(&root).unwrap();
            let state = root.join("state");
            workspace::create(&state).unwrap();
            let artifact = |name: &str| Artifact {
                relative_path: name.into(),
                size: 1,
                sha256: "c".repeat(64),
            };
            let plan = LocalRepairPlan {
                schema: 1, trigger: "local_recovery".into(), transaction_id: "b".repeat(32),
                installation_id: "a".repeat(32), target_key: "windows-x86_64-msi-sos".into(),
                build_seq: 2026100502, version: "1.5.3".into(), original_normal: "missing".into(),
                fallback_path: r"C:\ProgramData\RustDesk\Client\sos\RustDesk".into(),
                fallback_command: r#""C:\ProgramData\RustDesk\Client\sos\RustDesk\RustDesk.exe" --service"#.into(),
                fallback_start: "auto".into(),
                agent_command: r#"C:\ProgramData\RustDesk\ManagementAgent\rustdesk-management-agent.exe --service --edition sos --package-kind msi --normal-install-path "C:\Program Files\RustDesk""#.into(),
                agent_start: "auto".into(),
                identity_path: r"C:\Windows\ServiceProfiles\LocalService\AppData\Roaming\RustDesk\config\RustDesk.toml".into(),
                product_code: "{71A6889B-6003-42DB-989F-B7AE68260DDB}".into(),
                source_transaction: "d".repeat(32), approval_source: None, total_loss: None, created_at: 100,
                package: artifact("fixture.msi"), client_inventory: artifact("client-inventory.json"),
                agent_backup: artifact("agent-backup.exe"), identity_backup: artifact("identity-backup.toml"),
                config_backup: artifact("config-backup.toml"), bootstrap_backup: artifact("bootstrap-backup.exe"),
                bootstrap_manifest: artifact("bootstrap-backup.json"), helper: artifact("runtime/helper.exe"),
            };
            plan.validate().unwrap();
            let bytes = serde_json::to_vec(&plan).unwrap();
            super::super::state_store::publish(&root.join("local-repair-plan.json"), &bytes)
                .unwrap();
            let handoff = Handoff::new(
                plan.transaction_id.clone(),
                hex::encode(Sha256::digest(&bytes)),
                "e".repeat(64),
                Owner {
                    pid: 100,
                    created_at: 1,
                },
                Owner {
                    pid: 101,
                    created_at: 2,
                },
                100,
            )
            .unwrap();
            save(&root.join("handoff.json"), &handoff).unwrap();
            let ready = handoff.ready_receipt(101).unwrap();
            save(&root.join("local-ready.json"), &ready).unwrap();
            save(
                &root.join("local-release.json"),
                &handoff.release_receipt(&ready, 102).unwrap(),
            )
            .unwrap();
            save(&state.join("agent.json"), &serde_json::json!({"agent_installation_id":plan.installation_id,"disabled":false,"management_agent_enabled":true})).unwrap();
            std::fs::write(root.join("intent.toml"), "[options]\n").unwrap();
            super::super::maintenance::claim(
                &state,
                &plan.installation_id,
                &plan.transaction_id,
                &handoff.plan_sha256,
                &handoff.nonce,
            )
            .unwrap();
            std::fs::write(root.join("task.fixture"), plan.transaction_id.as_bytes()).unwrap();
            let context = EventContext {
                path: root.join("events.json"),
                binding: Binding {
                    client_id: "fixture-device".into(),
                    client_uuid: "fixture-uuid".into(),
                    agent_installation_id: plan.installation_id.clone(),
                    enrollment_generation: 1,
                },
            };
            LocalEvents::open(&context.path)
                .unwrap()
                .remember_binding(context.binding.clone())
                .unwrap();
            Self {
                root,
                state,
                plan,
                handoff,
                context,
            }
        }

        pub(crate) fn transaction(&self) -> Transaction {
            let mut tx = Transaction::new(
                self.plan.transaction_id.clone(),
                self.plan.target_key.clone(),
                self.plan.package.sha256.clone(),
                self.plan.identity_backup.sha256.clone(),
                100,
            )
            .unwrap();
            let path = self.root.join("transaction.json");
            tx.save(&path).unwrap();
            for phase in [
                Phase::PackageVerified,
                Phase::Prepared,
                Phase::StoppingServices,
            ] {
                tx.advance(phase, &evidence(), 103).unwrap();
                tx.save(&path).unwrap();
            }
            self.plan.bind(&tx).unwrap();
            tx
        }

        pub(crate) fn remove_task(&self) -> Result<(), String> {
            assert!(
                !self.state.join("maintenance.json").exists(),
                "release precedes task cleanup"
            );
            std::fs::remove_file(self.root.join("task.fixture")).map_err(|e| e.to_string())
        }

        fn resume(&self, tx: &Transaction) -> Result<(), String> {
            local_recovery_events::with_context_fixture(self.context.clone(), || {
                finish_terminal_resume(
                    &self.root,
                    &self.plan,
                    &self.handoff,
                    tx,
                    &self.state,
                    &self.root.join("intent.toml"),
                    || self.remove_task(),
                )
            })
        }

        fn assert_cleanup(&self, phase: Phase, error: Option<&str>) {
            let receipt: serde_json::Value = serde_json::from_slice(
                &std::fs::read(self.root.join("local-result.json")).unwrap(),
            )
            .unwrap();
            assert_eq!(receipt["transaction_id"], self.plan.transaction_id);
            assert_eq!(receipt["phase"], serde_json::to_value(phase).unwrap());
            assert_eq!(receipt["error"], serde_json::to_value(error).unwrap());
            assert!(!self.state.join("maintenance.json").exists());
            assert!(!self.root.join("task.fixture").exists());
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.root).unwrap();
        }
    }

    #[derive(Clone, Copy)]
    pub(crate) enum Fault {
        None,
        WriterBusy,
        Storage,
    }
    pub(crate) struct Files<'a> {
        pub(crate) fixture: &'a Fixture,
        pub(crate) fault: Fault,
        pub(crate) rollback: bool,
        pub(crate) lock: Option<std::fs::File>,
        pub(crate) calls: Cell<usize>,
    }
    impl Files<'_> {
        fn validation(&mut self) -> Result<Validation, String> {
            BACKEND_CALLS.with(|calls| calls.set(calls.get() + 1));
            self.calls.set(self.calls.get() + 1);
            match self.fault {
                Fault::WriterBusy => {
                    self.lock = Some(
                        std::fs::OpenOptions::new()
                            .write(true)
                            .create(true)
                            .truncate(false)
                            .share_mode(0)
                            .open(self.fixture.context.path.with_extension("lock"))
                            .unwrap(),
                    )
                }
                Fault::Storage => std::fs::create_dir(
                    self.fixture
                        .context
                        .path
                        .with_extension(format!("tmp-{}", std::process::id())),
                )
                .unwrap(),
                Fault::None => {}
            }
            Ok(Validation {
                target_matches: true,
                identity_unchanged: true,
                client_ready: true,
                agent_ready: true,
            })
        }
        pub(crate) fn clear_fault(&mut self) {
            self.lock.take();
            if matches!(self.fault, Fault::Storage) {
                std::fs::remove_dir(
                    self.fixture
                        .context
                        .path
                        .with_extension(format!("tmp-{}", std::process::id())),
                )
                .unwrap();
            }
        }
    }
    impl Backend for Files<'_> {
        fn stop_services(&mut self) -> Result<(), String> {
            BACKEND_CALLS.with(|calls| calls.set(calls.get() + 1));
            self.calls.set(self.calls.get() + 1);
            Ok(())
        }
        fn apply_package(&mut self) -> Result<InstallOutcome, String> {
            BACKEND_CALLS.with(|calls| calls.set(calls.get() + 1));
            self.calls.set(self.calls.get() + 1);
            if self.rollback {
                Err("original fixture installer error 1603".into())
            } else {
                Ok(InstallOutcome::Installed)
            }
        }
        fn validate_target(&mut self) -> Result<Validation, String> {
            self.validation()
        }
        fn rollback_original(&mut self) -> Result<Validation, String> {
            self.validation()
        }
    }

    fn execution_fault(fault: Fault, rollback: bool) {
        let f = Fixture::new();
        let mut tx = f.transaction();
        let mut backend = Files {
            fixture: &f,
            fault,
            rollback,
            lock: None,
            calls: Cell::new(0),
        };
        let phase = if rollback {
            Phase::RolledBack
        } else {
            Phase::Committed
        };
        let expected_error = rollback.then_some("original fixture installer error 1603");
        let result = local_recovery_events::with_context_fixture(f.context.clone(), || {
            dispatch(
                &f.root.join("transaction.json"),
                &mut tx,
                &evidence(),
                &mut backend,
                104,
                false,
                None,
                || Ok(()),
            )
        });
        assert_eq!(tx.phase, phase);
        assert_eq!(
            Transaction::decode(&std::fs::read(f.root.join("transaction.json")).unwrap()).unwrap(),
            tx
        );
        let pending = LocalEvents::open(&f.context.path)
            .unwrap()
            .pending_events()
            .to_vec();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].payload["result"]["phase"], "started");
        let returned = result
            .as_ref()
            .map(|value| (value.phase, value.error.clone()))
            .map_err(Clone::clone);
        let finished = finish_execution(&f.root, &f.plan, &f.handoff, &f.state, result, || {
            f.remove_task()
        });
        assert!(
            finished.is_ok(),
            "real execution finalization: {finished:?}"
        );
        assert_eq!(
            returned,
            Ok((phase, expected_error.map(str::to_owned))),
            "event failure must preserve ExecutionResult"
        );
        f.assert_cleanup(phase, expected_error);
        assert_eq!(
            LocalEvents::open(&f.context.path).unwrap().pending_events(),
            pending
        );
        let calls = backend.calls.get();
        backend.clear_fault();
        let before = std::fs::read(f.root.join("transaction.json")).unwrap();
        local_recovery_events::with_context_fixture(f.context.clone(), || {
            local_recovery_events::report_repair_terminal(&tx, &f.plan.installation_id, 105)
        })
        .unwrap();
        assert_eq!(
            LocalEvents::open(&f.context.path)
                .unwrap()
                .pending_events()
                .len(),
            2
        );
        assert_eq!(backend.calls.get(), calls);
        assert_eq!(
            std::fs::read(f.root.join("transaction.json")).unwrap(),
            before
        );
    }

    #[test]
    fn committed_writer_busy_keeps_result_and_finalizes() {
        execution_fault(Fault::WriterBusy, false);
    }
    #[test]
    fn committed_storage_keeps_result_and_finalizes() {
        execution_fault(Fault::Storage, false);
    }
    #[test]
    fn rolled_back_writer_busy_keeps_original_error_and_finalizes() {
        execution_fault(Fault::WriterBusy, true);
    }
    #[test]
    fn rolled_back_storage_keeps_original_error_and_finalizes() {
        execution_fault(Fault::Storage, true);
    }

    fn terminal_fixture(rollback: bool) -> (Fixture, Transaction) {
        let f = Fixture::new();
        let mut tx = f.transaction();
        let result = {
            let mut backend = Files {
                fixture: &f,
                fault: Fault::None,
                rollback,
                lock: None,
                calls: Cell::new(0),
            };
            local_recovery_events::with_context_fixture(f.context.clone(), || {
                dispatch(
                    &f.root.join("transaction.json"),
                    &mut tx,
                    &evidence(),
                    &mut backend,
                    104,
                    false,
                    None,
                    || Ok(()),
                )
            })
            .unwrap()
        };
        save(&f.root.join("local-result.json"), &serde_json::json!({"schema":1,"transaction_id":tx.id,"phase":result.phase,"error":result.error,"reboot_required":result.reboot_required})).unwrap();
        (f, tx)
    }

    #[test]
    fn terminal_resume_existing_receipt_finishes_without_backend() {
        for rollback in [false, true] {
            let (f, tx) = terminal_fixture(rollback);
            let before = std::fs::read(f.root.join("transaction.json")).unwrap();
            let receipt = std::fs::read(f.root.join("local-result.json")).unwrap();
            f.resume(&tx).unwrap();
            f.assert_cleanup(
                tx.phase,
                rollback.then_some("original fixture installer error 1603"),
            );
            assert_eq!(
                std::fs::read(f.root.join("transaction.json")).unwrap(),
                before
            );
            assert_eq!(
                std::fs::read(f.root.join("local-result.json")).unwrap(),
                receipt
            );
        }
    }

    #[test]
    fn terminal_resume_foreign_marker_preserves_receipt_and_task() {
        let (f, tx) = terminal_fixture(false);
        let marker = f.state.join("maintenance.json");
        let mut value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&marker).unwrap()).unwrap();
        value["transaction_id"] = "f".repeat(32).into();
        save(&marker, &value).unwrap();
        let before = std::fs::read(&marker).unwrap();
        let receipt = std::fs::read(f.root.join("local-result.json")).unwrap();
        assert!(f
            .resume(&tx)
            .unwrap_err()
            .contains("maintenance binding mismatch"));
        assert_eq!(std::fs::read(&marker).unwrap(), before);
        assert_eq!(
            std::fs::read(f.root.join("local-result.json")).unwrap(),
            receipt
        );
        assert!(f.root.join("task.fixture").exists());
    }

    #[test]
    fn terminal_resume_foreign_result_preserves_marker_and_receipt() {
        let (f, tx) = terminal_fixture(true);
        let path = f.root.join("local-result.json");
        let mut value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        value["transaction_id"] = "f".repeat(32).into();
        save(&path, &value).unwrap();
        let before = std::fs::read(&path).unwrap();
        assert!(f
            .resume(&tx)
            .unwrap_err()
            .contains("result binding mismatch"));
        assert_eq!(std::fs::read(path).unwrap(), before);
        assert!(f.state.join("maintenance.json").exists());
        assert!(f.root.join("task.fixture").exists());
    }

    #[test]
    fn terminal_resume_missing_rollback_result_never_invents_success() {
        let (f, tx) = terminal_fixture(true);
        std::fs::remove_file(f.root.join("local-result.json")).unwrap();
        let before = std::fs::read(f.root.join("transaction.json")).unwrap();
        assert!(f
            .resume(&tx)
            .unwrap_err()
            .contains("terminal result evidence unavailable"));
        assert!(!f.root.join("local-result.json").exists());
        assert_eq!(
            std::fs::read(f.root.join("transaction.json")).unwrap(),
            before
        );
        assert!(f.state.join("maintenance.json").exists());
        assert!(f.root.join("task.fixture").exists());
    }

    #[test]
    fn terminal_resume_committed_missing_result_reconstructs_from_bound_durable_transaction() {
        let (f, tx) = terminal_fixture(false);
        std::fs::remove_file(f.root.join("local-result.json")).unwrap();
        let before = std::fs::read(f.root.join("transaction.json")).unwrap();
        f.resume(&tx).unwrap();
        f.assert_cleanup(Phase::Committed, None);
        assert_eq!(
            std::fs::read(f.root.join("transaction.json")).unwrap(),
            before
        );
    }

    #[test]
    fn terminal_resume_missing_result_rejects_foreign_transaction() {
        let (f, mut tx) = terminal_fixture(false);
        std::fs::remove_file(f.root.join("local-result.json")).unwrap();
        tx.id = "f".repeat(32);
        let before = std::fs::read(f.root.join("transaction.json")).unwrap();
        assert!(f
            .resume(&tx)
            .unwrap_err()
            .contains("transaction binding mismatch"));
        assert!(!f.root.join("local-result.json").exists());
        assert_eq!(
            std::fs::read(f.root.join("transaction.json")).unwrap(),
            before
        );
        assert!(f.state.join("maintenance.json").exists());
        assert!(f.root.join("task.fixture").exists());
    }

    #[test]
    fn terminal_resume_event_writer_failure_does_not_block_receipt_cleanup() {
        let f = Fixture::new();
        let mut tx = f.transaction();
        let mut backend = Files {
            fixture: &f,
            fault: Fault::WriterBusy,
            rollback: true,
            lock: None,
            calls: Cell::new(0),
        };
        let _result = local_recovery_events::with_context_fixture(f.context.clone(), || {
            dispatch(
                &f.root.join("transaction.json"),
                &mut tx,
                &evidence(),
                &mut backend,
                104,
                false,
                None,
                || Ok(()),
            )
        });
        assert_eq!(tx.phase, Phase::RolledBack);
        // Emulate interruption after the original owner durably saved its exact execution result.
        save(&f.root.join("local-result.json"), &serde_json::json!({"schema":1,"transaction_id":tx.id,"phase":tx.phase,"error":"original fixture installer error 1603","reboot_required":false})).unwrap();
        let pending = LocalEvents::open(&f.context.path)
            .unwrap()
            .pending_events()
            .to_vec();
        let before = std::fs::read(f.root.join("transaction.json")).unwrap();
        f.resume(&tx).unwrap();
        f.assert_cleanup(
            Phase::RolledBack,
            Some("original fixture installer error 1603"),
        );
        assert_eq!(
            LocalEvents::open(&f.context.path).unwrap().pending_events(),
            pending
        );
        assert_eq!(
            std::fs::read(f.root.join("transaction.json")).unwrap(),
            before
        );
        backend.clear_fault();
    }

    #[test]
    fn terminal_resume_local_disable_retains_all_evidence() {
        for marker in ["disable-agent", "disable-recovery"] {
            let (f, tx) = terminal_fixture(false);
            std::fs::write(f.state.join(marker), b"explicit intent").unwrap();
            let result = std::fs::read(f.root.join("local-result.json")).unwrap();
            assert!(f.resume(&tx).unwrap_err().contains("locally blocked"));
            assert_eq!(
                std::fs::read(f.root.join("local-result.json")).unwrap(),
                result
            );
            assert!(f.state.join("maintenance.json").exists());
            assert!(f.root.join("task.fixture").exists());
        }
    }

    #[test]
    fn terminal_resume_release_nonce_mismatch_retains_marker_and_result() {
        let (f, tx) = terminal_fixture(false);
        let path = f.root.join("local-release.json");
        let mut receipt: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        receipt["nonce"] = "f".repeat(64).into();
        save(&path, &receipt).unwrap();
        let before = std::fs::read(&path).unwrap();
        assert!(f
            .resume(&tx)
            .unwrap_err()
            .contains("release binding mismatch"));
        assert_eq!(std::fs::read(path).unwrap(), before);
        assert!(f.state.join("maintenance.json").exists());
        assert!(f.root.join("task.fixture").exists());
    }

    #[test]
    fn terminal_resume_wrong_result_semantics_preserve_evidence() {
        for field in ["phase", "error", "reboot_required", "schema"] {
            let (f, tx) = terminal_fixture(true);
            let path = f.root.join("local-result.json");
            let mut receipt: serde_json::Value =
                serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
            receipt[field] = match field {
                "phase" => "committed".into(),
                "error" => serde_json::Value::Null,
                "reboot_required" => true.into(),
                _ => 2.into(),
            };
            save(&path, &receipt).unwrap();
            let before = std::fs::read(&path).unwrap();
            assert!(f
                .resume(&tx)
                .unwrap_err()
                .contains("result binding mismatch"));
            assert_eq!(std::fs::read(path).unwrap(), before);
            assert!(f.state.join("maintenance.json").exists());
            assert!(f.root.join("task.fixture").exists());
        }
    }

    #[test]
    fn terminal_resume_error_option_boundaries_preserve_evidence() {
        for rollback in [false, true] {
            for error in [None, Some(""), Some("original fixture installer error 1603")] {
                let (f, tx) = terminal_fixture(rollback);
                let path = f.root.join("local-result.json");
                let mut receipt: serde_json::Value =
                    serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
                receipt["error"] = serde_json::json!(error);
                save(&path, &receipt).unwrap();
                let before = std::fs::read(&path).unwrap();
                let transaction = std::fs::read(f.root.join("transaction.json")).unwrap();
                let marker = std::fs::read(f.state.join("maintenance.json")).unwrap();
                let task = std::fs::read(f.root.join("task.fixture")).unwrap();
                let accepted = if rollback {
                    error == Some("original fixture installer error 1603")
                } else {
                    error.is_none()
                };
                let result = f.resume(&tx);
                if accepted {
                    result.unwrap();
                    f.assert_cleanup(tx.phase, error);
                } else {
                    assert_eq!(result.unwrap_err(), "local repair result binding mismatch");
                    assert_eq!(
                        std::fs::read(f.state.join("maintenance.json")).unwrap(),
                        marker
                    );
                    assert_eq!(std::fs::read(f.root.join("task.fixture")).unwrap(), task);
                }
                assert_eq!(std::fs::read(&path).unwrap(), before);
                assert_eq!(
                    std::fs::read(f.root.join("transaction.json")).unwrap(),
                    transaction
                );
            }
        }
    }

    #[test]
    fn terminal_resume_after_marker_release_retries_task_only() {
        let (f, tx) = terminal_fixture(true);
        let receipt = std::fs::read(f.root.join("local-result.json")).unwrap();
        let result = local_recovery_events::with_context_fixture(f.context.clone(), || {
            finish_terminal_resume(
                &f.root,
                &f.plan,
                &f.handoff,
                &tx,
                &f.state,
                &f.root.join("intent.toml"),
                || Err("task fixture unavailable".into()),
            )
        });
        assert_eq!(result, Err("task fixture unavailable".into()));
        assert!(!f.state.join("maintenance.json").exists());
        assert!(f.root.join("task.fixture").exists());
        f.resume(&tx).unwrap();
        f.assert_cleanup(tx.phase, Some("original fixture installer error 1603"));
        assert_eq!(
            std::fs::read(f.root.join("local-result.json")).unwrap(),
            receipt
        );
    }

    #[test]
    fn terminal_resume_stale_snapshot_cannot_finalize() {
        let (f, mut tx) = terminal_fixture(false);
        tx.revision += 1;
        let before = std::fs::read(f.root.join("transaction.json")).unwrap();
        assert!(f
            .resume(&tx)
            .unwrap_err()
            .contains("transaction binding mismatch"));
        assert_eq!(
            std::fs::read(f.root.join("transaction.json")).unwrap(),
            before
        );
        assert!(f.state.join("maintenance.json").exists());
        assert!(f.root.join("task.fixture").exists());
    }

    #[test]
    fn rollback_failed_retains_original_error_and_blocks_cleanup() {
        let f = Fixture::new();
        let mut tx = f.transaction();
        tx.advance(Phase::RollingBack, &evidence(), 104).unwrap();
        tx.save(&f.root.join("transaction.json")).unwrap();
        tx.advance(Phase::RollbackFailed, &evidence(), 105).unwrap();
        tx.save(&f.root.join("transaction.json")).unwrap();
        let result = finish_execution(
            &f.root,
            &f.plan,
            &f.handoff,
            &f.state,
            Ok(install_execution::ExecutionResult {
                phase: Phase::RollbackFailed,
                error: Some("original; rollback failed".into()),
                reboot_required: false,
            }),
            || panic!("no task cleanup"),
        );
        assert!(result.unwrap_err().contains("RollbackFailed"));
        let receipt = std::fs::read(f.root.join("local-result.json")).unwrap();
        assert!(String::from_utf8_lossy(&receipt).contains("original; rollback failed"));
        assert!(f.resume(&tx).unwrap_err().contains("manual reconciliation"));
        assert_eq!(
            std::fs::read(f.root.join("local-result.json")).unwrap(),
            receipt
        );
        assert!(f.state.join("maintenance.json").exists());
        assert!(f.root.join("task.fixture").exists());
    }

    #[test]
    fn terminal_resume_foreign_event_binding_blocks_cleanup() {
        let (f, tx) = terminal_fixture(false);
        let mut foreign = f.context.clone();
        foreign.binding.agent_installation_id = "f".repeat(32);
        let before = std::fs::read(&f.context.path).unwrap();
        let result = local_recovery_events::with_context_fixture(foreign, || {
            finish_terminal_resume(
                &f.root,
                &f.plan,
                &f.handoff,
                &tx,
                &f.state,
                &f.root.join("intent.toml"),
                || panic!("foreign event cannot authorize cleanup"),
            )
        });
        assert!(result.unwrap_err().contains("event identity mismatch"));
        assert_eq!(std::fs::read(&f.context.path).unwrap(), before);
        assert!(f.state.join("maintenance.json").exists());
        assert!(f.root.join("task.fixture").exists());
    }
}
