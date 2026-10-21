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

// SCM restarts a crashed service within seconds once failure actions are set,
// instead of waiting for the recovery scheduled task (up to 5 minutes).
const FAILURE_RESTART_DELAY: Duration = Duration::from_secs(5);
const FAILURE_RESET_PERIOD: Duration = Duration::from_secs(86_400);

fn failure_actions() -> windows_service::service::ServiceFailureActions {
    use windows_service::service::{
        ServiceAction, ServiceActionType, ServiceFailureActions, ServiceFailureResetPeriod,
    };
    ServiceFailureActions {
        reset_period: ServiceFailureResetPeriod::After(FAILURE_RESET_PERIOD),
        reboot_msg: None,
        command: None,
        // Restart three times in a row; the reset window gives a machine that
        // crashes once a day its own budget instead of tripping into no-action.
        actions: Some(
            (0..3)
                .map(|_| ServiceAction {
                    action_type: ServiceActionType::Restart,
                    delay: FAILURE_RESTART_DELAY,
                })
                .collect(),
        ),
    }
}

// Idempotent on every agent start. An in-place upgrade stops and restarts the
// services without running the install script that creates them, so configuring
// failure actions only at install time would never reach an upgraded machine.
// Running this from the agent's own startup makes an upgrade behave like a fresh
// install for this capability: the new binary configures what it needs. A missing
// service is skipped (bootstrap/repair owns creation); success means every present
// service accepted the configuration, so a transient SCM error retries on the next
// start instead of being silently lost.
pub fn ensure_service_failure_actions() -> Result<bool, String> {
    use windows_service::service::ServiceAccess;
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)
        .map_err(|error| format!("service manager: {error}"))?;
    let mut configured = true;
    let app_name = crate::get_app_name();
    for name in [app_name.as_str(), "RustDeskManagementAgent"] {
        let service = match manager.open_service(name, ServiceAccess::CHANGE_CONFIG) {
            Ok(service) => service,
            Err(windows_service::Error::Winapi(error)) if is_missing_service(&error) => continue,
            Err(error) => return Err(format!("service {name} open: {error}")),
        };
        if let Err(error) = service.update_failure_actions(failure_actions()) {
            configured = false;
            hbb_common::throttled_log!(
                Duration::from_secs(3600),
                warn,
                "service failure actions for {name} refused: {error}"
            );
        }
    }
    Ok(configured)
}

fn is_missing_service(error: &std::io::Error) -> bool {
    // ERROR_SERVICE_DOES_NOT_EXIST (1060)
    error.raw_os_error() == Some(1060)
}

#[cfg(test)]
mod tests {
    use super::*;

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
