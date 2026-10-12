use std::{
    path::PathBuf,
    time::{Duration, Instant},
};
use windows_service::{
    service::{Service, ServiceAccess, ServiceStartType, ServiceState, ServiceType},
    service_manager::{ServiceManager, ServiceManagerAccess},
};

#[derive(Clone, Debug)]
pub struct Contract {
    pub name: String,
    pub executable: PathBuf,
    pub arguments: Vec<String>,
    pub start_type: ServiceStartType,
}

pub struct Observation {
    pub state: ServiceState,
    pub pid: Option<u32>,
}

impl Contract {
    pub fn validate(&self) -> Result<(), String> {
        let allowed = self.name == "RustDesk" || self.name == "RustDeskManagementAgent";
        #[cfg(test)]
        let allowed = allowed || self.name.starts_with("RustDeskAgentTestService-");
        if !allowed
            || !self.executable.is_absolute()
            || self
                .arguments
                .iter()
                .any(|v| v.chars().any(char::is_control) || v.contains('"'))
        {
            return Err("service contract invalid".into());
        }
        if self.executable.to_string_lossy().contains('"') {
            return Err("service executable path invalid".into());
        }
        Ok(())
    }

    fn command(&self, quoted: bool) -> String {
        let mut value = if quoted {
            format!("\"{}\"", self.executable.display())
        } else {
            self.executable.to_string_lossy().into_owned()
        };
        for arg in &self.arguments {
            value.push(' ');
            if arg.chars().any(char::is_whitespace) {
                value.push('"');
                value.push_str(arg);
                value.push('"');
            } else {
                value.push_str(arg);
            }
        }
        value
    }

    pub fn matches_command(&self, command: &str) -> bool {
        command.eq_ignore_ascii_case(&self.command(true))
            || (!self
                .executable
                .to_string_lossy()
                .chars()
                .any(char::is_whitespace)
                && command.eq_ignore_ascii_case(&self.command(false)))
    }
}

fn open(contract: &Contract, access: ServiceAccess) -> Result<Service, String> {
    contract.validate()?;
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)
        .map_err(|e| format!("service manager: {e}"))?;
    let service = manager
        .open_service(
            &contract.name,
            access | ServiceAccess::QUERY_CONFIG | ServiceAccess::QUERY_STATUS,
        )
        .map_err(|e| format!("service open: {e}"))?;
    let config = service
        .query_config()
        .map_err(|e| format!("service config: {e}"))?;
    if config.service_type != ServiceType::OWN_PROCESS
        || config.start_type != contract.start_type
        || !config
            .account_name
            .as_ref()
            .map(|v| v.to_string_lossy().eq_ignore_ascii_case("LocalSystem"))
            .unwrap_or(false)
        || !contract.matches_command(&config.executable_path.to_string_lossy())
    {
        return Err("service installed contract mismatch".into());
    }
    use std::os::windows::fs::MetadataExt;
    for ancestor in contract.executable.ancestors() {
        let metadata = std::fs::symlink_metadata(ancestor)
            .map_err(|e| format!("service executable unavailable: {e}"))?;
        if metadata.file_attributes() & 0x400 != 0 {
            return Err("service executable reparse path refused".into());
        }
    }
    if !contract.executable.is_file() {
        return Err("service executable is not regular file".into());
    }
    Ok(service)
}

pub fn inspect(contract: &Contract) -> Result<Observation, String> {
    let service = open(contract, ServiceAccess::QUERY_STATUS)?;
    let status = service.query_status().map_err(|e| e.to_string())?;
    Ok(Observation {
        state: status.current_state,
        pid: status.process_id,
    })
}

pub fn stop(contract: &Contract, timeout: Duration) -> Result<(), String> {
    let service = open(contract, ServiceAccess::STOP)?;
    let initial = service.query_status().map_err(|e| e.to_string())?;
    if initial.current_state == ServiceState::Stopped {
        return Ok(());
    }
    if initial.current_state != ServiceState::Running
        && initial.current_state != ServiceState::StopPending
    {
        return Err("service stop state not stable".into());
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut std::ffi::c_void;
        fn WaitForSingleObject(handle: *mut std::ffi::c_void, milliseconds: u32) -> u32;
        fn CloseHandle(handle: *mut std::ffi::c_void) -> i32;
    }
    struct Process(*mut std::ffi::c_void);
    impl Drop for Process {
        fn drop(&mut self) {
            if unsafe { CloseHandle(self.0) } == 0 {
                eprintln!(
                    "service process handle cleanup failed: {}",
                    std::io::Error::last_os_error()
                );
            }
        }
    }
    let pid = initial
        .process_id
        .ok_or("running service process ID unavailable")?;
    let handle = unsafe { OpenProcess(0x00100000, 0, pid) };
    if handle.is_null() {
        return Err(format!(
            "service process wait handle unavailable: {}",
            std::io::Error::last_os_error()
        ));
    }
    let process = Process(handle);
    if initial.current_state == ServiceState::Running {
        service.stop().map_err(|e| format!("service stop: {e}"))?;
    }
    let deadline = Instant::now() + timeout;
    loop {
        let status = service.query_status().map_err(|e| e.to_string())?;
        let process_state = unsafe { WaitForSingleObject(process.0, 0) };
        if status.current_state == ServiceState::Stopped && process_state == 0 {
            return Ok(());
        }
        if process_state != 0 && process_state != 0x102 {
            return Err(format!(
                "service process wait failed: {}",
                std::io::Error::last_os_error()
            ));
        }
        if Instant::now() >= deadline {
            return Err("service stop/exit timeout".into());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

pub fn start(contract: &Contract, timeout: Duration) -> Result<(), String> {
    if contract.start_type == ServiceStartType::Disabled {
        return Err("service start disabled by contract".into());
    }
    let service = open(contract, ServiceAccess::START)?;
    let initial = service.query_status().map_err(|e| e.to_string())?;
    if initial.current_state == ServiceState::Running {
        return Ok(());
    }
    if initial.current_state == ServiceState::Stopped {
        service
            .start::<&str>(&[])
            .map_err(|e| format!("service start: {e}"))?;
    } else if initial.current_state != ServiceState::StartPending {
        return Err("service start state not stable".into());
    }
    let deadline = Instant::now() + timeout;
    loop {
        let status = service.query_status().map_err(|e| e.to_string())?;
        if status.current_state == ServiceState::Running && status.process_id.is_some() {
            return Ok(());
        }
        if status.current_state != ServiceState::StartPending || Instant::now() >= deadline {
            return Err(format!(
                "service not ready: {:?}, {:?}",
                status.current_state, status.exit_code
            ));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}
