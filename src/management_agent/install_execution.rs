use super::transaction::{Evidence, Phase, Transaction};
use std::path::Path;

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "status", content = "detail", rename_all = "snake_case")]
pub enum InstallOutcome {
    Installed,
    RebootRequired,
    Busy,
    Pending { pid: u32, created_at: u64 },
    Failed(i32),
}

#[derive(Clone, Debug)]
pub struct Validation {
    pub target_matches: bool,
    pub identity_unchanged: bool,
    pub client_ready: bool,
    pub agent_ready: bool,
}

impl Validation {
    fn ready(&self) -> bool {
        self.target_matches && self.identity_unchanged && self.client_ready && self.agent_ready
    }
}

pub trait Backend {
    fn stop_services(&mut self) -> Result<(), String>;
    fn apply_package(&mut self) -> Result<InstallOutcome, String>;
    fn validate_target(&mut self) -> Result<Validation, String>;
    fn rollback_original(&mut self) -> Result<Validation, String>;
}

#[derive(Debug)]
pub struct ExecutionResult {
    pub phase: Phase,
    pub error: Option<String>,
    pub reboot_required: bool,
}

fn persist_phase(
    path: &Path,
    transaction: &mut Transaction,
    phase: Phase,
    evidence: &Evidence,
    now: u64,
) -> Result<(), String> {
    let mut next = transaction.clone();
    next.advance(phase, evidence, now)?;
    next.save(path)?;
    *transaction = next;
    Ok(())
}

fn rollback(
    path: &Path,
    transaction: &mut Transaction,
    evidence: &Evidence,
    backend: &mut impl Backend,
    now: u64,
    error: String,
) -> Result<ExecutionResult, String> {
    persist_phase(path, transaction, Phase::RollingBack, evidence, now)?;
    let restored = backend.rollback_original();
    let mut proof = *evidence;
    let phase = match &restored {
        Ok(value) if value.ready() => {
            proof.rollback_verified = true;
            proof.identity_unchanged = true;
            proof.client_ready = true;
            proof.agent_ready = true;
            Phase::RolledBack
        }
        _ => Phase::RollbackFailed,
    };
    persist_phase(path, transaction, phase, &proof, now)?;
    let error = match restored {
        Ok(_) => error,
        Err(restore) => format!("{error}; rollback: {restore}"),
    };
    Ok(ExecutionResult {
        phase,
        error: Some(error),
        reboot_required: false,
    })
}

pub fn execute(
    path: &Path,
    transaction: &mut Transaction,
    evidence: &Evidence,
    backend: &mut impl Backend,
    now: u64,
) -> Result<ExecutionResult, String> {
    if !transaction.may_stop_services(path, evidence) {
        return Err("installation execution stop authorization unavailable".into());
    }
    transaction.save(path)?;
    if let Err(error) = backend.stop_services() {
        return rollback(path, transaction, evidence, backend, now, error);
    }
    let mut proof = *evidence;
    proof.services_stopped = true;
    persist_phase(path, transaction, Phase::ServicesStopped, &proof, now)?;
    persist_phase(path, transaction, Phase::Applying, &proof, now)?;
    let outcome = match backend.apply_package() {
        Ok(InstallOutcome::Pending { pid, .. }) => {
            return Ok(ExecutionResult {
                phase: Phase::Applying,
                error: Some(format!(
                    "installer still active; reconciliation required; pid={pid}"
                )),
                reboot_required: false,
            })
        }
        Ok(InstallOutcome::Busy) => {
            return rollback(
                path,
                transaction,
                &proof,
                backend,
                now,
                "installer busy (1618)".into(),
            )
        }
        Ok(InstallOutcome::Failed(code)) => {
            return rollback(
                path,
                transaction,
                &proof,
                backend,
                now,
                format!("installer failed ({code})"),
            )
        }
        Err(error) => return rollback(path, transaction, &proof, backend, now, error),
        Ok(outcome) => outcome,
    };
    persist_phase(path, transaction, Phase::Validating, &proof, now)?;
    let validation = backend.validate_target();
    if outcome == InstallOutcome::RebootRequired {
        return Ok(ExecutionResult {
            phase: Phase::Validating,
            error: Some("installer requires reboot before final verification (3010)".into()),
            reboot_required: true,
        });
    }
    match validation {
        Ok(value) if value.ready() => {
            proof.target_verified = true;
            proof.identity_unchanged = true;
            proof.client_ready = true;
            proof.agent_ready = true;
            persist_phase(path, transaction, Phase::Committed, &proof, now)?;
            Ok(ExecutionResult {
                phase: Phase::Committed,
                error: None,
                reboot_required: false,
            })
        }
        Ok(_) => rollback(
            path,
            transaction,
            &proof,
            backend,
            now,
            "installation target validation failed".into(),
        ),
        Err(error) => rollback(
            path,
            transaction,
            &proof,
            backend,
            now,
            format!("installation target validation failed: {error}"),
        ),
    }
}

pub fn resume(
    path: &Path,
    transaction: &mut Transaction,
    evidence: &Evidence,
    backend: &mut impl Backend,
    installer_result: Option<InstallOutcome>,
    now: u64,
) -> Result<ExecutionResult, String> {
    let persisted = Transaction::decode(&std::fs::read(path).map_err(|e| e.to_string())?)?;
    if persisted != *transaction {
        return Err("installation resume snapshot mismatch".into());
    }
    if !evidence.maintenance_owned || !evidence.backups_verified {
        return Err("installation resume ownership/backups unavailable".into());
    }
    match transaction.phase {
        Phase::Committed | Phase::RolledBack | Phase::RollbackFailed => {
            return Ok(ExecutionResult {
                phase: transaction.phase,
                error: None,
                reboot_required: false,
            });
        }
        Phase::StoppingServices | Phase::ServicesStopped => {
            return rollback(
                path,
                transaction,
                evidence,
                backend,
                now,
                "installation interrupted before apply; restoring original".into(),
            );
        }
        Phase::RollingBack => {
            let restored = backend.rollback_original();
            let mut proof = *evidence;
            let phase = match &restored {
                Ok(value) if value.ready() => {
                    proof.rollback_verified = true;
                    proof.identity_unchanged = true;
                    proof.client_ready = true;
                    proof.agent_ready = true;
                    Phase::RolledBack
                }
                _ => Phase::RollbackFailed,
            };
            persist_phase(path, transaction, phase, &proof, now)?;
            return Ok(ExecutionResult {
                phase,
                error: restored.err(),
                reboot_required: false,
            });
        }
        Phase::Applying => match installer_result {
            None | Some(InstallOutcome::Pending { .. }) => {
                return Err("installation installer result unknown; reconciliation required".into())
            }
            Some(InstallOutcome::Busy) => {
                return rollback(
                    path,
                    transaction,
                    evidence,
                    backend,
                    now,
                    "installer busy (1618)".into(),
                )
            }
            Some(InstallOutcome::Failed(code)) => {
                return rollback(
                    path,
                    transaction,
                    evidence,
                    backend,
                    now,
                    format!("installer failed ({code})"),
                )
            }
            Some(InstallOutcome::Installed | InstallOutcome::RebootRequired) => {
                persist_phase(path, transaction, Phase::Validating, evidence, now)?;
            }
        },
        Phase::Validating => {}
        _ => return Err("installation resume phase not executable".into()),
    }
    match backend.validate_target() {
        Ok(value) if value.ready() => {
            let mut proof = *evidence;
            proof.target_verified = true;
            proof.identity_unchanged = true;
            proof.client_ready = true;
            proof.agent_ready = true;
            persist_phase(path, transaction, Phase::Committed, &proof, now)?;
            Ok(ExecutionResult {
                phase: Phase::Committed,
                error: None,
                reboot_required: false,
            })
        }
        Ok(_) => rollback(
            path,
            transaction,
            evidence,
            backend,
            now,
            "installation resumed validation failed".into(),
        ),
        Err(error) => rollback(
            path,
            transaction,
            evidence,
            backend,
            now,
            format!("installation resumed validation failed: {error}"),
        ),
    }
}
