use super::*;

#[test]
fn delivery_disabled_wire_is_lowercase_without_changing_observer_contract() {
    let root=std::env::temp_dir().join(format!("agent-status-contract-disabled-{}",std::process::id()));
    let paths=AgentPaths::from_program_data(&root,super::super::super::Edition::Standard);
    let state=AgentState::new(super::super::super::Edition::Standard,&paths);
    TEST_START_TYPE.with(|value| *value.borrow_mut()=Some("Disabled".into()));
    let result=snapshot(&state,&paths,&"a".repeat(32),&"b".repeat(32),1);
    TEST_START_TYPE.with(|value| *value.borrow_mut()=None);
    let payload=result.unwrap();
    assert_eq!(payload["service"]["start_type"],"disabled");
    assert_eq!(payload["recovery"]["effective_enabled"],false);
    let dirs=crate::management_agent::system_dirs::get();
    assert_eq!(payload["paths"]["fixed_normal_install"],dirs.normal_install().to_string_lossy().as_ref());
    assert_eq!(payload["paths"]["agent"],paths.agent.to_string_lossy().as_ref());
    assert_eq!(payload["paths"]["system_drive"],&dirs.windows.to_string_lossy()[..2]);
}

#[test]
fn delivery_two_installations_use_real_enroll_and_signed_status_http() {
    signed_status_scenario(None);
}

#[test]
fn delivery_corrupt_binding_preserves_status_and_rejects_new_instance() {
    signed_status_scenario(Some("corrupt"));
}

#[test]
fn delivery_unreadable_binding_preserves_status_and_rejects_new_instance() {
    signed_status_scenario(Some("unreadable"));
}

#[test]
fn delivery_invalid_record_preserves_status_and_rejects_new_instance() {
    signed_status_scenario(Some("invalid_record"));
}

fn signed_status_scenario(fault: Option<&str>) {
    use std::process::{Command, Stdio};
    let api = std::path::PathBuf::from(std::env::var_os("AGENT_TEST_API_ROOT").expect("frozen API required"));
    let php = std::env::var("AGENT_TEST_PHP").unwrap();
    let ini = std::env::var("AGENT_TEST_PHP_INI").unwrap();
    let root = std::env::temp_dir().join(format!("agent-status-test-contract-http-{}-{}", std::process::id(),fault.unwrap_or("healthy")));
    std::fs::create_dir(&root).unwrap();
    let db = root.join("agent-status-test.db");
    let identity = identity::load_existing().unwrap();
    let sql = |code: &str| {
        let out = Command::new(&php).args(["-c", &ini, "-r", code])
            .env("API", &api).env("DB", &db).env("ID", &identity.id).env("UUID", &identity.uuid)
            .env("PK", STANDARD.encode(identity.public.as_ref())).output().unwrap();
        assert!(out.status.success(), "PHP fixture: {}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8(out.stdout).unwrap()
    };
    let setup = r#"require getenv('API').'/sqlite/lib.php';$db=new PDO('sqlite:'.getenv('DB'));configure_pdo($db);ensure_schema($db);$pk=getenv('PK');db_insert_ignore($db,'device_update_keys',['device_id'=>getenv('ID'),'uuid'=>getenv('UUID'),'public_key'=>$pk,'first_seen_at'=>time(),'last_seen_at'=>time()]);"#;
    sql(setup);
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();drop(listener);
    let child = Command::new(&php).args(["-c", &ini, "-S", &address.to_string()])
        .arg(api.join("tests/agent_status_http_router.php"))
        .env("AGENT_HTTP_FIXTURE_DB", &db).env("RUSTDESK_DB_DRIVER", "sqlite")
        .env("RUSTDESK_INSTALL_CONFIG", root.join("absent.json"))
        .stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap();
    struct Server(std::process::Child);
    impl Drop for Server { fn drop(&mut self) { let _ = self.0.kill(); let _ = self.0.wait(); } }
    let server = Server(child);
    for _ in 0..100 { if std::net::TcpStream::connect(address).is_ok() {break;} thread::sleep(Duration::from_millis(20)); }
    let paths = AgentPaths::from_program_data(&root, super::super::super::Edition::Standard);
    std::fs::create_dir_all(&paths.state).unwrap();
    let mut state = AgentState::new(super::super::super::Edition::Standard, &paths);
    state.agent_installation_id = "a".repeat(32);state.status_session_generation = 91;
    let url = format!("http://{address}/rd/update/v1/agent/status");
    let first = snapshot(&state, &paths, &state.agent_installation_id, &"d".repeat(32), 1).unwrap();
    assert!(first.get("enrollment_generation").is_none());
    send_with_identity(&first, &identity, &url).unwrap();
    let base = format!("http://{address}");
    let error = super::super::super::operations_runtime::delivery_contract_poll(&base,&state,&paths).unwrap_err();
    assert!(matches!(error.failure, super::super::super::network_policy::Failure::Http(403)));
    let authorize = r#"require getenv('API').'/sqlite/lib.php';require getenv('API').'/sqlite/agent_operations.php';$db=new PDO('sqlite:'.getenv('DB'));configure_pdo($db);$actor=['id'=>1,'address_book_scope'=>'all'];agent_binding_authorize($db,$actor,getenv('ID'),getenv('UUID'),str_repeat('a',32),true,time());"#;
    sql(authorize);
    super::super::super::operations_runtime::delivery_contract_poll(&base,&state,&paths).unwrap();
    let accepted = snapshot(&state,&paths,&state.agent_installation_id,&"d".repeat(32),2).unwrap();
    send_with_identity(&accepted,&identity,&url).unwrap();
    let store = paths.state.join("local-events/events.json");
    let mut locked = None;
    let mut damaged = None;
    if let Some(fault) = fault {
        let original = std::fs::read(&store).unwrap();
        let bytes = match fault {
            "corrupt" => b"{private-key=never-report-this;unknown-original-bytes".to_vec(),
            "invalid_record" => {
                let mut value: serde_json::Value = serde_json::from_slice(&original).unwrap();
                value["events"] = serde_json::json!([{"secret":"never-report-this"}]);
                serde_json::to_vec(&value).unwrap()
            }
            _ => original,
        };
        std::fs::write(&store, &bytes).unwrap();
        if fault == "unreadable" {
            use std::os::windows::fs::OpenOptionsExt;
            locked = Some(std::fs::OpenOptions::new().read(true).share_mode(0).open(&store).unwrap());
            assert!(std::fs::read(&store).is_err(), "must exercise actual sharing violation");
        }
        let previous = sql(r#"$db=new PDO('sqlite:'.getenv('DB'));echo $db->query('SELECT payload FROM device_agent_status')->fetchColumn();"#);
        let result = snapshot(&state,&paths,&state.agent_installation_id,&"d".repeat(32),3);
        if result.is_err() {
            assert_eq!(sql(r#"$db=new PDO('sqlite:'.getenv('DB'));echo $db->query('SELECT payload FROM device_agent_status')->fetchColumn();"#),previous);
        }
        let degraded = result.expect("binding read failure must not suppress base status");
        assert!(degraded.get("enrollment_generation").is_none());
        assert_eq!(degraded["enrollment_diagnostic"]["error_code"],if fault=="unreadable" {"binding_store_unavailable"} else {"binding_store_invalid"});
        assert_eq!(degraded["enrollment_diagnostic"]["state"],"unavailable");
        assert!(!degraded.to_string().contains("never-report-this"));
        send_with_identity(&degraded,&identity,&url).unwrap();
        assert_eq!(serde_json::from_str::<serde_json::Value>(&sql(r#"$db=new PDO('sqlite:'.getenv('DB'));echo $db->query('SELECT payload FROM device_agent_status')->fetchColumn();"#)).unwrap(),degraded);
        damaged=Some(bytes);
    }
    sql(r#"require getenv('API').'/sqlite/lib.php';require getenv('API').'/sqlite/agent_operations.php';$db=new PDO('sqlite:'.getenv('DB'));configure_pdo($db);$actor=['id'=>1,'address_book_scope'=>'all'];agent_binding_authorize($db,$actor,getenv('ID'),getenv('UUID'),str_repeat('a',32),false,time());agent_binding_authorize($db,$actor,getenv('ID'),getenv('UUID'),str_repeat('b',32),true,time());"#);
    state.agent_installation_id="b".repeat(32);state.status_session_generation=1;
    if let Some(bytes) = damaged {
        let previous=sql(r#"$db=new PDO('sqlite:'.getenv('DB'));echo $db->query('SELECT payload FROM device_agent_status')->fetchColumn();"#);
        let rebound=snapshot(&state,&paths,&state.agent_installation_id,&"e".repeat(32),1).unwrap();
        assert!(rebound.get("enrollment_generation").is_none());
        assert_eq!(send_with_identity(&rebound,&identity,&url).unwrap_err(),"agent status HTTP 409 Conflict");
        assert_eq!(sql(r#"$db=new PDO('sqlite:'.getenv('DB'));echo $db->query('SELECT payload FROM device_agent_status')->fetchColumn();"#),previous);
        drop(locked);
        assert_eq!(std::fs::read(&store).unwrap(),bytes,"unknown raw store must remain byte-identical");
        drop(server);std::fs::remove_dir_all(root).unwrap();
        return;
    }
    super::super::super::operations_runtime::delivery_contract_poll(&base,&state,&paths).unwrap();
    let rebound=snapshot(&state,&paths,&state.agent_installation_id,&"e".repeat(32),1).unwrap();
    let result=send_with_identity(&rebound,&identity,&url);
    assert!(result.is_ok(), "actual snapshot must allow authorized new installation: {result:?}");
    assert_eq!(accepted["enrollment_generation"],1);
    assert_eq!(rebound["enrollment_generation"],2);
    let stored=sql(r#"$db=new PDO('sqlite:'.getenv('DB'));echo $db->query('SELECT payload FROM device_agent_status')->fetchColumn();"#);
    assert_eq!(serde_json::from_str::<serde_json::Value>(&stored).unwrap(),rebound);
    assert!(send_with_identity(&accepted,&identity,&url).is_err());
    assert_eq!(sql(r#"$db=new PDO('sqlite:'.getenv('DB'));echo $db->query('SELECT payload FROM device_agent_status')->fetchColumn();"#),stored);
    drop(server);std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn delivery_status_uses_accepted_enrollment_and_preserves_unbound_snapshot() {
    let root = std::env::temp_dir().join(format!("agent-status-contract-{}", std::process::id()));
    let paths = AgentPaths::from_program_data(&root, super::super::super::Edition::Standard);
    std::fs::create_dir_all(&paths.state).unwrap();
    let mut state = AgentState::new(super::super::super::Edition::Standard, &paths);
    state.agent_installation_id = "a".repeat(32);
    state.status_session_generation = 97;
    let first = snapshot(&state, &paths, &state.agent_installation_id, &"b".repeat(32), 1).unwrap();
    assert!(first.get("enrollment_generation").is_none());
    let directory = paths.state.join("local-events");
    super::super::super::workspace::create(&directory).unwrap();
    let path = directory.join("events.json");
    let mut log = super::super::super::local_events::LocalEvents::open(&path).unwrap();
    log.remember_binding(super::super::super::local_events::Binding {
        client_id: first["client_id"].as_str().unwrap().into(),
        client_uuid: first["client_uuid"].as_str().unwrap().into(),
        agent_installation_id: state.agent_installation_id.clone(),
        enrollment_generation: 7,
    }).unwrap();
    let before = std::fs::read(&path).unwrap();
    let accepted = snapshot(&state, &paths, &state.agent_installation_id, &"b".repeat(32), 2).unwrap();
    assert_eq!(accepted["enrollment_generation"], 7, "must use accepted binding, never session_generation");
    assert_eq!(std::fs::read(&path).unwrap(), before);
    state.agent_installation_id = "c".repeat(32);
    let foreign = snapshot(&state, &paths, &state.agent_installation_id, &"b".repeat(32), 3).unwrap();
    assert!(foreign.get("enrollment_generation").is_none());
    state.agent_installation_id = "a".repeat(32);
    let mut wrong = log.verified_binding().unwrap().clone();
    wrong.client_uuid = "foreign-uuid".into();
    log.remember_binding(wrong).unwrap();
    let foreign = snapshot(&state, &paths, &state.agent_installation_id, &"b".repeat(32), 4).unwrap();
    assert!(foreign.get("enrollment_generation").is_none());
    std::fs::remove_dir_all(root).unwrap();
}
