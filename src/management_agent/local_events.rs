//! Local R1/R3/R4 receipts and outbox, never action authority.
//! ACK keeps receipts: the action owner must retain its operation ID across boots.
//! The uploader binds installation/generation and signs the existing event wire.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Read,
    path::{Path, PathBuf},
};

pub const MAX_EVENTS: usize = 256;
pub const MAX_OPERATIONS: usize = 256;
pub const MAX_RECORD_BYTES: usize = 2048;
pub const MAX_STORE_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_ERROR_INPUT: usize = 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RecoveryCode {
    R1,
    R3,
    R4,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    StartService,
    RestoreFallback,
    RepairInstallation,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Operation {
    pub operation_id: String,
    pub code: RecoveryCode,
    pub action: Action,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Binding {
    pub client_id: String,
    pub client_uuid: String,
    pub agent_installation_id: String,
    pub enrollment_generation: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Started,
    Progress,
    Terminal,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Running,
    Succeeded,
    Failed,
    Deferred,
    RolledBack,
    RollbackFailed,
    ReconciliationRequired,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PathKey {
    RustDeskService,
    NormalClient,
    RescueClient,
    ManagementAgent,
    RecoveryBootstrap,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    DiskFull,
    WriterBusy,
    LocallyBlocked,
    VerificationFailed,
    HealthUnconfirmed,
    Redacted,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Record {
    pub phase: Phase,
    pub attempt_seq: u32,
    pub outcome: Outcome,
    pub path_key: PathKey,
    pub error_code: Option<ErrorCode>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PendingEvent {
    pub event_id: String,
    pub digest: String,
    pub payload: serde_json::Value,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventError {
    InvalidRecord,
    Conflict,
    Terminal,
    Full,
    AckDigest,
    AckUnknown,
    StaleWriter,
    WriterBusy,
    DiskFull,
    Storage,
    UnsafePath,
    Corrupt,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Receipt {
    operation: Operation,
    records: Vec<Record>,
    #[serde(default)]
    binding: Option<Binding>,
    #[serde(default)]
    terminal_at: Option<u64>,
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Snapshot {
    schema: u32,
    operations: BTreeMap<String, Receipt>,
    events: Vec<PendingEvent>,
    #[serde(default)]
    verified_binding: Option<Binding>,
    #[serde(default)]
    next_operation: u64,
}

pub struct LocalEvents {
    path: PathBuf,
    snapshot: Snapshot,
    disk_digest: Option<String>,
    #[cfg(test)]
    publish_error: Option<i32>,
}

impl LocalEvents {
    pub(super) fn active_repairs(&self) -> Vec<(Operation, Binding)> {
        self.snapshot
            .operations
            .values()
            .filter(|receipt| {
                receipt.operation.code == RecoveryCode::R4
                    && receipt
                        .records
                        .last()
                        .is_some_and(|record| record.phase != Phase::Terminal)
            })
            .filter_map(|receipt| {
                receipt
                    .binding
                    .clone()
                    .map(|binding| (receipt.operation.clone(), binding))
            })
            .take(8)
            .collect()
    }
    pub fn operation_record(&self, id: &str) -> Option<(Operation, Binding, Record)> {
        let receipt = self.snapshot.operations.get(id)?;
        Some((
            receipt.operation.clone(),
            receipt.binding.clone()?,
            *receipt.records.last()?,
        ))
    }
    pub fn active_fallback(&self) -> Option<(Operation, Binding)> {
        self.snapshot
            .operations
            .values()
            .find(|r| {
                r.operation.code == RecoveryCode::R3
                    && r.records.last().is_some_and(|v| v.phase != Phase::Terminal)
            })
            .and_then(|r| r.binding.clone().map(|b| (r.operation.clone(), b)))
    }
    pub fn begin_fallback(&mut self, binding: &Binding, now: u64) -> Result<Operation, EventError> {
        validate_binding(binding)?;
        if self.active_fallback().is_some() {
            return Err(EventError::Conflict);
        }
        let sequence = self
            .snapshot
            .next_operation
            .checked_add(1)
            .ok_or(EventError::Full)?;
        let operation = Operation {
            operation_id: digest(
                format!(
                    "local-r3-v1:{}:{}:{sequence}",
                    binding.agent_installation_id, binding.enrollment_generation
                )
                .as_bytes(),
            )[..32]
                .into(),
            code: RecoveryCode::R3,
            action: Action::RestoreFallback,
        };
        self.record_inner(
            &operation,
            Record {
                phase: Phase::Started,
                attempt_seq: 1,
                outcome: Outcome::Running,
                path_key: PathKey::RescueClient,
                error_code: None,
            },
            Some(binding),
            now,
            Some(sequence),
        )?;
        Ok(operation)
    }
    pub fn remember_binding(&mut self, binding: Binding) -> Result<(), EventError> {
        validate_binding(&binding)?;
        if self.snapshot.verified_binding.as_ref() == Some(&binding) {
            return Ok(());
        }
        let mut next = self.snapshot.clone();
        next.verified_binding = Some(binding);
        self.commit(next)
    }
    pub fn verified_binding(&self) -> Option<&Binding> {
        self.snapshot.verified_binding.as_ref()
    }
    pub fn begin_r1(&mut self, binding: &Binding, now: u64) -> Result<Operation, EventError> {
        validate_binding(binding)?;
        if let Some((operation, original)) = self.active_r1() {
            return if original == *binding {
                Ok(operation)
            } else {
                Err(EventError::Conflict)
            };
        }
        let sequence = self
            .snapshot
            .next_operation
            .checked_add(1)
            .ok_or(EventError::Full)?;
        let operation = Operation {
            operation_id: digest(
                format!(
                    "local-r1-v1:{}:{}:{}:{}:{sequence}",
                    binding.client_id,
                    binding.client_uuid,
                    binding.agent_installation_id,
                    binding.enrollment_generation
                )
                .as_bytes(),
            )[..32]
                .into(),
            code: RecoveryCode::R1,
            action: Action::StartService,
        };
        self.record_inner(
            &operation,
            Record {
                phase: Phase::Started,
                attempt_seq: 1,
                outcome: Outcome::Running,
                path_key: PathKey::RustDeskService,
                error_code: None,
            },
            Some(binding),
            now,
            Some(sequence),
        )?;
        Ok(operation)
    }
    pub fn active_r1(&self) -> Option<(Operation, Binding)> {
        self.snapshot
            .operations
            .values()
            .find(|receipt| {
                receipt.operation.code == RecoveryCode::R1
                    && receipt.binding.is_some()
                    && receipt
                        .records
                        .last()
                        .is_some_and(|record| record.phase != Phase::Terminal)
            })
            .and_then(|receipt| {
                receipt
                    .binding
                    .as_ref()
                    .map(|binding| (receipt.operation.clone(), binding.clone()))
            })
    }
    pub fn record_bound_at(
        &mut self,
        operation: &Operation,
        record: Record,
        binding: &Binding,
        now: u64,
    ) -> Result<PendingEvent, EventError> {
        validate_binding(binding)?;
        self.record_inner(operation, record, Some(binding), now, None)
    }
    pub fn prune_confirmed(&mut self, now: u64) -> Result<usize, EventError> {
        let pending = self
            .snapshot
            .events
            .iter()
            .filter_map(|event| event.payload["result"]["operation_id"].as_str())
            .collect::<BTreeSet<_>>();
        let mut next = self.snapshot.clone();
        next.operations.retain(|id, receipt| {
            pending.contains(id.as_str())
                || !receipt
                    .records
                    .last()
                    .is_some_and(|record| record.phase == Phase::Terminal)
                || !receipt
                    .terminal_at
                    .is_some_and(|time| now >= time && now - time >= 30 * 86400)
        });
        let removed = self.snapshot.operations.len() - next.operations.len();
        if removed > 0 {
            self.commit(next)?;
        }
        Ok(removed)
    }
    pub fn open(path: &Path) -> Result<Self, EventError> {
        verify_parent(path)?;
        let bytes = read_store(path)?;
        let snapshot = match &bytes {
            Some(bytes) => {
                let snapshot: Snapshot =
                    serde_json::from_slice(bytes).map_err(|_| EventError::Corrupt)?;
                validate_snapshot(&snapshot)?;
                snapshot
            }
            None => Snapshot {
                schema: 1,
                ..Snapshot::default()
            },
        };
        Ok(Self {
            path: path.into(),
            snapshot,
            disk_digest: bytes.as_ref().map(|bytes| digest(bytes)),
            #[cfg(test)]
            publish_error: None,
        })
    }

    /// A 32-lowercase-hex operation ID is supplied by the existing action owner.
    /// ID = SHA256("agent-local-event-v1:{operation_id}:{phase}:{attempt_seq}")[..32].
    /// Exact replay is read-only, including after ACK; conflicting content fails.
    pub fn record(
        &mut self,
        operation: &Operation,
        record: Record,
    ) -> Result<PendingEvent, EventError> {
        self.record_inner(operation, record, None, 0, None)
    }

    fn record_inner(
        &mut self,
        operation: &Operation,
        record: Record,
        binding: Option<&Binding>,
        now: u64,
        allocation: Option<u64>,
    ) -> Result<PendingEvent, EventError> {
        validate_operation(operation)?;
        validate_record(record)?;
        if let Some(receipt) = self.snapshot.operations.get(&operation.operation_id) {
            if receipt.operation != *operation || receipt.binding.as_ref() != binding {
                return Err(EventError::Conflict);
            }
            if let Some((index, previous)) =
                receipt.records.iter().enumerate().find(|(_, previous)| {
                    previous.phase == record.phase && previous.attempt_seq == record.attempt_seq
                })
            {
                if *previous != record {
                    return Err(EventError::Conflict);
                }
                return bound_event(operation, record, index as u64 + 1, binding);
            }
            validate_next(&receipt.records, record)?;
        } else {
            validate_next(&[], record)?;
            if self.snapshot.operations.len() >= MAX_OPERATIONS {
                return Err(EventError::Full);
            }
        }
        let mut next = self.snapshot.clone();
        if let Some(sequence) = allocation {
            next.next_operation = sequence;
        }
        let receipt = next
            .operations
            .entry(operation.operation_id.clone())
            .or_insert_with(|| Receipt {
                operation: operation.clone(),
                records: Vec::new(),
                binding: binding.cloned(),
                terminal_at: None,
            });
        let event = bound_event(operation, record, receipt.records.len() as u64 + 1, binding)?;
        if record.phase == Phase::Terminal && now > 0 {
            receipt.terminal_at = Some(now);
        }
        receipt.records.push(record);
        next.events.push(event.clone());
        self.commit(next)?;
        Ok(event)
    }

    /// Returns persisted, unacknowledged events only; never schedules an action.
    pub fn pending_events(&self) -> &[PendingEvent] {
        &self.snapshot.events
    }

    /// Observation only. An existing receipt must prevent caller-side reexecution.
    pub fn operation_seen(&self, operation_id: &str) -> bool {
        self.snapshot.operations.contains_key(operation_id)
    }

    /// Caller must first validate the signed-transport response for this event.
    /// Both unknown (including duplicate) ACK and a wrong local digest are errors.
    pub fn acknowledge(&mut self, event_id: &str, digest: &str) -> Result<(), EventError> {
        let pending = self
            .snapshot
            .events
            .iter()
            .find(|event| event.event_id == event_id)
            .ok_or(EventError::AckUnknown)?;
        if pending.digest != digest {
            return Err(EventError::AckDigest);
        }
        let mut next = self.snapshot.clone();
        next.events.retain(|event| event.event_id != event_id);
        self.commit(next)
    }

    fn commit(&mut self, next: Snapshot) -> Result<(), EventError> {
        let bytes = bounded_snapshot(&next)?;
        verify_parent(&self.path)?;
        let _writer = writer_lock(&self.path.with_extension("lock"))?;
        let current = read_store(&self.path)?.as_ref().map(|bytes| digest(bytes));
        if current != self.disk_digest {
            return Err(EventError::StaleWriter);
        }
        #[cfg(test)]
        if let Some(error) = self.publish_error.take() {
            return Err(storage_error(std::io::Error::from_raw_os_error(error)));
        }
        super::state_store::publish(&self.path, &bytes).map_err(storage_error)?;
        self.disk_digest = Some(digest(&bytes));
        self.snapshot = next;
        Ok(())
    }

    /// One-shot OS failure at the publish boundary, after actual lock/CAS.
    #[cfg(test)]
    pub fn fail_next_publish_for_test(&mut self, os_error: i32) {
        self.publish_error = Some(os_error);
    }
}

/// Accept bounded diagnostic text only to map known codes or discard it entirely.
/// Raw text, paths, dumps, secrets and arbitrary JSON never enter a Record.
pub fn redact_error(text: &str) -> Result<ErrorCode, EventError> {
    if text.len() > MAX_ERROR_INPUT {
        return Err(EventError::InvalidRecord);
    }
    Ok(match text {
        "disk_full" => ErrorCode::DiskFull,
        "writer_busy" => ErrorCode::WriterBusy,
        "locally_blocked" => ErrorCode::LocallyBlocked,
        "verification_failed" => ErrorCode::VerificationFailed,
        "health_unconfirmed" => ErrorCode::HealthUnconfirmed,
        _ => ErrorCode::Redacted,
    })
}

fn digest(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn validate_operation(operation: &Operation) -> Result<(), EventError> {
    if operation.operation_id.len() != 32
        || !operation
            .operation_id
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        || !matches!(
            (operation.code, operation.action),
            (RecoveryCode::R1, Action::StartService)
                | (RecoveryCode::R3, Action::RestoreFallback)
                | (RecoveryCode::R4, Action::RepairInstallation)
        )
    {
        return Err(EventError::InvalidRecord);
    }
    Ok(())
}

fn validate_record(record: Record) -> Result<(), EventError> {
    if record.attempt_seq == 0
        || (record.phase != Phase::Terminal) != (record.outcome == Outcome::Running)
    {
        return Err(EventError::InvalidRecord);
    }
    Ok(())
}

fn validate_next(records: &[Record], record: Record) -> Result<(), EventError> {
    if let Some(previous) = records.last() {
        if previous.phase == Phase::Terminal {
            return Err(EventError::Terminal);
        }
        if record.phase == Phase::Started || record.attempt_seq < previous.attempt_seq {
            return Err(EventError::InvalidRecord);
        }
    } else if record.phase != Phase::Started {
        return Err(EventError::InvalidRecord);
    }
    Ok(())
}

fn event(operation: &Operation, record: Record, sequence: u64) -> Result<PendingEvent, EventError> {
    let phase = match record.phase {
        Phase::Started => "started",
        Phase::Progress => "progress",
        Phase::Terminal => "terminal",
    };
    let id = digest(
        format!(
            "agent-local-event-v1:{}:{phase}:{}",
            operation.operation_id, record.attempt_seq
        )
        .as_bytes(),
    )[..32]
        .to_owned();
    let payload = serde_json::json!({"schema":1,"event_id":id,"command_id":null,"sequence":sequence,
        "status":record.outcome,"result":{"operation_id":operation.operation_id,"code":operation.code,
        "action":operation.action,"phase":record.phase,"attempt_seq":record.attempt_seq,
        "outcome":record.outcome,"path_key":record.path_key,"error_code":record.error_code}});
    let mut payload = payload;
    if operation.code == RecoveryCode::R4 && record.outcome == Outcome::ReconciliationRequired {
        payload["result"]["client_recovered"] = false.into();
    }
    let bytes = serde_json::to_vec(&payload).map_err(|_| EventError::Storage)?;
    if bytes.len() > MAX_RECORD_BYTES {
        return Err(EventError::Full);
    }
    Ok(PendingEvent {
        event_id: id,
        digest: digest(&bytes),
        payload,
    })
}

fn bounded_snapshot(snapshot: &Snapshot) -> Result<Vec<u8>, EventError> {
    let active = snapshot
        .operations
        .values()
        .filter(|receipt| {
            receipt
                .records
                .last()
                .is_some_and(|record| record.phase != Phase::Terminal)
        })
        .count();
    if snapshot.operations.len() > MAX_OPERATIONS || snapshot.events.len() + active > MAX_EVENTS {
        return Err(EventError::Full);
    }
    let bytes = serde_json::to_vec(snapshot).map_err(|_| EventError::Storage)?;
    // One fixed record plus its pending event per active operation; do not evict receipts.
    if bytes.len() + active * (MAX_RECORD_BYTES * 2) > MAX_STORE_BYTES {
        return Err(EventError::Full);
    }
    Ok(bytes)
}

fn validate_snapshot(snapshot: &Snapshot) -> Result<(), EventError> {
    if snapshot.schema != 1 {
        return Err(EventError::Corrupt);
    }
    bounded_snapshot(snapshot)?;
    if let Some(binding) = &snapshot.verified_binding {
        validate_binding(binding).map_err(|_| EventError::Corrupt)?;
    }
    let mut known = BTreeMap::new();
    for (id, receipt) in &snapshot.operations {
        validate_operation(&receipt.operation).map_err(|_| EventError::Corrupt)?;
        if let Some(binding) = &receipt.binding {
            validate_binding(binding).map_err(|_| EventError::Corrupt)?;
        }
        if receipt.terminal_at.is_some()
            && !receipt
                .records
                .last()
                .is_some_and(|r| r.phase == Phase::Terminal)
        {
            return Err(EventError::Corrupt);
        }
        if id != &receipt.operation.operation_id || receipt.records.is_empty() {
            return Err(EventError::Corrupt);
        }
        for (index, record) in receipt.records.iter().enumerate() {
            validate_record(*record).map_err(|_| EventError::Corrupt)?;
            validate_next(&receipt.records[..index], *record).map_err(|_| EventError::Corrupt)?;
            let event = bound_event(
                &receipt.operation,
                *record,
                index as u64 + 1,
                receipt.binding.as_ref(),
            )
            .map_err(|_| EventError::Corrupt)?;
            if known.insert(event.event_id.clone(), event).is_some() {
                return Err(EventError::Corrupt);
            }
        }
    }
    let mut pending = BTreeSet::new();
    for event in &snapshot.events {
        if known.get(&event.event_id) != Some(event) || !pending.insert(&event.event_id) {
            return Err(EventError::Corrupt);
        }
    }
    Ok(())
}

fn validate_binding(binding: &Binding) -> Result<(), EventError> {
    if binding.client_id.is_empty()
        || binding.client_id.len() > 128
        || binding.client_id.chars().any(char::is_control)
        || binding.client_uuid.is_empty()
        || binding.client_uuid.len() > 256
        || binding.client_uuid.chars().any(char::is_control)
        || binding.agent_installation_id.len() != 32
        || !binding
            .agent_installation_id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        || binding.enrollment_generation == 0
        || binding.enrollment_generation > i64::MAX as u64
    {
        return Err(EventError::InvalidRecord);
    }
    Ok(())
}

fn bound_event(
    operation: &Operation,
    record: Record,
    sequence: u64,
    binding: Option<&Binding>,
) -> Result<PendingEvent, EventError> {
    let mut result = event(operation, record, sequence)?;
    if let Some(binding) = binding {
        result.payload["client_id"] = binding.client_id.clone().into();
        result.payload["client_uuid"] = binding.client_uuid.clone().into();
        result.payload["agent_installation_id"] = binding.agent_installation_id.clone().into();
        result.payload["enrollment_generation"] = binding.enrollment_generation.into();
        let bytes = serde_json::to_vec(&result.payload).map_err(|_| EventError::Storage)?;
        if bytes.len() > MAX_RECORD_BYTES {
            return Err(EventError::Full);
        }
        result.digest = digest(&bytes);
    }
    Ok(result)
}

fn verify_parent(path: &Path) -> Result<(), EventError> {
    // Reject ADS and alternate lexical paths before deriving companion filenames.
    if !path.is_absolute()
        || path
            .components()
            .any(|part| matches!(part, std::path::Component::ParentDir))
        || path
            .file_name()
            .and_then(|name| name.to_str())
            .map_or(true, |name| {
                name.contains(':') || name.ends_with('.') || name.ends_with(' ')
            })
    {
        return Err(EventError::UnsafePath);
    }
    super::workspace::verify(path.parent().ok_or(EventError::UnsafePath)?)
        .map_err(|_| EventError::UnsafePath)
}

fn read_store(path: &Path) -> Result<Option<Vec<u8>>, EventError> {
    match std::fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(storage_error(error)),
        Ok(_) => super::workspace::verify_file(path).map_err(|_| EventError::UnsafePath)?,
    }
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .map_err(storage_error)?
        .take(MAX_STORE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(storage_error)?;
    if bytes.len() > MAX_STORE_BYTES {
        return Err(EventError::Full);
    }
    Ok(Some(bytes))
}

fn writer_lock(path: &Path) -> Result<std::fs::File, EventError> {
    use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if !metadata.is_file() || metadata.file_attributes() & 0x400 != 0 => {
            return Err(EventError::UnsafePath)
        }
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
            return Err(storage_error(error))
        }
        _ => {}
    }
    std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .share_mode(0)
        .custom_flags(0x00200000)
        .open(path)
        .map_err(|error| {
            if matches!(error.raw_os_error(), Some(32 | 33)) {
                EventError::WriterBusy
            } else {
                storage_error(error)
            }
        })
}

fn storage_error(error: std::io::Error) -> EventError {
    if matches!(error.raw_os_error(), Some(39 | 112)) {
        EventError::DiskFull
    } else {
        EventError::Storage
    }
}
