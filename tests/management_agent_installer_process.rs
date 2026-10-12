#![cfg(windows)]
#[path = "../src/management_agent/handoff.rs"]
mod handoff;
#[path = "../src/management_agent/install_execution.rs"]
mod install_execution;
#[path = "../src/management_agent/installer_process.rs"]
mod installer_process;
#[path = "../src/management_agent/package_gate.rs"]
mod package_gate;
#[path = "../src/management_agent/transaction.rs"]
mod transaction;
#[path = "../src/management_agent/state_store.rs"]
mod state_store;
#[path = "../src/management_agent/workspace.rs"]
mod workspace;

use install_execution::InstallOutcome;
use installer_process::{msi_arguments, run};

#[test]
fn msi_arguments_are_fixed_silent_no_restart_and_do_not_touch_defender_or_shortcuts() {
    let package = std::path::Path::new(r"C:\ProgramData\RustDesk\transactions\a\package.msi");
    let log = std::path::Path::new(r"C:\ProgramData\RustDesk\transactions\a\installer.log");
    let args = msi_arguments(package, log).unwrap();
    for arg in [
        "/i",
        "/qn",
        "/norestart",
        "REBOOT=ReallySuppress",
        "LAUNCH_TRAY_APP=N",
        "RUSTDESK_AGENT_RECOVERY=1",
    ] {
        assert!(args.iter().any(|v| v == arg));
    }
    assert!(!args
        .iter()
        .any(|v| v.to_string_lossy().contains("DEFENDER_EXCLUSIONS=1")));
    assert!(msi_arguments(std::path::Path::new("relative.msi"), log).is_err());
    assert!(msi_arguments(std::path::Path::new(r"C:\package.exe"), log).is_err());
}

#[test]
fn nested_msi_and_log_paths_use_windows_separators_without_changing_targets() {
    let root = std::path::Path::new(r"C:\ProgramData\RustDesk\transactions\a");
    let package = root.join("original/agent-health-v25.msi");
    let log = root.join("logs/rollback.log");
    let args = msi_arguments(&package, &log).unwrap();
    assert_eq!(args[1], r"C:\ProgramData\RustDesk\transactions\a\original\agent-health-v25.msi");
    assert_eq!(args.last().unwrap(), r"C:\ProgramData\RustDesk\transactions\a\logs\rollback.log");
    assert_eq!(package.file_name().unwrap(), "agent-health-v25.msi");
}

#[test]
fn native_installer_fixture() {
    if std::env::var_os("RUSTDESK_INSTALLER_PROCESS_FIXTURE").is_some() {
        let code = std::env::var("RUSTDESK_INSTALLER_PROCESS_EXIT")
            .unwrap()
            .parse::<i32>()
            .unwrap();
        let evidence = std::env::var_os("RUSTDESK_INSTALLER_PROCESS_EVIDENCE").unwrap();
        std::fs::write(
            evidence,
            std::env::args().skip(1).collect::<Vec<_>>().join("\n"),
        )
        .unwrap();
        std::process::exit(code);
    }
    let root =
        std::env::temp_dir().join(format!("rustdesk-installer-process-{}", std::process::id()));
    std::fs::create_dir(&root).unwrap();
    for (code, expected) in [
        (0, InstallOutcome::Installed),
        (3010, InstallOutcome::RebootRequired),
        (1618, InstallOutcome::Busy),
        (1603, InstallOutcome::Failed(1603)),
        (1641, InstallOutcome::Failed(1641)),
    ] {
        let evidence = root.join(format!("args-{code}.txt"));
        let mut command = std::process::Command::new(std::env::current_exe().unwrap());
        command
            .args(["--exact", "native_installer_fixture", "--nocapture"])
            .env("RUSTDESK_INSTALLER_PROCESS_FIXTURE", "yes")
            .env("RUSTDESK_INSTALLER_PROCESS_EXIT", code.to_string())
            .env("RUSTDESK_INSTALLER_PROCESS_EVIDENCE", &evidence);
        assert_eq!(
            run(&mut command, std::time::Duration::from_secs(5)).unwrap(),
            expected
        );
        assert!(std::fs::read_to_string(evidence)
            .unwrap()
            .contains("native_installer_fixture"));
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn timeout_preserves_active_installer_and_returns_reconciliation_instead_of_success() {
    let executable = std::env::var_os("SystemRoot")
        .map(std::path::PathBuf::from)
        .unwrap()
        .join("System32/ping.exe");
    let mut command = std::process::Command::new(executable);
    command
        .args(["-n", "20", "127.0.0.1"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    let timer = std::time::Instant::now();
    let outcome = run(&mut command, std::time::Duration::from_millis(100)).unwrap();
    let InstallOutcome::Pending { pid, created_at } = outcome else {
        panic!("timeout must return an active process identity");
    };
    let observed = handoff::ObservedProcess::open(pid).unwrap();
    assert_eq!(observed.owner().created_at, created_at);
    assert!(created_at > 0);
    let status = std::process::Command::new(
        std::env::var_os("SystemRoot")
            .map(std::path::PathBuf::from)
            .unwrap()
            .join("System32/taskkill.exe"),
    )
    .args(["/PID", &pid.to_string(), "/F"])
    .output()
    .unwrap();
    assert!(
        status.status.success(),
        "fixture process must still be alive for explicit test cleanup"
    );
    assert!(timer.elapsed() < std::time::Duration::from_secs(3));
}

#[test]
fn already_exited_short_process_has_a_result_even_if_process_identity_cannot_be_opened() {
    let executable = std::env::current_exe().unwrap();
    let root=std::env::temp_dir().join(format!("rustdesk-installer-short-{}",std::process::id()));
    std::fs::create_dir(&root).unwrap();
    for _ in 0..20 {
        let mut command = std::process::Command::new(&executable);
        command.args(["--exact","native_installer_fixture","--nocapture"])
            .env("RUSTDESK_INSTALLER_PROCESS_FIXTURE","yes")
            .env("RUSTDESK_INSTALLER_PROCESS_EXIT","0")
            .env("RUSTDESK_INSTALLER_PROCESS_EVIDENCE",root.join("result.txt"))
            .stdout(std::process::Stdio::null());
        assert_eq!(
            run(&mut command, std::time::Duration::from_secs(5)).unwrap(),
            InstallOutcome::Installed
        );
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn durable_result_reuses_completion_and_refuses_uncertain_launch_without_reexecution() {
    let root=std::env::temp_dir().join(format!("rustdesk-recorded-installer-{}",std::process::id()));
    workspace::create(&root).unwrap();
    let evidence = root.join("fixture-executed.txt");
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    command.args(["--exact","native_installer_fixture","--nocapture"])
        .env("RUSTDESK_INSTALLER_PROCESS_FIXTURE","yes")
        .env("RUSTDESK_INSTALLER_PROCESS_EXIT","0")
        .env("RUSTDESK_INSTALLER_PROCESS_EVIDENCE",&evidence)
        .stdout(std::process::Stdio::null());
    let id="a".repeat(32); let hash="b".repeat(64);
    assert_eq!(installer_process::run_recorded(&mut command, std::time::Duration::from_secs(5), &root, "install", &id, &hash).unwrap(), InstallOutcome::Installed);
    assert_eq!(installer_process::read_recorded(&root, "install", &id, &hash).unwrap(), Some(InstallOutcome::Installed));
    std::fs::write(&evidence,b"do-not-execute-again").unwrap();
    assert_eq!(installer_process::run_recorded(&mut command, std::time::Duration::from_secs(5), &root, "install", &id, &hash).unwrap(), InstallOutcome::Installed);
    assert_eq!(std::fs::read(&evidence).unwrap(),b"do-not-execute-again");
    let receipt = root.join("install-result.json");
    let before = std::fs::read(&receipt).unwrap();
    assert!(installer_process::read_recorded(&root, "install", &"c".repeat(32), &hash).unwrap_err().contains("binding"));
    assert!(installer_process::run_recorded(&mut command,std::time::Duration::from_secs(5),&root,"install",&id,&"c".repeat(64)).unwrap_err().contains("binding"));
    assert_eq!(std::fs::read(&receipt).unwrap(),before);
    std::fs::remove_file(&receipt).unwrap();
    assert_eq!(installer_process::read_recorded(&root, "install", &id, &hash).unwrap(), None);
    assert!(installer_process::run_recorded(&mut command,std::time::Duration::from_secs(5),&root,"install",&id,&hash).unwrap_err().contains("reconciliation"));
    assert_eq!(std::fs::read(&evidence).unwrap(),b"do-not-execute-again");
    assert!(!receipt.exists());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn outcome_write_failure_preserves_launch_intent_and_prevents_second_installer() {
    let root = std::env::temp_dir().join(format!("rustdesk-recorded-failure-{}", std::process::id()));
    workspace::create(&root).unwrap();
    let evidence = root.join("fixture-executed.txt");
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    command.args(["--exact", "native_installer_fixture", "--nocapture"])
        .env("RUSTDESK_INSTALLER_PROCESS_FIXTURE", "yes")
        .env("RUSTDESK_INSTALLER_PROCESS_EXIT", "0")
        .env("RUSTDESK_INSTALLER_PROCESS_EVIDENCE", &evidence)
        .stdout(std::process::Stdio::null());
    // Occupy the atomic-publication staging file, not the final receipt.
    let staging = root.join(format!("install-result.tmp-{}", std::process::id()));
    std::fs::write(&staging, b"keep-existing-staging").unwrap();
    let error = installer_process::run_recorded(&mut command, std::time::Duration::from_secs(5), &root, "install", &"a".repeat(32), &"b".repeat(64)).unwrap_err();
    assert!(error.contains("result persistence failed"));
    assert!(error.contains("reconciliation"));
    assert!(root.join("install-started.json").is_file());
    assert!(!root.join("install-result.json").exists());
    assert_eq!(std::fs::read(&staging).unwrap(), b"keep-existing-staging");
    assert!(evidence.is_file(), "installer actually ran before receipt failure");
    std::fs::write(&evidence, b"do-not-run-again").unwrap();
    assert!(installer_process::run_recorded(&mut command, std::time::Duration::from_secs(5), &root, "install", &"a".repeat(32), &"b".repeat(64)).unwrap_err().contains("reconciliation"));
    assert_eq!(std::fs::read(&evidence).unwrap(), b"do-not-run-again");
    std::fs::remove_dir_all(root).unwrap();
}
