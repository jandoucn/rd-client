#![cfg(windows)]
#[path = "../src/platform/windows_system_dirs.rs"]
#[allow(dead_code)]
mod system_dirs;
// 代理源码里有 `crate::management_agent::system_dirs` 的写法，测试入口补同名路径。
#[allow(unused_imports)]
mod management_agent {
    pub(crate) use super::system_dirs;
}
#[path = "../src/management_agent/bootstrap_update.rs"]
mod bootstrap_update;
#[path = "../src/management_agent/directory_backup.rs"]
mod directory_backup;
#[path = "../src/management_agent/file_restore.rs"]
mod file_restore;
#[path = "../src/management_agent/handoff.rs"]
mod handoff;
#[path = "../src/management_agent/health.rs"]
mod health;
#[path = "../src/management_agent/health_windows.rs"]
mod health_windows;
#[path = "../src/management_agent/install_execution.rs"]
mod install_execution;
#[path = "../src/platform/windows_agent_guard.rs"]
mod install_guard;
#[path = "../src/management_agent/install_plan.rs"]
mod install_plan;
#[path = "../src/management_agent/installer_process.rs"]
mod installer_process;
#[path = "../src/management_agent/msi_metadata.rs"]
mod msi_metadata;
#[path = "../src/management_agent/native_install.rs"]
mod native_install;
#[path = "../src/management_agent/package_gate.rs"]
mod package_gate;
#[path = "../src/management_agent/service_control.rs"]
mod service_control;
#[path = "../src/management_agent/state_store.rs"]
mod state_store;
#[path = "../src/management_agent/transaction.rs"]
mod transaction;
#[path = "../src/management_agent/workspace.rs"]
mod workspace;

use install_plan::{Artifact, InstallPlan, OriginalService, Trigger};
use sha2::{Digest, Sha256};
use std::path::Path;
use transaction::{Evidence, Phase, Transaction};

const ID: &str = "e2360000000000000000000000000001";
const OLD: &str = "{E2350000-0000-0000-0000-000000000001}";
const TARGET: &str = "{E2330000-0000-0000-0000-000000000003}";

fn root() -> std::path::PathBuf {
    Path::new(r"C:\ProgramData\RustDesk\ManagementAgent\transactions").join(ID)
}

fn product_state(code: &str) -> i32 {
    #[link(name = "msi")]
    extern "system" {
        fn MsiQueryProductStateW(code: *const u16) -> i32;
    }
    let code = code.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
    unsafe { MsiQueryProductStateW(code.as_ptr()) }
}

fn require_vm() {
    base::lab_guard::require_lab_os("win11");
    assert_eq!(
        std::env::var("RUSTDESK_ADMISSION_NATIVE_LAB").unwrap(),
        "v236"
    );
    assert_eq!(option_env!("RUSTDESK_MSI_ADMISSION"), Some("1"));
}

fn evidence() -> Evidence {
    Evidence {
        package_verified: true,
        backups_verified: true,
        helper_verified: true,
        resume_task_verified: true,
        handoff_verified: true,
        maintenance_owned: true,
        services_stopped: false,
        target_verified: false,
        identity_unchanged: false,
        client_ready: false,
        agent_ready: false,
        rollback_verified: false,
        locally_blocked: false,
    }
}

#[test]
#[ignore = "Win11 SYSTEM only: verified SOS0502 snapshot and fixed v235-old/v233-failure MSI"]
fn native_failure_then_agent_rollback_restores_old_product_and_health() {
    use windows_service::{
        service::ServiceAccess,
        service_manager::{ServiceManager, ServiceManagerAccess},
    };
    require_vm();
    assert_eq!(product_state(OLD), 5);
    assert_eq!(product_state(TARGET), -1);
    let root = root();
    workspace::create(&root).unwrap();
    let normal = Path::new(r"C:\Program Files\RustDesk");
    let agent = Path::new(r"C:\ProgramData\RustDesk\ManagementAgent\rustdesk-management-agent.exe");
    let identity = Path::new(
        r"C:\Windows\ServiceProfiles\LocalService\AppData\Roaming\RustDesk\config\RustDesk.toml",
    );
    let installation: serde_json::Value = serde_json::from_slice(
        &std::fs::read(agent.parent().unwrap().join("state/agent.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        installation["agent_installation_id"],
        "dcc73dd2fba22bf04c7df497053ae2df"
    );
    let original = Path::new(r"C:\RustDeskAgentLab\agent-sos-admission-v235-old.msi");
    let target = Path::new(r"C:\RustDeskAgentLab\agent-sos-admission-v233-failure.msi");
    assert_eq!(
        hex::encode(Sha256::digest(std::fs::read(original).unwrap())),
        "92d7169cb6fe6a978809cabcaf9e4be10693015f3b26e69201217af79895c29d"
    );
    assert_eq!(
        hex::encode(Sha256::digest(std::fs::read(target).unwrap())),
        "54a56f31eac198f87526cbed8cc31df44c4121619e91565beba8e996a1dbc41a"
    );
    for name in ["backups", "runtime", "original"] {
        workspace::create(&root.join(name)).unwrap();
    }
    let backup = directory_backup::capture(normal, &root.join("client-backup")).unwrap();
    for (source, destination) in [
        (original, "original/agent-sos-admission-v235-old.msi"),
        (target, "agent-sos-admission-v233-failure.msi"),
        (agent, "backups/agent.exe"),
        (agent, "target-agent.exe"),
        (identity, "backups/identity.toml"),
    ] {
        std::fs::copy(source, root.join(destination)).unwrap();
    }
    std::fs::copy(
        std::env::current_exe().unwrap(),
        root.join("runtime/helper.exe"),
    )
    .unwrap();
    std::fs::write(
        root.join("target-client.json"),
        serde_json::to_vec(&backup).unwrap(),
    )
    .unwrap();
    let artifact = |relative: &str| {
        let bytes = std::fs::read(root.join(relative)).unwrap();
        Artifact {
            relative_path: relative.into(),
            size: bytes.len() as u64,
            sha256: hex::encode(Sha256::digest(bytes)),
        }
    };
    let manager =
        ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT).unwrap();
    let services = ["RustDesk", "RustDeskManagementAgent"]
        .iter()
        .map(|name| {
            let service = manager
                .open_service(
                    name,
                    ServiceAccess::QUERY_CONFIG | ServiceAccess::QUERY_STATUS,
                )
                .unwrap();
            let config = service.query_config().unwrap();
            assert_eq!(
                config.start_type,
                windows_service::service::ServiceStartType::AutoStart
            );
            assert_eq!(
                service.query_status().unwrap().current_state,
                windows_service::service::ServiceState::Running
            );
            OriginalService {
                name: (*name).into(),
                command: config.executable_path.to_string_lossy().into_owned(),
                account: config.account_name.unwrap().to_string_lossy().into_owned(),
                start_type: "auto".into(),
                running: true,
            }
        })
        .collect();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let plan = InstallPlan {
        schema: 1,
        transaction_id: ID.into(),
        trigger: Trigger::LocalRecovery,
        command_id: None,
        installation_id: installation["agent_installation_id"]
            .as_str()
            .unwrap()
            .into(),
        identity_fingerprint: artifact("backups/identity.toml").sha256,
        target_key: "windows-x86_64-msi-sos".into(),
        original_version: "1.5.3".into(),
        original_build_seq: 2026100502,
        target_version: "1.5.3".into(),
        target_build_seq: 2026100502,
        normal_install_path: normal.to_string_lossy().into_owned(),
        package: artifact("agent-sos-admission-v233-failure.msi"),
        client_backup: artifact("client-backup/manifest.json"),
        agent_backup: artifact("backups/agent.exe"),
        identity_backup: artifact("backups/identity.toml"),
        helper: artifact("runtime/helper.exe"),
        helper_version: "1.5.3".into(),
        resume_task: format!("RustDeskManagementAgentResume-{ID}"),
        created_at: now,
        services,
    };
    let proof = evidence();
    let mut tx = Transaction::new(
        ID.into(),
        plan.target_key.clone(),
        plan.package.sha256.clone(),
        plan.identity_fingerprint.clone(),
        now,
    )
    .unwrap();
    for phase in [
        Phase::PackageVerified,
        Phase::Prepared,
        Phase::StoppingServices,
    ] {
        tx.advance(phase, &proof, now).unwrap();
    }
    tx.save(&root.join("transaction.json")).unwrap();
    plan.save_bound(&root.join("prepared.json"), &tx).unwrap();
    let assets = native_install::NativeAssets {
        schema: 2,
        transaction_id: ID.into(),
        client_target_manifest: artifact("target-client.json"),
        agent_target: artifact("target-agent.exe"),
        original_msi: artifact("original/agent-sos-admission-v235-old.msi"),
        original_product_code: OLD.into(),
        target_product_code: TARGET.into(),
        identity_path: identity.to_string_lossy().into_owned(),
    };
    bootstrap_update::prepare(&root, &plan, &assets.agent_target).unwrap();
    std::fs::write(
        root.join("native-assets.json"),
        serde_json::to_vec(&assets).unwrap(),
    )
    .unwrap();
    let parent = handoff::ObservedProcess::open(std::process::id()).unwrap();
    std::fs::write(
        root.join("parent.json"),
        serde_json::to_vec(parent.owner()).unwrap(),
    )
    .unwrap();
    let status = std::process::Command::new(root.join("runtime/helper.exe"))
        .args([
            "--ignored",
            "--exact",
            "native_admission_child",
            "--test-threads=1",
            "--nocapture",
        ])
        .status()
        .unwrap();
    assert!(
        status.success(),
        "child failed; preserve transaction/maintenance and service evidence"
    );
    let result: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("native-proof.json")).unwrap()).unwrap();
    assert_eq!(result["phase"], "rolled_back");
    assert_eq!(product_state(OLD), 5);
    assert_eq!(product_state(TARGET), -1);
    assert!(backup.matches_directory(normal).unwrap());
    assert_eq!(
        std::fs::read(identity).unwrap(),
        std::fs::read(root.join("backups/identity.toml")).unwrap()
    );
    assert_eq!(
        hex::encode(Sha256::digest(std::fs::read(agent).unwrap())),
        plan.agent_backup.sha256
    );
    assert!(native_install::client_ready(
        &normal.join("RustDesk.exe"),
        2026100502,
        true
    ));
}

#[test]
#[ignore = "child invoked only by fixed Win11 native admission fixture"]
fn native_admission_child() {
    require_vm();
    let root = root();
    assert_eq!(
        std::env::current_exe().unwrap(),
        root.join("runtime/helper.exe")
    );
    let _guard = install_guard::InstallGuard::try_acquire().unwrap().unwrap();
    let plan = InstallPlan::decode(&std::fs::read(root.join("prepared.json")).unwrap()).unwrap();
    let mut tx =
        Transaction::decode(&std::fs::read(root.join("transaction.json")).unwrap()).unwrap();
    let proof = evidence();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let owner = handoff::ObservedProcess::open(std::process::id()).unwrap();
    let parent: handoff::Owner =
        serde_json::from_slice(&std::fs::read(root.join("parent.json")).unwrap()).unwrap();
    assert!(handoff::ObservedProcess::owner_alive(&parent).unwrap());
    let plan_hash = hex::encode(Sha256::digest(
        std::fs::read(root.join("prepared.json")).unwrap(),
    ));
    let nonce = "e".repeat(64);
    for (name, value) in [
        (
            "handoff.json",
            serde_json::json!({"schema":1,"transaction_id":ID,"plan_sha256":plan_hash,"nonce":nonce,"created_at":now,"parent":parent,"helper":owner.owner()}),
        ),
        (
            "helper-ready.json",
            serde_json::json!({"schema":1,"transaction_id":ID,"plan_sha256":plan_hash,"nonce":nonce,"ready_at":now,"helper":owner.owner()}),
        ),
        (
            "helper-release.json",
            serde_json::json!({"schema":1,"transaction_id":ID,"plan_sha256":plan_hash,"nonce":nonce,"released_at":now,"parent":parent,"helper":owner.owner()}),
        ),
    ] {
        std::fs::write(root.join(name), serde_json::to_vec(&value).unwrap()).unwrap();
    }
    let marker = root
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("state/maintenance.json");
    let staged = root.join("maintenance.fixture.json");
    let bytes = serde_json::to_vec(&serde_json::json!({"schema":1,"installation_id":plan.installation_id,"transaction_id":ID,"plan_sha256":plan_hash,"nonce":nonce})).unwrap();
    std::fs::write(&staged, &bytes).unwrap();
    assert!(!marker.exists());
    std::fs::rename(staged, &marker).unwrap();
    workspace::verify_file(&marker).unwrap();
    let mut backend = native_install::NativeInstaller::open(&root).unwrap();
    let result = install_execution::execute(
        &root.join("transaction.json"),
        &mut tx,
        &proof,
        &mut backend,
        now,
    )
    .unwrap();
    std::fs::write(root.join("native-proof.json"), serde_json::to_vec(&serde_json::json!({"phase":result.phase,"error":result.error,"old":product_state(OLD),"target":product_state(TARGET)})).unwrap()).unwrap();
    assert_eq!(result.phase, Phase::RolledBack, "{:?}", result.error);
    assert!(result.error.unwrap().contains("1603"));
    let install: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("install-result.json")).unwrap()).unwrap();
    let rollback: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("rollback-result.json")).unwrap()).unwrap();
    assert_eq!(
        install["outcome"],
        serde_json::json!({"status":"failed","detail":1603})
    );
    assert_eq!(
        rollback["outcome"],
        serde_json::json!({"status":"installed"})
    );
    let permit: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("msi-rollback-permit.json")).unwrap())
            .unwrap();
    assert!(permit["remove"].is_null());
    assert_eq!(permit["product_code"], OLD);
    let log = std::fs::read(root.join("installer.log")).unwrap();
    let log = String::from_utf16_lossy(
        &log.chunks_exact(2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
            .collect::<Vec<_>>(),
    );
    assert!(
        log.contains("AdmissionLabFail"),
        "failure must be injected after native installation began"
    );
    assert_eq!(std::fs::read(&marker).unwrap(), bytes);
    std::fs::remove_file(marker).unwrap();
}
