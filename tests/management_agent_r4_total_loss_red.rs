#![cfg(windows)]
#[path = "../src/platform/windows_system_dirs.rs"]
#[allow(dead_code)]
mod system_dirs;
// 代理源码里有 `crate::management_agent::system_dirs` 的写法，测试入口补同名路径。
#[allow(unused_imports)]
mod management_agent {
    pub(crate) use super::system_dirs;
}
#[path = "management_agent_local_events/preservation_state_read_fixture.rs"]
mod state_read_fixture;
// 仅由 VM owner 准备现场并串行运行；测试不删除/恢复安装目录，不伪造健康响应。
#[path = "../src/management_agent/agent_restore.rs"]
mod agent_restore;
#[path = "../src/management_agent/bootstrap_update.rs"]
mod bootstrap_update;
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
#[path = "../src/management_agent/helper_runtime.rs"]
mod helper_runtime;
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
#[path = "../src/management_agent/local_repair_plan.rs"]
mod local_repair_plan;
#[path = "../src/management_agent/maintenance.rs"]
mod maintenance;
#[path = "../src/management_agent/msi_assets.rs"]
mod msi_assets;
#[path = "../src/management_agent/msi_metadata.rs"]
mod msi_metadata;
#[path = "../src/management_agent/native_install.rs"]
mod native_install;
#[path = "../src/management_agent/package_gate.rs"]
mod package_gate;
#[path = "../src/management_agent/parent_handoff.rs"]
mod parent_handoff;
#[path = "../src/management_agent/preparation.rs"]
mod preparation;
#[path = "../src/management_agent/recovery.rs"]
mod recovery;
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
mod local_repair {
    include!("../src/management_agent/local_repair.rs");
    pub fn legacy_preflight(plan: &LocalRepairPlan) -> Result<(), String> {
        check(
            Path::new(r"C:\RustDeskAgentLab\total-loss-no-legacy-io"),
            plan,
            None,
        )
    }
    pub fn total_loss_prepare_fixture(id: &str, public: &[u8]) -> Result<PathBuf, String> {
        total_loss_runtime::prepare(id, Some(public))
    }
    pub fn resume_isolated_fixture(directory: &Path, public: &[u8]) -> Result<(), String> {
        total_loss_runtime::reconcile_fixture(directory, public)
    }
    pub fn no_client_fixture() -> Result<(), String> {
        total_loss_runtime::no_client_process()
    }
    pub fn other_transactions_fixture(directory: &Path) -> Result<(), String> {
        other_transactions(directory)
    }
    pub fn owned_other_transactions_fixture(
        directory: &Path,
        binding: &super::handoff::Handoff,
    ) -> Result<(), String> {
        other_transactions_owned(directory, binding)
    }
    pub fn resumed_other_transactions_fixture(directory: &Path) -> Result<(), String> {
        let binding =
            super::handoff::Handoff::decode(&read(&directory.join("handoff.json"), 4096)?)?;
        let plan =
            LocalRepairPlan::decode(&read(&directory.join("local-repair-plan.json"), 65536)?)?;
        let agent = directory
            .parent()
            .and_then(Path::parent)
            .ok_or("fixture root missing")?;
        let _guard = super::install_guard::InstallGuard::try_acquire()
            .map_err(|e| e.to_string())?
            .ok_or("fixture lock busy")?;
        let authorized = authorize_resume_competition(
            directory,
            &plan,
            &binding,
            &agent.join("state"),
            &agent.join("intent.toml"),
            || workspace::verify_file(&directory.join("resume-task.fixture")),
        )?;
        other_transactions_resumed(directory, &authorized)
    }
    pub fn resume_product_fixture(id: &str, public: &[u8]) -> Result<(), String> {
        run_checked(id, Some(public), true, false)
    }
    pub fn native_persist_plan_fixture(
        directory: &Path,
        plan: &LocalRepairPlan,
        public: &[u8],
    ) -> Result<(), String> {
        persist_installed_plan(
            directory,
            plan,
            &super::system_dirs::get().agent().join("state"),
            &read(&directory.join("identity-backup.toml"), 65536)?,
            Some(public),
        )
    }
    pub fn cleanup_task_fixture(directory: &Path) -> Result<(), String> {
        let plan =
            LocalRepairPlan::decode(&read(&directory.join("local-repair-plan.json"), 65536)?)?;
        total_loss_runtime::cleanup_task(directory, &plan, true)
    }
    pub fn register_task_fixture(directory: &Path) -> Result<(), String> {
        let plan =
            LocalRepairPlan::decode(&read(&directory.join("local-repair-plan.json"), 65536)?)?;
        super::resume_task::register_definition(
            directory,
            &plan.transaction_id,
            &definition(&plan, directory)?,
        )
    }
}

mod local_repair_schedule {
    include!("../src/management_agent/local_repair_schedule.rs");
    pub fn observed_tick(tick: &Tick, public: &[u8]) -> Result<Decision, String> {
        run_once(tick, Some(public))
    }
    pub fn total_loss_existing_dispatch_fixture(
        root: &Path,
        plan: &LocalRepairPlan,
    ) -> Result<Decision, String> {
        let approval = Approval {
            schema: 1,
            installation_id: plan.installation_id.clone(),
            source_transaction: plan.source_transaction.clone(),
            source_kind: None,
            installation_receipt: None,
            installed_source_id: None,
            source_reference: "f".repeat(64),
            target_key: plan.target_key.clone(),
            build_seq: plan.build_seq,
            package: plan.package.clone(),
        };
        let id = attempt_id(&approval);
        let directory = root.join(&id);
        super::workspace::create(&directory)?;
        let before = std::fs::read_dir(root).map_err(|e| e.to_string())?.count();
        for _ in 0..3 {
            if run_total_loss(&approval, root, None)? != Decision::Existing(id.clone()) {
                return Err("duplicate total loss dispatch".into());
            }
        }
        if std::fs::read_dir(root).map_err(|e| e.to_string())?.count() != before
            || directory.join("install-started.json").exists()
        {
            return Err("unexpected scheduled installation".into());
        }
        Ok(Decision::Existing(id))
    }
}
mod rescue_recovery {
    include!("../src/management_agent/rescue_recovery.rs");
    pub fn observed_processes() -> Result<bool, String> {
        rustdesk_process_present()
    }
}

use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    time::Duration,
};
use windows_service::{
    service::{ServiceAccess, ServiceState},
    service_manager::{ServiceManager, ServiceManagerAccess},
};

const AGENT: &str = r"C:\ProgramData\RustDesk\ManagementAgent";
const NORMAL: &str = r"C:\Program Files\RustDesk";

#[test]
fn native_ordinary_resume_child() {
    use hbb_common::base64::{engine::general_purpose::STANDARD, Engine as _};
    let id = std::env::var("NATIVE_ORDINARY_RESUME_ID").unwrap();
    let public = STANDARD
        .decode(std::env::var("NATIVE_ORDINARY_RESUME_KEY").unwrap())
        .unwrap();
    local_repair_plan::total_loss::product_fixture(|| {
        local_repair::resume_product_fixture(&id, &public)
    })
    .unwrap();
}

#[test]
fn native_ordinary_resume_with_real_source_b_and_task() {
    use hbb_common::{
        base64::{engine::general_purpose::STANDARD, Engine as _},
        sodiumoxide::crypto::sign,
    };
    base::lab_guard::require_lab_os("win11");
    let agent = Path::new(AGENT);
    let lab = Path::new(r"C:\RustDeskAgentLab\native-ordinary-resume");
    workspace::create(lab).unwrap();
    let protected_before = snapshot(&[
        agent.join("transactions"),
        agent.join("rescue/current"),
        hbb_common::config::Config::file(),
        hbb_common::config::Config2::file(),
    ]);
    let original = agent.join("transactions/d9c041beca18488f957238579f542934");
    let original_plan = local_repair_plan::LocalRepairPlan::decode(
        &std::fs::read(original.join("local-repair-plan.json")).unwrap(),
    )
    .unwrap();
    let key = original_plan.target_key.clone();
    let mut manifest: hbb_common::UpdateManifest =
        serde_json::from_slice(&std::fs::read(original.join("approved-manifest.json")).unwrap())
            .unwrap();
    let original_reference =
        package_gate::package_reference(&serde_json::to_value(&manifest).unwrap(), &key).unwrap();
    let (public, secret) = sign::gen_keypair();
    manifest.targets.get_mut(&key).unwrap().source_commit =
        "native-resume-source-b-lab-20261014".into();
    manifest.targets.get_mut(&key).unwrap().signature_key_id = "fixture-key".into();
    let payload =
        base::update::update_signature_payload(&manifest, &key, &manifest.targets[&key]).unwrap();
    let signature = STANDARD.encode(sign::sign_detached(&payload, &secret));
    manifest.targets.get_mut(&key).unwrap().signature = signature;
    package_gate::verify_with_key(
        &manifest,
        &key,
        original_plan.build_seq,
        "fixture-key",
        public.as_ref(),
    )
    .unwrap();
    let reference =
        package_gate::package_reference(&serde_json::to_value(&manifest).unwrap(), &key).unwrap();
    assert_ne!(reference, original_reference);
    assert_eq!(manifest.targets[&key].sha256, original_plan.package.sha256);
    let approval = agent.join("release-cache/first-install.json");
    let old_approval = std::fs::read(&approval).unwrap();
    std::fs::write(lab.join("original-first-install.json"), &old_approval).unwrap();
    state_store::publish(&approval,&serde_json::to_vec(&serde_json::json!({"schema":1,"product_code":original_plan.product_code,"manifest":manifest})).unwrap()).unwrap();
    let generated =
        rescue_assets::first_install::prepare_native_resume_source_fixture(public.as_ref());
    state_store::publish(&approval, &old_approval).unwrap();
    generated.unwrap();
    let source_b = Path::new(r"C:\RustDeskAgentLab\native-resume-source-b\current");
    let receipt_bytes = std::fs::read(source_b.join("installation-receipt.json")).unwrap();
    let receipt: serde_json::Value = serde_json::from_slice(&receipt_bytes).unwrap();
    assert_eq!(receipt["source_reference"], reference);
    assert_ne!(receipt["source_id"], original_plan.approval_id());
    assert_eq!(
        snapshot(&[
            agent.join("transactions"),
            agent.join("rescue/current"),
            hbb_common::config::Config::file(),
            hbb_common::config::Config2::file()
        ]),
        protected_before
    );
    let id = uuid::Uuid::new_v4().simple().to_string();
    let directory = agent.join("transactions").join(&id);
    workspace::create(&directory).unwrap();
    workspace::create(&directory.join("runtime")).unwrap();
    let mut plan = original_plan.clone();
    plan.transaction_id = id.clone();
    plan.created_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    plan.approval_source = Some(local_repair_plan::ApprovalSource {
        source_kind: "installed_msi".into(),
        source_id: receipt["source_id"].as_str().unwrap().into(),
        source_reference: reference.clone(),
        installation_receipt: install_plan::Artifact {
            relative_path: "installation-receipt.json".into(),
            size: receipt_bytes.len() as u64,
            sha256: hex::encode(Sha256::digest(&receipt_bytes)),
        },
    });
    for artifact in original_plan.artifacts() {
        if artifact.relative_path != "runtime/helper.exe" {
            std::fs::copy(
                original.join(&artifact.relative_path),
                directory.join(&artifact.relative_path),
            )
            .unwrap();
        }
    }
    std::fs::copy(
        std::env::current_exe().unwrap(),
        directory.join("runtime/helper.exe"),
    )
    .unwrap();
    plan.helper.size = std::fs::metadata(directory.join("runtime/helper.exe"))
        .unwrap()
        .len();
    plan.helper.sha256 = hex::encode(Sha256::digest(
        std::fs::read(directory.join("runtime/helper.exe")).unwrap(),
    ));
    std::fs::write(
        directory.join("approved-manifest.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    std::fs::write(directory.join("installation-receipt.json"), &receipt_bytes).unwrap();
    local_repair::native_persist_plan_fixture(&directory, &plan, public.as_ref()).unwrap();
    let mut tx = transaction::Transaction::decode(
        &std::fs::read(directory.join("transaction.json")).unwrap(),
    )
    .unwrap();
    let proof = transaction::Evidence {
        package_verified: true,
        backups_verified: true,
        helper_verified: true,
        resume_task_verified: true,
        handoff_verified: true,
        maintenance_owned: true,
        services_stopped: true,
        target_verified: false,
        identity_unchanged: false,
        client_ready: false,
        agent_ready: false,
        rollback_verified: false,
        locally_blocked: false,
    };
    for phase in [
        transaction::Phase::Prepared,
        transaction::Phase::StoppingServices,
        transaction::Phase::ServicesStopped,
        transaction::Phase::Applying,
        transaction::Phase::Validating,
    ] {
        tx.advance(
            phase,
            &proof,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
        )
        .unwrap();
        tx.save(&directory.join("transaction.json")).unwrap();
    }
    let mut old_helper = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "total_loss_process_wait_entry"])
        .stdout(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let previous = handoff::ObservedProcess::open(old_helper.id())
        .unwrap()
        .owner()
        .clone();
    old_helper.kill().unwrap();
    old_helper.wait().unwrap();
    let binding = handoff::Handoff::new(
        id.clone(),
        hex::encode(Sha256::digest(
            std::fs::read(directory.join("local-repair-plan.json")).unwrap(),
        )),
        "8".repeat(64),
        handoff::ObservedProcess::open(std::process::id())
            .unwrap()
            .owner()
            .clone(),
        previous,
        plan.created_at,
    )
    .unwrap();
    let ready = binding.ready_receipt(plan.created_at).unwrap();
    let release = binding.release_receipt(&ready, plan.created_at).unwrap();
    for (name, value) in [
        ("handoff.json", serde_json::to_value(&binding).unwrap()),
        ("local-ready.json", serde_json::to_value(&ready).unwrap()),
        (
            "local-release.json",
            serde_json::to_value(&release).unwrap(),
        ),
    ] {
        state_store::publish(&directory.join(name), &serde_json::to_vec(&value).unwrap()).unwrap();
    }
    let old_id = "e4a04f8690f44bd3a6837fcff57eb642";
    let old = agent.join("transactions").join(old_id);
    workspace::create(&old).unwrap();
    let imported = Path::new(r"C:\RustDeskAgentLab\native-resume-import\chain-export3-records");
    for name in [
        "transaction.json",
        "local-repair-plan.json",
        "handoff.json",
        "local-ready.json",
        "local-release.json",
        "agent-preservation.json",
        "local-result.json",
        "total-loss-install-error.json",
        "total-loss-reconciliation.json",
    ] {
        std::fs::copy(imported.join(format!("{old_id}_{name}")), old.join(name)).unwrap();
    }
    let old_before = snapshot(&[old.clone()]);
    local_repair_plan::total_loss::product_fixture(|| {
        local_repair_plan::total_loss::completed_preservation(&old)
    })
    .unwrap();
    local_repair::register_task_fixture(&directory).unwrap();
    maintenance::claim(
        &agent.join("state"),
        &plan.installation_id,
        &id,
        &binding.plan_sha256,
        &binding.nonce,
    )
    .unwrap();
    let services = service_snapshot();
    let exit = std::process::Command::new(directory.join("runtime/helper.exe"))
        .args(["--exact", "native_ordinary_resume_child", "--nocapture"])
        .env("NATIVE_ORDINARY_RESUME_ID", &id)
        .env(
            "NATIVE_ORDINARY_RESUME_KEY",
            STANDARD.encode(public.as_ref()),
        )
        .status()
        .unwrap();
    assert!(
        exit.success(),
        "native resume evidence retained={}",
        directory.display()
    );
    let final_tx = transaction::Transaction::decode(
        &std::fs::read(directory.join("transaction.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(final_tx.phase, transaction::Phase::Committed);
    for name in [
        "install-started.json",
        "installer.log",
        "install-result.json",
    ] {
        assert!(
            !directory.join(name).exists(),
            "no installer expected {name}"
        );
    }
    assert_eq!(snapshot(&[old]), old_before);
    assert_eq!(service_snapshot(), services);
    assert!(!agent.join("state/maintenance.json").exists());
    assert!(resume_task::read_back(&id).is_err());
    state_store::publish(&lab.join("proof.json"),&serde_json::to_vec_pretty(&serde_json::json!({"old_source":original_plan.approval_id(),"new_source":plan.approval_id(),"reference":reference,
        "old_id":old_id,"new_id":id,"final_phase":final_tx.phase,"installer_calls":0,"old_history":old_before,"helper_hash":plan.helper.sha256})).unwrap()).unwrap();
    println!("NATIVE_RESUME_COMMITTED id={id}; installer_calls=0; history_unchanged=true");
}

#[test]
fn preservation_resume_child() {
    let directory = PathBuf::from(std::env::var_os("PRESERVATION_RESUME_ROOT").unwrap());
    let before = snapshot(&[directory.parent().unwrap().join("a".repeat(32))]);
    let result = local_repair::resumed_other_transactions_fixture(&directory);
    if std::env::var("PRESERVATION_RESUME_REJECT").is_ok() {
        assert!(result.is_err(), "invalid resume accepted");
    } else {
        assert!(
            result.is_ok(),
            "legitimate new resume process refused: {result:?}"
        );
    }
    assert_eq!(
        snapshot(&[directory.parent().unwrap().join("a".repeat(32))]),
        before
    );
}

#[test]
fn completed_preservation_new_source_admission() {
    base::lab_guard::require_lab_os("win11");
    let root =
        std::env::temp_dir().join(format!("completed-preservation-{}", uuid::Uuid::new_v4()));
    workspace::create(&root).unwrap();
    let transactions = root.join("transactions");
    workspace::create(&transactions).unwrap();
    let state = root.join("state");
    workspace::create(&state).unwrap();
    workspace::create(&root.join("tasks")).unwrap();
    let old = transactions.join("a".repeat(32));
    workspace::create(&old).unwrap();
    let mut plan = local_repair_plan::LocalRepairPlan::decode(
        &serde_json::to_vec(&total_loss_plan_value()).unwrap(),
    )
    .unwrap();
    let mut owner_child = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "total_loss_process_wait_entry"])
        .stdout(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let owner = handoff::ObservedProcess::open(owner_child.id())
        .unwrap()
        .owner()
        .clone();
    owner_child.kill().unwrap();
    owner_child.wait().unwrap();
    let bytes = serde_json::to_vec(&plan).unwrap();
    std::fs::write(old.join("local-repair-plan.json"), &bytes).unwrap();
    let binding = handoff::Handoff::new(
        plan.transaction_id.clone(),
        hex::encode(Sha256::digest(&bytes)),
        "f".repeat(64),
        handoff::Owner {
            pid: owner.pid + 1,
            created_at: owner.created_at,
        },
        owner.clone(),
        100,
    )
    .unwrap();
    // The distinct parent may be a reused PID; only the dead helper and reconciliation owner authorize completion.
    let handoff_bytes = serde_json::to_vec(&binding).unwrap();
    std::fs::write(old.join("handoff.json"), &handoff_bytes).unwrap();
    let ready = binding.ready_receipt(101).unwrap();
    std::fs::write(
        old.join("local-ready.json"),
        serde_json::to_vec(&ready).unwrap(),
    )
    .unwrap();
    std::fs::write(
        old.join("local-release.json"),
        serde_json::to_vec(&binding.release_receipt(&ready, 102).unwrap()).unwrap(),
    )
    .unwrap();
    let mut tx = transaction::Transaction::new(
        plan.transaction_id.clone(),
        plan.target_key.clone(),
        plan.package.sha256.clone(),
        plan.identity_backup.sha256.clone(),
        plan.created_at,
    )
    .unwrap();
    tx.save(&old.join("transaction.json")).unwrap();
    let proof = transaction::Evidence {
        package_verified: true,
        backups_verified: true,
        helper_verified: true,
        resume_task_verified: true,
        handoff_verified: true,
        maintenance_owned: true,
        services_stopped: true,
        target_verified: false,
        identity_unchanged: false,
        client_ready: false,
        agent_ready: false,
        rollback_verified: false,
        locally_blocked: false,
    };
    for phase in [
        transaction::Phase::PackageVerified,
        transaction::Phase::Prepared,
        transaction::Phase::StoppingServices,
        transaction::Phase::ServicesStopped,
        transaction::Phase::Applying,
        transaction::Phase::RollingBack,
    ] {
        tx.advance(phase, &proof, 101).unwrap();
        tx.save(&old.join("transaction.json")).unwrap();
    }
    let expected = local_repair_plan::total_loss::Binding {
        transaction_id: tx.id.clone(),
        installation_id: plan.installation_id.clone(),
        plan_sha256: binding.plan_sha256.clone(),
        handoff_sha256: hex::encode(Sha256::digest(&handoff_bytes)),
    };
    let receipt = local_repair_plan::total_loss::PreservationReceipt::new(
        expected,
        "original install failed".into(),
        Ok(()),
        true,
    );
    let receipt_bytes = serde_json::to_vec(&receipt).unwrap();
    std::fs::write(old.join("agent-preservation.json"), &receipt_bytes).unwrap();
    std::fs::write(old.join("total-loss-install-error.json"),serde_json::to_vec(&serde_json::json!({"transaction_id":tx.id,"plan_sha256":binding.plan_sha256,"error":receipt.installation_error})).unwrap()).unwrap();
    std::fs::write(old.join("local-result.json"),serde_json::to_vec(&serde_json::json!({"schema":1,"transaction_id":tx.id,"phase":"preservation","receipt":receipt})).unwrap()).unwrap();
    let reconciled=serde_json::to_vec(&serde_json::json!({"schema":1,"transaction_id":tx.id,"state":"agent_preserved_client_unrecovered","receipt":receipt,"observed_helper":owner})).unwrap();
    std::fs::write(old.join("total-loss-reconciliation.json"), &reconciled).unwrap();
    std::fs::write(state.join("agent.json"),serde_json::to_vec(&serde_json::json!({"agent_installation_id":plan.installation_id,"disabled":false,"management_agent_enabled":true})).unwrap()).unwrap();
    std::fs::write(state.join("service-recovery-enabled"), b"enabled").unwrap();
    std::fs::write(root.join("intent.toml"), b"[options]\nstop-service='N'\n").unwrap();
    let before = snapshot(&[old.clone()]);
    assert_eq!(
        local_repair_schedule::existing(
            &transactions,
            &plan.installation_id,
            plan.approval_id(),
            &plan.target_key,
            &plan.package.sha256
        )
        .unwrap(),
        Some(tx.id.clone())
    );
    assert_eq!(
        local_repair_schedule::existing(
            &transactions,
            &plan.installation_id,
            plan.approval_id(),
            &plan.target_key,
            &"e".repeat(64)
        )
        .unwrap(),
        Some(tx.id.clone())
    );
    assert_eq!(
        local_repair_schedule::existing(
            &transactions,
            &plan.installation_id,
            &"d".repeat(32),
            &plan.target_key,
            &"e".repeat(64)
        )
        .unwrap(),
        None,
        "completed preservation must not reserve a different source"
    );
    let new = transactions.join("d".repeat(32));
    workspace::create(&new).unwrap();
    plan.transaction_id = "d".repeat(32);
    plan.source_transaction = "d".repeat(32);
    plan.package.sha256 = "e".repeat(64);
    std::fs::write(
        new.join("local-repair-plan.json"),
        serde_json::to_vec(&plan).unwrap(),
    )
    .unwrap();
    local_repair::other_transactions_fixture(&new).unwrap();
    let new_plan_bytes = serde_json::to_vec(&plan).unwrap();
    plan.source_transaction = "c".repeat(32);
    std::fs::write(
        new.join("local-repair-plan.json"),
        serde_json::to_vec(&plan).unwrap(),
    )
    .unwrap();
    assert!(local_repair::other_transactions_fixture(&new)
        .unwrap_err()
        .contains("competing transaction"));
    std::fs::write(new.join("local-repair-plan.json"), &new_plan_bytes).unwrap();
    for fault in [
        "unconfirmed",
        "receipt-hash",
        "reconciliation",
        "active-owner",
        "maintenance",
        "task",
        "task-unknown",
        "disabled",
        "optin",
        "stop",
    ] {
        match fault {
            "unconfirmed" => {
                let mut v: serde_json::Value = serde_json::from_slice(&receipt_bytes).unwrap();
                v["agent_verified"] = false.into();
                std::fs::write(
                    old.join("agent-preservation.json"),
                    serde_json::to_vec(&v).unwrap(),
                )
                .unwrap();
            }
            "receipt-hash" => {
                let mut v: serde_json::Value = serde_json::from_slice(&receipt_bytes).unwrap();
                v["binding"]["plan_sha256"] = "0".repeat(64).into();
                std::fs::write(
                    old.join("agent-preservation.json"),
                    serde_json::to_vec(&v).unwrap(),
                )
                .unwrap();
            }
            "reconciliation" => {
                std::fs::remove_file(old.join("total-loss-reconciliation.json")).unwrap()
            }
            "active-owner" => {
                let mut v: serde_json::Value = serde_json::from_slice(&reconciled).unwrap();
                v["observed_helper"] = serde_json::to_value(
                    handoff::ObservedProcess::open(std::process::id())
                        .unwrap()
                        .owner(),
                )
                .unwrap();
                std::fs::write(
                    old.join("total-loss-reconciliation.json"),
                    serde_json::to_vec(&v).unwrap(),
                )
                .unwrap();
            }
            "maintenance" => std::fs::write(state.join("maintenance.json"), b"retained").unwrap(),
            "task" => std::fs::write(
                root.join("tasks")
                    .join(format!("RustDeskManagementAgentResume-{}", tx.id)),
                b"retained",
            )
            .unwrap(),
            "task-unknown" => std::fs::remove_dir(root.join("tasks")).unwrap(),
            "disabled" => std::fs::write(state.join("disable-recovery"), b"disabled").unwrap(),
            "optin" => std::fs::remove_file(state.join("service-recovery-enabled")).unwrap(),
            _ => {
                std::fs::write(root.join("intent.toml"), b"[options]\nstop-service='Y'\n").unwrap()
            }
        }
        let fault_bytes = snapshot(&[old.clone()]);
        assert!(
            local_repair::other_transactions_fixture(&new).is_err(),
            "{fault}"
        );
        std::fs::remove_file(new.join("local-repair-plan.json")).unwrap();
        std::fs::remove_dir(&new).unwrap();
        assert_eq!(
            local_repair_schedule::existing(
                &transactions,
                &plan.installation_id,
                &"d".repeat(32),
                &plan.target_key,
                &"e".repeat(64)
            )
            .unwrap(),
            Some(tx.id.clone()),
            "{fault}"
        );
        workspace::create(&new).unwrap();
        std::fs::write(new.join("local-repair-plan.json"), &new_plan_bytes).unwrap();
        assert_eq!(
            snapshot(&[old.clone()]),
            fault_bytes,
            "{fault} changed old evidence"
        );
        std::fs::write(old.join("agent-preservation.json"), &receipt_bytes).unwrap();
        std::fs::write(old.join("total-loss-reconciliation.json"), &reconciled).unwrap();
        if !root.join("tasks").exists() {
            workspace::create(&root.join("tasks")).unwrap();
        }
        for marker in [
            state.join("maintenance.json"),
            state.join("disable-recovery"),
            root.join("tasks")
                .join(format!("RustDeskManagementAgentResume-{}", tx.id)),
        ] {
            if marker.exists() {
                std::fs::remove_file(marker).unwrap();
            }
        }
        std::fs::write(state.join("service-recovery-enabled"), b"enabled").unwrap();
        std::fs::write(root.join("intent.toml"), b"[options]\nstop-service='N'\n").unwrap();
    }
    for total_loss in [false, true] {
        let mut current = local_repair_plan::LocalRepairPlan::decode(&new_plan_bytes).unwrap();
        if !total_loss {
            current.total_loss = None;
        }
        let current_bytes = serde_json::to_vec(&current).unwrap();
        std::fs::write(new.join("local-repair-plan.json"), &current_bytes).unwrap();
        let current_binding = handoff::Handoff::new(
            current.transaction_id.clone(),
            hex::encode(Sha256::digest(&current_bytes)),
            "9".repeat(64),
            owner.clone(),
            handoff::ObservedProcess::open(std::process::id())
                .unwrap()
                .owner()
                .clone(),
            100,
        )
        .unwrap();
        std::fs::write(
            new.join("handoff.json"),
            serde_json::to_vec(&current_binding).unwrap(),
        )
        .unwrap();
        let current_ready = current_binding.ready_receipt(101).unwrap();
        std::fs::write(
            new.join("local-ready.json"),
            serde_json::to_vec(&current_ready).unwrap(),
        )
        .unwrap();
        std::fs::write(
            new.join("local-release.json"),
            serde_json::to_vec(
                &current_binding
                    .release_receipt(&current_ready, 102)
                    .unwrap(),
            )
            .unwrap(),
        )
        .unwrap();
        let mut current_tx = transaction::Transaction::new(
            current.transaction_id.clone(),
            current.target_key.clone(),
            current.package.sha256.clone(),
            current.identity_backup.sha256.clone(),
            current.created_at,
        )
        .unwrap();
        if new.join("transaction.json").exists() {
            std::fs::remove_file(new.join("transaction.json")).unwrap();
        }
        current_tx.save(&new.join("transaction.json")).unwrap();
        for phase in [
            transaction::Phase::PackageVerified,
            transaction::Phase::Prepared,
            transaction::Phase::StoppingServices,
        ] {
            current_tx.advance(phase, &proof, 101).unwrap();
            current_tx.save(&new.join("transaction.json")).unwrap();
        }
        local_repair::other_transactions_fixture(&new).unwrap();
        maintenance::claim(
            &state,
            &current.installation_id,
            &current.transaction_id,
            &current_binding.plan_sha256,
            &current_binding.nonce,
        )
        .unwrap();
        local_repair::owned_other_transactions_fixture(&new, &current_binding)
            .expect("new owned maintenance must not invalidate completed old preservation");
        assert!(
            local_repair::other_transactions_fixture(&new).is_err(),
            "unowned preflight must reject current marker"
        );
        let marker = std::fs::read(state.join("maintenance.json")).unwrap();
        std::fs::write(state.join("maintenance.json"), b"foreign marker").unwrap();
        assert!(local_repair::owned_other_transactions_fixture(&new, &current_binding).is_err());
        std::fs::write(state.join("maintenance.json"), &marker).unwrap();
        std::fs::remove_file(new.join("handoff.json")).unwrap();
        assert!(local_repair::owned_other_transactions_fixture(&new, &current_binding).is_err());
        std::fs::write(
            new.join("handoff.json"),
            serde_json::to_vec(&current_binding).unwrap(),
        )
        .unwrap();
        let current_handoff_bytes = serde_json::to_vec(&current_binding).unwrap();
        for fault in [
            "wrong-nonce",
            "wrong-plan-hash",
            "no-ready",
            "local-disable",
            "old-task",
            "same-source",
        ] {
            let mut presented = current_binding.clone();
            match fault {
                "wrong-nonce" => {
                    let mut v: serde_json::Value = serde_json::from_slice(&marker).unwrap();
                    v["nonce"] = "8".repeat(64).into();
                    std::fs::write(
                        state.join("maintenance.json"),
                        serde_json::to_vec(&v).unwrap(),
                    )
                    .unwrap();
                }
                "wrong-plan-hash" => {
                    let mut v: serde_json::Value = serde_json::from_slice(&marker).unwrap();
                    v["plan_sha256"] = "8".repeat(64).into();
                    std::fs::write(
                        state.join("maintenance.json"),
                        serde_json::to_vec(&v).unwrap(),
                    )
                    .unwrap();
                }
                "no-ready" => std::fs::remove_file(new.join("local-ready.json")).unwrap(),
                "local-disable" => {
                    std::fs::write(state.join("disable-agent"), b"disabled").unwrap()
                }
                "old-task" => std::fs::write(
                    root.join("tasks")
                        .join(format!("RustDeskManagementAgentResume-{}", tx.id)),
                    b"old pending task",
                )
                .unwrap(),
                _ => {
                    let mut same = current.clone();
                    same.source_transaction = "c".repeat(32);
                    let same_bytes = serde_json::to_vec(&same).unwrap();
                    presented.plan_sha256 = hex::encode(Sha256::digest(&same_bytes));
                    std::fs::write(new.join("local-repair-plan.json"), &same_bytes).unwrap();
                    std::fs::write(
                        new.join("handoff.json"),
                        serde_json::to_vec(&presented).unwrap(),
                    )
                    .unwrap();
                    let same_ready = presented.ready_receipt(101).unwrap();
                    std::fs::write(
                        new.join("local-ready.json"),
                        serde_json::to_vec(&same_ready).unwrap(),
                    )
                    .unwrap();
                    std::fs::write(
                        new.join("local-release.json"),
                        serde_json::to_vec(&presented.release_receipt(&same_ready, 102).unwrap())
                            .unwrap(),
                    )
                    .unwrap();
                    let mut value: serde_json::Value = serde_json::from_slice(&marker).unwrap();
                    value["plan_sha256"] = presented.plan_sha256.clone().into();
                    std::fs::write(
                        state.join("maintenance.json"),
                        serde_json::to_vec(&value).unwrap(),
                    )
                    .unwrap();
                }
            }
            assert!(
                local_repair::owned_other_transactions_fixture(&new, &presented).is_err(),
                "{total_loss}/{fault}"
            );
            std::fs::write(state.join("maintenance.json"), &marker).unwrap();
            std::fs::write(
                new.join("local-ready.json"),
                serde_json::to_vec(&current_ready).unwrap(),
            )
            .unwrap();
            std::fs::write(new.join("local-repair-plan.json"), &current_bytes).unwrap();
            std::fs::write(new.join("handoff.json"), &current_handoff_bytes).unwrap();
            std::fs::write(
                new.join("local-release.json"),
                serde_json::to_vec(
                    &current_binding
                        .release_receipt(&current_ready, 102)
                        .unwrap(),
                )
                .unwrap(),
            )
            .unwrap();
            for path in [
                state.join("disable-agent"),
                root.join("tasks")
                    .join(format!("RustDeskManagementAgentResume-{}", tx.id)),
            ] {
                if path.exists() {
                    std::fs::remove_file(path).unwrap();
                }
            }
        }
        local_repair::owned_other_transactions_fixture(&new, &current_binding).unwrap();
        maintenance::release_owned(
            &state,
            &current.installation_id,
            &current.transaction_id,
            &current_binding.plan_sha256,
            &current_binding.nonce,
        )
        .unwrap();
        assert_eq!(snapshot(&[old.clone()]), before);
    }
    let mut resumed = local_repair_plan::LocalRepairPlan::decode(&new_plan_bytes).unwrap();
    resumed.total_loss = None;
    workspace::create(&new.join("runtime")).unwrap();
    let helper = new.join("runtime/helper.exe");
    std::fs::copy(std::env::current_exe().unwrap(), &helper).unwrap();
    resumed.helper.size = std::fs::metadata(&helper).unwrap().len();
    resumed.helper.sha256 = hex::encode(Sha256::digest(std::fs::read(&helper).unwrap()));
    resumed.created_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let resumed_bytes = serde_json::to_vec(&resumed).unwrap();
    std::fs::write(new.join("local-repair-plan.json"), &resumed_bytes).unwrap();
    let mut resume_tx = transaction::Transaction::new(
        resumed.transaction_id.clone(),
        resumed.target_key.clone(),
        resumed.package.sha256.clone(),
        resumed.identity_backup.sha256.clone(),
        resumed.created_at,
    )
    .unwrap();
    std::fs::remove_file(new.join("transaction.json")).unwrap();
    resume_tx.save(&new.join("transaction.json")).unwrap();
    for phase in [
        transaction::Phase::PackageVerified,
        transaction::Phase::Prepared,
        transaction::Phase::StoppingServices,
    ] {
        resume_tx
            .advance(phase, &proof, resumed.created_at)
            .unwrap();
        resume_tx.save(&new.join("transaction.json")).unwrap();
    }
    let resumed_binding = handoff::Handoff::new(
        resumed.transaction_id.clone(),
        hex::encode(Sha256::digest(&resumed_bytes)),
        "7".repeat(64),
        handoff::ObservedProcess::open(std::process::id())
            .unwrap()
            .owner()
            .clone(),
        owner.clone(),
        resumed.created_at,
    )
    .unwrap();
    let rr = resumed_binding.ready_receipt(resumed.created_at).unwrap();
    std::fs::write(
        new.join("handoff.json"),
        serde_json::to_vec(&resumed_binding).unwrap(),
    )
    .unwrap();
    std::fs::write(
        new.join("local-ready.json"),
        serde_json::to_vec(&rr).unwrap(),
    )
    .unwrap();
    std::fs::write(
        new.join("local-release.json"),
        serde_json::to_vec(
            &resumed_binding
                .release_receipt(&rr, resumed.created_at)
                .unwrap(),
        )
        .unwrap(),
    )
    .unwrap();
    maintenance::claim(
        &state,
        &resumed.installation_id,
        &resumed.transaction_id,
        &resumed_binding.plan_sha256,
        &resumed_binding.nonce,
    )
    .unwrap();
    std::fs::write(
        new.join("resume-task.fixture"),
        b"fixed task verified by fixture",
    )
    .unwrap();
    let child = std::process::Command::new(&helper)
        .args(["--exact", "preservation_resume_child", "--nocapture"])
        .env("PRESERVATION_RESUME_ROOT", &new)
        .status()
        .unwrap();
    assert!(
        child.success(),
        "resume fixture retained={}",
        root.display()
    );
    let valid_marker = std::fs::read(state.join("maintenance.json")).unwrap();
    let valid_handoff = std::fs::read(new.join("handoff.json")).unwrap();
    for fault in [
        "foreign-marker",
        "bad-plan",
        "pending-old-task",
        "missing-resume-task",
        "live-old-owner",
        "disabled",
    ] {
        match fault {
            "foreign-marker" => std::fs::write(state.join("maintenance.json"), b"{}").unwrap(),
            "bad-plan" => std::fs::write(new.join("local-repair-plan.json"), b"{}").unwrap(),
            "pending-old-task" => std::fs::write(
                root.join("tasks")
                    .join(format!("RustDeskManagementAgentResume-{}", tx.id)),
                b"pending",
            )
            .unwrap(),
            "missing-resume-task" => std::fs::remove_file(new.join("resume-task.fixture")).unwrap(),
            "live-old-owner" => {
                let mut live = resumed_binding.clone();
                live.parent = owner.clone();
                live.helper = handoff::ObservedProcess::open(std::process::id())
                    .unwrap()
                    .owner()
                    .clone();
                std::fs::write(new.join("handoff.json"), serde_json::to_vec(&live).unwrap())
                    .unwrap();
            }
            _ => std::fs::write(state.join("disable-recovery"), b"disabled").unwrap(),
        }
        let evidence = snapshot(&[old.clone(), new.clone()]);
        let rejected = std::process::Command::new(&helper)
            .args(["--exact", "preservation_resume_child", "--nocapture"])
            .env("PRESERVATION_RESUME_ROOT", &new)
            .env("PRESERVATION_RESUME_REJECT", fault)
            .status()
            .unwrap();
        assert!(rejected.success(), "{fault}");
        assert_eq!(snapshot(&[old.clone(), new.clone()]), evidence);
        std::fs::write(state.join("maintenance.json"), &valid_marker).unwrap();
        std::fs::write(new.join("local-repair-plan.json"), &resumed_bytes).unwrap();
        std::fs::write(new.join("handoff.json"), &valid_handoff).unwrap();
        std::fs::write(
            new.join("resume-task.fixture"),
            b"fixed task verified by fixture",
        )
        .unwrap();
        for path in [
            state.join("disable-recovery"),
            root.join("tasks")
                .join(format!("RustDeskManagementAgentResume-{}", tx.id)),
        ] {
            if path.exists() {
                std::fs::remove_file(path).unwrap();
            }
        }
    }
    assert_eq!(snapshot(&[old]), before);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn acl_boundary_revocation_after_sd_save_blocks_write_and_stop() {
    base::lab_guard::require_lab_os("win11");
    let root = std::env::temp_dir().join(format!("acl-boundary-{}", uuid::Uuid::new_v4()));
    workspace::create(&root).unwrap();
    let marker = root.join("enabled");
    std::fs::write(&marker, b"enabled").unwrap();
    let saved_marker = marker.clone();
    let original_acl = workspace::agent_acl_bytes_fixture().unwrap();
    let stops = std::cell::Cell::new(0);
    let result = (|| {
        workspace::verify_file(&marker)?;
        workspace::agent_acl_revoke_fixture(
            &root,
            move || {
                std::fs::remove_file(saved_marker).unwrap();
            },
            || {
                workspace::verify_file(&marker)
                    .map_err(|_| "fixture opt-in revoked after SD save".into())
            },
        )?;
        stops.set(stops.get() + 1);
        Ok::<(), String>(())
    })();
    assert!(root.join("agent-directory-original.sd").is_file());
    assert!(
        result.is_err(),
        "revocation after actual SD sync must refuse ACL write; retained={}",
        root.display()
    );
    assert_eq!(result.unwrap_err(), "fixture opt-in revoked after SD save");
    assert_eq!(stops.get(), 0);
    assert_eq!(workspace::agent_acl_bytes_fixture().unwrap(), original_acl);
    assert_eq!(
        std::fs::read(root.join("agent-directory-original.sd")).unwrap(),
        original_acl
    );
    let normal = root.join("authorized");
    workspace::create(&normal).unwrap();
    std::fs::write(&marker, b"enabled").unwrap();
    workspace::agent_acl_revoke_fixture(&normal, || {}, || workspace::verify_file(&marker))
        .unwrap();
    assert_ne!(workspace::agent_acl_bytes_fixture().unwrap(), original_acl);
    assert_eq!(
        std::fs::read(normal.join("agent-directory-original.sd")).unwrap(),
        original_acl
    );
}

#[test]
fn coordination_helper_entry() {
    use hbb_common::base64::{engine::general_purpose::STANDARD, Engine as _};
    let directory = PathBuf::from(std::env::var("RUSTDESK_COORDINATION_ROOT").unwrap());
    let public = STANDARD
        .decode(std::env::var("RUSTDESK_COORDINATION_KEY").unwrap())
        .unwrap();
    let before = std::fs::read(directory.join("transaction.json")).unwrap();
    if std::env::var("RUSTDESK_COORDINATION_MODE").as_deref() == Ok("committed") {
        local_repair::resume_isolated_fixture(&directory, &public).unwrap();
        assert_eq!(
            std::fs::read(directory.join("transaction.json")).unwrap(),
            before
        );
        assert!(!directory.join("fixture-state/maintenance.json").exists());
        assert!(!directory.join("install-started.json").exists());
        return;
    }
    if std::env::var("RUSTDESK_COORDINATION_MODE").as_deref() == Ok("alive") {
        let error = local_repair::resume_isolated_fixture(&directory, &public).unwrap_err();
        assert!(error.contains("prior helper still owns"), "{error}");
        assert_eq!(
            std::fs::read(directory.join("transaction.json")).unwrap(),
            before
        );
        return;
    }
    if std::env::var("RUSTDESK_COORDINATION_MODE").as_deref() == Ok("preserved") {
        let error = local_repair::resume_isolated_fixture(&directory, &public).unwrap_err();
        assert!(
            error.contains("agent preserved; client unrecovered; reconciliation recorded"),
            "{error}"
        );
        assert_eq!(
            std::fs::read(directory.join("transaction.json")).unwrap(),
            before
        );
        assert!(!directory.join("fixture-state/maintenance.json").exists());
        assert!(!directory.join("install-started.json").exists());
        let record: serde_json::Value = serde_json::from_slice(
            &std::fs::read(directory.join("total-loss-reconciliation.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(record["state"], "agent_preserved_client_unrecovered");
        assert_eq!(record["receipt"]["client_recovered"], false);
        local_repair::cleanup_task_fixture(&directory).unwrap();
        local_repair::cleanup_task_fixture(&directory).unwrap();
        return;
    }
    let error = local_repair::resume_isolated_fixture(&directory, &public).unwrap_err();
    assert!(error.contains("no new installer"), "{error}");
    assert_eq!(
        std::fs::read(directory.join("transaction.json")).unwrap(),
        before
    );
    assert!(!directory.join("install-started.json").exists());
    let release = std::fs::read(directory.join("local-release.json")).unwrap();
    std::fs::write(directory.join("local-release.json"), b"{}").unwrap();
    assert!(local_repair::resume_isolated_fixture(&directory, &public)
        .unwrap_err()
        .contains("release invalid"));
    std::fs::write(directory.join("local-release.json"), release).unwrap();
    std::fs::write(
        directory.join("fixture-state/disable-recovery"),
        b"disabled",
    )
    .unwrap();
    assert!(local_repair::resume_isolated_fixture(&directory, &public)
        .unwrap_err()
        .contains("locally blocked"));
    std::fs::remove_file(directory.join("fixture-state/disable-recovery")).unwrap();
    std::fs::write(
        directory.join("fixture-intent.toml"),
        b"[options]\nstop-service='Y'\n",
    )
    .unwrap();
    assert!(local_repair::resume_isolated_fixture(&directory, &public)
        .unwrap_err()
        .contains("stop intent"));
    assert_eq!(
        std::fs::read(directory.join("transaction.json")).unwrap(),
        before
    );
    assert!(!directory.join("install-started.json").exists());
}

#[test]
fn coordination_applying_real_helper_preserves_unknown_result() {
    use hbb_common::base64::{engine::general_purpose::STANDARD, Engine as _};
    base::lab_guard::require_lab_os("win11");
    let source = Path::new(AGENT).join("transactions/d9c041beca18488f957238579f542934");
    let bundle: serde_json::Value = serde_json::from_slice(
        &std::fs::read(Path::new(AGENT).join("rescue/current/bundle.json")).unwrap(),
    )
    .unwrap();
    let public = rescue_assets::fixed_source_public_fixture(&bundle).unwrap();
    let root =
        std::env::temp_dir().join(format!("total-loss-coordination-{}", uuid::Uuid::new_v4()));
    workspace::create(&root).unwrap();
    workspace::create(&root.join("runtime")).unwrap();
    let mut plan = local_repair_plan::LocalRepairPlan::decode(
        &std::fs::read(source.join("local-repair-plan.json")).unwrap(),
    )
    .unwrap();
    for artifact in plan.artifacts() {
        if artifact.relative_path != "runtime/helper.exe" {
            std::fs::copy(
                source.join(&artifact.relative_path),
                root.join(&artifact.relative_path),
            )
            .unwrap();
        }
    }
    for name in ["approved-manifest.json", "installation-receipt.json"] {
        std::fs::copy(source.join(name), root.join(name)).unwrap();
    }
    let helper = root.join("runtime/helper.exe");
    std::fs::copy(std::env::current_exe().unwrap(), &helper).unwrap();
    plan.helper.size = std::fs::metadata(&helper).unwrap().len();
    plan.helper.sha256 = hex::encode(Sha256::digest(std::fs::read(&helper).unwrap()));
    plan.transaction_id = uuid::Uuid::new_v4().simple().to_string();
    plan.total_loss = Some(local_repair_plan::total_loss::Prestate {
        client_anchor: "none".into(),
        original_service: local_repair_plan::total_loss::OriginalService::Absent,
    });
    let bytes = serde_json::to_vec(&plan).unwrap();
    std::fs::write(root.join("local-repair-plan.json"), &bytes).unwrap();
    let mut tx = transaction::Transaction::new(
        plan.transaction_id.clone(),
        plan.target_key.clone(),
        plan.package.sha256.clone(),
        plan.identity_backup.sha256.clone(),
        plan.created_at,
    )
    .unwrap();
    tx.save(&root.join("transaction.json")).unwrap();
    let proof = transaction::Evidence {
        package_verified: true,
        backups_verified: true,
        helper_verified: true,
        resume_task_verified: true,
        handoff_verified: true,
        maintenance_owned: true,
        services_stopped: true,
        target_verified: false,
        identity_unchanged: false,
        client_ready: false,
        agent_ready: false,
        rollback_verified: false,
        locally_blocked: false,
    };
    for phase in [
        transaction::Phase::PackageVerified,
        transaction::Phase::Prepared,
        transaction::Phase::StoppingServices,
        transaction::Phase::ServicesStopped,
        transaction::Phase::Applying,
    ] {
        tx.advance(phase, &proof, plan.created_at).unwrap();
        tx.save(&root.join("transaction.json")).unwrap();
    }
    let mut previous = std::process::Command::new(r"C:\Windows\System32\cmd.exe")
        .args(["/d", "/c", "ping -n 2 127.0.0.1 >nul"])
        .spawn()
        .unwrap();
    let previous_owner = handoff::ObservedProcess::open(previous.id())
        .unwrap()
        .owner()
        .clone();
    previous.wait().unwrap();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let binding = handoff::Handoff::new(
        plan.transaction_id.clone(),
        hex::encode(Sha256::digest(&bytes)),
        "f".repeat(64),
        handoff::ObservedProcess::open(std::process::id())
            .unwrap()
            .owner()
            .clone(),
        previous_owner,
        now,
    )
    .unwrap();
    let ready = binding.ready_receipt(now).unwrap();
    let release = binding.release_receipt(&ready, now).unwrap();
    for (name, data) in [
        ("handoff.json", serde_json::to_vec(&binding).unwrap()),
        ("local-ready.json", serde_json::to_vec(&ready).unwrap()),
        ("local-release.json", serde_json::to_vec(&release).unwrap()),
    ] {
        std::fs::write(root.join(name), data).unwrap();
    }
    let state = root.join("fixture-state");
    workspace::create(&state).unwrap();
    std::fs::write(state.join("agent.json"),serde_json::to_vec(&serde_json::json!({"agent_installation_id":plan.installation_id,"disabled":false,"management_agent_enabled":true})).unwrap()).unwrap();
    std::fs::write(state.join("service-recovery-enabled"), b"enabled").unwrap();
    std::fs::write(
        root.join("fixture-intent.toml"),
        b"[options]\nstop-service='N'\n",
    )
    .unwrap();
    maintenance::claim(
        &state,
        &plan.installation_id,
        &plan.transaction_id,
        &binding.plan_sha256,
        &binding.nonce,
    )
    .unwrap();
    let mut alive_binding = binding.clone();
    alive_binding.helper = handoff::ObservedProcess::open(std::process::id())
        .unwrap()
        .owner()
        .clone();
    alive_binding.parent = handoff::Owner {
        pid: 1,
        created_at: 1,
    };
    std::fs::write(
        root.join("handoff.json"),
        serde_json::to_vec(&alive_binding).unwrap(),
    )
    .unwrap();
    let alive_status = std::process::Command::new(&helper)
        .args(["--exact", "coordination_helper_entry", "--nocapture"])
        .env("RUSTDESK_COORDINATION_ROOT", &root)
        .env("RUSTDESK_COORDINATION_KEY", STANDARD.encode(&public))
        .env("RUSTDESK_COORDINATION_MODE", "alive")
        .status()
        .unwrap();
    assert!(alive_status.success());
    std::fs::write(
        root.join("handoff.json"),
        serde_json::to_vec(&binding).unwrap(),
    )
    .unwrap();
    let status = std::process::Command::new(&helper)
        .args(["--exact", "coordination_helper_entry", "--nocapture"])
        .env("RUSTDESK_COORDINATION_ROOT", &root)
        .env("RUSTDESK_COORDINATION_KEY", STANDARD.encode(&public))
        .status()
        .unwrap();
    assert!(
        status.success(),
        "helper fixture retained={}",
        root.display()
    );
    let namespace = root.join("schedule");
    workspace::create(&namespace).unwrap();
    let saved = namespace.join(&plan.transaction_id);
    workspace::create(&saved).unwrap();
    std::fs::copy(
        root.join("transaction.json"),
        saved.join("transaction.json"),
    )
    .unwrap();
    std::fs::copy(
        root.join("local-repair-plan.json"),
        saved.join("local-repair-plan.json"),
    )
    .unwrap();
    let before = snapshot(&[namespace.clone()]);
    for _ in 0..3 {
        assert_eq!(
            local_repair_schedule::existing(
                &namespace,
                &plan.installation_id,
                plan.approval_id(),
                &plan.target_key,
                &plan.package.sha256
            )
            .unwrap(),
            Some(plan.transaction_id.clone())
        );
    }
    assert_eq!(snapshot(&[namespace.clone()]), before);
    for phase in [
        transaction::Phase::Validating,
        transaction::Phase::Committed,
    ] {
        let mut final_proof = proof;
        final_proof.target_verified = true;
        final_proof.identity_unchanged = true;
        final_proof.client_ready = true;
        final_proof.agent_ready = true;
        tx.advance(phase, &final_proof, plan.created_at).unwrap();
        tx.save(&saved.join("transaction.json")).unwrap();
    }
    let committed = snapshot(&[namespace.clone()]);
    assert_eq!(
        local_repair_schedule::existing(
            &namespace,
            &plan.installation_id,
            plan.approval_id(),
            &plan.target_key,
            &plan.package.sha256
        )
        .unwrap(),
        Some(plan.transaction_id.clone())
    );
    assert_eq!(snapshot(&[namespace]), committed);
    let dispatch_root = root.join("schedule-dispatch");
    workspace::create(&dispatch_root).unwrap();
    assert!(matches!(
        local_repair_schedule::total_loss_existing_dispatch_fixture(&dispatch_root, &plan).unwrap(),
        local_repair_schedule::Decision::Existing(_)
    ));
    let applying = std::fs::read(root.join("transaction.json")).unwrap();
    std::fs::write(
        root.join("transaction.json"),
        serde_json::to_vec(&tx).unwrap(),
    )
    .unwrap();
    std::fs::write(
        root.join("fixture-intent.toml"),
        b"[options]\nstop-service='N'\n",
    )
    .unwrap();
    state_store::publish(&root.join("local-result.json"),&serde_json::to_vec(&serde_json::json!({"schema":1,"transaction_id":tx.id,"phase":"committed","error":null,"reboot_required":false})).unwrap()).unwrap();
    for _ in 0..2 {
        let status = std::process::Command::new(&helper)
            .args(["--exact", "coordination_helper_entry", "--nocapture"])
            .env("RUSTDESK_COORDINATION_ROOT", &root)
            .env("RUSTDESK_COORDINATION_KEY", STANDARD.encode(&public))
            .env("RUSTDESK_COORDINATION_MODE", "committed")
            .status()
            .unwrap();
        assert!(status.success());
    }
    std::fs::write(root.join("transaction.json"), applying).unwrap();
    maintenance::claim(
        &state,
        &plan.installation_id,
        &plan.transaction_id,
        &binding.plan_sha256,
        &binding.nonce,
    )
    .unwrap();
    // Restore the separate execution fixture to its actual Applying snapshot, then model a crash after preservation.
    let mut interrupted =
        transaction::Transaction::decode(&std::fs::read(root.join("transaction.json")).unwrap())
            .unwrap();
    interrupted
        .advance(transaction::Phase::RollingBack, &proof, plan.created_at)
        .unwrap();
    interrupted.save(&root.join("transaction.json")).unwrap();
    std::fs::write(
        root.join("fixture-intent.toml"),
        b"[options]\nstop-service='N'\n",
    )
    .unwrap();
    let receipt_binding = local_repair_plan::total_loss::Binding {
        transaction_id: plan.transaction_id.clone(),
        installation_id: plan.installation_id.clone(),
        plan_sha256: binding.plan_sha256.clone(),
        handoff_sha256: hex::encode(Sha256::digest(
            std::fs::read(root.join("handoff.json")).unwrap(),
        )),
    };
    let actual = local_repair_plan::total_loss::product_fixture(|| {
        local_repair_plan::total_loss::native_verify_running(&plan)
    });
    assert!(actual.is_ok(), "{actual:?}");
    let mut bad = plan.clone();
    bad.agent_backup.sha256 = "0".repeat(64);
    let preserved_services = service_snapshot();
    assert!(local_repair_plan::total_loss::product_fixture(|| {
        local_repair_plan::total_loss::native_verify_running(&bad)
    })
    .is_err());
    assert_eq!(service_snapshot(), preserved_services);
    assert!(local_repair::no_client_fixture()
        .unwrap_err()
        .contains("fixed client process active"));
    let receipt = local_repair_plan::total_loss::PreservationReceipt::new(
        receipt_binding,
        "fixture original installation error".into(),
        actual,
        true,
    );
    state_store::publish(
        &root.join("agent-preservation.json"),
        &serde_json::to_vec(&receipt).unwrap(),
    )
    .unwrap();
    let services_before = service_snapshot();
    local_repair::register_task_fixture(&root).unwrap();
    for _ in 0..2 {
        let status = std::process::Command::new(&helper)
            .args(["--exact", "coordination_helper_entry", "--nocapture"])
            .env("RUSTDESK_COORDINATION_ROOT", &root)
            .env("RUSTDESK_COORDINATION_KEY", STANDARD.encode(&public))
            .env("RUSTDESK_COORDINATION_MODE", "preserved")
            .status()
            .unwrap();
        assert!(
            status.success(),
            "preserved helper fixture retained={}",
            root.display()
        );
    }
    assert_eq!(service_snapshot(), services_before);
    std::fs::remove_file(root.join("agent-preservation.json")).unwrap();
    maintenance::claim(
        &state,
        &plan.installation_id,
        &plan.transaction_id,
        &binding.plan_sha256,
        &binding.nonce,
    )
    .unwrap();
    state_store::publish(&root.join("total-loss-install-error.json"),&serde_json::to_vec(&serde_json::json!({
        "transaction_id":plan.transaction_id,"plan_sha256":binding.plan_sha256,"error":"fixture original installation error"})).unwrap()).unwrap();
    let status = std::process::Command::new(&helper)
        .args(["--exact", "coordination_helper_entry", "--nocapture"])
        .env("RUSTDESK_COORDINATION_ROOT", &root)
        .env("RUSTDESK_COORDINATION_KEY", STANDARD.encode(&public))
        .env("RUSTDESK_COORDINATION_MODE", "preserved")
        .status()
        .unwrap();
    assert!(
        status.success(),
        "pre-preservation resume retained={}",
        root.display()
    );
    assert_eq!(service_snapshot(), services_before);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn acl_slice_protect_fixture() {
    base::lab_guard::require_lab_os("win11");
    workspace::protect_agent_directory_fixture().unwrap();
    assert_eq!(
        std::fs::read(r"C:\RustDeskAgentLab\total-loss-acl-fixture\existing.exe").unwrap(),
        b"old trusted bytes"
    );
    std::fs::rename(
        r"C:\RustDeskAgentLab\total-loss-acl-fixture\missing.exe",
        r"C:\RustDeskAgentLab\total-loss-acl-fixture\squat-evidence.exe",
    )
    .unwrap();
}

fn vm_only() {
    base::lab_guard::require_lab();
    assert_eq!(
        std::env::var("RUSTDESK_R4_TOTAL_LOSS_RED").as_deref(),
        Ok("owner-prepared")
    );
}

#[test]
fn total_loss_chain_child() {
    use hbb_common::base64::{engine::general_purpose::STANDARD, Engine as _};
    let id = std::env::var("RUSTDESK_TOTAL_LOSS_CHILD").unwrap();
    let public = STANDARD
        .decode(std::env::var("RUSTDESK_TOTAL_LOSS_PUBLIC").unwrap())
        .unwrap();
    if std::env::var("RUSTDESK_TOTAL_LOSS_RESUME").as_deref() == Ok("1") {
        let error = local_repair::resume_product_fixture(&id, &public).unwrap_err();
        assert!(
            error.contains("agent preserved; client unrecovered; reconciliation recorded"),
            "{error}"
        );
        return;
    }
    let fail = std::env::var("RUSTDESK_TOTAL_LOSS_FAIL").as_deref() == Ok("1");
    let result = local_repair::run_fixture(&id, &public, fail);
    if fail {
        let error = result.unwrap_err();
        assert!(error.contains("client not recovered"), "{error}");
    } else {
        result.unwrap();
    }
}

#[test]
fn total_loss_chain_offline_success() {
    vm_only();
    absent(Path::new(NORMAL));
    absent(Path::new(r"C:\ProgramData\RustDesk\Client\sos\RustDesk"));
    let bundle: serde_json::Value = serde_json::from_slice(
        &std::fs::read(Path::new(AGENT).join("rescue/current/bundle.json")).unwrap(),
    )
    .unwrap();
    let public = rescue_assets::fixed_source_public_fixture(&bundle).unwrap();
    let id = uuid::Uuid::new_v4().simple().to_string();
    let directory = local_repair::total_loss_prepare_fixture(&id, &public).unwrap();
    let mut child = local_repair::launch_fixture(&directory, &public).unwrap();
    let exit = child.wait().unwrap();
    assert!(
        exit.success(),
        "child failed; retained={}",
        directory.display()
    );
    let tx = transaction::Transaction::decode(
        &std::fs::read(directory.join("transaction.json")).unwrap(),
    )
    .unwrap();
    if std::env::var("RUSTDESK_TOTAL_LOSS_FAIL").as_deref() == Ok("1") {
        assert_eq!(tx.phase, transaction::Phase::RollingBack);
        let bytes = std::fs::read(directory.join("local-repair-plan.json")).unwrap();
        let handoff = std::fs::read(directory.join("handoff.json")).unwrap();
        let plan = local_repair_plan::LocalRepairPlan::decode(&bytes).unwrap();
        let binding = local_repair_plan::total_loss::Binding {
            transaction_id: tx.id.clone(),
            installation_id: plan.installation_id.clone(),
            plan_sha256: hex::encode(Sha256::digest(&bytes)),
            handoff_sha256: hex::encode(Sha256::digest(&handoff)),
        };
        let receipt = local_repair_plan::total_loss::reopen_receipt(&directory, &binding).unwrap();
        assert!(receipt.confirmed());
        assert!(!receipt.client_recovered);
        assert!(receipt
            .installation_error
            .contains("injected target verification failure"));
        println!("TOTAL_LOSS_AGENT_PRESERVED={}", directory.display());
        let original_tx = std::fs::read(directory.join("transaction.json")).unwrap();
        let before_services = service_snapshot();
        use hbb_common::base64::{engine::general_purpose::STANDARD, Engine as _};
        let resumed = std::process::Command::new(directory.join("runtime/helper.exe"))
            .args(["--exact", "total_loss_chain_child", "--nocapture"])
            .env("RUSTDESK_TOTAL_LOSS_CHILD", &id)
            .env("RUSTDESK_TOTAL_LOSS_PUBLIC", STANDARD.encode(&public))
            .env("RUSTDESK_TOTAL_LOSS_RESUME", "1")
            .status()
            .unwrap();
        assert!(resumed.success());
        assert_eq!(
            std::fs::read(directory.join("transaction.json")).unwrap(),
            original_tx
        );
        assert_eq!(service_snapshot(), before_services);
        assert!(!Path::new(AGENT).join("state/maintenance.json").exists());
        assert!(resume_task::read_back(&id).is_err());
        for _ in 0..3 {
            assert_eq!(
                local_repair_schedule::existing(
                    &Path::new(AGENT).join("transactions"),
                    &binding.installation_id,
                    plan.approval_id(),
                    &plan.target_key,
                    &plan.package.sha256
                )
                .unwrap(),
                Some(id.clone())
            );
        }
    } else {
        assert_eq!(tx.phase, transaction::Phase::Committed);
    }
    println!(
        "TOTAL_LOSS_PHASE={:?}; directory={}",
        tx.phase,
        directory.display()
    );
}

#[test]
fn worker_event_lock_process() {
    use std::os::windows::fs::OpenOptionsExt;
    base::lab_guard::require_lab_os("win11");
    let root = Path::new(r"C:\RustDeskAgentLab\total-loss-worker-events");
    let deadline = std::time::Instant::now() + Duration::from_secs(180);
    while !root.join("at-started").exists() {
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
    let lock = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .share_mode(0)
        .open(root.join("events.lock"))
        .unwrap();
    std::fs::write(root.join("lock-held"), b"locked").unwrap();
    let directory = PathBuf::from(std::fs::read_to_string(root.join("at-started")).unwrap());
    while !directory.join("local-event-unreported.json").exists() {
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
    drop(lock);
    std::fs::write(root.join("lock-released"), b"released after unreported").unwrap();
}

#[test]
fn total_loss_worker_offline_new_transaction() {
    vm_only();
    absent(Path::new(NORMAL));
    absent(Path::new(r"C:\ProgramData\RustDesk\Client\sos\RustDesk"));
    let transactions = Path::new(AGENT).join("transactions");
    assert!(
        !transactions.exists() || std::fs::read_dir(&transactions).unwrap().next().is_none(),
        "requires original no-transaction baseline"
    );
    let bundle: serde_json::Value = serde_json::from_slice(
        &std::fs::read(Path::new(AGENT).join("rescue/current/bundle.json")).unwrap(),
    )
    .unwrap();
    let public = rescue_assets::fixed_source_public_fixture(&bundle).unwrap();
    let event_root = Path::new(r"C:\RustDeskAgentLab\total-loss-worker-events");
    workspace::create(event_root).unwrap();
    let device = identity::load_existing().unwrap();
    let event_binding = local_events::Binding {
        client_id: device.id.clone(),
        client_uuid: device.uuid.clone(),
        agent_installation_id: bundle["installation_id"].as_str().unwrap().into(),
        enrollment_generation: 1,
    };
    let event_path = event_root.join("events.json");
    local_events::LocalEvents::open(&event_path)
        .unwrap()
        .remember_binding(event_binding.clone())
        .unwrap();
    let context = local_recovery_events::EventContext {
        path: event_path.clone(),
        binding: event_binding.clone(),
    };
    let context_path = event_root.join("context.json");
    std::fs::write(&context_path, serde_json::to_vec(&context).unwrap()).unwrap();
    std::env::set_var("RUSTDESK_LOCAL_EVENTS_FIXTURE", &context_path);
    let lock_fault = std::env::var("RUSTDESK_TEST_STARTED_LOCK").as_deref() == Ok("1");
    let mut locker = if lock_fault {
        std::env::set_var("RUSTDESK_EVENT_LOCK_BOUNDARY", event_root);
        Some(
            std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "worker_event_lock_process", "--nocapture"])
                .spawn()
                .unwrap(),
        )
    } else {
        None
    };
    let worker = local_repair_schedule::start_fixture(public).unwrap();
    let wait = || {
        let deadline = std::time::Instant::now() + Duration::from_secs(900);
        loop {
            if let Some(result) = worker.result() {
                break result;
            }
            assert!(std::time::Instant::now() < deadline, "worker timed out");
            std::thread::sleep(Duration::from_millis(100));
        }
    };
    let identity = std::fs::read(hbb_common::config::Config::file()).unwrap();
    let intent = hbb_common::config::Config2::file();
    let original = std::fs::read(&intent).unwrap();
    let mut options: hbb_common::toml::Value =
        hbb_common::toml::from_str(std::str::from_utf8(&original).unwrap()).unwrap();
    options
        .as_table_mut()
        .unwrap()
        .entry("options")
        .or_insert_with(|| hbb_common::toml::Value::Table(Default::default()))
        .as_table_mut()
        .unwrap()
        .insert(
            "stop-service".into(),
            hbb_common::toml::Value::String("Y".into()),
        );
    std::fs::write(&intent, hbb_common::toml::to_string(&options).unwrap()).unwrap();
    worker.notify(tick(true)).unwrap();
    let blocked = wait();
    std::fs::write(&intent, &original).unwrap();
    assert_eq!(blocked, "Deferred");
    assert!(!transactions.exists() || std::fs::read_dir(&transactions).unwrap().next().is_none());
    worker.notify(tick(true)).unwrap();
    let result = wait();
    let directories = std::fs::read_dir(&transactions)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect::<Vec<_>>();
    assert_eq!(directories.len(), 1, "{result}");
    let directory = &directories[0];
    let tx = transaction::Transaction::decode(
        &std::fs::read(directory.join("transaction.json")).unwrap(),
    )
    .unwrap();
    let events = local_events::LocalEvents::open(&event_path).unwrap();
    assert_eq!(events.verified_binding(), Some(&event_binding));
    let pending = serde_json::to_value(events.pending_events()).unwrap();
    println!("ACTUAL_HELPER_EVENTS={pending}");
    if let Some(child) = locker.as_mut() {
        assert!(child.wait().unwrap().success());
        assert!(event_root.join("lock-released").exists());
        let unreported: serde_json::Value = serde_json::from_slice(
            &std::fs::read(directory.join("local-event-unreported.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(unreported["transaction_id"], tx.id);
        assert_eq!(
            unreported["binding"],
            serde_json::to_value(&event_binding).unwrap()
        );
        assert_eq!(pending.as_array().unwrap().len(), 0);
    } else {
        assert_eq!(
            pending.as_array().unwrap().len(),
            1,
            "actual helper must write exactly one Started"
        );
        assert_eq!(pending[0]["payload"]["result"]["operation_id"], tx.id);
        assert_eq!(pending[0]["payload"]["result"]["phase"], "started");
    }
    if std::env::var("RUSTDESK_TOTAL_LOSS_FAIL").as_deref() == Ok("1") {
        assert_eq!(tx.phase, transaction::Phase::RollingBack, "{result}");
        let receipt: local_repair_plan::total_loss::PreservationReceipt = serde_json::from_slice(
            &std::fs::read(directory.join("agent-preservation.json")).unwrap(),
        )
        .unwrap();
        assert!(receipt.confirmed(), "{receipt:?}");
        assert!(!receipt.client_recovered);
        assert!(receipt
            .installation_error
            .contains("injected target verification failure"));
    } else {
        assert_eq!(tx.phase, transaction::Phase::Committed, "{result}");
        assert!(result.starts_with("Existing"));
    }
    let before = snapshot(&[transactions.clone()]);
    for _ in 0..3 {
        let observer = local_repair_schedule::start_fixture(
            rescue_assets::fixed_source_public_fixture(&bundle).unwrap(),
        )
        .unwrap();
        observer.notify(tick(true)).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        loop {
            if let Some(value) = observer.result() {
                assert!(
                    value.starts_with("Existing") || value == "Observe" || value == "Deferred",
                    "{value}"
                );
                break;
            }
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(100));
        }
    }
    assert_eq!(snapshot(&[transactions]), before);
    assert_eq!(
        std::fs::read(hbb_common::config::Config::file()).unwrap(),
        identity
    );
    println!("WORKER_RESULT={result}; TX={}; PHASE={:?}", tx.id, tx.phase);
}

fn absent(path: &Path) {
    assert_eq!(
        std::fs::symlink_metadata(path).unwrap_err().kind(),
        std::io::ErrorKind::NotFound,
        "fixture requires explicit absence: {}",
        path.display()
    );
}

fn service_snapshot() -> Vec<String> {
    let scm = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT).unwrap();
    ["RustDesk", "RustDeskManagementAgent"]
        .into_iter()
        .map(|name| {
            match scm.open_service(
                name,
                ServiceAccess::QUERY_CONFIG | ServiceAccess::QUERY_STATUS,
            ) {
                Ok(service) => {
                    let c = service.query_config().unwrap();
                    let s = service.query_status().unwrap();
                    format!(
                        "{name}:{:?}:{:?}:{:?}:{:?}:{:?}",
                        c.executable_path,
                        c.account_name,
                        c.start_type,
                        s.current_state,
                        s.process_id
                    )
                }
                Err(windows_service::Error::Winapi(e))
                    if e.raw_os_error() == Some(1060) && name == "RustDesk" =>
                {
                    format!("{name}:absent")
                }
                Err(e) => panic!("fixture SCM observation failed: {e}"),
            }
        })
        .collect()
}

fn snapshot(roots: &[PathBuf]) -> BTreeMap<PathBuf, String> {
    fn visit(path: &Path, out: &mut BTreeMap<PathBuf, String>) {
        use std::os::windows::fs::MetadataExt;
        let m = match std::fs::symlink_metadata(path) {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                out.insert(path.into(), "missing".into());
                return;
            }
            Err(e) => panic!("fixture snapshot unavailable: {e}"),
        };
        if m.file_attributes() & 0x400 != 0 {
            out.insert(
                path.into(),
                format!("reparse:{:?}", std::fs::read_link(path).unwrap()),
            );
        } else if m.is_dir() {
            out.insert(path.into(), "directory".into());
            for item in std::fs::read_dir(path).unwrap() {
                visit(&item.unwrap().path(), out);
            }
        } else {
            out.insert(
                path.into(),
                hex::encode(Sha256::digest(std::fs::read(path).unwrap())),
            );
        }
    }
    let mut out = BTreeMap::new();
    for root in roots {
        visit(root, &mut out);
    }
    out
}

fn roots() -> Vec<PathBuf> {
    vec![
        Path::new(AGENT).join("rescue/current"),
        Path::new(AGENT).join("transactions"),
        Path::new(AGENT).join("state"),
        Path::new(AGENT).join("rustdesk-management-agent.exe"),
        r"C:\ProgramData\RustDesk\RecoveryBootstrap".into(),
        hbb_common::config::Config::file(),
        hbb_common::config::Config2::file(),
        NORMAL.into(),
        r"C:\ProgramData\RustDesk\Client".into(),
    ]
}

fn tick(enabled: bool) -> local_repair_schedule::Tick {
    local_repair_schedule::Tick {
        elapsed: Duration::from_secs(181),
        enabled,
        retry_at: 0,
    }
}

#[test]
fn missing_agent_verified_backup_restores_without_existing_destination() {
    missing_agent_case("success");
}

#[test]
fn missing_agent_native_query_isolated_service_without_file() {
    use windows_service::service::{
        ServiceErrorControl, ServiceInfo, ServiceStartType, ServiceType,
    };
    base::lab_guard::require_lab_os("win11");
    let scm = ServiceManager::local_computer(
        None::<&str>,
        ServiceManagerAccess::CONNECT | ServiceManagerAccess::CREATE_SERVICE,
    )
    .unwrap();
    let name = "RustDeskAgentTestService-TotalLossNative";
    let mut plan = local_repair_plan::LocalRepairPlan::decode(
        &serde_json::to_vec(&total_loss_plan_value()).unwrap(),
    )
    .unwrap();
    let missing = local_repair_plan::total_loss::native_observation(&plan)
        .err()
        .expect("absent fixture must fail");
    assert!(missing.contains("unknown or access denied"), "{missing}");
    let info = ServiceInfo {
        name: name.into(),
        display_name: name.into(),
        service_type: ServiceType::OWN_PROCESS,
        start_type: ServiceStartType::OnDemand,
        error_control: ServiceErrorControl::Normal,
        executable_path: r"C:\RustDeskAgentLab\total-loss-native\fixture-agent.exe".into(),
        launch_arguments: vec![
            "--exact".into(),
            "total_loss_native_service_entry".into(),
            "--nocapture".into(),
        ],
        dependencies: vec![],
        account_name: None,
        account_password: None,
    };
    let service = scm
        .create_service(
            &info,
            ServiceAccess::QUERY_CONFIG
                | ServiceAccess::QUERY_STATUS
                | ServiceAccess::DELETE
                | ServiceAccess::STOP,
        )
        .unwrap();
    let observation = local_repair_plan::total_loss::native_observation(&plan);
    let tested = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        assert!(
            observation.is_ok(),
            "fixed stopped fixture query: {:?}",
            observation.err()
        );
        let denied =
            local_repair_plan::total_loss::native_access_failure_fixture(&plan).unwrap_err();
        assert!(denied.contains("SCM config unavailable"), "{denied}");
        let root = Path::new(r"C:\RustDeskAgentLab\total-loss-native");
        if root.exists() {
            workspace::verify(root).unwrap();
        } else {
            workspace::create(root).unwrap();
        }
        let exe = root.join("fixture-agent.exe");
        if exe.exists() {
            workspace::verify_file(&exe).unwrap();
            std::fs::remove_file(&exe).unwrap();
        }
        std::fs::copy(std::env::current_exe().unwrap(), &exe).unwrap();
        let bytes = std::fs::read(&exe).unwrap();
        plan.agent_backup.size = bytes.len() as u64;
        plan.agent_backup.sha256 = hex::encode(Sha256::digest(&bytes));
        std::fs::write(root.join("identity.toml"), b"native fixture identity").unwrap();
        plan.identity_backup.size = 23;
        plan.identity_backup.sha256 = hex::encode(Sha256::digest(b"native fixture identity"));
        local_repair_plan::total_loss::native_verify(&plan).unwrap();
        assert_eq!(
            service.query_status().unwrap().current_state,
            ServiceState::Running
        );
        assert!(local_repair_plan::total_loss::native_observation(&plan)
            .err()
            .unwrap()
            .contains("requires stopped"));
    }));
    if service.query_status().unwrap().current_state == ServiceState::Running {
        service.stop().unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while service.query_status().unwrap().current_state != ServiceState::Stopped {
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(100));
        }
    }
    service.delete().unwrap();
    drop(service);
    if let Err(e) = tested {
        std::panic::resume_unwind(e);
    }
}

windows_service::define_windows_service!(total_loss_service_ffi, total_loss_service_main);
#[test]
fn total_loss_process_wait_entry() {
    std::thread::sleep(Duration::from_secs(20));
}

#[test]
fn native_slice_foreign_basename_rejected_with_image_evidence() {
    use windows_service::service::{
        ServiceErrorControl, ServiceInfo, ServiceStartType, ServiceType,
    };
    let root = std::env::temp_dir().join(format!("native-image-{}", uuid::Uuid::new_v4()));
    workspace::create(&root).unwrap();
    let exe = root.join("fixture-agent.exe");
    std::fs::copy(std::env::current_exe().unwrap(), &exe).unwrap();
    let mut child = std::process::Command::new(&exe)
        .args(["--exact", "total_loss_process_wait_entry"])
        .spawn()
        .unwrap();
    let scm = ServiceManager::local_computer(
        None::<&str>,
        ServiceManagerAccess::CONNECT | ServiceManagerAccess::CREATE_SERVICE,
    )
    .unwrap();
    let info = ServiceInfo {
        name: "RustDeskAgentTestService-TotalLossNative".into(),
        display_name: "RustDesk total loss fixture".into(),
        service_type: ServiceType::OWN_PROCESS,
        start_type: ServiceStartType::OnDemand,
        error_control: ServiceErrorControl::Normal,
        executable_path: r"C:\RustDeskAgentLab\total-loss-native\fixture-agent.exe".into(),
        launch_arguments: vec![
            "--exact".into(),
            "total_loss_native_service_entry".into(),
            "--nocapture".into(),
        ],
        dependencies: vec![],
        account_name: None,
        account_password: None,
    };
    let service = scm
        .create_service(&info, ServiceAccess::DELETE | ServiceAccess::QUERY_STATUS)
        .unwrap();
    let plan = local_repair_plan::LocalRepairPlan::decode(
        &serde_json::to_vec(&total_loss_plan_value()).unwrap(),
    )
    .unwrap();
    let result = local_repair_plan::total_loss::native_observation(&plan);
    let stopped = service.query_status().unwrap().current_state;
    service.delete().unwrap();
    drop(service);
    child.kill().unwrap();
    child.wait().unwrap();
    assert_eq!(stopped, ServiceState::Stopped);
    assert!(result.err().unwrap().contains("process image mismatch"));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn native_slice_contract_rejections_and_wrapper() {
    use windows_service::service::{
        ServiceErrorControl, ServiceInfo, ServiceStartType, ServiceType,
    };
    base::lab_guard::require_lab_os("win11");
    let root = Path::new(r"C:\RustDeskAgentLab\total-loss-native");
    if root.exists() {
        workspace::verify(root).unwrap();
    } else {
        workspace::create(root).unwrap();
    }
    let scm = ServiceManager::local_computer(
        None::<&str>,
        ServiceManagerAccess::CONNECT | ServiceManagerAccess::CREATE_SERVICE,
    )
    .unwrap();
    let plan = local_repair_plan::LocalRepairPlan::decode(
        &serde_json::to_vec(&total_loss_plan_value()).unwrap(),
    )
    .unwrap();
    for case in ["account", "args", "disabled", "wrapper"] {
        let mut info = ServiceInfo {
            name: "RustDeskAgentTestService-TotalLossNative".into(),
            display_name: "RustDesk total loss fixture".into(),
            service_type: ServiceType::OWN_PROCESS,
            start_type: ServiceStartType::OnDemand,
            error_control: ServiceErrorControl::Normal,
            executable_path: root.join("fixture-agent.exe"),
            launch_arguments: vec![
                "--exact".into(),
                "total_loss_native_service_entry".into(),
                "--nocapture".into(),
            ],
            dependencies: vec![],
            account_name: None,
            account_password: None,
        };
        match case {
            "account" => info.account_name = Some("NT AUTHORITY\\LocalService".into()),
            "args" => info.launch_arguments.push("--extra".into()),
            "disabled" => info.start_type = ServiceStartType::Disabled,
            _ => {}
        }
        let service = scm
            .create_service(
                &info,
                ServiceAccess::DELETE | ServiceAccess::QUERY_STATUS | ServiceAccess::STOP,
            )
            .unwrap();
        let before = snapshot(&[root.to_path_buf()]);
        let tested = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            if case == "wrapper" {
                if info.executable_path.exists() {
                    workspace::verify_file(&info.executable_path).unwrap();
                    std::fs::remove_file(&info.executable_path).unwrap();
                }
                missing_agent_case("native");
                assert_eq!(
                    service.query_status().unwrap().current_state,
                    ServiceState::Running
                );
                let pid = service.query_status().unwrap().process_id;
                missing_agent_case("native-existing");
                assert_eq!(service.query_status().unwrap().process_id, pid);
            } else {
                assert!(local_repair_plan::total_loss::native_observation(&plan)
                    .err()
                    .unwrap()
                    .contains("fixed contract mismatch"));
                assert!(local_repair_plan::total_loss::native_verify(&plan).is_err());
                assert_eq!(snapshot(&[root.to_path_buf()]), before);
                assert_eq!(
                    service.query_status().unwrap().current_state,
                    ServiceState::Stopped
                );
            }
        }));
        if service.query_status().unwrap().current_state == ServiceState::Running {
            service.stop().unwrap();
        }
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while service.query_status().unwrap().current_state != ServiceState::Stopped {
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(100));
        }
        service.delete().unwrap();
        drop(service);
        if let Err(e) = tested {
            std::panic::resume_unwind(e);
        }
    }
}
fn total_loss_service_main(_: Vec<std::ffi::OsString>) {
    use windows_service::{
        service::*,
        service_control_handler::{self, ServiceControlHandlerResult},
    };
    let (tx, rx) = std::sync::mpsc::channel();
    let handler = service_control_handler::register(
        "RustDeskAgentTestService-TotalLossNative",
        move |event| {
            if event == ServiceControl::Stop {
                let _ = tx.send(());
            }
            ServiceControlHandlerResult::NoError
        },
    )
    .unwrap();
    let mut status = ServiceStatus {
        service_type: ServiceType::OWN_PROCESS,
        current_state: ServiceState::Running,
        controls_accepted: ServiceControlAccept::STOP,
        exit_code: ServiceExitCode::Win32(0),
        checkpoint: 0,
        wait_hint: Duration::ZERO,
        process_id: None,
    };
    handler.set_service_status(status.clone()).unwrap();
    let _ = rx.recv_timeout(Duration::from_secs(45));
    status.current_state = ServiceState::Stopped;
    handler.set_service_status(status).unwrap();
}
#[test]
fn total_loss_native_service_entry() {
    assert_eq!(
        std::env::current_exe().unwrap(),
        Path::new(r"C:\RustDeskAgentLab\total-loss-native\fixture-agent.exe")
    );
    windows_service::service_dispatcher::start(
        "RustDeskAgentTestService-TotalLossNative",
        total_loss_service_ffi,
    )
    .unwrap();
}

#[test]
fn missing_agent_failures_preserve_old_data_and_both_errors() {
    for case in [
        "bad-backup",
        "unknown-process",
        "revoked",
        "occupied",
        "verify-failed",
        "identity-changed",
        "foreign-binding",
        "pending",
        "running",
        "disabled",
        "late-occupied",
        "late-revoked",
        "live-process",
        "receipt-write-failed",
        "foreign-receipt",
        "post-publish-revoked",
        "late-receipt-failure",
    ] {
        missing_agent_case(case);
    }
}

fn missing_agent_case(case: &str) {
    use local_repair_plan::total_loss::{self, AgentObservation, Binding};
    use std::cell::Cell;
    base::lab_guard::require_lab_os("win11");
    let root = std::env::temp_dir().join(format!("missing-agent-{}", uuid::Uuid::new_v4()));
    workspace::create(&root).unwrap();
    let native = matches!(case, "native" | "native-existing");
    let native_bytes = if native {
        std::fs::read(std::env::current_exe().unwrap()).unwrap()
    } else {
        vec![]
    };
    let bytes: &[u8] = if native {
        &native_bytes
    } else {
        b"isolated agent backup; never executed"
    };
    std::fs::write(root.join("agent-backup.exe"), bytes).unwrap();
    let state = root.join("state");
    workspace::create(&state).unwrap();
    let live = if native {
        PathBuf::from(r"C:\RustDeskAgentLab\total-loss-native\identity.toml")
    } else {
        root.join("identity.toml")
    };
    let intent = root.join("intent.toml");
    std::fs::write(&live, b"preserved identity").unwrap();
    std::fs::write(root.join("identity-backup.toml"), b"preserved identity").unwrap();
    std::fs::write(&intent, b"[options]\nstop-service = 'N'\n").unwrap();
    let mut value = total_loss_plan_value();
    value["agent_backup"]["size"] = bytes.len().into();
    value["agent_backup"]["sha256"] = hex::encode(Sha256::digest(bytes)).into();
    value["identity_backup"]["size"] = 18.into();
    value["identity_backup"]["sha256"] = hex::encode(Sha256::digest(b"preserved identity")).into();
    let plan_bytes = serde_json::to_vec(&value).unwrap();
    let plan = local_repair_plan::LocalRepairPlan::decode(&plan_bytes).unwrap();
    std::fs::write(root.join("local-repair-plan.json"), &plan_bytes).unwrap();
    let mut tx = transaction::Transaction::new(
        plan.transaction_id.clone(),
        plan.target_key.clone(),
        plan.package.sha256.clone(),
        plan.identity_backup.sha256.clone(),
        plan.created_at,
    )
    .unwrap();
    tx.save(&root.join("transaction.json")).unwrap();
    let proof = transaction::Evidence {
        package_verified: true,
        backups_verified: true,
        helper_verified: true,
        resume_task_verified: true,
        handoff_verified: true,
        maintenance_owned: true,
        services_stopped: true,
        target_verified: false,
        identity_unchanged: false,
        client_ready: false,
        agent_ready: false,
        rollback_verified: false,
        locally_blocked: false,
    };
    for phase in [
        transaction::Phase::PackageVerified,
        transaction::Phase::Prepared,
        transaction::Phase::StoppingServices,
        transaction::Phase::ServicesStopped,
        transaction::Phase::Applying,
    ] {
        tx.advance(phase, &proof, 101).unwrap();
        tx.save(&root.join("transaction.json")).unwrap();
    }
    if case != "pending" {
        tx.advance(transaction::Phase::RollingBack, &proof, 102)
            .unwrap();
    }
    tx.save(&root.join("transaction.json")).unwrap();
    let mut handoff = handoff::Handoff::new(
        plan.transaction_id.clone(),
        hex::encode(Sha256::digest(&plan_bytes)),
        "e".repeat(64),
        handoff::Owner {
            pid: 123,
            created_at: 90,
        },
        handoff::Owner {
            pid: 124,
            created_at: 91,
        },
        100,
    )
    .unwrap();
    if native {
        handoff.helper = handoff::ObservedProcess::open(std::process::id())
            .unwrap()
            .owner()
            .clone();
        handoff.created_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let ready = handoff.ready_receipt(handoff.created_at).unwrap();
        let release = handoff.release_receipt(&ready, handoff.created_at).unwrap();
        std::fs::write(
            root.join("local-ready.json"),
            serde_json::to_vec(&ready).unwrap(),
        )
        .unwrap();
        std::fs::write(
            root.join("local-release.json"),
            serde_json::to_vec(&release).unwrap(),
        )
        .unwrap();
    }
    let handoff_bytes = serde_json::to_vec(&handoff).unwrap();
    std::fs::write(root.join("handoff.json"), &handoff_bytes).unwrap();
    let mut binding = Binding {
        transaction_id: plan.transaction_id.clone(),
        installation_id: plan.installation_id.clone(),
        plan_sha256: handoff.plan_sha256.clone(),
        handoff_sha256: hex::encode(Sha256::digest(&handoff_bytes)),
    };
    std::fs::write(state.join("agent.json"), serde_json::to_vec(&serde_json::json!({
        "agent_installation_id":plan.installation_id,"disabled":false,"management_agent_enabled":true})).unwrap()).unwrap();
    std::fs::write(state.join("service-recovery-enabled"), b"enabled").unwrap();
    maintenance::claim(
        &state,
        &plan.installation_id,
        &plan.transaction_id,
        &handoff.plan_sha256,
        &handoff.nonce,
    )
    .unwrap();
    let destination = if native {
        PathBuf::from(r"C:\RustDeskAgentLab\total-loss-native\fixture-agent.exe")
    } else {
        root.join("rustdesk-management-agent.exe")
    };
    if case == "bad-backup" {
        std::fs::write(root.join("agent-backup.exe"), b"corrupt").unwrap();
    }
    if case == "occupied" {
        std::fs::write(&destination, b"existing destination").unwrap();
    }
    if case == "identity-changed" {
        std::fs::write(&live, b"changed identity").unwrap();
    }
    if case == "foreign-binding" {
        binding.installation_id = "f".repeat(32);
    }
    let receipt_path = root.join("agent-preservation.json");
    if case == "receipt-write-failed" {
        std::fs::create_dir(&receipt_path).unwrap();
    }
    if case == "foreign-receipt" {
        std::fs::write(&receipt_path, b"foreign evidence").unwrap();
    }
    let preserved = [
        root.join("agent-backup.exe"),
        root.join("identity-backup.toml"),
        live.clone(),
        root.join("local-repair-plan.json"),
        root.join("transaction.json"),
        root.join("handoff.json"),
        state.join("maintenance.json"),
        state.join("agent.json"),
        intent.clone(),
    ];
    let before = snapshot(&preserved);
    let observations = Cell::new(0);
    let verified = Cell::new(false);
    let receipt = if native {
        total_loss::preserve_missing_agent_native_bound(
            &root,
            &plan,
            &binding,
            "installer failed (1603)".into(),
        )
        .unwrap()
    } else {
        total_loss::preserve_missing_agent(
            &root,
            &state,
            &intent,
            &live,
            &destination,
            &binding,
            "installer failed (1603)".into(),
            || {
                observations.set(observations.get() + 1);
                if case == "unknown-process" {
                    return Err("fixture process observation unknown".into());
                }
                if case == "revoked" && observations.get() == 1 {
                    std::fs::remove_file(state.join("service-recovery-enabled")).unwrap();
                }
                if case == "late-revoked" && observations.get() == 2 {
                    std::fs::remove_file(state.join("service-recovery-enabled")).unwrap();
                }
                if case == "late-occupied" && observations.get() == 2 {
                    std::fs::write(&destination, b"racing destination").unwrap();
                }
                Ok(AgentObservation {
                    stopped: case != "running",
                    disabled: case == "disabled",
                    process_present: case == "live-process",
                })
            },
            || {
                assert_eq!(std::fs::read(&destination).unwrap(), bytes);
                verified.set(true);
                if case == "late-receipt-failure" {
                    std::fs::write(
                        root.join("agent-preservation-staging"),
                        b"preserve blocking evidence",
                    )
                    .unwrap();
                }
                if case == "post-publish-revoked" {
                    std::fs::remove_file(state.join("service-recovery-enabled")).unwrap();
                }
                if case == "verify-failed" {
                    Err("fixture agent verify failed".into())
                } else {
                    Ok(())
                }
            },
        )
    };
    assert_eq!(
        snapshot(&preserved),
        before,
        "{case}: protected data changed"
    );
    assert_eq!(receipt.installation_error, "installer failed (1603)");
    assert!(!receipt.client_recovered);
    if case == "success" || native {
        assert!(
            receipt.confirmed(),
            "{receipt:?}; retained={}",
            root.display()
        );
        receipt.verify(&binding).unwrap();
        if !native {
            assert!(verified.get());
            assert_eq!(observations.get(), 2);
        }
        assert_eq!(std::fs::read(&destination).unwrap(), bytes);
        workspace::verify_file(&receipt_path).unwrap();
        let reopened: total_loss::PreservationReceipt =
            serde_json::from_slice(&std::fs::read(&receipt_path).unwrap()).unwrap();
        assert_eq!(reopened, receipt);
        reopened.verify(&binding).unwrap();
        assert_eq!(
            total_loss::reopen_receipt(&root, &binding).unwrap(),
            receipt
        );
        let mut foreign = binding.clone();
        foreign.handoff_sha256 = "f".repeat(64);
        assert!(reopened.verify(&foreign).is_err());
        assert!(total_loss::reopen_receipt(&root, &foreign).is_err());
    } else {
        assert!(!receipt.confirmed(), "{case}");
        let error = receipt
            .agent_error
            .as_deref()
            .or(receipt.persistence_error.as_deref())
            .unwrap();
        let expected = match case {
            "unknown-process" => "fixture process observation unknown",
            "revoked" | "late-revoked" => "opt-in revoked",
            "occupied" => "occupied",
            "verify-failed" => "fixture agent verify failed",
            "foreign-binding" => "binding mismatch",
            "pending" => "phase invalid",
            "receipt-write-failed" | "foreign-receipt" | "late-receipt-failure" => "receipt",
            "post-publish-revoked" => "opt-in revoked",
            "running" | "disabled" | "live-process" => "requires stopped",
            "late-occupied" => "no-overwrite publication failed",
            _ => "mismatch",
        };
        assert!(error.contains(expected), "{case}: {error}");
        assert_eq!(
            verified.get(),
            matches!(
                case,
                "verify-failed" | "post-publish-revoked" | "late-receipt-failure"
            )
        );
        match case {
            "verify-failed" | "post-publish-revoked" | "late-receipt-failure" => {
                assert_eq!(std::fs::read(&destination).unwrap(), bytes)
            }
            "occupied" => assert_eq!(
                std::fs::read(&destination).unwrap(),
                b"existing destination"
            ),
            "late-occupied" => {
                assert_eq!(std::fs::read(&destination).unwrap(), b"racing destination")
            }
            _ => absent(&destination),
        }
        if case == "foreign-receipt" {
            assert_eq!(std::fs::read(&receipt_path).unwrap(), b"foreign evidence");
        }
    }
    std::fs::remove_dir_all(root).unwrap();
}

fn total_loss_plan_value() -> serde_json::Value {
    let artifact =
        |name: &str| serde_json::json!({"relative_path":name,"size":1,"sha256":"a".repeat(64)});
    serde_json::json!({
        "schema":1,"trigger":"local_recovery","transaction_id":"a".repeat(32),
        "installation_id":"b".repeat(32),"target_key":"windows-x86_64-msi-sos",
        "build_seq":2026100502u64,"version":"1.5.3","original_normal":"missing",
        "fallback_path":r"C:\ProgramData\RustDesk\Client\sos\RustDesk",
        "fallback_command":r#""C:\ProgramData\RustDesk\Client\sos\RustDesk\RustDesk.exe" --service"#,
        "fallback_start":"auto",
        "agent_command":r#"C:\ProgramData\RustDesk\ManagementAgent\rustdesk-management-agent.exe --service --edition sos --package-kind msi --normal-install-path "C:\Program Files\RustDesk""#,
        "agent_start":"auto","identity_path":r"C:\Windows\ServiceProfiles\LocalService\AppData\Roaming\RustDesk\config\RustDesk.toml",
        "product_code":"{71A6889B-6003-42DB-989F-B7AE68260DDB}","source_transaction":"c".repeat(32),"created_at":100,
        "package":artifact("source.msi"),"client_inventory":artifact("client-inventory.json"),
        "agent_backup":artifact("agent-backup.exe"),"identity_backup":artifact("identity-backup.toml"),
        "config_backup":artifact("config-backup.toml"),"bootstrap_backup":artifact("bootstrap-backup.exe"),
        "bootstrap_manifest":artifact("bootstrap-backup.json"),"helper":artifact("runtime/helper.exe"),
        "total_loss":{"client_anchor":"none","original_service":{"state":"stopped","command":r#""C:\Program Files\RustDesk\RustDesk.exe" --service"#,"start":"auto"}}
    })
}

#[test]
fn total_loss_plan_stopped_and_absent_roundtrip_without_changing_schema() {
    for state in ["stopped", "absent"] {
        let mut value = total_loss_plan_value();
        if state == "absent" {
            value["total_loss"]["original_service"] = serde_json::json!({"state":"absent"});
        }
        let bytes = serde_json::to_vec(&value).unwrap();
        let plan = local_repair_plan::LocalRepairPlan::decode(&bytes)
            .expect("explicit no-client-anchor plan must decode");
        assert_eq!(serde_json::to_value(plan).unwrap(), value);
    }
}

#[test]
fn total_loss_plan_legacy_roundtrip_keeps_feature_off() {
    let mut value = total_loss_plan_value();
    value.as_object_mut().unwrap().remove("total_loss");
    let plan =
        local_repair_plan::LocalRepairPlan::decode(&serde_json::to_vec(&value).unwrap()).unwrap();
    assert_eq!(serde_json::to_value(plan).unwrap(), value);
}

#[test]
fn total_loss_legacy_executor_rejects_before_artifact_access() {
    let value = total_loss_plan_value();
    let plan =
        local_repair_plan::LocalRepairPlan::decode(&serde_json::to_vec(&value).unwrap()).unwrap();
    assert_eq!(
        local_repair::legacy_preflight(&plan).unwrap_err(),
        "total loss requires dedicated execution path"
    );
    assert_eq!(serde_json::to_value(plan).unwrap(), value);
}

#[test]
fn total_loss_pending_installer_keeps_transaction_and_never_calls_rollback() {
    use install_execution::{Backend, InstallOutcome, Validation};
    use transaction::{Evidence, Phase, Transaction};
    struct PendingBackend {
        rollback_calls: usize,
    }
    impl Backend for PendingBackend {
        fn stop_services(&mut self) -> Result<(), String> {
            Ok(())
        }
        fn apply_package(&mut self) -> Result<InstallOutcome, String> {
            Ok(InstallOutcome::Pending {
                pid: 123,
                created_at: 456,
            })
        }
        fn validate_target(&mut self) -> Result<Validation, String> {
            panic!("Pending must not validate")
        }
        fn rollback_original(&mut self) -> Result<Validation, String> {
            self.rollback_calls += 1;
            panic!("Pending must not compete with installer")
        }
    }
    let root = std::env::temp_dir().join(format!("total-loss-pending-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&root).unwrap();
    let path = root.join("transaction.json");
    let mut tx = Transaction::new(
        "a".repeat(32),
        "windows-x86_64-msi-sos".into(),
        "b".repeat(64),
        "c".repeat(64),
        100,
    )
    .unwrap();
    let proof = Evidence {
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
    };
    tx.save(&path).unwrap();
    for phase in [
        Phase::PackageVerified,
        Phase::Prepared,
        Phase::StoppingServices,
    ] {
        tx.advance(phase, &proof, 101).unwrap();
        tx.save(&path).unwrap();
    }
    let mut backend = PendingBackend { rollback_calls: 0 };
    let result = install_execution::execute(&path, &mut tx, &proof, &mut backend, 102).unwrap();
    assert_eq!(result.phase, Phase::Applying);
    assert!(result.error.unwrap().contains("installer still active"));
    let before = std::fs::read(&path).unwrap();
    let error =
        install_execution::resume(&path, &mut tx, &proof, &mut backend, None, 103).unwrap_err();
    assert!(error.contains("installer result unknown"));
    assert_eq!(std::fs::read(&path).unwrap(), before);
    assert_eq!(backend.rollback_calls, 0);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn total_loss_plan_rejects_unknown_active_disabled_and_foreign_service() {
    for service in [
        serde_json::json!({"state":"unknown"}),
        serde_json::json!({"state":"running","command":r#""C:\Program Files\RustDesk\RustDesk.exe" --service"#,"start":"auto"}),
        serde_json::json!({"state":"stopped","command":r#""C:\Program Files\RustDesk\RustDesk.exe" --service"#,"start":"disabled"}),
        serde_json::json!({"state":"stopped","command":r#""C:\foreign\RustDesk.exe" --service"#,"start":"auto"}),
    ] {
        let mut value = total_loss_plan_value();
        value["total_loss"]["original_service"] = service;
        let before = serde_json::to_vec(&value).unwrap();
        assert!(local_repair_plan::LocalRepairPlan::decode(&before).is_err());
        assert_eq!(serde_json::to_vec(&value).unwrap(), before);
    }
}

#[test]
fn r4_total_loss_verified_offline_msi_must_not_require_fallback_ready() {
    vm_only();
    absent(Path::new(NORMAL));
    assert!(
        !rescue_recovery::observed_processes().unwrap(),
        "fixture has live RustDesk process"
    );
    let root = Path::new(AGENT).join("rescue/current");
    let bundle: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("bundle.json")).unwrap()).unwrap();
    let public = rescue_assets::fixed_source_public_fixture(&bundle).unwrap();
    let (manifest, package, _, build, fallback, installation, source_tx, approval) =
        rescue_assets::local_repair_source(Some(&public)).unwrap();
    absent(&fallback);
    let key = bundle["target_key"].as_str().unwrap();
    let target =
        package_gate::verify_with_key(&manifest, key, build, "fixture-key", &public).unwrap();
    let metadata = msi_metadata::inspect(&package).unwrap();
    let registered = msi_metadata::installed_source(&metadata.product_code).unwrap();
    assert_eq!(
        registered.package_name,
        package_gate::cache_name(&target.primary, "msi").unwrap()
    );
    let _source = package_gate::lock_verified_file(&registered.source, target).unwrap();
    let agent = service_control::Contract {
        name: "RustDeskManagementAgent".into(),
        executable: Path::new(AGENT).join("rustdesk-management-agent.exe"),
        arguments: vec![
            "--service".into(),
            "--edition".into(),
            if key.ends_with("-sos") {
                "sos"
            } else {
                "standard"
            }
            .into(),
            "--package-kind".into(),
            "msi".into(),
            "--normal-install-path".into(),
            NORMAL.into(),
        ],
        start_type: windows_service::service::ServiceStartType::AutoStart,
    };
    assert_eq!(
        service_control::inspect(&agent).unwrap().state,
        ServiceState::Running
    );
    let expected_agent: install_plan::Artifact =
        serde_json::from_value(bundle["agent"].clone()).unwrap();
    assert_eq!(
        hex::encode(Sha256::digest(std::fs::read(&agent.executable).unwrap())),
        expected_agent.sha256
    );
    let scm = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT).unwrap();
    match scm.open_service(
        "RustDesk",
        ServiceAccess::QUERY_STATUS | ServiceAccess::QUERY_CONFIG,
    ) {
        Ok(service) => {
            assert_eq!(
                service.query_status().unwrap().current_state,
                ServiceState::Stopped
            );
            assert_ne!(
                service.query_config().unwrap().start_type,
                windows_service::service::ServiceStartType::Disabled
            );
        }
        Err(windows_service::Error::Winapi(e)) if e.raw_os_error() == Some(1060) => {}
        Err(e) => panic!("fixture SCM unknown: {e}"),
    }
    let transactions = Path::new(AGENT).join("transactions");
    workspace::verify(&transactions).unwrap();
    let source_id = approval
        .as_ref()
        .map(|a| a.source_id.as_str())
        .unwrap_or(&source_tx);
    assert_eq!(
        local_repair_schedule::existing_approval_fixture(
            &transactions,
            &installation,
            source_id,
            key,
            &target.sha256,
            bundle["source_reference"].as_str().unwrap()
        )
        .unwrap(),
        None,
        "fixture must not be short-circuited by prior transaction"
    );
    let mut preserved = roots();
    preserved.push(registered.source);
    let before = snapshot(&preserved);
    let services = service_snapshot();
    let result = local_repair_schedule::observed_tick(&tick(true), &public);
    // 本切片只捕获当前拒绝门槛；未来执行 GREEN 必须另行安排破坏性事务验收。
    assert_eq!(
        snapshot(&preserved),
        before,
        "RED probe changed protected assets"
    );
    assert_eq!(service_snapshot(), services, "RED probe changed services");
    assert!(matches!(result, Ok(local_repair_schedule::Decision::Prepare | local_repair_schedule::Decision::Existing(_))),
        "verified total-loss offline recovery must be admitted without fallback ready; actual={result:?}");
}

#[test]
fn r4_total_loss_disabled_tick_preserves_assets_and_services() {
    vm_only();
    absent(Path::new(NORMAL));
    let preserved = roots();
    let before = snapshot(&preserved);
    let services = service_snapshot();
    let result = local_repair_schedule::observed_tick(&tick(false), &[]);
    assert_eq!(snapshot(&preserved), before);
    assert_eq!(service_snapshot(), services);
    assert_eq!(result.unwrap(), local_repair_schedule::Decision::Deferred);
}

#[test]
fn r4_total_loss_stop_intent_preserves_assets_and_services() {
    vm_only();
    absent(Path::new(NORMAL));
    let config: hbb_common::toml::Value = hbb_common::toml::from_str(
        &std::fs::read_to_string(hbb_common::config::Config2::file()).unwrap(),
    )
    .unwrap();
    assert_eq!(
        config["options"]["stop-service"].as_str(),
        Some("Y"),
        "owner must prepare explicit stop intent"
    );
    let preserved = roots();
    let before = snapshot(&preserved);
    let services = service_snapshot();
    let result = local_repair_schedule::observed_tick(&tick(true), &[]);
    assert_eq!(snapshot(&preserved), before);
    assert_eq!(service_snapshot(), services);
    assert_eq!(result.unwrap(), local_repair_schedule::Decision::Deferred);
}

#[test]
fn r4_total_loss_unknown_normal_reparse_preserves_assets_and_services() {
    use std::os::windows::fs::MetadataExt;
    vm_only();
    assert_ne!(
        std::fs::symlink_metadata(NORMAL).unwrap().file_attributes() & 0x400,
        0,
        "owner must prepare reparse observation fixture"
    );
    let preserved = roots();
    let before = snapshot(&preserved);
    let services = service_snapshot();
    let result = local_repair_schedule::observed_tick(&tick(true), &[]);
    assert_eq!(snapshot(&preserved), before);
    assert_eq!(service_snapshot(), services);
    assert_eq!(
        result.unwrap_err(),
        "automatic local repair normal reparse refused"
    );
}
