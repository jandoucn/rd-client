//! Windows management-agent policy and recovery primitives.
//!
//! The state machine and manifest handling are platform independent so they can be
//! tested on CI hosts which are not Windows. Windows service plumbing belongs in
//! the binary entry point and is deliberately kept behind `cfg(windows)`.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::time::Duration;

#[cfg(all(test, windows))]
#[path = "../tests/management_agent_local_events/r4_poll.rs"]
mod r4_poll_tests;

use hbb_common::base64::{
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
    Engine as _,
};

#[cfg(windows)]
#[path = "platform/windows_system_dirs.rs"]
pub(crate) mod system_dirs;
#[cfg(windows)]
#[path = "management_agent/identity.rs"]
mod identity;
#[path = "management_agent/recovery.rs"]
mod recovery;
#[cfg(windows)]
#[path = "management_agent/windows.rs"]
mod recovery_windows;
#[cfg(windows)]
#[path = "management_agent/status.rs"]
mod status;
#[path = "management_agent/status_generation.rs"]
mod status_generation;
#[path = "management_agent/bootstrap_policy.rs"]
mod bootstrap_policy;
#[cfg(windows)]
#[path = "management_agent/diagnostic_logs.rs"]
mod diagnostic_logs;
#[cfg(windows)]
#[path = "management_agent/bootstrap.rs"]
pub mod bootstrap;
#[cfg(windows)]
#[path = "management_agent/journal.rs"]
mod journal;
#[cfg(windows)]
#[path = "management_agent/local_events.rs"]
mod local_events;
#[cfg(windows)]
#[path = "management_agent/local_event_runtime.rs"]
mod local_event_runtime;
#[cfg(windows)]
#[path = "management_agent/local_recovery_events.rs"]
mod local_recovery_events;
#[path = "uac_policy.rs"]
mod uac_policy;
#[path = "management_agent/script_run.rs"]
pub(crate) mod script_run;
#[cfg(windows)]
#[path = "management_agent/terminal/mod.rs"]
pub(crate) mod terminal;
#[cfg(windows)]
#[path = "management_agent/operations.rs"]
mod operations;
#[cfg(windows)]
#[path = "management_agent/operations_runtime.rs"]
mod operations_runtime;
#[cfg(windows)]
#[path = "management_agent/network_policy.rs"]
mod network_policy;
#[cfg(windows)]
#[path = "management_agent/state_store.rs"]
mod state_store;
#[cfg(windows)]
#[path = "management_agent/health.rs"]
mod health;
#[cfg(windows)]
#[path = "management_agent/health_windows.rs"]
mod health_windows;
#[cfg(windows)]
#[path = "management_agent/registration_reconnect.rs"]
mod registration_reconnect;
#[cfg(windows)]
#[path = "management_agent/handoff.rs"]
mod handoff;
#[cfg(windows)]
#[path = "management_agent/package_gate.rs"]
mod package_gate;
#[cfg(windows)]
#[path = "management_agent/first_install_approval_delivery.rs"]
mod first_install_approval_delivery;
#[cfg(windows)]
#[path = "management_agent/workspace.rs"]
mod workspace;
#[cfg(windows)]
#[path = "management_agent/integrity.rs"]
mod integrity;
#[cfg(windows)]
#[path = "management_agent/network_probe.rs"]
mod network_probe;
#[cfg(windows)]
use recovery_windows::install_guard;
#[cfg(windows)]
#[path = "management_agent/install_plan.rs"]
mod install_plan;
#[cfg(windows)]
#[path = "management_agent/transaction.rs"]
mod transaction;
#[cfg(windows)]
#[path = "management_agent/install_execution.rs"]
mod install_execution;
#[cfg(windows)]
#[path = "management_agent/installer_process.rs"]
mod installer_process;
#[cfg(windows)]
#[path = "management_agent/directory_backup.rs"]
mod directory_backup;
#[cfg(windows)]
#[path = "management_agent/file_restore.rs"]
mod file_restore;
#[cfg(windows)]
#[path = "management_agent/native_install.rs"]
mod native_install;
#[cfg(windows)]
#[path = "management_agent/msi_metadata.rs"]
mod msi_metadata;
#[cfg(windows)]
#[path = "management_agent/purge_policy.rs"]
mod purge_policy;
#[cfg(windows)]
#[path = "management_agent/purge_discovery.rs"]
mod purge_discovery;
#[cfg(windows)]
#[path = "management_agent/purge_approval.rs"]
mod purge_approval;
#[cfg(windows)]
#[path = "management_agent/purge_preflight.rs"]
mod purge_preflight;
#[cfg(windows)]
#[path = "management_agent/purge_marker.rs"]
mod purge_marker;
#[cfg(windows)]
#[path = "management_agent/purge_service.rs"]
mod purge_service;
#[cfg(windows)]
#[path = "management_agent/purge_execution.rs"]
mod purge_execution;
#[cfg(windows)]
#[path = "management_agent/purge_handoff.rs"]
mod purge_handoff;
#[cfg(all(windows, not(test)))]
#[path = "management_agent/purge_native.rs"]
mod purge_native;

#[cfg(all(windows, not(test)))]
pub fn run_purge_helper(id: &str) -> Result<(), String> { purge_native::run_helper(id) }
#[cfg(all(windows, not(test)))]
pub fn inspect_purge_identity() -> Result<String, String> {
    let approved = purge_discovery::approved_installed()?;
    purge_discovery::verified_native(&approved).map(|probe| format!("{probe:?}"))
}
#[cfg(windows)]
#[path = "management_agent/msi_assets.rs"]
mod msi_assets;
#[cfg(windows)]
#[path = "management_agent/preparation.rs"]
mod preparation;
#[cfg(windows)]
#[path = "management_agent/exe_assets.rs"]
mod exe_assets;
#[cfg(windows)]
#[path = "management_agent/exe_preparation.rs"]
mod exe_preparation;
#[cfg(windows)]
#[path = "management_agent/exe_install.rs"]
mod exe_install;
#[cfg(windows)]
#[path = "management_agent/exe_service.rs"]
mod exe_service;
#[cfg(windows)]
#[path = "management_agent/exe_runtime.rs"]
mod exe_runtime;
#[cfg(windows)]
pub fn run_exe_transaction(arguments:&[std::ffi::OsString],mode:&str)->Result<(),String>{exe_runtime::dispatch(arguments,mode)}
#[cfg(windows)]
#[path = "management_agent/service_control.rs"]
mod service_control;
#[cfg(windows)]
#[path = "management_agent/maintenance.rs"]
mod maintenance;
#[cfg(windows)]
#[path = "management_agent/resume_task.rs"]
mod resume_task;
#[cfg(windows)]
#[path = "management_agent/helper_runtime.rs"]
mod helper_runtime;
#[cfg(windows)]
#[path = "management_agent/transaction_entry.rs"]
mod transaction_entry;
#[cfg(windows)]
#[path = "management_agent/command_transaction.rs"]
mod command_transaction;
#[cfg(windows)]
#[path = "management_agent/parent_handoff.rs"]
mod parent_handoff;
#[cfg(windows)]
#[path = "management_agent/bootstrap_update.rs"]
mod bootstrap_update;
#[cfg(windows)]
#[path = "management_agent/package_download.rs"]
mod package_download;
#[cfg(windows)]
#[path = "management_agent/rescue_assets.rs"]
mod rescue_assets;
#[cfg(windows)]
#[path = "management_agent/agent_restore.rs"]
mod agent_restore;
#[cfg(windows)]
#[path = "management_agent/rescue_status.rs"]
mod rescue_status;
#[cfg(windows)]
#[path = "management_agent/rescue_recovery.rs"]
mod rescue_recovery;
#[cfg(windows)]
#[path = "management_agent/local_repair_plan.rs"]
mod local_repair_plan;
#[cfg(windows)]
#[path = "management_agent/local_repair.rs"]
mod local_repair;
#[cfg(windows)]
#[path = "management_agent/local_repair_schedule.rs"]
mod local_repair_schedule;

#[cfg(windows)]
pub fn run_local_repair(arguments: &[std::ffi::OsString], mode: &str) -> Result<(), String> {
    if !unsafe { windows::Win32::UI::Shell::IsUserAnAdmin().as_bool() } {
        return Err("local repair requires administrator".into());
    }
    if arguments.len() != 2 || arguments[0] != "--transaction-id" {
        return Err("local repair accepts only fixed transaction ID".into());
    }
    let id = arguments[1]
        .to_str()
        .ok_or("local repair ID encoding invalid")?;
    match mode {
        "local-repair-prepare" => {
            let root = local_repair::prepare(id)?;
            local_repair::launch(&root)?;
            Ok(())
        }
        "local-repair-helper" => local_repair::run_helper(id, false),
        "local-repair-resume" => local_repair::run_helper(id, true),
        _ => Err("local repair command invalid".into()),
    }
}

#[cfg(windows)]
pub fn run_transaction_helper(arguments: &[std::ffi::OsString]) -> Result<(), String> {
    if !unsafe { windows::Win32::UI::Shell::IsUserAnAdmin().as_bool() } {
        return Err("transaction helper requires administrator privileges".into());
    }
    let program_data = system_dirs::get().program_data.clone();
    let root = program_data.join("RustDesk").join("ManagementAgent").join("transactions");
    let arguments = arguments.iter().map(|value| value.to_str().map(str::to_owned)
        .ok_or_else(|| "invalid transaction helper arguments".to_owned()))
        .collect::<Result<Vec<_>, _>>()?;
    let executable = std::env::current_exe().map_err(|error| error.to_string())?;
    let directory = transaction_entry::resolve(&root, &executable, &arguments)?;
    workspace::verify(&directory)?;
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    while !directory.join("handoff.json").exists() && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(100));
    }
    helper_runtime::prepare_ready(&directory)?;
    helper_runtime::wait_for_release(&directory)?;
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let mut owner = loop {
        match helper_runtime::take_over(&directory) {
            Ok(owner) => break owner,
            Err(error) if error == "helper install guard busy" && std::time::Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(error) => return Err(error),
        }
    };
    let mut backend = native_install::NativeInstaller::open(&directory)?;
    let result = owner.execute(&mut backend)?;
    match result.phase {
        transaction::Phase::Committed => {
            owner.finalize()?;
            drop(owner);
            if let Err(error) = rescue_assets::after_commit(&directory) {
                hbb_common::log::warn!("rescue assets preparation incomplete: {error}");
            }
            Ok(())
        }
        transaction::Phase::RolledBack => owner.finalize(),
        _ => Err(format!("transaction requires reconciliation: {:?}", result.phase)),
    }
}

#[cfg(windows)]
pub fn resume_transaction_helper(arguments: &[std::ffi::OsString]) -> Result<(), String> {
    if !unsafe { windows::Win32::UI::Shell::IsUserAnAdmin().as_bool() } {
        return Err("transaction helper requires administrator privileges".into());
    }
    let program_data = system_dirs::get().program_data.clone();
    let root = program_data.join("RustDesk").join("ManagementAgent").join("transactions");
    let arguments = arguments.iter().map(|value| value.to_str().map(str::to_owned)
        .ok_or_else(|| "invalid transaction helper arguments".to_owned()))
        .collect::<Result<Vec<_>, _>>()?;
    let executable = std::env::current_exe().map_err(|error| error.to_string())?;
    let directory = transaction_entry::resolve(&root, &executable, &arguments)?;
    let mut owner = helper_runtime::resume_installation(&directory)?;
    let mut backend = native_install::NativeInstaller::open(&directory)?;
    let result = owner.resume(&mut backend)?;
    match result.phase {
        transaction::Phase::Committed => {
            owner.finalize()?;
            drop(owner);
            if let Err(error) = rescue_assets::after_commit(&directory) {
                hbb_common::log::warn!("rescue assets preparation incomplete: {error}");
            }
            Ok(())
        }
        transaction::Phase::RolledBack => owner.finalize(),
        _ => Err(format!("transaction requires reconciliation: {:?}", result.phase)),
    }
}

const UPDATE_SIGNATURE_KEY_ID: &str = "yan-release-2026";
const UPDATE_PUBLIC_KEY_BASE64: &str = "YNphn2SGjnetwp0bb/uEGpzfQi8OavMWTHCmqPbMuxg=";

pub const MANIFEST_URL: &str = "https://rdapi.yan.life/rd/update/v1/manifest/stable.json";
pub const OSS_DOWNLOAD_PREFIX: &str = "https://download.yan.life/";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Edition {
    Standard,
    Sos,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PackageKind {
    Msi,
    Exe,
}

impl Default for PackageKind {
    fn default() -> Self {
        Self::Msi
    }
}

impl PackageKind {
    pub fn extension(self) -> &'static str {
        match self {
            Self::Msi => "msi",
            Self::Exe => "exe",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.to_ascii_lowercase().as_str() {
            "msi" => Some(Self::Msi),
            "exe" => Some(Self::Exe),
            _ => None,
        }
    }
}

impl Edition {
    pub fn target_key(self) -> &'static str {
        self.target_key_for(PackageKind::Msi)
    }

    pub fn target_key_for(self, package_kind: PackageKind) -> &'static str {
        match self {
            Self::Standard => match package_kind {
                PackageKind::Msi => "windows-x86_64-msi-standard",
                PackageKind::Exe => "windows-x86_64-exe-standard",
            },
            Self::Sos => match package_kind {
                PackageKind::Msi => "windows-x86_64-msi-sos",
                PackageKind::Exe => "windows-x86_64-exe-sos",
            },
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Standard => "standard",
            Self::Sos => "sos",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.to_ascii_lowercase().as_str() {
            "standard" => Some(Self::Standard),
            "sos" => Some(Self::Sos),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentPaths {
    pub root: PathBuf,
    pub agent: PathBuf,
    pub state: PathBuf,
    pub cache: PathBuf,
    pub logs: PathBuf,
    pub scripts: PathBuf,
    pub client: PathBuf,
}

impl AgentPaths {
    pub fn from_program_data(program_data: impl AsRef<Path>, edition: Edition) -> Self {
        let root = program_data.as_ref().join("RustDesk");
        let agent = root.join("ManagementAgent");
        Self {
            state: agent.join("state"),
            cache: agent.join("cache"),
            logs: agent.join("logs"),
            scripts: agent.join("scripts"),
            client: root.join("Client").join(edition.name()).join("RustDesk"),
            root,
            agent,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentState {
    #[serde(default = "default_agent_schema_version")]
    pub schema_version: u32,
    pub edition: Edition,
    #[serde(default)]
    pub package_kind: PackageKind,
    pub target_key: String,
    pub manifest_url: String,
    #[serde(default)]
    pub normal_install_path: String,
    #[serde(default)]
    pub recovery_install_path: String,
    #[serde(default)]
    pub next_check_at: Option<String>,
    #[serde(default)]
    pub retry_stage: u8,
    #[serde(default)]
    pub last_version: String,
    #[serde(default)]
    pub last_primary_url: String,
    #[serde(default)]
    pub last_error: String,
    #[serde(default)]
    pub disabled: bool,
    #[serde(default = "default_management_agent_enabled")]
    pub management_agent_enabled: bool,
    #[serde(default)]
    pub management_agent_policy_revision: u64,
    #[serde(default)]
    pub last_policy_check_at: Option<String>,
    #[serde(default)]
    pub last_policy_error: String,
    #[serde(default)]
    pub service_recovery_failures: u8,
    #[serde(default)]
    pub service_recovery_next_at: u64,
    #[serde(default)]
    pub service_healthy_since: Option<u64>,
    #[serde(default)]
    pub last_service_recovery_at: Option<String>,
    #[serde(default)]
    pub service_recovery_error: String,
    #[serde(default)]
    pub agent_installation_id: String,
    #[serde(default)]
    pub status_session_generation: u64,
    // Opaque forward-compatible state only; never consulted for authorization.
    #[serde(flatten)]
    extra_fields: std::collections::BTreeMap<String, serde_json::Value>,
}

const fn default_agent_schema_version() -> u32 {
    1
}
const fn default_management_agent_enabled() -> bool {
    true
}

impl AgentState {
    pub fn new(edition: Edition, paths: &AgentPaths) -> Self {
        Self {
            schema_version: default_agent_schema_version(),
            edition,
            package_kind: PackageKind::Msi,
            target_key: edition.target_key_for(PackageKind::Msi).to_owned(),
            manifest_url: MANIFEST_URL.to_owned(),
            normal_install_path: String::new(),
            recovery_install_path: paths.client.to_string_lossy().into_owned(),
            next_check_at: None,
            retry_stage: 0,
            last_version: String::new(),
            last_primary_url: String::new(),
            last_error: String::new(),
            disabled: false,
            management_agent_enabled: true,
            management_agent_policy_revision: 0,
            last_policy_check_at: None,
            last_policy_error: String::new(),
            service_recovery_failures: 0,
            service_recovery_next_at: 0,
            service_healthy_since: None,
            last_service_recovery_at: None,
            service_recovery_error: String::new(),
            agent_installation_id: String::new(),
            status_session_generation: 0,
            extra_fields: std::collections::BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct Manifest {
    pub schema: u32,
    #[serde(default)]
    pub catalog_revision: u64,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub build_number: String,
    #[serde(default)]
    pub build_seq: u64,
    #[serde(default)]
    pub product: String,
    #[serde(default)]
    pub edition: String,
    #[serde(default)]
    pub channel: String,
    #[serde(default)]
    pub source_commit: String,
    pub targets: std::collections::BTreeMap<String, ManifestTarget>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct ManifestTarget {
    pub version: String,
    pub build_seq: u64,
    #[serde(default)]
    pub build_number: String,
    #[serde(default)]
    pub source_commit: String,
    #[serde(default)]
    pub source_tag: String,
    pub primary: String,
    #[serde(default)]
    pub mirrors: Vec<String>,
    pub size: u64,
    pub sha256: String,
    pub signature: String,
    pub signature_key_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ManifestError {
    MissingTarget(String),
    InvalidPrimary(String),
    InvalidSha256,
    InvalidSize,
    InvalidSignature,
    InvalidManifestSignature(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceConfig {
    pub edition: Edition,
    pub package_kind: PackageKind,
    pub normal_install_path: Option<PathBuf>,
}

impl Default for ServiceConfig {
    fn default() -> Self {
        Self {
            edition: Edition::Standard,
            package_kind: PackageKind::Msi,
            normal_install_path: None,
        }
    }
}

pub fn parse_service_arguments<I, S>(arguments: I) -> Result<ServiceConfig, String>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let arguments = arguments.into_iter().map(Into::into).collect::<Vec<_>>();
    let mut config = ServiceConfig::default();
    let mut index = 0;
    while index < arguments.len() {
        match arguments[index].as_str() {
            "--edition" => {
                let value = arguments
                    .get(index + 1)
                    .ok_or_else(|| "--edition requires a value".to_owned())?;
                config.edition =
                    Edition::parse(value).ok_or_else(|| format!("unsupported edition: {value}"))?;
                index += 2;
            }
            "--package-kind" => {
                let value = arguments
                    .get(index + 1)
                    .ok_or_else(|| "--package-kind requires a value".to_owned())?;
                config.package_kind = PackageKind::parse(value)
                    .ok_or_else(|| format!("unsupported package kind: {value}"))?;
                index += 2;
            }
            "--normal-install-path" => {
                let value = arguments
                    .get(index + 1)
                    .ok_or_else(|| "--normal-install-path requires a value".to_owned())?;
                let value = value.trim().trim_matches('"').trim();
                let mut value = value.trim_end_matches(['\\', '/']).trim_matches('"');
                while value.ends_with('\\') || value.ends_with('/') {
                    value = value.trim_end_matches(['\\', '/']);
                }
                if value.is_empty() {
                    return Err("--normal-install-path must not be empty".to_owned());
                }
                config.normal_install_path = Some(PathBuf::from(value));
                index += 2;
            }
            _ => index += 1,
        }
    }
    Ok(config)
}

pub fn select_oss_target<'a>(
    manifest: &'a Manifest,
    edition: Edition,
) -> Result<&'a ManifestTarget, ManifestError> {
    select_oss_target_for(manifest, edition, PackageKind::Msi)
}

pub fn select_oss_target_for<'a>(
    manifest: &'a Manifest,
    edition: Edition,
    package_kind: PackageKind,
) -> Result<&'a ManifestTarget, ManifestError> {
    let target = manifest
        .targets
        .get(edition.target_key_for(package_kind))
        .ok_or_else(|| {
            ManifestError::MissingTarget(edition.target_key_for(package_kind).to_owned())
        })?;
    if !target.primary.starts_with(OSS_DOWNLOAD_PREFIX) {
        return Err(ManifestError::InvalidPrimary(target.primary.clone()));
    }
    if target.size == 0 {
        return Err(ManifestError::InvalidSize);
    }
    if target.sha256.len() != 64 || !target.sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(ManifestError::InvalidSha256);
    }
    if target.signature.is_empty() || target.signature_key_id.is_empty() {
        return Err(ManifestError::InvalidSignature);
    }
    Ok(target)
}

pub fn verify_download(bytes: &[u8], target: &ManifestTarget) -> Result<(), ManifestError> {
    if bytes.len() as u64 != target.size {
        return Err(ManifestError::InvalidSize);
    }
    let digest = Sha256::digest(bytes);
    let expected = hex::decode(&target.sha256).map_err(|_| ManifestError::InvalidSha256)?;
    if digest.as_slice() != expected.as_slice() {
        return Err(ManifestError::InvalidSha256);
    }
    Ok(())
}

pub fn verify_manifest_signature(
    manifest: &Manifest,
    target_key: &str,
    target: &ManifestTarget,
) -> Result<(), ManifestError> {
    if target.signature_key_id != UPDATE_SIGNATURE_KEY_ID {
        return Err(ManifestError::InvalidSignature);
    }
    let public_key = decode_signature_value(UPDATE_PUBLIC_KEY_BASE64)?;
    let signature = decode_signature_value(&target.signature)?;
    let catalog_target = manifest
        .targets
        .get(target_key)
        .ok_or_else(|| ManifestError::MissingTarget(target_key.to_owned()))?;
    if catalog_target != target {
        return Err(ManifestError::InvalidManifestSignature(
            "target mismatch".into(),
        ));
    }
    let signed_manifest = hbb_common::UpdateManifest {
        schema: manifest.schema,
        catalog_revision: manifest.catalog_revision,
        version: manifest.version.clone(),
        build_number: manifest.build_number.clone(),
        build_seq: manifest.build_seq,
        product: manifest.product.clone(),
        edition: manifest.edition.clone(),
        channel: manifest.channel.clone(),
        source_commit: manifest.source_commit.clone(),
        targets: std::iter::once((
            target_key.to_owned(),
            hbb_common::UpdateTarget {
                version: catalog_target.version.clone(),
                build_number: catalog_target.build_number.clone(),
                build_seq: catalog_target.build_seq,
                source_commit: catalog_target.source_commit.clone(),
                source_tag: catalog_target.source_tag.clone(),
                primary: catalog_target.primary.clone(),
                mirrors: catalog_target.mirrors.clone(),
                size: catalog_target.size,
                sha256: catalog_target.sha256.clone(),
                signature: catalog_target.signature.clone(),
                signature_key_id: catalog_target.signature_key_id.clone(),
            },
        ))
        .collect(),
        ..Default::default()
    };
    let target = hbb_common::UpdateTarget {
        version: catalog_target.version.clone(),
        build_number: catalog_target.build_number.clone(),
        build_seq: catalog_target.build_seq,
        source_commit: catalog_target.source_commit.clone(),
        source_tag: catalog_target.source_tag.clone(),
        primary: catalog_target.primary.clone(),
        mirrors: catalog_target.mirrors.clone(),
        size: catalog_target.size,
        sha256: catalog_target.sha256.clone(),
        signature: catalog_target.signature.clone(),
        signature_key_id: catalog_target.signature_key_id.clone(),
    };
    let _ = hbb_common::sodiumoxide::init();
    let payload = base::update::update_signature_payload(&signed_manifest, target_key, &target)
        .map_err(|error| ManifestError::InvalidManifestSignature(error.to_string()))?;
    let public_key = hbb_common::sodiumoxide::crypto::sign::PublicKey::from_slice(&public_key)
        .ok_or_else(|| ManifestError::InvalidManifestSignature("invalid public key".into()))?;
    let signature = hbb_common::sodiumoxide::crypto::sign::Signature::from_bytes(&signature)
        .map_err(|_| ManifestError::InvalidManifestSignature("invalid signature".into()))?;
    if !hbb_common::sodiumoxide::crypto::sign::verify_detached(&signature, &payload, &public_key) {
        return Err(ManifestError::InvalidManifestSignature(
            "signature verification failed".into(),
        ));
    }
    Ok(())
}

fn decode_signature_value(value: &str) -> Result<Vec<u8>, ManifestError> {
    STANDARD
        .decode(value)
        .or_else(|_| URL_SAFE_NO_PAD.decode(value))
        .map_err(|error| ManifestError::InvalidManifestSignature(error.to_string()))
}

pub fn retry_delay(stage: u8) -> Duration {
    match stage {
        0 => Duration::from_secs(5 * 60),
        1 => Duration::from_secs(15 * 60),
        2 => Duration::from_secs(30 * 60),
        _ => Duration::from_secs(60 * 60),
    }
}

pub fn record_failure(state: &mut AgentState, message: impl Into<String>) {
    let delay = retry_delay(state.retry_stage);
    state.retry_stage = state.retry_stage.saturating_add(1).min(3);
    state.last_error = message.into();
    state.next_check_at = Some((unix_timestamp() + delay.as_secs()).to_string());
}

pub fn schedule_initial_check(state: &mut AgentState) {
    if state.next_check_at.is_none() {
        state.next_check_at = Some((unix_timestamp() + retry_delay(0).as_secs()).to_string());
    }
}

pub fn record_success(state: &mut AgentState, target: &ManifestTarget) {
    state.retry_stage = 0;
    state.last_error.clear();
    state.last_version = target.version.clone();
    state.last_primary_url = target.primary.clone();
    state.next_check_at = None;
}

fn apply_service_config(state: &mut AgentState, config: &ServiceConfig, paths: &AgentPaths) {
    state.edition = config.edition;
    state.package_kind = config.package_kind;
    state.target_key = config
        .edition
        .target_key_for(config.package_kind)
        .to_owned();
    state.recovery_install_path = paths.client.to_string_lossy().into_owned();
    if let Some(path) = config.normal_install_path.as_deref() {
        state.normal_install_path = path.to_string_lossy().into_owned();
    }
}

pub fn recovery_msi_args(paths: &AgentPaths) -> Vec<String> {
    vec![
        "/qn".into(),
        "/norestart".into(),
        format!("INSTALLFOLDER={}", paths.client.display()),
        "STARTMENUSHORTCUTS=0".into(),
        "DESKTOPSHORTCUTS=0".into(),
        "STARTUPSHORTCUTS=0".into(),
        "LAUNCH_TRAY_APP=N".into(),
        "RUSTDESK_AGENT_RECOVERY=1".into(),
    ]
}

// Fixed verbose log of the silent recovery MSI, overwritten on each run, beside
// the agent's other fixed directories.
pub fn recovery_msi_log_args(paths: &AgentPaths) -> Vec<String> {
    vec![
        "/L*v".into(),
        paths.logs.join("recovery-msi.log").display().to_string(),
    ]
}

// msiexec aborts with 1622 when the log directory is missing; recovery must
// not fail just because its log cannot be written.
#[cfg(windows)]
fn usable_recovery_msi_log_args(paths: &AgentPaths) -> Vec<String> {
    match std::fs::create_dir_all(&paths.logs) {
        Ok(()) => recovery_msi_log_args(paths),
        Err(error) => {
            hbb_common::log::warn!("recovery MSI log directory unavailable: {error}");
            Vec::new()
        }
    }
}

#[cfg(windows)]
const RECOVERY_DEFERRED: &str = "management agent recovery delegated to helper";

#[cfg(windows)]
fn spawn_msi_recovery_helper(
    cache: &Path,
    paths: &AgentPaths,
    state: &AgentState,
    target: &ManifestTarget,
) -> Result<(), String> {
    let source = std::env::current_exe().map_err(|error| error.to_string())?;
    let helper = paths
        .agent
        .join(format!("recovery-helper-{}.exe", std::process::id()));
    std::fs::copy(&source, &helper).map_err(|error| error.to_string())?;
    let result = std::process::Command::new(&helper)
        .arg("--management-agent-recovery-helper")
        .arg("--edition")
        .arg(state.edition.name())
        .arg("--package-kind")
        .arg(state.package_kind.extension())
        .arg("--recovery-cache")
        .arg(cache)
        .arg("--recovery-version")
        .arg(&target.version)
        .arg("--recovery-primary-url")
        .arg(&target.primary)
        .spawn();
    if let Err(error) = result {
        let _ = std::fs::remove_file(&helper);
        return Err(error.to_string());
    }
    Ok(())
}

#[cfg(windows)]
pub fn run_recovery_helper(arguments: &[std::ffi::OsString]) -> Result<(), String> {
    fn value(arguments: &[std::ffi::OsString], name: &str) -> Result<String, String> {
        arguments
            .windows(2)
            .find(|pair| pair[0] == name)
            .map(|pair| pair[1].to_string_lossy().into_owned())
            .ok_or_else(|| format!("{name} requires a value"))
    }

    let config = parse_service_arguments(
        arguments
            .iter()
            .skip(1)
            .map(|argument| argument.to_string_lossy().into_owned()),
    )?;
    let cache = PathBuf::from(value(arguments, "--recovery-cache")?);
    let version = value(arguments, "--recovery-version")?;
    let primary_url = value(arguments, "--recovery-primary-url")?;
    let helper = std::env::current_exe().map_err(|error| error.to_string())?;
    let program_data = system_dirs::get().program_data.clone();
    let paths = AgentPaths::from_program_data(program_data, config.edition);
    let state_path = paths.state.join("agent.json");

    let stop_script = "$ErrorActionPreference='Stop'; $s=Get-Service -Name 'RustDeskManagementAgent' -ErrorAction SilentlyContinue; if($s -and $s.Status -ne 'Stopped'){ Stop-Service -InputObject $s -Force; $s.WaitForStatus('Stopped',[TimeSpan]::FromSeconds(90)) }";
    let stop_status = std::process::Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            stop_script,
        ])
        .status()
        .map_err(|error| error.to_string())?;
    if !stop_status.success() {
        let _ = std::fs::remove_file(&helper);
        return Err(format!("failed to stop management agent: {stop_status}"));
    }

    let result = (|| {
        let status = std::process::Command::new("msiexec.exe")
            .arg("/i")
            .arg(&cache)
            .args(recovery_msi_args(&paths))
            .args(usable_recovery_msi_log_args(&paths))
            .status()
            .map_err(|error| error.to_string())?;
        if !status.success() {
            return Err(format!(
                "msiexec exited with exit code: {}",
                status.code().unwrap_or(-1)
            ));
        }
        if !recovery_client_present(&paths) {
            return Err("MSI completed without RustDesk.exe".to_owned());
        }
        Ok(())
    })();
    remove_download_file(&cache);

    let mut state = std::fs::read(&state_path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<AgentState>(&bytes).ok())
        .unwrap_or_else(|| AgentState::new(config.edition, &paths));
    apply_service_config(&mut state, &config, &paths);
    let result = match result {
        Ok(()) => {
            let target = ManifestTarget {
                version,
                build_seq: 0,
                build_number: String::new(),
                source_commit: String::new(),
                source_tag: String::new(),
                primary: primary_url,
                mirrors: Vec::new(),
                size: 0,
                sha256: String::new(),
                signature: String::new(),
                signature_key_id: String::new(),
            };
            record_success(&mut state, &target);
            Ok(())
        }
        Err(error) => {
            record_failure(&mut state, error.clone());
            Err(error)
        }
    };
    let start_script = "$ErrorActionPreference='Stop'; $s=Get-Service -Name 'RustDeskManagementAgent' -ErrorAction SilentlyContinue; if($s){ if($s.Status -ne 'Running'){ Start-Service -InputObject $s }; $s.WaitForStatus('Running',[TimeSpan]::FromSeconds(90)) }";
    let start_status = std::process::Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            start_script,
        ])
        .status()
        .map_err(|error| error.to_string())?;
    let _ = std::fs::remove_file(helper);
    if !start_status.success() {
        let error = format!("failed to restart management agent: {start_status}");
        record_failure(&mut state, error.clone());
        let _ = write_state(&state_path, &state);
        return Err(error);
    }
    write_state(&state_path, &state).map_err(|error| error.to_string())?;
    result
}

pub fn recovery_exe_args(paths: &AgentPaths) -> Vec<String> {
    vec![
        "--management-agent-install".into(),
        format!("--management-agent-install-path={}", paths.client.display()),
        "--management-agent-no-shortcuts".into(),
        "--management-agent-no-tray".into(),
    ]
}

pub fn recovery_allowed(management_agent_enabled: bool, marker_exists: bool) -> bool {
    management_agent_enabled && !marker_exists
}

pub fn purge_marker_path(paths: &AgentPaths) -> PathBuf {
    paths.state.join("disable-recovery")
}

pub fn recovery_client_present(paths: &AgentPaths) -> bool {
    paths.client.join("RustDesk.exe").is_file()
}

pub fn normal_client_exe(install_path: &str) -> Option<PathBuf> {
    let mut path = install_path.trim();
    while path.ends_with('\\') || path.ends_with('/') {
        path = path.trim_end_matches(['\\', '/']).trim_end();
    }
    if path.is_empty() {
        return None;
    }
    Some(Path::new(path).join("RustDesk.exe"))
}

pub fn normal_client_present(state: &AgentState) -> bool {
    normal_client_exe(&state.normal_install_path)
        .map(|path| path.is_file())
        .unwrap_or(false)
}

#[cfg(windows)]
fn discover_normal_install_path() -> Option<PathBuf> {
    let dirs = system_dirs::get();
    [&dirs.program_files, &dirs.program_files_x86]
        .into_iter()
        .map(|root| root.join("RustDesk"))
        .find(|path| path.join("RustDesk.exe").is_file())
}

pub fn ensure_agent_layout(paths: &AgentPaths) -> std::io::Result<()> {
    for directory in [
        &paths.root,
        &paths.agent,
        &paths.state,
        &paths.cache,
        &paths.logs,
        &paths.scripts,
    ] {
        std::fs::create_dir_all(directory)?;
        #[cfg(windows)]
        set_hidden_system_attribute(directory)?;
    }
    #[cfg(windows)]
    if paths.client.exists() {
        set_hidden_system_attribute(&paths.client)?;
    }
    Ok(())
}

pub fn enable_service_recovery(
    paths: &AgentPaths,
    intent: recovery::Intent,
) -> std::io::Result<bool> {
    if purge_marker_path(paths).exists()
        || paths.state.join("disable-agent").exists()
        || paths.state.join("maintenance.json").exists()
        || intent != recovery::Intent::Enabled
    {
        return Ok(false);
    }
    let marker = paths.state.join("service-recovery-enabled");
    if marker.exists() {
        return Ok(false);
    }
    std::fs::write(marker, b"enabled\n")?;
    Ok(true)
}

#[cfg(windows)]
pub fn local_status(paths: &AgentPaths) -> Result<serde_json::Value, String> {
    use std::io::Read;
    let path = paths.state.join("agent.json");
    let file = std::fs::File::open(&path).map_err(|error| format!("agent state unavailable: {error}"))?;
    let mut bytes = Vec::new();
    file.take(65537).read_to_end(&mut bytes).map_err(|error| error.to_string())?;
    if bytes.len() > 65536 { return Err("agent state too large".into()); }
    let state: AgentState = serde_json::from_slice(&bytes).map_err(|_| "agent state malformed".to_owned())?;
    if state.agent_installation_id.len() != 32 || state.status_session_generation == 0 {
        return Err("agent state identity uninitialized".into());
    }
    let mut payload = status::snapshot(&state, paths, &state.agent_installation_id, &"0".repeat(32), 1)?;
    if let Some(object) = payload.as_object_mut() {
        object.remove("boot_id");
        object.remove("sequence");
        object.insert("observation_source".into(), "local_cli".into());
    }
    Ok(payload)
}

fn replace_verified_download(cache: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(parent) = cache.parent() {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    remove_download_file(cache);
    let temporary = cache.with_extension(format!("download-{}", std::process::id()));
    remove_download_file(&temporary);
    std::fs::write(&temporary, bytes).map_err(|error| error.to_string())?;
    if let Err(error) = std::fs::rename(&temporary, cache) {
        remove_download_file(&temporary);
        remove_download_file(cache);
        return Err(error.to_string());
    }
    Ok(())
}

fn remove_download_file(path: &Path) {
    if path.is_dir() {
        let _ = std::fs::remove_dir_all(path);
    } else {
        let _ = std::fs::remove_file(path);
    }
}

pub(crate) fn unix_timestamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

fn check_due(state: &AgentState) -> bool {
    state
        .next_check_at
        .as_deref()
        .and_then(|value| value.parse::<u64>().ok())
        .map(|timestamp| timestamp <= unix_timestamp())
        .unwrap_or(true)
}

#[cfg(windows)]
fn set_hidden_system_attribute(path: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::{
        SetFileAttributesW, FILE_ATTRIBUTE_HIDDEN, FILE_ATTRIBUTE_SYSTEM,
    };
    let wide = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    unsafe {
        SetFileAttributesW(
            PCWSTR(wide.as_ptr()),
            FILE_ATTRIBUTE_HIDDEN | FILE_ATTRIBUTE_SYSTEM,
        )
        .map_err(std::io::Error::other)
    }
}

/// 后台告知代号过旧时，把本地启动代号前进到后台值之上并先落盘再生效。
/// 本地已经领先或后台值不可信时不改动；写盘失败时内存状态保持原值。
fn apply_stale_session_generation(
    state: &mut AgentState,
    state_path: &Path,
    server_current: u64,
) -> std::io::Result<Option<u64>> {
    let Some(next) = status_generation::advance(state.status_session_generation, server_current)
    else {
        return Ok(None);
    };
    let advanced = AgentState {
        status_session_generation: next,
        ..state.clone()
    };
    write_state(state_path, &advanced)?;
    *state = advanced;
    Ok(Some(next))
}

pub fn write_state(path: &Path, state: &AgentState) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let bytes = serde_json::to_vec_pretty(state).map_err(std::io::Error::other)?;
    #[cfg(windows)]
    return state_store::publish(path, &bytes);
    #[cfg(not(windows))]
    {
        let temporary = path.with_extension(format!("tmp-{}", std::process::id()));
        std::fs::write(&temporary, bytes)?;
        let backup = path.with_extension(format!("bak-{}", std::process::id()));
        if path.exists() {
            if backup.exists() {
                std::fs::remove_file(&backup)?;
            }
            std::fs::rename(path, &backup)?;
        }
        match std::fs::rename(&temporary, path) {
            Ok(()) => {
                let _ = std::fs::remove_file(backup);
                #[cfg(windows)]
                set_hidden_system_attribute(path)?;
                Ok(())
            }
            Err(error) => {
                if backup.exists() {
                    let _ = std::fs::rename(&backup, path);
                }
                let _ = std::fs::remove_file(&temporary);
                Err(error)
            }
        }
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
struct AgentPolicyResponse {
    client_id: String,
    client_uuid: String,
    policy_revision: u64,
    #[serde(default = "default_management_agent_enabled")]
    management_agent_enabled: bool,
}

fn apply_agent_policy(
    state: &mut AgentState,
    policy: AgentPolicyResponse,
    client_id: &str,
    client_uuid: &str,
) -> Result<(), String> {
    if policy.client_id != client_id || policy.client_uuid != client_uuid {
        return Err("agent policy identity mismatch".to_owned());
    }
    if policy.policy_revision < state.management_agent_policy_revision {
        return Err("agent policy revision rollback".to_owned());
    }
    state.management_agent_enabled = policy.management_agent_enabled;
    state.management_agent_policy_revision = policy.policy_revision;
    state.last_policy_check_at = Some(unix_timestamp().to_string());
    state.last_policy_error.clear();
    Ok(())
}

#[cfg(windows)]
fn refresh_agent_policy(state: &mut AgentState) -> Result<(), String> {
    if purge_marker_path(&AgentPaths::from_program_data(
        system_dirs::get().program_data.clone(),
        state.edition,
    ))
    .exists()
    {
        return Ok(());
    }
    let identity = identity::load_existing()?;
    let client_id = identity.id;
    let client_uuid = identity.uuid;
    let base_url = std::env::var("RUSTDESK_API_BASE_URL")
        .unwrap_or_else(|_| "https://rdapi.yan.life".to_owned());
    let mut url = url::Url::parse(&format!(
        "{}/rd/update/v1/agent/policy",
        base_url.trim_end_matches('/')
    ))
    .map_err(|error| error.to_string())?;
    url.query_pairs_mut()
        .append_pair("client_id", &client_id)
        .append_pair("client_uuid", &client_uuid)
        .append_pair(
            "after_revision",
            &state.management_agent_policy_revision.to_string(),
        );
    let timestamp = unix_timestamp() as i64;
    let nonce = hex::encode(hbb_common::sodiumoxide::randombytes::randombytes(16));
    let body_sha256 = hex::encode(Sha256::digest(b""));
    let canonical = update_device_auth_payload_for_agent(
        "GET",
        "/rd/update/v1/agent/policy",
        &client_id,
        &client_uuid,
        timestamp,
        &nonce,
        &body_sha256,
    );
    let public_key = identity.public;
    let signature =
        hbb_common::sodiumoxide::crypto::sign::sign_detached(&canonical, &identity.secret);
    let response = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|error| error.to_string())?
        .get(url)
        .header("X-RustDesk-Device-ID", &client_id)
        .header(
            "X-RustDesk-Device-Public-Key",
            STANDARD.encode(public_key.as_ref()),
        )
        .header("X-RustDesk-Device-Timestamp", timestamp.to_string())
        .header("X-RustDesk-Device-Nonce", nonce)
        .header(
            "X-RustDesk-Device-Signature",
            STANDARD.encode(signature.to_bytes()),
        )
        .send()
        .map_err(|error| error.to_string())?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(());
    }
    let response = response
        .error_for_status()
        .map_err(|error| error.to_string())?;
    let policy = response
        .json::<AgentPolicyResponse>()
        .map_err(|error| error.to_string())?;
    apply_agent_policy(state, policy, &client_id, &client_uuid)
}

fn update_device_auth_payload_for_agent(
    method: &str,
    path: &str,
    client_id: &str,
    client_uuid: &str,
    timestamp: i64,
    nonce: &str,
    body_sha256: &str,
) -> Vec<u8> {
    format!(
        "rustdesk-update-auth-v1\nmethod={method}\npath={path}\nclient_id={client_id}\nclient_uuid={client_uuid}\ntimestamp={timestamp}\nnonce={nonce}\ncommand_id=\nbody_sha256={body_sha256}\n"
    )
    .into_bytes()
}

#[cfg(windows)]
fn install_exe_package(package: &Path, paths: &AgentPaths) -> Result<(), String> {
    let install_dir = &paths.client;
    let temporary_dir = install_dir.with_file_name(format!(
        "{}-download-{}",
        install_dir
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("RustDesk"),
        std::process::id()
    ));
    let backup_dir = install_dir.with_file_name(format!(
        "{}-backup-{}",
        install_dir
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("RustDesk"),
        std::process::id()
    ));
    let installed_exe = install_dir.join("RustDesk.exe");
    let result_path = std::env::temp_dir().join(format!(
        "rustdesk-exe-recovery-{}-{}.txt",
        std::process::id(),
        unix_timestamp()
    ));
    let script = format!(
        concat!(
            "$ErrorActionPreference='Stop'; $dir={dir}; $tmp={tmp}; $backup={backup}; $src={src}; $dst={dst}; $result={result}; ",
            "Remove-Item -LiteralPath $tmp -Recurse -Force -ErrorAction SilentlyContinue; ",
            "$extract=Start-Process -FilePath $src -ArgumentList ('--management-agent-extract-path='+$tmp) -Wait -PassThru -WindowStyle Hidden; ",
            "if($extract.ExitCode -ne 0){{throw \"portable EXE extraction failed: $($extract.ExitCode)\"}}; ",
            "if(!(Test-Path -LiteralPath $dst.Replace($dir,$tmp))){{throw 'portable EXE did not extract RustDesk.exe'}}; ",
            "$service=Get-Service -Name 'RustDesk' -ErrorAction SilentlyContinue; ",
            "$old_bin=(Get-CimInstance Win32_Service -Filter \"Name='RustDesk'\" -ErrorAction SilentlyContinue).PathName; ",
            "if($service){{Stop-Service -Name 'RustDesk' -Force -ErrorAction SilentlyContinue; ",
            "try{{$service.WaitForStatus('Stopped',[TimeSpan]::FromSeconds(90))}}catch{{}}}}; ",
            "if(Get-Service -Name 'RustDesk' -ErrorAction SilentlyContinue){{sc.exe delete RustDesk | Out-Null; Start-Sleep -Seconds 2}}; ",
            "if(Test-Path -LiteralPath $backup){{Remove-Item -LiteralPath $backup -Recurse -Force}}; ",
            "if(Test-Path -LiteralPath $dir){{Move-Item -LiteralPath $dir -Destination $backup}}; ",
            "Move-Item -LiteralPath $tmp -Destination $dir; ",
            "$recovery_ok=$false; $recovery_error=''; ",
            "try{{New-Service -Name 'RustDesk' -BinaryPathName ('\"'+$dst+'\" --service') -DisplayName 'RustDesk' -StartupType Automatic | Out-Null; ",
            "Start-Service -Name 'RustDesk'; Remove-Item -LiteralPath $backup -Recurse -Force -ErrorAction SilentlyContinue; $recovery_ok=$true}}catch{{",
            "$recovery_error=$_.Exception.Message; ",
            "if(Test-Path -LiteralPath $dir){{Remove-Item -LiteralPath $dir -Recurse -Force}}; ",
            "if(Test-Path -LiteralPath $backup){{Move-Item -LiteralPath $backup -Destination $dir}}; ",
            "if($old_bin){{New-Service -Name 'RustDesk' -BinaryPathName $old_bin -DisplayName 'RustDesk' -StartupType Automatic | Out-Null; Start-Service -Name 'RustDesk'}}}}; ",
            "Set-Content -LiteralPath $result -Value $(if ($recovery_ok) {{'ok'}} else {{$recovery_error}}) -Encoding ASCII; exit $(if ($recovery_ok) {{0}} else {{1}})"
        ),
        dir = powershell_literal(install_dir),
        tmp = powershell_literal(&temporary_dir),
        backup = powershell_literal(&backup_dir),
        src = powershell_literal(package),
        dst = powershell_literal(&installed_exe),
        result = powershell_literal(&result_path),
    );
    let _ = std::fs::remove_file(&result_path);
    let mut command = std::process::Command::new(windows_powershell_exe());
    command.args([
        "-NoProfile",
        "-NonInteractive",
        "-ExecutionPolicy",
        "Bypass",
        "-Command",
        &script,
    ]);
    let mut child = command.spawn().map_err(|error| error.to_string())?;
    let status = child.wait().map_err(|error| error.to_string())?;
    let detail = wait_for_exe_recovery_result(&result_path, status.success());
    let _ = std::fs::remove_file(&result_path);
    if detail.trim() == "ok" && recovery_client_present(paths) {
        Ok(())
    } else if detail.trim().is_empty() {
        Err(format!(
            "EXE service installer exited with exit code: {}",
            status.code().unwrap_or(-1)
        ))
    } else {
        Err(format!(
            "EXE service installer failed: {}",
            detail.trim()
        ))
    }
}

#[cfg(windows)]
fn windows_powershell_exe() -> PathBuf {
    system_dirs::get()
        .system32()
        .join(r"WindowsPowerShell\v1.0\powershell.exe")
}

#[cfg(windows)]
fn wait_for_exe_recovery_result(path: &Path, exited_cleanly: bool) -> String {
    let waits = if exited_cleanly { 1 } else { 90 };
    for _ in 0..waits {
        if let Ok(detail) = std::fs::read_to_string(path) {
            if !detail.trim().is_empty() {
                return detail;
            }
        }
        std::thread::sleep(Duration::from_secs(1));
    }
    std::fs::read_to_string(path).unwrap_or_default()
}

#[cfg(windows)]
pub fn check_now(edition: Edition, paths: &AgentPaths) -> Result<(), String> {
    check_now_with_normal_path(edition, paths, None)
}

#[cfg(windows)]
fn package_recovery_preflight(paths: &AgentPaths) -> Result<(), String> {
    for marker in ["disable-recovery", "disable-agent", "maintenance.json"] {
        match std::fs::symlink_metadata(paths.state.join(marker)) {
            Ok(_) => return Err("package recovery locally blocked".into()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err("package recovery local gate unreadable".into()),
        }
    }
    Ok(())
}

#[cfg(windows)]
pub fn check_now_with_normal_path(
    edition: Edition,
    paths: &AgentPaths,
    normal_install_path: Option<&Path>,
) -> Result<(), String> {
    check_now_with_package(edition, PackageKind::Msi, paths, normal_install_path)
}

#[cfg(windows)]
pub fn check_now_with_package(
    edition: Edition,
    package_kind: PackageKind,
    paths: &AgentPaths,
    normal_install_path: Option<&Path>,
) -> Result<(), String> {
    package_recovery_preflight(paths)?;
    let state_path = paths.state.join("agent.json");
    let mut state = if state_path.exists() {
        let value = std::fs::read(&state_path).map_err(|e| e.to_string())?;
        serde_json::from_slice::<AgentState>(&value).map_err(|e| e.to_string())?
    } else {
        AgentState::new(edition, paths)
    };
    if !state.management_agent_enabled || state.disabled {
        return Err("management agent is disabled".into());
    }
    if recovery_windows::read_intent()? != recovery::Intent::Enabled {
        return Err("package recovery user stop intent".into());
    }
    let Some(_guard) = recovery_windows::install_guard::InstallGuard::try_acquire()
        .map_err(|error| format!("package recovery install guard: {error}"))?
    else {
        return Err("package recovery installation busy".into());
    };
    package_recovery_preflight(paths)?;
    if recovery_windows::inspect(&state, paths)?.disabled_start {
        return Err("package recovery service disabled".into());
    }
    ensure_agent_layout(paths).map_err(|error| error.to_string())?;
    apply_service_config(
        &mut state,
        &ServiceConfig {
            edition,
            package_kind,
            normal_install_path: normal_install_path.map(PathBuf::from),
        },
        paths,
    );
    if purge_marker_path(paths).exists() {
        return Err("recovery is disabled".to_owned());
    }
    #[cfg(windows)]
    if let Err(error) = refresh_agent_policy(&mut state) {
        state.last_policy_error = error;
        let _ = write_state(&state_path, &state);
    }
    if !recovery_allowed(state.management_agent_enabled && !state.disabled, false) {
        return Err("management agent is disabled".to_owned());
    }
    if state.normal_install_path.is_empty() {
        let discovered = normal_install_path
            .map(PathBuf::from)
            .or_else(discover_normal_install_path);
        if let Some(path) = discovered {
            state.normal_install_path = path.to_string_lossy().into_owned();
        }
    }
    state.manifest_url = MANIFEST_URL.to_owned();
    if normal_client_present(&state) || recovery_client_present(paths) {
        return write_state(&state_path, &state).map_err(|e| e.to_string());
    }
    let result = (|| {
        let client = reqwest::blocking::Client::builder()
            .connect_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(60))
            .build()
            .map_err(|e| e.to_string())?;
        let response = client
            .get(&state.manifest_url)
            .send()
            .map_err(|e| e.to_string())?;
        if !response.status().is_success() {
            return Err(format!("manifest HTTP {}", response.status()));
        }
        let manifest = response.json::<Manifest>().map_err(|e| e.to_string())?;
        let target = select_oss_target_for(&manifest, edition, state.package_kind)
            .map_err(|e| format!("manifest: {e:?}"))?;
        verify_manifest_signature(&manifest, state.target_key.as_str(), target)
            .map_err(|e| format!("manifest signature: {e:?}"))?;
        let response = client
            .get(&target.primary)
            .send()
            .map_err(|e| e.to_string())?;
        if !response.status().is_success() {
            return Err(format!("download HTTP {}", response.status()));
        }
        let bytes = response.bytes().map_err(|e| e.to_string())?;
        verify_download(&bytes, target).map_err(|e| format!("package: {e:?}"))?;
        let cache = paths.cache.join(format!(
            "rustdesk-{}-latest.{}",
            edition.name(),
            state.package_kind.extension()
        ));
        replace_verified_download(&cache, &bytes)?;
        #[cfg(windows)]
        set_hidden_system_attribute(&cache).map_err(|e| e.to_string())?;
        if state.package_kind == PackageKind::Msi {
            let spawned = spawn_msi_recovery_helper(&cache, paths, &state, target);
            if spawned.is_err() {
                remove_download_file(&cache);
            }
            spawned?;
            return Err(RECOVERY_DEFERRED.to_owned());
        }
        let installed = install_exe_package(&cache, paths);
        remove_download_file(&cache);
        installed?;
        record_success(&mut state, target);
        Ok(())
    })();
    if let Err(error) = result {
        if error == RECOVERY_DEFERRED {
            return Ok(());
        }
        record_failure(&mut state, error.clone());
        let _ = write_state(&state_path, &state);
        return Err(error);
    }
    write_state(&state_path, &state).map_err(|e| e.to_string())
}

#[cfg(windows)]
pub fn run_service(config: ServiceConfig, paths: AgentPaths) -> Result<(), String> {
    use windows_service::service_dispatcher;
    let _ = (config, paths);
    service_dispatcher::start("RustDeskManagementAgent", ffi_service_main)
        .map_err(|e| e.to_string())
}

#[cfg(windows)]
windows_service::define_windows_service!(ffi_service_main, service_entry);

#[cfg(windows)]
fn service_entry(arguments: Vec<std::ffi::OsString>) {
    if let Err(error) = service_main(arguments) {
        hbb_common::log::error!("management agent service failed: {error}");
    }
}

#[cfg(windows)]
fn service_main(_arguments: Vec<std::ffi::OsString>) -> windows_service::Result<()> {
    use std::sync::mpsc;
    use windows_service::service::{
        ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus,
        ServiceType,
    };
    use windows_service::service_control_handler::{self, ServiceControlHandlerResult};
    let program_data = system_dirs::get().program_data.clone();
    // SCM callback arguments are not the service ImagePath. Read the original
    // process command line so edition/package parameters survive SCM dispatch.
    let config = parse_service_arguments(std::env::args().skip(1)).map_err(|error| {
        windows_service::Error::Winapi(std::io::Error::new(std::io::ErrorKind::InvalidInput, error))
    })?;
    let paths = AgentPaths::from_program_data(program_data, config.edition);
    ensure_agent_layout(&paths).map_err(windows_service::Error::Winapi)?;
    let intent = if hbb_common::config::Config2::file().exists() {
        recovery_windows::read_intent().unwrap_or(recovery::Intent::Unknown)
    } else {
        recovery::Intent::Enabled
    };
    // Re-apply SCM failure actions on every start so an in-place upgrade (which
    // never re-runs the install script) still gains sub-second crash recovery.
    // Gated by the same local stop intent as every other mutating action: an
    // operator who disabled recovery is troubleshooting, and rewriting restart
    // actions behind their back would undo that. Failing to configure is
    // non-fatal; failing to even start the worker is treated like every other
    // worker thread here.
    //
    // The gate is re-read on every attempt rather than decided once here: an
    // in-place upgrade holds maintenance.json while its own new agent starts, so a
    // start-time decision would skip configuration for exactly the upgrade this
    // capability exists for, and never revisit it until the next restart.
    let failures_stopped = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let terminal_stopped = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let failure_actions_stop = failures_stopped.clone();
    let failure_actions_paths = paths.clone();
    std::thread::Builder::new()
        .name("agent-failure-actions".into())
        .spawn(move || {
            // The installer creates the client service near InstallFinalize, after
            // this agent service starts, so the first attempt can legitimately find
            // it absent; and a pending transaction can hold the gate for its whole
            // duration. Retrying in the background covers both without blocking
            // service startup.
            use recovery_windows::{FailureActionsEnd as End, FailureActionsStep as Step};
            let mut retry = recovery_windows::FailureActionsRetry::new(std::time::Instant::now());
            let mut reported = false;
            let end = loop {
                let stopped = failure_actions_stop.load(std::sync::atomic::Ordering::SeqCst);
                let allowed = recovery_windows::failure_actions_allowed_now(&failure_actions_paths);
                match retry.before_attempt(std::time::Instant::now(), stopped, allowed) {
                    Step::Done(end) => break end,
                    Step::Wait => {
                        std::thread::sleep(Duration::from_secs(5));
                        continue;
                    }
                    Step::Attempt => {}
                }
                let complete = match recovery_windows::ensure_service_failure_actions() {
                    Ok(complete) => complete,
                    Err(error) => {
                        if !reported {
                            reported = true;
                            hbb_common::throttled_log!(
                                Duration::from_secs(3600),
                                warn,
                                "service failure actions setup failed: {error}"
                            );
                        }
                        false
                    }
                };
                match retry.after_attempt(std::time::Instant::now(), complete) {
                    Step::Done(end) => break end,
                    _ => std::thread::sleep(Duration::from_secs(5)),
                }
            };
            match end {
                End::Configured | End::Stopped => {}
                // The operator's local stop intent, not a fault.
                End::HeldByGate => {
                    hbb_common::log::info!("service failure actions left unchanged by local gate")
                }
                End::Incomplete => hbb_common::throttled_log!(
                    Duration::from_secs(3600),
                    warn,
                    "service failure actions not fully applied after retries"
                ),
            }
        })
        .map_err(windows_service::Error::Winapi)?;
    enable_service_recovery(&paths, intent).map_err(windows_service::Error::Winapi)?;
    hbb_common::sodiumoxide::init().map_err(|_| {
        windows_service::Error::Winapi(std::io::Error::other("agent crypto initialization failed"))
    })?;
    let (stop_tx, stop_rx) = mpsc::channel();
    let terminal_stop_for_scm = terminal_stopped.clone();
    let status_handle = service_control_handler::register(
        "RustDeskManagementAgent",
        move |control| match control {
            ServiceControl::Stop | ServiceControl::Shutdown => {
                failures_stopped.store(true, std::sync::atomic::Ordering::SeqCst);
                terminal_stop_for_scm.store(true, std::sync::atomic::Ordering::SeqCst);
                let _ = stop_tx.send(());
                ServiceControlHandlerResult::NoError
            }
            _ => ServiceControlHandlerResult::NotImplemented,
        },
    )?;
    status_handle.set_service_status(ServiceStatus {
        service_type: ServiceType::OWN_PROCESS,
        current_state: ServiceState::Running,
        controls_accepted: ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN,
        exit_code: ServiceExitCode::Win32(0),
        checkpoint: 0,
        wait_hint: Duration::default(),
        process_id: None,
    })?;
    let started = std::time::Instant::now();
    let boot_id = hex::encode(hbb_common::sodiumoxide::randombytes::randombytes(16));
    let mut status_sequence = 0u64;
    let mut session_generation = None;
    // 每次启动最多按后台代号前进一次：整机克隆的两台设备共用身份时，
    // 不让它们无休止地互相把代号顶上去，第二次起只记日志。
    let mut stale_generation_advanced = false;
    let mut bootstrap_initialized = false;
    let local_repair_worker =
        local_repair_schedule::start().map_err(windows_service::Error::Winapi)?;
    let rescue_status_worker = rescue_status::start().map_err(windows_service::Error::Winapi)?;
    let first_install_worker =
        rescue_assets::first_install::start().map_err(windows_service::Error::Winapi)?;
    let registration_worker =
        registration_reconnect::start().map_err(windows_service::Error::Winapi)?;
    let terminal_paths = paths.clone();
    let terminal_stop = terminal_stopped.clone();
    std::thread::Builder::new()
        .name("agent-terminal".into())
        .spawn(move || terminal::runtime::run(terminal_paths, terminal_stop))
        .map_err(windows_service::Error::Winapi)?;
    let (status_tx, status_rx) = mpsc::sync_channel::<(Option<serde_json::Value>, AgentState)>(1);
    let (stale_generation_tx, stale_generation_rx) = mpsc::sync_channel::<u64>(1);
    let (policy_tx, policy_rx) = mpsc::sync_channel::<AgentState>(1);
    let (operations_tx, operations_rx) = mpsc::sync_channel::<AgentState>(1);
    let operations_paths = paths.clone();
    std::thread::Builder::new()
        .name("agent-commands".into())
        .spawn(move || {
            let mut budget = network_policy::NetworkBudget::default();
            for state in operations_rx {
                if !budget.due(std::time::Instant::now()) { continue; }
                match operations_runtime::poll(&state, &operations_paths) {
                    Ok(()) => budget.succeeded(),
                    Err(error) => {
                        budget.failed(error.failure, error.retry_after, std::time::Instant::now());
                        hbb_common::throttled_log!(Duration::from_secs(60), warn, "agent command poll failed: {error}");
                    }
                }
            }
        })
        .map_err(windows_service::Error::Winapi)?;
    std::thread::Builder::new()
        .name("agent-status".into())
        .spawn(move || {
            for (payload, mut state) in status_rx {
                if let Err(error) = refresh_agent_policy(&mut state) {
                    state.last_policy_error = error;
                }
                match policy_tx.try_send(state) {
                    Ok(()) | Err(mpsc::TrySendError::Full(_)) => {}
                    Err(mpsc::TrySendError::Disconnected(_)) => break,
                }
                if let Some(payload) = payload {
                    if let Err(error) = status::send(&payload) {
                        if let status::SendError::StaleSession { current } = error {
                            // 交给主循环前进本地代号；满了说明上一条还没处理，丢弃即可。
                            let _ = stale_generation_tx.try_send(current);
                        }
                        hbb_common::throttled_log!(
                            Duration::from_secs(60),
                            warn,
                            "agent status upload failed: {error}"
                        );
                    }
                }
            }
        })
        .map_err(windows_service::Error::Winapi)?;
    loop {
        if stop_rx.recv_timeout(Duration::from_secs(30)).is_ok() {
            break;
        }
        if purge_marker_path(&paths).exists() {
            continue;
        }
        let state_path = paths.state.join("agent.json");
        let mut state = match std::fs::read(&state_path) {
            Ok(bytes) => match serde_json::from_slice::<AgentState>(&bytes) {
                Ok(state) => state,
                Err(error) => {
                    hbb_common::throttled_log!(
                        Duration::from_secs(60),
                        warn,
                        "agent state parse failed: {error}"
                    );
                    continue;
                }
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                AgentState::new(config.edition, &paths)
            }
            Err(error) => {
                hbb_common::throttled_log!(
                    Duration::from_secs(60),
                    warn,
                    "agent state read failed: {error}"
                );
                continue;
            }
        };
        apply_service_config(&mut state, &config, &paths);
        if let Ok(policy) = policy_rx.try_recv() {
            if policy.management_agent_policy_revision >= state.management_agent_policy_revision {
                state.management_agent_enabled = policy.management_agent_enabled;
                state.management_agent_policy_revision = policy.management_agent_policy_revision;
                state.last_policy_check_at = policy.last_policy_check_at;
                state.last_policy_error = policy.last_policy_error;
            }
        }
        if state.agent_installation_id.is_empty() {
            state.agent_installation_id =
                hex::encode(hbb_common::sodiumoxide::randombytes::randombytes(16));
            if let Err(error) = write_state(&state_path, &state) {
                hbb_common::throttled_log!(
                    Duration::from_secs(60),
                    warn,
                    "agent installation identity save failed: {error}"
                );
                continue;
            }
        }
        if session_generation.is_none() {
            let generation = state
                .status_session_generation
                .checked_add(1)
                .filter(|value| *value <= i64::MAX as u64)
                .ok_or_else(|| {
                    windows_service::Error::Winapi(std::io::Error::other("agent session overflow"))
                })?;
            state.status_session_generation = generation;
            state.service_healthy_since = None;
            if let Err(error) = write_state(&state_path, &state) {
                hbb_common::throttled_log!(
                    Duration::from_secs(60),
                    warn,
                    "agent session save failed: {error}"
                );
                continue;
            }
            session_generation = Some(generation);
        }
        if let Ok(current) = stale_generation_rx.try_recv() {
            if stale_generation_advanced {
                hbb_common::throttled_log!(
                    Duration::from_secs(300),
                    warn,
                    "agent session generation still stale after advancing (server {current}); another instance may share this identity"
                );
            } else {
                // 写盘失败时内存状态不变，下一次 409 会再触发，不跳过本轮其余工作。
                match apply_stale_session_generation(&mut state, &state_path, current) {
                    Ok(Some(generation)) => {
                        hbb_common::log::info!(
                            "agent session generation advanced past server {current} to {generation}"
                        );
                        session_generation = Some(generation);
                        stale_generation_advanced = true;
                    }
                    Ok(None) => {}
                    Err(error) => hbb_common::throttled_log!(
                        Duration::from_secs(60),
                        warn,
                        "agent session advance save failed: {error}"
                    ),
                }
            }
        }
        if state.normal_install_path.is_empty() {
            if let Some(path) = discover_normal_install_path() {
                state.normal_install_path = path.to_string_lossy().into_owned();
            }
        }
        if purge_marker_path(&paths).exists() {
            continue;
        }
        if let Err(error) = write_state(&state_path, &state) {
            hbb_common::throttled_log!(Duration::from_secs(60), warn, "agent policy save failed: {error}");
            continue;
        }
        if let Err(error) = local_repair_worker.notify(local_repair_schedule::Tick {
            elapsed: started.elapsed(),
            enabled: state.management_agent_enabled && !state.disabled,
            retry_at: state.service_recovery_next_at,
        }) {
            hbb_common::throttled_log!(
                Duration::from_secs(60),
                warn,
                "automatic local repair unavailable: {error}"
            );
        }
        if let Some(result) = local_repair_worker.result() {
            hbb_common::throttled_log!(
                Duration::from_secs(60),
                info,
                "automatic local repair: {result}"
            );
        }
        if let Err(error) = rescue_status_worker.notify(rescue_status::Tick {
            installation: state.agent_installation_id.clone(),
            target_key: state.target_key.clone(),
        }) {
            hbb_common::throttled_log!(
                Duration::from_secs(300),
                warn,
                "rescue readonly status unavailable: {error}"
            );
        }
        if let Err(error) = registration_worker.notify(&state.agent_installation_id) {
            hbb_common::throttled_log!(
                Duration::from_secs(300),
                warn,
                "R0 worker unavailable: {error}"
            );
        }
        if !bootstrap_initialized {
            match bootstrap::initialize_if_allowed(&config, &paths, &state) {
                Ok(initialized) => bootstrap_initialized = initialized,
                Err(error) => hbb_common::throttled_log!(Duration::from_secs(300), warn, "agent bootstrap initialization failed: {error}"),
            }
        }
        // Notified after bootstrap initialization: the first-install worker copies the
        // recovery bootstrap, and on the first tick it used to run before the directory
        // existed. Its ancestor walk then failed with "os error 3" and left a staging
        // directory behind, which the next attempt refused.
        if let Err(error) = first_install_worker.notify() {
            hbb_common::throttled_log!(
                Duration::from_secs(300),
                warn,
                "first installation rescue unavailable: {error}"
            );
        }
        let mut rescue_error = None;
        let signed_rescue = match rescue_recovery::bundle_present() {
            Ok(present) => present,
            Err(error) => { rescue_error = Some(error); true }
        };
        let mut allow_legacy_service = !signed_rescue;
        let mut allow_legacy_package = !signed_rescue;
        if signed_rescue {
            let mut budget = recovery::RetryBudget { failures:state.service_recovery_failures,next_at:state.service_recovery_next_at,healthy_since:state.service_healthy_since };
            let result = rescue_recovery::budgeted(started.elapsed(), &mut budget, unix_timestamp(), |next| {
                let mut persisted=state.clone();
                persisted.service_recovery_failures=next.failures;persisted.service_recovery_next_at=next.next_at;persisted.service_healthy_since=next.healthy_since;
                write_state(&state_path,&persisted).map_err(|error|error.to_string())
            }, || match rescue_error {Some(error)=>Err(error),None=>rescue_recovery::periodic(started.elapsed())});
            state.service_recovery_failures=budget.failures;state.service_recovery_next_at=budget.next_at;state.service_healthy_since=budget.healthy_since;
            let route=rescue_recovery::routing(&result);
            allow_legacy_service=route.legacy_service;allow_legacy_package=route.legacy_package;
            match result {
                Ok(rescue_recovery::Cycle::Recovered) => state.service_recovery_error = "R3 fallback running; degraded".into(),
                Ok(_) => {},
                Err(error) => state.service_recovery_error = error,
            }
            if let Err(error)=write_state(&state_path,&state) {hbb_common::throttled_log!(Duration::from_secs(60),warn,"R3 result persistence failed: {error}");}
        }
        if allow_legacy_service { if let Err(error) = inspect_and_start_service(&mut state, &paths, started.elapsed()) {
            hbb_common::throttled_log!(
                Duration::from_secs(60),
                warn,
                "agent service observation failed: {error}"
            );
        } }
        status_sequence = status_sequence.saturating_add(1);
        match operations_tx.try_send(state.clone()) {
            Ok(()) | Err(mpsc::TrySendError::Full(_)) => {}
            Err(mpsc::TrySendError::Disconnected(_)) => {
                hbb_common::throttled_log!(Duration::from_secs(60), warn, "agent command worker unavailable");
            }
        }
        let payload = match status::snapshot(
            &state,
            &paths,
            &state.agent_installation_id,
            &boot_id,
            status_sequence,
        ) {
            Ok(payload) => Some(payload),
            Err(error) => {
                hbb_common::throttled_log!(
                    Duration::from_secs(60),
                    warn,
                    "agent status collection failed: {error}"
                );
                None
            }
        };
        match status_tx.try_send((payload, state.clone())) {
            Ok(()) | Err(mpsc::TrySendError::Full(_)) => {}
            Err(mpsc::TrySendError::Disconnected(_)) => {
                hbb_common::throttled_log!(
                    Duration::from_secs(60),
                    warn,
                    "agent status worker unavailable"
                );
            }
        }
        match rescue_recovery::persist_before_package_skip(allow_legacy_package, || {
            write_state(&state_path, &state)
        }) {
            Ok(true) => continue,
            Ok(false) => {}
            Err(error) => {
                hbb_common::throttled_log!(
                    Duration::from_secs(60),
                    warn,
                    "agent observed state save failed: {error}"
                );
                continue;
            }
        }
        if recovery_client_present(&paths) {
            if let Err(error) = write_state(&state_path, &state) {
                hbb_common::throttled_log!(
                    Duration::from_secs(60),
                    warn,
                    "agent state save failed: {error}"
                );
            }
            continue;
        }
        if normal_client_present(&state) {
            let _ = write_state(&state_path, &state);
            continue;
        }
        if !recovery_allowed(state.management_agent_enabled && !state.disabled, false)
            || !check_due(&state)
        {
            continue;
        }
        if state.next_check_at.is_none() {
            schedule_initial_check(&mut state);
            let _ = write_state(&state_path, &state);
            continue;
        }
        if let Err(error) = check_now_with_package(
            config.edition,
            config.package_kind,
            &paths,
            config.normal_install_path.as_deref(),
        ) {
            hbb_common::log::error!("management agent recovery failed: {error}");
        }
    }
    status_handle.set_service_status(ServiceStatus {
        service_type: ServiceType::OWN_PROCESS,
        current_state: ServiceState::Stopped,
        controls_accepted: ServiceControlAccept::empty(),
        exit_code: ServiceExitCode::Win32(0),
        checkpoint: 0,
        wait_hint: Duration::default(),
        process_id: None,
    })?;
    Ok(())
}

#[cfg(windows)]
fn inspect_and_start_service(
    state: &mut AgentState,
    paths: &AgentPaths,
    elapsed: Duration,
) -> Result<(), String> {
    local_event_runtime::reconcile_running(state, paths);
    let observed = recovery_windows::inspect(state, paths)?;
    let intent = recovery_windows::read_intent()?;
    let mut budget = recovery::RetryBudget {
        failures: state.service_recovery_failures,
        next_at: state.service_recovery_next_at,
        healthy_since: state.service_healthy_since,
    };
    budget.observe_health(
        observed.state == recovery::ServiceState::Running && observed.trusted_path,
        unix_timestamp(),
    );
    state.service_recovery_failures = budget.failures;
    state.service_recovery_next_at = budget.next_at;
    state.service_healthy_since = budget.healthy_since;
    let decision = recovery::decide(&recovery::Observation {
        // New recovery is enabled only after the coordinated updater is installed and verified.
        enabled: state.management_agent_enabled
            && !state.disabled
            && paths.state.join("service-recovery-enabled").is_file(),
        locally_blocked: purge_marker_path(paths).exists() || paths.state.join("disable-agent").exists(),
        intent,
        maintenance: paths.state.join("maintenance.json").exists() || observed.disabled_start,
        startup_grace: elapsed < Duration::from_secs(180),
        retry_due: budget.due(unix_timestamp()),
        service: observed.state,
        trusted_service_path: observed.trusted_path,
        normal_available: observed.normal_present,
        fallback_available: observed.fallback_present,
    });
    match decision {
        recovery::Action::Observe => return Ok(()),
        recovery::Action::Start => {}
        recovery::Action::RepairNormal => {
            persist_recovery_attempt(state, paths, unix_timestamp())?;
            let result = recovery_windows::repair_service(state, paths, false);
            return persist_recovery_result(state, paths, result, unix_timestamp());
        }
        recovery::Action::RepairFallback => {
            persist_recovery_attempt(state, paths, unix_timestamp())?;
            let result = recovery_windows::repair_service(state, paths, true);
            return persist_recovery_result(state, paths, result, unix_timestamp());
        }
        recovery::Action::RecoverPackage => {
            // The service loop owns package recovery and its independently persisted
            // retry state. A nested check here would be overwritten by this snapshot.
            return Ok(());
        }
    }
    // Save the retry boundary before a side effect so process restarts cannot cause a start storm.
    budget.record_failure(unix_timestamp());
    state.service_recovery_failures = budget.failures;
    state.service_recovery_next_at = budget.next_at;
    state.service_healthy_since = budget.healthy_since;
    write_state(&paths.state.join("agent.json"), state).map_err(|error| error.to_string())?;
    let local_event = local_event_runtime::start_existing(state, paths);
    match local_event.action {
        Ok(true) => {
            state.last_service_recovery_at = Some(unix_timestamp().to_string());
            state.service_recovery_error.clear();
        }
        Ok(false) => {}
        Err(error) => {
            state.service_recovery_error = error;
        }
    }
    state.service_recovery_error = local_event_runtime::format_recovery_error(
        &state.service_recovery_error,
        local_event.reporting,
    );
    write_state(&paths.state.join("agent.json"), state).map_err(|error| error.to_string())
}

#[cfg(windows)]
fn persist_recovery_attempt(state: &mut AgentState, paths: &AgentPaths, now: u64) -> Result<(), String> {
    let mut budget = recovery::RetryBudget {
        failures: state.service_recovery_failures,
        next_at: state.service_recovery_next_at,
        healthy_since: state.service_healthy_since,
    };
    budget.record_failure(now);
    let mut next = state.clone();
    next.service_recovery_failures = budget.failures;
    next.service_recovery_next_at = budget.next_at;
    next.service_healthy_since = budget.healthy_since;
    write_state(&paths.state.join("agent.json"), &next).map_err(|error| error.to_string())?;
    *state = next;
    Ok(())
}

#[cfg(windows)]
fn persist_recovery_result(state: &mut AgentState, paths: &AgentPaths, result: Result<bool, String>, now: u64) -> Result<(), String> {
    let mut next = state.clone();
    match result {
        Ok(true) => {
            next.last_service_recovery_at = Some(now.to_string());
            next.service_recovery_error.clear();
        }
        Ok(false) => next.service_recovery_error = "local recovery action refused".into(),
        Err(error) => next.service_recovery_error = error,
    }
    write_state(&paths.state.join("agent.json"), &next).map_err(|error| error.to_string())?;
    *state = next;
    Ok(())
}

#[cfg(all(windows, not(test)))]
pub fn purge(paths: &AgentPaths) -> Result<(), String> {
    let dirs = system_dirs::get();
    if paths.root != dirs.rustdesk_data()
        || paths.state != dirs.agent().join("state") {
        return Err("purge requires fixed installed state".into());
    }
    workspace::verify_restore_target(&paths.state.join("agent.json"), false)?;
    if let Ok(bytes) = std::fs::read(paths.state.join("agent.json")) {
        if let Ok(state) = serde_json::from_slice::<serde_json::Value>(&bytes) {
            let edition = match state["edition"].as_str() {
                Some("standard") => Edition::Standard,
                Some("sos") => Edition::Sos,
                _ => return Err("purge edition unknown; refusing cleanup".into()),
            };
            let kind = match state["package_kind"].as_str() {
                Some("msi") => PackageKind::Msi,
                Some("exe") => PackageKind::Exe,
                _ => return Err("purge package kind unknown; refusing cleanup".into()),
            };
            if state["target_key"] != edition.target_key_for(kind) {
                return Err("purge state target mismatch".into());
            }
            return match state["package_kind"].as_str() {
                Some("msi") => purge_native::start(),
                Some("exe") => legacy_purge(paths),
                _ => Err("purge package kind unknown; refusing cleanup".into()),
            };
        }
    }
    Err("purge state unknown; refusing cleanup".into())
}

#[cfg(windows)]
fn legacy_purge(paths: &AgentPaths) -> Result<(), String> {
    use std::process::Command;
    if !unsafe { windows::Win32::UI::Shell::IsUserAnAdmin().as_bool() } {
        return Err("administrator privileges are required".to_owned());
    }
    ensure_agent_layout(paths).map_err(|e| e.to_string())?;
    let state_path = paths.state.join("agent.json");
    let normal_install_path = std::fs::read(&state_path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<AgentState>(&bytes).ok())
        .map(|state| state.normal_install_path)
        .filter(|path| !path.is_empty());
    std::fs::write(purge_marker_path(paths), b"disabled\n").map_err(|e| e.to_string())?;
    if paths.root.join("RecoveryBootstrap").exists() {
        bootstrap::remove_task()?;
    }
    let normal_install_path =
        normal_install_path.filter(|path| approved_normal_install_path(Path::new(path)));
    let _ = Command::new("sc.exe")
        .args(["stop", "RustDeskManagementAgent"])
        .status();
    let _ = Command::new("sc.exe")
        .args(["delete", "RustDeskManagementAgent"])
        .status();
    let script = format!(
        "$agent = Get-Service -Name 'RustDeskManagementAgent' -ErrorAction SilentlyContinue; if ($agent) {{ Stop-Service -InputObject $agent -Force -ErrorAction SilentlyContinue; try {{ $agent.WaitForStatus('Stopped', [TimeSpan]::FromSeconds(90)) }} catch {{ }} }}; & sc.exe delete RustDeskManagementAgent | Out-Null; Start-Sleep -Seconds 2; Stop-Service -Name 'RustDesk' -Force -ErrorAction SilentlyContinue; & sc.exe delete RustDesk | Out-Null; Remove-Item -LiteralPath {} -Recurse -Force -ErrorAction SilentlyContinue; {}",
        powershell_literal(&paths.root),
        normal_install_path
            .as_deref()
            .map(|path| format!("Remove-Item -LiteralPath {} -Recurse -Force -ErrorAction SilentlyContinue;", powershell_literal(Path::new(path))))
            .unwrap_or_default(),
    );
    Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            &script,
        ])
        .spawn()
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(windows)]
fn powershell_literal(path: &Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', "''"))
}

#[cfg(windows)]
fn approved_normal_install_path(path: &Path) -> bool {
    let normalized = path
        .to_string_lossy()
        .replace('/', "\\")
        .to_ascii_lowercase();
    let is_rustdesk_dir = path
        .file_name()
        .map(|name| name.to_string_lossy().eq_ignore_ascii_case("rustdesk"))
        .unwrap_or(false);
    let dirs = system_dirs::get();
    let under_program_files = [&dirs.program_files, &dirs.program_files_x86]
        .into_iter()
        .map(|root| {
            root.to_string_lossy()
                .replace('/', "\\")
                .to_ascii_lowercase()
        })
        .any(|root| normalized == root || normalized.starts_with(&format!("{root}\\")));
    is_rustdesk_dir && under_program_files
}

#[cfg(windows)]
pub fn cleanup(root: &Path, normal_install_path: Option<&Path>) -> Result<(), String> {
    std::thread::sleep(Duration::from_secs(2));
    let _ = std::process::Command::new("sc.exe")
        .args(["stop", "RustDesk"])
        .status();
    let _ = std::process::Command::new("sc.exe")
        .args(["delete", "RustDesk"])
        .status();
    if root.exists() {
        std::fs::remove_dir_all(root).map_err(|e| e.to_string())?;
    }
    if let Some(path) = normal_install_path {
        if path.exists() {
            std::fs::remove_dir_all(path).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

#[cfg(all(test, windows))]
#[path = "../tests/management_agent_status_contract/legacy_unknown_fields.rs"]
mod legacy_unknown_fields_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn target(primary: &str, bytes: &[u8]) -> ManifestTarget {
        ManifestTarget {
            version: "1.5.3".into(),
            build_seq: 1,
            build_number: "2026100501".into(),
            source_commit: "source-commit".into(),
            source_tag: "v1.5.3-build-2026.10.05-01".into(),
            primary: primary.into(),
            mirrors: vec![],
            size: bytes.len() as u64,
            sha256: hex::encode(Sha256::digest(bytes)),
            signature: "signed".into(),
            signature_key_id: "yan-release-2026".into(),
        }
    }

    #[test]
    fn edition_selects_stable_target_key_and_hidden_layout() {
        let paths = AgentPaths::from_program_data(r"C:\ProgramData", Edition::Sos);
        assert_eq!(Edition::Sos.target_key(), "windows-x86_64-msi-sos");
        assert_eq!(
            paths.client,
            PathBuf::from(r"C:\ProgramData/RustDesk/Client/sos/RustDesk")
        );
    }

    #[test]
    fn target_key_and_recovery_args_cover_msi_and_exe() {
        assert_eq!(
            Edition::Standard.target_key_for(PackageKind::Msi),
            "windows-x86_64-msi-standard"
        );
        assert_eq!(
            Edition::Sos.target_key_for(PackageKind::Exe),
            "windows-x86_64-exe-sos"
        );
        let paths = AgentPaths::from_program_data("/tmp", Edition::Standard);
        assert!(recovery_exe_args(&paths)
            .iter()
            .any(|arg| arg == "--management-agent-install"));
    }

    #[test]
    #[cfg(windows)]
    fn state_updates_keep_installation_and_generation_without_a_backup_rename_window() {
        let root = std::env::temp_dir().join(format!(
            "rustdesk-agent-state-{}-{}",
            std::process::id(),
            hex::encode(hbb_common::sodiumoxide::randombytes::randombytes(8))
        ));
        let paths = AgentPaths::from_program_data(&root, Edition::Standard);
        let path = paths.state.join("agent.json");
        let mut state = AgentState::new(Edition::Standard, &paths);
        state.agent_installation_id = "a".repeat(32);
        for generation in 1..=3 {
            state.status_session_generation = generation;
            write_state(&path, &state).unwrap();
            let saved: AgentState = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
            assert_eq!(saved, state);
            assert!(!path.with_extension(format!("bak-{}", std::process::id())).exists());
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn stale_session_generation_advances_forward_only_and_persists() {
        let root = std::env::temp_dir().join(format!(
            "rustdesk-agent-stale-generation-{}-{}",
            std::process::id(),
            hex::encode(hbb_common::sodiumoxide::randombytes::randombytes(8))
        ));
        let paths = AgentPaths::from_program_data(&root, Edition::Standard);
        let path = paths.state.join("agent.json");
        let saved = |path: &Path| -> AgentState {
            serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
        };
        let mut state = AgentState::new(Edition::Standard, &paths);
        state.agent_installation_id = "a".repeat(32);
        // 回退：本地 54、后台 60。
        state.status_session_generation = 54;
        write_state(&path, &state).unwrap();
        assert_eq!(apply_stale_session_generation(&mut state, &path, 60).unwrap(), Some(61));
        assert_eq!(state.status_session_generation, 61);
        assert_eq!(saved(&path).status_session_generation, 61);
        assert_eq!(saved(&path).agent_installation_id, "a".repeat(32));
        // 相等：本地与后台都是 61 也要越过。
        assert_eq!(apply_stale_session_generation(&mut state, &path, 61).unwrap(), Some(62));
        assert_eq!(saved(&path).status_session_generation, 62);
        // 本地已领先（迟到的旧 409）：不后退、不写盘。
        let before = std::fs::read(&path).unwrap();
        assert_eq!(apply_stale_session_generation(&mut state, &path, 60).unwrap(), None);
        assert_eq!(apply_stale_session_generation(&mut state, &path, u64::MAX).unwrap(), None);
        assert_eq!(state.status_session_generation, 62);
        assert_eq!(std::fs::read(&path).unwrap(), before);
        // 写盘失败时内存状态保持原值。
        std::fs::remove_dir_all(&root).unwrap();
        std::fs::create_dir_all(&paths.agent).unwrap();
        std::fs::write(&paths.state, b"not-a-directory").unwrap();
        assert!(apply_stale_session_generation(&mut state, &path, 70).is_err());
        assert_eq!(state.status_session_generation, 62);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn old_agent_state_migrates_to_enabled_msi_defaults() {
        let value = serde_json::json!({
            "edition": "standard",
            "target_key": "windows-x86_64-msi-standard",
            "manifest_url": MANIFEST_URL
        });
        let state: AgentState = serde_json::from_value(value).expect("legacy state should parse");
        assert_eq!(state.schema_version, 1);
        assert_eq!(state.package_kind, PackageKind::Msi);
        assert!(state.management_agent_enabled);
    }

    #[test]
    fn local_disable_marker_wins_over_api_enable() {
        assert!(!recovery_allowed(false, false));
        assert!(!recovery_allowed(true, true));
        assert!(recovery_allowed(true, false));
    }

    #[test]
    fn service_recovery_marker_requires_enabled_intent_and_is_idempotent() {
        let root = std::env::temp_dir().join(format!(
            "rustdesk-agent-recovery-marker-{}-{}",
            std::process::id(),
            hex::encode(hbb_common::sodiumoxide::randombytes::randombytes(8))
        ));
        let paths = AgentPaths::from_program_data(&root, Edition::Standard);
        ensure_agent_layout(&paths).unwrap();
        assert!(!enable_service_recovery(&paths, recovery::Intent::Disabled).unwrap());
        assert!(!paths.state.join("service-recovery-enabled").exists());
        assert!(!enable_service_recovery(&paths, recovery::Intent::Unknown).unwrap());
        assert!(!paths.state.join("service-recovery-enabled").exists());
        assert!(enable_service_recovery(&paths, recovery::Intent::Enabled).unwrap());
        assert!(!enable_service_recovery(&paths, recovery::Intent::Enabled).unwrap());
        std::fs::write(paths.state.join("disable-agent"), b"disabled\n").unwrap();
        std::fs::remove_file(paths.state.join("service-recovery-enabled")).unwrap();
        assert!(!enable_service_recovery(&paths, recovery::Intent::Enabled).unwrap());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    #[cfg(windows)]
    fn local_status_rejects_missing_or_corrupt_state_without_replacing_it() {
        let root = std::env::temp_dir().join(format!("agent-status-readonly-{}", std::process::id()));
        let paths = AgentPaths::from_program_data(&root, Edition::Standard);
        assert!(local_status(&paths).unwrap_err().starts_with("agent state unavailable"));
        assert!(!paths.state.exists());
        std::fs::create_dir_all(&paths.state).unwrap();
        let path = paths.state.join("agent.json");
        std::fs::write(&path, b"invalid-state").unwrap();
        assert_eq!(local_status(&paths).unwrap_err(), "agent state malformed");
        assert_eq!(std::fs::read(&path).unwrap(), b"invalid-state");
        let large = vec![b'x'; 65537];
        std::fs::write(&path, &large).unwrap();
        assert_eq!(local_status(&paths).unwrap_err(), "agent state too large");
        assert_eq!(std::fs::read(&path).unwrap(), large);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    #[cfg(windows)]
    fn repair_attempt_persists_cooldown_and_failure_without_erasing_identity() {
        let root = std::env::temp_dir().join(format!("agent-repair-budget-{}", std::process::id()));
        let paths = AgentPaths::from_program_data(&root, Edition::Standard);
        std::fs::create_dir_all(&paths.state).unwrap();
        let mut state = AgentState::new(Edition::Standard, &paths);
        state.agent_installation_id = "a".repeat(32);
        state.status_session_generation = 7;
        let original = state.agent_installation_id.clone();
        persist_recovery_attempt(&mut state, &paths, 1000).unwrap();
        let bytes = std::fs::read(paths.state.join("agent.json")).unwrap();
        let before: AgentState = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(before.service_recovery_failures, 1);
        assert_eq!(before.service_recovery_next_at, 1030);
        persist_recovery_result(&mut state, &paths, Err("fixture repair failed".into()), 1001).unwrap();
        let failed: AgentState = serde_json::from_slice(&std::fs::read(paths.state.join("agent.json")).unwrap()).unwrap();
        assert_eq!(failed.service_recovery_error, "fixture repair failed");
        assert_eq!(failed.service_recovery_next_at, 1030);
        assert_eq!(failed.agent_installation_id, original);
        assert_eq!(failed.status_session_generation, 7);
        persist_recovery_result(&mut state, &paths, Ok(false), 1002).unwrap();
        assert_eq!(state.service_recovery_error, "local recovery action refused");
        persist_recovery_result(&mut state, &paths, Ok(true), 1003).unwrap();
        assert_eq!(state.last_service_recovery_at, Some("1003".into()));
        assert!(state.service_recovery_error.is_empty());
        assert_eq!(state.service_recovery_failures, 1);
        let persisted = std::fs::read(paths.state.join("agent.json")).unwrap();
        let unchanged = serde_json::to_vec(&state).unwrap();
        use std::os::windows::fs::OpenOptionsExt;
        let locked = std::fs::OpenOptions::new().read(true).share_mode(1)
            .open(paths.state.join("agent.json")).unwrap();
        assert!(persist_recovery_attempt(&mut state, &paths, 1030).is_err());
        assert_eq!(serde_json::to_vec(&state).unwrap(), unchanged);
        assert_eq!(std::fs::read(paths.state.join("agent.json")).unwrap(), persisted);
        assert!(persist_recovery_result(&mut state, &paths, Ok(true), 1040).is_err());
        assert_eq!(serde_json::to_vec(&state).unwrap(), unchanged);
        assert_eq!(std::fs::read(paths.state.join("agent.json")).unwrap(), persisted);
        drop(locked);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    #[cfg(windows)]
    fn local_package_blocks_fail_before_layout_network_or_state_mutation() {
        let root = std::env::temp_dir().join(format!("agent-package-preflight-{}", std::process::id()));
        let paths = AgentPaths::from_program_data(&root, Edition::Standard);
        std::fs::create_dir_all(&paths.state).unwrap();
        let state_file = paths.state.join("agent.json");
        std::fs::write(&state_file, b"preserve-original-state").unwrap();
        for marker in ["disable-agent", "maintenance.json", "disable-recovery"] {
            let path = paths.state.join(marker);
            std::fs::write(&path, b"preserve-local-intent").unwrap();
            assert!(package_recovery_preflight(&paths).is_err(), "{marker} must block");
            assert_eq!(check_now_with_package(Edition::Standard, PackageKind::Msi, &paths, None).unwrap_err(), "package recovery locally blocked");
            assert_eq!(std::fs::read(&path).unwrap(), b"preserve-local-intent");
            assert_eq!(std::fs::read(&state_file).unwrap(), b"preserve-original-state");
            assert!(!paths.cache.exists());
            std::fs::remove_file(path).unwrap();
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    #[cfg(windows)]
    fn disabled_package_state_is_preserved_before_policy_refresh() {
        let root = std::env::temp_dir().join(format!("agent-disabled-package-state-{}", std::process::id()));
        let paths = AgentPaths::from_program_data(&root, Edition::Standard);
        std::fs::create_dir_all(&paths.state).unwrap();
        for disabled in [false, true] {
            let mut state = AgentState::new(Edition::Standard, &paths);
            state.agent_installation_id = "a".repeat(32);
            state.disabled = disabled;
            state.management_agent_enabled = disabled;
            state.last_policy_error = "preserve-previous-error".into();
            let bytes = serde_json::to_vec(&state).unwrap();
            let file = paths.state.join("agent.json");
            std::fs::write(&file, &bytes).unwrap();
            assert_eq!(check_now_with_package(Edition::Standard, PackageKind::Msi, &paths, None).unwrap_err(), "management agent is disabled");
            assert_eq!(std::fs::read(&file).unwrap(), bytes);
            assert!(!paths.cache.exists());
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn manifest_must_use_oss_primary() {
        let mut targets = BTreeMap::new();
        targets.insert(
            Edition::Standard.target_key().into(),
            target("https://github.com/x", b"x"),
        );
        let manifest = Manifest {
            schema: 2,
            catalog_revision: 2026100501,
            version: "1.5.3".into(),
            build_number: "20261005.1".into(),
            build_seq: 2026100501,
            product: "rustdesk".into(),
            edition: "multi".into(),
            channel: "stable".into(),
            source_commit: "source-commit".into(),
            targets,
        };
        assert!(matches!(
            select_oss_target(&manifest, Edition::Standard),
            Err(ManifestError::InvalidPrimary(_))
        ));
    }

    #[test]
    fn download_integrity_and_retry_schedule_are_strict() {
        let bytes = b"msi";
        let t = target("https://download.yan.life/rustdesk.msi", bytes);
        assert!(verify_download(bytes, &t).is_ok());
        assert!(matches!(
            verify_download(b"no", &t),
            Err(ManifestError::InvalidSize)
        ));
        assert_eq!(retry_delay(0), Duration::from_secs(300));
        assert_eq!(retry_delay(1), Duration::from_secs(900));
        assert_eq!(retry_delay(2), Duration::from_secs(1800));
        assert_eq!(retry_delay(3), Duration::from_secs(3600));
    }

    #[test]
    fn failure_caps_at_hourly_retry_and_success_resets_state() {
        let paths = AgentPaths::from_program_data("/tmp", Edition::Standard);
        let mut state = AgentState::new(Edition::Standard, &paths);
        for _ in 0..8 {
            record_failure(&mut state, "network");
        }
        assert_eq!(state.retry_stage, 3);
        let t = target("https://download.yan.life/rustdesk.msi", b"ok");
        record_success(&mut state, &t);
        assert_eq!(state.retry_stage, 0);
        assert!(state.last_error.is_empty());
    }

    #[test]
    fn recovery_msi_writes_fixed_verbose_log_under_agent_logs() {
        let paths = AgentPaths::from_program_data(r"C:\ProgramData", Edition::Standard);
        assert_eq!(
            recovery_msi_log_args(&paths),
            vec![
                "/L*v".to_owned(),
                paths.logs.join("recovery-msi.log").display().to_string(),
            ]
        );
        assert!(paths
            .logs
            .join("recovery-msi.log")
            .starts_with(&paths.agent));
        let original = recovery_msi_args(&paths);
        assert!(!original.iter().any(|argument| argument.starts_with("/L")));
    }

    #[test]
    fn recovery_args_disable_all_user_entrypoints() {
        let args = recovery_msi_args(&AgentPaths::from_program_data("/tmp", Edition::Standard));
        assert!(args.iter().any(|a| a == "STARTMENUSHORTCUTS=0"));
        assert!(args.iter().any(|a| a == "DESKTOPSHORTCUTS=0"));
        assert!(args.iter().any(|a| a == "STARTUPSHORTCUTS=0"));
        assert!(args.iter().any(|a| a == "LAUNCH_TRAY_APP=N"));
    }

    #[test]
    fn service_arguments_select_edition_and_normal_path() {
        let config = parse_service_arguments([
            "--service",
            "--edition",
            "sos",
            "--package-kind",
            "exe",
            "--normal-install-path",
            r"C:\Program Files\RustDesk\",
        ])
        .expect("service arguments should parse");
        assert_eq!(config.edition, Edition::Sos);
        assert_eq!(config.package_kind, PackageKind::Exe);
        assert_eq!(
            config.normal_install_path,
            Some(PathBuf::from(r"C:\Program Files\RustDesk\"))
        );
    }

    #[test]
    fn service_arguments_reject_missing_or_unknown_values() {
        assert!(parse_service_arguments(["--edition"]).is_err());
        assert!(parse_service_arguments(["--edition", "enterprise"]).is_err());
        assert!(parse_service_arguments(["--package-kind", "zip"]).is_err());
        assert!(parse_service_arguments(["--normal-install-path"]).is_err());
        let quoted = parse_service_arguments([
            "--normal-install-path",
            r#"C:\Program Files\RustDesk"\"#,
        ])
        .expect("quoted trailing separator should parse");
        assert_eq!(
            quoted.normal_install_path,
            Some(PathBuf::from(r"C:\Program Files\RustDesk"))
        );
    }

    #[test]
    fn service_config_sync_keeps_edition_package_target_and_recovery_path_aligned() {
        let paths = AgentPaths::from_program_data("/tmp/program-data", Edition::Sos);
        let mut state = AgentState::new(Edition::Standard, &paths);
        let config = ServiceConfig {
            edition: Edition::Sos,
            package_kind: PackageKind::Exe,
            normal_install_path: Some(PathBuf::from(r"C:\Program Files\RustDesk")),
        };

        apply_service_config(&mut state, &config, &paths);

        assert_eq!(state.edition, Edition::Sos);
        assert_eq!(state.package_kind, PackageKind::Exe);
        assert_eq!(state.target_key, "windows-x86_64-exe-sos");
        assert_eq!(
            state.recovery_install_path,
            paths.client.to_string_lossy().as_ref()
        );
        assert_eq!(state.normal_install_path, r"C:\Program Files\RustDesk");
    }

    #[test]
    fn trailing_separator_stays_inside_normal_install_dir() {
        let exe = normal_client_exe(r"C:\Program Files\RustDesk\").unwrap();
        assert_eq!(exe, PathBuf::from(r"C:\Program Files\RustDesk\RustDesk.exe"));
        assert_eq!(
            normal_client_exe(r"C:\Program Files\RustDesk\\\").unwrap(),
            PathBuf::from(r"C:\Program Files\RustDesk\RustDesk.exe")
        );
        assert!(normal_client_exe("  ").is_none());
    }

    #[test]
    fn agent_policy_requires_matching_identity_and_monotonic_revision() {
        let paths = AgentPaths::from_program_data("/tmp", Edition::Standard);
        let mut state = AgentState::new(Edition::Standard, &paths);
        let policy = AgentPolicyResponse {
            client_id: "client".into(),
            client_uuid: "uuid".into(),
            policy_revision: 2,
            management_agent_enabled: false,
        };
        assert!(apply_agent_policy(&mut state, policy.clone(), "other", "uuid").is_err());
        assert!(apply_agent_policy(&mut state, policy, "client", "uuid").is_ok());
        assert!(!state.management_agent_enabled);
        assert_eq!(state.management_agent_policy_revision, 2);
        let rollback = AgentPolicyResponse {
            client_id: "client".into(),
            client_uuid: "uuid".into(),
            policy_revision: 1,
            management_agent_enabled: true,
        };
        assert!(apply_agent_policy(&mut state, rollback, "client", "uuid").is_err());
        assert!(!state.management_agent_enabled);
    }

    #[test]
    fn failures_schedule_the_next_retry_and_success_clears_it() {
        let paths = AgentPaths::from_program_data("/tmp", Edition::Standard);
        let mut state = AgentState::new(Edition::Standard, &paths);
        record_failure(&mut state, "network");
        assert_eq!(state.retry_stage, 1);
        let next = state
            .next_check_at
            .as_deref()
            .and_then(|value| value.parse::<u64>().ok())
            .expect("failure must schedule retry");
        assert!(next >= unix_timestamp() + 4 * 60);
        record_failure(&mut state, "network");
        let next = state
            .next_check_at
            .as_deref()
            .and_then(|value| value.parse::<u64>().ok())
            .expect("second failure must schedule retry");
        assert!(next >= unix_timestamp() + 14 * 60);
        record_success(
            &mut state,
            &target("https://download.yan.life/rustdesk.msi", b"ok"),
        );
        assert_eq!(state.next_check_at, None);
    }

    #[test]
    fn signature_rejects_untrusted_key_and_target_mismatch() {
        let mut targets = BTreeMap::new();
        let mut signed = target("https://download.yan.life/rustdesk.msi", b"ok");
        signed.signature = "AAAA".into();
        targets.insert(Edition::Standard.target_key().into(), signed.clone());
        let manifest = Manifest {
            schema: 2,
            catalog_revision: 2026100501,
            version: "1.5.3".into(),
            build_number: "20261005.1".into(),
            build_seq: 2026100501,
            product: "rustdesk".into(),
            edition: "standard".into(),
            channel: "stable".into(),
            source_commit: "source-commit".into(),
            targets,
        };
        assert!(matches!(
            verify_manifest_signature(&manifest, Edition::Standard.target_key(), &signed),
            Err(ManifestError::InvalidManifestSignature(_))
        ));
        let mut untrusted = signed;
        untrusted.signature_key_id = "other-key".into();
        assert!(matches!(
            verify_manifest_signature(&manifest, Edition::Standard.target_key(), &untrusted),
            Err(ManifestError::InvalidSignature)
        ));
    }
}
