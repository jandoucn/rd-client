#![cfg(windows)]
#[path = "../src/management_agent/bootstrap_update.rs"] mod bootstrap_update;
#[path = "../src/management_agent/preparation.rs"] mod preparation;
#[path = "../src/management_agent/msi_assets.rs"] mod msi_assets;
#[path = "../src/management_agent/msi_metadata.rs"] mod msi_metadata;
#[path = "../src/management_agent/package_gate.rs"] mod package_gate;
#[path = "../src/management_agent/package_download.rs"] mod package_download;
#[path = "../src/management_agent/workspace.rs"] mod workspace;
#[path = "../src/management_agent/installer_process.rs"] mod installer_process;
#[path = "../src/management_agent/install_execution.rs"] mod install_execution;
#[path = "../src/management_agent/transaction.rs"] mod transaction;
#[path = "../src/management_agent/handoff.rs"] mod handoff;
#[path = "../src/management_agent/state_store.rs"] mod state_store;
#[path = "../src/management_agent/directory_backup.rs"] mod directory_backup;
#[path = "../src/management_agent/install_plan.rs"] mod install_plan;
#[path = "../src/management_agent/native_install.rs"] mod native_install;
#[path = "../src/management_agent/file_restore.rs"] mod file_restore;
#[path = "../src/management_agent/service_control.rs"] mod service_control;
#[path = "../src/management_agent/health.rs"] mod health;
#[path = "../src/management_agent/health_windows.rs"] mod health_windows;
#[path = "../src/platform/windows_agent_guard.rs"] mod install_guard;
#[path = "../src/management_agent/parent_handoff.rs"] mod parent_handoff;
#[path = "../src/management_agent/helper_runtime.rs"] mod helper_runtime;
#[path = "../src/management_agent/maintenance.rs"] mod maintenance;
#[path = "../src/management_agent/resume_task.rs"] mod resume_task;

#[test]
fn native_transaction_child() {
    let Some(root) = std::env::var_os("RUSTDESK_HELPER_RUNTIME_FIXTURE") else { return; };
    use hbb_common::base64::{engine::general_purpose::STANDARD, Engine as _};
    use std::{path::PathBuf, time::{Duration, Instant}};
    base::lab_guard::require_lab();
    let root=PathBuf::from(root);
    assert!(root.starts_with(r"C:\RustDeskAgentLab"));
    let public=STANDARD.decode(std::env::var("RUSTDESK_HELPER_FIXTURE_PUBLIC").unwrap()).unwrap();
    let deadline=Instant::now()+Duration::from_secs(120);
    while !root.join("handoff.json").exists() && Instant::now()<deadline { std::thread::sleep(Duration::from_millis(100)); }
    helper_runtime::prepare_ready(&root).unwrap();
    helper_runtime::wait_for_release(&root).unwrap();
    let deadline=Instant::now()+Duration::from_secs(10);
    let mut owner=loop {
        match helper_runtime::take_over_native_fixture(&root,&public) {
            Ok(owner)=>break owner,
            Err(error) if error.contains("guard busy") && Instant::now()<deadline=>std::thread::sleep(Duration::from_millis(100)),
            Err(error)=>panic!("native helper takeover: {error}"),
        }
    };
    let mut backend=native_install::NativeInstaller::open(&root).unwrap();
    let result=owner.execute(&mut backend).unwrap();
    assert_eq!(result.phase,transaction::Phase::Committed,"{:?}",result.error);
    owner.finalize().unwrap();
}

#[test]
#[ignore = "Requires SYSTEM lab VM with verified healthy v25 or v65 package"]
fn real_preparation_binds_signed_assets_and_preserves_running_installation() {
    use hbb_common::{base64::{engine::general_purpose::STANDARD, Engine as _}, sodiumoxide::crypto::sign, UpdateManifest, UpdateTarget};
    use sha2::{Digest, Sha256};
    use std::path::{Path, PathBuf};
    use windows_service::{service::ServiceAccess, service_manager::{ServiceManager, ServiceManagerAccess}};
    base::lab_guard::require_lab();
    let requested_package = std::env::var("RUSTDESK_PREPARATION_PACKAGE").unwrap_or_default();
    let activity_package = requested_package == "v65";
    let package_name = if requested_package == "v139" { "agent-auto-bootstrap-v139.msi" }
        else if requested_package == "v128" { "agent-msi-bootstrap-v128.msi" }
        else if activity_package { "agent-versioned-v65.msi" } else { "agent-health-v25.msi" };
    let package_path = Path::new(r"C:\RustDeskAgentLab").join(package_name);
    let package = package_path.as_path();
    let bytes = std::fs::read(package).unwrap();
    let hash = hex::encode(Sha256::digest(&bytes));
    assert_eq!(hash, if requested_package == "v139" { "5c4e4508ee116d64d4314d289cb7b0ad0d6d1e6c213d3607b3722a697ac05979" }
        else if requested_package == "v128" { "682a6f4da21ba2a01863496dceb48e9b6ebcf853fe55d383f6702ae0234dfd90" }
        else if activity_package { "b332083068c9346b6b381e459a63a22f3c2deef73088325c5225b14826467050" }
        else { "d158bbb84ff98b7b1ac84cd373181555f9b97a9a2782c6690aaaa8859f4956d3" });
    let state_path = Path::new(r"C:\ProgramData\RustDesk\ManagementAgent\state\agent.json");
    let before: serde_json::Value = serde_json::from_slice(&std::fs::read(state_path).unwrap()).unwrap();
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT).unwrap();
    let snapshot = || ["RustDesk","RustDeskManagementAgent"].iter().map(|name| {
        let svc=manager.open_service(name,ServiceAccess::QUERY_CONFIG|ServiceAccess::QUERY_STATUS).unwrap();
        let status=svc.query_status().unwrap(); let config=svc.query_config().unwrap();
        (status.current_state,status.process_id,config.executable_path,config.account_name,config.start_type)
    }).collect::<Vec<_>>();
    let services = snapshot();
    hbb_common::sodiumoxide::init().unwrap();
    let (public,secret)=sign::gen_keypair();
    let key="windows-x86_64-msi-standard";
    let mut manifest=UpdateManifest {schema:2,product:"rustdesk-yan".into(),edition:"multi".into(),channel:"stable".into(),catalog_revision:2026100502,..Default::default()};
    let mut target=UpdateTarget {version:"1.5.3".into(),build_seq:2026100502,source_commit:"fixture-commit".into(),primary:format!("https://download.yan.life/rustdesk/stable/1.5.3/{package_name}"),size:bytes.len() as u64,sha256:hash,signature_key_id:"fixture-key".into(),..Default::default()};
    target.signature=STANDARD.encode(sign::sign_detached(&base::update::update_signature_payload(&manifest,key,&target).unwrap(),&secret));
    manifest.targets.insert(key.into(),target);
    let download_root = PathBuf::from(r"C:\RustDeskAgentLab").join(format!("http-packages-{}",std::process::id()));
    let downloaded = if std::env::var("RUSTDESK_PREPARATION_HTTP").ok().as_deref() == Some("yes") {
        use std::{io::{Read, Write}, net::TcpListener, time::{Duration, Instant}};
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        listener.set_nonblocking(true).unwrap();
        let source = package.to_path_buf();
        let server = std::thread::spawn(move || {
            let mut received = Vec::new();
            let deadline = Instant::now()+Duration::from_secs(120);
            for _ in 0..2 {
                let mut stream = loop {
                    match listener.accept() {
                        Ok((stream,_)) => break stream,
                        Err(error) if error.kind()==std::io::ErrorKind::WouldBlock && Instant::now()<deadline => std::thread::sleep(Duration::from_millis(20)),
                        Err(error) => panic!("local package server: {error}"),
                    }
                };
                stream.set_nonblocking(false).unwrap();
                stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
                stream.set_write_timeout(Some(Duration::from_secs(60))).unwrap();
                let mut request = Vec::new();
                while !request.ends_with(b"\r\n\r\n") {
                    let mut byte=[0];stream.read_exact(&mut byte).unwrap();request.push(byte[0]);
                    assert!(request.len()<8192);
                }
                assert!(String::from_utf8_lossy(&request).starts_with(&format!("GET /{package_name} HTTP/1.1\r\n")));
                let mut file = std::fs::File::open(&source).unwrap();
                let size = file.metadata().unwrap().len();
                write!(stream,"HTTP/1.1 200 OK\r\nContent-Length: {size}\r\nConnection: close\r\n\r\n").unwrap();
                assert_eq!(std::io::copy(&mut file,&mut stream).unwrap(),size);
                received.push(size);
            }
            received
        });
        workspace::create(&download_root).unwrap();
        let target = package_gate::verify_with_key(&manifest,key,2026100502,"fixture-key",public.as_ref()).unwrap();
        assert!(package_gate::verify(&manifest,key,2026100502).is_err());
        let current = package_download::download_fixture(&download_root.join("target"),target,address).unwrap();
        let rollback = package_download::download_fixture(&download_root.join("rollback"),target,address).unwrap();
        assert_eq!(server.join().unwrap(),vec![target.size,target.size]);
        println!("Local HTTP downloaded and verified target and rollback: {} bytes each",target.size);
        Some((current,rollback))
    } else { None };
    let target_path = downloaded.as_ref().map(|value|value.0.0.as_path()).unwrap_or(package);
    let rollback_path = downloaded.as_ref().map(|value|value.1.0.as_path()).unwrap_or(package);
    let root=PathBuf::from(r"C:\RustDeskAgentLab").join(format!("preparation-{}",std::process::id()));
    println!("Prepared transaction evidence: {}", root.display());
    let id=hex::encode(sign::gen_keypair().0.as_ref())[..32].to_owned();
    let command="d".repeat(32);
    let request=preparation::Request {transaction_id:&id,command_id:&command,installation_id:before["agent_installation_id"].as_str().unwrap(),target_key:key,current_build:2026100502,package:target_path,manifest:&manifest,rollback_package:rollback_path,rollback_manifest:&manifest};
    assert!(preparation::prepare(&root,&request).is_err(),"fixture authority must not enter production preparation");
    assert!(!root.exists());
    let mut invalid = preparation::Request { installation_id: "00000000000000000000000000000000", ..request };
    assert_eq!(preparation::prepare_fixture(&root,&invalid,public.as_ref()).unwrap_err(), "installation preparation identity or policy mismatch");
    assert!(!root.exists());
    invalid.installation_id = request.installation_id;
    invalid.current_build += 1;
    assert!(preparation::prepare_fixture(&root,&invalid,public.as_ref()).unwrap_err().contains("downgrade"));
    assert!(!root.exists());
    let mut changed = manifest.clone();
    changed.targets.get_mut(key).unwrap().sha256 = "0".repeat(64);
    invalid.current_build = request.current_build;
    invalid.rollback_manifest = &changed;
    assert!(preparation::prepare_fixture(&root,&invalid,public.as_ref()).unwrap_err().contains("signature"));
    assert!(!root.exists());
    assert_eq!(snapshot(), services);
    let plan=preparation::prepare_fixture(&root,&request,public.as_ref()).unwrap();
    assert_eq!(plan.command_id.as_deref(),Some(command.as_str()));
    assert_eq!(plan.installation_id,request.installation_id);
    assert!(plan.package.relative_path.ends_with(package_name));
    plan.verify_assets(&root).unwrap();
    let tx=transaction::Transaction::decode(&std::fs::read(root.join("transaction.json")).unwrap()).unwrap();
    assert_eq!(tx.phase,transaction::Phase::PackageVerified);
    plan.bind_transaction(&tx).unwrap();
    native_install::NativeInstaller::open(&root).unwrap();
    let bootstrap_record: serde_json::Value = serde_json::from_slice(
        &std::fs::read(root.join("bootstrap-update.json")).expect("bootstrap preparation must be durable before service stop")
    ).unwrap();
    assert_eq!(bootstrap_record["transaction_id"], plan.transaction_id);
    assert_eq!(bootstrap_record["plan_sha256"], hex::encode(Sha256::digest(serde_json::to_vec(&plan).unwrap())));
    assert_eq!(snapshot(),services);
    let after:serde_json::Value=serde_json::from_slice(&std::fs::read(state_path).unwrap()).unwrap();
    assert_eq!(after["agent_installation_id"],before["agent_installation_id"]);
    assert_eq!(after["status_session_generation"],before["status_session_generation"]);
    assert!(!state_path.parent().unwrap().join("maintenance.json").exists());
    let prepared=std::fs::read(root.join("prepared.json")).unwrap();
    assert!(preparation::prepare_fixture(&root,&request,public.as_ref()).is_err());
    assert_eq!(std::fs::read(root.join("prepared.json")).unwrap(),prepared);
    if std::env::var("RUSTDESK_PREPARATION_NATIVE_EXECUTE").ok().as_deref()==Some("yes") {
        let mut child=parent_handoff::launch_native_fixture(&root,public.as_ref()).unwrap();
        let deadline=std::time::Instant::now()+std::time::Duration::from_secs(600);
        while child.try_wait().unwrap().is_none() && std::time::Instant::now()<deadline {
            std::thread::sleep(std::time::Duration::from_millis(250));
        }
        let status=child.try_wait().unwrap().expect("native helper timeout; preserve running installer");
        assert!(status.success(),"{}",std::fs::read_to_string(root.join("parent-helper.stderr.log")).unwrap());
        let tx=transaction::Transaction::decode(&std::fs::read(root.join("transaction.json")).unwrap()).unwrap();
        assert_eq!(tx.phase,transaction::Phase::Committed);
        assert!(!state_path.parent().unwrap().join("maintenance.json").exists());
        assert!(resume_task::read_back(&plan.transaction_id).is_err());
        let status:serde_json::Value=serde_json::from_slice(&std::fs::read(state_path).unwrap()).unwrap();
        assert_eq!(status["agent_installation_id"],before["agent_installation_id"]);
        assert!(native_install::client_ready(std::path::Path::new(r"C:\Program Files\RustDesk\RustDesk.exe"),2026100502,true));
        for service in snapshot() { assert_eq!(service.0,windows_service::service::ServiceState::Running); }
        println!("native transaction committed, identity retained, application ready, maintenance and task removed");
        println!("Retained registered MSI source and transaction evidence: {}", root.display());
        drop(downloaded);
        if download_root.exists() { std::fs::remove_dir_all(download_root).unwrap(); }
        return;
    }
    std::fs::remove_dir_all(root).unwrap();
    drop(downloaded);
    if download_root.exists() { std::fs::remove_dir_all(download_root).unwrap(); }
}
