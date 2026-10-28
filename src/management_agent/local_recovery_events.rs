use super::{
    install_execution::{self, Backend, ExecutionResult, InstallOutcome},
    local_events::{
        Action, Binding, ErrorCode, EventError, LocalEvents, Operation, Outcome, PathKey, Phase,
        Record, RecoveryCode,
    },
    transaction::{Evidence, Phase as TxPhase, Transaction},
};
use std::path::{Path, PathBuf};

#[derive(Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EventContext {
    pub path: PathBuf,
    pub binding: Binding,
}

#[cfg(test)]
std::thread_local! {
    static FIXTURE: std::cell::RefCell<Option<EventContext>> = const { std::cell::RefCell::new(None) };
    pub(super) static REPORT_KEY: std::cell::RefCell<Option<Vec<u8>>> = const { std::cell::RefCell::new(None) };
}

pub fn context() -> Result<Option<EventContext>, String> {
    #[cfg(test)]
    if let Some(value) = FIXTURE.with(|slot| slot.borrow().clone()) {
        return Ok(Some(value));
    }
    #[cfg(test)]
    if let Some(path) = std::env::var_os("RUSTDESK_LOCAL_EVENTS_FIXTURE") {
        let path = PathBuf::from(path);
        if !base::lab_guard::is_lab_os("win11")
            || !path.starts_with(r"C:\RustDeskAgentLab")
        {
            return Err("local events fixture outside scheduled lab".into());
        }
        super::workspace::verify_file(&path)?;
        let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
        if bytes.len() > 4096 {
            return Err("local events fixture oversized".into());
        }
        let value: EventContext =
            serde_json::from_slice(&bytes).map_err(|_| "local events fixture malformed")?;
        if value.path.parent() != path.parent() {
            return Err("local events fixture path mismatch".into());
        }
        super::workspace::verify(value.path.parent().ok_or("local event parent absent")?)?;
        return Ok(Some(value));
    }
    let agent_dir = super::system_dirs::get().agent();
    let path = agent_dir.join(r"state\local-events\events.json");
    let path = path.as_path();
    if !path.exists() {
        return Ok(None);
    }
    let log = LocalEvents::open(path).map_err(|e| format!("local events {e:?}"))?;
    let Some(binding) = log.verified_binding().cloned() else {
        return Ok(None);
    };
    let identity = super::identity::load_existing()?;
    let state: serde_json::Value = serde_json::from_slice(
        &std::fs::read(agent_dir.join(r"state\agent.json"))
            .map_err(|_| "local events state unavailable")?,
    )
    .map_err(|_| "local events state malformed")?;
    if binding.client_id != identity.id
        || binding.client_uuid != identity.uuid
        || state["agent_installation_id"] != binding.agent_installation_id
    {
        return Err("local events installation mismatch".into());
    }
    Ok(Some(EventContext {
        path: path.into(),
        binding,
    }))
}
pub fn now() -> Result<u64, String> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|v| v.as_secs())
        .map_err(|_| "local event clock invalid".into())
}

fn event_error(error: super::local_events::EventError) -> String {
    format!("local events {error:?}")
}
fn same_installation(left: &Binding, right: &Binding) -> bool {
    left.client_id == right.client_id
        && left.client_uuid == right.client_uuid
        && left.agent_installation_id == right.agent_installation_id
}
fn terminal(
    log: &mut LocalEvents,
    operation: &Operation,
    binding: &Binding,
    outcome: Outcome,
    now: u64,
) -> Result<(), EventError> {
    log.record_bound_at(
        operation,
        Record {
            phase: Phase::Terminal,
            attempt_seq: 1,
            outcome,
            path_key: if operation.code == RecoveryCode::R3 {
                PathKey::RescueClient
            } else {
                PathKey::NormalClient
            },
            error_code: if outcome == Outcome::ReconciliationRequired {
                Some(ErrorCode::Redacted)
            } else if outcome == Outcome::Succeeded {
                None
            } else {
                Some(ErrorCode::VerificationFailed)
            },
        },
        binding,
        now,
    )?;
    Ok(())
}
pub fn fallback_owned(
    context: Option<&EventContext>,
    now: u64,
    action: impl FnOnce() -> Result<bool, String>,
) -> Result<bool, String> {
    let Some(context) = context else {
        return action();
    };
    let mut log = LocalEvents::open(&context.path).map_err(event_error)?;
    if log.active_fallback().is_some() {
        return Err("local R3 requires reconciliation; action not repeated".into());
    }
    log.prune_confirmed(now).map_err(event_error)?;
    let operation = log
        .begin_fallback(&context.binding, now)
        .map_err(event_error)?;
    let result = action();
    // An error after staging/configuration cannot prove that no mutation occurred.
    if matches!(result, Ok(true)) {
        terminal(
            &mut log,
            &operation,
            &context.binding,
            Outcome::Succeeded,
            now,
        )
        .map_err(event_error)?;
    }
    result
}

pub fn reconcile_fallback(
    context: Option<&EventContext>,
    now: u64,
    verified: impl FnOnce() -> Result<bool, String>,
) -> Result<(), String> {
    let Some(context) = context else {
        return Ok(());
    };
    let mut log = LocalEvents::open(&context.path).map_err(event_error)?;
    let Some((operation, binding)) = log.active_fallback() else {
        return Ok(());
    };
    if !same_installation(&binding, &context.binding) {
        return Err("local R3 original installation mismatch".into());
    }
    if !verified()? {
        return Err("local R3 requires reconciliation".into());
    }
    terminal(&mut log, &operation, &binding, Outcome::Succeeded, now).map_err(event_error)
}

pub fn repair_dispatch(
    path: &Path,
    tx: &mut Transaction,
    proof: &Evidence,
    backend: &mut impl Backend,
    now: u64,
    resume: bool,
    outcome: Option<InstallOutcome>,
) -> Result<ExecutionResult, String> {
    let context = context()?;
    let mut event = None;
    if let Some(context) = &context {
        let mut log = LocalEvents::open(&context.path).map_err(event_error)?;
        if let Some((operation, binding, record)) = log.operation_record(&tx.id) {
            if operation.code != RecoveryCode::R4 || !same_installation(&binding, &context.binding)
            {
                return Err("local R4 event identity mismatch".into());
            }
            if record.phase == Phase::Terminal {
                return Err("local R4 terminal event is not execution authorization".into());
            }
            if !resume {
                return Err("local R4 Started requires transaction resume authorization".into());
            }
            event = Some((log, operation, binding));
        } else if !resume {
            if !tx.may_stop_services(path, proof) {
                return Err("installation execution stop authorization unavailable".into());
            }
            let operation = Operation {
                operation_id: tx.id.clone(),
                code: RecoveryCode::R4,
                action: Action::RepairInstallation,
            };
            log.record_bound_at(
                &operation,
                Record {
                    phase: Phase::Started,
                    attempt_seq: 1,
                    outcome: Outcome::Running,
                    path_key: PathKey::NormalClient,
                    error_code: None,
                },
                &context.binding,
                now,
            )
            .map_err(event_error)?;
            event = Some((log, operation, context.binding.clone()));
        }
    }
    let result = if resume {
        install_execution::resume(path, tx, proof, backend, outcome, now)
    } else {
        install_execution::execute(path, tx, proof, backend, now)
    };
    if let Some((mut log, operation, binding)) = event {
        if let Some(outcome) = repair_outcome(tx.phase) {
            if let Err(error) = terminal(&mut log, &operation, &binding, outcome, now) {
                warn_repair_terminal(&event_error(error));
            }
        }
    }
    result
}

pub(super) fn warn_repair_terminal(error: &str) {
    hbb_common::throttled_log!(
        std::time::Duration::from_secs(60),
        warn,
        "local R4 terminal reporting incomplete; execution result retained: {error}"
    );
}

pub(super) fn begin_total_loss(
    directory: &Path,
    plan: &super::local_repair_plan::LocalRepairPlan,
    tx: &Transaction,
) -> Result<(), String> {
    plan.bind(tx)?;
    if plan.total_loss.is_none()
        || tx.phase != TxPhase::StoppingServices
        || Transaction::decode(&report_read(&directory.join("transaction.json"), 65536)?)? != *tx
    {
        return Err("total loss event execution binding mismatch".into());
    }
    let Some(context) = context()? else {
        return Ok(());
    };
    if context.binding.agent_installation_id != plan.installation_id {
        return Err("total loss event installation mismatch".into());
    }
    if directory.join("local-event-unreported.json").exists() {
        return Err(
            "total loss event already started; unreported execution requires reconciliation".into(),
        );
    }
    let mut log = LocalEvents::open(&context.path).map_err(event_error)?;
    if log.operation_seen(&tx.id) {
        return Err("total loss event already started; reconciliation required".into());
    }
    let recorded = log.record_bound_at(
        &Operation {
            operation_id: tx.id.clone(),
            code: RecoveryCode::R4,
            action: Action::RepairInstallation,
        },
        Record {
            phase: Phase::Started,
            attempt_seq: 1,
            outcome: Outcome::Running,
            path_key: PathKey::NormalClient,
            error_code: None,
        },
        &context.binding,
        now()?,
    );
    match recorded {
        Ok(_) => {}
        Err(
            error @ (EventError::WriterBusy
            | EventError::Full
            | EventError::Storage
            | EventError::DiskFull
            | EventError::StaleWriter),
        ) => {
            use sha2::{Digest, Sha256};
            let receipt = Unreported {
                schema: 1,
                binding: context.binding,
                transaction_id: tx.id.clone(),
                plan_sha256: hex::encode(Sha256::digest(report_read(
                    &directory.join("local-repair-plan.json"),
                    65536,
                )?)),
                handoff_sha256: hex::encode(Sha256::digest(report_read(
                    &directory.join("handoff.json"),
                    4096,
                )?)),
            };
            let path = directory.join("local-event-unreported.json");
            if path.exists() {
                return Err(
                    "total loss unreported receipt occupied; reconcile existing execution".into(),
                );
            }
            super::state_store::publish(
                &path,
                &serde_json::to_vec(&receipt).map_err(|e| e.to_string())?,
            )
            .map_err(|e| format!("total loss audit unavailable: {e}"))?;
            hbb_common::throttled_log!(
                std::time::Duration::from_secs(60),
                warn,
                "total loss event degraded {error:?}; bound unreported receipt persisted"
            );
        }
        Err(error) => return Err(event_error(error)),
    }
    Ok(())
}

pub(super) fn total_loss_event_step<T>(
    directory: &Path,
    plan: &super::local_repair_plan::LocalRepairPlan,
    tx: &Transaction,
    authorize: impl FnOnce() -> Result<(), String>,
    action: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    authorize()?;
    #[cfg(test)]
    if let Some(root) = std::env::var_os("RUSTDESK_EVENT_LOCK_BOUNDARY") {
        let root = std::path::PathBuf::from(root);
        if !base::lab_guard::is_lab_os("win11")
            || root != Path::new(r"C:\RustDeskAgentLab\total-loss-worker-events")
        {
            return Err("event lock fixture outside fixed lab".into());
        }
        std::fs::write(
            root.join("at-started"),
            directory.to_string_lossy().as_bytes(),
        )
        .map_err(|e| e.to_string())?;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while !root.join("lock-held").exists() {
            if std::time::Instant::now() >= deadline {
                return Err("event lock fixture handshake timeout".into());
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
    begin_total_loss(directory, plan, tx)?;
    action()
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Unreported {
    schema: u32,
    binding: Binding,
    transaction_id: String,
    plan_sha256: String,
    handoff_sha256: String,
}

fn reconcile_unreported(
    log: &mut LocalEvents,
    identity: &super::identity::DeviceIdentity,
    current: &Binding,
    agent: &Path,
) -> Result<(), String> {
    use sha2::{Digest, Sha256};
    let root = agent.join("transactions");
    let entries = match std::fs::read_dir(&root) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e.to_string()),
    };
    super::workspace::verify(&root)?;
    let mut admitted = 0;
    for (index, entry) in entries.enumerate() {
        if index >= 64 {
            return Err("total loss unreported scan capacity exceeded".into());
        }
        if admitted >= 8 {
            break;
        }
        let directory = entry.map_err(|e| e.to_string())?.path();
        super::workspace::verify(&directory)?;
        let path = directory.join("local-event-unreported.json");
        match std::fs::symlink_metadata(&path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e.to_string()),
            Ok(_) => {}
        }
        admitted += 1;
        let candidate = (|| -> Result<(), String> {
            let receipt: Unreported = serde_json::from_slice(&report_read(&path, 4096)?)
                .map_err(|_| "total loss unreported receipt malformed")?;
            if receipt.schema != 1
                || directory.file_name().and_then(|n| n.to_str())
                    != Some(receipt.transaction_id.as_str())
                || !same_installation(&receipt.binding, current)
                || receipt.plan_sha256
                    != hex::encode(Sha256::digest(report_read(
                        &directory.join("local-repair-plan.json"),
                        65536,
                    )?))
                || receipt.handoff_sha256
                    != hex::encode(Sha256::digest(report_read(
                        &directory.join("handoff.json"),
                        4096,
                    )?))
            {
                return Err("total loss unreported receipt binding mismatch".into());
            }
            if log.operation_seen(&receipt.transaction_id) {
                std::fs::remove_file(&path).map_err(|e| e.to_string())?;
                return Ok(());
            }
            if verified_repair_terminal(
                &directory,
                &receipt.transaction_id,
                &receipt.binding,
                identity,
            )?
            .is_none()
            {
                return Ok(());
            }
            let operation = Operation {
                operation_id: receipt.transaction_id,
                code: RecoveryCode::R4,
                action: Action::RepairInstallation,
            };
            match log.record_bound_at(
                &operation,
                Record {
                    phase: Phase::Started,
                    attempt_seq: 1,
                    outcome: Outcome::Running,
                    path_key: PathKey::NormalClient,
                    error_code: None,
                },
                &receipt.binding,
                now()?,
            ) {
                Ok(_) => {
                    std::fs::remove_file(&path).map_err(|e| e.to_string())?;
                }
                Err(EventError::Full) => {
                    return Err("unreported receipt waiting for event capacity".into())
                }
                Err(e) => return Err(event_error(e)),
            }
            Ok(())
        })();
        if let Err(error) = candidate {
            warn_repair_terminal(&error);
        }
    }
    Ok(())
}

fn repair_outcome(phase: TxPhase) -> Option<Outcome> {
    match phase {
        TxPhase::Committed => Some(Outcome::Succeeded),
        TxPhase::RolledBack => Some(Outcome::RolledBack),
        TxPhase::RollbackFailed => Some(Outcome::RollbackFailed),
        _ => None,
    }
}

pub(super) fn reconcile_repairs(
    identity: &super::identity::DeviceIdentity,
    current: &Binding,
    state: &Path,
    agent: &Path,
) -> Result<(), String> {
    if current.client_id != identity.id
        || current.client_uuid != identity.uuid
        || current.enrollment_generation == 0
        || current.enrollment_generation > i64::MAX as u64
    {
        return Err("local R4 poll identity mismatch".into());
    }
    #[cfg(not(test))]
    if agent != super::system_dirs::get().agent() {
        return Err("local R4 fixed transaction root required".into());
    }
    let path = state.join("local-events/events.json");
    let mut log = LocalEvents::open(&path).map_err(event_error)?;
    if let Err(error) = reconcile_unreported(&mut log, identity, current, agent) {
        warn_repair_terminal(&error);
    }
    let mut first_error = None;
    for (operation, binding) in log.active_repairs() {
        let result = (|| {
            if !same_installation(&binding, current) {
                return Err("local R4 original installation mismatch".into());
            }
            let directory = agent.join("transactions").join(&operation.operation_id);
            let Some(outcome) =
                verified_repair_terminal(&directory, &operation.operation_id, &binding, identity)?
            else {
                return Ok(());
            };
            terminal(&mut log, &operation, &binding, outcome, now()?).map_err(event_error)
        })();
        if let Err(error) = result {
            warn_repair_terminal(&error);
            if first_error.is_none() {
                first_error = Some(error);
            }
        }
    }
    match first_error {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

fn report_read(path: &Path, limit: u64) -> Result<Vec<u8>, String> {
    use std::io::Read;
    super::workspace::verify_file(path)?;
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .map_err(|e| e.to_string())?
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > limit {
        return Err("local R4 evidence oversized".into());
    }
    Ok(bytes)
}

pub(super) fn verified_repair_terminal(
    directory: &Path,
    id: &str,
    binding: &Binding,
    identity: &super::identity::DeviceIdentity,
) -> Result<Option<Outcome>, String> {
    use sha2::{Digest, Sha256};
    super::workspace::verify(directory)?;
    let tx_bytes = report_read(&directory.join("transaction.json"), 65536)?;
    let tx = Transaction::decode(&tx_bytes)?;
    if tx.id != id {
        return Err("local R4 transaction binding mismatch".into());
    }
    if repair_outcome(tx.phase).is_none() && tx.phase != TxPhase::RollingBack {
        return Ok(None);
    }
    let plan_bytes = report_read(&directory.join("local-repair-plan.json"), 65536)?;
    let plan = super::local_repair_plan::LocalRepairPlan::decode(&plan_bytes)?;
    plan.bind(&tx)?;
    let preservation = plan.total_loss.is_some() && tx.phase == TxPhase::RollingBack;
    if tx.phase == TxPhase::RollingBack && !preservation {
        return Ok(None);
    }
    if plan.installation_id != binding.agent_installation_id {
        return Err("local R4 plan installation mismatch".into());
    }
    let handoff =
        super::handoff::Handoff::decode(&report_read(&directory.join("handoff.json"), 4096)?)?;
    if handoff.transaction_id != id
        || handoff.plan_sha256 != hex::encode(Sha256::digest(&plan_bytes))
    {
        return Err("local R4 handoff binding mismatch".into());
    }
    let ready = serde_json::from_slice(&report_read(&directory.join("local-ready.json"), 4096)?)
        .map_err(|_| "local R4 ready receipt malformed")?;
    let release: super::handoff::ReleaseReceipt =
        serde_json::from_slice(&report_read(&directory.join("local-release.json"), 4096)?)
            .map_err(|_| "local R4 release receipt malformed")?;
    handoff.verify_release(&ready, &release, &handoff.helper, release.released_at)?;
    let result_bytes = report_read(&directory.join("local-result.json"), 65536)?;
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct TerminalResult {
        schema: u32,
        transaction_id: String,
        phase: TxPhase,
        error: Option<String>,
        reboot_required: bool,
    }
    let preservation_bytes = if preservation {
        Some(verify_preservation_result(
            directory,
            &plan,
            &handoff,
            &result_bytes,
        )?)
    } else {
        None
    };
    if !preservation {
        let result: TerminalResult =
            serde_json::from_slice(&result_bytes).map_err(|_| "local R4 result malformed")?;
        if result.schema != 1
            || result.transaction_id != id
            || result.phase != tx.phase
            || result.reboot_required
            || (tx.phase == TxPhase::Committed && result.error.is_some())
            || (tx.phase != TxPhase::Committed && result.error.as_deref().map_or(true, str::is_empty))
        {
            return Err("local R4 result binding mismatch".into());
        }
    }
    let identity_bytes = report_read(&directory.join(&plan.identity_backup.relative_path), 65536)?;
    if identity_bytes.len() as u64 != plan.identity_backup.size
        || hex::encode(Sha256::digest(&identity_bytes)) != tx.identity_fingerprint
    {
        return Err("local R4 identity digest mismatch".into());
    }
    #[derive(serde::Deserialize)]
    struct StoredKey {
        key_pair: (Vec<u8>, Vec<u8>),
    }
    let stored: StoredKey = hbb_common::toml::from_str(
        std::str::from_utf8(&identity_bytes).map_err(|_| "local R4 identity encoding")?,
    )
    .map_err(|_| "local R4 identity malformed")?;
    if stored.key_pair.0 != identity.secret.as_ref()
        || stored.key_pair.1 != identity.public.as_ref()
    {
        return Err("local R4 original device key mismatch".into());
    }
    let manifest_bytes = report_read(&directory.join("approved-manifest.json"), 262144)?;
    let manifest: hbb_common::UpdateManifest =
        serde_json::from_slice(&manifest_bytes).map_err(|_| "local R4 manifest malformed")?;
    #[cfg(test)]
    let fixture_key = REPORT_KEY.with(|slot| slot.borrow().clone());
    #[cfg(not(test))]
    let fixture_key: Option<Vec<u8>> = None;
    let target = match fixture_key.as_deref() {
        #[cfg(test)]
        Some(key) => super::package_gate::verify_with_key(
            &manifest,
            &plan.target_key,
            plan.build_seq,
            "fixture-key",
            key,
        )?,
        _ => super::package_gate::verify(&manifest, &plan.target_key, plan.build_seq)?,
    };
    let (version, build, _) =
        base::update::update_target_release(&manifest, target).map_err(|e| e.to_string())?;
    if version != plan.version
        || build != plan.build_seq
        || target.sha256 != plan.package.sha256
        || target.size != plan.package.size
        || super::package_gate::cache_name(&target.primary, "msi")? != plan.package.relative_path
    {
        return Err("local R4 signed target mismatch".into());
    }
    if plan.total_loss.is_some() {
        let state = directory
            .parent()
            .and_then(Path::parent)
            .ok_or("total loss transaction root unavailable")?
            .join("state");
        plan.verify_approval_source(directory, &state, &identity_bytes, fixture_key.as_deref())?;
    }
    // Keep the package locked while binding the terminal evidence; no installer/SCM calls.
    let _package = super::package_gate::lock_verified_file(
        &directory.join(&plan.package.relative_path),
        target,
    )?;
    if report_read(&directory.join("transaction.json"), 65536)? != tx_bytes
        || report_read(&directory.join("local-repair-plan.json"), 65536)? != plan_bytes
        || report_read(&directory.join("local-result.json"), 65536)? != result_bytes
    {
        return Err("local R4 terminal evidence changed".into());
    }
    if let Some(bytes) = preservation_bytes {
        if report_read(&directory.join("agent-preservation.json"), 65536)? != bytes {
            return Err("total loss preservation evidence changed".into());
        }
    }
    Ok(Some(if preservation {
        Outcome::ReconciliationRequired
    } else {
        repair_outcome(tx.phase).ok_or("local R4 outcome unavailable")?
    }))
}

fn verify_preservation_result(
    directory: &Path,
    plan: &super::local_repair_plan::LocalRepairPlan,
    handoff: &super::handoff::Handoff,
    result_bytes: &[u8],
) -> Result<Vec<u8>, String> {
    use super::local_repair_plan::total_loss::{
        Binding as PreservationBinding, PreservationReceipt,
    };
    use sha2::{Digest, Sha256};
    let expected = PreservationBinding {
        transaction_id: plan.transaction_id.clone(),
        installation_id: plan.installation_id.clone(),
        plan_sha256: handoff.plan_sha256.clone(),
        handoff_sha256: hex::encode(Sha256::digest(report_read(
            &directory.join("handoff.json"),
            4096,
        )?)),
    };
    let bytes = report_read(&directory.join("agent-preservation.json"), 65536)?;
    let receipt: PreservationReceipt =
        serde_json::from_slice(&bytes).map_err(|_| "total loss preservation malformed")?;
    receipt.verify(&expected)?;
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct ResultReceipt {
        schema: u32,
        transaction_id: String,
        phase: String,
        receipt: PreservationReceipt,
    }
    let result: ResultReceipt =
        serde_json::from_slice(result_bytes).map_err(|_| "total loss result malformed")?;
    if result.schema != 1
        || result.transaction_id != plan.transaction_id
        || result.phase != "preservation"
        || result.receipt != receipt
    {
        return Err("total loss result binding mismatch".into());
    }
    let cause: serde_json::Value = serde_json::from_slice(&report_read(
        &directory.join("total-loss-install-error.json"),
        65536,
    )?)
    .map_err(|_| "total loss cause malformed")?;
    if cause["transaction_id"] != plan.transaction_id
        || cause["plan_sha256"] != handoff.plan_sha256
        || cause["error"] != receipt.installation_error
    {
        return Err("total loss cause binding mismatch".into());
    }
    match std::fs::symlink_metadata(directory.join("total-loss-reconciliation.json")) {
        Ok(_) => {
            let reconciliation: serde_json::Value = serde_json::from_slice(&report_read(
                &directory.join("total-loss-reconciliation.json"),
                65536,
            )?)
            .map_err(|_| "total loss reconciliation malformed")?;
            if reconciliation["schema"] != 1
                || reconciliation["transaction_id"] != plan.transaction_id
                || reconciliation["state"] != "agent_preserved_client_unrecovered"
                || reconciliation["receipt"]
                    != serde_json::to_value(&receipt).map_err(|e| e.to_string())?
            {
                return Err("total loss reconciliation binding mismatch".into());
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.to_string()),
    }
    Ok(bytes)
}

pub fn report_repair_terminal(
    tx: &Transaction,
    installation: &str,
    now: u64,
) -> Result<(), String> {
    report_repair_terminal_inner(tx, installation, now, false)
}

pub(super) fn report_repair_terminal_for_cleanup(
    tx: &Transaction,
    installation: &str,
    now: u64,
) -> Result<(), String> {
    report_repair_terminal_inner(tx, installation, now, true)
}

fn report_repair_terminal_inner(
    tx: &Transaction,
    installation: &str,
    now: u64,
    cleanup: bool,
) -> Result<(), String> {
    let Some(outcome) = repair_outcome(tx.phase) else {
        return Ok(());
    };
    let Some(context) = context()? else {
        return Ok(());
    };
    let mut log = LocalEvents::open(&context.path).map_err(event_error)?;
    let Some((operation, binding, _)) = log.operation_record(&tx.id) else {
        return Ok(());
    };
    if operation.code != RecoveryCode::R4
        || binding.agent_installation_id != installation
        || !same_installation(&binding, &context.binding)
    {
        return Err("local R4 terminal event identity mismatch".into());
    }
    match terminal(&mut log, &operation, &binding, outcome, now) {
        Err(
            error @ (EventError::WriterBusy
            | EventError::StaleWriter
            | EventError::DiskFull
            | EventError::Storage
            | EventError::Full),
        ) if cleanup => {
            warn_repair_terminal(&event_error(error));
            Ok(())
        }
        result => result.map_err(event_error),
    }
}

#[cfg(test)]
pub fn with_context_fixture<T>(context: EventContext, run: impl FnOnce() -> T) -> T {
    struct Restore(Option<EventContext>);
    impl Drop for Restore {
        fn drop(&mut self) {
            FIXTURE.with(|slot| *slot.borrow_mut() = self.0.take());
        }
    }
    let _restore = Restore(FIXTURE.with(|slot| slot.replace(Some(context))));
    run()
}

#[cfg(test)]
mod tests {
    use super::super::transaction::Phase;
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    static SEQ: AtomicUsize = AtomicUsize::new(0);
    struct Fixture(EventContext);
    impl Fixture {
        fn new() -> Self {
            base::lab_guard::require_lab_os("win10");
            assert_eq!(std::env::var("LOCAL_EVENTS_R34_SAFE").unwrap(), "1");
            let root = PathBuf::from(std::env::var_os("RUSTDESK_LOCAL_EVENTS_ROOT").unwrap()).join(
                format!(
                    "r34-{}-{}",
                    std::process::id(),
                    SEQ.fetch_add(1, Ordering::SeqCst)
                ),
            );
            super::super::workspace::create(&root).unwrap();
            let ctx = EventContext {
                path: root.join("events.json"),
                binding: Binding {
                    client_id: "test-device".into(),
                    client_uuid: "test-uuid".into(),
                    agent_installation_id: "a".repeat(32),
                    enrollment_generation: 1,
                },
            };
            LocalEvents::open(&ctx.path)
                .unwrap()
                .remember_binding(ctx.binding.clone())
                .unwrap();
            FIXTURE.with(|slot| *slot.borrow_mut() = Some(ctx.clone()));
            Self(ctx)
        }
        fn log(&self) -> LocalEvents {
            LocalEvents::open(&self.0.path).unwrap()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            FIXTURE.with(|slot| *slot.borrow_mut() = None);
            std::fs::remove_dir_all(self.0.path.parent().unwrap()).unwrap();
        }
    }
    fn proof() -> Evidence {
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
    fn tx(f: &Fixture) -> (PathBuf, Transaction) {
        let path = f.0.path.with_file_name("transaction.json");
        let mut tx = Transaction::new(
            "b".repeat(32),
            "windows-x86_64-msi-sos".into(),
            "c".repeat(64),
            "d".repeat(64),
            100,
        )
        .unwrap();
        tx.save(&path).unwrap();
        for phase in [
            Phase::PackageVerified,
            Phase::Prepared,
            Phase::StoppingServices,
        ] {
            tx.advance(phase, &proof(), 101).unwrap();
            tx.save(&path).unwrap();
        }
        (path, tx)
    }
    struct Files {
        context: EventContext,
        calls: usize,
        fail: bool,
    }
    impl Backend for Files {
        fn stop_services(&mut self) -> Result<(), String> {
            self.calls += 1;
            let log = LocalEvents::open(&self.context.path).unwrap();
            let last = log.pending_events().last().unwrap();
            assert_eq!(
                last.payload["status"], "running",
                "Started must precede backend"
            );
            assert_eq!(last.payload["result"]["action"], "repair_installation");
            std::fs::write(
                self.context.path.with_file_name("stopped.fixture"),
                b"fixture only",
            )
            .unwrap();
            Ok(())
        }
        fn apply_package(&mut self) -> Result<InstallOutcome, String> {
            self.calls += 1;
            Ok(if self.fail {
                InstallOutcome::Failed(1603)
            } else {
                InstallOutcome::Installed
            })
        }
        fn validate_target(&mut self) -> Result<install_execution::Validation, String> {
            self.calls += 1;
            Ok(install_execution::Validation {
                target_matches: true,
                identity_unchanged: true,
                client_ready: true,
                agent_ready: true,
            })
        }
        fn rollback_original(&mut self) -> Result<install_execution::Validation, String> {
            self.validate_target()
        }
    }
    #[test]
    fn r34_fallback_owned_started_terminal_and_upload_retry_never_repeat_action() {
        let f = Fixture::new();
        let mut calls = 0;
        assert_eq!(
            fallback_owned(Some(&f.0), 100, || {
                calls += 1;
                let log = f.log();
                assert_eq!(log.pending_events()[0].payload["status"], "running");
                assert_eq!(log.pending_events()[0].payload["result"]["code"], "r3");
                Ok(true)
            }),
            Ok(true)
        );
        let log = f.log();
        assert_eq!(log.pending_events().len(), 2);
        assert_eq!(log.pending_events()[1].payload["status"], "succeeded");
        let before = std::fs::read(&f.0.path).unwrap();
        assert_eq!(f.log().pending_events(), log.pending_events());
        assert_eq!(std::fs::read(&f.0.path).unwrap(), before);
        assert_eq!(calls, 1);
    }
    #[test]
    fn r34_fallback_unknown_started_retains_original_generation_and_refuses_repeat() {
        let f = Fixture::new();
        assert!(fallback_owned(Some(&f.0), 100, || Err("SCM uncertain secret".into())).is_err());
        let before = std::fs::read(&f.0.path).unwrap();
        let mut newer = f.0.clone();
        newer.binding.enrollment_generation = 2;
        let result = fallback_owned(Some(&newer), 101, || {
            panic!("interrupted R3 cannot reexecute")
        });
        assert!(result.unwrap_err().contains("reconciliation"));
        assert_eq!(std::fs::read(&f.0.path).unwrap(), before);
        assert!(!String::from_utf8(before).unwrap().contains("secret"));
    }
    #[test]
    fn r34_started_lock_failure_prevents_action_and_keeps_original_store() {
        use std::os::windows::fs::OpenOptionsExt;
        let f = Fixture::new();
        let before = std::fs::read(&f.0.path).unwrap();
        let _lock = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .share_mode(0)
            .open(f.0.path.with_extension("lock"))
            .unwrap();
        assert!(fallback_owned(Some(&f.0), 100, || panic!("no Started no mutation")).is_err());
        assert_eq!(std::fs::read(&f.0.path).unwrap(), before);
    }
    #[test]
    fn r34_repair_dispatch_uses_real_transaction_authorization_and_terminal_replay() {
        let f = Fixture::new();
        let (path, mut tx) = tx(&f);
        let mut backend = Files {
            context: f.0.clone(),
            calls: 0,
            fail: false,
        };
        let mut denied = proof();
        denied.locally_blocked = true;
        assert!(super::super::local_repair::event_dispatch_fixture(
            &path,
            &mut tx,
            &denied,
            &mut backend,
            102,
            false,
            None
        )
        .is_err());
        assert_eq!(backend.calls, 0);
        let result = super::super::local_repair::event_dispatch_fixture(
            &path,
            &mut tx,
            &proof(),
            &mut backend,
            103,
            false,
            None,
        )
        .unwrap();
        assert_eq!(result.phase, Phase::Committed);
        let before = std::fs::read(&path).unwrap();
        let events = f.log().pending_events().to_vec();
        assert_eq!(events.len(), 2);
        assert_eq!(events[1].payload["status"], "succeeded");
        assert_eq!(events[0].payload["result"]["operation_id"], tx.id);
        let calls = backend.calls;
        report_repair_terminal(&tx, &f.0.binding.agent_installation_id, 104).unwrap();
        assert_eq!(f.log().pending_events(), events);
        assert_eq!(std::fs::read(&path).unwrap(), before);
        assert_eq!(backend.calls, calls);
    }
    #[test]
    fn r34_repair_rollback_reports_real_outcome_not_success_and_keeps_identity() {
        let f = Fixture::new();
        let (path, mut tx) = tx(&f);
        let mut backend = Files {
            context: f.0.clone(),
            calls: 0,
            fail: true,
        };
        let result = super::super::local_repair::event_dispatch_fixture(
            &path,
            &mut tx,
            &proof(),
            &mut backend,
            103,
            false,
            None,
        )
        .unwrap();
        assert_eq!(result.phase, Phase::RolledBack);
        let events = f.log().pending_events().to_vec();
        assert_eq!(events.len(), 2);
        assert_eq!(events[1].payload["status"], "rolled_back");
        assert_eq!(events[1].payload["enrollment_generation"], 1);
        assert_eq!(
            events[1].payload["agent_installation_id"],
            f.0.binding.agent_installation_id
        );
    }
    #[test]
    fn r34_terminal_write_failure_retains_started_and_reconcile_never_reexecutes() {
        use std::os::windows::fs::OpenOptionsExt;
        let f = Fixture::new();
        let mut lock = None;
        let result = fallback_owned(Some(&f.0), 100, || {
            lock = Some(
                std::fs::OpenOptions::new()
                    .write(true)
                    .create(true)
                    .truncate(false)
                    .share_mode(0)
                    .open(f.0.path.with_extension("lock"))
                    .unwrap(),
            );
            Ok(true)
        });
        assert!(result.unwrap_err().contains("WriterBusy"));
        let before = f.log().pending_events().to_vec();
        assert_eq!(before.len(), 1);
        drop(lock);
        let mut newer = f.0.clone();
        newer.binding.enrollment_generation = 2;
        reconcile_fallback(Some(&newer), 101, || Ok(true)).unwrap();
        let log = f.log();
        assert_eq!(log.pending_events().len(), 2);
        assert_eq!(log.pending_events()[0], before[0]);
        assert_eq!(log.pending_events()[1].payload["enrollment_generation"], 1);
        let bytes = std::fs::read(&f.0.path).unwrap();
        reconcile_fallback(Some(&newer), 102, || panic!("already terminal")).unwrap();
        assert_eq!(std::fs::read(&f.0.path).unwrap(), bytes);
    }
    #[test]
    fn r34_repair_applying_resume_never_reapplies_and_retains_original_binding() {
        let f = Fixture::new();
        let (path, mut tx) = tx(&f);
        struct Pending(Files);
        impl Backend for Pending {
            fn stop_services(&mut self) -> Result<(), String> {
                self.0.stop_services()
            }
            fn apply_package(&mut self) -> Result<InstallOutcome, String> {
                self.0.calls += 1;
                Ok(InstallOutcome::Pending {
                    pid: 123,
                    created_at: 456,
                })
            }
            fn validate_target(&mut self) -> Result<install_execution::Validation, String> {
                self.0.validate_target()
            }
            fn rollback_original(&mut self) -> Result<install_execution::Validation, String> {
                self.0.rollback_original()
            }
        }
        let mut backend = Pending(Files {
            context: f.0.clone(),
            calls: 0,
            fail: false,
        });
        let pending = super::super::local_repair::event_dispatch_fixture(
            &path,
            &mut tx,
            &proof(),
            &mut backend,
            103,
            false,
            None,
        )
        .unwrap();
        assert_eq!(pending.phase, Phase::Applying);
        let mut newer = f.0.clone();
        newer.binding.enrollment_generation = 2;
        struct Resume;
        impl Backend for Resume {
            fn stop_services(&mut self) -> Result<(), String> {
                panic!("no second stop")
            }
            fn apply_package(&mut self) -> Result<InstallOutcome, String> {
                panic!("no second install")
            }
            fn validate_target(&mut self) -> Result<install_execution::Validation, String> {
                Ok(install_execution::Validation {
                    target_matches: true,
                    identity_unchanged: true,
                    client_ready: true,
                    agent_ready: true,
                })
            }
            fn rollback_original(&mut self) -> Result<install_execution::Validation, String> {
                panic!("no unexpected rollback")
            }
        }
        with_context_fixture(newer, || {
            let before = std::fs::read(&path).unwrap();
            let events = std::fs::read(&f.0.path).unwrap();
            assert!(super::super::local_repair::event_dispatch_fixture(
                &path,
                &mut tx,
                &proof(),
                &mut Resume,
                104,
                true,
                None
            )
            .unwrap_err()
            .contains("unknown"));
            assert_eq!(std::fs::read(&path).unwrap(), before);
            assert_eq!(std::fs::read(&f.0.path).unwrap(), events);
            let result = super::super::local_repair::event_dispatch_fixture(
                &path,
                &mut tx,
                &proof(),
                &mut Resume,
                104,
                true,
                Some(InstallOutcome::Installed),
            )
            .unwrap();
            assert_eq!(result.phase, Phase::Committed);
        });
        let events = f.log().pending_events().to_vec();
        assert_eq!(events.len(), 2);
        assert_eq!(events[1].payload["enrollment_generation"], 1);
        assert_eq!(
            events[0].payload["result"]["operation_id"],
            events[1].payload["result"]["operation_id"]
        );
    }
    #[test]
    fn r34_repair_terminal_publish_failure_preserves_committed_transaction_for_report_only() {
        use std::os::windows::fs::OpenOptionsExt;
        let f = Fixture::new();
        let (path, mut tx) = tx(&f);
        struct Locked {
            files: Files,
            lock: Option<std::fs::File>,
        }
        impl Backend for Locked {
            fn stop_services(&mut self) -> Result<(), String> {
                self.files.stop_services()
            }
            fn apply_package(&mut self) -> Result<InstallOutcome, String> {
                self.files.apply_package()
            }
            fn validate_target(&mut self) -> Result<install_execution::Validation, String> {
                let result = self.files.validate_target();
                self.lock = Some(
                    std::fs::OpenOptions::new()
                        .write(true)
                        .create(true)
                        .truncate(false)
                        .share_mode(0)
                        .open(self.files.context.path.with_extension("lock"))
                        .unwrap(),
                );
                result
            }
            fn rollback_original(&mut self) -> Result<install_execution::Validation, String> {
                panic!("do not rollback for event failure")
            }
        }
        let mut backend = Locked {
            files: Files {
                context: f.0.clone(),
                calls: 0,
                fail: false,
            },
            lock: None,
        };
        let result = super::super::local_repair::event_dispatch_fixture(
            &path,
            &mut tx,
            &proof(),
            &mut backend,
            103,
            false,
            None,
        )
        .unwrap();
        assert_eq!(result.phase, Phase::Committed);
        assert_eq!(result.error, None);
        assert_eq!(tx.phase, Phase::Committed);
        let before = std::fs::read(&path).unwrap();
        assert_eq!(f.log().pending_events().len(), 1);
        drop(backend.lock.take());
        report_repair_terminal(&tx, &f.0.binding.agent_installation_id, 104).unwrap();
        assert_eq!(f.log().pending_events()[1].payload["status"], "succeeded");
        assert_eq!(std::fs::read(&path).unwrap(), before);
        let events = std::fs::read(&f.0.path).unwrap();
        report_repair_terminal(&tx, &f.0.binding.agent_installation_id, 105).unwrap();
        assert_eq!(std::fs::read(&f.0.path).unwrap(), events);
    }
    #[cfg(local_recovery_events_http_test)]
    #[test]
    fn r34_signed_original_api_receives_action_events_lost_ack_and_old_generation() {
        use hbb_common::{
            base64::{engine::general_purpose::STANDARD, Engine as _},
            sodiumoxide::crypto::sign,
        };
        use std::{
            process::{Command, Stdio},
            time::{Duration, Instant},
        };
        let f = Fixture::new();
        hbb_common::sodiumoxide::init().unwrap();
        let api = PathBuf::from(std::env::var_os("LOCAL_EVENTS_API_FIXTURE").unwrap());
        let php = PathBuf::from(std::env::var_os("LOCAL_EVENTS_PHP").unwrap());
        let ini = PathBuf::from(std::env::var_os("LOCAL_EVENTS_PHP_INI").unwrap());
        let db = f.0.path.with_file_name("agent-status-test-r34.db");
        let (public, secret) = sign::gen_keypair();
        let identity = super::super::identity::DeviceIdentity {
            id: "test-device".into(),
            uuid: "test-uuid".into(),
            public,
            secret,
        };
        let command = || {
            let mut c = Command::new(&php);
            c.arg("-c")
                .arg(&ini)
                .env("AGENT_FIXTURE_ROOT", &api)
                .env("AGENT_HTTP_FIXTURE_DB", &db)
                .env("RUSTDESK_DB_DRIVER", "sqlite")
                .env("RUSTDESK_ADMIN_PATH", "/agent-test")
                .env(
                    "RUSTDESK_INSTALL_CONFIG",
                    f.0.path.with_file_name("absent.json"),
                );
            c
        };
        let seed = r#"require getenv('AGENT_FIXTURE_ROOT').'/sqlite/lib.php';require getenv('AGENT_FIXTURE_ROOT').'/sqlite/agent_operations.php';$db=new PDO('sqlite:'.getenv('AGENT_HTTP_FIXTURE_DB'));configure_pdo($db);ensure_schema($db);$now=time();db_insert_ignore($db,'device_reports',['id'=>'test-device','uuid'=>'test-uuid','payload'=>'{}','last_seen'=>$now,'last_heartbeat'=>$now,'heartbeat_payload'=>'{}']);db_insert_ignore($db,'device_update_keys',['device_id'=>'test-device','uuid'=>'test-uuid','public_key'=>getenv('PK'),'first_seen_at'=>$now,'last_seen_at'=>$now]);agent_binding_authorize($db,['id'=>1,'address_book_scope'=>'all'],'test-device','test-uuid',str_repeat('a',32),true,$now);"#;
        let seeded = command()
            .args(["-r", seed])
            .env("PK", STANDARD.encode(identity.public.as_ref()))
            .output()
            .unwrap();
        assert!(
            seeded.status.success(),
            "{}",
            String::from_utf8_lossy(&seeded.stderr)
        );
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);
        struct Server(std::process::Child);
        impl Drop for Server {
            fn drop(&mut self) {
                self.0.kill().unwrap();
                self.0.wait().unwrap();
            }
        }
        let _server = Server(
            command()
                .args(["-S", &address.to_string()])
                .arg(api.join("tests/agent_status_http_router.php"))
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
        );
        let deadline = Instant::now() + Duration::from_secs(15);
        while std::net::TcpStream::connect(address).is_err() {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(25));
        }
        let url = format!("http://{address}");
        fallback_owned(Some(&f.0), 100, || Ok(true)).unwrap();
        let (path, mut tx) = tx(&f);
        let mut backend = Files {
            context: f.0.clone(),
            calls: 0,
            fail: false,
        };
        super::super::local_repair::event_dispatch_fixture(
            &path,
            &mut tx,
            &proof(),
            &mut backend,
            103,
            false,
            None,
        )
        .unwrap();
        let mut log = f.log();
        let bytes = std::fs::read(&f.0.path).unwrap();
        let mut sent = None;
        assert!(
            super::super::local_event_runtime::upload(&mut log, &f.0.binding, |body| {
                let reply = super::super::local_event_runtime::send_signed(&identity, &url, body)?;
                assert_eq!(reply["duplicate"], false);
                sent = Some(body.clone());
                Err("lost ACK fixture".into())
            })
            .is_err()
        );
        assert_eq!(std::fs::read(&f.0.path).unwrap(), bytes);
        let mut duplicates = 0;
        super::super::local_event_runtime::upload(&mut log, &f.0.binding, |body| {
            let reply = super::super::local_event_runtime::send_signed(&identity, &url, body)?;
            if reply["duplicate"] == true {
                duplicates += 1;
                assert_eq!(Some(body), sent.as_ref());
            }
            Ok(reply)
        })
        .unwrap();
        assert_eq!(duplicates, 1);
        assert!(f.log().pending_events().is_empty());
        fallback_owned(Some(&f.0), 104, || Err("unknown".into())).unwrap_err();
        let mut log = f.log();
        let before = std::fs::read(&f.0.path).unwrap();
        assert!(command().args(["-r",r#"$db=new PDO('sqlite:'.getenv('AGENT_HTTP_FIXTURE_DB'));$db->exec('UPDATE device_agent_bindings SET generation=2');"#]).status().unwrap().success());
        assert_eq!(
            super::super::local_event_runtime::send_signed(
                &identity,
                &url,
                &log.pending_events()[0].payload
            )
            .unwrap_err(),
            "local event HTTP 403"
        );
        let mut newer = f.0.binding.clone();
        newer.enrollment_generation = 2;
        assert!(
            super::super::local_event_runtime::upload(&mut log, &newer, |_| panic!("no rebind"))
                .is_err()
        );
        assert_eq!(std::fs::read(&f.0.path).unwrap(), before);
        let rows=command().args(["-r",r#"$db=new PDO('sqlite:'.getenv('AGENT_HTTP_FIXTURE_DB'));echo json_encode($db->query('SELECT command_id,generation,payload,digest FROM device_agent_events')->fetchAll(PDO::FETCH_ASSOC));"#]).output().unwrap();
        assert!(rows.status.success());
        let rows: serde_json::Value = serde_json::from_slice(&rows.stdout).unwrap();
        assert_eq!(rows.as_array().unwrap().len(), 4);
        let mut codes = std::collections::BTreeMap::new();
        for row in rows.as_array().unwrap() {
            use sha2::{Digest, Sha256};
            assert!(row["command_id"].is_null());
            assert_eq!(row["generation"], 1);
            let payload = row["payload"].as_str().unwrap();
            assert_eq!(
                row["digest"],
                hex::encode(Sha256::digest(payload.as_bytes()))
            );
            let payload: serde_json::Value = serde_json::from_str(payload).unwrap();
            *codes
                .entry(payload["result"]["code"].as_str().unwrap().to_owned())
                .or_insert(0) += 1;
        }
        assert_eq!(codes.get("r3"), Some(&2));
        assert_eq!(codes.get("r4"), Some(&2));
    }

    #[test]
    fn r34_repair_failed_rollback_keeps_precise_terminal_semantics() {
        let f = Fixture::new();
        let (path, mut tx) = tx(&f);
        struct FailedRollback(Files);
        impl Backend for FailedRollback {
            fn stop_services(&mut self) -> Result<(), String> {
                self.0.stop_services()
            }
            fn apply_package(&mut self) -> Result<InstallOutcome, String> {
                Ok(InstallOutcome::Failed(1603))
            }
            fn validate_target(&mut self) -> Result<install_execution::Validation, String> {
                panic!("failed installer")
            }
            fn rollback_original(&mut self) -> Result<install_execution::Validation, String> {
                Err("fixture rollback failed".into())
            }
        }
        let mut backend = FailedRollback(Files {
            context: f.0.clone(),
            calls: 0,
            fail: true,
        });
        let result = super::super::local_repair::event_dispatch_fixture(
            &path,
            &mut tx,
            &proof(),
            &mut backend,
            103,
            false,
            None,
        )
        .unwrap();
        assert_eq!(result.phase, Phase::RollbackFailed);
        assert!(result.error.unwrap().contains("rollback failed"));
        assert_eq!(
            f.log().pending_events()[1].payload["status"],
            "rollback_failed"
        );
    }
}
