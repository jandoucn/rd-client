use super::{
    directory_backup::DirectoryBackup,
    file_restore,
    install_execution::{Backend, InstallOutcome, Validation},
    install_plan::{Artifact, InstallPlan},
    installer_process,
    service_control::{self, Contract},
    workspace,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    io::Read,
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};
use windows_service::service::ServiceStartType;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeAssets {
    pub schema: u32,
    pub transaction_id: String,
    pub client_target_manifest: Artifact,
    pub agent_target: Artifact,
    pub original_msi: Artifact,
    pub original_product_code: String,
    pub target_product_code: String,
    pub identity_path: String,
}

fn product_code(value: &str) -> bool {
    value.len() == 38
        && value.starts_with('{')
        && value.ends_with('}')
        && value.bytes().enumerate().all(|(i, b)| match i {
            0 => b == b'{',
            37 => b == b'}',
            9 | 14 | 19 | 24 => b == b'-',
            _ => b.is_ascii_hexdigit(),
        })
}

impl NativeAssets {
    pub fn validate(&self) -> Result<(), String> {
        if !matches!(self.schema, 1 | 2)
            || self.transaction_id.len() != 32
            || !self
                .transaction_id
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            || !product_code(&self.original_product_code)
            || !product_code(&self.target_product_code)
        {
            return Err("native installation assets/product identity invalid".into());
        }
        let dirs = super::system_dirs::get();
        if ![dirs.local_service_roaming(), dirs.system_profile_roaming()].iter().any(|root| root.join(r"RustDesk\config\RustDesk.toml").to_string_lossy().eq_ignore_ascii_case(&self.identity_path)){return Err("native installation identity path not approved".into());}
        if !self.original_msi.relative_path.ends_with(".msi")
            || !self.agent_target.relative_path.ends_with(".exe")
            || !self.client_target_manifest.relative_path.ends_with(".json")
        {
            return Err("native installation artifact type invalid".into());
        }
        Ok(())
    }
}

pub struct NativeInstaller {
    root: PathBuf,
    plan: InstallPlan,
    assets: NativeAssets,
    original_directory: DirectoryBackup,
    target_directory: DirectoryBackup,
    main: Contract,
    agent: Contract,
}

fn read(path: &Path, limit: u64) -> Result<Vec<u8>, String> {
    workspace::verify_file(path)?;
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .map_err(|e| e.to_string())?
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > limit {
        return Err("native install metadata too large".into());
    }
    Ok(bytes)
}

fn start_type(value: &str) -> Result<ServiceStartType, String> {
    match value {
        "auto" => Ok(ServiceStartType::AutoStart),
        "manual" => Ok(ServiceStartType::OnDemand),
        "disabled" => Ok(ServiceStartType::Disabled),
        _ => Err("native original start type invalid".into()),
    }
}

fn identity_matches(path: &Path, backup: &Path) -> Result<bool, String> {
    #[derive(Deserialize, PartialEq)]
    struct Identity {
        #[serde(default)]
        id: String,
        #[serde(default)]
        enc_id: String,
        key_pair: (Vec<u8>, Vec<u8>),
    }
    let parse = |path: &Path| -> Result<Identity, String> {
        let mut bytes = Vec::new();
        std::fs::File::open(path)
            .map_err(|e| e.to_string())?
            .take(65537)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        if bytes.len() > 65536 {
            return Err("identity config too large".into());
        }
        let text = std::str::from_utf8(&bytes).map_err(|e| e.to_string())?;
        hbb_common::toml::from_str(text).map_err(|_| "identity config malformed".into())
    };
    let old = parse(backup)?;
    let new = parse(path)?;
    Ok(old == new
        && old.key_pair.0.len() == 64
        && old.key_pair.1.len() == 32
        && (!old.id.is_empty() || !old.enc_id.is_empty()))
}

fn installed_product(code: &str) -> Result<bool, String> {
    #[link(name = "msi")]
    extern "system" {
        fn MsiQueryProductStateW(code: *const u16) -> i32;
    }
    let value = code.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
    match unsafe { MsiQueryProductStateW(value.as_ptr()) } {
        5 => Ok(true),
        -1 | 2 => Ok(false),
        other => Err(format!("MSI product state unconfirmed: {other}")),
    }
}

fn requires_original_preconditions(phase: super::transaction::Phase) -> bool {
    use super::transaction::Phase;
    matches!(phase, Phase::Planned | Phase::PackageVerified | Phase::Prepared | Phase::StoppingServices | Phase::ServicesStopped)
}

pub fn client_ready(executable: &Path, build: u64, running: bool) -> bool {
    readiness_probe(r"\\.\pipe\RustDesk\management-health-v1", executable, build, running)
}

pub fn remote_stop_preflight(plan: &InstallPlan) -> Result<(), String> {
    if plan.trigger != super::install_plan::Trigger::RemoteCommand {
        return Ok(());
    }
    remote_activity_probe(
        r"\\.\pipe\RustDesk\management-health-v2",
        &Path::new(&plan.normal_install_path).join("RustDesk.exe"),
        plan.original_build_seq,
    )
}

fn remote_activity_probe(pipe: &str, executable: &Path, build: u64) -> Result<(), String> {
    let response = super::health_windows::probe_blocking(pipe, executable, build, Duration::from_secs(3))
        .map_err(|_| "installation activity unconfirmed")?;
    match response.no_active_sessions {
        Some(true) if response.local_responsive => Ok(()),
        Some(false) => Err("installation active sessions".into()),
        _ => Err("installation activity unconfirmed".into()),
    }
}

fn readiness_probe(pipe: &str, executable: &Path, build: u64, running: bool) -> bool {
    running && executable.is_file() && build > 0
        && super::health_windows::probe_blocking(pipe, executable, build, Duration::from_secs(3))
            .map(|response| response.local_responsive).unwrap_or(false)
}

impl NativeInstaller {
    pub fn open(root: &Path) -> Result<Self, String> {
        workspace::verify(root)?;
        let plan = InstallPlan::decode(&read(&root.join("prepared.json"), 65536)?)?;
        let transaction = super::transaction::Transaction::decode(&read(&root.join("transaction.json"), 65536)?)?;
        plan.bind_transaction(&transaction)?;
        if !plan.target_key.contains("-msi-") {
            return Err("native installer currently supports MSI transaction only".into());
        }
        let assets: NativeAssets =
            serde_json::from_slice(&read(&root.join("native-assets.json"), 65536)?)
                .map_err(|_| "native assets malformed".to_owned())?;
        assets.validate()?;
        if assets.transaction_id != plan.transaction_id {
            return Err("native assets transaction mismatch".into());
        }
        if assets.schema == 2 {
            super::bootstrap_update::verify_prepared(root, &plan, &assets.agent_target)?;
        }
        for artifact in [
            &assets.client_target_manifest,
            &assets.agent_target,
            &assets.original_msi,
        ] {
            artifact.verify(root)?;
            workspace::verify_file(&root.join(&artifact.relative_path))?;
        }
        let original_package = super::msi_metadata::inspect(&root.join(&assets.original_msi.relative_path))?;
        let target_package = super::msi_metadata::inspect(&root.join(&plan.package.relative_path))?;
        super::msi_metadata::verify_pair(&original_package, &target_package,
            &assets.original_product_code, &assets.target_product_code)?;
        let original_directory = DirectoryBackup::decode(&read(
            &root.join(&plan.client_backup.relative_path),
            8 * 1024 * 1024,
        )?)?;
        original_directory.verify(&root.join("client-backup"))?;
        let target_directory = DirectoryBackup::decode(&read(
            &root.join(&assets.client_target_manifest.relative_path),
            8 * 1024 * 1024,
        )?)?;
        let edition = if plan.target_key.ends_with("-sos") {
            "sos"
        } else {
            "standard"
        };
        let main_snapshot = plan
            .services
            .iter()
            .find(|s| s.name == "RustDesk")
            .ok_or("native main contract missing")?;
        let agent_snapshot = plan
            .services
            .iter()
            .find(|s| s.name == "RustDeskManagementAgent")
            .ok_or("native agent contract missing")?;
        let main = Contract {
            name: "RustDesk".into(),
            executable: Path::new(&plan.normal_install_path).join("RustDesk.exe"),
            arguments: vec!["--service".into()],
            start_type: start_type(&main_snapshot.start_type)?,
        };
        let program_data = super::system_dirs::get().program_data.clone();
        let agent = Contract {
            name: "RustDeskManagementAgent".into(),
            executable: program_data.join("RustDesk").join("ManagementAgent").join("rustdesk-management-agent.exe"),
            arguments: vec![
                "--service".into(),
                "--edition".into(),
                edition.into(),
                "--package-kind".into(),
                "msi".into(),
                "--normal-install-path".into(),
                plan.normal_install_path.clone(),
            ],
            start_type: start_type(&agent_snapshot.start_type)?,
        };
        if !main.matches_command(&main_snapshot.command)
            || !agent.matches_command(&agent_snapshot.command)
        {
            return Err("native installation service command differs from preparation".into());
        }
        if requires_original_preconditions(transaction.phase) && !installed_product(&assets.original_product_code)? {
            return Err("native original MSI product not registered".into());
        }
        if requires_original_preconditions(transaction.phase) && !identity_matches(
            Path::new(&assets.identity_path),
            &root.join(&plan.identity_backup.relative_path),
        )? {
            return Err("native original identity differs from backup".into());
        }
        Ok(Self {
            root: root.into(),
            plan,
            assets,
            original_directory,
            target_directory,
            main,
            agent,
        })
    }

    fn installer(&self, package: &Artifact, name: &str) -> Result<InstallOutcome, String> {
        let manifest = hbb_common::UpdateTarget {
            size: package.size,
            sha256: package.sha256.clone(),
            ..Default::default()
        };
        let _locked = super::package_gate::lock_verified_file(
            &self.root.join(&package.relative_path),
            &manifest,
        )?;
        let tool = super::system_dirs::get().system32().join("msiexec.exe");
        let mut command = Command::new(tool);
        command.args(installer_process::msi_arguments(
            &self.root.join(&package.relative_path),
            &self.root.join(name),
        )?);
        let step = if name == "installer.log" { "install" } else { "rollback" };
        match installer_process::run_recorded(
            &mut command, Duration::from_secs(600), &self.root, step,
            &self.plan.transaction_id, &package.sha256,
        ) {
            Ok(outcome) => Ok(outcome),
            Err(error) if self.root.join(format!("{step}-started.json")).exists() => {
                // A durable launch intent means an installer may still own the
                // files. Unknown receipts must not trigger a competing rollback.
                hbb_common::log::warn!("installer outcome requires reconciliation: {error}");
                Ok(InstallOutcome::Pending { pid: 0, created_at: 0 })
            }
            Err(error) => Err(error),
        }
    }

    fn validation(&self, target: bool) -> Result<Validation, String> {
        let product = if target {
            &self.assets.target_product_code
        } else {
            &self.assets.original_product_code
        };
        let files = if target {
            &self.target_directory
        } else {
            &self.original_directory
        };
        let directory = Path::new(&self.plan.normal_install_path);
        let actual = files.matches_directory(directory)?;
        let agent_reference = if target {
            &self.assets.agent_target
        } else {
            &self.plan.agent_backup
        };
        let bytes = std::fs::read(&self.agent.executable).map_err(|e| e.to_string())?;
        let agent_match = bytes.len() as u64 == agent_reference.size
            && hex::encode(Sha256::digest(bytes)) == agent_reference.sha256;
        let identity = identity_matches(
            Path::new(&self.assets.identity_path),
            &self.root.join(&self.plan.identity_backup.relative_path),
        )?;
        let main = service_control::inspect(&self.main)?;
        let agent = service_control::inspect(&self.agent)?;
        let client_running = main.state == windows_service::service::ServiceState::Running;
        let agent_running = agent.state == windows_service::service::ServiceState::Running;
        Ok(Validation {
            target_matches: actual && agent_match && installed_product(product)?,
            identity_unchanged: identity,
            client_ready: client_ready(&self.main.executable, if target { self.plan.target_build_seq } else { self.plan.original_build_seq }, client_running),
            agent_ready: agent_running,
        })
    }
}

impl Backend for NativeInstaller {
    fn stop_services(&mut self) -> Result<(), String> {
        service_control::stop(&self.main, Duration::from_secs(90))?;
        service_control::stop(&self.agent, Duration::from_secs(90))
    }
    fn apply_package(&mut self) -> Result<InstallOutcome, String> {
        self.installer(&self.plan.package, "installer.log")
    }
    fn validate_target(&mut self) -> Result<Validation, String> {
        service_control::start(&self.agent, Duration::from_secs(60))?;
        let original = self
            .plan
            .services
            .iter()
            .find(|s| s.name == "RustDesk")
            .ok_or("main service snapshot missing")?;
        if original.running {
            service_control::start(&self.main, Duration::from_secs(60))?;
        }
        let validation = self.validation(true)?;
        if self.assets.schema == 2 && validation.target_matches && validation.identity_unchanged
            && validation.client_ready && validation.agent_ready {
            super::bootstrap_update::synchronize(&self.root, &self.plan, &self.assets.agent_target, true)?;
        }
        Ok(validation)
    }
    fn rollback_original(&mut self) -> Result<Validation, String> {
        service_control::stop(&self.main, Duration::from_secs(90))?;
        service_control::stop(&self.agent, Duration::from_secs(90))?;
        match self.installer(&self.assets.original_msi, "rollback-installer.log")? {
            InstallOutcome::Installed => {}
            other => return Err(format!("original MSI restore not complete: {other:?}")),
        }
        // MSI custom actions may restart the services. Quiesce them again before
        // replacing the restored directory and identity from verified backups.
        service_control::stop(&self.main, Duration::from_secs(90))?;
        service_control::stop(&self.agent, Duration::from_secs(90))?;
        self.original_directory.restore(
            &self.root.join("client-backup"),
            Path::new(&self.plan.normal_install_path),
        )?;
        file_restore::restore(&self.root, &self.plan.agent_backup, &self.agent.executable)?;
        file_restore::restore(
            &self.root,
            &self.plan.identity_backup,
            Path::new(&self.assets.identity_path),
        )?;
        if self.assets.schema == 2 {
            super::bootstrap_update::synchronize(&self.root, &self.plan, &self.assets.agent_target, false)?;
        }
        service_control::start(&self.agent, Duration::from_secs(60))?;
        if self
            .plan
            .services
            .iter()
            .any(|s| s.name == "RustDesk" && s.running)
        {
            service_control::start(&self.main, Duration::from_secs(60))?;
        }
        self.validation(false)
    }
}

#[cfg(test)]
mod readiness_tests {
    use super::*;

    #[hbb_common::tokio::test]
    async fn remote_stop_requires_fresh_idle_proof_and_rejects_busy_unknown_or_wrong_build() {
        use std::sync::{atomic::{AtomicBool, Ordering}, Arc};
        let pipe = format!(r"\\.\pipe\RustDeskAgentTestStopActivity-{}", std::process::id());
        let listener = super::super::health_windows::bind(&pipe).unwrap();
        let idle = Arc::new(AtomicBool::new(true));
        let observed_idle = Arc::clone(&idle);
        let server = hbb_common::tokio::spawn(super::super::health_windows::serve_activity(
            listener, "1.5.3".into(), 2026100502, || false,
            move || observed_idle.load(Ordering::SeqCst),
        ));
        let executable = std::env::current_exe().unwrap();
        hbb_common::tokio::task::spawn_blocking(move || {
            assert!(remote_activity_probe(&pipe, &executable, 2026100502).is_ok());
            idle.store(false, Ordering::SeqCst);
            assert_eq!(remote_activity_probe(&pipe, &executable, 2026100502).unwrap_err(), "installation active sessions");
            idle.store(true, Ordering::SeqCst);
            assert_eq!(remote_activity_probe(&pipe, &executable, 2026100503).unwrap_err(), "installation activity unconfirmed");
            assert_eq!(remote_activity_probe(&pipe, &executable.with_extension("foreign.exe"), 2026100502).unwrap_err(), "installation activity unconfirmed");
        }).await.unwrap();
        server.abort();
        let _ = server.await;

        let pipe = format!(r"\\.\pipe\RustDeskAgentTestStopLegacy-{}", std::process::id());
        let listener = super::super::health_windows::bind(&pipe).unwrap();
        let server = hbb_common::tokio::spawn(super::super::health_windows::serve(listener, "1.5.3".into(), 2026100502, || false));
        hbb_common::tokio::task::spawn_blocking(move || {
            assert_eq!(remote_activity_probe(&pipe, &std::env::current_exe().unwrap(), 2026100502).unwrap_err(), "installation activity unconfirmed");
        }).await.unwrap();
        server.abort();
        let _ = server.await;
    }

    #[test]
    fn recovery_after_apply_does_not_require_original_product_or_unchanged_current_identity() {
        use super::super::transaction::Phase;
        for phase in [Phase::Planned, Phase::PackageVerified, Phase::Prepared, Phase::StoppingServices, Phase::ServicesStopped] {
            assert!(requires_original_preconditions(phase));
        }
        for phase in [Phase::Applying, Phase::Validating, Phase::RollingBack, Phase::RolledBack, Phase::RollbackFailed, Phase::Committed] {
            assert!(!requires_original_preconditions(phase), "{phase:?} must load verified backups even after the original product or identity changed");
        }
    }

    #[hbb_common::tokio::test]
    async fn actual_pipe_proof_requires_matching_build_and_process_and_not_external_registration() {
        let pipe = format!(r"\\.\pipe\RustDeskAgentTestNativeReady-{}", std::process::id());
        let listener = super::super::health_windows::bind(&pipe).unwrap();
        let server = hbb_common::tokio::spawn(super::super::health_windows::serve(listener, "1.5.3".into(), 2026100502, || false));
        let executable = std::env::current_exe().unwrap();
        hbb_common::tokio::task::spawn_blocking(move || {
            assert!(readiness_probe(&pipe, &executable, 2026100502, true));
            assert!(!readiness_probe(&pipe, &executable, 2026100503, true));
            assert!(!readiness_probe(&pipe, &executable, 2026100502, false));
        }).await.unwrap();
        server.abort();
        let _ = server.await;
    }
}
