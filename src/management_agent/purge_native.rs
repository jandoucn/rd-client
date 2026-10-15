use super::{purge_discovery::{self, Approved}, purge_execution::{self, Backend}, purge_handoff, purge_policy::{InstallerResult, Outcome, Probe}, purge_preflight::Context};
use std::{path::Path, process::Command, time::{Duration, Instant}};

const ROOT: &str = r"C:\ProgramData\RustDesk";
const NORMAL: &str = r"C:\Program Files\RustDesk";

pub(super) struct Native {
    context: Context,
    approved: Option<Approved>,
    marker: Option<super::purge_marker::HeldMarker>,
}

impl Native {
    pub(super) fn new(context: Context, approved: Approved) -> Self {
        Self { context, approved: Some(approved), marker: None }
    }
}

impl Backend for Native {
    fn recheck(&mut self) -> Result<(), String> { self.context.recheck() }

    fn probe(&mut self) -> Result<Probe, String> {
        purge_discovery::verified_native(self.approved.as_ref().ok_or("purge approval already released")?)
    }

    fn disable_recovery(&mut self) -> Result<(), String> {
        self.marker=Some(super::purge_marker::create(||self.context.recheck())?);
        Ok(())
    }

    fn uninstall(&mut self) -> Result<InstallerResult, String> {
        self.context.recheck()?;
        let approved = self.approved.as_ref().ok_or("purge approval missing")?;
        // Never pass package paths, recovery flags, transforms or admission overrides.
        let log = fixed_log()?;
        let mut child = Command::new(r"C:\Windows\System32\msiexec.exe")
            .args(["/x", approved.product_code(), "/qn", "/norestart", "REBOOT=ReallySuppress", "/L*v"])
            .arg(log).spawn().map_err(|error| format!("purge installer launch: {error}"))?;
        let deadline = Instant::now() + Duration::from_secs(300);
        loop {
            match child.try_wait() {
                Ok(Some(status)) => return status.code().map(|code| InstallerResult::Exited(code as u32))
                    .ok_or_else(|| "purge installer exit unknown; preserve and reconcile".into()),
                Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(100)),
                Ok(None) => {
                    eprintln!("purge installer pending; pid={}; do not retry", child.id());
                    return Ok(InstallerResult::Pending);
                }
                Err(error) => return Err(format!("purge installer wait unknown; pid={}; preserve and reconcile: {error}", child.id())),
            }
        }
    }

    fn release_approval(&mut self) { drop(self.approved.take()); drop(self.marker.take()); }

    fn cleanup(&mut self) -> Result<Outcome, String> {
        self.context.recheck()?;
        let current = std::env::current_exe().map_err(|e| e.to_string())?;
        let parent = current.parent().ok_or("purge helper directory absent")?;
        if current.file_name().and_then(|s| s.to_str()) != Some("purge-helper.exe")
            || parent.parent() != Some(Path::new(r"C:\ProgramData\RustDeskPurge")) {
            return Err("purge cleanup requires fixed external helper".into());
        }
        super::workspace::verify(parent)?;
        stop_remove_recovery_task(&self.context)?;
        for service in ["RustDeskManagementAgent", "RustDesk"] {
            super::purge_service::stop_and_delete(service, &self.context)?;
        }
        for service in ["RustDeskManagementAgent", "RustDesk"] {
            let deadline = Instant::now() + Duration::from_secs(30);
            loop {
                let status = Command::new(r"C:\Windows\System32\sc.exe").args(["query", service])
                    .status().map_err(|e| format!("purge service verify: {e}"))?;
                if status.code() == Some(1060) { break; }
                if !status.success() || Instant::now() >= deadline {
                    return Err(format!("purge service still present/unknown: {service}: {status}"));
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        }
        for path in [Path::new(NORMAL).to_path_buf(), Path::new(ROOT).to_path_buf()] {
            self.context.recheck()?;
            match std::fs::symlink_metadata(&path) {
                Ok(metadata) => {
                    use std::os::windows::fs::MetadataExt;
                    if !metadata.is_dir() || metadata.file_attributes() & 0x400 != 0 { return Err("purge asset directory invalid".into()); }
                    std::fs::remove_dir_all(&path).map_err(|e| format!("purge assets: {e}"))?;
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {},
                Err(e) => return Err(format!("purge asset observation: {e}")),
            }
            match std::fs::symlink_metadata(&path) {
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {},
                _ => return Err("purge assets remain or unknown".into()),
            }
        }
        Ok(Outcome::Complete)
    }
}

fn stop_remove_recovery_task(context: &Context) -> Result<(), String> {
    context.recheck()?;
    // Fixed task only. COM distinguishes missing registration from query/access failures.
    let script = r#"$ErrorActionPreference='Stop'; try {
        $scheduler=New-Object -ComObject 'Schedule.Service'; $scheduler.Connect();
        $folder=$scheduler.GetFolder('\'); $task=$null;
        try { $task=$folder.GetTask('RustDeskManagementAgentRecovery') }
        catch { $e=$_.Exception; while($e.InnerException){$e=$e.InnerException}; if ($e.HResult -ne -2147024894) { throw } }
        if ($task) {
            $task.Stop(0); $deadline=[DateTime]::UtcNow.AddSeconds(30);
            while ($task.GetInstances(0).Count -ne 0) {
                if ([DateTime]::UtcNow -ge $deadline) { throw 'purge recovery task still running' }
                Start-Sleep -Milliseconds 100
            }
        }
        exit 0
    } catch { [Console]::Error.WriteLine($_.Exception.ToString()); exit 1 }"#;
    let output = Command::new(r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", script]).output()
        .map_err(|e| format!("purge task lifecycle: {e}"))?;
    if !output.status.success() {
        return Err(format!("purge task lifecycle failed: {}; {}", output.status, String::from_utf8_lossy(&output.stderr)));
    }
    context.recheck()?;
    let delete = r#"$ErrorActionPreference='Stop'; try {
        $scheduler=New-Object -ComObject 'Schedule.Service'; $scheduler.Connect();
        $folder=$scheduler.GetFolder('\'); $task=$null;
        try { $task=$folder.GetTask('RustDeskManagementAgentRecovery') }
        catch { $e=$_.Exception; while($e.InnerException){$e=$e.InnerException}; if ($e.HResult -ne -2147024894) { throw } }
        if ($task) {
            if ($task.GetInstances(0).Count -ne 0) { throw 'purge task restarted; preserve registration' }
            $folder.DeleteTask('RustDeskManagementAgentRecovery',0)
        }
        try { $null=$folder.GetTask('RustDeskManagementAgentRecovery'); throw 'purge recovery task remains' }
        catch { $e=$_.Exception; while($e.InnerException){$e=$e.InnerException}; if ($e.HResult -ne -2147024894) { throw } }
        exit 0
    } catch { [Console]::Error.WriteLine($_.Exception.ToString()); exit 1 }"#;
    let output = Command::new(r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", delete]).output()
        .map_err(|e| format!("purge task deletion: {e}"))?;
    if !output.status.success() {
        return Err(format!("purge task deletion failed: {}; {}", output.status, String::from_utf8_lossy(&output.stderr)));
    }
    Ok(())
}

fn fixed_log() -> Result<std::path::PathBuf, String> {
    let current = std::env::current_exe().map_err(|e| e.to_string())?;
    let directory = current.parent().ok_or("purge log helper parent absent")?;
    if current.file_name().and_then(|v| v.to_str()) != Some("purge-helper.exe")
        || directory.parent() != Some(Path::new(r"C:\ProgramData\RustDeskPurge")) {
        return Err("purge log outside fixed helper".into());
    }
    super::workspace::verify(directory)?;
    let log = directory.join("purge-msi.log");
    match std::fs::symlink_metadata(&log) {
        Ok(_) => { super::workspace::verify_file(&log)?; }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            std::fs::OpenOptions::new().write(true).create_new(true).open(&log)
                .map_err(|e| format!("purge log create: {e}"))?;
        }
        Err(error) => return Err(format!("purge log query: {error}")),
    }
    Ok(log)
}

pub(super) fn start() -> Result<(), String> {
    let context = super::purge_preflight::acquire()?;
    let approved = purge_discovery::approved_installed()?;
    match purge_discovery::verified_native(&approved)? {
        Probe::RegisteredVerified | Probe::AbsentVerified => {},
        other => return Err(format!("purge identity refused: {other:?}")),
    }
    context.recheck()?;
    let id = uuid::Uuid::new_v4().simple().to_string();
    let directory = purge_handoff::prepare(&id, approved.installation_id(), approved.product_code())?;
    let helper = directory.join("purge-helper.exe");
    let status = Command::new(&helper)
        .args(["--purge-helper", "--transaction-id", &id])
        .spawn().map_err(|e| format!("purge helper launch: {e}"))?;
    let _ = status;
    Err(format!("purge pending; helper transaction {id}"))
}

pub(crate) fn run_helper(id: &str) -> Result<(), String> {
    let mut handoff = purge_handoff::open(id)?;
    handoff.wait_parent(Duration::from_secs(300))?;
    let installation_id = handoff.plan.installation_id.clone();
    let product_code = handoff.plan.product_code.clone();
    let outcome = handoff.execute_once(|| {
        let context = super::purge_preflight::acquire()?;
        let approved = purge_discovery::approved_installed()?;
        if approved.product_code() != product_code || approved.installation_id() != installation_id {
            return Err("purge helper installed identity binding mismatch".into());
        }
        purge_execution::execute(&mut Native::new(context, approved))
    })?;
    if outcome == Outcome::Complete { Ok(()) } else { Err(format!("purge finished: {outcome:?}")) }
}
