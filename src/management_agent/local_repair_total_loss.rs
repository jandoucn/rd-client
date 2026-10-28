use serde::{Deserialize, Serialize};

#[cfg(windows)]
#[path = "preservation_state_read.rs"]
mod state_read;
#[cfg(windows)]
use super::super::workspace;

#[cfg(test)]
pub fn verified_state_read_fixture(
    state: &std::path::Path,
    intent: &std::path::Path,
) -> Result<(), String> {
    state_read::verify(state, intent).map(|_| ())
}

#[cfg(all(windows, test))]
std::thread_local! { static PRODUCT_FIXTURE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) }; }
#[cfg(all(windows, test))]
pub fn product_fixture<T>(run: impl FnOnce() -> T) -> T {
    base::lab_guard::require_lab_os("win11");
    struct Reset(bool);
    impl Drop for Reset {
        fn drop(&mut self) {
            PRODUCT_FIXTURE.with(|v| v.set(self.0));
        }
    }
    let _reset = Reset(PRODUCT_FIXTURE.with(|v| v.replace(true)));
    run()
}

#[cfg(windows)]
pub struct AgentObservation {
    pub stopped: bool,
    pub disabled: bool,
    pub process_present: bool,
}

// This slice publishes only the missing agent; the caller supplies SCM observation and verification.
#[cfg(windows)]
pub fn preserve_missing_agent(
    directory: &std::path::Path,
    state: &std::path::Path,
    intent: &std::path::Path,
    identity: &std::path::Path,
    destination: &std::path::Path,
    expected: &Binding,
    installation_error: String,
    observe: impl Fn() -> Result<AgentObservation, String>,
    verify: impl FnOnce() -> Result<(), String>,
) -> PreservationReceipt {
    preserve_agent_checked(
        directory,
        state,
        intent,
        identity,
        destination,
        expected,
        installation_error,
        observe,
        verify,
        false,
    )
}

#[cfg(windows)]
fn preserve_agent_checked(
    directory: &std::path::Path,
    state: &std::path::Path,
    intent: &std::path::Path,
    identity: &std::path::Path,
    destination: &std::path::Path,
    expected: &Binding,
    installation_error: String,
    observe: impl Fn() -> Result<AgentObservation, String>,
    verify: impl FnOnce() -> Result<(), String>,
    existing_allowed: bool,
) -> PreservationReceipt {
    use super::super::{
        agent_restore,
        handoff::Handoff,
        install_guard::InstallGuard,
        local_repair_plan::LocalRepairPlan,
        maintenance, package_gate,
        transaction::{Phase, Transaction},
        workspace,
    };
    use sha2::{Digest, Sha256};
    use std::{io::Read, path::Path};
    fn read(path: &Path) -> Result<Vec<u8>, String> {
        workspace::verify_file(path)?;
        let mut bytes = Vec::new();
        std::fs::File::open(path)
            .map_err(|e| e.to_string())?
            .take(65537)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        if bytes.len() > 65536 {
            return Err("total loss record too large".into());
        }
        Ok(bytes)
    }
    let mut identity_unchanged = false;
    let mut guard = None;
    let mut bound = false;
    let result = (|| {
        guard = Some(
            InstallGuard::try_acquire()
                .map_err(|e| e.to_string())?
                .ok_or("total loss installation lock busy")?,
        );
        #[cfg(not(test))]
        if destination
            != crate::management_agent::system_dirs::get()
                .agent()
                .join("rustdesk-management-agent.exe")
            || state
                != crate::management_agent::system_dirs::get()
                    .agent()
                    .join("state")
            || directory
                != crate::management_agent::system_dirs::get()
                    .agent()
                    .join("transactions")
                    .join(&expected.transaction_id)
            || intent != hbb_common::config::Config2::file()
        {
            return Err("total loss fixed paths invalid".into());
        }
        let plan_bytes = read(&directory.join("local-repair-plan.json"))?;
        let plan = LocalRepairPlan::decode(&plan_bytes)?;
        if plan.total_loss.is_none() || installation_error.is_empty() {
            return Err("total loss preservation context invalid".into());
        }
        #[cfg(not(test))]
        if identity != Path::new(&plan.identity_path) {
            return Err("total loss identity path invalid".into());
        }
        let handoff_bytes = read(&directory.join("handoff.json"))?;
        let handoff = Handoff::decode(&handoff_bytes)?;
        if expected.transaction_id != plan.transaction_id
            || expected.installation_id != plan.installation_id
            || expected.plan_sha256 != hex::encode(Sha256::digest(&plan_bytes))
            || expected.handoff_sha256 != hex::encode(Sha256::digest(&handoff_bytes))
            || handoff.transaction_id != plan.transaction_id
            || handoff.plan_sha256 != expected.plan_sha256
        {
            return Err("total loss preservation binding mismatch".into());
        }
        let check = || -> Result<(), String> {
            if read(&directory.join("local-repair-plan.json"))? != plan_bytes
                || read(&directory.join("handoff.json"))? != handoff_bytes
            {
                return Err("total loss records changed".into());
            }
            let tx = Transaction::decode(&read(&directory.join("transaction.json"))?)?;
            plan.bind(&tx)?;
            if tx.phase != Phase::RollingBack {
                return Err("total loss preservation phase invalid".into());
            }
            maintenance::verify(
                state,
                intent,
                &plan.installation_id,
                &plan.transaction_id,
                &expected.plan_sha256,
                &handoff.nonce,
            )?;
            workspace::verify_restore_target(&state.join("service-recovery-enabled"), false)
                .map_err(|e| format!("total loss recovery opt-in revoked or invalid: {e}"))?;
            let target = hbb_common::UpdateTarget {
                size: plan.identity_backup.size,
                sha256: plan.identity_backup.sha256.clone(),
                ..Default::default()
            };
            let _backup = package_gate::lock_verified_file(
                &directory.join(&plan.identity_backup.relative_path),
                &target,
            )?;
            workspace::verify_restore_target(identity, true)?;
            let _live = package_gate::lock_verified_file(identity, &target)?;
            Ok(())
        };
        let before_publish = || {
            check()?;
            let observation = observe()?;
            agent_restore::gate(
                if observation.stopped {
                    "stopped"
                } else {
                    "unknown"
                },
                observation.disabled,
                observation.process_present,
            )?;
            check()
        };
        check()?;
        bound = true;
        match std::fs::symlink_metadata(directory.join("agent-preservation.json")) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            _ => return Err("total loss receipt occupied or unreadable; preserve evidence".into()),
        }
        let existing = match std::fs::symlink_metadata(destination) {
            Ok(_) if existing_allowed => true,
            Ok(_) => false,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
            Err(e) => return Err(format!("total loss destination unknown: {e}")),
        };
        if existing {
            workspace::verify_restore_target(destination, false)?;
            let _file = package_gate::lock_verified_file(
                destination,
                &hbb_common::UpdateTarget {
                    size: plan.agent_backup.size,
                    sha256: plan.agent_backup.sha256.clone(),
                    ..Default::default()
                },
            )?;
            check()?;
        } else {
            let parent = destination
                .parent()
                .ok_or("total loss destination parent missing")?;
            if parent == crate::management_agent::system_dirs::get().agent() {
                workspace::verify_agent_install_directory(parent)?;
            } else {
                workspace::verify(parent)?;
            }
            agent_restore::publish_checked(
                &directory.join(&plan.agent_backup.relative_path),
                destination,
                plan.agent_backup.size,
                &plan.agent_backup.sha256,
                &before_publish,
                &before_publish,
            )?;
        }
        check()?;
        verify()?;
        check()?;
        let _published = package_gate::lock_verified_file(
            destination,
            &hbb_common::UpdateTarget {
                size: plan.agent_backup.size,
                sha256: plan.agent_backup.sha256.clone(),
                ..Default::default()
            },
        )?;
        identity_unchanged = true;
        Ok(())
    })();
    let mut receipt = PreservationReceipt::new(
        expected.clone(),
        installation_error,
        result,
        identity_unchanged,
    );
    if bound {
        if let Err(error) = persist_receipt(directory, &receipt) {
            receipt.persistence_error = Some(error);
        }
    }
    receipt
}

#[cfg(windows)]
fn persist_receipt(
    directory: &std::path::Path,
    receipt: &PreservationReceipt,
) -> Result<(), String> {
    use super::super::{agent_restore, workspace};
    use sha2::{Digest, Sha256};
    use std::io::Write;
    let run = || -> Result<(), String> {
        workspace::verify(directory)?;
        let stage = directory.join("agent-preservation-staging");
        workspace::create(&stage)?;
        let source = stage.join("receipt.json");
        let bytes = serde_json::to_vec(receipt).map_err(|e| e.to_string())?;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&source)
            .map_err(|e| e.to_string())?;
        file.write_all(&bytes)
            .and_then(|_| file.sync_all())
            .map_err(|e| e.to_string())?;
        drop(file);
        agent_restore::publish_checked(
            &source,
            &directory.join("agent-preservation.json"),
            bytes.len() as u64,
            &hex::encode(Sha256::digest(&bytes)),
            || verify_receipt_binding(directory, &receipt.binding),
            || verify_receipt_binding(directory, &receipt.binding),
        )
    };
    run().map_err(|e| format!("total loss receipt persistence failed: {e}"))
}

#[cfg(windows)]
fn receipt_read(path: &std::path::Path) -> Result<Vec<u8>, String> {
    use std::io::Read;
    super::super::workspace::verify_file(path)?;
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .map_err(|e| e.to_string())?
        .take(65537)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > 65536 {
        return Err("total loss receipt record too large".into());
    }
    Ok(bytes)
}

#[cfg(windows)]
fn verify_receipt_binding(directory: &std::path::Path, binding: &Binding) -> Result<(), String> {
    use sha2::{Digest, Sha256};
    let bytes = receipt_read(&directory.join("local-repair-plan.json"))?;
    let handoff_bytes = receipt_read(&directory.join("handoff.json"))?;
    let plan = super::LocalRepairPlan::decode(&bytes)?;
    let handoff = super::super::handoff::Handoff::decode(&handoff_bytes)?;
    let tx = super::super::transaction::Transaction::decode(&receipt_read(
        &directory.join("transaction.json"),
    )?)?;
    plan.bind(&tx)?;
    if plan.total_loss.is_none()
        || tx.phase != super::super::transaction::Phase::RollingBack
        || binding.transaction_id != tx.id
        || binding.installation_id != plan.installation_id
        || binding.plan_sha256 != hex::encode(Sha256::digest(&bytes))
        || binding.handoff_sha256 != hex::encode(Sha256::digest(&handoff_bytes))
        || handoff.transaction_id != tx.id
        || handoff.plan_sha256 != binding.plan_sha256
    {
        return Err("total loss receipt current binding mismatch".into());
    }
    Ok(())
}

#[cfg(windows)]
pub fn reopen_receipt(
    directory: &std::path::Path,
    expected: &Binding,
) -> Result<PreservationReceipt, String> {
    verify_receipt_binding(directory, expected)?;
    let receipt: PreservationReceipt =
        serde_json::from_slice(&receipt_read(&directory.join("agent-preservation.json"))?)
            .map_err(|_| "total loss receipt malformed")?;
    receipt.verify(expected)?;
    Ok(receipt)
}

#[cfg(windows)]
pub fn completed_preservation(
    directory: &std::path::Path,
) -> Result<super::LocalRepairPlan, String> {
    completed_preservation_for(directory, None)
}

#[cfg(windows)]
#[derive(Clone, Copy)]
pub(crate) enum CurrentExecution<'a> {
    OriginalHelper(&'a super::super::handoff::Handoff),
    ResumeAuthorized(&'a super::super::local_repair::ResumeAuthorized<'a>),
}

#[cfg(windows)]
pub(crate) fn completed_preservation_for(
    directory: &std::path::Path,
    current: Option<(&std::path::Path, CurrentExecution<'_>)>,
) -> Result<super::LocalRepairPlan, String> {
    use super::super::{
        handoff::{Handoff, ObservedProcess, Owner, ReleaseReceipt},
        maintenance, workspace,
    };
    use sha2::{Digest, Sha256};
    use std::path::Path;
    fn absent(path: &Path) -> Result<(), String> {
        match std::fs::symlink_metadata(path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            _ => Err("completed preservation cleanup absent/unknown refused".into()),
        }
    }
    workspace::verify(directory)?;
    if directory
        .parent()
        .and_then(Path::file_name)
        .and_then(|v| v.to_str())
        != Some("transactions")
    {
        return Err("completed preservation transaction namespace mismatch".into());
    }
    let agent = directory
        .parent()
        .and_then(Path::parent)
        .ok_or("completed preservation root missing")?;
    #[cfg(not(test))]
    if agent != crate::management_agent::system_dirs::get().agent() {
        return Err("completed preservation fixed root mismatch".into());
    }
    let bytes = receipt_read(&directory.join("local-repair-plan.json"))?;
    let plan = super::LocalRepairPlan::decode(&bytes)?;
    if directory.file_name().and_then(|v| v.to_str()) != Some(plan.transaction_id.as_str()) {
        return Err("completed preservation directory mismatch".into());
    }
    let handoff_bytes = receipt_read(&directory.join("handoff.json"))?;
    let handoff = Handoff::decode(&handoff_bytes)?;
    let binding = Binding {
        transaction_id: plan.transaction_id.clone(),
        installation_id: plan.installation_id.clone(),
        plan_sha256: hex::encode(Sha256::digest(&bytes)),
        handoff_sha256: hex::encode(Sha256::digest(&handoff_bytes)),
    };
    let receipt = reopen_receipt(directory, &binding)?;
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Reconciled {
        schema: u32,
        transaction_id: String,
        state: String,
        receipt: PreservationReceipt,
        observed_helper: Owner,
    }
    let reconciled: Reconciled = serde_json::from_slice(&receipt_read(
        &directory.join("total-loss-reconciliation.json"),
    )?)
    .map_err(|_| "completed preservation reconciliation malformed")?;
    if reconciled.schema != 1
        || reconciled.transaction_id != plan.transaction_id
        || reconciled.state != "agent_preserved_client_unrecovered"
        || reconciled.receipt != receipt
    {
        return Err("completed preservation reconciliation mismatch".into());
    }
    let ready = serde_json::from_slice(&receipt_read(&directory.join("local-ready.json"))?)
        .map_err(|_| "completed preservation ready malformed")?;
    let release: ReleaseReceipt =
        serde_json::from_slice(&receipt_read(&directory.join("local-release.json"))?)
            .map_err(|_| "completed preservation release malformed")?;
    handoff.verify_release(&ready, &release, &handoff.helper, release.released_at)?;
    for owner in [&handoff.helper, &reconciled.observed_helper] {
        if ObservedProcess::owner_alive(owner)? {
            return Err("completed preservation active owner".into());
        }
    }
    match std::fs::symlink_metadata(directory.join("total-loss-resume-owner.json")) {
        Ok(_) => {
            let owner: ResumeOwner = serde_json::from_slice(&receipt_read(
                &directory.join("total-loss-resume-owner.json"),
            )?)
            .map_err(|_| "completed preservation resume owner malformed")?;
            if owner.schema != 1
                || owner.binding != binding
                || ObservedProcess::owner_alive(&owner.owner)?
            {
                return Err("completed preservation resume owner mismatch/active".into());
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.to_string()),
    }
    let result: serde_json::Value =
        serde_json::from_slice(&receipt_read(&directory.join("local-result.json"))?)
            .map_err(|_| "completed preservation result malformed")?;
    if result["schema"] != 1
        || result["transaction_id"] != plan.transaction_id
        || result["phase"] != "preservation"
        || result["receipt"] != serde_json::to_value(&receipt).map_err(|e| e.to_string())?
    {
        return Err("completed preservation result mismatch".into());
    }
    let cause: serde_json::Value = serde_json::from_slice(&receipt_read(
        &directory.join("total-loss-install-error.json"),
    )?)
    .map_err(|_| "completed preservation error malformed")?;
    if cause["transaction_id"] != plan.transaction_id
        || cause["plan_sha256"] != binding.plan_sha256
        || cause["error"] != receipt.installation_error
    {
        return Err("completed preservation original error mismatch".into());
    }
    let state = agent.join("state");
    #[cfg(not(test))]
    let intent = hbb_common::config::Config2::file();
    #[cfg(test)]
    let intent = if PRODUCT_FIXTURE.with(|v| v.get()) {
        hbb_common::config::Config2::file()
    } else {
        agent.join("intent.toml")
    };
    let _state_read = state_read::verify(&state, &intent)?;
    if let Some((current_directory, context)) = current {
        let expected_handoff = match context {
            CurrentExecution::OriginalHelper(value) => value,
            CurrentExecution::ResumeAuthorized(value) => value.binding(),
        };
        if current_directory.parent() != directory.parent() || current_directory == directory {
            return Err("completed preservation current namespace mismatch".into());
        }
        workspace::verify(current_directory)?;
        let current_bytes = receipt_read(&current_directory.join("local-repair-plan.json"))?;
        let current_plan = super::LocalRepairPlan::decode(&current_bytes)?;
        let current_tx = super::super::transaction::Transaction::decode(&receipt_read(
            &current_directory.join("transaction.json"),
        )?)?;
        current_plan.bind(&current_tx)?;
        let actual_handoff =
            Handoff::decode(&receipt_read(&current_directory.join("handoff.json"))?)?;
        if &actual_handoff != expected_handoff
            || current_plan.installation_id != plan.installation_id
            || current_plan.transaction_id != actual_handoff.transaction_id
            || current_directory.file_name().and_then(|v| v.to_str())
                != Some(current_tx.id.as_str())
            || actual_handoff.plan_sha256 != hex::encode(Sha256::digest(&current_bytes))
            || !matches!(
                current_tx.phase,
                super::super::transaction::Phase::StoppingServices
                    | super::super::transaction::Phase::ServicesStopped
                    | super::super::transaction::Phase::Applying
                    | super::super::transaction::Phase::Validating
                    | super::super::transaction::Phase::RollingBack
            )
        {
            return Err("completed preservation current execution mismatch".into());
        }
        let owner = ObservedProcess::open(std::process::id())?;
        if !owner.is_alive()? {
            return Err("completed preservation current helper unavailable".into());
        }
        match context {
            CurrentExecution::OriginalHelper(_) if owner.owner() != &actual_handoff.helper => {
                return Err("completed preservation original helper mismatch".into())
            }
            CurrentExecution::ResumeAuthorized(value) => {
                if owner.owner() != value.owner()
                    || !value.alive()?
                    || ObservedProcess::owner_alive(&actual_handoff.helper)?
                {
                    return Err("completed preservation resume helper mismatch".into());
                }
            }
            _ => {}
        }
        let current_ready =
            serde_json::from_slice(&receipt_read(&current_directory.join("local-ready.json"))?)
                .map_err(|_| "current ready malformed")?;
        let current_release: ReleaseReceipt = serde_json::from_slice(&receipt_read(
            &current_directory.join("local-release.json"),
        )?)
        .map_err(|_| "current release malformed")?;
        actual_handoff.verify_release(
            &current_ready,
            &current_release,
            &actual_handoff.helper,
            current_release.released_at,
        )?;
        maintenance::verify(
            &state,
            &intent,
            &current_plan.installation_id,
            &current_plan.transaction_id,
            &actual_handoff.plan_sha256,
            &actual_handoff.nonce,
        )?;
    } else {
        maintenance::verify_terminal_cleanup(
            &state,
            &intent,
            &plan.installation_id,
            &plan.transaction_id,
            &binding.plan_sha256,
            &handoff.nonce,
        )?;
        absent(&state.join("maintenance.json"))?;
    }
    workspace::verify_restore_target(&state.join("service-recovery-enabled"), false)?;
    #[cfg(not(test))]
    let tasks = crate::management_agent::system_dirs::get()
        .system32()
        .join("Tasks");
    #[cfg(test)]
    let tasks = if PRODUCT_FIXTURE.with(|v| v.get()) {
        crate::management_agent::system_dirs::get()
            .system32()
            .join("Tasks")
    } else {
        agent.join("tasks")
    };
    // Observe the parent first: an unavailable task namespace is not evidence of cleanup.
    let metadata = std::fs::symlink_metadata(&tasks).map_err(|e| e.to_string())?;
    use std::os::windows::fs::MetadataExt;
    if !metadata.is_dir() || metadata.file_attributes() & 0x400 != 0 {
        return Err("completed preservation tasks namespace invalid".into());
    }
    absent(&tasks.join(format!(
        "RustDeskManagementAgentResume-{}",
        plan.transaction_id
    )))?;
    Ok(plan)
}

#[cfg(windows)]
fn native_service(
    plan: &super::LocalRepairPlan,
    access: windows_service::service::ServiceAccess,
) -> Result<windows_service::service::Service, String> {
    use windows_service::{
        service::ServiceAccess,
        service_manager::{ServiceManager, ServiceManagerAccess},
    };
    plan.validate()?;
    #[cfg(not(test))]
    let name = "RustDeskManagementAgent";
    #[cfg(test)]
    let name = if PRODUCT_FIXTURE.with(|v| v.get()) {
        "RustDeskManagementAgent"
    } else {
        "RustDeskAgentTestService-TotalLossNative"
    };
    let scm = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)
        .map_err(|e| format!("total loss SCM unavailable: {e}"))?;
    let service = scm
        .open_service(
            name,
            access | ServiceAccess::QUERY_CONFIG | ServiceAccess::QUERY_STATUS,
        )
        .map_err(|e| format!("total loss SCM service unknown or access denied: {e}"))?;
    validate_native_service(plan, &service)?;
    Ok(service)
}

#[cfg(windows)]
fn validate_native_service(
    plan: &super::LocalRepairPlan,
    service: &windows_service::service::Service,
) -> Result<(), String> {
    use windows_service::service::{ServiceStartType, ServiceType};
    plan.validate()?;
    let config = service
        .query_config()
        .map_err(|e| format!("total loss SCM config unavailable: {e}"))?;
    #[cfg(not(test))]
    let command = plan.agent_command.as_str();
    #[cfg(test)]
    let command = if PRODUCT_FIXTURE.with(|v| v.get()) {
        plan.agent_command.as_str()
    } else {
        r"C:\RustDeskAgentLab\total-loss-native\fixture-agent.exe --exact total_loss_native_service_entry --nocapture"
    };
    #[cfg(not(test))]
    let command_matches = config.executable_path.to_string_lossy() == command;
    #[cfg(test)]
    let command_matches = config.executable_path.to_string_lossy() == command
        || config.executable_path.to_string_lossy()
            == r#""C:\RustDeskAgentLab\total-loss-native\fixture-agent.exe" --exact total_loss_native_service_entry --nocapture"#;
    if config.service_type != ServiceType::OWN_PROCESS
        || config.start_type == ServiceStartType::Disabled
        || !config
            .account_name
            .as_ref()
            .is_some_and(|v| v.to_string_lossy().eq_ignore_ascii_case("LocalSystem"))
        || !command_matches
    {
        return Err("total loss SCM fixed contract mismatch".into());
    }
    Ok(())
}

#[cfg(all(windows, test))]
pub fn native_access_failure_fixture(plan: &super::LocalRepairPlan) -> Result<(), String> {
    use windows_service::{
        service::ServiceAccess,
        service_manager::{ServiceManager, ServiceManagerAccess},
    };
    let scm = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)
        .map_err(|e| e.to_string())?;
    let service = scm
        .open_service(
            "RustDeskAgentTestService-TotalLossNative",
            ServiceAccess::QUERY_STATUS,
        )
        .map_err(|e| e.to_string())?;
    validate_native_service(plan, &service)
}

#[cfg(windows)]
pub fn native_observation(plan: &super::LocalRepairPlan) -> Result<AgentObservation, String> {
    use windows_service::service::{ServiceAccess, ServiceState};
    let service = native_service(plan, ServiceAccess::QUERY_STATUS)?;
    let status = service
        .query_status()
        .map_err(|e| format!("total loss SCM status unavailable: {e}"))?;
    if status.current_state != ServiceState::Stopped
        || status.process_id.is_some_and(|pid| pid != 0)
    {
        return Err("total loss SCM requires stopped without PID".into());
    }
    let present = native_process_present()?;
    if present {
        return Err("total loss agent process present".into());
    }
    Ok(AgentObservation {
        stopped: true,
        disabled: false,
        process_present: false,
    })
}

#[cfg(windows)]
fn native_process_present() -> Result<bool, String> {
    use windows::Win32::{
        Foundation::CloseHandle,
        System::Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
            TH32CS_SNAPPROCESS,
        },
    };
    let snapshot =
        unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }.map_err(|e| e.to_string())?;
    let result = (|| {
        let mut entry = PROCESSENTRY32W::default();
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        unsafe { Process32FirstW(snapshot, &mut entry) }.map_err(|e| e.to_string())?;
        loop {
            let n = entry
                .szExeFile
                .iter()
                .position(|&c| c == 0)
                .unwrap_or(entry.szExeFile.len());
            if native_agent_path().file_name().is_some_and(|name| {
                String::from_utf16_lossy(&entry.szExeFile[..n])
                    .eq_ignore_ascii_case(&name.to_string_lossy())
            }) {
                verify_process_image(entry.th32ProcessID)?;
                return Ok(true);
            }
            if let Err(error) = unsafe { Process32NextW(snapshot, &mut entry) } {
                if error.code().0 as u32 != 0x80070012 {
                    return Err(error.to_string());
                }
                break;
            }
        }
        Ok(false)
    })();
    unsafe { CloseHandle(snapshot) }.map_err(|e| e.to_string())?;
    result
}

#[cfg(windows)]
fn native_agent_path() -> std::path::PathBuf {
    #[cfg(not(test))]
    let value = crate::management_agent::system_dirs::get()
        .agent()
        .join("rustdesk-management-agent.exe");
    #[cfg(test)]
    let value = if PRODUCT_FIXTURE.with(|v| v.get()) {
        crate::management_agent::system_dirs::get()
            .agent()
            .join("rustdesk-management-agent.exe")
    } else {
        std::path::PathBuf::from(r"C:\RustDeskAgentLab\total-loss-native\fixture-agent.exe")
    };
    value
}

#[cfg(windows)]
fn verify_process_image(pid: u32) -> Result<(), String> {
    use super::super::handoff::ObservedProcess;
    let observed = ObservedProcess::open(pid)?;
    if !observed.is_alive()? {
        return Err("total loss process exited during observation".into());
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut std::ffi::c_void;
        fn QueryFullProcessImageNameW(
            handle: *mut std::ffi::c_void,
            flags: u32,
            path: *mut u16,
            size: *mut u32,
        ) -> i32;
        fn CloseHandle(handle: *mut std::ffi::c_void) -> i32;
    }
    let handle = unsafe { OpenProcess(0x1000, 0, pid) };
    if handle.is_null() {
        return Err(format!(
            "total loss process image access: {}",
            std::io::Error::last_os_error()
        ));
    }
    let mut path = vec![0u16; 32768];
    let mut size = path.len() as u32;
    let ok = unsafe { QueryFullProcessImageNameW(handle, 0, path.as_mut_ptr(), &mut size) };
    let error = std::io::Error::last_os_error();
    if unsafe { CloseHandle(handle) } == 0 {
        return Err("total loss process image handle cleanup failed".into());
    }
    if ok == 0 {
        return Err(format!("total loss process image query failed: {error}"));
    }
    if !String::from_utf16_lossy(&path[..size as usize])
        .eq_ignore_ascii_case(&native_agent_path().to_string_lossy())
    {
        return Err("total loss process image mismatch".into());
    }
    let current = ObservedProcess::open(pid)?;
    if current.owner() != observed.owner() || !observed.is_alive()? || !current.is_alive()? {
        return Err("total loss process identity changed".into());
    }
    Ok(())
}

#[cfg(windows)]
pub fn native_verify(plan: &super::LocalRepairPlan) -> Result<(), String> {
    native_verify_mode(plan, true)
}

#[cfg(windows)]
pub fn native_verify_running(plan: &super::LocalRepairPlan) -> Result<(), String> {
    native_verify_mode(plan, false)
}

#[cfg(windows)]
fn native_verify_mode(plan: &super::LocalRepairPlan, may_start: bool) -> Result<(), String> {
    use super::super::{package_gate, workspace};
    use std::{
        path::Path,
        time::{Duration, Instant},
    };
    use windows_service::service::{ServiceAccess, ServiceState};
    #[cfg(not(test))]
    let exe = &crate::management_agent::system_dirs::get()
        .agent()
        .join("rustdesk-management-agent.exe");
    #[cfg(test)]
    let exe = &native_agent_path();
    #[cfg(not(test))]
    let identity = Path::new(&plan.identity_path);
    #[cfg(test)]
    let identity = if PRODUCT_FIXTURE.with(|v| v.get()) {
        Path::new(&plan.identity_path)
    } else {
        Path::new(r"C:\RustDeskAgentLab\total-loss-native\identity.toml")
    };
    workspace::verify_restore_target(exe, false)?;
    let _agent = package_gate::lock_verified_file(
        exe,
        &hbb_common::UpdateTarget {
            size: plan.agent_backup.size,
            sha256: plan.agent_backup.sha256.clone(),
            ..Default::default()
        },
    )?;
    workspace::verify_restore_target(identity, true)?;
    let _identity = package_gate::lock_verified_file(
        identity,
        &hbb_common::UpdateTarget {
            size: plan.identity_backup.size,
            sha256: plan.identity_backup.sha256.clone(),
            ..Default::default()
        },
    )?;
    let service = native_service(
        plan,
        if may_start {
            ServiceAccess::START
        } else {
            ServiceAccess::QUERY_STATUS
        },
    )?;
    let status = service.query_status().map_err(|e| e.to_string())?;
    if status.current_state == ServiceState::Running {
        verify_process_image(status.process_id.ok_or("total loss running PID missing")?)?;
        return Ok(());
    }
    if !may_start {
        return Err("total loss preserved agent is no longer running".into());
    }
    if status.current_state != ServiceState::Stopped {
        return Err("total loss agent start requires stopped".into());
    }
    service
        .start::<&str>(&[])
        .map_err(|e| format!("total loss agent start failed: {e}"))?;
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let status = service.query_status().map_err(|e| e.to_string())?;
        if status.current_state == ServiceState::Running
            && status.process_id.is_some_and(|pid| pid != 0)
        {
            verify_process_image(status.process_id.ok_or("total loss running PID missing")?)?;
            validate_native_service(plan, &service)?;
            return Ok(());
        }
        if status.current_state != ServiceState::StartPending || Instant::now() >= deadline {
            return Err("total loss agent running verification failed".into());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[cfg(all(windows, not(test)))]
pub fn preserve_missing_agent_native(
    expected: &Binding,
    installation_error: String,
) -> Result<PreservationReceipt, String> {
    use std::{io::Read, path::Path};
    let directory = crate::management_agent::system_dirs::get()
        .agent()
        .join("transactions")
        .join(&expected.transaction_id);
    if expected.transaction_id.len() != 32
        || !expected
            .transaction_id
            .bytes()
            .all(|b| b.is_ascii_hexdigit())
    {
        return Err("total loss transaction ID invalid".into());
    }
    super::super::workspace::verify_file(&directory.join("local-repair-plan.json"))?;
    let mut bytes = Vec::new();
    std::fs::File::open(directory.join("local-repair-plan.json"))
        .map_err(|e| e.to_string())?
        .take(65537)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    let plan = super::LocalRepairPlan::decode(&bytes)?;
    preserve_missing_agent_native_bound(&directory, &plan, expected, installation_error)
}

#[cfg(windows)]
pub fn preserve_missing_agent_native_bound(
    directory: &std::path::Path,
    plan: &super::LocalRepairPlan,
    expected: &Binding,
    installation_error: String,
) -> Result<PreservationReceipt, String> {
    use std::path::Path;
    #[cfg(not(test))]
    let (state, intent, identity) = (
        crate::management_agent::system_dirs::get()
            .agent()
            .join("state"),
        hbb_common::config::Config2::file(),
        Path::new(&plan.identity_path).to_path_buf(),
    );
    #[cfg(test)]
    let (state, intent, identity) = if PRODUCT_FIXTURE.with(|v| v.get()) {
        (
            crate::management_agent::system_dirs::get()
                .agent()
                .join("state"),
            hbb_common::config::Config2::file(),
            Path::new(&plan.identity_path).to_path_buf(),
        )
    } else {
        (
            directory.join("state"),
            directory.join("intent.toml"),
            Path::new(r"C:\RustDeskAgentLab\total-loss-native\identity.toml").to_path_buf(),
        )
    };
    preserve_native_at(
        directory,
        plan,
        expected,
        installation_error,
        &state,
        &intent,
        &identity,
    )
}

#[cfg(test)]
pub fn preserve_resume_fixture(
    directory: &std::path::Path,
    plan: &super::LocalRepairPlan,
    expected: &Binding,
    error: String,
) -> Result<PreservationReceipt, String> {
    product_fixture(|| {
        preserve_native_at(
            directory,
            plan,
            expected,
            error,
            &directory.join("fixture-state"),
            &directory.join("fixture-intent.toml"),
            std::path::Path::new(&plan.identity_path),
        )
    })
}

#[cfg(windows)]
fn preserve_native_at(
    directory: &std::path::Path,
    plan: &super::LocalRepairPlan,
    expected: &Binding,
    installation_error: String,
    state: &std::path::Path,
    intent: &std::path::Path,
    identity: &std::path::Path,
) -> Result<PreservationReceipt, String> {
    let authorize = || -> Result<(), String> {
        use super::super::handoff::{Handoff, ObservedProcess};
        verify_receipt_binding(directory, expected)?;
        let stored = super::LocalRepairPlan::decode(&receipt_read(
            &directory.join("local-repair-plan.json"),
        )?)?;
        if serde_json::to_value(&stored).map_err(|e| e.to_string())?
            != serde_json::to_value(plan).map_err(|e| e.to_string())?
        {
            return Err("total loss native plan changed".into());
        }
        let handoff = Handoff::decode(&receipt_read(&directory.join("handoff.json"))?)?;
        let ready = serde_json::from_slice(&receipt_read(&directory.join("local-ready.json"))?)
            .map_err(|_| "total loss ready invalid")?;
        let release: super::super::handoff::ReleaseReceipt =
            serde_json::from_slice(&receipt_read(&directory.join("local-release.json"))?)
                .map_err(|_| "total loss release invalid")?;
        let helper = ObservedProcess::open(std::process::id())?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_secs();
        if release.released_at > now {
            return Err("total loss future release refused".into());
        }
        handoff.verify_release(&ready, &release, &handoff.helper, release.released_at)?;
        if helper.owner() != &handoff.helper {
            let resumed: ResumeOwner = serde_json::from_slice(&receipt_read(
                &directory.join("total-loss-resume-owner.json"),
            )?)
            .map_err(|_| "total loss resume owner malformed")?;
            if resumed.schema != 1
                || resumed.binding != *expected
                || resumed.owner != *helper.owner()
                || ObservedProcess::owner_alive(&handoff.helper)?
            {
                return Err("total loss resume owner unauthorized".into());
            }
        }
        if !helper.is_alive()? {
            return Err("total loss helper exited".into());
        }
        Ok(())
    };
    Ok(preserve_agent_checked(
        directory,
        state,
        intent,
        identity,
        &native_agent_path(),
        expected,
        installation_error,
        || {
            authorize()?;
            native_observation(plan)
        },
        || {
            authorize()?;
            native_verify(plan)
        },
        true,
    ))
}

#[cfg(windows)]
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ResumeOwner {
    pub schema: u32,
    pub binding: Binding,
    pub owner: super::super::handoff::Owner,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Binding {
    pub transaction_id: String,
    pub plan_sha256: String,
    pub handoff_sha256: String,
    pub installation_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreservationReceipt {
    schema: u32,
    binding: Binding,
    pub installation_error: String,
    pub agent_error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub persistence_error: Option<String>,
    agent_verified: bool,
    identity_unchanged: bool,
    pub client_recovered: bool,
}

impl PreservationReceipt {
    pub fn new(
        binding: Binding,
        installation_error: String,
        agent: Result<(), String>,
        identity_unchanged: bool,
    ) -> Self {
        Self {
            schema: 1,
            binding,
            installation_error,
            agent_verified: agent.is_ok(),
            agent_error: agent.err(),
            persistence_error: None,
            identity_unchanged,
            client_recovered: false,
        }
    }

    pub fn confirmed(&self) -> bool {
        self.schema == 1
            && self.agent_verified
            && self.agent_error.is_none()
            && self.persistence_error.is_none()
            && self.identity_unchanged
            && !self.client_recovered
            && !self.installation_error.is_empty()
    }

    pub fn verify(&self, expected: &Binding) -> Result<(), String> {
        let token = |s: &str, len| {
            s.len() == len
                && s.bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        };
        if !self.confirmed()
            || &self.binding != expected
            || !token(&expected.transaction_id, 32)
            || !token(&expected.installation_id, 32)
            || !token(&expected.plan_sha256, 64)
            || !token(&expected.handoff_sha256, 64)
        {
            return Err("total loss preservation unconfirmed or binding mismatch".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum OriginalService {
    Absent,
    Stopped { command: String, start: String },
}

#[cfg(test)]
mod preservation_tests {
    use super::*;

    fn binding() -> Binding {
        Binding {
            transaction_id: "a".repeat(32),
            plan_sha256: "b".repeat(64),
            handoff_sha256: "c".repeat(64),
            installation_id: "d".repeat(32),
        }
    }

    #[test]
    fn preservation_failure_keeps_install_and_restore_errors() {
        let receipt = PreservationReceipt::new(
            binding(),
            "installer failed (1603)".into(),
            Err("agent restoration denied".into()),
            true,
        );
        assert_eq!(receipt.installation_error, "installer failed (1603)");
        assert_eq!(
            receipt.agent_error.as_deref(),
            Some("agent restoration denied")
        );
        assert!(!receipt.confirmed());
        assert!(receipt.verify(&binding()).is_err());
    }

    #[test]
    fn preservation_no_client_anchor_never_means_client_rollback() {
        let receipt =
            PreservationReceipt::new(binding(), "target verification failed".into(), Ok(()), true);
        assert!(receipt.confirmed());
        assert!(!receipt.client_recovered);
        receipt.verify(&binding()).unwrap();
        let encoded = serde_json::to_vec(&receipt).unwrap();
        let decoded: PreservationReceipt = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(decoded, receipt);
    }

    #[test]
    fn preservation_foreign_binding_or_changed_identity_never_authorizes_cleanup() {
        let receipt = PreservationReceipt::new(binding(), "installer failed".into(), Ok(()), true);
        for field in 0..4 {
            let mut foreign = binding();
            match field {
                0 => foreign.transaction_id = "e".repeat(32),
                1 => foreign.plan_sha256 = "e".repeat(64),
                2 => foreign.handoff_sha256 = "e".repeat(64),
                _ => foreign.installation_id = "e".repeat(32),
            }
            let before = serde_json::to_vec(&receipt).unwrap();
            assert!(receipt.verify(&foreign).is_err());
            assert_eq!(serde_json::to_vec(&receipt).unwrap(), before);
        }
        let changed = PreservationReceipt::new(binding(), "installer failed".into(), Ok(()), false);
        assert!(!changed.confirmed());
        assert!(changed.verify(&binding()).is_err());
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Prestate {
    pub client_anchor: String,
    pub original_service: OriginalService,
}

impl Prestate {
    pub fn validate(&self, fallback_command: &str) -> Result<(), String> {
        if self.client_anchor != "none" {
            return Err("total loss client anchor must be absent".into());
        }
        if let OriginalService::Stopped { command, start } = &self.original_service {
            let normal_command = format!(
                r#""{}" --service"#,
                crate::management_agent::system_dirs::get()
                    .normal_install()
                    .join("RustDesk.exe")
                    .display()
            );
            if !["auto", "manual"].contains(&start.as_str())
                || ![normal_command.as_str(), fallback_command].contains(&command.as_str())
            {
                return Err("total loss original service contract invalid".into());
            }
        }
        Ok(())
    }
}
