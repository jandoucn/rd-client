#![cfg(windows)]
#[path = "../src/management_agent/service_control.rs"]
mod service_control;

use service_control::{inspect, start, stop, Contract};
use std::{path::PathBuf, time::Duration};
use windows_service::{
    service::{
        ServiceAccess, ServiceControl, ServiceControlAccept, ServiceErrorControl, ServiceExitCode,
        ServiceInfo, ServiceStartType, ServiceState, ServiceStatus, ServiceType,
    },
    service_manager::{ServiceManager, ServiceManagerAccess},
};

fn fixture_contract(name: &str, exe: PathBuf) -> Contract {
    Contract {
        name: name.into(),
        executable: exe,
        arguments: vec![
            "--exact".into(),
            "native_service_fixture".into(),
            "--nocapture".into(),
        ],
        start_type: ServiceStartType::OnDemand,
    }
}

#[test]
fn contract_rejects_unrelated_service_unquoted_space_path_and_extra_arguments() {
    let exe = PathBuf::from(r"C:\Program Files\RustDesk\RustDesk.exe");
    let contract = Contract {
        name: "RustDesk".into(),
        executable: exe.clone(),
        arguments: vec!["--service".into()],
        start_type: ServiceStartType::AutoStart,
    };
    assert!(contract.matches_command(r#""C:\Program Files\RustDesk\RustDesk.exe" --service"#));
    assert!(!contract.matches_command(r"C:\Program Files\RustDesk\RustDesk.exe --service"));
    assert!(
        !contract.matches_command(r#""C:\Program Files\RustDesk\RustDesk.exe" --service --extra"#)
    );
    let mut other = contract.clone();
    other.name = "Unrelated".into();
    assert!(other.validate().is_err());
    other.name = "RustDesk".into();
    other.executable = PathBuf::from("relative.exe");
    assert!(other.validate().is_err());
}

windows_service::define_windows_service!(ffi_fixture_main, fixture_main);
fn fixture_main(_arguments: Vec<std::ffi::OsString>) {
    let name = std::env::current_exe()
        .unwrap()
        .file_stem()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    let (tx, rx) = std::sync::mpsc::channel();
    let handler=windows_service::service_control_handler::register(&name,move|event|{
        match event{ServiceControl::Stop=>{tx.send(()).unwrap();windows_service::service_control_handler::ServiceControlHandlerResult::NoError},ServiceControl::Interrogate=>windows_service::service_control_handler::ServiceControlHandlerResult::NoError,_=>windows_service::service_control_handler::ServiceControlHandlerResult::NotImplemented}
    }).unwrap();
    let mut status = ServiceStatus {
        service_type: ServiceType::OWN_PROCESS,
        current_state: ServiceState::Running,
        controls_accepted: ServiceControlAccept::STOP,
        exit_code: ServiceExitCode::Win32(0),
        checkpoint: 0,
        wait_hint: Duration::ZERO,
        process_id: None,
    };
    handler.set_service_status(status.clone()).unwrap();
    rx.recv_timeout(Duration::from_secs(120)).unwrap();
    status.current_state = ServiceState::Stopped;
    status.controls_accepted = ServiceControlAccept::empty();
    handler.set_service_status(status).unwrap();
}

#[test]
fn native_service_fixture() {
    let exe = std::env::current_exe().unwrap();
    let name = exe.file_stem().unwrap().to_string_lossy();
    if name.starts_with("RustDeskAgentTestService-") {
        windows_service::service_dispatcher::start(name.as_ref(), ffi_fixture_main).unwrap();
        return;
    }
    let id = hex::encode(hbb_common::sodiumoxide::randombytes::randombytes(8));
    let name = format!("RustDeskAgentTestService-{}-{id}", std::process::id());
    let root = std::env::temp_dir().join(&name);
    std::fs::create_dir(&root).unwrap();
    let fixture = root.join(format!("{name}.exe"));
    std::fs::copy(&exe, &fixture).unwrap();
    let manager = ServiceManager::local_computer(
        None::<&str>,
        ServiceManagerAccess::CONNECT | ServiceManagerAccess::CREATE_SERVICE,
    )
    .unwrap();
    let info = ServiceInfo {
        name: name.clone().into(),
        display_name: name.clone().into(),
        service_type: ServiceType::OWN_PROCESS,
        start_type: ServiceStartType::OnDemand,
        error_control: ServiceErrorControl::Normal,
        executable_path: fixture.clone(),
        launch_arguments: vec![
            "--exact".into(),
            "native_service_fixture".into(),
            "--nocapture".into(),
        ],
        dependencies: vec![],
        account_name: None,
        account_password: None,
    };
    let created = manager
        .create_service(&info, ServiceAccess::QUERY_CONFIG | ServiceAccess::QUERY_STATUS | ServiceAccess::START | ServiceAccess::STOP | ServiceAccess::CHANGE_CONFIG | ServiceAccess::DELETE)
        .unwrap();
    let contract = fixture_contract(&name, fixture);
    let tested = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let initial = inspect(&contract).unwrap();
        assert_eq!(initial.state, ServiceState::Stopped);
        start(&contract, Duration::from_secs(15)).unwrap();
        let running = inspect(&contract).unwrap();
        assert_eq!(running.state, ServiceState::Running);
        assert!(running.pid.unwrap() > 0);
        let mut wrong = contract.clone();
        wrong.arguments.push("--extra".into());
        assert!(stop(&wrong, Duration::from_secs(15))
            .unwrap_err()
            .contains("contract"));
        assert_eq!(inspect(&contract).unwrap().state, ServiceState::Running);
        stop(&contract, Duration::from_secs(15)).unwrap();
        assert_eq!(inspect(&contract).unwrap().state, ServiceState::Stopped);
        start(&contract, Duration::from_secs(15)).unwrap();
        stop(&contract, Duration::from_secs(15)).unwrap();
        let mut disabled = info.clone();
        disabled.start_type = ServiceStartType::Disabled;
        created.change_config(&disabled).unwrap();
        assert!(start(&contract, Duration::from_secs(15)).is_err());
        assert_eq!(
            created.query_status().unwrap().current_state,
            ServiceState::Stopped
        );
    }));
    let _ = created.stop();
    created.delete().unwrap();
    drop(created);
    drop(manager);
    std::fs::remove_dir_all(root).unwrap();
    if let Err(payload) = tested {
        std::panic::resume_unwind(payload);
    }
}
