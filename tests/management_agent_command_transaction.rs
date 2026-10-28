// install_plan.rs 引用 `super::system_dirs`，该模块在生产 crate 里只在 windows 下编译。
#![cfg(windows)]
#[path = "../src/platform/windows_system_dirs.rs"]
#[allow(dead_code)]
mod system_dirs;
#[path = "../src/management_agent/command_transaction.rs"] mod command_transaction;
#[path = "../src/management_agent/install_plan.rs"] mod install_plan;
#[path = "../src/management_agent/transaction.rs"] mod transaction;
#[path = "../src/management_agent/state_store.rs"] mod state_store;
#[cfg(windows)]
#[path = "../src/management_agent/workspace.rs"] mod workspace;

use install_plan::{Artifact, InstallPlan, OriginalService, Trigger};
use transaction::{Phase, Transaction};

fn fixture() -> (serde_json::Value, serde_json::Value, InstallPlan, Transaction, serde_json::Value) {
    let artifact = |path: &str| Artifact {relative_path:path.into(), size:10, sha256:"e".repeat(64)};
    let plan=InstallPlan {schema:1,transaction_id:"a".repeat(32),trigger:Trigger::RemoteCommand,
        command_id:Some("a".repeat(32)),installation_id:"b".repeat(32),identity_fingerprint:"c".repeat(64),
        target_key:"windows-x86_64-msi-standard".into(),original_version:"1.5.3".into(),original_build_seq:2026100502,
        target_version:"1.5.4".into(),target_build_seq:2026100503,normal_install_path:r"C:\Program Files\RustDesk".into(),
        package:artifact("package.msi"),client_backup:artifact("backups/client.json"),agent_backup:artifact("backups/agent.exe"),
        identity_backup:artifact("backups/identity.toml"),helper:artifact("runtime/helper.exe"),helper_version:"1.5.3".into(),
        resume_task:format!("RustDeskManagementAgentResume-{}","a".repeat(32)),created_at:1000,
        services:vec![OriginalService{name:"RustDesk".into(),command:r#""C:\Program Files\RustDesk\RustDesk.exe" --service"#.into(),account:"LocalSystem".into(),start_type:"auto".into(),running:true},
            OriginalService{name:"RustDeskManagementAgent".into(),command:r#"C:\ProgramData\RustDesk\ManagementAgent\rustdesk-management-agent.exe --service --edition standard --package-kind msi --normal-install-path "C:\Program Files\RustDesk""#.into(),account:"LocalSystem".into(),start_type:"auto".into(),running:true}]};
    let mut tx=Transaction::new(plan.transaction_id.clone(),plan.target_key.clone(),plan.package.sha256.clone(),plan.identity_fingerprint.clone(),1000).unwrap();
    tx.phase=Phase::Committed;tx.revision=7;tx.updated_at=1100;
    let wire=serde_json::json!({"id":"a".repeat(32),"digest":"d".repeat(64),"type":"package.update","client_id":"device","client_uuid":"uuid","agent_installation_id":"b".repeat(32),"enrollment_generation":3,"args":{"target_key":plan.target_key,"target_build_seq":plan.target_build_seq,"package_id":"f".repeat(64),"allow_downgrade":false}});
    let binding=serde_json::json!({"schema":1,"command_id":wire["id"],"command_digest":wire["digest"],"command_type":wire["type"],"client_id":"device","client_uuid":"uuid","installation_id":plan.installation_id,"enrollment_generation":3,"package_id":wire["args"]["package_id"],"plan_sha256":hex::encode(sha2::Sha256::digest(serde_json::to_vec(&plan).unwrap()))});
    let result=serde_json::json!({"schema":1,"transaction_id":plan.transaction_id,"command_id":plan.command_id,"phase":"committed","error":null,"reboot_required":false});
    (wire,binding,plan,tx,result)
}
use sha2::Digest;

#[test]
fn terminal_result_matches_command_generation_plan_and_transaction_without_mutation() {
    let (wire,binding,plan,mut tx,mut result)=fixture();
    plan.validate().unwrap();
    assert_eq!(command_transaction::terminal_status(&wire,&binding,&plan,&tx,&result).unwrap(),Some("succeeded"));
    for (phase,name,status) in [(Phase::RolledBack,"rolled_back","rolled_back"),(Phase::RollbackFailed,"rollback_failed","rollback_failed")] {
        tx.phase=phase;result["phase"]=name.into();result["error"]="fixture failed".into();
        assert_eq!(command_transaction::terminal_status(&wire,&binding,&plan,&tx,&result).unwrap(),Some(status));
    }
    tx.phase=Phase::Applying;result["phase"]="applying".into();
    assert_eq!(command_transaction::terminal_status(&wire,&binding,&plan,&tx,&result).unwrap(),None);
}

#[test]
fn substituted_receipts_or_reboot_pending_never_mark_a_command_succeeded() {
    let (wire,binding,plan,tx,result)=fixture();
    for key in ["command_id","command_digest","command_type","client_id","client_uuid","installation_id","enrollment_generation","package_id","plan_sha256"] {
        let mut changed=binding.clone();changed[key]="foreign".into();
        assert!(command_transaction::terminal_status(&wire,&changed,&plan,&tx,&result).is_err(),"{key}");
        assert_eq!(binding,fixture().1);
    }
    for key in ["transaction_id","command_id","phase","reboot_required","schema"] {
        let mut changed=result.clone();changed[key]="wrong".into();
        assert!(command_transaction::terminal_status(&wire,&binding,&plan,&tx,&changed).is_err(),"{key}");
    }
    let mut changed=plan.clone();changed.target_build_seq+=1;
    assert!(command_transaction::terminal_status(&wire,&binding,&changed,&tx,&result).is_err());
    let mut changed=tx.clone();changed.package_sha256="0".repeat(64);
    assert!(command_transaction::terminal_status(&wire,&binding,&plan,&changed,&result).is_err());
    let mut changed=result.clone();changed["reboot_required"]=true.into();
    assert!(command_transaction::terminal_status(&wire,&binding,&plan,&tx,&changed).is_err());
}

#[cfg(windows)]
#[test]
fn protected_result_readback_requires_all_records_and_preserves_failure_evidence() {
    let (wire, binding, plan, tx, result) = fixture();
    let root = std::env::temp_dir().join(format!("agent-result-readback-{}", std::process::id()));
    assert!(command_transaction::read_terminal(&root, &wire).is_err());
    assert!(!root.exists());
    workspace::create(&root).unwrap();
    for (name, value) in [("command-binding.json", binding.clone()), ("prepared.json", serde_json::to_value(&plan).unwrap()),
        ("transaction.json", serde_json::to_value(&tx).unwrap())] {
        std::fs::write(root.join(name), serde_json::to_vec(&value).unwrap()).unwrap();
    }
    assert_eq!(command_transaction::read_terminal(&root, &wire).unwrap(), None);
    let path = root.join("execution-result.json");
    let bytes = serde_json::to_vec(&result).unwrap();
    std::fs::write(&path, &bytes).unwrap();
    assert_eq!(command_transaction::read_terminal(&root, &wire).unwrap(), Some("succeeded"));
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    let binding_path = root.join("command-binding.json");
    let mut changed = binding;
    changed["enrollment_generation"] = 4.into();
    let mismatched = serde_json::to_vec(&changed).unwrap();
    std::fs::write(&binding_path, &mismatched).unwrap();
    assert!(command_transaction::read_terminal(&root, &wire).unwrap_err().contains("binding"));
    assert_eq!(std::fs::read(&binding_path).unwrap(), mismatched);
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    std::fs::write(&path, b"broken-result").unwrap();
    assert!(command_transaction::read_terminal(&root, &wire).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), b"broken-result");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn incomplete_or_wrongly_typed_terminal_results_cannot_authorize_command_completion() {
    let (wire, binding, plan, mut tx, mut result) = fixture();
    for (phase, value) in [
        (Phase::Committed, "committed"),
        (Phase::RolledBack, "rolled_back"),
        (Phase::RollbackFailed, "rollback_failed"),
    ] {
        tx.phase = phase;
        result["phase"] = value.into();
        for key in ["error", "schema", "transaction_id", "command_id", "phase", "reboot_required"] {
            let mut incomplete = result.clone();
            incomplete.as_object_mut().unwrap().remove(key);
            let before = incomplete.clone();
            assert_eq!(
                command_transaction::terminal_status(&wire, &binding, &plan, &tx, &incomplete),
                Err("command transaction result malformed".into()),
                "{value}: missing {key}"
            );
            assert_eq!(incomplete, before);
        }
        for invalid in [serde_json::json!(false), serde_json::json!(123), serde_json::json!([]), serde_json::json!({})] {
            let mut malformed = result.clone();
            malformed["error"] = invalid;
            let before = malformed.clone();
            assert_eq!(
                command_transaction::terminal_status(&wire, &binding, &plan, &tx, &malformed),
                Err("command transaction result malformed".into())
            );
            assert_eq!(malformed, before);
        }
    }
}
