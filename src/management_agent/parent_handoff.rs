use super::{handoff::{Handoff, ObservedProcess, ReadyReceipt}, install_guard::InstallGuard,
    install_plan::InstallPlan, maintenance, package_gate, resume_task, state_store,
    transaction::{Evidence, Phase, Transaction}, workspace};
use sha2::{Digest, Sha256};
use std::{io::Read, path::{Path, PathBuf}, process::{Child, Command, Stdio}, time::{Duration, Instant, SystemTime, UNIX_EPOCH}};

pub fn launch(root: &Path) -> Result<Child, String> {
    launch_checked(root, None, false)
}

#[cfg(test)]
pub fn launch_fixture(root: &Path, public: &[u8]) -> Result<Child, String> {
    launch_checked(root, Some(public), false)
}

#[cfg(test)]
pub fn launch_native_fixture(root: &Path, public: &[u8]) -> Result<Child, String> {
    launch_checked(root, Some(public), true)
}

fn read(path: &Path, limit: u64) -> Result<Vec<u8>, String> {
    workspace::verify_file(path)?;
    let mut bytes = Vec::new();
    std::fs::File::open(path).map_err(|e| e.to_string())?.take(limit + 1)
        .read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    if bytes.len() as u64 > limit { return Err("parent handoff record too large".into()); }
    Ok(bytes)
}

fn now() -> Result<u64, String> {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|value| value.as_secs())
        .map_err(|_| "parent handoff clock invalid".into())
}

fn preflight(state: &Path, intent: &Path, installation: &str) -> Result<(), String> {
    for marker in ["disable-agent", "disable-recovery", "maintenance.json"] {
        match std::fs::symlink_metadata(state.join(marker)) {
            Ok(_) => return Err("parent handoff locally blocked".into()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(format!("parent handoff local gate unreadable: {error}")),
        }
    }
    let bytes = std::fs::read(state.join("agent.json")).map_err(|e| e.to_string())?;
    if bytes.len() > 65536 { return Err("parent handoff state too large".into()); }
    let value: serde_json::Value = serde_json::from_slice(&bytes).map_err(|_| "parent handoff state malformed")?;
    if value["agent_installation_id"] != installation || value["disabled"] != false || value["management_agent_enabled"] != true {
        return Err("parent handoff state blocked or identity changed".into());
    }
    #[derive(serde::Deserialize)]
    struct Intent { #[serde(default)] options: std::collections::HashMap<String, String> }
    let text = std::fs::read_to_string(intent).map_err(|_| "parent handoff intent unreadable")?;
    if text.len() > 65536 { return Err("parent handoff intent too large".into()); }
    let value: Intent = hbb_common::toml::from_str(&text).map_err(|_| "parent handoff intent malformed")?;
    if !matches!(value.options.get("stop-service").map(String::as_str), None | Some("" | "N")) {
        return Err("parent handoff user stop intent".into());
    }
    Ok(())
}

fn launch_checked(root: &Path, fixture_key: Option<&[u8]>, native_fixture: bool) -> Result<Child, String> {
    workspace::verify(root)?;
    let bytes = read(&root.join("prepared.json"), 65536)?;
    let plan = InstallPlan::decode(&bytes)?;
    let agent_root = std::env::var_os("ProgramData").map(PathBuf::from)
        .ok_or("ProgramData unavailable")?.join("RustDesk").join("ManagementAgent");
    if fixture_key.is_none() && root != agent_root.join("transactions").join(&plan.transaction_id) {
        return Err("parent handoff root outside fixed transaction directory".into());
    }
    let state = agent_root.join("state");
    let intent = hbb_common::config::Config2::file();
    #[cfg(test)]
    let (state, intent) = if fixture_key.is_some() && !native_fixture { (root.join("state"), root.join("intent.toml")) } else { (state, intent) };
    preflight(&state, &intent, &plan.installation_id)?;
    let Some(guard) = InstallGuard::try_acquire().map_err(|e| e.to_string())? else {
        return Err("parent handoff installation busy".into());
    };
    let mut transaction = Transaction::decode(&read(&root.join("transaction.json"), 65536)?)?;
    plan.bind_transaction(&transaction)?;
    if transaction.phase != Phase::PackageVerified { return Err("parent handoff phase already started".into()); }
    for name in ["handoff.json", "helper-ready.json", "helper-release.json", "resume-definition.xml"] {
        match std::fs::symlink_metadata(root.join(name)) {
            Ok(_) => return Err("parent handoff previous attempt requires reconciliation".into()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.to_string()),
        }
    }
    plan.verify_assets(root)?;
    let manifest: hbb_common::UpdateManifest = serde_json::from_slice(&read(&root.join("approved-manifest.json"), 262144)?)
        .map_err(|_| "parent handoff manifest malformed")?;
    let target = match fixture_key {
        Some(public) => package_gate::verify_with_key(&manifest, &plan.target_key, plan.original_build_seq, "fixture-key", public)?,
        None => package_gate::verify(&manifest, &plan.target_key, plan.original_build_seq)?,
    };
    let (version, build, _) = base::update::update_target_release(&manifest, target).map_err(|e| e.to_string())?;
    if version != plan.target_version || build != plan.target_build_seq || target.size != plan.package.size || target.sha256 != plan.package.sha256 {
        return Err("parent handoff approved package differs from plan".into());
    }
    let _package_lock = package_gate::lock_verified_file(&root.join(&plan.package.relative_path), target)?;
    let helper = root.join(&plan.helper.relative_path);
    workspace::verify_file(&helper)?;
    if fixture_key.is_none() || native_fixture { super::native_install::NativeInstaller::open(root)?; }
    preflight(&state, &intent, &plan.installation_id)?;
    let digest = hex::encode(Sha256::digest(&bytes));
    let nonce = hex::encode(hbb_common::sodiumoxide::randombytes::randombytes(32));
    maintenance::claim(&state, &plan.installation_id, &plan.transaction_id, &digest, &nonce)?;
    let mut command = Command::new(&helper);
    command.args(["transaction-helper", "--transaction-id", &plan.transaction_id]);
    #[cfg(test)]
    if let Some(public) = fixture_key {
        use hbb_common::base64::{engine::general_purpose::STANDARD, Engine as _};
        command = Command::new(&helper);
        let child_test = if native_fixture && std::env::current_exe().map_err(|e|e.to_string())?
            .file_name().is_some_and(|name| name.to_string_lossy().starts_with("rustdesk_management_agent-")) {
            "management_agent::operations_runtime::tests::local_command_native_child"
        } else if native_fixture { "native_transaction_child" }
        else { "helper_process_prepares_only_after_exact_assets_identity_and_parent_validation" };
        command.args(["--exact", child_test, "--nocapture"])
            .env("RUSTDESK_HELPER_RUNTIME_FIXTURE", root)
            .env("RUSTDESK_HELPER_FIXTURE_PUBLIC", STANDARD.encode(public))
            .env("RUSTDESK_HELPER_VALID_TASK", "yes")
            .env("RUSTDESK_HELPER_LOCAL_BLOCK", "");
    }
    command.stdout(Stdio::null()).stderr(std::fs::OpenOptions::new().write(true).create_new(true)
        .open(root.join("parent-helper.stderr.log")).map_err(|e| e.to_string())?);
    let mut child = command.spawn().map_err(|e| format!("parent helper launch: {e}"))?;
    // No release exists until every prerequisite below has been persisted.
    let publish = (|| {
        let parent = ObservedProcess::open(std::process::id())?;
        let process = ObservedProcess::open(child.id())?;
        let binding = Handoff::new(plan.transaction_id.clone(), digest.clone(), nonce.clone(),
            parent.owner().clone(), process.owner().clone(), now()?)?;
        state_store::publish(&root.join("handoff.json"), &serde_json::to_vec(&binding).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
        let deadline = Instant::now() + Duration::from_secs(120);
        while !root.join("helper-ready.json").exists() {
            if child.try_wait().map_err(|e| e.to_string())?.is_some() { return Err("helper exited before readiness".into()); }
            if Instant::now() >= deadline { return Err("helper readiness timed out".into()); }
            std::thread::sleep(Duration::from_millis(100));
        }
        let ready: ReadyReceipt = serde_json::from_slice(&read(&root.join("helper-ready.json"), 4096)?)
            .map_err(|_| "helper readiness malformed")?;
        binding.verify_ready(&ready, now()?)?;
        let definition = resume_task::definition(&helper, &plan.transaction_id)?;
        state_store::publish(&root.join("resume-definition.xml"), definition.as_bytes()).map_err(|e| e.to_string())?;
        resume_task::register_definition(root, &plan.transaction_id, &definition)?;
        resume_task::verify(&resume_task::read_back(&plan.transaction_id)?, &helper, &plan.transaction_id)?;
        maintenance::verify(&state, &intent, &plan.installation_id, &plan.transaction_id, &digest, &nonce)?;
        plan.verify_assets(root)?;
        let evidence = Evidence {package_verified:true,backups_verified:true,helper_verified:true,resume_task_verified:true,
            handoff_verified:true,maintenance_owned:true,services_stopped:false,target_verified:false,identity_unchanged:false,
            client_ready:false,agent_ready:false,rollback_verified:false,locally_blocked:false};
        for phase in [Phase::Prepared, Phase::StoppingServices] {
            transaction.advance(phase, &evidence, now()?)?;
            transaction.save(&root.join("transaction.json"))?;
        }
        let release = binding.release_receipt(&ready, now()?)?;
        state_store::publish(&root.join("helper-release.json"), &serde_json::to_vec(&release).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
        Ok::<(), String>(())
    })();
    drop(guard);
    publish?;
    Ok(child)
}
