use super::install_plan::InstallPlan;
use std::path::Path;

pub struct Request<'a> {
    pub transaction_id: &'a str,
    pub command_id: &'a str,
    pub installation_id: &'a str,
    pub target_key: &'a str,
    pub current_build: u64,
    pub package: &'a Path,
    pub manifest: &'a hbb_common::UpdateManifest,
    pub rollback_package: &'a Path,
    pub rollback_manifest: &'a hbb_common::UpdateManifest,
}

pub fn prepare(root: &Path, request: &Request<'_>) -> Result<InstallPlan, String> {
    prepare_checked(root, request, None)
}

#[cfg(test)]
pub fn prepare_fixture(root: &Path, request: &Request<'_>, public: &[u8]) -> Result<InstallPlan, String> {
    prepare_checked(root, request, Some(public))
}

fn prepare_checked(root: &Path, request: &Request<'_>, fixture_key: Option<&[u8]>) -> Result<InstallPlan, String> {
    use super::{directory_backup, install_guard, install_plan::{OriginalService, Trigger}, msi_assets,
        msi_metadata, native_install, package_gate, state_store, transaction::{Evidence, Phase, Transaction}, workspace};
    use windows_service::{service::{ServiceAccess, ServiceState, ServiceStartType, ServiceType}, service_manager::{ServiceManager, ServiceManagerAccess}};
    let token = |value: &str| value.len() == 32 && value.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
    if !token(request.transaction_id) || !token(request.command_id) || !token(request.installation_id)
        || !matches!(request.target_key, "windows-x86_64-msi-standard" | "windows-x86_64-msi-sos") || request.current_build == 0 {
        return Err("installation preparation request invalid".into());
    }
    let target = match fixture_key {
        Some(public) => package_gate::verify_with_key(request.manifest,request.target_key,request.current_build,"fixture-key",public)?,
        None => package_gate::verify(request.manifest,request.target_key,request.current_build)?,
    };
    let original = match fixture_key {
        Some(public) => package_gate::verify_with_key(request.rollback_manifest,request.target_key,request.current_build,"fixture-key",public)?,
        None => package_gate::verify(request.rollback_manifest,request.target_key,request.current_build)?,
    };
    let (old_version, old_build, _) = base::update::update_target_release(request.rollback_manifest,original).map_err(|e| e.to_string())?;
    let (new_version, new_build, _) = base::update::update_target_release(request.manifest,target).map_err(|e| e.to_string())?;
    if old_build != request.current_build { return Err("rollback package must match current build".into()); }
    let _target_lock = package_gate::lock_verified_file(request.package,target)?;
    let _original_lock = package_gate::lock_verified_file(request.rollback_package,original)?;
    let original_metadata = msi_metadata::inspect(request.rollback_package)?;
    let target_metadata = msi_metadata::inspect(request.package)?;
    msi_metadata::verify_pair(&original_metadata,&target_metadata,&original_metadata.product_code,&target_metadata.product_code)?;
    let installed_source = msi_metadata::registered_package(&original_metadata.product_code,request.rollback_package)?;
    if matches!(installed_source, msi_metadata::RegisteredPackage::NotInstalled { .. }) { return Err("installation preparation original product absent".into()); }
    let target_name = package_gate::cache_name(&target.primary,"msi")?;
    let agent_root = super::system_dirs::get().agent();
    let expected_root = agent_root.join("transactions").join(request.transaction_id);
    if root != expected_root && fixture_key.is_none() { return Err("preparation root outside fixed transaction directory".into()); }
    if root.exists() { return Err("installation preparation already exists".into()); }
    let state_directory = agent_root.join("state");
    check_local_state(&state_directory,request.installation_id,request.target_key)?;
    let Some(_guard) = install_guard::InstallGuard::try_acquire().map_err(|e| e.to_string())? else {
        return Err("installation preparation busy".into());
    };
    check_local_state(&state_directory,request.installation_id,request.target_key)?;
    let edition = if request.target_key.ends_with("-sos") { "sos" } else { "standard" };
    let normal_install = super::system_dirs::get().normal_install();
    let normal = normal_install.as_path();
    let agent = agent_root.join("rustdesk-management-agent.exe");
    let manager = ServiceManager::local_computer(None::<&str>,ServiceManagerAccess::CONNECT).map_err(|e| e.to_string())?;
    let mut services = Vec::new();
    for name in ["RustDesk","RustDeskManagementAgent"] {
        let service = manager.open_service(name,ServiceAccess::QUERY_CONFIG|ServiceAccess::QUERY_STATUS).map_err(|e| e.to_string())?;
        let config = service.query_config().map_err(|e| e.to_string())?;
        let status = service.query_status().map_err(|e| e.to_string())?;
        let start_type = match config.start_type {
            ServiceStartType::AutoStart => "auto", ServiceStartType::OnDemand => "manual",
            _ => return Err("installation preparation service disabled or unsupported".into()),
        };
        if config.service_type != ServiceType::OWN_PROCESS || !matches!(status.current_state,ServiceState::Running|ServiceState::Stopped)
            || !config.account_name.as_ref().is_some_and(|value| value.to_string_lossy().eq_ignore_ascii_case("LocalSystem")) {
            return Err("installation preparation service contract invalid".into());
        }
        let contract = super::service_control::Contract { name:name.into(),
            executable:if name == "RustDesk" {normal.join("RustDesk.exe")} else {agent.clone()},
            arguments:if name == "RustDesk" {vec!["--service".into()]} else {vec!["--service".into(),"--edition".into(),edition.into(),"--package-kind".into(),"msi".into(),"--normal-install-path".into(),normal.to_string_lossy().into_owned()]},
            start_type:config.start_type };
        super::service_control::inspect(&contract)?;
        if !contract.matches_command(&config.executable_path.to_string_lossy()) { return Err("installation preparation service command mismatch".into()); }
        services.push(OriginalService {name:name.into(),command:config.executable_path.to_string_lossy().into_owned(),account:"LocalSystem".into(),start_type:start_type.into(),running:status.current_state==ServiceState::Running});
    }
    let configured_identity = hbb_common::config::Config::file();
    let dirs = super::system_dirs::get();
    let identity = [dirs.local_service_roaming().join(r"RustDesk\config\RustDesk.toml"),dirs.system_profile_roaming().join(r"RustDesk\config\RustDesk.toml")]
        .into_iter().find(|path| configured_identity.to_string_lossy().eq_ignore_ascii_case(&path.to_string_lossy())).ok_or("preparation identity path outside fixed service profiles")?;
    workspace::verify_restore_target(&identity,true)?;
    workspace::verify_restore_target(&agent,false)?;
    workspace::create(root)?;
    for directory in ["backups","original","runtime"] { workspace::create(&root.join(directory))?; }
    let package = copy_asset(root,request.package,&target_name,2*1024*1024*1024)?;
    let rollback = copy_asset(root,request.rollback_package,&format!("original/{}",installed_source.package_name()),2*1024*1024*1024)?;
    let _copied_target = package_gate::lock_verified_file(&root.join(&package.relative_path),target)?;
    let _copied_original = package_gate::lock_verified_file(&root.join(&rollback.relative_path),original)?;
    let extracted = match fixture_key {
        #[cfg(test)]
        Some(public) => msi_assets::extract_fixture(root,&root.join(&package.relative_path),request.manifest,request.target_key,public)?,
        _ => msi_assets::extract(root,&root.join(&package.relative_path),request.manifest,request.target_key,request.current_build)?,
    };
    let sealed = msi_assets::seal_target(root,&extracted)?;
    let backup = directory_backup::capture(normal,&root.join("client-backup"))?;
    backup.verify(&root.join("client-backup"))?;
    let client_backup = describe(root,"client-backup/manifest.json")?;
    let agent_backup = copy_asset(root,&agent,"backups/agent.exe",512*1024*1024)?;
    let identity_backup = copy_asset(root,&identity,"backups/identity.toml",65536)?;
    let helper = copy_asset(root,&std::env::current_exe().map_err(|e| e.to_string())?,"runtime/helper.exe",512*1024*1024)?;
    let metadata:serde_json::Value = serde_json::from_str(include_str!("../../version.json")).map_err(|e|e.to_string())?;
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_err(|_|"preparation clock invalid")?.as_secs();
    let plan=InstallPlan {schema:1,transaction_id:request.transaction_id.into(),trigger:Trigger::RemoteCommand,command_id:Some(request.command_id.into()),installation_id:request.installation_id.into(),identity_fingerprint:identity_backup.sha256.clone(),target_key:request.target_key.into(),original_version:old_version.into(),original_build_seq:old_build,target_version:new_version.into(),target_build_seq:new_build,normal_install_path:normal.to_string_lossy().into_owned(),package,client_backup,agent_backup,identity_backup,helper,helper_version:metadata["version"].as_str().ok_or("helper version absent")?.into(),resume_task:format!("RustDeskManagementAgentResume-{}",request.transaction_id),created_at:now,services};
    plan.validate()?;
    plan.verify_assets(root)?;
    super::bootstrap_update::prepare(root, &plan, &sealed.agent)?;
    let assets=native_install::NativeAssets {schema:2,transaction_id:request.transaction_id.into(),client_target_manifest:sealed.client_manifest,agent_target:sealed.agent,original_msi:rollback,original_product_code:original_metadata.product_code,target_product_code:target_metadata.product_code,identity_path:identity.to_string_lossy().into_owned()};
    assets.validate()?;
    state_store::publish(&root.join("native-assets.json"),&serde_json::to_vec(&assets).map_err(|e|e.to_string())?).map_err(|e|e.to_string())?;
    state_store::publish(&root.join("approved-manifest.json"),&serde_json::to_vec(request.manifest).map_err(|e|e.to_string())?).map_err(|e|e.to_string())?;
    state_store::publish(&root.join("rollback-manifest.json"),&serde_json::to_vec(request.rollback_manifest).map_err(|e|e.to_string())?).map_err(|e|e.to_string())?;
    let mut tx=Transaction::new(plan.transaction_id.clone(),plan.target_key.clone(),plan.package.sha256.clone(),plan.identity_fingerprint.clone(),now)?;
    tx.save(&root.join("transaction.json"))?;
    let evidence=Evidence {package_verified:true,backups_verified:false,helper_verified:false,resume_task_verified:false,handoff_verified:false,maintenance_owned:false,services_stopped:false,target_verified:false,identity_unchanged:false,client_ready:false,agent_ready:false,rollback_verified:false,locally_blocked:false};
    tx.advance(Phase::PackageVerified,&evidence,now)?;
    tx.save(&root.join("transaction.json"))?;
    plan.save_bound(&root.join("prepared.json"),&tx)?;
    check_local_state(&state_directory,request.installation_id,request.target_key)?;
    native_install::NativeInstaller::open(root)?;
    Ok(plan)
}

fn check_local_state(directory: &Path, installation: &str, target_key: &str) -> Result<(), String> {
    for marker in ["disable-agent","disable-recovery","maintenance.json"] {
        match std::fs::symlink_metadata(directory.join(marker)) {
            Ok(_) => return Err("installation preparation locally blocked".into()),
            Err(error) if error.kind()==std::io::ErrorKind::NotFound => {},
            Err(_) => return Err("installation preparation local gate unreadable".into()),
        }
    }
    let state:serde_json::Value=serde_json::from_slice(&std::fs::read(directory.join("agent.json")).map_err(|e|e.to_string())?).map_err(|e|e.to_string())?;
    if state["agent_installation_id"]!=installation || state["target_key"]!=target_key || state["management_agent_enabled"]!=true || state["disabled"]!=false {
        return Err("installation preparation identity or policy mismatch".into());
    }
    #[derive(serde::Deserialize)]
    struct Intent { #[serde(default)] options:std::collections::HashMap<String,String> }
    let text=std::fs::read_to_string(hbb_common::config::Config2::file()).map_err(|_|"installation intent unavailable")?;
    let intent:Intent=hbb_common::toml::from_str(&text).map_err(|_|"installation intent malformed")?;
    if !matches!(intent.options.get("stop-service").map(String::as_str),None|Some(""|"N")) {
        return Err("installation preparation user stopped".into());
    }
    Ok(())
}

fn describe(root: &Path, relative: &str) -> Result<super::install_plan::Artifact,String> {
    use sha2::{Digest,Sha256};
    use std::io::Read;
    let path=root.join(relative);
    super::workspace::verify_file(&path)?;
    let mut file=std::fs::File::open(&path).map_err(|e|e.to_string())?;
    let mut hash=Sha256::new();let mut count=0u64;let mut buffer=[0u8;65536];
    loop {let read=file.read(&mut buffer).map_err(|e|e.to_string())?;if read==0{break;}count+=read as u64;hash.update(&buffer[..read]);}
    Ok(super::install_plan::Artifact {relative_path:relative.into(),size:count,sha256:hex::encode(hash.finalize())})
}

fn copy_asset(root: &Path, source: &Path, relative: &str, limit:u64) -> Result<super::install_plan::Artifact,String> {
    use std::{io::{Read,Write},os::windows::fs::{MetadataExt,OpenOptionsExt}};
    for ancestor in source.ancestors() {
        if std::fs::symlink_metadata(ancestor).map_err(|e|e.to_string())?.file_attributes() & 0x400 != 0 {return Err("preparation reparse asset refused".into());}
    }
    let mut input=std::fs::OpenOptions::new().read(true).share_mode(1).custom_flags(0x00200000).open(source).map_err(|e|e.to_string())?;
    let metadata=input.metadata().map_err(|e|e.to_string())?;
    if !metadata.is_file() || metadata.len()==0 || metadata.len()>limit {return Err("preparation asset size invalid".into());}
    let mut output=std::fs::OpenOptions::new().write(true).create_new(true).open(root.join(relative)).map_err(|e|e.to_string())?;
    let count=std::io::copy(&mut (&mut input).take(limit+1),&mut output).map_err(|e|e.to_string())?;
    output.flush().and_then(|_|output.sync_all()).map_err(|e|e.to_string())?;drop(output);
    if count!=metadata.len(){return Err("preparation source changed".into());}
    describe(root,relative)
}
