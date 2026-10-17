use super::{
    workspace, Binding, Channel, Outbox, PreparationError, Terminal, VerifiedRunningTarget, ROOT,
};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

#[path = "../../management_agent/handoff.rs"]
mod handoff;
#[path = "../../management_agent/health.rs"]
mod health;
#[path = "../../management_agent/health_windows.rs"]
mod health_windows;
use super::supervisor::guard as install_guard;

const INSTALLED: &str = r"C:\Program Files\RustDesk\RustDesk.exe";
static STARTED: AtomicBool = AtomicBool::new(false);
const ORDINARY_MSI_TRANSACTIONS_ENABLED: bool = false;

pub(crate) fn use_receipt_path() -> Result<bool, String> {
    refuse_pending_at(Path::new(ROOT))?;
    if !ORDINARY_MSI_TRANSACTIONS_ENABLED {
        return Ok(false);
    }
    install_allowed()?;
    receipt_path_at(Path::new(r"C:\ProgramData\RustDesk\ManagementAgent\state"))
}

fn refuse_pending_at(root: &Path) -> Result<(), String> {
    match fs::symlink_metadata(root) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.to_string()),
        Ok(_) => workspace::verify(root)?,
    }
    for entry in fs::read_dir(root).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let name = entry.file_name();
        let id = name.to_str().ok_or("ordinary transaction id invalid")?;
        let outbox = Outbox::open_at(root, id)?;
        match fs::symlink_metadata(outbox.directory.join("terminal.json")) {
            Ok(_) => {
                if outbox.pending(Channel::Update)?.is_some()
                    || (outbox.binding().command_id.is_some()
                        && outbox.pending(Channel::Command)?.is_some())
                {
                    return Err("ordinary prior installation requires reconciliation".into());
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                if outbox.blocks_preparation_retry() {
                    return Err("ordinary prior installation requires reconciliation".into());
                }
            }
            Err(error) => return Err(error.to_string()),
        }
    }
    Ok(())
}

fn receipt_path_at(directory: &Path) -> Result<bool, String> {
    let path = directory.join("agent.json");
    workspace::verify_restore_target(&path, false)?;
    for name in ["disable-recovery", "disable-agent", "maintenance.json"] {
        match fs::symlink_metadata(directory.join(name)) {
            Ok(_) => return Err("ordinary local maintenance/disable gate refused".into()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.to_string()),
        }
    }
    let mut bytes = Vec::new();
    use std::os::windows::fs::OpenOptionsExt;
    fs::OpenOptions::new()
        .read(true)
        .share_mode(1)
        .custom_flags(0x00200000)
        .open(&path)
        .map_err(|e| e.to_string())?
        .take(65537)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > 65536 {
        return Err("ordinary installation state too large".into());
    }
    #[derive(serde::Deserialize)]
    struct RouteState {
        #[serde(default = "schema_one")]
        schema_version: u32,
        edition: String,
        #[serde(default = "msi_kind")]
        package_kind: String,
        target_key: String,
        manifest_url: String,
        #[serde(default)]
        disabled: bool,
        #[serde(default = "enabled")]
        management_agent_enabled: bool,
    }
    fn schema_one() -> u32 {
        1
    }
    fn msi_kind() -> String {
        "msi".into()
    }
    fn enabled() -> bool {
        true
    }
    let state: RouteState =
        serde_json::from_slice(&bytes).map_err(|_| "ordinary installation state malformed")?;
    if state.schema_version != 1
        || state.edition != crate::EDITION
        || state.package_kind != "msi"
        || state.target_key != format!("windows-x86_64-msi-{}", crate::EDITION)
        || state.manifest_url != "https://download.yan.life/rustdesk/stable/manifest.json"
    {
        return Err("ordinary installation state invalid".into());
    }
    if state.disabled || !state.management_agent_enabled {
        return Err("ordinary installation state disabled".into());
    }
    let fields: serde_json::Map<String, serde_json::Value> =
        serde_json::from_slice(&bytes).map_err(|_| "ordinary installation state malformed")?;
    for name in [
        "normal_install_path",
        "recovery_install_path",
        "last_version",
        "last_primary_url",
        "last_error",
        "last_policy_error",
        "service_recovery_error",
    ] {
        if fields.get(name).is_some_and(|value| !value.is_string()) {
            return Err("ordinary installation state malformed".into());
        }
    }
    for name in [
        "next_check_at",
        "last_policy_check_at",
        "last_service_recovery_at",
    ] {
        if fields
            .get(name)
            .is_some_and(|value| !value.is_null() && !value.is_string())
        {
            return Err("ordinary installation state malformed".into());
        }
    }
    for (name, max) in [
        ("retry_stage", 255),
        ("service_recovery_failures", 255),
        ("management_agent_policy_revision", u64::MAX),
        ("service_recovery_next_at", u64::MAX),
        ("status_session_generation", u64::MAX),
    ] {
        if fields
            .get(name)
            .is_some_and(|value| !value.as_u64().is_some_and(|v| v <= max))
        {
            return Err("ordinary installation state malformed".into());
        }
    }
    if fields
        .get("service_healthy_since")
        .is_some_and(|v| !v.is_null() && v.as_u64().is_none())
    {
        return Err("ordinary installation state malformed".into());
    }
    match fields.get("agent_installation_id") {
        None => Ok(false),
        Some(value)
            if value.as_str().is_some_and(|id| {
                id.len() == 32
                    && id
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            }) =>
        {
            Ok(true)
        }
        Some(_) => Err("ordinary installation identity mismatch".into()),
    }
}

fn instance() -> Result<String, String> {
    let path = Path::new(r"C:\ProgramData\RustDesk\ManagementAgent\state\agent.json");
    workspace::verify_restore_target(path, false)?;
    let mut bytes = Vec::new();
    use std::os::windows::fs::OpenOptionsExt;
    fs::OpenOptions::new()
        .read(true)
        .share_mode(1)
        .custom_flags(0x00200000)
        .open(path)
        .map_err(|e| e.to_string())?
        .take(65537)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > 65536 {
        return Err("ordinary installation state too large".into());
    }
    let state: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|_| "ordinary installation state malformed")?;
    let id = state["agent_installation_id"]
        .as_str()
        .ok_or("ordinary installation id absent")?;
    if id.len() != 32
        || !id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        || state["edition"] != crate::EDITION
        || state["package_kind"] != "msi"
    {
        return Err("ordinary installation identity mismatch".into());
    }
    Ok(id.into())
}

pub(super) fn install_allowed() -> Result<(), String> {
    if hbb_common::config::is_disable_installation()
        || hbb_common::config::Config::get_option("stop-service") == "Y"
    {
        return Err("ordinary local installation policy refused".into());
    }
    let state = Path::new(r"C:\ProgramData\RustDesk\ManagementAgent\state");
    for name in ["disable-recovery", "disable-agent", "maintenance.json"] {
        match fs::symlink_metadata(state.join(name)) {
            Ok(_) => return Err("ordinary local maintenance/disable gate refused".into()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.to_string()),
        }
    }
    Ok(())
}

fn copy_locked(
    source: &Path,
    destination: &Path,
    expected: Option<&str>,
    stage: &str,
) -> Result<(), PreparationError> {
    use std::os::windows::fs::OpenOptionsExt;
    let mut input = fs::OpenOptions::new()
        .read(true)
        .share_mode(1)
        .custom_flags(0x00200000)
        .open(source)
        .map_err(|e| PreparationError::io(stage, e))?;
    let mut output = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)
        .map_err(|e| PreparationError::io(stage, e))?;
    let mut hash = Sha256::new();
    let mut buffer = [0; 65536];
    loop {
        let n = input
            .read(&mut buffer)
            .map_err(|e| PreparationError::io(stage, e))?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
        output
            .write_all(&buffer[..n])
            .map_err(|e| PreparationError::io(stage, e))?;
    }
    output
        .sync_all()
        .map_err(|e| PreparationError::io(stage, e))?;
    if expected.is_some_and(|expected| expected != hex::encode(hash.finalize())) {
        return Err(PreparationError::refused(stage));
    }
    workspace::verify_file(destination).map_err(|_| PreparationError::refused(stage))
}

pub(crate) fn install(
    package: &Path,
    approved_name: &str,
    package_sha256: &str,
    target_key: &str,
    version: &str,
    build: u64,
    source: &str,
    command_id: Option<String>,
) -> Result<(), String> {
    super::admission::check_installed(package)?;
    let package_name = super::package_name::validate(approved_name)?;
    super::validate_windows_package_basename(&package_name)?;
    if hbb_common::config::is_disable_installation() {
        return Err("ordinary installation disabled".into());
    }
    install_allowed()?;
    let _guard = install_guard::InstallGuard::try_acquire()
        .map_err(|e| e.to_string())?
        .ok_or("ordinary installation busy")?;
    super::admission::check_installed(package)?;
    let current = std::env::current_exe().map_err(|e| e.to_string())?;
    if !current.to_string_lossy().eq_ignore_ascii_case(INSTALLED) {
        return Err("ordinary writer requires fixed installed image".into());
    }
    let identity = super::super::current_update_client_identity();
    if let Some(id) = command_id.as_deref() {
        if owns_command(id) {
            return Ok(());
        }
    }
    if let Ok(entries) = fs::read_dir(ROOT) {
        for entry in entries.filter_map(Result::ok) {
            if let Some(existing) = entry
                .file_name()
                .to_str()
                .and_then(|id| Outbox::open_existing(id).ok())
            {
                if existing.binding.client_id == identity.client_id
                    && existing.binding.client_uuid == identity.client_uuid
                    && !existing.directory.join("terminal.json").exists()
                    && existing.blocks_preparation_retry()
                {
                    return Err("ordinary prior installation requires reconciliation".into());
                }
            }
        }
    }
    let binding = super::package_contract::binding(super::package_contract::BindingFields {
        transaction_id: super::super::new_update_transaction_id(),
        client_id: identity.client_id,
        client_uuid: identity.client_uuid,
        installation_id: instance()?,
        target_key,
        package_sha256: package_sha256.into(),
        from_version: crate::VERSION,
        from_build_seq: crate::BUILD_SEQ,
        to_version: version,
        to_build_seq: build,
        source,
        command_id,
    })
    .into();
    let outbox = Outbox::prepare(binding, &package_name)?;
    let prepared = (|| -> Result<(), PreparationError> {
        let runtime = outbox.directory.join("runtime");
        workspace::create(&runtime).map_err(|_| PreparationError::refused("create_runtime"))?;
        let parent = Path::new(INSTALLED)
            .parent()
            .ok_or_else(|| PreparationError::refused("copy_writer"))?;
        workspace::verify_restore_target(&current, false)
            .map_err(|_| PreparationError::refused("copy_writer"))?;
        copy_locked(
            &current,
            &runtime.join("ordinary-msi-writer.exe"),
            None,
            "copy_writer",
        )?;
        for entry in fs::read_dir(parent).map_err(|e| PreparationError::io("copy_dependency", e))? {
            let path = entry
                .map_err(|e| PreparationError::io("copy_dependency", e))?
                .path();
            if path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("dll"))
            {
                workspace::verify_restore_target(&path, false)
                    .map_err(|_| PreparationError::refused("copy_dependency"))?;
                copy_locked(
                    &path,
                    &runtime.join(
                        path.file_name()
                            .ok_or_else(|| PreparationError::refused("copy_dependency"))?,
                    ),
                    None,
                    "copy_dependency",
                )?;
            }
        }
        copy_locked(
            package,
            &outbox.directory.join(&package_name),
            Some(package_sha256),
            "copy_package",
        )?;
        super::supervisor::seal_runtime(&outbox)
            .map_err(|_| PreparationError::refused("seal_runtime"))?;
        Ok(())
    })();
    if let Err(error) = prepared {
        let code = error.code.clone();
        outbox.preparation_failed(error)?;
        return Err(code);
    }
    // This command has no caller-selected executable, arguments or output path.
    super::super::report_update_event("installing", version, build, source);
    if let Err(error) = super::supervisor::launch(&outbox, _guard) {
        hbb_common::log::warn!(
            "ordinary MSI runner returned; durable outcome requires reconciliation: {error}"
        );
    }
    Ok(())
}

pub(crate) fn runner_commands(id: &str, digest: &str) -> Result<String, String> {
    install_allowed()?;
    super::OutcomeWriteRequest::parse(&[id.into(), digest.into(), "0".into()])?;
    let outbox = Outbox::open_existing(id)?;
    if outbox.intent_digest()? != digest {
        return Err("ordinary intent digest mismatch".into());
    }
    let writer = outbox.directory.join(r"runtime\RustDesk.exe");
    let package = super::package_contract::runtime_package_destination(
        &outbox.directory,
        Some(&outbox.package_name()?),
    )?;
    workspace::verify_file(&writer)?;
    workspace::verify_file(&package)?;
    let mut hash = Sha256::new();
    let mut file = fs::File::open(&package).map_err(|e| e.to_string())?;
    let mut bytes = [0; 65536];
    loop {
        let n = file.read(&mut bytes).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        hash.update(&bytes[..n]);
    }
    if hex::encode(hash.finalize()) != outbox.binding.package_sha256 {
        return Err("ordinary runner package digest mismatch".into());
    }
    let outcome_args = super::package_contract::runner_arguments(id, digest, "%RD_MSI_RAW_EXIT%");
    Ok(format!(
        "\"C:\\Windows\\System32\\msiexec.exe\" /i \"{}\" /qn LAUNCH_TRAY_APP=N REBOOT=ReallySuppress /norestart\r\n\
         set \"RD_MSI_RAW_EXIT=%errorlevel%\"\r\n\
         \"{}\" {}\r\n\
         if errorlevel 1 exit /b 75\r\n\
         if \"%RD_MSI_RAW_EXIT%\"==\"3010\" exit /b 0\r\n\
         exit /b %RD_MSI_RAW_EXIT%",
        package.display(),
        writer.display(),
        outcome_args.join(" ")
    ))
}

pub(crate) fn start() {
    if !crate::is_server() || !crate::platform::is_installed() {
        return;
    }
    let Ok(handle) = hbb_common::tokio::runtime::Handle::try_current() else {
        return;
    };
    if STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    handle.spawn(async {
        loop {
            if let Err(error) = reconcile_all().await {
                hbb_common::log::warn!("ordinary update reconciliation: {error}");
            }
            hbb_common::tokio::time::sleep(Duration::from_secs(60)).await;
        }
    });
}

pub(crate) fn owns_command(command_id: &str) -> bool {
    let Ok(entries) = fs::read_dir(ROOT) else {
        return false;
    };
    entries.filter_map(Result::ok).any(|entry| {
        entry
            .file_name()
            .to_str()
            .and_then(|id| Outbox::open_existing(id).ok())
            .is_some_and(|outbox| {
                outbox.binding.command_id.as_deref() == Some(command_id)
                    && outbox.blocks_preparation_retry()
            })
    })
}

async fn reconcile_all() -> Result<(), String> {
    if !Path::new(ROOT).exists() {
        return Ok(());
    }
    workspace::verify(Path::new(ROOT))?;
    let identity = super::super::current_update_client_identity();
    let installation = instance()?;
    for entry in fs::read_dir(ROOT).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let Some(id) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        let outbox = match Outbox::open_existing(&id) {
            Ok(v) => v,
            Err(e) => {
                hbb_common::log::warn!("ordinary receipt refused: {e}");
                continue;
            }
        };
        let b = outbox.binding();
        if !outbox.blocks_preparation_retry() {
            continue;
        }
        if b.client_id != identity.client_id
            || b.client_uuid != identity.client_uuid
            || b.installation_id != installation
        {
            continue;
        }
        if !outbox.directory.join("terminal.json").exists() {
            let verified = if b.target_key == format!("windows-x86_64-msi-{}", crate::EDITION)
                && b.to_build_seq == crate::BUILD_SEQ
                && b.to_version == crate::VERSION
            {
                let build = b.to_build_seq;
                let handle = hbb_common::tokio::runtime::Handle::current();
                hbb_common::tokio::task::spawn_blocking(move || {
                    handle.block_on(health_windows::probe(
                        r"\\.\pipe\RustDesk\management-health-v2",
                        Path::new(INSTALLED),
                        build,
                        Duration::from_secs(3),
                    ))
                })
                .await
                .ok()
                .and_then(Result::ok)
                .filter(|response| response.registered && response.version == b.to_version)
                .map(|_| VerifiedRunningTarget { binding: b.clone() })
            } else {
                None
            };
            if instance()? != installation
                || super::super::current_update_client_identity() != identity
            {
                continue;
            }
            if outbox.reconcile(verified.as_ref()).is_err() {
                continue;
            }
        }
        hbb_common::tokio::task::spawn_blocking(move || {
            outbox.report_pending(report)?;
            if let Some(id) = &outbox.binding.command_id {
                if outbox.pending(Channel::Update)?.is_none()
                    && outbox.pending(Channel::Command)?.is_none()
                {
                    if let Some(state) = super::super::load_pending_update_commands()
                        .into_iter()
                        .find(|state| &state.command.command_id == id)
                    {
                        super::super::complete_update_command(&state.command);
                    }
                }
            }
            Ok::<(), String>(())
        })
        .await
        .map_err(|e| e.to_string())??;
    }
    Ok(())
}

fn report(channel: Channel, binding: &Binding, terminal: &Terminal) -> bool {
    let command = matches!(channel, Channel::Command);
    let status = if command && terminal.status == "installed" {
        "completed"
    } else {
        &terminal.status
    };
    const URL: &str = "https://rdapi.yan.life/rd/update/v1/events";
    let payload = serde_json::json!({"client_id":binding.client_id,"client_uuid":binding.client_uuid,
        "command_id":if command {binding.command_id.as_deref()}else{None},"command_action":if command {Some("install")}else{None},
        "status":status,"from_version":binding.from_version,"from_build_seq":binding.from_build_seq,"to_version":binding.to_version,
        "to_build_seq":binding.to_build_seq,"source":binding.source,"error_code":terminal.error_code});
    let Ok(body) = serde_json::to_vec(&payload) else {
        return false;
    };
    let identity = base::update::UpdateClientIdentity {
        client_id: binding.client_id.clone(),
        client_uuid: binding.client_uuid.clone(),
    };
    let Ok(auth) = super::super::update_device_auth_headers(
        "POST",
        URL,
        &identity,
        if command {
            binding.command_id.as_deref().unwrap_or("")
        } else {
            ""
        },
        &body,
    ) else {
        return false;
    };
    let Ok(client) = crate::hbbs_http::create_http_client_with_url_strict(URL) else {
        return false;
    };
    let Ok(response) = client
        .post(URL)
        .header("Content-Type", "application/json")
        .header("X-RustDesk-Device-ID", auth.device_id)
        .header("X-RustDesk-Device-Public-Key", auth.public_key)
        .header("X-RustDesk-Device-Timestamp", auth.timestamp)
        .header("X-RustDesk-Device-Nonce", auth.nonce)
        .header("X-RustDesk-Device-Signature", auth.signature)
        .body(body)
        .send()
    else {
        return false;
    };
    if !response.status().is_success() {
        return false;
    }
    let mut bytes = Vec::new();
    response.take(4097).read_to_end(&mut bytes).is_ok()
        && bytes.len() <= 4096
        && serde_json::from_slice::<serde_json::Value>(&bytes)
            .ok()
            .is_some_and(|v| v["ok"] == true)
}
