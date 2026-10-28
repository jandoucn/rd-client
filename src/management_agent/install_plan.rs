use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    io::Read,
    path::{Component, Path},
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Trigger {
    LocalRecovery,
    RemoteCommand,
    LegacyMigration,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Artifact {
    pub relative_path: String,
    pub size: u64,
    pub sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OriginalService {
    pub name: String,
    pub command: String,
    pub account: String,
    pub start_type: String,
    pub running: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstallPlan {
    pub schema: u32,
    pub transaction_id: String,
    pub trigger: Trigger,
    pub command_id: Option<String>,
    pub installation_id: String,
    pub identity_fingerprint: String,
    pub target_key: String,
    pub original_version: String,
    pub original_build_seq: u64,
    pub target_version: String,
    pub target_build_seq: u64,
    pub normal_install_path: String,
    pub package: Artifact,
    pub client_backup: Artifact,
    pub agent_backup: Artifact,
    pub identity_backup: Artifact,
    pub helper: Artifact,
    pub helper_version: String,
    pub resume_task: String,
    pub created_at: u64,
    pub services: Vec<OriginalService>,
}

fn token(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

impl Artifact {
    fn validate(&self) -> Result<(), String> {
        let path = Path::new(&self.relative_path);
        if self.relative_path.is_empty()
            || self.relative_path.contains(['\\', ':'])
            || self.relative_path.chars().any(char::is_control)
            || self.relative_path.split('/').any(|part| {
                part.is_empty() || part == "." || part == ".." || part.ends_with(['.', ' '])
            })
            || !path
                .components()
                .all(|part| matches!(part, Component::Normal(_)))
            || self.size == 0
            || !token(&self.sha256, 64)
        {
            return Err("prepared artifact reference invalid".into());
        }
        Ok(())
    }

    pub fn verify(&self, root: &Path) -> Result<(), String> {
        self.validate()?;
        if !root.is_absolute() {
            return Err("prepared artifact root not absolute".into());
        }
        let path = root.join(&self.relative_path);
        for ancestor in path.ancestors() {
            let metadata = std::fs::symlink_metadata(ancestor)
                .map_err(|e| format!("artifact path unavailable: {e}"))?;
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                if metadata.file_attributes() & 0x400 != 0 {
                    return Err("artifact reparse path refused".into());
                }
            }
            if metadata.file_type().is_symlink() {
                return Err("artifact symlink refused".into());
            }
        }
        let mut file = std::fs::File::open(&path).map_err(|e| format!("artifact open: {e}"))?;
        let metadata = file.metadata().map_err(|e| e.to_string())?;
        if !metadata.is_file() || metadata.len() != self.size {
            return Err("artifact size mismatch".into());
        }
        let mut hash = Sha256::new();
        let mut total = 0u64;
        let mut buffer = [0u8; 65536];
        loop {
            let count = file
                .read(&mut buffer)
                .map_err(|e| format!("artifact read: {e}"))?;
            if count == 0 {
                break;
            }
            total = total.saturating_add(count as u64);
            if total > self.size {
                return Err("artifact size changed".into());
            }
            hash.update(&buffer[..count]);
        }
        if total != self.size {
            return Err("artifact size changed".into());
        }
        if hex::encode(hash.finalize()) != self.sha256 {
            return Err("artifact hash mismatch".into());
        }
        Ok(())
    }
}

impl InstallPlan {
    #[cfg(windows)]
    pub fn authorize_stop(
        &self,
        root: &Path,
        preparation_path: &Path,
        transaction_path: &Path,
        transaction: &super::transaction::Transaction,
        evidence: &super::transaction::Evidence,
    ) -> Result<(), String> {
        self.bind_transaction(transaction)?;
        let bytes = std::fs::read(preparation_path)
            .map_err(|e| format!("installation preparation read: {e}"))?;
        let persisted = Self::decode(&bytes)?;
        if persisted != *self {
            return Err("installation preparation snapshot mismatch".into());
        }
        self.verify_assets(root)?;
        if !transaction.may_stop_services(transaction_path, evidence) {
            return Err("installation stop boundary not durably prepared".into());
        }
        Ok(())
    }

    pub fn verify_assets(&self, root: &Path) -> Result<(), String> {
        self.validate()?;
        for artifact in [
            &self.package,
            &self.client_backup,
            &self.agent_backup,
            &self.identity_backup,
            &self.helper,
        ] {
            artifact.verify(root)?;
        }
        Ok(())
    }

    pub fn bind_transaction(
        &self,
        transaction: &super::transaction::Transaction,
    ) -> Result<(), String> {
        self.validate()?;
        if self.transaction_id != transaction.id
            || self.target_key != transaction.target_key
            || self.package.sha256 != transaction.package_sha256
            || self.identity_fingerprint != transaction.identity_fingerprint
            || self.created_at != transaction.created_at
        {
            return Err("installation preparation transaction binding mismatch".into());
        }
        Ok(())
    }

    #[cfg(windows)]
    pub fn save_bound(
        &self,
        path: &Path,
        transaction: &super::transaction::Transaction,
    ) -> Result<(), String> {
        use std::os::windows::fs::OpenOptionsExt;
        self.bind_transaction(transaction)?;
        let _writer = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .share_mode(0)
            .open(path.with_extension("lock"))
            .map_err(|e| format!("preparation writer unavailable: {e}"))?;
        match std::fs::read(path) {
            Ok(bytes) => {
                let existing = Self::decode(&bytes)?;
                if existing != *self {
                    return Err("installation preparation immutable conflict".into());
                }
                existing.bind_transaction(transaction)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let bytes = serde_json::to_vec(self).map_err(|e| e.to_string())?;
                super::state_store::publish(path, &bytes)
                    .map_err(|e| format!("installation preparation publish: {e}"))
            }
            Err(error) => Err(format!("installation preparation unavailable: {error}")),
        }
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() > 65536 {
            return Err("installation preparation too large".into());
        }
        let plan: Self = serde_json::from_slice(bytes)
            .map_err(|_| "installation preparation malformed".to_owned())?;
        plan.validate()?;
        Ok(plan)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.schema != 1
            || !token(&self.transaction_id, 32)
            || !token(&self.installation_id, 32)
            || !token(&self.identity_fingerprint, 64)
            || self.created_at == 0
        {
            return Err("installation preparation identity invalid".into());
        }
        match (&self.trigger, &self.command_id) {
            (Trigger::RemoteCommand, Some(id)) if token(id, 32) => {}
            (Trigger::LocalRecovery | Trigger::LegacyMigration, None) => {}
            _ => return Err("installation command binding invalid".into()),
        }
        let (kind, edition) = match self.target_key.as_str() {
            "windows-x86_64-msi-standard" => ("msi", "standard"),
            "windows-x86_64-msi-sos" => ("msi", "sos"),
            "windows-x86_64-exe-standard" => ("exe", "standard"),
            "windows-x86_64-exe-sos" => ("exe", "sos"),
            _ => return Err("installation target invalid".into()),
        };
        for version in [
            &self.original_version,
            &self.target_version,
            &self.helper_version,
        ] {
            if version.is_empty()
                || version.len() > 64
                || !version
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b".-+".contains(&b))
            {
                return Err("installation version invalid".into());
            }
        }
        if self.original_build_seq == 0 || self.target_build_seq < self.original_build_seq {
            return Err("installation downgrade refused".into());
        }
        if self.normal_install_path != super::system_dirs::get().normal_install().to_string_lossy() {
            return Err("installation normal path not approved".into());
        }
        if self.resume_task != format!("RustDeskManagementAgentResume-{}", self.transaction_id) {
            return Err("installation resume task not bound".into());
        }
        let artifacts = [
            &self.package,
            &self.client_backup,
            &self.agent_backup,
            &self.identity_backup,
            &self.helper,
        ];
        for (index, artifact) in artifacts.iter().enumerate() {
            artifact.validate()?;
            if artifacts[..index].iter().any(|other| {
                other
                    .relative_path
                    .eq_ignore_ascii_case(&artifact.relative_path)
            }) {
                return Err("installation artifact references overlap".into());
            }
        }
        if !self.package.relative_path.ends_with(&format!(".{kind}"))
            || !self.helper.relative_path.ends_with(".exe")
        {
            return Err("installation package/helper type mismatch".into());
        }
        if self.services.len() != 2 {
            return Err("installation service snapshot incomplete".into());
        }
        let main = self
            .services
            .iter()
            .find(|s| s.name == "RustDesk")
            .ok_or("main service snapshot missing")?;
        let agent = self
            .services
            .iter()
            .find(|s| s.name == "RustDeskManagementAgent")
            .ok_or("agent service snapshot missing")?;
        for service in [main, agent] {
            if service.account != "LocalSystem"
                || !["auto", "manual", "disabled"].contains(&service.start_type.as_str())
            {
                return Err("installation original service contract invalid".into());
            }
        }
        let main_command = format!("\"{}\\RustDesk.exe\" --service", self.normal_install_path);
        let agent_args = format!(
            " --service --edition {edition} --package-kind {kind} --normal-install-path \"{}\"",
            self.normal_install_path
        );
        let agent_path = super::system_dirs::get()
            .agent()
            .join("rustdesk-management-agent.exe")
            .to_string_lossy()
            .into_owned();
        if !main.command.eq_ignore_ascii_case(&main_command)
            || !(agent
                .command
                .eq_ignore_ascii_case(&format!("\"{agent_path}\"{agent_args}"))
                || agent
                    .command
                    .eq_ignore_ascii_case(&format!("{agent_path}{agent_args}")))
        {
            return Err("installation original service target mismatch".into());
        }
        Ok(())
    }
}
