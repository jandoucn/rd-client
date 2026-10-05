//! Windows management-agent policy and recovery primitives.
//!
//! The state machine and manifest handling are platform independent so they can be
//! tested on CI hosts which are not Windows. Windows service plumbing belongs in
//! the binary entry point and is deliberately kept behind `cfg(windows)`.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::time::Duration;

use hbb_common::base64::{
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
    Engine as _,
};

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

impl Edition {
    pub fn target_key(self) -> &'static str {
        match self {
            Self::Standard => "windows-x86_64-msi-standard",
            Self::Sos => "windows-x86_64-msi-sos",
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
    pub edition: Edition,
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
}

impl AgentState {
    pub fn new(edition: Edition, paths: &AgentPaths) -> Self {
        Self {
            edition,
            target_key: edition.target_key().to_owned(),
            manifest_url: MANIFEST_URL.to_owned(),
            normal_install_path: String::new(),
            recovery_install_path: paths.client.to_string_lossy().into_owned(),
            next_check_at: None,
            retry_stage: 0,
            last_version: String::new(),
            last_primary_url: String::new(),
            last_error: String::new(),
            disabled: false,
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
    pub normal_install_path: Option<PathBuf>,
}

impl Default for ServiceConfig {
    fn default() -> Self {
        Self {
            edition: Edition::Standard,
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
            "--normal-install-path" => {
                let value = arguments
                    .get(index + 1)
                    .ok_or_else(|| "--normal-install-path requires a value".to_owned())?;
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
    let target = manifest
        .targets
        .get(edition.target_key())
        .ok_or_else(|| ManifestError::MissingTarget(edition.target_key().to_owned()))?;
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

pub fn recovery_msi_args(paths: &AgentPaths) -> Vec<String> {
    vec![
        "/qn".into(),
        "/norestart".into(),
        format!("INSTALLFOLDER={}", paths.client.display()),
        "STARTMENUSHORTCUTS=0".into(),
        "DESKTOPSHORTCUTS=0".into(),
        "STARTUPSHORTCUTS=0".into(),
        "LAUNCH_TRAY_APP=N".into(),
    ]
}

pub fn purge_marker_path(paths: &AgentPaths) -> PathBuf {
    paths.state.join("disable-recovery")
}

pub fn recovery_client_present(paths: &AgentPaths) -> bool {
    paths.client.join("RustDesk.exe").is_file()
}

pub fn normal_client_present(state: &AgentState) -> bool {
    if state.normal_install_path.is_empty() {
        return false;
    }
    Path::new(&state.normal_install_path)
        .join("RustDesk.exe")
        .is_file()
}

#[cfg(windows)]
fn discover_normal_install_path() -> Option<PathBuf> {
    ["ProgramFiles", "ProgramFiles(x86)"]
        .into_iter()
        .filter_map(std::env::var_os)
        .map(PathBuf::from)
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

fn unix_timestamp() -> u64 {
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

pub fn write_state(path: &Path, state: &AgentState) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let bytes = serde_json::to_vec_pretty(state).map_err(std::io::Error::other)?;
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

#[cfg(windows)]
pub fn check_now(edition: Edition, paths: &AgentPaths) -> Result<(), String> {
    check_now_with_normal_path(edition, paths, None)
}

#[cfg(windows)]
pub fn check_now_with_normal_path(
    edition: Edition,
    paths: &AgentPaths,
    normal_install_path: Option<&Path>,
) -> Result<(), String> {
    ensure_agent_layout(paths).map_err(|error| error.to_string())?;
    let state_path = paths.state.join("agent.json");
    let mut state = if state_path.exists() {
        let value = std::fs::read(&state_path).map_err(|e| e.to_string())?;
        serde_json::from_slice::<AgentState>(&value).map_err(|e| e.to_string())?
    } else {
        AgentState::new(edition, paths)
    };
    state.edition = edition;
    state.target_key = edition.target_key().to_owned();
    if state.disabled || purge_marker_path(paths).exists() {
        return Err("recovery is disabled".to_owned());
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
        let target =
            select_oss_target(&manifest, edition).map_err(|e| format!("manifest: {e:?}"))?;
        verify_manifest_signature(&manifest, edition.target_key(), target)
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
        let cache = paths
            .cache
            .join(format!("rustdesk-{}-latest.msi", edition.name()));
        let temporary = cache.with_extension(format!("download-{}", std::process::id()));
        std::fs::write(&temporary, &bytes).map_err(|e| e.to_string())?;
        if cache.exists() {
            std::fs::remove_file(&cache).map_err(|e| e.to_string())?;
        }
        std::fs::rename(&temporary, &cache).map_err(|e| e.to_string())?;
        #[cfg(windows)]
        set_hidden_system_attribute(&cache).map_err(|e| e.to_string())?;
        let status = std::process::Command::new("msiexec.exe")
            .arg("/i")
            .arg(&cache)
            .args(recovery_msi_args(paths))
            .status()
            .map_err(|e| e.to_string())?;
        if !status.success() {
            return Err(format!("msiexec exited with {status}"));
        }
        record_success(&mut state, target);
        Ok(())
    })();
    if let Err(error) = result {
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
windows_service::define_windows_service!(ffi_service_main, service_main);

#[cfg(windows)]
fn service_main(_arguments: Vec<std::ffi::OsString>) -> windows_service::Result<()> {
    use std::sync::mpsc;
    use std::time::Duration;
    use windows_service::service::{
        ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus,
        ServiceType,
    };
    use windows_service::service_control_handler::{self, ServiceControlHandlerResult};
    let program_data = std::env::var_os("ProgramData").unwrap_or_else(|| "C:\\ProgramData".into());
    let config = parse_service_arguments(
        _arguments
            .into_iter()
            .map(|argument| argument.to_string_lossy().into_owned()),
    )
    .map_err(|error| {
        windows_service::Error::Winapi(std::io::Error::new(std::io::ErrorKind::InvalidInput, error))
    })?;
    let paths = AgentPaths::from_program_data(program_data, config.edition);
    let (stop_tx, stop_rx) = mpsc::channel();
    let status_handle = service_control_handler::register(
        "RustDeskManagementAgent",
        move |control| match control {
            ServiceControl::Stop | ServiceControl::Shutdown => {
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
    loop {
        if stop_rx.recv_timeout(Duration::from_secs(30)).is_ok() {
            break;
        }
        if purge_marker_path(&paths).exists() || recovery_client_present(&paths) {
            continue;
        }
        let state_path = paths.state.join("agent.json");
        let mut state = std::fs::read(&state_path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<AgentState>(&bytes).ok())
            .unwrap_or_else(|| AgentState::new(config.edition, &paths));
        if let Some(path) = config.normal_install_path.as_deref() {
            state.normal_install_path = path.to_string_lossy().into_owned();
        }
        if state.normal_install_path.is_empty() {
            if let Some(path) = discover_normal_install_path() {
                state.normal_install_path = path.to_string_lossy().into_owned();
            }
        }
        if normal_client_present(&state) {
            let _ = write_state(&state_path, &state);
            continue;
        }
        if state.disabled || !check_due(&state) {
            continue;
        }
        if state.next_check_at.is_none() {
            schedule_initial_check(&mut state);
            let _ = write_state(&state_path, &state);
            continue;
        }
        if let Err(error) = check_now_with_normal_path(
            config.edition,
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
pub fn purge(paths: &AgentPaths) -> Result<(), String> {
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
    let under_program_files = ["ProgramFiles", "ProgramFiles(x86)"]
        .into_iter()
        .filter_map(std::env::var_os)
        .map(|root| {
            PathBuf::from(root)
                .to_string_lossy()
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
            "--normal-install-path",
            r"C:\Program Files\RustDesk\",
        ])
        .expect("service arguments should parse");
        assert_eq!(config.edition, Edition::Sos);
        assert_eq!(
            config.normal_install_path,
            Some(PathBuf::from(r"C:\Program Files\RustDesk\"))
        );
    }

    #[test]
    fn service_arguments_reject_missing_or_unknown_values() {
        assert!(parse_service_arguments(["--edition"]).is_err());
        assert!(parse_service_arguments(["--edition", "enterprise"]).is_err());
        assert!(parse_service_arguments(["--normal-install-path"]).is_err());
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
