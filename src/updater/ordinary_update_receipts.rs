use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

#[path = "ordinary_update_receipts/admission.rs"]
pub(crate) mod admission;
#[path = "ordinary_update_receipts/package_contract.rs"]
pub(crate) mod package_contract;
#[path = "ordinary_update_receipts/package_name.rs"]
pub(crate) mod package_name;
#[path = "ordinary_update_receipts/supervisor.rs"]
pub(crate) mod supervisor;
#[path = "../management_agent/workspace.rs"]
mod workspace;

const ROOT: &str = r"C:\ProgramData\RustDesk\ordinary-updates";
#[cfg(not(t02_receipts))]
#[path = "ordinary_update_receipts/runtime.rs"]
pub(crate) mod runtime;
const MAX_BYTES: u64 = 65536;
static STAGING: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PreparationError {
    pub stage: String,
    pub kind: String,
    pub os_code: Option<i32>,
    pub code: String,
}

impl PreparationError {
    pub(crate) fn io(stage: &str, error: std::io::Error) -> Self {
        let kind = match error.raw_os_error() {
            Some(32 | 33) => "sharing_violation",
            _ => match error.kind() {
                std::io::ErrorKind::AlreadyExists => "already_exists",
                std::io::ErrorKind::PermissionDenied => "permission_denied",
                _ => "io_failure",
            },
        };
        Self {
            stage: stage.into(),
            kind: kind.into(),
            os_code: error.raw_os_error(),
            code: format!("preparation_{stage}_{kind}"),
        }
    }
    pub(crate) fn refused(stage: &str) -> Self {
        Self {
            stage: stage.into(),
            kind: "validation_failed".into(),
            os_code: None,
            code: format!("preparation_{stage}_validation_failed"),
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PreparationFailure {
    phase: String,
    error: PreparationError,
}

/// Fixed-object writer request; never accepts a filesystem path or terminal status.
pub(crate) struct OutcomeWriteRequest {
    transaction_id: String,
    intent_sha256: String,
    exit_code: u32,
}

impl OutcomeWriteRequest {
    pub(crate) fn parse(args: &[String]) -> Result<Self, String> {
        let hex = |value: &str, length| {
            value.len() == length
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        };
        if args.len() != 3 || !hex(&args[0], 32) || !hex(&args[1], 64) {
            return Err("ordinary outcome writer arguments invalid".into());
        }
        let exit_code: u32 = args[2]
            .parse()
            .map_err(|_| "ordinary outcome exit invalid")?;
        if exit_code.to_string() != args[2] {
            return Err("ordinary outcome exit invalid".into());
        }
        Ok(Self {
            transaction_id: args[0].clone(),
            intent_sha256: args[1].clone(),
            exit_code,
        })
    }

    pub(crate) fn write(&self) -> Result<(), String> {
        self.write_at(Path::new(ROOT))
    }

    #[cfg(test)]
    pub(crate) fn fixture_write(&self, root: &Path) -> Result<(), String> {
        self.write_at(root)
    }

    fn write_at(&self, root: &Path) -> Result<(), String> {
        workspace::verify(root)?;
        let directory = root.join(&self.transaction_id);
        workspace::verify(&directory)?;
        let path = directory.join("intent.json");
        workspace::verify_file(&path)?;
        use std::os::windows::fs::OpenOptionsExt;
        let mut bytes = Vec::new();
        OpenOptions::new()
            .read(true)
            .share_mode(1)
            .custom_flags(0x00200000)
            .open(path)
            .map_err(|error| error.to_string())?
            .take(MAX_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| error.to_string())?;
        if bytes.len() as u64 > MAX_BYTES {
            return Err("ordinary receipt too large".into());
        }
        if hex::encode(Sha256::digest(&bytes)) != self.intent_sha256 {
            return Err("ordinary intent digest mismatch".into());
        }
        let value: serde_json::Value =
            serde_json::from_slice(&bytes).map_err(|_| "ordinary receipt malformed")?;
        let schema = value["schema"]
            .as_u64()
            .ok_or("ordinary receipt malformed")?;
        let (binding, approved_package_name) = if schema == 2 {
            let record: IntentRecord =
                serde_json::from_value(value).map_err(|_| "ordinary receipt malformed")?;
            (
                record.binding,
                Some(record.intent_binding.approved_package_name),
            )
        } else {
            let record: Record<()> =
                serde_json::from_value(value).map_err(|_| "ordinary receipt malformed")?;
            (record.binding, None)
        };
        binding.validate()?;
        if !matches!(schema, 1 | 2) || binding.transaction_id != self.transaction_id {
            return Err("ordinary receipt binding mismatch".into());
        }
        let outbox = Outbox {
            directory,
            binding,
            approved_package_name,
            schema: schema as u32,
        };
        let outcome = if matches!(self.exit_code, 0 | 3010) {
            InstallerOutcome::Succeeded {
                exit_code: self.exit_code,
            }
        } else {
            InstallerOutcome::Failed {
                exit_code: self.exit_code,
                error_code: format!("msi_exit_{}", self.exit_code),
            }
        };
        outbox.record_outcome(outcome)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Binding {
    pub transaction_id: String,
    pub client_id: String,
    pub client_uuid: String,
    pub installation_id: String,
    pub target_key: String,
    pub package_sha256: String,
    pub from_version: String,
    pub from_build_seq: u64,
    pub to_version: String,
    pub to_build_seq: u64,
    pub source: String,
    pub command_id: Option<String>,
}

impl From<package_contract::BindingRecord> for Binding {
    fn from(value: package_contract::BindingRecord) -> Self {
        Self {
            transaction_id: value.transaction_id,
            client_id: value.client_id,
            client_uuid: value.client_uuid,
            installation_id: value.installation_id,
            target_key: value.target_key,
            package_sha256: value.package_sha256,
            from_version: value.from_version,
            from_build_seq: value.from_build_seq,
            to_version: value.to_version,
            to_build_seq: value.to_build_seq,
            source: value.source,
            command_id: value.command_id,
        }
    }
}

impl Binding {
    fn validate(&self) -> Result<(), String> {
        let hex = |value: &str, length| {
            value.len() == length
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        };
        if !hex(&self.transaction_id, 32)
            || !hex(&self.package_sha256, 64)
            || [
                &self.client_id,
                &self.client_uuid,
                &self.installation_id,
                &self.from_version,
                &self.to_version,
            ]
            .iter()
            .any(|value| value.is_empty() || value.len() > 256)
            || !matches!(
                self.target_key.as_str(),
                "windows-x86_64-msi-standard" | "windows-x86_64-msi-sos"
            )
            || !matches!(self.source.as_str(), "primary" | "mirror")
            || self.from_build_seq == 0
            || self.to_build_seq == 0
            || self
                .command_id
                .as_ref()
                .is_some_and(|id| id.is_empty() || id.len() > 256)
        {
            return Err("ordinary receipt binding invalid".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record<T> {
    schema: u32,
    binding: Binding,
    value: T,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct IntentRecord {
    schema: u32,
    binding: Binding,
    intent_binding: IntentBinding,
    value: (),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct IntentBinding {
    approved_package_name: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum InstallerOutcome {
    Succeeded { exit_code: u32 },
    Failed { exit_code: u32, error_code: String },
}

/// Supplied only after the caller has verified the fixed installed image and
/// live health response. A build/status heartbeat alone is not this evidence.
pub(crate) struct VerifiedRunningTarget {
    pub binding: Binding,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Terminal {
    pub status: String,
    pub error_code: String,
}

#[derive(Clone, Copy)]
pub(crate) enum Channel {
    Update,
    Command,
}

pub(crate) struct Outbox {
    directory: PathBuf,
    binding: Binding,
    approved_package_name: Option<String>,
    schema: u32,
}

impl Outbox {
    fn preparation_undispatched(&self) -> Result<bool, String> {
        for name in [
            "dispatch.json",
            "supervisor-release.json",
            "outcome.json",
            "terminal.json",
        ] {
            match fs::symlink_metadata(self.directory.join(name)) {
                Ok(_) => return Ok(false),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => return Err("ordinary preparation state unavailable".into()),
            }
        }
        Ok(true)
    }

    pub(crate) fn preparation_failed(&self, error: PreparationError) -> Result<(), String> {
        self.intent()?;
        if !self.preparation_undispatched()? {
            return Err("ordinary preparation failure after dispatch refused".into());
        }
        self.write_once(
            "preparation-failed.json",
            PreparationFailure {
                phase: "preparation_failed".into(),
                error,
            },
        )
        .map_err(|_| "ordinary preparation failure persistence unconfirmed".to_owned())
    }

    pub(crate) fn preparation_retryable(&self) -> Result<bool, String> {
        if !self.preparation_undispatched()? {
            return Ok(false);
        }
        match fs::symlink_metadata(self.directory.join("preparation-failed.json")) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(_) => return Err("ordinary preparation failure unavailable".into()),
            _ => {}
        }
        let failure: PreparationFailure = self.read("preparation-failed.json")?;
        let e = &failure.error;
        if failure.phase != "preparation_failed"
            || ![
                "create_runtime",
                "copy_writer",
                "copy_dependency",
                "copy_package",
                "seal_runtime",
            ]
            .contains(&e.stage.as_str())
            || ![
                "already_exists",
                "permission_denied",
                "sharing_violation",
                "io_failure",
                "validation_failed",
            ]
            .contains(&e.kind.as_str())
            || e.code != format!("preparation_{}_{}", e.stage, e.kind)
        {
            return Err("ordinary preparation failure invalid".into());
        }
        Ok(true)
    }

    pub(crate) fn blocks_preparation_retry(&self) -> bool {
        !self.preparation_retryable().unwrap_or(false)
    }
    pub(crate) fn binding(&self) -> &Binding {
        &self.binding
    }

    pub(crate) fn open_existing(transaction_id: &str) -> Result<Self, String> {
        Self::open_at(Path::new(ROOT), transaction_id)
    }

    fn open_at(root: &Path, transaction_id: &str) -> Result<Self, String> {
        if transaction_id.len() != 32
            || !transaction_id
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err("ordinary transaction id invalid".into());
        }
        workspace::verify(root)?;
        let directory = root.join(transaction_id);
        workspace::verify(&directory)?;
        let path = directory.join("intent.json");
        workspace::verify_file(&path)?;
        let mut bytes = Vec::new();
        fs::File::open(path)
            .map_err(|e| e.to_string())?
            .take(MAX_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        if bytes.len() as u64 > MAX_BYTES {
            return Err("ordinary receipt too large".into());
        }
        let value: serde_json::Value =
            serde_json::from_slice(&bytes).map_err(|_| "ordinary receipt malformed")?;
        let schema = value["schema"]
            .as_u64()
            .ok_or("ordinary receipt malformed")?;
        let (binding, approved_package_name) = if schema == 2 {
            let record: IntentRecord =
                serde_json::from_value(value).map_err(|_| "ordinary receipt malformed")?;
            package_name::validate(&record.intent_binding.approved_package_name)?;
            (
                record.binding,
                Some(record.intent_binding.approved_package_name),
            )
        } else {
            let record: Record<()> =
                serde_json::from_value(value).map_err(|_| "ordinary receipt malformed")?;
            (record.binding, None)
        };
        binding.validate()?;
        if !matches!(schema, 1 | 2) || binding.transaction_id != transaction_id {
            return Err("ordinary receipt binding mismatch".into());
        }
        Ok(Self {
            directory,
            binding,
            approved_package_name,
            schema: schema as u32,
        })
    }

    pub(crate) fn intent_digest(&self) -> Result<String, String> {
        self.intent()?;
        Ok(hex::encode(Sha256::digest(
            fs::read(self.directory.join("intent.json")).map_err(|e| e.to_string())?,
        )))
    }

    pub(crate) fn package_name(&self) -> Result<String, String> {
        if self.schema == 2 {
            return package_name::validate(
                self.approved_package_name
                    .as_deref()
                    .ok_or("ordinary package name binding missing")?,
            );
        }
        if self.schema != 1 {
            return Err("ordinary legacy package name compatibility refused".into());
        }
        match self.read::<String>("package-name.json") {
            Ok(name) => package_name::validate(&name),
            Err(error) if error == "ordinary receipt missing: package-name.json" => {
                if legacy_msi_installation_evidence(&self.binding)? {
                    package_contract::receipt_package_name(None)
                } else {
                    Err("ordinary legacy package name compatibility refused".into())
                }
            }
            Err(error) => Err(error),
        }
    }

    pub(crate) fn dispatch(
        &self,
        run: impl FnOnce(&str, &str) -> Result<(), String>,
    ) -> Result<(), String> {
        if self.preparation_retryable()? {
            return Err("ordinary failed preparation requires new attempt".into());
        }
        let digest = self.intent_digest()?;
        let _lease = supervisor::acquire_ready(self)?;
        // A second caller must not launch an installer whose first launch is unresolved.
        let marker = self.directory.join("dispatch.json");
        let mut marker = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(marker)
            .map_err(|_| "ordinary installer already dispatched or marker unavailable")?;
        marker
            .write_all(digest.as_bytes())
            .map_err(|e| e.to_string())?;
        marker.sync_all().map_err(|e| e.to_string())?;
        run(&self.binding.transaction_id, &digest)
    }

    pub(crate) fn report_pending(
        &self,
        mut send: impl FnMut(Channel, &Binding, &Terminal) -> bool,
    ) -> Result<(), String> {
        for channel in [Channel::Update, Channel::Command] {
            if matches!(channel, Channel::Command) && self.binding.command_id.is_none() {
                continue;
            }
            if let Some((terminal, digest)) = self.pending(channel)? {
                if send(channel, &self.binding, &terminal) {
                    self.acknowledge(channel, &digest)?;
                }
            }
        }
        Ok(())
    }

    /// The caller must persist this before allowing the installer to start.
    pub(crate) fn prepare(binding: Binding, approved_package_name: &str) -> Result<Self, String> {
        Self::prepare_at(Path::new(ROOT), binding, approved_package_name)
    }

    fn prepare_at(root: &Path, binding: Binding, package_name: &str) -> Result<Self, String> {
        binding.validate()?;
        let approved_package_name = package_name::validate(package_name)?;
        validate_windows_package_basename(&approved_package_name)?;
        ensure_directory(root)?;
        let directory = root.join(&binding.transaction_id);
        ensure_directory(&directory)?;
        let outbox = Self {
            directory,
            binding,
            approved_package_name: Some(approved_package_name),
            schema: 2,
        };
        outbox.write_intent()?;
        Ok(outbox)
    }

    #[cfg(test)]
    pub(crate) fn fixture(root: &Path, binding: Binding) -> Result<Self, String> {
        Self::prepare_at(root, binding, "package.msi")
    }

    #[cfg(test)]
    pub(crate) fn fixture_named(
        root: &Path,
        binding: Binding,
        package_name: &str,
    ) -> Result<Self, String> {
        Self::prepare_at(root, binding, package_name)
    }

    #[cfg(test)]
    pub(crate) fn fixture_open(root: &Path, id: &str) -> Result<Self, String> {
        Self::open_at(root, id)
    }

    /// Interface for the independent installer result writer. No executor is
    /// launched here; absence of this record must remain unresolved on restart.
    pub(crate) fn record_outcome(&self, outcome: InstallerOutcome) -> Result<(), String> {
        match &outcome {
            InstallerOutcome::Succeeded { exit_code } if !matches!(exit_code, 0 | 3010) => {
                return Err("ordinary installer success exit invalid".into())
            }
            InstallerOutcome::Failed {
                exit_code,
                error_code,
            } if matches!(exit_code, 0 | 3010)
                || error_code.is_empty()
                || error_code.len() > 128 =>
            {
                return Err("ordinary installer failure invalid".into())
            }
            _ => {}
        }
        self.intent()?;
        self.write_once("outcome.json", outcome)
    }

    pub(crate) fn reconcile(
        &self,
        health: Option<&VerifiedRunningTarget>,
    ) -> Result<Terminal, String> {
        self.intent()?;
        let outcome: InstallerOutcome = self.read("outcome.json")?;
        let terminal = match outcome {
            InstallerOutcome::Succeeded {
                exit_code: 0 | 3010,
            } => {
                if !health.is_some_and(|health| health.binding == self.binding) {
                    return Err("ordinary running target unconfirmed".into());
                }
                Terminal {
                    status: "installed".into(),
                    error_code: String::new(),
                }
            }
            InstallerOutcome::Failed {
                exit_code,
                error_code,
            } if !matches!(exit_code, 0 | 3010)
                && !error_code.is_empty()
                && error_code.len() <= 128 =>
            {
                Terminal {
                    status: "failed".into(),
                    error_code,
                }
            }
            _ => return Err("ordinary installer outcome invalid".into()),
        };
        self.write_once("terminal.json", &terminal)?;
        Ok(terminal)
    }

    pub(crate) fn pending(&self, channel: Channel) -> Result<Option<(Terminal, String)>, String> {
        self.intent()?;
        let terminal: Terminal = self.read("terminal.json")?;
        let digest = self.terminal_digest(&terminal)?;
        match self.read::<String>(ack_name(channel)) {
            Ok(ack) if ack == digest => Ok(None),
            Ok(_) => Err("ordinary terminal acknowledgement mismatch".into()),
            Err(error)
                if !self.directory.join(ack_name(channel)).exists()
                    && error.starts_with("ordinary receipt missing:") =>
            {
                Ok(Some((terminal, digest)))
            }
            Err(error) => Err(error),
        }
    }

    pub(crate) fn acknowledge(&self, channel: Channel, digest: &str) -> Result<(), String> {
        if matches!(channel, Channel::Command) && self.binding.command_id.is_none() {
            return Err("ordinary command acknowledgement without command".into());
        }
        self.intent()?;
        let terminal: Terminal = self.read("terminal.json")?;
        if digest != self.terminal_digest(&terminal)? {
            return Err("ordinary terminal acknowledgement mismatch".into());
        }
        self.write_once(ack_name(channel), digest)
    }

    fn terminal_digest(&self, terminal: &Terminal) -> Result<String, String> {
        if self.schema == 2 {
            let bytes = serde_json::to_vec(&Record {
                schema: 2,
                binding: self.binding.clone(),
                value: (self.intent_digest()?, terminal),
            })
            .map_err(|error| error.to_string())?;
            return Ok(hex::encode(Sha256::digest(bytes)));
        }
        let bytes = serde_json::to_vec(&Record {
            schema: 1,
            binding: self.binding.clone(),
            value: terminal,
        })
        .map_err(|error| error.to_string())?;
        Ok(hex::encode(Sha256::digest(bytes)))
    }

    fn intent(&self) -> Result<(), String> {
        let path = self.directory.join("intent.json");
        workspace::verify_file(&path)?;
        let bytes = read_raw_file(&path)?;
        let value: serde_json::Value =
            serde_json::from_slice(&bytes).map_err(|_| "ordinary receipt malformed")?;
        let schema = value["schema"]
            .as_u64()
            .ok_or("ordinary receipt malformed")?;
        let approved = if schema == 2 {
            let record: IntentRecord =
                serde_json::from_value(value).map_err(|_| "ordinary receipt malformed")?;
            if record.binding != self.binding {
                return Err("ordinary receipt binding mismatch".into());
            }
            Some(record.intent_binding.approved_package_name)
        } else {
            let _: Record<()> =
                serde_json::from_value(value).map_err(|_| "ordinary receipt malformed")?;
            None
        };
        if schema != self.schema as u64 || approved != self.approved_package_name {
            return Err("ordinary package name binding mismatch".into());
        }
        Ok(())
    }

    fn write_intent(&self) -> Result<(), String> {
        workspace::verify(&self.directory)?;
        let bytes = serde_json::to_vec(&IntentRecord {
            schema: 2,
            binding: self.binding.clone(),
            intent_binding: IntentBinding {
                approved_package_name: self
                    .approved_package_name
                    .clone()
                    .ok_or("ordinary package name binding missing")?,
            },
            value: (),
        })
        .map_err(|error| error.to_string())?;
        self.write_bytes_once("intent.json", bytes)
    }

    fn read<T: serde::de::DeserializeOwned>(&self, name: &str) -> Result<T, String> {
        workspace::verify(&self.directory)?;
        let path = self.directory.join(name);
        match fs::symlink_metadata(&path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Err(format!("ordinary receipt missing: {name}"))
            }
            Err(error) => return Err(error.to_string()),
            Ok(_) => {}
        }
        workspace::verify_file(&path)?;
        let mut bytes = Vec::new();
        use std::os::windows::fs::OpenOptionsExt;
        OpenOptions::new()
            .read(true)
            .share_mode(1)
            .custom_flags(0x00200000)
            .open(&path)
            .map_err(|error| error.to_string())?
            .take(MAX_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| error.to_string())?;
        if bytes.len() as u64 > MAX_BYTES {
            return Err("ordinary receipt too large".into());
        }
        let record: Record<T> =
            serde_json::from_slice(&bytes).map_err(|_| "ordinary receipt malformed".to_owned())?;
        if name == "intent.json" {
            return Err("ordinary intent record invalid".into());
        }
        if record.schema != 1 || record.binding != self.binding {
            return Err("ordinary receipt binding mismatch".into());
        }
        Ok(record.value)
    }

    fn write_once<T: Serialize>(&self, name: &str, value: T) -> Result<(), String> {
        workspace::verify(&self.directory)?;
        let bytes = serde_json::to_vec(&Record {
            schema: 1,
            binding: self.binding.clone(),
            value,
        })
        .map_err(|error| error.to_string())?;
        self.write_bytes_once(name, bytes)
    }

    fn write_bytes_once(&self, name: &str, bytes: Vec<u8>) -> Result<(), String> {
        if bytes.len() as u64 > MAX_BYTES {
            return Err("ordinary receipt too large".into());
        }
        let destination = self.directory.join(name);
        if destination.exists() {
            workspace::verify_file(&destination)?;
            return if fs::read(&destination).map_err(|error| error.to_string())? == bytes {
                Ok(())
            } else {
                Err("ordinary receipt immutable conflict".into())
            };
        }
        let staging = self.directory.join(format!(
            "staging-{}-{}",
            std::process::id(),
            STAGING.fetch_add(1, Ordering::SeqCst)
        ));
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&staging)
            .map_err(|error| error.to_string())?;
        let result = (|| {
            file.write_all(&bytes).map_err(|error| error.to_string())?;
            file.sync_all().map_err(|error| error.to_string())?;
            drop(file);
            workspace::verify_file(&staging)?;
            #[cfg(test)]
            publication_barrier_fixture(name)?;
            use std::os::windows::ffi::OsStrExt;
            #[link(name = "kernel32")]
            extern "system" {
                fn MoveFileExW(source: *const u16, destination: *const u16, flags: u32) -> i32;
            }
            let source: Vec<u16> = staging.as_os_str().encode_wide().chain(Some(0)).collect();
            let target: Vec<u16> = destination
                .as_os_str()
                .encode_wide()
                .chain(Some(0))
                .collect();
            // WRITE_THROUGH, without REPLACE_EXISTING: immutable, flushed publication.
            if unsafe { MoveFileExW(source.as_ptr(), target.as_ptr(), 8) } == 0 {
                let error = std::io::Error::last_os_error();
                if matches!(error.raw_os_error(), Some(80 | 183)) {
                    let existing = read_raw_file(&destination)?;
                    if existing == bytes {
                        fs::remove_file(&staging).map_err(|cleanup| {
                            format!("ordinary staging cleanup failed: {cleanup}")
                        })?;
                        return Ok(());
                    }
                    return Err("ordinary receipt immutable conflict".into());
                }
                return Err(error.to_string());
            }
            Ok(())
        })();
        if result.is_err() {
            fs::remove_file(&staging)
                .map_err(|error| format!("ordinary staging cleanup failed: {error}"))?;
        }
        result
    }
}

#[cfg(test)]
fn publication_barrier_fixture(name: &str) -> Result<(), String> {
    if name != "outcome.json" {
        return Ok(());
    }
    let Some(root) = std::env::var_os("T02_RECEIPTS_ROOT") else {
        return Ok(());
    };
    let Some(role) = std::env::var_os("T02_WRITER_ROLE") else {
        return Ok(());
    };
    let root = PathBuf::from(root);
    let role = role.to_string_lossy();
    fs::write(root.join(format!("writer-{role}-ready")), b"ready")
        .map_err(|error| error.to_string())?;
    while !root.join(format!("writer-release-{role}")).exists() {
        std::thread::yield_now();
    }
    Ok(())
}

fn read_raw_file(path: &Path) -> Result<Vec<u8>, String> {
    workspace::verify_file(path)?;
    let mut bytes = Vec::new();
    use std::os::windows::fs::OpenOptionsExt;
    OpenOptions::new()
        .read(true)
        .share_mode(1)
        .custom_flags(0x00200000)
        .open(path)
        .map_err(|error| error.to_string())?
        .take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err("ordinary receipt too large".into());
    }
    Ok(bytes)
}

fn ack_name(channel: Channel) -> &'static str {
    match channel {
        Channel::Update => "ack-update.json",
        Channel::Command => "ack-command.json",
    }
}

fn ensure_directory(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(_) => workspace::verify(path),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => workspace::create(path),
        Err(error) => Err(error.to_string()),
    }
}

fn legacy_msi_installation_evidence(binding: &Binding) -> Result<bool, String> {
    if !matches!(
        binding.target_key.as_str(),
        "windows-x86_64-msi-standard" | "windows-x86_64-msi-sos"
    ) {
        return Ok(false);
    }
    let path = Path::new(r"C:\ProgramData\RustDesk\ManagementAgent\state\agent.json");
    workspace::verify_restore_target(path, false)?;
    let mut bytes = Vec::new();
    use std::os::windows::fs::OpenOptionsExt;
    OpenOptions::new()
        .read(true)
        .share_mode(1)
        .custom_flags(0x00200000)
        .open(path)
        .map_err(|error| error.to_string())?
        .take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err("ordinary installation state too large".into());
    }
    let state: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|_| "ordinary installation state malformed")?;
    Ok(state["schema"] == 1
        && state["agent_installation_id"] == binding.installation_id
        && state["edition"]
            == if binding.target_key.ends_with("-sos") {
                "sos"
            } else {
                "standard"
            }
        && state["package_kind"] == "msi"
        && state["install_version"]
            .as_str()
            .is_some_and(|version| !version.is_empty()))
}

pub(super) fn validate_windows_package_basename(name: &str) -> Result<(), String> {
    let stem = name
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    if ["CON", "PRN", "AUX", "NUL"].contains(&stem.as_str())
        || ((stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.len() == 4
            && matches!(stem.as_bytes()[3], b'1'..=b'9'))
    {
        return Err("ordinary package basename invalid".into());
    }
    Ok(())
}
