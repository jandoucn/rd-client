use super::*;
use std::{cell::RefCell, path::PathBuf, rc::Rc};

struct Fixture {
    root: PathBuf,
    paths: AgentPaths,
    state: AgentState,
    calls: Rc<RefCell<Vec<PathBuf>>>,
}

impl Fixture {
    fn new(
        command: String,
        normal: bool,
        fallback: bool,
        trusted: bool,
        running: bool,
        healthy: bool,
    ) -> Self {
        let root = std::env::temp_dir().join(format!(
            "status-health-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        let paths = AgentPaths::from_program_data(&root, super::super::super::Edition::Standard);
        std::fs::create_dir_all(&paths.state).unwrap();
        let mut state = AgentState::new(super::super::super::Edition::Standard, &paths);
        state.normal_install_path = r"C:\Program Files\RustDesk".into();
        state.agent_installation_id = "a".repeat(32);
        state.status_session_generation = 7;
        std::fs::write(
            paths.state.join("agent.json"),
            serde_json::to_vec(&state).unwrap(),
        )
        .unwrap();
        let calls = Rc::new(RefCell::new(Vec::new()));
        let captured = calls.clone();
        TEST_HEALTH_OBSERVATION.with(|input| {
            *input.borrow_mut() = Some(TestHealthObservation {
                identity: || Ok(test_identity()),
                observe: Box::new(move || {
                    Ok(recovery_windows::ServiceObservation {
                        state: if running {
                            recovery::ServiceState::Running
                        } else {
                            recovery::ServiceState::Stopped
                        },
                        command: command.clone(),
                        trusted_path: trusted,
                        normal_present: normal,
                        fallback_present: fallback,
                        disabled_start: false,
                        process_id: Some(42),
                        account: Some("LocalSystem".into()),
                        start_type: "AutoStart".into(),
                    })
                }),
                probe: Box::new(move |exe, build| {
                    captured.borrow_mut().push(exe.to_owned());
                    if !healthy {
                        return Err("fixture health access denied".into());
                    }
                    super::super::super::health::Response::new(
                        &super::super::super::health::Request::new("c".repeat(64)).unwrap(),
                        42,
                        123,
                        "1.5.3".into(),
                        build,
                        true,
                        false,
                    )
                }),
            })
        });
        Self {
            root,
            paths,
            state,
            calls,
        }
    }

    fn resident(&self) -> serde_json::Value {
        snapshot(
            &self.state,
            &self.paths,
            &self.state.agent_installation_id,
            &"b".repeat(32),
            2,
        )
        .unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        TEST_HEALTH_OBSERVATION.with(|input| *input.borrow_mut() = None);
        std::fs::remove_dir_all(&self.root).unwrap();
    }
}

#[test]
fn expected_executable_follows_trusted_scm_for_normal_fallback_and_both() {
    for (normal, fallback, use_normal) in [
        (true, false, true),
        (false, true, false),
        (true, true, true),
        (true, true, false),
    ] {
        let fixture = Fixture::new(String::new(), normal, fallback, true, true, true);
        let expected = if use_normal {
            PathBuf::from(r"C:\Program Files\RustDesk\RustDesk.exe")
        } else {
            fixture.paths.client.join("RustDesk.exe")
        };
        TEST_HEALTH_OBSERVATION.with(|input| {
            let mut input = input.borrow_mut();
            input.as_mut().unwrap().observe = Box::new({
                let command = format!(r#""{}" --service"#, expected.display());
                move || {
                    Ok(recovery_windows::ServiceObservation {
                        state: recovery::ServiceState::Running,
                        command: command.clone(),
                        trusted_path: true,
                        normal_present: normal,
                        fallback_present: fallback,
                        disabled_start: false,
                        process_id: Some(42),
                        account: Some("LocalSystem".into()),
                        start_type: "AutoStart".into(),
                    })
                }
            });
        });
        let bytes = std::fs::read(fixture.paths.state.join("agent.json")).unwrap();
        let resident = fixture.resident();
        let cli = super::super::super::local_status(&fixture.paths).unwrap();
        assert_eq!(
            cli.get("observation_source"),
            Some(&serde_json::json!("local_cli"))
        );
        assert!(resident.get("observation_source").is_none());
        assert!(cli.get("boot_id").is_none());
        assert!(cli.get("sequence").is_none());
        assert_eq!(resident["sequence"], 2);
        assert_eq!(resident["session_generation"], cli["session_generation"]);
        let mut normalized = resident.clone();
        normalized.as_object_mut().unwrap().remove("boot_id");
        normalized.as_object_mut().unwrap().remove("sequence");
        normalized["observation_source"] = "local_cli".into();
        assert_eq!(
            normalized, cli,
            "equal frozen inputs must differ only by the CLI envelope"
        );
        assert_eq!(resident["application"], cli["application"]);
        assert_eq!(resident["application"]["state"], "ready");
        assert_eq!(
            resident["application"].get("registered"),
            Some(&serde_json::json!(false))
        );
        assert_eq!(*fixture.calls.borrow(), vec![expected.clone(), expected]);
        assert_eq!(
            std::fs::read(fixture.paths.state.join("agent.json")).unwrap(),
            bytes
        );
    }
}

#[test]
fn missing_normal_scm_or_untrusted_or_stopped_does_not_probe_fallback() {
    for (trusted, running) in [(false, true), (true, false), (false, false)] {
        let fixture = Fixture::new(
            r#""C:\Program Files\RustDesk\RustDesk.exe" --service"#.into(),
            false,
            true,
            trusted,
            running,
            true,
        );
        let result = fixture.resident();
        assert_eq!(result["application"]["state"], "unknown");
        assert!(result["application"].get("registered").is_none());
        assert!(fixture.calls.borrow().is_empty());
    }
}

#[test]
fn success_then_failure_omits_old_identity_fields_and_keeps_independent_observations() {
    let fixture = Fixture::new("fallback".into(), false, true, true, true, true);
    let resident = fixture.resident();
    let saved = resident.clone();
    TEST_HEALTH_OBSERVATION.with(|input| {
        input.borrow_mut().as_mut().unwrap().probe = Box::new(|_, _| Err("fixture denied".into()))
    });
    let cli = super::super::super::local_status(&fixture.paths).unwrap();
    let failed = fixture.resident();
    assert_eq!(cli["application"], failed["application"]);
    assert_eq!(failed["application"]["error_code"], "health_unconfirmed");
    for key in [
        "registered",
        "pid",
        "process_created_at",
        "version",
        "build_seq",
    ] {
        assert!(
            failed["application"].get(key).is_none(),
            "{key} must be absent, not null/false/0"
        );
    }
    assert_eq!(resident, saved);
    assert_eq!(resident["application"]["state"], "ready");
    assert_eq!(
        resident["application"].get("registered"),
        Some(&serde_json::json!(false))
    );
}

#[test]
fn cli_state_read_failure_preserves_bytes_and_previous_resident_result() {
    use std::os::windows::fs::OpenOptionsExt;
    let fixture = Fixture::new("fallback".into(), false, true, true, true, true);
    let path = fixture.paths.state.join("agent.json");
    let bytes = std::fs::read(&path).unwrap();
    let resident = fixture.resident();
    let saved = resident.clone();
    let locked = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(&path)
        .unwrap();
    assert!(std::fs::read(&path).is_err());
    let error = super::super::super::local_status(&fixture.paths).unwrap_err();
    assert!(error.starts_with("agent state unavailable:"), "{error}");
    assert_eq!(
        fixture.calls.borrow().len(),
        1,
        "read failure must precede probe"
    );
    drop(locked);
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    assert_eq!(resident, saved);
    assert_eq!(fixture.resident()["application"], resident["application"]);
}

#[test]
fn identity_read_failure_is_distinct_from_probe_failure_and_preserves_old_data() {
    let fixture = Fixture::new("fallback".into(), false, true, true, true, true);
    let path = fixture.paths.state.join("agent.json");
    let bytes = std::fs::read(&path).unwrap();
    let resident = fixture.resident();
    let saved = resident.clone();
    TEST_HEALTH_OBSERVATION.with(|input| {
        input.borrow_mut().as_mut().unwrap().identity =
            || Err("identity read: fixture access denied".into());
    });
    for result in [
        snapshot(
            &fixture.state,
            &fixture.paths,
            &fixture.state.agent_installation_id,
            &"b".repeat(32),
            3,
        ),
        super::super::super::local_status(&fixture.paths),
    ] {
        assert_eq!(result.unwrap_err(), "identity read: fixture access denied");
    }
    assert_eq!(
        fixture.calls.borrow().len(),
        1,
        "identity failure must precede health probe"
    );
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    assert_eq!(resident, saved);
}

#[test]
fn observer_read_failure_preserves_old_data_and_does_not_probe() {
    let fixture = Fixture::new("fallback".into(), false, true, true, true, true);
    let path = fixture.paths.state.join("agent.json");
    let bytes = std::fs::read(&path).unwrap();
    TEST_HEALTH_OBSERVATION.with(|input| {
        input.borrow_mut().as_mut().unwrap().observe =
            Box::new(|| Err("service query: fixture access denied".into()));
    });
    assert_eq!(
        super::super::super::local_status(&fixture.paths).unwrap_err(),
        "service query: fixture access denied"
    );
    assert!(fixture.calls.borrow().is_empty());
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
}

#[test]
fn blocking_pipe_missing_reports_connect_error_without_creating_endpoint() {
    let pipe = format!(
        r"\\.\pipe\RustDeskAgentTestMissing-{}",
        uuid::Uuid::new_v4()
    );
    let exe = std::env::current_exe().unwrap();
    let before = std::fs::read(&exe).unwrap();
    let error = super::super::super::health_windows::probe_blocking(
        &pipe,
        &exe,
        2026101501,
        Duration::from_millis(250),
    )
    .unwrap_err();
    assert!(error.starts_with("health blocking connect:"), "{error}");
    assert_eq!(std::fs::read(&exe).unwrap(), before);
}

#[hbb_common::tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn blocking_pipe_malformed_response_preserves_executable() {
    use hbb_common::tokio::io::{AsyncReadExt, AsyncWriteExt};
    let pipe = format!(
        r"\\.\pipe\RustDeskAgentTestMalformed-{}",
        uuid::Uuid::new_v4()
    );
    let mut listener = super::super::super::health_windows::bind(&pipe).unwrap();
    let server = hbb_common::tokio::spawn(async move {
        let mut stream = listener.next().await.unwrap().unwrap();
        let count = stream.read_u32_le().await.unwrap();
        assert!(count <= 4096);
        let mut request = vec![0; count as usize];
        stream.read_exact(&mut request).await.unwrap();
        super::super::super::health::Request::decode(&request).unwrap();
        stream.write_u32_le(7).await.unwrap();
        stream.write_all(b"{broken").await.unwrap();
        stream.flush().await.unwrap();
        hbb_common::tokio::time::sleep(Duration::from_millis(500)).await;
    });
    let exe = std::env::current_exe().unwrap();
    let before = std::fs::read(&exe).unwrap();
    let probe_exe = exe.clone();
    let error = hbb_common::tokio::task::spawn_blocking(move || {
        super::super::super::health_windows::probe_blocking(
            &pipe,
            &probe_exe,
            2026101501,
            Duration::from_secs(2),
        )
    })
    .await
    .unwrap()
    .unwrap_err();
    assert_eq!(error, "health response malformed");
    server.await.unwrap();
    assert_eq!(std::fs::read(&exe).unwrap(), before);
}

#[hbb_common::tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn blocking_pipe_wrong_existing_image_is_not_a_missing_file_failure() {
    let pipe = format!(
        r"\\.\pipe\RustDeskAgentTestWrongImage-{}",
        uuid::Uuid::new_v4()
    );
    let listener = super::super::super::health_windows::bind(&pipe).unwrap();
    let server = hbb_common::tokio::spawn(super::super::super::health_windows::serve(
        listener,
        "1.5.3".into(),
        2026101501,
        || false,
    ));
    let other =
        std::env::temp_dir().join(format!("status-wrong-image-{}.exe", uuid::Uuid::new_v4()));
    std::fs::copy(std::env::current_exe().unwrap(), &other).unwrap();
    let before = std::fs::read(&other).unwrap();
    let expected = other.clone();
    let error = hbb_common::tokio::task::spawn_blocking(move || {
        super::super::super::health_windows::probe_blocking(
            &pipe,
            &expected,
            2026101501,
            Duration::from_secs(2),
        )
    })
    .await
    .unwrap()
    .unwrap_err();
    assert_eq!(error, "health server executable mismatch");
    assert_eq!(std::fs::read(&other).unwrap(), before);
    server.abort();
    let _ = server.await;
    std::fs::remove_file(other).unwrap();
}

fn dual_token_root() -> PathBuf {
    base::lab_guard::require_lab_os("win10");
    let root = PathBuf::from(std::env::var_os("T04Z_DUAL_ROOT").unwrap());
    assert_eq!(
        root,
        PathBuf::from(r"C:\RustDeskAgentLab\t04z-dual-20261016")
    );
    root
}

#[hbb_common::tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "Win10 isolated dual-token fixture server; explicitly selected only"]
async fn dual_token_fixture_server() {
    let root = dual_token_root();
    let pipe = std::fs::read_to_string(root.join("pipe.txt")).unwrap();
    let listener = super::super::super::health_windows::bind(&pipe).unwrap();
    let owner = super::super::super::handoff::ObservedProcess::open(std::process::id()).unwrap();
    let exe = std::env::current_exe().unwrap();
    let hash = hex::encode(Sha256::digest(std::fs::read(&exe).unwrap()));
    let server = hbb_common::tokio::spawn(super::super::super::health_windows::serve(
        listener,
        "1.5.3".into(),
        2026101501,
        || false,
    ));
    std::fs::write(
        root.join("owner.json"),
        serde_json::to_vec(&serde_json::json!({
            "pid":owner.owner().pid,"process_created_at":owner.owner().created_at,
            "executable":exe,"sha256":hash,
        }))
        .unwrap(),
    )
    .unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(45);
    while !(root.join("admin.json").is_file() && root.join("system.json").is_file()) {
        assert!(
            std::time::Instant::now() < deadline,
            "dual-token client receipts missing"
        );
        hbb_common::tokio::time::sleep(Duration::from_millis(100)).await;
    }
    server.abort();
    let _ = server.await;
}

#[test]
#[ignore = "Win10 isolated dual-token fixture client; explicitly selected only"]
fn dual_token_fixture_client() {
    let root = dual_token_root();
    let role = std::env::var("T04Z_DUAL_ROLE").unwrap();
    assert!(matches!(role.as_str(), "admin" | "system"));
    let pipe = std::fs::read_to_string(root.join("pipe.txt")).unwrap();
    let owner: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("owner.json")).unwrap()).unwrap();
    let exe = PathBuf::from(owner["executable"].as_str().unwrap());
    let hash = hex::encode(Sha256::digest(std::fs::read(&exe).unwrap()));
    assert_eq!(owner["sha256"], hash);
    let first = super::super::super::health_windows::probe_blocking(
        &pipe,
        &exe,
        2026101501,
        Duration::from_secs(3),
    )
    .unwrap();
    let second = super::super::super::health_windows::probe_blocking(
        &pipe,
        &exe,
        2026101501,
        Duration::from_secs(3),
    )
    .unwrap();
    assert_ne!(
        first.nonce, second.nonce,
        "each real probe must issue a new nonce"
    );
    for response in [&first, &second] {
        assert_eq!(serde_json::json!(response.pid), owner["pid"]);
        assert_eq!(
            serde_json::json!(response.process_created_at),
            owner["process_created_at"]
        );
        assert!(response.local_responsive);
        assert!(!response.registered);
    }
    let application = application_snapshot(&pipe, &exe, 2026101501);
    assert_eq!(application["state"], "ready");
    assert_eq!(
        application.get("registered"),
        Some(&serde_json::json!(false))
    );
    let result = serde_json::json!({"role":role,"observer_pid":std::process::id(),
        "observed_at":chrono::Utc::now().to_rfc3339(),"owner":owner,
        "first":first,"second":second,"application":application});
    std::fs::write(
        root.join(format!("{role}.json")),
        serde_json::to_vec_pretty(&result).unwrap(),
    )
    .unwrap();
    println!("T04Z_DUAL={result}");
}

async fn boundary_probe_response(fault: &'static str) -> String {
    use hbb_common::tokio::io::{AsyncReadExt, AsyncWriteExt};
    let pipe = format!(
        r"\\.\pipe\RustDeskAgentTestBoundary-{}",
        uuid::Uuid::new_v4()
    );
    let mut listener = super::super::super::health_windows::bind(&pipe).unwrap();
    let exe = std::env::current_exe().unwrap();
    let before = hex::encode(Sha256::digest(std::fs::read(&exe).unwrap()));
    let server = hbb_common::tokio::spawn(async move {
        let mut stream = listener.next().await.unwrap().unwrap();
        let count = stream.read_u32_le().await.unwrap();
        assert!(count <= 4096);
        let mut bytes = vec![0; count as usize];
        stream.read_exact(&mut bytes).await.unwrap();
        let request = super::super::super::health::Request::decode(&bytes).unwrap();
        if fault == "timeout" {
            hbb_common::tokio::time::sleep(Duration::from_millis(700)).await;
            return;
        }
        let (pid, created_at) = {
            let process =
                super::super::super::handoff::ObservedProcess::open(std::process::id()).unwrap();
            (process.owner().pid, process.owner().created_at)
        };
        let mut response = super::super::super::health::Response::new(
            &request,
            pid,
            created_at,
            "1.5.3".into(),
            2026101501,
            true,
            false,
        )
        .unwrap();
        match fault {
            "nonce" => {
                let replacement = if request.nonce == "a".repeat(64) {
                    "b"
                } else {
                    "a"
                };
                response.nonce = replacement.repeat(64);
            }
            "created_at" => response.process_created_at += 1,
            "pid" => response.pid += 1,
            "build" => response.build_seq += 1,
            _ => panic!("unknown fixture fault"),
        }
        let bytes = serde_json::to_vec(&response).unwrap();
        stream.write_u32_le(bytes.len() as u32).await.unwrap();
        stream.write_all(&bytes).await.unwrap();
        stream.flush().await.unwrap();
        hbb_common::tokio::time::sleep(Duration::from_millis(350)).await;
    });
    let expected = exe.clone();
    let started = std::time::Instant::now();
    let error = hbb_common::tokio::task::spawn_blocking(move || {
        super::super::super::health_windows::probe_blocking(
            &pipe,
            &expected,
            2026101501,
            Duration::from_millis(250),
        )
    })
    .await
    .unwrap()
    .unwrap_err();
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "probe must remain bounded"
    );
    server.await.unwrap();
    assert_eq!(
        hex::encode(Sha256::digest(std::fs::read(&exe).unwrap())),
        before
    );
    println!("BOUNDARY fault={fault} error={error}");
    error
}

#[hbb_common::tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn new_boundary_blocking_nonce_mismatch_is_rejected() {
    assert_eq!(
        boundary_probe_response("nonce").await,
        "health response identity/readiness mismatch"
    );
}

#[hbb_common::tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn new_boundary_blocking_owner_creation_pid_and_build_mismatch_are_rejected() {
    for fault in ["created_at", "pid", "build"] {
        assert_eq!(
            boundary_probe_response(fault).await,
            "health response identity/readiness mismatch",
            "{fault}"
        );
    }
}

#[hbb_common::tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn new_boundary_blocking_stalled_peer_has_read_deadline() {
    assert_eq!(
        boundary_probe_response("timeout").await,
        "health blocking read timeout"
    );
}
