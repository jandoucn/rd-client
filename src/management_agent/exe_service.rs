use super::{
    exe_install::Platform,
    exe_preparation::{self, Layout},
    handoff::{ObservedProcess, Owner},
    health_windows,
    install_guard::InstallGuard,
    install_plan::InstallPlan,
    service_control::{self, Contract},
    workspace,
};
use std::{
    path::Path,
    time::{Duration, Instant},
};
use windows_service::service::ServiceState;

pub(crate) struct FixedPlatformAdapter {
    contracts: [Contract; 2],
    pipes: [String; 2],
    layout: Layout,
    plan: InstallPlan,
    processes: Vec<ObservedProcess>,
    owners: Vec<Owner>,
    timeout: Duration,
    stop_authorized: bool,
    health_build: u64,
    fixture: bool,
    _guard: InstallGuard,
    authority: Option<super::exe_runtime::Authority>,
    recovery: bool,
}

impl FixedPlatformAdapter {
    pub fn fixed(plan: &InstallPlan) -> Result<Self, String> {
        Self::fixed_checked(plan, None, false)
    }
    pub(crate) fn fixed_owned(
        plan: &InstallPlan,
        authority: super::exe_runtime::Authority,
    ) -> Result<Self, String> {
        Self::fixed_checked(plan, Some(authority), false)
    }
    pub(crate) fn fixed_recovery(
        plan: &InstallPlan,
        authority: super::exe_runtime::Authority,
    ) -> Result<Self, String> {
        Self::fixed_checked(plan, Some(authority), true)
    }
    fn fixed_checked(
        plan: &InstallPlan,
        authority: Option<super::exe_runtime::Authority>,
        recovery: bool,
    ) -> Result<Self, String> {
        use windows_service::service::ServiceStartType;
        plan.validate()?;
        let edition = Self::edition(plan)?;
        let dirs = crate::management_agent::system_dirs::get();
        let agent_root = dirs.agent();
        let configured = hbb_common::config::Config::file();
        let identity = [dirs.local_service_roaming(), dirs.system_profile_roaming()]
            .map(|p| p.join("RustDesk").join("config").join("RustDesk.toml"))
            .into_iter()
            .find(|p| {
                configured
                    .to_string_lossy()
                    .eq_ignore_ascii_case(&p.to_string_lossy())
            })
            .ok_or("EXE fixed identity path unavailable")?;
        let layout = Layout {
            normal: dirs.normal_install(),
            agent: agent_root.join("rustdesk-management-agent.exe"),
            state: agent_root.join("state"),
            config: identity.parent().ok_or("EXE fixed config missing")?.into(),
        };
        let contracts = ["RustDesk", "RustDeskManagementAgent"].map(|name| {
            let original = plan
                .services
                .iter()
                .find(|s| s.name == name)
                .ok_or("EXE service snapshot missing")?;
            if !original.running {
                return Err("EXE original service is not running");
            }
            let start_type = match original.start_type.as_str() {
                "auto" => ServiceStartType::AutoStart,
                "manual" => ServiceStartType::OnDemand,
                _ => return Err("EXE service disabled"),
            };
            Ok(Contract {
                name: name.into(),
                executable: if name == "RustDesk" {
                    layout.normal.join("RustDesk.exe")
                } else {
                    layout.agent.clone()
                },
                arguments: if name == "RustDesk" {
                    vec!["--service".into()]
                } else {
                    vec![
                        "--service".into(),
                        "--edition".into(),
                        edition.into(),
                        "--package-kind".into(),
                        "exe".into(),
                        "--normal-install-path".into(),
                        layout.normal.to_string_lossy().into_owned(),
                    ]
                },
                start_type,
            })
        });
        let [main, agent] = contracts;
        Self::new(
            layout,
            plan,
            [main.map_err(str::to_owned)?, agent.map_err(str::to_owned)?],
            [
                r"\\.\pipe\RustDesk\management-health-v2".into(),
                String::new(),
            ],
            Duration::from_secs(30),
            false,
            authority,
            recovery,
        )
    }

    fn edition(plan: &InstallPlan) -> Result<&'static str, String> {
        match plan.target_key.as_str() {
            "windows-x86_64-exe-standard" => Ok("standard"),
            "windows-x86_64-exe-sos" => Ok("sos"),
            _ => Err("EXE adapter target kind mismatch".into()),
        }
    }

    fn new(
        layout: Layout,
        plan: &InstallPlan,
        contracts: [Contract; 2],
        pipes: [String; 2],
        timeout: Duration,
        fixture: bool,
        authority: Option<super::exe_runtime::Authority>,
        recovery: bool,
    ) -> Result<Self, String> {
        Self::edition(plan)?;
        let guard = InstallGuard::try_acquire()
            .map_err(|e| e.to_string())?
            .ok_or("EXE SCM installation busy")?;
        if recovery {
            let root = if fixture {
                layout
                    .normal
                    .parent()
                    .ok_or("fixture parent")?
                    .join("transaction")
            } else {
                crate::management_agent::system_dirs::get()
                    .agent()
                    .join("transactions")
                    .join(&plan.transaction_id)
            };
            super::exe_runtime::recovery_gates(&root, &layout, plan, authority.as_ref())?;
        } else if let Some(a) = &authority {
            a.gates(&layout, plan)?;
        } else {
            exe_preparation::gates(&layout, &plan.installation_id, Self::edition(plan)?)?;
        }
        for c in &contracts {
            service_control::inspect(c)?;
            workspace::verify_restore_target(&c.executable, false)?;
        }
        Ok(Self {
            contracts,
            pipes,
            layout,
            plan: plan.clone(),
            processes: vec![],
            owners: vec![],
            timeout,
            stop_authorized: false,
            health_build: plan.original_build_seq,
            fixture,
            _guard: guard,
            authority,
            recovery,
        })
    }

    #[cfg(test)]
    pub fn fixture(
        root: &Path,
        plan: &InstallPlan,
        contracts: [Contract; 2],
        pipes: [String; 2],
        timeout: Duration,
    ) -> Result<Self, String> {
        Self::fixture_owned(root, plan, contracts, pipes, timeout, None)
    }
    #[cfg(test)]
    pub(crate) fn fixture_owned(
        root: &Path,
        plan: &InstallPlan,
        contracts: [Contract; 2],
        pipes: [String; 2],
        timeout: Duration,
        authority: Option<super::exe_runtime::Authority>,
    ) -> Result<Self, String> {
        Self::fixture_checked(root, plan, contracts, pipes, timeout, authority, false)
    }
    #[cfg(test)]
    pub(crate) fn fixture_recovery(
        root: &Path,
        plan: &InstallPlan,
        contracts: [Contract; 2],
        pipes: [String; 2],
        timeout: Duration,
        authority: super::exe_runtime::Authority,
    ) -> Result<Self, String> {
        Self::fixture_checked(root, plan, contracts, pipes, timeout, Some(authority), true)
    }
    #[cfg(test)]
    fn fixture_checked(
        root: &Path,
        plan: &InstallPlan,
        contracts: [Contract; 2],
        pipes: [String; 2],
        timeout: Duration,
        authority: Option<super::exe_runtime::Authority>,
        recovery: bool,
    ) -> Result<Self, String> {
        let layout = exe_preparation::fixture_layout(root)?;
        for (i, c) in contracts.iter().enumerate() {
            let prefix = if i == 0 {
                "RustDeskAgentTestService-ExeMain-"
            } else {
                "RustDeskAgentTestService-ExeAgent-"
            };
            let suffix = c
                .name
                .strip_prefix(prefix)
                .ok_or("EXE fixture service name refused")?;
            if suffix.len() != 32
                || !suffix.bytes().all(|b| b.is_ascii_hexdigit())
                || c.executable
                    != if i == 0 {
                        layout.normal.join("RustDesk.exe")
                    } else {
                        layout.agent.clone()
                    }
                || !pipes[i].starts_with(r"\\.\pipe\RustDeskExeTest-")
            {
                return Err("EXE fixture contract refused".into());
            }
        }
        Self::new(
            layout, plan, contracts, pipes, timeout, true, authority, recovery,
        )
    }

    #[cfg(test)]
    pub fn replace_observed_owner(&mut self, index: usize, owner: Owner) {
        self.owners[index] = owner;
    }

    fn local(&self) -> Result<(), String> {
        if let Some(a) = &self.authority {
            return a.gates(&self.layout, &self.plan);
        }
        exe_preparation::gates(
            &self.layout,
            &self.plan.installation_id,
            Self::edition(&self.plan)?,
        )
    }

    fn idle(&self, build: u64) -> Result<(), String> {
        let reply = health_windows::probe_blocking(
            &self.pipes[0],
            &self.contracts[0].executable,
            build,
            self.timeout,
        )
        .map_err(|e| format!("EXE activity unknown: {e}"))?;
        if reply.no_active_sessions != Some(true) {
            return Err("EXE activity unknown or active".into());
        }
        Ok(())
    }

    fn observe_running(&mut self) -> Result<(), String> {
        let mut processes = vec![];
        for c in &self.contracts {
            let status = service_control::inspect(c)?;
            if status.state != ServiceState::Running {
                return Err("EXE SCM service not running".into());
            }
            let process = ObservedProcess::open(status.pid.ok_or("EXE SCM PID missing")?)?;
            if !process.is_alive()? {
                return Err("EXE SCM process exited".into());
            }
            processes.push(process);
        }
        self.owners = processes.iter().map(|p| p.owner().clone()).collect();
        self.processes = processes;
        Ok(())
    }

    fn check_observed(&self) -> Result<(), String> {
        if self.owners.len() != 2 || self.processes.len() != 2 {
            return Err("EXE SCM process identity not captured".into());
        }
        for (i, c) in self.contracts.iter().enumerate() {
            let status = service_control::inspect(c)?;
            if self.processes[i].owner() != &self.owners[i] {
                return Err("EXE SCM process identity changed".into());
            }
            if status.state == ServiceState::Stopped {
                if !self.stop_authorized || self.processes[i].is_alive()? {
                    return Err(format!(
                        "EXE SCM unconfirmed process exit: {} authorized={} alive={}",
                        c.name,
                        self.stop_authorized,
                        self.processes[i].is_alive()?
                    ));
                }
            } else if status.state != ServiceState::Running
                || status.pid != Some(self.owners[i].pid)
                || !self.processes[i].is_alive()?
            {
                return Err("EXE SCM process identity changed".into());
            }
        }
        Ok(())
    }

    fn start_verified(&mut self, build: u64) -> Result<(bool, bool), String> {
        self.local()?;
        if !self.stop_authorized || !self.stopped()? {
            return Err("EXE SCM restart without confirmed stop".into());
        }
        self.health_build = build;
        for i in 0..self.contracts.len() {
            if !self.recovery {
                self.check_observed()?;
            }
            if service_control::inspect(&self.contracts[i])?.state != ServiceState::Stopped {
                return Err("EXE foreign running service before start".into());
            }
            service_control::start(&self.contracts[i], self.timeout)?;
            let status = service_control::inspect(&self.contracts[i])?;
            let process =
                ObservedProcess::open(status.pid.ok_or("EXE started service PID missing")?)?;
            if status.state != ServiceState::Running || !process.is_alive()? {
                return Err("EXE started service exited".into());
            }
            if i < self.owners.len() {
                self.owners[i] = process.owner().clone();
                self.processes[i] = process;
            } else {
                self.owners.push(process.owner().clone());
                self.processes.push(process);
            }
        }
        #[cfg(test)]
        if !self.fixture
            && build != self.plan.original_build_seq
            && std::env::var("EXE_NATIVE_FAIL_NEW_AGENT").ok().as_deref() == Some("yes")
        {
            self.stop_observed(1)?;
            return Err("fixture new agent actually stopped after first start".into());
        }
        health_windows::probe_blocking(
            &self.pipes[0],
            &self.contracts[0].executable,
            build,
            self.timeout,
        )?;
        if self.pipes[1].is_empty() {
            self.agent_readiness(build)?;
        } else {
            health_windows::probe_blocking(
                &self.pipes[1],
                &self.contracts[1].executable,
                build,
                self.timeout,
            )?;
        }
        #[cfg(test)]
        if self.fixture && std::env::var("EXE_CORRUPT_INTERRUPT").ok().as_deref() == Some("yes") {
            std::process::exit(80);
        }
        self.local()?;
        Ok((true, true))
    }

    fn agent_readiness(&self, build: u64) -> Result<(), String> {
        let mut child = std::process::Command::new(&self.contracts[1].executable)
            .args([
                "status",
                "--json",
                "--edition",
                Self::edition(&self.plan)?,
                "--package-kind",
                "exe",
            ])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|e| e.to_string())?;
        use std::io::Read;
        let stdout = child.stdout.take().ok_or("EXE agent status pipe missing")?;
        let reader = std::thread::spawn(move || {
            let mut bytes = vec![];
            stdout.take(262145).read_to_end(&mut bytes).map(|_| bytes)
        });
        let deadline = Instant::now() + self.timeout;
        let status = loop {
            if let Some(s) = child.try_wait().map_err(|e| e.to_string())? {
                break s;
            }
            if Instant::now() >= deadline {
                child.kill().map_err(|e| e.to_string())?;
                child.wait().map_err(|e| e.to_string())?;
                return Err("EXE agent status timeout".into());
            }
            std::thread::sleep(Duration::from_millis(30));
        };
        let bytes = reader
            .join()
            .map_err(|_| "EXE status reader failed")?
            .map_err(|e| e.to_string())?;
        if !status.success() || bytes.len() > 262144 {
            return Err("EXE agent readiness process failed".into());
        }
        let status: serde_json::Value =
            serde_json::from_slice(&bytes).map_err(|_| "EXE agent status malformed")?;
        if status["agent"]["build_seq"] != build
            || status["agent_installation_id"] != self.plan.installation_id
        {
            return Err("EXE agent build/identity not ready".into());
        }
        self.check_observed()
    }

    fn stop_observed(&self, index: usize) -> Result<(), String> {
        use windows_service::{
            service::{ServiceAccess, ServiceType},
            service_manager::{ServiceManager, ServiceManagerAccess},
        };
        let contract = &self.contracts[index];
        let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)
            .map_err(|e| e.to_string())?;
        let service = manager
            .open_service(
                &contract.name,
                ServiceAccess::STOP | ServiceAccess::QUERY_STATUS | ServiceAccess::QUERY_CONFIG,
            )
            .map_err(|e| e.to_string())?;
        let config = service.query_config().map_err(|e| e.to_string())?;
        if config.service_type != ServiceType::OWN_PROCESS
            || config.start_type != contract.start_type
            || !config
                .account_name
                .is_some_and(|a| a.to_string_lossy().eq_ignore_ascii_case("LocalSystem"))
            || !contract.matches_command(&config.executable_path.to_string_lossy())
        {
            return Err("EXE SCM stop contract changed".into());
        }
        let status = service.query_status().map_err(|e| e.to_string())?;
        if status.current_state == ServiceState::Stopped && !self.processes[index].is_alive()? {
            return Ok(());
        }
        if status.current_state != ServiceState::Running
            || status.process_id != Some(self.owners[index].pid)
            || self.processes[index].owner() != &self.owners[index]
            || !self.processes[index].is_alive()?
        {
            return Err("EXE SCM stop process identity changed".into());
        }
        service
            .stop()
            .map_err(|e| format!("EXE SCM stop refused: {e}"))?;
        let deadline = Instant::now() + self.timeout;
        loop {
            let status = service.query_status().map_err(|e| e.to_string())?;
            if status.current_state == ServiceState::Stopped && !self.processes[index].is_alive()? {
                return Ok(());
            }
            if status
                .process_id
                .is_some_and(|pid| pid != self.owners[index].pid)
            {
                return Err("EXE SCM stop process identity changed".into());
            }
            if Instant::now() >= deadline {
                return Err("EXE SCM stop/exit timeout".into());
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn stop_fixed_children(&self) -> Result<(), String> {
        if self.fixture {
            return Ok(());
        }
        use hbb_common::sysinfo::System;
        let mut system = System::new();
        system.refresh_processes();
        for process in system.processes().values() {
            if !process.exe().to_string_lossy().eq_ignore_ascii_case(
                &crate::management_agent::system_dirs::get()
                    .normal_install()
                    .join("RustDesk.exe")
                    .to_string_lossy(),
            ) {
                continue;
            }
            if !matches!(
                process.cmd().get(1).map(String::as_str),
                Some("--server" | "--tray")
            ) {
                return Err("EXE unexpected fixed client process still running".into());
            }
            let observed = ObservedProcess::open(process.pid().as_u32())?;
            #[link(name = "kernel32")]
            extern "system" {
                fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut std::ffi::c_void;
                fn QueryFullProcessImageNameW(
                    handle: *mut std::ffi::c_void,
                    flags: u32,
                    name: *mut u16,
                    size: *mut u32,
                ) -> i32;
                fn TerminateProcess(handle: *mut std::ffi::c_void, code: u32) -> i32;
                fn CloseHandle(handle: *mut std::ffi::c_void) -> i32;
                fn WaitForSingleObject(handle: *mut std::ffi::c_void, timeout: u32) -> u32;
            }
            let handle = unsafe { OpenProcess(0x00101001, 0, observed.owner().pid) };
            if handle.is_null() {
                return Err("EXE fixed child process inaccessible".into());
            }
            let result: Result<(), String> = (|| {
                let mut name = [0u16; 32768];
                let mut size = name.len() as u32;
                if unsafe { QueryFullProcessImageNameW(handle, 0, name.as_mut_ptr(), &mut size) }
                    == 0
                    || !String::from_utf16_lossy(&name[..size as usize]).eq_ignore_ascii_case(
                        &crate::management_agent::system_dirs::get()
                            .normal_install()
                            .join("RustDesk.exe")
                            .to_string_lossy(),
                    )
                    || !observed.is_alive()?
                {
                    return Err("EXE fixed child identity changed".into());
                }
                if unsafe { TerminateProcess(handle, 0) } == 0 {
                    return Err("EXE fixed child exit failed".into());
                }
                if unsafe {
                    WaitForSingleObject(
                        handle,
                        self.timeout.as_millis().min(u32::MAX as u128) as u32,
                    )
                } != 0
                {
                    return Err("EXE fixed child exit timeout".into());
                }
                Ok(())
            })();
            if unsafe { CloseHandle(handle) } == 0 {
                return Err("EXE fixed child handle cleanup failed".into());
            }
            result?;
        }
        Ok(())
    }
}

impl Platform for FixedPlatformAdapter {
    fn preflight(&mut self) -> Result<(), String> {
        self.local()?;
        self.idle(self.plan.original_build_seq)?;
        self.observe_running()?;
        Ok(())
    }
    fn stop(&mut self) -> Result<(), String> {
        if self.stop_authorized || self.recovery {
            let root = if self.fixture {
                self.layout
                    .normal
                    .parent()
                    .ok_or("fixture parent missing")?
                    .join("transaction")
            } else {
                crate::management_agent::system_dirs::get()
                    .agent()
                    .join("transactions")
                    .join(&self.plan.transaction_id)
            };
            if root.join("stopped-state.json").exists() {
                super::exe_runtime::recovery_gates(
                    &root,
                    &self.layout,
                    &self.plan,
                    self.authority.as_ref(),
                )?;
            } else {
                self.local()?;
            }
        } else {
            self.local()?;
        }
        if self.recovery {
            let main = service_control::inspect(&self.contracts[0])?;
            if main.state == ServiceState::Running {
                if self.idle(self.plan.target_build_seq).is_err() {
                    self.idle(self.plan.original_build_seq)?;
                }
            } else if main.state != ServiceState::Stopped {
                return Err("EXE recovery main state unknown".into());
            }
            for c in &self.contracts {
                service_control::stop(c, self.timeout)?;
            }
            self.stop_fixed_children()?;
            self.stop_authorized = true;
            return Ok(());
        }
        if !self.fixture && self.authority.is_none() {
            return Err("EXE native helper/maintenance backend not ready".into());
        }
        if self.owners.is_empty() {
            self.preflight()?;
        }
        self.check_observed()?;
        let main = service_control::inspect(&self.contracts[0])?;
        if main.state == ServiceState::Running {
            self.idle(self.health_build)?;
        }
        // The whole pair is checked before the first control. Never follow a recycled PID.
        self.check_observed()?;
        self.stop_authorized = true;
        for i in [0, 1] {
            self.check_observed()?;
            self.stop_observed(i)?;
            if self.processes[i].is_alive()? {
                return Err("EXE SCM old process still alive".into());
            }
        }
        if !self.stopped()? {
            return Err("EXE SCM services not confirmed stopped".into());
        }
        self.stop_fixed_children()?;
        Ok(())
    }
    fn stopped(&mut self) -> Result<bool, String> {
        if !self.stop_authorized {
            return Ok(false);
        }
        if self.recovery {
            return self.contracts.iter().try_fold(true, |ready, c| {
                Ok(ready && service_control::inspect(c)?.state == ServiceState::Stopped)
            });
        }
        self.check_observed()?;
        for (i, c) in self.contracts.iter().enumerate() {
            if service_control::inspect(c)?.state != ServiceState::Stopped
                || self.processes[i].is_alive()?
            {
                return Ok(false);
            }
        }
        Ok(true)
    }
    fn start_and_validate(&mut self, build: u64) -> Result<(bool, bool), String> {
        self.start_verified(build)
    }
    fn restore_services(&mut self, build: u64) -> Result<(bool, bool), String> {
        self.start_verified(build)
    }
}
