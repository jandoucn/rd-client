use std::{path::Path, time::Duration};
use windows_service::{
    service::{ServiceAccess, ServiceStartType, ServiceState, ServiceType},
    service_manager::{ServiceManager, ServiceManagerAccess},
};

#[derive(Debug, PartialEq, Eq)]
pub enum Cycle {
    Deferred,
    Unchanged,
    Recovered,
}

pub fn persist_before_package_skip(
    allow_legacy_package: bool,
    persist: impl FnOnce() -> std::io::Result<()>,
) -> std::io::Result<bool> {
    if allow_legacy_package {
        return Ok(false);
    }
    persist()?;
    Ok(true)
}

pub struct Routing {
    pub management: bool,
    pub legacy_service: bool,
    pub legacy_package: bool,
}
pub fn routing(result: &Result<Cycle, String>) -> Routing {
    Routing {
        management: true,
        legacy_service: matches!(result, Ok(Cycle::Unchanged)),
        legacy_package: false,
    }
}

pub fn budgeted(
    elapsed: Duration,
    budget: &mut super::recovery::RetryBudget,
    now: u64,
    mut persist: impl FnMut(&super::recovery::RetryBudget) -> Result<(), String>,
    action: impl FnOnce() -> Result<Cycle, String>,
) -> Result<Cycle, String> {
    if elapsed < Duration::from_secs(180) || !budget.due(now) {
        return Ok(Cycle::Deferred);
    }
    let previous = *budget;
    let mut next = previous;
    next.record_failure(now);
    persist(&next)?;
    *budget = next;
    let result = action();
    if matches!(result, Ok(Cycle::Unchanged | Cycle::Deferred)) {
        persist(&previous)?;
        *budget = previous;
    }
    result
}

pub fn periodic(elapsed: Duration) -> Result<Cycle, String> {
    periodic_checked(elapsed, None)
}

fn periodic_checked(elapsed: Duration, public: Option<&[u8]>) -> Result<Cycle, String> {
    if elapsed < Duration::from_secs(180) {
        return Ok(Cycle::Deferred);
    }
    if !super::rescue_assets::r3_cycle_allowed()? {
        return Ok(Cycle::Deferred);
    }
    recover(public).map(|changed| {
        if changed {
            Cycle::Recovered
        } else {
            Cycle::Unchanged
        }
    })
}

#[cfg(test)]
pub fn periodic_fixture(elapsed: Duration, public: &[u8]) -> Result<Cycle, String> {
    periodic_checked(elapsed, Some(public))
}

fn gate(state: &str, disabled: bool, processes: bool) -> Result<(), String> {
    if disabled {
        return Err("R3 service disabled".into());
    }
    if !matches!(state, "stopped" | "absent") || processes {
        return Err("R3 active or unknown sessions/process state".into());
    }
    Ok(())
}

#[cfg(test)]
pub fn gate_fixture(state: &str, disabled: bool, processes: bool) -> Result<(), String> {
    gate(state, disabled, processes)
}
#[cfg(test)]
pub fn recover_fixture(public: &[u8]) -> Result<bool, String> {
    recover(Some(public))
}

pub fn bundle_present() -> Result<bool, String> {
    match std::fs::symlink_metadata(r"C:\ProgramData\RustDesk\ManagementAgent\rescue\current") {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(_) => Err("R3 bundle observation failed".into()),
    }
}

pub fn recover(public: Option<&[u8]>) -> Result<bool, String> {
    if !bundle_present()? {
        return Ok(false);
    }
    let _guard = super::install_guard::InstallGuard::try_acquire()
        .map_err(|_| "R3 install guard unavailable")?
        .ok_or("R3 installation busy")?;
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)
        .map_err(|_| "R3 SCM unavailable")?;
    let service = match manager.open_service(
        "RustDesk",
        ServiceAccess::QUERY_CONFIG | ServiceAccess::QUERY_STATUS,
    ) {
        Ok(service) => Some(service),
        Err(windows_service::Error::Winapi(error)) if error.raw_os_error() == Some(1060) => None,
        Err(_) => return Err("R3 service observation unavailable".into()),
    };
    if let Some(service) = &service {
        let config = service
            .query_config()
            .map_err(|_| "R3 service configuration unavailable")?;
        let status = service
            .query_status()
            .map_err(|_| "R3 service status unavailable")?;
        if status.current_state == ServiceState::Running {
            let context = super::local_recovery_events::context()?;
            super::local_recovery_events::reconcile_fallback(
                context.as_ref(),
                super::local_recovery_events::now()?,
                || {
                    let (inventory, build, destination) =
                        super::rescue_assets::r3_verified(public)?;
                    if !super::rescue_assets::r3_existing_fallback(&inventory, &destination)? {
                        return Ok(false);
                    }
                    let contract = super::service_control::Contract {
                        name: "RustDesk".into(),
                        executable: destination.join("RustDesk.exe"),
                        arguments: vec!["--service".into()],
                        start_type: ServiceStartType::AutoStart,
                    };
                    Ok(
                        contract.matches_command(&config.executable_path.to_string_lossy())
                            && super::health_windows::probe_blocking(
                                r"\\.\pipe\RustDesk\management-health-v1",
                                &contract.executable,
                                build,
                                Duration::from_secs(3),
                            )
                            .is_ok(),
                    )
                },
            )?;
            return Ok(false);
        }
        gate(
            if status.current_state == ServiceState::Stopped {
                "stopped"
            } else {
                "unknown"
            },
            config.start_type == ServiceStartType::Disabled,
            false,
        )?;
        if config.service_type != ServiceType::OWN_PROCESS
            || !config
                .account_name
                .is_some_and(|name| name.to_string_lossy().eq_ignore_ascii_case("LocalSystem"))
        {
            return Err("R3 service identity mismatch".into());
        }
    }
    if rustdesk_process_present()? {
        return Err("R3 active or unknown sessions/process state".into());
    }
    if !super::rescue_assets::r3_normal_missing(Path::new(r"C:\Program Files\RustDesk"))? {
        return Ok(false);
    }
    let context = super::local_recovery_events::context()?;
    super::local_recovery_events::fallback_owned(
        context.as_ref(),
        super::local_recovery_events::now()?,
        || {
            let (destination, build) = super::rescue_assets::r3_stage(public)?;
            if rustdesk_process_present()? {
                return Err("R3 process appeared before switch".into());
            }
            if let Some(service) = &service {
                let config = service
                    .query_config()
                    .map_err(|_| "R3 service recheck unavailable")?;
                let status = service
                    .query_status()
                    .map_err(|_| "R3 service recheck unavailable")?;
                gate(
                    if status.current_state == ServiceState::Stopped {
                        "stopped"
                    } else {
                        "unknown"
                    },
                    config.start_type == ServiceStartType::Disabled,
                    false,
                )?;
                if config.service_type != ServiceType::OWN_PROCESS
                    || !config.account_name.is_some_and(|name| {
                        name.to_string_lossy().eq_ignore_ascii_case("LocalSystem")
                    })
                {
                    return Err("R3 service identity changed".into());
                }
            }
            super::rescue_assets::r3_verified(public)?;
            let exe = destination.join("RustDesk.exe");
            let command = format!("\"{}\" --service", exe.display());
            let action = if service.is_some() {
                "config"
            } else {
                "create"
            };
            let output = std::process::Command::new(r"C:\Windows\System32\sc.exe")
                .args([
                    action,
                    "RustDesk",
                    "binPath=",
                    &command,
                    "start=",
                    "auto",
                    "obj=",
                    "LocalSystem",
                    "DisplayName=",
                    "RustDesk Service",
                ])
                .output()
                .map_err(|_| "R3 fixed service configuration failed")?;
            if !output.status.success() {
                return Err("R3 fixed service configuration failed".into());
            }
            let contract = super::service_control::Contract {
                name: "RustDesk".into(),
                executable: exe.clone(),
                arguments: vec!["--service".into()],
                start_type: ServiceStartType::AutoStart,
            };
            super::service_control::start(&contract, Duration::from_secs(30))?;
            let deadline = std::time::Instant::now() + Duration::from_secs(30);
            loop {
                if super::health_windows::probe_blocking(
                    r"\\.\pipe\RustDesk\management-health-v1",
                    &exe,
                    build,
                    Duration::from_secs(3),
                )
                .is_ok()
                {
                    return Ok(true);
                }
                if std::time::Instant::now() >= deadline {
                    return Err("R3 fallback health unconfirmed; degraded assets retained".into());
                }
                std::thread::sleep(Duration::from_millis(250));
            }
        },
    )
}

fn rustdesk_process_present() -> Result<bool, String> {
    use windows::Win32::{
        Foundation::{CloseHandle, INVALID_HANDLE_VALUE},
        System::Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
            TH32CS_SNAPPROCESS,
        },
    };
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }
        .map_err(|_| "R3 process snapshot unavailable")?;
    if snapshot == INVALID_HANDLE_VALUE {
        return Err("R3 process snapshot invalid".into());
    }
    let mut entry = PROCESSENTRY32W::default();
    entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
    let mut found = false;
    let result = (|| {
        unsafe { Process32FirstW(snapshot, &mut entry) }.map_err(|_| "R3 process list unknown")?;
        loop {
            let count = entry
                .szExeFile
                .iter()
                .position(|&c| c == 0)
                .unwrap_or(entry.szExeFile.len());
            if String::from_utf16_lossy(&entry.szExeFile[..count])
                .eq_ignore_ascii_case("RustDesk.exe")
            {
                found = true;
                break;
            }
            if let Err(error) = unsafe { Process32NextW(snapshot, &mut entry) } {
                if error.code().0 as u32 != 0x80070012 {
                    return Err("R3 process enumeration failed");
                }
                break;
            }
        }
        Ok(())
    })();
    unsafe { CloseHandle(snapshot) }.map_err(|_| "R3 process snapshot close failed")?;
    result?;
    Ok(found)
}
