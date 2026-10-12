use super::{
    bootstrap_policy::{self, BootstrapAction, BootstrapObservation},
    recovery, recovery_windows, AgentPaths, ServiceConfig,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    process::Command,
};
use windows_service::{
    service::{ServiceAccess, ServiceStartType, ServiceState, ServiceType},
    service_manager::{ServiceManager, ServiceManagerAccess},
};

const TASK_NAME: &str = "RustDeskManagementAgentRecovery";

pub fn initialize_if_allowed(config: &ServiceConfig, paths: &AgentPaths, state: &super::AgentState) -> Result<bool, String> {
    let Some(_guard) = recovery_windows::install_guard::InstallGuard::try_acquire()
        .map_err(|error| error.to_string())? else {
        return Ok(false);
    };
    initialize_checked(config, paths, state, || recovery_windows::read_intent(), || install(config, paths))
}

fn initialize_checked(
    config: &ServiceConfig,
    paths: &AgentPaths,
    state: &super::AgentState,
    intent: impl FnOnce() -> Result<recovery::Intent, String>,
    install: impl FnOnce() -> Result<(), String>,
) -> Result<bool, String> {
    if state.disabled || !state.management_agent_enabled || state.agent_installation_id.is_empty()
        || config.edition != state.edition || config.package_kind != state.package_kind {
        return Ok(false);
    }
    for marker in ["disable-agent", "disable-recovery", "maintenance.json"] {
        match std::fs::symlink_metadata(paths.state.join(marker)) {
            Ok(_) => return Ok(false),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(format!("bootstrap initialization local gate unreadable: {error}")),
        }
    }
    if !paths.state.join("service-recovery-enabled").is_file() || intent()? != recovery::Intent::Enabled {
        return Ok(false);
    }
    install()?;
    Ok(true)
}

#[derive(Serialize, Deserialize)]
struct BootstrapManifest {
    schema: u32,
    agent_sha256: String,
    edition: super::Edition,
    package_kind: super::PackageKind,
    normal_install_path: String,
}

#[derive(Default, Serialize, Deserialize)]
struct BootstrapBudget {
    failures: u8,
    next_at: u64,
}

fn root(paths: &AgentPaths) -> PathBuf {
    paths.root.join("RecoveryBootstrap")
}

fn agent_path(paths: &AgentPaths) -> PathBuf {
    paths.agent.join("rustdesk-management-agent.exe")
}

fn file_hash(path: &Path) -> Result<String, String> {
    use std::os::windows::fs::MetadataExt;
    if !path.is_absolute() || !path.is_file() {
        return Err("bootstrap file unavailable".into());
    }
    for ancestor in path.ancestors() {
        if std::fs::symlink_metadata(ancestor)
            .map_err(|error| error.to_string())?
            .file_attributes()
            & 0x400
            != 0
        {
            return Err("bootstrap file path is a reparse point".into());
        }
    }
    use std::io::Read;
    let mut file = std::fs::File::open(path).map_err(|error| error.to_string())?;
    let mut hash = Sha256::new();
    let mut buffer = [0; 65536];
    loop {
        let length = file.read(&mut buffer).map_err(|error| error.to_string())?;
        if length == 0 {
            break;
        }
        hash.update(&buffer[..length]);
    }
    Ok(hex::encode(hash.finalize()))
}

fn save_budget(path: &Path, budget: &BootstrapBudget) -> Result<(), String> {
    use std::{io::Write, os::windows::ffi::OsStrExt};
    #[link(name = "kernel32")]
    extern "system" {
        fn MoveFileExW(source: *const u16, destination: *const u16, flags: u32) -> i32;
    }
    let temporary = path.with_extension(format!("tmp-{}", std::process::id()));
    let bytes = serde_json::to_vec(budget).map_err(|error| error.to_string())?;
    let mut file = std::fs::File::create(&temporary).map_err(|error| error.to_string())?;
    file.write_all(&bytes)
        .and_then(|_| file.sync_all())
        .map_err(|error| error.to_string())?;
    drop(file);
    let source = temporary
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let destination = path
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    if unsafe { MoveFileExW(source.as_ptr(), destination.as_ptr(), 1 | 8) } == 0 {
        let error = std::io::Error::last_os_error();
        if let Err(cleanup) = std::fs::remove_file(&temporary) {
            eprintln!("bootstrap temporary cleanup failed: {cleanup}");
        }
        return Err(format!("bootstrap budget publish failed: {error}"));
    }
    Ok(())
}

fn xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn task_output(bytes: &[u8]) -> String {
    if bytes.starts_with(&[0xff, 0xfe]) || (bytes.len() > 1 && bytes[1] == 0) {
        let bytes = bytes.strip_prefix(&[0xff, 0xfe]).unwrap_or(bytes);
        let units = bytes
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect::<Vec<_>>();
        String::from_utf16_lossy(&units)
    } else {
        String::from_utf8_lossy(bytes).into_owned()
    }
}

fn write_task_xml(path: &Path, source: &str) -> Result<(), String> {
    let source = source.replace("encoding=\"UTF-8\"", "encoding=\"UTF-16\"");
    let mut bytes = vec![0xff, 0xfe];
    for unit in source.encode_utf16() {
        bytes.extend_from_slice(&unit.to_le_bytes());
    }
    std::fs::write(path, bytes).map_err(|error| error.to_string())
}

pub fn task_xml(executable: &Path, config: &ServiceConfig) -> Result<String, String> {
    let path = executable
        .to_str()
        .ok_or_else(|| "bootstrap path encoding invalid".to_owned())?;
    if path.chars().any(char::is_control) {
        return Err("bootstrap path invalid".into());
    }
    let mut arguments = format!(
        "bootstrap --edition {} --package-kind {}",
        config.edition.name(),
        config.package_kind.extension()
    );
    if let Some(normal) = &config.normal_install_path {
        if !super::approved_normal_install_path(normal) {
            return Err("bootstrap normal path not approved".into());
        }
        let normal = normal
            .to_str()
            .ok_or_else(|| "normal path encoding invalid".to_owned())?;
        if normal.contains('"') || normal.chars().any(char::is_control) {
            return Err("normal path invalid".into());
        }
        arguments.push_str(&format!(
            " --normal-install-path \"{}\"",
            normal.trim_end_matches(['\\', '/'])
        ));
    }
    Ok(format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
<Triggers><BootTrigger><Enabled>true</Enabled><Delay>PT1M</Delay></BootTrigger><TimeTrigger><Repetition><Interval>PT5M</Interval><StopAtDurationEnd>false</StopAtDurationEnd></Repetition><StartBoundary>2026-01-01T00:00:00</StartBoundary><Enabled>true</Enabled></TimeTrigger></Triggers>
<Principals><Principal id="System"><UserId>S-1-5-18</UserId><RunLevel>HighestAvailable</RunLevel></Principal></Principals>
<Settings><MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy><DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries><StopIfGoingOnBatteries>false</StopIfGoingOnBatteries><StartWhenAvailable>true</StartWhenAvailable><ExecutionTimeLimit>PT1M</ExecutionTimeLimit><Enabled>true</Enabled></Settings>
<Actions Context="System"><Exec><Command>{}</Command><Arguments>{}</Arguments></Exec></Actions></Task>"#,
        xml(path),
        xml(&arguments)
    ))
}

fn system_executable(name: &str) -> PathBuf {
    std::env::var_os("SystemRoot")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Windows"))
        .join("System32")
        .join(name)
}

fn require_admin() -> Result<(), String> {
    #[link(name = "shell32")]
    extern "system" {
        fn IsUserAnAdmin() -> i32;
    }
    if unsafe { IsUserAnAdmin() != 0 } {
        Ok(())
    } else {
        Err("bootstrap requires administrator privileges".into())
    }
}

fn verify_existing(directory: &Path, digest: &str, config: &ServiceConfig) -> Result<(), String> {
    file_hash(&directory.join("bootstrap.json"))?;
    let bytes = std::fs::read(directory.join("bootstrap.json")).map_err(|e| e.to_string())?;
    let manifest: BootstrapManifest =
        serde_json::from_slice(&bytes).map_err(|_| "bootstrap manifest malformed".to_owned())?;
    let normal = config
        .normal_install_path
        .as_deref()
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_default();
    if manifest.schema != 1
        || manifest.agent_sha256 != digest
        || manifest.edition != config.edition
        || manifest.package_kind != config.package_kind
        || manifest.normal_install_path != normal
        || file_hash(&directory.join("rustdesk-recovery-bootstrap.exe"))? != digest
    {
        return Err("bootstrap drift detected; transactional upgrade required".into());
    }
    Ok(())
}

fn element<'a>(source: &'a str, name: &str) -> Result<&'a str, String> {
    let opening = format!("<{name}>");
    let closing = format!("</{name}>");
    if source.matches(&opening).count() != 1 || source.matches(&closing).count() != 1 {
        return Err(format!("bootstrap task {name} ambiguous"));
    }
    let start = source
        .find(&opening)
        .ok_or_else(|| format!("bootstrap task {name} missing"))?
        + opening.len();
    let end = source[start..]
        .find(&closing)
        .ok_or_else(|| format!("bootstrap task {name} malformed"))?
        + start;
    Ok(source[start..end].trim())
}

fn verify_task(source: &str, executable: &Path, config: &ServiceConfig) -> Result<(), String> {
    let expected = task_xml(executable, config)?;
    if source.contains("<!")
        || source.matches("<Exec>").count()
            + source.matches("<Exec/").count()
            + source.matches("<Exec ").count()
            != 1
        || source.matches("<Principal ").count() != 1
        || source.matches("<Actions ").count() != 1
        || !source.contains("<Actions Context=\"System\">")
    {
        return Err("bootstrap task actions/principal invalid".into());
    }
    for name in [
        "Command",
        "Arguments",
        "UserId",
        "RunLevel",
        "MultipleInstancesPolicy",
        "ExecutionTimeLimit",
    ] {
        if xml(element(source, name)?.replace("&quot;", "\"").as_str())
            != xml(element(&expected, name)?.replace("&quot;", "\"").as_str())
        {
            return Err(format!("bootstrap task {name} mismatch"));
        }
    }
    let settings = element(source, "Settings")?;
    let enabled = |source: &str| -> Result<bool, String> {
        if !source.contains("<Enabled") && !source.contains("</Enabled>") {
            Ok(true)
        } else {
            Ok(element(source, "Enabled")? == "true")
        }
    };
    if !enabled(settings)? {
        return Err("bootstrap task setting Enabled mismatch".into());
    }
    for name in [
        "StartWhenAvailable",
        "DisallowStartIfOnBatteries",
        "StopIfGoingOnBatteries",
    ] {
        if element(settings, name)? != element(element(&expected, "Settings")?, name)? {
            return Err(format!("bootstrap task setting {name} mismatch"));
        }
    }
    if !enabled(element(source, "BootTrigger")?)?
        || !enabled(element(source, "TimeTrigger")?)?
        || element(element(source, "Repetition")?, "Interval")? != "PT5M"
    {
        return Err("bootstrap task triggers mismatch".into());
    }
    Ok(())
}

fn verify_registered_task(executable: &Path, config: &ServiceConfig) -> Result<(), String> {
    let observed = Command::new(system_executable("schtasks.exe"))
        .args(["/Query", "/TN", TASK_NAME, "/XML"])
        .output()
        .map_err(|e| e.to_string())?;
    if !observed.status.success() {
        return Err(format!(
            "bootstrap task readback failed: {}",
            observed.status
        ));
    }
    verify_task(&task_output(&observed.stdout), executable, config)
}

pub fn install(config: &ServiceConfig, paths: &AgentPaths) -> Result<(), String> {
    require_admin()?;
    if super::purge_marker_path(paths).exists() {
        return Err("recovery locally disabled".into());
    }
    let source = agent_path(paths);
    let current = std::env::current_exe().map_err(|error| error.to_string())?;
    let digest = file_hash(&source)?;
    if file_hash(&current)? != digest {
        return Err("bootstrap must be installed by the active agent build".into());
    }
    let Some(_guard) = recovery_windows::install_guard::InstallGuard::try_acquire()
        .map_err(|error| error.to_string())?
    else {
        return Err("installation in progress".into());
    };
    let directory = root(paths);
    for ancestor in directory.ancestors().skip(1) {
        use std::os::windows::fs::MetadataExt;
        if std::fs::symlink_metadata(ancestor)
            .map_err(|error| error.to_string())?
            .file_attributes()
            & 0x400
            != 0
        {
            return Err("bootstrap directory ancestor is a reparse point".into());
        }
    }
    if directory.exists() {
        verify_existing(&directory, &digest, config)?;
        return verify_registered_task(&directory.join("rustdesk-recovery-bootstrap.exe"), config);
    }
    let executable = directory.join("rustdesk-recovery-bootstrap.exe");
    let task = task_xml(&executable, config)?;
    std::fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
    let protected = Command::new(system_executable("icacls.exe"))
        .arg(&directory)
        .args([
            "/inheritance:r",
            "/grant:r",
            "*S-1-5-18:(OI)(CI)F",
            "*S-1-5-32-544:(OI)(CI)F",
        ])
        .status()
        .map_err(|error| error.to_string())?;
    if !protected.success() {
        return Err(format!("bootstrap ACL failed: {protected}"));
    }
    std::fs::copy(source, &executable).map_err(|error| error.to_string())?;
    if file_hash(&executable)? != digest {
        return Err("bootstrap copy hash mismatch".into());
    }
    let manifest = BootstrapManifest {
        schema: 1,
        agent_sha256: digest,
        edition: config.edition,
        package_kind: config.package_kind,
        normal_install_path: config
            .normal_install_path
            .as_deref()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default(),
    };
    let bytes = serde_json::to_vec_pretty(&manifest).map_err(|error| error.to_string())?;
    std::fs::write(directory.join("bootstrap.json"), bytes).map_err(|error| error.to_string())?;
    let definition = directory.join("task.xml");
    write_task_xml(&definition, &task)?;
    // No /F: do not overwrite an unrelated or already-managed task.
    let registered = Command::new(system_executable("schtasks.exe"))
        .args(["/Create", "/TN", TASK_NAME, "/XML"])
        .arg(&definition)
        .status()
        .map_err(|error| error.to_string())?;
    if !registered.success() {
        return Err(format!("bootstrap task registration failed: {registered}"));
    }
    verify_registered_task(&executable, config)
}

fn agent_command_matches(observed: &str, expected: &str) -> bool {
    if observed.eq_ignore_ascii_case(expected) {
        return true;
    }
    let Some((path, arguments)) = expected
        .strip_prefix('"')
        .and_then(|value| value.split_once('"'))
    else {
        return false;
    };
    !path.chars().any(char::is_whitespace)
        && observed.eq_ignore_ascii_case(&format!("{path}{arguments}"))
}

pub fn run_once(config: &ServiceConfig, paths: &AgentPaths) -> Result<(), String> {
    require_admin()?;
    if super::purge_marker_path(paths).exists()
        || paths.state.join("disable-agent").exists()
        || paths.state.join("maintenance.json").exists()
    {
        return Ok(());
    }
    let directory = root(paths);
    let current = std::env::current_exe().map_err(|error| error.to_string())?;
    if current != directory.join("rustdesk-recovery-bootstrap.exe") {
        return Err("bootstrap runtime path mismatch".into());
    }
    file_hash(&current)?;
    let source =
        std::fs::read(directory.join("bootstrap.json")).map_err(|error| error.to_string())?;
    let manifest: BootstrapManifest =
        serde_json::from_slice(&source).map_err(|_| "bootstrap manifest malformed".to_owned())?;
    if manifest.schema != 1
        || manifest.edition != config.edition
        || manifest.package_kind != config.package_kind
        || manifest.normal_install_path
            != config
                .normal_install_path
                .as_deref()
                .map(|path| path.to_string_lossy().into_owned())
                .unwrap_or_default()
    {
        return Err("bootstrap configuration mismatch".into());
    }
    if file_hash(&current)? != manifest.agent_sha256
        || file_hash(&agent_path(paths))? != manifest.agent_sha256
    {
        return Err("agent integrity unavailable; no start attempted".into());
    }
    let Some(_guard) = recovery_windows::install_guard::InstallGuard::try_acquire()
        .map_err(|error| error.to_string())?
    else {
        return Ok(());
    };
    if super::purge_marker_path(paths).exists()
        || paths.state.join("disable-agent").exists()
        || paths.state.join("maintenance.json").exists()
    {
        return Ok(());
    }
    let budget_path = directory.join("budget.json");
    let mut budget = match std::fs::read(&budget_path) {
        Ok(bytes) => serde_json::from_slice::<BootstrapBudget>(&bytes)
            .map_err(|_| "bootstrap budget malformed".to_owned())?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => BootstrapBudget::default(),
        Err(error) => return Err(format!("bootstrap budget unavailable: {error}")),
    };
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)
        .map_err(|error| error.to_string())?;
    let service = manager
        .open_service(
            "RustDeskManagementAgent",
            ServiceAccess::QUERY_CONFIG | ServiceAccess::QUERY_STATUS | ServiceAccess::START,
        )
        .map_err(|error| error.to_string())?;
    let observed = service.query_status().map_err(|error| error.to_string())?;
    let registered = service.query_config().map_err(|error| error.to_string())?;
    let mut expected = format!(
        "\"{}\" --service --edition {} --package-kind {}",
        agent_path(paths).display(),
        config.edition.name(),
        config.package_kind.extension()
    );
    if !manifest.normal_install_path.is_empty() {
        expected.push_str(&format!(
            " --normal-install-path \"{}\"",
            manifest.normal_install_path.trim_end_matches(['\\', '/'])
        ));
    }
    let trusted = agent_command_matches(&registered.executable_path.to_string_lossy(), &expected)
        && registered.service_type == ServiceType::OWN_PROCESS
        && registered
            .account_name
            .as_ref()
            .map(|name| name.to_string_lossy().eq_ignore_ascii_case("LocalSystem"))
            .unwrap_or(false);
    let action = bootstrap_policy::decide(&BootstrapObservation {
        agent: match observed.current_state {
            ServiceState::Stopped => recovery::ServiceState::Stopped,
            ServiceState::Running => recovery::ServiceState::Running,
            _ => recovery::ServiceState::Unknown,
        },
        trusted_agent: trusted,
        disabled_start: registered.start_type == ServiceStartType::Disabled,
        agent_locally_disabled: super::purge_marker_path(paths).exists()
            || paths.state.join("disable-agent").exists(),
        maintenance: paths.state.join("maintenance.json").exists(),
        retry_due: super::unix_timestamp() >= budget.next_at,
    });
    if action == BootstrapAction::StartAgent {
        budget.failures = budget.failures.saturating_add(1);
        budget.next_at =
            super::unix_timestamp().saturating_add(if budget.failures <= 3 { 300 } else { 3600 });
        save_budget(&budget_path, &budget)?;
        service
            .start::<&str>(&[])
            .map_err(|error| format!("bootstrap start failed: {error}"))?;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        loop {
            let current = service
                .query_status()
                .map_err(|error| format!("bootstrap verify failed: {error}"))?;
            match current.current_state {
                ServiceState::Running => break,
                ServiceState::StartPending if std::time::Instant::now() < deadline => {
                    std::thread::sleep(std::time::Duration::from_millis(250))
                }
                _ => {
                    return Err(format!(
                        "agent did not reach running: {:?}, {:?}",
                        current.current_state, current.exit_code
                    ))
                }
            }
        }
    }
    Ok(())
}

pub fn remove_task() -> Result<(), String> {
    require_admin()?;
    let result = Command::new(system_executable("schtasks.exe"))
        .args(["/Delete", "/TN", TASK_NAME, "/F"])
        .status()
        .map_err(|error| error.to_string())?;
    if result.success() {
        Ok(())
    } else {
        Err(format!("bootstrap task removal failed: {result}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initialization_defers_while_another_installation_owns_the_machine_lock() {
        let directory = std::env::temp_dir().join(format!("agent-bootstrap-lock-{}", std::process::id()));
        let paths = AgentPaths::from_program_data(&directory, super::super::Edition::Standard);
        std::fs::create_dir_all(&paths.state).unwrap();
        std::fs::write(paths.state.join("service-recovery-enabled"), b"enabled").unwrap();
        let mut state = super::super::AgentState::new(super::super::Edition::Standard, &paths);
        state.agent_installation_id = "a".repeat(32);
        let config = ServiceConfig { edition:state.edition, package_kind:state.package_kind,
            normal_install_path:Some(PathBuf::from(r"C:\Program Files\RustDesk")) };
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let owner = std::thread::spawn(move || {
            let guard = recovery_windows::install_guard::InstallGuard::try_acquire().unwrap().unwrap();
            ready_tx.send(()).unwrap();
            release_rx.recv_timeout(std::time::Duration::from_secs(15)).unwrap();
            drop(guard);
        });
        ready_rx.recv_timeout(std::time::Duration::from_secs(15)).unwrap();
        let result = initialize_if_allowed(&config, &paths, &state);
        release_tx.send(()).unwrap();
        owner.join().unwrap();
        assert!(!root(&paths).exists());
        assert_eq!(std::fs::read(paths.state.join("service-recovery-enabled")).unwrap(), b"enabled");
        std::fs::remove_dir_all(directory).unwrap();
        assert_eq!(result, Ok(false), "busy installation must defer before intent and executable inspection");
    }

    #[test]
    fn bootstrap_initialization_checks_gates_before_creating_or_reading_anything() {
        let directory = std::env::temp_dir().join(format!("agent-bootstrap-initialize-{}", std::process::id()));
        let paths = AgentPaths::from_program_data(&directory, super::super::Edition::Standard);
        std::fs::create_dir_all(&paths.state).unwrap();
        let mut state = super::super::AgentState::new(super::super::Edition::Standard, &paths);
        state.agent_installation_id = "a".repeat(32);
        let config = ServiceConfig { edition:state.edition, package_kind:state.package_kind,
            normal_install_path:Some(PathBuf::from(r"C:\Program Files\RustDesk")) };
        let marker = paths.state.join("service-recovery-enabled");
        std::fs::write(&marker,b"enabled").unwrap();
        assert!(initialize_checked(&config,&paths,&state,||Ok(recovery::Intent::Enabled),||Ok(())).unwrap(),
            "an enabled installed agent must initialize its recovery entry");
        for gate in ["disable-agent","disable-recovery","maintenance.json"] {
            let path=paths.state.join(gate);
            std::fs::write(&path,b"retain local intent").unwrap();
            assert!(!initialize_checked(&config,&paths,&state,||panic!("blocked path must not inspect intent"),||panic!("blocked initialization")).unwrap());
            assert_eq!(std::fs::read(&path).unwrap(),b"retain local intent");
            assert!(!root(&paths).exists());
            std::fs::remove_file(path).unwrap();
        }
        for intent in [recovery::Intent::Disabled,recovery::Intent::Unknown] {
            assert!(!initialize_checked(&config,&paths,&state,||Ok(intent),||panic!("intent refused")).unwrap());
        }
        assert_eq!(initialize_checked(&config,&paths,&state,||Err("intent read failed".into()),||panic!("unreadable intent")).unwrap_err(),"intent read failed");
        assert_eq!(initialize_checked(&config,&paths,&state,||Ok(recovery::Intent::Enabled),||Err("registration refused".into())).unwrap_err(),"registration refused");
        state.disabled=true;
        assert!(!initialize_checked(&config,&paths,&state,||panic!("disabled"),||panic!("disabled")).unwrap());
        state.disabled=false;state.management_agent_enabled=false;
        assert!(!initialize_checked(&config,&paths,&state,||panic!("policy disabled"),||panic!("policy disabled")).unwrap());
        state.management_agent_enabled=true;
        std::fs::remove_file(&marker).unwrap();
        assert!(!initialize_checked(&config,&paths,&state,||panic!("marker absent"),||panic!("marker absent")).unwrap());
        assert!(!root(&paths).exists());
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn task_readback_requires_exact_action_arguments_and_security_settings() {
        let config = ServiceConfig {
            edition: super::super::Edition::Standard,
            package_kind: super::super::PackageKind::Exe,
            normal_install_path: Some(PathBuf::from(r"C:\Program Files\RustDesk")),
        };
        let executable =
            Path::new(r"C:\ProgramData\RustDesk\RecoveryBootstrap\rustdesk-recovery-bootstrap.exe");
        let definition = task_xml(executable, &config).unwrap();
        assert!(verify_task(&definition, executable, &config).is_ok());
        for invalid in [
            definition.replace("package-kind exe", "package-kind msi"),
            definition.replace("IgnoreNew", "Parallel"),
            definition.replace("HighestAvailable", "LeastPrivilege"),
            definition.replace("PT1M</ExecutionTimeLimit>", "PT1H</ExecutionTimeLimit>"),
            definition.replace(
                "<Enabled>true</Enabled></Settings>",
                "<Enabled>false</Enabled></Settings>",
            ),
            definition.replace("<Exec>", "<Exec/><Exec>"),
        ] {
            assert!(verify_task(&invalid, executable, &config).is_err());
        }
    }

    #[test]
    fn agent_command_accepts_unquoted_space_free_path_without_relaxing_arguments() {
        let expected = r#""C:\ProgramData\RustDesk\ManagementAgent\rustdesk-management-agent.exe" --service --edition standard --package-kind msi --normal-install-path "C:\Program Files\RustDesk""#;
        let observed = expected.replacen('"', "", 1).replacen(".exe\"", ".exe", 1);
        assert!(agent_command_matches(&observed, expected));
        assert!(agent_command_matches(expected, expected));
        assert!(!agent_command_matches(
            &observed.replace("--edition standard", "--edition sos"),
            expected
        ));
        assert!(!agent_command_matches(
            &(observed.clone() + " --extra"),
            expected
        ));
        assert!(!agent_command_matches(
            &observed.replace("ManagementAgent", "OtherAgent"),
            expected
        ));
        assert!(!agent_command_matches(
            r"C:\Program Files\Agent.exe --service",
            r#""C:\Program Files\Agent.exe" --service"#
        ));
    }

    #[test]
    fn scheduler_readback_accepts_literal_quotes_but_rejects_changed_arguments() {
        let executable =
            Path::new(r"C:\ProgramData\RustDesk\RecoveryBootstrap\rustdesk-recovery-bootstrap.exe");
        let config = ServiceConfig {
            edition: super::super::Edition::Standard,
            package_kind: super::super::PackageKind::Msi,
            normal_install_path: Some(PathBuf::from(r"C:\Program Files\RustDesk")),
        };
        let definition = task_xml(executable, &config).unwrap();
        let readback = definition.replace("&quot;", "\"");
        verify_task(&readback, executable, &config).unwrap();
        let changed = readback.replace("--edition standard", "--edition sos");
        assert!(verify_task(&changed, executable, &config).is_err());
        let defaults_omitted = readback.replace("<Enabled>true</Enabled>", "");
        verify_task(&defaults_omitted, executable, &config).unwrap();
        let disabled = defaults_omitted.replace("<Settings>", "<Settings><Enabled>false</Enabled>");
        assert!(verify_task(&disabled, executable, &config).is_err());
    }

    #[test]
    fn identical_existing_bootstrap_is_idempotent_but_drift_is_not_reapproved() {
        let directory = std::env::temp_dir().join(format!(
            "rustdesk-bootstrap-existing-{}-{}",
            std::process::id(),
            hex::encode(hbb_common::sodiumoxide::randombytes::randombytes(8))
        ));
        std::fs::create_dir(&directory).unwrap();
        let executable = directory.join("rustdesk-recovery-bootstrap.exe");
        std::fs::write(&executable, b"approved bootstrap").unwrap();
        let digest = file_hash(&executable).unwrap();
        let config = ServiceConfig {
            edition: super::super::Edition::Standard,
            package_kind: super::super::PackageKind::Exe,
            normal_install_path: None,
        };
        let manifest = BootstrapManifest {
            schema: 1,
            agent_sha256: digest.clone(),
            edition: config.edition,
            package_kind: config.package_kind,
            normal_install_path: String::new(),
        };
        let path = directory.join("bootstrap.json");
        std::fs::write(&path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        assert!(verify_existing(&directory, &digest, &config).is_ok());
        assert!(verify_existing(&directory, &"0".repeat(64), &config).is_err());
        std::fs::write(&executable, b"changed").unwrap();
        assert!(verify_existing(&directory, &digest, &config).is_err());
        let before = std::fs::read(&path).unwrap();
        assert_eq!(before, serde_json::to_vec(&manifest).unwrap());
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn scheduled_task_is_system_single_instance_periodic_and_boot_triggered() {
        let config = ServiceConfig {
            edition: super::super::Edition::Sos,
            package_kind: super::super::PackageKind::Msi,
            normal_install_path: Some(PathBuf::from(r"C:\Program Files\RustDesk")),
        };
        let xml = task_xml(
            Path::new(r"C:\ProgramData\RustDesk\RecoveryBootstrap\rustdesk-recovery-bootstrap.exe"),
            &config,
        )
        .unwrap();
        for expected in [
            "<BootTrigger>",
            "<Interval>PT5M</Interval>",
            "<UserId>S-1-5-18</UserId>",
            "<MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>",
            "<ExecutionTimeLimit>PT1M</ExecutionTimeLimit>",
            "bootstrap --edition sos --package-kind msi",
            "&quot;C:\\Program Files\\RustDesk&quot;",
        ] {
            assert!(xml.contains(expected), "missing {expected}");
        }
        assert!(!xml.contains("powershell"));
        assert!(!xml.contains("cmd.exe"));
    }

    #[test]
    fn bootstrap_budget_replacement_is_readable_and_retains_previous_on_bad_path() {
        let directory = std::env::temp_dir().join(format!(
            "rustdesk-bootstrap-budget-{}-{}",
            std::process::id(),
            hex::encode(hbb_common::sodiumoxide::randombytes::randombytes(8))
        ));
        std::fs::create_dir(&directory).unwrap();
        let path = directory.join("budget.json");
        save_budget(
            &path,
            &BootstrapBudget {
                failures: 1,
                next_at: 1000,
            },
        )
        .unwrap();
        save_budget(
            &path,
            &BootstrapBudget {
                failures: 2,
                next_at: 2000,
            },
        )
        .unwrap();
        let state: BootstrapBudget =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(state.failures, 2);
        assert_eq!(state.next_at, 2000);
        assert!(save_budget(
            &directory.join("missing/budget.json"),
            &BootstrapBudget::default()
        )
        .is_err());
        let state: BootstrapBudget =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(state.next_at, 2000);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn scheduler_xml_roundtrip_uses_disabled_unique_fixture_task() {
        require_admin().expect("scheduler fixture requires admin");
        let suffix = hex::encode(hbb_common::sodiumoxide::randombytes::randombytes(8));
        let name = format!("RustDeskAgentTest-{}-{suffix}", std::process::id());
        let config = ServiceConfig {
            edition: super::super::Edition::Standard,
            package_kind: super::super::PackageKind::Exe,
            normal_install_path: Some(PathBuf::from(r"C:\Program Files\RustDesk")),
        };
        let definition = task_xml(&system_executable("whoami.exe"), &config)
            .unwrap()
            .replace("<Enabled>true</Enabled>", "<Enabled>false</Enabled>");
        let file = std::env::temp_dir().join(format!(
            "rustdesk-agent-task-{}-{suffix}.xml",
            std::process::id()
        ));
        write_task_xml(&file, &definition).unwrap();
        let status = Command::new(system_executable("schtasks.exe"))
            .args(["/Create", "/TN", &name, "/XML"])
            .arg(&file)
            .output()
            .unwrap();
        std::fs::remove_file(&file).unwrap();
        assert!(
            status.status.success(),
            "scheduler create: {} {}",
            task_output(&status.stdout),
            task_output(&status.stderr)
        );
        let observed = Command::new(system_executable("schtasks.exe"))
            .args(["/Query", "/TN", &name, "/XML"])
            .output()
            .unwrap();
        let cleanup = Command::new(system_executable("schtasks.exe"))
            .args(["/Delete", "/TN", &name, "/F"])
            .output()
            .unwrap();
        assert!(cleanup.status.success(), "fixture task cleanup failed");
        assert!(observed.status.success());
        let source = task_output(&observed.stdout);
        let enabled_fixture = source.replace("<Enabled>false</Enabled>", "<Enabled>true</Enabled>");
        verify_task(&enabled_fixture, &system_executable("whoami.exe"), &config).unwrap();
        for expected in [
            "<Interval>PT5M</Interval>",
            "S-1-5-18",
            "IgnoreNew",
            "<Enabled>false</Enabled>",
            "whoami.exe",
        ] {
            assert!(
                source.contains(expected),
                "scheduler readback missing {expected}: {source}"
            );
        }
    }
}
