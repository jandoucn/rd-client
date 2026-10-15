use super::install_plan::Artifact;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    io::Read,
    path::{Path, PathBuf},
};

#[path = "local_repair_total_loss.rs"]
pub mod total_loss;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApprovalSource {
    pub source_kind: String,
    pub source_id: String,
    pub source_reference: String,
    pub installation_receipt: Artifact,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InstallationReceipt {
    schema: u32,
    source_kind: String,
    source_id: String,
    installation_id: String,
    target_key: String,
    source_reference: String,
    build_seq: u64,
    product_code: String,
    registered_source: PathBuf,
    package: Artifact,
    identity_sha256: String,
    client_pid: u32,
    agent_pid: u32,
    observed_at: u64,
}

fn bounded_read(path: &Path, limit: u64) -> Result<Vec<u8>, String> {
    #[cfg(windows)]
    use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
    for ancestor in path.ancestors() {
        let metadata = std::fs::symlink_metadata(ancestor).map_err(|e| e.to_string())?;
        if metadata.file_type().is_symlink() {
            return Err("local repair source symlink refused".into());
        }
        #[cfg(windows)]
        if metadata.file_attributes() & 0x400 != 0 {
            return Err("local repair source reparse refused".into());
        }
    }
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    options.share_mode(1).custom_flags(0x00200000);
    let mut bytes = Vec::new();
    options
        .open(path)
        .map_err(|e| e.to_string())?
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.is_empty() || bytes.len() as u64 > limit {
        return Err("local repair source size invalid".into());
    }
    Ok(bytes)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocalRepairPlan {
    pub schema: u32,
    pub trigger: String,
    pub transaction_id: String,
    pub installation_id: String,
    pub target_key: String,
    pub build_seq: u64,
    pub version: String,
    pub original_normal: String,
    pub fallback_path: String,
    pub fallback_command: String,
    pub fallback_start: String,
    pub agent_command: String,
    pub agent_start: String,
    pub identity_path: String,
    pub product_code: String,
    pub source_transaction: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approval_source: Option<ApprovalSource>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_loss: Option<total_loss::Prestate>,
    pub created_at: u64,
    pub package: Artifact,
    pub client_inventory: Artifact,
    pub agent_backup: Artifact,
    pub identity_backup: Artifact,
    pub config_backup: Artifact,
    pub bootstrap_backup: Artifact,
    pub bootstrap_manifest: Artifact,
    pub helper: Artifact,
}

impl LocalRepairPlan {
    #[cfg(test)]
    pub fn approval_source_fixture(
        bytes: &[u8],
        execution_root: &std::path::Path,
        live_identity: &[u8],
        public: &[u8],
    ) -> Result<(), String> {
        let plan = Self::decode(bytes)?;
        plan.verify_approval_source(execution_root, execution_root, live_identity, Some(public))
    }

    pub fn approval_id(&self) -> &str {
        self.approval_source
            .as_ref()
            .map(|source| source.source_id.as_str())
            .unwrap_or(&self.source_transaction)
    }

    pub fn verify_approval_source(
        &self,
        execution_root: &Path,
        state: &Path,
        live_identity: &[u8],
        public: Option<&[u8]>,
    ) -> Result<(), String> {
        self.check_approval_source(execution_root, state, live_identity, public, true)
    }

    pub fn preflight_approval_source(
        &self,
        execution_root: &Path,
        state: &Path,
        live_identity: &[u8],
        public: Option<&[u8]>,
    ) -> Result<(), String> {
        self.check_approval_source(execution_root, state, live_identity, public, false)
    }

    fn check_approval_source(
        &self,
        execution_root: &Path,
        state: &Path,
        live_identity: &[u8],
        public: Option<&[u8]>,
        require_transaction: bool,
    ) -> Result<(), String> {
        self.validate()?;
        let Some(source) = &self.approval_source else {
            return Ok(());
        };
        for marker in ["disable-agent", "disable-recovery"] {
            match std::fs::symlink_metadata(state.join(marker)) {
                Ok(_) => return Err("local repair locally blocked".into()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => return Err("local repair local gate unreadable".into()),
            }
        }
        if require_transaction {
            let transaction_path = execution_root.join("transaction.json");
            match std::fs::symlink_metadata(&transaction_path) {
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    return Err("local repair execution transaction missing".into())
                }
                Err(_) => return Err("local repair execution transaction unreadable".into()),
                Ok(_) => {}
            }
            let transaction =
                super::transaction::Transaction::decode(&bounded_read(&transaction_path, 65536)?)?;
            self.bind(&transaction)?;
        }
        let receipt_bytes = bounded_read(&execution_root.join("installation-receipt.json"), 65536)?;
        if receipt_bytes.len() as u64 != source.installation_receipt.size
            || hex::encode(Sha256::digest(&receipt_bytes)) != source.installation_receipt.sha256
        {
            return Err("local repair receipt digest mismatch".into());
        }
        let receipt: InstallationReceipt =
            serde_json::from_slice(&receipt_bytes).map_err(|_| "local repair receipt malformed")?;
        let source_id = hex::encode(Sha256::digest(format!(
            "installed-msi-v1\n{}\n{}\n{}",
            receipt.installation_id, receipt.source_reference, receipt.identity_sha256
        )));
        if receipt.schema != 1
            || receipt.source_kind != "installed_msi"
            || receipt.source_id != source.source_id
            || source_id != source.source_id
            || receipt.source_reference != source.source_reference
            || receipt.installation_id != self.installation_id
            || receipt.target_key != self.target_key
            || receipt.build_seq != self.build_seq
            || receipt.product_code != self.product_code
            || receipt.package != self.package
            || receipt.observed_at == 0
            || receipt.observed_at > self.created_at
            || receipt.client_pid == 0
            || receipt.agent_pid == 0
            || !receipt.registered_source.is_absolute()
            || receipt
                .registered_source
                .components()
                .any(|p| matches!(p, std::path::Component::ParentDir))
        {
            return Err("local repair receipt binding mismatch".into());
        }
        if hex::encode(Sha256::digest(live_identity)) != receipt.identity_sha256
            || self.identity_backup.sha256 != receipt.identity_sha256
            || bounded_read(
                &execution_root.join(&self.identity_backup.relative_path),
                65536,
            )? != live_identity
        {
            return Err("local repair identity mismatch".into());
        }
        let raw: serde_json::Value = serde_json::from_slice(&bounded_read(
            &execution_root.join("approved-manifest.json"),
            262144,
        )?)
        .map_err(|_| "local repair approval manifest malformed")?;
        if super::package_gate::package_reference(&raw, &self.target_key)?
            != source.source_reference
        {
            return Err("local repair approval reference mismatch".into());
        }
        let manifest: hbb_common::UpdateManifest =
            serde_json::from_value(raw).map_err(|_| "local repair approval manifest malformed")?;
        let target = match public {
            #[cfg(test)]
            Some(public) => super::package_gate::verify_with_key(
                &manifest,
                &self.target_key,
                self.build_seq,
                "fixture-key",
                public,
            )?,
            _ => super::package_gate::verify(&manifest, &self.target_key, self.build_seq)?,
        };
        let (version, build, _) =
            base::update::update_target_release(&manifest, target).map_err(|e| e.to_string())?;
        if version != self.version
            || build != self.build_seq
            || target.sha256 != self.package.sha256
            || target.size != self.package.size
            || super::package_gate::cache_name(&target.primary, "msi")?
                != self.package.relative_path
        {
            return Err("local repair signed source mismatch".into());
        }
        super::package_gate::lock_verified_file(
            &execution_root.join(&self.package.relative_path),
            target,
        )
        .map_err(|_| "local repair package integrity mismatch")?;
        Ok(())
    }

    pub fn validate(&self) -> Result<(), String> {
        if let Some(prestate) = &self.total_loss {
            prestate.validate(&self.fallback_command)?;
        }
        let token = |v: &str, n| {
            v.len() == n
                && v.bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        };
        let edition = match self.target_key.as_str() {
            "windows-x86_64-msi-sos" => "sos",
            "windows-x86_64-msi-standard" => "standard",
            _ => return Err("local repair target invalid".into()),
        };
        match &self.approval_source {
            None if token(&self.source_transaction, 32) => {}
            Some(_) if !self.source_transaction.is_empty() => {
                return Err("local repair mixed approval source".into())
            }
            Some(source)
                if source.source_kind == "installed_msi"
                    && token(&source.source_id, 64)
                    && token(&source.source_reference, 64)
                    && source.installation_receipt.relative_path == "installation-receipt.json"
                    && source.installation_receipt.size > 0
                    && source.installation_receipt.size <= 65536
                    && token(&source.installation_receipt.sha256, 64) => {}
            _ => return Err("local repair approval source invalid".into()),
        }
        if self.schema != 1
            || self.trigger != "local_recovery"
            || !token(&self.transaction_id, 32)
            || !token(&self.installation_id, 32)
            || self.build_seq == 0
            || self.created_at == 0
            || self.version.is_empty()
            || self.original_normal != "missing"
            || self.fallback_path != format!(r"C:\ProgramData\RustDesk\Client\{edition}\RustDesk")
            || self.fallback_command
                != format!("\"{}\\RustDesk.exe\" --service", self.fallback_path)
            || !["auto", "manual"].contains(&self.fallback_start.as_str())
            || self.agent_start != "auto"
        {
            return Err("local repair fixed source contract invalid".into());
        }
        let agent = format!(
            r#"C:\ProgramData\RustDesk\ManagementAgent\rustdesk-management-agent.exe --service --edition {edition} --package-kind msi --normal-install-path "C:\Program Files\RustDesk""#
        );
        let quoted = format!(
            r#""C:\ProgramData\RustDesk\ManagementAgent\rustdesk-management-agent.exe" --service --edition {edition} --package-kind msi --normal-install-path "C:\Program Files\RustDesk""#
        );
        if self.agent_command != agent && self.agent_command != quoted {
            return Err("local repair agent contract invalid".into());
        }
        if ![r"C:\Windows\ServiceProfiles\LocalService\AppData\Roaming\RustDesk\config\RustDesk.toml",r"C:\Windows\System32\config\systemprofile\AppData\Roaming\RustDesk\config\RustDesk.toml"].contains(&self.identity_path.as_str()){
            return Err("local repair identity path invalid".into());
        }
        if self.product_code.len() != 38
            || !self.product_code.starts_with('{')
            || !self.product_code.ends_with('}')
        {
            return Err("local repair MSI product invalid".into());
        }
        let mut names = std::collections::HashSet::new();
        for a in self.artifacts() {
            if !token(&a.sha256, 64)
                || a.size == 0
                || a.relative_path.contains(['\\', ':'])
                || a.relative_path
                    .split('/')
                    .any(|p| p.is_empty() || p == "." || p == "..")
                || !names.insert(&a.relative_path)
            {
                return Err("local repair artifact invalid".into());
            }
        }
        if self.package.relative_path
            != super::package_gate::cache_name(
                &format!(
                    "https://download.yan.life/rustdesk/stable/{}",
                    self.package.relative_path
                ),
                "msi",
            )?
        {
            return Err("local repair MSI name invalid".into());
        }
        for (actual, expected) in [
            (&self.client_inventory, "client-inventory.json"),
            (&self.agent_backup, "agent-backup.exe"),
            (&self.identity_backup, "identity-backup.toml"),
            (&self.config_backup, "config-backup.toml"),
            (&self.bootstrap_backup, "bootstrap-backup.exe"),
            (&self.bootstrap_manifest, "bootstrap-backup.json"),
            (&self.helper, "runtime/helper.exe"),
        ] {
            if actual.relative_path != expected {
                return Err("local repair fixed artifact path invalid".into());
            }
        }
        Ok(())
    }
    pub fn artifacts(&self) -> [&Artifact; 8] {
        [
            &self.package,
            &self.client_inventory,
            &self.agent_backup,
            &self.identity_backup,
            &self.config_backup,
            &self.bootstrap_backup,
            &self.bootstrap_manifest,
            &self.helper,
        ]
    }
    pub fn decode(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() > 65536 {
            return Err("local repair plan too large".into());
        }
        let plan: Self =
            serde_json::from_slice(bytes).map_err(|_| "local repair plan malformed")?;
        plan.validate()?;
        Ok(plan)
    }
    pub fn bind(&self, tx: &super::transaction::Transaction) -> Result<(), String> {
        self.validate()?;
        if self.transaction_id != tx.id
            || self.target_key != tx.target_key
            || self.package.sha256 != tx.package_sha256
            || self.identity_backup.sha256 != tx.identity_fingerprint
            || self.created_at != tx.created_at
        {
            return Err("local repair transaction binding mismatch".into());
        }
        Ok(())
    }
}
