use hbb_common::{bail, log, tokio, ResultType};
use std::process::Command;

const STALE_PRIVACY_SERVICES: [&str; 2] = ["ScreenCapture", "Accessibility"];

pub(super) fn parse_designated_requirement(output: &str) -> Option<String> {
    output.lines().find_map(|line| {
        let (_, requirement) = line.split_once("designated =>")?;
        let requirement = requirement.trim();
        (!requirement.is_empty()).then(|| requirement.to_owned())
    })
}

/// TCC binds a grant to the designated requirement. Ad-hoc builds use a
/// per-build cdhash, so the old grant no longer matches the new bundle and
/// only blocks the fresh request; a stable Developer ID requirement keeps it.
pub(super) fn stale_privacy_reset_required(old: Option<&str>, new: Option<&str>) -> bool {
    match (old, new) {
        (Some(old), Some(new)) => old != new,
        _ => true,
    }
}

fn safe_bundle_id(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-'))
}

fn safe_single_quoted(value: &str) -> bool {
    !value.is_empty() && !value.contains('\'') && !value.contains('\n')
}

const LSREGISTER: &str = "/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister";

/// Shell definitions for the root update script: `reset_stale_privacy` clears
/// the Screen Recording and Accessibility grants of the replaced bundle, each
/// step bounded by `privacy_run`. `None` defines a no-op for an unchanged
/// signing identity, so callers can invoke it unconditionally.
pub(super) fn stale_privacy_reset_functions(
    bundle_ids: Option<&[String]>,
    app_bundle: &str,
    uids_var: &str,
    log_path: &str,
    marker_path: &str,
) -> ResultType<String> {
    if !safe_single_quoted(app_bundle)
        || !safe_single_quoted(log_path)
        || !safe_single_quoted(marker_path)
        || !marker_path.starts_with('/')
    {
        bail!("[root-update] unsafe path for privacy reset");
    }
    let marker_dir = match marker_path.rsplit_once('/') {
        Some((dir, _)) if !dir.is_empty() => dir,
        _ => bail!("[root-update] unsafe path for privacy reset"),
    };
    // Tells the relaunched GUI that this build's grants were already handled,
    // so it does not ask for the administrator password a second time.
    let record_marker =
        format!("    /bin/mkdir -p '{marker_dir}' && : > '{marker_path}' || true\n");
    if uids_var.is_empty()
        || !uids_var
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    {
        bail!("[root-update] unsafe shell variable for privacy reset");
    }
    let Some(bundle_ids) = bundle_ids else {
        return Ok(format!(
            "privacy_reset_done=0\nreset_stale_privacy() {{\n{record_marker}    return 0\n}}\n"
        ));
    };
    if bundle_ids.is_empty() {
        bail!("[root-update] no bundle identifier for privacy reset");
    }
    if let Some(bad) = bundle_ids.iter().find(|id| !safe_bundle_id(id)) {
        bail!("[root-update] unsafe bundle identifier for privacy reset: {bad:?}");
    }
    let mut unique: Vec<&str> = Vec::new();
    for id in bundle_ids {
        if !unique.contains(&id.as_str()) {
            unique.push(id);
        }
    }
    let id_list = unique.join(" ");
    let log = format!("'{log_path}'");
    let mut script = format!(
        r#"privacy_reset_done=0
privacy_run() {{
    "$@" >> {log} 2>&1 &
    privacy_pid=$!
    privacy_wait=0
    while kill -0 "$privacy_pid" 2>/dev/null; do
        if [ "$privacy_wait" -ge 100 ]; then
            kill -KILL "$privacy_pid" 2>/dev/null || true
            echo "[root-update] privacy reset step timed out: $*" >> {log}
            break
        fi
        sleep 0.1
        privacy_wait=$((privacy_wait + 1))
    done
    wait "$privacy_pid" 2>/dev/null || true
}}
reset_stale_privacy() {{
    [ "$privacy_reset_done" -eq 0 ] || return 0
    privacy_reset_done=1
    echo "[root-update] resetting stale ScreenCapture/Accessibility grants for {id_list}" >> {log}
    /usr/bin/logger -t rustdesk-root-update "resetting stale ScreenCapture/Accessibility grants for {id_list}" || true
    privacy_run {LSREGISTER} -f '{app_bundle}'
"#
    );
    for service in STALE_PRIVACY_SERVICES {
        for id in &unique {
            script.push_str(&format!(
                "    privacy_run /usr/bin/tccutil reset {service} '{id}'\n"
            ));
        }
    }
    script.push_str(&format!("    for privacy_uid in ${uids_var}; do\n"));
    for service in STALE_PRIVACY_SERVICES {
        for id in &unique {
            script.push_str(&format!(
                "        privacy_run launchctl asuser \"$privacy_uid\" /usr/bin/sudo -u \"#$privacy_uid\" /usr/bin/tccutil reset {service} '{id}'\n"
            ));
        }
    }
    script.push_str(&format!(
        "    done\n    echo \"[root-update] stale privacy grant reset finished\" >> {log}\n    /usr/bin/logger -t rustdesk-root-update \"stale privacy grant reset finished\" || true\n{record_marker}    return 0\n}}\n"
    ));
    Ok(script)
}

/// Root-owned marker written by the updater once it has handled the privacy
/// grants of `build_seq`; `/Library/Application Support` is admin-writable
/// only, so an ordinary user cannot plant a symlink for root to truncate.
pub(crate) fn privacy_handled_marker(app_name: &str, build_seq: u64) -> ResultType<String> {
    if !safe_single_quoted(app_name) || app_name.contains('/') || matches!(app_name, "." | "..") {
        bail!("unsafe application name for privacy marker: {app_name:?}");
    }
    Ok(format!(
        "/Library/Application Support/{app_name}/privacy-handled-{build_seq}"
    ))
}

/// AppleScript run by the GUI on the first launch of a build that was not
/// installed by the updater (fresh install or manual DMG copy), clearing any
/// grants left by a previous signature behind one administrator prompt.
pub(crate) fn admin_privacy_reset_script(
    bundle_id: &str,
    app_name: &str,
    services: &[&str],
) -> ResultType<String> {
    if !safe_bundle_id(bundle_id) {
        bail!("unsafe bundle identifier for privacy reset: {bundle_id:?}");
    }
    if app_name.is_empty() || app_name.contains(['"', '\\', '\n']) {
        bail!("unsafe application name for privacy reset: {app_name:?}");
    }
    let commands = privacy_reset_shell(bundle_id, services)?;
    Ok(format!(
        "do shell script \"{commands}\" with prompt \"{app_name} needs to refresh its screen recording and accessibility permissions\" with administrator privileges"
    ))
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum AdminSetup {
    Skip,
    ResetOnly,
    InstallDaemon { reset: bool },
}

/// One administrator prompt on the first launch of a build: install the
/// launchd daemon/agent when missing (so the device stays online without the
/// GUI) and clear stale grants unless the updater already did.
/// Resetting a grant that currently works would only force the user to grant
/// it again, so clear grants only when they are missing and the updater has
/// not already cleared them for this build.
/// Maps the GUI's comma separated list of missing permissions to TCC service
/// names, so only grants that are actually missing are cleared.
pub(crate) fn missing_services(list: &str) -> Vec<&'static str> {
    let mut services = Vec::new();
    for item in list.split(',') {
        let service = match item.trim() {
            "screen" => "ScreenCapture",
            "accessibility" => "Accessibility",
            _ => continue,
        };
        if !services.contains(&service) {
            services.push(service);
        }
    }
    services
}

/// A relaunch or a daemon handover also terminates the app; only a user quit
/// of an online device switches the service off.
pub(crate) fn quit_should_go_offline(programmatic: bool, stop_service: &str) -> bool {
    !programmatic && stop_service != "Y"
}

pub(crate) fn needs_privacy_reset(permissions_missing: bool, updater_handled: bool) -> bool {
    permissions_missing && !updater_handled
}

pub(crate) fn admin_setup_plan(
    needs_reset: bool,
    daemon_installed: bool,
    app_installed: bool,
    user_stopped_service: bool,
) -> AdminSetup {
    let install_daemon = app_installed && !daemon_installed && !user_stopped_service;
    match (install_daemon, needs_reset) {
        (true, reset) => AdminSetup::InstallDaemon { reset },
        (false, true) => AdminSetup::ResetOnly,
        (false, false) => AdminSetup::Skip,
    }
}

pub(crate) fn privacy_reset_shell(bundle_id: &str, services: &[&str]) -> ResultType<String> {
    if !safe_bundle_id(bundle_id) {
        bail!("unsafe bundle identifier for privacy reset: {bundle_id:?}");
    }
    if services.is_empty() || !services.iter().all(|s| STALE_PRIVACY_SERVICES.contains(s)) {
        bail!("unsupported privacy services for reset: {services:?}");
    }
    Ok(services
        .iter()
        .map(|service| format!("/usr/bin/tccutil reset {service} '{bundle_id}' || true;"))
        .collect::<Vec<_>>()
        .join(" "))
}

const INSTALL_SH_ANCHOR: &str = "set sh to sh1 & sh2 & sh3 & sh4 & sh5";

/// Appends `extra_sh` to the existing daemon install script so it runs inside
/// the same administrator prompt; the script itself stays the single source of
/// the install steps (including copying the identity config to root).
pub(crate) fn install_script_with_extra(
    install_script: &str,
    extra_sh: &str,
) -> ResultType<String> {
    if extra_sh.contains(['"', '\\', '\n']) {
        bail!("unsafe extra shell for install script");
    }
    if install_script.matches(INSTALL_SH_ANCHOR).count() != 1 {
        bail!("install script layout changed, refusing to patch");
    }
    if extra_sh.is_empty() {
        return Ok(install_script.to_owned());
    }
    // Keep the daemon load status (last command of sh5) as the exit status.
    Ok(install_script.replace(
        INSTALL_SH_ANCHOR,
        &format!("{INSTALL_SH_ANCHOR} & \" install_rc=$?; {extra_sh} exit $install_rc\""),
    ))
}

fn current_bundle_path() -> Option<std::path::PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let bundle = exe.parent()?.parent()?.parent()?;
    (bundle.extension().and_then(|ext| ext.to_str()) == Some("app")).then(|| bundle.to_owned())
}

fn install_daemon_with_extra(extra_sh: &str) -> bool {
    // Rename the template first so the appended bundle id is never rewritten.
    let template = super::macos::correct_app_name(include_str!("privileges_scripts/install.scpt"));
    let script = install_script_with_extra(&template, extra_sh);
    let script = match script {
        Ok(script) => script,
        Err(err) => {
            log::warn!("[permission-guide] skip daemon install: {err}");
            return false;
        }
    };
    let daemon = super::macos::correct_app_name(include_str!("privileges_scripts/daemon.plist"));
    let agent = super::macos::correct_app_name(include_str!("privileges_scripts/agent.plist"));
    let status = Command::new("/usr/bin/osascript")
        .args([
            "-e",
            &script,
            &daemon,
            &agent,
            &super::macos::get_active_username(),
        ])
        .status();
    let agent_plist = format!(
        "/Library/LaunchAgents/{}_server.plist",
        crate::get_full_name()
    );
    let installed =
        matches!(status, Ok(ref s) if s.success()) && std::path::Path::new(&agent_plist).exists();
    log::info!("[permission-guide] daemon install status={status:?} installed={installed}");
    if installed {
        Command::new("launchctl")
            .args(["load", "-w", &agent_plist])
            .status()
            .ok();
    }
    installed
}

fn run_reset_only(bundle_id: Option<&str>, app_name: &str, services: &[&str]) -> String {
    let Some(bundle_id) = bundle_id else {
        log::warn!("[permission-guide] bundle identifier unavailable, skip privacy reset");
        return "failed".to_owned();
    };
    let script = match admin_privacy_reset_script(bundle_id, app_name, services) {
        Ok(script) => script,
        Err(err) => {
            log::warn!("[permission-guide] skip admin privacy reset: {err}");
            return "failed".to_owned();
        }
    };
    match Command::new("/usr/bin/osascript")
        .args(["-e", &script])
        .status()
    {
        Ok(status) if status.success() => "reset".to_owned(),
        Ok(status) => {
            log::info!("[permission-guide] admin privacy reset declined: {status}");
            "failed".to_owned()
        }
        Err(err) => {
            log::warn!("[permission-guide] run osascript failed: {err}");
            "failed".to_owned()
        }
    }
}

fn install_extra_for(reset: bool, bundle_id: Option<&str>, services: &[&str]) -> String {
    if !reset {
        return String::new();
    }
    let Some(id) = bundle_id else {
        log::warn!("[permission-guide] bundle identifier unavailable, skip privacy reset");
        return String::new();
    };
    privacy_reset_shell(id, services).unwrap_or_else(|err| {
        log::warn!("[permission-guide] skip privacy reset: {err}");
        String::new()
    })
}

/// Called from the GUI (not a Tokio thread) on the first launch of a build;
/// `missing` lists the permissions that are not granted. It may block on the
/// administrator prompt. Returns "skipped", "reset", "installed" or "failed".
pub(crate) fn run_admin_setup(missing: &str) -> String {
    let services = missing_services(missing);
    let app_name = crate::get_app_name();
    let updater_handled = privacy_handled_marker(&app_name, crate::BUILD_SEQ)
        .map(|marker| std::path::Path::new(&marker).exists())
        .unwrap_or(false);
    // Read the flag before the service switch: restore_online_after_quit clears
    // the switch first and the flag last, so no interleaving looks "stopped".
    let quit_flag = hbb_common::config::LocalConfig::get_option(QUIT_OFFLINE_FLAG);
    let user_stopped_service = current_stop_service() == "Y" && quit_flag != "Y";
    let plan = admin_setup_plan(
        needs_privacy_reset(!services.is_empty(), updater_handled),
        super::macos::is_installed_daemon(false),
        super::macos::is_installed(),
        user_stopped_service,
    );
    log::info!("[permission-guide] admin setup plan {plan:?} missing={services:?}");
    let bundle_id =
        current_bundle_path().and_then(|bundle| read_bundle_identifier(&bundle.to_string_lossy()));
    match plan {
        AdminSetup::Skip => "skipped".to_owned(),
        AdminSetup::ResetOnly => run_reset_only(bundle_id.as_deref(), &app_name, &services),
        AdminSetup::InstallDaemon { reset } => {
            let extra = install_extra_for(reset, bundle_id.as_deref(), &services);
            if install_daemon_with_extra(&extra) {
                "installed".to_owned()
            } else {
                "failed".to_owned()
            }
        }
    }
}

fn read_bundle_identifier(bundle: &str) -> Option<String> {
    let output = Command::new("/usr/libexec/PlistBuddy")
        .args([
            "-c",
            "Print :CFBundleIdentifier",
            &format!("{bundle}/Contents/Info.plist"),
        ])
        .output()
        .ok()?;
    let id = String::from_utf8(output.stdout).ok()?.trim().to_owned();
    (output.status.success() && !id.is_empty()).then_some(id)
}

fn read_designated_requirement(bundle: &str) -> Option<String> {
    let output = Command::new("/usr/bin/codesign")
        .args(["-d", "-r-", bundle])
        .output()
        .ok()?;
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.push('\n');
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    parse_designated_requirement(&text)
}

/// Shell definitions for the root update script; see
/// [`stale_privacy_reset_functions`]. Falls back to the no-op definition when
/// the signing identity is unchanged or the reset script cannot be built.
pub(super) fn stale_privacy_reset_for_update(
    installed_bundle: &str,
    staged_bundle: &str,
    uids_var: &str,
    log_path: &str,
    marker_path: &str,
) -> String {
    let noop = || {
        stale_privacy_reset_functions(None, installed_bundle, uids_var, log_path, marker_path)
            .unwrap_or_else(|_| {
                "privacy_reset_done=0\nreset_stale_privacy() {\n    return 0\n}\n".to_owned()
            })
    };
    let old_requirement = read_designated_requirement(installed_bundle);
    let new_requirement = read_designated_requirement(staged_bundle);
    if !stale_privacy_reset_required(old_requirement.as_deref(), new_requirement.as_deref()) {
        log::info!("[root-update] signing identity unchanged, keeping privacy grants");
        return noop();
    }
    let bundle_ids: Vec<String> = [staged_bundle, installed_bundle]
        .into_iter()
        .filter_map(read_bundle_identifier)
        .collect();
    log::info!(
        "[root-update] signing identity changed (old: {:?}, new: {:?}), will reset privacy grants for {:?}",
        old_requirement,
        new_requirement,
        bundle_ids
    );
    match stale_privacy_reset_functions(
        Some(&bundle_ids),
        installed_bundle,
        uids_var,
        log_path,
        marker_path,
    ) {
        Ok(script) => script,
        Err(err) => {
            log::warn!("[root-update] skip stale privacy reset: {err}");
            noop()
        }
    }
}

/// `None` means this user's server did not answer over IPC. Like the root
/// updater, that only counts as idle when no `--server` process is running;
/// any reply other than an explicit idle fails closed.
pub(crate) fn session_active_from_reply(reply: Option<Option<bool>>, server_running: bool) -> bool {
    match reply {
        None => server_running,
        Some(idle) => idle != Some(true),
    }
}

fn own_server_process_running() -> bool {
    let uid = unsafe { hbb_common::libc::getuid() }.to_string();
    let Ok(output) = Command::new("/usr/bin/pgrep")
        .args(["-u", &uid, "-x", &crate::get_app_name()])
        .output()
    else {
        return false;
    };
    String::from_utf8_lossy(&output.stdout).lines().any(|pid| {
        Command::new("/bin/ps")
            .args(["-p", pid.trim(), "-o", "args="])
            .output()
            .map(|args| {
                String::from_utf8_lossy(&args.stdout)
                    .split_whitespace()
                    .any(|arg| arg == "--server")
            })
            .unwrap_or(false)
    })
}

#[tokio::main(flavor = "current_thread")]
async fn probe_own_server() -> Option<Option<bool>> {
    let Ok(mut conn) = crate::ipc::connect(1000, "").await else {
        return None;
    };
    if conn
        .send(&crate::ipc::Data::HasNoActiveConns(None))
        .await
        .is_err()
    {
        return Some(None);
    }
    match conn.next_timeout(1000).await {
        Ok(Some(crate::ipc::Data::HasNoActiveConns(idle))) => Some(idle),
        _ => Some(None),
    }
}

static LAST_SESSION_ACTIVE: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(2);

/// Whether this user's server currently hosts an incoming session. Called from
/// the GUI (not a Tokio thread) before the permission guide restarts the agent.
pub(crate) fn has_active_incoming_connections() -> bool {
    let reply = probe_own_server();
    let server_running = reply.is_none() && own_server_process_running();
    let active = session_active_from_reply(reply, server_running);
    let previous = LAST_SESSION_ACTIVE.swap(active as u8, std::sync::atomic::Ordering::Relaxed);
    if previous != active as u8 {
        log::info!(
            "[permission-guide] incoming session probe reply={:?} server_running={} active={}",
            reply,
            server_running,
            active
        );
    }
    active
}

const QUIT_OFFLINE_FLAG: &str = "mac-stopped-by-quit";

/// Only an explicit quit taken offline by this feature is undone on the next
/// launch; a "stop service" chosen in settings is left alone.
pub(crate) fn should_restore_after_quit(flag: &str, stop_service: &str) -> bool {
    flag == "Y" && stop_service == "Y"
}

static PROGRAMMATIC_QUIT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Marks the coming termination as a relaunch or handover, not a user quit.
pub(crate) fn mark_programmatic_quit() {
    PROGRAMMATIC_QUIT.store(true, std::sync::atomic::Ordering::SeqCst);
}

// Goes over IPC in both install modes: the launchd agent or this GUI process
// hosts the server, and the IPC path also restarts the rendezvous mediator
// (it falls back to the local config when no IPC server answers).
fn current_stop_service() -> String {
    crate::ipc::get_options()
        .get("stop-service")
        .cloned()
        .unwrap_or_default()
}

/// Called from the native `applicationWillTerminate` (Cmd+Q, Dock "Quit"):
/// take the device offline without uninstalling the daemon, and remember why.
pub(crate) fn go_offline_for_quit() {
    let programmatic = PROGRAMMATIC_QUIT.load(std::sync::atomic::Ordering::SeqCst);
    if !quit_should_go_offline(programmatic, &current_stop_service()) {
        log::info!("[quit-offline] quit without switching service (programmatic={programmatic})");
        return;
    }
    hbb_common::config::LocalConfig::set_option(QUIT_OFFLINE_FLAG.to_owned(), "Y".to_owned());
    crate::ipc::set_option("stop-service", "Y");
    log::info!("[quit-offline] explicit quit, service switched offline");
}

/// Called on GUI launch: bring the device back online after an explicit quit.
pub(crate) fn restore_online_after_quit() {
    let flag = hbb_common::config::LocalConfig::get_option(QUIT_OFFLINE_FLAG);
    if flag != "Y" {
        return;
    }
    if should_restore_after_quit(&flag, &current_stop_service()) {
        crate::ipc::set_option("stop-service", "");
        log::info!("[quit-offline] relaunched after explicit quit, service back online");
    }
    hbb_common::config::LocalConfig::set_option(QUIT_OFFLINE_FLAG.to_owned(), "".to_owned());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_probe_reply_maps_to_activity() {
        assert!(
            !session_active_from_reply(None, false),
            "no server process and no IPC means no session"
        );
        assert!(
            session_active_from_reply(None, true),
            "unreachable but running server fails closed"
        );
        for running in [false, true] {
            assert!(
                !session_active_from_reply(Some(Some(true)), running),
                "idle reply"
            );
            assert!(
                session_active_from_reply(Some(Some(false)), running),
                "active reply"
            );
            assert!(
                session_active_from_reply(Some(None), running),
                "invalid reply"
            );
        }
    }

    #[test]
    fn parses_adhoc_cdhash_requirement() {
        let output = "Executable=/Applications/RustDesk Yan.app/Contents/MacOS/RustDesk Yan\n\
                      # designated => cdhash H\"bf9d272f0d66ce4e55f29c5acd70fb146279be70\"\n";
        assert_eq!(
            parse_designated_requirement(output).as_deref(),
            Some("cdhash H\"bf9d272f0d66ce4e55f29c5acd70fb146279be70\"")
        );
    }

    #[test]
    fn parses_developer_id_requirement() {
        let output = "designated => identifier \"com.carriez.rustdesk\" and anchor apple generic \
                      and certificate leaf[subject.OU] = \"ABCDE12345\"\n";
        assert_eq!(
            parse_designated_requirement(output).as_deref(),
            Some(
                "identifier \"com.carriez.rustdesk\" and anchor apple generic \
                 and certificate leaf[subject.OU] = \"ABCDE12345\""
            )
        );
    }

    #[test]
    fn missing_or_empty_requirement_is_none() {
        assert_eq!(parse_designated_requirement(""), None);
        assert_eq!(
            parse_designated_requirement("code object is not signed at all\n"),
            None
        );
        assert_eq!(parse_designated_requirement("designated =>   \n"), None);
    }

    #[test]
    fn unchanged_signing_identity_keeps_existing_grants() {
        let developer_id = "identifier \"com.carriez.rustdesk\" and anchor apple generic";
        assert!(!stale_privacy_reset_required(
            Some(developer_id),
            Some(developer_id)
        ));
    }

    #[test]
    fn changed_or_unknown_signing_identity_resets_grants() {
        assert!(stale_privacy_reset_required(
            Some("cdhash H\"aa\""),
            Some("cdhash H\"bb\"")
        ));
        assert!(stale_privacy_reset_required(None, Some("cdhash H\"bb\"")));
        assert!(stale_privacy_reset_required(Some("cdhash H\"aa\""), None));
        assert!(stale_privacy_reset_required(None, None));
    }

    fn ids(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    const APP: &str = "/Applications/RustDesk Yan.app";
    const LOG: &str = "/tmp/x/rustdesk_root_update.log";
    const MARKER: &str = "/Library/Application Support/RustDesk Yan/privacy-handled-2026103101";

    fn functions(bundle_ids: &[&str]) -> String {
        stale_privacy_reset_functions(Some(&ids(bundle_ids)), APP, "privacy_uids", LOG, MARKER)
            .expect("functions")
    }

    #[test]
    fn functions_reset_only_screen_and_accessibility_for_each_bundle() {
        let script = functions(&[
            "com.carriez.rustdesk",
            "com.carriez.rustdesk",
            "com.example.old",
        ]);
        for service in ["ScreenCapture", "Accessibility"] {
            for bundle in ["com.carriez.rustdesk", "com.example.old"] {
                let root = format!("    privacy_run /usr/bin/tccutil reset {service} '{bundle}'\n");
                let user = format!(
                    "        privacy_run launchctl asuser \"$privacy_uid\" /usr/bin/sudo -u \"#$privacy_uid\" /usr/bin/tccutil reset {service} '{bundle}'\n"
                );
                assert_eq!(script.matches(&root).count(), 1, "{root}\n{script}");
                assert_eq!(script.matches(&user).count(), 1, "{user}\n{script}");
            }
        }
        for forbidden in ["reset All", "ListenEvent", "Microphone", "PostEvent"] {
            assert!(
                !script.contains(forbidden),
                "{forbidden} must stay untouched"
            );
        }
        assert!(script.contains("    for privacy_uid in $privacy_uids; do\n"));
    }

    #[test]
    fn functions_bound_each_step_and_run_once() {
        let script = functions(&["com.carriez.rustdesk"]);
        assert!(script.contains("privacy_reset_done=0\n"));
        assert!(script.contains("privacy_run() {\n"));
        assert!(script.contains("reset_stale_privacy() {\n"));
        assert!(script.contains("[ \"$privacy_reset_done\" -eq 0 ] || return 0"));
        assert!(script.contains("if [ \"$privacy_wait\" -ge 100 ]; then"));
        assert!(script.contains("kill -KILL \"$privacy_pid\""));
        assert!(script.contains(&format!("\"$@\" >> '{LOG}' 2>&1 &")));
        let register = script.find("privacy_run /System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister -f '/Applications/RustDesk Yan.app'").expect("lsregister");
        let first_reset = script
            .find("privacy_run /usr/bin/tccutil reset")
            .expect("reset");
        assert!(register < first_reset);
    }

    #[test]
    fn functions_leave_a_durable_trace() {
        let script = functions(&["com.carriez.rustdesk"]);
        let logged: Vec<_> = script
            .lines()
            .filter(|line| line.contains("/usr/bin/logger -t rustdesk-root-update"))
            .collect();
        assert_eq!(logged.len(), 2, "{script}");
        assert!(logged[0].contains("com.carriez.rustdesk"));
    }

    #[test]
    fn unchanged_identity_defines_a_noop_reset() {
        let script =
            stale_privacy_reset_functions(None, APP, "privacy_uids", LOG, MARKER).expect("noop");
        assert!(script.contains("privacy_reset_done=0\n"));
        assert!(script.contains("reset_stale_privacy() {\n"));
        assert!(!script.contains("tccutil"));
        assert!(!script.contains("lsregister"));
    }

    fn run_sh(body: &str) -> std::process::Output {
        Command::new("/bin/sh")
            .args(["-c", body])
            .output()
            .expect("bash")
    }

    #[test]
    fn generated_functions_are_valid_shell() {
        for script in [
            functions(&["com.carriez.rustdesk"]),
            stale_privacy_reset_functions(None, APP, "privacy_uids", LOG, MARKER).expect("noop"),
        ] {
            for shell in ["/bin/sh", "/bin/bash"] {
                let status = Command::new(shell)
                    .args(["-n", "-c", &format!("privacy_uids=\"0 501\"\n{script}")])
                    .status()
                    .expect("shell");
                assert!(status.success(), "{shell}: {script}");
            }
        }
    }

    #[test]
    fn noop_reset_succeeds_and_privacy_run_logs_output() {
        let dir = std::env::temp_dir().join(format!("rdtest-tcc-run-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("dir");
        let log = dir.join("update.log").to_string_lossy().into_owned();
        let marker = dir.join("privacy-handled-1").to_string_lossy().into_owned();
        let script =
            stale_privacy_reset_functions(None, APP, "privacy_uids", &log, &marker).expect("noop");
        let out = run_sh(&format!("{script}\nreset_stale_privacy; echo rc=$?"));
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "rc=0");
        assert!(
            std::path::Path::new(&marker).is_file(),
            "noop reset still records that the updater handled this build"
        );

        let script = functions(&["com.carriez.rustdesk"]).replace(LOG, &log);
        let out = run_sh(&format!(
            "{script}\nprivacy_run /bin/echo probe-line; echo rc=$?"
        ));
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "rc=0");
        assert!(std::fs::read_to_string(&log)
            .expect("log")
            .contains("probe-line"));
        std::fs::remove_dir_all(&dir).expect("cleanup");
    }

    #[test]
    fn functions_reject_unsafe_inputs() {
        for bad in [
            "",
            "com.carriez.rust'desk",
            "com carriez",
            "com.carriez;rm",
            "a$(id)",
        ] {
            assert!(
                stale_privacy_reset_functions(Some(&ids(&[bad])), APP, "privacy_uids", LOG, MARKER)
                    .is_err(),
                "accepted bundle id {bad:?}"
            );
        }
        assert!(
            stale_privacy_reset_functions(Some(&[]), APP, "privacy_uids", LOG, MARKER).is_err()
        );
        let ok = ids(&["com.carriez.rustdesk"]);
        assert!(stale_privacy_reset_functions(
            Some(&ok),
            "/Applications/R'.app",
            "privacy_uids",
            LOG,
            MARKER
        )
        .is_err());
        assert!(
            stale_privacy_reset_functions(Some(&ok), APP, "privacy uids", LOG, MARKER).is_err()
        );
        assert!(
            stale_privacy_reset_functions(Some(&ok), APP, "privacy_uids", "/tmp/l'", MARKER)
                .is_err()
        );
        assert!(
            stale_privacy_reset_functions(None, APP, "privacy_uids", "/tmp/l'", MARKER).is_err()
        );
        assert!(
            stale_privacy_reset_functions(Some(&ok), APP, "privacy_uids", LOG, "/tmp/m'").is_err()
        );
        assert!(
            stale_privacy_reset_functions(None, APP, "privacy_uids", LOG, "relative/m").is_err()
        );
    }

    #[test]
    fn both_variants_record_the_marker_inside_a_root_owned_directory() {
        for script in [
            functions(&["com.carriez.rustdesk"]),
            stale_privacy_reset_functions(None, APP, "privacy_uids", LOG, MARKER).expect("noop"),
        ] {
            assert!(
                script.contains(&format!(
                    "    /bin/mkdir -p '/Library/Application Support/RustDesk Yan' && : > '{MARKER}' || true\n"
                )),
                "{script}"
            );
        }
    }

    #[test]
    fn marker_path_is_per_app_and_build() {
        assert_eq!(
            privacy_handled_marker("RustDesk Yan", 2026103101).expect("marker"),
            "/Library/Application Support/RustDesk Yan/privacy-handled-2026103101"
        );
        for bad in ["", "RustDesk/Yan", "RustDesk'Yan", "..", "."] {
            assert!(privacy_handled_marker(bad, 1).is_err(), "accepted {bad:?}");
        }
    }

    #[test]
    fn admin_reset_script_targets_only_this_bundle() {
        let script = admin_privacy_reset_script(
            "com.carriez.rustdesk",
            "RustDesk Yan",
            &STALE_PRIVACY_SERVICES,
        )
        .expect("script");
        assert!(script.starts_with("do shell script \""));
        assert!(script.ends_with("with administrator privileges"));
        assert!(script.contains(
            "/usr/bin/tccutil reset ScreenCapture 'com.carriez.rustdesk' || true; /usr/bin/tccutil reset Accessibility 'com.carriez.rustdesk' || true;"
        ));
        assert!(script.contains("with prompt \"RustDesk Yan"));
        for bad_id in ["", "com.x'y", "a b", "a\"b"] {
            assert!(
                admin_privacy_reset_script(bad_id, "RustDesk Yan", &STALE_PRIVACY_SERVICES)
                    .is_err()
            );
        }
        for bad_name in ["Rust\"Desk", "Rust\\Desk"] {
            assert!(admin_privacy_reset_script(
                "com.carriez.rustdesk",
                bad_name,
                &STALE_PRIVACY_SERVICES
            )
            .is_err());
        }
    }

    #[test]
    fn privacy_reset_never_touches_identity_or_key_configuration() {
        let scripts = [
            functions(&["com.carriez.rustdesk"]),
            stale_privacy_reset_functions(None, APP, "privacy_uids", LOG, MARKER).expect("noop"),
            admin_privacy_reset_script(
                "com.carriez.rustdesk",
                "RustDesk Yan",
                &STALE_PRIVACY_SERVICES,
            )
            .expect("admin"),
        ];
        for script in scripts {
            for forbidden in [
                "Preferences",
                "RustDesk2.toml",
                "RustDesk.toml",
                "key_pair",
                "rm ",
                "defaults ",
            ] {
                assert!(!script.contains(forbidden), "{forbidden} in:\n{script}");
            }
        }
    }

    #[test]
    fn admin_setup_asks_for_one_password_covering_everything_missing() {
        use AdminSetup::*;
        // (needs_reset, daemon_installed, app_installed, user_stopped_service)
        assert_eq!(
            admin_setup_plan(true, false, true, false),
            InstallDaemon { reset: true }
        );
        assert_eq!(
            admin_setup_plan(false, false, true, false),
            InstallDaemon { reset: false }
        );
        assert_eq!(admin_setup_plan(true, true, true, false), ResetOnly);
        assert_eq!(admin_setup_plan(false, true, true, false), Skip);
        // Running outside /Applications (e.g. from the DMG): never install the
        // daemon from a transient location.
        assert_eq!(admin_setup_plan(true, false, false, false), ResetOnly);
        assert_eq!(admin_setup_plan(false, false, false, false), Skip);
        // "Stop service" in settings uninstalls the daemon; keep that choice.
        assert_eq!(admin_setup_plan(false, false, true, true), Skip);
        assert_eq!(admin_setup_plan(true, false, true, true), ResetOnly);
    }

    #[test]
    fn missing_services_map_to_tcc_names() {
        assert_eq!(
            missing_services("screen,accessibility"),
            ["ScreenCapture", "Accessibility"]
        );
        assert_eq!(missing_services("screen"), ["ScreenCapture"]);
        assert_eq!(missing_services("accessibility"), ["Accessibility"]);
        assert!(missing_services("").is_empty());
        assert_eq!(missing_services("screen,bogus,screen"), ["ScreenCapture"]);
    }

    #[test]
    fn quit_goes_offline_only_for_a_user_quit_of_an_online_device() {
        assert!(quit_should_go_offline(false, ""));
        assert!(
            !quit_should_go_offline(true, ""),
            "relaunch or daemon handover"
        );
        assert!(
            !quit_should_go_offline(false, "Y"),
            "already stopped in settings"
        );
        assert!(!quit_should_go_offline(true, "Y"));
    }

    #[test]
    fn grants_are_reset_only_when_missing_and_not_already_handled() {
        assert!(needs_privacy_reset(true, false));
        assert!(!needs_privacy_reset(true, true), "updater already reset");
        assert!(!needs_privacy_reset(false, false), "valid grants are kept");
        assert!(!needs_privacy_reset(false, true));
    }

    #[test]
    fn privacy_reset_shell_is_two_bounded_commands() {
        assert_eq!(
            privacy_reset_shell("com.carriez.rustdesk", &STALE_PRIVACY_SERVICES).expect("shell"),
            "/usr/bin/tccutil reset ScreenCapture 'com.carriez.rustdesk' || true; \
             /usr/bin/tccutil reset Accessibility 'com.carriez.rustdesk' || true;"
        );
        let screen_only =
            privacy_reset_shell("com.carriez.rustdesk", &["ScreenCapture"]).expect("shell");
        assert!(
            !screen_only.contains("Accessibility"),
            "a working grant is kept"
        );
        assert!(privacy_reset_shell("com.carriez.rustdesk", &[]).is_err());
        assert!(privacy_reset_shell("com.carriez.rustdesk", &["All"]).is_err());
        assert!(privacy_reset_shell("com.x'y", &["ScreenCapture"]).is_err());
    }

    const INSTALL_SCPT: &str = include_str!("privileges_scripts/install.scpt");

    #[test]
    fn install_script_gets_the_reset_appended_inside_the_same_prompt() {
        let extra =
            privacy_reset_shell("com.carriez.rustdesk", &STALE_PRIVACY_SERVICES).expect("shell");
        let script = install_script_with_extra(INSTALL_SCPT, &extra).expect("patched");
        assert!(
            script.contains(&format!(
                "set sh to sh1 & sh2 & sh3 & sh4 & sh5 & \" install_rc=$?; {extra} exit $install_rc\""
            )),
            "the daemon load status must stay the script's exit status"
        );
        assert_eq!(script.matches("with administrator privileges").count(), 1);
        // The identity-preserving copy of the user config to root stays intact.
        assert!(script.contains("set sh3 to \"cp -rf \" & prefs_toml"));
        assert!(script.contains("set sh4 to \"cp -rf \" & prefs2_toml"));
        assert_eq!(
            script.replace(
                &format!(" & \" install_rc=$?; {extra} exit $install_rc\""),
                ""
            ),
            INSTALL_SCPT,
            "only the reset is added"
        );
    }

    #[test]
    fn install_script_without_extra_is_unchanged_and_bad_input_is_refused() {
        assert_eq!(
            install_script_with_extra(INSTALL_SCPT, "").expect("same"),
            INSTALL_SCPT
        );
        assert!(install_script_with_extra(INSTALL_SCPT, "echo \"x\"").is_err());
        assert!(install_script_with_extra(INSTALL_SCPT, "echo \\x").is_err());
        assert!(install_script_with_extra("on run {a}\nend run\n", "true;").is_err());
    }

    #[test]
    fn launch_restores_only_what_quit_took_offline() {
        assert!(should_restore_after_quit("Y", "Y"));
        assert!(!should_restore_after_quit("", "Y"), "stopped from settings");
        assert!(!should_restore_after_quit("Y", ""), "already online");
        assert!(!should_restore_after_quit("", ""));
    }

    fn adhoc_bundle(root: &std::path::Path, name: &str, payload: &[u8]) -> String {
        let bundle = root.join(format!("{name}.app"));
        let macos = bundle.join("Contents/MacOS");
        std::fs::create_dir_all(&macos).expect("bundle dirs");
        std::fs::write(
            bundle.join("Contents/Info.plist"),
            format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<plist version=\"1.0\"><dict>\
                 <key>CFBundleIdentifier</key><string>com.rdtest.privacy</string>\
                 <key>CFBundleExecutable</key><string>{name}</string></dict></plist>\n"
            ),
        )
        .expect("plist");
        let exe = macos.join(name);
        std::fs::copy("/usr/bin/true", &exe).expect("exe");
        let resources = bundle.join("Contents/Resources");
        std::fs::create_dir_all(&resources).expect("resources");
        std::fs::write(resources.join("payload"), payload).expect("payload");
        let status = Command::new("/usr/bin/codesign")
            .args(["--force", "--sign", "-"])
            .arg(&bundle)
            .status()
            .expect("codesign");
        assert!(status.success());
        bundle.to_string_lossy().into_owned()
    }

    #[test]
    fn real_adhoc_bundles_reset_only_when_identity_changes() {
        let root = std::env::temp_dir().join(format!("rdtest-tcc-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let old = adhoc_bundle(&root.join("old"), "Probe", b"build-1");
        let new = adhoc_bundle(&root.join("new"), "Probe", b"build-2");
        let same = adhoc_bundle(&root.join("same"), "Probe", b"build-1");

        assert!(read_designated_requirement(&old)
            .expect("requirement")
            .starts_with("cdhash H\""));
        assert_eq!(
            read_bundle_identifier(&new).as_deref(),
            Some("com.rdtest.privacy")
        );

        let changed =
            stale_privacy_reset_for_update(&old, &new, "privacy_uids", "/tmp/l", "/tmp/m/marker");
        assert!(changed.contains("/usr/bin/tccutil reset ScreenCapture 'com.rdtest.privacy'"));
        assert!(changed.contains("/usr/bin/tccutil reset Accessibility 'com.rdtest.privacy'"));

        let unchanged =
            stale_privacy_reset_for_update(&old, &same, "privacy_uids", "/tmp/l", "/tmp/m/marker");
        assert!(unchanged.contains("reset_stale_privacy() {"));
        assert!(!unchanged.contains("tccutil"));

        std::fs::remove_dir_all(&root).expect("cleanup");
    }
}
