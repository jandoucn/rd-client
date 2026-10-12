use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Planned,
    PackageVerified,
    Prepared,
    StoppingServices,
    ServicesStopped,
    Applying,
    Validating,
    Committed,
    RollingBack,
    RolledBack,
    RollbackFailed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Transaction {
    pub schema: u32,
    pub id: String,
    pub target_key: String,
    pub package_sha256: String,
    pub identity_fingerprint: String,
    pub created_at: u64,
    pub updated_at: u64,
    pub attempts: u8,
    pub phase: Phase,
    pub revision: u64,
}

#[derive(Clone, Copy)]
pub struct Evidence {
    pub package_verified: bool,
    pub backups_verified: bool,
    pub helper_verified: bool,
    pub resume_task_verified: bool,
    pub handoff_verified: bool,
    pub maintenance_owned: bool,
    pub services_stopped: bool,
    pub target_verified: bool,
    pub identity_unchanged: bool,
    pub client_ready: bool,
    pub agent_ready: bool,
    pub rollback_verified: bool,
    pub locally_blocked: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub enum RecoveryStep {
    Prepare,
    Reconcile,
    Rollback,
    WaitForOwner,
    ReportOnly,
    ManualReconciliation,
}

fn hex_token(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

impl Transaction {
    pub fn new(
        id: String,
        target_key: String,
        package_sha256: String,
        identity_fingerprint: String,
        now: u64,
    ) -> Result<Self, String> {
        let transaction = Self {
            schema: 1,
            id,
            target_key,
            package_sha256,
            identity_fingerprint,
            created_at: now,
            updated_at: now,
            attempts: 0,
            phase: Phase::Planned,
            revision: 0,
        };
        transaction.validate()?;
        Ok(transaction)
    }

    fn validate(&self) -> Result<(), String> {
        if self.schema != 1
            || !hex_token(&self.id, 32)
            || !hex_token(&self.package_sha256, 64)
            || !hex_token(&self.identity_fingerprint, 64)
            || ![
                "windows-x86_64-msi-standard",
                "windows-x86_64-msi-sos",
                "windows-x86_64-exe-standard",
                "windows-x86_64-exe-sos",
            ]
            .contains(&self.target_key.as_str())
            || self.updated_at < self.created_at
        {
            return Err("transaction schema or identity invalid".into());
        }
        Ok(())
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() > 65536 {
            return Err("transaction snapshot too large".into());
        }
        let transaction: Self = serde_json::from_slice(bytes)
            .map_err(|_| "transaction snapshot malformed".to_owned())?;
        transaction.validate()?;
        Ok(transaction)
    }

    pub fn advance(&mut self, next: Phase, evidence: &Evidence, now: u64) -> Result<(), String> {
        self.validate()?;
        if now < self.updated_at {
            return Err("transaction clock moved backwards".into());
        }
        let permitted = match (self.phase, next) {
            (Phase::Planned, Phase::PackageVerified) => {
                evidence.package_verified && !evidence.locally_blocked
            }
            (Phase::PackageVerified, Phase::Prepared) => {
                evidence.package_verified
                    && evidence.backups_verified
                    && evidence.helper_verified
                    && evidence.resume_task_verified
                    && evidence.handoff_verified
                    && evidence.maintenance_owned
                    && !evidence.locally_blocked
            }
            (Phase::Prepared, Phase::StoppingServices) => {
                evidence.package_verified
                    && evidence.backups_verified
                    && evidence.helper_verified
                    && evidence.resume_task_verified
                    && evidence.handoff_verified
                    && evidence.maintenance_owned
                    && !evidence.locally_blocked
            }
            (Phase::StoppingServices, Phase::ServicesStopped) => {
                evidence.backups_verified
                    && evidence.helper_verified
                    && evidence.resume_task_verified
                    && evidence.handoff_verified
                    && evidence.maintenance_owned
                    && evidence.services_stopped
                    && !evidence.locally_blocked
            }
            (Phase::ServicesStopped, Phase::Applying) => {
                evidence.package_verified
                    && evidence.backups_verified
                    && evidence.services_stopped
                    && evidence.maintenance_owned
                    && !evidence.locally_blocked
            }
            (Phase::Applying, Phase::Validating) => evidence.maintenance_owned,
            (Phase::Validating, Phase::Committed) => {
                evidence.target_verified
                    && evidence.identity_unchanged
                    && evidence.client_ready
                    && evidence.agent_ready
                    && evidence.maintenance_owned
            }
            (
                Phase::Prepared
                | Phase::StoppingServices
                | Phase::ServicesStopped
                | Phase::Applying
                | Phase::Validating,
                Phase::RollingBack,
            ) => evidence.backups_verified && evidence.maintenance_owned,
            (Phase::RollingBack, Phase::RolledBack) => {
                evidence.rollback_verified
                    && evidence.identity_unchanged
                    && evidence.client_ready
                    && evidence.agent_ready
            }
            (Phase::RollingBack, Phase::RollbackFailed) => true,
            _ => false,
        };
        if !permitted {
            return Err("transaction transition or evidence refused".into());
        }
        let revision = self
            .revision
            .checked_add(1)
            .ok_or_else(|| "transaction revision overflow".to_owned())?;
        self.phase = next;
        self.updated_at = now;
        self.revision = revision;
        Ok(())
    }

    pub fn recovery_step(&self, owner_alive: bool, now: u64) -> RecoveryStep {
        if self.validate().is_err() {
            return RecoveryStep::ManualReconciliation;
        }
        match self.phase {
            Phase::Committed | Phase::RolledBack => return RecoveryStep::ReportOnly,
            Phase::RollbackFailed => return RecoveryStep::ManualReconciliation,
            _ => {}
        }
        if owner_alive {
            return RecoveryStep::WaitForOwner;
        }
        if now < self.updated_at
            || now.saturating_sub(self.created_at) >= 86400
            || self.attempts >= 3
        {
            return RecoveryStep::ManualReconciliation;
        }
        match self.phase {
            Phase::Planned | Phase::PackageVerified => RecoveryStep::Prepare,
            Phase::RollingBack => RecoveryStep::Rollback,
            _ => RecoveryStep::Reconcile,
        }
    }

    pub fn may_stop_services(&self, path: &Path, evidence: &Evidence) -> bool {
        self.validate().is_ok()
            && std::fs::read(path)
                .ok()
                .and_then(|bytes| Self::decode(&bytes).ok())
                .map(|persisted| persisted == *self)
                .unwrap_or(false)
            && self.phase == Phase::StoppingServices
            && evidence.package_verified
            && evidence.backups_verified
            && evidence.helper_verified
            && evidence.resume_task_verified
            && evidence.handoff_verified
            && evidence.maintenance_owned
            && !evidence.locally_blocked
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        use std::io::Write;
        self.validate()?;
        #[cfg(windows)]
        let _writer = {
            use std::os::windows::fs::OpenOptionsExt;
            std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .share_mode(0)
                .open(path.with_extension("lock"))
                .map_err(|error| format!("transaction writer unavailable: {error}"))?
        };
        match std::fs::read(path) {
            Ok(bytes) => {
                let previous = Self::decode(&bytes)?;
                if previous.id != self.id
                    || previous.target_key != self.target_key
                    || previous.package_sha256 != self.package_sha256
                    || previous.identity_fingerprint != self.identity_fingerprint
                    || previous.created_at != self.created_at
                    || previous.updated_at > self.updated_at
                {
                    return Err("transaction immutable identity conflict".into());
                }
                if previous.phase != self.phase
                    && matches!(
                        previous.phase,
                        Phase::Committed | Phase::RolledBack | Phase::RollbackFailed
                    )
                {
                    return Err("transaction terminal record immutable".into());
                }
                if previous != *self && previous.revision.checked_add(1) != Some(self.revision) {
                    return Err("transaction stale writer or skipped persistence boundary".into());
                }
                if previous != *self
                    && !matches!(
                        (previous.phase, self.phase),
                        (Phase::Planned, Phase::PackageVerified)
                            | (Phase::PackageVerified, Phase::Prepared)
                            | (Phase::Prepared, Phase::StoppingServices)
                            | (Phase::StoppingServices, Phase::ServicesStopped)
                            | (Phase::ServicesStopped, Phase::Applying)
                            | (Phase::Applying, Phase::Validating)
                            | (Phase::Validating, Phase::Committed)
                            | (
                                Phase::Prepared
                                    | Phase::StoppingServices
                                    | Phase::ServicesStopped
                                    | Phase::Applying
                                    | Phase::Validating,
                                Phase::RollingBack
                            )
                            | (
                                Phase::RollingBack,
                                Phase::RolledBack | Phase::RollbackFailed
                            )
                    )
                {
                    return Err("transaction persisted phase transition refused".into());
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                if self.revision != 0 || self.phase != Phase::Planned {
                    return Err("transaction initial snapshot must be planned".into());
                }
            }
            Err(error) => {
                return Err(format!(
                    "transaction previous snapshot unavailable: {error}"
                ))
            }
        }
        let bytes = serde_json::to_vec(self).map_err(|error| error.to_string())?;
        let temporary = path.with_extension("pending");
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|error| error.to_string())?;
        let result = (|| {
            file.write_all(&bytes)
                .and_then(|_| file.sync_all())
                .map_err(|error| error.to_string())?;
            drop(file);
            publish(&temporary, path)
        })();
        if let Err(error) = result {
            if let Err(cleanup) = std::fs::remove_file(temporary) {
                return Err(format!(
                    "{error}; transaction staging cleanup failed: {cleanup}"
                ));
            }
            return Err(error);
        }
        Ok(())
    }
}

#[cfg(windows)]
fn publish(source: &Path, destination: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "kernel32")]
    extern "system" {
        fn MoveFileExW(source: *const u16, destination: *const u16, flags: u32) -> i32;
    }
    let source: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let destination: Vec<u16> = destination
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    if unsafe { MoveFileExW(source.as_ptr(), destination.as_ptr(), 1 | 8) } == 0 {
        return Err(format!(
            "transaction publish failed: {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(())
}

#[cfg(not(windows))]
fn publish(source: &Path, destination: &Path) -> Result<(), String> {
    std::fs::rename(source, destination).map_err(|error| error.to_string())?;
    std::fs::File::open(
        destination
            .parent()
            .ok_or("transaction parent unavailable")?,
    )
    .and_then(|file| file.sync_all())
    .map_err(|error| error.to_string())
}
