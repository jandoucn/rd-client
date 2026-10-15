use super::*;
use agent::local_repair_plan::total_loss::{
    Binding as PreservationBinding, OriginalService, PreservationReceipt, Prestate,
};
use std::process::{Command, Stdio};

fn pending_unreported_case(kind: &str) {
    let f = FullLoss::new(false, false);
    let receipt = f.c.directory().join("local-event-unreported.json");
    let transaction = std::fs::read(f.c.directory().join("transaction.json")).unwrap();
    if kind == "capacity" {
        for index in 0..65 {
            workspace::create(
                &f.c.paths
                    .agent
                    .join("transactions")
                    .join(format!("{index:032x}")),
            )
            .unwrap();
        }
    } else if kind == "reparse" {
        let foreign = f.c.f.root.join("foreign-audit.json");
        std::fs::write(&foreign, b"foreign evidence remains").unwrap();
        std::os::windows::fs::symlink_file(&foreign, &receipt).unwrap();
    } else if kind == "malformed" {
        std::fs::write(&receipt, b"{broken audit evidence").unwrap();
    } else {
        let mut value = serde_json::json!({"schema":1,"binding":f.c.f.context.binding,
            "transaction_id":f.c.tx.id,"plan_sha256":f.c.f.handoff.plan_sha256,
            "handoff_sha256":hex::encode(Sha256::digest(std::fs::read(f.c.directory().join("handoff.json")).unwrap()))});
        if kind == "foreign" {
            value["binding"]["agent_installation_id"] = "f".repeat(32).into();
        }
        save(&receipt, &value).unwrap();
        if kind == "evidence" {
            std::fs::remove_file(f.c.directory().join("local-result.json")).unwrap();
        }
    }
    let original = std::fs::read(&receipt).ok();
    for index in 1..=2 {
        let op = Operation {
            operation_id: format!("{index:032x}"),
            code: RecoveryCode::R1,
            action: Action::StartService,
        };
        LocalEvents::open(&f.c.f.context.path)
            .unwrap()
            .record_bound_at(
                &op,
                Record {
                    phase: EventPhase::Started,
                    attempt_seq: 1,
                    outcome: Outcome::Running,
                    path_key: PathKey::RustDeskService,
                    error_code: None,
                },
                &f.c.f.context.binding,
                100,
            )
            .unwrap();
        f.poll(1);
        assert_eq!(
            f.api.rows().as_array().unwrap().len(),
            index,
            "unreported scan must not block legitimate pending"
        );
        assert_eq!(std::fs::read(&receipt).ok(), original);
        assert_eq!(
            std::fs::read(f.c.directory().join("transaction.json")).unwrap(),
            transaction
        );
        assert!(LocalEvents::open(&f.c.f.context.path)
            .unwrap()
            .operation_record(&f.c.tx.id)
            .is_none());
    }
}

#[test]
fn full_loss_unreported_bad_json_allows_pending() {
    pending_unreported_case("malformed");
}
#[test]
fn full_loss_unreported_foreign_binding_allows_pending() {
    pending_unreported_case("foreign");
}
#[test]
fn full_loss_unreported_bad_evidence_allows_pending() {
    pending_unreported_case("evidence");
}
#[test]
fn full_loss_unreported_scan_limit_allows_pending() {
    pending_unreported_case("capacity");
}
#[test]
fn full_loss_unreported_reparse_allows_pending_without_following() {
    pending_unreported_case("reparse");
}

struct Php {
    child: std::process::Child,
    address: String,
    db: PathBuf,
    root: PathBuf,
}
impl Php {
    fn command(&self) -> Command {
        let mut cmd = Command::new(std::env::var_os("LOCAL_EVENTS_PHP").unwrap());
        cmd.arg("-c")
            .arg(std::env::var_os("LOCAL_EVENTS_PHP_INI").unwrap())
            .env("AGENT_FIXTURE_ROOT", &self.root)
            .env("AGENT_HTTP_FIXTURE_DB", &self.db)
            .env("RUSTDESK_DB_DRIVER", "sqlite")
            .env("RUSTDESK_ADMIN_PATH", "/agent-test")
            .env(
                "RUSTDESK_INSTALL_CONFIG",
                self.db.with_extension("absent.json"),
            );
        cmd
    }
    fn sql(&self, code: &str) -> serde_json::Value {
        let output = self.command().args(["-r", code]).output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).unwrap()
    }
    fn rows(&self) -> serde_json::Value {
        self.sql(r#"$db=new PDO('sqlite:'.getenv('AGENT_HTTP_FIXTURE_DB'));echo json_encode($db->query('SELECT event_id,generation,status,payload,digest FROM device_agent_events ORDER BY sequence')->fetchAll(PDO::FETCH_ASSOC));"#)
    }
}
impl Drop for Php {
    fn drop(&mut self) {
        self.child.kill().unwrap();
        self.child.wait().unwrap();
    }
}

struct FullLoss {
    api: Php,
    c: Case,
}
impl FullLoss {
    fn new(failed: bool, begin: bool) -> Self {
        let mut c = Case::new(Phase::Committed);
        let directory = c.directory();
        c.f.plan.total_loss = Some(Prestate {
            client_anchor: "none".into(),
            original_service: OriginalService::Absent,
        });
        save(&directory.join("local-repair-plan.json"), &c.f.plan).unwrap();
        c.f.handoff.plan_sha256 = hex::encode(Sha256::digest(
            std::fs::read(directory.join("local-repair-plan.json")).unwrap(),
        ));
        save(&directory.join("handoff.json"), &c.f.handoff).unwrap();
        let ready = c.f.handoff.ready_receipt(101).unwrap();
        save(&directory.join("local-ready.json"), &ready).unwrap();
        save(
            &directory.join("local-release.json"),
            &c.f.handoff.release_receipt(&ready, 102).unwrap(),
        )
        .unwrap();
        std::fs::remove_file(&c.f.context.path).unwrap();
        LocalEvents::open(&c.f.context.path)
            .unwrap()
            .remember_binding(c.f.context.binding.clone())
            .unwrap();
        let mut prepared = c.tx.clone();
        prepared.phase = Phase::StoppingServices;
        save(&directory.join("transaction.json"), &prepared).unwrap();
        if begin {
            local_recovery_events::with_context_fixture(c.f.context.clone(), || {
                local_recovery_events::begin_total_loss(&directory, &c.f.plan, &prepared)
            })
            .unwrap();
        }
        c.tx.phase = if failed {
            Phase::RollingBack
        } else {
            Phase::Committed
        };
        save(&directory.join("transaction.json"), &c.tx).unwrap();
        if failed {
            let binding = PreservationBinding {
                transaction_id: c.tx.id.clone(),
                installation_id: c.f.plan.installation_id.clone(),
                plan_sha256: c.f.handoff.plan_sha256.clone(),
                handoff_sha256: hex::encode(Sha256::digest(
                    std::fs::read(directory.join("handoff.json")).unwrap(),
                )),
            };
            let receipt = PreservationReceipt::new(
                binding,
                "fixture install 1603 password=secret C:\\private".into(),
                Ok(()),
                true,
            );
            save(&directory.join("agent-preservation.json"), &receipt).unwrap();
            save(&directory.join("total-loss-install-error.json"),&serde_json::json!({"transaction_id":c.tx.id,"plan_sha256":c.f.handoff.plan_sha256,"error":receipt.installation_error})).unwrap();
            save(&directory.join("local-result.json"),&serde_json::json!({"schema":1,"transaction_id":c.tx.id,"phase":"preservation","receipt":receipt})).unwrap();
            save(&directory.join("total-loss-reconciliation.json"),&serde_json::json!({"schema":1,"transaction_id":c.tx.id,"state":"agent_preserved_client_unrecovered","receipt":receipt,"observed_helper":{"pid":102,"created_at":200}})).unwrap();
        }
        let root = PathBuf::from(std::env::var_os("LOCAL_EVENTS_API_FIXTURE").unwrap());
        let db = c.f.root.join("agent-status-test-full-loss.db");
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener);
        let php = std::env::var_os("LOCAL_EVENTS_PHP").unwrap();
        let ini = std::env::var_os("LOCAL_EVENTS_PHP_INI").unwrap();
        let mut command = Command::new(&php);
        command
            .arg("-c")
            .arg(&ini)
            .env("AGENT_FIXTURE_ROOT", &root)
            .env("AGENT_HTTP_FIXTURE_DB", &db)
            .env("RUSTDESK_DB_DRIVER", "sqlite")
            .env("RUSTDESK_ADMIN_PATH", "/agent-test")
            .env("RUSTDESK_INSTALL_CONFIG", db.with_extension("absent.json"));
        let seed = r#"require getenv('AGENT_FIXTURE_ROOT').'/sqlite/lib.php';require getenv('AGENT_FIXTURE_ROOT').'/sqlite/agent_operations.php';$db=new PDO('sqlite:'.getenv('AGENT_HTTP_FIXTURE_DB'));configure_pdo($db);ensure_schema($db);$now=time();db_insert_ignore($db,'device_reports',['id'=>getenv('ID'),'uuid'=>getenv('UUID'),'payload'=>'{}','last_seen'=>$now,'last_heartbeat'=>$now,'heartbeat_payload'=>'{}']);db_insert_ignore($db,'device_update_keys',['device_id'=>getenv('ID'),'uuid'=>getenv('UUID'),'public_key'=>getenv('PK'),'first_seen_at'=>$now,'last_seen_at'=>$now]);agent_binding_authorize($db,['id'=>1,'address_book_scope'=>'all'],getenv('ID'),getenv('UUID'),str_repeat('a',32),true,$now);"#;
        let out = command
            .args(["-r", seed])
            .env("ID", &c.identity.id)
            .env("UUID", &c.identity.uuid)
            .env("PK", STANDARD.encode(c.identity.public.as_ref()))
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let child = Command::new(&php)
            .arg("-c")
            .arg(&ini)
            .args(["-S", &addr.to_string()])
            .arg(root.join("tests/agent_status_http_router.php"))
            .env("AGENT_HTTP_FIXTURE_DB", &db)
            .env("RUSTDESK_DB_DRIVER", "sqlite")
            .env("RUSTDESK_ADMIN_PATH", "/agent-test")
            .env("RUSTDESK_INSTALL_CONFIG", db.with_extension("absent.json"))
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while std::net::TcpStream::connect(addr).is_err() {
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(20));
        }
        Self {
            api: Php {
                child,
                address: format!("http://{addr}"),
                db,
                root,
            },
            c,
        }
    }
    fn poll(&self, generation: u64) {
        let calls = BACKEND_CALLS.with(Cell::get);
        agent::local_recovery_events::REPORT_KEY
            .with(|v| *v.borrow_mut() = Some(self.c.key.clone()));
        let mut b = self.c.f.context.binding.clone();
        b.enrollment_generation = generation;
        agent::local_event_runtime::poll(
            &self.c.identity,
            &self.api.address,
            &serde_json::to_value(b).unwrap(),
            &self.c.paths,
        );
        agent::local_recovery_events::REPORT_KEY.with(|v| *v.borrow_mut() = None);
        assert_eq!(calls, BACKEND_CALLS.with(Cell::get));
    }
}

#[test]
fn full_loss_success_poll_php_persists_once() {
    let f = FullLoss::new(false, true);
    let before = std::fs::read(f.c.directory().join("transaction.json")).unwrap();
    f.poll(1);
    let rows = f.api.rows();
    assert_eq!(
        rows.as_array().unwrap().len(),
        2,
        "full loss must produce Started and terminal through production hook"
    );
    assert_eq!(rows[1]["status"], "succeeded");
    f.poll(1);
    assert_eq!(f.api.rows(), rows);
    assert_eq!(
        std::fs::read(f.c.directory().join("transaction.json")).unwrap(),
        before
    );
    for row in rows.as_array().unwrap() {
        assert_eq!(
            row["digest"],
            hex::encode(Sha256::digest(row["payload"].as_str().unwrap().as_bytes()))
        );
    }
}

#[test]
fn full_loss_actual_worker_terminal_php_ack() {
    assert_eq!(std::env::var("COMPUTERNAME").as_deref(), Ok("RDLAB-WIN11"));
    let event_root = PathBuf::from(r"C:\RustDeskAgentLab\total-loss-worker-events");
    let context: local_recovery_events::EventContext =
        serde_json::from_slice(&std::fs::read(event_root.join("context.json")).unwrap()).unwrap();
    let identity = agent::identity::load_existing().unwrap();
    assert_eq!(context.binding.client_id, identity.id);
    assert_eq!(context.binding.client_uuid, identity.uuid);
    let mut paths = agent::AgentPaths::from_program_data(r"C:\ProgramData", agent::Edition::Sos);
    let poll_state = event_root.join(format!("poll-state-{}", std::process::id()));
    workspace::create(&poll_state).unwrap();
    let events = poll_state.join("local-events");
    workspace::create(&events).unwrap();
    let mut source = context.path.clone();
    for entry in std::fs::read_dir(&event_root).unwrap() {
        let path = entry.unwrap().path().join("local-events/events.json");
        if path.is_file()
            && LocalEvents::open(&path)
                .unwrap()
                .operation_seen("bbe5e1a522d0cb0276c3581f984ac363")
        {
            source = path;
            break;
        }
    }
    std::fs::copy(&source, events.join("events.json")).unwrap();
    paths.state = poll_state;
    let txroot = paths.agent.join("transactions");
    let dirs = std::fs::read_dir(&txroot)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect::<Vec<_>>();
    assert_eq!(dirs.len(), 1);
    let directory = &dirs[0];
    let before = std::fs::read(directory.join("transaction.json")).unwrap();
    let tx = Transaction::decode(&before).unwrap();
    assert_eq!(tx.phase, Phase::Committed);
    let bundle: serde_json::Value = serde_json::from_slice(
        &std::fs::read(paths.agent.join("rescue/current/bundle.json")).unwrap(),
    )
    .unwrap();
    let key = agent::rescue_assets::fixed_source_public_fixture(&bundle).unwrap();
    let root = PathBuf::from(std::env::var_os("LOCAL_EVENTS_API_FIXTURE").unwrap());
    let db = event_root.join(format!(
        "agent-status-test-actual-worker-{}.db",
        std::process::id()
    ));
    let php = std::env::var_os("LOCAL_EVENTS_PHP").unwrap();
    let ini = std::env::var_os("LOCAL_EVENTS_PHP_INI").unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);
    let seed = r#"require getenv('AGENT_FIXTURE_ROOT').'/sqlite/lib.php';require getenv('AGENT_FIXTURE_ROOT').'/sqlite/agent_operations.php';$db=new PDO('sqlite:'.getenv('AGENT_HTTP_FIXTURE_DB'));configure_pdo($db);ensure_schema($db);$now=time();db_insert_ignore($db,'device_reports',['id'=>getenv('ID'),'uuid'=>getenv('UUID'),'payload'=>'{}','last_seen'=>$now,'last_heartbeat'=>$now,'heartbeat_payload'=>'{}']);db_insert_ignore($db,'device_update_keys',['device_id'=>getenv('ID'),'uuid'=>getenv('UUID'),'public_key'=>getenv('PK'),'first_seen_at'=>$now,'last_seen_at'=>$now]);agent_binding_authorize($db,['id'=>1,'address_book_scope'=>'all'],getenv('ID'),getenv('UUID'),str_repeat('a',32),true,$now);"#;
    let seed = seed.replace("str_repeat('a',32)", "getenv('INSTALLATION')");
    let output = Command::new(&php)
        .arg("-c")
        .arg(&ini)
        .args(["-r", &seed])
        .env("AGENT_FIXTURE_ROOT", &root)
        .env("INSTALLATION", &context.binding.agent_installation_id)
        .env("AGENT_HTTP_FIXTURE_DB", &db)
        .env("RUSTDESK_DB_DRIVER", "sqlite")
        .env("RUSTDESK_ADMIN_PATH", "/agent-test")
        .env("RUSTDESK_INSTALL_CONFIG", db.with_extension("absent.json"))
        .env("ID", &identity.id)
        .env("UUID", &identity.uuid)
        .env("PK", STANDARD.encode(identity.public.as_ref()))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let child = Command::new(&php)
        .arg("-c")
        .arg(&ini)
        .args(["-S", &addr.to_string()])
        .arg(root.join("tests/agent_status_http_router.php"))
        .env("AGENT_HTTP_FIXTURE_DB", &db)
        .env("RUSTDESK_DB_DRIVER", "sqlite")
        .env("RUSTDESK_ADMIN_PATH", "/agent-test")
        .env("RUSTDESK_INSTALL_CONFIG", db.with_extension("absent.json"))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let api = Php {
        child,
        address: format!("http://{addr}"),
        db,
        root,
    };
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while std::net::TcpStream::connect(addr).is_err() {
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(20));
    }
    agent::local_recovery_events::REPORT_KEY.with(|v| *v.borrow_mut() = Some(key));
    assert!(local_recovery_events::verified_repair_terminal(
        directory,
        &tx.id,
        &context.binding,
        &identity
    )
    .unwrap()
    .is_some());
    for _ in 0..2 {
        agent::local_event_runtime::poll(
            &identity,
            &api.address,
            &serde_json::to_value(&context.binding).unwrap(),
            &paths,
        );
    }
    agent::local_recovery_events::REPORT_KEY.with(|v| *v.borrow_mut() = None);
    let rows = api.rows();
    assert_eq!(rows.as_array().unwrap().len(), 2, "{rows}");
    assert_eq!(rows[0]["status"], "running");
    assert_eq!(rows[1]["status"], "succeeded");
    assert!(LocalEvents::open(&events.join("events.json"))
        .unwrap()
        .pending_events()
        .is_empty());
    assert_eq!(
        std::fs::read(directory.join("transaction.json")).unwrap(),
        before
    );
    std::fs::write(
        event_root.join("actual-poll-ack.json"),
        serde_json::to_vec_pretty(&rows).unwrap(),
    )
    .unwrap();
    println!("ACTUAL_WORKER_PHP_ACK={rows}");
}

#[test]
fn full_loss_actual_pending_upload_only() {
    assert_eq!(std::env::var("COMPUTERNAME").as_deref(), Ok("RDLAB-WIN11"));
    let fixture = PathBuf::from(r"C:\RustDeskAgentLab\event-upload-only");
    let source = fixture.join("evidence/event-evidence/poll-state-2340/local-events/events.json");
    let original = std::fs::read(&source).unwrap();
    let run = fixture.join(format!("run-{}", std::process::id()));
    workspace::create(&run).unwrap();
    let store = run.join("events.json");
    std::fs::write(&store, &original).unwrap();
    let mut log = LocalEvents::open(&store).unwrap();
    let binding = log.verified_binding().cloned().unwrap();
    let pending = log.pending_events().to_vec();
    assert_eq!(pending.len(), 2);
    let identity = agent::identity::load_existing().unwrap();
    assert_eq!(binding.client_id, identity.id);
    assert_eq!(binding.client_uuid, identity.uuid);
    assert_eq!(binding.enrollment_generation, 1);
    let root = PathBuf::from(std::env::var_os("LOCAL_EVENTS_API_FIXTURE").unwrap());
    let db = run.join("agent-status-test-actual-worker-events.db");
    let php = std::env::var_os("LOCAL_EVENTS_PHP").unwrap();
    let ini = std::env::var_os("LOCAL_EVENTS_PHP_INI").unwrap();
    let seed = r#"require getenv('AGENT_FIXTURE_ROOT').'/sqlite/lib.php';require getenv('AGENT_FIXTURE_ROOT').'/sqlite/agent_operations.php';$db=new PDO('sqlite:'.getenv('AGENT_HTTP_FIXTURE_DB'));configure_pdo($db);ensure_schema($db);$now=time();db_insert_ignore($db,'device_reports',['id'=>getenv('ID'),'uuid'=>getenv('UUID'),'payload'=>'{}','last_seen'=>$now,'last_heartbeat'=>$now,'heartbeat_payload'=>'{}']);db_insert_ignore($db,'device_update_keys',['device_id'=>getenv('ID'),'uuid'=>getenv('UUID'),'public_key'=>getenv('PK'),'first_seen_at'=>$now,'last_seen_at'=>$now]);agent_binding_authorize($db,['id'=>1,'address_book_scope'=>'all'],getenv('ID'),getenv('UUID'),getenv('INSTALLATION'),true,$now);"#;
    let output = Command::new(&php)
        .arg("-c")
        .arg(&ini)
        .args(["-r", seed])
        .env("AGENT_FIXTURE_ROOT", &root)
        .env("AGENT_HTTP_FIXTURE_DB", &db)
        .env("RUSTDESK_DB_DRIVER", "sqlite")
        .env("RUSTDESK_ADMIN_PATH", "/agent-test")
        .env("RUSTDESK_INSTALL_CONFIG", db.with_extension("absent.json"))
        .env("ID", &identity.id)
        .env("UUID", &identity.uuid)
        .env("INSTALLATION", &binding.agent_installation_id)
        .env("PK", STANDARD.encode(identity.public.as_ref()))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "seed: {} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);
    let child = Command::new(&php)
        .arg("-c")
        .arg(&ini)
        .args(["-S", &addr.to_string()])
        .arg(root.join("tests/agent_status_http_router.php"))
        .env("AGENT_HTTP_FIXTURE_DB", &db)
        .env("RUSTDESK_DB_DRIVER", "sqlite")
        .env("RUSTDESK_ADMIN_PATH", "/agent-test")
        .env("RUSTDESK_INSTALL_CONFIG", db.with_extension("absent.json"))
        .stdout(std::fs::File::create(run.join("php.stdout")).unwrap())
        .stderr(std::fs::File::create(run.join("php.stderr")).unwrap())
        .spawn()
        .unwrap();
    let api = Php {
        child,
        address: format!("http://{addr}"),
        db,
        root,
    };
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while std::net::TcpStream::connect(addr).is_err() {
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(20));
    }
    let seeded=api.sql(r#"$db=new PDO('sqlite:'.getenv('AGENT_HTTP_FIXTURE_DB'));echo json_encode($db->query('SELECT * FROM device_agent_bindings')->fetchAll(PDO::FETCH_ASSOC));"#);
    assert_eq!(seeded[0]["installation_id"], binding.agent_installation_id);
    // Deliberately discard the valid first ACK without acknowledging the durable queue.
    let first =
        agent::local_event_runtime::send_signed(&identity, &api.address, &pending[0].payload);
    std::fs::write(
        run.join("first-response.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"result":first,"pending_before":pending.len()}),
        )
        .unwrap(),
    )
    .unwrap();
    println!(
        "UPLOAD_ONLY_FIRST_RESPONSE={first:?}; evidence={}",
        run.display()
    );
    if first.is_err() {
        assert!(api.rows().as_array().unwrap().is_empty());
        assert_eq!(std::fs::read(&store).unwrap(), original);
        eprintln!(
            "PHP_DIAGNOSTIC={}",
            std::fs::read_to_string(run.join("php.stderr")).unwrap()
        );
    }
    let first = first.unwrap();
    assert_eq!(first["ok"], true);
    assert_eq!(first["event_id"], pending[0].event_id);
    assert_eq!(api.rows().as_array().unwrap().len(), 1);
    assert_eq!(std::fs::read(&store).unwrap(), original);
    agent::local_event_runtime::upload(&mut log, &binding, |body| {
        agent::local_event_runtime::send_signed(&identity, &api.address, body)
    })
    .unwrap();
    let rows = api.rows();
    assert_eq!(rows.as_array().unwrap().len(), 2);
    for (index, event) in pending.iter().enumerate() {
        assert_eq!(rows[index]["event_id"], event.event_id);
        assert_eq!(rows[index]["generation"], binding.enrollment_generation);
        assert_eq!(rows[index]["digest"], event.digest);
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(rows[index]["payload"].as_str().unwrap())
                .unwrap(),
            event.payload
        );
    }
    assert_eq!(rows[0]["status"], "running");
    assert_eq!(rows[1]["status"], "succeeded");
    assert!(LocalEvents::open(&store)
        .unwrap()
        .pending_events()
        .is_empty());
    let mut reopened = LocalEvents::open(&store).unwrap();
    agent::local_event_runtime::upload(&mut reopened, &binding, |_| {
        panic!("ACKed events must not resend or execute actions")
    })
    .unwrap();
    assert_eq!(api.rows(), rows);
    assert_eq!(std::fs::read(&source).unwrap(), original);
    std::fs::write(run.join("proof.json"),serde_json::to_vec_pretty(&serde_json::json!({"rows":rows,"acked_pending":0,"lost_ack_duplicate_rows":1,"binding":binding})).unwrap()).unwrap();
    println!(
        "UPLOAD_ONLY_ACK_GREEN rows=2 pending=0 duplicate=1; evidence={}",
        run.display()
    );
}
#[test]
fn full_loss_failure_is_reconciliation_required_and_redacted() {
    let f = FullLoss::new(true, true);
    let result = std::fs::read(f.c.directory().join("local-result.json")).unwrap();
    f.poll(1);
    let rows = f.api.rows();
    assert_eq!(
        rows.as_array().unwrap().len(),
        2,
        "full loss failure must be reported"
    );
    assert_eq!(rows[1]["status"], "reconciliation_required");
    let body: serde_json::Value =
        serde_json::from_str(rows[1]["payload"].as_str().unwrap()).unwrap();
    assert_eq!(body["result"]["client_recovered"], false);
    assert_eq!(body["result"]["error_code"], "redacted");
    assert!(!rows.to_string().contains("secret"));
    assert!(!rows.to_string().contains("private"));
    assert_eq!(
        std::fs::read(f.c.directory().join("local-result.json")).unwrap(),
        result
    );
    assert_eq!(
        Transaction::decode(&std::fs::read(f.c.directory().join("transaction.json")).unwrap())
            .unwrap()
            .phase,
        Phase::RollingBack
    );
}
#[test]
fn full_loss_unclaimed_terminal_never_scanned() {
    let f = FullLoss::new(false, false);
    f.poll(1);
    assert!(f.api.rows().as_array().unwrap().is_empty());
    assert!(LocalEvents::open(&f.c.f.context.path)
        .unwrap()
        .operation_record(&f.c.tx.id)
        .is_none());
}
#[test]
fn full_loss_foreign_receipt_preserves_started() {
    let f = FullLoss::new(true, true);
    let p = f.c.directory().join("agent-preservation.json");
    let mut v: serde_json::Value = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
    v["binding"]["transaction_id"] = "f".repeat(32).into();
    save(&p, &v).unwrap();
    let before = std::fs::read(&f.c.f.context.path).unwrap();
    f.poll(1);
    assert_eq!(std::fs::read(&f.c.f.context.path).unwrap(), before);
    assert!(f.api.rows().as_array().unwrap().is_empty());
}

#[test]
fn full_loss_lost_ack_restart_conflict_and_stale_generation() {
    let f = FullLoss::new(true, true);
    local_recovery_events::REPORT_KEY.with(|v| *v.borrow_mut() = Some(f.c.key.clone()));
    local_recovery_events::reconcile_repairs(
        &f.c.identity,
        &f.c.f.context.binding,
        &f.c.paths.state,
        &f.c.paths.agent,
    )
    .unwrap();
    local_recovery_events::REPORT_KEY.with(|v| *v.borrow_mut() = None);
    let mut log = LocalEvents::open(&f.c.f.context.path).unwrap();
    let original = std::fs::read(&f.c.f.context.path).unwrap();
    assert!(
        agent::local_event_runtime::upload(&mut log, &f.c.f.context.binding, |body| {
            agent::local_event_runtime::send_signed(&f.c.identity, &f.api.address, body)?;
            Err("lost accepted ACK fixture".into())
        })
        .is_err()
    );
    assert_eq!(std::fs::read(&f.c.f.context.path).unwrap(), original);
    drop(log);
    f.poll(1);
    let rows = f.api.rows();
    assert_eq!(rows.as_array().unwrap().len(), 2);
    let body: serde_json::Value =
        serde_json::from_str(rows[1]["payload"].as_str().unwrap()).unwrap();
    let duplicate =
        agent::local_event_runtime::send_signed(&f.c.identity, &f.api.address, &body).unwrap();
    assert_eq!(duplicate["duplicate"], true);
    let mut conflict = body.clone();
    conflict["result"]["client_recovered"] = true.into();
    assert_eq!(
        agent::local_event_runtime::send_signed(&f.c.identity, &f.api.address, &conflict)
            .unwrap_err(),
        "local event HTTP 409"
    );
    f.api.sql(r#"$db=new PDO('sqlite:'.getenv('AGENT_HTTP_FIXTURE_DB'));$db->exec('UPDATE device_agent_bindings SET generation=2');echo 'true';"#);
    assert_eq!(
        agent::local_event_runtime::send_signed(&f.c.identity, &f.api.address, &body).unwrap_err(),
        "local event HTTP 403"
    );
    assert_eq!(f.api.rows(), rows);
}

#[test]
fn full_loss_generation_change_never_rebinds_receipt() {
    let f = FullLoss::new(true, true);
    f.poll(2);
    let log = LocalEvents::open(&f.c.f.context.path).unwrap();
    assert_eq!(log.pending_events().len(), 2);
    for event in log.pending_events() {
        assert_eq!(event.payload["enrollment_generation"], 1);
    }
    assert!(f.api.rows().as_array().unwrap().is_empty());
}

#[test]
fn full_loss_started_failure_blocks_mutation_and_duplicate_claim() {
    let f = FullLoss::new(false, false);
    let mut tx = f.c.tx.clone();
    tx.phase = Phase::StoppingServices;
    save(&f.c.directory().join("transaction.json"), &tx).unwrap();
    let before = std::fs::read(&f.c.f.context.path).unwrap();
    let lock = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .share_mode(0)
        .open(f.c.f.context.path.with_extension("lock"))
        .unwrap();
    let result = local_recovery_events::with_context_fixture(f.c.f.context.clone(), || {
        local_recovery_events::begin_total_loss(&f.c.directory(), &f.c.f.plan, &tx)
    });
    assert!(result.is_ok());
    assert_eq!(std::fs::read(&f.c.f.context.path).unwrap(), before);
    assert!(f.c.directory().join("local-event-unreported.json").exists());
    drop(lock);
    let before = std::fs::read(&f.c.f.context.path).unwrap();
    assert!(
        local_recovery_events::with_context_fixture(f.c.f.context.clone(), || {
            local_recovery_events::begin_total_loss(&f.c.directory(), &f.c.f.plan, &tx)
        })
        .unwrap_err()
        .contains("already started")
    );
    assert_eq!(std::fs::read(&f.c.f.context.path).unwrap(), before);
}

#[test]
fn full_loss_unconfirmed_or_bad_signature_never_reports_success() {
    for name in [
        "agent-preservation.json",
        "approved-manifest.json",
        "total-loss-reconciliation.json",
        "identity-backup.toml",
    ] {
        let f = FullLoss::new(true, true);
        let path = f.c.directory().join(name);
        if name == "identity-backup.toml" {
            std::fs::write(&path, b"changed identity").unwrap();
        } else {
            let mut value: serde_json::Value =
                serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
            match name {
                "agent-preservation.json" => value["client_recovered"] = true.into(),
                "total-loss-reconciliation.json" => value["transaction_id"] = "f".repeat(32).into(),
                _ => {
                    value["targets"]["windows-x86_64-msi-sos"]["signature"] =
                        STANDARD.encode([0u8; 64]).into()
                }
            };
            save(&path, &value).unwrap();
        }
        let before = std::fs::read(&f.c.f.context.path).unwrap();
        f.poll(1);
        assert_eq!(std::fs::read(&f.c.f.context.path).unwrap(), before);
        assert!(f.api.rows().as_array().unwrap().is_empty());
    }
}

#[test]
fn full_loss_degraded_worker_child() {
    let Some(input) = std::env::var_os("FULL_LOSS_WORKER_INPUT") else {
        return;
    };
    let input: serde_json::Value = serde_json::from_slice(&std::fs::read(input).unwrap()).unwrap();
    let directory = PathBuf::from(input["directory"].as_str().unwrap());
    assert!(directory.starts_with(r"C:\RustDeskAgentLab"));
    let plan = agent::local_repair_plan::LocalRepairPlan::decode(
        &std::fs::read(directory.join("local-repair-plan.json")).unwrap(),
    )
    .unwrap();
    let tx =
        Transaction::decode(&std::fs::read(directory.join("transaction.json")).unwrap()).unwrap();
    let context: local_recovery_events::EventContext =
        serde_json::from_value(input["context"].clone()).unwrap();
    let staging = context
        .path
        .with_extension(format!("tmp-{}", std::process::id()));
    if input["storage"] == true {
        std::fs::create_dir(&staging).unwrap();
    }
    let result = local_recovery_events::with_context_fixture(context, || {
        local_recovery_events::total_loss_event_step(
            &directory,
            &plan,
            &tx,
            || {
                if directory.join("disable.fixture").exists() {
                    Err("locally blocked".into())
                } else {
                    Ok(())
                }
            },
            || {
                // The child owns this execution; a later poll must never enter this action again.
                std::fs::write(
                    directory.join("action-once.fixture"),
                    b"one authorized action",
                )
                .map_err(|e| e.to_string())?;
                let terminal: Transaction =
                    serde_json::from_value(input["terminal"].clone()).map_err(|e| e.to_string())?;
                save(&directory.join("transaction.json"), &terminal)
            },
        )
    });
    if input["storage"] == true {
        std::fs::remove_dir(&staging).unwrap();
    }
    if input["disabled"] == true {
        assert_eq!(result, Err("locally blocked".into()));
        assert!(!directory.join("action-once.fixture").exists());
        return;
    }
    assert!(
        result.is_ok(),
        "full loss offline worker must survive event failure: {result:?}"
    );
}

fn degraded_worker_case(fault: &str) {
    let f = FullLoss::new(false, false);
    let directory = f.c.directory();
    let mut prepared = f.c.tx.clone();
    prepared.phase = Phase::StoppingServices;
    save(&directory.join("transaction.json"), &prepared).unwrap();
    let mut lock = None;
    if fault == "locked" {
        lock = Some(
            std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(false)
                .share_mode(0)
                .open(f.c.f.context.path.with_extension("lock"))
                .unwrap(),
        );
    } else if fault == "storage" {
        // The child PID is not known yet; its production event publish boundary is injected below.
    } else if fault == "disabled" {
        std::fs::write(directory.join("disable.fixture"), b"explicit local intent").unwrap();
    } else if fault == "full" {
        let mut log = LocalEvents::open(&f.c.f.context.path).unwrap();
        for index in 0..128 {
            let op = Operation {
                operation_id: format!("{index:032x}"),
                code: RecoveryCode::R1,
                action: Action::StartService,
            };
            for (phase, outcome) in [
                (EventPhase::Started, Outcome::Running),
                (EventPhase::Terminal, Outcome::Succeeded),
            ] {
                log.record_bound_at(
                    &op,
                    Record {
                        phase,
                        attempt_seq: 1,
                        outcome,
                        path_key: PathKey::RustDeskService,
                        error_code: None,
                    },
                    &f.c.f.context.binding,
                    100,
                )
                .unwrap();
            }
        }
    }
    let input = f.c.f.root.join("worker-input.json");
    save(
        &input,
        &serde_json::json!({"directory":directory,"context":f.c.f.context,"terminal":f.c.tx,"disabled":fault=="disabled","storage":fault=="storage"}),
    )
    .unwrap();
    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "management_agent::r4_poll_tests::full_loss::full_loss_degraded_worker_child",
            "--nocapture",
        ])
        .env("FULL_LOSS_WORKER_INPUT", &input)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "full loss helper exited before recovery: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    if fault == "disabled" {
        assert!(!directory.join("action-once.fixture").exists());
        assert!(!directory.join("local-event-unreported.json").exists());
        assert_eq!(
            Transaction::decode(&std::fs::read(directory.join("transaction.json")).unwrap())
                .unwrap(),
            prepared
        );
        return;
    }
    assert_eq!(
        std::fs::read(directory.join("action-once.fixture")).unwrap(),
        b"one authorized action"
    );
    drop(lock.take());
    if fault == "full" {
        // Normal polls must drain existing events while the durable unreported receipt waits.
        for _ in 0..35 {
            f.poll(1);
        }
    } else {
        f.poll(1);
    }
    let log = LocalEvents::open(&f.c.f.context.path).unwrap();
    assert_eq!(
        log.operation_record(&f.c.tx.id).unwrap().2.phase,
        EventPhase::Terminal
    );
    let events = f.api.rows();
    let before = events.as_array().unwrap().len();
    f.poll(1);
    assert_eq!(f.api.rows().as_array().unwrap().len(), before);
    assert_eq!(
        std::fs::read(directory.join("action-once.fixture")).unwrap(),
        b"one authorized action"
    );
}

#[test]
fn full_loss_locked_worker_exits_then_normal_poll_reports() {
    degraded_worker_case("locked");
}
#[test]
fn full_loss_full_worker_recovers_before_normal_poll_drains() {
    degraded_worker_case("full");
}

#[test]
fn full_loss_disabled_worker_retains_installation_intent() {
    degraded_worker_case("disabled");
}

#[test]
fn full_loss_storage_worker_exits_then_normal_poll_reports() {
    degraded_worker_case("storage");
}
