use super::{
    directory_backup::DirectoryBackup,
    exe_install::{ExeInstaller, Platform},
    exe_preparation::{self, Layout},
    handoff::{Handoff, ObservedProcess, ReadyReceipt},
    install_guard::InstallGuard,
    install_plan::InstallPlan,
    maintenance, resume_task, state_store,
    transaction::{Evidence, Phase, Transaction},
    workspace,
};
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

fn now() -> Result<u64, String> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|v| v.as_secs())
        .map_err(|e| e.to_string())
}
fn load(root: &Path) -> Result<InstallPlan, String> {
    InstallPlan::decode(&exe_preparation::read(&root.join("prepared.json"), 65536)?)
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

#[derive(Clone)]
pub(crate) struct Authority {
    installation: String,
    transaction: String,
    digest: String,
    nonce: String,
    marker: Vec<u8>,
}
impl Authority {
    fn capture_recovery(
        root: &Path,
        layout: &Layout,
        plan: &InstallPlan,
        binding: &Handoff,
    ) -> Result<Self, String> {
        let marker = exe_preparation::read(&layout.state.join("maintenance.json"), 4096)?;
        let value: serde_json::Value =
            serde_json::from_slice(&marker).map_err(|_| "EXE maintenance malformed")?;
        if value["schema"] != 1
            || value["installation_id"] != plan.installation_id
            || value["transaction_id"] != plan.transaction_id
            || value["plan_sha256"] != binding.plan_sha256
            || value["nonce"] != binding.nonce
        {
            return Err("EXE recovery maintenance binding mismatch".into());
        }
        let authority = Self {
            installation: plan.installation_id.clone(),
            transaction: plan.transaction_id.clone(),
            digest: binding.plan_sha256.clone(),
            nonce: binding.nonce.clone(),
            marker,
        };
        recovery_gates(root, layout, plan, Some(&authority))?;
        Ok(authority)
    }
    pub(crate) fn marker_digest(&self) -> String {
        hex::encode(Sha256::digest(&self.marker))
    }
    fn capture(layout: &Layout, plan: &InstallPlan, binding: &Handoff) -> Result<Self, String> {
        maintenance::verify(
            &layout.state,
            &layout.config.join("RustDesk2.toml"),
            &plan.installation_id,
            &plan.transaction_id,
            &binding.plan_sha256,
            &binding.nonce,
        )?;
        Ok(Self {
            installation: plan.installation_id.clone(),
            transaction: plan.transaction_id.clone(),
            digest: binding.plan_sha256.clone(),
            nonce: binding.nonce.clone(),
            marker: exe_preparation::read(&layout.state.join("maintenance.json"), 4096)?,
        })
    }
    pub(crate) fn intent(&self, layout: &Layout) -> Result<(), String> {
        exe_preparation::local_intent_checked(layout, true)?;
        if exe_preparation::read(&layout.state.join("maintenance.json"), 4096)? != self.marker {
            return Err("EXE maintenance ownership changed".into());
        }
        Ok(())
    }
    pub(crate) fn gates(&self, layout: &Layout, plan: &InstallPlan) -> Result<(), String> {
        self.intent(layout)?;
        exe_preparation::identity_gate(
            layout,
            &self.installation,
            if plan.target_key.ends_with("-sos") {
                "sos"
            } else {
                "standard"
            },
        )
    }
    pub(crate) fn state_matches(
        &self,
        snapshot: &DirectoryBackup,
        path: &Path,
    ) -> Result<bool, String> {
        if snapshot.files.iter().any(|f| f.path == "maintenance.json") {
            return Err("EXE old snapshot contains maintenance".into());
        }
        let mut expected = snapshot.clone();
        expected.files.push(super::directory_backup::BackupFile {
            path: "maintenance.json".into(),
            size: self.marker.len() as u64,
            sha256: hex::encode(Sha256::digest(&self.marker)),
        });
        expected.files.sort_by(|a, b| a.path.cmp(&b.path));
        expected.matches_directory(path)
    }
    pub(crate) fn restore_state(
        &self,
        root: &Path,
        snapshot: &DirectoryBackup,
        path: &Path,
    ) -> Result<(), String> {
        let source = root.join("stopped-state");
        snapshot.verify(&source)?;
        let marker = snapshot
            .files
            .iter()
            .find(|f| f.path == "maintenance.json")
            .ok_or("EXE stopped marker missing")?;
        if marker.sha256 != self.marker_digest() || marker.size != self.marker.len() as u64 {
            return Err("EXE stopped marker binding mismatch".into());
        }
        let combined = root.join("owned-state-rollback");
        if combined.exists() {
            return Err("EXE owned rollback already exists; reconcile".into());
        }
        workspace::create(&combined)?;
        let files = combined.join("files");
        workspace::create(&files)?;
        for d in &snapshot.directories {
            std::fs::create_dir_all(files.join(d)).map_err(|e| e.to_string())?;
        }
        for f in &snapshot.files {
            std::fs::copy(source.join("files").join(&f.path), files.join(&f.path))
                .map_err(|e| e.to_string())?;
        }
        let expected = snapshot.clone();
        expected.verify(&combined)?;
        let displaced = path.with_file_name(format!("state.exe-displaced-{}", self.transaction));
        if displaced.exists() {
            return Err("EXE state rollback displaced occupied".into());
        }
        std::fs::rename(path, &displaced).map_err(|e| e.to_string())?;
        if let Err(error) = std::fs::rename(&files, path) {
            std::fs::rename(&displaced, path)
                .map_err(|e| format!("EXE state restore {error}; revert {e}"))?;
            return Err(error.to_string());
        }
        if !expected.matches_directory(path)? {
            return Err("EXE restored state mismatch".into());
        }
        workspace::verify_file(&path.join("maintenance.json"))
    }
    fn release(&self, layout: &Layout) -> Result<(), String> {
        maintenance::release_owned(
            &layout.state,
            &self.installation,
            &self.transaction,
            &self.digest,
            &self.nonce,
        )
    }
}

pub(crate) fn recovery_gates(
    root: &Path,
    layout: &Layout,
    plan: &InstallPlan,
    authority: Option<&Authority>,
) -> Result<(), String> {
    if let Some(a) = authority {
        a.intent(layout)?;
    } else {
        exe_preparation::local_intent(layout)?;
    }
    plan.verify_assets(root)?;
    let receipt: serde_json::Value = serde_json::from_slice(&exe_preparation::read(
        &root.join("stopped-state.json"),
        65536,
    )?)
    .map_err(|_| "EXE stopped receipt malformed")?;
    if receipt["schema"] != 1
        || receipt["transaction_id"] != plan.transaction_id
        || receipt["installation_id"] != plan.installation_id
        || receipt["package_sha256"] != plan.package.sha256
        || receipt["identity_fingerprint"] != plan.identity_fingerprint
        || receipt["plan_sha256"]
            != hex::encode(Sha256::digest(exe_preparation::read(
                &root.join("prepared.json"),
                65536,
            )?))
    {
        return Err("EXE recovery stopped-state binding mismatch".into());
    }
    let expected = authority.map(|a| a.marker_digest());
    if receipt["maintenance_sha256"] != serde_json::to_value(expected).map_err(|e| e.to_string())? {
        return Err("EXE stopped maintenance mismatch".into());
    }
    let artifact: super::install_plan::Artifact = serde_json::from_value(receipt["state"].clone())
        .map_err(|_| "EXE stopped manifest malformed")?;
    if artifact.relative_path != "stopped-state/manifest.json" {
        return Err("EXE stopped path mismatch".into());
    }
    artifact.verify(root)?;
    let snapshot = DirectoryBackup::decode(&exe_preparation::read(
        &root.join(&artifact.relative_path),
        8 * 1024 * 1024,
    )?)?;
    snapshot.verify(&root.join("stopped-state"))?;
    let stopped: serde_json::Value = serde_json::from_slice(&exe_preparation::read(
        &root.join("stopped-state/files/agent.json"),
        65536,
    )?)
    .map_err(|_| "EXE stopped state malformed")?;
    if stopped["schema_version"] != 1
        || stopped["agent_installation_id"] != plan.installation_id
        || stopped["package_kind"] != "exe"
        || stopped["edition"]
            != if plan.target_key.ends_with("-sos") {
                "sos"
            } else {
                "standard"
            }
    {
        return Err("EXE stopped identity mismatch".into());
    }
    let assets: exe_preparation::Assets = serde_json::from_slice(&exe_preparation::read(
        &root.join("exe-assets.json"),
        65536,
    )?)
    .map_err(|_| "EXE assets malformed")?;
    assets.config.verify(root)?;
    let config = DirectoryBackup::decode(&exe_preparation::read(
        &root.join(&assets.config.relative_path),
        8 * 1024 * 1024,
    )?)?;
    if !config.matches_directory(&layout.config)? {
        return Err("EXE real identity/config changed; preserve".into());
    }
    Ok(())
}

fn fixed_root(id: &str) -> Result<PathBuf, String> {
    if id.len() != 32
        || !id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err("EXE transaction ID invalid".into());
    }
    Ok(crate::management_agent::system_dirs::get()
        .agent()
        .join("transactions")
        .join(id))
}

pub(crate) fn dispatch(args: &[std::ffi::OsString], mode: &str) -> Result<(), String> {
    if args.len() != 2 || args[0] != "--transaction-id" {
        return Err("EXE runtime accepts only transaction ID".into());
    }
    let root = fixed_root(args[1].to_str().ok_or("EXE transaction ID encoding")?)?;
    let layout = exe_preparation::fixed_layout()?;
    match mode {
        "exe-transaction-prepare" => prepare_cli(&root),
        "exe-transaction-launch" => launch(&root, &layout, None).map(|_| ()),
        "exe-transaction-helper" => helper(&root, &layout, None),
        "exe-transaction-resume" => resume(&root, &layout, None),
        "exe-selfcheck" => selfcheck(&root, &layout),
        _ => Err("EXE runtime command invalid".into()),
    }
}

fn prepare_cli(root: &Path) -> Result<(), String> {
    let id = root
        .file_name()
        .and_then(|v| v.to_str())
        .ok_or("EXE transaction ID missing")?;
    let inbox = crate::management_agent::system_dirs::get()
        .agent()
        .join("exe-inbox")
        .join(id);
    workspace::verify(&inbox)?;
    let manifest: hbb_common::UpdateManifest = serde_json::from_slice(&exe_preparation::read(
        &inbox.join("approved-manifest.json"),
        262144,
    )?)
    .map_err(|_| "EXE inbox manifest malformed")?;
    let layout = exe_preparation::fixed_layout()?;
    workspace::verify_restore_target(&layout.state.join("agent.json"), false)?;
    let state: serde_json::Value = serde_json::from_slice(
        &std::fs::read(layout.state.join("agent.json")).map_err(|e| e.to_string())?,
    )
    .map_err(|_| "EXE installed state malformed")?;
    if state["package_kind"] != "exe" {
        return Err("EXE installed kind mismatch".into());
    }
    let installation = state["agent_installation_id"]
        .as_str()
        .ok_or("EXE installed identity missing")?;
    let edition = state["edition"]
        .as_str()
        .ok_or("EXE installed edition missing")?;
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Input {
        current_build: u64,
    }
    let input: Input =
        serde_json::from_slice(&exe_preparation::read(&inbox.join("prepare.json"), 4096)?)
            .map_err(|_| "EXE preparation input malformed")?;
    let package = inbox.join("package.exe");
    let request = exe_preparation::Request {
        transaction_id: id,
        installation_id: installation,
        edition,
        current_build: input.current_build,
        package: &package,
        manifest: &manifest,
    };
    exe_preparation::prepare(&request).map(|_| ())
}

#[cfg(test)]
pub(crate) fn launch_fixture(root: &Path, public: &[u8]) -> Result<Child, String> {
    launch(
        root,
        &exe_preparation::fixture_layout(root.parent().ok_or("fixture parent")?)?,
        Some(public),
    )
}
#[cfg(test)]
pub(crate) fn helper_fixture(root: &Path, public: &[u8]) -> Result<(), String> {
    helper(
        root,
        &exe_preparation::fixture_layout(root.parent().ok_or("fixture parent")?)?,
        Some(public),
    )
}
#[cfg(test)]
pub(crate) fn resume_fixture(root: &Path, public: &[u8]) -> Result<(), String> {
    resume(
        root,
        &exe_preparation::fixture_layout(root.parent().ok_or("fixture parent")?)?,
        Some(public),
    )
}

#[cfg(test)]
pub(crate) fn launch_native_fixture(root: &Path, public: &[u8]) -> Result<Child, String> {
    launch(root, &exe_preparation::fixed_layout()?, Some(public))
}
#[cfg(test)]
pub(crate) fn helper_native_fixture(root: &Path, public: &[u8]) -> Result<(), String> {
    helper(root, &exe_preparation::fixed_layout()?, Some(public))
}

fn native(root: &Path) -> bool {
    root.parent()
        == Some(
            crate::management_agent::system_dirs::get()
                .agent()
                .join("transactions")
                .as_path(),
        )
}

fn launch(root: &Path, layout: &Layout, public: Option<&[u8]>) -> Result<Child, String> {
    workspace::verify(root)?;
    let plan = load(root)?;
    let _package = exe_preparation::verify_bound(root, &plan, public)?;
    let guard = InstallGuard::try_acquire()
        .map_err(|e| e.to_string())?
        .ok_or("EXE parent install lock busy")?;
    let mut tx = Transaction::decode(&exe_preparation::read(
        &root.join("transaction.json"),
        65536,
    )?)?;
    plan.bind_transaction(&tx)?;
    if tx.phase != Phase::PackageVerified {
        return Err("EXE transaction already launched; reconcile".into());
    }
    for name in [
        "handoff.json",
        "helper-ready.json",
        "helper-release.json",
        "resume-task.xml",
    ] {
        if root.join(name).exists() {
            return Err("EXE previous handoff requires reconciliation".into());
        }
    }
    parent_preflight(root, layout, &plan, public)?;
    let digest = hex::encode(Sha256::digest(exe_preparation::read(
        &root.join("prepared.json"),
        65536,
    )?));
    let nonce = hex::encode(hbb_common::sodiumoxide::randombytes::randombytes(32));
    maintenance::claim(
        &layout.state,
        &plan.installation_id,
        &plan.transaction_id,
        &digest,
        &nonce,
    )?;
    let helper = root.join(&plan.helper.relative_path);
    let mut command = Command::new(&helper);
    command.args([
        "exe-transaction-helper",
        "--transaction-id",
        &plan.transaction_id,
    ]);
    #[cfg(test)]
    if let Some(key) = public {
        use hbb_common::base64::{engine::general_purpose::STANDARD, Engine as _};
        command = Command::new(&helper);
        command
            .args([
                "--exact",
                if native(root) {
                    "native_exe_runtime_child"
                } else {
                    "runtime_helper_owned_maintenance_commits_and_preflight_failure_keeps_journal"
                },
                "--nocapture",
            ])
            .env("EXE_RUNTIME_TEST_ROOT", root)
            .env("EXE_RUNTIME_PUBLIC", STANDARD.encode(key));
    }
    let mut child = command
        .stdout(Stdio::null())
        .stderr(
            std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(root.join("exe-helper.stderr.log"))
                .map_err(|e| e.to_string())?,
        )
        .spawn()
        .map_err(|e| e.to_string())?;
    let parent = ObservedProcess::open(std::process::id())?;
    let observed = ObservedProcess::open(child.id())?;
    let binding = Handoff::new(
        plan.transaction_id.clone(),
        digest,
        nonce,
        parent.owner().clone(),
        observed.owner().clone(),
        now()?,
    )?;
    state_store::publish(
        &root.join("handoff.json"),
        &serde_json::to_vec(&binding).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    let deadline = Instant::now() + Duration::from_secs(120);
    while !root.join("helper-ready.json").exists() {
        if child.try_wait().map_err(|e| e.to_string())?.is_some() {
            return Err("EXE helper failed before ready".into());
        }
        if Instant::now() >= deadline {
            return Err("EXE helper ready timeout".into());
        }
        std::thread::sleep(Duration::from_millis(30));
    }
    let ready: ReadyReceipt = serde_json::from_slice(&exe_preparation::read(
        &root.join("helper-ready.json"),
        4096,
    )?)
    .map_err(|_| "EXE ready malformed")?;
    binding.verify_ready(&ready, now()?)?;
    let definition = resume_task::definition(&helper, &plan.transaction_id)?.replace(
        "resume-install --transaction-id",
        "exe-transaction-resume --transaction-id",
    );
    resume_task::register_definition(root, &plan.transaction_id, &definition)?;
    verify_task(root, &plan)?;
    let authority = Authority::capture(layout, &plan, &binding)?;
    authority.gates(layout, &plan)?;
    for phase in [Phase::Prepared, Phase::StoppingServices] {
        tx.advance(phase, &evidence(), now()?)?;
        tx.save(&root.join("transaction.json"))?;
    }
    let release = binding.release_receipt(&ready, now()?)?;
    // Release is published under the lock. The helper must acquire that lock before stopping.
    state_store::publish(
        &root.join("helper-release.json"),
        &serde_json::to_vec(&release).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    drop(guard);
    Ok(child)
}

fn verify_task(root: &Path, plan: &InstallPlan) -> Result<(), String> {
    let xml = resume_task::read_back(&plan.transaction_id)?;
    if !xml.contains("exe-transaction-resume --transaction-id") {
        return Err("EXE resume task action mismatch".into());
    }
    resume_task::verify(
        &xml.replace(
            "exe-transaction-resume --transaction-id",
            "resume-install --transaction-id",
        ),
        &root.join(&plan.helper.relative_path),
        &plan.transaction_id,
    )
}

fn task_present(plan: &InstallPlan) -> Result<bool, String> {
    plan.validate()?;
    let script = format!(
        r#"$ErrorActionPreference='Stop';$s=New-Object -ComObject Schedule.Service;$s.Connect();try{{$null=$s.GetFolder('\').GetTask('RustDeskManagementAgentResume-{}');[Console]::Write('PRESENT')}}catch{{$e=$_.Exception;while($e.InnerException){{$e=$e.InnerException}};if($e.HResult -eq -2147024894){{[Console]::Write('ABSENT')}}else{{throw}}}}"#,
        plan.transaction_id
    );
    let output = Command::new(
        crate::management_agent::system_dirs::get()
            .system32()
            .join(r"WindowsPowerShell\v1.0\powershell.exe"),
    )
    .args(["-NoProfile", "-NonInteractive", "-Command", &script])
    .output()
    .map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err("EXE task query failed".into());
    }
    match output.stdout.as_slice() {
        b"PRESENT" => Ok(true),
        b"ABSENT" => Ok(false),
        _ => Err("EXE task query ambiguous".into()),
    }
}

fn terminal_cleanup(
    root: &Path,
    layout: &Layout,
    plan: &InstallPlan,
    binding: &Handoff,
    tx: &Transaction,
) -> Result<(), String> {
    plan.bind_transaction(tx)?;
    if !matches!(tx.phase, Phase::Committed | Phase::RolledBack) {
        return Err("EXE cleanup requires terminal transaction".into());
    }
    let result: serde_json::Value = serde_json::from_slice(&exe_preparation::read(
        &root.join("execution-result.json"),
        65536,
    )?)
    .map_err(|_| "EXE terminal receipt malformed")?;
    if result["schema"] != 1
        || result["transaction_id"] != plan.transaction_id
        || result["installation_id"] != plan.installation_id
        || result["plan_sha256"] != binding.plan_sha256
        || result["stopped_state_sha256"]
            != hex::encode(Sha256::digest(exe_preparation::read(
                &root.join("stopped-state.json"),
                65536,
            )?))
        || result["phase"] != serde_json::to_value(tx.phase).map_err(|e| e.to_string())?
        || result["reboot_required"] != false
        || !result
            .get("error")
            .is_some_and(|e| e.is_null() || e.is_string())
        || (tx.phase == Phase::Committed && !result["error"].is_null())
    {
        return Err("EXE terminal receipt binding incomplete or inconsistent".into());
    }
    exe_preparation::local_intent_checked(layout, true)?;
    exe_preparation::identity_gate(
        layout,
        &plan.installation_id,
        if plan.target_key.ends_with("-sos") {
            "sos"
        } else {
            "standard"
        },
    )?;
    let authority = match std::fs::symlink_metadata(layout.state.join("maintenance.json")) {
        Ok(_) => Some(Authority::capture(layout, plan, binding)?),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(e.to_string()),
    };
    let present = task_present(plan)?;
    if present {
        verify_task(root, plan)?;
    }
    if let Some(authority) = authority {
        authority.release(layout)?;
    }
    #[cfg(test)]
    if std::env::var("EXE_CLEANUP_INTERRUPT").ok().as_deref() == Some("yes") {
        std::process::exit(79);
    }
    if present {
        verify_task(root, plan)?;
        resume_task::remove(&plan.transaction_id)?;
    }
    if task_present(plan)? || layout.state.join("maintenance.json").exists() {
        return Err("EXE cleanup completion not confirmed".into());
    }
    let cleanup=serde_json::to_vec(&serde_json::json!({"schema":1,"cleanup_finished":true,"transaction_id":plan.transaction_id,"installation_id":plan.installation_id,"phase":tx.phase,
        "helper":binding.helper,"plan_sha256":binding.plan_sha256,
        "transaction_sha256":hex::encode(Sha256::digest(exe_preparation::read(&root.join("transaction.json"),65536)?)),
        "execution_result_sha256":hex::encode(Sha256::digest(exe_preparation::read(&root.join("execution-result.json"),65536)?)),
        "handoff_sha256":hex::encode(Sha256::digest(exe_preparation::read(&root.join("handoff.json"),4096)?))})).map_err(|e|e.to_string())?;
    let path = root.join("cleanup-complete.json");
    if path.exists() {
        if exe_preparation::read(&path, 65536)? != cleanup {
            return Err("EXE cleanup receipt immutable conflict".into());
        }
    } else {
        state_store::publish(&path, &cleanup).map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn publish_result(
    root: &Path,
    plan: &InstallPlan,
    result: &super::install_execution::ExecutionResult,
) -> Result<(), String> {
    let receipt = serde_json::json!({"schema":1,"transaction_id":plan.transaction_id,"installation_id":plan.installation_id,
        "plan_sha256":hex::encode(Sha256::digest(exe_preparation::read(&root.join("prepared.json"),65536)?)),
        "stopped_state_sha256":hex::encode(Sha256::digest(exe_preparation::read(&root.join("stopped-state.json"),65536)?)),
        "phase":result.phase,"error":result.error,"reboot_required":result.reboot_required});
    state_store::publish(
        &root.join("execution-result.json"),
        &serde_json::to_vec(&receipt).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())
}

fn parent_preflight(
    root: &Path,
    layout: &Layout,
    plan: &InstallPlan,
    public: Option<&[u8]>,
) -> Result<(), String> {
    #[cfg(test)]
    if public.is_some() && !native(root) {
        if root.parent().is_some_and(|p| p.join("scm.json").exists()) {
            let platform = fixture_scm(root, plan, None)?;
            let mut backend = ExeInstaller::open_checked(root, layout.clone(), platform, public)?;
            return backend.preflight();
        }
        let mut backend = ExeInstaller::open_checked(
            root,
            layout.clone(),
            FixturePlatform { stopped: false },
            public,
        )?;
        return backend.preflight();
    }
    let platform = super::exe_service::FixedPlatformAdapter::fixed(plan)?;
    let mut backend = ExeInstaller::open_checked(root, layout.clone(), platform, public)?;
    backend.preflight()?;
    run_selfcheck(root, plan)
}

fn helper(root: &Path, layout: &Layout, public: Option<&[u8]>) -> Result<(), String> {
    let plan = load(root)?;
    if std::env::current_exe().map_err(|e| e.to_string())? != root.join(&plan.helper.relative_path)
    {
        return Err("EXE helper exact path mismatch".into());
    }
    let deadline = Instant::now() + Duration::from_secs(120);
    while !root.join("handoff.json").exists() {
        if Instant::now() >= deadline {
            return Err("EXE handoff timeout".into());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let binding = Handoff::decode(&exe_preparation::read(&root.join("handoff.json"), 4096)?)?;
    let current = ObservedProcess::open(std::process::id())?;
    if current.owner() != &binding.helper
        || binding.transaction_id != plan.transaction_id
        || binding.plan_sha256
            != hex::encode(Sha256::digest(exe_preparation::read(
                &root.join("prepared.json"),
                65536,
            )?))
    {
        return Err("EXE handoff identity mismatch".into());
    }
    let parent = ObservedProcess::open(binding.parent.pid)?;
    if parent.owner() != &binding.parent || !parent.is_alive()? {
        return Err("EXE parent no longer owns handoff".into());
    }
    let _package = exe_preparation::verify_bound(root, &plan, public)?;
    let authority = Authority::capture(layout, &plan, &binding)?;
    let ready = binding.ready_receipt(now()?)?;
    state_store::publish(
        &root.join("helper-ready.json"),
        &serde_json::to_vec(&ready).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    while !root.join("helper-release.json").exists() {
        if Instant::now() >= deadline {
            return Err("EXE release timeout".into());
        }
        std::thread::sleep(Duration::from_millis(30));
    }
    let release = serde_json::from_slice(&exe_preparation::read(
        &root.join("helper-release.json"),
        4096,
    )?)
    .map_err(|_| "EXE release malformed")?;
    binding.verify_release(&ready, &release, current.owner(), now()?)?;
    let _guard = loop {
        if let Some(g) = InstallGuard::try_acquire().map_err(|e| e.to_string())? {
            break g;
        }
        if Instant::now() >= deadline {
            return Err("EXE handoff lock timeout".into());
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    verify_task(root, &plan)?;
    authority.gates(layout, &plan)?;
    let mut tx = Transaction::decode(&exe_preparation::read(
        &root.join("transaction.json"),
        65536,
    )?)?;
    #[cfg(test)]
    if public.is_some() && !native(root) {
        if root.parent().is_some_and(|p| p.join("scm.json").exists()) {
            let platform = fixture_scm(root, &plan, Some(authority.clone()))?;
            return execute_owned(root, layout, &plan, &authority, &mut tx, platform, public);
        }
        return execute_owned(
            root,
            layout,
            &plan,
            &authority,
            &mut tx,
            FixturePlatform { stopped: false },
            public,
        );
    }
    run_selfcheck(root, &plan)?;
    let mut platform =
        super::exe_service::FixedPlatformAdapter::fixed_owned(&plan, authority.clone())?;
    platform.preflight()?;
    execute_owned(root, layout, &plan, &authority, &mut tx, platform, public)
}

fn execute_owned<P: Platform>(
    root: &Path,
    layout: &Layout,
    plan: &InstallPlan,
    authority: &Authority,
    tx: &mut Transaction,
    platform: P,
    public: Option<&[u8]>,
) -> Result<(), String> {
    let mut backend = ExeInstaller::open_checked(root, layout.clone(), platform, public)?;
    backend.own(authority.clone());
    let result = backend.execute(tx, &evidence(), now()?)?;
    publish_result(root, plan, &result)?;
    #[cfg(test)]
    if std::env::var("EXE_TERMINAL_INTERRUPT").ok().as_deref() == Some("yes") {
        std::process::exit(78);
    }
    if !matches!(result.phase, Phase::Committed | Phase::RolledBack) {
        return Err("EXE transaction requires reconciliation".into());
    }
    let binding = Handoff::decode(&exe_preparation::read(&root.join("handoff.json"), 4096)?)?;
    terminal_cleanup(root, layout, plan, &binding, tx)
}

fn resume(root: &Path, layout: &Layout, public: Option<&[u8]>) -> Result<(), String> {
    let plan = load(root)?;
    if std::env::current_exe().map_err(|e| e.to_string())? != root.join(&plan.helper.relative_path)
    {
        return Err("EXE resume exact helper path mismatch".into());
    }
    let _guard = InstallGuard::try_acquire()
        .map_err(|e| e.to_string())?
        .ok_or("EXE resume install lock busy")?;
    let _package = exe_preparation::verify_bound(root, &plan, public)?;
    let binding = Handoff::decode(&exe_preparation::read(&root.join("handoff.json"), 4096)?)?;
    if binding.transaction_id != plan.transaction_id
        || binding.plan_sha256
            != hex::encode(Sha256::digest(exe_preparation::read(
                &root.join("prepared.json"),
                65536,
            )?))
    {
        return Err("EXE resume binding mismatch".into());
    }
    if ObservedProcess::owner_alive(&binding.helper)? {
        return Err("EXE original helper still alive".into());
    }
    let mut tx = Transaction::decode(&exe_preparation::read(
        &root.join("transaction.json"),
        65536,
    )?)?;
    plan.bind_transaction(&tx)?;
    if matches!(
        tx.phase,
        Phase::ServicesStopped | Phase::Applying | Phase::Validating | Phase::RollingBack
    ) {
        let authority = Authority::capture_recovery(root, layout, &plan, &binding)?;
        verify_task(root, &plan)?;
        #[cfg(test)]
        if public.is_some() && !native(root) {
            if root.parent().is_some_and(|p| p.join("scm.json").exists()) {
                let platform = fixture_scm_checked(root, &plan, Some(authority.clone()), true)?;
                return resume_owned(root, layout, &plan, &authority, &mut tx, platform, public);
            }
            return resume_owned(
                root,
                layout,
                &plan,
                &authority,
                &mut tx,
                FixturePlatform { stopped: false },
                public,
            );
        }
        let platform =
            super::exe_service::FixedPlatformAdapter::fixed_recovery(&plan, authority.clone())?;
        return resume_owned(root, layout, &plan, &authority, &mut tx, platform, public);
    }
    if !matches!(tx.phase, Phase::Committed | Phase::RolledBack) {
        return Err("EXE interrupted before confirmed stop; no destructive replay".into());
    }
    terminal_cleanup(root, layout, &plan, &binding, &tx)
}

fn resume_owned<P: Platform>(
    root: &Path,
    layout: &Layout,
    plan: &InstallPlan,
    authority: &Authority,
    tx: &mut Transaction,
    platform: P,
    public: Option<&[u8]>,
) -> Result<(), String> {
    let mut backend = ExeInstaller::open_checked(root, layout.clone(), platform, public)?;
    backend.own(authority.clone());
    let result = backend.resume_rollback(tx, &evidence(), now()?)?;
    publish_result(root, plan, &result)?;
    if result.phase != Phase::RolledBack {
        return Err(format!(
            "EXE interrupted rollback failed: {:?}",
            result.error
        ));
    }
    let binding = Handoff::decode(&exe_preparation::read(&root.join("handoff.json"), 4096)?)?;
    terminal_cleanup(root, layout, plan, &binding, tx)
}

fn run_selfcheck(root: &Path, plan: &InstallPlan) -> Result<(), String> {
    let target = root.join("target/files/rustdesk-management-agent.exe");
    let mut child = Command::new(target)
        .args(["exe-selfcheck", "--transaction-id", &plan.transaction_id])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| e.to_string())?;
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            return if status.success() {
                Ok(())
            } else {
                Err("EXE target schema selfcheck failed".into())
            };
        }
        if Instant::now() >= deadline {
            child.kill().map_err(|e| e.to_string())?;
            child.wait().map_err(|e| e.to_string())?;
            return Err("EXE target selfcheck timeout".into());
        }
        std::thread::sleep(Duration::from_millis(30));
    }
}

fn selfcheck(root: &Path, layout: &Layout) -> Result<(), String> {
    let plan = load(root)?;
    if std::env::current_exe().map_err(|e| e.to_string())?
        != root.join("target/files/rustdesk-management-agent.exe")
    {
        return Err("EXE selfcheck exact path mismatch".into());
    }
    plan.verify_assets(root)?;
    let metadata: serde_json::Value =
        serde_json::from_str(include_str!("../../version.json")).map_err(|e| e.to_string())?;
    if metadata["build_seq"] != plan.target_build_seq {
        return Err("EXE target selfcheck build mismatch".into());
    }
    exe_preparation::identity_gate(
        layout,
        &plan.installation_id,
        if plan.target_key.ends_with("-sos") {
            "sos"
        } else {
            "standard"
        },
    )?;
    #[cfg(not(test))]
    {
        use std::io::Read;
        let state_path = layout.state.join("agent.json");
        workspace::verify_restore_target(&state_path, false)?;
        let mut bytes = vec![];
        std::fs::File::open(&state_path)
            .map_err(|e| e.to_string())?
            .take(65537)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        if bytes.len() > 65536 {
            return Err("EXE state exceeds capacity".into());
        }
        let _: super::AgentState = serde_json::from_slice(&bytes)
            .map_err(|e| format!("EXE state schema unreadable: {e}"))?;
        for entry in std::fs::read_dir(&layout.state).map_err(|e| e.to_string())? {
            let path = entry.map_err(|e| e.to_string())?.path();
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .ok_or("EXE state filename")?;
            if name.starts_with("commands-") && name.ends_with(".json") {
                workspace::verify_restore_target(&path, false)?;
                let _journal = super::journal::CommandJournal::open(&path)?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
fn fixture_scm(
    root: &Path,
    plan: &InstallPlan,
    authority: Option<Authority>,
) -> Result<super::exe_service::FixedPlatformAdapter, String> {
    fixture_scm_checked(root, plan, authority, false)
}
#[cfg(test)]
fn fixture_scm_checked(
    root: &Path,
    plan: &InstallPlan,
    authority: Option<Authority>,
    recovery: bool,
) -> Result<super::exe_service::FixedPlatformAdapter, String> {
    let parent = root.parent().ok_or("EXE fixture parent missing")?;
    let config: serde_json::Value =
        serde_json::from_slice(&exe_preparation::read(&parent.join("scm.json"), 65536)?)
            .map_err(|e| e.to_string())?;
    let contracts = [0, 1].map(|i| {
        Ok(super::service_control::Contract {
            name: config["names"][i].as_str().ok_or("fixture name")?.into(),
            executable: parent.join(if i == 0 {
                "normal/RustDesk.exe"
            } else {
                "agent/agent.exe"
            }),
            arguments: vec![
                "--exact".into(),
                "scm_fixture_service_entry".into(),
                "--nocapture".into(),
            ],
            start_type: windows_service::service::ServiceStartType::OnDemand,
        })
    });
    let [main, agent]: [Result<super::service_control::Contract, String>; 2] = contracts;
    let pipes = [
        config["pipes"][0].as_str().ok_or("pipe")?.into(),
        config["pipes"][1].as_str().ok_or("pipe")?.into(),
    ];
    if recovery {
        return super::exe_service::FixedPlatformAdapter::fixture_recovery(
            parent,
            plan,
            [main?, agent?],
            pipes,
            Duration::from_secs(5),
            authority.ok_or("EXE recovery authority missing")?,
        );
    }
    super::exe_service::FixedPlatformAdapter::fixture_owned(
        parent,
        plan,
        [main?, agent?],
        pipes,
        Duration::from_secs(5),
        authority,
    )
}

#[cfg(test)]
struct FixturePlatform {
    stopped: bool,
}
#[cfg(test)]
impl Platform for FixturePlatform {
    fn preflight(&mut self) -> Result<(), String> {
        Ok(())
    }
    fn stop(&mut self) -> Result<(), String> {
        self.stopped = true;
        Ok(())
    }
    fn stopped(&mut self) -> Result<bool, String> {
        Ok(self.stopped)
    }
    fn start_and_validate(&mut self, _: u64) -> Result<(bool, bool), String> {
        if std::env::var("EXE_RUNTIME_INTERRUPT_TEST").ok().as_deref() == Some("yes") {
            std::process::exit(77);
        }
        Ok((true, true))
    }
    fn restore_services(&mut self, _: u64) -> Result<(bool, bool), String> {
        Ok((true, true))
    }
}
