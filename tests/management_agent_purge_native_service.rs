#![cfg(windows)]
#[path = "../src/platform/windows_system_dirs.rs"]
#[allow(dead_code)]
mod system_dirs;
// 代理源码里有 `crate::management_agent::system_dirs` 的写法，测试入口补同名路径。
#[allow(unused_imports)]
mod management_agent {
    pub(crate) use super::system_dirs;
}
#[path="../src/management_agent/handoff.rs"] mod handoff;
#[path="../src/platform/windows_agent_guard.rs"] mod install_guard;
#[path="../src/management_agent/purge_preflight.rs"] mod purge_preflight;
#[path="../src/management_agent/purge_service.rs"] mod purge_service;
use std::{path::Path, time::{Duration, Instant}};
use windows_service::{service::*, service_manager::*};
const NAME: &str = "RustDeskAgentTestService-PurgeExit";
const ROOT: &str = r"C:\RustDeskAgentLab\purge-native-service";
windows_service::define_windows_service!(ffi_main, service_main);
fn service_main(_: Vec<std::ffi::OsString>) {
    let (tx,rx)=std::sync::mpsc::channel();
    let handle=windows_service::service_control_handler::register(NAME,move |event| {
        if event==ServiceControl::Stop { tx.send(()).unwrap(); }
        windows_service::service_control_handler::ServiceControlHandlerResult::NoError
    }).unwrap();
    let mut state=ServiceStatus { service_type:ServiceType::OWN_PROCESS,current_state:ServiceState::Running,
        controls_accepted:ServiceControlAccept::STOP,exit_code:ServiceExitCode::Win32(0),checkpoint:0,wait_hint:Duration::ZERO,process_id:None };
    handle.set_service_status(state.clone()).unwrap();rx.recv_timeout(Duration::from_secs(60)).unwrap();
    state.current_state=ServiceState::StopPending;state.wait_hint=Duration::from_secs(5);state.checkpoint=1;
    handle.set_service_status(state.clone()).unwrap();std::thread::sleep(Duration::from_millis(250));
    state.current_state=ServiceState::Stopped;state.controls_accepted=ServiceControlAccept::empty();
    handle.set_service_status(state).unwrap();
    std::fs::write(Path::new(ROOT).join("stopped-alive"), b"alive").unwrap();
    let deadline=Instant::now()+Duration::from_secs(15);
    while !Path::new(ROOT).join("release-exit").exists() && Instant::now()<deadline { std::thread::sleep(Duration::from_millis(25)); }
}
#[test]
fn native_purge_service_child() {
    if std::env::current_exe().unwrap()==Path::new(ROOT).join("fixture.exe") {
        windows_service::service_dispatcher::start(NAME,ffi_main).unwrap();
        // The dispatcher returns on SERVICE_STOPPED before the callback thread
        // necessarily returns. Keep the executable alive independently of it.
        let deadline=Instant::now()+Duration::from_secs(15);
        while !Path::new(ROOT).join("release-exit").exists() && Instant::now()<deadline { std::thread::sleep(Duration::from_millis(25)); }
    }
}
#[test]
fn native_purge_service_waits_for_actual_process_exit() {
    service_case(false);
}
#[test]
fn native_purge_service_initial_stopped_is_not_exited() {
    service_case(true);
}
fn service_case(initial_stopped: bool) {
    base::lab_guard::require_lab_os("win11");
    let root=Path::new(ROOT);std::fs::create_dir(root).unwrap();
    std::fs::create_dir(root.join("payload")).unwrap();
    std::fs::write(root.join("payload/keep"),b"preserve until actual exit").unwrap();
    std::fs::copy(std::env::current_exe().unwrap(),root.join("fixture.exe")).unwrap();
    let manager=ServiceManager::local_computer(None::<&str>,ServiceManagerAccess::CONNECT|ServiceManagerAccess::CREATE_SERVICE).unwrap();
    let info=ServiceInfo { name:NAME.into(),display_name:NAME.into(),service_type:ServiceType::OWN_PROCESS,
        start_type:ServiceStartType::OnDemand,error_control:ServiceErrorControl::Normal,executable_path:root.join("fixture.exe"),
        launch_arguments:vec!["--exact".into(),"native_purge_service_child".into(),"--nocapture".into()],dependencies:vec![],account_name:None,account_password:None };
    let service=manager.create_service(&info,ServiceAccess::QUERY_CONFIG|ServiceAccess::QUERY_STATUS|ServiceAccess::START|ServiceAccess::STOP|ServiceAccess::DELETE).unwrap();
    let tested=std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        service.start::<&str>(&[]).unwrap();
        let deadline=Instant::now()+Duration::from_secs(10);
        while service.query_status().unwrap().current_state!=ServiceState::Running { assert!(Instant::now()<deadline);std::thread::sleep(Duration::from_millis(25)); }
        let pid=service.query_status().unwrap().process_id.unwrap();
        let original=handoff::ObservedProcess::open(pid).unwrap();
        if initial_stopped {
            service.stop().unwrap();
            while service.query_status().unwrap().current_state!=ServiceState::Stopped {
                assert!(Instant::now()<deadline);std::thread::sleep(Duration::from_millis(10));
            }
            assert!(original.is_alive().unwrap());
            assert_eq!(service.query_status().unwrap().process_id,None);
        }
        let monitor=std::thread::spawn(move || {
            let observed=handoff::ObservedProcess::open(pid).unwrap();
            let manager=ServiceManager::local_computer(None::<&str>,ServiceManagerAccess::CONNECT).unwrap();
            let service=manager.open_service(NAME,ServiceAccess::QUERY_STATUS).unwrap();
            let deadline=Instant::now()+Duration::from_secs(10);
            loop { let s=service.query_status().unwrap();
                if s.current_state==ServiceState::Stopped {
                    assert!(observed.is_alive().unwrap(),"fixture must reproduce stopped-but-alive");
                    std::thread::sleep(Duration::from_secs(1));
                    assert!(observed.is_alive().unwrap());
                    assert_eq!(std::fs::read(Path::new(ROOT).join("payload/keep")).unwrap(),b"preserve until actual exit");
                    let manager=ServiceManager::local_computer(None::<&str>,ServiceManagerAccess::CONNECT).unwrap();
                    assert!(manager.open_service(NAME,ServiceAccess::QUERY_STATUS).is_ok(),"registration deleted before actual exit");
                    std::fs::write(Path::new(ROOT).join("release-exit"),b"exit now").unwrap();break;
                }
                assert!(Instant::now()<deadline);std::thread::sleep(Duration::from_millis(10));
            }
        });
        let context=purge_preflight::acquire_fixture(&root.join("maintenance.json")).unwrap();
        let start=Instant::now();let result=purge_service::stop_fixture(&context);
        monitor.join().unwrap();
        if let Err(error)=result {
            assert!(initial_stopped);
            assert_eq!(error,"purge process image query failed");
            assert_eq!(std::fs::read(root.join("payload/keep")).unwrap(),b"preserve until actual exit");
            assert!(manager.open_service(NAME,ServiceAccess::QUERY_STATUS).is_ok());
            let deadline=Instant::now()+Duration::from_secs(10);
            while original.is_alive().unwrap() {assert!(Instant::now()<deadline);std::thread::sleep(Duration::from_millis(25));}
            println!("EXIT_ENUMERATION_RACE=refused;error={error};registration_and_payload_preserved=true;retry_after_confirmed_exit=true");
            purge_service::stop_fixture(&context).unwrap();
        }
        assert!(!original.is_alive().unwrap(),"SCM stopped is not proof of process exit");
        assert!(start.elapsed()>=Duration::from_millis(800));
        assert!(root.join("stopped-alive").is_file());
        std::fs::remove_dir_all(root.join("payload")).unwrap();
        assert!(!root.join("payload").exists());
        println!("SCM_STOPPED_WHILE_ALIVE=observed;initial_stopped={initial_stopped};pid={pid};delete_waited_for_exit=true;payload_preserved_until_exit=true");
    }));
    std::fs::write(root.join("release-exit"),b"cleanup").unwrap();
    let _=service.stop();let _=service.delete();drop(service);drop(manager);
    std::thread::sleep(Duration::from_secs(4));std::fs::remove_dir_all(root).unwrap();
    if let Err(e)=tested { std::panic::resume_unwind(e); }
}

#[test]
fn native_purge_com_missing_and_maintenance_preserves_task() {
    use std::process::Command;
    base::lab_guard::require_lab_os("win11");
    const TASK: &str="RustDeskAgentTestTask-PurgeCom";
    fn run(script: &str) -> std::process::Output {
        Command::new(r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe")
            .args(["-NoProfile","-NonInteractive","-Command",script]).output().unwrap()
    }
    let source=include_str!("../src/management_agent/purge_native.rs");
    let extract=|name: &str| {
        source.split_once(&format!("let {name} = r#\"")).unwrap().1.split_once("\"#;").unwrap().0
            .replace("RustDeskManagementAgentRecovery",TASK)
    };
    let stop=extract("script");let delete=extract("delete");
    let missing=format!(r#"$ErrorActionPreference='Stop';$s=New-Object -ComObject Schedule.Service;$s.Connect();$f=$s.GetFolder('\');try{{$null=$f.GetTask('{TASK}');throw 'fixture occupied'}}catch{{$e=$_.Exception;while($e.InnerException){{$e=$e.InnerException}};if($e.HResult -ne -2147024894){{throw}};[Console]::WriteLine('COM_MISSING_HRESULT=0x80070002')}}"#);
    let absent=run(&missing);assert!(absent.status.success(),"{}",String::from_utf8_lossy(&absent.stderr));
    assert!(String::from_utf8_lossy(&absent.stdout).contains("COM_MISSING_HRESULT=0x80070002"));
    let root=Path::new(ROOT);std::fs::create_dir(root).unwrap();
    let context=purge_preflight::acquire_fixture(&root.join("maintenance.json")).unwrap();
    let tested=std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        context.recheck().unwrap();let out=run(&stop);assert!(out.status.success(),"{}",String::from_utf8_lossy(&out.stderr));
        context.recheck().unwrap();let out=run(&delete);assert!(out.status.success(),"{}",String::from_utf8_lossy(&out.stderr));
        let create=format!(r#"$ErrorActionPreference='Stop';$a=New-ScheduledTaskAction -Execute 'C:\Windows\System32\cmd.exe' -Argument '/c exit 0';Register-ScheduledTask -TaskName '{TASK}' -Action $a -User SYSTEM -RunLevel Highest|Out-Null;Export-ScheduledTask -TaskName '{TASK}'"#);
        let before=run(&create);assert!(before.status.success(),"{}",String::from_utf8_lossy(&before.stderr));
        context.recheck().unwrap();let out=run(&stop);assert!(out.status.success());
        std::fs::write(root.join("maintenance.json"),b"external maintenance").unwrap();
        assert_eq!(context.recheck().unwrap_err(),"purge execution maintenance present");
        let after=run(&format!("$ErrorActionPreference='Stop';Export-ScheduledTask -TaskName '{TASK}'"));
        assert!(after.status.success());assert_eq!(before.stdout,after.stdout);
        assert_eq!(std::fs::read(root.join("maintenance.json")).unwrap(),b"external maintenance");
        println!("COM_MISSING_HRESULT=0x80070002;actual_stop_and_delete_scripts=success;maintenance_recheck=refused;task_xml_preserved=true");
    }));
    let clean=run(&format!("$ErrorActionPreference='Stop';Unregister-ScheduledTask -TaskName '{TASK}' -Confirm:$false"));
    assert!(clean.status.success(),"{}",String::from_utf8_lossy(&clean.stderr));
    let absent=run(&missing);assert!(absent.status.success());
    drop(context);std::fs::remove_dir_all(root).unwrap();
    if let Err(e)=tested { std::panic::resume_unwind(e); }
}
