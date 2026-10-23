use super::{identity, recovery, recovery_windows, unix_timestamp, AgentPaths, AgentState};
use hbb_common::{
    base64::{engine::general_purpose::STANDARD, Engine as _},
    sodiumoxide::crypto::sign,
};
use sha2::{Digest, Sha256};
use std::{io::Read, time::Duration};

#[derive(serde::Deserialize)]
struct BuildIdentity {
    version: String,
    build_seq: u64,
}

fn capabilities() -> &'static [&'static str] {
    &[
        "agent.status@1",
        "service.inspect@1",
        "service.start@1",
        "service.restart@1",
        "service.repair@1",
        "recovery.reconcile@1",
        "package.verify@1",
        "diagnostics.collect@1",
        "network.probe@1",
        "diagnostics.logs@1",
    ]
}

fn capabilities_for(state: &AgentState) -> Vec<&'static str> {
    let mut result = capabilities().to_vec();
    if cfg!(all(windows, target_arch = "x86_64"))
        && state.package_kind == super::PackageKind::Msi
        && matches!(
            state.edition,
            super::Edition::Standard | super::Edition::Sos
        )
    {
        result.extend(["package.reinstall@1", "package.update@1", "agent.update@1"]);
    }
    result
}

fn recovery_enabled(state: &AgentState, paths: &AgentPaths, intent: recovery::Intent, disabled_start: bool) -> bool {
    state.management_agent_enabled && !state.disabled
        && intent == recovery::Intent::Enabled && !disabled_start
        && !super::purge_marker_path(paths).exists()
        && !paths.state.join("disable-agent").exists()
        && !paths.state.join("maintenance.json").exists()
        && paths.state.join("service-recovery-enabled").is_file()
}

fn application_snapshot(pipe: &str, executable: &std::path::Path, build: u64) -> serde_json::Value {
    #[cfg(test)]
    let result = TEST_HEALTH_OBSERVATION
        .with(|input| {
            input.borrow().as_ref().map(|input| (input.probe)(executable, build))
        })
        .unwrap_or_else(|| super::health_windows::probe_blocking(pipe, executable, build, Duration::from_secs(3)));
    #[cfg(not(test))]
    let result = super::health_windows::probe_blocking(pipe, executable, build, Duration::from_secs(3));
    match result {
        Ok(response) => serde_json::json!({
            "state":"ready", "probe_supported":true, "registered":response.registered,
            "pid":response.pid, "process_created_at":response.process_created_at,
            "version":response.version, "build_seq":response.build_seq,
        }),
        Err(_) => serde_json::json!({"state":"unknown","probe_supported":true,"error_code":"health_unconfirmed"}),
    }
}

fn operation_snapshot(paths: &AgentPaths) -> serde_json::Value {
    let unknown = || serde_json::json!({"state":"unknown","command_id":null,"transaction_id":null});
    let entries = match std::fs::read_dir(&paths.state) {
        Ok(entries) => entries,
        Err(_) => return unknown(),
    };
    let mut active = serde_json::json!({"state":"idle","command_id":null,"transaction_id":null});
    let mut inspected = 0;
    for entry in entries {
        let entry = match entry { Ok(entry) => entry, Err(_) => return unknown() };
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue; };
        let Some(generation) = name.strip_prefix("commands-").and_then(|name| name.strip_suffix(".json")) else { continue; };
        if generation.is_empty() || !generation.bytes().all(|byte| byte.is_ascii_digit()) { continue; }
        inspected += 1;
        if inspected > 64 { return unknown(); }
        let metadata = match entry.file_type() { Ok(metadata) => metadata, Err(_) => return unknown() };
        if !metadata.is_file() || metadata.is_symlink() { return unknown(); }
        use std::os::windows::fs::MetadataExt;
        if std::fs::symlink_metadata(entry.path()).map(|metadata| metadata.file_attributes() & 0x400 != 0).unwrap_or(true) {
            return unknown();
        }
        let journal = match super::journal::CommandJournal::open(&entry.path()) {
            Ok(journal) => journal,
            Err(_) => return unknown(),
        };
        let operation = journal.operation_snapshot();
        if operation["state"] == "reconciliation_required" ||
            (operation["state"] == "running" && active["state"] != "reconciliation_required") {
            active = operation;
        } else if operation["state"] == "deferred" && active["state"] == "idle" {
            active = operation;
        }
    }
    if paths.state.join("maintenance.json").exists() && active["state"] == "idle" {
        active["state"] = "maintenance".into();
    }
    active
}

pub fn snapshot(
    state: &AgentState,
    paths: &AgentPaths,
    installation: &str,
    boot: &str,
    sequence: u64,
) -> Result<serde_json::Value, String> {
    #[cfg(test)]
    let identity = TEST_HEALTH_OBSERVATION
        .with(|input| input.borrow().as_ref().map(|input| (input.identity)()))
        .unwrap_or_else(identity::load_existing)?;
    #[cfg(not(test))]
    let identity = identity::load_existing()?;
    #[cfg(test)]
    let observed = TEST_HEALTH_OBSERVATION.with(|input| {
        input.borrow().as_ref().map(|input| (input.observe)())
    }).unwrap_or_else(|| recovery_windows::inspect(state, paths))?;
    #[cfg(not(test))]
    let observed = recovery_windows::inspect(state, paths)?;
    #[cfg(test)]
    let observed = {
        let mut observed = observed;
        TEST_START_TYPE.with(|value| {
            if let Some(start_type) = value.borrow().as_ref() {
                observed.start_type = start_type.clone();
                observed.disabled_start = start_type == "Disabled";
            }
        });
        observed
    };
    let intent = recovery_windows::read_intent().unwrap_or(recovery::Intent::Unknown);
    let build: BuildIdentity = serde_json::from_str(include_str!("../../version.json"))
        .map_err(|error| format!("agent build metadata: {error}"))?;
    let service = match observed.state {
        recovery::ServiceState::Absent => "absent",
        recovery::ServiceState::Stopped => "stopped",
        recovery::ServiceState::StartPending => "start_pending",
        recovery::ServiceState::StopPending => "stop_pending",
        recovery::ServiceState::Running => "running",
        recovery::ServiceState::Paused => "paused",
        recovery::ServiceState::Unknown => "unknown",
    };
    let mut application = serde_json::json!({"state":"unknown","probe_supported":true});
    if observed.trusted_path && observed.state == recovery::ServiceState::Running {
        let executable = super::normal_client_exe(&state.normal_install_path)
            .filter(|path| path.parent().is_some_and(super::approved_normal_install_path))
            .filter(|path| recovery::service_path_matches(&observed.command, &path.to_string_lossy()))
            .unwrap_or_else(|| paths.client.join("RustDesk.exe"));
        application = application_snapshot(r"\\.\pipe\RustDesk\management-health-v1", &executable, build.build_seq);
    }
    let mut payload = serde_json::json!({
        "schema": 1,
        "client_id": identity.id,
        "client_uuid": identity.uuid,
        "agent_installation_id": installation,
        "boot_id": boot,
        "sequence": sequence,
        "session_generation": state.status_session_generation,
        "agent": { "version": build.version, "build_seq": build.build_seq, "executor_version": 1 },
        "capabilities": capabilities_for(state),
        "package_identity": { "edition": state.edition.name(), "arch": std::env::consts::ARCH, "kind": state.package_kind.extension() },
        "service": { "state": service, "binpath": observed.command, "pid": observed.process_id, "trusted_path": observed.trusted_path, "account": observed.account, "start_type": if observed.disabled_start { "disabled" } else { &observed.start_type } },
        "application": application,
        "recovery": { "effective_enabled": recovery_enabled(state, paths, intent, observed.disabled_start), "applied_policy_revision": state.management_agent_policy_revision },
        "intent": { "state": match intent { recovery::Intent::Enabled => "enabled", recovery::Intent::Disabled => "disabled", recovery::Intent::Unknown => "unknown" } },
        "files": { "normal_present": observed.normal_present, "fallback_present": observed.fallback_present },
        "rescue": super::rescue_status::snapshot(&paths.agent, installation, &state.target_key),
        "operation": operation_snapshot(paths),
        "last_service_recovery_at": state.last_service_recovery_at,
        "service_recovery_error": state.service_recovery_error,
    });
    let binding_path = paths.state.join("local-events").join("events.json");
    let binding_error = match std::fs::symlink_metadata(&binding_path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(_) => Some("binding_store_unavailable"),
        Ok(_) => {
            match super::local_events::LocalEvents::open(&binding_path) {
                Ok(log) => {
                    if let Some(binding) = log.verified_binding().filter(|binding| {
                        binding.client_id == identity.id
                            && binding.client_uuid == identity.uuid
                            && binding.agent_installation_id == installation
                    }) {
                        payload["enrollment_generation"] = binding.enrollment_generation.into();
                    }
                    None
                }
                Err(super::local_events::EventError::Storage | super::local_events::EventError::DiskFull) => Some("binding_store_unavailable"),
                Err(_) => Some("binding_store_invalid"),
            }
        }
    };
    if let Some(code) = binding_error {
        payload["enrollment_diagnostic"] = serde_json::json!({"state":"unavailable","error_code":code});
        hbb_common::throttled_log!(Duration::from_secs(60), warn, "agent status enrollment omitted: {code}");
    }
    Ok(payload)
}

pub fn send(payload: &serde_json::Value) -> Result<(), String> {
    let identity = identity::load_existing()?;
    send_with_identity(
        payload,
        &identity,
        "https://rdapi.yan.life/rd/update/v1/agent/status",
    )
}

fn send_with_identity(
    payload: &serde_json::Value,
    identity: &identity::DeviceIdentity,
    url: &str,
) -> Result<(), String> {
    if payload["client_id"].as_str() != Some(&identity.id)
        || payload["client_uuid"].as_str() != Some(&identity.uuid)
    {
        return Err("agent status identity changed".into());
    }
    let body = serde_json::to_vec(payload).map_err(|error| error.to_string())?;
    if body.len() > 65536 {
        return Err("agent status too large".into());
    }
    let timestamp = unix_timestamp() as i64;
    let nonce = hex::encode(hbb_common::sodiumoxide::randombytes::randombytes(16));
    let canonical = super::update_device_auth_payload_for_agent(
        "POST",
        "/rd/update/v1/agent/status",
        &identity.id,
        &identity.uuid,
        timestamp,
        &nonce,
        &hex::encode(Sha256::digest(&body)),
    );
    let signature = sign::sign_detached(&canonical, &identity.secret);
    let response = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|error| error.to_string())?
        .post(url)
        .header("Content-Type", "application/json")
        .header("X-RustDesk-Device-ID", &identity.id)
        .header(
            "X-RustDesk-Device-Public-Key",
            STANDARD.encode(identity.public.as_ref()),
        )
        .header("X-RustDesk-Device-Timestamp", timestamp.to_string())
        .header("X-RustDesk-Device-Nonce", nonce)
        .header(
            "X-RustDesk-Device-Signature",
            STANDARD.encode(signature.to_bytes()),
        )
        .body(body)
        .send()
        .map_err(|error| error.to_string())?;
    let code = response.status();
    if !code.is_success() {
        return Err(format!("agent status HTTP {code}"));
    }
    let mut acknowledgement = Vec::new();
    response
        .take(65537)
        .read_to_end(&mut acknowledgement)
        .map_err(|error| error.to_string())?;
    if acknowledgement.len() > 65536 {
        return Err("agent status acknowledgement too large".into());
    }
    let ack: serde_json::Value =
        serde_json::from_slice(&acknowledgement).map_err(|error| error.to_string())?;
    if ack["ok"] != true || ack["accepted_sequence"] != payload["sequence"] {
        return Err("agent status acknowledgement mismatch".into());
    }
    Ok(())
}

#[cfg(test)]
std::thread_local! {
    static TEST_START_TYPE: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
    static TEST_HEALTH_OBSERVATION: std::cell::RefCell<Option<TestHealthObservation>> = const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
struct TestHealthObservation {
    identity: fn() -> Result<identity::DeviceIdentity, String>,
    observe: Box<dyn Fn() -> Result<recovery_windows::ServiceObservation, String>>,
    probe: Box<dyn Fn(&std::path::Path, u64) -> Result<super::health::Response, String>>,
}

#[cfg(test)]
mod tests {
    use super::*;
    mod health_observation {
        include!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/management_agent_status_contract/health_observation.rs"));
    }
    mod delivery_contract {
        include!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/management_agent_status_contract/mod.rs"));
    }
    use std::{
        io::{Read, Write},
        net::TcpListener,
        thread,
    };

    #[hbb_common::tokio::test]
    async fn application_projection_requires_live_identity_proof_and_keeps_registration_separate() {
        let pipe = format!(r"\\.\pipe\RustDeskAgentTestStatus-{}", std::process::id());
        let listener = super::super::health_windows::bind(&pipe).unwrap();
        let server = hbb_common::tokio::spawn(super::super::health_windows::serve(listener, "1.5.3".into(), 2026100502, || false));
        hbb_common::tokio::task::spawn_blocking(move || {
            let exe = std::env::current_exe().unwrap();
            let value = application_snapshot(&pipe, &exe, 2026100502);
            assert_eq!(value["state"], "ready");
            assert_eq!(value["probe_supported"], true);
            assert_eq!(value["registered"], false);
            assert_eq!(value["pid"], std::process::id());
            let mismatch = application_snapshot(&pipe, &exe, 2026100503);
            assert_eq!(mismatch["state"], "unknown");
            assert_eq!(mismatch["error_code"], "health_unconfirmed");
            assert!(mismatch.get("registered").is_none());
            assert!(!mismatch.to_string().contains(&exe.to_string_lossy().to_string()));
        }).await.unwrap();
        server.abort();
        let _ = server.await;
    }

    #[test]
    fn effective_recovery_reflects_all_local_blocks_without_mutating_them() {
        let root = std::env::temp_dir().join(format!("agent-effective-recovery-{}", std::process::id()));
        let paths = AgentPaths::from_program_data(&root, super::super::Edition::Standard);
        std::fs::create_dir_all(&paths.state).unwrap();
        let mut state = AgentState::new(super::super::Edition::Standard, &paths);
        assert!(!recovery_enabled(&state, &paths, recovery::Intent::Enabled, false));
        std::fs::write(paths.state.join("service-recovery-enabled"), b"enabled").unwrap();
        assert!(recovery_enabled(&state, &paths, recovery::Intent::Enabled, false));
        for marker in ["maintenance.json", "disable-agent", "disable-recovery"] {
            let file = paths.state.join(marker);
            std::fs::write(&file, b"preserve").unwrap();
            assert!(!recovery_enabled(&state, &paths, recovery::Intent::Enabled, false), "{marker}");
            assert_eq!(std::fs::read(&file).unwrap(), b"preserve");
            std::fs::remove_file(file).unwrap();
        }
        for intent in [recovery::Intent::Disabled, recovery::Intent::Unknown] {
            assert!(!recovery_enabled(&state, &paths, intent, false));
        }
        assert!(!recovery_enabled(&state, &paths, recovery::Intent::Enabled, true));
        state.disabled = true;
        assert!(!recovery_enabled(&state, &paths, recovery::Intent::Enabled, false));
        state.disabled = false;
        state.management_agent_enabled = false;
        assert!(!recovery_enabled(&state, &paths, recovery::Intent::Enabled, false));
        assert_eq!(std::fs::read(paths.state.join("service-recovery-enabled")).unwrap(), b"enabled");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn operation_projection_preserves_journals_and_reports_uncertainty() {
        let root = std::env::temp_dir().join(format!("agent-operation-status-{}", std::process::id()));
        let paths = AgentPaths::from_program_data(&root, super::super::Edition::Standard);
        assert_eq!(operation_snapshot(&paths)["state"], "unknown");
        assert!(!paths.state.exists());
        std::fs::create_dir_all(&paths.state).unwrap();
        assert_eq!(operation_snapshot(&paths)["state"], "idle");
        let file = paths.state.join("commands-1.json");
        let mut journal = super::super::journal::CommandJournal::open(&file).unwrap();
        let id = "a".repeat(32);
        journal.prepare(&id, &"b".repeat(64), 2000, 1000).unwrap();
        journal.receipt(&id).unwrap();
        journal.start(&id, 1001).unwrap();
        let before = std::fs::read(&file).unwrap();
        assert_eq!(operation_snapshot(&paths)["state"], "running");
        assert_eq!(operation_snapshot(&paths)["command_id"], id);
        assert_eq!(std::fs::read(&file).unwrap(), before);
        std::fs::write(paths.state.join("commands-2.json"), b"invalid").unwrap();
        assert_eq!(operation_snapshot(&paths)["state"], "unknown");
        assert_eq!(std::fs::read(paths.state.join("commands-2.json")).unwrap(), b"invalid");
        std::fs::remove_file(paths.state.join("commands-2.json")).unwrap();
        journal.finish(&id, "failed", serde_json::json!({"error_code":"fixture"}), 1002).unwrap();
        std::fs::write(paths.state.join("maintenance.json"), b"local maintenance").unwrap();
        assert_eq!(operation_snapshot(&paths)["state"], "maintenance");
        assert_eq!(std::fs::read(paths.state.join("maintenance.json")).unwrap(), b"local maintenance");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn status_snapshot_declares_stable_agent_capabilities() {
        let actual = capabilities();
        for required in [
            "agent.status@1",
            "service.inspect@1",
            "service.start@1",
            "service.restart@1",
            "service.repair@1",
            "recovery.reconcile@1",
            "package.verify@1",
            "diagnostics.collect@1",
            "network.probe@1",
            "diagnostics.logs@1",
        ] {
            assert!(actual.contains(&required), "{required}");
        }
        assert_eq!(actual.len(), actual.iter().collect::<std::collections::HashSet<_>>().len());
        for unsupported in ["cmd.execute@1", "package.update@1", "package.reinstall@1", "agent.update@1"] {
            assert!(!actual.contains(&unsupported));
        }
    }

    #[test]
    #[ignore = "Win11 SYSTEM read-only installed snapshot; no service mutations"]
    fn installed_snapshot_declares_msi_package_capabilities() {
        base::lab_guard::require_lab_os("win11");
        let root = std::path::Path::new(r"C:\ProgramData");
        let paths = AgentPaths::from_program_data(root, super::super::Edition::Sos);
        let bytes = std::fs::read(paths.state.join("agent.json")).unwrap();
        let state: AgentState = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(
            state.agent_installation_id,
            "dcc73dd2fba22bf04c7df497053ae2df"
        );
        assert_eq!(state.package_kind, super::super::PackageKind::Msi);
        let payload = snapshot(
            &state,
            &paths,
            &state.agent_installation_id,
            &"b".repeat(32),
            1,
        )
        .unwrap();
        assert_eq!(
            std::fs::read(paths.state.join("agent.json")).unwrap(),
            bytes
        );
        assert_eq!(payload["package_identity"]["kind"], "msi");
        assert_eq!(payload["application"]["state"], "ready");
        println!("CAPABILITY_SNAPSHOT={}", payload);
        let actual = payload["capabilities"].as_array().unwrap();
        for required in ["package.reinstall@1", "package.update@1", "agent.update@1"] {
            assert!(
                actual.iter().any(|value| value == required),
                "missing {required}"
            );
        }
        assert_eq!(actual.len(), 12);
        for edition in [super::super::Edition::Standard, super::super::Edition::Sos] {
            for kind in [
                super::super::PackageKind::Msi,
                super::super::PackageKind::Exe,
            ] {
                let mut variant = state.clone();
                variant.edition = edition;
                variant.package_kind = kind;
                let payload = snapshot(
                    &variant,
                    &paths,
                    &variant.agent_installation_id,
                    &"b".repeat(32),
                    2,
                )
                .unwrap();
                let values = payload["capabilities"].as_array().unwrap();
                let supported = cfg!(all(windows, target_arch = "x86_64"))
                    && kind == super::super::PackageKind::Msi;
                assert_eq!(
                    values.len(),
                    if supported { 12 } else { 9 },
                    "{edition:?}/{kind:?}"
                );
                for capability in ["package.reinstall@1", "package.update@1", "agent.update@1"] {
                    assert_eq!(values.iter().any(|v| v == capability), supported);
                }
                assert!(!values.iter().any(|v| v == "cmd.execute@1"));
            }
        }
        assert_eq!(
            std::fs::read(paths.state.join("agent.json")).unwrap(),
            bytes
        );
    }

    fn test_identity() -> identity::DeviceIdentity {
        hbb_common::sodiumoxide::init().unwrap();
        let (public, secret) = sign::gen_keypair();
        identity::DeviceIdentity {
            id: "test-device".into(),
            uuid: "test-uuid".into(),
            secret,
            public,
        }
    }

    fn response_fixture(response: &str) -> (String, thread::JoinHandle<Vec<u8>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!(
            "http://{}/rd/update/v1/agent/status",
            listener.local_addr().unwrap()
        );
        let response = response.to_owned();
        let handle = thread::spawn(move || {
            let deadline = std::time::Instant::now() + Duration::from_secs(10);
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(error)
                        if error.kind() == std::io::ErrorKind::WouldBlock
                            && std::time::Instant::now() < deadline =>
                    {
                        thread::sleep(Duration::from_millis(10))
                    }
                    Err(error) => panic!("fixture accept: {error}"),
                }
            };
            stream.set_nonblocking(false).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut request = Vec::new();
            let mut buffer = [0; 4096];
            loop {
                let read = stream.read(&mut buffer).unwrap();
                assert!(read > 0, "incomplete HTTP request");
                request.extend_from_slice(&buffer[..read]);
                if let Some(end) = request.windows(4).position(|bytes| bytes == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&request[..end]).to_ascii_lowercase();
                    let length: usize = headers
                        .lines()
                        .find_map(|line| line.strip_prefix("content-length:").map(str::trim))
                        .unwrap()
                        .parse()
                        .unwrap();
                    if request.len() >= end + 4 + length {
                        break;
                    }
                }
                assert!(request.len() <= 131072);
            }
            stream.write_all(response.as_bytes()).unwrap();
            request
        });
        (url, handle)
    }

    #[test]
    fn sends_signed_body_and_checks_acknowledgement() {
        let identity = test_identity();
        let payload = serde_json::json!({ "client_id": identity.id, "client_uuid": identity.uuid, "sequence": 9 });
        let body = r#"{"ok":true,"accepted_sequence":9}"#;
        let (url, fixture) = response_fixture(&format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        ));
        send_with_identity(&payload, &identity, &url).unwrap();
        let request = fixture.join().unwrap();
        let end = request
            .windows(4)
            .position(|bytes| bytes == b"\r\n\r\n")
            .unwrap();
        let raw_headers = String::from_utf8(request[..end].to_vec()).unwrap();
        let header = |name: &str| -> String {
            raw_headers
                .lines()
                .find_map(|line| {
                    let (key, value) = line.split_once(':')?;
                    key.eq_ignore_ascii_case(name)
                        .then(|| value.trim().to_owned())
                })
                .unwrap()
        };
        let canonical = super::super::update_device_auth_payload_for_agent(
            "POST",
            "/rd/update/v1/agent/status",
            &identity.id,
            &identity.uuid,
            header("x-rustdesk-device-timestamp").parse().unwrap(),
            &header("x-rustdesk-device-nonce"),
            &hex::encode(Sha256::digest(&request[end + 4..])),
        );
        let signature = STANDARD
            .decode(header("x-rustdesk-device-signature"))
            .unwrap();
        assert!(sign::verify_detached(
            &sign::Signature::from_bytes(&signature).unwrap(),
            &canonical,
            &identity.public
        ));
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&request[end + 4..]).unwrap(),
            payload
        );
    }

    #[test]
    fn refuses_identity_change_before_network_and_rejects_bad_ack() {
        let identity = test_identity();
        let mismatched =
            serde_json::json!({"client_id":"other", "client_uuid":identity.uuid, "sequence":9});
        assert_eq!(
            send_with_identity(&mismatched, &identity, "http://127.0.0.1:1").unwrap_err(),
            "agent status identity changed"
        );
        let payload =
            serde_json::json!({ "client_id":identity.id,"client_uuid":identity.uuid,"sequence":9 });
        for body in [r#"{"ok":true,"accepted_sequence":8}"#, "not-json"] {
            let (url, fixture) = response_fixture(&format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            ));
            assert!(send_with_identity(&payload, &identity, &url).is_err());
            fixture.join().unwrap();
        }
    }
}
