use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    io::Write,
    path::{Path, PathBuf},
};

const MAX_EVENTS: usize = 10_000;
const MAX_BYTES: usize = 64 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionState {
    ClaimPrepared,
    Accepted,
    Deferred,
    Running,
    Succeeded,
    Failed,
    Expired,
    Unsupported,
    Rejected,
    RolledBack,
    RollbackFailed,
    ReconciliationRequired,
    Cancelled,
}

#[derive(Clone, Serialize, Deserialize)]
struct Receipt {
    digest: String,
    expires_at: u64,
    state: ExecutionState,
    sequence: u64,
    updated_at: u64,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct PendingEvent {
    pub event_id: String,
    pub digest: String,
    pub payload: serde_json::Value,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Snapshot {
    schema: u32,
    commands: BTreeMap<String, Receipt>,
    events: Vec<PendingEvent>,
}

pub struct CommandJournal {
    path: PathBuf,
    snapshot: Snapshot,
    disk_digest: Option<String>,
}

fn token(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

impl CommandJournal {
    pub fn defer(&mut self, id: &str, reason: &str, now: u64) -> Result<(), String> {
        if !matches!(reason, "active_sessions" | "activity_unknown") {
            return Err("command deferral reason invalid".into());
        }
        if self.state(id) == Some(ExecutionState::Deferred) { return Ok(()); }
        self.transition(id, &[ExecutionState::Accepted], ExecutionState::Deferred,
            "deferred", serde_json::json!({"error_code":reason}), now)
    }
    pub fn operation_snapshot(&self) -> serde_json::Value {
        for wanted in [ExecutionState::ReconciliationRequired, ExecutionState::Running, ExecutionState::Deferred] {
            if let Some((id, _)) = self.snapshot.commands.iter().find(|(_, receipt)| receipt.state == wanted) {
                return serde_json::json!({"state":wanted,"command_id":id,"transaction_id":null});
            }
        }
        serde_json::json!({"state":"idle","command_id":null,"transaction_id":null})
    }

    pub fn open(path: &Path) -> Result<Self, String> {
        let mut disk_digest = None;
        let snapshot = match std::fs::File::open(path) {
            Ok(file) => {
                use std::io::Read;
                let mut bytes = Vec::new();
                file.take(MAX_BYTES as u64 + 1)
                    .read_to_end(&mut bytes)
                    .map_err(|e| e.to_string())?;
                if bytes.len() > MAX_BYTES {
                    return Err("journal capacity exceeded".into());
                }
                disk_digest = Some(hex::encode(Sha256::digest(&bytes)));
                let snapshot: Snapshot =
                    serde_json::from_slice(&bytes).map_err(|_| "journal malformed".to_owned())?;
                if snapshot.schema != 1 || snapshot.events.len() > MAX_EVENTS {
                    return Err("journal schema or capacity invalid".into());
                }
                for (id, receipt) in &snapshot.commands {
                    if !token(id, 32) || !token(&receipt.digest, 64) {
                        return Err("journal command malformed".into());
                    }
                }
                for event in &snapshot.events {
                    let digest = hex::encode(Sha256::digest(
                        serde_json::to_vec(&event.payload).map_err(|e| e.to_string())?,
                    ));
                    if !token(&event.event_id, 32)
                        || event.payload["event_id"] != event.event_id
                        || digest != event.digest
                    {
                        return Err("journal event malformed".into());
                    }
                }
                snapshot
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Snapshot {
                schema: 1,
                commands: BTreeMap::new(),
                events: Vec::new(),
            },
            Err(error) => return Err(format!("journal unavailable: {error}")),
        };
        Ok(Self {
            path: path.into(),
            snapshot,
            disk_digest,
        })
    }

    pub fn state(&self, id: &str) -> Option<ExecutionState> {
        self.snapshot.commands.get(id).map(|receipt| receipt.state)
    }

    pub fn prune_confirmed(&mut self, now: u64) -> Result<usize, String> {
        let mut next = self.snapshot.clone();
        let pending = self
            .snapshot
            .events
            .iter()
            .filter_map(|event| event.payload["command_id"].as_str())
            .collect::<std::collections::BTreeSet<_>>();
        next.commands.retain(|id, receipt| {
            !matches!(
                receipt.state,
                ExecutionState::Succeeded
                    | ExecutionState::Failed
                    | ExecutionState::Expired
                    | ExecutionState::Unsupported
                    | ExecutionState::Rejected
                    | ExecutionState::RolledBack
                    | ExecutionState::RollbackFailed
                    | ExecutionState::Cancelled
            ) || pending.contains(id.as_str())
                || now.saturating_sub(receipt.updated_at) < 30 * 86400
        });
        let removed = self.snapshot.commands.len() - next.commands.len();
        if removed > 0 {
            self.commit(next)?;
        }
        Ok(removed)
    }

    fn commit(&mut self, next: Snapshot) -> Result<(), String> {
        if next.events.len() > MAX_EVENTS {
            return Err("outbox full".into());
        }
        let bytes = serde_json::to_vec(&next).map_err(|e| e.to_string())?;
        if bytes.len() > MAX_BYTES {
            return Err("outbox capacity exceeded".into());
        }
        #[cfg(windows)]
        let _writer = {
            use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
            let lock = self.path.with_extension("lock");
            if let Ok(metadata) = std::fs::symlink_metadata(&lock) {
                if !metadata.is_file() || metadata.file_attributes() & 0x400 != 0 {
                    return Err("journal writer lock path invalid".into());
                }
            }
            let writer = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .share_mode(0)
                .custom_flags(0x00200000)
                .open(&lock)
                .map_err(|error| format!("journal writer unavailable: {error}"))?;
            let current = match std::fs::read(&self.path) {
                Ok(bytes) => Some(hex::encode(Sha256::digest(bytes))),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                Err(error) => return Err(format!("journal preimage unavailable: {error}")),
            };
            if current != self.disk_digest {
                return Err("journal changed by another writer".into());
            }
            let staging = self.path.with_extension("pending");
            match std::fs::symlink_metadata(&staging) {
                Ok(metadata) => {
                    if !metadata.is_file() || metadata.file_attributes() & 0x400 != 0 {
                        return Err("journal staging path invalid".into());
                    }
                    std::fs::remove_file(staging).map_err(|error| {
                        format!("journal interrupted staging cleanup failed: {error}")
                    })?;
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(format!("journal staging inspection failed: {error}")),
            }
            writer
        };
        let temporary = self.path.with_extension("pending");
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|e| format!("journal staging unavailable: {e}"))?;
        let result = (|| {
            let current = match std::fs::read(&self.path) {
                Ok(bytes) => Some(hex::encode(Sha256::digest(bytes))),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                Err(error) => return Err(format!("journal preimage unavailable: {error}")),
            };
            if current != self.disk_digest {
                return Err("journal changed by another writer".into());
            }
            file.write_all(&bytes)
                .and_then(|_| file.sync_all())
                .map_err(|e| e.to_string())?;
            drop(file);
            replace(&temporary, &self.path)
        })();
        if let Err(error) = result {
            if let Err(cleanup) = std::fs::remove_file(&temporary) {
                return Err(format!("{error}; journal staging cleanup: {cleanup}"));
            }
            return Err(error);
        }
        self.snapshot = next;
        self.disk_digest = Some(hex::encode(Sha256::digest(bytes)));
        Ok(())
    }

    pub fn prepare(
        &mut self,
        id: &str,
        digest: &str,
        expires_at: u64,
        now: u64,
    ) -> Result<(), String> {
        if !token(id, 32) || !token(digest, 64) {
            return Err("command identity invalid".into());
        }
        if let Some(receipt) = self.snapshot.commands.get(id) {
            return if receipt.digest == digest {
                Ok(())
            } else {
                Err("command digest conflict".into())
            };
        }
        if expires_at <= now || expires_at.saturating_sub(now) > 86400 {
            return Err("command expired or lifetime invalid".into());
        }
        if self.snapshot.commands.len() >= 10_000 {
            return Err("command journal full".into());
        }
        if self.snapshot.events.len() > MAX_EVENTS - 3 {
            return Err("outbox lacks command receipt capacity".into());
        }
        let mut next = self.snapshot.clone();
        next.commands.insert(
            id.into(),
            Receipt {
                digest: digest.into(),
                expires_at,
                state: ExecutionState::ClaimPrepared,
                sequence: 0,
                updated_at: now,
            },
        );
        self.commit(next)
    }

    fn transition(
        &mut self,
        id: &str,
        expected: &[ExecutionState],
        state: ExecutionState,
        status: &str,
        result: serde_json::Value,
        now: u64,
    ) -> Result<(), String> {
        let mut next = self.snapshot.clone();
        let receipt = next
            .commands
            .get_mut(id)
            .ok_or_else(|| "command not journaled".to_owned())?;
        if !expected.contains(&receipt.state) {
            return Err("command transition refused".into());
        }
        receipt.sequence = receipt
            .sequence
            .checked_add(1)
            .ok_or_else(|| "event sequence overflow".to_owned())?;
        receipt.state = state;
        receipt.updated_at = now;
        let event_id = hex::encode(Sha256::digest(format!(
            "agent-event-v1:{id}:{}",
            receipt.sequence
        )))[..32]
            .to_owned();
        let payload = serde_json::json!({"schema":1,"event_id":event_id,"command_id":id,"sequence":receipt.sequence,"status":status,"result":result});
        let bytes = serde_json::to_vec(&payload).map_err(|e| e.to_string())?;
        if bytes.len() > 65536 {
            return Err("event too large".into());
        }
        next.events.push(PendingEvent {
            event_id,
            digest: hex::encode(Sha256::digest(bytes)),
            payload,
        });
        self.commit(next)
    }

    pub fn receipt(&mut self, id: &str) -> Result<(), String> {
        if matches!(self.state(id), Some(ExecutionState::Accepted | ExecutionState::Deferred)) {
            return Ok(());
        }
        self.transition(
            id,
            &[ExecutionState::ClaimPrepared],
            ExecutionState::Accepted,
            "receipt_persisted",
            serde_json::json!({}),
            0,
        )
    }

    pub fn start(&mut self, id: &str, now: u64) -> Result<(), String> {
        let receipt = self
            .snapshot
            .commands
            .get(id)
            .ok_or_else(|| "command not journaled".to_owned())?;
        if !matches!(receipt.state, ExecutionState::Accepted | ExecutionState::Deferred) {
            return Err("command not accepted or already started".into());
        }
        if now >= receipt.expires_at {
            self.transition(
                id,
                &[ExecutionState::Accepted, ExecutionState::Deferred],
                ExecutionState::Expired,
                "expired",
                serde_json::json!({"error_code":"expired"}),
                now,
            )?;
            return Err("command expired".into());
        }
        if self.snapshot.commands.iter().any(|(key, value)| {
            key != id
                && matches!(
                    value.state,
                    ExecutionState::Running | ExecutionState::ReconciliationRequired
                )
        }) {
            return Err("another operation requires reconciliation".into());
        }
        if self.snapshot.events.len() > MAX_EVENTS - 2 {
            return Err("outbox lacks terminal result capacity".into());
        }
        if serde_json::to_vec(&self.snapshot)
            .map_err(|error| error.to_string())?
            .len()
            > MAX_BYTES - 65536 - 2048
        {
            return Err("outbox lacks terminal result capacity".into());
        }
        self.transition(
            id,
            &[ExecutionState::Accepted, ExecutionState::Deferred],
            ExecutionState::Running,
            "running",
            serde_json::json!({}),
            now,
        )
    }

    pub fn finish(
        &mut self,
        id: &str,
        status: &str,
        result: serde_json::Value,
        now: u64,
    ) -> Result<(), String> {
        let state = match status {
            "succeeded" => ExecutionState::Succeeded,
            "failed" => ExecutionState::Failed,
            "rolled_back" => ExecutionState::RolledBack,
            "rollback_failed" => ExecutionState::RollbackFailed,
            "reconciliation_required" => ExecutionState::ReconciliationRequired,
            _ => return Err("command result invalid".into()),
        };
        self.transition(
            id,
            &[
                ExecutionState::Running,
                ExecutionState::ReconciliationRequired,
            ],
            state,
            status,
            result,
            now,
        )
    }

    pub fn pending_events(&self) -> &[PendingEvent] {
        &self.snapshot.events
    }

    pub fn abandon(&mut self, id: &str, status: &str, now: u64) -> Result<(), String> {
        let mut next = self.snapshot.clone();
        let receipt = next
            .commands
            .get_mut(id)
            .ok_or_else(|| "command not journaled".to_owned())?;
        if !matches!(
            receipt.state,
            ExecutionState::ClaimPrepared | ExecutionState::Accepted | ExecutionState::Deferred
        ) {
            return Err("started command cannot be abandoned".into());
        }
        receipt.state = match status {
            "cancelled" => ExecutionState::Cancelled,
            "expired" if now >= receipt.expires_at => ExecutionState::Expired,
            _ => return Err("command abandonment invalid".into()),
        };
        receipt.updated_at = now;
        self.commit(next)
    }

    pub fn reject(
        &mut self,
        id: &str,
        status: &str,
        result: serde_json::Value,
        now: u64,
    ) -> Result<(), String> {
        let state = match status {
            "unsupported" => ExecutionState::Unsupported,
            "rejected" => ExecutionState::Rejected,
            _ => return Err("command rejection invalid".into()),
        };
        self.transition(id, &[ExecutionState::Accepted, ExecutionState::Deferred], state, status, result, now)
    }

    pub fn acknowledge(&mut self, id: &str, digest: &str) -> Result<(), String> {
        let Some(event) = self
            .snapshot
            .events
            .iter()
            .find(|event| event.event_id == id)
        else {
            return Ok(());
        };
        if event.digest != digest {
            return Err("event acknowledgement conflict".into());
        }
        let mut next = self.snapshot.clone();
        next.events.retain(|event| event.event_id != id);
        self.commit(next)
    }
}

#[cfg(windows)]
fn replace(source: &Path, destination: &Path) -> Result<(), String> {
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
            "journal publish failed: {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(())
}

#[cfg(not(windows))]
fn replace(source: &Path, destination: &Path) -> Result<(), String> {
    std::fs::rename(source, destination).map_err(|e| e.to_string())?;
    let parent = destination
        .parent()
        .ok_or_else(|| "journal parent unavailable".to_owned())?;
    std::fs::File::open(parent)
        .and_then(|file| file.sync_all())
        .map_err(|e| e.to_string())
}
