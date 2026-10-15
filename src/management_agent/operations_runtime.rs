use super::{
    identity,
    journal::{CommandJournal, ExecutionState},
    network_policy, operations, package_gate, recovery_windows, unix_timestamp, workspace, AgentPaths, AgentState,
};
use hbb_common::{
    base64::{engine::general_purpose::STANDARD, Engine as _},
    sodiumoxide::crypto::sign,
};
use std::{io::Read, time::Duration};

const API: &str = "https://rdapi.yan.life";

#[cfg(test)]
std::thread_local! {
    static LOCAL_PACKAGE_FIXTURE: std::cell::RefCell<Option<(Vec<u8>, std::net::SocketAddr)>> = const { std::cell::RefCell::new(None) };
}

fn verify_resolved_target<'a>(manifest: &'a hbb_common::UpdateManifest, key: &str, current: u64) -> Result<&'a hbb_common::UpdateTarget, String> {
    #[cfg(test)]
    if let Some((public, _)) = LOCAL_PACKAGE_FIXTURE.with(|value| value.borrow().clone()) {
        return package_gate::verify_with_key(manifest, key, current, "fixture-key", &public);
    }
    package_gate::verify(manifest, key, current)
}

fn fixed_network_probe() -> Result<serde_json::Value, String> {
    super::network_probe::fixed(Duration::from_secs(30))
}

fn activity_deferral(kind: &str, state: &AgentState, paths: &AgentPaths) -> Option<&'static str> {
    operations::activity_deferral(kind, None)?;
    let observed = match recovery_windows::inspect(state, paths) {
        Ok(observed) => observed,
        Err(_) => return Some("activity_unknown"),
    };
    if matches!(observed.state, super::recovery::ServiceState::Absent | super::recovery::ServiceState::Stopped) {
        return None;
    }
    if observed.state != super::recovery::ServiceState::Running { return Some("activity_unknown"); }
    if !observed.trusted_path { return Some("activity_unknown"); }
    let executable = super::normal_client_exe(&state.normal_install_path)
        .filter(|path| path.parent().is_some_and(super::approved_normal_install_path))
        .filter(|path| super::recovery::service_path_matches(&observed.command, &path.to_string_lossy()))
        .unwrap_or_else(|| paths.client.join("RustDesk.exe"));
    let build: serde_json::Value = match serde_json::from_str(include_str!("../../version.json")) {
        Ok(build) => build,
        Err(_) => return Some("activity_unknown"),
    };
    let idle = super::health_windows::probe_blocking(r"\\.\pipe\RustDesk\management-health-v2",
        &executable, build["build_seq"].as_u64().unwrap_or(0), Duration::from_secs(3))
        .ok().and_then(|response| response.no_active_sessions);
    operations::activity_deferral(kind, idle)
}

struct ApprovedPackages {
    manifest: hbb_common::UpdateManifest,
    rollback_manifest: hbb_common::UpdateManifest,
    package: std::path::PathBuf,
    rollback_package: std::path::PathBuf,
    rollback_reference: String,
    _locks: Vec<package_gate::LockedPackage>,
}

fn resolve_approved_package(
    identity: &identity::DeviceIdentity,
    base: &str,
    binding: &serde_json::Value,
    state: &AgentState,
    command: &operations::CommandEnvelope,
    paths: &AgentPaths,
) -> Result<ApprovedPackages, PollError> {
    let command_id = command.id.as_str();
    if command_id.len() != 32 || !command_id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("approved package command identifier invalid".into());
    }
    let reference = command.args["package_id"].as_str().ok_or("approved package reference missing")?;
    let target_key = command.args["target_key"].as_str().ok_or("approved package target missing")?;
    let build = command.args["target_build_seq"].as_u64().ok_or("approved package build missing")?;
    if state.package_kind != super::PackageKind::Msi
        || target_key != state.edition.target_key_for(state.package_kind) || command.args["allow_downgrade"] != false {
        return Err("approved package command target mismatch".into());
    }
    let metadata: serde_json::Value = serde_json::from_str(include_str!("../../version.json")).map_err(|e| e.to_string())?;
    let current = metadata["build_seq"].as_u64().ok_or("current agent build missing")?;
    if build < current { return Err("approved package downgrade refused".into()); }
    let mut request_body = binding.clone();
    request_body["command_id"] = command_id.into();
    let response = request(
        identity,
        base,
        "POST",
        "/rd/update/v1/agent/package/resolve",
        command_id,
        &request_body,
    )?;
    let manifest = package_gate::bind_resolved(&response, reference, target_key, build)?;
    let target = verify_resolved_target(&manifest, target_key, current)?;
    let rollback_reference = response["rollback"]["package_id"].as_str().ok_or("approved rollback reference missing")?;
    let rollback_manifest = package_gate::bind_resolved(&response["rollback"], rollback_reference, target_key, current)?;
    let rollback_target = verify_resolved_target(&rollback_manifest, target_key, current)?;
    let (package, target_lock) = download_approved(&paths.agent.join(format!("approved-{command_id}")), target)?;
    let (rollback_package, rollback_lock) = download_approved(&paths.agent.join(format!("rollback-{command_id}")), rollback_target)?;
    Ok(ApprovedPackages {manifest, rollback_manifest, package, rollback_package, rollback_reference: rollback_reference.into(), _locks: vec![target_lock, rollback_lock]})
}

fn download_approved(directory: &std::path::Path, target: &hbb_common::UpdateTarget) -> Result<(std::path::PathBuf, package_gate::LockedPackage), PollError> {
    #[cfg(test)]
    if let Some((_, address)) = LOCAL_PACKAGE_FIXTURE.with(|value| value.borrow().clone()) {
        let started = std::time::Instant::now();
        let result = super::package_download::download_fixture(directory, target, address);
        let elapsed_ms = started.elapsed().as_millis();
        let evidence = serde_json::json!({"elapsed_ms":elapsed_ms,"size":target.size,"sha256":target.sha256,
            "primary":target.primary,"mirrors":target.mirrors,"success":result.is_ok(),"error":result.as_ref().err()});
        std::fs::write(directory.with_extension("download-timing.json"),serde_json::to_vec_pretty(&evidence).map_err(|e|e.to_string())?).map_err(|e|e.to_string())?;
        return result.map_err(Into::into);
    }
    super::package_download::download(directory, target).map_err(Into::into)
}

#[cfg(test)]
fn receive_approved(directory: &std::path::Path, target: &hbb_common::UpdateTarget, download: reqwest::blocking::Response) -> Result<(std::path::PathBuf, package_gate::LockedPackage), PollError> {
    super::package_download::receive(directory, target, download).map_err(Into::into)
}

fn launch_package_command(
    identity: &identity::DeviceIdentity,
    base: &str,
    binding: &serde_json::Value,
    state: &AgentState,
    paths: &AgentPaths,
    wire: &serde_json::Value,
) -> Result<(), PollError> {
    let generation = binding["enrollment_generation"].as_u64().ok_or("package enrollment generation absent")?;
    let command = operations::command_envelope(wire, &identity.id, &identity.uuid, &state.agent_installation_id, generation)?;
    if state.package_kind != super::PackageKind::Msi || state.disabled || !state.management_agent_enabled {
        return Err("package command disabled or package kind unsupported".into());
    }
    super::package_recovery_preflight(paths)?;
    let packages = resolve_approved_package(identity, base, binding, state, &command, paths)?;
    let metadata: serde_json::Value = serde_json::from_str(include_str!("../../version.json")).map_err(|e|e.to_string())?;
    let current = metadata["build_seq"].as_u64().ok_or("current agent build missing")?;
    let transactions = paths.agent.join("transactions");
    if transactions.exists() { workspace::verify(&transactions)?; } else { workspace::create(&transactions)?; }
    let root = transactions.join(&command.id);
    let preparation = super::preparation::Request {
        transaction_id: &command.id, command_id: &command.id, installation_id: &state.agent_installation_id,
        target_key: command.args["target_key"].as_str().ok_or("target absent")?, current_build: current,
        package: &packages.package, manifest: &packages.manifest,
        rollback_package: &packages.rollback_package, rollback_manifest: &packages.rollback_manifest,
    };
    #[cfg(test)]
    let fixture = LOCAL_PACKAGE_FIXTURE.with(|value| value.borrow().clone());
    #[cfg(test)]
    let plan = match &fixture {
        Some((public, _)) => super::preparation::prepare_fixture(&root, &preparation, public),
        None => super::preparation::prepare(&root, &preparation),
    }?;
    #[cfg(not(test))]
    let plan = super::preparation::prepare(&root, &preparation)?;
    use sha2::{Digest, Sha256};
    let record = serde_json::json!({"schema":1,"command_id":command.id,"command_digest":wire["digest"],
        "command_type":command.command_type,"client_id":identity.id,"client_uuid":identity.uuid,
        "installation_id":state.agent_installation_id,"enrollment_generation":generation,
        "package_id":command.args["package_id"],"plan_sha256":hex::encode(Sha256::digest(serde_json::to_vec(&plan).map_err(|e|e.to_string())?))});
    super::state_store::publish(&root.join("command-binding.json"), &serde_json::to_vec(&record).map_err(|e|e.to_string())?).map_err(|e|e.to_string())?;
    #[cfg(test)]
    if fixture.is_some() && paths.agent.join(format!("fixture-fail-{}", command.id)).is_file() {
        std::fs::write(root.join("fixture-validation-failure"), b"test-only validation failure").map_err(|e| e.to_string())?;
    }
    // Resolve again just before handing off: approval, permission and instance
    // may have changed while package download and backups were in progress.
    let mut recheck = binding.clone();
    recheck["command_id"] = command.id.clone().into();
    let response = request(identity, base, "POST", "/rd/update/v1/agent/package/resolve", &command.id, &recheck)?;
    if response["package_id"] != command.args["package_id"]
        || response["rollback"]["package_id"] != packages.rollback_reference {
        return Err("package approval changed during preparation".into());
    }
    if let Some(reason) = activity_deferral(&command.command_type, state, paths) { return Err(reason.into()); }
    super::package_recovery_preflight(paths)?;
    #[cfg(test)]
    if let Some((public, _)) = fixture {
        let _child = super::parent_handoff::launch_native_fixture(&root, &public)?;
        return Ok(());
    }
    let _child = super::parent_handoff::launch(&root)?;
    Ok(())
}

fn execute_command(
    command_type: &str,
    state: &AgentState,
    paths: &AgentPaths,
) -> Result<serde_json::Value, String> {
    let observed = recovery_windows::inspect(state, paths)?;
    match command_type {
        "package.verify" => Ok(fixed_integrity_report(state, paths)),
        "diagnostics.collect" => Ok(serde_json::json!({
            "schema": 1,
            "service": { "state": format!("{:?}", observed.state).to_ascii_lowercase(), "trusted_path": observed.trusted_path, "pid": observed.process_id, "disabled_start": observed.disabled_start },
            "agent": { "edition": state.edition.name(), "package_kind": state.package_kind.extension(), "session_generation": state.status_session_generation },
            "errors": { "update_error_present": !state.last_error.is_empty(), "recovery_error_present": !state.service_recovery_error.is_empty() },
            "local_gates": { "disable_recovery": super::purge_marker_path(paths).exists(), "disable_agent": paths.state.join("disable-agent").exists(), "maintenance": paths.state.join("maintenance.json").exists() },
            "integrity": fixed_integrity_report(state, paths),
        })),
        "service.inspect" => Ok(serde_json::json!({
            "service": { "state": format!("{:?}", observed.state).to_ascii_lowercase(), "binpath": observed.command, "trusted_path": observed.trusted_path, "pid": observed.process_id, "account": observed.account, "start_type": observed.start_type },
            "files": { "normal_present": observed.normal_present, "fallback_present": observed.fallback_present },
            "agent": { "installation_id": state.agent_installation_id, "edition": state.edition.name(), "package_kind": state.package_kind.extension(), "last_error": state.last_error, "recovery_error": state.service_recovery_error },
        })),
        "service.start" => {
            if observed.state == super::recovery::ServiceState::Running {
                return Ok(serde_json::json!({ "service": "running", "changed": false }));
            }
            if observed.trusted_path {
                if !recovery_windows::start_existing(state, paths)? {
                    return Err("local recovery gates refused service start".into());
                }
            } else {
                if !recovery_windows::repair_service(
                    state,
                    paths,
                    !observed.normal_present && observed.fallback_present,
                )? {
                    return Err("local recovery gates refused service repair".into());
                }
            }
            Ok(serde_json::json!({ "service": "running", "changed": true }))
        }
        "service.restart" => {
            operations::require_service_started(recovery_windows::restart_service(state, paths)?)?;
            Ok(serde_json::json!({ "service": "running", "changed": true }))
        }
        "service.repair" | "recovery.reconcile" => {
            let fallback = !observed.normal_present && observed.fallback_present;
            if observed.trusted_path && observed.state == super::recovery::ServiceState::Running {
                return Ok(serde_json::json!({ "service": "running", "action": "observe" }));
            }
            if !recovery_windows::repair_service(state, paths, fallback)? {
                return Err("fixed service repair did not start the client".into());
            }
            Ok(serde_json::json!({ "service": "running", "action": if fallback { "repair_fallback" } else { "repair_normal" } }))
        }
        "network.probe" => fixed_network_probe(),
        "package.reinstall" | "package.update" | "agent.update" => {
            Err("approved package is resolved only through the persisted installer transaction; direct command execution is intentionally gated".into())
        }
        _ => Err("unsupported agent operation".into()),
    }
}

fn fixed_integrity_report(state: &AgentState, paths: &AgentPaths) -> serde_json::Value {
    let normal = super::normal_client_exe(&state.normal_install_path)
        .filter(|path| path.parent().is_some_and(super::approved_normal_install_path))
        .map(|path| super::integrity::inspect(&path, None, None))
        .unwrap_or(super::integrity::Report { status: "unapproved_path".into(), size: None, sha256: None });
    let agent = super::integrity::inspect(&paths.agent.join("rustdesk-management-agent.exe"), None, None);
    let fallback = super::integrity::inspect(&paths.client.join("RustDesk.exe"), None, None);
    serde_json::json!({
        "schema": 1,
        "verified": false,
        "reference_status": "approved_file_manifest_unavailable",
        "objects": { "normal": normal, "agent": agent, "fallback": fallback },
    })
}

#[derive(Debug)]
pub struct PollError {
    pub failure: network_policy::Failure,
    pub retry_after: Option<u64>,
    message: String,
}

impl std::fmt::Display for PollError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl From<String> for PollError {
    fn from(message: String) -> Self {
        Self {
            failure: network_policy::Failure::Local,
            retry_after: None,
            message,
        }
    }
}
impl From<&str> for PollError {
    fn from(message: &str) -> Self {
        message.to_owned().into()
    }
}
impl From<PollError> for String {
    fn from(error: PollError) -> Self {
        error.message
    }
}

fn request(
    identity: &identity::DeviceIdentity,
    base: &str,
    method: &str,
    path: &str,
    command: &str,
    payload: &serde_json::Value,
) -> Result<serde_json::Value, PollError> {
    if !path.starts_with("/rd/update/v1/agent/") || path.contains(['?', '#']) {
        return Err("agent route invalid".into());
    }
    if payload["client_id"] != identity.id || payload["client_uuid"] != identity.uuid {
        return Err("agent request identity mismatch".into());
    }
    let body = if method == "GET" {
        Vec::new()
    } else {
        serde_json::to_vec(payload).map_err(|e| e.to_string())?
    };
    if body.len() > 65536 {
        return Err("agent request too large".into());
    }
    let timestamp = unix_timestamp() as i64;
    let nonce = hex::encode(hbb_common::sodiumoxide::randombytes::randombytes(16));
    let canonical = operations::canonical_request(
        method,
        path,
        (&identity.id, &identity.uuid),
        timestamp,
        &nonce,
        command,
        &body,
    );
    let client = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| e.to_string())?;
    let mut builder = client.request(
        reqwest::Method::from_bytes(method.as_bytes()).map_err(|e| e.to_string())?,
        format!("{base}{path}"),
    );
    if method == "GET" {
        let object = payload
            .as_object()
            .ok_or_else(|| "agent query invalid".to_owned())?;
        let pairs = object
            .iter()
            .map(|(key, value)| {
                (
                    key.clone(),
                    value
                        .as_str()
                        .map(str::to_owned)
                        .unwrap_or_else(|| value.to_string()),
                )
            })
            .collect::<Vec<_>>();
        builder = builder.query(&pairs);
    } else {
        builder = builder
            .body(body)
            .header("Content-Type", "application/json");
    }
    let response = builder
        .header("X-RustDesk-Device-ID", &identity.id)
        .header(
            "X-RustDesk-Device-Public-Key",
            STANDARD.encode(identity.public.as_ref()),
        )
        .header("X-RustDesk-Device-Timestamp", timestamp.to_string())
        .header("X-RustDesk-Device-Nonce", nonce)
        .header(
            "X-RustDesk-Device-Signature",
            STANDARD.encode(sign::sign_detached(canonical.as_bytes(), &identity.secret).to_bytes()),
        )
        .send()
        .map_err(|error| PollError {
            failure: network_policy::Failure::Transport,
            retry_after: None,
            message: error.to_string(),
        })?;
    if !response.status().is_success() {
        return Err(PollError {
            failure: network_policy::Failure::Http(response.status().as_u16()),
            retry_after: network_policy::retry_after(
                response
                    .headers()
                    .get(reqwest::header::RETRY_AFTER)
                    .and_then(|value| value.to_str().ok()),
                unix_timestamp(),
            ),
            message: format!("agent operation HTTP {}", response.status()),
        });
    }
    let mut bytes = Vec::new();
    response
        .take(262145)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > 262144 {
        return Err("agent operation response too large".into());
    }
    serde_json::from_slice(&bytes).map_err(|_| "agent operation response malformed".into())
}

pub fn poll(state: &AgentState, paths: &AgentPaths) -> Result<(), PollError> {
    if state.agent_installation_id.is_empty() || super::purge_marker_path(paths).exists() {
        return Ok(());
    }
    let identity = identity::load_existing()?;
    poll_with_identity(&identity, API, state, paths)
}

fn poll_with_identity(
    identity: &identity::DeviceIdentity,
    base: &str,
    state: &AgentState,
    paths: &AgentPaths,
) -> Result<(), PollError> {
    let mut binding = serde_json::json!({"schema":1,"client_id":identity.id,"client_uuid":identity.uuid,"agent_installation_id":state.agent_installation_id});
    let enrolled = request(
        identity,
        base,
        "POST",
        "/rd/update/v1/agent/enroll",
        "",
        &binding,
    )?;
    let generation = enrolled["enrollment_generation"]
        .as_u64()
        .filter(|value| *value > 0)
        .ok_or_else(|| "agent enrollment acknowledgement invalid".to_owned())?;
    if enrolled["ok"] != true || enrolled["agent_installation_id"] != state.agent_installation_id {
        return Err("agent enrollment identity mismatch".into());
    }
    binding["enrollment_generation"] = generation.into();
    let directory = paths.state.join("local-events");
    if directory.exists() { workspace::verify(&directory)?; } else { workspace::create(&directory)?; }
    let mut credentials = super::local_events::LocalEvents::open(&directory.join("events.json"))
        .map_err(|_| "agent enrollment binding unavailable")?;
    credentials.remember_binding(super::local_events::Binding {
        client_id: identity.id.clone(), client_uuid: identity.uuid.clone(),
        agent_installation_id: state.agent_installation_id.clone(), enrollment_generation: generation,
    }).map_err(|_| "agent enrollment binding persistence failed")?;
    super::local_event_runtime::poll(identity, base, &binding, paths);
    let mut log = CommandJournal::open(&paths.state.join(format!("commands-{generation}.json")))?;
    log.prune_confirmed(unix_timestamp())?;
    upload_events(identity, base, &binding, &mut log)?;
    let mut commands = Vec::new();
    let mut query = binding.clone();
    let mut cursor = String::new();
    for _ in 0..5 {
        let response = request(
            identity,
            base,
            "GET",
            "/rd/update/v1/agent/commands",
            "",
            &query,
        )?;
        let page = response["commands"]
            .as_array()
            .filter(|items| items.len() <= 20)
            .ok_or_else(|| "agent command list invalid".to_owned())?;
        commands.extend(page.iter().cloned());
        let next = response["next_cursor"]
            .as_str()
            .ok_or_else(|| "agent command cursor invalid".to_owned())?;
        if next.is_empty() {
            break;
        }
        if !operations::cursor_advances(&cursor, next) || page.is_empty() {
            return Err("agent command cursor did not advance".into());
        }
        cursor = next.into();
        query["cursor"] = next.into();
    }
    let mut executed = 0;
    for wire in &commands {
        let command = match operations::delivery(
            wire,
            &identity.id,
            &identity.uuid,
            &state.agent_installation_id,
            generation,
        ) {
            Ok(command) => command,
            Err(error) => {
                hbb_common::throttled_log!(
                    Duration::from_secs(60),
                    warn,
                    "agent invalid delivery refused: {error}"
                );
                continue;
            }
        };
        let digest = command.digest.as_str();
        if log.state(&command.id).is_none() && unix_timestamp() >= command.expires_at {
            hbb_common::throttled_log!(
                Duration::from_secs(60),
                warn,
                "agent expired unreceived command refused"
            );
            continue;
        }
        log.prepare(&command.id, digest, command.expires_at, unix_timestamp())?;
        if matches!(
            log.state(&command.id),
            Some(
                ExecutionState::Succeeded
                    | ExecutionState::Failed
                    | ExecutionState::Unsupported
                    | ExecutionState::Rejected
                    | ExecutionState::Expired
                    | ExecutionState::Cancelled
                    | ExecutionState::RolledBack
                    | ExecutionState::RollbackFailed
            )
        ) {
            continue;
        }
        if executed >= 2 {
            break;
        }
        executed += 1;
        if matches!(log.state(&command.id), Some(ExecutionState::Running | ExecutionState::ReconciliationRequired))
            && matches!(command_type(wire)?, "package.reinstall" | "package.update" | "agent.update") {
            let root = paths.agent.join("transactions").join(&command.id);
            match super::command_transaction::read_terminal(&root, wire) {
                Ok(Some(status)) => {
                    log.finish(&command.id, status, terminal_event_result(&command.id, status), unix_timestamp())?;
                }
                Ok(None) => {}
                Err(error) => {
                    if log.state(&command.id) == Some(ExecutionState::Running) {
                        log.finish(&command.id, "reconciliation_required", serde_json::json!({"error_code":"transaction_evidence_unconfirmed"}), unix_timestamp())?;
                    }
                    hbb_common::throttled_log!(Duration::from_secs(60), warn, "agent transaction evidence refused: {error}");
                }
            }
            upload_events(identity, base, &binding, &mut log)?;
            continue;
        }
        if log.state(&command.id) == Some(ExecutionState::ReconciliationRequired) {
            continue;
        }
        if log.state(&command.id) == Some(ExecutionState::Running) {
            // An inspection is repeatable; a mutating command must instead be reconciled.
            if !operations::may_repeat_running(wire) {
                log.finish(&command.id, "reconciliation_required", serde_json::json!({"error_code":"interrupted_mutation"}), unix_timestamp())?;
                upload_events(identity, base, &binding, &mut log)?;
                continue;
            }
        } else {
            let mut claim = binding.clone();
            claim["digest"] = digest.into();
            let path = format!("/rd/update/v1/agent/commands/{}/claim", command.id);
            let ack = request(identity, base, "POST", &path, &command.id, &claim)?;
            if ack["id"] != command.id
                || ack["digest"] != digest
                || ack["agent_installation_id"] != state.agent_installation_id
                || ack["enrollment_generation"] != generation
            {
                return Err("agent claim acknowledgement mismatch".into());
            }
            if ack["status"] != "accepted" && ack["status"] != "deferred" {
                if let Some(status @ ("cancelled" | "expired")) = ack["status"].as_str() {
                    log.abandon(&command.id, status, unix_timestamp())?;
                    continue;
                }
                return Err("agent claim is not executable".into());
            }
            log.receipt(&command.id)?;
            upload_events(identity, base, &binding, &mut log)?;
            if let Some(rejection) = operations::execution_rejection(wire) {
                log.reject(
                    &command.id,
                    rejection,
                    serde_json::json!({"error_code":rejection}),
                    unix_timestamp(),
                )?;
                continue;
            }
            if unix_timestamp() >= command.expires_at {
                if let Err(error) = log.start(&command.id, unix_timestamp()) {
                    if log.state(&command.id) == Some(ExecutionState::Expired) {
                        continue;
                    }
                    return Err(error.into());
                }
                return Err("expired command unexpectedly started".into());
            }
            if let Some(reason) = activity_deferral(command_type(wire)?, state, paths) {
                log.defer(&command.id, reason, unix_timestamp())?;
                upload_events(identity, base, &binding, &mut log)?;
                continue;
            }
            log.start(&command.id, unix_timestamp())?;
        }
        if matches!(command_type(wire)?, "package.reinstall" | "package.update" | "agent.update") {
            // Persist and acknowledge running before the helper may stop this
            // agent. The next process reconciles the durable helper result.
            upload_events(identity, base, &binding, &mut log)?;
            if let Err(error) = launch_package_command(identity, base, &binding, state, paths, wire) {
                let root = paths.agent.join("transactions").join(&command.id);
                let status = if root.exists() { "reconciliation_required" } else { "failed" };
                log.finish(&command.id, status, serde_json::json!({"error_code":"package_preparation_failed","message":error.to_string()}), unix_timestamp())?;
            }
            upload_events(identity, base, &binding, &mut log)?;
            break;
        }
        let outcome = if command_type(wire)? == "network.probe" {
            super::network_probe::fixed(Duration::from_secs(wire["timeout_seconds"].as_u64().unwrap_or(1)))
        } else {
            execute_command(&command_type(wire)?, state, paths)
        };
        match outcome {
            Ok(result)=>log.finish(&command.id,"succeeded",result,unix_timestamp())?,
            Err(error)=>log.finish(&command.id,"failed",serde_json::json!({"error_code":"service_query_failed","message":error}),unix_timestamp())?,
        }
    }
    upload_events(identity, base, &binding, &mut log)
}

#[cfg(test)]
pub(super) fn delivery_contract_poll(base: &str, state: &AgentState, paths: &AgentPaths) -> Result<(), PollError> {
    poll_with_identity(&identity::load_existing()?, base, state, paths)
}

fn command_type(wire: &serde_json::Value) -> Result<&str, PollError> {
    wire["type"].as_str().ok_or_else(|| PollError::from("command type missing"))
}

fn terminal_event_result(id: &str, status: &str) -> serde_json::Value {
    let mut result = serde_json::json!({"transaction_id":id});
    match status {
        "rolled_back" => result["error_code"] = "installation_failed_rolled_back".into(),
        "rollback_failed" => result["error_code"] = "installation_rollback_failed".into(),
        _ => {}
    }
    result
}

fn upload_events(
    identity: &identity::DeviceIdentity,
    base: &str,
    binding: &serde_json::Value,
    log: &mut CommandJournal,
) -> Result<(), PollError> {
    for event in log
        .pending_events()
        .iter()
        .take(8)
        .cloned()
        .collect::<Vec<_>>()
    {
        let mut payload = event.payload.clone();
        for key in [
            "client_id",
            "client_uuid",
            "agent_installation_id",
            "enrollment_generation",
        ] {
            payload[key] = binding[key].clone();
        }
        let command = payload["command_id"].as_str().unwrap_or("");
        let ack = request(
            identity,
            base,
            "POST",
            "/rd/update/v1/agent/events",
            command,
            &payload,
        )?;
        if ack["ok"] != true || ack["event_id"] != event.event_id {
            return Err("agent event acknowledgement mismatch".into());
        }
        log.acknowledge(&event.event_id, &event.digest)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delivery_rollback_events_have_fixed_codes_and_keep_local_error() {
        let directory=std::env::temp_dir().join(format!("agent-rollback-contract-{}",std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let error=b"private-key=secret-token; C:\\private\\user; installation validation failed";
        let receipt=directory.join("execution-result.json");
        std::fs::write(&receipt,error).unwrap();
        for (status,code) in [("rolled_back","installation_failed_rolled_back"),("rollback_failed","installation_rollback_failed")] {
            let payload=terminal_event_result(&"a".repeat(32),status);
            assert_eq!(payload["error_code"],code);
            assert_eq!(payload.as_object().unwrap().len(),2);
            assert!(!payload.to_string().contains("secret"));
            assert_eq!(std::fs::read(&receipt).unwrap(),error);
        }
        assert_eq!(terminal_event_result(&"a".repeat(32),"succeeded"),serde_json::json!({"transaction_id":"a".repeat(32)}));
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn local_command_native_child() {
        let Some(root) = std::env::var_os("RUSTDESK_HELPER_RUNTIME_FIXTURE") else { return; };
        assert!(matches!(std::env::var("COMPUTERNAME").unwrap().as_str(), "RDLAB-WIN10" | "RDLAB-WIN11"));
        let root = std::path::PathBuf::from(root);
        assert!(root.starts_with(r"C:\RustDeskAgentLab"));
        let public = STANDARD.decode(std::env::var("RUSTDESK_HELPER_FIXTURE_PUBLIC").unwrap()).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(120);
        while !root.join("handoff.json").exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(100));
        }
        super::super::helper_runtime::prepare_ready(&root).unwrap();
        super::super::helper_runtime::wait_for_release(&root).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        let mut owner = loop {
            match super::super::helper_runtime::take_over_native_fixture(&root, &public) {
                Ok(owner) => break owner,
                Err(error) if error.contains("guard busy") && std::time::Instant::now() < deadline => std::thread::sleep(Duration::from_millis(100)),
                Err(error) => panic!("local command helper: {error}"),
            }
        };
        let mut backend = super::super::native_install::NativeInstaller::open(&root).unwrap();
        let rollback = root.join("fixture-validation-failure").is_file();
        let result = if rollback {
            struct FailValidation(super::super::native_install::NativeInstaller);
            impl super::super::install_execution::Backend for FailValidation {
                fn stop_services(&mut self) -> Result<(), String> { self.0.stop_services() }
                fn apply_package(&mut self) -> Result<super::super::install_execution::InstallOutcome, String> { self.0.apply_package() }
                fn validate_target(&mut self) -> Result<super::super::install_execution::Validation, String> {
                    let mut proof = self.0.validate_target()?;
                    assert!(proof.target_matches && proof.identity_unchanged && proof.client_ready && proof.agent_ready,
                        "failure injection requires a successfully installed and healthy target");
                    proof.agent_ready = false;
                    Ok(proof)
                }
                fn rollback_original(&mut self) -> Result<super::super::install_execution::Validation, String> { self.0.rollback_original() }
            }
            owner.execute(&mut FailValidation(backend)).unwrap()
        } else { owner.execute(&mut backend).unwrap() };
        let expected = if rollback { super::super::transaction::Phase::RolledBack } else { super::super::transaction::Phase::Committed };
        assert_eq!(result.phase, expected, "{:?}", result.error);
        if rollback { assert!(result.error.as_deref().unwrap().contains("validation failed")); }
        let receipt = std::fs::read(root.join("execution-result.json")).unwrap();
        if matches!(
            std::env::var("RUSTDESK_P09_RUN").ok().as_deref(),
            Some("p09-v201" | "p09-v203" | "p09-v209")
        ) {
            use sha2::{Digest, Sha256};
            let plan = super::super::install_plan::InstallPlan::decode(
                &std::fs::read(root.join("prepared.json")).unwrap(),
            )
            .unwrap();
            let task = super::super::resume_task::read_back(&plan.transaction_id).unwrap();
            super::super::resume_task::verify(
                &task,
                &root.join(&plan.helper.relative_path),
                &plan.transaction_id,
            )
            .unwrap();
            let marker =
                std::fs::read(r"C:\ProgramData\RustDesk\ManagementAgent\state\maintenance.json")
                    .unwrap();
            let marker_released = std::env::var("RUSTDESK_P09_RUN").unwrap() == "p09-v209";
            if marker_released {
                let binding = super::super::handoff::Handoff::decode(
                    &std::fs::read(root.join("handoff.json")).unwrap(),
                )
                .unwrap();
                super::super::maintenance::release_owned(
                    std::path::Path::new(r"C:\ProgramData\RustDesk\ManagementAgent\state"),
                    &plan.installation_id,
                    &plan.transaction_id,
                    &binding.plan_sha256,
                    &binding.nonce,
                )
                .unwrap();
                assert!(!std::path::Path::new(
                    r"C:\ProgramData\RustDesk\ManagementAgent\state\maintenance.json"
                )
                .exists());
                assert_eq!(
                    super::super::resume_task::read_back(&plan.transaction_id).unwrap(),
                    task
                );
            }
            let digest = |bytes: &[u8]| hex::encode(Sha256::digest(bytes));
            let barrier = serde_json::json!({"schema":1,"run_id":std::env::var("RUSTDESK_P09_RUN").unwrap(),"root":root,"transaction_id":plan.transaction_id,
                "phase":result.phase,"error":result.error,"public":STANDARD.encode(&public),
                "helper_sha256":digest(&std::fs::read(root.join(&plan.helper.relative_path)).unwrap()),
                "receipt_sha256":digest(&receipt),"transaction_sha256":digest(&std::fs::read(root.join("transaction.json")).unwrap()),
                "maintenance_sha256":digest(&marker),"marker_released":marker_released,"task_sha256":digest(task.as_bytes()),
                "helper_owner":super::super::handoff::ObservedProcess::open(std::process::id()).unwrap().owner(),
                "barrier_utc":unix_timestamp()});
            super::super::state_store::publish(
                &root.join("p09-barrier.json"),
                &serde_json::to_vec_pretty(&barrier).unwrap(),
            )
            .unwrap();
            let deadline = std::time::Instant::now() + Duration::from_secs(900);
            while std::time::Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(250));
            }
            panic!("P09 poweroff did not arrive; preserve terminal scene");
        }
        struct NoTerminalMutation;
        impl super::super::install_execution::Backend for NoTerminalMutation {
            fn stop_services(&mut self) -> Result<(), String> { panic!("terminal resume stopped services") }
            fn apply_package(&mut self) -> Result<super::super::install_execution::InstallOutcome, String> { panic!("terminal resume reinstalled") }
            fn validate_target(&mut self) -> Result<super::super::install_execution::Validation, String> { panic!("terminal resume revalidated") }
            fn rollback_original(&mut self) -> Result<super::super::install_execution::Validation, String> { panic!("terminal resume repeated rollback") }
        }
        let resumed = owner.resume(&mut NoTerminalMutation).unwrap();
        assert_eq!(resumed.phase, result.phase);
        assert_eq!(resumed.error, result.error);
        assert_eq!(std::fs::read(root.join("execution-result.json")).unwrap(), receipt);
        owner.finalize().unwrap();
        std::fs::write(root.join("fixture-helper-finished"), serde_json::to_vec(&result.phase).unwrap()).unwrap();
    }

    #[test]
    #[ignore = "Requires SYSTEM lab VM, healthy v128 MSI baseline and local PHP fixture"]
    fn local_api_command_downloads_installs_and_reports_exactly_once() {
        local_api_command_scenario(false, false, false);
    }

    #[test]
    #[ignore = "Requires SYSTEM lab VM, healthy v128 MSI baseline and local PHP fixture"]
    fn local_api_command_validation_failure_rolls_back_and_reports_exactly_once() {
        local_api_command_scenario(true, false, false);
    }

    #[test]
    #[ignore = "Requires SYSTEM lab VM, healthy v128 MSI baseline and local PHP fixture"]
    fn local_api_command_new_process_reports_after_launch_process_exits() {
        local_api_command_scenario(false, true, false);
    }

    #[test]
    #[ignore = "Requires SYSTEM lab VM with v139 baseline and locally built 2026100503 MSI"]
    fn local_api_command_upgrades_build_and_keeps_exact_old_rollback() {
        local_api_command_scenario(false, true, true);
    }

    #[test]
    #[ignore = "Requires SYSTEM lab VM with v139 baseline and locally built 2026100503 MSI"]
    fn local_api_cross_build_failure_restores_old_build_and_bootstrap() {
        local_api_command_scenario(true, false, true);
    }

    #[test]
    #[ignore = "Requires SYSTEM lab VM with v139 baseline and real cross-build MSI mirror sources"]
    fn local_api_mirrors_upgrade_real_msi_and_report_once() {
        local_api_command_scenario_with_mirrors(false, true, true, true);
    }

    #[test]
    #[ignore = "Requires SYSTEM lab VM with v139 baseline and real cross-build MSI mirror sources"]
    fn local_api_mirrors_validation_failure_restores_real_msi_and_reports_once() {
        local_api_command_scenario_with_mirrors(true, false, true, true);
    }

    #[test]
    #[ignore = "Win10 SYSTEM P09 automatic boot bridge only; never a production CLI test"]
    fn p09_boot_bridge_resumes_terminal_without_backend() {
        use sha2::{Digest, Sha256};
        use std::path::{Path, PathBuf};
        assert_eq!(std::env::var("COMPUTERNAME").unwrap(), "RDLAB-WIN10");
        let root = PathBuf::from(std::env::var_os("RUSTDESK_P09_ROOT").unwrap());
        let lab = root.ancestors().nth(4).unwrap();
        assert_eq!(lab.parent(), Some(Path::new(r"C:\RustDeskAgentLab")));
        assert!(lab
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("local-command-"));
        workspace::verify(&root).unwrap();
        workspace::verify_file(&root.join("p09-barrier.json")).unwrap();
        let barrier: serde_json::Value =
            serde_json::from_slice(&std::fs::read(root.join("p09-barrier.json")).unwrap()).unwrap();
        assert!(matches!(
            barrier["run_id"].as_str(),
            Some("p09-v201" | "p09-v203" | "p09-v209")
        ));
        assert_eq!(barrier["root"], root.to_str().unwrap());
        let hash = |path: &Path| hex::encode(Sha256::digest(std::fs::read(path).unwrap()));
        let plan = super::super::install_plan::InstallPlan::decode(
            &std::fs::read(root.join("prepared.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(barrier["transaction_id"], plan.transaction_id);
        assert_eq!(
            std::env::current_exe().unwrap(),
            root.join(&plan.helper.relative_path)
        );
        assert_eq!(
            hash(&std::env::current_exe().unwrap()),
            barrier["helper_sha256"]
        );
        for (file, field) in [
            ("execution-result.json", "receipt_sha256"),
            ("transaction.json", "transaction_sha256"),
        ] {
            assert_eq!(hash(&root.join(file)), barrier[field]);
        }
        let task = super::super::resume_task::read_back(&plan.transaction_id).unwrap();
        assert_eq!(
            hex::encode(Sha256::digest(task.as_bytes())),
            barrier["task_sha256"]
        );
        let live = Path::new(r"C:\ProgramData\RustDesk\ManagementAgent\state");
        let intent = hbb_common::config::Config2::file();
        let handoff = super::super::handoff::Handoff::decode(
            &std::fs::read(root.join("handoff.json")).unwrap(),
        )
        .unwrap();
        let missing_marker = barrier["run_id"] == "p09-v209";
        if missing_marker {
            assert_eq!(barrier["marker_released"], true);
            assert!(!live.join("maintenance.json").exists());
            super::super::maintenance::verify_terminal_cleanup(
                live,
                &intent,
                &plan.installation_id,
                &plan.transaction_id,
                &handoff.plan_sha256,
                &handoff.nonce,
            )
            .unwrap();
        } else {
            super::super::maintenance::verify(
                live,
                &intent,
                &plan.installation_id,
                &plan.transaction_id,
                &handoff.plan_sha256,
                &handoff.nonce,
            )
            .unwrap();
            assert_eq!(
                hash(&live.join("maintenance.json")),
                barrier["maintenance_sha256"]
            );
        }
        // The existing fixture resume API uses local state paths. Copy only after verifying the live gates.
        workspace::create(&root.join("state")).unwrap();
        std::fs::copy(live.join("agent.json"), root.join("state/agent.json")).unwrap();
        if !missing_marker {
            std::fs::copy(
                live.join("maintenance.json"),
                root.join("state/maintenance.json"),
            )
            .unwrap();
        }
        std::fs::copy(&intent, root.join("intent.toml")).unwrap();
        let public = STANDARD
            .decode(barrier["public"].as_str().unwrap())
            .unwrap();
        let mut owner =
            super::super::helper_runtime::resume_with_fixture_key(&root, "fixture-key", &public)
                .unwrap();
        struct NoBackend(u32);
        impl super::super::install_execution::Backend for NoBackend {
            fn stop_services(&mut self) -> Result<(), String> {
                self.0 += 1;
                Err("P09 unexpected stop".into())
            }
            fn apply_package(
                &mut self,
            ) -> Result<super::super::install_execution::InstallOutcome, String> {
                self.0 += 1;
                Err("P09 unexpected install".into())
            }
            fn validate_target(
                &mut self,
            ) -> Result<super::super::install_execution::Validation, String> {
                self.0 += 1;
                Err("P09 unexpected validation".into())
            }
            fn rollback_original(
                &mut self,
            ) -> Result<super::super::install_execution::Validation, String> {
                self.0 += 1;
                Err("P09 unexpected rollback".into())
            }
        }
        let mut backend = NoBackend(0);
        let result = owner.resume(&mut backend).unwrap();
        assert_eq!(backend.0, 0);
        assert_eq!(
            serde_json::to_value(result.phase).unwrap(),
            barrier["phase"]
        );
        assert_eq!(
            serde_json::to_value(&result.error).unwrap(),
            barrier["error"]
        );
        assert_eq!(
            hash(&root.join("execution-result.json")),
            barrier["receipt_sha256"]
        );
        assert_eq!(
            hash(&root.join("transaction.json")),
            barrier["transaction_sha256"]
        );
        if missing_marker {
            assert!(!live.join("maintenance.json").exists());
            assert!(!root.join("state/maintenance.json").exists());
            super::super::maintenance::verify_terminal_cleanup(
                live,
                &intent,
                &plan.installation_id,
                &plan.transaction_id,
                &handoff.plan_sha256,
                &handoff.nonce,
            )
            .unwrap();
        } else {
            super::super::maintenance::verify(
                live,
                &intent,
                &plan.installation_id,
                &plan.transaction_id,
                &handoff.plan_sha256,
                &handoff.nonce,
            )
            .unwrap();
            super::super::maintenance::release_owned(
                live,
                &plan.installation_id,
                &plan.transaction_id,
                &handoff.plan_sha256,
                &handoff.nonce,
            )
            .unwrap();
        }
        owner.finalize().unwrap();
        assert!(!live.join("maintenance.json").exists());
        assert!(!root.join("state/maintenance.json").exists());
        assert!(super::super::resume_task::read_back(&plan.transaction_id).is_err());
        super::super::state_store::publish(&root.join("p09-resumed.json"),&serde_json::to_vec_pretty(&serde_json::json!({
            "scope":"test-only automatic boot bridge; production CLI unverified","backend_calls":backend.0,"missing_marker_not_recreated":missing_marker,
            "phase":result.phase,"error":result.error,"receipt_sha256":hash(&root.join("execution-result.json")),
            "transaction_sha256":hash(&root.join("transaction.json")),"marker_removed":true,"original_task_removed":true,
            "resume_owner":super::super::handoff::ObservedProcess::open(std::process::id()).unwrap().owner()
        })).unwrap()).unwrap();
    }

    #[test]
    fn local_command_poll_child() {
        let Some(root) = std::env::var_os("RUSTDESK_LOCAL_POLL_FIXTURE") else { return; };
        assert!(matches!(std::env::var("COMPUTERNAME").unwrap().as_str(), "RDLAB-WIN10" | "RDLAB-WIN11"));
        let root = std::path::PathBuf::from(root);
        assert_eq!(root.parent(), Some(std::path::Path::new(r"C:\RustDeskAgentLab")));
        assert!(root.file_name().unwrap().to_string_lossy().starts_with("local-command-"));
        workspace::verify(&root).unwrap();
        let input_path = root.join("poll-fixture.json");
        workspace::verify_file(&input_path).unwrap();
        let bytes = std::fs::read(&input_path).unwrap();
        assert!(bytes.len() <= 65536);
        let input: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let api: std::net::SocketAddr = input["api"].as_str().unwrap().parse().unwrap();
        let download: std::net::SocketAddr = input["download"].as_str().unwrap().parse().unwrap();
        assert!(api.ip().is_loopback() && download.ip().is_loopback());
        hbb_common::sodiumoxide::init().unwrap();
        let identity = identity::DeviceIdentity {
            id: "test-device".into(), uuid: "test-uuid".into(),
            public: sign::PublicKey::from_slice(&STANDARD.decode(input["public"].as_str().unwrap()).unwrap()).unwrap(),
            secret: sign::SecretKey::from_slice(&STANDARD.decode(input["secret"].as_str().unwrap()).unwrap()).unwrap(),
        };
        let actual: AgentState = serde_json::from_value(input["state"].clone()).unwrap();
        let paths = AgentPaths::from_program_data(&root, super::super::Edition::Standard);
        let phase = std::env::var("RUSTDESK_LOCAL_POLL_PHASE").unwrap();
        assert!(matches!(phase.as_str(), "launch" | "report" | "duplicate"));
        if phase == "launch" {
            let release = STANDARD.decode(input["release_public"].as_str().unwrap()).unwrap();
            LOCAL_PACKAGE_FIXTURE.with(|value| *value.borrow_mut() = Some((release, download)));
        }
        let result = poll_with_identity(&identity, &format!("http://{api}"), &actual, &paths);
        LOCAL_PACKAGE_FIXTURE.with(|value| *value.borrow_mut() = None);
        result.unwrap();
        let command = input["command"].as_str().unwrap();
        let journal = CommandJournal::open(&paths.state.join("commands-1.json")).unwrap();
        assert_eq!(journal.state(command), Some(if phase == "launch" { ExecutionState::Running } else { ExecutionState::Succeeded }));
        assert!(journal.pending_events().is_empty());
        let process = super::super::handoff::ObservedProcess::open(std::process::id()).unwrap();
        std::fs::write(root.join(format!("poll-{phase}-owner.json")), serde_json::to_vec(process.owner()).unwrap()).unwrap();
    }

    #[test]
    #[ignore = "Win10 SYSTEM protected original device identity, report-only post-powercut fixture"]
    fn powercut_original_identity_report_child() {
        use sha2::{Digest, Sha256};
        use std::path::{Path, PathBuf};
        assert_eq!(std::env::var("COMPUTERNAME").unwrap(), "RDLAB-WIN10");
        let root = PathBuf::from(std::env::var_os("RUSTDESK_REPORT_ROOT").unwrap());
        assert_eq!(root.parent(), Some(Path::new(r"C:\RustDeskAgentLab")));
        assert!(root
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("local-command-"));
        workspace::verify(&root).unwrap();
        let secret_path = root.join("report-identity.json");
        workspace::verify_file(&secret_path).unwrap();
        let input: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&secret_path).unwrap()).unwrap();
        assert!(matches!(
            input["run_id"].as_str(),
            Some("p09-v203" | "p09-v209")
        ));
        assert_eq!(input["root"], root.to_str().unwrap());
        assert_eq!(
            input["db"],
            root.join("agent-status-test.db").to_str().unwrap()
        );
        let public = sign::PublicKey::from_slice(
            &STANDARD.decode(input["public"].as_str().unwrap()).unwrap(),
        )
        .unwrap();
        let secret = sign::SecretKey::from_slice(
            &STANDARD.decode(input["secret"].as_str().unwrap()).unwrap(),
        )
        .unwrap();
        let proof = sign::sign_detached(
            input["command_digest"].as_str().unwrap().as_bytes(),
            &secret,
        );
        assert!(sign::verify_detached(
            &proof,
            input["command_digest"].as_str().unwrap().as_bytes(),
            &public
        ));
        let identity = identity::DeviceIdentity {
            id: input["client_id"].as_str().unwrap().into(),
            uuid: input["client_uuid"].as_str().unwrap().into(),
            public,
            secret,
        };
        let state: AgentState = serde_json::from_value(input["state"].clone()).unwrap();
        assert_eq!(input["installation_id"], state.agent_installation_id);
        assert_eq!(input["generation"], 1);
        let paths = AgentPaths::from_program_data(&root, super::super::Edition::Standard);
        let command = input["command"].as_str().unwrap();
        let tx = paths.agent.join("transactions").join(command);
        let binding: serde_json::Value =
            serde_json::from_slice(&std::fs::read(tx.join("command-binding.json")).unwrap())
                .unwrap();
        assert_eq!(binding["command_digest"], input["command_digest"]);
        let receipt = std::fs::read(tx.join("execution-result.json")).unwrap();
        let transaction = std::fs::read(tx.join("transaction.json")).unwrap();
        let terminal: serde_json::Value = serde_json::from_slice(&receipt).unwrap();
        assert_eq!(terminal["phase"], "rolled_back");
        let journal = CommandJournal::open(&paths.state.join("commands-1.json")).unwrap();
        assert!(matches!(
            journal.state(command),
            Some(ExecutionState::Running | ExecutionState::RolledBack)
        ));
        let address: std::net::SocketAddr = std::env::var("RUSTDESK_REPORT_API")
            .unwrap()
            .parse()
            .unwrap();
        assert!(address.ip().is_loopback() && address.port() > 0);
        let phase = std::env::var("RUSTDESK_REPORT_PHASE").unwrap();
        assert!(matches!(phase.as_str(), "lost-ack" | "retry" | "duplicate"));
        let result = poll_with_identity(&identity, &format!("http://{address}"), &state, &paths);
        let journal = CommandJournal::open(&paths.state.join("commands-1.json")).unwrap();
        assert_eq!(journal.state(command), Some(ExecutionState::RolledBack));
        if phase == "lost-ack" {
            assert!(
                result.is_err(),
                "transport fixture must actually lose the accepted response"
            );
            assert_eq!(journal.pending_events().len(), 1);
        } else {
            result.unwrap();
            assert!(journal.pending_events().is_empty());
        }
        assert_eq!(
            std::fs::read(tx.join("execution-result.json")).unwrap(),
            receipt
        );
        assert_eq!(
            std::fs::read(tx.join("transaction.json")).unwrap(),
            transaction
        );
        let events = journal
            .pending_events()
            .iter()
            .map(|e| serde_json::json!({"event_id":e.event_id,"digest":e.digest}))
            .collect::<Vec<_>>();
        let evidence = serde_json::json!({"phase":phase,"owner":super::super::handoff::ObservedProcess::open(std::process::id()).unwrap().owner(),
            "pending_events":events,"receipt_sha256":hex::encode(Sha256::digest(&receipt)),"transaction_sha256":hex::encode(Sha256::digest(&transaction))});
        super::super::state_store::publish(
            &root.join(format!("report-{phase}.json")),
            &serde_json::to_vec_pretty(&evidence).unwrap(),
        )
        .unwrap();
    }

    fn local_api_command_scenario(rollback: bool, restart: bool, cross_build: bool) {
        local_api_command_scenario_with_mirrors(rollback, restart, cross_build, false);
    }

    fn local_api_command_scenario_with_mirrors(rollback: bool, restart: bool, cross_build: bool, mirrors: bool) {
        use std::{io::{Read, Write}, net::TcpListener, path::{Path, PathBuf}, process::{Command, Stdio},
            sync::{Arc, atomic::{AtomicBool, Ordering}}, time::Instant};
        use sha2::{Digest, Sha256};
        assert!(matches!(std::env::var("COMPUTERNAME").unwrap().as_str(), "RDLAB-WIN10" | "RDLAB-WIN11"));
        assert_eq!(std::env::var("RUSTDESK_LOCAL_COMMAND_LAB").unwrap(), "v142");
        let installed_paths = AgentPaths::from_program_data(r"C:\ProgramData", super::super::Edition::Standard);
        let actual: AgentState = serde_json::from_slice(&std::fs::read(installed_paths.state.join("agent.json")).unwrap()).unwrap();
        let before_agent = hex::encode(Sha256::digest(std::fs::read(installed_paths.agent.join("rustdesk-management-agent.exe")).unwrap()));
        let before_dll = hex::encode(Sha256::digest(std::fs::read(r"C:\Program Files\RustDesk\librustdesk.dll").unwrap()));
        let before_bootstrap = std::fs::read(installed_paths.root.join("RecoveryBootstrap/bootstrap.json")).unwrap();
        assert!(actual.management_agent_enabled && !actual.disabled);
        assert_eq!(actual.package_kind, super::super::PackageKind::Msi);
        assert_eq!(activity_deferral("package.reinstall", &actual, &installed_paths), None);
        let package = Path::new(if cross_build { r"C:\RustDeskAgentLab\agent-cross-v153.msi" }
            else { r"C:\RustDeskAgentLab\agent-msi-bootstrap-v128.msi" });
        let package_bytes = std::fs::read(package).unwrap();
        let package_hash = hex::encode(Sha256::digest(&package_bytes));
        assert_eq!(package_hash, if cross_build { "af62de5a7edf73b8f8a515513b7680f8e48ed1c03bfe73a653efbc9f78b97782" }
            else { "682a6f4da21ba2a01863496dceb48e9b6ebcf853fe55d383f6702ae0234dfd90" });
        let target_build = if cross_build { 2026100503 } else { 2026100502 };
        let target_name = package.file_name().unwrap().to_str().unwrap().to_owned();
        let rollback_bytes = if cross_build { std::fs::read(r"C:\RustDeskAgentLab\agent-auto-bootstrap-v139.msi").unwrap() }
            else { package_bytes.clone() };
        let rollback_name = if cross_build { "agent-auto-bootstrap-v139.msi" } else { "agent-msi-bootstrap-v128.msi" };
        if cross_build {
            assert_eq!(hex::encode(Sha256::digest(&rollback_bytes)),"5c4e4508ee116d64d4314d289cb7b0ad0d6d1e6c213d3607b3722a697ac05979");
        }
        let root = PathBuf::from(r"C:\RustDeskAgentLab").join(format!("local-command-{}", std::process::id()));
        workspace::create(&root).unwrap();
        println!("Local command evidence: {}", root.display());
        let source_api = PathBuf::from(std::env::var_os("AGENT_TEST_API_ROOT").unwrap());
        let api = root.join("api");
        std::fs::create_dir(&api).unwrap();
        std::fs::create_dir(api.join("sqlite")).unwrap();
        std::fs::create_dir(api.join("tests")).unwrap();
        for entry in std::fs::read_dir(source_api.join("sqlite")).unwrap() {
            let entry = entry.unwrap();
            if entry.file_type().unwrap().is_file() {
                std::fs::copy(entry.path(), api.join("sqlite").join(entry.file_name())).unwrap();
            }
        }
        std::fs::copy(source_api.join("tests/agent_status_http_router.php"),api.join("tests/agent_status_http_router.php")).unwrap();
        // Only the disposable API copy recognizes this ephemeral release key ID.
        let operations_file = api.join("sqlite/agent_operations.php");
        let source = std::fs::read_to_string(&operations_file).unwrap();
        assert!(source.contains("'yan-release-2026'"));
        std::fs::write(&operations_file, source.replace("'yan-release-2026'", "'fixture-key'")).unwrap();
        hbb_common::sodiumoxide::init().unwrap();
        let (public, secret) = sign::gen_keypair();
        let identity = identity::DeviceIdentity {id:"test-device".into(),uuid:"test-uuid".into(),public,secret};
        let (release_public, release_secret) = sign::gen_keypair();
        let key = "windows-x86_64-msi-standard";
        let mut manifest = hbb_common::UpdateManifest {schema:2,product:"rustdesk-yan".into(),edition:"multi".into(),channel:"stable".into(),catalog_revision:target_build,..Default::default()};
        let mut target = hbb_common::UpdateTarget {version:"1.5.3".into(),build_seq:target_build,source_commit:"fixture".into(),
            primary:format!("https://download.yan.life/rustdesk/stable/1.5.3/{target_name}"),
            size:package_bytes.len() as u64,sha256:package_hash,signature_key_id:"fixture-key".into(),..Default::default()};
        if mirrors {
            assert!(cross_build);
            assert!(target.size > 60 * 1024 * 1024);
            target.mirrors =
                vec!["https://download.yan.life/rustdesk/stable/mirror/target-mirror.msi".into()];
        }
        target.signature = STANDARD.encode(sign::sign_detached(&base::update::update_signature_payload(&manifest,key,&target).unwrap(), &release_secret));
        manifest.targets.insert(key.into(),target);
        let mut rollback_manifest = manifest.clone();
        rollback_manifest.catalog_revision = 2026100502;
        let mut original = manifest.targets[key].clone();
        original.build_seq = 2026100502;
        original.primary = format!("https://download.yan.life/rustdesk/stable/1.5.3/{rollback_name}");
        original.size = rollback_bytes.len() as u64;
        original.sha256 = hex::encode(Sha256::digest(&rollback_bytes));
        if mirrors {
            assert!(original.size > 60 * 1024 * 1024);
            original.mirrors =
                vec!["https://download.yan.life/rustdesk/stable/mirror/rollback-mirror.msi".into()];
        }
        original.signature = STANDARD.encode(sign::sign_detached(&base::update::update_signature_payload(&rollback_manifest,key,&original).unwrap(),&release_secret));
        rollback_manifest.targets.insert(key.into(),original);
        if mirrors {
            std::fs::write(
                root.join("mirror-target-manifest.json"),
                serde_json::to_vec_pretty(&manifest).unwrap(),
            )
            .unwrap();
            std::fs::write(
                root.join("mirror-rollback-manifest.json"),
                serde_json::to_vec_pretty(&rollback_manifest).unwrap(),
            )
            .unwrap();
        }
        assert!(package_gate::verify(&manifest,key,2026100502).is_err());
        let db = root.join("agent-status-test.db");
        let php = std::env::var("AGENT_TEST_PHP").unwrap();
        let ini = std::env::var("AGENT_TEST_PHP_INI").unwrap();
        let php_command = || {let mut cmd=Command::new(&php);cmd.args(["-c",&ini]);cmd};
        let seed = r#"require getenv('AGENT_FIXTURE_ROOT').'/sqlite/lib.php';require getenv('AGENT_FIXTURE_ROOT').'/sqlite/agent_status.php';require getenv('AGENT_FIXTURE_ROOT').'/sqlite/agent_operations.php';function fail(int $status,string $message):never{throw new RuntimeException($status.':'.$message);}$db=new PDO('sqlite:'.getenv('AGENT_HTTP_FIXTURE_DB'));configure_pdo($db);ensure_schema($db);$now=time();$pk=getenv('AGENT_FIXTURE_PK');$installation=getenv('AGENT_FIXTURE_INSTALLATION');db_insert_ignore($db,'device_reports',['id'=>'test-device','uuid'=>'test-uuid','payload'=>'{}','last_seen'=>123,'last_heartbeat'=>123,'heartbeat_payload'=>'{}']);db_insert_ignore($db,'device_update_keys',['device_id'=>'test-device','uuid'=>'test-uuid','public_key'=>$pk,'first_seen_at'=>$now,'last_seen_at'=>$now]);$actor=['id'=>1,'address_book_scope'=>'all'];agent_binding_authorize($db,$actor,'test-device','test-uuid',$installation,true,$now);$payload=['schema'=>1,'client_id'=>'test-device','client_uuid'=>'test-uuid','agent_installation_id'=>$installation,'boot_id'=>str_repeat('b',32),'session_generation'=>1,'sequence'=>1,'agent'=>['version'=>'1.5.3','build_seq'=>2026100502],'capabilities'=>['agent.status@1','package.reinstall@1'],'package_identity'=>['edition'=>'standard','arch'=>'x86_64','kind'=>'msi'],'service'=>['state'=>'running'],'application'=>['state'=>'ready'],'recovery'=>['effective_enabled'=>true]];agent_status_store($db,$payload,$now);$manifest=json_decode(getenv('AGENT_REFERENCE_MANIFEST'),true,64,JSON_THROW_ON_ERROR);db_insert_ignore($db,'update_releases',['version'=>'1.5.3','build_seq'=>2026100502,'channel'=>'stable','manifest'=>agent_ops_json($manifest),'active'=>1,'published_at'=>$now]);echo agent_ops_json(agent_command_create($db,$actor,'test-device','test-uuid',['type'=>'package.reinstall','type_version'=>1,'idempotency_key'=>'local-http-install','args'=>['package_id'=>agent_package_reference($manifest,'windows-x86_64-msi-standard'),'target_key'=>'windows-x86_64-msi-standard','target_build_seq'=>2026100502,'allow_downgrade'=>false],'expires_in'=>3600],$now));"#;
        let seed = if cross_build {
            seed.replace("'package.reinstall'","'package.update'")
                .replace("'package.reinstall@1'","'package.update@1'")
                .replace("'target_build_seq'=>2026100502","'target_build_seq'=>2026100503")
                .replace("['version'=>'1.5.3','build_seq'=>2026100502,'channel'=>'stable','manifest'=>agent_ops_json($manifest)",
                    "['version'=>'1.5.3','build_seq'=>2026100503,'channel'=>'stable','manifest'=>agent_ops_json($manifest)")
                .replace("$manifest=json_decode(getenv('AGENT_REFERENCE_MANIFEST'),true,64,JSON_THROW_ON_ERROR);",
                    "$rollback=json_decode(getenv('AGENT_ROLLBACK_MANIFEST'),true,64,JSON_THROW_ON_ERROR);db_insert_ignore($db,'update_releases',['version'=>'1.5.3','build_seq'=>2026100502,'channel'=>'stable','manifest'=>agent_ops_json($rollback),'active'=>1,'published_at'=>$now]);$manifest=json_decode(getenv('AGENT_REFERENCE_MANIFEST'),true,64,JSON_THROW_ON_ERROR);")
        } else { seed.to_owned() };
        let seeded = php_command().args(["-r",&seed]).env("AGENT_FIXTURE_ROOT",&api).env("AGENT_HTTP_FIXTURE_DB",&db)
            .env("AGENT_FIXTURE_PK",STANDARD.encode(identity.public.as_ref())).env("AGENT_FIXTURE_INSTALLATION",&actual.agent_installation_id)
            .env("AGENT_ROLLBACK_MANIFEST",serde_json::to_string(&rollback_manifest).unwrap())
            .env("AGENT_REFERENCE_MANIFEST",serde_json::to_string(&manifest).unwrap()).output().unwrap();
        assert!(seeded.status.success(),"{}",String::from_utf8_lossy(&seeded.stderr));
        let wire:serde_json::Value=serde_json::from_slice(&seeded.stdout).unwrap();
        let command=wire["id"].as_str().unwrap().to_owned();
        if matches!(std::env::var("RUSTDESK_P09_RUN").ok().as_deref(),Some("p09-v203"|"p09-v209")) {
            use std::io::Write;
            let saved = serde_json::json!({"schema":1,"run_id":std::env::var("RUSTDESK_P09_RUN").unwrap(),"root":root,"db":db,
                "client_id":identity.id,"client_uuid":identity.uuid,"installation_id":actual.agent_installation_id,
                "generation":1,"command":command,"command_digest":wire["digest"],"wire":wire,"state":actual,
                "public":STANDARD.encode(identity.public.as_ref()),"secret":STANDARD.encode(identity.secret.as_ref())});
            let path = root.join("report-identity.json");
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
                .unwrap();
            file.write_all(&serde_json::to_vec(&saved).unwrap())
                .unwrap();
            file.sync_all().unwrap();
            drop(file);
            workspace::verify_file(&path).unwrap();
            let readback: serde_json::Value =
                serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
            let restored = sign::SecretKey::from_slice(
                &STANDARD
                    .decode(readback["secret"].as_str().unwrap())
                    .unwrap(),
            )
            .unwrap();
            let challenge = wire["digest"].as_str().unwrap().as_bytes();
            let signature = sign::sign_detached(challenge, &restored);
            assert!(sign::verify_detached(
                &signature,
                challenge,
                &identity.public
            ));
            let proof=php_command().args(["-r",r#"$db=new PDO('sqlite:'.getenv('AGENT_HTTP_FIXTURE_DB'));$db->setAttribute(PDO::ATTR_ERRMODE,PDO::ERRMODE_EXCEPTION);$key=$db->query('SELECT public_key FROM device_update_keys')->fetchColumn();$wire=json_decode($db->query('SELECT envelope FROM device_agent_commands')->fetchColumn(),true);if(!sodium_crypto_sign_verify_detached(base64_decode(getenv('PROOF')),getenv('DIGEST'),base64_decode($key)))exit(7);echo json_encode(['verified'=>true,'command'=>$wire['id']]);"#])
                .env("AGENT_HTTP_FIXTURE_DB",&db).env("PROOF",STANDARD.encode(signature)).env("DIGEST",wire["digest"].as_str().unwrap()).output().unwrap();
            assert!(
                proof.status.success(),
                "persisted identity did not verify against registered DB key"
            );
            super::super::state_store::publish(
                &root.join("identity-persistence-proof.json"),
                &proof.stdout,
            )
            .unwrap();
        }
        let listener=TcpListener::bind("127.0.0.1:0").unwrap();
        let download_address=listener.local_addr().unwrap();listener.set_nonblocking(true).unwrap();
        let stop=Arc::new(AtomicBool::new(false));let stop_server=Arc::clone(&stop);
        let request_log = root.join("mirror-http-requests.json");
        let server_target_name = target_name.clone();
        let downloader=std::thread::spawn(move || {
            let mut requests=0;
            let mut evidence=Vec::new();
            while !stop_server.load(Ordering::SeqCst) && requests < if mirrors { 4 } else { 2 } {
                let mut stream=match listener.accept(){Ok((stream,_))=>stream,Err(error) if error.kind()==std::io::ErrorKind::WouldBlock=>{std::thread::sleep(Duration::from_millis(20));continue},Err(error)=>panic!("download accept: {error}")};
                stream.set_nonblocking(false).unwrap();stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();stream.set_write_timeout(Some(Duration::from_secs(60))).unwrap();
                let mut bytes=Vec::new();while !bytes.ends_with(b"\r\n\r\n"){let mut byte=[0];stream.read_exact(&mut byte).unwrap();bytes.push(byte[0]);assert!(bytes.len()<8192);}
                let name = if mirrors {
                    match requests { 0 => server_target_name.as_str(), 1 => "target-mirror.msi", 2 => rollback_name, _ => "rollback-mirror.msi" }
                } else if requests == 0 { server_target_name.as_str() } else { rollback_name };
                assert!(String::from_utf8_lossy(&bytes).starts_with(&format!("GET /{name} HTTP/1.1\r\n")));
                if mirrors && requests % 2 == 0 {
                    evidence.push(serde_json::json!({"path":format!("/{name}"),"status":503,"bytes":0}));
                    std::fs::write(&request_log,serde_json::to_vec_pretty(&evidence).unwrap()).unwrap();
                    write!(stream,"HTTP/1.1 503 Fixture primary unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
                    requests+=1;
                    continue;
                }
                let body = if requests == 0 || (mirrors && requests == 1) { &package_bytes } else { &rollback_bytes };
                write!(stream,"HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",body.len()).unwrap();
                if mirrors {
                    evidence.push(serde_json::json!({"path":format!("/{name}"),"status":200,"bytes":body.len(),"sha256":hex::encode(Sha256::digest(body))}));
                    std::fs::write(&request_log, serde_json::to_vec_pretty(&evidence).unwrap())
                        .unwrap();
                }
                stream.write_all(body).unwrap();requests+=1;
            }
            requests
        });
        let probe=TcpListener::bind("127.0.0.1:0").unwrap();let address=probe.local_addr().unwrap();drop(probe);
        let mut server=php_command().args(["-S",&address.to_string()]).arg(api.join("tests/agent_status_http_router.php"))
            .env("AGENT_HTTP_FIXTURE_DB",&db).env("RUSTDESK_DB_DRIVER","sqlite").env("RUSTDESK_ADMIN_PATH","/agent-test")
            .env("RUSTDESK_INSTALL_CONFIG",root.join("absent.json"))
            .stdout(Stdio::null()).stderr(std::fs::File::create(root.join("php.stderr.log")).unwrap()).spawn().unwrap();
        let paths=AgentPaths::from_program_data(&root,super::super::Edition::Standard);
        std::fs::create_dir_all(&paths.state).unwrap();
        let transaction_root=paths.agent.join("transactions").join(&command);
        if restart {
            let input = serde_json::json!({"api":address.to_string(),"download":download_address.to_string(),
                "public":STANDARD.encode(identity.public.as_ref()),"secret":STANDARD.encode(identity.secret.as_ref()),
                "release_public":STANDARD.encode(release_public.as_ref()),"state":actual,"command":command});
            let input_path = root.join("poll-fixture.json");
            std::fs::write(&input_path,serde_json::to_vec(&input).unwrap()).unwrap();
            workspace::verify_file(&input_path).unwrap();
        }
        let run_poll_child = |phase: &str| {
            let mut child=Command::new(std::env::current_exe().unwrap())
                .args(["--exact","management_agent::operations_runtime::tests::local_command_poll_child","--nocapture"])
                .env("RUSTDESK_LOCAL_POLL_FIXTURE",&root).env("RUSTDESK_LOCAL_POLL_PHASE",phase)
                .stdout(std::fs::File::create(root.join(format!("poll-{phase}.stdout.txt"))).unwrap())
                .stderr(std::fs::File::create(root.join(format!("poll-{phase}.stderr.txt"))).unwrap()).spawn().unwrap();
            let observed=super::super::handoff::ObservedProcess::open(child.id()).unwrap();
            let owner=observed.owner().clone();
            let deadline=Instant::now()+Duration::from_secs(600);
            let exit=loop {
                if let Some(status)=child.try_wait().unwrap(){break status;}
                assert!(Instant::now()<deadline,"poll child timeout; preserve process and evidence");
                std::thread::sleep(Duration::from_millis(100));
            };
            assert!(exit.success(),"poll {phase}: {}",std::fs::read_to_string(root.join(format!("poll-{phase}.stderr.txt"))).unwrap());
            assert!(!observed.is_alive().unwrap());
            let recorded:super::super::handoff::Owner=serde_json::from_slice(&std::fs::read(root.join(format!("poll-{phase}-owner.json"))).unwrap()).unwrap();
            assert_eq!(recorded,owner);
            owner
        };
        if rollback {
            std::fs::write(paths.agent.join(format!("fixture-fail-{command}")), b"inject validation failure after real installation").unwrap();
        }
        LOCAL_PACKAGE_FIXTURE.with(|value|*value.borrow_mut()=Some((release_public.as_ref().to_vec(),download_address)));
        let outcome=std::panic::catch_unwind(std::panic::AssertUnwindSafe(||{
            let deadline=Instant::now()+Duration::from_secs(10);
            while std::net::TcpStream::connect(address).is_err(){assert!(Instant::now()<deadline);std::thread::sleep(Duration::from_millis(25));}
            let base=format!("http://{address}");
            let launch_owner=if restart {Some(run_poll_child("launch"))}else{
                poll_with_identity(&identity,&base,&actual,&paths).unwrap();None
            };
            let journal_path=paths.state.join("commands-1.json");
            assert_eq!(CommandJournal::open(&journal_path).unwrap().state(&command),Some(ExecutionState::Running),
                "launch must not report success or bypass preparation failure");
            if mirrors {
                let mut cache_evidence = Vec::new();
                for (directory, expected_name, approved) in [
                    (
                        paths.agent.join(format!("approved-{command}")),
                        target_name.as_str(),
                        &manifest.targets[key],
                    ),
                    (
                        paths.agent.join(format!("rollback-{command}")),
                        rollback_name,
                        &rollback_manifest.targets[key],
                    ),
                ] {
                    let path = directory.join(expected_name);
                    let timing: serde_json::Value = serde_json::from_slice(
                        &std::fs::read(directory.with_extension("download-timing.json")).unwrap(),
                    )
                    .unwrap();
                    assert_eq!(timing["success"], true);
                    assert!(
                        timing["elapsed_ms"].as_u64().unwrap() < 60000,
                        "real package including verification must meet unchanged budget: {timing}"
                    );
                    let locked = package_gate::lock_verified_file(&path, approved).unwrap();
                    assert!(
                        std::fs::write(&path, b"must not replace a locked approved package")
                            .is_err()
                    );
                    assert!(!directory.join("target-mirror.msi").exists());
                    assert!(!directory.join("rollback-mirror.msi").exists());
                    cache_evidence.push(serde_json::json!({"path":path,"basename":expected_name,"size":approved.size,"sha256":approved.sha256,"download_elapsed_ms":timing["elapsed_ms"]}));
                    drop(locked);
                }
                std::fs::write(
                    root.join("mirror-cache-verified.json"),
                    serde_json::to_vec_pretty(&cache_evidence).unwrap(),
                )
                .unwrap();
                let handoff = super::super::handoff::Handoff::decode(
                    &std::fs::read(transaction_root.join("handoff.json")).unwrap(),
                )
                .unwrap();
                std::fs::write(
                    root.join("mirror-handoff-verified.json"),
                    serde_json::to_vec_pretty(&handoff).unwrap(),
                )
                .unwrap();
            }
            if restart {
                assert!(!transaction_root.join("fixture-helper-finished").exists(), "launch process must exit before helper finalization");
                let binding=super::super::handoff::Handoff::decode(&std::fs::read(transaction_root.join("handoff.json")).unwrap()).unwrap();
                assert_eq!(Some(&binding.parent),launch_owner.as_ref());
                assert!(!super::super::handoff::ObservedProcess::owner_alive(&binding.parent).unwrap());
            }
            let deadline=Instant::now()+Duration::from_secs(600);
            while !transaction_root.join("fixture-helper-finished").exists(){
                assert!(Instant::now()<deadline,"helper did not finalize: {}",std::fs::read_to_string(transaction_root.join("parent-helper.stderr.log")).unwrap_or_default());
                std::thread::sleep(Duration::from_millis(500));
            }
            let evidence=std::fs::read(transaction_root.join("install-result.json")).unwrap();
            if restart {
                let report_owner=run_poll_child("report");
                let duplicate_owner=run_poll_child("duplicate");
                assert_ne!(launch_owner.as_ref().unwrap(),&report_owner);
                assert_ne!(report_owner,duplicate_owner);
            } else {
                poll_with_identity(&identity,&base,&actual,&paths).unwrap();
                poll_with_identity(&identity,&base,&actual,&paths).unwrap();
            }
            let journal=CommandJournal::open(&journal_path).unwrap();
            assert_eq!(journal.state(&command),Some(if rollback { ExecutionState::RolledBack } else { ExecutionState::Succeeded }));assert!(journal.pending_events().is_empty());
            assert_eq!(std::fs::read(transaction_root.join("install-result.json")).unwrap(),evidence);
            let query=r#"$db=new PDO('sqlite:'.getenv('AGENT_HTTP_FIXTURE_DB'));echo json_encode([$db->query('SELECT status,event_sequence,receipt_persisted FROM device_agent_commands')->fetch(PDO::FETCH_ASSOC),$db->query('SELECT status FROM device_agent_events ORDER BY sequence')->fetchAll(PDO::FETCH_COLUMN),$db->query('SELECT last_seen,last_heartbeat FROM device_reports')->fetch(PDO::FETCH_NUM),(int)$db->query('SELECT COUNT(*) FROM agent_operations_audit')->fetchColumn()]);"#;
            let queried=php_command().args(["-r",query]).env("AGENT_HTTP_FIXTURE_DB",&db).output().unwrap();
            assert!(queried.status.success());let sql:serde_json::Value=serde_json::from_slice(&queried.stdout).unwrap();
            let terminal = if rollback { "rolled_back" } else { "succeeded" };
            assert_eq!(sql[0],serde_json::json!({"status":terminal,"event_sequence":3,"receipt_persisted":1}));
            assert_eq!(sql[1],serde_json::json!(["receipt_persisted","running",terminal]));
            assert_eq!(sql[2],serde_json::json!([123,123]));assert!(sql[3].as_u64().unwrap()>=2);
            let after:AgentState=serde_json::from_slice(&std::fs::read(installed_paths.state.join("agent.json")).unwrap()).unwrap();
            assert_eq!(after.agent_installation_id,actual.agent_installation_id);
            assert!(!installed_paths.state.join("maintenance.json").exists());
            assert!(super::super::native_install::client_ready(Path::new(r"C:\Program Files\RustDesk\RustDesk.exe"),if rollback {2026100502} else {target_build},true));
            if cross_build {
                let plan=super::super::install_plan::InstallPlan::decode(&std::fs::read(transaction_root.join("prepared.json")).unwrap()).unwrap();
                assert_eq!(plan.original_build_seq,2026100502);
                assert_eq!(plan.target_build_seq,2026100503);
                let rollback_saved:hbb_common::UpdateManifest=serde_json::from_slice(&std::fs::read(transaction_root.join("rollback-manifest.json")).unwrap()).unwrap();
                assert_eq!(rollback_saved.targets[key].build_seq,2026100502);
                let agent_hash=hex::encode(Sha256::digest(std::fs::read(installed_paths.agent.join("rustdesk-management-agent.exe")).unwrap()));
                assert_eq!(agent_hash, if rollback { before_agent.as_str() } else { "091831fac3da7c33c27ac81672f5f7e8572fde17e1b7ce300d12217025f0d504" });
                let bootstrap=installed_paths.root.join("RecoveryBootstrap");
                assert_eq!(hex::encode(Sha256::digest(std::fs::read(bootstrap.join("rustdesk-recovery-bootstrap.exe")).unwrap())),agent_hash);
                let status=Command::new(installed_paths.agent.join("rustdesk-management-agent.exe"))
                    .args(["status","--json","--edition","standard","--package-kind","msi"]).output().unwrap();
                assert!(status.status.success());
                let status:serde_json::Value=serde_json::from_slice(&status.stdout).unwrap();
                let expected_build = if rollback { 2026100502u64 } else { 2026100503u64 };
                assert_eq!(status["agent"]["build_seq"],expected_build);
                assert_eq!(status["application"]["build_seq"],expected_build);
                assert_eq!(status["application"]["state"],"ready");
                if mirrors && !rollback {
                    assert_eq!(
                        hex::encode(Sha256::digest(
                            std::fs::read(r"C:\Program Files\RustDesk\librustdesk.dll").unwrap()
                        )),
                        "8c523baf9937057c6d6260b0698963c544685c40d3bf6de485acad924143a9ef"
                    );
                }
                std::fs::write(root.join("upgraded-status.json"),serde_json::to_vec_pretty(&status).unwrap()).unwrap();
            }
            if rollback {
                assert_eq!(std::fs::read(installed_paths.root.join("RecoveryBootstrap/bootstrap.json")).unwrap(), before_bootstrap);
                assert_eq!(hex::encode(Sha256::digest(std::fs::read(installed_paths.agent.join("rustdesk-management-agent.exe")).unwrap())), before_agent);
                assert_eq!(hex::encode(Sha256::digest(std::fs::read(r"C:\Program Files\RustDesk\librustdesk.dll").unwrap())), before_dll);
                let restored: serde_json::Value = serde_json::from_slice(&std::fs::read(transaction_root.join("rollback-result.json")).unwrap()).unwrap();
                assert_eq!(restored["outcome"]["status"], "installed");
                let result: serde_json::Value = serde_json::from_slice(&std::fs::read(transaction_root.join("execution-result.json")).unwrap()).unwrap();
                assert_eq!(result["phase"], "rolled_back");
                assert!(result["error"].as_str().unwrap().contains("validation failed"));
            }
            std::fs::write(root.join("verified-sql.json"),serde_json::to_vec_pretty(&sql).unwrap()).unwrap();
        }));
        LOCAL_PACKAGE_FIXTURE.with(|value|*value.borrow_mut()=None);
        stop.store(true,Ordering::SeqCst);let requests=downloader.join().unwrap();
        server.kill().unwrap();server.wait().unwrap();
        if restart { std::fs::remove_file(root.join("poll-fixture.json")).unwrap(); }
        if let Err(error)=outcome{std::panic::resume_unwind(error);}
        assert_eq!(requests,if mirrors {4} else {2});
        if mirrors {
            let requests:serde_json::Value=serde_json::from_slice(&std::fs::read(root.join("mirror-http-requests.json")).unwrap()).unwrap();
            assert_eq!(requests.as_array().unwrap().iter().map(|v|v["path"].as_str().unwrap()).collect::<Vec<_>>(),
                vec![format!("/{target_name}"),"/target-mirror.msi".into(),format!("/{rollback_name}"),"/rollback-mirror.msi".into()]);
            assert_eq!(requests[0]["status"],503);assert_eq!(requests[2]["status"],503);
            assert_eq!(requests[1]["bytes"],manifest.targets[key].size);
            assert_eq!(requests[3]["bytes"],rollback_manifest.targets[key].size);
        }
        println!("Local signed API/SQLite -> two HTTP downloads -> native helper -> one {} event: PASS", if rollback { "rolled_back" } else { "succeeded" });
        println!("Retained registered MSI transaction source: {}",transaction_root.display());
    }

    #[test]
    fn downloaded_response_never_overwrites_cache_and_failure_preserves_evidence() {
        use sha2::{Digest, Sha256};
        use std::{io::{Read, Write}, net::TcpListener};
        let root = std::env::temp_dir().join(format!("agent-download-fixture-{}", std::process::id()));
        workspace::create(&root).unwrap();
        let response = |status: u16, body: &[u8], declared: Option<usize>| {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let address = listener.local_addr().unwrap();
            let body = body.to_vec();
            let server = std::thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                stream.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
                let mut request = [0; 4096];
                assert!(stream.read(&mut request).unwrap() > 0);
                let length = declared.map(|count| format!("Content-Length: {count}\r\n")).unwrap_or_default();
                write!(stream,"HTTP/1.1 {status} Fixture\r\n{length}Connection: close\r\nLocation: http://127.0.0.1:1/forbidden\r\n\r\n").unwrap();
                stream.write_all(&body).unwrap();
            });
            let reply = reqwest::blocking::Client::builder().no_proxy().timeout(Duration::from_secs(3))
                .redirect(reqwest::redirect::Policy::none()).build().unwrap()
                .get(format!("http://{address}/package")).send().unwrap();
            server.join().unwrap();
            reply
        };
        let target = hbb_common::UpdateTarget { primary:"https://download.yan.life/rustdesk/stable/1.5.3/test.msi".into(),
            size:4, sha256:hex::encode(Sha256::digest(b"good")), ..Default::default() };
        let cache = root.join("test.msi");
        std::fs::write(&cache, b"previous").unwrap();
        let attempt = receive_approved(&root, &target, response(200, b"good", Some(4)));
        assert!(attempt.is_err(), "publication must not replace an occupied final path");
        assert_eq!(std::fs::read(&cache).unwrap(), b"previous");
        std::fs::remove_file(&cache).unwrap();
        if root.join("package.partial").exists() { std::fs::remove_file(root.join("package.partial")).unwrap(); }
        for (name, code, body, length, error_text) in [
            ("http",503,b"good".as_slice(),Some(4),"HTTP 503"),
            ("redirect",302,b"good".as_slice(),Some(4),"HTTP 302"),
            ("length",200,b"bad".as_slice(),Some(3),"content length mismatch"),
            ("short",200,b"bad".as_slice(),None,"size mismatch"),
            ("long",200,b"toolong".as_slice(),None,"size mismatch"),
            ("hash",200,b"evil".as_slice(),Some(4),"integrity mismatch"),
        ] {
            let directory = root.join(name);
            workspace::create(&directory).unwrap();
            std::fs::write(directory.join("old-record"), b"retain").unwrap();
            let error = receive_approved(&directory, &target, response(code, body, length)).err().unwrap();
            assert!(error.to_string().contains(error_text), "{name}: {error}");
            assert!(!directory.join("test.msi").exists());
            assert_eq!(std::fs::read(directory.join("old-record")).unwrap(), b"retain");
        }
        let directory = root.join("truncated");
        workspace::create(&directory).unwrap();
        assert!(receive_approved(&directory, &target, response(200,b"go",Some(4))).is_err());
        assert!(!directory.join("test.msi").exists());
        let directory = root.join("success");
        workspace::create(&directory).unwrap();
        let (cache, locked) = receive_approved(&directory, &target, response(200,b"good",Some(4))).unwrap();
        assert_eq!(std::fs::read(&cache).unwrap(), b"good");
        assert!(std::fs::write(&cache,b"evil").is_err());
        drop(locked);
        let (reused, locked) = download_approved(&directory, &target).unwrap();
        assert_eq!(reused,cache);
        drop(locked);
        let interrupted = root.join("interrupted");
        workspace::create(&interrupted).unwrap();
        std::fs::write(interrupted.join("package.partial"),b"incomplete").unwrap();
        assert!(download_approved(&interrupted,&target).is_err());
        assert_eq!(std::fs::read(interrupted.join("package.partial")).unwrap(),b"incomplete");
        assert!(!interrupted.join("test.msi").exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn fixed_integrity_and_diagnostics_report_measurements_without_false_trust_or_secrets() {
        let root = std::env::temp_dir().join(format!("agent-fixed-report-{}", std::process::id()));
        let paths = AgentPaths::from_program_data(&root, super::super::Edition::Standard);
        std::fs::create_dir_all(&paths.agent).unwrap();
        std::fs::create_dir_all(&paths.client).unwrap();
        let agent = paths.agent.join("rustdesk-management-agent.exe");
        std::fs::write(&agent, b"unapproved-fixture").unwrap();
        let mut state = AgentState::new(super::super::Edition::Standard, &paths);
        state.last_error = "secret-token-123 https://example.invalid/?password=secret".into();
        state.service_recovery_error = "secret-user-profile".into();
        state.normal_install_path = root.join("arbitrary").to_string_lossy().into_owned();
        let report = execute_command("package.verify", &state, &paths).unwrap();
        assert_eq!(report["verified"], false);
        assert_eq!(report["objects"]["agent"]["status"], "unverified");
        assert_eq!(report["objects"]["fallback"]["status"], "missing");
        assert_eq!(report["objects"]["normal"]["status"], "unapproved_path");
        assert_eq!(report["objects"]["agent"]["size"], 18);
        let diagnostics = execute_command("diagnostics.collect", &state, &paths).unwrap();
        let text = serde_json::to_string(&diagnostics).unwrap();
        for secret in ["secret-token", "secret-user-profile", "password=", "example.invalid", "arbitrary"] {
            assert!(!text.contains(secret), "diagnostic leaked {secret}");
        }
        assert_eq!(diagnostics["schema"], 1);
        assert_eq!(diagnostics["errors"]["update_error_present"], true);
        assert_eq!(diagnostics["integrity"]["verified"], false);
        assert_eq!(std::fs::read(agent).unwrap(), b"unapproved-fixture");
        assert_eq!(state.last_error, "secret-token-123 https://example.invalid/?password=secret");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn transport_rejects_wrong_identity_and_non_agent_routes_before_network() {
        hbb_common::sodiumoxide::init().unwrap();
        let (public, secret) = hbb_common::sodiumoxide::crypto::sign::gen_keypair();
        let identity = identity::DeviceIdentity {
            id: "device".into(),
            uuid: "uuid".into(),
            public,
            secret,
        };
        let wrong = serde_json::json!({"client_id":"other","client_uuid":"uuid"});
        assert!(request(
            &identity,
            "http://127.0.0.1:1",
            "POST",
            "/rd/update/v1/agent/enroll",
            "",
            &wrong
        )
        .is_err());
        let payload = serde_json::json!({"client_id":"device","client_uuid":"uuid"});
        assert!(request(
            &identity,
            "http://127.0.0.1:1",
            "POST",
            "/arbitrary",
            "",
            &payload
        )
        .is_err());
    }

    #[test]
    fn http_rate_limit_carries_bounded_retry_after_without_accepting_the_body() {
        use std::{
            io::{Read, Write},
            net::TcpListener,
        };
        hbb_common::sodiumoxide::init().unwrap();
        let (public, secret) = sign::gen_keypair();
        let identity = identity::DeviceIdentity {
            id: "device".into(),
            uuid: "uuid".into(),
            public,
            secret,
        };
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let fixture = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            stream
                .set_write_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut received = Vec::new();
            let mut buffer = [0; 4096];
            loop {
                let size = stream.read(&mut buffer).unwrap();
                assert!(size > 0);
                received.extend_from_slice(&buffer[..size]);
                if let Some(end) = received.windows(4).position(|bytes| bytes == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&received[..end]).to_ascii_lowercase();
                    let length: usize = headers
                        .lines()
                        .find_map(|line| line.strip_prefix("content-length:").map(str::trim))
                        .unwrap()
                        .parse()
                        .unwrap();
                    if received.len() >= end + 4 + length {
                        break;
                    }
                }
            }
            stream.write_all(b"HTTP/1.1 429 Too Many Requests\r\nRetry-After: 999999\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
        });
        let payload = serde_json::json!({"client_id":"device","client_uuid":"uuid"});
        let error = request(
            &identity,
            &base,
            "POST",
            "/rd/update/v1/agent/enroll",
            "",
            &payload,
        )
        .unwrap_err();
        assert!(matches!(error.failure, network_policy::Failure::Http(429)));
        assert_eq!(error.retry_after, Some(3600));
        fixture.join().unwrap();
        let error = request(
            &identity,
            "http://127.0.0.1:1",
            "POST",
            "/rd/update/v1/agent/enroll",
            "",
            &payload,
        )
        .unwrap_err();
        assert!(matches!(error.failure, network_policy::Failure::Transport));
        assert_eq!(error.retry_after, None);
    }

    #[test]
    fn signed_rust_command_roundtrip_uses_real_php_sqlite_and_journal() {
        use std::{
            net::TcpListener,
            path::Path,
            process::{Command, Stdio},
        };
        hbb_common::sodiumoxide::init().unwrap();
        let (public, secret) = sign::gen_keypair();
        let identity = identity::DeviceIdentity {
            id: "test-device".into(),
            uuid: "test-uuid".into(),
            public,
            secret,
        };
        let source = Path::new(file!());
        let source = if source.is_absolute() {
            source.to_owned()
        } else {
            Path::new(env!("CARGO_MANIFEST_DIR")).join(source)
        };
        let api = source
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("rustdesk-api");
        let api = std::env::var_os("AGENT_TEST_API_ROOT")
            .map(std::path::PathBuf::from)
            .unwrap_or(api);
        let php = std::env::var("AGENT_TEST_PHP").unwrap_or_else(|_| "php".into());
        let ini = std::env::var("AGENT_TEST_PHP_INI").unwrap_or_default();
        let php_command = || {
            let mut cmd = Command::new(&php);
            if !ini.is_empty() {
                cmd.args(["-c", &ini]);
            }
            cmd
        };
        let target_key = "windows-x86_64-msi-standard";
        let manifest = serde_json::json!({
            "schema":2,"product":"rustdesk-yan","edition":"multi","channel":"stable",
            "catalog_revision":2026100503u64,
            "targets": {target_key:{"version":"1.5.4","build_seq":2026100503u64,
                "source_commit":"fixture","primary":"https://download.yan.life/rustdesk/stable/1.5.4/client.msi",
                "size":4,"sha256":"a".repeat(64),"signature_key_id":"yan-release-2026",
                "signature":STANDARD.encode([0u8;64]),"mirrors":[]}},
            "notes":"恢复验证：保留中文、斜杠 / 和换行\n"
        });
        let reference = php_command().args(["-r", r#"require getenv('AGENT_FIXTURE_ROOT').'/sqlite/lib.php';require getenv('AGENT_FIXTURE_ROOT').'/sqlite/agent_operations.php';$manifest=json_decode(getenv('AGENT_REFERENCE_MANIFEST'),true,64,JSON_THROW_ON_ERROR);echo agent_ops_json(['package_id'=>agent_package_reference($manifest,'windows-x86_64-msi-standard'),'target_key'=>'windows-x86_64-msi-standard','manifest'=>$manifest]);"#])
            .env("AGENT_FIXTURE_ROOT", &api)
            .env("AGENT_REFERENCE_MANIFEST", serde_json::to_string(&manifest).unwrap())
            .output().unwrap();
        assert!(reference.status.success(), "PHP package reference: {}", String::from_utf8_lossy(&reference.stderr));
        let reply: serde_json::Value = serde_json::from_slice(&reference.stdout).unwrap();
        let reference = reply["package_id"].as_str().unwrap();
        assert_eq!(package_gate::package_reference(&reply["manifest"],target_key).unwrap(), reference);
        let resolved = package_gate::bind_resolved(&reply,reference,target_key,2026100503).unwrap();
        assert_eq!(resolved.targets[target_key].size,4);
        assert!(package_gate::verify(&resolved,target_key,2026100502).is_err(), "matching reference is not a valid release signature");
        let mut changed = reply.clone();
        changed["manifest"]["targets"][target_key]["size"] = 5.into();
        assert!(package_gate::bind_resolved(&changed,reference,target_key,2026100503).is_err());
        assert_eq!(reply["manifest"],manifest,"reference verification must preserve unknown manifest fields");
        let directory = std::env::temp_dir().join(format!(
            "agent-status-test-rust-{}-{}",
            std::process::id(),
            hex::encode(hbb_common::sodiumoxide::randombytes::randombytes(8))
        ));
        std::fs::create_dir(&directory).unwrap();
        let db = directory.join("agent-status-test.db");
        let seed = r#"require getenv('AGENT_FIXTURE_ROOT').'/sqlite/lib.php';$db=new PDO('sqlite:'.getenv('AGENT_HTTP_FIXTURE_DB'));configure_pdo($db);ensure_schema($db);$pk=getenv('AGENT_FIXTURE_PK');$now=time();db_insert_ignore($db,'device_reports',['id'=>'test-device','uuid'=>'test-uuid','payload'=>'{}','last_seen'=>123,'last_heartbeat'=>123,'heartbeat_payload'=>'{}']);db_insert_ignore($db,'device_update_keys',['device_id'=>'test-device','uuid'=>'test-uuid','public_key'=>$pk,'first_seen_at'=>$now,'last_seen_at'=>$now]);db_exec($db,'INSERT INTO device_agent_bindings(device_id,uuid,installation_id,generation,key_fingerprint,authorized,updated_at) VALUES(:id,:uuid,:installation,1,:fp,1,:now)',['id'=>'test-device','uuid'=>'test-uuid','installation'=>str_repeat('a',32),'fp'=>hash('sha256',base64_decode($pk)),'now'=>$now]);$envelope=['schema'=>1,'id'=>str_repeat('c',32),'type'=>'service.inspect','type_version'=>1,'client_id'=>'test-device','client_uuid'=>'test-uuid','agent_installation_id'=>str_repeat('a',32),'enrollment_generation'=>1,'created_at'=>$now,'expires_at'=>$now+300,'timeout_seconds'=>900,'preconditions'=>['active_sessions'=>'defer'],'args'=>[]];$encoded=agent_ops_json($envelope);$digest=hash('sha256',$encoded);db_exec($db,'INSERT INTO device_agent_commands(command_id,device_id,uuid,installation_id,generation,type,type_version,envelope,digest,status,idempotency_key,request_digest,created_at,expires_at,updated_at,actor_id) VALUES(:command,:id,:uuid,:installation,1,\'service.inspect\',1,:envelope,:digest,\'pending\',\'fixture\',:digest,:now,:expires,:now,1)',['command'=>$envelope['id'],'id'=>'test-device','uuid'=>'test-uuid','installation'=>str_repeat('a',32),'envelope'=>$encoded,'digest'=>$digest,'now'=>$now,'expires'=>$now+300]);"#;
        let result = php_command()
            .args(["-r", seed])
            .env("AGENT_FIXTURE_ROOT", &api)
            .env("AGENT_HTTP_FIXTURE_DB", &db)
            .env(
                "AGENT_FIXTURE_PK",
                STANDARD.encode(identity.public.as_ref()),
            )
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{} {}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
        let queue_seed = r#"require getenv('AGENT_FIXTURE_ROOT').'/sqlite/lib.php';require getenv('AGENT_FIXTURE_ROOT').'/sqlite/agent_operations.php';$db=new PDO('sqlite:'.getenv('AGENT_HTTP_FIXTURE_DB'));configure_pdo($db);$row=db_one($db,'SELECT * FROM device_agent_commands');$wire=json_decode($row['envelope'],true);$wire['schema']=2;$encoded=agent_ops_json($wire);db_exec($db,'UPDATE device_agent_commands SET envelope=:envelope,digest=:digest',['envelope'=>$encoded,'digest'=>hash('sha256',$encoded)]);$wire['schema']=1;$wire['id']=str_repeat('d',32);$encoded=agent_ops_json($wire);$row['command_id']=$wire['id'];$row['envelope']=$encoded;$row['digest']=hash('sha256',$encoded);$row['idempotency_key']='after-unsupported';db_insert_ignore($db,'device_agent_commands',$row);"#;
        let result = php_command()
            .args(["-r", queue_seed])
            .env("AGENT_FIXTURE_ROOT", &api)
            .env("AGENT_HTTP_FIXTURE_DB", &db)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stdout)
        );
        let invalid_page_seed = r#"require getenv('AGENT_FIXTURE_ROOT').'/sqlite/lib.php';require getenv('AGENT_FIXTURE_ROOT').'/sqlite/agent_operations.php';$db=new PDO('sqlite:'.getenv('AGENT_HTTP_FIXTURE_DB'));configure_pdo($db);$row=db_one($db,'SELECT * FROM device_agent_commands ORDER BY command_id LIMIT 1');for($i=0;$i<20;$i++){$wire=json_decode($row['envelope'],true);$wire['id']=sprintf('%032x',$i+1);$wire['client_id']='foreign-device';$wire['created_at']-=1;$encoded=agent_ops_json($wire);$row['command_id']=$wire['id'];$row['envelope']=$encoded;$row['digest']=hash('sha256',$encoded);$row['idempotency_key']='foreign-'.$i;$row['created_at']=$wire['created_at'];db_insert_ignore($db,'device_agent_commands',$row);}"#;
        let result = php_command()
            .args(["-r", invalid_page_seed])
            .env("AGENT_FIXTURE_ROOT", &api)
            .env("AGENT_HTTP_FIXTURE_DB", &db)
            .output()
            .unwrap();
        assert!(result.status.success());
        let probe = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = probe.local_addr().unwrap();
        drop(probe);
        let mut server = php_command()
            .args(["-S", &address.to_string()])
            .arg(api.join("tests/agent_status_http_router.php"))
            .env("AGENT_HTTP_FIXTURE_DB", &db)
            .env("RUSTDESK_DB_DRIVER", "sqlite")
            .env("RUSTDESK_ADMIN_PATH", "/agent-test")
            .env("AGENT_HTTP_FIXTURE_CANCEL_COMMAND", "e".repeat(32))
            .env("RUSTDESK_INSTALL_CONFIG", directory.join("absent.json"))
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let paths = AgentPaths::from_program_data(&directory, super::super::Edition::Standard);
        std::fs::create_dir_all(&paths.state).unwrap();
        let mut state = AgentState::new(super::super::Edition::Standard, &paths);
        state.agent_installation_id = "a".repeat(32);
        let result = (|| {
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            while std::net::TcpStream::connect(address).is_err() {
                if std::time::Instant::now() >= deadline {
                    return Err("PHP fixture startup failed".to_owned());
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            let binding = serde_json::json!({"schema":1,"client_id":identity.id,"client_uuid":identity.uuid,"agent_installation_id":state.agent_installation_id,"enrollment_generation":1});
            let mut claim = binding.clone();
            let lookup = r#"$db=new PDO('sqlite:'.getenv('AGENT_HTTP_FIXTURE_DB'));$s=$db->prepare('SELECT digest FROM device_agent_commands WHERE command_id=?');$s->execute([str_repeat('c',32)]);echo $s->fetchColumn();"#;
            let digest = php_command()
                .args(["-r", lookup])
                .env("AGENT_HTTP_FIXTURE_DB", &db)
                .output()
                .map_err(|e| e.to_string())?;
            claim["digest"] = String::from_utf8(digest.stdout)
                .map_err(|e| e.to_string())?
                .into();
            let claimed = request(
                &identity,
                &format!("http://{address}"),
                "POST",
                &format!("/rd/update/v1/agent/commands/{}/claim", "c".repeat(32)),
                &"c".repeat(32),
                &claim,
            )?;
            let mut initial = CommandJournal::open(&paths.state.join("commands-1.json"))?;
            initial.prepare(
                &"c".repeat(32),
                claimed["digest"].as_str().ok_or("fixture digest absent")?,
                claimed["expires_at"]
                    .as_u64()
                    .ok_or("fixture expiry absent")?,
                unix_timestamp(),
            )?;
            initial.receipt(&"c".repeat(32))?;
            let event = initial.pending_events()[0].clone();
            let mut uploaded = event.payload;
            for key in [
                "client_id",
                "client_uuid",
                "agent_installation_id",
                "enrollment_generation",
            ] {
                uploaded[key] = binding[key].clone();
            }
            // Persist on the server but deliberately discard its acknowledgement.
            request(
                &identity,
                &format!("http://{address}"),
                "POST",
                "/rd/update/v1/agent/events",
                &"c".repeat(32),
                &uploaded,
            )?;
            drop(initial);
            let cancel_seed = r#"require getenv('AGENT_FIXTURE_ROOT').'/sqlite/lib.php';require getenv('AGENT_FIXTURE_ROOT').'/sqlite/agent_operations.php';$db=new PDO('sqlite:'.getenv('AGENT_HTTP_FIXTURE_DB'));configure_pdo($db);$row=db_one($db,'SELECT * FROM device_agent_commands WHERE idempotency_key=\'after-unsupported\'');$wire=json_decode($row['envelope'],true);$wire['id']=str_repeat('e',32);$row['command_id']=$wire['id'];$row['envelope']=agent_ops_json($wire);$row['digest']=hash('sha256',$row['envelope']);$row['idempotency_key']='cancelled-at-claim';db_insert_ignore($db,'device_agent_commands',$row);$wire['id']=str_repeat('f',32);$wire['created_at']=time()-400;$wire['expires_at']=time()-100;$row['command_id']=$wire['id'];$row['envelope']=agent_ops_json($wire);$row['digest']=hash('sha256',$row['envelope']);$row['idempotency_key']='expired-unreceived';$row['status']='accepted';$row['created_at']=$wire['created_at'];$row['expires_at']=$wire['expires_at'];db_insert_ignore($db,'device_agent_commands',$row);"#;
            let seeded = php_command()
                .args(["-r", cancel_seed])
                .env("AGENT_FIXTURE_ROOT", &api)
                .env("AGENT_HTTP_FIXTURE_DB", &db)
                .output()
                .map_err(|e| e.to_string())?;
            if !seeded.status.success() {
                return Err("cancellation fixture creation failed".into());
            }
            poll_with_identity(&identity, &format!("http://{address}"), &state, &paths)?;
            poll_with_identity(&identity, &format!("http://{address}"), &state, &paths)?;
            let log = CommandJournal::open(&paths.state.join("commands-1.json"))?;
            if log.state(&"c".repeat(32)) != Some(ExecutionState::Unsupported)
                || log.state(&"d".repeat(32)) != Some(ExecutionState::Succeeded)
                || log.state(&"e".repeat(32)) != Some(ExecutionState::Cancelled)
                || log.state(&"f".repeat(32)).is_some()
                || !log.pending_events().is_empty()
            {
                return Err("command journal not acknowledged".into());
            }
            let query = r#"$db=new PDO('sqlite:'.getenv('AGENT_HTTP_FIXTURE_DB'));echo json_encode([$db->query("SELECT status,receipt_persisted FROM device_agent_commands WHERE idempotency_key NOT LIKE 'foreign-%' ORDER BY command_id")->fetchAll(PDO::FETCH_ASSOC),(int)$db->query('SELECT COUNT(*) FROM device_agent_events')->fetchColumn(),$db->query('SELECT last_seen,last_heartbeat FROM device_reports')->fetch(PDO::FETCH_NUM),(int)$db->query("SELECT COUNT(*) FROM device_agent_commands WHERE idempotency_key LIKE 'foreign-%' AND status='pending'")->fetchColumn()]);"#;
            let persisted = php_command()
                .args(["-r", query])
                .env("AGENT_HTTP_FIXTURE_DB", &db)
                .output()
                .map_err(|e| e.to_string())?;
            let values: serde_json::Value =
                serde_json::from_slice(&persisted.stdout).map_err(|e| e.to_string())?;
            if values
                != serde_json::json!([[{"status":"unsupported","receipt_persisted":1},{"status":"succeeded","receipt_persisted":1},{"status":"cancelled","receipt_persisted":0},{"status":"accepted","receipt_persisted":0}],5,[123,123],20])
            {
                return Err(format!("SQL mismatch: {values}"));
            }
            let before = recovery_windows::inspect(&state, &paths)?;
            if before.state != super::super::recovery::ServiceState::Running || before.trusted_path {
                return Err("deferred regression requires running lab service and untrusted fixture paths".into());
            }
            let defer_seed = r#"require getenv('AGENT_FIXTURE_ROOT').'/sqlite/lib.php';require getenv('AGENT_FIXTURE_ROOT').'/sqlite/agent_operations.php';$db=new PDO('sqlite:'.getenv('AGENT_HTTP_FIXTURE_DB'));configure_pdo($db);$row=db_one($db,"SELECT * FROM device_agent_commands WHERE idempotency_key='after-unsupported'");$wire=json_decode($row['envelope'],true);$wire['id']=str_repeat('9',32);$wire['type']='service.restart';$wire['created_at']=time();$wire['expires_at']=time()+300;$row['command_id']=$wire['id'];$row['type']=$wire['type'];$row['envelope']=agent_ops_json($wire);$row['digest']=hash('sha256',$row['envelope']);$row['idempotency_key']='deferred-restart';$row['status']='pending';$row['event_sequence']=0;$row['receipt_persisted']=0;$row['created_at']=$wire['created_at'];$row['expires_at']=$wire['expires_at'];db_insert_ignore($db,'device_agent_commands',$row);"#;
            let seeded = php_command().args(["-r", defer_seed]).env("AGENT_FIXTURE_ROOT", &api)
                .env("AGENT_HTTP_FIXTURE_DB", &db).output().map_err(|e| e.to_string())?;
            if !seeded.status.success() { return Err("deferred fixture seed failed".into()); }
            let mut blocked_state = state.clone();
            blocked_state.disabled = true;
            for _ in 0..2 {
                poll_with_identity(&identity, &format!("http://{address}"), &blocked_state, &paths)?;
            }
            let journal = CommandJournal::open(&paths.state.join("commands-1.json"))?;
            if journal.state(&"9".repeat(32)) != Some(ExecutionState::Deferred) || !journal.pending_events().is_empty() {
                return Err("deferred command journal/outbox mismatch".into());
            }
            let deferred_query = r#"$db=new PDO('sqlite:'.getenv('AGENT_HTTP_FIXTURE_DB'));echo json_encode([$db->query("SELECT status,receipt_persisted,event_sequence FROM device_agent_commands WHERE idempotency_key='deferred-restart'")->fetch(PDO::FETCH_ASSOC),(int)$db->query("SELECT COUNT(*) FROM device_agent_events WHERE command_id='99999999999999999999999999999999'")->fetchColumn()]);"#;
            let persisted = php_command().args(["-r", deferred_query]).env("AGENT_HTTP_FIXTURE_DB", &db)
                .output().map_err(|e| e.to_string())?;
            let values: serde_json::Value = serde_json::from_slice(&persisted.stdout).map_err(|e| e.to_string())?;
            if values != serde_json::json!([{"status":"deferred","receipt_persisted":1,"event_sequence":2},2]) {
                return Err(format!("deferred SQL mismatch: {values}"));
            }
            let after = recovery_windows::inspect(&state, &paths)?;
            if after.process_id != before.process_id || after.state != before.state || after.command != before.command {
                return Err("deferred command changed actual service".into());
            }
            // Model an agent restart after a helper has committed. Only the
            // disposable fixture records are changed; no installer is launched.
            use super::super::install_plan::{Artifact, InstallPlan, OriginalService, Trigger};
            use super::super::transaction::{Phase, Transaction};
            use sha2::{Digest, Sha256};
            let command_id = "8".repeat(32);
            let now = unix_timestamp();
            let mut wire = serde_json::json!({
                "schema":1,"id":command_id,"type":"package.update","type_version":1,
                "client_id":identity.id,"client_uuid":identity.uuid,
                "agent_installation_id":state.agent_installation_id,"enrollment_generation":1,
                "created_at":now,"expires_at":now+300,"timeout_seconds":900,
                "preconditions":{"active_sessions":"defer"},
                "args":{"package_id":"f".repeat(64),"target_key":"windows-x86_64-msi-standard",
                    "target_build_seq":2026100503u64,"allow_downgrade":false}
            });
            let envelope = serde_json::to_string(&wire).map_err(|e| e.to_string())?;
            let digest = hex::encode(Sha256::digest(envelope.as_bytes()));
            wire["digest"] = digest.clone().into();
            wire["status"] = "running".into();
            wire["receipt_persisted"] = true.into();
            let terminal_seed = r#"require getenv('AGENT_FIXTURE_ROOT').'/sqlite/lib.php';require getenv('AGENT_FIXTURE_ROOT').'/sqlite/agent_operations.php';$db=new PDO('sqlite:'.getenv('AGENT_HTTP_FIXTURE_DB'));configure_pdo($db);$wire=json_decode(getenv('AGENT_TERMINAL_ENVELOPE'),true);$encoded=agent_ops_json($wire);db_exec($db,'INSERT INTO device_agent_commands(command_id,device_id,uuid,installation_id,generation,type,type_version,envelope,digest,status,event_sequence,receipt_persisted,idempotency_key,request_digest,created_at,expires_at,updated_at,actor_id) VALUES(:command,:id,:uuid,:installation,1,\'package.update\',1,:envelope,:digest,\'running\',2,1,\'terminal-reconcile\',:digest,:now,:expires,:now,1)',['command'=>$wire['id'],'id'=>$wire['client_id'],'uuid'=>$wire['client_uuid'],'installation'=>$wire['agent_installation_id'],'envelope'=>$encoded,'digest'=>hash('sha256',$encoded),'now'=>$wire['created_at'],'expires'=>$wire['expires_at']]);"#;
            let seeded = php_command().args(["-r", terminal_seed])
                .env("AGENT_FIXTURE_ROOT", &api).env("AGENT_HTTP_FIXTURE_DB", &db)
                .env("AGENT_TERMINAL_ENVELOPE", &envelope).output().map_err(|e|e.to_string())?;
            if !seeded.status.success() {
                return Err(format!("terminal fixture seed: {}", String::from_utf8_lossy(&seeded.stderr)));
            }
            let mut journal = CommandJournal::open(&paths.state.join("commands-1.json"))?;
            journal.prepare(&command_id, &digest, now+300, now)?;
            journal.receipt(&command_id)?;
            journal.start(&command_id, now)?;
            for event in journal.pending_events().to_vec() {
                journal.acknowledge(&event.event_id, &event.digest)?;
            }
            drop(journal);
            let artifact = |name: &str| Artifact {relative_path:name.into(),size:10,sha256:"e".repeat(64)};
            let plan = InstallPlan {
                schema:1,transaction_id:command_id.clone(),trigger:Trigger::RemoteCommand,
                command_id:Some(command_id.clone()),installation_id:state.agent_installation_id.clone(),
                identity_fingerprint:"c".repeat(64),target_key:"windows-x86_64-msi-standard".into(),
                original_version:"1.5.3".into(),original_build_seq:2026100502,
                target_version:"1.5.4".into(),target_build_seq:2026100503,
                normal_install_path:r"C:\Program Files\RustDesk".into(),
                package:artifact("package.msi"),client_backup:artifact("backups/client.json"),
                agent_backup:artifact("backups/agent.exe"),identity_backup:artifact("backups/identity.toml"),
                helper:artifact("runtime/helper.exe"),helper_version:"1.5.3".into(),
                resume_task:format!("RustDeskManagementAgentResume-{command_id}"),created_at:now,
                services:vec![
                    OriginalService{name:"RustDesk".into(),command:r#""C:\Program Files\RustDesk\RustDesk.exe" --service"#.into(),account:"LocalSystem".into(),start_type:"auto".into(),running:true},
                    OriginalService{name:"RustDeskManagementAgent".into(),command:r#"C:\ProgramData\RustDesk\ManagementAgent\rustdesk-management-agent.exe --service --edition standard --package-kind msi --normal-install-path "C:\Program Files\RustDesk""#.into(),account:"LocalSystem".into(),start_type:"auto".into(),running:true},
                ],
            };
            let root = paths.agent.join("transactions").join(&command_id);
            std::fs::create_dir_all(root.parent().ok_or("fixture transaction parent absent")?).map_err(|e|e.to_string())?;
            workspace::create(&root)?;
            let plan_bytes = serde_json::to_vec(&plan).map_err(|e|e.to_string())?;
            let mut transaction = Transaction::new(command_id.clone(),plan.target_key.clone(),plan.package.sha256.clone(),plan.identity_fingerprint.clone(),now)?;
            transaction.phase = Phase::Committed;
            transaction.revision = 7;
            let mut receipt = serde_json::json!({"schema":1,"command_id":command_id,"command_digest":digest,
                "command_type":"package.update","client_id":identity.id,"client_uuid":identity.uuid,
                "installation_id":state.agent_installation_id,"enrollment_generation":2,
                "package_id":wire["args"]["package_id"],"plan_sha256":hex::encode(Sha256::digest(&plan_bytes))});
            for (name, bytes) in [
                ("prepared.json",plan_bytes),
                ("transaction.json",serde_json::to_vec(&transaction).map_err(|e|e.to_string())?),
                ("execution-result.json",serde_json::to_vec(&serde_json::json!({"schema":1,"transaction_id":command_id,
                    "command_id":command_id,"phase":"committed","error":null,"reboot_required":false})).map_err(|e|e.to_string())?),
                ("command-binding.json",serde_json::to_vec(&receipt).map_err(|e|e.to_string())?),
            ] { std::fs::write(root.join(name),bytes).map_err(|e|e.to_string())?; }
            let result_before = std::fs::read(root.join("execution-result.json")).map_err(|e|e.to_string())?;
            poll_with_identity(&identity, &format!("http://{address}"), &blocked_state, &paths)?;
            if CommandJournal::open(&paths.state.join("commands-1.json"))?.state(&command_id) != Some(ExecutionState::ReconciliationRequired) {
                return Err("foreign-generation receipt authorized terminal result".into());
            }
            receipt["enrollment_generation"] = 1.into();
            std::fs::write(root.join("command-binding.json"),serde_json::to_vec(&receipt).map_err(|e|e.to_string())?).map_err(|e|e.to_string())?;
            for _ in 0..2 { poll_with_identity(&identity, &format!("http://{address}"), &blocked_state, &paths)?; }
            let journal = CommandJournal::open(&paths.state.join("commands-1.json"))?;
            if journal.state(&command_id) != Some(ExecutionState::Succeeded) || !journal.pending_events().is_empty() {
                return Err("terminal reconciliation not durably acknowledged".into());
            }
            let terminal_query = r#"$db=new PDO('sqlite:'.getenv('AGENT_HTTP_FIXTURE_DB'));echo json_encode([$db->query("SELECT status,event_sequence FROM device_agent_commands WHERE idempotency_key='terminal-reconcile'")->fetch(PDO::FETCH_ASSOC),$db->query("SELECT status FROM device_agent_events WHERE command_id='88888888888888888888888888888888' ORDER BY sequence")->fetchAll(PDO::FETCH_COLUMN)]);"#;
            let persisted = php_command().args(["-r",terminal_query]).env("AGENT_HTTP_FIXTURE_DB",&db).output().map_err(|e|e.to_string())?;
            let values:serde_json::Value = serde_json::from_slice(&persisted.stdout).map_err(|e|e.to_string())?;
            if values != serde_json::json!([{"status":"succeeded","event_sequence":4},["reconciliation_required","succeeded"]]) {
                return Err(format!("terminal reconciliation SQL mismatch: {values}"));
            }
            if std::fs::read(root.join("execution-result.json")).map_err(|e|e.to_string())? != result_before {
                return Err("result reconciliation modified helper evidence".into());
            }
            let after = recovery_windows::inspect(&state, &paths)?;
            if after.process_id != before.process_id || after.state != before.state || after.command != before.command {
                return Err("terminal reconciliation changed actual service".into());
            }
            let package_seed = r#"require getenv('AGENT_FIXTURE_ROOT').'/sqlite/lib.php';require getenv('AGENT_FIXTURE_ROOT').'/sqlite/agent_status.php';require getenv('AGENT_FIXTURE_ROOT').'/sqlite/agent_operations.php';$db=new PDO('sqlite:'.getenv('AGENT_HTTP_FIXTURE_DB'));configure_pdo($db);$now=time();$manifest=json_decode(getenv('AGENT_REFERENCE_MANIFEST'),true,64,JSON_THROW_ON_ERROR);$key='windows-x86_64-msi-standard';$rollback=$manifest;$rollback['targets'][$key]['build_seq']=2026100502;foreach([$manifest,$rollback] as $release)db_insert_ignore($db,'update_releases',['version'=>'1.5.4','build_seq'=>$release['targets'][$key]['build_seq'],'channel'=>'stable','manifest'=>agent_ops_json($release),'active'=>1,'published_at'=>$now]);$payload=['schema'=>1,'client_id'=>'test-device','client_uuid'=>'test-uuid','agent_installation_id'=>str_repeat('a',32),'boot_id'=>str_repeat('b',32),'session_generation'=>1,'sequence'=>1,'agent'=>['version'=>'1.5.3','build_seq'=>2026100502],'capabilities'=>['agent.status@1','package.update@1'],'package_identity'=>['edition'=>'standard','arch'=>'x86_64','kind'=>'msi'],'service'=>['state'=>'running'],'application'=>['state'=>'ready'],'recovery'=>['effective_enabled'=>true]];agent_status_store($db,$payload,$now);echo agent_ops_json(agent_command_create($db,['id'=>1,'address_book_scope'=>'all'],'test-device','test-uuid',['type'=>'package.update','type_version'=>1,'idempotency_key'=>'package-resolver-rejection','args'=>['package_id'=>agent_package_reference($manifest,$key),'target_key'=>$key,'target_build_seq'=>2026100503,'allow_downgrade'=>false]],$now));"#;
            let seeded = php_command().args(["-r",package_seed]).env("AGENT_FIXTURE_ROOT",&api)
                .env("AGENT_HTTP_FIXTURE_DB",&db).env("AGENT_REFERENCE_MANIFEST",serde_json::to_string(&manifest).map_err(|e|e.to_string())?)
                .output().map_err(|e|e.to_string())?;
            if !seeded.status.success() { return Err(format!("package fixture seed: {}",String::from_utf8_lossy(&seeded.stderr))); }
            let package_wire: serde_json::Value = serde_json::from_slice(&seeded.stdout).map_err(|e|e.to_string())?;
            let command = operations::command_envelope(&package_wire,&identity.id,&identity.uuid,&state.agent_installation_id,1)?;
            let mut package_state = state.clone();
            package_state.package_kind = super::super::PackageKind::Msi;
            let before_records = |sql: &str| -> Result<Vec<u8>,String> {
                let query = php_command().args(["-r",sql]).env("AGENT_HTTP_FIXTURE_DB",&db).output().map_err(|e|e.to_string())?;
                if !query.status.success() { return Err(String::from_utf8_lossy(&query.stderr).into_owned()); }
                Ok(query.stdout)
            };
            let evidence_sql = r#"$db=new PDO('sqlite:'.getenv('AGENT_HTTP_FIXTURE_DB'));echo json_encode([$db->query('SELECT * FROM device_agent_commands ORDER BY command_id')->fetchAll(PDO::FETCH_ASSOC),$db->query('SELECT * FROM device_agent_events ORDER BY event_id')->fetchAll(PDO::FETCH_ASSOC),$db->query('SELECT last_seen,last_heartbeat FROM device_reports')->fetchAll(PDO::FETCH_ASSOC)]);"#;
            let preserved = before_records(evidence_sql)?;
            for (update, expected_http) in [
                ("UPDATE device_agent_bindings SET permissions='[\"observe\"]'", Some(403)),
                ("UPDATE device_agent_bindings SET permissions='[\"observe\",\"install\"]'; UPDATE update_releases SET active=0 WHERE build_seq=2026100502", Some(409)),
                ("UPDATE update_releases SET active=1", None),
            ] {
                let sql = format!(r#"$db=new PDO('sqlite:'.getenv('AGENT_HTTP_FIXTURE_DB'));$db->exec({});"#,serde_json::to_string(update).map_err(|e|e.to_string())?);
                before_records(&sql)?;
                let error = match resolve_approved_package(&identity,&format!("http://{address}"),&binding,&package_state,&command,&paths) {
                    Err(error) => error,
                    Ok(_) => return Err("unapproved package unexpectedly resolved".into()),
                };
                if let Some(code) = expected_http {
                    if !matches!(error.failure, network_policy::Failure::Http(actual) if actual == code) {
                        return Err(format!("package resolver expected HTTP {code}: {error}"));
                    }
                } else if !error.to_string().to_ascii_lowercase().contains("signature") {
                    return Err(format!("package resolver expected signature refusal: {error}"));
                }
                if paths.agent.join(format!("approved-{}",command.id)).exists()
                    || paths.agent.join(format!("rollback-{}",command.id)).exists()
                    || paths.agent.join("transactions").join(&command.id).exists()
                    || before_records(evidence_sql)? != preserved {
                    return Err("package rejection changed cache, transaction or database evidence".into());
                }
            }
            let after = recovery_windows::inspect(&state,&paths)?;
            if after.process_id != before.process_id || after.state != before.state || after.command != before.command {
                return Err("package rejection changed installed service".into());
            }
            Ok(())
        })();
        server.kill().unwrap();
        server.wait().unwrap();
        std::fs::remove_dir_all(&directory).unwrap();
        result.unwrap();
    }
}
