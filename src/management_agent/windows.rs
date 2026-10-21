use super::{normal_client_exe, recovery, AgentPaths, AgentState};
use hbb_common::config::Config2;
use serde::Deserialize;
use std::{path::Path, process::Command, time::Duration};
use windows_service::{
    service::{ServiceAccess, ServiceStartType, ServiceState as ScmState, ServiceType},
    service_manager::{ServiceManager, ServiceManagerAccess},
};

#[path = "../platform/windows_agent_guard.rs"]
pub mod install_guard;

pub fn read_intent() -> Result<recovery::Intent, String> {
    read_intent_from(&Config2::file())
}

#[derive(Deserialize)]
struct IntentConfig {
    #[serde(default)]
    options: std::collections::HashMap<String, String>,
}

pub fn read_intent_from(path: &Path) -> Result<recovery::Intent, String> {
    let bytes = std::fs::read(path).map_err(|error| format!("intent read: {error}"))?;
    let text = std::str::from_utf8(&bytes).map_err(|error| format!("intent encoding: {error}"))?;
    let config: IntentConfig = hbb_common::toml::from_str(text)
        .map_err(|_| "intent configuration malformed".to_owned())?;
    Ok(
        match config.options.get("stop-service").map(String::as_str) {
            Some("Y") => recovery::Intent::Disabled,
            None | Some("") | Some("N") => recovery::Intent::Enabled,
            _ => recovery::Intent::Unknown,
        },
    )
}

fn regular_local_file(path: &Path) -> bool {
    use std::os::windows::fs::MetadataExt;
    path.is_absolute()
        && path.is_file()
        && path.ancestors().all(|ancestor| {
            std::fs::symlink_metadata(ancestor)
                .map(|metadata| metadata.file_attributes() & 0x400 == 0)
                .unwrap_or(false)
        })
}

pub struct ServiceObservation {
    pub state: recovery::ServiceState,
    pub command: String,
    pub trusted_path: bool,
    pub normal_present: bool,
    pub fallback_present: bool,
    pub disabled_start: bool,
    pub process_id: Option<u32>,
    pub account: Option<String>,
    pub start_type: String,
}

pub fn inspect(state: &AgentState, paths: &AgentPaths) -> Result<ServiceObservation, String> {
    let normal = normal_client_exe(&state.normal_install_path).filter(|path| {
        path.parent()
            .map(super::approved_normal_install_path)
            .unwrap_or(false)
    });
    let fallback = paths.client.join("RustDesk.exe");
    let normal_present = normal.as_deref().map(regular_local_file).unwrap_or(false);
    let fallback_present = regular_local_file(&fallback);
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)
        .map_err(|error| format!("service manager: {error}"))?;
    let service = match manager.open_service(
        "RustDesk",
        ServiceAccess::QUERY_STATUS | ServiceAccess::QUERY_CONFIG,
    ) {
        Ok(service) => service,
        Err(windows_service::Error::Winapi(error)) if error.raw_os_error() == Some(1060) => {
            return Ok(ServiceObservation {
                state: recovery::ServiceState::Absent,
                command: String::new(),
                trusted_path: false,
                normal_present,
                fallback_present,
                disabled_start: false,
                process_id: None,
                account: None,
                start_type: "absent".into(),
            });
        }
        Err(error) => return Err(format!("service query: {error}")),
    };
    let status = service
        .query_status()
        .map_err(|error| format!("service status: {error}"))?;
    let config = service
        .query_config()
        .map_err(|error| format!("service config: {error}"))?;
    let command = config.executable_path.to_string_lossy().into_owned();
    let system_account = config
        .account_name
        .as_ref()
        .map(|account| {
            account
                .to_string_lossy()
                .eq_ignore_ascii_case("LocalSystem")
        })
        .unwrap_or(false);
    let trusted_path = config.service_type == ServiceType::OWN_PROCESS
        && system_account
        && ((normal_present
            && normal
                .as_deref()
                .map(|path| recovery::service_path_matches(&command, &path.to_string_lossy()))
                .unwrap_or(false))
            || (fallback_present
                && recovery::service_path_matches(&command, &fallback.to_string_lossy())));
    let state = match status.current_state {
        ScmState::Stopped => recovery::ServiceState::Stopped,
        ScmState::StartPending => recovery::ServiceState::StartPending,
        ScmState::StopPending => recovery::ServiceState::StopPending,
        ScmState::Running => recovery::ServiceState::Running,
        ScmState::Paused => recovery::ServiceState::Paused,
        _ => recovery::ServiceState::Unknown,
    };
    Ok(ServiceObservation {
        state,
        command,
        trusted_path,
        normal_present,
        fallback_present,
        disabled_start: config.start_type == ServiceStartType::Disabled,
        process_id: status.process_id,
        account: config.account_name.map(|value| value.to_string_lossy().into_owned()),
        start_type: format!("{:?}", config.start_type),
    })
}

fn mutation_allowed(state: &AgentState, paths: &AgentPaths) -> Result<bool, String> {
    if super::purge_marker_path(paths).exists()
        || paths.state.join("disable-agent").exists()
        || paths.state.join("maintenance.json").exists()
        || !paths.state.join("service-recovery-enabled").is_file()
        || !state.management_agent_enabled
        || state.disabled
    {
        return Ok(false);
    }
    Ok(read_intent()? == recovery::Intent::Enabled)
}

pub fn start_existing(state: &AgentState, paths: &AgentPaths) -> Result<bool, String> {
    let Some(_guard) = install_guard::InstallGuard::try_acquire()
        .map_err(|error| format!("install guard: {error}"))?
    else {
        return Ok(false);
    };
    if !mutation_allowed(state, paths)? {
        return Ok(false);
    }
    let observed = inspect(state, paths)?;
    if observed.state != recovery::ServiceState::Stopped
        || !observed.trusted_path
        || observed.disabled_start
    {
        return Ok(false);
    }
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)
        .map_err(|error| format!("service manager: {error}"))?;
    let service = manager
        .open_service(
            "RustDesk",
            ServiceAccess::START | ServiceAccess::QUERY_STATUS,
        )
        .map_err(|error| format!("service start access: {error}"))?;
    if let Err(error) = service.start::<&str>(&[]) {
        match error {
            windows_service::Error::Winapi(error) if error.raw_os_error() == Some(1056) => {}
            error => return Err(format!("service start: {error}")),
        }
    }
    // This confirms the SCM transition only; it does not claim remote/application readiness.
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    loop {
        let status = service
            .query_status()
            .map_err(|error| format!("service verify: {error}"))?;
        match status.current_state {
            ScmState::Running => return Ok(true),
            ScmState::StartPending if std::time::Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(250));
            }
            _ => {
                return Err(format!(
                    "service not running: {:?}, {:?}",
                    status.current_state, status.exit_code
                ))
            }
        }
    }
}

pub fn repair_service(state: &AgentState, paths: &AgentPaths, fallback: bool) -> Result<bool, String> {
    let Some(_guard) = install_guard::InstallGuard::try_acquire()
        .map_err(|error| format!("install guard: {error}"))?
    else {
        return Ok(false);
    };
    if !mutation_allowed(state, paths)? {
        return Ok(false);
    }
    let observed = inspect(state, paths)?;
    if observed.disabled_start
        || !matches!(observed.state, recovery::ServiceState::Absent | recovery::ServiceState::Stopped)
    {
        return Ok(false);
    }
    let executable = if fallback {
        paths.client.join("RustDesk.exe")
    } else {
        normal_client_exe(&state.normal_install_path)
            .ok_or_else(|| "normal RustDesk path unavailable".to_owned())?
    };
    if !regular_local_file(&executable)
        || (!fallback && !executable.parent().map(super::approved_normal_install_path).unwrap_or(false))
    {
        return Ok(false);
    }
    let binary = format!(r#""{}" --service"#, executable.display());
    let command = if observed.state == recovery::ServiceState::Absent { "create" } else { "config" };
    let sc = std::env::var_os("SystemRoot").map(std::path::PathBuf::from)
        .ok_or("Windows system directory unavailable")?.join("System32/sc.exe");
    let status = Command::new(sc)
        .args([
            command,
            "RustDesk",
            "binPath=",
            &binary,
            "start=",
            "auto",
            "obj=",
            "LocalSystem",
            "DisplayName=",
            "RustDesk Service",
        ])
        .status()
        .map_err(|error| format!("service {command}: {error}"))?;
    if !status.success() {
        return Err(format!("service {command} failed: {status}"));
    }
    start_existing(state, paths)
}

pub fn restart_service(state: &AgentState, paths: &AgentPaths) -> Result<bool, String> {
    let Some(_guard) = install_guard::InstallGuard::try_acquire()
        .map_err(|error| format!("install guard: {error}"))?
    else {
        return Ok(false);
    };
    if !mutation_allowed(state, paths)? {
        return Ok(false);
    }
    let observed = inspect(state, paths)?;
    if observed.disabled_start || !observed.trusted_path {
        return Ok(false);
    }
    if observed.state == recovery::ServiceState::Running {
        let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)
            .map_err(|error| format!("service manager: {error}"))?;
        let service = manager.open_service("RustDesk", ServiceAccess::STOP | ServiceAccess::QUERY_STATUS)
            .map_err(|error| format!("service stop access: {error}"))?;
        service.stop().map_err(|error| format!("service stop: {error}"))?;
        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        loop {
            let status = service.query_status().map_err(|error| format!("service stop verify: {error}"))?;
            if status.current_state == ScmState::Stopped { break; }
            if status.current_state != ScmState::StopPending || std::time::Instant::now() >= deadline {
                return Err("service stop timeout or unexpected state".into());
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
    let observed = inspect(state, paths)?;
    if observed.trusted_path {
        return start_existing(state, paths);
    }
    repair_service(state, paths, !observed.normal_present && observed.fallback_present)
}

// SCM restart delays are expressed in milliseconds; 5000 is the smallest value
// the SCM accepts that still leaves the process time to release its socket.
const FAILURE_RESET_SECONDS: u32 = 86_400;
const FAILURE_RESTART_DELAY_MS: u32 = 5_000;
const FAILURE_ACTION_COUNT: u32 = 3;

// `sc failure` takes the form `reset= <sec> actions= <a>/<ms>/<a>/<ms>...`, with a
// space after each `=` and reset before actions. Returned as separate argv tokens
// so the process is launched without any shell: cmd.exe and PowerShell quote and
// escape differently, and neither is involved here.
fn failure_actions_tokens() -> Vec<String> {
    let actions = vec![format!("restart/{FAILURE_RESTART_DELAY_MS}"); FAILURE_ACTION_COUNT as usize]
        .join("/");
    vec![
        "reset=".to_owned(),
        FAILURE_RESET_SECONDS.to_string(),
        "actions=".to_owned(),
        actions,
    ]
}

#[cfg(test)]
fn failure_actions_arg() -> String {
    failure_actions_tokens().join(" ")
}

// Whether the service is installed is decided by the SCM, not by parsing sc.exe
// output: that text is localized (a Chinese Windows never prints "does not exist"
// and wraps the 1060 code in translated words), so matching on it would silently
// misread a present service as absent and skip configuring it.
fn service_exists(name: &str) -> Result<bool, String> {
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)
        .map_err(|error| format!("service manager: {error}"))?;
    match manager.open_service(name, ServiceAccess::QUERY_CONFIG) {
        Ok(_) => Ok(true),
        Err(windows_service::Error::Winapi(error)) if error.raw_os_error() == Some(1060) => {
            Ok(false)
        }
        Err(error) => Err(format!("service query {name}: {error}")),
    }
}

// Idempotent on every agent start. An in-place upgrade stops and restarts the
// services without running the install script that creates them, so configuring
// failure actions only at install time would never reach an upgraded machine.
// Running this from the agent's own startup makes an upgrade behave like a fresh
// install for this capability: the new binary configures what it needs.
//
// Uses `sc.exe` rather than ChangeServiceConfig2 through the windows-service
// crate: the shell command is the same mechanism repair_service and
// rescue_recovery already use to write service configuration, and it was
// verified against the real SCM on the test machine. A missing service is
// skipped (bootstrap/repair owns creation); success means every present service
// accepted the configuration, so a transient failure retries on the next start
// instead of being silently lost.
pub fn ensure_service_failure_actions() -> Result<bool, String> {
    let sc = std::env::var_os("SystemRoot")
        .map(std::path::PathBuf::from)
        .ok_or("Windows system directory unavailable")?
        .join("System32/sc.exe");
    let tokens = failure_actions_tokens();
    let mut configured = true;
    // The agent binary has no access to the client crate root, so the service
    // name is the same fixed literal used by inspect/repair/rescue_recovery.
    for name in ["RustDesk", "RustDeskManagementAgent"] {
        // Absent services belong to bootstrap/repair; nothing to configure here.
        if !service_exists(name).unwrap_or(true) {
            continue;
        }
        let output = Command::new(&sc)
            .args(["failure", name])
            .args(&tokens)
            .output()
            .map_err(|error| format!("service failure actions {name}: {error}"))?;
        if output.status.success() {
            continue;
        }
        configured = false;
        // The message is localized and its encoding is the console codepage, so
        // it is logged for an operator to read rather than interpreted here.
        hbb_common::throttled_log!(
            Duration::from_secs(3600),
            warn,
            "service failure actions for {name} refused: {}",
            output.status
        );
    }
    Ok(configured)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failure_actions_argument_is_restart_three_times_with_daily_reset() {
        let arg = failure_actions_arg();
        // sc.exe requires reset= before actions= and a space after each "=".
        assert_eq!(
            arg,
            "reset= 86400 actions= restart/5000/restart/5000/restart/5000"
        );
        assert!(arg.find("reset=").unwrap() < arg.find("actions=").unwrap());
        assert_eq!(arg.matches("restart/").count(), 3);
    }

    // The tokens are passed to sc.exe as separate argv entries with no shell in
    // between, so each "=" must stay glued to its value and carries its own space
    // only when joined. This is what keeps cmd.exe and PowerShell quoting rules
    // (which differ) out of the path entirely.
    #[test]
    fn failure_actions_tokens_keep_equals_signs_attached() {
        assert_eq!(
            failure_actions_tokens(),
            vec!["reset=", "86400", "actions=", "restart/5000/restart/5000/restart/5000"]
                .into_iter()
                .map(str::to_owned)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn disabled_mutations_do_not_read_intent_or_change_the_service() {
        let root = std::env::temp_dir().join(format!("agent-command-gates-{}", std::process::id()));
        let paths = AgentPaths::from_program_data(&root, super::super::Edition::Standard);
        std::fs::create_dir_all(&paths.state).unwrap();
        std::fs::write(paths.state.join("service-recovery-enabled"), b"enabled").unwrap();
        let state = AgentState::new(super::super::Edition::Standard, &paths);
        for marker in ["disable-recovery", "disable-agent", "maintenance.json"] {
            let path = paths.state.join(marker);
            let original = b"preserve-local-intent";
            std::fs::write(&path, original).unwrap();
            assert_eq!(mutation_allowed(&state, &paths), Ok(false), "{marker}");
            assert_eq!(std::fs::read(&path).unwrap(), original);
            std::fs::remove_file(path).unwrap();
        }
        let mut disabled = state;
        disabled.disabled = true;
        assert_eq!(mutation_allowed(&disabled, &paths), Ok(false));
        disabled.disabled = false;
        disabled.management_agent_enabled = false;
        assert_eq!(mutation_allowed(&disabled, &paths), Ok(false));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn blocked_public_service_actions_preserve_actual_service() {
        let computer = std::env::var("COMPUTERNAME").unwrap_or_default();
        assert!(matches!(computer.as_str(), "RDLAB-WIN10" | "RDLAB-WIN11"), "VM-only service acceptance");
        let root = std::env::temp_dir().join(format!("agent-public-gates-{}", std::process::id()));
        let paths = AgentPaths::from_program_data(&root, super::super::Edition::Standard);
        std::fs::create_dir_all(&paths.state).unwrap();
        std::fs::write(paths.state.join("service-recovery-enabled"), b"enabled").unwrap();
        let mut state = AgentState::new(super::super::Edition::Standard, &paths);
        state.normal_install_path = r"C:\Program Files\RustDesk".into();
        let before = inspect(&state, &paths).unwrap();
        for marker in ["disable-recovery", "disable-agent", "maintenance.json"] {
            let path = paths.state.join(marker);
            std::fs::write(&path, b"local-block").unwrap();
            assert_eq!(start_existing(&state, &paths), Ok(false));
            assert_eq!(restart_service(&state, &paths), Ok(false));
            assert_eq!(repair_service(&state, &paths, false), Ok(false));
            assert_eq!(repair_service(&state, &paths, true), Ok(false));
            let after = inspect(&state, &paths).unwrap();
            assert_eq!(after.state, before.state, "{marker}");
            assert_eq!(after.process_id, before.process_id, "{marker}");
            assert_eq!(after.command, before.command, "{marker}");
            assert_eq!(after.account, before.account, "{marker}");
            assert_eq!(after.start_type, before.start_type, "{marker}");
            assert_eq!(std::fs::read(&path).unwrap(), b"local-block");
            std::fs::remove_file(path).unwrap();
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn intent_reader_distinguishes_missing_malformed_and_explicit_stop() {
        let file = std::env::temp_dir().join(format!(
            "rustdesk-intent-test-{}-{}.toml", std::process::id(),
            hex::encode(hbb_common::sodiumoxide::randombytes::randombytes(8))
        ));
        assert!(read_intent_from(&file).is_err());
        std::fs::write(&file, "[options]\nstop-service='Y'\n").unwrap();
        assert_eq!(read_intent_from(&file).unwrap(), recovery::Intent::Disabled);
        std::fs::write(&file, "[options]\nstop-service='N'\n").unwrap();
        assert_eq!(read_intent_from(&file).unwrap(), recovery::Intent::Enabled);
        std::fs::write(&file, "[options]\nstop-service=42\n").unwrap();
        assert!(read_intent_from(&file).is_err());
        std::fs::write(&file, "[options]\nstop-service='unexpected'\n").unwrap();
        assert_eq!(read_intent_from(&file).unwrap(), recovery::Intent::Unknown);
        std::fs::remove_file(file).unwrap();
    }
}
