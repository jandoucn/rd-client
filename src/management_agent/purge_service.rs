use super::{handoff::ObservedProcess, purge_preflight::Context};
use std::{path::PathBuf, time::{Duration, Instant}};
use windows_service::{service::{ServiceAccess, ServiceState, ServiceType}, service_manager::{ServiceManager, ServiceManagerAccess}};

pub fn stop_and_delete(name: &str, context: &Context) -> Result<(), String> {
    let executable = match name {
        "RustDesk" => PathBuf::from(r"C:\Program Files\RustDesk\RustDesk.exe"),
        "RustDeskManagementAgent" => PathBuf::from(r"C:\ProgramData\RustDesk\ManagementAgent\rustdesk-management-agent.exe"),
        _ => return Err("purge service outside fixed contract".into()),
    };
    if name == "RustDeskManagementAgent" {
        let bytes = std::fs::read(r"C:\ProgramData\RustDesk\ManagementAgent\state\agent.json").map_err(|e| e.to_string())?;
        let state: serde_json::Value = serde_json::from_slice(&bytes).map_err(|_| "purge service state malformed")?;
        let edition = state["edition"].as_str().ok_or("purge service edition missing")?;
        if !matches!(edition, "standard" | "sos") || state["package_kind"] != "msi" { return Err("purge agent contract state mismatch".into()); }
        return stop_bound(name, executable, &format!("--service --edition {edition} --package-kind msi --normal-install-path \"C:\\Program Files\\RustDesk\""), context);
    }
    stop_bound(name, executable, "--service", context)
}

fn stop_bound(name: &str, executable: PathBuf, arguments: &str, context: &Context) -> Result<(), String> {
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT).map_err(|e| e.to_string())?;
    let service = match manager.open_service(name, ServiceAccess::QUERY_CONFIG | ServiceAccess::QUERY_STATUS | ServiceAccess::STOP | ServiceAccess::DELETE) {
        Ok(service) => service,
        Err(windows_service::Error::Winapi(error)) if error.raw_os_error() == Some(1060) => return Ok(()),
        Err(error) => return Err(format!("purge service open: {error}")),
    };
    let config = service.query_config().map_err(|e| e.to_string())?;
    let command = config.executable_path.to_string_lossy();
    if config.service_type != ServiceType::OWN_PROCESS
        || !config.account_name.as_ref().is_some_and(|a| a.to_string_lossy().eq_ignore_ascii_case("LocalSystem"))
        || !exact_command(&executable, arguments, &command) {
        return Err("purge service command/account mismatch".into());
    }
    let initial = service.query_status().map_err(|e| e.to_string())?;
    let observed = match initial.process_id.filter(|pid| *pid != 0) {
        Some(pid) => {
            let observed = ObservedProcess::open(pid)?;
            verify_image(pid, &executable)?;
            if service.query_status().map_err(|e| e.to_string())?.process_id != Some(pid) { return Err("purge service PID changed".into()); }
            Some(observed)
        },
        None if initial.current_state == ServiceState::Stopped => None,
        None => return Err("purge service process unknown".into()),
    };
    if !matches!(initial.current_state, ServiceState::Running | ServiceState::StopPending | ServiceState::Stopped) {
        return Err("purge service state unstable".into());
    }
    context.recheck()?;
    if initial.current_state == ServiceState::Running { service.stop().map_err(|e| format!("purge service stop: {e}"))?; }
    wait_stopped(Duration::from_secs(30), || {
        let status = service.query_status().map_err(|e| e.to_string())?;
        let alive = match &observed {
            Some(process) => process.is_alive()?,
            None => matching_process_alive(&executable)?,
        };
        if status.process_id.filter(|pid| *pid != 0).is_some_and(|pid| observed.as_ref().map(|v| v.owner().pid) != Some(pid)) {
            return Err("purge service replacement process".into());
        }
        Ok((status.current_state == ServiceState::Stopped, alive))
    })?;
    context.recheck()?;
    service.delete().map_err(|e| format!("purge service delete: {e}"))
}

fn exact_command(executable: &std::path::Path, arguments: &str, command: &str) -> bool {
    command.eq_ignore_ascii_case(&format!("\"{}\" {arguments}", executable.display()))
        || (!executable.to_string_lossy().contains(char::is_whitespace)
            && command.eq_ignore_ascii_case(&format!("{} {arguments}", executable.display())))
}

#[cfg(test)]
#[test]
fn installed_agent_complete_command_accepts_unquoted_but_never_foreign_arguments() {
    let executable = std::path::Path::new(r"C:\ProgramData\RustDesk\ManagementAgent\rustdesk-management-agent.exe");
    let args = r#"--service --edition sos --package-kind msi --normal-install-path "C:\Program Files\RustDesk""#;
    let actual = format!("{} {args}", executable.display());
    assert!(exact_command(executable, args, &actual));
    assert!(exact_command(executable, args, &format!("\"{}\" {args}", executable.display())));
    for foreign in [actual.replace("sos", "standard"), actual.replace("msi", "exe"),
        actual.replace("C:\\Program Files\\RustDesk", "C:\\Other"), format!("{actual} --extra"),
        actual.replace("rustdesk-management-agent.exe", "other.exe")] {
        assert!(!exact_command(executable, args, &foreign), "{foreign}");
    }
}

fn matching_process_alive(expected: &std::path::Path) -> Result<bool, String> {
    use hbb_common::sysinfo::System;
    let expected_name = expected.file_name().and_then(|v| v.to_str()).ok_or("purge executable name invalid")?;
    let system = System::new_all();
    for (pid, process) in system.processes() {
        if !process.name().eq_ignore_ascii_case(expected_name) { continue; }
        // Open failures, including an exit during enumeration, stay unknown.
        let observed = ObservedProcess::open(pid.as_u32())?;
        verify_image(pid.as_u32(), expected)?;
        if observed.is_alive()? { return Ok(true); }
    }
    Ok(false)
}

#[cfg(test)]
pub fn stop_fixture(context: &Context) -> Result<(), String> {
    stop_bound("RustDeskAgentTestService-PurgeExit", PathBuf::from(r"C:\RustDeskAgentLab\purge-native-service\fixture.exe"), "--exact native_purge_service_child --nocapture", context)
}

pub(super) fn wait_stopped(timeout: Duration, mut query: impl FnMut() -> Result<(bool, bool), String>) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    loop {
        let (stopped, alive) = query()?;
        if stopped && !alive { return Ok(()); }
        if Instant::now() >= deadline { return Err("purge service process exit not confirmed".into()); }
        std::thread::sleep(Duration::from_millis(25));
    }
}

fn verify_image(pid: u32, expected: &std::path::Path) -> Result<(), String> {
    #[link(name="kernel32")]
    extern "system" {
        fn OpenProcess(access:u32,inherit:i32,pid:u32)->*mut std::ffi::c_void;
        fn QueryFullProcessImageNameW(handle:*mut std::ffi::c_void,flags:u32,name:*mut u16,size:*mut u32)->i32;
        fn CloseHandle(handle:*mut std::ffi::c_void)->i32;
    }
    let handle = unsafe { OpenProcess(0x1000,0,pid) };
    if handle.is_null() { return Err("purge process image inaccessible".into()); }
    let mut name = vec![0u16;32768]; let mut size = name.len() as u32;
    let result = unsafe { QueryFullProcessImageNameW(handle,0,name.as_mut_ptr(),&mut size) };
    let closed = unsafe { CloseHandle(handle) };
    if result == 0 || closed == 0 { return Err("purge process image query failed".into()); }
    let image = String::from_utf16(&name[..size as usize]).map_err(|_| "purge process image encoding")?;
    if !image.eq_ignore_ascii_case(&expected.to_string_lossy()) { return Err("purge process image mismatch".into()); }
    Ok(())
}
