use super::{
    directory_backup::{self, DirectoryBackup},
    exe_assets,
    install_guard::InstallGuard,
    install_plan::{Artifact, InstallPlan, OriginalService, Trigger},
    package_gate, state_store,
    transaction::{Evidence, Phase, Transaction},
    workspace,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    io::Read,
    path::{Path, PathBuf},
};

pub struct Request<'a> {
    pub transaction_id: &'a str,
    pub installation_id: &'a str,
    pub edition: &'a str,
    pub current_build: u64,
    pub package: &'a Path,
    pub manifest: &'a hbb_common::UpdateManifest,
}

#[derive(Clone)]
pub(crate) struct Layout {
    pub normal: PathBuf,
    pub agent: PathBuf,
    pub state: PathBuf,
    pub config: PathBuf,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Assets {
    pub schema: u32,
    pub transaction_id: String,
    pub client: Artifact,
    pub state: Artifact,
    pub config: Artifact,
    pub target: Artifact,
    pub approved_manifest: Artifact,
    pub bootstrap_manifest: Option<Artifact>,
}

pub(crate) fn verify_bound(
    root: &Path,
    plan: &InstallPlan,
    public: Option<&[u8]>,
) -> Result<package_gate::LockedPackage, String> {
    if plan.client_backup.relative_path != "exe-assets.json" {
        return Err("EXE binding envelope missing".into());
    }
    plan.verify_assets(root)?;
    let assets: Assets = serde_json::from_slice(&read(&root.join("exe-assets.json"), 65536)?)
        .map_err(|_| "EXE binding malformed")?;
    if assets.schema != 3
        || assets.transaction_id != plan.transaction_id
        || assets.approved_manifest.relative_path != "approved-manifest.json"
    {
        return Err("EXE binding identity mismatch".into());
    }
    for reference in [
        &assets.client,
        &assets.state,
        &assets.config,
        &assets.target,
        &assets.approved_manifest,
    ] {
        reference.verify(root)?;
    }
    verify_bootstrap(root, plan, &assets)?;
    let manifest: hbb_common::UpdateManifest =
        serde_json::from_slice(&read(&root.join("approved-manifest.json"), 262144)?)
            .map_err(|_| "EXE manifest malformed")?;
    let target = match public {
        #[cfg(test)]
        Some(key) => package_gate::verify_with_key(
            &manifest,
            &plan.target_key,
            plan.original_build_seq,
            "fixture-key",
            key,
        )?,
        _ => {
            let _ = public;
            package_gate::verify(&manifest, &plan.target_key, plan.original_build_seq)?
        }
    };
    let (version, build, _) =
        base::update::update_target_release(&manifest, target).map_err(|e| e.to_string())?;
    if version != plan.target_version
        || build != plan.target_build_seq
        || target.size != plan.package.size
        || target.sha256 != plan.package.sha256
    {
        return Err("EXE signed target/plan mismatch".into());
    }
    package_gate::lock_verified_file(&root.join(&plan.package.relative_path), target)
}

pub(crate) fn bootstrap_directory(root: &Path) -> PathBuf {
    #[cfg(test)]
    if root
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|p| p.to_str())
        .is_some_and(|p| p.starts_with("exe-tx-v218-"))
    {
        return root.parent().unwrap().join("RecoveryBootstrap");
    }
    crate::management_agent::system_dirs::get().recovery_bootstrap()
}

pub(crate) fn bootstrap_target(root: &Path) -> Result<Artifact, String> {
    describe(root, "target/files/rustdesk-management-agent.exe")
}

pub(crate) fn verify_bootstrap(
    root: &Path,
    plan: &InstallPlan,
    assets: &Assets,
) -> Result<(), String> {
    let target = bootstrap_target(root)?;
    super::bootstrap_update::verify_prepared(root, plan, &target)?;
    let prepared: serde_json::Value =
        serde_json::from_slice(&read(&root.join("bootstrap-update.json"), 65536)?)
            .map_err(|_| "EXE bootstrap preparation malformed")?;
    match &assets.bootstrap_manifest {
        Some(artifact) => {
            if artifact.relative_path != "bootstrap-backup/original.json" {
                return Err("EXE bootstrap original path invalid".into());
            }
            artifact.verify(root)?;
            if prepared["manifests"][0]
                != serde_json::to_value(artifact).map_err(|e| e.to_string())?
            {
                return Err("EXE bootstrap manifest not plan-bound".into());
            }
            let mut expected: serde_json::Value =
                serde_json::from_slice(&read(&root.join(&artifact.relative_path), 65536)?)
                    .map_err(|_| "EXE bootstrap original malformed")?;
            expected["agent_sha256"] = target.sha256.into();
            let bytes = serde_json::to_vec(&expected).map_err(|e| e.to_string())?;
            let new: Artifact = serde_json::from_value(prepared["manifests"][1].clone())
                .map_err(|_| "EXE bootstrap target missing")?;
            if new.size != bytes.len() as u64 || new.sha256 != hex::encode(Sha256::digest(&bytes)) {
                return Err("EXE bootstrap target fields changed".into());
            }
        }
        None => {
            if !prepared["manifests"].is_null() {
                return Err("EXE unexpected bootstrap preparation".into());
            }
        }
    }
    Ok(())
}

pub(crate) fn synchronize_bootstrap(
    root: &Path,
    plan: &InstallPlan,
    assets: &Assets,
    installing: bool,
) -> Result<(), String> {
    verify_bootstrap(root, plan, assets)?;
    let target = bootstrap_target(root)?;
    #[cfg(test)]
    {
        let directory = bootstrap_directory(root);
        return super::bootstrap_update::synchronize_fixture(
            root, &directory, plan, &target, installing,
        );
    }
    #[cfg(not(test))]
    super::bootstrap_update::synchronize(root, plan, &target, installing)
}

fn token(value: &str) -> bool {
    value.len() == 32
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

pub(crate) fn read(path: &Path, limit: u64) -> Result<Vec<u8>, String> {
    workspace::verify_file(path)?;
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .map_err(|e| e.to_string())?
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > limit {
        return Err("EXE record too large".into());
    }
    Ok(bytes)
}

pub(crate) fn gates(layout: &Layout, installation: &str, edition: &str) -> Result<(), String> {
    local_intent(layout)?;
    identity_gate(layout, installation, edition)
}

pub(crate) fn identity_gate(
    layout: &Layout,
    installation: &str,
    edition: &str,
) -> Result<(), String> {
    let state_path = layout.state.join("agent.json");
    workspace::verify_restore_target(&state_path, false)?;
    let bytes = std::fs::read(&state_path).map_err(|e| e.to_string())?;
    if bytes.len() > 65536 {
        return Err("EXE state too large".into());
    }
    let value: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|_| "EXE state malformed")?;
    if value["agent_installation_id"] != installation
        || value["edition"] != edition
        || value["package_kind"] != "exe"
        || value["schema_version"] != 1
        || value["disabled"] != false
        || value["management_agent_enabled"] != true
    {
        return Err("EXE installed identity/kind/schema mismatch".into());
    }
    Ok(())
}

pub(crate) fn local_intent(layout: &Layout) -> Result<(), String> {
    local_intent_checked(layout, false)
}

pub(crate) fn local_intent_checked(layout: &Layout, owned: bool) -> Result<(), String> {
    for name in if owned {
        &["disable-agent", "disable-recovery"][..]
    } else {
        &["disable-agent", "disable-recovery", "maintenance.json"][..]
    } {
        match std::fs::symlink_metadata(layout.state.join(name)) {
            Ok(_) => return Err("EXE preparation locally blocked".into()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err("EXE local gate unknown".into()),
        }
    }
    workspace::verify_restore_target(&layout.state.join("service-recovery-enabled"), false)?;
    workspace::verify_restore_target(&layout.config.join("RustDesk2.toml"), true)?;
    let text = std::fs::read_to_string(layout.config.join("RustDesk2.toml"))
        .map_err(|_| "EXE intent unavailable")?;
    if text.len() > 65536 {
        return Err("EXE intent too large".into());
    }
    let config: hbb_common::toml::Value =
        hbb_common::toml::from_str(&text).map_err(|_| "EXE intent malformed")?;
    if config
        .get("options")
        .and_then(|v| v.get("stop-service"))
        .is_some()
        && !matches!(
            config
                .get("options")
                .and_then(|v| v.get("stop-service"))
                .and_then(|v| v.as_str()),
            Some("" | "N")
        )
    {
        return Err("EXE user stop intent".into());
    }
    Ok(())
}

pub(crate) fn fixed_layout() -> Result<Layout, String> {
    let configured = hbb_common::config::Config::file();
    let dirs = crate::management_agent::system_dirs::get();
    let identity = [dirs.local_service_roaming(), dirs.system_profile_roaming()]
        .map(|p| p.join("RustDesk").join("config").join("RustDesk.toml"))
        .into_iter()
        .find(|p| {
            configured
                .to_string_lossy()
                .eq_ignore_ascii_case(&p.to_string_lossy())
        })
        .ok_or("EXE fixed identity path unavailable")?;
    Ok(Layout {
        normal: dirs.normal_install(),
        agent: dirs.agent().join("rustdesk-management-agent.exe"),
        state: dirs.agent().join("state"),
        config: identity.parent().ok_or("EXE config missing")?.into(),
    })
}

pub(crate) fn describe(root: &Path, name: &str) -> Result<Artifact, String> {
    workspace::verify_file(&root.join(name))?;
    let mut file = std::fs::File::open(root.join(name)).map_err(|e| e.to_string())?;
    let mut hash = Sha256::new();
    let mut size = 0u64;
    let mut buffer = [0u8; 65536];
    loop {
        let count = file.read(&mut buffer).map_err(|e| e.to_string())?;
        if count == 0 {
            break;
        }
        size += count as u64;
        if size > 2 * 1024 * 1024 * 1024 {
            return Err("EXE asset too large".into());
        }
        hash.update(&buffer[..count]);
    }
    Ok(Artifact {
        relative_path: name.into(),
        size,
        sha256: hex::encode(hash.finalize()),
    })
}

fn copy(root: &Path, from: &Path, name: &str, identity: bool) -> Result<Artifact, String> {
    workspace::verify_restore_target(from, identity)?;
    let mut input = std::fs::File::open(from).map_err(|e| e.to_string())?;
    let size = input.metadata().map_err(|e| e.to_string())?.len();
    if size == 0 || size > 2 * 1024 * 1024 * 1024 {
        return Err("EXE asset size invalid".into());
    }
    let mut output = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(root.join(name))
        .map_err(|e| e.to_string())?;
    if std::io::copy(&mut (&mut input).take(size + 1), &mut output).map_err(|e| e.to_string())?
        != size
    {
        return Err("EXE source changed".into());
    }
    output.sync_all().map_err(|e| e.to_string())?;
    drop(output);
    describe(root, name)
}

fn snapshots(edition: &str) -> Vec<OriginalService> {
    let dirs = crate::management_agent::system_dirs::get();
    let normal = dirs.normal_install();
    let agent = dirs.agent().join("rustdesk-management-agent.exe");
    vec![
        OriginalService {
            name: "RustDesk".into(),
            command: format!(r#""{}" --service"#, normal.join("RustDesk.exe").display()),
            account: "LocalSystem".into(),
            start_type: "auto".into(),
            running: true,
        },
        OriginalService {
            name: "RustDeskManagementAgent".into(),
            command: format!(
                r#""{}" --service --edition {edition} --package-kind exe --normal-install-path "{}""#,
                agent.display(),
                normal.display()
            ),
            account: "LocalSystem".into(),
            start_type: "auto".into(),
            running: true,
        },
    ]
}

pub fn prepare(request: &Request<'_>) -> Result<InstallPlan, String> {
    let configured = hbb_common::config::Config::file();
    let dirs = crate::management_agent::system_dirs::get();
    let identity = [dirs.local_service_roaming(), dirs.system_profile_roaming()]
        .map(|p| p.join("RustDesk").join("config").join("RustDesk.toml"))
        .into_iter()
        .find(|p| {
            configured
                .to_string_lossy()
                .eq_ignore_ascii_case(&p.to_string_lossy())
        })
        .ok_or("EXE identity outside fixed profiles")?;
    let agent_root = dirs.agent();
    let layout = Layout {
        normal: dirs.normal_install(),
        agent: agent_root.join("rustdesk-management-agent.exe"),
        state: agent_root.join("state"),
        config: identity.parent().ok_or("EXE profile missing")?.into(),
    };
    prepare_checked(
        &agent_root.join("transactions").join(request.transaction_id),
        &layout,
        request,
        None,
        false,
    )
}

#[cfg(test)]
pub fn prepare_fixture(
    root: &Path,
    request: &Request<'_>,
    public: &[u8],
) -> Result<InstallPlan, String> {
    prepare_checked(
        &root.join("transaction"),
        &fixture_layout(root)?,
        request,
        Some(public),
        true,
    )
}

#[cfg(test)]
pub fn prepare_native_fixture(request: &Request<'_>, public: &[u8]) -> Result<InstallPlan, String> {
    let layout = fixed_layout()?;
    prepare_checked(
        &crate::management_agent::system_dirs::get()
            .agent()
            .join("transactions")
            .join(request.transaction_id),
        &layout,
        request,
        Some(public),
        false,
    )
}

#[cfg(test)]
pub(crate) fn fixture_layout(root: &Path) -> Result<Layout, String> {
    if root.parent() != Some(Path::new(r"C:\RustDeskAgentLab"))
        || !root
            .file_name()
            .and_then(|v| v.to_str())
            .is_some_and(|v| v.starts_with("exe-tx-v218-"))
    {
        return Err("EXE fixture root invalid".into());
    }
    workspace::verify(root)?;
    Ok(Layout {
        normal: root.join("normal"),
        agent: root.join("agent/agent.exe"),
        state: root.join("state"),
        config: root.join("config"),
    })
}

fn live_preflight(
    layout: &Layout,
    build: u64,
    services: &mut [OriginalService],
) -> Result<String, String> {
    use windows_service::{
        service::{ServiceAccess, ServiceStartType, ServiceState, ServiceType},
        service_manager::{ServiceManager, ServiceManagerAccess},
    };
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)
        .map_err(|e| e.to_string())?;
    for original in services {
        let service = manager
            .open_service(
                &original.name,
                ServiceAccess::QUERY_CONFIG | ServiceAccess::QUERY_STATUS,
            )
            .map_err(|e| e.to_string())?;
        let config = service.query_config().map_err(|e| e.to_string())?;
        let observed = config.executable_path.to_string_lossy();
        let expected = original.command.as_str();
        let unquoted = if original.name == "RustDeskManagementAgent" {
            expected.replacen('"', "", 2)
        } else {
            expected.into()
        };
        if !(observed.eq_ignore_ascii_case(expected) || observed.eq_ignore_ascii_case(&unquoted))
            || config.service_type != ServiceType::OWN_PROCESS
            || !matches!(
                config.start_type,
                ServiceStartType::AutoStart | ServiceStartType::OnDemand
            )
            || !config
                .account_name
                .is_some_and(|a| a.to_string_lossy().eq_ignore_ascii_case("LocalSystem"))
            || service
                .query_status()
                .map_err(|e| e.to_string())?
                .current_state
                != ServiceState::Running
        {
            return Err("EXE fixed installed service contract mismatch".into());
        }
        original.command = observed.into_owned();
        original.start_type = if config.start_type == ServiceStartType::AutoStart {
            "auto"
        } else {
            "manual"
        }
        .into();
    }
    let response = super::health_windows::probe_blocking(
        r"\\.\pipe\RustDesk\management-health-v2",
        &layout.normal.join("RustDesk.exe"),
        build,
        std::time::Duration::from_secs(3),
    )?;
    if response.no_active_sessions != Some(true) {
        return Err("EXE current activity unknown or busy".into());
    }
    Ok(response.version)
}

fn prepare_checked(
    root: &Path,
    layout: &Layout,
    request: &Request<'_>,
    public: Option<&[u8]>,
    fixture: bool,
) -> Result<InstallPlan, String> {
    if !token(request.transaction_id)
        || !token(request.installation_id)
        || !matches!(request.edition, "standard" | "sos")
        || request.current_build == 0
    {
        return Err("EXE request identity invalid".into());
    }
    let key = format!("windows-x86_64-exe-{}", request.edition);
    gates(layout, request.installation_id, request.edition)?;
    let _guard = InstallGuard::try_acquire()
        .map_err(|e| e.to_string())?
        .ok_or("EXE preparation busy")?;
    gates(layout, request.installation_id, request.edition)?;
    if root.exists() {
        return Err("EXE preparation already exists".into());
    }
    workspace::verify(root.parent().ok_or("EXE parent missing")?)?;
    let mut services = snapshots(request.edition);
    let original_version = if fixture {
        "1.5.3".into()
    } else {
        live_preflight(layout, request.current_build, &mut services)?
    };
    let target = match public {
        #[cfg(test)]
        Some(key_bytes) => package_gate::verify_with_key(
            request.manifest,
            &key,
            request.current_build,
            "fixture-key",
            key_bytes,
        )?,
        _ => package_gate::verify(request.manifest, &key, request.current_build)?,
    };
    let (version, build, _) =
        base::update::update_target_release(request.manifest, target).map_err(|e| e.to_string())?;
    let _package = package_gate::lock_verified_file(request.package, target)?;
    workspace::create(root)?;
    for dir in ["backups", "runtime", "assets"] {
        workspace::create(&root.join(dir))?;
    }
    // Extract anew through signed code. Caller-provided inventory is never preparation authority.
    let asset_parent = if fixture {
        root.join("assets")
    } else {
        crate::management_agent::system_dirs::get()
            .agent()
            .join("exe-assets")
    };
    if !asset_parent.exists() {
        workspace::create(&asset_parent)?;
    }
    workspace::verify(&asset_parent)?;
    let sealed = match public {
        #[cfg(test)]
        Some(key_bytes) => exe_assets::prepare_fixture(
            &asset_parent,
            request.package,
            request.manifest,
            &key,
            "exe",
            request.edition,
            request.current_build,
            key_bytes,
        ),
        _ => exe_assets::prepare(
            &asset_parent,
            request.package,
            request.manifest,
            &key,
            "exe",
            request.edition,
            request.current_build,
        ),
    }
    .map_err(|e| format!("EXE sealed assets: {e:?}"))?;
    let target_backup =
        directory_backup::capture(&sealed.root.join("payload"), &root.join("target"))?;
    if target_backup.files.len() != sealed.files.len() {
        return Err("EXE signed file set mismatch".into());
    }
    for (path, size, sha) in &sealed.files {
        if !target_backup
            .files
            .iter()
            .any(|f| &f.path == path && f.size == *size && &f.sha256 == sha)
        {
            return Err("EXE signed inventory mismatch".into());
        }
    }
    for (source, name) in [
        (&layout.normal, "client-backup"),
        (&layout.state, "state-backup"),
        (&layout.config, "config-backup"),
    ] {
        let backup = directory_backup::capture(source, &root.join(name))?;
        backup.verify(&root.join(name))?;
        for f in &backup.files {
            workspace::verify_restore_target(&source.join(&f.path), source == &layout.config)?;
            workspace::verify_file(&root.join(name).join("files").join(&f.path))?;
        }
    }
    let package = copy(root, request.package, "package.exe", false)?;
    let agent_backup = copy(root, &layout.agent, "backups/agent.exe", false)?;
    let identity_backup = copy(
        root,
        &layout.config.join("RustDesk.toml"),
        "backups/identity.toml",
        true,
    )?;
    // Independent current helper snapshot; preparation alone never authorizes stop.
    let helper_source = std::env::current_exe().map_err(|e| e.to_string())?;
    let helper = if cfg!(test) && public.is_some() {
        let mut input = std::fs::File::open(&helper_source).map_err(|e| e.to_string())?;
        let mut output = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(root.join("runtime/helper.exe"))
            .map_err(|e| e.to_string())?;
        std::io::copy(&mut input, &mut output).map_err(|e| e.to_string())?;
        output.sync_all().map_err(|e| e.to_string())?;
        drop(output);
        describe(root, "runtime/helper.exe")?
    } else {
        copy(root, &helper_source, "runtime/helper.exe", false)?
    };
    let metadata: serde_json::Value =
        serde_json::from_str(include_str!("../../version.json")).map_err(|e| e.to_string())?;
    state_store::publish(
        &root.join("approved-manifest.json"),
        &serde_json::to_vec(request.manifest).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    let bootstrap = bootstrap_directory(root);
    let bootstrap_manifest = match std::fs::symlink_metadata(&bootstrap) {
        Ok(_) => {
            workspace::verify(&bootstrap)?;
            let bytes = read(&bootstrap.join("bootstrap.json"), 65536)?;
            Some(Artifact {
                relative_path: "bootstrap-backup/original.json".into(),
                size: bytes.len() as u64,
                sha256: hex::encode(Sha256::digest(&bytes)),
            })
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(e.to_string()),
    };
    let assets = Assets {
        schema: 3,
        transaction_id: request.transaction_id.into(),
        client: describe(root, "client-backup/manifest.json")?,
        state: describe(root, "state-backup/manifest.json")?,
        config: describe(root, "config-backup/manifest.json")?,
        target: describe(root, "target/manifest.json")?,
        approved_manifest: describe(root, "approved-manifest.json")?,
        bootstrap_manifest,
    };
    state_store::publish(
        &root.join("exe-assets.json"),
        &serde_json::to_vec(&assets).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    let plan = InstallPlan {
        schema: 1,
        transaction_id: request.transaction_id.into(),
        trigger: Trigger::LegacyMigration,
        command_id: None,
        installation_id: request.installation_id.into(),
        identity_fingerprint: identity_backup.sha256.clone(),
        target_key: key,
        original_version,
        original_build_seq: request.current_build,
        target_version: version.into(),
        target_build_seq: build,
        normal_install_path: crate::management_agent::system_dirs::get()
            .normal_install()
            .to_string_lossy()
            .into_owned(),
        package,
        client_backup: describe(root, "exe-assets.json")?,
        agent_backup,
        identity_backup,
        helper,
        helper_version: metadata["version"]
            .as_str()
            .ok_or("EXE helper version missing")?
            .into(),
        resume_task: format!("RustDeskManagementAgentResume-{}", request.transaction_id),
        created_at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_secs(),
        services,
    };
    plan.validate()?;
    plan.verify_assets(root)?;
    let target = bootstrap_target(root)?;
    #[cfg(test)]
    super::bootstrap_update::prepare_fixture(root, &bootstrap, &plan, &target)?;
    #[cfg(not(test))]
    super::bootstrap_update::prepare(root, &plan, &target)?;
    verify_bootstrap(root, &plan, &assets)?;
    let mut tx = Transaction::new(
        plan.transaction_id.clone(),
        plan.target_key.clone(),
        plan.package.sha256.clone(),
        plan.identity_fingerprint.clone(),
        plan.created_at,
    )?;
    tx.save(&root.join("transaction.json"))?;
    let evidence = Evidence {
        package_verified: true,
        backups_verified: false,
        helper_verified: false,
        resume_task_verified: false,
        handoff_verified: false,
        maintenance_owned: false,
        services_stopped: false,
        target_verified: false,
        identity_unchanged: false,
        client_ready: false,
        agent_ready: false,
        rollback_verified: false,
        locally_blocked: false,
    };
    tx.advance(Phase::PackageVerified, &evidence, plan.created_at)?;
    tx.save(&root.join("transaction.json"))?;
    plan.save_bound(&root.join("prepared.json"), &tx)?;
    gates(layout, request.installation_id, request.edition)?;
    for (source, name) in [
        (&layout.normal, "client-backup"),
        (&layout.state, "state-backup"),
        (&layout.config, "config-backup"),
    ] {
        let backup = DirectoryBackup::decode(&read(
            &root.join(name).join("manifest.json"),
            8 * 1024 * 1024,
        )?)?;
        if !backup.matches_directory(source)? {
            return Err("EXE old data changed during preparation".into());
        }
    }
    Ok(plan)
}
