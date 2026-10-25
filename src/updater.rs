#[cfg(target_os = "windows")]
#[path = "updater/ordinary_update_receipts.rs"]
#[allow(dead_code)]
pub(crate) mod ordinary_update_receipts;

#[path = "updater/install_state.rs"]
mod install_state;

use crate::{
    common::{
        do_check_software_update, do_check_software_update_with_context,
        do_check_software_update_with_context_result,
    },
    hbbs_http::create_http_client_with_url_strict,
};
#[cfg(target_os = "linux")]
use base::update::replace_file_transaction;
use base::{
    config::keys,
    update::{
        classify_update_preinstall_failure, decide_update_action,
        normalize_scheduled_update_interval_hours, parse_update_stream_sse_event,
        policy_resume_revision, should_run_scheduled_update, should_run_startup_update,
        try_update_sources, update_client_identity, update_option_enabled,
        update_policy_stream_url, update_signature_payload, update_target_key,
        update_target_release, validate_manifest_contract, validate_target_metadata,
        PendingUpdateEvent, PolicyDecision, ProcessedUpdateCommands, UpdateAction, UpdateCommand,
        UpdateCommandAction, UpdateCommandDecision, UpdateCommandState, UpdatePolicy, UpdateSource,
        UpdateStreamEvent, DEFAULT_SCHEDULED_UPDATE_INTERVAL_HOURS,
    },
};
use hbb_common::base64::{
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
    Engine as _,
};
use hbb_common::{bail, config, log, ResultType};
use sha2::{Digest, Sha256};
use std::{
    fs::{File, OpenOptions},
    io::{BufRead, BufReader, Write},
    path::{Component, Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc::{channel, Receiver, Sender},
        Mutex,
    },
    time::{Duration, Instant},
};

#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};

#[cfg(target_os = "linux")]
use std::os::unix::fs::PermissionsExt;

#[cfg(target_os = "macos")]
use std::os::unix::fs::MetadataExt;

enum UpdateMsg {
    CheckUpdate,
    /// 桌面用户显式点了「立即安装更新」。它与后台/定时检查的区别是：这一次
    /// 允许真的安装，而被动检查只按 mode 决定是否提示或下载。
    InstallUpdate,
    ConnectivityRestored,
    ScheduleChanged,
    Command(UpdateCommand),
    Exit,
}

lazy_static::lazy_static! {
    static ref TX_MSG : Mutex<Sender<UpdateMsg>> = Mutex::new(start_auto_update_check());
}

#[cfg(target_os = "macos")]
lazy_static::lazy_static! {
    static ref MAC_SCHEDULER_WAKE: Mutex<Option<Sender<()>>> = Mutex::new(None);
}

static CONTROLLING_SESSION_COUNT: AtomicUsize = AtomicUsize::new(0);
static POLICY_STREAM_STARTED: AtomicBool = AtomicBool::new(false);

// The origin of the update request currently being served, so the event pushed
// to the local UI can say whether the person at the desk asked for it. Checks
// run one at a time on the update loop, so a single slot is enough; the value is
// only read while building the event for that same check.
lazy_static::lazy_static! {
    static ref CURRENT_UPDATE_ORIGIN: Mutex<String> = Mutex::new(String::new());
}

fn set_update_request_origin(origin: &str) {
    if let Ok(mut slot) = CURRENT_UPDATE_ORIGIN.lock() {
        slot.clear();
        slot.push_str(origin);
    }
}

// Only the local UI event reads this, and that path is built with `flutter` alone.
#[cfg_attr(not(feature = "flutter"), allow(dead_code))]
fn update_request_origin() -> String {
    CURRENT_UPDATE_ORIGIN
        .lock()
        .map(|slot| slot.clone())
        .unwrap_or_default()
}

/// Initial wait after startup before the first update check (30 seconds).
pub const INITIAL_CHECK_DELAY: Duration = Duration::from_secs(30);

/// Minimum interval between consecutive update checks (10 minutes).
pub const MIN_INTERVAL: Duration = Duration::from_secs(60 * 10);

/// Retry interval when an update check fails or a session is active (30 minutes).
pub const RETRY_INTERVAL: Duration = Duration::from_secs(60 * 30);
pub const POLICY_EOF_RECONNECT_DELAY: Duration = Duration::from_secs(1);
pub const POLICY_ERROR_RECONNECT_DELAY: Duration = Duration::from_secs(2);
const UPDATE_COMMAND_RETRY_INTERVAL: Duration = Duration::from_secs(30);
const DEFERRED_UPDATE_RETRY_INTERVAL: Duration = Duration::from_secs(30);

pub(crate) struct UpdateDeviceAuthHeaders {
    pub device_id: String,
    pub public_key: String,
    pub timestamp: String,
    pub nonce: String,
    pub signature: String,
}

pub(crate) fn update_device_auth_headers(
    method: &str,
    url: &str,
    identity: &base::update::UpdateClientIdentity,
    command_id: &str,
    body: &[u8],
) -> ResultType<UpdateDeviceAuthHeaders> {
    let path = url::Url::parse(url)?.path().to_owned();
    let timestamp = chrono::Utc::now().timestamp().to_string();
    let nonce = hex::encode(hbb_common::sodiumoxide::randombytes::randombytes(16));
    let body_sha256 = hex::encode(Sha256::digest(body));
    let canonical = base::update::update_device_auth_payload(
        method,
        &path,
        &identity.client_id,
        &identity.client_uuid,
        timestamp.parse()?,
        &nonce,
        command_id,
        &body_sha256,
    );
    let (secret_key, public_key) = config::Config::get_key_pair();
    let secret_key = hbb_common::sodiumoxide::crypto::sign::SecretKey::from_slice(&secret_key)
        .ok_or_else(|| hbb_common::anyhow::anyhow!("invalid device signing key"))?;
    let signature = hbb_common::sodiumoxide::crypto::sign::sign_detached(&canonical, &secret_key);
    Ok(UpdateDeviceAuthHeaders {
        device_id: config::Config::get_id(),
        public_key: STANDARD.encode(public_key),
        timestamp,
        nonce,
        signature: STANDARD.encode(signature.to_bytes()),
    })
}

pub fn update_controlling_session_count(count: usize) {
    CONTROLLING_SESSION_COUNT.store(count, Ordering::SeqCst);
}

#[allow(dead_code)]
pub fn start_auto_update() {
    if !background_updater_runs_here() {
        return;
    }
    #[cfg(target_os = "windows")]
    ordinary_update_receipts::runtime::start();
    start_update_policy_stream();
    let _sender = TX_MSG.lock().unwrap();
}

pub fn update_schedule_changed() {
    let sender = TX_MSG.lock().unwrap();
    let _ = sender.send(UpdateMsg::ScheduleChanged);
    #[cfg(target_os = "macos")]
    if let Some(sender) = MAC_SCHEDULER_WAKE.lock().unwrap().as_ref() {
        let _ = sender.send(());
    }
}

pub fn start_update_policy_stream() {
    if POLICY_STREAM_STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    if let Err(err) = std::thread::Builder::new()
        .name("rustdesk-update-policy".to_owned())
        .spawn(update_policy_stream_loop)
    {
        POLICY_STREAM_STARTED.store(false, Ordering::SeqCst);
        log::warn!("Failed to start update policy stream: {err}");
    }
}

fn applied_policy_revision(identity: &base::update::UpdateClientIdentity) -> Option<u64> {
    let stored_uuid = config::Config::get_option(keys::OPTION_UPDATE_POLICY_CLIENT_UUID);
    let revision = config::Config::get_option(keys::OPTION_UPDATE_POLICY_REVISION)
        .parse()
        .ok();
    let revision = policy_resume_revision(&stored_uuid, &identity.client_uuid, revision);
    if revision.is_none() && stored_uuid != identity.client_uuid {
        config::Config::set_option(
            keys::OPTION_UPDATE_POLICY_REVISION.to_owned(),
            String::new(),
        );
        config::Config::set_option(
            keys::OPTION_UPDATE_POLICY_CLIENT_UUID.to_owned(),
            identity.client_uuid.clone(),
        );
    }
    revision
}

fn current_update_client_identity() -> base::update::UpdateClientIdentity {
    update_client_identity(
        &hbb_common::config::Config::get_id(),
        &crate::encode64(hbb_common::get_uuid()),
    )
}

fn apply_update_policy(policy: UpdatePolicy) {
    let identity = current_update_client_identity();
    if policy.decision(applied_policy_revision(&identity)) == PolicyDecision::IgnoreStale {
        return;
    }
    let check_value = if policy.check_on_startup { "Y" } else { "N" };
    let auto_value = if policy.auto_update { "Y" } else { "N" };
    let scheduled_value = if policy.scheduled_update { "Y" } else { "N" };
    let scheduled_hours =
        normalize_scheduled_update_interval_hours(policy.scheduled_update_interval_hours);
    config::Config::set_option(
        keys::OPTION_ENABLE_CHECK_UPDATE.to_owned(),
        check_value.to_owned(),
    );
    config::Config::set_option(
        keys::OPTION_ALLOW_AUTO_UPDATE.to_owned(),
        auto_value.to_owned(),
    );
    config::Config::set_option(
        keys::OPTION_ENABLE_SCHEDULED_UPDATE.to_owned(),
        scheduled_value.to_owned(),
    );
    config::Config::set_option(
        keys::OPTION_SCHEDULED_UPDATE_INTERVAL_HOURS.to_owned(),
        scheduled_hours.to_string(),
    );
    config::Config::set_option(
        keys::OPTION_UPDATE_POLICY_REVISION.to_owned(),
        policy.revision.to_string(),
    );
    config::Config::set_option(
        keys::OPTION_UPDATE_POLICY_CLIENT_UUID.to_owned(),
        policy.client_uuid.clone(),
    );
    config::Status::set("sysinfo_hash", String::new());
    crate::ui_interface::refresh_options();
    #[cfg(feature = "flutter")]
    {
        let event = serde_json::json!({
            "name": "update_policy_changed",
            "enable_check_update": policy.check_on_startup,
            "allow_auto_update": policy.auto_update,
            "enable_scheduled_update": policy.scheduled_update,
            "scheduled_update_interval_hours": scheduled_hours,
            "management_agent_enabled": policy.management_agent_enabled,
            "policy_revision": policy.revision,
        });
        let _ = crate::flutter::push_global_event(crate::flutter::APP_TYPE_MAIN, event.to_string());
    }
    update_schedule_changed();
}

fn notify_update_connectivity_restored() {
    let sender = TX_MSG.lock().unwrap();
    let _ = sender.send(UpdateMsg::ConnectivityRestored);
}

/// 发布广播走和「连接恢复」同一条后台检查通道。它绝不能折成 `manually=true`
/// 的手动检查，否则 `mode=notify` 的机器会被广播直接推进安装。
fn on_release_published(notice: base::update::ReleaseNotice) {
    if !release_notice_should_check(&notice, crate::BUILD_SEQ, background_update_enabled()) {
        return;
    }
    log::debug!(
        "Release broadcast {} ({}) is newer than the running build",
        notice.version,
        notice.build_seq
    );
    notify_update_connectivity_restored();
}

fn update_policy_stream_loop() {
    const POLICY_BASE_URL: &str = "https://rdapi.yan.life";
    let mut connection_seen = false;
    loop {
        let identity = current_update_client_identity();
        let revision = applied_policy_revision(&identity);
        let notify_on_connect = connection_seen;
        let result = read_update_policy_stream_once(
            POLICY_BASE_URL,
            &identity,
            revision,
            || {
                if notify_on_connect {
                    notify_update_connectivity_restored();
                }
            },
            apply_update_policy,
            enqueue_update_command,
        );
        if result.is_ok() {
            connection_seen = true;
        }
        let reconnect_delay = match result {
            Ok(()) => POLICY_EOF_RECONNECT_DELAY,
            Err(err) => {
                log::trace!("Update policy stream disconnected: {err}");
                POLICY_ERROR_RECONNECT_DELAY
            }
        };
        std::thread::sleep(reconnect_delay);
    }
}

fn read_update_policy_stream_once(
    base_url: &str,
    identity: &base::update::UpdateClientIdentity,
    revision: Option<u64>,
    mut on_connected: impl FnMut(),
    mut on_policy: impl FnMut(UpdatePolicy),
    mut on_command: impl FnMut(UpdateCommand),
) -> ResultType<()> {
    let url = update_policy_stream_url(base_url, identity, revision);
    let client = if url.starts_with("https://") {
        create_http_client_with_url_strict(&url)?
    } else {
        #[cfg(test)]
        {
            reqwest::blocking::Client::new()
        }
        #[cfg(not(test))]
        {
            bail!("update policy stream requires HTTPS")
        }
    };
    let send_stream = |authenticated: bool| -> ResultType<reqwest::blocking::Response> {
        let mut request = client.get(&url).header("Accept", "text/event-stream");
        if authenticated {
            let auth = update_device_auth_headers("GET", &url, identity, "", &[])?;
            request = request
                .header("X-RustDesk-Device-ID", auth.device_id)
                .header("X-RustDesk-Device-Public-Key", auth.public_key)
                .header("X-RustDesk-Device-Timestamp", auth.timestamp)
                .header("X-RustDesk-Device-Nonce", auth.nonce)
                .header("X-RustDesk-Device-Signature", auth.signature);
        }
        if let Some(revision) = revision {
            request = request.header("Last-Event-ID", revision.to_string());
        }
        Ok(request.send()?)
    };
    let response = send_stream(true)?;
    let (response, authenticated) = if response.status() == reqwest::StatusCode::UNAUTHORIZED {
        (send_stream(false)?, false)
    } else {
        (response, true)
    };
    let response = response.error_for_status()?;
    on_connected();
    let mut reader = BufReader::new(response);
    let mut event = String::new();
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            break;
        }
        event.push_str(&line);
        if line == "\n" || line == "\r\n" {
            match parse_update_stream_sse_event(&event)? {
                Some(UpdateStreamEvent::Policy(policy)) => {
                    if policy.matches_identity(identity) {
                        on_policy(policy);
                    } else {
                        log::warn!("Ignored update policy for a different client identity");
                    }
                }
                Some(UpdateStreamEvent::Command(command)) if authenticated => on_command(command),
                Some(UpdateStreamEvent::Command(_)) => {
                    log::warn!("Ignored update command from unauthenticated policy stream");
                }
                Some(UpdateStreamEvent::ReleasePublished(notice)) => on_release_published(notice),
                None => {}
            }
            event.clear();
        }
    }
    Ok(())
}

fn update_command_should_defer(action: UpdateCommandAction, has_active_session: bool) -> bool {
    action == UpdateCommandAction::Install && has_active_session
}

fn update_command_matches_target(
    command_version: &str,
    command_build_seq: u64,
    manifest_version: &str,
    manifest_build_seq: u64,
) -> bool {
    command_version == manifest_version && command_build_seq == manifest_build_seq
}

// The server's `mode` is the authority for whether an update may install itself.
// A manual check only decides to run the check now, so it must not turn `notify`
// into an install. The person at the desk asking to install is a separate,
// explicit request, and it only lifts `download` to an install: `notify` still
// never installs, and `disabled` still installs nothing at all.
fn update_action_for(
    response: &hbb_common::VersionCheckResponse,
    local_auto_update: bool,
    command: Option<&UpdateCommand>,
    install_requested: bool,
) -> UpdateAction {
    if let Some(command) = command {
        return match command.action {
            UpdateCommandAction::Check => UpdateAction::Notify,
            UpdateCommandAction::Install => UpdateAction::AutoInstall,
        };
    }
    let action = decide_update_action(response, local_auto_update);
    if install_requested && action == UpdateAction::Download {
        UpdateAction::AutoInstall
    } else {
        action
    }
}

fn update_now_unix() -> i64 {
    chrono::Utc::now().timestamp()
}

// A launched installer that exits without replacing this build used to look like
// a success, so a build that crashes on start was reinstalled forever. The
// attempt is now recorded before the launch and confirmed after the restart.
fn note_install_attempt(automatic: bool, version: &str, build_seq: u64) {
    let now = update_now_unix();
    if automatic {
        install_state::record_auto_install_attempt(version, build_seq, now);
    }
}

// Called once per process before the update loop reads pending commands. The
// installer terminates this process while it replaces the files, so the only
// place left to decide whether it worked is the next start.
fn resolve_pending_install() {
    if std::env::args().any(|arg| arg == "--update") {
        return;
    }
    let Some(pending) = install_state::load().pending else {
        return;
    };
    let event = pending.event;
    let confirmation = install_state::install_confirmation(
        &event.version,
        event.build_seq,
        crate::VERSION,
        crate::BUILD_SEQ,
        update_now_unix(),
        pending.deadline_unix,
    );
    match confirmation {
        install_state::InstallConfirmation::Pending => {}
        install_state::InstallConfirmation::Confirmed => {
            log::info!(
                "Previous update to {} is confirmed by the running build",
                event.version
            );
            install_state::clear_pending(&event.transaction_id);
            install_state::clear_auto_install_attempts();
            report_update_event(
                "installed",
                &event.version,
                event.build_seq,
                event.source.as_str(),
            );
        }
        install_state::InstallConfirmation::Failed => {
            log::warn!(
                "Previous update to {} never took effect; the running build is still {}",
                event.version,
                crate::VERSION
            );
            install_state::clear_pending(&event.transaction_id);
            report_update_event(
                "failed",
                &event.version,
                event.build_seq,
                event.source.as_str(),
            );
        }
    }
}

// The installer normally kills this process within seconds. If it is still alive
// after the confirmation window, the installer exited without replacing the
// build, and the update counts as failed instead of started.
#[cfg(target_os = "windows")]
fn spawn_install_confirmation_watchdog(event: PendingUpdateEvent) {
    let deadline =
        update_now_unix().saturating_add(install_state::INSTALL_CONFIRM_TIMEOUT.as_secs() as i64);
    let spawned = std::thread::Builder::new()
        .name("update-install-confirm".to_owned())
        .spawn(move || {
            std::thread::sleep(install_state::INSTALL_CONFIRM_TIMEOUT);
            let still_old = install_state::install_confirmation(
                &event.version,
                event.build_seq,
                crate::VERSION,
                crate::BUILD_SEQ,
                update_now_unix(),
                deadline,
            ) == install_state::InstallConfirmation::Failed;
            if !still_old {
                return;
            }
            log::error!(
                "Update to {} did not replace the running build {}",
                event.version,
                crate::VERSION
            );
            install_state::clear_pending(&event.transaction_id);
            report_update_event(
                "failed",
                &event.version,
                event.build_seq,
                event.source.as_str(),
            );
        });
    if let Err(err) = spawned {
        log::warn!("Failed to start install confirmation watchdog: {err}");
    }
}

#[derive(Debug, PartialEq, Eq)]
enum UpdateCommandRunOutcome {
    NoUpdate,
    CheckCompleted,
    Deferred,
    AwaitingInstallResult,
    Installed,
    Refused,
    /// 同一目标版本已连续自动尝试到上限，暂不再拉起安装器。
    Suspended,
}

fn load_pending_update_commands() -> Vec<UpdateCommandState> {
    let value = config::Config::get_option(keys::OPTION_PENDING_UPDATE_COMMANDS);
    if value.is_empty() {
        return Vec::new();
    }
    match serde_json::from_str(&value) {
        Ok(commands) => commands,
        Err(err) => match serde_json::from_str::<Vec<UpdateCommand>>(&value) {
            Ok(commands) => commands
                .into_iter()
                .map(UpdateCommandState::pending)
                .collect(),
            Err(_) => {
                log::warn!("Failed to load pending update commands: {err}");
                Vec::new()
            }
        },
    }
}

fn store_pending_update_commands(commands: &[UpdateCommandState]) -> ResultType<()> {
    let value = serde_json::to_string(commands)?;
    config::Config::set_option(keys::OPTION_PENDING_UPDATE_COMMANDS.to_owned(), value);
    Ok(())
}

fn load_processed_update_commands() -> ProcessedUpdateCommands {
    let value = config::Config::get_option(keys::OPTION_PROCESSED_UPDATE_COMMANDS);
    if value.is_empty() {
        return ProcessedUpdateCommands::default();
    }
    serde_json::from_str(&value).unwrap_or_else(|_| {
        let mut history = ProcessedUpdateCommands::default();
        for command_id in value.split(',').filter(|value| !value.is_empty()) {
            history.remember(command_id.to_owned());
        }
        history
    })
}

fn store_processed_update_commands(history: &ProcessedUpdateCommands) -> ResultType<()> {
    config::Config::set_option(
        keys::OPTION_PROCESSED_UPDATE_COMMANDS.to_owned(),
        serde_json::to_string(history)?,
    );
    Ok(())
}

fn enqueue_update_command(command: UpdateCommand) {
    let sender = TX_MSG.lock().unwrap().clone();
    let identity = current_update_client_identity();
    let history = load_processed_update_commands();
    let mut pending = load_pending_update_commands();
    let already_seen = history.contains(&command.command_id)
        || pending
            .iter()
            .any(|value| value.command.command_id == command.command_id);
    match command.decision(&identity, chrono::Utc::now().timestamp(), already_seen) {
        UpdateCommandDecision::Execute => {
            pending.push(UpdateCommandState::pending(command.clone()));
            if let Err(err) = store_pending_update_commands(&pending) {
                log::error!("Failed to persist update command: {err}");
                report_update_command_event(&command, "failed", "persistence_failed");
                return;
            }
            report_update_command_event(&command, "accepted", "");
            if let Err(err) = sender.send(UpdateMsg::Command(command)) {
                log::warn!("Failed to enqueue update command: {err}");
            }
        }
        UpdateCommandDecision::Expired => {
            report_update_command_event(&command, "expired", "expired");
        }
        UpdateCommandDecision::IdentityMismatch => {
            log::warn!("Ignored update command for a different client identity");
        }
        UpdateCommandDecision::Duplicate => {
            if pending.iter().any(|state| {
                state.command.command_id == command.command_id && state.terminal.is_some()
            }) {
                retry_update_command_terminal(&command.command_id);
            }
        }
    }
}

fn complete_update_command(command: &UpdateCommand) {
    let mut history = load_processed_update_commands();
    history.remember(command.command_id.clone());
    if let Err(err) = store_processed_update_commands(&history) {
        log::error!("Failed to persist processed update command: {err}");
        return;
    }
    let mut pending = load_pending_update_commands();
    pending.retain(|value| value.command.command_id != command.command_id);
    if let Err(err) = store_pending_update_commands(&pending) {
        log::error!("Failed to persist completed update command: {err}");
    }
}

fn persist_and_report_update_command_terminal(
    command: &UpdateCommand,
    status: &str,
    error_code: &str,
) -> bool {
    let mut pending = load_pending_update_commands();
    let Some(state) = pending
        .iter_mut()
        .find(|state| state.command.command_id == command.command_id)
    else {
        log::warn!("Missing pending update command {}", command.command_id);
        return false;
    };
    state.set_terminal(status, error_code);
    if let Err(err) = store_pending_update_commands(&pending) {
        log::error!("Failed to persist update command terminal state: {err}");
        return false;
    }
    if report_update_command_event(command, status, error_code) {
        complete_update_command(command);
        true
    } else {
        schedule_update_command_terminal_retry(command.command_id.clone());
        false
    }
}

fn retry_update_command_terminal(command_id: &str) {
    let Some(state) = load_pending_update_commands()
        .into_iter()
        .find(|state| state.command.command_id == command_id)
    else {
        return;
    };
    if let Some(terminal) = state.terminal {
        if report_update_command_event(&state.command, &terminal.status, &terminal.error_code) {
            complete_update_command(&state.command);
        } else {
            schedule_update_command_terminal_retry(state.command.command_id);
        }
    }
}

fn report_update_command_deferred_once(command: &UpdateCommand) {
    let mut pending = load_pending_update_commands();
    let Some(state) = pending
        .iter_mut()
        .find(|state| state.command.command_id == command.command_id)
    else {
        return;
    };
    if !state.mark_deferred_reported() {
        return;
    }
    if let Err(err) = store_pending_update_commands(&pending) {
        log::error!("Failed to persist deferred update command state: {err}");
        return;
    }
    report_update_command_event(command, "deferred", "active_session");
}

fn schedule_update_command_terminal_retry(command_id: String) {
    std::thread::spawn(move || {
        std::thread::sleep(UPDATE_COMMAND_RETRY_INTERVAL);
        retry_update_command_terminal(&command_id);
    });
}

#[cfg(test)]
mod policy_stream_tests {
    use super::*;
    use std::{
        io::{Read, Write},
        net::TcpListener,
        sync::{
            atomic::{AtomicBool, Ordering},
            mpsc, Arc,
        },
        thread,
    };

    #[test]
    fn policy_stream_sends_resume_state_and_delivers_real_sse() {
        assert!(POLICY_EOF_RECONNECT_DELAY <= Duration::from_secs(1));
        assert!(POLICY_ERROR_RECONNECT_DELAY <= Duration::from_secs(2));
        let listener = TcpListener::bind("127.0.0.1:0").expect("fixture should bind");
        let address = listener.local_addr().expect("fixture should have address");
        let fixture = thread::spawn(move || {
            let (mut socket, _) = listener.accept().expect("client should connect");
            let mut request = [0_u8; 4096];
            let size = socket.read(&mut request).expect("request should read");
            let request = String::from_utf8_lossy(&request[..size]).into_owned();
            let body = concat!(
                "event: update-policy\n",
                "id: 4\n",
                "data: {\"client_id\":\"83077683\",\"client_uuid\":\"01ab\",",
                "\"policy_revision\":4,\"enable_check_update\":true,",
                "\"allow_auto_update\":false}\n\n"
            );
            write!(
                socket,
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            )
            .expect("response should write");
            request
        });
        let identity = update_client_identity("83077683", "01ab");
        let (tx, rx) = mpsc::channel();

        read_update_policy_stream_once(
            &format!("http://{address}"),
            &identity,
            Some(3),
            || {},
            |policy| tx.send(policy).expect("policy should send"),
            |_| {},
        )
        .expect("stream should complete");

        let request = fixture.join().expect("fixture should finish");
        assert!(request.contains("GET /rd/update/v1/policy/stream?client_id=83077683&client_uuid=01ab&after_revision=3 HTTP/1.1"));
        assert!(request.to_ascii_lowercase().contains("last-event-id: 3"));
        assert_eq!(rx.recv().expect("policy should arrive").revision, 4);
    }

    #[test]
    fn fresh_policy_stream_omits_resume_query_and_header() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("fixture should bind");
        let address = listener.local_addr().expect("fixture should have address");
        let fixture = thread::spawn(move || {
            let (mut socket, _) = listener.accept().expect("client should connect");
            let mut request = [0_u8; 4096];
            let size = socket.read(&mut request).expect("request should read");
            let request = String::from_utf8_lossy(&request[..size]).into_owned();
            let body = ": heartbeat\n\n";
            write!(
                socket,
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            )
            .expect("response should write");
            request
        });
        let identity = update_client_identity("83077683", "01ab");
        let connected = Arc::new(AtomicBool::new(false));
        let connected_for_callback = Arc::clone(&connected);

        read_update_policy_stream_once(
            &format!("http://{address}"),
            &identity,
            None,
            move || connected_for_callback.store(true, Ordering::SeqCst),
            |_| {},
            |_| {},
        )
        .expect("stream should complete");

        let request = fixture.join().expect("fixture should finish");
        assert!(request.contains(
            "GET /rd/update/v1/policy/stream?client_id=83077683&client_uuid=01ab HTTP/1.1"
        ));
        assert!(!request.to_ascii_lowercase().contains("last-event-id:"));
        assert!(!request.contains("after_revision="));
        assert!(connected.load(Ordering::SeqCst));
    }

    #[test]
    fn policy_stream_retries_unsigned_when_device_key_is_not_registered() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("fixture should bind");
        let address = listener.local_addr().expect("fixture should have address");
        let fixture = thread::spawn(move || {
            let mut requests = Vec::new();
            for status in [401, 200] {
                let (mut socket, _) = listener.accept().expect("client should connect");
                let mut request = [0_u8; 4096];
                let size = socket.read(&mut request).expect("request should read");
                requests.push(String::from_utf8_lossy(&request[..size]).into_owned());
                let body = if status == 200 {
                    concat!(
                        "event: update-policy\n",
                        "id: 1\n",
                        "data: {\"client_id\":\"83077683\",\"client_uuid\":\"01ab\",",
                        "\"policy_revision\":1,\"enable_check_update\":true,",
                        "\"allow_auto_update\":false}\n\n",
                        "event: update-command\n",
                        "id: unsigned-command\n",
                        "data: {\"command_id\":\"unsigned-command\",\"action\":\"check\",",
                        "\"client_id\":\"83077683\",\"client_uuid\":\"01ab\",",
                        "\"expires_at\":4102444800}\n\n"
                    )
                } else {
                    ""
                };
                write!(
                    socket,
                    "HTTP/1.1 {status} {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    if status == 200 { "OK" } else { "Unauthorized" },
                    body.len(),
                    body
                )
                .expect("response should write");
            }
            requests
        });
        let identity = update_client_identity("83077683", "01ab");
        let (tx, rx) = mpsc::channel();
        let (command_tx, command_rx) = mpsc::channel();

        read_update_policy_stream_once(
            &format!("http://{address}"),
            &identity,
            None,
            || {},
            |policy| tx.send(policy).expect("policy should send"),
            |command| command_tx.send(command).expect("command should send"),
        )
        .expect("unsigned fallback should complete");

        let requests = fixture.join().expect("fixture should finish");
        assert!(requests[0]
            .to_ascii_lowercase()
            .contains("x-rustdesk-device-signature:"));
        assert!(!requests[1]
            .to_ascii_lowercase()
            .contains("x-rustdesk-device-signature:"));
        assert_eq!(rx.recv().expect("policy should arrive").revision, 1);
        assert!(command_rx.try_recv().is_err());
    }

    #[test]
    fn policy_stream_delivers_real_update_command_sse() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("fixture should bind");
        let address = listener.local_addr().expect("fixture should have address");
        let fixture = thread::spawn(move || {
            let (mut socket, _) = listener.accept().expect("client should connect");
            let mut request = [0_u8; 4096];
            let _ = socket.read(&mut request).expect("request should read");
            let body = concat!(
                "event: update-command\n",
                "id: cmd-install-1\n",
                "data: {\"command_id\":\"cmd-install-1\",\"action\":\"install\",",
                "\"client_id\":\"83077683\",\"client_uuid\":\"01ab\",",
                "\"target_version\":\"1.5.0\",\"target_build_seq\":50,",
                "\"expires_at\":4102444800}\n\n"
            );
            write!(
                socket,
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            )
            .expect("response should write");
        });
        let identity = update_client_identity("83077683", "01ab");
        let (tx, rx) = mpsc::channel();

        read_update_policy_stream_once(
            &format!("http://{address}"),
            &identity,
            Some(3),
            || {},
            |_| {},
            |command| tx.send(command).expect("command should send"),
        )
        .expect("stream should complete");

        fixture.join().expect("fixture should finish");
        let command = rx.recv().expect("command should arrive");
        assert_eq!(command.command_id, "cmd-install-1");
        assert_eq!(command.action, UpdateCommandAction::Install);
    }

    #[test]
    fn command_history_persists_all_ids_and_deduplicates_reconnects() {
        let mut history = ProcessedUpdateCommands::default();
        for index in 0..40 {
            history.remember(format!("cmd-{index}"));
        }

        assert!(history.contains("cmd-0"));
        assert!(history.contains("cmd-39"));
        assert_eq!(history.ids.len(), 40);
        history.remember("cmd-39".to_owned());
        assert_eq!(history.ids.len(), 40);
    }

    #[test]
    fn only_install_commands_defer_for_active_sessions() {
        assert!(DEFERRED_UPDATE_RETRY_INTERVAL <= Duration::from_secs(30));
        assert!(!update_command_should_defer(
            UpdateCommandAction::Check,
            true
        ));
        assert!(update_command_should_defer(
            UpdateCommandAction::Install,
            true
        ));
        assert!(!update_command_should_defer(
            UpdateCommandAction::Install,
            false
        ));
    }

    #[test]
    fn command_target_requires_exact_manifest_version_and_build() {
        assert!(update_command_matches_target("1.5.0", 50, "1.5.0", 50));
        assert!(!update_command_matches_target("1.5.0", 50, "1.5.1", 50));
        assert!(!update_command_matches_target("1.5.0", 50, "1.5.0", 51));
    }

    #[test]
    fn command_outcomes_distinguish_check_install_and_async_completion() {
        assert_ne!(
            UpdateCommandRunOutcome::NoUpdate,
            UpdateCommandRunOutcome::CheckCompleted
        );
        assert_ne!(
            UpdateCommandRunOutcome::Deferred,
            UpdateCommandRunOutcome::AwaitingInstallResult
        );
        assert_ne!(
            UpdateCommandRunOutcome::AwaitingInstallResult,
            UpdateCommandRunOutcome::Installed
        );
        assert_ne!(
            UpdateCommandRunOutcome::Refused,
            UpdateCommandRunOutcome::AwaitingInstallResult
        );
    }

    // The local UI must be able to tell a desk-started update from a command or
    // policy one; a time window cannot, because both can be in flight at once.
    #[test]
    fn local_update_event_payload_carries_the_request_origin() {
        let event = super::local_update_event_payload(
            "failed",
            "1.5.4",
            2026102401,
            "primary",
            "manual",
            "https://download.yan.life/x".to_owned(),
        );
        assert_eq!(event["name"], "software_update_event");
        assert_eq!(event["status"], "failed");
        assert_eq!(event["request_origin"], "manual");
        assert_eq!(event["url"], "https://download.yan.life/x");

        let remote = super::local_update_event_payload(
            "failed",
            "1.5.4",
            2026102401,
            "primary",
            "command_install",
            String::new(),
        );
        assert_eq!(remote["request_origin"], "command_install");
        assert_ne!(remote["request_origin"], "manual");
    }

    // Every event pushed during a check must carry that check's origin, so the
    // reader can decide without a time window.
    #[test]
    fn the_request_origin_is_stored_and_read_back() {
        super::set_update_request_origin("command_install");
        assert_eq!(super::update_request_origin(), "command_install");
        super::set_update_request_origin("manual");
        assert_eq!(super::update_request_origin(), "manual");
    }

    // A manual check that fails after the update URL is known must still end the
    // pending local request; background checks keep failing silently.
    #[test]
    fn only_manual_check_failures_emit_a_failed_update_event() {
        let mut statuses = Vec::new();
        report_manual_update_check_failure(true, true, |status| statuses.push(status));
        assert_eq!(statuses, vec!["failed"]);
        for (manually, url_known) in [(false, true), (false, false)] {
            let mut statuses = Vec::new();
            report_manual_update_check_failure(manually, url_known, |status| statuses.push(status));
            assert!(statuses.is_empty());
        }
    }

    // Before the URL is known the check itself failed and its finish event already
    // carries the error text; a failed status on top would overwrite it.
    #[test]
    fn manual_check_failure_before_the_url_keeps_the_check_error() {
        let mut statuses = Vec::new();
        report_manual_update_check_failure(true, false, |status| statuses.push(status));
        assert!(statuses.is_empty());
    }

    #[test]
    fn android_install_status_only_completes_on_terminal_callback() {
        assert_eq!(
            android_install_terminal_status("installed"),
            Some("installed")
        );
        assert_eq!(android_install_terminal_status("failed"), Some("failed"));
        assert_eq!(android_install_terminal_status("installing"), None);
        assert_eq!(android_install_terminal_status("permission_required"), None);
    }

    #[test]
    fn mac_update_policy_allows_missing_server_but_fails_closed_for_broken_server_ipc() {
        assert!(super::mac_update_allowed(
            false,
            super::MacIpcProbe::Unavailable
        ));
        assert!(!super::mac_update_allowed(
            true,
            super::MacIpcProbe::Unavailable
        ));
        assert!(!super::mac_update_allowed(
            true,
            super::MacIpcProbe::ActiveConnections
        ));
        assert!(super::mac_update_allowed(
            true,
            super::MacIpcProbe::NoActiveConnections
        ));
    }

    #[test]
    fn mac_update_policy_rejects_any_server_that_is_not_safe() {
        assert!(!super::mac_update_allowed(
            true,
            super::MacIpcProbe::ActiveConnections
        ));
        assert!(!super::mac_update_allowed(
            true,
            super::MacIpcProbe::Unavailable
        ));
    }

    #[test]
    fn android_pending_install_survives_missing_flutter_and_failed_reporting() {
        let pending = PendingAndroidUpdate {
            event: PendingUpdateEvent {
                transaction_id: "0123456789abcdef0123456789abcdef".to_owned(),
                from_version: "1.5.1".to_owned(),
                from_build_seq: 2026100202,
                version: "1.5.2".to_owned(),
                build_seq: 2026100203,
                source: UpdateSource::Primary,
                command_id: Some("cmd-android-1".to_owned()),
            },
            path: "/private/update.apk".to_owned(),
            download_url: "https://download.yan.life/rustdesk.apk".to_owned(),
            terminal_status: String::new(),
            error_code: String::new(),
        };
        let encoded = serde_json::to_string(&pending).unwrap();

        let ready = pending_android_update_ready_event(&encoded).unwrap();
        assert_eq!(ready["path"], "/private/update.apk");
        assert_eq!(ready["command_id"], "cmd-android-1");
        assert_eq!(ready["url"], "https://download.yan.life/rustdesk.apk");
        assert!(!android_terminal_report_should_clear(false));
        assert!(android_terminal_report_should_clear(true));
    }

    #[test]
    fn android_terminal_state_blocks_duplicate_install_and_keeps_safe_error() {
        let pending = PendingAndroidUpdate {
            event: PendingUpdateEvent {
                transaction_id: "0123456789abcdef0123456789abcdef".to_owned(),
                from_version: "1.5.1".to_owned(),
                from_build_seq: 2026100202,
                version: "1.5.2".to_owned(),
                build_seq: 2026100203,
                source: UpdateSource::Primary,
                command_id: None,
            },
            path: "/private/update.apk".to_owned(),
            download_url: "https://download.yan.life/rustdesk.apk".to_owned(),
            terminal_status: "failed".to_owned(),
            error_code: "blocked:policy\nignored".to_owned(),
        };
        let encoded = serde_json::to_string(&pending).unwrap();

        assert!(pending_android_update_ready_event(&encoded).is_none());
        assert_eq!(
            sanitize_android_install_error("blocked:policy\nignored"),
            "blocked:policyignored"
        );
        assert_eq!(sanitize_android_install_error(""), "install_failed");
    }

    #[test]
    fn device_auth_headers_sign_the_exact_request_body() {
        let identity = base::update::UpdateClientIdentity {
            client_id: "83077683".to_owned(),
            client_uuid: "01ab".to_owned(),
        };
        let body = br#"{"command_id":"cmd-1"}"#;
        let headers = update_device_auth_headers(
            "POST",
            "https://rdapi.yan.life/rd/update/v1/check?ignored=true",
            &identity,
            "cmd-1",
            body,
        )
        .expect("headers should be signed");
        let canonical = base::update::update_device_auth_payload(
            "POST",
            "/rd/update/v1/check",
            &identity.client_id,
            &identity.client_uuid,
            headers.timestamp.parse().expect("timestamp should parse"),
            &headers.nonce,
            "cmd-1",
            &hex::encode(Sha256::digest(body)),
        );
        let public_key = hbb_common::sodiumoxide::crypto::sign::PublicKey::from_slice(
            &STANDARD
                .decode(headers.public_key)
                .expect("public key should decode"),
        )
        .expect("public key should be valid");
        let signature = hbb_common::sodiumoxide::crypto::sign::Signature::from_bytes(
            &STANDARD
                .decode(headers.signature)
                .expect("signature should decode"),
        )
        .expect("signature should be valid");

        assert!(!headers.device_id.is_empty());
        assert!(hbb_common::sodiumoxide::crypto::sign::verify_detached(
            &signature,
            &canonical,
            &public_key,
        ));
    }
}

#[allow(dead_code)]
pub fn manually_check_update() -> ResultType<()> {
    let sender = TX_MSG.lock().unwrap();
    sender.send(UpdateMsg::CheckUpdate)?;
    Ok(())
}

/// 桌面用户显式要求安装。与 `manually_check_update` 分开，是因为后台/连接恢复
/// 的检查也会按 `manually` 处理，而只有这一条消息代表「这个人现在就要装」。
#[allow(dead_code)]
pub fn install_update_now() -> ResultType<()> {
    let sender = TX_MSG.lock().unwrap();
    sender.send(UpdateMsg::InstallUpdate)?;
    Ok(())
}

#[allow(dead_code)]
pub fn stop_auto_update() {
    let sender = TX_MSG.lock().unwrap();
    sender.send(UpdateMsg::Exit).unwrap_or_default();
}

#[inline]
/// Returns true when there are no active incoming or outgoing connections.
/// Used to avoid updating while a remote session is in progress.
pub fn has_no_active_conns() -> bool {
    let conns = crate::Connection::alive_conns();
    conns.is_empty() && has_no_controlling_conns()
}

#[cfg(any(not(target_os = "windows"), feature = "flutter"))]
fn has_no_controlling_conns() -> bool {
    CONTROLLING_SESSION_COUNT.load(Ordering::SeqCst) == 0
}

#[cfg(not(any(not(target_os = "windows"), feature = "flutter")))]
fn has_no_controlling_conns() -> bool {
    let app_exe = format!("{}.exe", crate::get_app_name().to_lowercase());
    for arg in [
        "--connect",
        "--play",
        "--file-transfer",
        "--view-camera",
        "--port-forward",
        "--rdp",
    ] {
        if !crate::platform::get_pids_of_process_with_first_arg(&app_exe, arg).is_empty() {
            return false;
        }
    }
    true
}

// Without an owning service, a remote, policy or startup install from this process
// would raise UAC (Windows `runas`) or the macOS administrator password prompt.
// Such installs fail with a fixed error code and only leave a log line. The command
// terminal is persisted here, so the caller reports nothing further for it.
#[cfg(any(target_os = "windows", target_os = "macos"))]
fn remote_install_needs_refused_elevation(
    manually: bool,
    command: Option<&UpdateCommand>,
    version: &str,
    build_seq: u64,
) -> bool {
    #[cfg(target_os = "windows")]
    let elevated =
        crate::platform::is_root() || crate::platform::is_elevated(None).unwrap_or(false);
    #[cfg(target_os = "macos")]
    let elevated = crate::platform::is_root();
    let manual = !base::update::update_is_remote(manually, command.is_some());
    if base::update::remote_install_may_request_elevation(manual, elevated) {
        return false;
    }
    let code = base::update::ELEVATION_REQUIRED_REMOTE;
    log::warn!("Remote update install to {version} refused: {code}");
    static REFUSAL_REPORTED: AtomicBool = AtomicBool::new(false);
    if base::update::should_report_refused_elevation(
        command.is_some(),
        command.is_none() && REFUSAL_REPORTED.swap(true, Ordering::SeqCst),
    ) {
        report_update_event_with_origin_and_error(
            "failed",
            crate::VERSION,
            crate::BUILD_SEQ,
            version,
            build_seq,
            "none",
            code,
        );
    }
    if let Some(command) = command {
        persist_and_report_update_command_terminal(command, "failed", code);
    }
    true
}

fn background_updater_runs_here() -> bool {
    #[cfg(target_os = "windows")]
    {
        base::update::process_runs_background_updater(
            crate::platform::is_installed(),
            crate::is_server(),
            crate::platform::is_self_service_installed(),
        )
    }
    #[cfg(target_os = "macos")]
    {
        let daemon = crate::platform::is_installed_daemon(false);
        base::update::process_runs_background_updater(daemon, crate::platform::is_root(), daemon)
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        true
    }
}

// User-session process of an installed client: only the person at the desk can
// start an update here; pending remote commands and timed checks belong to the
// service and are neither replayed nor scheduled.
fn start_manual_update_check() -> Sender<UpdateMsg> {
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let mut last_check_time = None;
        let mut next_scheduled_check = None;
        while let Ok(msg) = rx.recv() {
            match msg {
                UpdateMsg::CheckUpdate => run_update_check(
                    true,
                    "manual",
                    &mut last_check_time,
                    &mut next_scheduled_check,
                ),
                UpdateMsg::InstallUpdate => run_update_check_with_action(
                    true,
                    "manual",
                    true,
                    &mut last_check_time,
                    &mut next_scheduled_check,
                ),
                UpdateMsg::Exit => break,
                _ => {}
            }
        }
    });
    tx
}

fn start_auto_update_check() -> Sender<UpdateMsg> {
    if !background_updater_runs_here() {
        return start_manual_update_check();
    }
    let (tx, rx) = channel();
    let pending = load_pending_update_commands();
    let startup_tx = tx.clone();
    std::thread::spawn(move || start_auto_update_check_(rx));
    for state in pending {
        #[cfg(target_os = "windows")]
        if ordinary_update_receipts::runtime::owns_command(&state.command.command_id) {
            continue;
        }
        if state.terminal.is_some() {
            retry_update_command_terminal(&state.command.command_id);
        } else {
            let _ = startup_tx.send(UpdateMsg::Command(state.command));
        }
    }
    return tx;
}

fn start_auto_update_check_(rx_msg: Receiver<UpdateMsg>) {
    #[cfg(target_os = "macos")]
    consume_mac_update_result();
    // 上一次安装是否真的生效，只有重启后的这里才能判断：安装器会终止旧进程，
    // 终态上报来不及发出。失败会在此计入熔断，避免一个装不上的版本被反复拉起。
    resolve_pending_install();
    let started_at = Instant::now();
    let mut startup_check_pending = true;
    let mut last_check_time = None;
    let mut next_scheduled_check = scheduled_update_interval().map(|value| started_at + value);
    loop {
        let now = Instant::now();
        let startup_deadline = startup_check_pending.then_some(started_at + INITIAL_CHECK_DELAY);
        let next_deadline = [startup_deadline, next_scheduled_check]
            .into_iter()
            .flatten()
            .min();
        let wait = next_deadline
            .map(|deadline| deadline.saturating_duration_since(now))
            .unwrap_or(Duration::from_secs(60 * 60 * 24));
        match rx_msg.recv_timeout(wait) {
            Ok(UpdateMsg::CheckUpdate) => {
                run_update_check(
                    true,
                    "manual",
                    &mut last_check_time,
                    &mut next_scheduled_check,
                );
            }
            Ok(UpdateMsg::InstallUpdate) => {
                run_update_check_with_action(
                    true,
                    "manual",
                    true,
                    &mut last_check_time,
                    &mut next_scheduled_check,
                );
            }
            Ok(UpdateMsg::ConnectivityRestored) => {
                if background_update_enabled()
                    && last_check_time
                        .map(|value: Instant| value.elapsed() >= MIN_INTERVAL)
                        .unwrap_or(true)
                {
                    run_update_check(
                        false,
                        "system",
                        &mut last_check_time,
                        &mut next_scheduled_check,
                    );
                }
            }
            Ok(UpdateMsg::ScheduleChanged) => {
                next_scheduled_check =
                    scheduled_update_interval().map(|value| Instant::now() + value);
            }
            Ok(UpdateMsg::Command(command)) => {
                let identity = current_update_client_identity();
                let history = load_processed_update_commands();
                match command.accepted_decision(&identity, history.contains(&command.command_id)) {
                    UpdateCommandDecision::IdentityMismatch => complete_update_command(&command),
                    UpdateCommandDecision::Duplicate => complete_update_command(&command),
                    UpdateCommandDecision::Expired | UpdateCommandDecision::Execute => {
                        if update_command_should_defer(command.action, !has_no_active_conns()) {
                            report_update_command_deferred_once(&command);
                            schedule_update_command_retry(command.clone());
                            continue;
                        }
                        report_update_command_event(&command, "started", "");
                        let result = check_update_for_command(&command);
                        match result {
                            Ok(UpdateCommandRunOutcome::NoUpdate) => {
                                persist_and_report_update_command_terminal(
                                    &command,
                                    "no_update",
                                    "",
                                );
                            }
                            Ok(UpdateCommandRunOutcome::CheckCompleted) => {
                                persist_and_report_update_command_terminal(
                                    &command,
                                    "completed",
                                    "",
                                );
                            }
                            Ok(UpdateCommandRunOutcome::Deferred) => {
                                report_update_command_deferred_once(&command);
                                schedule_update_command_retry(command.clone());
                            }
                            Ok(UpdateCommandRunOutcome::AwaitingInstallResult) => {}
                            Ok(UpdateCommandRunOutcome::Refused) => {}
                            Ok(UpdateCommandRunOutcome::Suspended) => {
                                persist_and_report_update_command_terminal(
                                    &command,
                                    "failed",
                                    "auto_install_suspended",
                                );
                            }
                            Ok(UpdateCommandRunOutcome::Installed) => {
                                persist_and_report_update_command_terminal(
                                    &command,
                                    "completed",
                                    "",
                                );
                            }
                            Err(err) => {
                                log::error!("Update command {} failed: {err}", command.command_id);
                                persist_and_report_update_command_terminal(
                                    &command,
                                    "failed",
                                    "command_failed",
                                );
                            }
                        }
                    }
                }
            }
            Ok(UpdateMsg::Exit) => break,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                let now = Instant::now();
                let startup_due =
                    startup_check_pending && now.duration_since(started_at) >= INITIAL_CHECK_DELAY;
                if startup_due {
                    startup_check_pending = false;
                }
                let scheduled_due = next_scheduled_check
                    .map(|deadline| now >= deadline)
                    .unwrap_or(false);
                if startup_due && startup_update_enabled() {
                    run_update_check(
                        false,
                        "startup",
                        &mut last_check_time,
                        &mut next_scheduled_check,
                    );
                } else if scheduled_due {
                    run_update_check(
                        false,
                        "system",
                        &mut last_check_time,
                        &mut next_scheduled_check,
                    );
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
}

fn startup_update_enabled() -> bool {
    should_run_startup_update(base::update::check_on_startup_enabled(
        &config::Config::get_option(keys::OPTION_ENABLE_CHECK_UPDATE),
        &config::LocalConfig::get_option(keys::OPTION_ENABLE_CHECK_UPDATE),
    ))
}

/// 收到服务端的发布广播后，本机是否应该主动检查一次更新。
///
/// 广播只是「有新版本了」的提示，不是一条命令：它必须完全走后台路径，不能把
/// `manually` 置真，否则会把 `mode=notify` 的设备直接推进安装，正是 D37 那种
/// 绕过 mode / auto_install / allow_auto_update 的写法。
fn release_notice_should_check(
    notice: &base::update::ReleaseNotice,
    running_build_seq: u64,
    background_enabled: bool,
) -> bool {
    background_enabled
        && notice.build_seq > running_build_seq
        && (notice.channel.is_empty() || notice.channel == crate::CHANNEL)
        // `map_or(true, ..)` 而不是 `is_none_or`：后者在发布用的 Rust 1.90 上仍是
        // unstable（E0658），macOS 构建会直接编译失败。
        && notice
            .device_build_seq
            .map_or(true, |seen| seen == running_build_seq)
}

fn should_check_on_connectivity_restored(check_on_startup: bool, scheduled_update: bool) -> bool {
    check_on_startup || scheduled_update
}

fn background_update_enabled() -> bool {
    should_check_on_connectivity_restored(
        startup_update_enabled(),
        update_option_enabled(&config::Config::get_option(
            keys::OPTION_ENABLE_SCHEDULED_UPDATE,
        )),
    )
}

fn scheduled_update_interval() -> Option<Duration> {
    let hours = config::Config::get_option(keys::OPTION_SCHEDULED_UPDATE_INTERVAL_HOURS)
        .parse()
        .unwrap_or(DEFAULT_SCHEDULED_UPDATE_INTERVAL_HOURS);
    scheduled_update_delay(
        update_option_enabled(&config::Config::get_option(
            keys::OPTION_ENABLE_SCHEDULED_UPDATE,
        )),
        hours,
    )
}

fn scheduled_update_delay(enabled: bool, hours: u64) -> Option<Duration> {
    if !should_run_scheduled_update(enabled) {
        return None;
    }
    Some(Duration::from_secs(
        normalize_scheduled_update_interval_hours(hours) * 60 * 60,
    ))
}

fn run_update_check(
    manually: bool,
    request_origin: &str,
    last_check_time: &mut Option<Instant>,
    next_scheduled_check: &mut Option<Instant>,
) {
    run_update_check_with_action(
        manually,
        request_origin,
        false,
        last_check_time,
        next_scheduled_check,
    )
}

fn run_update_check_with_action(
    manually: bool,
    request_origin: &str,
    install_requested: bool,
    last_check_time: &mut Option<Instant>,
    next_scheduled_check: &mut Option<Instant>,
) {
    match check_update_request_with_action(manually, None, request_origin, install_requested) {
        Ok(UpdateCommandRunOutcome::Deferred) => {
            *next_scheduled_check = Some(Instant::now() + DEFERRED_UPDATE_RETRY_INTERVAL);
        }
        Ok(_) => {
            *last_check_time = Some(Instant::now());
            *next_scheduled_check = scheduled_update_interval().map(|value| Instant::now() + value);
        }
        Err(err) => {
            log::error!("Error checking for updates: {err}");
            report_manual_update_check_failure(
                manually,
                !crate::common::SOFTWARE_UPDATE_URL
                    .lock()
                    .unwrap()
                    .is_empty(),
                push_local_update_failed_event,
            );
            *next_scheduled_check = Some(Instant::now() + RETRY_INTERVAL);
        }
    }
}

// A manual check that fails after the URL is known (contract, target or manifest
// errors) has no failed event of its own, which would leave the local request
// pending. Before the URL is known the check's finish event already carries the
// error. Only the local UI is told: the backend keeps its existing events, and
// paths that already reported `failed` would otherwise get a second copy there.
fn report_manual_update_check_failure(
    manually: bool,
    url_known: bool,
    emit: impl FnOnce(&'static str),
) {
    if manually && url_known {
        emit("failed");
    }
}

fn push_local_update_failed_event(status: &str) {
    #[cfg(feature = "flutter")]
    {
        let (version, build_seq) = crate::common::SOFTWARE_UPDATE_RESPONSE
            .lock()
            .unwrap()
            .as_ref()
            .map(|response| (response.target_version.clone(), response.target_build_seq))
            .unwrap_or_default();
        let event = local_update_event_payload(
            status,
            &version,
            build_seq,
            "none",
            "manual",
            crate::common::SOFTWARE_UPDATE_URL.lock().unwrap().clone(),
        );
        let _ = crate::flutter::push_global_event(crate::flutter::APP_TYPE_MAIN, event.to_string());
    }
    #[cfg(not(feature = "flutter"))]
    let _ = status;
}

// The local UI must be able to tell an update the person at the desk started
// from one a command or policy started; only the former may fall back to the
// browser. A time window cannot make that distinction, because both can be in
// flight at once in the same process.
#[cfg_attr(not(feature = "flutter"), allow(dead_code))]
fn local_update_event_payload(
    status: &str,
    version: &str,
    build_seq: u64,
    source: &str,
    request_origin: &str,
    url: String,
) -> serde_json::Value {
    serde_json::json!({
        "name": "software_update_event",
        "status": status,
        "version": version,
        "build_seq": build_seq,
        "source": source,
        "request_origin": request_origin,
        "url": url,
    })
}

fn schedule_update_command_retry(command: UpdateCommand) {
    std::thread::spawn(move || {
        std::thread::sleep(UPDATE_COMMAND_RETRY_INTERVAL);
        let sender = TX_MSG.lock().unwrap();
        let _ = sender.send(UpdateMsg::Command(command));
    });
}

/// 安装命令在安装过程中会终止本进程，终态来不及上报。重启后恢复该命令时，
/// 若当前运行的就是目标版本（或更新），安装其实已经完成；此时再查一次更新
/// 只会得到「无更新」，把已完成的安装改写成 no_update。
fn install_command_satisfied(
    command: &UpdateCommand,
    running_version: &str,
    running_build_seq: u64,
) -> bool {
    if command.action != UpdateCommandAction::Install {
        return false;
    }
    let (Some(version), Some(build_seq)) = (&command.target_version, command.target_build_seq)
    else {
        return false;
    };
    !hbb_common::is_newer_version(version, build_seq, running_version, running_build_seq)
}

/// 只有同时满足「运行的就是目标版本」和「这条命令确由本机发起并已交给安装器」，
/// 才能断定安装已完成。缺了后者，一条与本机签名清单不符、本机从未执行过的命令
/// 会被记成安装成功，跳过目标与签名校验。
fn install_command_already_applied(
    command: &UpdateCommand,
    running_version: &str,
    running_build_seq: u64,
    launched_by_this_install: bool,
) -> bool {
    launched_by_this_install
        && install_command_satisfied(command, running_version, running_build_seq)
}

fn check_update_for_command(command: &UpdateCommand) -> ResultType<UpdateCommandRunOutcome> {
    if install_command_already_applied(
        command,
        crate::VERSION,
        crate::BUILD_SEQ,
        install_state::pending_install_owns_command(&command.command_id),
    ) {
        return Ok(UpdateCommandRunOutcome::Installed);
    }
    check_update_request(false, Some(command), "command")
}

fn check_update_request(
    manually: bool,
    command: Option<&UpdateCommand>,
    request_origin: &str,
) -> ResultType<UpdateCommandRunOutcome> {
    check_update_request_with_action(manually, command, request_origin, false)
}

fn check_update_request_with_action(
    manually: bool,
    command: Option<&UpdateCommand>,
    request_origin: &str,
    install_requested: bool,
) -> ResultType<UpdateCommandRunOutcome> {
    // On macOS, auto-update is handled by check_update_as_root() in the service process.
    // The shared check_update() path is only used for manual update checks from the GUI.
    #[cfg(target_os = "macos")]
    if !manually && command.is_none() && request_origin != "startup" {
        return Ok(UpdateCommandRunOutcome::NoUpdate);
    }
    let command_result = if let Some(command) = command {
        let request_origin = if command.action == UpdateCommandAction::Install {
            "command_install"
        } else {
            "command"
        };
        set_update_request_origin(request_origin);
        Some(do_check_software_update_with_context_result(
            request_origin,
            &command.command_id,
        )?)
    } else {
        set_update_request_origin(request_origin);
        do_check_software_update_with_context(request_origin, "")?;
        None
    };

    let response = command_result
        .as_ref()
        .and_then(|result| result.response.clone())
        .or_else(|| {
            crate::common::SOFTWARE_UPDATE_RESPONSE
                .lock()
                .unwrap()
                .clone()
        });
    let Some(response) = response else {
        log::debug!("No update available.");
        return Ok(UpdateCommandRunOutcome::NoUpdate);
    };
    if !response.update_available || (response.mode == "disabled" && !manually && command.is_none())
    {
        return Ok(UpdateCommandRunOutcome::NoUpdate);
    }
    let Some(ref manifest) = response.manifest else {
        bail!("update response is missing a signed manifest");
    };
    let target_key = command_result
        .as_ref()
        .map(|result| result.target_key.clone())
        .unwrap_or_else(|| {
            crate::common::SOFTWARE_UPDATE_TARGET_KEY
                .lock()
                .unwrap()
                .clone()
        });
    if target_key.is_empty() {
        bail!("update target selection is missing");
    }
    #[cfg(target_os = "windows")]
    let update_msi = target_key.contains("-msi-");
    validate_manifest_contract(
        &response,
        &manifest,
        crate::VERSION,
        crate::BUILD_SEQ,
        crate::PRODUCT,
        crate::EDITION,
        crate::CHANNEL,
        &target_key,
    )?;
    let target = manifest.targets.get(&target_key).cloned().ok_or_else(|| {
        hbb_common::anyhow::anyhow!("signed manifest is missing the selected update target")
    })?;
    let (target_version, target_build_seq, _) = update_target_release(manifest, &target)?;
    if let Some(command) = command.filter(|value| value.action == UpdateCommandAction::Install) {
        let command_version = command.target_version.as_deref().ok_or_else(|| {
            hbb_common::anyhow::anyhow!("install command target version is missing")
        })?;
        let command_build_seq = command.target_build_seq.ok_or_else(|| {
            hbb_common::anyhow::anyhow!("install command target build is missing")
        })?;
        if !update_command_matches_target(
            command_version,
            command_build_seq,
            target_version,
            target_build_seq,
        ) {
            bail!("update command target does not match signed manifest");
        }
    }
    let mut action = decide_update_action(
        &response,
        update_option_enabled(&config::Config::get_option(keys::OPTION_ALLOW_AUTO_UPDATE)),
    );
    // `disabled` 是设备上真正的开关，显式安装请求也不能越过它。
    if install_requested && action == UpdateAction::Download {
        action = UpdateAction::AutoInstall;
    }
    if let Some(command) = command {
        action = match command.action {
            UpdateCommandAction::Check => UpdateAction::Notify,
            UpdateCommandAction::Install => UpdateAction::AutoInstall,
        };
    }
    if matches!(action, UpdateAction::Ignore | UpdateAction::Notify) {
        return Ok(if command.is_some() {
            UpdateCommandRunOutcome::CheckCompleted
        } else {
            UpdateCommandRunOutcome::NoUpdate
        });
    }
    let should_install = action == UpdateAction::AutoInstall;
    let version = target_version;
    if should_install && !has_no_active_conns() {
        report_update_event("deferred", version, target_build_seq, "none");
        report_command_update_event(command, "deferred", "active_session");
        return Ok(UpdateCommandRunOutcome::Deferred);
    }
    // 只有真正会拉安装器的路径才受尝试次数与安装租约约束：检查、下载、
    // 提示都不碰本机安装，不该被上一次安装的失败记录挡住。
    let _install_lease = if should_install {
        if !install_state::auto_install_allowed(version, target_build_seq, update_now_unix()) {
            log::warn!(
                "Automatic update to {version} ({target_build_seq}) is suspended after repeated attempts"
            );
            return Ok(UpdateCommandRunOutcome::Suspended);
        }
        match install_state::try_acquire_install_lease() {
            Some(lease) => Some(lease),
            None => {
                report_command_update_event(command, "deferred", "install_in_progress");
                return Ok(UpdateCommandRunOutcome::Deferred);
            }
        }
    } else {
        None
    };
    #[cfg(any(target_os = "windows", target_os = "macos"))]
    if should_install
        && remote_install_needs_refused_elevation(manually, command, version, target_build_seq)
    {
        return Ok(UpdateCommandRunOutcome::Refused);
    }
    report_update_event("started", version, target_build_seq, "none");
    let (file_path, source) = match download_verified_target(manifest, &target_key, &target) {
        Ok(downloaded) => downloaded,
        Err(err) => {
            report_update_event("failed", version, target_build_seq, "none");
            report_command_update_event(command, "failed", "all_sources_failed");
            return Err(err);
        }
    };
    let source = source.as_str();
    report_update_event("downloaded", version, target_build_seq, source);
    report_command_update_event(command, "downloaded", "");
    {
        #[cfg(target_os = "windows")]
        log::debug!("New version available: {}", version);
        // Recheck because a session can start while the verified asset is downloading.
        if should_install && has_no_active_conns() {
            #[cfg(target_os = "windows")]
            if update_msi
                && ordinary_update_receipts::runtime::use_receipt_path()
                    .map_err(|error| hbb_common::anyhow::anyhow!(error))?
            {
                let approved_name =
                    ordinary_update_receipts::package_contract::primary_package_name(
                        &target.primary,
                        "msi",
                    )
                    .map_err(|error| hbb_common::anyhow::anyhow!(error))?;
                ordinary_update_receipts::runtime::install(
                    &file_path,
                    &approved_name,
                    &target.sha256,
                    &target_key,
                    version,
                    target_build_seq,
                    source,
                    command.map(|value| value.command_id.clone()),
                )
                .map_err(|error| hbb_common::anyhow::anyhow!(error))?;
                return Ok(UpdateCommandRunOutcome::AwaitingInstallResult);
            }
            report_update_event("installing", version, target_build_seq, source);
            report_command_update_event(command, "installing", "");
            #[cfg(target_os = "windows")]
            update_new_version(
                update_msi,
                version,
                target_build_seq,
                source,
                &file_path,
                command.map(|value| value.command_id.as_str()),
                base::update::update_is_remote(manually, command.is_some()),
            )?;
            #[cfg(target_os = "windows")]
            if update_msi {
                report_command_update_event(command, "installed", "");
            }
            #[cfg(target_os = "linux")]
            if let Err(err) = install_linux_appimage(&file_path) {
                log::error!("Failed to install AppImage update: {}", err);
                report_update_event("rolled_back", version, target_build_seq, source);
                remove_download_artifact(&file_path);
                return Err(err);
            } else {
                report_update_event("installed", version, target_build_seq, source);
                remove_download_artifact(&file_path);
            }
            #[cfg(target_os = "macos")]
            if let Some(path) = file_path.to_str() {
                let event = PendingUpdateEvent {
                    transaction_id: new_update_transaction_id(),
                    from_version: crate::VERSION.to_owned(),
                    from_build_seq: crate::BUILD_SEQ,
                    version: version.to_owned(),
                    build_seq: target_build_seq,
                    source: if source == UpdateSource::Mirror.as_str() {
                        UpdateSource::Mirror
                    } else {
                        UpdateSource::Primary
                    },
                    command_id: command.map(|value| value.command_id.clone()),
                };
                if let Err(err) = crate::platform::request_update_from_dmg_as_root(path, &event) {
                    log::error!("Failed to install verified macOS update: {}", err);
                    report_update_event(
                        classify_update_preinstall_failure(&err.to_string()),
                        version,
                        target_build_seq,
                        source,
                    );
                    remove_download_artifact(&file_path);
                    return Err(err);
                }
                remove_download_artifact(&file_path);
            } else {
                remove_download_artifact(&file_path);
                bail!("downloaded update path is not valid UTF-8");
            }
            #[cfg(target_os = "android")]
            if let Err(err) = request_android_update_install(
                &file_path,
                version,
                target_build_seq,
                source,
                &target.primary,
                command.map(|value| value.command_id.clone()),
            ) {
                report_update_event("failed", version, target_build_seq, source);
                report_command_update_event(command, "failed", "installer_unavailable");
                remove_download_artifact(&file_path);
                return Err(err);
            }
        } else if should_install {
            report_update_event("deferred", version, target_build_seq, source);
            remove_download_artifact(&file_path);
            return Ok(UpdateCommandRunOutcome::Deferred);
        }
    }
    #[cfg(target_os = "windows")]
    if should_install && !update_msi {
        return Ok(UpdateCommandRunOutcome::AwaitingInstallResult);
    }
    #[cfg(target_os = "macos")]
    if should_install {
        return Ok(UpdateCommandRunOutcome::AwaitingInstallResult);
    }
    #[cfg(target_os = "android")]
    if should_install {
        return Ok(UpdateCommandRunOutcome::AwaitingInstallResult);
    }
    Ok(if should_install {
        UpdateCommandRunOutcome::Installed
    } else {
        UpdateCommandRunOutcome::CheckCompleted
    })
}

pub fn current_update_target_key() -> ResultType<String> {
    current_update_target().map(|(target_key, _)| target_key)
}

fn new_update_transaction_id() -> String {
    hex::encode(hbb_common::sodiumoxide::randombytes::randombytes(16))
}

pub fn current_update_target() -> ResultType<(String, String)> {
    let platform = std::env::consts::OS;
    let arch = std::env::consts::ARCH;
    let kind = update_target_kind()?;
    Ok((
        update_target_key(platform, arch, kind, crate::EDITION),
        kind.to_owned(),
    ))
}

#[cfg(target_os = "windows")]
fn update_target_kind() -> ResultType<&'static str> {
    Ok(if crate::platform::is_msi_installed()? {
        "msi"
    } else {
        "exe"
    })
}

#[cfg(target_os = "linux")]
fn update_target_kind() -> ResultType<&'static str> {
    Ok("appimage")
}

#[cfg(target_os = "macos")]
fn update_target_kind() -> ResultType<&'static str> {
    Ok("dmg")
}

#[cfg(target_os = "android")]
fn update_target_kind() -> ResultType<&'static str> {
    Ok("apk")
}

#[cfg(not(any(
    target_os = "windows",
    target_os = "linux",
    target_os = "macos",
    target_os = "android"
)))]
fn update_target_kind() -> ResultType<&'static str> {
    Ok("pkg")
}

#[cfg(target_os = "android")]
fn request_android_update_install(
    file_path: &Path,
    version: &str,
    build_seq: u64,
    source: &str,
    download_url: &str,
    command_id: Option<String>,
) -> ResultType<()> {
    let path = file_path
        .to_str()
        .ok_or_else(|| hbb_common::anyhow::anyhow!("downloaded update path is not valid UTF-8"))?;
    let pending = PendingUpdateEvent {
        transaction_id: new_update_transaction_id(),
        from_version: crate::VERSION.to_owned(),
        from_build_seq: crate::BUILD_SEQ,
        version: version.to_owned(),
        build_seq,
        source: if source == UpdateSource::Mirror.as_str() {
            UpdateSource::Mirror
        } else {
            UpdateSource::Primary
        },
        command_id,
    };
    let pending = PendingAndroidUpdate {
        event: pending,
        path: path.to_owned(),
        download_url: download_url.to_owned(),
        terminal_status: String::new(),
        error_code: String::new(),
    };
    config::Config::set_option(
        keys::OPTION_PENDING_ANDROID_UPDATE.to_owned(),
        serde_json::to_string(&pending)?,
    );
    let event = serde_json::json!({
        "name": "android_update_ready",
        "path": path,
        "version": version,
        "build_seq": build_seq,
        "source": source,
        "url": pending.download_url,
        "command_id": pending.event.command_id,
    });
    let _ = crate::flutter::push_global_event(crate::flutter::APP_TYPE_MAIN, event.to_string());
    Ok(())
}

#[cfg(any(test, target_os = "android"))]
#[derive(Debug, serde::Deserialize, serde::Serialize)]
struct PendingAndroidUpdate {
    event: PendingUpdateEvent,
    path: String,
    #[serde(default)]
    download_url: String,
    #[serde(default)]
    terminal_status: String,
    #[serde(default)]
    error_code: String,
}

#[cfg(any(test, target_os = "android"))]
fn pending_android_update_ready_event(value: &str) -> Option<serde_json::Value> {
    let pending: PendingAndroidUpdate = serde_json::from_str(value).ok()?;
    if pending.path.is_empty() || !pending.terminal_status.is_empty() {
        return None;
    }
    Some(serde_json::json!({
        "name": "android_update_ready",
        "path": pending.path,
        "version": pending.event.version,
        "build_seq": pending.event.build_seq,
        "source": pending.event.source.as_str(),
        "url": pending.download_url,
        "command_id": pending.event.command_id,
    }))
}

#[cfg(target_os = "android")]
pub fn pending_android_update_ready() -> String {
    pending_android_update_ready_event(&config::Config::get_option(
        keys::OPTION_PENDING_ANDROID_UPDATE,
    ))
    .map(|event| event.to_string())
    .unwrap_or_default()
}

#[cfg(target_os = "android")]
pub fn pending_android_update_state() -> String {
    config::Config::get_option(keys::OPTION_PENDING_ANDROID_UPDATE)
}

#[cfg(any(test, target_os = "android"))]
fn android_install_terminal_status(status: &str) -> Option<&'static str> {
    match status {
        "installed" => Some("installed"),
        "failed" => Some("failed"),
        _ => None,
    }
}

#[cfg(any(test, target_os = "android"))]
fn sanitize_android_install_error(value: &str) -> String {
    let sanitized: String = value
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | ':' | '.'))
        .take(128)
        .collect();
    if sanitized.is_empty() {
        "install_failed".to_owned()
    } else {
        sanitized
    }
}

#[cfg(any(test, target_os = "android"))]
fn android_terminal_report_should_clear(reported: bool) -> bool {
    reported
}

#[cfg(target_os = "android")]
pub fn complete_android_update_install(value: &str) -> ResultType<()> {
    let result: serde_json::Value = serde_json::from_str(value)?;
    let status = result
        .get("status")
        .and_then(|value| value.as_str())
        .unwrap_or_default();
    let Some(terminal_status) = android_install_terminal_status(status) else {
        return Ok(());
    };
    let error_code = if terminal_status == "failed" {
        sanitize_android_install_error(
            result
                .get("error")
                .and_then(|value| value.as_str())
                .unwrap_or_default(),
        )
    } else {
        String::new()
    };
    let mut pending: PendingAndroidUpdate = serde_json::from_str(&config::Config::get_option(
        keys::OPTION_PENDING_ANDROID_UPDATE,
    ))
    .map_err(|_| hbb_common::anyhow::anyhow!("pending Android update is missing"))?;
    pending.terminal_status = terminal_status.to_owned();
    pending.error_code = error_code.clone();
    config::Config::set_option(
        keys::OPTION_PENDING_ANDROID_UPDATE.to_owned(),
        serde_json::to_string(&pending)?,
    );
    let reported =
        report_pending_update_terminal_with_error(terminal_status, &pending.event, &error_code);
    if android_terminal_report_should_clear(reported) {
        config::Config::set_option(
            keys::OPTION_PENDING_ANDROID_UPDATE.to_owned(),
            String::new(),
        );
        Ok(())
    } else {
        bail!("Android update terminal event reporting is pending retry")
    }
}

const UPDATE_SIGNATURE_KEY_ID: &str = "yan-release-2026";
const UPDATE_PUBLIC_KEY_BASE64: &str = "YNphn2SGjnetwp0bb/uEGpzfQi8OavMWTHCmqPbMuxg=";

fn decode_signature_value(value: &str) -> ResultType<Vec<u8>> {
    STANDARD
        .decode(value)
        .or_else(|_| URL_SAFE_NO_PAD.decode(value))
        .map_err(|e| hbb_common::anyhow::anyhow!("invalid update signature encoding: {e}"))
}

fn verify_update_signature(
    manifest: &hbb_common::UpdateManifest,
    target_key: &str,
    target: &hbb_common::UpdateTarget,
) -> ResultType<()> {
    if target.signature_key_id != UPDATE_SIGNATURE_KEY_ID {
        bail!("update signing key not trusted");
    }
    let public_key = decode_signature_value(UPDATE_PUBLIC_KEY_BASE64)?;
    let signature = decode_signature_value(&target.signature)?;
    let payload = update_signature_payload(manifest, target_key, target)?;
    verify_detached_signature(&payload, &signature, &public_key)
}

fn verify_detached_signature(data: &[u8], signature: &[u8], public_key: &[u8]) -> ResultType<()> {
    let public_key = hbb_common::sodiumoxide::crypto::sign::PublicKey::from_slice(&public_key)
        .ok_or_else(|| hbb_common::anyhow::anyhow!("invalid update public key"))?;
    let signature = hbb_common::sodiumoxide::crypto::sign::Signature::from_bytes(signature)
        .map_err(|_| hbb_common::anyhow::anyhow!("invalid update signature"))?;
    if !hbb_common::sodiumoxide::crypto::sign::verify_detached(&signature, data, &public_key) {
        bail!("update signature verification failed");
    }
    Ok(())
}

const UPDATE_DOWNLOAD_DIR_PREFIX: &str = ".rustdesk-update-";

struct UpdateDownloadStaging {
    dir: PathBuf,
    file: PathBuf,
    keep: bool,
}

impl UpdateDownloadStaging {
    fn create(filename: &str) -> ResultType<Self> {
        if !is_plain_update_filename(filename) {
            bail!("invalid update filename");
        }

        for _ in 0..16 {
            let suffix = hex::encode(hbb_common::sodiumoxide::randombytes::randombytes(16));
            let dir = std::env::temp_dir().join(format!("{UPDATE_DOWNLOAD_DIR_PREFIX}{suffix}"));
            let mut builder = std::fs::DirBuilder::new();
            #[cfg(unix)]
            builder.mode(0o700);
            match builder.create(&dir) {
                Ok(()) => {
                    return Ok(Self {
                        file: dir.join(filename),
                        dir,
                        keep: false,
                    });
                }
                Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(err) => return Err(err.into()),
            }
        }
        bail!("failed to allocate a unique update download directory")
    }

    fn create_file(&self) -> ResultType<File> {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        options
            .mode(0o600)
            .custom_flags(hbb_common::libc::O_NOFOLLOW | hbb_common::libc::O_CLOEXEC);
        let file = options.open(&self.file)?;
        Ok(file)
    }

    fn persist(mut self) -> PathBuf {
        self.keep = true;
        self.file.clone()
    }
}

impl Drop for UpdateDownloadStaging {
    fn drop(&mut self) {
        if !self.keep {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }
}

#[cfg(target_os = "macos")]
struct DownloadArtifactCleanup(PathBuf);

#[cfg(target_os = "macos")]
impl Drop for DownloadArtifactCleanup {
    fn drop(&mut self) {
        remove_download_artifact(&self.0);
    }
}

#[cfg(target_os = "macos")]
struct UpdateDirectoryCleanup(PathBuf);

#[cfg(target_os = "macos")]
impl Drop for UpdateDirectoryCleanup {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn update_filename_from_url(url: &str) -> Option<String> {
    let filename = url::Url::parse(url)
        .ok()?
        .path_segments()?
        .next_back()?
        .to_owned();
    is_plain_update_filename(&filename).then_some(filename)
}

fn remove_download_artifact(path: &Path) {
    let _ = std::fs::remove_file(path);
    let Some(parent) = path.parent() else {
        return;
    };
    let is_update_dir = parent
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with(UPDATE_DOWNLOAD_DIR_PREFIX));
    if is_update_dir && parent.parent() == Some(std::env::temp_dir().as_path()) {
        let _ = std::fs::remove_dir(parent);
    }
}

fn download_and_verify(
    url: &str,
    manifest: &hbb_common::UpdateManifest,
    target_key: &str,
    target: &hbb_common::UpdateTarget,
) -> ResultType<PathBuf> {
    validate_target_metadata(target, UPDATE_SIGNATURE_KEY_ID)?;
    let client = create_http_client_with_url_strict(url)?;
    let Some(filename) = update_filename_from_url(url) else {
        bail!("invalid update URL");
    };
    let response = client.get(url).send()?;
    if !response.status().is_success() {
        bail!("download failed: {}", response.status());
    }
    let data = response.bytes()?;
    if data.len() as u64 != target.size {
        bail!("size mismatch");
    }
    let actual = hex::encode(Sha256::digest(&data));
    if !actual.eq_ignore_ascii_case(&target.sha256) {
        bail!("sha256 mismatch");
    }
    verify_update_signature(manifest, target_key, target)?;
    let staging = UpdateDownloadStaging::create(&filename)?;
    let mut file = staging.create_file()?;
    file.write_all(&data)?;
    file.sync_all()?;
    drop(file);
    Ok(staging.persist())
}

fn download_verified_target(
    manifest: &hbb_common::UpdateManifest,
    target_key: &str,
    target: &hbb_common::UpdateTarget,
) -> ResultType<(PathBuf, UpdateSource)> {
    validate_target_metadata(target, UPDATE_SIGNATURE_KEY_ID)?;
    try_update_sources(target, |url| {
        download_and_verify(url, manifest, target_key, target)
    })
    .map_err(|failures| {
        for failure in &failures {
            log::warn!(
                "Update {} source failed: {}: {}",
                failure.source.as_str(),
                failure.url,
                failure.error
            );
        }
        hbb_common::anyhow::anyhow!("all update sources failed")
    })
}

pub(crate) fn report_update_event(status: &str, version: &str, build_seq: u64, source: &str) {
    let _ = report_update_event_with_origin(
        status,
        crate::VERSION,
        crate::BUILD_SEQ,
        version,
        build_seq,
        source,
    );
}

fn report_command_update_event(command: Option<&UpdateCommand>, status: &str, error_code: &str) {
    if let Some(command) = command {
        report_update_command_event(command, status, error_code);
    }
}

fn report_update_command_event(command: &UpdateCommand, status: &str, error_code: &str) -> bool {
    const EVENTS_URL: &str = "https://rdapi.yan.life/rd/update/v1/events";
    let Ok(client) = create_http_client_with_url_strict(EVENTS_URL) else {
        log::warn!("Failed to create update command event HTTP client");
        return false;
    };
    let identity = current_update_client_identity();
    let payload = serde_json::json!({
        "client_id": identity.client_id,
        "client_uuid": identity.client_uuid,
        "command_id": command.command_id,
        "command_action": command.action,
        "status": status,
        "from_version": crate::VERSION,
        "from_build_seq": crate::BUILD_SEQ,
        "product": crate::PRODUCT,
        "edition": crate::EDITION,
        "build_number": crate::BUILD_NUMBER,
        "channel": crate::CHANNEL,
        "source_commit": crate::SOURCE_COMMIT,
        "to_version": command.target_version,
        "to_build_seq": command.target_build_seq,
        "error_code": error_code,
        "source": "none",
    });
    let Ok(body) = serde_json::to_vec(&payload) else {
        log::warn!("Failed to serialize update command event");
        return false;
    };
    let Ok(auth) =
        update_device_auth_headers("POST", EVENTS_URL, &identity, &command.command_id, &body)
    else {
        log::warn!("Failed to sign update command event");
        return false;
    };
    match client
        .post(EVENTS_URL)
        .header("Content-Type", "application/json")
        .header("X-RustDesk-Device-ID", auth.device_id)
        .header("X-RustDesk-Device-Public-Key", auth.public_key)
        .header("X-RustDesk-Device-Timestamp", auth.timestamp)
        .header("X-RustDesk-Device-Nonce", auth.nonce)
        .header("X-RustDesk-Device-Signature", auth.signature)
        .body(body)
        .send()
    {
        Ok(response) if response.status().is_success() => true,
        Ok(response) => {
            log::warn!(
                "Update command event rejected with HTTP {}",
                response.status()
            );
            false
        }
        Err(err) => {
            log::warn!("Failed to report update command event: {err}");
            false
        }
    }
}

pub(crate) fn report_update_event_with_origin(
    status: &str,
    from_version: &str,
    from_build_seq: u64,
    version: &str,
    build_seq: u64,
    source: &str,
) -> bool {
    let error_code = match status {
        "failed" => "all_sources_failed",
        "deferred" => "active_session",
        "rolled_back" => "install_failed",
        "rollback_failed" => "rollback_failed",
        _ => "",
    };
    report_update_event_with_origin_and_error(
        status,
        from_version,
        from_build_seq,
        version,
        build_seq,
        source,
        error_code,
    )
}

fn report_update_event_with_origin_and_error(
    status: &str,
    from_version: &str,
    from_build_seq: u64,
    version: &str,
    build_seq: u64,
    source: &str,
    error_code: &str,
) -> bool {
    #[cfg(feature = "flutter")]
    if base::update::update_event_reaches_local_ui(error_code) {
        let event = local_update_event_payload(
            status,
            version,
            build_seq,
            source,
            &update_request_origin(),
            crate::common::SOFTWARE_UPDATE_URL.lock().unwrap().clone(),
        );
        let _ = crate::flutter::push_global_event(crate::flutter::APP_TYPE_MAIN, event.to_string());
    }
    const EVENTS_URL: &str = "https://rdapi.yan.life/rd/update/v1/events";
    let Ok(client) = create_http_client_with_url_strict(EVENTS_URL) else {
        log::warn!("Failed to create update event HTTP client");
        return false;
    };
    let identity = current_update_client_identity();
    let payload = serde_json::json!({
        "client_id": identity.client_id,
        "client_uuid": identity.client_uuid,
        "status": status,
        "from_version": from_version,
        "from_build_seq": from_build_seq,
        "product": crate::PRODUCT,
        "edition": crate::EDITION,
        "build_number": crate::BUILD_NUMBER,
        "channel": crate::CHANNEL,
        "source_commit": crate::SOURCE_COMMIT,
        "to_version": version,
        "to_build_seq": build_seq,
        "error_code": error_code,
        "source": source,
    });
    let Ok(body) = serde_json::to_vec(&payload) else {
        log::warn!("Failed to serialize update event");
        return false;
    };
    let Ok(auth) = update_device_auth_headers("POST", EVENTS_URL, &identity, "", &body) else {
        log::warn!("Failed to sign update event");
        return false;
    };
    match client
        .post(EVENTS_URL)
        .header("Content-Type", "application/json")
        .header("X-RustDesk-Device-ID", auth.device_id)
        .header("X-RustDesk-Device-Public-Key", auth.public_key)
        .header("X-RustDesk-Device-Timestamp", auth.timestamp)
        .header("X-RustDesk-Device-Nonce", auth.nonce)
        .header("X-RustDesk-Device-Signature", auth.signature)
        .body(body)
        .send()
    {
        Ok(response) if response.status().is_success() => {
            use std::io::Read;
            let mut body = Vec::new();
            if response.take(4097).read_to_end(&mut body).is_err() || body.len() > 4096 {
                log::warn!("Update event acknowledgement unreadable or too large");
                return false;
            }
            match serde_json::from_slice::<serde_json::Value>(&body) {
                Ok(ack) if ack.get("ok").and_then(|value| value.as_bool()) == Some(true) => true,
                _ => {
                    log::warn!("Update event acknowledgement rejected");
                    false
                }
            }
        }
        Ok(response) => {
            log::warn!("Update event rejected with HTTP {}", response.status());
            false
        }
        Err(err) => {
            log::warn!("Failed to report update event: {}", err);
            false
        }
    }
}

pub(crate) fn report_pending_update_terminal(status: &str, event: &PendingUpdateEvent) -> bool {
    report_pending_update_terminal_with_error(
        status,
        event,
        pending_update_terminal_error_code(status),
    )
}

fn pending_update_terminal_error_code(status: &str) -> &str {
    if status == "installed" {
        ""
    } else {
        status
    }
}

fn report_pending_update_terminal_with_error(
    status: &str,
    event: &PendingUpdateEvent,
    error_code: &str,
) -> bool {
    let command_reported = if let Some(command_id) = &event.command_id {
        let state = load_pending_update_commands()
            .into_iter()
            .find(|state| state.command.command_id == *command_id);
        if state.is_none() && load_processed_update_commands().contains(command_id) {
            true
        } else if let Some(state) = state {
            let (terminal_status, terminal_error_code) = if status == "installed" {
                ("completed", "")
            } else {
                ("failed", error_code)
            };
            persist_and_report_update_command_terminal(
                &state.command,
                terminal_status,
                terminal_error_code,
            )
        } else {
            log::warn!("Missing pending update command {command_id}");
            return false;
        }
    } else {
        true
    };
    let update_reported = report_update_event_with_origin_and_error(
        status,
        &event.from_version,
        event.from_build_seq,
        &event.version,
        event.build_seq,
        event.source.as_str(),
        error_code,
    );
    update_reported && command_reported
}

#[cfg(target_os = "linux")]
fn install_linux_appimage(downloaded: &Path) -> ResultType<()> {
    let current = std::env::current_exe()?;
    let is_appimage = std::env::var_os("APPIMAGE").is_some()
        || current.extension().and_then(|ext| ext.to_str()) == Some("AppImage");
    if !is_appimage {
        bail!("running executable is not an AppImage");
    }
    replace_file_transaction(&current, downloaded, |installed| {
        std::fs::set_permissions(installed, std::fs::Permissions::from_mode(0o755))?;
        let metadata = std::fs::metadata(installed)?;
        if !metadata.is_file() || metadata.len() == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "installed AppImage is empty or not a regular file",
            ));
        }
        Ok(())
    })?;
    Ok(())
}

#[cfg(target_os = "windows")]
fn update_new_version(
    update_msi: bool,
    version: &str,
    build_seq: u64,
    source: &str,
    file_path: &PathBuf,
    command_id: Option<&str>,
    remote: bool,
) -> ResultType<()> {
    log::debug!(
        "New version is downloaded, update begin, update msi: {update_msi}, version: {version}, file: {:?}",
        file_path.to_str()
    );
    if let Some(p) = file_path.to_str() {
        if let Some(session_id) = crate::platform::get_current_process_session_id() {
            if update_msi {
                let installed = if remote {
                    crate::platform::update_me_msi_remote(p)
                } else {
                    crate::platform::update_me_msi(p, true)
                };
                match installed {
                    Ok(_) => {
                        log::debug!("New version \"{}\" updated.", version);
                        report_update_event("installed", version, build_seq, source);
                        remove_download_artifact(file_path);
                    }
                    Err(e) => {
                        log::error!(
                            "Failed to install the new msi version  \"{}\": {}",
                            version,
                            e
                        );
                        report_update_event("rolled_back", version, build_seq, source);
                        remove_download_artifact(&file_path);
                        return Err(e.into());
                    }
                }
            } else {
                let pending_event = PendingUpdateEvent {
                    transaction_id: new_update_transaction_id(),
                    from_version: crate::VERSION.to_owned(),
                    from_build_seq: crate::BUILD_SEQ,
                    version: version.to_owned(),
                    build_seq,
                    source: if source == UpdateSource::Mirror.as_str() {
                        UpdateSource::Mirror
                    } else {
                        UpdateSource::Primary
                    },
                    command_id: command_id.map(str::to_owned),
                };
                let pending_args =
                    base::update::update_launch_args(remote, &pending_event).join(" ");
                let custom_client_staging_dir = if crate::is_custom_client() {
                    let custom_client_staging_dir =
                        crate::platform::get_custom_client_staging_dir();
                    if let Err(e) = crate::platform::handle_custom_client_staging_dir_before_update(
                        &custom_client_staging_dir,
                    ) {
                        log::error!(
                            "Failed to handle custom client staging dir before update: {}",
                            e
                        );
                        remove_download_artifact(&file_path);
                        return Err(e.into());
                    }
                    Some(custom_client_staging_dir)
                } else {
                    // Clean up any residual staging directory from previous custom client
                    let staging_dir = crate::platform::get_custom_client_staging_dir();
                    hbb_common::allow_err!(crate::platform::remove_custom_client_staging_dir(
                        &staging_dir
                    ));
                    None
                };
                let update_launched = match crate::platform::launch_privileged_process(
                    session_id,
                    &format!("{} {}", p, pending_args),
                ) {
                    Ok(h) => {
                        if h.is_null() {
                            log::error!("Failed to update to the new version: {}", version);
                            report_update_event("rolled_back", version, build_seq, source);
                            false
                        } else {
                            log::debug!("New version \"{}\" is launched.", version);
                            install_state::store_pending_install_command(
                                &pending_event,
                                update_now_unix(),
                            );
                            true
                        }
                    }
                    Err(e) => {
                        log::error!("Failed to run the new version: {}", e);
                        report_update_event("rolled_back", version, build_seq, source);
                        false
                    }
                };
                if !update_launched {
                    if let Some(dir) = custom_client_staging_dir {
                        hbb_common::allow_err!(crate::platform::remove_custom_client_staging_dir(
                            &dir
                        ));
                    }
                    remove_download_artifact(&file_path);
                    bail!("failed to launch the downloaded updater");
                }
            }
        } else {
            log::error!(
                "Failed to get the current process session id, Error {}",
                std::io::Error::last_os_error()
            );
            remove_download_artifact(&file_path);
            bail!("current process session is unavailable");
        }
    } else {
        // unreachable!()
        log::error!(
            "Failed to convert the file path to string: {}",
            file_path.display()
        );
        remove_download_artifact(file_path);
        bail!("downloaded update path is not valid UTF-8");
    }
    Ok(())
}

pub fn get_update_download_file_from_url(url: &str) -> Option<PathBuf> {
    let parsed = url::Url::parse(url).ok()?;
    // Check the raw prefix before Url normalizes default ports.
    if !url.starts_with("https://github.com/")
        || parsed.scheme() != "https"
        || parsed.host_str() != Some("github.com")
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.port().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return None;
    }

    let mut segments = parsed.path_segments()?;
    let owner = segments.next()?;
    let repo = segments.next()?;
    let releases = segments.next()?;
    let download = segments.next()?;
    let tag = segments.next()?;
    let filename = segments.next()?;

    if owner != "rustdesk"
        || repo != "rustdesk"
        || releases != "releases"
        || download != "download"
        || tag.is_empty()
        || segments.next().is_some()
        || !is_plain_update_filename(filename)
    {
        return None;
    }

    Some(std::env::temp_dir().join(filename))
}

fn is_plain_update_filename(filename: &str) -> bool {
    if filename.is_empty()
        || filename.contains('/')
        || filename.contains('\\')
        || filename.contains(':')
    {
        return false;
    }

    let mut components = Path::new(filename).components();
    matches!(
        components.next(),
        Some(Component::Normal(name)) if name.to_str() == Some(filename)
    ) && components.next().is_none()
}

pub fn get_download_file_from_url(url: &str) -> Option<PathBuf> {
    get_update_download_file_from_url(url)
}

/// Queries all active connections (remote, file-transfer, port-forward, camera, terminal)
/// from every logged-in user's --server process via IPC.
/// The root service cannot read connection state directly since connections
/// live in user --server processes. Handles fast user switching by querying
/// all GUI users, including the login-window server at UID 0. A missing
/// `--server` process is treated as idle; a live server with broken IPC stays
/// fail-closed to avoid updating during an unknown session state.
#[cfg(any(test, target_os = "macos"))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MacIpcProbe {
    NoActiveConnections,
    ActiveConnections,
    Unavailable,
}

#[cfg(any(test, target_os = "macos"))]
fn mac_update_allowed(server_running: bool, probe: MacIpcProbe) -> bool {
    match probe {
        MacIpcProbe::NoActiveConnections => true,
        MacIpcProbe::ActiveConnections => false,
        MacIpcProbe::Unavailable => !server_running,
    }
}

#[cfg(target_os = "macos")]
fn mac_server_process_running(uid: u32) -> bool {
    let Ok(output) = std::process::Command::new("/usr/bin/pgrep")
        .args(["-u", &uid.to_string(), "-x", &crate::get_app_name()])
        .output()
    else {
        return false;
    };
    if !output.status.success() {
        return false;
    }
    String::from_utf8_lossy(&output.stdout).lines().any(|pid| {
        std::process::Command::new("/bin/ps")
            .args(["-p", pid.trim(), "-o", "args="])
            .output()
            .ok()
            .map(|args| {
                String::from_utf8_lossy(&args.stdout)
                    .split_whitespace()
                    .any(|arg| arg == "--server")
            })
            .unwrap_or(false)
    })
}

#[cfg(target_os = "macos")]
pub fn has_no_active_conns_ipc() -> bool {
    let rt = match hbb_common::tokio::runtime::Runtime::new() {
        Ok(rt) => rt,
        Err(_) => return false,
    };
    rt.block_on(async {
        // Use the same GUI-domain-filtered UID set as the update script.
        // Shell-only SSH/TTY users are excluded, while an empty GUI set maps
        // to UID 0 so the LoginWindow server is queried rather than assumed idle.
        let uids = crate::platform::get_logged_in_uids();
        // Check each user's server — fail closed if any has active connections
        for uid in uids {
            let server_running = mac_server_process_running(uid);
            if let Ok(mut conn) = crate::ipc::connect_for_uid(1000, uid, "").await {
                if conn
                    .send(&crate::ipc::Data::HasNoActiveConns(None))
                    .await
                    .is_ok()
                {
                    match conn.next_timeout(1000).await {
                        Ok(Some(crate::ipc::Data::HasNoActiveConns(Some(true)))) => {
                            log::info!(
                                "[root-update] session probe uid={} server_running={} result=idle",
                                uid,
                                server_running
                            );
                        }
                        Ok(Some(crate::ipc::Data::HasNoActiveConns(Some(false)))) => {
                            log::info!(
                                "[root-update] session probe uid={} server_running={} result=active",
                                uid,
                                server_running
                            );
                            return false;
                        }
                        _ => {
                            log::warn!(
                                "[root-update] session probe uid={} server_running={} result=invalid-response",
                                uid,
                                server_running
                            );
                            return false;
                        }
                    }
                } else {
                    log::warn!(
                        "[root-update] session probe uid={} server_running={} result=send-failed",
                        uid,
                        server_running
                    );
                    return false;
                }
            } else {
                log::info!(
                    "[root-update] session probe uid={} server_running={} result=unavailable",
                    uid,
                    server_running
                );
                if !mac_update_allowed(server_running, MacIpcProbe::Unavailable) {
                    return false;
                }
                continue;
            }
        }
        true // All users explicitly confirmed no active connections
    })
}

#[cfg(target_os = "macos")]
fn wait_for_failed_update_retry() {
    const FAILURE_MARKER: &str = "/var/root/.rustdeskupdate_failed";
    let marker = std::path::Path::new(FAILURE_MARKER);
    if !marker.exists() {
        return;
    }

    // The updater script records failure immediately before launchd restarts
    // the old daemon. Preserve the retry deadline across that restart instead
    // of consuming the marker and retrying the same broken release in 30 sec.
    let remaining = std::fs::metadata(marker)
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(|modified| std::time::SystemTime::now().duration_since(modified).ok())
        .map(|elapsed| RETRY_INTERVAL.saturating_sub(elapsed))
        .unwrap_or(RETRY_INTERVAL);
    if !remaining.is_zero() {
        log::info!(
            "[root-update] Previous update failed; retrying in {} seconds.",
            remaining.as_secs()
        );
        std::thread::sleep(remaining);
    }
    match std::fs::remove_file(marker) {
        Ok(()) => log::info!("[root-update] Previous update retry interval elapsed."),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => log::warn!("[root-update] Failed to clear failure marker: {}", err),
    }
}

/// Starts the background silent auto-update scheduler for macOS.
/// Called from `start_os_service()` which runs as root via LaunchDaemon.
#[cfg(target_os = "macos")]
pub fn start_auto_update_macos() {
    start_update_policy_stream();
    let (schedule_tx, schedule_rx) = channel();
    *MAC_SCHEDULER_WAKE.lock().unwrap() = Some(schedule_tx);
    let spawn_result = std::thread::Builder::new()
        .name("rustdesk-auto-update".to_owned())
        .spawn(move || {
            log::info!("[root-update] Auto-update scheduler thread started.");
            consume_mac_update_result();
            wait_for_failed_update_retry();
            let mut next_delay = Some(INITIAL_CHECK_DELAY);
            loop {
                if wait_for_mac_schedule_change(&schedule_rx, next_delay) {
                    log::info!("[root-update] Update policy changed; recalculating schedule.");
                    next_delay = scheduled_update_interval();
                    continue;
                }
                if scheduled_update_interval().is_none() {
                    next_delay = None;
                    continue;
                }
                log::info!("[root-update] Running scheduled update check...");
                let no_active_conns = has_no_active_conns_ipc();
                next_delay = if !no_active_conns {
                    log::info!("[root-update] Active session in progress, retrying in 10 min.");
                    Some(MIN_INTERVAL)
                } else {
                    match check_update_as_root() {
                        Ok(update_started) => {
                            if update_started {
                                // The replacement script is detached and may fail
                                // after this process returns. Always retry at the
                                // failure interval until the new daemon replaces us.
                                Some(RETRY_INTERVAL)
                            } else {
                                scheduled_update_interval()
                            }
                        }
                        Err(e) => {
                            log::error!("[root-update] Update check failed: {}", e);
                            Some(RETRY_INTERVAL)
                        }
                    }
                };
            }
        });
    if let Err(err) = spawn_result {
        log::error!("[root-update] Failed to start scheduler thread: {}", err);
    }
}

#[cfg(target_os = "macos")]
fn wait_for_mac_schedule_change(rx: &Receiver<()>, interval: Option<Duration>) -> bool {
    match interval {
        Some(interval) => rx.recv_timeout(interval).is_ok(),
        None => rx.recv().is_ok(),
    }
}

#[cfg(target_os = "macos")]
fn consume_mac_update_result() {
    for _ in 0..600 {
        let Some((result, claimed)) = crate::platform::consume_update_result() else {
            return;
        };
        if result.status == "pending" {
            std::thread::sleep(Duration::from_secs(1));
            continue;
        }
        if report_pending_update_terminal(&result.status, &result.event) {
            if !claimed {
                log::warn!("[root-update] terminal update result was not atomically claimed");
                return;
            }
            if let Err(err) = crate::platform::clear_claimed_update_result() {
                log::warn!("[root-update] Failed to clear reported update result: {err}");
            }
            return;
        }
        std::thread::sleep(Duration::from_secs(1));
    }
    log::warn!("[root-update] Timed out waiting for detached update result");
}

#[cfg(target_os = "macos")]
pub fn check_update_as_root() -> ResultType<bool> {
    // Allow-auto-update setting
    if !update_option_enabled(&config::Config::get_option(keys::OPTION_ALLOW_AUTO_UPDATE)) {
        log::info!("[root-update] Auto update is disabled, skipping.");
        return Ok(false);
    }
    // Clean up only old temp dirs from previous failed updates. The detached
    // installer keeps using its update directory after this process exits and
    // releases the advisory lock, so a newly-started daemon must not remove a
    // directory that still belongs to the active transaction.
    if let Ok(entries) = std::fs::read_dir("/tmp") {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if name_str.starts_with(".rustdeskupdate-root-")
                || name_str.starts_with(".rustdeskdownload-")
                || name_str.starts_with(UPDATE_DOWNLOAD_DIR_PREFIX)
            {
                let path = entry.path();
                let Ok(metadata) = std::fs::symlink_metadata(&path) else {
                    continue;
                };
                let mode = metadata.mode() & 0o7777;
                let is_stale = metadata
                    .modified()
                    .ok()
                    .and_then(|modified| std::time::SystemTime::now().duration_since(modified).ok())
                    .is_some_and(|age| age >= RETRY_INTERVAL);
                if metadata.file_type().is_dir() && metadata.uid() == 0 && mode == 0o700 && is_stale
                {
                    if let Err(err) = std::fs::remove_dir_all(&path) {
                        log::warn!(
                            "[root-update] Failed to remove stale temp dir {}: {}",
                            path.display(),
                            err
                        );
                    }
                }
            }
        }
    }
    if let Err(e) = do_check_software_update() {
        bail!("[root-update] Failed to check for software update: {}", e);
    }
    let response = crate::common::SOFTWARE_UPDATE_RESPONSE
        .lock()
        .unwrap()
        .clone();
    let Some(response) = response else {
        log::info!("[root-update] No update available.");
        return Ok(false);
    };
    let Some(ref manifest) = response.manifest else {
        return Ok(false);
    };
    let target_key = crate::common::SOFTWARE_UPDATE_TARGET_KEY
        .lock()
        .unwrap()
        .clone();
    if target_key.is_empty() {
        bail!("[root-update] update target selection is missing");
    }
    validate_manifest_contract(
        &response,
        &manifest,
        crate::VERSION,
        crate::BUILD_SEQ,
        crate::PRODUCT,
        crate::EDITION,
        crate::CHANNEL,
        &target_key,
    )?;
    let target = manifest.targets.get(&target_key).cloned();
    let Some(target) = target else {
        return Ok(false);
    };
    let (target_version, target_build_seq, _) = update_target_release(manifest, &target)?;
    let action = decide_update_action(
        &response,
        update_option_enabled(&config::Config::get_option(keys::OPTION_ALLOW_AUTO_UPDATE)),
    );
    if action != UpdateAction::AutoInstall {
        return Ok(false);
    }
    let version = target_version.to_owned();
    report_update_event("started", &version, target_build_seq, "none");
    let (file_path, source) = match download_verified_target(manifest, &target_key, &target) {
        Ok(downloaded) => downloaded,
        Err(err) => {
            report_update_event("failed", &version, target_build_seq, "none");
            return Err(err);
        }
    };
    let _download_cleanup = DownloadArtifactCleanup(file_path.clone());
    let source = source.as_str();
    report_update_event("downloaded", &version, target_build_seq, source);
    // Use mktemp so a local user cannot pre-create a predictable path and
    // permanently deny updates for a reused service PID.
    let private_tmp_output = std::process::Command::new("/usr/bin/mktemp")
        .args(["-d", "/tmp/.rustdeskdownload-XXXXXX"])
        .output()?;
    if !private_tmp_output.status.success() {
        bail!(
            "[root-update] Failed to create private download directory: {}",
            String::from_utf8_lossy(&private_tmp_output.stderr).trim()
        );
    }
    let private_tmp = String::from_utf8(private_tmp_output.stdout)
        .map_err(|err| hbb_common::anyhow::anyhow!("[root-update] mktemp output error: {}", err))?
        .trim()
        .to_owned();
    if private_tmp.is_empty() {
        bail!("[root-update] mktemp returned an empty download directory");
    }
    let _private_tmp_cleanup = UpdateDirectoryCleanup(PathBuf::from(&private_tmp));
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&private_tmp, std::fs::Permissions::from_mode(0o700))?;
    }
    let filename = file_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("rustdesk.dmg");
    let staged_path = std::path::PathBuf::from(format!("{}/{}", private_tmp, filename));
    std::fs::rename(&file_path, &staged_path)?;
    let tmp_path = staged_path.to_string_lossy().to_string();
    log::info!("[root-update] Downloaded to {}", tmp_path);
    // Recheck active sessions before installing — download can take minutes
    if !has_no_active_conns_ipc() {
        if let Err(e) = std::fs::remove_dir_all(&private_tmp) {
            log::warn!(
                "[root-update] Failed to remove temp dir {}: {}",
                private_tmp,
                e
            );
        }
        report_update_event("deferred", &version, target_build_seq, source);
        bail!("[root-update] Active session started during download, deferring update.");
    }
    // Install silently as root
    report_update_event("installing", &version, target_build_seq, source);
    let event = PendingUpdateEvent {
        transaction_id: new_update_transaction_id(),
        from_version: crate::VERSION.to_owned(),
        from_build_seq: crate::BUILD_SEQ,
        version: version.clone(),
        build_seq: target_build_seq,
        source: if source == UpdateSource::Mirror.as_str() {
            UpdateSource::Mirror
        } else {
            UpdateSource::Primary
        },
        command_id: None,
    };
    let result = crate::platform::update_from_dmg_as_root(&tmp_path, &version, &event, 0);
    // Clean up download directory
    if let Err(e) = std::fs::remove_dir_all(&private_tmp) {
        log::warn!(
            "[root-update] Failed to remove temp dir {}: {}",
            private_tmp,
            e
        );
    }
    match result {
        Ok(_) => Ok(true),
        Err(err) => {
            report_update_event(
                classify_update_preinstall_failure(&err.to_string()),
                &version,
                target_build_seq,
                source,
            );
            Err(err)
        }
    }
}

#[cfg(test)]
mod tests {
    #[cfg(target_os = "macos")]
    use super::wait_for_mac_schedule_change;
    use super::UpdateCommand;
    use super::{
        get_download_file_from_url, install_command_already_applied, install_command_satisfied,
        release_notice_should_check, scheduled_update_delay, update_filename_from_url,
        verify_detached_signature, UpdateCommandAction, UpdateDownloadStaging,
    };
    use std::time::Duration;

    fn release_notice(
        build_seq: u64,
        channel: &str,
        device_build_seq: Option<u64>,
    ) -> base::update::ReleaseNotice {
        base::update::ReleaseNotice {
            version: "1.5.4".into(),
            build_seq,
            channel: channel.into(),
            device_build_seq,
        }
    }

    fn install_command(version: Option<&str>, build_seq: Option<u64>) -> UpdateCommand {
        UpdateCommand {
            command_id: "c".repeat(32),
            action: UpdateCommandAction::Install,
            client_id: "1".repeat(10),
            client_uuid: "u".into(),
            target_version: version.map(str::to_owned),
            target_build_seq: build_seq,
            expires_at: i64::MAX,
        }
    }

    // 安装命令会在安装过程中终止本进程，终态来不及上报。重启后恢复该命令时，
    // 若运行的已经是目标版本，说明安装已完成；此时再走一次检查更新只会得到
    // 「无更新」，把已完成的安装改写成 no_update。
    #[test]
    fn install_command_already_applied_is_satisfied_by_the_running_build() {
        for (running_version, running_build) in [("1.5.4", 2026102402), ("1.5.5", 1)] {
            assert!(
                install_command_satisfied(
                    &install_command(Some("1.5.4"), Some(2026102402)),
                    running_version,
                    running_build,
                ),
                "{running_version}+{running_build}"
            );
        }
    }

    #[test]
    fn install_command_with_a_newer_target_is_not_satisfied() {
        assert!(!install_command_satisfied(
            &install_command(Some("1.5.4"), Some(2026102404)),
            "1.5.4",
            2026102402,
        ));
    }

    // 目标版本相符但本机从未发起过这次安装，说明这条命令不是本机装的。
    // 此时必须继续走目标与签名校验，不能直接记成安装完成。
    #[test]
    fn an_install_command_this_machine_never_launched_is_not_already_applied() {
        assert!(!install_command_already_applied(
            &install_command(Some("1.5.4"), Some(2026102402)),
            "1.5.4",
            2026102402,
            false,
        ));
    }

    // 本机发起过、且运行的已经是目标版本，才判为已完成。
    #[test]
    fn an_install_command_this_machine_launched_is_already_applied() {
        assert!(install_command_already_applied(
            &install_command(Some("1.5.4"), Some(2026102402)),
            "1.5.4",
            2026102402,
            true,
        ));
    }

    // 本机发起过但运行版本没跟上，说明安装没生效，必须继续走检查路径。
    #[test]
    fn an_install_command_this_machine_launched_but_not_effective_is_not_applied() {
        assert!(!install_command_already_applied(
            &install_command(Some("1.5.4"), Some(2026102404)),
            "1.5.4",
            2026102402,
            true,
        ));
    }

    // 检查更新命令即使目标版本相符、即使本机有同名记录，也不能被短路。
    #[test]
    fn a_check_command_is_never_already_applied() {
        let mut command = install_command(Some("1.5.4"), Some(2026102402));
        command.action = UpdateCommandAction::Check;
        assert!(!install_command_already_applied(
            &command, "1.5.4", 2026102402, true,
        ));
    }

    // 检查更新命令必须每次都真的去查，不能被「已是最新」短路。
    #[test]
    fn check_command_is_never_satisfied_by_the_running_build() {
        let mut command = install_command(Some("1.5.4"), Some(2026102402));
        command.action = UpdateCommandAction::Check;
        assert!(!install_command_satisfied(&command, "1.5.4", 2026102402));
    }

    // 发布广播只是「有新版本了」的提示，必须完全受后台开关约束；关掉后台更新的
    // 机器不能因为收到广播就去检查，否则等于绕过了用户的设置。
    #[test]
    fn a_release_notice_does_not_wake_a_disabled_background_updater() {
        assert!(!release_notice_should_check(
            &release_notice(2026102405, "", None),
            2026102404,
            false,
        ));
    }

    // 广播的构建号不比本机新时不必再查一次；相同或更旧都跳过。
    #[test]
    fn an_older_release_notice_is_ignored() {
        for build in [2026102404, 2026102403] {
            assert!(
                !release_notice_should_check(&release_notice(build, "", None), 2026102404, true),
                "{build}"
            );
        }
    }

    // 通道不符的广播与运行在 stable 的客户端无关。
    #[test]
    fn a_release_notice_for_another_channel_is_ignored() {
        assert!(!release_notice_should_check(
            &release_notice(2026102405, "beta", None),
            2026102404,
            true,
        ));
    }

    // 后端判定时看到的本机构建号与现在不同，说明这条广播已经过期。
    #[test]
    fn a_release_notice_for_a_stale_device_build_is_ignored() {
        assert!(!release_notice_should_check(
            &release_notice(2026102405, "", Some(2026102402)),
            2026102404,
            true,
        ));
    }

    // 正常情形：后台更新开启、构建号更新、通道与本机一致，才去检查。
    #[test]
    fn a_newer_release_notice_on_the_same_channel_triggers_a_check() {
        assert!(release_notice_should_check(
            &release_notice(2026102405, "", Some(2026102404)),
            2026102404,
            true,
        ));
    }

    #[test]
    fn install_command_without_a_target_is_not_satisfied() {
        for command in [
            install_command(None, Some(2026102402)),
            install_command(Some("1.5.4"), None),
            install_command(None, None),
        ] {
            assert!(!install_command_satisfied(&command, "1.5.4", 2026102402));
        }
    }

    #[test]
    fn update_download_file_accepts_expected_github_asset_urls() {
        let file = get_download_file_from_url(
            "https://github.com/rustdesk/rustdesk/releases/download/1.4.0/rustdesk-1.4.0-x86_64.dmg",
        )
        .expect("valid GitHub release asset URL");

        assert_eq!(
            file.file_name().and_then(|name| name.to_str()),
            Some("rustdesk-1.4.0-x86_64.dmg")
        );
    }

    #[test]
    fn update_download_file_rejects_untrusted_or_malformed_urls() {
        for url in [
            "http://github.com/rustdesk/rustdesk/releases/download/1/rustdesk.exe",
            "https://example.com/rustdesk.exe",
            "https://github.com/other/project/releases/download/1/rustdesk.exe",
            "https://github.com/rustdesk/rustdesk/releases/download/1/",
            "https://github.com/rustdesk/rustdesk/releases/download/1/nested/rustdesk.exe",
            "https://github.com/rustdesk/rustdesk/releases/download/1/C:rustdesk.exe",
            "https://user@github.com/rustdesk/rustdesk/releases/download/1/rustdesk.exe",
            "https://github.com:443/rustdesk/rustdesk/releases/download/1/rustdesk.exe",
            "https://github.com/rustdesk/rustdesk/releases/download/1/rustdesk.exe?download=1",
            "https://github.com/rustdesk/rustdesk/releases/download/1/rustdesk.exe#download",
            "not a url",
        ] {
            assert!(get_download_file_from_url(url).is_none(), "{url}");
        }
    }

    #[test]
    fn update_filename_rejects_non_plain_final_path_segments() {
        assert_eq!(
            update_filename_from_url("https://download.yan.life/stable/rustdesk.exe"),
            Some("rustdesk.exe".to_owned())
        );
        for url in [
            "https://download.yan.life/stable/",
            "https://download.yan.life/stable/C:rustdesk.exe",
            "not a url",
        ] {
            assert!(update_filename_from_url(url).is_none(), "{url}");
        }
    }

    #[test]
    fn update_download_staging_is_unique_and_cleans_up_on_drop() {
        let first = UpdateDownloadStaging::create("rustdesk.exe").expect("first staging dir");
        let second = UpdateDownloadStaging::create("rustdesk.exe").expect("second staging dir");
        let first_dir = first.dir.clone();
        let second_dir = second.dir.clone();

        assert_ne!(first_dir, second_dir);
        assert!(first_dir.is_dir());
        assert!(second_dir.is_dir());
        drop(first);
        drop(second);
        assert!(!first_dir.exists());
        assert!(!second_dir.exists());
    }

    #[test]
    fn update_download_file_creation_is_exclusive() {
        let staging = UpdateDownloadStaging::create("rustdesk.exe").expect("staging dir");
        let dir = staging.dir.clone();
        let file = staging.create_file().expect("new file");
        assert!(staging.create_file().is_err());
        drop(file);
        drop(staging);
        assert!(!dir.exists());
    }

    #[cfg(unix)]
    #[test]
    fn update_download_staging_has_private_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let staging = UpdateDownloadStaging::create("rustdesk.dmg").expect("staging dir");
        let file = staging.create_file().expect("new file");
        let dir_mode = std::fs::metadata(&staging.dir)
            .expect("staging metadata")
            .permissions()
            .mode()
            & 0o777;
        let file_mode = file.metadata().expect("file metadata").permissions().mode() & 0o777;

        assert_eq!(dir_mode, 0o700);
        assert_eq!(file_mode, 0o600);
    }

    #[test]
    fn detached_update_signature_accepts_valid_bytes_and_rejects_tampering() {
        let (public_key, secret_key) = hbb_common::sodiumoxide::crypto::sign::gen_keypair();
        let data = b"rustdesk-yan update fixture";
        let signature = hbb_common::sodiumoxide::crypto::sign::sign_detached(data, &secret_key);
        assert!(verify_detached_signature(data, signature.as_ref(), public_key.as_ref()).is_ok());
        assert!(
            verify_detached_signature(b"tampered", signature.as_ref(), public_key.as_ref())
                .is_err()
        );
        let (other_public_key, _) = hbb_common::sodiumoxide::crypto::sign::gen_keypair();
        assert!(
            verify_detached_signature(data, signature.as_ref(), other_public_key.as_ref()).is_err()
        );
    }

    #[test]
    fn scheduled_update_delay_honors_policy_and_bounds() {
        assert_eq!(scheduled_update_delay(false, 5), None);
        assert_eq!(
            scheduled_update_delay(true, 1),
            Some(Duration::from_secs(60 * 60))
        );
        assert_eq!(
            scheduled_update_delay(true, 5),
            Some(Duration::from_secs(5 * 60 * 60))
        );
        assert_eq!(
            scheduled_update_delay(true, 168),
            Some(Duration::from_secs(168 * 60 * 60))
        );
        assert_eq!(
            scheduled_update_delay(true, 0),
            Some(Duration::from_secs(60 * 60))
        );
        assert_eq!(
            scheduled_update_delay(true, 999),
            Some(Duration::from_secs(168 * 60 * 60))
        );
    }

    #[test]
    fn connectivity_restore_only_checks_when_background_checks_are_enabled() {
        assert!(!super::should_check_on_connectivity_restored(false, false));
        assert!(super::should_check_on_connectivity_restored(true, false));
        assert!(super::should_check_on_connectivity_restored(false, true));
        assert!(super::should_check_on_connectivity_restored(true, true));
    }

    #[test]
    fn successful_pending_update_has_no_error_code() {
        assert_eq!(super::pending_update_terminal_error_code("installed"), "");
        assert_eq!(
            super::pending_update_terminal_error_code("rolled_back"),
            "rolled_back"
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn mac_scheduler_wakes_when_policy_changes() {
        let (tx, rx) = std::sync::mpsc::channel();
        tx.send(()).expect("policy wake should be delivered");
        assert!(wait_for_mac_schedule_change(
            &rx,
            Some(Duration::from_secs(60 * 60))
        ));
    }
}
