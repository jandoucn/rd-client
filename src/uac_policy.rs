//! Machine-wide UAC disable policy for Windows install and agent repair.
//!
//! Writes four DWORD values under
//! `HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Policies\System`.
//! `EnableLUA=0` takes full effect after the next user reboot; this module never
//! reboots. Uninstall does not restore these values.

pub const REG_SUBKEY: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Policies\System";

pub const DESIRED_VALUES: &[(&str, u32)] = &[
    ("EnableLUA", 0),
    ("ConsentPromptBehaviorAdmin", 0),
    ("PromptOnSecureDesktop", 0),
    ("FilterAdministratorToken", 0),
];

/// Sentinel OS username for terminal-admin connections. Non-empty so the peer
/// takes the admin-terminal path; the password is ignored and the process token
/// (SYSTEM when installed as a service) is used.
pub const TERMINAL_ADMIN_OS_MARKER: &str = ".";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UacSnapshot {
    pub enable_lua: Option<u32>,
    pub consent_prompt_behavior_admin: Option<u32>,
    pub prompt_on_secure_desktop: Option<u32>,
    pub filter_administrator_token: Option<u32>,
}

impl UacSnapshot {
    pub fn from_pairs(pairs: &[(&str, Option<u32>)]) -> Self {
        let mut snap = Self {
            enable_lua: None,
            consent_prompt_behavior_admin: None,
            prompt_on_secure_desktop: None,
            filter_administrator_token: None,
        };
        for (name, value) in pairs {
            match *name {
                "EnableLUA" => snap.enable_lua = *value,
                "ConsentPromptBehaviorAdmin" => snap.consent_prompt_behavior_admin = *value,
                "PromptOnSecureDesktop" => snap.prompt_on_secure_desktop = *value,
                "FilterAdministratorToken" => snap.filter_administrator_token = *value,
                _ => {}
            }
        }
        snap
    }

    pub fn is_fully_disabled(&self) -> bool {
        self.enable_lua == Some(0)
            && self.consent_prompt_behavior_admin == Some(0)
            && self.prompt_on_secure_desktop == Some(0)
            && self.filter_administrator_token == Some(0)
    }
}

/// `EnableLUA` was not already 0 before apply, so a reboot is still needed for
/// full effect. Idempotent re-apply when already 0 does not require reboot.
pub fn reboot_required_after_apply(before: &UacSnapshot) -> bool {
    before.enable_lua != Some(0)
}

/// CMD fragment appended to after-install / install_me. No reboot.
pub fn after_install_reg_commands() -> String {
    DESIRED_VALUES
        .iter()
        .map(|(name, value)| {
            format!(
                "reg add HKEY_LOCAL_MACHINE\\Software\\Microsoft\\Windows\\CurrentVersion\\Policies\\System /f /v {name} /t REG_DWORD /d {value}"
            )
        })
        .collect::<Vec<_>>()
        .join("\n    ")
}

/// Local gates win over the remote command: purge marker, `disable-agent`, an
/// active maintenance window and a user stop request (or an unreadable stop
/// intent) all refuse the write.
pub fn local_gate_refusal(
    disable_recovery: bool,
    disable_agent: bool,
    maintenance: bool,
    stop_requested: bool,
) -> Option<&'static str> {
    if disable_recovery {
        Some("local_gate_disable_recovery")
    } else if disable_agent {
        Some("local_gate_disable_agent")
    } else if maintenance {
        Some("local_gate_maintenance")
    } else if stop_requested {
        Some("local_gate_stop_requested")
    } else {
        None
    }
}

/// Error code reported for a failed UAC command, so the console can tell a
/// local refusal from a registry failure.
pub fn command_error_code(error: &str) -> Option<&'static str> {
    if error.starts_with("local_gate_") {
        Some("local_gate_refused")
    } else if error.starts_with("uac_policy_apply_failed") {
        Some("uac_policy_apply_failed")
    } else if error.starts_with("uac_policy_query_failed") {
        Some("uac_policy_query_failed")
    } else if error.starts_with("uac_policy_verify_failed") {
        Some("uac_policy_verify_failed")
    } else {
        None
    }
}

/// Login error an older peer returns when it still checks the OS account for
/// an admin terminal and the marker login is refused.
pub const LEGACY_TERMINAL_OS_LOGIN_FAILED: &str = "Incorrect username or password.";

/// Admin terminals never ask for OS credentials. Kept so a refusal of the
/// marker login can never fall back to an account dialog.
pub fn terminal_admin_os_login_fallback(
    _is_terminal_admin: bool,
    _already_fallen_back: bool,
    _has_password: bool,
    _err: &str,
) -> Option<&'static str> {
    None
}

/// Hint shown when an older peer still refuses the admin marker login and would
/// otherwise have prompted for an OS account. Shown once per connection; that
/// peer needs a build that accepts the marker.
pub fn terminal_admin_legacy_peer_hint(
    is_terminal_admin: bool,
    already_shown: bool,
    err: &str,
) -> Option<&'static str> {
    if is_terminal_admin && !already_shown && err == LEGACY_TERMINAL_OS_LOGIN_FAILED {
        Some(LEGACY_TERMINAL_PEER_HINT)
    } else {
        None
    }
}

/// Abnormal case, so deliberately not translated, like the service-mismatch error.
pub const LEGACY_TERMINAL_PEER_HINT: &str =
    "The controlled side runs an older build that still checks an OS account for the admin terminal. Upgrade it to open the admin terminal without an OS password.";

/// OS credentials sent with a terminal login. Admin terminals send only the
/// marker so the peer never needs, and never receives, an OS password.
pub fn terminal_login_os_credentials(
    is_terminal_admin: bool,
    os_username: String,
    os_password: String,
) -> (String, String) {
    if is_terminal_admin {
        (TERMINAL_ADMIN_OS_MARKER.to_string(), String::new())
    } else {
        (os_username, os_password)
    }
}

/// What the controller does after the peer hash arrives for an admin terminal.
/// Never opens an OS account dialog; only the RustDesk connection password.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalAdminAfterHash {
    /// No remembered connection password yet → ask for it only.
    PromptConnectionPassword,
    /// Connection password already known → login with the marker immediately.
    AutoLoginWithMarker,
}

/// `msgbox` type for [`TerminalAdminAfterHash::PromptConnectionPassword`].
/// Same type as a normal empty-password login so Flutter shows one password box.
pub const TERMINAL_ADMIN_CONNECTION_PASSWORD_MSGBOX: &str = "input-password";

/// Decide the admin-terminal path after hash. Independent of whether the peer
/// is old or new; OS credentials are never requested here.
pub fn terminal_admin_after_hash(connection_password_empty: bool) -> TerminalAdminAfterHash {
    if connection_password_empty {
        TerminalAdminAfterHash::PromptConnectionPassword
    } else {
        TerminalAdminAfterHash::AutoLoginWithMarker
    }
}

#[cfg(windows)]
pub fn read_snapshot() -> Result<UacSnapshot, String> {
    use winreg::{
        enums::{HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_64KEY},
        RegKey,
    };
    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    let key = match hklm.open_subkey_with_flags(REG_SUBKEY, KEY_READ | KEY_WOW64_64KEY) {
        Ok(key) => key,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Ok(UacSnapshot {
                enable_lua: None,
                consent_prompt_behavior_admin: None,
                prompt_on_secure_desktop: None,
                filter_administrator_token: None,
            });
        }
        Err(err) => return Err(err.to_string()),
    };
    let read = |name: &str| -> Option<u32> { key.get_value::<u32, _>(name).ok() };
    Ok(UacSnapshot {
        enable_lua: read("EnableLUA"),
        consent_prompt_behavior_admin: read("ConsentPromptBehaviorAdmin"),
        prompt_on_secure_desktop: read("PromptOnSecureDesktop"),
        filter_administrator_token: read("FilterAdministratorToken"),
    })
}

#[cfg(windows)]
pub fn apply_disabled() -> Result<(UacSnapshot, UacSnapshot, bool), String> {
    use winreg::{
        enums::{HKEY_LOCAL_MACHINE, KEY_READ, KEY_SET_VALUE, KEY_WOW64_64KEY},
        RegKey,
    };
    let before = read_snapshot()?;
    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    let (key, _) = hklm
        .create_subkey_with_flags(REG_SUBKEY, KEY_READ | KEY_SET_VALUE | KEY_WOW64_64KEY)
        .map_err(|err| err.to_string())?;
    for (name, value) in DESIRED_VALUES {
        key.set_value(*name, value)
            .map_err(|err| format!("set {name}: {err}"))?;
    }
    let after = read_snapshot()?;
    if !after.is_fully_disabled() {
        return Err("uac_policy_verify_failed".into());
    }
    let reboot_required = reboot_required_after_apply(&before);
    Ok((before, after, reboot_required))
}

fn snapshot_fields(snap: &UacSnapshot) -> serde_json::Value {
    serde_json::json!({
        "EnableLUA": snap.enable_lua,
        "ConsentPromptBehaviorAdmin": snap.consent_prompt_behavior_admin,
        "PromptOnSecureDesktop": snap.prompt_on_secure_desktop,
        "FilterAdministratorToken": snap.filter_administrator_token,
    })
}

/// Status heartbeat field and query-command payload (no secrets).
pub fn status_json(snap: &UacSnapshot) -> serde_json::Value {
    let mut value = snapshot_fields(snap);
    if let Some(obj) = value.as_object_mut() {
        obj.insert(
            "fully_disabled".into(),
            serde_json::Value::Bool(snap.is_fully_disabled()),
        );
    }
    value
}

/// Agent disable-command result payload (no secrets).
pub fn command_result_json(
    before: &UacSnapshot,
    after: &UacSnapshot,
    reboot_required: bool,
) -> serde_json::Value {
    serde_json::json!({
        "schema": 1,
        "applied": after.is_fully_disabled(),
        "reboot_required": reboot_required,
        "before": snapshot_fields(before),
        "after": snapshot_fields(after),
    })
}

/// Agent query-command result payload (no secrets).
pub fn query_result_json(snap: &UacSnapshot) -> serde_json::Value {
    serde_json::json!({
        "schema": 1,
        "uac": status_json(snap),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desired_values_match_disable_uac_script() {
        assert_eq!(DESIRED_VALUES.len(), 4);
        assert_eq!(DESIRED_VALUES[0], ("EnableLUA", 0));
        assert_eq!(DESIRED_VALUES[1], ("ConsentPromptBehaviorAdmin", 0));
        assert_eq!(DESIRED_VALUES[2], ("PromptOnSecureDesktop", 0));
        assert_eq!(DESIRED_VALUES[3], ("FilterAdministratorToken", 0));
    }

    #[test]
    fn fully_disabled_requires_all_four_zeros() {
        let full = UacSnapshot {
            enable_lua: Some(0),
            consent_prompt_behavior_admin: Some(0),
            prompt_on_secure_desktop: Some(0),
            filter_administrator_token: Some(0),
        };
        assert!(full.is_fully_disabled());
        let partial = UacSnapshot {
            enable_lua: Some(0),
            consent_prompt_behavior_admin: Some(0),
            prompt_on_secure_desktop: Some(0),
            filter_administrator_token: Some(1),
        };
        assert!(!partial.is_fully_disabled());
        assert!(!UacSnapshot {
            enable_lua: None,
            consent_prompt_behavior_admin: Some(0),
            prompt_on_secure_desktop: Some(0),
            filter_administrator_token: Some(0),
        }
        .is_fully_disabled());
    }

    #[test]
    fn reboot_required_when_enable_lua_was_not_zero() {
        let already = UacSnapshot {
            enable_lua: Some(0),
            consent_prompt_behavior_admin: Some(0),
            prompt_on_secure_desktop: Some(0),
            filter_administrator_token: Some(0),
        };
        assert!(!reboot_required_after_apply(&already));
        let enabled = UacSnapshot {
            enable_lua: Some(1),
            consent_prompt_behavior_admin: Some(5),
            prompt_on_secure_desktop: Some(1),
            filter_administrator_token: Some(0),
        };
        assert!(reboot_required_after_apply(&enabled));
        let missing = UacSnapshot {
            enable_lua: None,
            consent_prompt_behavior_admin: None,
            prompt_on_secure_desktop: None,
            filter_administrator_token: None,
        };
        assert!(reboot_required_after_apply(&missing));
    }

    #[test]
    fn after_install_commands_write_all_four_keys_without_reboot() {
        let cmds = after_install_reg_commands();
        for name in [
            "EnableLUA",
            "ConsentPromptBehaviorAdmin",
            "PromptOnSecureDesktop",
            "FilterAdministratorToken",
        ] {
            assert!(
                cmds.contains(&format!("/v {name} /t REG_DWORD /d 0")),
                "missing {name} in {cmds}"
            );
        }
        assert!(!cmds.to_ascii_lowercase().contains("shutdown"));
        assert!(!cmds.to_ascii_lowercase().contains("restart-computer"));
    }

    #[test]
    fn command_result_json_is_stable() {
        let before = UacSnapshot {
            enable_lua: Some(1),
            consent_prompt_behavior_admin: Some(5),
            prompt_on_secure_desktop: Some(1),
            filter_administrator_token: Some(0),
        };
        let after = UacSnapshot {
            enable_lua: Some(0),
            consent_prompt_behavior_admin: Some(0),
            prompt_on_secure_desktop: Some(0),
            filter_administrator_token: Some(0),
        };
        let v = command_result_json(&before, &after, true);
        assert_eq!(v["applied"], true);
        assert_eq!(v["reboot_required"], true);
        assert_eq!(v["before"]["EnableLUA"], 1);
        assert_eq!(v["after"]["EnableLUA"], 0);
    }

    #[test]
    fn query_and_status_json_expose_four_keys_and_fully_disabled() {
        let disabled = UacSnapshot {
            enable_lua: Some(0),
            consent_prompt_behavior_admin: Some(0),
            prompt_on_secure_desktop: Some(0),
            filter_administrator_token: Some(0),
        };
        let status = status_json(&disabled);
        assert_eq!(status["fully_disabled"], true);
        assert_eq!(status["EnableLUA"], 0);
        let query = query_result_json(&disabled);
        assert_eq!(query["schema"], 1);
        assert_eq!(query["uac"]["fully_disabled"], true);
        let enabled = UacSnapshot {
            enable_lua: Some(1),
            consent_prompt_behavior_admin: Some(5),
            prompt_on_secure_desktop: Some(1),
            filter_administrator_token: Some(0),
        };
        assert_eq!(status_json(&enabled)["fully_disabled"], false);
    }

    #[test]
    fn terminal_admin_marker_is_non_empty() {
        assert!(!TERMINAL_ADMIN_OS_MARKER.trim().is_empty());
    }

    #[test]
    fn local_gates_refuse_the_remote_write() {
        assert_eq!(local_gate_refusal(false, false, false, false), None);
        assert_eq!(
            local_gate_refusal(true, false, false, false),
            Some("local_gate_disable_recovery")
        );
        assert_eq!(
            local_gate_refusal(false, true, false, false),
            Some("local_gate_disable_agent")
        );
        assert_eq!(
            local_gate_refusal(false, false, true, false),
            Some("local_gate_maintenance")
        );
        assert_eq!(
            local_gate_refusal(false, false, false, true),
            Some("local_gate_stop_requested")
        );
        assert_eq!(
            local_gate_refusal(true, true, true, true),
            Some("local_gate_disable_recovery")
        );
    }

    #[test]
    fn command_errors_separate_local_refusal_from_registry_failure() {
        for refusal in [
            "local_gate_disable_recovery",
            "local_gate_disable_agent",
            "local_gate_maintenance",
            "local_gate_stop_requested",
        ] {
            assert_eq!(command_error_code(refusal), Some("local_gate_refused"));
        }
        assert_eq!(
            command_error_code("uac_policy_apply_failed:set EnableLUA: denied"),
            Some("uac_policy_apply_failed")
        );
        assert_eq!(
            command_error_code("uac_policy_query_failed:denied"),
            Some("uac_policy_query_failed")
        );
        assert_eq!(command_error_code("service query timed out"), None);
        assert_eq!(command_error_code("intent read: denied"), None);
    }

    #[test]
    fn admin_terminal_sends_only_the_marker_and_never_an_os_password() {
        assert_eq!(
            terminal_login_os_credentials(true, "Administrator".into(), "secret".into()),
            (TERMINAL_ADMIN_OS_MARKER.to_string(), String::new())
        );
        assert_eq!(
            terminal_login_os_credentials(true, String::new(), String::new()),
            (TERMINAL_ADMIN_OS_MARKER.to_string(), String::new())
        );
    }

    #[test]
    fn admin_terminal_after_hash_never_opens_an_os_account_dialog() {
        assert_eq!(
            terminal_admin_after_hash(true),
            TerminalAdminAfterHash::PromptConnectionPassword
        );
        assert_eq!(
            terminal_admin_after_hash(false),
            TerminalAdminAfterHash::AutoLoginWithMarker
        );
        // The only dialog type we may emit is the ordinary connection-password box.
        assert_eq!(TERMINAL_ADMIN_CONNECTION_PASSWORD_MSGBOX, "input-password");
        assert_ne!(
            TERMINAL_ADMIN_CONNECTION_PASSWORD_MSGBOX,
            "terminal-admin-login"
        );
        assert_ne!(
            TERMINAL_ADMIN_CONNECTION_PASSWORD_MSGBOX,
            "terminal-admin-login-password"
        );
    }

    #[test]
    fn non_admin_logins_keep_the_os_credentials_they_were_given() {
        assert_eq!(
            terminal_login_os_credentials(false, "user".into(), "pw".into()),
            ("user".to_string(), "pw".to_string())
        );
        assert_eq!(
            terminal_login_os_credentials(false, String::new(), String::new()),
            (String::new(), String::new())
        );
    }

    #[test]
    fn admin_terminal_never_asks_for_os_credentials() {
        for has_password in [true, false] {
            assert_eq!(
                terminal_admin_os_login_fallback(
                    true,
                    false,
                    has_password,
                    LEGACY_TERMINAL_OS_LOGIN_FAILED
                ),
                None,
                "admin terminal must not open an OS account dialog"
            );
        }
    }

    #[test]
    fn older_peer_gets_an_upgrade_hint_instead_of_a_credential_prompt() {
        assert_eq!(
            terminal_admin_legacy_peer_hint(true, false, LEGACY_TERMINAL_OS_LOGIN_FAILED),
            Some(LEGACY_TERMINAL_PEER_HINT)
        );
        assert_eq!(
            terminal_admin_legacy_peer_hint(true, true, LEGACY_TERMINAL_OS_LOGIN_FAILED),
            None,
            "hint is shown once per connection"
        );
        assert_eq!(
            terminal_admin_legacy_peer_hint(false, false, LEGACY_TERMINAL_OS_LOGIN_FAILED),
            None
        );
        assert_eq!(
            terminal_admin_legacy_peer_hint(true, false, "Wrong Password"),
            None
        );
    }

    #[test]
    fn os_account_fallback_happens_once_and_only_for_that_error_on_admin_terminals() {
        assert_eq!(
            terminal_admin_os_login_fallback(true, true, true, LEGACY_TERMINAL_OS_LOGIN_FAILED),
            None
        );
        assert_eq!(
            terminal_admin_os_login_fallback(false, false, true, LEGACY_TERMINAL_OS_LOGIN_FAILED),
            None
        );
        for other in ["Wrong Password", "Please try again in 15 seconds.", ""] {
            assert_eq!(
                terminal_admin_os_login_fallback(true, false, true, other),
                None
            );
        }
    }
}
