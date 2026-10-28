#![cfg(windows)]
#[path = "../src/platform/windows_system_dirs.rs"]
#[allow(dead_code)]
mod system_dirs;
// 代理源码里有 `crate::management_agent::system_dirs` 的写法，测试入口补同名路径。
#[allow(unused_imports)]
mod management_agent {
    pub(crate) use super::system_dirs;
}
#[path = "../src/management_agent/msi_assets.rs"]
mod msi_assets;
#[path = "../src/management_agent/msi_metadata.rs"]
mod msi_metadata;
#[path = "../src/management_agent/package_gate.rs"]
mod package_gate;
#[path = "../src/management_agent/workspace.rs"]
mod workspace;
#[path = "../src/management_agent/installer_process.rs"]
mod installer_process;
#[path = "../src/management_agent/install_execution.rs"]
mod install_execution;
#[path = "../src/management_agent/transaction.rs"]
mod transaction;
#[path = "../src/management_agent/handoff.rs"]
mod handoff;
#[path = "../src/management_agent/state_store.rs"]
mod state_store;
#[path = "../src/management_agent/directory_backup.rs"]
mod directory_backup;
#[path = "../src/management_agent/install_plan.rs"]
mod install_plan;

#[test]
#[ignore = "Requires verified v25 MSI and SYSTEM lab VM"]
fn approved_msi_assets_extract_without_changing_installed_services_or_identity() {
    use hbb_common::{base64::{engine::general_purpose::STANDARD, Engine as _}, sodiumoxide::crypto::sign, UpdateManifest, UpdateTarget};
    use sha2::{Digest, Sha256};
    use std::path::Path;
    use windows_service::{service::ServiceAccess, service_manager::{ServiceManager, ServiceManagerAccess}};
    base::lab_guard::require_lab();
    let package = Path::new(r"C:\RustDeskAgentLab\agent-health-v25.msi");
    let bytes = std::fs::read(package).unwrap();
    let hash = hex::encode(Sha256::digest(&bytes));
    assert_eq!(hash, "d158bbb84ff98b7b1ac84cd373181555f9b97a9a2782c6690aaaa8859f4956d3");
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT).unwrap();
    let snapshot = || {
        ["RustDesk", "RustDeskManagementAgent"].iter().map(|name| {
            let service = manager.open_service(name, ServiceAccess::QUERY_CONFIG | ServiceAccess::QUERY_STATUS).unwrap();
            let status = service.query_status().unwrap();
            let config = service.query_config().unwrap();
            (status.current_state, status.process_id, config.executable_path, config.account_name, config.start_type)
        }).collect::<Vec<_>>()
    };
    let before = snapshot();
    let state = Path::new(r"C:\ProgramData\RustDesk\ManagementAgent\state\agent.json");
    let before_state: serde_json::Value = serde_json::from_slice(&std::fs::read(state).unwrap()).unwrap();
    hbb_common::sodiumoxide::init().unwrap();
    let (public, secret) = sign::gen_keypair();
    let key = "windows-x86_64-msi-standard";
    let mut manifest = UpdateManifest { schema:2,product:"rustdesk-yan".into(),edition:"multi".into(),channel:"stable".into(),catalog_revision:2026100502,..Default::default() };
    let mut target = UpdateTarget {version:"1.5.3".into(),build_seq:2026100502,source_commit:"fixture-commit".into(),primary:"https://download.yan.life/rustdesk/stable/1.5.3/agent-health-v25.msi".into(),size:bytes.len() as u64,sha256:hash.clone(),signature_key_id:"fixture-key".into(),..Default::default()};
    target.signature=STANDARD.encode(sign::sign_detached(&base::update::update_signature_payload(&manifest,key,&target).unwrap(),&secret));
    manifest.targets.insert(key.into(),target);
    let root_path = std::path::PathBuf::from(r"C:\RustDeskAgentLab").join(format!("msi-extraction-{}", std::process::id()));
    let root = root_path.as_path();
    workspace::create(root).unwrap();
    assert!(msi_assets::extract(root, package, &manifest, key, 2026100502).is_err(), "fixture key must not authorize production");
    assert_eq!(std::fs::read_dir(root).unwrap().count(), 0);
    let extracted = msi_assets::extract_fixture(root, package, &manifest, key, public.as_ref()).unwrap();
    assert!(extracted.client.join("RustDesk.exe").is_file());
    assert!(extracted.client.join("data/flutter_assets").is_dir());
    assert_eq!(hex::encode(Sha256::digest(std::fs::read(extracted.client.join("librustdesk.dll")).unwrap())), "e678cba1da59a7d70cd64149436c041326f027999cb8047daab49290fe467769");
    assert_eq!(hex::encode(Sha256::digest(std::fs::read(&extracted.agent).unwrap())), "9c08ecb3b3bfc3eb6ebe90734251bfe5ac437196bddeaafa5be467a18465611b");
    let prepared = msi_assets::seal_target(root, &extracted).unwrap();
    prepared.client_manifest.verify(root).unwrap();
    prepared.agent.verify(root).unwrap();
    let manifest_bytes = std::fs::read(root.join(&prepared.client_manifest.relative_path)).unwrap();
    let inventory = directory_backup::DirectoryBackup::decode(&manifest_bytes).unwrap();
    assert!(inventory.matches_directory(&extracted.client).unwrap());
    assert_eq!(prepared.agent.sha256, "9c08ecb3b3bfc3eb6ebe90734251bfe5ac437196bddeaafa5be467a18465611b");
    assert!(msi_assets::seal_target(root, &extracted).is_err(), "existing prepared assets must not be overwritten");
    assert_eq!(std::fs::read(root.join(&prepared.client_manifest.relative_path)).unwrap(), manifest_bytes);
    assert_eq!(snapshot(), before);
    let after: serde_json::Value = serde_json::from_slice(&std::fs::read(state).unwrap()).unwrap();
    assert_eq!(after["agent_installation_id"], before_state["agent_installation_id"]);
    assert_eq!(after["status_session_generation"], before_state["status_session_generation"]);
    assert_eq!(hex::encode(Sha256::digest(std::fs::read(package).unwrap())), hash);
    assert!(msi_assets::extract_fixture(root, package, &manifest, key, public.as_ref()).is_err(), "existing extraction must never be overwritten");
    std::fs::remove_dir_all(root).unwrap();
}
