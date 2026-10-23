#![cfg(windows)]
#[path = "../src/management_agent/bootstrap_update.rs"] mod bootstrap_update;
#[path = "../src/management_agent/msi_assets.rs"] mod msi_assets;
#[path = "../src/management_agent/directory_backup.rs"]
mod directory_backup;
#[path = "../src/management_agent/file_restore.rs"]
mod file_restore;
#[path = "../src/management_agent/handoff.rs"]
mod handoff;
#[path = "../src/management_agent/install_execution.rs"]
mod install_execution;
#[path = "../src/management_agent/install_plan.rs"]
mod install_plan;
#[path = "../src/management_agent/installer_process.rs"]
mod installer_process;
#[path = "../src/management_agent/native_install.rs"]
mod native_install;
#[path = "../src/management_agent/msi_metadata.rs"]
mod msi_metadata;
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
#[path = "../src/management_agent/health.rs"]
mod health;
#[path = "../src/management_agent/health_windows.rs"]
mod health_windows;
#[path = "../src/platform/windows_agent_guard.rs"]
mod install_guard;

#[test]
#[ignore = "Requires explicit SYSTEM lab VM with verified v25 MSI and snapshot"]
fn actual_msi_validation_failure_restores_verified_original_and_identity() {
    actual_msi_rollback(false, false);
}

#[test]
#[ignore = "Requires SYSTEM lab VM restored to v25 and verified v65 upgrade package"]
fn actual_new_msi_validation_failure_restores_old_product_payload_and_identity() {
    actual_msi_rollback(true, false);
}

#[test]
#[ignore = "Requires SYSTEM lab VM with v25 bootstrap and verified v65 MSI"]
fn actual_bootstrap_upgrade_then_failure_restores_old_recovery_entry() {
    actual_msi_rollback(true, true);
}

fn actual_msi_rollback(upgrade: bool, bootstrap: bool) {
    use install_plan::{Artifact, InstallPlan, OriginalService, Trigger};
    use sha2::{Digest, Sha256};
    use std::path::{Path, PathBuf};
    use transaction::{Evidence, Phase, Transaction};
    use windows_service::{service::ServiceAccess, service_manager::{ServiceManager, ServiceManagerAccess}};
    base::lab_guard::require_lab();
    assert_eq!(std::env::var("RUSTDESK_NATIVE_MSI_LAB").unwrap(), "rollback-v28");
    let package = Path::new(r"C:\RustDeskAgentLab\agent-health-v25.msi");
    assert_eq!(hex::encode(Sha256::digest(std::fs::read(package).unwrap())), "d158bbb84ff98b7b1ac84cd373181555f9b97a9a2782c6690aaaa8859f4956d3");
    let target_package = if upgrade { Path::new(r"C:\RustDeskAgentLab\agent-versioned-v65.msi") } else { package };
    if upgrade {
        assert_eq!(hex::encode(Sha256::digest(std::fs::read(target_package).unwrap())), "b332083068c9346b6b381e459a63a22f3c2deef73088325c5225b14826467050");
    }
    let product = msi_metadata::inspect(package).unwrap().product_code;
    let target_product = msi_metadata::inspect(target_package).unwrap().product_code;
    if upgrade { assert_ne!(product, target_product, "fixture must really change the installed MSI product"); }
    let root = if bootstrap {
        PathBuf::from(r"C:\RustDeskAgentLab").join(format!("native-bootstrap-rollback-{}", std::process::id()))
    } else {
        PathBuf::from(if upgrade { r"C:\RustDeskAgentLab\native-upgrade-rollback-v76" } else { r"C:\RustDeskAgentLab\native-rollback-v28" })
    };
    println!("Native rollback evidence: {}", root.display());
    workspace::create(&root).unwrap();
    let _guard = install_guard::InstallGuard::try_acquire().unwrap().unwrap();
    let normal = Path::new(r"C:\Program Files\RustDesk");
    let agent = Path::new(r"C:\ProgramData\RustDesk\ManagementAgent\rustdesk-management-agent.exe");
    if upgrade {
        assert_eq!(hex::encode(Sha256::digest(std::fs::read(agent).unwrap())), "9c08ecb3b3bfc3eb6ebe90734251bfe5ac437196bddeaafa5be467a18465611b", "restore verified v25 snapshot first");
    }
    let state_path = Path::new(r"C:\ProgramData\RustDesk\ManagementAgent\state\agent.json");
    let state: serde_json::Value = serde_json::from_slice(&std::fs::read(state_path).unwrap()).unwrap();
    let installation = state["agent_installation_id"].as_str().unwrap().to_owned();
    let bootstrap_root = Path::new(r"C:\ProgramData\RustDesk\RecoveryBootstrap");
    let bootstrap_before = bootstrap.then(|| {
        let bytes = std::fs::read(bootstrap_root.join("bootstrap.json")).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(value["agent_sha256"], hex::encode(Sha256::digest(std::fs::read(agent).unwrap())));
        assert_eq!(std::fs::read(bootstrap_root.join("rustdesk-recovery-bootstrap.exe")).unwrap(), std::fs::read(agent).unwrap());
        bytes
    });
    let identity = [r"C:\Windows\System32\config\systemprofile\AppData\Roaming\RustDesk\config\RustDesk.toml", r"C:\Windows\ServiceProfiles\LocalService\AppData\Roaming\RustDesk\config\RustDesk.toml"]
        .into_iter().map(PathBuf::from).find(|path| path.is_file()).unwrap();
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT).unwrap();
    let mut services = Vec::new();
    for name in ["RustDesk", "RustDeskManagementAgent"] {
        let service = manager.open_service(name, ServiceAccess::QUERY_CONFIG | ServiceAccess::QUERY_STATUS).unwrap();
        let config = service.query_config().unwrap();
        assert_eq!(config.start_type, windows_service::service::ServiceStartType::AutoStart);
        assert_eq!(service.query_status().unwrap().current_state, windows_service::service::ServiceState::Running);
        services.push(OriginalService {name:name.into(), command:config.executable_path.to_string_lossy().into_owned(), account:config.account_name.unwrap().to_string_lossy().into_owned(), start_type:"auto".into(), running:true});
    }
    let backup = directory_backup::capture(normal, &root.join("client-backup")).unwrap();
    std::fs::create_dir(root.join("backups")).unwrap();
    std::fs::create_dir(root.join("runtime")).unwrap();
    std::fs::create_dir(root.join("original")).unwrap();
    let package_name = if upgrade { "agent-versioned-v65.msi" } else { "agent-health-v25.msi" };
    for (source, destination) in [(target_package, package_name), (package, "original/agent-health-v25.msi"), (agent, "backups/agent.exe"), (agent, "target-agent.exe"), (identity.as_path(), "backups/identity.toml")] {
        std::fs::copy(source, root.join(destination)).unwrap();
    }
    std::fs::copy(std::env::current_exe().unwrap(), root.join("runtime/helper.exe")).unwrap();
    let artifact = |relative: &str| {
        let bytes = std::fs::read(root.join(relative)).unwrap();
        Artifact {relative_path:relative.into(),size:bytes.len() as u64,sha256:hex::encode(Sha256::digest(bytes))}
    };
    let mut target = backup.clone();
    let mut target_agent = artifact("target-agent.exe");
    target.files.iter_mut().find(|file| file.path.eq_ignore_ascii_case("RustDesk.exe")).unwrap().sha256 = "0".repeat(64);
    if bootstrap {
        use hbb_common::{base64::{engine::general_purpose::STANDARD, Engine as _}, sodiumoxide::crypto::sign, UpdateManifest, UpdateTarget};
        hbb_common::sodiumoxide::init().unwrap();
        let (public, secret) = sign::gen_keypair();
        let key = "windows-x86_64-msi-standard";
        let bytes = std::fs::read(target_package).unwrap();
        let mut manifest = UpdateManifest { schema:2, product:"rustdesk-yan".into(), edition:"multi".into(), channel:"stable".into(), catalog_revision:2026100502, ..Default::default() };
        let mut package_target = UpdateTarget { version:"1.5.3".into(), build_seq:2026100502, source_commit:"fixture-commit".into(),
            primary:"https://download.yan.life/rustdesk/stable/1.5.3/agent-versioned-v65.msi".into(),
            size:bytes.len() as u64, sha256:hex::encode(Sha256::digest(&bytes)), signature_key_id:"fixture-key".into(), ..Default::default() };
        package_target.signature = STANDARD.encode(sign::sign_detached(&base::update::update_signature_payload(&manifest,key,&package_target).unwrap(), &secret));
        manifest.targets.insert(key.into(), package_target);
        let extracted = msi_assets::extract_fixture(&root, target_package, &manifest, key, public.as_ref()).unwrap();
        let sealed = msi_assets::seal_target(&root, &extracted).unwrap();
        target = directory_backup::DirectoryBackup::decode(&std::fs::read(root.join(sealed.client_manifest.relative_path)).unwrap()).unwrap();
        target_agent = sealed.agent;
    }
    std::fs::write(root.join("target-client.json"), serde_json::to_vec(&target).unwrap()).unwrap();
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
    let id = "28".repeat(16);
    let plan = InstallPlan {schema:1,transaction_id:id.clone(),trigger:Trigger::LocalRecovery,command_id:None,installation_id:installation.clone(),identity_fingerprint:artifact("backups/identity.toml").sha256,
        target_key:"windows-x86_64-msi-standard".into(),original_version:"1.5.3".into(),original_build_seq:2026100502,target_version:"1.5.3".into(),target_build_seq:2026100502,normal_install_path:normal.to_string_lossy().into_owned(),package:artifact(package_name),client_backup:artifact("client-backup/manifest.json"),agent_backup:artifact("backups/agent.exe"),identity_backup:artifact("backups/identity.toml"),helper:artifact("runtime/helper.exe"),helper_version:"1.5.3".into(),resume_task:format!("RustDeskManagementAgentResume-{id}"),created_at:now,services};
    let mut tx = Transaction::new(id.clone(),plan.target_key.clone(),plan.package.sha256.clone(),plan.identity_fingerprint.clone(),now).unwrap();
    tx.save(&root.join("transaction.json")).unwrap();
    let evidence = Evidence {package_verified:true,backups_verified:true,helper_verified:true,resume_task_verified:true,handoff_verified:true,maintenance_owned:true,services_stopped:false,target_verified:false,identity_unchanged:false,client_ready:false,agent_ready:false,rollback_verified:false,locally_blocked:false};
    for phase in [Phase::PackageVerified,Phase::Prepared,Phase::StoppingServices] {tx.advance(phase,&evidence,now).unwrap();tx.save(&root.join("transaction.json")).unwrap();}
    plan.save_bound(&root.join("prepared.json"),&tx).unwrap();
    let assets = native_install::NativeAssets {schema:if bootstrap {2} else {1},transaction_id:id.clone(),client_target_manifest:artifact("target-client.json"),agent_target:target_agent,original_msi:artifact("original/agent-health-v25.msi"),original_product_code:product.clone(),target_product_code:target_product.clone(),identity_path:identity.to_string_lossy().into_owned()};
    if bootstrap { bootstrap_update::prepare(&root, &plan, &assets.agent_target).unwrap(); }
    std::fs::write(root.join("native-assets.json"),serde_json::to_vec(&assets).unwrap()).unwrap();
    let maintenance=state_path.parent().unwrap().join("maintenance.json");
    use std::io::Write;
    let mut marker=std::fs::OpenOptions::new().write(true).create_new(true).open(&maintenance).unwrap();
    let marker_bytes=serde_json::to_vec(&serde_json::json!({"fixture":"native-rollback-v28","transaction_id":id})).unwrap();
    marker.write_all(&marker_bytes).unwrap();marker.sync_all().unwrap();drop(marker);
    let mut backend = native_install::NativeInstaller::open(&root).unwrap();
    let result = if bootstrap {
        struct FailAfterBootstrap {
            inner: native_install::NativeInstaller,
            target_hash: String,
            upgraded: bool,
        }
        impl install_execution::Backend for FailAfterBootstrap {
            fn stop_services(&mut self) -> Result<(), String> { self.inner.stop_services() }
            fn apply_package(&mut self) -> Result<install_execution::InstallOutcome, String> { self.inner.apply_package() }
            fn validate_target(&mut self) -> Result<install_execution::Validation, String> {
                use sha2::{Digest, Sha256};
                let mut validation = self.inner.validate_target()?;
                assert!(validation.target_matches && validation.identity_unchanged && validation.client_ready && validation.agent_ready,
                    "must validate new package and bootstrap before injecting failure");
                let root = std::path::Path::new(r"C:\ProgramData\RustDesk\RecoveryBootstrap");
                let value: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("bootstrap.json")).unwrap()).unwrap();
                assert_eq!(value["agent_sha256"], self.target_hash);
                assert_eq!(hex::encode(Sha256::digest(std::fs::read(root.join("rustdesk-recovery-bootstrap.exe")).unwrap())), self.target_hash);
                self.upgraded = true;
                validation.agent_ready = false;
                Ok(validation)
            }
            fn rollback_original(&mut self) -> Result<install_execution::Validation, String> { self.inner.rollback_original() }
        }
        let mut wrapped = FailAfterBootstrap {inner:backend, target_hash:assets.agent_target.sha256.clone(), upgraded:false};
        let result = install_execution::execute(&root.join("transaction.json"), &mut tx, &evidence, &mut wrapped, now+1).unwrap();
        assert!(wrapped.upgraded, "target bootstrap never reached validation");
        result
    } else {
        install_execution::execute(&root.join("transaction.json"),&mut tx,&evidence,&mut backend,now+1).unwrap()
    };
    std::fs::write(root.join("test-result.json"),serde_json::to_vec(&serde_json::json!({"phase":result.phase,"error":result.error,"upgrade":upgrade,"original_product":product,"target_product":target_product,"scope":"real MSI validation-failure rollback; fixture release build metadata unchanged; not remote command orchestration"})).unwrap()).unwrap();
    assert_eq!(result.phase,Phase::RolledBack,"{:?}",result.error);
    assert!(backup.matches_directory(normal).unwrap());
    assert_eq!(hex::encode(Sha256::digest(std::fs::read(agent).unwrap())),plan.agent_backup.sha256);
    let after:serde_json::Value=serde_json::from_slice(&std::fs::read(state_path).unwrap()).unwrap();
    assert_eq!(after["agent_installation_id"],installation);
    assert!(native_install::client_ready(&normal.join("RustDesk.exe"),2026100502,true));
    if let Some(original_manifest) = bootstrap_before {
        assert_eq!(std::fs::read(bootstrap_root.join("bootstrap.json")).unwrap(), original_manifest);
        assert_eq!(std::fs::read(bootstrap_root.join("rustdesk-recovery-bootstrap.exe")).unwrap(), std::fs::read(agent).unwrap());
    }
    if upgrade {
        #[link(name = "msi")]
        extern "system" { fn MsiQueryProductStateW(code: *const u16) -> i32; }
        let old = product.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
        let new = target_product.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
        assert_eq!(unsafe { MsiQueryProductStateW(old.as_ptr()) }, 5, "original product registration restored");
        assert_ne!(unsafe { MsiQueryProductStateW(new.as_ptr()) }, 5, "failed target product cannot remain installed");
        let receipt: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("install-result.json")).unwrap()).unwrap();
        assert_eq!(receipt["outcome"]["status"], "installed", "upgrade must have actually applied before rollback");
    }
    assert_eq!(std::fs::read(&maintenance).unwrap(),marker_bytes);
    std::fs::remove_file(maintenance).unwrap();
}

#[test]
fn running_service_is_not_ready_when_executable_or_health_identity_is_unavailable() {
    let root = std::env::temp_dir().join(format!("agent-native-readiness-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("missing.exe");
    assert!(!native_install::client_ready(&path, 2026100502, true), "SCM Running alone must not commit installation");
    assert!(!path.exists());
    std::fs::write(&path, b"not-the-health-server").unwrap();
    assert!(!native_install::client_ready(&path, 2026100502, true));
    assert!(!native_install::client_ready(&path, 2026100502, false));
    assert_eq!(std::fs::read(&path).unwrap(), b"not-the-health-server");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn native_installer_rejects_incomplete_or_unprotected_assets_without_touching_services() {
    let root = std::env::temp_dir().join(format!("rustdesk-native-install-{}", std::process::id()));
    std::fs::create_dir(&root).unwrap();
    assert!(native_install::NativeInstaller::open(&root).is_err());
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), 0);
    std::fs::remove_dir(&root).unwrap();
    workspace::create(&root).unwrap();
    assert!(native_install::NativeInstaller::open(&root).is_err());
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), 0);
    std::fs::remove_dir(&root).unwrap();
}

#[test]
fn target_file_manifest_and_original_product_codes_are_strictly_bound() {
    use native_install::NativeAssets;
    let mut assets = NativeAssets {
        schema: 1,
        transaction_id: "a".repeat(32),
        client_target_manifest: install_plan::Artifact {
            relative_path: "target-client.json".into(),
            size: 123,
            sha256: "b".repeat(64),
        },
        agent_target: install_plan::Artifact {
            relative_path: "target-agent.exe".into(),
            size: 456,
            sha256: "c".repeat(64),
        },
        original_msi: install_plan::Artifact {
            relative_path: "original.msi".into(),
            size: 789,
            sha256: "d".repeat(64),
        },
        original_product_code: "{12345678-1234-1234-1234-123456789ABC}".into(),
        target_product_code: "{87654321-4321-4321-4321-CBA987654321}".into(),
        identity_path:
            r"C:\Windows\ServiceProfiles\LocalService\AppData\Roaming\RustDesk\config\RustDesk.toml"
                .into(),
    };
    assets.validate().unwrap();
    let legacy = serde_json::to_vec(&assets).unwrap();
    let decoded: NativeAssets = serde_json::from_slice(&legacy).unwrap();
    decoded.validate().unwrap();
    assert_eq!(decoded.schema, 1);
    assets.schema = 2;
    assets.validate().unwrap();
    let current = serde_json::to_vec(&assets).unwrap();
    let decoded: NativeAssets = serde_json::from_slice(&current).unwrap();
    decoded.validate().unwrap();
    assert_eq!(decoded.schema, 2);
    for schema in [0, 3, u32::MAX] {
        assets.schema = schema;
        assert_eq!(assets.validate().unwrap_err(), "native installation assets/product identity invalid");
    }
    assets.schema = 2;
    for invalid in [
        "",
        "RustDesk",
        "{bad}",
        "{12345678-1234-1234-1234-123456789ABC} /qn",
        "../escape",
    ] {
        assets.target_product_code = invalid.into();
        assert!(assets.validate().is_err());
    }
    assets.target_product_code = "{87654321-4321-4321-4321-CBA987654321}".into();
    assets.identity_path = r"C:\Users\user\secret".into();
    assert!(assets.validate().is_err());
}
