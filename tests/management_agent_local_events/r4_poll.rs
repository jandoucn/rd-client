use super::local_repair::r4_finalize_tests::{
    poll_dispatch as dispatch, poll_evidence as evidence,
    poll_finish_execution as finish_execution, poll_save as save, Fault, Files, Fixture,
    BACKEND_CALLS,
};
use crate::management_agent::{
    self as agent, install_execution,
    local_events::{
        Action, LocalEvents, Operation, Outcome, PathKey, Phase as EventPhase, Record, RecoveryCode,
    },
    local_recovery_events,
    transaction::{Phase, Transaction},
    workspace,
};
use hbb_common::{
    base64::{engine::general_purpose::STANDARD, Engine as _},
    sodiumoxide::crypto::sign,
};
use sha2::{Digest, Sha256};
use std::{
    cell::Cell,
    io::{Read, Write},
    net::TcpListener,
    os::windows::fs::OpenOptionsExt,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

struct Api {
    address: String,
    stop: Arc<AtomicBool>,
    bodies: Arc<Mutex<Vec<serde_json::Value>>>,
    worker: Option<std::thread::JoinHandle<()>>,
}
impl Api {
    fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = format!("http://{}", listener.local_addr().unwrap());
        listener.set_nonblocking(true).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let end = stop.clone();
        let bodies = Arc::new(Mutex::new(Vec::new()));
        let received = bodies.clone();
        let worker = std::thread::spawn(move || {
            while !end.load(Ordering::SeqCst) {
                let (mut stream, _) = match listener.accept() {
                    Ok(v) => v,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(5));
                        continue;
                    }
                    Err(e) => panic!("{e}"),
                };
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut header = Vec::new();
                while !header.ends_with(b"\r\n\r\n") {
                    let mut byte = [0];
                    stream.read_exact(&mut byte).unwrap();
                    header.push(byte[0]);
                    assert!(header.len() < 16384);
                }
                let text = String::from_utf8(header).unwrap();
                assert!(text.starts_with("POST /rd/update/v1/agent/events HTTP/1.1"));
                let headers = text
                    .lines()
                    .skip(1)
                    .filter_map(|l| l.split_once(':'))
                    .map(|(k, v)| (k.to_ascii_lowercase(), v.trim().to_owned()))
                    .collect::<std::collections::HashMap<_, _>>();
                let mut bytes = vec![0; headers["content-length"].parse::<usize>().unwrap()];
                stream.read_exact(&mut bytes).unwrap();
                let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
                let canonical = agent::operations::canonical_request(
                    "POST",
                    "/rd/update/v1/agent/events",
                    (
                        body["client_id"].as_str().unwrap(),
                        body["client_uuid"].as_str().unwrap(),
                    ),
                    headers["x-rustdesk-device-timestamp"].parse().unwrap(),
                    &headers["x-rustdesk-device-nonce"],
                    "",
                    &bytes,
                );
                let public = sign::PublicKey::from_slice(
                    &STANDARD
                        .decode(&headers["x-rustdesk-device-public-key"])
                        .unwrap(),
                )
                .unwrap();
                let signature = sign::Signature::from_bytes(
                    &STANDARD
                        .decode(&headers["x-rustdesk-device-signature"])
                        .unwrap(),
                )
                .unwrap();
                assert!(sign::verify_detached(
                    &signature,
                    canonical.as_bytes(),
                    &public
                ));
                let reply = serde_json::json!({"ok":true,"event_id":body["event_id"]}).to_string();
                received.lock().unwrap().push(body);
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    reply.len(),
                    reply
                )
                .unwrap();
            }
        });
        Self {
            address,
            stop,
            bodies,
            worker: Some(worker),
        }
    }
}
impl Drop for Api {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        self.worker.take().unwrap().join().unwrap();
    }
}

struct Case {
    f: Fixture,
    tx: Transaction,
    paths: agent::AgentPaths,
    identity: agent::identity::DeviceIdentity,
    api: Api,
    key: Vec<u8>,
}

mod full_loss {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/management_agent_local_events/full_loss.rs"
    ));
}
impl Case {
    fn new(phase: Phase) -> Self {
        assert_eq!(std::env::var("COMPUTERNAME").unwrap(), "RDLAB-WIN11");
        let mut f = Fixture::new();
        hbb_common::sodiumoxide::init().unwrap();
        let (public, secret) = sign::gen_keypair();
        let identity = agent::identity::DeviceIdentity {
            id: f.context.binding.client_id.clone(),
            uuid: f.context.binding.client_uuid.clone(),
            public,
            secret,
        };
        let identity_bytes=hbb_common::toml::to_string(&serde_json::json!({"id":identity.id,"key_pair":[identity.secret.as_ref(),identity.public.as_ref()]})).unwrap().into_bytes();
        std::fs::write(f.root.join("identity-backup.toml"), &identity_bytes).unwrap();
        f.plan.identity_backup.size = identity_bytes.len() as u64;
        f.plan.identity_backup.sha256 = hex::encode(Sha256::digest(&identity_bytes));
        let (release_public, release_secret) = sign::gen_keypair();
        let package = b"signed fixture, never installed";
        std::fs::write(f.root.join("fixture.msi"), package).unwrap();
        f.plan.package.size = package.len() as u64;
        f.plan.package.sha256 = hex::encode(Sha256::digest(package));
        let mut manifest = hbb_common::UpdateManifest {
            schema: 2,
            product: "rustdesk-yan".into(),
            edition: "multi".into(),
            channel: "stable".into(),
            ..Default::default()
        };
        let mut target = hbb_common::UpdateTarget {
            version: f.plan.version.clone(),
            build_seq: f.plan.build_seq,
            source_commit: "fixture".into(),
            primary: "https://download.yan.life/rustdesk/stable/fixture.msi".into(),
            size: f.plan.package.size,
            sha256: f.plan.package.sha256.clone(),
            signature_key_id: "fixture-key".into(),
            ..Default::default()
        };
        target.signature = STANDARD.encode(sign::sign_detached(
            &base::update::update_signature_payload(&manifest, &f.plan.target_key, &target)
                .unwrap(),
            &release_secret,
        ));
        manifest.targets.insert(f.plan.target_key.clone(), target);
        save(&f.root.join("approved-manifest.json"), &manifest).unwrap();
        save(&f.root.join("local-repair-plan.json"), &f.plan).unwrap();
        f.handoff.plan_sha256 = hex::encode(Sha256::digest(
            std::fs::read(f.root.join("local-repair-plan.json")).unwrap(),
        ));
        save(&f.root.join("handoff.json"), &f.handoff).unwrap();
        let ready = f.handoff.ready_receipt(101).unwrap();
        save(&f.root.join("local-ready.json"), &ready).unwrap();
        save(
            &f.root.join("local-release.json"),
            &f.handoff.release_receipt(&ready, 102).unwrap(),
        )
        .unwrap();
        std::fs::remove_file(f.state.join("maintenance.json")).unwrap();
        agent::maintenance::claim(
            &f.state,
            &f.plan.installation_id,
            &f.plan.transaction_id,
            &f.handoff.plan_sha256,
            &f.handoff.nonce,
        )
        .unwrap();
        workspace::create(&f.root.join("poll")).unwrap();
        let paths = agent::AgentPaths::from_program_data(f.root.join("poll"), agent::Edition::Sos);
        workspace::create(&paths.root).unwrap();
        workspace::create(&paths.agent).unwrap();
        workspace::create(&paths.state).unwrap();
        workspace::create(&paths.state.join("local-events")).unwrap();
        workspace::create(&paths.agent.join("transactions")).unwrap();
        f.context.path = paths.state.join("local-events/events.json");
        LocalEvents::open(&f.context.path)
            .unwrap()
            .remember_binding(f.context.binding.clone())
            .unwrap();
        save(
            &paths.state.join("agent.json"),
            &serde_json::json!({"agent_installation_id":f.plan.installation_id}),
        )
        .unwrap();
        let mut tx = f.transaction();
        if phase == Phase::RollbackFailed {
            let operation = Operation {
                operation_id: tx.id.clone(),
                code: RecoveryCode::R4,
                action: Action::RepairInstallation,
            };
            LocalEvents::open(&f.context.path)
                .unwrap()
                .record_bound_at(
                    &operation,
                    Record {
                        phase: EventPhase::Started,
                        attempt_seq: 1,
                        outcome: Outcome::Running,
                        path_key: PathKey::NormalClient,
                        error_code: None,
                    },
                    &f.context.binding,
                    104,
                )
                .unwrap();
            tx.advance(Phase::RollingBack, &evidence(), 104).unwrap();
            tx.save(&f.root.join("transaction.json")).unwrap();
            tx.advance(phase, &evidence(), 105).unwrap();
            tx.save(&f.root.join("transaction.json")).unwrap();
            let result = finish_execution(
                &f.root,
                &f.plan,
                &f.handoff,
                &f.state,
                Ok(install_execution::ExecutionResult {
                    phase,
                    error: Some("original; rollback failed".into()),
                    reboot_required: false,
                }),
                || panic!("manual reconciliation"),
            );
            assert!(result.is_err());
        } else {
            let mut backend = Files {
                fixture: &f,
                fault: Fault::WriterBusy,
                rollback: phase == Phase::RolledBack,
                lock: None,
                calls: Cell::new(0),
            };
            let result = local_recovery_events::with_context_fixture(f.context.clone(), || {
                dispatch(
                    &f.root.join("transaction.json"),
                    &mut tx,
                    &evidence(),
                    &mut backend,
                    104,
                    false,
                    None,
                    || Ok(()),
                )
            });
            finish_execution(&f.root, &f.plan, &f.handoff, &f.state, result, || {
                f.remove_task()
            })
            .unwrap();
            backend.clear_fault();
            assert!(!f.root.join("task.fixture").exists());
            assert!(!f.state.join("maintenance.json").exists());
        }
        let directory = paths.agent.join("transactions").join(&tx.id);
        workspace::create(&directory).unwrap();
        for name in [
            "transaction.json",
            "local-repair-plan.json",
            "handoff.json",
            "local-ready.json",
            "local-release.json",
            "local-result.json",
            "approved-manifest.json",
            "identity-backup.toml",
            "fixture.msi",
        ] {
            std::fs::copy(f.root.join(name), directory.join(name)).unwrap();
        }
        // ACK Started before recovery: the durable operation, not the pending queue, is the source.
        let mut log = LocalEvents::open(&f.context.path).unwrap();
        for e in log.pending_events().to_vec() {
            log.acknowledge(&e.event_id, &e.digest).unwrap();
        }
        Self {
            f,
            tx,
            paths,
            identity,
            api: Api::new(),
            key: release_public.as_ref().to_vec(),
        }
    }
    fn directory(&self) -> PathBuf {
        self.paths.agent.join("transactions").join(&self.tx.id)
    }
    fn poll(&self, generation: u64) {
        let calls = BACKEND_CALLS.with(Cell::get);
        agent::local_recovery_events::REPORT_KEY
            .with(|slot| *slot.borrow_mut() = Some(self.key.clone()));
        let mut binding = self.f.context.binding.clone();
        binding.enrollment_generation = generation;
        agent::local_event_runtime::poll(
            &self.identity,
            &self.api.address,
            &serde_json::to_value(binding).unwrap(),
            &self.paths,
        );
        assert_eq!(
            BACKEND_CALLS.with(Cell::get),
            calls,
            "poll must make zero backend calls"
        );
        agent::local_recovery_events::REPORT_KEY.with(|slot| *slot.borrow_mut() = None);
    }
    fn assert_reconciled(&self) {
        let tx = std::fs::read(self.directory().join("transaction.json")).unwrap();
        let result = std::fs::read(self.directory().join("local-result.json")).unwrap();
        self.poll(1);
        let log = LocalEvents::open(&self.f.context.path).unwrap();
        assert_eq!(
            log.operation_record(&self.tx.id).unwrap().2.phase,
            EventPhase::Terminal,
            "normal poll must reconcile R4"
        );
        {
            let bodies = self.api.bodies.lock().unwrap();
            assert_eq!(bodies.len(), 1);
            assert_eq!(bodies[0]["result"]["operation_id"], self.tx.id);
            assert!(bodies[0]["command_id"].is_null());
            assert_eq!(
                bodies[0]["status"],
                match self.tx.phase {
                    Phase::Committed => "succeeded",
                    Phase::RolledBack => "rolled_back",
                    _ => "rollback_failed",
                }
            );
        }
        self.poll(1);
        assert_eq!(self.api.bodies.lock().unwrap().len(), 1);
        assert_eq!(
            std::fs::read(self.directory().join("transaction.json")).unwrap(),
            tx
        );
        assert_eq!(
            std::fs::read(self.directory().join("local-result.json")).unwrap(),
            result
        );
        if self.tx.phase == Phase::RollbackFailed {
            assert!(self.f.state.join("maintenance.json").exists());
            assert!(self.f.root.join("task.fixture").exists());
        }
    }
}
#[test]
fn r4_poll_committed_after_cleanup() {
    Case::new(Phase::Committed).assert_reconciled();
}
#[test]
fn r4_poll_rolled_back_after_cleanup() {
    Case::new(Phase::RolledBack).assert_reconciled();
}
#[test]
fn r4_poll_rollback_failed_keeps_manual_reconciliation() {
    Case::new(Phase::RollbackFailed).assert_reconciled();
}
#[test]
fn r4_poll_old_generation_preserved() {
    let c = Case::new(Phase::Committed);
    c.poll(2);
    let log = LocalEvents::open(&c.f.context.path).unwrap();
    assert_eq!(
        log.operation_record(&c.tx.id).unwrap().2.phase,
        EventPhase::Terminal
    );
    assert_eq!(log.pending_events()[0].payload["enrollment_generation"], 1);
    assert!(c.api.bodies.lock().unwrap().is_empty());
}
#[test]
fn r4_poll_locked_then_released() {
    let c = Case::new(Phase::Committed);
    let lock = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .share_mode(0)
        .open(c.f.context.path.with_extension("lock"))
        .unwrap();
    c.poll(1);
    assert_eq!(
        LocalEvents::open(&c.f.context.path)
            .unwrap()
            .operation_record(&c.tx.id)
            .unwrap()
            .2
            .phase,
        EventPhase::Started
    );
    drop(lock);
    c.assert_reconciled();
}
#[test]
fn r4_poll_foreign_or_missing_evidence_retains_started() {
    for name in [
        "transaction.json",
        "local-repair-plan.json",
        "local-result.json",
        "approved-manifest.json",
        "local-release.json",
        "identity-backup.toml",
    ] {
        let c = Case::new(Phase::Committed);
        let path = c.directory().join(name);
        if name == "local-result.json" {
            std::fs::remove_file(&path).unwrap();
        } else if name == "identity-backup.toml" {
            std::fs::write(&path, b"foreign identity").unwrap();
        } else {
            let mut v: serde_json::Value =
                serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
            match name {
                "transaction.json" => v["id"] = "f".repeat(32).into(),
                "local-repair-plan.json" => v["installation_id"] = "f".repeat(32).into(),
                "local-release.json" => v["nonce"] = "f".repeat(64).into(),
                _ => {
                    v["targets"]["windows-x86_64-msi-sos"]["signature"] =
                        STANDARD.encode([0u8; 64]).into()
                }
            };
            save(&path, &v).unwrap();
        }
        let before = std::fs::read(&c.f.context.path).unwrap();
        c.poll(1);
        assert_eq!(std::fs::read(&c.f.context.path).unwrap(), before);
        assert!(c.api.bodies.lock().unwrap().is_empty());
    }
}

#[test]
fn r4_poll_invalid_enrollment_never_connects() {
    let c = Case::new(Phase::Committed);
    let before = std::fs::read(&c.f.context.path).unwrap();
    c.poll(0);
    assert!(c.api.bodies.lock().unwrap().is_empty());
    assert_eq!(std::fs::read(&c.f.context.path).unwrap(), before);
    let mut binding = c.f.context.binding.clone();
    binding.client_id = "foreign-device".into();
    agent::local_event_runtime::poll(
        &c.identity,
        &c.api.address,
        &serde_json::to_value(binding).unwrap(),
        &c.paths,
    );
    assert!(c.api.bodies.lock().unwrap().is_empty());
    assert_eq!(std::fs::read(&c.f.context.path).unwrap(), before);
}

#[test]
fn r4_poll_foreign_result_is_not_terminal_authority() {
    let c = Case::new(Phase::RolledBack);
    let path = c.directory().join("local-result.json");
    let mut value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    value["transaction_id"] = "f".repeat(32).into();
    save(&path, &value).unwrap();
    let before = std::fs::read(&c.f.context.path).unwrap();
    c.poll(1);
    assert_eq!(std::fs::read(&c.f.context.path).unwrap(), before);
    assert!(c.api.bodies.lock().unwrap().is_empty());
}

#[test]
fn r4_poll_reconciles_at_most_eight_then_next_batch() {
    let c = Case::new(Phase::Committed);
    for index in 1..=8 {
        let id = format!("{index:032x}");
        let directory = c.paths.agent.join("transactions").join(&id);
        workspace::create(&directory).unwrap();
        for entry in std::fs::read_dir(c.directory()).unwrap() {
            let path = entry.unwrap().path();
            std::fs::copy(&path, directory.join(path.file_name().unwrap())).unwrap();
        }
        let mut tx = c.tx.clone();
        tx.id = id.clone();
        save(&directory.join("transaction.json"), &tx).unwrap();
        let mut plan = c.f.plan.clone();
        plan.transaction_id = id.clone();
        save(&directory.join("local-repair-plan.json"), &plan).unwrap();
        let mut handoff = c.f.handoff.clone();
        handoff.transaction_id = id.clone();
        handoff.plan_sha256 = hex::encode(Sha256::digest(
            std::fs::read(directory.join("local-repair-plan.json")).unwrap(),
        ));
        save(&directory.join("handoff.json"), &handoff).unwrap();
        let ready = handoff.ready_receipt(101).unwrap();
        save(&directory.join("local-ready.json"), &ready).unwrap();
        save(
            &directory.join("local-release.json"),
            &handoff.release_receipt(&ready, 102).unwrap(),
        )
        .unwrap();
        save(&directory.join("local-result.json"),&serde_json::json!({"schema":1,"transaction_id":id,"phase":"committed","error":null,"reboot_required":false})).unwrap();
        let mut log = LocalEvents::open(&c.f.context.path).unwrap();
        let event = log
            .record_bound_at(
                &Operation {
                    operation_id: id,
                    code: RecoveryCode::R4,
                    action: Action::RepairInstallation,
                },
                Record {
                    phase: EventPhase::Started,
                    attempt_seq: 1,
                    outcome: Outcome::Running,
                    path_key: PathKey::NormalClient,
                    error_code: None,
                },
                &c.f.context.binding,
                104,
            )
            .unwrap();
        log.acknowledge(&event.event_id, &event.digest).unwrap();
    }
    c.poll(1);
    assert_eq!(c.api.bodies.lock().unwrap().len(), 8);
    assert_eq!(
        LocalEvents::open(&c.f.context.path)
            .unwrap()
            .active_repairs()
            .len(),
        1
    );
    c.poll(1);
    assert_eq!(c.api.bodies.lock().unwrap().len(), 9);
    c.poll(1);
    assert_eq!(c.api.bodies.lock().unwrap().len(), 9);
}
