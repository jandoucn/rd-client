use super::super::local_repair_plan::total_loss::{OriginalService, Prestate};
use super::*;

fn absent(path: &Path) -> Result<(), String> {
    match std::fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        _ => Err("total loss requires explicit directory absence".into()),
    }
}

fn prestate(fallback: &Path) -> Result<Prestate, String> {
    no_client_process()?;
    absent(&normal())?;
    absent(fallback)?;
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)
        .map_err(|e| e.to_string())?;
    let original_service = match manager.open_service(
        "RustDesk",
        ServiceAccess::QUERY_CONFIG | ServiceAccess::QUERY_STATUS,
    ) {
        Err(windows_service::Error::Winapi(e)) if e.raw_os_error() == Some(1060) => {
            OriginalService::Absent
        }
        Err(e) => return Err(format!("total loss client SCM unknown: {e}")),
        Ok(service) => {
            let config = service.query_config().map_err(|e| e.to_string())?;
            let status = service.query_status().map_err(|e| e.to_string())?;
            if status.current_state != ServiceState::Stopped
                || status.process_id.is_some_and(|p| p != 0)
                || config.service_type != windows_service::service::ServiceType::OWN_PROCESS
                || !config
                    .account_name
                    .is_some_and(|s| s.to_string_lossy().eq_ignore_ascii_case("LocalSystem"))
            {
                return Err("total loss client service not safely stopped".into());
            }
            OriginalService::Stopped {
                command: config.executable_path.to_string_lossy().into_owned(),
                start: match config.start_type {
                    ServiceStartType::AutoStart => "auto",
                    ServiceStartType::OnDemand => "manual",
                    _ => return Err("total loss client disabled or unknown start".into()),
                }
                .into(),
            }
        }
    };
    let result = Prestate {
        client_anchor: "none".into(),
        original_service,
    };
    result.validate(&format!(
        "\"{}\\RustDesk.exe\" --service",
        fallback.display()
    ))?;
    Ok(result)
}

pub(super) fn no_client_process() -> Result<(), String> {
    use windows::Win32::{
        Foundation::CloseHandle,
        System::Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
            TH32CS_SNAPPROCESS,
        },
    };
    let snapshot =
        unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }.map_err(|e| e.to_string())?;
    let result = (|| {
        let mut entry = PROCESSENTRY32W::default();
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        unsafe { Process32FirstW(snapshot, &mut entry) }.map_err(|e| e.to_string())?;
        loop {
            let n = entry
                .szExeFile
                .iter()
                .position(|&c| c == 0)
                .unwrap_or(entry.szExeFile.len());
            if String::from_utf16_lossy(&entry.szExeFile[..n]).eq_ignore_ascii_case("RustDesk.exe")
            {
                let process = super::super::handoff::ObservedProcess::open(entry.th32ProcessID)?;
                #[link(name = "kernel32")]
                extern "system" {
                    fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut std::ffi::c_void;
                    fn QueryFullProcessImageNameW(
                        handle: *mut std::ffi::c_void,
                        flags: u32,
                        path: *mut u16,
                        size: *mut u32,
                    ) -> i32;
                    fn CloseHandle(handle: *mut std::ffi::c_void) -> i32;
                }
                let handle = unsafe { OpenProcess(0x1000, 0, entry.th32ProcessID) };
                if handle.is_null() {
                    return Err("total loss client image inaccessible".into());
                }
                let mut path = vec![0u16; 32768];
                let mut size = path.len() as u32;
                let queried =
                    unsafe { QueryFullProcessImageNameW(handle, 0, path.as_mut_ptr(), &mut size) };
                if unsafe { CloseHandle(handle) } == 0 {
                    return Err("total loss client query handle cleanup failed".into());
                }
                if queried == 0 {
                    return Err("total loss client image unknown".into());
                }
                let image = String::from_utf16_lossy(&path[..size as usize]);
                let current = super::super::handoff::ObservedProcess::open(entry.th32ProcessID)?;
                if current.owner() != process.owner() {
                    return Err("total loss client PID reused".into());
                }
                if process.is_alive()? {
                    let dirs = crate::management_agent::system_dirs::get();
                    let fixed = [
                        dirs.normal_install().join("RustDesk.exe"),
                        dirs.client_root().join(r"sos\RustDesk\RustDesk.exe"),
                        dirs.client_root().join(r"standard\RustDesk\RustDesk.exe"),
                    ];
                    return Err(if fixed
                        .iter()
                        .any(|p| image.eq_ignore_ascii_case(&p.to_string_lossy()))
                    {
                        "total loss fixed client process active"
                    } else {
                        "total loss foreign client image; fail closed"
                    }
                    .into());
                }
                return Err("total loss client process observation raced; retry required".into());
            }
            if let Err(e) = unsafe { Process32NextW(snapshot, &mut entry) } {
                if e.code().0 as u32 != 0x80070012 {
                    return Err(e.to_string());
                }
                break;
            }
        }
        Ok(())
    })();
    unsafe { CloseHandle(snapshot) }.map_err(|e| e.to_string())?;
    result
}

pub(super) fn prepare(id: &str, public: Option<&[u8]>) -> Result<PathBuf, String> {
    let directory = root(id)?;
    absent(&directory)?;
    let _guard = super::super::install_guard::InstallGuard::try_acquire()
        .map_err(|e| e.to_string())?
        .ok_or("total loss installation busy")?;
    require_optin(&agent_root().join("state"))?;
    if !super::super::rescue_assets::r3_cycle_allowed()? {
        return Err("total loss locally blocked".into());
    }
    capacity_admission(
        directory.parent().ok_or("total loss parent absent")?,
        None,
        true,
    )?;
    let (manifest, source, _, build, fallback, installation, source_tx, approval_source) =
        super::super::rescue_assets::local_repair_source(public)?;
    let before = prestate(&fallback)?;
    let key = if fallback.to_string_lossy().contains("\\sos\\") {
        "windows-x86_64-msi-sos"
    } else {
        "windows-x86_64-msi-standard"
    };
    let approved = target(&manifest, key, build, public)?;
    let (version, actual_build, _) =
        base::update::update_target_release(&manifest, approved).map_err(|e| e.to_string())?;
    if actual_build != build {
        return Err("total loss same build required".into());
    }
    let _source = package_gate::lock_verified_file(&source, approved)?;
    let metadata = super::super::msi_metadata::inspect(&source)?;
    let registered = super::super::msi_metadata::installed_source(&metadata.product_code)?;
    let name = package_gate::cache_name(&approved.primary, "msi")?;
    if registered.package_name != name {
        return Err("total loss registered source name mismatch".into());
    }
    let _registered = package_gate::lock_verified_file(&registered.source, approved)?;
    let edition = if key.ends_with("-sos") {
        "sos"
    } else {
        "standard"
    };
    let agent = Contract {
        name: "RustDeskManagementAgent".into(),
        executable: agent_root().join("rustdesk-management-agent.exe"),
        arguments: vec![
            "--service".into(),
            "--edition".into(),
            edition.into(),
            "--package-kind".into(),
            "msi".into(),
            "--normal-install-path".into(),
            normal().to_string_lossy().into_owned(),
        ],
        start_type: ServiceStartType::AutoStart,
    };
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)
        .map_err(|e| e.to_string())?;
    let service = manager
        .open_service(
            "RustDeskManagementAgent",
            ServiceAccess::QUERY_CONFIG | ServiceAccess::QUERY_STATUS,
        )
        .map_err(|e| e.to_string())?;
    let config = service.query_config().map_err(|e| e.to_string())?;
    let observation = service.query_status().map_err(|e| e.to_string())?;
    if !agent.matches_command(&config.executable_path.to_string_lossy())
        || config.start_type != ServiceStartType::AutoStart
        || config.service_type != windows_service::service::ServiceType::OWN_PROCESS
        || !config
            .account_name
            .is_some_and(|s| s.to_string_lossy().eq_ignore_ascii_case("LocalSystem"))
        || !matches!(
            observation.current_state,
            ServiceState::Stopped | ServiceState::Running
        )
    {
        return Err("total loss agent prestate contract mismatch".into());
    }
    let agent_command = config.executable_path.to_string_lossy().into_owned();
    let configured = hbb_common::config::Config::file();
    let identity = [
        crate::management_agent::system_dirs::get()
            .local_service_roaming()
            .join(r"RustDesk\config\RustDesk.toml"),
        crate::management_agent::system_dirs::get()
            .system_profile_roaming()
            .join(r"RustDesk\config\RustDesk.toml"),
    ]
    .into_iter()
    .find(|path| {
        configured
            .to_string_lossy()
            .eq_ignore_ascii_case(&path.to_string_lossy())
    })
    .ok_or("total loss identity outside fixed profiles")?;
    let config = hbb_common::config::Config2::file();
    if !config
        .to_string_lossy()
        .eq_ignore_ascii_case(&identity.with_file_name("RustDesk2.toml").to_string_lossy())
    {
        return Err("total loss identity profile mismatch".into());
    }
    workspace::create(&directory)?;
    workspace::create(&directory.join("runtime"))?;
    let rescue = agent_root().join("rescue/current");
    freeze_approval(
        &directory,
        &rescue,
        &manifest,
        approval_source.as_ref(),
        key,
    )?;
    if let Some(approval) = &approval_source {
        if copy(
            &directory,
            &rescue.join("installation-receipt.json"),
            "installation-receipt.json",
            65536,
        )? != approval.installation_receipt
        {
            return Err("total loss approval changed".into());
        }
    }
    let plan = LocalRepairPlan {
        schema: 1,
        trigger: "local_recovery".into(),
        transaction_id: id.into(),
        installation_id: installation,
        target_key: key.into(),
        build_seq: build,
        version: version.into(),
        original_normal: "missing".into(),
        fallback_path: fallback.to_string_lossy().into_owned(),
        fallback_command: format!("\"{}\\RustDesk.exe\" --service", fallback.display()),
        fallback_start: "auto".into(),
        agent_command,
        agent_start: "auto".into(),
        identity_path: identity.to_string_lossy().into_owned(),
        product_code: metadata.product_code,
        source_transaction: source_tx,
        approval_source,
        total_loss: Some(before.clone()),
        created_at: now()?,
        package: copy(&directory, &source, &name, 2 * 1024 * 1024 * 1024)?,
        client_inventory: copy(
            &directory,
            &rescue.join("client/manifest.json"),
            "client-inventory.json",
            8 * 1024 * 1024,
        )?,
        agent_backup: copy(
            &directory,
            &agent.executable,
            "agent-backup.exe",
            512 * 1024 * 1024,
        )?,
        identity_backup: copy(&directory, &identity, "identity-backup.toml", 65536)?,
        config_backup: copy(&directory, &config, "config-backup.toml", 65536)?,
        bootstrap_backup: copy(
            &directory,
            &crate::management_agent::system_dirs::get()
                .recovery_bootstrap()
                .join("rustdesk-recovery-bootstrap.exe"),
            "bootstrap-backup.exe",
            512 * 1024 * 1024,
        )?,
        bootstrap_manifest: copy(
            &directory,
            &crate::management_agent::system_dirs::get()
                .recovery_bootstrap()
                .join("bootstrap.json"),
            "bootstrap-backup.json",
            65536,
        )?,
        helper: copy(
            &directory,
            &std::env::current_exe().map_err(|e| e.to_string())?,
            "runtime/helper.exe",
            512 * 1024 * 1024,
        )?,
    };
    plan.validate()?;
    let bundle: serde_json::Value =
        serde_json::from_slice(&read(&rescue.join("bundle.json"), 65536)?)
            .map_err(|e| e.to_string())?;
    if bundle["agent"]["sha256"] != plan.agent_backup.sha256
        || bundle["bootstrap"]["sha256"] != plan.bootstrap_backup.sha256
    {
        return Err("total loss backup differs from approved bundle".into());
    }
    save(&directory.join("local-repair-plan.json"), &plan)?;
    let mut tx = Transaction::new(
        id.into(),
        key.into(),
        plan.package.sha256.clone(),
        plan.identity_backup.sha256.clone(),
        plan.created_at,
    )?;
    tx.save(&directory.join("transaction.json"))?;
    tx.advance(Phase::PackageVerified, &evidence(), now()?)?;
    tx.save(&directory.join("transaction.json"))?;
    check_assets(&directory, &plan, public)?;
    if prestate(&fallback)? != before {
        return Err("total loss prestate changed".into());
    }
    Ok(directory)
}

fn check_assets(
    directory: &Path,
    plan: &LocalRepairPlan,
    public: Option<&[u8]>,
) -> Result<(), String> {
    plan.validate()?;
    if plan.total_loss.is_none() {
        return Err("total loss explicit prestate required".into());
    }
    for artifact in plan.artifacts() {
        workspace::verify_file(&directory.join(&artifact.relative_path))?;
        artifact.verify(directory)?;
    }
    let manifest =
        serde_json::from_slice(&read(&directory.join("approved-manifest.json"), 262144)?)
            .map_err(|e| e.to_string())?;
    let approved = target(&manifest, &plan.target_key, plan.build_seq, public)?;
    let (version, build, _) =
        base::update::update_target_release(&manifest, approved).map_err(|e| e.to_string())?;
    if version != plan.version
        || build != plan.build_seq
        || approved.sha256 != plan.package.sha256
        || approved.size != plan.package.size
    {
        return Err("total loss approved package mismatch".into());
    }
    if package_gate::cache_name(&approved.primary, "msi")? != plan.package.relative_path {
        return Err("total loss approved package name mismatch".into());
    }
    let metadata =
        super::super::msi_metadata::inspect(&directory.join(&plan.package.relative_path))?;
    if metadata.product_name != "RustDesk" || metadata.product_code != plan.product_code {
        return Err("total loss MSI identity mismatch".into());
    }
    plan.verify_approval_source(
        directory,
        &agent_root().join("state"),
        &read(&directory.join("identity-backup.toml"), 65536)?,
        public,
    )?;
    original_identity(directory, plan)
}

pub(super) fn launch(
    directory: &Path,
    public: Option<&[u8]>,
) -> Result<std::process::Child, String> {
    let plan = load(directory)?;
    check_assets(directory, &plan, public)?;
    let guard = super::super::install_guard::InstallGuard::try_acquire()
        .map_err(|e| e.to_string())?
        .ok_or("total loss installation busy")?;
    other_transactions(directory)?;
    require_optin(&agent_root().join("state"))?;
    if !super::super::rescue_assets::r3_cycle_allowed()?
        || Some(prestate(Path::new(&plan.fallback_path))?) != plan.total_loss
    {
        return Err("total loss launch prestate or gate changed".into());
    }
    let mut tx = Transaction::decode(&read(&directory.join("transaction.json"), 65536)?)?;
    plan.bind(&tx)?;
    if tx.phase != Phase::PackageVerified {
        return Err("total loss already launched".into());
    }
    let digest = hex::encode(Sha256::digest(read(
        &directory.join("local-repair-plan.json"),
        65536,
    )?));
    let nonce = hex::encode(hbb_common::sodiumoxide::randombytes::randombytes(32));
    super::super::maintenance::claim(
        &agent_root().join("state"),
        &plan.installation_id,
        &plan.transaction_id,
        &digest,
        &nonce,
    )?;
    let helper = directory.join(&plan.helper.relative_path);
    let mut command = std::process::Command::new(&helper);
    command.args([
        "local-repair-helper",
        "--transaction-id",
        &plan.transaction_id,
    ]);
    #[cfg(test)]
    if let Some(public) = public {
        use hbb_common::base64::{engine::general_purpose::STANDARD, Engine as _};
        command = std::process::Command::new(&helper);
        command
            .args(["--exact", "total_loss_chain_child", "--nocapture"])
            .env("RUSTDESK_TOTAL_LOSS_CHILD", &plan.transaction_id)
            .env("RUSTDESK_TOTAL_LOSS_PUBLIC", STANDARD.encode(public));
    }
    command
        .stdout(
            std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(directory.join("child.stdout.log"))
                .map_err(|e| e.to_string())?,
        )
        .stderr(
            std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(directory.join("child.stderr.log"))
                .map_err(|e| e.to_string())?,
        );
    let mut child = command.spawn().map_err(|e| e.to_string())?;
    let parent = super::super::handoff::ObservedProcess::open(std::process::id())?;
    let helper = super::super::handoff::ObservedProcess::open(child.id())?;
    let handoff = super::super::handoff::Handoff::new(
        plan.transaction_id.clone(),
        digest,
        nonce,
        parent.owner().clone(),
        helper.owner().clone(),
        now()?,
    )?;
    save(&directory.join("handoff.json"), &handoff)?;
    let deadline = Instant::now() + Duration::from_secs(180);
    while !directory.join("local-ready.json").exists() {
        if child.try_wait().map_err(|e| e.to_string())?.is_some() || Instant::now() >= deadline {
            return Err("total loss helper readiness failed".into());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let receipt = serde_json::from_slice(&read(&directory.join("local-ready.json"), 4096)?)
        .map_err(|_| "total loss ready malformed")?;
    handoff.verify_ready(&receipt, now()?)?;
    super::super::resume_task::register_definition(
        directory,
        &plan.transaction_id,
        &definition(&plan, directory)?,
    )?;
    verify_task(&plan, directory)?;
    preflight_owned(
        &agent_root().join("state"),
        &hbb_common::config::Config2::file(),
        &plan,
        &handoff,
        || {
            if Some(prestate(Path::new(&plan.fallback_path))?) != plan.total_loss {
                return Err("total loss release prestate changed".into());
            }
            original_identity(directory, &plan)
        },
    )?;
    for phase in [Phase::Prepared, Phase::StoppingServices] {
        tx.advance(phase, &evidence(), now()?)?;
        tx.save(&directory.join("transaction.json"))?;
    }
    save(
        &directory.join("local-release.json"),
        &handoff.release_receipt(&receipt, now()?)?,
    )?;
    drop(guard);
    Ok(child)
}

pub(super) fn run(
    directory: &Path,
    plan: LocalRepairPlan,
    public: Option<&[u8]>,
    resume: bool,
    fail_validation: bool,
) -> Result<(), String> {
    if resume {
        return reconcile(directory, &plan, public);
    }
    check_assets(directory, &plan, public)?;
    let deadline = Instant::now() + Duration::from_secs(180);
    while !directory.join("handoff.json").exists() {
        if Instant::now() >= deadline {
            return Err("total loss handoff timeout".into());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let handoff =
        super::super::handoff::Handoff::decode(&read(&directory.join("handoff.json"), 4096)?)?;
    let current = super::super::handoff::ObservedProcess::open(std::process::id())?;
    if handoff.transaction_id != plan.transaction_id
        || handoff.plan_sha256
            != hex::encode(Sha256::digest(read(
                &directory.join("local-repair-plan.json"),
                65536,
            )?))
        || current.owner() != &handoff.helper
        || !super::super::handoff::ObservedProcess::owner_alive(&handoff.parent)?
    {
        return Err("total loss helper ownership mismatch".into());
    }
    save(
        &directory.join("local-ready.json"),
        &handoff.ready_receipt(now()?)?,
    )?;
    while !directory.join("local-release.json").exists() {
        if Instant::now() >= deadline {
            return Err("total loss release timeout".into());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let ready = serde_json::from_slice(&read(&directory.join("local-ready.json"), 4096)?)
        .map_err(|_| "total loss ready malformed")?;
    let release = serde_json::from_slice(&read(&directory.join("local-release.json"), 4096)?)
        .map_err(|_| "total loss release malformed")?;
    handoff.verify_release(&ready, &release, current.owner(), now()?)?;
    let _guard = loop {
        if let Some(guard) =
            super::super::install_guard::InstallGuard::try_acquire().map_err(|e| e.to_string())?
        {
            break guard;
        }
        if Instant::now() >= deadline {
            return Err("total loss installation busy".into());
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    other_transactions_owned(directory, &handoff)?;
    verify_task(&plan, directory)?;
    preflight_owned(
        &agent_root().join("state"),
        &hbb_common::config::Config2::file(),
        &plan,
        &handoff,
        || {
            if Some(prestate(Path::new(&plan.fallback_path))?) != plan.total_loss {
                return Err("total loss execution prestate changed".into());
            }
            check_assets(directory, &plan, public)
        },
    )?;
    let mut tx = Transaction::decode(&read(&directory.join("transaction.json"), 65536)?)?;
    plan.bind(&tx)?;
    if tx.phase != Phase::StoppingServices {
        return Err("total loss execution phase invalid".into());
    }
    save(
        &directory.join("agent-directory-acl-intent.json"),
        &serde_json::json!({"transaction_id":tx.id,"object":agent_root().to_string_lossy(),"remove_users_create_mask":6}),
    )?;
    let authorize_mutation = || {
        preflight_owned(
            &agent_root().join("state"),
            &hbb_common::config::Config2::file(),
            &plan,
            &handoff,
            || {
                if Some(prestate(Path::new(&plan.fallback_path))?) != plan.total_loss {
                    return Err("total loss mutation prestate changed".into());
                }
                original_identity(directory, &plan)
            },
        )
    };
    super::super::local_recovery_events::total_loss_event_step(
        directory,
        &plan,
        &tx,
        &authorize_mutation,
        || workspace::protect_agent_install_directory(directory, &authorize_mutation),
    )?;
    authorize_mutation()?;
    service_control::stop(&agent_contract(&plan), Duration::from_secs(60))?;
    let mut proof = evidence();
    proof.services_stopped = true;
    for phase in [Phase::ServicesStopped, Phase::Applying] {
        tx.advance(phase, &proof, now()?)?;
        tx.save(&directory.join("transaction.json"))?;
    }
    let mut backend = LocalBackend {
        directory: directory.into(),
        plan: plan.clone(),
        binding: handoff.clone(),
        fail_validation,
    };
    let installation = match backend.apply_package() {
        Ok(InstallOutcome::Pending { .. }) => {
            save(
                &directory.join("local-result.json"),
                &serde_json::json!({"phase":"applying","error":"installer pending; no competing preservation"}),
            )?;
            return Err("total loss installer pending; evidence retained".into());
        }
        Ok(InstallOutcome::RebootRequired | InstallOutcome::Busy) => {
            save(
                &directory.join("local-result.json"),
                &serde_json::json!({"schema":1,"transaction_id":tx.id,"phase":"applying","error":"installer reboot or busy; reconcile before preservation"}),
            )?;
            return Err(
                "total loss installer outcome requires reconciliation; no competing preservation"
                    .into(),
            );
        }
        Ok(InstallOutcome::Installed) => {
            tx.advance(Phase::Validating, &proof, now()?)?;
            tx.save(&directory.join("transaction.json"))?;
            #[cfg(test)]
            if fail_validation {
                let agent = agent_contract(&plan);
                service_control::stop(&agent, Duration::from_secs(60))?;
                std::fs::rename(
                    &agent.executable,
                    directory.join("fault-original-agent.exe"),
                )
                .map_err(|e| e.to_string())?;
            }
            #[cfg(test)]
            let injected = fail_validation;
            #[cfg(not(test))]
            let injected = false;
            if injected {
                Err("total loss injected target verification failure after MSI".into())
            } else {
                backend.validate_target().and_then(|v| {
                    if v.target_matches && v.identity_unchanged && v.client_ready && v.agent_ready {
                        Ok(())
                    } else {
                        Err("total loss target validation failed".into())
                    }
                })
            }
        }
        Ok(value) => Err(format!("total loss installer failed: {value:?}")),
        Err(error) => Err(error),
    };
    match installation {
        Ok(()) => {
            proof.target_verified = true;
            proof.identity_unchanged = true;
            proof.client_ready = true;
            proof.agent_ready = true;
            tx.advance(Phase::Committed, &proof, now()?)?;
            tx.save(&directory.join("transaction.json"))?;
            save(
                &directory.join("local-result.json"),
                &serde_json::json!({"schema":1,"transaction_id":tx.id,"phase":tx.phase,"error":null,"reboot_required":false}),
            )?;
            super::super::maintenance::release_owned(
                &agent_root().join("state"),
                &plan.installation_id,
                &plan.transaction_id,
                &handoff.plan_sha256,
                &handoff.nonce,
            )?;
            super::super::resume_task::remove(&plan.transaction_id)
        }
        Err(error) => {
            save(
                &directory.join("total-loss-install-error.json"),
                &serde_json::json!({"transaction_id":tx.id,
                "plan_sha256":handoff.plan_sha256,"error":error}),
            )?;
            tx.advance(Phase::RollingBack, &proof, now()?)?;
            tx.save(&directory.join("transaction.json"))?;
            let binding = super::super::local_repair_plan::total_loss::Binding {
                transaction_id: plan.transaction_id.clone(),
                installation_id: plan.installation_id.clone(),
                plan_sha256: handoff.plan_sha256.clone(),
                handoff_sha256: hex::encode(Sha256::digest(read(
                    &directory.join("handoff.json"),
                    4096,
                )?)),
            };
            let preserve = || {
                super::super::local_repair_plan::total_loss::preserve_missing_agent_native_bound(
                    directory, &plan, &binding, error,
                )
            };
            #[cfg(test)]
            let receipt = super::super::local_repair_plan::total_loss::product_fixture(preserve)?;
            #[cfg(not(test))]
            let receipt = preserve()?;
            save(
                &directory.join("local-result.json"),
                &serde_json::json!({"schema":1,"transaction_id":tx.id,"phase":"preservation","receipt":receipt}),
            )?;
            Err("total loss client not recovered; preservation evidence retained".into())
        }
    }
}

fn reconcile(
    directory: &Path,
    plan: &LocalRepairPlan,
    public: Option<&[u8]>,
) -> Result<(), String> {
    reconcile_at(
        directory,
        plan,
        public,
        &agent_root().join("state"),
        &hbb_common::config::Config2::file(),
    )
}

#[cfg(test)]
pub(super) fn reconcile_fixture(directory: &Path, public: &[u8]) -> Result<(), String> {
    let plan = LocalRepairPlan::decode(&read(&directory.join("local-repair-plan.json"), 65536)?)?;
    if std::env::current_exe().map_err(|e| e.to_string())? != directory.join("runtime/helper.exe") {
        return Err("total loss fixture helper path mismatch".into());
    }
    reconcile_at(
        directory,
        &plan,
        Some(public),
        &directory.join("fixture-state"),
        &directory.join("fixture-intent.toml"),
    )
}

fn reconcile_at(
    directory: &Path,
    plan: &LocalRepairPlan,
    public: Option<&[u8]>,
    state: &Path,
    intent: &Path,
) -> Result<(), String> {
    use super::super::{
        handoff::{Handoff, ObservedProcess, ReleaseReceipt},
        local_repair_plan::total_loss,
    };
    let _guard = super::super::install_guard::InstallGuard::try_acquire()
        .map_err(|e| e.to_string())?
        .ok_or("total loss reconciliation busy")?;
    check_assets(directory, plan, public)?;
    let handoff_bytes = read(&directory.join("handoff.json"), 4096)?;
    let handoff = Handoff::decode(&handoff_bytes)?;
    let digest = hex::encode(Sha256::digest(read(
        &directory.join("local-repair-plan.json"),
        65536,
    )?));
    if handoff.transaction_id != plan.transaction_id || handoff.plan_sha256 != digest {
        return Err("total loss resume binding mismatch".into());
    }
    let current = ObservedProcess::open(std::process::id())?;
    if current.owner() == &handoff.helper || ObservedProcess::owner_alive(&handoff.helper)? {
        return Err("total loss prior helper still owns transaction".into());
    }
    if !current.is_alive()? {
        return Err("total loss resume helper not alive".into());
    }
    let ready = serde_json::from_slice(&read(&directory.join("local-ready.json"), 4096)?)
        .map_err(|_| "total loss resume ready invalid")?;
    let release: ReleaseReceipt =
        serde_json::from_slice(&read(&directory.join("local-release.json"), 4096)?)
            .map_err(|_| "total loss resume release invalid")?;
    if release.released_at > now()? {
        return Err("total loss resume future release".into());
    }
    handoff.verify_release(&ready, &release, &handoff.helper, release.released_at)?;
    require_optin(state)?;
    super::super::maintenance::verify_terminal_cleanup(
        state,
        intent,
        &plan.installation_id,
        &plan.transaction_id,
        &digest,
        &handoff.nonce,
    )?;
    let tx = Transaction::decode(&read(&directory.join("transaction.json"), 65536)?)?;
    plan.bind(&tx)?;
    match tx.phase {
        Phase::Applying => {
            let outcome = super::super::installer_process::read_recorded(
                directory,
                "install",
                &tx.id,
                &plan.package.sha256,
            )?;
            Err(format!(
                "total loss installer reconciliation required; no new installer: {outcome:?}"
            ))
        }
        Phase::RollingBack => {
            let binding = total_loss::Binding {
                transaction_id: tx.id.clone(),
                installation_id: plan.installation_id.clone(),
                plan_sha256: digest.clone(),
                handoff_sha256: hex::encode(Sha256::digest(&handoff_bytes)),
            };
            match std::fs::symlink_metadata(directory.join("agent-preservation.json")) {
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    super::super::maintenance::verify(
                        state,
                        intent,
                        &plan.installation_id,
                        &plan.transaction_id,
                        &digest,
                        &handoff.nonce,
                    )?;
                    if state == agent_root().join("state") {
                        verify_task(plan, directory)?;
                    }
                    let cause: serde_json::Value = serde_json::from_slice(&read(
                        &directory.join("total-loss-install-error.json"),
                        65536,
                    )?)
                    .map_err(|_| "total loss original error malformed")?;
                    if cause["transaction_id"] != tx.id || cause["plan_sha256"] != digest {
                        return Err("total loss original error binding mismatch".into());
                    }
                    let error = cause["error"]
                        .as_str()
                        .filter(|s| !s.is_empty())
                        .ok_or("total loss original error unavailable")?;
                    let owner = total_loss::ResumeOwner {
                        schema: 1,
                        binding: binding.clone(),
                        owner: current.owner().clone(),
                    };
                    save(&directory.join("total-loss-resume-owner.json"), &owner)?;
                    let preserve = || {
                        total_loss::preserve_missing_agent_native_bound(
                            directory,
                            plan,
                            &binding,
                            error.into(),
                        )
                    };
                    #[cfg(test)]
                    let receipt = if state == directory.join("fixture-state") {
                        total_loss::preserve_resume_fixture(
                            directory,
                            plan,
                            &binding,
                            error.into(),
                        )?
                    } else {
                        total_loss::product_fixture(preserve)?
                    };
                    #[cfg(not(test))]
                    let receipt = preserve()?;
                    if !receipt.confirmed() {
                        return Err(format!(
                            "total loss resumed preservation unconfirmed: {:?}; {:?}",
                            receipt.agent_error, receipt.persistence_error
                        ));
                    }
                }
                Err(e) => return Err(e.to_string()),
                Ok(_) => {}
            }
            let receipt = total_loss::reopen_receipt(directory, &binding)?;
            #[cfg(test)]
            total_loss::product_fixture(|| total_loss::native_verify_running(plan))?;
            #[cfg(not(test))]
            total_loss::native_verify_running(plan)?;
            super::super::maintenance::verify_terminal_cleanup(
                state,
                intent,
                &plan.installation_id,
                &plan.transaction_id,
                &digest,
                &handoff.nonce,
            )?;
            let product_state = state == agent_root().join("state");
            if product_state {
                cleanup_task(directory, plan, false)?;
            }
            // A failed client has no rollback anchor. Never manufacture RolledBack or re-run MSI.
            save(
                &directory.join("total-loss-reconciliation.json"),
                &serde_json::json!({"schema":1,"transaction_id":tx.id,
                "state":"agent_preserved_client_unrecovered","receipt":receipt,"observed_helper":current.owner()}),
            )?;
            match std::fs::symlink_metadata(state.join("maintenance.json")) {
                Ok(_) => super::super::maintenance::release_owned(
                    state,
                    &plan.installation_id,
                    &plan.transaction_id,
                    &digest,
                    &handoff.nonce,
                )?,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e.to_string()),
            }
            if product_state {
                cleanup_task(directory, plan, true)?;
            }
            Err("total loss agent preserved; client unrecovered; reconciliation recorded".into())
        }
        Phase::Committed => {
            cleanup_task(directory, plan, false)?;
            let result: serde_json::Value =
                serde_json::from_slice(&read(&directory.join("local-result.json"), 65536)?)
                    .map_err(|_| "total loss committed result invalid")?;
            if result["transaction_id"] != tx.id
                || result["phase"] != "committed"
                || !result["error"].is_null()
                || result["reboot_required"] != false
            {
                return Err("total loss committed result binding mismatch".into());
            }
            match std::fs::symlink_metadata(state.join("maintenance.json")) {
                Ok(_) => super::super::maintenance::release_owned(
                    state,
                    &plan.installation_id,
                    &plan.transaction_id,
                    &digest,
                    &handoff.nonce,
                )?,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e.to_string()),
            }
            cleanup_task(directory, plan, true)
        }
        _ => {
            Err("total loss incomplete state retained; no installer or rollback authorized".into())
        }
    }
}

pub(super) fn cleanup_task(
    directory: &Path,
    plan: &LocalRepairPlan,
    remove: bool,
) -> Result<(), String> {
    plan.validate()?;
    let path = crate::management_agent::system_dirs::get()
        .system32()
        .join("Tasks")
        .join(format!(
            "RustDeskManagementAgentResume-{}",
            plan.transaction_id
        ));
    match std::fs::symlink_metadata(&path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("total loss cleanup task observation unknown: {e}")),
        Ok(_) => {
            verify_task(plan, directory)?;
            if remove {
                super::super::resume_task::remove(&plan.transaction_id)?;
            }
            Ok(())
        }
    }
}
