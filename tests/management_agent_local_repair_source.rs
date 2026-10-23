#![cfg(windows)]
#[path = "../src/management_agent/agent_restore.rs"]
mod agent_restore;
#[path = "../src/management_agent/directory_backup.rs"]
mod directory_backup;
#[path = "../src/management_agent/file_restore.rs"]
mod file_restore;
#[path = "../src/management_agent/handoff.rs"]
mod handoff;
#[path = "../src/management_agent/health.rs"]
mod health;
#[path = "../src/management_agent/health_windows.rs"]
mod health_windows;
#[path = "../src/management_agent/identity.rs"]
mod identity;
#[path = "../src/management_agent/install_execution.rs"]
mod install_execution;
#[path = "../src/platform/windows_agent_guard.rs"]
mod install_guard;
#[path = "../src/management_agent/install_plan.rs"]
mod install_plan;
#[path = "../src/management_agent/installer_process.rs"]
mod installer_process;
#[path = "../src/management_agent/local_events.rs"]
mod local_events;
#[path = "../src/management_agent/local_recovery_events.rs"]
mod local_recovery_events;
#[path = "../src/management_agent/local_repair.rs"]
mod local_repair;
#[path = "../src/management_agent/local_repair_plan.rs"]
mod local_repair_plan;
#[path = "../src/management_agent/maintenance.rs"]
mod maintenance;
#[path = "../src/management_agent/msi_assets.rs"]
mod msi_assets;
#[path = "../src/management_agent/msi_metadata.rs"]
mod msi_metadata;
#[path = "../src/management_agent/package_gate.rs"]
mod package_gate;
#[path = "../src/management_agent/rescue_assets.rs"]
mod rescue_assets;
#[path = "../src/management_agent/resume_task.rs"]
mod resume_task;
#[path = "../src/management_agent/service_control.rs"]
mod service_control;
#[path = "../src/management_agent/state_store.rs"]
mod state_store;
#[path = "../src/management_agent/transaction.rs"]
mod transaction;
#[path = "../src/management_agent/workspace.rs"]
mod workspace;

use hbb_common::{
    base64::{engine::general_purpose::STANDARD, Engine as _},
    sodiumoxide::crypto::sign,
    UpdateManifest, UpdateTarget,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::PathBuf;

fn hash(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}
fn artifact(name: &str, bytes: &[u8]) -> Value {
    json!({"relative_path":name,"size":bytes.len(),"sha256":hash(bytes)})
}
fn save(path: impl AsRef<std::path::Path>, value: &Value) {
    std::fs::write(path, serde_json::to_vec(value).unwrap()).unwrap();
}

struct Fixture {
    root: PathBuf,
    plan: Value,
    public: sign::PublicKey,
    identity: Vec<u8>,
}
impl Fixture {
    fn new() -> Self {
        base::lab_guard::require_lab();
        let root = std::env::temp_dir().join(format!("local-source-v223-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        hbb_common::sodiumoxide::init().unwrap();
        let (public, secret) = sign::gen_keypair();
        let (ip, is) = sign::gen_keypair();
        let identity = hbb_common::toml::to_string(
            &json!({"id":"isolated-original-id","key_pair":[is.as_ref(),ip.as_ref()]}),
        )
        .unwrap()
        .into_bytes();
        let package = b"isolated signed package bytes; never executed";
        let mut manifest = UpdateManifest {
            schema: 2,
            product: "rustdesk-yan".into(),
            edition: "multi".into(),
            channel: "stable".into(),
            ..Default::default()
        };
        let key = "windows-x86_64-msi-sos";
        let mut target = UpdateTarget {
            version: "1.5.3".into(),
            build_seq: 2026100502,
            source_commit: "fixture".into(),
            primary: "https://download.yan.life/rustdesk/stable/1.5.3/source.msi".into(),
            size: package.len() as u64,
            sha256: hash(package),
            signature_key_id: "fixture-key".into(),
            ..Default::default()
        };
        target.signature = STANDARD.encode(sign::sign_detached(
            &base::update::update_signature_payload(&manifest, key, &target).unwrap(),
            &secret,
        ));
        manifest.targets.insert(key.into(), target);
        let manifest = serde_json::to_value(manifest).unwrap();
        let reference = package_gate::package_reference(&manifest, key).unwrap();
        let installation = "c".repeat(32);
        let fingerprint = hash(&identity);
        let source_id = hash(
            format!("installed-msi-v1\n{installation}\n{reference}\n{fingerprint}").as_bytes(),
        );
        let product = "{71A6889B-6003-42DB-989F-B7AE68260DDB}";
        let receipt = json!({"schema":1,"source_kind":"installed_msi","source_id":source_id,"installation_id":installation,"target_key":key,"source_reference":reference,"build_seq":2026100502u64,"product_code":product,"registered_source":root.join("source.msi"),"package":artifact("source.msi",package),"identity_sha256":fingerprint,"client_pid":100,"agent_pid":101,"observed_at":100});
        save(root.join("installation-receipt.json"), &receipt);
        save(root.join("approved-manifest.json"), &manifest);
        std::fs::write(root.join("source.msi"), package).unwrap();
        std::fs::write(root.join("identity-backup.toml"), &identity).unwrap();
        let mut plan = json!({"schema":1,"trigger":"local_recovery","transaction_id":"a".repeat(32),"installation_id":installation,"target_key":key,"build_seq":2026100502u64,"version":"1.5.3","original_normal":"missing","fallback_path":r"C:\ProgramData\RustDesk\Client\sos\RustDesk","fallback_command":r#""C:\ProgramData\RustDesk\Client\sos\RustDesk\RustDesk.exe" --service"#,"fallback_start":"auto","agent_command":r#"C:\ProgramData\RustDesk\ManagementAgent\rustdesk-management-agent.exe --service --edition sos --package-kind msi --normal-install-path "C:\Program Files\RustDesk""#,"agent_start":"auto","identity_path":r"C:\Windows\ServiceProfiles\LocalService\AppData\Roaming\RustDesk\config\RustDesk.toml","product_code":product,"source_transaction":"b".repeat(32),"created_at":200,"package":artifact("source.msi",package),"identity_backup":artifact("identity-backup.toml",&identity)});
        for (field, name) in [
            ("client_inventory", "client-inventory.json"),
            ("agent_backup", "agent-backup.exe"),
            ("config_backup", "config-backup.toml"),
            ("bootstrap_backup", "bootstrap-backup.exe"),
            ("bootstrap_manifest", "bootstrap-backup.json"),
            ("helper", "runtime/helper.exe"),
        ] {
            plan[field] = artifact(name, b"retained fixture");
        }
        let tx = transaction::Transaction::new(
            "a".repeat(32),
            key.into(),
            hash(package),
            fingerprint,
            200,
        )
        .unwrap();
        save(
            root.join("transaction.json"),
            &serde_json::to_value(tx).unwrap(),
        );
        Self {
            root,
            plan,
            public,
            identity,
        }
    }
    fn installed(&mut self) {
        let receipt: Value = serde_json::from_slice(
            &std::fs::read(self.root.join("installation-receipt.json")).unwrap(),
        )
        .unwrap();
        self.plan["source_transaction"] = "".into();
        self.plan["approval_source"] = json!({"source_kind":"installed_msi","source_id":receipt["source_id"],"source_reference":receipt["source_reference"],"installation_receipt":artifact("installation-receipt.json",&std::fs::read(self.root.join("installation-receipt.json")).unwrap())});
    }
    fn check(&self) -> Result<(), String> {
        local_repair_plan::LocalRepairPlan::approval_source_fixture(
            &serde_json::to_vec(&self.plan).unwrap(),
            &self.root,
            &self.identity,
            self.public.as_ref(),
        )
    }
    fn retained(&self) -> Vec<(String, Vec<u8>)> {
        let mut files = std::fs::read_dir(&self.root)
            .unwrap()
            .map(|e| {
                let p = e.unwrap().path();
                (
                    p.file_name().unwrap().to_string_lossy().into_owned(),
                    std::fs::read(p).unwrap(),
                )
            })
            .collect::<Vec<_>>();
        files.sort_by(|a, b| a.0.cmp(&b.0));
        files
    }
    fn refusal(&self, code: &str) {
        let before = self.retained();
        let error = self.check().unwrap_err();
        assert!(error.contains(code), "expected {code}, got {error}");
        assert_eq!(self.retained(), before);
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.root).unwrap();
    }
}

#[test]
fn source_v223_schema1_legacy_plan_roundtrip_keeps_source_transaction_and_execution_binding() {
    let f = Fixture::new();
    let bytes = serde_json::to_vec(&f.plan).unwrap();
    let p = local_repair_plan::LocalRepairPlan::decode(&bytes).unwrap();
    let tx =
        transaction::Transaction::decode(&std::fs::read(f.root.join("transaction.json")).unwrap())
            .unwrap();
    p.bind(&tx).unwrap();
    assert_eq!(p.source_transaction, "b".repeat(32));
    assert_eq!(p.transaction_id, "a".repeat(32));
    assert_eq!(serde_json::to_value(p).unwrap(), f.plan);
    assert_eq!(tx.phase, transaction::Phase::Planned);
}

#[test]
fn source_v223_installed_receipt_signed_package_uses_independent_persisted_execution_transaction() {
    let mut f = Fixture::new();
    f.installed();
    let before = f.retained();
    // The fixture uses the exact first-install receipt format, without a source transaction.
    let manifest: UpdateManifest =
        serde_json::from_slice(&std::fs::read(f.root.join("approved-manifest.json")).unwrap())
            .unwrap();
    let target = package_gate::verify_with_key(
        &manifest,
        "windows-x86_64-msi-sos",
        2026100502,
        "fixture-key",
        f.public.as_ref(),
    )
    .unwrap();
    let _lock = package_gate::lock_verified_file(&f.root.join("source.msi"), target).unwrap();
    f.check().unwrap();
    assert_eq!(f.retained(), before);
    assert_eq!(f.plan["source_transaction"], "");
    let tx =
        transaction::Transaction::decode(&std::fs::read(f.root.join("transaction.json")).unwrap())
            .unwrap();
    assert_eq!(tx.phase, transaction::Phase::Planned);
    assert_eq!(tx.id, "a".repeat(32));
}

#[test]
fn source_v223_receipt_digest_tamper_retains_execution_and_approval() {
    let mut f = Fixture::new();
    f.installed();
    std::fs::write(f.root.join("installation-receipt.json"), b"{}").unwrap();
    f.refusal("receipt digest mismatch");
}
#[test]
fn source_v223_invalid_signature_cannot_be_reapproved_by_rehashing_receipt() {
    let mut f = Fixture::new();
    let p = f.root.join("approved-manifest.json");
    let mut m: Value = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
    m["targets"]["windows-x86_64-msi-sos"]["signature"] = STANDARD.encode([0u8; 64]).into();
    save(&p, &m);
    let p = f.root.join("installation-receipt.json");
    let mut r: Value = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
    let reference = package_gate::package_reference(&m, "windows-x86_64-msi-sos").unwrap();
    r["source_reference"] = reference.clone().into();
    r["source_id"] = hash(
        format!(
            "installed-msi-v1\n{}\n{reference}\n{}",
            r["installation_id"].as_str().unwrap(),
            r["identity_sha256"].as_str().unwrap()
        )
        .as_bytes(),
    )
    .into();
    save(p, &r);
    f.installed();
    f.refusal("signature");
}
#[test]
fn source_v223_package_hash_mismatch_preserves_original_identity() {
    let mut f = Fixture::new();
    f.installed();
    std::fs::write(f.root.join("source.msi"), b"changed package").unwrap();
    f.refusal("package integrity mismatch");
}
#[test]
fn source_v223_live_identity_mismatch_preserves_receipt_and_transaction() {
    let mut f = Fixture::new();
    f.installed();
    f.identity = b"different identity".to_vec();
    f.refusal("identity mismatch");
}
#[test]
fn source_v223_missing_execution_transaction_cannot_use_installation_receipt_as_anchor() {
    let mut f = Fixture::new();
    f.installed();
    std::fs::remove_file(f.root.join("transaction.json")).unwrap();
    f.refusal("execution transaction missing");
}
#[test]
fn source_v223_wrong_execution_transaction_rejected_independently_of_source_id() {
    let mut f = Fixture::new();
    f.installed();
    let path = f.root.join("transaction.json");
    let mut tx: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    tx["id"] = "d".repeat(32).into();
    save(path, &tx);
    f.refusal("transaction binding mismatch");
}
#[test]
fn source_v223_local_disable_gate_precedes_source_acceptance() {
    let mut f = Fixture::new();
    f.installed();
    std::fs::write(f.root.join("disable-recovery"), b"local intent").unwrap();
    f.refusal("locally blocked");
}
#[test]
fn source_v223_mixed_transaction_and_installed_approval_is_rejected() {
    let mut f = Fixture::new();
    f.installed();
    f.plan["source_transaction"] = "b".repeat(32).into();
    f.refusal("mixed approval source");
}

#[test]
fn source_v223_unknown_fields_in_plan_source_and_receipt_are_rejected() {
    for location in ["plan", "source", "receipt"] {
        let mut f = Fixture::new();
        f.installed();
        if location == "receipt" {
            let path = f.root.join("installation-receipt.json");
            let mut r: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
            r["phase"] = "committed".into();
            save(path, &r);
            f.installed();
        } else if location == "plan" {
            f.plan["unexpected"] = true.into();
        } else {
            f.plan["approval_source"]["unexpected"] = true.into();
        }
        f.refusal("malformed");
    }
}
#[test]
fn source_v223_receipt_rehashed_with_wrong_identity_or_sourcehash_is_not_authority() {
    for field in [
        "installation_id",
        "source_id",
        "source_reference",
        "identity_sha256",
    ] {
        let mut f = Fixture::new();
        let path = f.root.join("installation-receipt.json");
        let mut r: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        r[field] = if field == "installation_id" {
            "d".repeat(32)
        } else {
            "d".repeat(64)
        }
        .into();
        save(path, &r);
        f.installed();
        f.refusal("binding mismatch");
    }
}
#[test]
fn source_v223_both_local_disable_markers_preserve_bytes() {
    for marker in ["disable-agent", "disable-recovery"] {
        let mut f = Fixture::new();
        f.installed();
        std::fs::write(f.root.join(marker), b"preserve explicit intent").unwrap();
        f.refusal("locally blocked");
    }
}
#[test]
fn source_v223_installed_plan_roundtrip_keeps_execution_resume_anchor() {
    let mut f = Fixture::new();
    f.installed();
    let bytes = serde_json::to_vec(&f.plan).unwrap();
    let plan = local_repair_plan::LocalRepairPlan::decode(&bytes).unwrap();
    assert_eq!(serde_json::to_value(&plan).unwrap(), f.plan);
    assert_ne!(plan.approval_id(), plan.transaction_id);
    assert!(plan.source_transaction.is_empty());
    let tx =
        transaction::Transaction::decode(&std::fs::read(f.root.join("transaction.json")).unwrap())
            .unwrap();
    plan.bind(&tx).unwrap();
    assert_eq!(
        tx.recovery_step(false, 201),
        transaction::RecoveryStep::Prepare
    );
    f.check().unwrap();
}
