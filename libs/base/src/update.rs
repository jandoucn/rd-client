use hbb_common::{is_newer_version, UpdateManifest, VersionCheckResponse};
use std::{fs, io, path::Path};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateAction {
    Ignore,
    Notify,
    Download,
    AutoInstall,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde_derive::Deserialize, serde_derive::Serialize)]
pub enum UpdateSource {
    Primary,
    Mirror,
}

pub const DEFAULT_SCHEDULED_UPDATE_INTERVAL_HOURS: u64 = 5;
pub const MIN_SCHEDULED_UPDATE_INTERVAL_HOURS: u64 = 1;
pub const MAX_SCHEDULED_UPDATE_INTERVAL_HOURS: u64 = 24 * 7;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateClientIdentity {
    pub client_id: String,
    pub client_uuid: String,
}

pub fn update_client_identity(client_id: &str, client_uuid: &str) -> UpdateClientIdentity {
    UpdateClientIdentity {
        client_id: client_id.to_owned(),
        client_uuid: client_uuid.to_owned(),
    }
}

pub fn update_option_enabled(value: &str) -> bool {
    value == "Y"
}

#[derive(Debug, Clone, PartialEq, Eq, serde_derive::Deserialize)]
pub struct UpdatePolicy {
    pub client_id: String,
    pub client_uuid: String,
    #[serde(rename = "policy_revision")]
    pub revision: u64,
    #[serde(rename = "enable_check_update")]
    pub check_on_startup: bool,
    #[serde(rename = "allow_auto_update")]
    pub auto_update: bool,
    #[serde(rename = "enable_scheduled_update", default)]
    pub scheduled_update: bool,
    #[serde(
        rename = "scheduled_update_interval_hours",
        default = "default_scheduled_update_interval_hours"
    )]
    pub scheduled_update_interval_hours: u64,
    #[serde(
        rename = "management_agent_enabled",
        default = "default_management_agent_enabled"
    )]
    pub management_agent_enabled: bool,
}

pub const fn default_scheduled_update_interval_hours() -> u64 {
    DEFAULT_SCHEDULED_UPDATE_INTERVAL_HOURS
}

pub const fn default_management_agent_enabled() -> bool {
    true
}

pub fn normalize_scheduled_update_interval_hours(hours: u64) -> u64 {
    hours.clamp(
        MIN_SCHEDULED_UPDATE_INTERVAL_HOURS,
        MAX_SCHEDULED_UPDATE_INTERVAL_HOURS,
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde_derive::Deserialize, serde_derive::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum UpdateCommandAction {
    Check,
    Install,
}

#[derive(Debug, Clone, PartialEq, Eq, serde_derive::Deserialize, serde_derive::Serialize)]
pub struct UpdateCommand {
    pub command_id: String,
    pub action: UpdateCommandAction,
    pub client_id: String,
    pub client_uuid: String,
    pub target_version: Option<String>,
    pub target_build_seq: Option<u64>,
    pub expires_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateStreamEvent {
    Policy(UpdatePolicy),
    Command(UpdateCommand),
    /// 服务端发布了一个新版本。它不是一条命令，客户端只据此知道「有新版了」，
    /// 是否检查、是否安装仍由本机策略和 mode 决定。
    ReleasePublished(ReleaseNotice),
}

/// 一次发布广播。`build_seq` 是服务端最新生效版本，`device_build_seq` 是广播
/// 判定时后端看到的本机构建号，用来识别「本机已经变了，这条广播是过期的」。
#[derive(Debug, Clone, PartialEq, Eq, serde_derive::Deserialize, serde_derive::Serialize)]
pub struct ReleaseNotice {
    pub version: String,
    pub build_seq: u64,
    #[serde(default)]
    pub channel: String,
    #[serde(default)]
    pub device_build_seq: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateCommandDecision {
    Execute,
    Duplicate,
    Expired,
    IdentityMismatch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateCommandOutcome {
    NoUpdate,
    CheckCompleted,
    AwaitingInstallResult,
    Installed,
}

pub fn command_outcome(
    action: UpdateCommandAction,
    update_available: bool,
    asynchronous_install: bool,
) -> UpdateCommandOutcome {
    if !update_available {
        UpdateCommandOutcome::NoUpdate
    } else if action == UpdateCommandAction::Check {
        UpdateCommandOutcome::CheckCompleted
    } else if asynchronous_install {
        UpdateCommandOutcome::AwaitingInstallResult
    } else {
        UpdateCommandOutcome::Installed
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde_derive::Deserialize, serde_derive::Serialize)]
pub struct UpdateCommandTerminal {
    pub status: String,
    pub error_code: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde_derive::Deserialize, serde_derive::Serialize)]
pub struct UpdateCommandState {
    pub command: UpdateCommand,
    pub terminal: Option<UpdateCommandTerminal>,
    #[serde(default)]
    pub acked: bool,
    #[serde(default)]
    pub deferred_reported: bool,
}

impl UpdateCommandState {
    pub fn pending(command: UpdateCommand) -> Self {
        Self {
            command,
            terminal: None,
            acked: false,
            deferred_reported: false,
        }
    }

    pub fn mark_deferred_reported(&mut self) -> bool {
        if self.deferred_reported {
            false
        } else {
            self.deferred_reported = true;
            true
        }
    }

    pub fn set_terminal(&mut self, status: &str, error_code: &str) {
        self.terminal = Some(UpdateCommandTerminal {
            status: status.to_owned(),
            error_code: error_code.to_owned(),
        });
        self.acked = false;
    }

    pub fn ack_terminal(&mut self) {
        self.acked = true;
    }
}

#[derive(
    Debug, Clone, Default, PartialEq, Eq, serde_derive::Deserialize, serde_derive::Serialize,
)]
pub struct ProcessedUpdateCommands {
    pub ids: Vec<String>,
}

impl ProcessedUpdateCommands {
    const LIMIT: usize = 128;

    pub fn contains(&self, command_id: &str) -> bool {
        self.ids.iter().any(|value| value == command_id)
    }

    pub fn remember(&mut self, command_id: String) {
        if self.contains(&command_id) {
            return;
        }
        self.ids.push(command_id);
        if self.ids.len() > Self::LIMIT {
            self.ids.drain(..self.ids.len() - Self::LIMIT);
        }
    }
}

pub fn update_device_auth_payload(
    method: &str,
    path: &str,
    client_id: &str,
    client_uuid: &str,
    timestamp: i64,
    nonce: &str,
    command_id: &str,
    body_sha256: &str,
) -> Vec<u8> {
    format!(
        concat!(
            "rustdesk-update-auth-v1\n",
            "method={}\n",
            "path={}\n",
            "client_id={}\n",
            "client_uuid={}\n",
            "timestamp={}\n",
            "nonce={}\n",
            "command_id={}\n",
            "body_sha256={}\n",
        ),
        method, path, client_id, client_uuid, timestamp, nonce, command_id, body_sha256,
    )
    .into_bytes()
}

impl UpdateCommand {
    pub fn decision(
        &self,
        identity: &UpdateClientIdentity,
        now: i64,
        already_seen: bool,
    ) -> UpdateCommandDecision {
        if !self.matches_identity(identity) {
            UpdateCommandDecision::IdentityMismatch
        } else if already_seen {
            UpdateCommandDecision::Duplicate
        } else if self.expires_at <= now {
            UpdateCommandDecision::Expired
        } else {
            UpdateCommandDecision::Execute
        }
    }

    pub fn accepted_decision(
        &self,
        identity: &UpdateClientIdentity,
        already_processed: bool,
    ) -> UpdateCommandDecision {
        if !self.matches_identity(identity) {
            UpdateCommandDecision::IdentityMismatch
        } else if already_processed {
            UpdateCommandDecision::Duplicate
        } else {
            UpdateCommandDecision::Execute
        }
    }

    pub fn matches_identity(&self, identity: &UpdateClientIdentity) -> bool {
        self.client_id == identity.client_id && self.client_uuid == identity.client_uuid
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolicyDecision {
    Apply,
    IgnoreStale,
}

impl UpdatePolicy {
    pub fn decision(&self, applied_revision: Option<u64>) -> PolicyDecision {
        if applied_revision.map_or(true, |revision| self.revision > revision) {
            PolicyDecision::Apply
        } else {
            PolicyDecision::IgnoreStale
        }
    }

    pub fn matches_identity(&self, identity: &UpdateClientIdentity) -> bool {
        self.client_id == identity.client_id && self.client_uuid == identity.client_uuid
    }
}

pub fn update_policy_stream_url(
    base_url: &str,
    identity: &UpdateClientIdentity,
    after_revision: Option<u64>,
) -> String {
    let mut url = format!(
        "{}/rd/update/v1/policy/stream?client_id={}&client_uuid={}",
        base_url.trim_end_matches('/'),
        identity.client_id,
        identity.client_uuid,
    );
    if let Some(revision) = after_revision {
        url.push_str(&format!("&after_revision={revision}"));
    }
    url
}

pub fn policy_resume_revision(
    stored_client_uuid: &str,
    current_client_uuid: &str,
    stored_revision: Option<u64>,
) -> Option<u64> {
    (stored_client_uuid == current_client_uuid)
        .then_some(stored_revision)
        .flatten()
}

pub fn should_run_startup_update(check_on_startup: bool) -> bool {
    check_on_startup
}

pub fn should_run_scheduled_update(scheduled_update: bool) -> bool {
    scheduled_update
}

// The startup-check switch lives in the machine-wide Config so the service that
// runs the background updater sees what the UI stored. Older builds kept it in
// the per-user LocalConfig; that copy only fills in while the global one is unset.
pub fn is_global_check_on_startup_option(key: &str) -> bool {
    key == crate::config::keys::OPTION_ENABLE_CHECK_UPDATE
}

pub fn check_on_startup_enabled(global: &str, legacy: &str) -> bool {
    update_option_enabled(if global.is_empty() { legacy } else { global })
}

pub fn resolve_check_on_startup_option(
    global: &str,
    legacy: &str,
    writable: bool,
    write_global: impl FnOnce(&str),
    clear_legacy: impl FnOnce(),
) -> String {
    if !global.is_empty() {
        if writable && !legacy.is_empty() {
            clear_legacy();
        }
        return global.to_owned();
    }
    if writable && !legacy.is_empty() {
        write_global(legacy);
        clear_legacy();
    }
    legacy.to_owned()
}

pub fn store_check_on_startup_option(
    value: &str,
    write_global: impl FnOnce(&str),
    clear_legacy: impl FnOnce(),
) {
    write_global(value);
    clear_legacy();
}

pub fn parse_update_policy_sse_event(event: &str) -> anyhow::Result<Option<UpdatePolicy>> {
    match parse_update_stream_sse_event(event)? {
        Some(UpdateStreamEvent::Policy(policy)) => Ok(Some(policy)),
        _ => Ok(None),
    }
}

pub fn parse_update_stream_sse_event(event: &str) -> anyhow::Result<Option<UpdateStreamEvent>> {
    let mut event_name = None;
    let mut event_id = None;
    let mut data = String::new();
    for line in event.lines() {
        if line.starts_with(':') {
            continue;
        }
        if let Some(value) = line.strip_prefix("event:") {
            event_name = Some(value.trim());
        } else if let Some(value) = line.strip_prefix("id:") {
            event_id = Some(value.trim());
        } else if let Some(value) = line.strip_prefix("data:") {
            if !data.is_empty() {
                data.push('\n');
            }
            data.push_str(value.trim_start());
        }
    }
    match event_name {
        Some("update-policy") => {
            let policy: UpdatePolicy = serde_json::from_str(&data)?;
            anyhow::ensure!(
                event_id == Some(policy.revision.to_string().as_str()),
                "policy SSE id mismatch"
            );
            Ok(Some(UpdateStreamEvent::Policy(policy)))
        }
        Some("update-command") => {
            let command: UpdateCommand = serde_json::from_str(&data)?;
            anyhow::ensure!(
                event_id == Some(command.command_id.as_str()),
                "command SSE id mismatch"
            );
            anyhow::ensure!(
                !command.command_id.is_empty()
                    && command.command_id.len() <= 128
                    && command.command_id.bytes().all(|byte| {
                        byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':')
                    }),
                "command id is invalid"
            );
            if let Some(version) = &command.target_version {
                anyhow::ensure!(!version.is_empty(), "command target version is empty");
            }
            if command.action == UpdateCommandAction::Install {
                anyhow::ensure!(
                    command.target_version.is_some() && command.target_build_seq.is_some(),
                    "install command target is missing"
                );
            }
            Ok(Some(UpdateStreamEvent::Command(command)))
        }
        Some("release-published") => {
            let notice: ReleaseNotice = serde_json::from_str(&data)?;
            anyhow::ensure!(
                !notice.version.is_empty(),
                "release notice version is empty"
            );
            anyhow::ensure!(notice.build_seq > 0, "release notice build is missing");
            Ok(Some(UpdateStreamEvent::ReleasePublished(notice)))
        }
        _ => Ok(None),
    }
}

#[cfg(test)]
mod policy_contract_tests {
    use super::*;

    #[test]
    fn update_identity_uses_explicit_device_id_and_real_device_uuid() {
        let identity = update_client_identity(
            "505054514",
            "YzUyN2U4YjAtY2I4Ny00OTIzLTg4OTAtYjQyYzM0M2YxYzY0",
        );

        assert_eq!(identity.client_id, "505054514");
        assert_eq!(
            identity.client_uuid,
            "YzUyN2U4YjAtY2I4Ny00OTIzLTg4OTAtYjQyYzM0M2YxYzY0"
        );
    }

    #[test]
    fn unset_update_options_default_to_disabled() {
        assert!(!update_option_enabled(""));
        assert!(!update_option_enabled("N"));
        assert!(update_option_enabled("Y"));
    }

    #[test]
    fn policy_revision_only_applies_newer_server_state() {
        let policy = UpdatePolicy {
            client_id: "83077683".to_owned(),
            client_uuid: "01ab".to_owned(),
            revision: 4,
            check_on_startup: true,
            auto_update: false,
            scheduled_update: false,
            scheduled_update_interval_hours: DEFAULT_SCHEDULED_UPDATE_INTERVAL_HOURS,
            management_agent_enabled: true,
        };

        assert_eq!(policy.decision(Some(3)), PolicyDecision::Apply);
        assert_eq!(policy.decision(Some(4)), PolicyDecision::IgnoreStale);
        assert_eq!(policy.decision(Some(5)), PolicyDecision::IgnoreStale);
    }

    #[test]
    fn initial_revision_zero_applies_once() {
        let policy = UpdatePolicy {
            client_id: "83077683".to_owned(),
            client_uuid: "01ab".to_owned(),
            revision: 0,
            check_on_startup: false,
            auto_update: false,
            scheduled_update: false,
            scheduled_update_interval_hours: DEFAULT_SCHEDULED_UPDATE_INTERVAL_HOURS,
            management_agent_enabled: true,
        };

        assert_eq!(policy.decision(None), PolicyDecision::Apply);
        assert_eq!(policy.decision(Some(0)), PolicyDecision::IgnoreStale);
    }

    #[test]
    fn policy_deserializes_server_contract() {
        let policy: UpdatePolicy = serde_json::from_str(
            r#"{"client_id":"83077683","client_uuid":"01ab","policy_revision":7,"enable_check_update":false,"allow_auto_update":true}"#,
        )
        .expect("policy should deserialize");

        assert_eq!(policy.revision, 7);
        assert!(!policy.check_on_startup);
        assert!(policy.auto_update);
        assert!(!policy.scheduled_update);
        assert_eq!(
            policy.scheduled_update_interval_hours,
            DEFAULT_SCHEDULED_UPDATE_INTERVAL_HOURS
        );
        assert!(policy.management_agent_enabled);
    }

    #[test]
    fn legacy_policy_defaults_management_agent_to_enabled() {
        let policy: UpdatePolicy = serde_json::from_str(
            r#"{"client_id":"83077683","client_uuid":"01ab","policy_revision":1,"enable_check_update":false,"allow_auto_update":false}"#,
        )
        .expect("legacy policy should deserialize");
        assert!(policy.management_agent_enabled);
    }

    #[test]
    fn policy_stream_url_contains_identity_and_resume_revision() {
        let identity = update_client_identity("83077683", "01ab");

        assert_eq!(
            update_policy_stream_url("https://rdapi.yan.life", &identity, Some(9)),
            "https://rdapi.yan.life/rd/update/v1/policy/stream?client_id=83077683&client_uuid=01ab&after_revision=9"
        );
    }

    #[test]
    fn fresh_policy_stream_has_no_resume_cursor() {
        let identity = update_client_identity("83077683", "01ab");

        assert_eq!(
            update_policy_stream_url("https://rdapi.yan.life", &identity, None),
            "https://rdapi.yan.life/rd/update/v1/policy/stream?client_id=83077683&client_uuid=01ab"
        );
    }

    #[test]
    fn policy_resume_revision_is_bound_to_client_uuid() {
        assert_eq!(policy_resume_revision("01ab", "01ab", Some(0)), Some(0));
        assert_eq!(policy_resume_revision("ffff", "01ab", Some(9)), None);
        assert_eq!(policy_resume_revision("", "01ab", Some(9)), None);
    }

    #[test]
    fn startup_and_scheduled_update_switches_are_independent() {
        assert!(!should_run_startup_update(false));
        assert!(should_run_startup_update(true));
        assert!(!should_run_scheduled_update(false));
        assert!(should_run_scheduled_update(true));
    }

    #[test]
    fn scheduled_update_policy_deserializes_and_bounds_interval() {
        let policy: UpdatePolicy = serde_json::from_str(
            r#"{"client_id":"83077683","client_uuid":"01ab","policy_revision":8,"enable_check_update":false,"allow_auto_update":false,"enable_scheduled_update":true,"scheduled_update_interval_hours":12}"#,
        )
        .expect("scheduled policy should deserialize");

        assert!(policy.scheduled_update);
        assert_eq!(policy.scheduled_update_interval_hours, 12);
        assert_eq!(normalize_scheduled_update_interval_hours(0), 1);
        assert_eq!(normalize_scheduled_update_interval_hours(5), 5);
        assert_eq!(normalize_scheduled_update_interval_hours(1000), 168);
    }

    #[test]
    fn parses_policy_sse_and_ignores_heartbeat() {
        let event = concat!(
            "event: update-policy\n",
            "id: 12\n",
            "data: {\"client_id\":\"83077683\",\"client_uuid\":\"01ab\",",
            "\"policy_revision\":12,\"enable_check_update\":true,",
            "\"allow_auto_update\":false}\n\n"
        );
        assert_eq!(
            parse_update_policy_sse_event(event).expect("event should parse"),
            Some(UpdatePolicy {
                client_id: "83077683".to_owned(),
                client_uuid: "01ab".to_owned(),
                revision: 12,
                check_on_startup: true,
                auto_update: false,
                scheduled_update: false,
                scheduled_update_interval_hours: DEFAULT_SCHEDULED_UPDATE_INTERVAL_HOURS,
                management_agent_enabled: true,
            })
        );
        assert_eq!(
            parse_update_policy_sse_event(": heartbeat\n\n").unwrap(),
            None
        );
    }

    #[test]
    fn rejects_policy_when_sse_id_does_not_match_revision() {
        let event = concat!(
            "event: update-policy\n",
            "id: 11\n",
            "data: {\"client_id\":\"83077683\",\"client_uuid\":\"01ab\",",
            "\"policy_revision\":12,\"enable_check_update\":true,",
            "\"allow_auto_update\":false}\n\n"
        );

        assert!(parse_update_policy_sse_event(event).is_err());
    }

    #[test]
    fn policy_is_bound_to_requested_device_identity() {
        let identity = update_client_identity("83077683", "01ab");
        let event = concat!(
            "event: update-policy\n",
            "id: 12\n",
            "data: {\"client_id\":\"other\",\"client_uuid\":\"01ab\",",
            "\"policy_revision\":12,\"enable_check_update\":true,",
            "\"allow_auto_update\":false}\n\n"
        );
        let policy = parse_update_policy_sse_event(event)
            .expect("event should parse")
            .expect("event should contain policy");

        assert!(!policy.matches_identity(&identity));
    }

    #[test]
    fn parses_check_and_install_command_sse_contract() {
        let check = concat!(
            "event: update-command\n",
            "id: cmd-check-1\n",
            "data: {\"command_id\":\"cmd-check-1\",\"action\":\"check\",",
            "\"client_id\":\"83077683\",\"client_uuid\":\"01ab\",",
            "\"target_version\":\"1.5.0\",\"target_build_seq\":50,",
            "\"expires_at\":2000}\n\n"
        );
        let install = check
            .replace("cmd-check-1", "cmd-install-1")
            .replace("\"action\":\"check\"", "\"action\":\"install\"");

        assert_eq!(
            parse_update_stream_sse_event(check).expect("check command should parse"),
            Some(UpdateStreamEvent::Command(UpdateCommand {
                command_id: "cmd-check-1".to_owned(),
                action: UpdateCommandAction::Check,
                client_id: "83077683".to_owned(),
                client_uuid: "01ab".to_owned(),
                target_version: Some("1.5.0".to_owned()),
                target_build_seq: Some(50),
                expires_at: 2000,
            }))
        );
        assert!(matches!(
            parse_update_stream_sse_event(&install).expect("install command should parse"),
            Some(UpdateStreamEvent::Command(UpdateCommand {
                action: UpdateCommandAction::Install,
                ..
            }))
        ));
    }

    #[test]
    fn rejects_command_when_sse_id_does_not_match_command_id() {
        let event = concat!(
            "event: update-command\n",
            "id: other\n",
            "data: {\"command_id\":\"cmd-1\",\"action\":\"check\",",
            "\"client_id\":\"83077683\",\"client_uuid\":\"01ab\",",
            "\"target_version\":\"1.5.0\",\"target_build_seq\":50,",
            "\"expires_at\":2000}\n\n"
        );

        assert!(parse_update_stream_sse_event(event).is_err());
    }

    #[test]
    fn check_command_allows_no_target_but_install_requires_one() {
        let check = concat!(
            "event: update-command\n",
            "id: cmd-check\n",
            "data: {\"command_id\":\"cmd-check\",\"action\":\"check\",",
            "\"client_id\":\"83077683\",\"client_uuid\":\"01ab\",",
            "\"expires_at\":2000}\n\n"
        );
        let install = check.replace("\"action\":\"check\"", "\"action\":\"install\"");

        assert!(matches!(
            parse_update_stream_sse_event(check),
            Ok(Some(UpdateStreamEvent::Command(UpdateCommand {
                action: UpdateCommandAction::Check,
                ..
            })))
        ));
        assert!(parse_update_stream_sse_event(&install).is_err());
    }

    #[test]
    fn command_decision_enforces_identity_expiry_and_deduplication() {
        let identity = update_client_identity("83077683", "01ab");
        let command = UpdateCommand {
            command_id: "cmd-1".to_owned(),
            action: UpdateCommandAction::Install,
            client_id: identity.client_id.clone(),
            client_uuid: identity.client_uuid.clone(),
            target_version: Some("1.5.0".to_owned()),
            target_build_seq: Some(50),
            expires_at: 2000,
        };

        assert_eq!(
            command.decision(&identity, 1999, false),
            UpdateCommandDecision::Execute
        );
        assert_eq!(
            command.decision(&identity, 2000, false),
            UpdateCommandDecision::Expired
        );
        assert_eq!(
            command.decision(&identity, 1999, true),
            UpdateCommandDecision::Duplicate
        );
        let mut wrong_identity = command.clone();
        wrong_identity.client_uuid = "ffff".to_owned();
        assert_eq!(
            wrong_identity.decision(&identity, 1999, false),
            UpdateCommandDecision::IdentityMismatch
        );
    }

    #[test]
    fn accepted_command_continues_after_original_expiry() {
        let identity = update_client_identity("83077683", "uuid-1");
        let command = UpdateCommand {
            command_id: "command-1".to_owned(),
            action: UpdateCommandAction::Install,
            client_id: identity.client_id.clone(),
            client_uuid: identity.client_uuid.clone(),
            target_version: Some("1.5.0".to_owned()),
            target_build_seq: Some(50),
            expires_at: 2000,
        };

        assert_eq!(
            command.accepted_decision(&identity, false),
            UpdateCommandDecision::Execute
        );
        assert_eq!(
            command.accepted_decision(&identity, true),
            UpdateCommandDecision::Duplicate
        );
    }

    #[test]
    fn command_outcomes_distinguish_no_update_check_and_async_install() {
        assert_eq!(
            command_outcome(UpdateCommandAction::Check, false, false),
            UpdateCommandOutcome::NoUpdate
        );
        assert_eq!(
            command_outcome(UpdateCommandAction::Check, true, false),
            UpdateCommandOutcome::CheckCompleted
        );
        assert_eq!(
            command_outcome(UpdateCommandAction::Install, true, true),
            UpdateCommandOutcome::AwaitingInstallResult
        );
    }

    #[test]
    fn terminal_outbox_only_becomes_processed_after_ack() {
        let command = UpdateCommand {
            command_id: "cmd-1".to_owned(),
            action: UpdateCommandAction::Install,
            client_id: "83077683".to_owned(),
            client_uuid: "01ab".to_owned(),
            target_version: Some("1.5.0".to_owned()),
            target_build_seq: Some(50),
            expires_at: 2000,
        };
        let mut state = UpdateCommandState::pending(command);

        state.set_terminal("installed", "");
        assert!(state.terminal.is_some());
        assert!(!state.acked);
        state.ack_terminal();
        assert!(state.acked);
    }

    #[test]
    fn deferred_command_is_reported_only_once_across_retries() {
        let mut state = UpdateCommandState::pending(UpdateCommand {
            command_id: "cmd-1".to_owned(),
            action: UpdateCommandAction::Install,
            client_id: "83077683".to_owned(),
            client_uuid: "01ab".to_owned(),
            target_version: Some("1.5.0".to_owned()),
            target_build_seq: Some(50),
            expires_at: 2000,
        });

        assert!(state.mark_deferred_reported());
        assert!(!state.mark_deferred_reported());
        assert!(state.deferred_reported);
    }

    #[test]
    fn processed_command_history_is_bounded_and_deduplicated() {
        let mut history = ProcessedUpdateCommands::default();
        for index in 0..140 {
            history.remember(format!("cmd-{index}"));
        }
        history.remember("cmd-139".to_owned());

        assert_eq!(history.ids.len(), 128);
        assert!(!history.contains("cmd-0"));
        assert!(history.contains("cmd-139"));
        assert_eq!(history.ids.iter().filter(|id| *id == "cmd-139").count(), 1);
    }

    #[test]
    fn device_auth_canonical_binds_command_and_body() {
        assert_eq!(
            update_device_auth_payload(
                "POST",
                "/rd/update/v1/check",
                "83077683",
                "01ab",
                123,
                "nonce",
                "cmd-1",
                "aabb"
            ),
            b"rustdesk-update-auth-v1\nmethod=POST\npath=/rd/update/v1/check\nclient_id=83077683\nclient_uuid=01ab\ntimestamp=123\nnonce=nonce\ncommand_id=cmd-1\nbody_sha256=aabb\n"
        );
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde_derive::Deserialize, serde_derive::Serialize)]
pub struct PendingUpdateEvent {
    pub transaction_id: String,
    pub from_version: String,
    pub from_build_seq: u64,
    pub version: String,
    pub build_seq: u64,
    pub source: UpdateSource,
    pub command_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MacUpdateResult {
    pub status: String,
    pub event: PendingUpdateEvent,
}

impl MacUpdateResult {
    pub fn encode(&self) -> anyhow::Result<String> {
        anyhow::ensure!(
            self.event.transaction_id.len() == 32
                && self
                    .event
                    .transaction_id
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit()),
            "invalid update transaction id"
        );
        for value in [
            self.event.from_version.as_str(),
            self.event.version.as_str(),
        ] {
            anyhow::ensure!(
                !value.is_empty()
                    && value
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric()
                            || matches!(byte, b'.' | b'_' | b'-')),
                "invalid update result version"
            );
        }
        anyhow::ensure!(
            matches!(
                self.status.as_str(),
                "pending" | "installed" | "rolled_back" | "rollback_failed"
            ),
            "invalid update result status"
        );
        Ok(format!(
            "transaction_id={}\nstatus={}\nfrom_version={}\nfrom_build_seq={}\nto_version={}\nto_build_seq={}\nsource={}\ncommand_id={}\n",
            self.event.transaction_id,
            self.status,
            self.event.from_version,
            self.event.from_build_seq,
            self.event.version,
            self.event.build_seq,
            self.event.source.as_str(),
            self.event.command_id.as_deref().unwrap_or_default(),
        ))
    }

    pub fn decode(value: &str) -> Option<Self> {
        let fields = value
            .lines()
            .map(|line| line.split_once('='))
            .collect::<Option<std::collections::HashMap<_, _>>>()?;
        if fields.len() != 7 && fields.len() != 8 {
            return None;
        }
        let transaction_id = *fields.get("transaction_id")?;
        if transaction_id.len() != 32
            || !transaction_id.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return None;
        }
        let status = *fields.get("status")?;
        if !matches!(
            status,
            "pending" | "installed" | "rolled_back" | "rollback_failed"
        ) {
            return None;
        }
        let source = match *fields.get("source")? {
            "primary" => UpdateSource::Primary,
            "mirror" => UpdateSource::Mirror,
            _ => return None,
        };
        let from_version = (*fields.get("from_version")?).to_owned();
        let version = (*fields.get("to_version")?).to_owned();
        if from_version.is_empty() || version.is_empty() {
            return None;
        }
        Some(Self {
            status: status.to_owned(),
            event: PendingUpdateEvent {
                transaction_id: transaction_id.to_owned(),
                from_version,
                from_build_seq: fields.get("from_build_seq")?.parse().ok()?,
                version,
                build_seq: fields.get("to_build_seq")?.parse().ok()?,
                source,
                command_id: fields
                    .get("command_id")
                    .filter(|value| !value.is_empty())
                    .map(|value| (*value).to_owned()),
            },
        })
    }
}

pub fn classify_update_preinstall_failure(message: &str) -> &'static str {
    if message.to_ascii_lowercase().contains("active session")
        || message.to_ascii_lowercase().contains("deferring")
    {
        "deferred"
    } else {
        "failed"
    }
}

impl PendingUpdateEvent {
    const ARG: &'static str = "--update-event";

    pub fn cli_args(&self) -> Vec<String> {
        vec![
            Self::ARG.to_owned(),
            self.transaction_id.clone(),
            self.from_version.clone(),
            self.from_build_seq.to_string(),
            self.version.clone(),
            self.build_seq.to_string(),
            self.source.as_str().to_owned(),
            self.command_id.clone().unwrap_or_default(),
        ]
    }

    pub fn from_cli_args(args: &[String]) -> Option<Self> {
        let index = args.iter().position(|arg| arg == Self::ARG)?;
        let transaction_id = args.get(index + 1)?.trim();
        let from_version = args.get(index + 2)?.trim();
        let from_build_seq = args.get(index + 3)?.parse().ok()?;
        let version = args.get(index + 4)?.trim();
        let build_seq = args.get(index + 5)?.parse().ok()?;
        let source = match args.get(index + 6)?.as_str() {
            "primary" => UpdateSource::Primary,
            "mirror" => UpdateSource::Mirror,
            _ => return None,
        };
        let command_id = args
            .get(index + 7)
            .filter(|value| !value.is_empty())
            .cloned();
        if transaction_id.len() != 32
            || !transaction_id.bytes().all(|byte| byte.is_ascii_hexdigit())
            || from_version.is_empty()
            || version.is_empty()
        {
            return None;
        }
        Some(Self {
            transaction_id: transaction_id.to_owned(),
            from_version: from_version.to_owned(),
            from_build_seq,
            version: version.to_owned(),
            build_seq,
            source,
            command_id,
        })
    }
}

// Marks an install or update started by the backend or by policy rather than by the
// person at the desk. Such runs must stay completely silent; only their logs remain.
pub const REMOTE_SILENT_ARG: &str = "--remote-silent";

pub fn update_is_remote(manually: bool, has_command: bool) -> bool {
    has_command || !manually
}

// The marker goes right after `--update`: the pending event that follows is parsed
// by position and may end with an empty command id, so anything appended after it
// would be read as that id.
pub fn update_launch_args(remote: bool, pending: &PendingUpdateEvent) -> Vec<String> {
    let mut args = vec!["--update".to_owned()];
    if remote {
        args.push(REMOTE_SILENT_ARG.to_owned());
    }
    args.extend(pending.cli_args());
    args
}

pub fn is_remote_silent(args: &[String]) -> bool {
    args.iter().any(|arg| arg == REMOTE_SILENT_ARG)
}

// Where the installed system service runs the updater, it alone owns the policy
// stream, remote commands and automatic checks. A user-session process repeating
// them would run the same remote command a second time, with UAC and windows.
// An installed client whose service was removed (stop-service) has no such owner,
// so its user process keeps the full loop.
pub fn process_runs_background_updater(
    installed: bool,
    is_service_process: bool,
    service_present: bool,
) -> bool {
    is_service_process || !(installed && service_present)
}

// The UI sends the whole option map over IPC, so a schedule change is a change of
// either schedule key between the old and the new map, not mere presence.
pub fn update_schedule_options_changed(
    old: &std::collections::HashMap<String, String>,
    new: &std::collections::HashMap<String, String>,
) -> bool {
    use crate::config::keys::{
        OPTION_ENABLE_SCHEDULED_UPDATE, OPTION_SCHEDULED_UPDATE_INTERVAL_HOURS,
    };
    [
        OPTION_ENABLE_SCHEDULED_UPDATE,
        OPTION_SCHEDULED_UPDATE_INTERVAL_HOURS,
    ]
    .iter()
    .any(|key| old.get(*key) != new.get(*key))
}

pub fn update_main_window_fallback_session(current: Option<u32>, remote: bool) -> Option<u32> {
    if remote {
        None
    } else {
        current
    }
}

// Reported when a remote, policy or startup install is refused because it would
// have to raise UAC or an administrator password prompt to proceed.
pub const ELEVATION_REQUIRED_REMOTE: &str = "elevation_required_remote";

pub fn remote_install_may_request_elevation(manually: bool, already_elevated: bool) -> bool {
    manually || already_elevated
}

// A remote refusal must stay silent at the desk: the backend and the log hear about
// it, the local UI never sees a failure it did not start.
pub fn update_event_reaches_local_ui(error_code: &str) -> bool {
    error_code != ELEVATION_REQUIRED_REMOTE
}

// Non-command refusals recur every policy/startup/scheduled cycle with the same
// cause; only the first one in this process is reported. A command always gets one.
pub fn should_report_refused_elevation(has_command: bool, already_reported: bool) -> bool {
    has_command || !already_reported
}

impl UpdateSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Primary => "primary",
            Self::Mirror => "mirror",
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct UpdateSourceFailure<E> {
    pub source: UpdateSource,
    pub url: String,
    pub error: E,
}

pub fn decide_update_action(
    response: &VersionCheckResponse,
    local_auto_update: bool,
) -> UpdateAction {
    if !response.update_available || response.mode == "disabled" {
        return UpdateAction::Ignore;
    }
    match response.mode.as_str() {
        "notify" => UpdateAction::Notify,
        "auto_install" if response.auto_install && local_auto_update => UpdateAction::AutoInstall,
        "download" | "auto_install" => UpdateAction::Download,
        _ => UpdateAction::Notify,
    }
}

pub fn update_target_key(platform: &str, arch: &str, kind: &str, edition: &str) -> String {
    format!("{platform}-{arch}-{kind}-{edition}")
}

pub fn update_target_release<'a>(
    manifest: &'a UpdateManifest,
    target: &'a hbb_common::UpdateTarget,
) -> anyhow::Result<(&'a str, u64, &'a str)> {
    if manifest.schema >= 2 {
        anyhow::ensure!(!target.version.is_empty(), "target version is missing");
        anyhow::ensure!(target.build_seq > 0, "target build is missing");
        anyhow::ensure!(
            !target.source_commit.is_empty(),
            "target source commit is missing"
        );
        Ok((&target.version, target.build_seq, &target.source_commit))
    } else {
        Ok((
            &manifest.version,
            manifest.build_seq,
            &manifest.source_commit,
        ))
    }
}

pub fn update_signature_payload(
    manifest: &UpdateManifest,
    target_key: &str,
    target: &hbb_common::UpdateTarget,
) -> anyhow::Result<Vec<u8>> {
    let (version, build_seq, source_commit) = update_target_release(manifest, target)?;
    for (name, value) in [
        ("product", manifest.product.as_str()),
        ("edition", manifest.edition.as_str()),
        ("channel", manifest.channel.as_str()),
        ("version", version),
        ("source_commit", source_commit),
        ("target_key", target_key),
        ("sha256", target.sha256.as_str()),
    ] {
        anyhow::ensure!(
            !value.contains(['\r', '\n']),
            "update signature field {name} contains a line break"
        );
    }
    Ok(format!(
        concat!(
            "rustdesk-update-v1\n",
            "product={}\n",
            "edition={}\n",
            "channel={}\n",
            "version={}\n",
            "build_seq={}\n",
            "source_commit={}\n",
            "target_key={}\n",
            "size={}\n",
            "sha256={}\n",
        ),
        manifest.product,
        manifest.edition,
        manifest.channel,
        version,
        build_seq,
        source_commit,
        target_key,
        target.size,
        target.sha256.to_ascii_lowercase(),
    )
    .into_bytes())
}

pub fn validate_target_metadata(
    target: &hbb_common::UpdateTarget,
    trusted_signature_key_id: &str,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        target.primary.starts_with("https://"),
        "update primary URL must use HTTPS"
    );
    anyhow::ensure!(
        target.mirrors.iter().all(|url| url.starts_with("https://")),
        "update mirror URL must use HTTPS"
    );
    anyhow::ensure!(target.size > 0, "update size is missing");
    anyhow::ensure!(
        target.sha256.len() == 64 && target.sha256.bytes().all(|byte| byte.is_ascii_hexdigit()),
        "update sha256 is invalid"
    );
    anyhow::ensure!(!target.signature.is_empty(), "update signature is missing");
    anyhow::ensure!(
        target.signature_key_id == trusted_signature_key_id,
        "update signing key is not trusted"
    );
    Ok(())
}

pub fn try_update_sources<T, E>(
    target: &hbb_common::UpdateTarget,
    mut attempt: impl FnMut(&str) -> Result<T, E>,
) -> Result<(T, UpdateSource), Vec<UpdateSourceFailure<E>>> {
    let mut failures = Vec::new();
    for (url, source) in std::iter::once((&target.primary, UpdateSource::Primary))
        .chain(target.mirrors.iter().map(|url| (url, UpdateSource::Mirror)))
    {
        if url.is_empty() {
            continue;
        }
        match attempt(url) {
            Ok(value) => return Ok((value, source)),
            Err(error) => failures.push(UpdateSourceFailure {
                source,
                url: url.clone(),
                error,
            }),
        }
    }
    Err(failures)
}

pub fn replace_file_transaction(
    current: &Path,
    downloaded: &Path,
    validate: impl FnOnce(&Path) -> io::Result<()>,
) -> io::Result<()> {
    let backup = current.with_extension("old");
    match fs::remove_file(&backup) {
        Ok(()) => {}
        Err(err) if err.kind() == io::ErrorKind::NotFound => {}
        Err(err) => return Err(err),
    }
    fs::rename(current, &backup)?;
    if let Err(install_error) = fs::rename(downloaded, current) {
        return match fs::rename(&backup, current) {
            Ok(()) => Err(install_error),
            Err(rollback_error) => Err(io::Error::new(
                rollback_error.kind(),
                format!("install failed: {install_error}; rollback failed: {rollback_error}"),
            )),
        };
    }
    if let Err(validation_error) = validate(current) {
        let _ = fs::remove_file(current);
        return match fs::rename(&backup, current) {
            Ok(()) => Err(validation_error),
            Err(rollback_error) => Err(io::Error::new(
                rollback_error.kind(),
                format!("validation failed: {validation_error}; rollback failed: {rollback_error}"),
            )),
        };
    }
    fs::remove_file(backup)?;
    Ok(())
}

pub fn validate_manifest_contract(
    response: &VersionCheckResponse,
    manifest: &UpdateManifest,
    current_version: &str,
    current_build_seq: u64,
    product: &str,
    edition: &str,
    channel: &str,
    target_key: &str,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        response.update_available,
        "response does not offer an update"
    );
    let target = manifest
        .targets
        .get(target_key)
        .ok_or_else(|| anyhow::anyhow!("target is missing"))?;
    let (target_version, target_build_seq, _) = update_target_release(manifest, target)?;
    anyhow::ensure!(
        target_version == response.target_version,
        "target version mismatch"
    );
    anyhow::ensure!(
        target_build_seq == response.target_build_seq,
        "target build mismatch"
    );
    anyhow::ensure!(manifest.product == product, "product mismatch");
    anyhow::ensure!(
        manifest.edition == edition || manifest.edition == "multi",
        "edition mismatch"
    );
    anyhow::ensure!(manifest.channel == channel, "channel mismatch");
    anyhow::ensure!(
        is_newer_version(
            target_version,
            target_build_seq,
            current_version,
            current_build_seq
        ),
        "manifest is not newer"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use hbb_common::{UpdateTarget, VersionCheckResponse};
    use std::{collections::HashMap, fs, io, path::PathBuf};

    fn pending(command_id: Option<&str>) -> PendingUpdateEvent {
        PendingUpdateEvent {
            transaction_id: "a".repeat(32),
            from_version: "1.5.3".to_owned(),
            from_build_seq: 2026101805,
            version: "1.5.4".to_owned(),
            build_seq: 2026102201,
            source: UpdateSource::Primary,
            command_id: command_id.map(str::to_owned),
        }
    }

    // Only the person at the desk starts a manual update; a backend command or a
    // policy-driven check is remote even when it reuses the same code path.
    #[test]
    fn update_origin_is_remote_unless_started_manually_without_a_command() {
        assert!(!update_is_remote(true, false));
        assert!(update_is_remote(false, false));
        assert!(update_is_remote(false, true));
        assert!(update_is_remote(true, true));
    }

    // The pending event is parsed by position, and an empty command id is the last
    // field, so the marker must never land where it could be read as the command id.
    #[test]
    fn remote_marker_never_shifts_the_positional_update_event() {
        for command in [None, Some("c".repeat(32))] {
            let event = pending(command.as_deref());
            let remote = update_launch_args(true, &event);
            assert_eq!(remote[0], "--update");
            assert!(is_remote_silent(&remote));
            assert_eq!(
                PendingUpdateEvent::from_cli_args(&remote),
                Some(event.clone())
            );
            let marker = remote
                .iter()
                .position(|arg| arg == REMOTE_SILENT_ARG)
                .unwrap();
            let start = remote
                .iter()
                .position(|arg| arg == "--update-event")
                .unwrap();
            assert!(marker < start, "{:?}", remote);
            let local = update_launch_args(false, &event);
            assert_eq!(local[0], "--update");
            assert!(!is_remote_silent(&local));
            assert_eq!(PendingUpdateEvent::from_cli_args(&local), Some(event));
        }
    }

    #[test]
    fn remote_marker_is_recognised_exactly() {
        let args = |v: &[&str]| v.iter().map(|a| a.to_string()).collect::<Vec<_>>();
        assert!(is_remote_silent(&args(&[
            "--silent-install",
            "--remote-silent"
        ])));
        assert!(!is_remote_silent(&args(&["--silent-install"])));
        assert!(!is_remote_silent(&args(&["--update", "--remote-silent-x"])));
        assert!(!is_remote_silent(&args(&["--update", "x--remote-silent"])));
    }

    fn temp_test_dir(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "rustdesk-update-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock should be after Unix epoch")
                .as_nanos()
        ));
        fs::create_dir_all(&path).expect("test directory should be created");
        path
    }

    fn response(mode: &str, auto_install: bool) -> VersionCheckResponse {
        VersionCheckResponse {
            update_available: true,
            mode: mode.to_owned(),
            auto_install,
            target_version: "1.5.0".to_owned(),
            target_build_seq: 2026093005,
            ..Default::default()
        }
    }

    #[test]
    fn update_modes_have_distinct_actions() {
        assert_eq!(
            decide_update_action(&response("disabled", false), true),
            UpdateAction::Ignore
        );
        assert_eq!(
            decide_update_action(&response("notify", false), true),
            UpdateAction::Notify
        );
        assert_eq!(
            decide_update_action(&response("download", false), true),
            UpdateAction::Download
        );
        assert_eq!(
            decide_update_action(&response("auto_install", true), true),
            UpdateAction::AutoInstall
        );
        assert_eq!(
            decide_update_action(&response("auto_install", false), true),
            UpdateAction::Download
        );
        assert_eq!(
            decide_update_action(&response("auto_install", true), false),
            UpdateAction::Download
        );
    }

    #[test]
    fn pending_install_event_round_trips_through_cli_args() {
        let event = PendingUpdateEvent {
            transaction_id: "0123456789abcdef0123456789abcdef".to_owned(),
            from_version: "1.5.0".to_owned(),
            from_build_seq: 2026093004,
            version: "1.5.0".to_owned(),
            build_seq: 2026093005,
            source: UpdateSource::Mirror,
            command_id: Some("cmd-install-1".to_owned()),
        };
        let mut args = vec!["--update".to_owned()];
        args.extend(event.cli_args());

        assert_eq!(PendingUpdateEvent::from_cli_args(&args), Some(event));
    }

    #[test]
    fn pending_install_event_rejects_incomplete_or_untrusted_values() {
        for args in [
            vec![
                "--update-event",
                "0123456789abcdef0123456789abcdef",
                "",
                "2026093004",
                "1.5.0",
                "2026093005",
                "primary",
            ],
            vec![
                "--update-event",
                "0123456789abcdef0123456789abcdef",
                "1.5.0",
                "invalid",
                "1.5.0",
                "2026093005",
                "primary",
            ],
            vec![
                "--update-event",
                "0123456789abcdef0123456789abcdef",
                "1.5.0",
                "2026093004",
                "1.5.0",
                "invalid",
                "primary",
            ],
            vec![
                "--update-event",
                "0123456789abcdef0123456789abcdef",
                "1.5.0",
                "2026093004",
                "1.5.0",
                "2026093005",
                "unknown",
            ],
            vec![
                "--update-event",
                "0123456789abcdef0123456789abcdef",
                "1.5.0",
                "2026093004",
                "1.5.0",
                "2026093005",
            ],
        ] {
            let args = args.into_iter().map(str::to_owned).collect::<Vec<_>>();
            assert_eq!(PendingUpdateEvent::from_cli_args(&args), None);
        }
    }

    #[test]
    fn mac_update_result_round_trips_and_rejects_invalid_state() {
        let result = MacUpdateResult {
            status: "rolled_back".to_owned(),
            event: PendingUpdateEvent {
                transaction_id: "fedcba9876543210fedcba9876543210".to_owned(),
                from_version: "1.5.0".to_owned(),
                from_build_seq: 2026093004,
                version: "1.5.0".to_owned(),
                build_seq: 2026093005,
                source: UpdateSource::Mirror,
                command_id: Some("cmd-install-1".to_owned()),
            },
        };
        let encoded = result.encode().expect("result should encode");
        assert_eq!(MacUpdateResult::decode(&encoded), Some(result.clone()));
        assert!(MacUpdateResult::decode(&encoded.replace("rolled_back", "failed")).is_none());

        let injected = MacUpdateResult {
            status: "installed".to_owned(),
            event: PendingUpdateEvent {
                transaction_id: "fedcba9876543210fedcba9876543210".to_owned(),
                from_version: "1.5.0\nstatus=installed".to_owned(),
                from_build_seq: 1,
                version: "1.5.1".to_owned(),
                build_seq: 2,
                source: UpdateSource::Primary,
                command_id: None,
            },
        };
        assert!(injected.encode().is_err());

        let mut invalid_transaction = result;
        invalid_transaction.event.transaction_id = "bad/transaction".to_owned();
        assert!(invalid_transaction.encode().is_err());
    }

    #[test]
    fn preinstall_failure_never_claims_rollback() {
        assert_eq!(
            classify_update_preinstall_failure("Active session detected, deferring update"),
            "deferred"
        );
        assert_eq!(
            classify_update_preinstall_failure("failed to extract DMG"),
            "failed"
        );
    }

    #[test]
    fn signature_payload_binds_release_and_target_identity() {
        let manifest = UpdateManifest {
            version: "1.5.0".to_owned(),
            build_seq: 2026093005,
            product: "rustdesk-yan".to_owned(),
            edition: "multi".to_owned(),
            channel: "stable".to_owned(),
            source_commit: "0123456789abcdef0123456789abcdef01234567".to_owned(),
            ..Default::default()
        };
        let target = UpdateTarget {
            size: 42,
            sha256: "AB".repeat(32),
            ..Default::default()
        };

        let payload = update_signature_payload(&manifest, "windows-x86_64-exe-standard", &target)
            .expect("metadata should serialize");

        assert_eq!(
            String::from_utf8(payload).expect("payload should be UTF-8"),
            concat!(
                "rustdesk-update-v1\n",
                "product=rustdesk-yan\n",
                "edition=multi\n",
                "channel=stable\n",
                "version=1.5.0\n",
                "build_seq=2026093005\n",
                "source_commit=0123456789abcdef0123456789abcdef01234567\n",
                "target_key=windows-x86_64-exe-standard\n",
                "size=42\n",
                "sha256=abababababababababababababababababababababababababababababababab\n",
            )
        );

        let mut changed = manifest.clone();
        changed.edition = "sos".to_owned();
        assert_ne!(
            update_signature_payload(&changed, "windows-x86_64-exe-standard", &target)
                .expect("metadata should serialize"),
            update_signature_payload(&manifest, "windows-x86_64-exe-standard", &target)
                .expect("metadata should serialize")
        );
    }

    #[test]
    fn schema_two_uses_selected_target_release_identity() {
        let target_key = "windows-x86_64-exe-standard";
        let target = UpdateTarget {
            version: "1.5.0".to_owned(),
            build_number: "20261001.7".to_owned(),
            build_seq: 2026100107,
            source_commit: "0123456789abcdef0123456789abcdef01234567".to_owned(),
            source_tag: "v1.5.0-build-2026.10.01-07".to_owned(),
            size: 42,
            sha256: "AB".repeat(32),
            ..Default::default()
        };
        let manifest = UpdateManifest {
            schema: 2,
            version: "1.5.0".to_owned(),
            build_number: "20261002.1".to_owned(),
            build_seq: 2026100201,
            product: "rustdesk-yan".to_owned(),
            edition: "multi".to_owned(),
            channel: "stable".to_owned(),
            source_commit: "fedcba9876543210fedcba9876543210fedcba98".to_owned(),
            targets: HashMap::from([(target_key.to_owned(), target.clone())]),
            ..Default::default()
        };
        let response = VersionCheckResponse {
            update_available: true,
            target_version: target.version.clone(),
            target_build_seq: target.build_seq,
            ..Default::default()
        };

        validate_manifest_contract(
            &response,
            &manifest,
            "1.5.0",
            2026100107,
            "rustdesk-yan",
            "standard",
            "stable",
            target_key,
        )
        .expect_err("an inherited desktop target must not become newer with the catalog");

        let payload = update_signature_payload(&manifest, target_key, &target)
            .expect("target metadata should serialize");
        let payload = String::from_utf8(payload).expect("payload should be UTF-8");
        assert!(payload.contains("version=1.5.0\nbuild_seq=2026100107\n"));
        assert!(payload.contains(&format!("source_commit={}\n", target.source_commit)));
        assert!(!payload.contains("build_seq=2026100201"));
    }

    #[test]
    fn schema_one_falls_back_to_manifest_release_identity() {
        let target_key = "windows-x86_64-exe-standard";
        let manifest = UpdateManifest {
            version: "1.5.0".to_owned(),
            build_seq: 2026100107,
            product: "rustdesk-yan".to_owned(),
            edition: "multi".to_owned(),
            channel: "stable".to_owned(),
            targets: HashMap::from([(target_key.to_owned(), UpdateTarget::default())]),
            ..Default::default()
        };
        let response = VersionCheckResponse {
            update_available: true,
            target_version: "1.5.0".to_owned(),
            target_build_seq: 2026100107,
            ..Default::default()
        };

        validate_manifest_contract(
            &response,
            &manifest,
            "1.5.0",
            2026100106,
            "rustdesk-yan",
            "standard",
            "stable",
            target_key,
        )
        .expect("legacy manifests must remain compatible");
    }

    #[test]
    fn signature_payload_rejects_line_break_injection() {
        let manifest = UpdateManifest {
            product: "rustdesk-yan\nchannel=beta".to_owned(),
            ..Default::default()
        };
        assert!(update_signature_payload(&manifest, "target", &UpdateTarget::default()).is_err());
    }

    #[test]
    fn manifest_identity_and_target_must_match_response() {
        let key = update_target_key("windows", "x86_64", "exe", "standard");
        let mut targets = HashMap::new();
        targets.insert(key.clone(), UpdateTarget::default());
        let manifest = UpdateManifest {
            version: "1.5.0".to_owned(),
            build_seq: 2026093005,
            product: "rustdesk-yan".to_owned(),
            edition: "standard".to_owned(),
            channel: "stable".to_owned(),
            targets,
            ..Default::default()
        };
        let offered = response("download", false);
        assert!(validate_manifest_contract(
            &offered,
            &manifest,
            "1.5.0",
            2026093004,
            "rustdesk-yan",
            "standard",
            "stable",
            &key
        )
        .is_ok());
        assert!(validate_manifest_contract(
            &offered,
            &manifest,
            "1.5.0",
            2026093004,
            "rustdesk-yan",
            "sos",
            "stable",
            &key
        )
        .is_err());
        assert!(validate_manifest_contract(
            &offered,
            &manifest,
            "1.5.0",
            2026093005,
            "rustdesk-yan",
            "standard",
            "stable",
            &key
        )
        .is_err());
    }

    #[test]
    fn target_metadata_is_fail_closed() {
        let valid = UpdateTarget {
            primary: "https://download.yan.life/rustdesk/stable/v1/rustdesk.exe".to_owned(),
            mirrors: vec![
                "https://github.com/jandoucn/rustdesk/releases/download/v1/rustdesk.exe".to_owned(),
            ],
            size: 42,
            sha256: "a".repeat(64),
            signature: "c2lnbmF0dXJl".to_owned(),
            signature_key_id: "yan-release-2026".to_owned(),
            ..Default::default()
        };
        assert!(validate_target_metadata(&valid, "yan-release-2026").is_ok());

        for invalid in [
            UpdateTarget {
                size: 0,
                ..valid.clone()
            },
            UpdateTarget {
                sha256: String::new(),
                ..valid.clone()
            },
            UpdateTarget {
                sha256: "z".repeat(64),
                ..valid.clone()
            },
            UpdateTarget {
                signature: String::new(),
                ..valid.clone()
            },
            UpdateTarget {
                signature_key_id: "other-key".to_owned(),
                ..valid.clone()
            },
            UpdateTarget {
                primary: "http://download.yan.life/rustdesk.exe".to_owned(),
                ..valid.clone()
            },
        ] {
            assert!(validate_target_metadata(&invalid, "yan-release-2026").is_err());
        }
    }

    #[test]
    fn update_source_falls_back_to_mirror() {
        let target = UpdateTarget {
            primary: "https://primary.invalid/update".to_owned(),
            mirrors: vec!["https://mirror.example/update".to_owned()],
            ..Default::default()
        };
        let mut attempted = Vec::new();
        let (value, source) = try_update_sources(&target, |url| {
            attempted.push(url.to_owned());
            if url.contains("primary") {
                Err("primary failed")
            } else {
                Ok("verified package")
            }
        })
        .expect("mirror should succeed");

        assert_eq!(value, "verified package");
        assert_eq!(source, UpdateSource::Mirror);
        assert_eq!(
            attempted,
            vec![target.primary.clone(), target.mirrors[0].clone()]
        );
    }

    #[test]
    fn update_source_reports_every_failure() {
        let target = UpdateTarget {
            primary: "https://primary.invalid/update".to_owned(),
            mirrors: vec!["https://mirror.example/update".to_owned()],
            ..Default::default()
        };
        let failures = try_update_sources(&target, |url| Err::<(), _>(format!("{url} failed")))
            .expect_err("all sources should fail");

        assert_eq!(failures.len(), 2);
        assert_eq!(failures[0].source, UpdateSource::Primary);
        assert_eq!(failures[1].source, UpdateSource::Mirror);
        assert!(failures[1].error.contains("mirror.example"));
    }

    #[test]
    fn failed_replacement_restores_the_previous_binary() {
        let dir = temp_test_dir("rollback");
        let current = dir.join("rustdesk.AppImage");
        let downloaded = dir.join("downloaded.AppImage");
        fs::write(&current, b"old working version").expect("old version should be written");
        fs::write(&downloaded, b"new broken version").expect("new version should be written");

        let result = replace_file_transaction(&current, &downloaded, |_| {
            Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "smoke check failed",
            ))
        });

        assert!(result.is_err());
        assert_eq!(
            fs::read(&current).expect("old version should remain"),
            b"old working version"
        );
        assert!(!downloaded.exists());
        assert!(!current.with_extension("old").exists());
        fs::remove_dir_all(dir).expect("test directory should be removed");
    }

    // Installed Windows/macOS with its service in place: the service owns every
    // background update path, so user-session processes (main window, --cm,
    // --install) stay out.
    #[test]
    fn background_updater_runs_only_in_the_service_when_the_service_is_enabled() {
        assert!(process_runs_background_updater(true, true, true));
        assert!(!process_runs_background_updater(true, false, true));
    }

    // Portable or not-yet-installed builds have no service; an installed client
    // whose service was stopped (stop-service, service deleted) has nobody else
    // to run policy, remote commands or timed checks, so the user process keeps them.
    #[test]
    fn background_updater_keeps_running_in_process_without_an_enabled_service() {
        assert!(process_runs_background_updater(false, false, true));
        assert!(process_runs_background_updater(false, false, false));
        assert!(process_runs_background_updater(true, false, false));
        assert!(process_runs_background_updater(false, true, true));
    }

    fn options(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    // The service must recompute its timer when the user edits the schedule in the
    // UI; the options map arrives whole over IPC, so only a real value change counts.
    #[test]
    fn update_schedule_change_is_detected_for_either_schedule_key() {
        use crate::config::keys::{
            OPTION_ENABLE_SCHEDULED_UPDATE as ENABLE,
            OPTION_SCHEDULED_UPDATE_INTERVAL_HOURS as HOURS,
        };
        let old = options(&[(ENABLE, "Y"), (HOURS, "5"), ("other", "a")]);
        assert!(update_schedule_options_changed(
            &old,
            &options(&[(ENABLE, "N"), (HOURS, "5"), ("other", "a")])
        ));
        assert!(update_schedule_options_changed(
            &old,
            &options(&[(ENABLE, "Y"), (HOURS, "8"), ("other", "a")])
        ));
        assert!(update_schedule_options_changed(
            &old,
            &options(&[(HOURS, "5"), ("other", "a")])
        ));
        assert!(update_schedule_options_changed(
            &options(&[]),
            &options(&[(ENABLE, "Y")])
        ));
    }

    #[test]
    fn unrelated_option_changes_do_not_touch_the_update_schedule() {
        use crate::config::keys::{
            OPTION_ENABLE_SCHEDULED_UPDATE as ENABLE,
            OPTION_SCHEDULED_UPDATE_INTERVAL_HOURS as HOURS,
        };
        let old = options(&[(ENABLE, "Y"), (HOURS, "5"), ("other", "a")]);
        assert!(!update_schedule_options_changed(&old, &old.clone()));
        assert!(!update_schedule_options_changed(
            &old,
            &options(&[(ENABLE, "Y"), (HOURS, "5"), ("other", "b")])
        ));
        assert!(!update_schedule_options_changed(
            &options(&[]),
            &options(&[])
        ));
    }

    // A remote update must not bring up a main window that was not running; a
    // local update keeps restoring into the updater's own session as before.
    #[test]
    fn remote_update_never_falls_back_to_restoring_the_main_window() {
        assert_eq!(update_main_window_fallback_session(Some(3), false), Some(3));
        assert_eq!(update_main_window_fallback_session(None, false), None);
        assert_eq!(update_main_window_fallback_session(Some(3), true), None);
        assert_eq!(update_main_window_fallback_session(None, true), None);
    }

    // Remote commands, policy and startup installs must never raise UAC or an
    // administrator password prompt; only an already privileged process may proceed.
    #[test]
    fn remote_install_never_requests_elevation_from_an_unprivileged_process() {
        assert!(!remote_install_may_request_elevation(false, false));
        assert!(remote_install_may_request_elevation(false, true));
    }

    // The person at the desk keeps the existing UAC / password prompt.
    #[test]
    fn manual_install_keeps_requesting_elevation() {
        assert!(remote_install_may_request_elevation(true, false));
        assert!(remote_install_may_request_elevation(true, true));
    }

    // A refused remote install stays completely silent at the desk: the backend
    // and the log hear about it, the local UI does not.
    #[test]
    fn refused_remote_elevation_never_reaches_the_local_ui() {
        assert!(!update_event_reaches_local_ui(ELEVATION_REQUIRED_REMOTE));
        for code in ["", "all_sources_failed", "install_failed", "active_session"] {
            assert!(update_event_reaches_local_ui(code), "{}", code);
        }
    }

    // A backend command gets its own terminal every time; a policy, startup or
    // scheduled refusal repeats each cycle with the same cause and is sent once.
    #[test]
    fn refused_elevation_is_reported_once_unless_a_command_asked() {
        assert!(should_report_refused_elevation(false, false));
        assert!(!should_report_refused_elevation(false, true));
        assert!(should_report_refused_elevation(true, false));
        assert!(should_report_refused_elevation(true, true));
    }

    // The backend stores error_code in a bounded column and only accepts [a-z_].
    #[test]
    fn remote_elevation_refusal_code_fits_the_backend_error_code_contract() {
        assert!(ELEVATION_REQUIRED_REMOTE.len() <= 128);
        assert!(ELEVATION_REQUIRED_REMOTE
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b == b'_'));
    }

    fn resolve_recorded(global: &str, legacy: &str, writable: bool) -> (String, Vec<String>) {
        let mut writes = Vec::new();
        let value = {
            let writes = std::cell::RefCell::new(&mut writes);
            resolve_check_on_startup_option(
                global,
                legacy,
                writable,
                |v| writes.borrow_mut().push(format!("global={}", v)),
                || writes.borrow_mut().push("legacy=cleared".to_owned()),
            )
        };
        (value, writes)
    }

    #[test]
    fn check_on_startup_reads_the_global_value_first() {
        assert_eq!(resolve_recorded("Y", "", true), ("Y".to_owned(), vec![]));
        assert_eq!(resolve_recorded("N", "", false), ("N".to_owned(), vec![]));
    }

    // The global value is the only source of truth; a stale per-user copy left
    // next to it is retired so it can never come back once the global is cleared.
    #[test]
    fn check_on_startup_global_value_retires_a_stale_legacy_copy() {
        assert_eq!(
            resolve_recorded("N", "Y", true),
            ("N".to_owned(), vec!["legacy=cleared".to_owned()])
        );
        assert_eq!(resolve_recorded("N", "Y", false), ("N".to_owned(), vec![]));
    }

    #[test]
    fn check_on_startup_migrates_the_legacy_user_value_once() {
        assert_eq!(
            resolve_recorded("", "Y", true),
            (
                "Y".to_owned(),
                vec!["global=Y".to_owned(), "legacy=cleared".to_owned()]
            )
        );
        assert_eq!(
            resolve_recorded("", "N", true),
            (
                "N".to_owned(),
                vec!["global=N".to_owned(), "legacy=cleared".to_owned()]
            )
        );
    }

    // A process that cannot write the global config yet (for example a UI whose
    // option cache has not synced with the service) keeps the old value and
    // leaves the legacy copy for a later migration.
    #[test]
    fn check_on_startup_keeps_the_legacy_value_without_writing_when_not_writable() {
        assert_eq!(resolve_recorded("", "Y", false), ("Y".to_owned(), vec![]));
    }

    #[test]
    fn check_on_startup_stays_disabled_by_default() {
        let (value, writes) = resolve_recorded("", "", true);
        assert_eq!(value, "");
        assert!(writes.is_empty());
        assert!(!update_option_enabled(&value));
    }

    #[test]
    fn storing_check_on_startup_writes_global_and_drops_the_legacy_copy() {
        for value in ["Y", "N", ""] {
            let mut writes = Vec::new();
            {
                let writes = std::cell::RefCell::new(&mut writes);
                store_check_on_startup_option(
                    value,
                    |v| writes.borrow_mut().push(format!("global={}", v)),
                    || writes.borrow_mut().push("legacy=cleared".to_owned()),
                );
            }
            assert_eq!(
                writes,
                vec![format!("global={}", value), "legacy=cleared".to_owned()]
            );
        }
    }

    // The service decides the startup check from what the UI stored globally,
    // whatever its own per-user copy says; the legacy copy only fills a gap.
    #[test]
    fn service_startup_check_follows_the_value_the_ui_stored() {
        for (global, legacy, enabled) in [
            ("Y", "", true),
            ("Y", "N", true),
            ("N", "Y", false),
            ("", "Y", true),
            ("", "N", false),
            ("", "", false),
        ] {
            assert_eq!(
                check_on_startup_enabled(global, legacy),
                enabled,
                "global={:?} legacy={:?}",
                global,
                legacy
            );
        }
    }

    #[test]
    fn only_the_check_on_startup_key_is_routed_to_the_global_config() {
        use crate::config::keys::{KEYS_LOCAL_SETTINGS, OPTION_ENABLE_CHECK_UPDATE};
        assert!(is_global_check_on_startup_option(
            OPTION_ENABLE_CHECK_UPDATE
        ));
        for key in KEYS_LOCAL_SETTINGS
            .iter()
            .filter(|k| **k != OPTION_ENABLE_CHECK_UPDATE)
        {
            assert!(!is_global_check_on_startup_option(key), "{}", key);
        }
        assert!(!is_global_check_on_startup_option(""));
    }

    // Portable or service-less: the global Config is in this very process, so the
    // migrated value is read straight back and nothing is migrated twice.
    #[test]
    fn portable_process_migrates_once_and_reads_its_own_global_value_back() {
        let mut global = String::new();
        let mut legacy = "Y".to_owned();
        for _ in 0..2 {
            let (g, l) = (global.clone(), legacy.clone());
            let mut new_global = None;
            let mut cleared = false;
            let value = resolve_check_on_startup_option(
                &g,
                &l,
                true,
                |v| new_global = Some(v.to_owned()),
                || cleared = true,
            );
            assert_eq!(value, "Y");
            if let Some(v) = new_global {
                global = v;
            }
            if cleared {
                legacy.clear();
            }
        }
        assert_eq!((global.as_str(), legacy.as_str()), ("Y", ""));
        assert!(check_on_startup_enabled(&global, &legacy));
    }
}
