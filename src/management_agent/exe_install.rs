use super::{
    directory_backup::DirectoryBackup,
    exe_preparation::{self, Assets, Layout},
    file_restore,
    install_execution::{Backend, InstallOutcome, Validation},
    install_guard::InstallGuard,
    install_plan::{Artifact, InstallPlan},
    workspace,
};
use std::path::{Path, PathBuf};

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct StoppedState {
    schema: u32,
    transaction_id: String,
    installation_id: String,
    plan_sha256: String,
    package_sha256: String,
    identity_fingerprint: String,
    state: Artifact,
    maintenance_sha256: Option<String>,
}

pub(crate) fn verify_helper_before_stop(root: &Path) -> Result<(), String> {
    use super::{
        handoff::{Handoff, ObservedProcess, ReadyReceipt, ReleaseReceipt},
        resume_task,
        transaction::{Phase, Transaction},
    };
    use sha2::{Digest, Sha256};
    workspace::verify(root)?;
    let bytes = exe_preparation::read(&root.join("prepared.json"), 65536)?;
    let plan = InstallPlan::decode(&bytes)?;
    let tx = Transaction::decode(&exe_preparation::read(
        &root.join("transaction.json"),
        65536,
    )?)?;
    plan.bind_transaction(&tx)?;
    if !plan.target_key.starts_with("windows-x86_64-exe-") || tx.phase != Phase::StoppingServices {
        return Err("EXE helper transaction not authorized to stop".into());
    }
    plan.verify_assets(root)?;
    if std::env::current_exe().map_err(|e| e.to_string())? != root.join(&plan.helper.relative_path)
    {
        return Err("EXE helper outside independent runtime".into());
    }
    let binding = Handoff::decode(&exe_preparation::read(&root.join("handoff.json"), 4096)?)?;
    if binding.transaction_id != plan.transaction_id
        || binding.plan_sha256 != hex::encode(Sha256::digest(&bytes))
    {
        return Err("EXE helper plan binding mismatch".into());
    }
    let ready: ReadyReceipt = serde_json::from_slice(&exe_preparation::read(
        &root.join("helper-ready.json"),
        4096,
    )?)
    .map_err(|_| "EXE helper ready malformed")?;
    let release: ReleaseReceipt = serde_json::from_slice(&exe_preparation::read(
        &root.join("helper-release.json"),
        4096,
    )?)
    .map_err(|_| "EXE helper release malformed")?;
    let current = ObservedProcess::open(std::process::id())?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_secs();
    binding.verify_release(&ready, &release, current.owner(), now)?;
    resume_task::verify(
        &resume_task::read_back(&plan.transaction_id)?,
        &root.join(&plan.helper.relative_path),
        &plan.transaction_id,
    )
}

// Kept private to the EXE slice. No new platform contract is imposed on MSI callers.
pub(crate) trait Platform {
    fn preflight(&mut self) -> Result<(), String>;
    fn stop(&mut self) -> Result<(), String>;
    fn stopped(&mut self) -> Result<bool, String>;
    fn start_and_validate(&mut self, build: u64) -> Result<(bool, bool), String>;
    fn restore_services(&mut self, build: u64) -> Result<(bool, bool), String>;
}

pub(crate) struct ExeInstaller<P: Platform> {
    platform: P,
    root: PathBuf,
    layout: Layout,
    plan: InstallPlan,
    assets: Assets,
    stopped: bool,
    applied: bool,
    _guard: InstallGuard,
    _package: super::package_gate::LockedPackage,
    authority: Option<super::exe_runtime::Authority>,
    stopped_state: Option<Artifact>,
    stopped_record_lock: Option<std::fs::File>,
}

impl<P: Platform> ExeInstaller<P> {
    pub(crate) fn execute(
        &mut self,
        transaction: &mut super::transaction::Transaction,
        evidence: &super::transaction::Evidence,
        now: u64,
    ) -> Result<super::install_execution::ExecutionResult, String> {
        let path = self.root.join("transaction.json");
        self.plan.bind_transaction(transaction)?;
        self.plan.authorize_stop(
            &self.root,
            &self.root.join("prepared.json"),
            &path,
            transaction,
            evidence,
        )?;
        self.preflight()?;
        // Pre-stop refusal is not a failed installation. Only the mutating stage
        // enters the shared executor, whose errors require rollback.
        super::install_execution::execute(
            &path,
            transaction,
            evidence,
            &mut PreparedExecution(self),
            now,
        )
    }

    #[cfg(test)]
    pub fn open_fixture(root: &Path, platform: P) -> Result<Self, String> {
        let public = std::fs::read(root.join("fixture-public.bin")).map_err(|e| e.to_string())?;
        Self::open_checked(
            &root.join("transaction"),
            exe_preparation::fixture_layout(root)?,
            platform,
            Some(&public),
        )
    }

    pub(crate) fn open_checked(
        root: &Path,
        layout: Layout,
        platform: P,
        public: Option<&[u8]>,
    ) -> Result<Self, String> {
        let guard = InstallGuard::try_acquire()
            .map_err(|e| e.to_string())?
            .ok_or("EXE install busy")?;
        workspace::verify(root)?;
        let plan =
            InstallPlan::decode(&exe_preparation::read(&root.join("prepared.json"), 65536)?)?;
        if !matches!(
            plan.target_key.as_str(),
            "windows-x86_64-exe-standard" | "windows-x86_64-exe-sos"
        ) {
            return Err("EXE backend target kind mismatch".into());
        }
        plan.verify_assets(root)?;
        let package = exe_preparation::verify_bound(root, &plan, public)?;
        let assets: Assets = serde_json::from_slice(&exe_preparation::read(
            &root.join("exe-assets.json"),
            65536,
        )?)
        .map_err(|_| "EXE assets malformed")?;
        if plan.client_backup.relative_path != "exe-assets.json"
            || assets.schema != 3
            || assets.transaction_id != plan.transaction_id
        {
            return Err("EXE assets binding mismatch".into());
        }
        if assets.approved_manifest.relative_path != "approved-manifest.json" {
            return Err("EXE manifest reference invalid".into());
        }
        assets.approved_manifest.verify(root)?;
        for (asset, name) in [
            (&assets.client, "client-backup/manifest.json"),
            (&assets.state, "state-backup/manifest.json"),
            (&assets.config, "config-backup/manifest.json"),
            (&assets.target, "target/manifest.json"),
        ] {
            if asset.relative_path != name {
                return Err("EXE backup reference invalid".into());
            }
            asset.verify(root)?;
            let backup = DirectoryBackup::decode(&exe_preparation::read(
                &root.join(name),
                8 * 1024 * 1024,
            )?)?;
            backup.verify(root.join(name).parent().ok_or("EXE backup path invalid")?)?;
        }
        Ok(Self {
            platform,
            root: root.into(),
            layout,
            plan,
            assets,
            stopped: false,
            applied: false,
            _guard: guard,
            _package: package,
            authority: None,
            stopped_state: None,
            stopped_record_lock: None,
        })
    }

    fn snapshot(&self, artifact: &Artifact) -> Result<DirectoryBackup, String> {
        artifact.verify(&self.root)?;
        let snapshot = DirectoryBackup::decode(&exe_preparation::read(
            &self.root.join(&artifact.relative_path),
            8 * 1024 * 1024,
        )?)?;
        snapshot.verify(
            self.root
                .join(&artifact.relative_path)
                .parent()
                .ok_or("EXE snapshot path invalid")?,
        )?;
        Ok(snapshot)
    }

    fn target_agent(&self) -> Result<Artifact, String> {
        let target = self.snapshot(&self.assets.target)?;
        let file = target
            .files
            .iter()
            .find(|f| f.path == "rustdesk-management-agent.exe")
            .ok_or("EXE target agent missing")?;
        Ok(Artifact {
            relative_path: format!("target/files/{}", file.path),
            size: file.size,
            sha256: file.sha256.clone(),
        })
    }

    fn local(&self) -> Result<(), String> {
        if let Some(authority) = &self.authority {
            return authority.gates(&self.layout, &self.plan);
        }
        exe_preparation::gates(
            &self.layout,
            &self.plan.installation_id,
            if self.plan.target_key.ends_with("-sos") {
                "sos"
            } else {
                "standard"
            },
        )
    }

    fn identity_unchanged(&self) -> Result<bool, String> {
        self.snapshot(&self.assets.config)?
            .matches_directory(&self.layout.config)
    }

    pub(crate) fn preflight(&mut self) -> Result<(), String> {
        self.local()?;
        self.platform.preflight()?;
        if !self
            .snapshot(&self.assets.client)?
            .matches_directory(&self.layout.normal)?
            || !self.state_matches()?
            || !self.identity_unchanged()?
        {
            return Err("EXE original snapshot changed before stop".into());
        }
        self.plan.verify_assets(&self.root)
    }

    pub(crate) fn own(&mut self, authority: super::exe_runtime::Authority) {
        self.authority = Some(authority);
    }

    pub(crate) fn resume_rollback(
        &mut self,
        transaction: &mut super::transaction::Transaction,
        evidence: &super::transaction::Evidence,
        now: u64,
    ) -> Result<super::install_execution::ExecutionResult, String> {
        use super::transaction::Phase;
        self.plan.bind_transaction(transaction)?;
        if !matches!(
            transaction.phase,
            Phase::ServicesStopped | Phase::Applying | Phase::Validating | Phase::RollingBack
        ) {
            return Err("EXE resume not past mutation boundary".into());
        }
        let path = self.root.join("transaction.json");
        if super::transaction::Transaction::decode(&exe_preparation::read(&path, 65536)?)?
            != *transaction
        {
            return Err("EXE resume snapshot changed".into());
        }
        self.load_stopped_state()?;
        if transaction.phase != Phase::RollingBack {
            transaction.advance(Phase::RollingBack, evidence, now)?;
            transaction.save(&path)?;
        }
        super::install_execution::resume(&path, transaction, evidence, self, None, now)
    }

    fn state_matches(&self) -> Result<bool, String> {
        let snapshot = self.snapshot(&self.assets.state)?;
        let exact = match &self.authority {
            Some(authority) => authority.state_matches(&snapshot, &self.layout.state),
            None => snapshot.matches_directory(&self.layout.state),
        }?;
        if exact {
            return Ok(true);
        }
        self.compatible_live_state(&snapshot)
    }

    fn compatible_live_state(&self, snapshot: &DirectoryBackup) -> Result<bool, String> {
        use sha2::{Digest, Sha256};
        use std::io::Read;
        let mut expected = snapshot.clone();
        for entry in std::fs::read_dir(&self.layout.state).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| "EXE state filename invalid")?;
            let is_journal = name
                .strip_prefix("commands-")
                .and_then(|n| n.strip_suffix(".json"))
                .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()));
            if name != "agent.json" && !is_journal {
                continue;
            }
            let path = entry.path();
            workspace::verify_restore_target(&path, false)?;
            let limit = if name == "agent.json" {
                65536
            } else {
                64 * 1024 * 1024
            };
            let mut bytes = vec![];
            std::fs::File::open(&path)
                .map_err(|e| e.to_string())?
                .take(limit + 1)
                .read_to_end(&mut bytes)
                .map_err(|e| e.to_string())?;
            if bytes.len() as u64 > limit {
                return Err("EXE live state exceeds capacity".into());
            }
            let before = snapshot.files.iter().find(|f| f.path == name);
            let sha = hex::encode(Sha256::digest(&bytes));
            if before.is_some_and(|f| f.size == bytes.len() as u64 && f.sha256 == sha) {
                continue;
            }
            if is_journal {
                if super::journal::CommandJournal::open(&path).is_err() {
                    return Ok(false);
                }
            } else {
                let mut old: serde_json::Value = serde_json::from_slice(&exe_preparation::read(
                    &self.root.join("state-backup/files/agent.json"),
                    65536,
                )?)
                .map_err(|_| "EXE original state malformed")?;
                let mut live: serde_json::Value =
                    serde_json::from_slice(&bytes).map_err(|_| "EXE live state malformed")?;
                let old = old.as_object_mut().ok_or("EXE old state not object")?;
                let live = live.as_object_mut().ok_or("EXE live state not object")?;
                for key in [
                    "next_check_at",
                    "retry_stage",
                    "last_version",
                    "last_primary_url",
                    "last_error",
                    "last_policy_check_at",
                    "last_policy_error",
                    "service_recovery_failures",
                    "service_recovery_next_at",
                    "service_healthy_since",
                    "last_service_recovery_at",
                    "service_recovery_error",
                ] {
                    if let Some(v) = live.get(key) {
                        let valid = match key {
                            "retry_stage" | "service_recovery_failures" => {
                                v.as_u64().is_some_and(|n| n <= 255)
                            }
                            "service_recovery_next_at" => v.is_u64(),
                            "service_healthy_since" => v.is_null() || v.is_u64(),
                            "next_check_at"
                            | "last_policy_check_at"
                            | "last_service_recovery_at" => v.is_null() || v.is_string(),
                            _ => v.is_string(),
                        };
                        if !valid {
                            return Err("EXE live observation field invalid".into());
                        }
                    }
                    old.remove(key);
                    live.remove(key);
                }
                if old != live {
                    return Ok(false);
                }
            }
            expected.files.retain(|f| f.path != name);
            expected.files.push(super::directory_backup::BackupFile {
                path: name,
                size: bytes.len() as u64,
                sha256: sha,
            });
        }
        expected.files.sort_by(|a, b| a.path.cmp(&b.path));
        match &self.authority {
            Some(a) => a.state_matches(&expected, &self.layout.state),
            None => expected.matches_directory(&self.layout.state),
        }
    }

    fn replace_normal(&self) -> Result<(), String> {
        let target = self.snapshot(&self.assets.target)?;
        let parent = self
            .layout
            .normal
            .parent()
            .ok_or("EXE normal parent missing")?;
        let staging = parent.join(format!("RustDesk.exe-staging-{}", self.plan.transaction_id));
        let displaced = parent.join(format!(
            "RustDesk.exe-original-{}",
            self.plan.transaction_id
        ));
        if staging.exists() || displaced.exists() {
            return Err("EXE normal swap already exists; reconciliation required".into());
        }
        std::fs::create_dir(&staging).map_err(|e| e.to_string())?;
        for d in &target.directories {
            std::fs::create_dir_all(staging.join(d)).map_err(|e| e.to_string())?;
        }
        for file in &target.files {
            let path = staging.join(&file.path);
            std::fs::copy(self.root.join("target/files").join(&file.path), &path)
                .map_err(|e| e.to_string())?;
            std::fs::OpenOptions::new()
                .write(true)
                .open(&path)
                .and_then(|f| f.sync_all())
                .map_err(|e| e.to_string())?;
        }
        if !target.matches_directory(&staging)? {
            return Err("EXE staging integrity mismatch".into());
        }
        std::fs::rename(&self.layout.normal, &displaced)
            .map_err(|e| format!("EXE old directory move: {e}"))?;
        if let Err(error) = std::fs::rename(&staging, &self.layout.normal) {
            std::fs::rename(&displaced, &self.layout.normal).map_err(|restore| {
                format!("EXE publication: {error}; old directory restore: {restore}")
            })?;
            return Err(format!("EXE publication: {error}"));
        }
        Ok(())
    }

    fn restore_normal(&self) -> Result<(), String> {
        let original = self.snapshot(&self.assets.client)?;
        let parent = self
            .layout
            .normal
            .parent()
            .ok_or("EXE normal parent missing")?;
        let displaced = parent.join(format!(
            "RustDesk.exe-original-{}",
            self.plan.transaction_id
        ));
        if !displaced.exists() {
            if original.matches_directory(&self.layout.normal)? {
                return Ok(());
            }
            return Err("EXE old directory unavailable; reconcile".into());
        }
        if !original.matches_directory(&displaced)? {
            return Err("EXE retained original directory changed".into());
        }
        let failed = parent.join(format!("RustDesk.exe-failed-{}", self.plan.transaction_id));
        if failed.exists() {
            return Err("EXE failed directory occupied".into());
        }
        std::fs::rename(&self.layout.normal, &failed).map_err(|e| e.to_string())?;
        if let Err(error) = std::fs::rename(&displaced, &self.layout.normal) {
            std::fs::rename(&failed, &self.layout.normal)
                .map_err(|e| format!("EXE restore {error}; revert {e}"))?;
            return Err(error.to_string());
        }
        if !original.matches_directory(&self.layout.normal)? {
            return Err("EXE restored normal integrity failed".into());
        }
        Ok(())
    }

    fn stop_prechecked(&mut self) -> Result<(), String> {
        self.platform.stop()?;
        if !self.platform.stopped()? {
            return Err("EXE services not confirmed stopped".into());
        }
        self.stopped = true;
        self.capture_stopped_state()?;
        Ok(())
    }

    fn unchanged_security(&self) -> Result<(), String> {
        self.local()?;
        if !self.identity_unchanged()? {
            return Err(
                "EXE config/identity changed at stop boundary; preserve current data".into(),
            );
        }
        let before: serde_json::Value = serde_json::from_slice(&exe_preparation::read(
            &self.root.join("state-backup/files/agent.json"),
            65536,
        )?)
        .map_err(|_| "EXE old state malformed")?;
        let path = self.layout.state.join("agent.json");
        workspace::verify_restore_target(&path, false)?;
        let after: serde_json::Value =
            serde_json::from_slice(&std::fs::read(path).map_err(|e| e.to_string())?)
                .map_err(|_| "EXE stopped state malformed")?;
        for key in [
            "schema_version",
            "agent_installation_id",
            "edition",
            "package_kind",
            "target_key",
            "normal_install_path",
            "manifest_url",
            "device_id",
            "device_uuid",
            "public_key",
            "private_key",
            "device_secret",
            "enrollment_generation",
        ] {
            if before.get(key) != after.get(key) {
                return Err(format!("EXE stopped state identity field changed: {key}"));
            }
        }
        Ok(())
    }

    fn capture_stopped_state(&mut self) -> Result<(), String> {
        use sha2::{Digest, Sha256};
        if !self.platform.stopped()? {
            return Err("EXE stop-state capture requires confirmed exit".into());
        }
        if self.stopped_state.is_some() {
            return Ok(());
        }
        self.unchanged_security()?;
        let record_path = self.root.join("stopped-state.json");
        if record_path.exists() {
            return self.load_stopped_state();
        }
        let destination = self.root.join("stopped-state");
        let snapshot = super::directory_backup::capture(&self.layout.state, &destination)?;
        for file in &snapshot.files {
            workspace::verify_file(&destination.join("files").join(&file.path))?;
        }
        self.unchanged_security()?;
        if !snapshot.matches_directory(&self.layout.state)? || !self.platform.stopped()? {
            return Err("EXE stopped state changed during capture".into());
        }
        let state = exe_preparation::describe(&self.root, "stopped-state/manifest.json")?;
        let record = StoppedState {
            schema: 1,
            transaction_id: self.plan.transaction_id.clone(),
            installation_id: self.plan.installation_id.clone(),
            plan_sha256: hex::encode(Sha256::digest(exe_preparation::read(
                &self.root.join("prepared.json"),
                65536,
            )?)),
            package_sha256: self.plan.package.sha256.clone(),
            identity_fingerprint: self.plan.identity_fingerprint.clone(),
            state,
            maintenance_sha256: self.authority.as_ref().map(|a| a.marker_digest()),
        };
        super::state_store::publish(
            &record_path,
            &serde_json::to_vec(&record).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        self.load_stopped_state()
    }

    fn load_stopped_state(&mut self) -> Result<(), String> {
        use sha2::{Digest, Sha256};
        use std::os::windows::fs::OpenOptionsExt;
        let path = self.root.join("stopped-state.json");
        workspace::verify_file(&path)?;
        let lock = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(1)
            .open(&path)
            .map_err(|e| e.to_string())?;
        let record: StoppedState = serde_json::from_slice(&exe_preparation::read(&path, 65536)?)
            .map_err(|_| "EXE stopped state receipt malformed")?;
        if record.schema != 1
            || record.transaction_id != self.plan.transaction_id
            || record.installation_id != self.plan.installation_id
            || record.package_sha256 != self.plan.package.sha256
            || record.identity_fingerprint != self.plan.identity_fingerprint
            || record.plan_sha256
                != hex::encode(Sha256::digest(exe_preparation::read(
                    &self.root.join("prepared.json"),
                    65536,
                )?))
            || record.state.relative_path != "stopped-state/manifest.json"
            || record.maintenance_sha256 != self.authority.as_ref().map(|a| a.marker_digest())
        {
            return Err("EXE stopped state receipt binding mismatch".into());
        }
        self.snapshot(&record.state)?;
        self.stopped_state = Some(record.state);
        self.stopped_record_lock = Some(lock);
        Ok(())
    }
}

struct PreparedExecution<'a, P: Platform>(&'a mut ExeInstaller<P>);

impl<P: Platform> Backend for PreparedExecution<'_, P> {
    fn stop_services(&mut self) -> Result<(), String> {
        self.0.stop_prechecked()
    }
    fn apply_package(&mut self) -> Result<InstallOutcome, String> {
        self.0.apply_package()
    }
    fn validate_target(&mut self) -> Result<Validation, String> {
        self.0.validate_target()
    }
    fn rollback_original(&mut self) -> Result<Validation, String> {
        self.0.rollback_original()
    }
}

// No production constructor/dispatch until independent helper/task handoff and native
// schema readiness are wired. The fixture exercises the same file transaction backend.
impl<P: Platform> Backend for ExeInstaller<P> {
    fn stop_services(&mut self) -> Result<(), String> {
        self.preflight()?;
        self.stop_prechecked()
    }
    fn apply_package(&mut self) -> Result<InstallOutcome, String> {
        if !self.stopped || !self.platform.stopped()? {
            return Err("EXE services not confirmed stopped".into());
        }
        self.local()?;
        let state = self
            .stopped_state
            .as_ref()
            .ok_or("EXE stopped state receipt missing")?;
        if !self
            .snapshot(state)?
            .matches_directory(&self.layout.state)?
        {
            return Err("EXE stopped state changed before replacement".into());
        }
        self.replace_normal()?;
        file_restore::restore(&self.root, &self.target_agent()?, &self.layout.agent)?;
        exe_preparation::synchronize_bootstrap(&self.root, &self.plan, &self.assets, true)?;
        self.applied = true;
        Ok(InstallOutcome::Installed)
    }
    fn validate_target(&mut self) -> Result<Validation, String> {
        if !self.applied {
            return Err("EXE target not applied".into());
        }
        let matches = self
            .snapshot(&self.assets.target)?
            .matches_directory(&self.layout.normal)?;
        let agent = self.target_agent()?;
        let target = hbb_common::UpdateTarget {
            size: agent.size,
            sha256: agent.sha256,
            ..Default::default()
        };
        let _lock = super::package_gate::lock_verified_file(&self.layout.agent, &target)?;
        self.local()?;
        let identity = self.identity_unchanged()?;
        if !matches || !identity {
            return Err("EXE target or identity validation failed".into());
        }
        let (client_ready, agent_ready) = self
            .platform
            .start_and_validate(self.plan.target_build_seq)?;
        self.local()?;
        Ok(Validation {
            target_matches: matches,
            identity_unchanged: self.identity_unchanged()?,
            client_ready,
            agent_ready,
        })
    }
    fn rollback_original(&mut self) -> Result<Validation, String> {
        if let Some(authority) = &self.authority {
            authority.intent(&self.layout)?;
        } else {
            exe_preparation::local_intent(&self.layout)?;
        }
        self.platform.stop()?;
        if !self.platform.stopped()? {
            return Err("EXE rollback services not confirmed stopped".into());
        }
        if self.stopped_state.is_none() {
            if self.root.join("stopped-state.json").exists() {
                self.load_stopped_state()?;
            } else {
                let tx = super::transaction::Transaction::decode(&exe_preparation::read(
                    &self.root.join("transaction.json"),
                    65536,
                )?)?;
                if self.applied
                    || !matches!(
                        tx.phase,
                        super::transaction::Phase::PackageVerified
                            | super::transaction::Phase::StoppingServices
                            | super::transaction::Phase::RollingBack
                    )
                {
                    return Err("EXE stopped state missing after mutation; reconcile without overwriting state".into());
                }
                self.capture_stopped_state()?;
            }
        }
        self.restore_normal()?;
        file_restore::restore(&self.root, &self.plan.agent_backup, &self.layout.agent)?;
        exe_preparation::synchronize_bootstrap(&self.root, &self.plan, &self.assets, false)?;
        let reference = self
            .stopped_state
            .as_ref()
            .ok_or("EXE rollback stop-state unavailable")?;
        let state = self.snapshot(reference)?;
        if let Some(authority) = &self.authority {
            authority.restore_state(&self.root, &state, &self.layout.state)?;
        } else {
            state.restore(&self.root.join("stopped-state"), &self.layout.state)?;
        }
        self.snapshot(&self.assets.config)?
            .restore(&self.root.join("config-backup"), &self.layout.config)?;
        let (client_ready, agent_ready) = self
            .platform
            .restore_services(self.plan.original_build_seq)?;
        self.local()?;
        Ok(Validation {
            target_matches: self
                .snapshot(&self.assets.client)?
                .matches_directory(&self.layout.normal)?,
            identity_unchanged: self.identity_unchanged()?,
            client_ready,
            agent_ready,
        })
    }
}
