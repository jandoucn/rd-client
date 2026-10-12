#[path = "../src/management_agent/handoff.rs"]
mod handoff;

use handoff::{Handoff, Owner};

fn owner(pid: u32, created_at: u64) -> Owner {
    Owner { pid, created_at }
}

#[test]
fn readiness_must_bind_transaction_plan_nonce_and_exact_helper_process() {
    let parent = owner(100, 1000);
    let helper = owner(200, 2000);
    let intent = Handoff::new(
        "a".repeat(32),
        "b".repeat(64),
        "c".repeat(64),
        parent.clone(),
        helper.clone(),
        3000,
    )
    .unwrap();
    let before = intent.clone();
    for index in 0..5 {
        let mut ready = intent.ready_receipt(3001).unwrap();
        match index {
            0 => ready.transaction_id = "d".repeat(32),
            1 => ready.plan_sha256 = "e".repeat(64),
            2 => ready.nonce = "f".repeat(64),
            3 => ready.helper.pid += 1,
            _ => ready.helper.created_at += 1,
        }
        assert!(intent.verify_ready(&ready, 3002).is_err(), "index={index}");
        assert_eq!(intent, before);
    }
    intent
        .verify_ready(&intent.ready_receipt(3001).unwrap(), 3002)
        .unwrap();
}

#[test]
fn live_parent_reused_pid_expired_or_future_receipts_do_not_authorize_takeover() {
    let intent = Handoff::new(
        "a".repeat(32),
        "b".repeat(64),
        "c".repeat(64),
        owner(100, 1000),
        owner(200, 2000),
        3000,
    )
    .unwrap();
    let ready = intent.ready_receipt(3001).unwrap();
    assert!(intent
        .may_take_over(&ready, &owner(200, 2000), true, 3002)
        .is_err());
    assert!(intent
        .may_take_over(&ready, &owner(200, 2001), false, 3002)
        .is_err());
    assert!(intent
        .may_take_over(&ready, &owner(201, 2000), false, 3002)
        .is_err());
    assert!(intent
        .may_take_over(&ready, &owner(200, 2000), false, 3300)
        .is_err());
    assert!(intent
        .may_take_over(&ready, &owner(200, 2000), false, 2999)
        .is_err());
    intent
        .may_take_over(&ready, &owner(200, 2000), false, 3002)
        .unwrap();
    let mut future = ready.clone();
    future.ready_at = 3100;
    assert!(intent.verify_ready(&future, 3002).is_err());
}

#[test]
fn malformed_owners_keys_and_unknown_schema_are_rejected_without_repair() {
    assert!(Handoff::new(
        "a".repeat(32),
        "b".repeat(64),
        "c".repeat(64),
        owner(0, 1000),
        owner(2, 2000),
        3000
    )
    .is_err());
    assert!(Handoff::new(
        "a".repeat(32),
        "b".repeat(64),
        "c".repeat(64),
        owner(1, 0),
        owner(2, 2000),
        3000
    )
    .is_err());
    assert!(Handoff::new(
        "a".repeat(32),
        "b".repeat(64),
        "c".repeat(64),
        owner(1, 1000),
        owner(1, 1000),
        3000
    )
    .is_err());
    assert!(Handoff::new(
        "bad".into(),
        "b".repeat(64),
        "c".repeat(64),
        owner(1, 1000),
        owner(2, 2000),
        3000
    )
    .is_err());
    let valid = Handoff::new(
        "a".repeat(32),
        "b".repeat(64),
        "c".repeat(64),
        owner(1, 1000),
        owner(2, 2000),
        3000,
    )
    .unwrap();
    let mut json = serde_json::to_value(&valid).unwrap();
    json["schema"] = 2.into();
    assert!(Handoff::decode(&serde_json::to_vec(&json).unwrap()).is_err());
    json["schema"] = 1.into();
    json["command"] = "arbitrary".into();
    assert!(Handoff::decode(&serde_json::to_vec(&json).unwrap()).is_err());
}

#[cfg(windows)]
#[test]
fn native_process_identity_uses_exact_creation_time_and_retained_handle() {
    let process = handoff::ObservedProcess::open(std::process::id()).unwrap();
    let identity = process.owner().clone();
    assert_eq!(identity.pid, std::process::id());
    assert!(identity.created_at > 0);
    assert!(process.is_alive().unwrap());
    assert!(handoff::ObservedProcess::owner_alive(&identity).unwrap());
    let reused = owner(identity.pid, identity.created_at + 1);
    assert!(!handoff::ObservedProcess::owner_alive(&reused).unwrap());
    assert!(handoff::ObservedProcess::owner_alive(&owner(0, 1)).is_err());
    assert_eq!(
        handoff::ObservedProcess::open(identity.pid)
            .unwrap()
            .owner(),
        &identity
    );
    assert!(handoff::ObservedProcess::open(0).is_err());
    let helper = Owner {
        pid: identity.pid + 1,
        created_at: identity.created_at + 1,
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let handshake = Handoff::new(
        "a".repeat(32),
        "b".repeat(64),
        "c".repeat(64),
        identity,
        helper,
        now,
    )
    .unwrap();
    assert!(handshake.ready_receipt(now).is_ok());
}

#[cfg(windows)]
#[test]
fn terminated_process_handle_reports_exit_without_confusing_reused_pid() {
    let executable = std::env::var_os("SystemRoot")
        .map(std::path::PathBuf::from)
        .unwrap()
        .join(r"System32\cmd.exe");
    let mut child = std::process::Command::new(executable)
        .args(["/c", "ping.exe -n 30 127.0.0.1 >nul"])
        .spawn()
        .unwrap();
    let observed = handoff::ObservedProcess::open(child.id()).unwrap();
    assert!(observed.is_alive().unwrap());
    child.kill().unwrap();
    assert!(!child.wait().unwrap().success());
    assert!(!observed.is_alive().unwrap());
    assert!(!handoff::ObservedProcess::owner_alive(observed.owner()).unwrap());
    assert_eq!(observed.owner().pid, child.id());
}

#[test]
fn explicit_lock_release_allows_live_parent_without_allowing_unbound_takeover() {
    let intent = Handoff::new(
        "a".repeat(32),
        "b".repeat(64),
        "c".repeat(64),
        owner(100, 1000),
        owner(200, 2000),
        3000,
    )
    .unwrap();
    let ready = intent.ready_receipt(3001).unwrap();
    let released = intent.release_receipt(&ready, 3002).unwrap();
    intent
        .verify_release(&ready, &released, &owner(200, 2000), 3003)
        .unwrap();
    for index in 0..5 {
        let mut bad = released.clone();
        match index {
            0 => bad.parent.pid += 1,
            1 => bad.parent.created_at += 1,
            2 => bad.nonce = "d".repeat(64),
            3 => bad.plan_sha256 = "e".repeat(64),
            _ => bad.released_at = 3000,
        }
        assert!(
            intent
                .verify_release(&ready, &bad, &owner(200, 2000), 3003)
                .is_err(),
            "index={index}"
        );
    }
    assert!(intent
        .verify_release(&ready, &released, &owner(200, 2001), 3003)
        .is_err());
    assert!(intent
        .verify_release(&ready, &released, &owner(200, 2000), 3300)
        .is_err());
}
