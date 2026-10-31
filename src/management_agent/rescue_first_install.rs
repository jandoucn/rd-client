use super::{
    msi_metadata, package_gate, read, system_dirs, workspace, write_new, Artifact, Bundle, Layout,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    sync::mpsc,
    time::{Duration, Instant},
};

fn agent_dir() -> PathBuf {
    system_dirs::get().agent()
}
// 首装线程和救援状态、注册重连等线程在同一轮周期任务里一起被唤醒，后者会毫秒级地
// 短暂持有安装锁。立即放弃会让首装每轮都恰好撞上、永远报 busy；在这个窗口内重试即可，
// 真有长期持锁者时照旧报 busy。
const FIRST_INSTALL_GUARD_WAIT: Duration = Duration::from_secs(10);
const FIRST_INSTALL_GUARD_RETRY: Duration = Duration::from_millis(200);

fn acquire_guard_patiently<G>(
    mut attempt: impl FnMut() -> std::io::Result<Option<G>>,
    wait: Duration,
    retry: Duration,
) -> Result<G, String> {
    let deadline = Instant::now() + wait;
    loop {
        match attempt() {
            Ok(Some(guard)) => return Ok(guard),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(retry),
            Ok(None) => return Err("first installation busy".into()),
            Err(_) => return Err("first installation guard unavailable".into()),
        }
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Approval {
    schema: u32,
    product_code: String,
    manifest: serde_json::Value,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Receipt {
    schema: u32,
    source_kind: String,
    source_id: String,
    installation_id: String,
    target_key: String,
    source_reference: String,
    build_seq: u64,
    product_code: String,
    registered_source: PathBuf,
    package: Artifact,
    identity_sha256: String,
    client_pid: u32,
    agent_pid: u32,
    observed_at: u64,
}

pub(super) fn verify_source(root: &Path, bundle: &Bundle) -> Result<(), String> {
    if bundle.schema == 1
        && super::token(&bundle.source_transaction, 32)
        && bundle.source_kind.is_none()
        && bundle.installation_receipt.is_none()
    {
        return Ok(());
    }
    if bundle.schema != 2
        || !bundle.source_transaction.is_empty()
        || bundle.source_kind.as_deref() != Some("installed_msi")
    {
        return Err("rescue source kind mismatch".into());
    }
    let artifact = bundle
        .installation_receipt
        .as_ref()
        .ok_or("rescue installation receipt missing")?;
    if artifact.relative_path != "installation-receipt.json" || artifact.size > 65536 {
        return Err("rescue installation receipt reference mismatch".into());
    }
    workspace::verify_file(&root.join(&artifact.relative_path))?;
    artifact.verify(root)?;
    let receipt: Receipt =
        serde_json::from_slice(&read(&root.join(&artifact.relative_path), 65536, false)?)
            .map_err(|_| "rescue installation receipt malformed")?;
    if receipt.schema != 1
        || receipt.source_kind != "installed_msi"
        || !super::token(&receipt.source_id, 64)
        || receipt.source_id
            != source_id(
                &receipt.installation_id,
                &receipt.source_reference,
                &receipt.identity_sha256,
            )
        || receipt.installation_id != bundle.installation_id
        || receipt.target_key != bundle.target_key
        || receipt.source_reference != bundle.source_reference
        || receipt.build_seq != bundle.build_seq
        || receipt.package != bundle.package
        || receipt.identity_sha256 != bundle.identity_sha256
        || receipt.client_pid == 0
        || receipt.agent_pid == 0
        || receipt.observed_at != bundle.verified_at
        || !receipt.registered_source.is_absolute()
    {
        return Err("rescue installation receipt binding mismatch".into());
    }
    Ok(())
}
fn source_id(installation: &str, reference: &str, identity: &str) -> String {
    hex::encode(Sha256::digest(format!(
        "installed-msi-v1\n{installation}\n{reference}\n{identity}"
    )))
}

pub(super) fn check_gates(layout: &Layout, installation: &str, key: &str) -> Result<(), String> {
    let state = layout.agent.join("state");
    let data = system_dirs::get().rustdesk_data();
    let boundary = if layout.agent == agent_dir() {
        data.as_path()
    } else {
        &layout.agent
    };
    super::verify_existing_directory(&state, boundary)?;
    for name in ["disable-agent", "disable-recovery", "maintenance.json"] {
        match std::fs::symlink_metadata(state.join(name)) {
            Ok(_) => return Err("first installation locally blocked".into()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err("first installation gate unreadable".into()),
        }
    }
    workspace::verify_restore_target(&state.join("service-recovery-enabled"), false)?;
    let value: serde_json::Value =
        serde_json::from_slice(&read(&state.join("agent.json"), 65536, true)?)
            .map_err(|_| "first installation state malformed")?;
    if !super::token(installation, 32)
        || !matches!(
            key,
            "windows-x86_64-msi-sos" | "windows-x86_64-msi-standard"
        )
        || value["agent_installation_id"] != installation
        || value["target_key"] != key
        || value["management_agent_enabled"] != true
        || value["disabled"] != false
    {
        return Err("first installation state binding mismatch".into());
    }
    let bytes = read(&layout.config, 65536, true)?;
    let config: hbb_common::toml::Value = hbb_common::toml::from_str(
        std::str::from_utf8(&bytes).map_err(|_| "first installation intent encoding")?,
    )
    .map_err(|_| "first installation intent malformed")?;
    match config.get("options").and_then(|o| o.get("stop-service")) {
        None => {}
        Some(value) if matches!(value.as_str(), Some("" | "N")) => {}
        _ => return Err("first installation user stopped".into()),
    }
    let txs = layout.agent.join("transactions");
    match std::fs::symlink_metadata(&txs) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(_) => return Err("first installation transaction observation failed".into()),
        Ok(_) => {}
    }
    super::verify_existing_directory(&txs, boundary)?;
    for (count, entry) in std::fs::read_dir(txs)
        .map_err(|_| "first installation transactions unreadable")?
        .enumerate()
    {
        if count >= 64 {
            return Err("first installation transaction capacity exceeded".into());
        }
        let path = entry
            .map_err(|_| "first installation transaction unreadable")?
            .path();
        workspace::verify(&path)?;
        let tx = super::Transaction::decode(&read(&path.join("transaction.json"), 65536, false)?)?;
        if !matches!(tx.phase, super::Phase::Committed | super::Phase::RolledBack) {
            return Err("first installation active transaction".into());
        }
        match std::fs::symlink_metadata(path.join("rotation.json")) {
            Ok(_) => {
                let rotation: serde_json::Value =
                    serde_json::from_slice(&read(&path.join("rotation.json"), 4096, false)?)
                        .map_err(|_| "first installation rotation malformed")?;
                if rotation["phase"] != "published" {
                    return Err("first installation rotation pending".into());
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err("first installation rotation unreadable".into()),
        }
    }
    Ok(())
}

fn fixed_layout() -> Result<Layout, String> {
    #[cfg(test)]
    if let Some(layout) = WORKER_SCOPE.with(|v| v.borrow().as_ref().map(|s| super::fixture(&s.root))) {
        return Ok(layout);
    }
    let agent = agent_dir();
    let identity = hbb_common::config::Config::file();
    if ![
        system_dirs::get().local_service_roaming(),
        system_dirs::get().system_profile_roaming(),
    ]
    .iter()
    .any(|p| {
        identity
            .to_string_lossy()
            .eq_ignore_ascii_case(&p.join(r"RustDesk\config\RustDesk.toml").to_string_lossy())
    }) {
        return Err("first installation identity outside fixed profiles".into());
    }
    Ok(Layout {
        destination: agent.join("rescue"),
        transaction: PathBuf::new(),
        client: system_dirs::get().normal_install(),
        agent_exe: agent.join("rustdesk-management-agent.exe"),
        agent,
        config: identity.with_file_name("RustDesk2.toml"),
        identity,
        bootstrap: system_dirs::get().recovery_bootstrap(),
    })
}

fn live(layout: &Layout, key: &str, build: u64) -> Result<(u32, u32), String> {
    #[cfg(test)]
    if WORKER_SCOPE.with(|v| {
        if let Some(scope) = v.borrow().as_ref() {
            scope.health_calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            true
        } else { false }
    }) {
        return Err("isolated worker health unavailable".into());
    }
    use super::super::service_control::{self, Contract};
    use windows_service::service::{ServiceStartType, ServiceState};
    let client = Contract {
        name: "RustDesk".into(),
        executable: layout.client.join("RustDesk.exe"),
        arguments: vec!["--service".into()],
        start_type: ServiceStartType::AutoStart,
    };
    let agent = Contract {
        name: "RustDeskManagementAgent".into(),
        executable: layout.agent_exe.clone(),
        arguments: vec![
            "--service".into(),
            "--edition".into(),
            if key.ends_with("-sos") {
                "sos"
            } else {
                "standard"
            }
            .into(),
            "--package-kind".into(),
            "msi".into(),
            "--normal-install-path".into(),
            system_dirs::get()
                .normal_install()
                .to_string_lossy()
                .into_owned(),
        ],
        start_type: ServiceStartType::AutoStart,
    };
    let a = service_control::inspect(&agent)?;
    let c = service_control::inspect(&client)?;
    if a.state != ServiceState::Running || c.state != ServiceState::Running {
        return Err("first installation service not healthy".into());
    }
    let reply = super::super::health_windows::probe_blocking(
        r"\\.\pipe\RustDesk\management-health-v2",
        &client.executable,
        build,
        Duration::from_secs(3),
    )?;
    if reply.no_active_sessions != Some(true) {
        return Err("first installation sessions active or unknown".into());
    }
    Ok((
        reply.pid,
        a.pid.ok_or("first installation agent PID missing")?,
    ))
}

fn absent(path: &Path) -> Result<bool, String> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Ok(false),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(true),
        Err(_) => Err("first installation path unreadable".into()),
    }
}

/// Remove a staging directory left by an attempt that did not finish. Its contents are
/// this flow's own scratch copies, recreated from the registered package, so nothing
/// here is an input another flow depends on. Refuse a reparse point: following one would
/// delete whatever it points at.
fn reclaim_staging(stage: &Path) -> Result<(), String> {
    use std::os::windows::fs::MetadataExt;
    let metadata = std::fs::symlink_metadata(stage)
        .map_err(|_| "first installation staging unreadable")?;
    if metadata.file_attributes() & 0x400 != 0 {
        return Err("first installation staging is a reparse point".into());
    }
    std::fs::remove_dir_all(stage).map_err(|_| "first installation staging not reclaimed".into())
}

/// Approval delivery refuses to run while `first-install-staging` exists, and only
/// `prepare` reclaimed it, after an approval was present. A leftover from a failed
/// attempt whose approval was later discarded therefore blocked delivery for good.
/// `prepare` only runs on this worker thread after this check, so without an
/// approval nothing here is using the directory; the guard keeps other processes'
/// install transactions out while it is removed.
fn reclaim_orphaned_staging(layout: &Layout) -> Result<bool, String> {
    let sidecar = layout.agent.join("release-cache/first-install.json");
    let stage = layout.destination.join("first-install-staging");
    let gated = || -> Result<bool, String> {
        for name in ["disable-agent", "disable-recovery", "maintenance.json"] {
            if !absent(&layout.agent.join("state").join(name))? {
                return Ok(true);
            }
        }
        Ok(false)
    };
    if !absent(&sidecar)? || absent(&stage)? || gated()? {
        return Ok(false);
    }
    let _guard = acquire_guard_patiently(
        super::InstallGuard::try_acquire,
        FIRST_INSTALL_GUARD_WAIT,
        FIRST_INSTALL_GUARD_RETRY,
    )?;
    if !absent(&sidecar)? || absent(&stage)? || gated()? {
        return Ok(false);
    }
    reclaim_staging(&stage)?;
    Ok(true)
}

const ROTATION: &str = "first-install-rotation";

#[derive(Debug, PartialEq)]
enum ExistingBundle {
    Keep,
    Preserve,
    Rotate,
}

/// A client updater upgrade installs a newer MSI without touching `rescue/current`,
/// which then restores the old build and its old product code. Any older bundle is
/// rotated; a same-or-newer bundle keeps the previous behaviour.
fn existing_bundle_action(
    old_schema: u32,
    old_build: u64,
    old_reference: &str,
    identity_matches: bool,
    build: u64,
    reference: &str,
) -> ExistingBundle {
    if old_build < build {
        // An older build can only restore an outdated client, whatever identity it was
        // captured with; the running build replaces it and it is kept as `previous`.
        ExistingBundle::Rotate
    } else if !identity_matches {
        ExistingBundle::Preserve
    } else if old_schema == 2 && old_reference == reference {
        ExistingBundle::Keep
    } else {
        ExistingBundle::Preserve
    }
}

fn current_superseded(bundle_build: Option<u64>, agent_build: Option<u64>, key: &str) -> bool {
    matches!(
        key,
        "windows-x86_64-msi-standard" | "windows-x86_64-msi-sos"
    ) && matches!((bundle_build, agent_build), (Some(bundle), Some(agent)) if bundle < agent)
}

fn agent_build() -> Option<u64> {
    serde_json::from_str::<serde_json::Value>(include_str!("../../version.json")).ok()?["build_seq"]
        .as_u64()
}

fn json_field(path: &Path, identity: bool, field: &str) -> Option<serde_json::Value> {
    let value =
        serde_json::from_slice::<serde_json::Value>(&read(path, 65536, identity).ok()?).ok()?;
    value.get(field).cloned()
}

fn current_needs_rotation(layout: &Layout) -> bool {
    let key = json_field(&layout.agent.join("state/agent.json"), true, "target_key");
    let bundle = json_field(
        &layout.destination.join("current/bundle.json"),
        false,
        "build_seq",
    );
    current_superseded(
        bundle.and_then(|value| value.as_u64()),
        agent_build(),
        key.as_ref()
            .and_then(|value| value.as_str())
            .unwrap_or_default(),
    )
}

#[cfg(not(test))]
fn finish_interrupted_rotation(layout: &Layout) -> Result<bool, String> {
    let rotation = layout.agent.join(ROTATION);
    if absent(&rotation)? {
        return Ok(false);
    }
    let state = layout.agent.join("state/agent.json");
    let installation = json_field(&state, true, "agent_installation_id")
        .and_then(|value| value.as_str().map(str::to_owned))
        .ok_or("first installation identity missing")?;
    let key = json_field(&state, true, "target_key")
        .and_then(|value| value.as_str().map(str::to_owned))
        .ok_or("first installation target missing")?;
    let _guard = acquire_guard_patiently(
        super::InstallGuard::try_acquire,
        FIRST_INSTALL_GUARD_WAIT,
        FIRST_INSTALL_GUARD_RETRY,
    )?;
    if locally_gated(&layout.agent)? {
        return Ok(false);
    }
    // The receipt binds the exact bundle.json digests of all three images, and they were
    // fully checked (identity, downgrade) before it was written, so verify is enough here.
    finish_pending_rotation(&layout.destination, &rotation, |path| {
        super::verify(path, &installation, &key, None).map(|_| ())
    })
}

/// Publishes a first-install staging over an existing bundle through the shared
/// receipt-driven rotation: current becomes previous, an older previous is retired
/// with the rotation directory. The directory is created before anything moves so
/// an interruption at any point is finished by `finish_pending_rotation`.
fn rotate_staged(
    destination: &Path,
    rotation: &Path,
    stage: &Path,
    validate: impl Fn(&Path) -> Result<(), String>,
    fault: Option<&str>,
) -> Result<(), String> {
    use super::rescue_rotation::{digest, rotate};
    if !absent(rotation)? {
        return Err("first installation rotation pending".into());
    }
    let old = digest(&destination.join("current"))?
        .ok_or("first installation rotation source missing")?;
    let previous = digest(&destination.join("previous"))?;
    let staging = destination.join("staging");
    if !absent(&staging)? {
        return Err("first installation occupied assets preserved".into());
    }
    workspace::create(rotation)?;
    std::fs::rename(stage, &staging)
        .map_err(|e| format!("first installation staging not moved: {e}"))?;
    let new = digest(&staging)?.ok_or("first installation staged metadata absent")?;
    rotate(
        destination,
        rotation,
        &old,
        &new,
        previous.as_deref(),
        validate,
        fault,
    )?;
    clean_rotation(rotation)
}

/// The receipt goes first: a partial delete (for example a file held by an antivirus
/// scan) then leaves a directory without receipt, which the next cleanup removes,
/// instead of a receipt whose retired archive no longer verifies.
fn clean_rotation(rotation: &Path) -> Result<(), String> {
    match std::fs::remove_file(rotation.join("rotation.json")) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
            return Err(format!(
                "first installation rotation receipt not cleared: {e}"
            ))
        }
        _ => {}
    }
    std::fs::remove_dir_all(rotation)
        .map_err(|e| format!("first installation rotation not cleaned: {e}"))
}

fn locally_gated(agent: &Path) -> Result<bool, String> {
    for name in ["disable-agent", "disable-recovery", "maintenance.json"] {
        if !absent(&agent.join("state").join(name))? {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Finishes a rotation left by an interruption. Without a receipt nothing has moved
/// yet, so the staging it had claimed is reclaimed and current stays as it was.
fn finish_pending_rotation(
    destination: &Path,
    rotation: &Path,
    validate: impl Fn(&Path) -> Result<(), String>,
) -> Result<bool, String> {
    use super::rescue_rotation::{binding, rotate};
    if absent(rotation)? {
        return Ok(false);
    }
    match binding(rotation)? {
        Some((old, new, previous)) => rotate(
            destination,
            rotation,
            &old,
            &new,
            previous.as_deref(),
            validate,
            None,
        )?,
        None => {
            let staging = destination.join("staging");
            if !absent(&staging)? {
                // An agent transaction rotation also stages into rescue/staging; only
                // this flow's own installed-MSI bundle is reclaimed.
                let kind = json_field(&staging.join("bundle.json"), false, "source_kind");
                if kind.as_ref().and_then(|value| value.as_str()) != Some("installed_msi") {
                    return Err("first installation rotation staging not owned".into());
                }
                reclaim_staging(&staging)?;
            }
        }
    }
    clean_rotation(rotation)?;
    Ok(true)
}

fn rotation_compatible(
    path: &Path,
    installation: &str,
    key: &str,
    build: u64,
    identity_sha256: &str,
    public: Option<&[u8]>,
) -> Result<(), String> {
    super::verify(path, installation, key, public)?;
    let bundle: Bundle = serde_json::from_slice(&read(&path.join("bundle.json"), 65536, false)?)
        .map_err(|_| "first installation rotation bundle malformed")?;
    rotation_image_allowed(
        bundle.build_seq,
        &bundle.identity_sha256,
        build,
        identity_sha256,
    )
}

/// Rotation validates every image it touches: the staged one, `current` before and
/// after publishing, and `previous`. The image of the running build must carry the
/// current identity. An older-build image is only being retired and may have been
/// captured under an identity this machine no longer has (reinstall, new ID); it
/// still passes `super::verify` (signature, installation, target) but is not held to
/// the current identity, or it could never be replaced.
fn rotation_image_allowed(
    image_build: u64,
    image_identity: &str,
    build: u64,
    identity_sha256: &str,
) -> Result<(), String> {
    if image_build > build {
        return Err("first installation rotation downgrade refused".into());
    }
    if image_build == build && image_identity != identity_sha256 {
        return Err("first installation rotation identity mismatch".into());
    }
    Ok(())
}

fn waiting() -> serde_json::Value {
    serde_json::json!({"state":"unavailable","error_code":"waiting_approval","ready":false})
}

/// The worker's `first-install-status` report. A refused preparation keeps its raw
/// error for local diagnostics and adds a stable `reason`; a failed or still-waiting
/// approval coordination adds `coordination_error`. Status forwards only those codes.
fn first_install_report(
    prepared: Result<serde_json::Value, String>,
    coordination: Option<&str>,
    build: Option<u64>,
) -> serde_json::Value {
    let mut report = match prepared {
        Ok(report) => report,
        Err(error) => serde_json::json!({"state":"unavailable","error_code":"preparation_refused",
            "reason":refusal_reason(&error),"error":error,"ready":false}),
    };
    if let Some(fields) = report.as_object_mut() {
        if let Some(code) = coordination {
            fields.insert("coordination_error".into(), code.into());
        }
        // Status forwards the reason only while the report describes the running build.
        if let Some(build) = build {
            fields.insert("agent_build".into(), build.into());
        }
    }
    report
}

/// Groups preparation refusals by what an administrator can do about them.
fn refusal_reason(error: &str) -> &'static str {
    const REASONS: [(&[&str], &str); 11] = [
        (&["sessions active"], "sessions_active"),
        (&["live source changed"], "source_changed"),
        (&["locally blocked", "user stopped"], "locally_blocked"),
        (
            &["busy", "guard unavailable", "active transaction"],
            "installation_busy",
        ),
        (&["service not healthy", "health"], "client_unhealthy"),
        (&["bootstrap not installed"], "bootstrap_missing"),
        (
            &["existing bundle preserved", "identity mismatch"],
            "existing_bundle_preserved",
        ),
        (&["rotation pending"], "rotation_pending"),
        (&["occupied", "staging preserved"], "staging_occupied"),
        (
            &["registered", "installed MSI source", "MSI binding"],
            "registered_source_mismatch",
        ),
        (&["package"], "package_invalid"),
    ];
    REASONS
        .iter()
        .find(|(needles, _)| needles.iter().any(|needle| error.contains(needle)))
        .map_or("preparation_refused", |(_, reason)| reason)
}

/// First-install approval and the rescue bundle exist for MSI installs only. An EXE
/// install has no approved package source and no MSI product code, so reporting
/// `waiting_approval` described a decision that would never be made.
fn not_applicable() -> serde_json::Value {
    serde_json::json!({"state":"unavailable","error_code":"unsupported_package_kind","ready":false})
}

fn fallback_compatible(
    layout: &Layout,
    key: &str,
    client: &super::DirectoryBackup,
) -> Result<(), String> {
    let known = known_client_manifests(layout)?;
    fallback_compatible_for(&known, &fallback_dir(layout, key), client)
}
/// 升级轮换时，现有保底目录是旧救援包发布的副本，用新包清单去比对必然不同。
/// 旧救援包自己记录了当时的清单，只有它才说明这个目录是不是我们自己放的；对不上才保留。
fn fallback_superseded(known: &[super::DirectoryBackup], fallback: &Path) -> Result<bool, String> {
    match std::fs::symlink_metadata(fallback) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(_) => return Err("R3 fallback unreadable".into()),
        Ok(_) => {}
    }
    Ok(known
        .iter()
        .any(|bundle| bundle.matches_directory(fallback).unwrap_or(false)))
}

fn fallback_compatible_for(
    known: &[super::DirectoryBackup],
    fallback: &Path,
    client: &super::DirectoryBackup,
) -> Result<(), String> {
    if fallback_superseded(known, fallback)? {
        return Ok(());
    }
    super::r3_existing_fallback(client, fallback).map(|_| ())
}

/// 旧救援包留下的保底目录随轮换一起换成新版。先复制到旁边并逐文件核对，
/// 再两次目录改名完成替换；改名失败时把原目录改回来，不留半换状态。
fn refresh_fallback(
    fallback: &Path,
    source: &Path,
    expected: &super::DirectoryBackup,
) -> Result<(), String> {
    let parent = fallback
        .parent()
        .ok_or("first installation fallback parent absent")?;
    workspace::verify(parent)?;
    let working = parent.join("RustDesk.new");
    let retired = parent.join("RustDesk.old");
    if !absent(&working)? {
        reclaim_staging(&working)?;
    }
    // 上一次替换在「原目录已改名、新副本未就位」时中断，RustDesk.old 就是保底目录的唯一
    // 副本；这时直接删掉它等于把保底客户端弄丢，先把目录改回来再继续。
    if !absent(&retired)? {
        if absent(fallback)? {
            std::fs::rename(&retired, fallback)
                .map_err(|e| format!("first installation fallback not restored: {e}"))?;
        } else {
            reclaim_staging(&retired)?;
        }
    }
    let copied = super::directory_backup::capture(source, &working)?;
    if copied != *expected {
        return Err("first installation fallback client changed during rotation".into());
    }
    let staged = working.join("files");
    if absent(&staged)? {
        return Err("first installation fallback staging missing".into());
    }
    std::fs::rename(fallback, &retired)
        .map_err(|e| format!("first installation fallback not retired: {e}"))?;
    if let Err(error) = std::fs::rename(&staged, fallback) {
        return Err(match std::fs::rename(&retired, fallback) {
            Ok(()) => format!("first installation fallback not replaced: {error}"),
            Err(restore) => format!(
                "first installation fallback not replaced: {error}; old directory not restored: {restore}"
            ),
        });
    }
    // 核对失败也要把原目录换回来；这里用 match 而不是 `?`，否则核对出错会直接返回，
    // 让新副本留在原位、原目录滞留在 RustDesk.old。
    let verified = copied.matches_directory(fallback);
    if !matches!(verified, Ok(true)) {
        let restored =
            std::fs::remove_dir_all(fallback).and_then(|()| std::fs::rename(&retired, fallback));
        let detail = match verified {
            Ok(_) => "inventory mismatch".to_owned(),
            Err(error) => format!("inventory unreadable: {error}"),
        };
        return Err(match restored {
            Ok(()) => format!("first installation fallback replaced {detail}"),
            Err(restore) => format!(
                "first installation fallback replaced {detail}; old directory not restored: {restore}"
            ),
        });
    }
    let _ = std::fs::remove_dir_all(&retired);
    let _ = std::fs::remove_dir_all(&working);
    Ok(())
}

/// 保底目录可能由 `current` 或已轮换到 `previous` 的救援包发布；两处的清单都要拿来比对。
/// 清单不存在就跳过，但存在却读不出、解不开时必须报错：那说明我们无法判断目录归属，
/// 静默跳过会让保底目录永远停在「不是我们的」，轮换每次都白白失败。
fn known_client_manifests(layout: &Layout) -> Result<Vec<super::DirectoryBackup>, String> {
    let mut known = Vec::new();
    for name in ["current", "previous"] {
        let path = layout.destination.join(name).join("client/manifest.json");
        if absent(&path)? {
            continue;
        }
        let bytes = read(&path, 8 * 1024 * 1024, false)?;
        known.push(
            super::DirectoryBackup::decode(&bytes)
                .map_err(|e| format!("{name} client manifest unusable: {e}"))?,
        );
    }
    Ok(known)
}

#[cfg_attr(not(test), allow(unused_variables))]
fn fallback_dir(layout: &Layout, key: &str) -> PathBuf {
    let edition = if key.ends_with("-sos") {
        "sos"
    } else {
        "standard"
    };
    #[cfg(test)]
    if layout.agent != agent_dir() {
        return layout.agent.join("fallback");
    }
    super::fallback_path(edition)
}

fn fallback_blocked(error: String) -> serde_json::Value {
    serde_json::json!({"state":"degraded","rescue_readiness":"degraded","ready":false,"published":false,"blocked_reason":"existing_fallback_incompatible","error_code":"existing_fallback_incompatible","error":error,"legacy_route_preserved":true})
}
#[cfg(test)]
pub fn fallback_compatibility_fixture(
    root: &Path,
    inventory: &super::DirectoryBackup,
) -> Result<(), String> {
    fallback_compatible(&super::fixture(root), "windows-x86_64-msi-sos", inventory)
}

fn installed_acl(path: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows::{
        core::{PCWSTR, PWSTR},
        Win32::{
            Foundation::{LocalFree, HLOCAL},
            Security::{
                Authorization::{ConvertSidToStringSidW, GetNamedSecurityInfoW, SE_FILE_OBJECT},
                GetAce, ACCESS_ALLOWED_ACE, ACE_HEADER, ACL, DACL_SECURITY_INFORMATION,
                OWNER_SECURITY_INFORMATION, PSID,
            },
        },
    };
    const TRUSTED: &str = "S-1-5-80-956008885-3418522649-1831038044-1853292631-2271478464";
    struct Memory(*mut std::ffi::c_void);
    impl Drop for Memory {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe {
                    if !LocalFree(Some(HLOCAL(self.0))).0.is_null() {
                        eprintln!("first installation ACL cleanup failed");
                    }
                }
            }
        }
    }
    let sid = |value: PSID| -> Result<String, String> {
        let mut text = PWSTR::null();
        unsafe { ConvertSidToStringSidW(value, &mut text) }
            .map_err(|_| "first installation ACL SID unavailable")?;
        let _memory = Memory(text.0.cast());
        unsafe { text.to_string() }.map_err(|_| "first installation ACL SID encoding".into())
    };
    let wide = path
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let mut owner = PSID::default();
    let mut acl: *mut ACL = std::ptr::null_mut();
    let mut descriptor = Default::default();
    let status = unsafe {
        GetNamedSecurityInfoW(
            PCWSTR(wide.as_ptr()),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            Some(&mut owner),
            None,
            Some(&mut acl),
            None,
            &mut descriptor,
        )
    };
    let _memory = Memory(descriptor.0);
    if status.0 != 0 || acl.is_null() || owner.0.is_null() {
        return Err("first installation ACL unavailable".into());
    }
    if !["S-1-5-18", "S-1-5-32-544", TRUSTED].contains(&sid(owner)?.as_str()) {
        return Err("first installation owner untrusted".into());
    }
    let mut required = [false, false];
    for index in 0..u32::from(unsafe { (*acl).AceCount }) {
        let mut pointer = std::ptr::null_mut();
        unsafe { GetAce(acl, index, &mut pointer) }
            .map_err(|_| "first installation ACE unavailable")?;
        if pointer.is_null() {
            return Err("first installation ACE missing".into());
        }
        let header = unsafe { &*pointer.cast::<ACE_HEADER>() };
        if header.AceType != 0
            || usize::from(header.AceSize) < std::mem::size_of::<ACCESS_ALLOWED_ACE>()
        {
            return Err("first installation ACE unsupported".into());
        }
        if header.AceFlags & 0x08 != 0 {
            continue;
        }
        let ace = unsafe { &*pointer.cast::<ACCESS_ALLOWED_ACE>() };
        let name = sid(PSID((&ace.SidStart as *const u32).cast_mut().cast()))?;
        match name.as_str() {
            "S-1-5-18" | "S-1-5-32-544" => {
                if ace.Mask & 0x001f01ff == 0x001f01ff {
                    required[usize::from(name != "S-1-5-18")] = true;
                }
            }
            TRUSTED => {}
            _ if ace.Mask & !0x001200a9 == 0 => {}
            _ => return Err("first installation non-admin write permission refused".into()),
        }
    }
    if required != [true, true] {
        return Err("first installation administrator access incomplete".into());
    }
    Ok(())
}
#[cfg(test)]
pub fn installed_acl_fixture(path: &Path) -> Result<(), String> {
    installed_acl(path)
}

fn bounded_inventory(root: &Path, deadline: Instant) -> Result<super::DirectoryBackup, String> {
    use std::{
        io::Read,
        os::windows::fs::{MetadataExt, OpenOptionsExt},
    };
    let mut pending = vec![root.to_path_buf()];
    let mut directories = Vec::new();
    let mut files = Vec::new();
    let mut total = 0u64;
    while let Some(directory) = pending.pop() {
        let meta = std::fs::symlink_metadata(&directory)
            .map_err(|_| "first installation inventory directory unavailable")?;
        if !meta.is_dir() || meta.file_attributes() & 0x400 != 0 {
            return Err("first installation inventory directory invalid".into());
        }
        installed_acl(&directory)?;
        for entry in
            std::fs::read_dir(&directory).map_err(|_| "first installation inventory unreadable")?
        {
            if Instant::now() >= deadline || directories.len() + files.len() >= 20000 {
                return Err("first installation inventory budget exceeded".into());
            }
            let path = entry
                .map_err(|_| "first installation inventory unreadable")?
                .path();
            let meta = std::fs::symlink_metadata(&path)
                .map_err(|_| "first installation inventory unavailable")?;
            if meta.file_attributes() & 0x400 != 0 {
                return Err("first installation inventory reparse refused".into());
            }
            let name = path
                .strip_prefix(root)
                .map_err(|_| "first installation inventory outside root")?
                .to_str()
                .ok_or("first installation inventory encoding")?
                .replace('\\', "/");
            if meta.is_dir() {
                directories.push(name);
                pending.push(path);
                continue;
            }
            if !meta.is_file() {
                return Err("first installation inventory object invalid".into());
            }
            total = total
                .checked_add(meta.len())
                .ok_or("first installation inventory overflow")?;
            if total > 8 * 1024 * 1024 * 1024 {
                return Err("first installation inventory byte budget exceeded".into());
            }
            installed_acl(&path).map_err(|error| {
                format!("first installation inventory {}: {error}", path.display())
            })?;
            let mut file = std::fs::OpenOptions::new()
                .read(true)
                .share_mode(1)
                .custom_flags(0x00200000)
                .open(&path)
                .map_err(|_| "first installation inventory open failed")?;
            let mut hash = Sha256::new();
            let mut count = 0u64;
            let mut buffer = [0u8; 65536];
            loop {
                if Instant::now() >= deadline {
                    return Err("first installation inventory deadline exceeded".into());
                }
                let n = file
                    .read(&mut buffer)
                    .map_err(|_| "first installation inventory read failed")?;
                if n == 0 {
                    break;
                }
                count += n as u64;
                if count > meta.len() {
                    return Err("first installation inventory size changed".into());
                }
                hash.update(&buffer[..n]);
            }
            if count != meta.len() {
                return Err("first installation inventory size changed".into());
            }
            files.push(super::directory_backup::BackupFile {
                path: name,
                size: count,
                sha256: hex::encode(hash.finalize()),
            });
        }
    }
    directories.sort();
    files.sort_by(|a, b| a.path.cmp(&b.path));
    let result = super::DirectoryBackup {
        schema: 1,
        directories,
        files,
    };
    super::DirectoryBackup::decode(
        &serde_json::to_vec(&result).map_err(|_| "first installation inventory encoding")?,
    )
}

fn capture_client(
    source: &Path,
    stage: &Path,
    deadline: Instant,
) -> Result<super::DirectoryBackup, String> {
    use std::{
        io::{Read, Write},
        os::windows::fs::OpenOptionsExt,
    };
    let inventory = bounded_inventory(source, deadline)?;
    let backup = stage.join("client");
    workspace::create(&backup)?;
    let files = backup.join("files");
    workspace::create(&files)?;
    let mut directories = inventory.directories.clone();
    directories.sort_by_key(|p| p.split('/').count());
    for directory in directories {
        workspace::create(&files.join(directory))?;
    }
    for item in &inventory.files {
        let mut input = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(1)
            .custom_flags(0x00200000)
            .open(source.join(&item.path))
            .map_err(|_| "first installation copy source unavailable")?;
        let mut output = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(files.join(&item.path))
            .map_err(|_| "first installation copy destination occupied")?;
        let mut count = 0u64;
        let mut hash = Sha256::new();
        let mut buffer = [0u8; 65536];
        loop {
            if Instant::now() >= deadline {
                return Err("first installation copy deadline exceeded".into());
            }
            let n = input
                .read(&mut buffer)
                .map_err(|_| "first installation copy read failed")?;
            if n == 0 {
                break;
            }
            count += n as u64;
            if count > item.size {
                return Err("first installation copy size changed".into());
            }
            hash.update(&buffer[..n]);
            output
                .write_all(&buffer[..n])
                .map_err(|_| "first installation copy write failed")?;
        }
        if count != item.size || hex::encode(hash.finalize()) != item.sha256 {
            return Err("first installation copy hash mismatch".into());
        }
        output
            .sync_all()
            .map_err(|_| "first installation copy sync failed")?;
    }
    write_new(
        &backup.join("manifest.json"),
        &serde_json::to_vec(&inventory).map_err(|_| "first installation inventory encoding")?,
    )?;
    Ok(inventory)
}

fn copy_registered(
    package: &Path,
    stage: &Path,
    name: &str,
    target: &hbb_common::UpdateTarget,
    deadline: Instant,
) -> Result<Artifact, String> {
    use std::{
        io::{Read, Write},
        os::windows::fs::OpenOptionsExt,
    };
    // Ordinary MSI registration can point to the installing user's directory. Authority is
    // the signed target plus a held deny-write/delete handle, not that directory's owner.
    let _locked = package_gate::lock_verified_file(package, target)?;
    let mut input = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(1)
        .custom_flags(0x00200000)
        .open(package)
        .map_err(|_| "first installation registered source unavailable")?;
    let mut output = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(stage.join(name))
        .map_err(|_| "first installation package destination occupied")?;
    let mut count = 0u64;
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 65536];
    loop {
        if Instant::now() >= deadline {
            return Err("first installation package copy deadline exceeded".into());
        }
        let n = input
            .read(&mut buffer)
            .map_err(|_| "first installation package read failed")?;
        if n == 0 {
            break;
        }
        count += n as u64;
        if count > target.size {
            return Err("first installation package size changed".into());
        }
        hash.update(&buffer[..n]);
        output
            .write_all(&buffer[..n])
            .map_err(|_| "first installation package write failed")?;
    }
    let hash = hex::encode(hash.finalize());
    if count != target.size || hash != target.sha256 {
        return Err("first installation package integrity mismatch".into());
    }
    output
        .sync_all()
        .map_err(|_| "first installation package sync failed")?;
    drop(output);
    workspace::verify_file(&stage.join(name))?;
    Ok(Artifact {
        relative_path: name.into(),
        size: count,
        sha256: hash,
    })
}

fn prepare(
    layout: &Layout,
    public: Option<&[u8]>,
    registered: impl Fn(&str, &str, &hbb_common::UpdateTarget) -> Result<PathBuf, String>,
    healthy: impl Fn(&Layout, &str, u64) -> Result<(u32, u32), String>,
) -> Result<serde_json::Value, String> {
    let deadline = Instant::now() + Duration::from_secs(600);
    let budget = || {
        if Instant::now() < deadline {
            Ok(())
        } else {
            Err("first installation budget exceeded".to_owned())
        }
    };
    // Read the state first: first-install approval only exists for MSI, and an EXE
    // install has no approved source to wait for. Checking the sidecar first made every
    // EXE install report `waiting_approval`, which the backend could only mask by
    // looking at the package kind.
    let state: serde_json::Value =
        serde_json::from_slice(&read(&layout.agent.join("state/agent.json"), 65536, true)?)
            .map_err(|_| "first installation state malformed")?;
    let installation = state["agent_installation_id"]
        .as_str()
        .ok_or("first installation identity missing")?;
    let key = state["target_key"]
        .as_str()
        .ok_or("first installation target missing")?;
    if !matches!(
        key,
        "windows-x86_64-msi-sos" | "windows-x86_64-msi-standard"
    ) {
        return Ok(not_applicable());
    }
    let cache = layout.agent.join("release-cache");
    let sidecar = cache.join("first-install.json");
    if absent(&sidecar)? {
        return Ok(waiting());
    }
    workspace::verify(&cache)?;
    let approved = read(&sidecar, 262144, false)?;
    let approval: Approval =
        serde_json::from_slice(&approved).map_err(|_| "first installation approval malformed")?;
    if approval.schema != 1 {
        return Err("first installation approval schema invalid".into());
    }
    let _guard = acquire_guard_patiently(
        super::InstallGuard::try_acquire,
        FIRST_INSTALL_GUARD_WAIT,
        FIRST_INSTALL_GUARD_RETRY,
    )?;
    check_gates(layout, installation, key)?;
    let manifest: hbb_common::UpdateManifest = serde_json::from_value(approval.manifest.clone())
        .map_err(|_| "first installation manifest malformed")?;
    let target = super::signed_target(&manifest, key, 1, public)?;
    let (version, build, _) = base::update::update_target_release(&manifest, target)
        .map_err(|_| "first installation release invalid")?;
    let reference = package_gate::package_reference(&approval.manifest, key)?;
    let name = package_gate::cache_name(&target.primary, "msi")?;
    if target.size > 2 * 1024 * 1024 * 1024 {
        return Err("first installation package budget exceeded".into());
    }
    let package_path = registered(&approval.product_code, &name, target)?;
    super::rotation_registered_source(&layout.destination, &package_path)?;
    let _package = package_gate::lock_verified_file(&package_path, target)?;
    let msi = msi_metadata::inspect(&package_path)?;
    if msi.product_name != "RustDesk"
        || !msi
            .product_code
            .eq_ignore_ascii_case(&approval.product_code)
    {
        return Err("first installation MSI binding mismatch".into());
    }
    let (client_pid, agent_pid) = healthy(layout, key, build)?;
    let identity = read(&layout.identity, 65536, true)?;
    super::valid_identity(&identity)?;
    let config = read(&layout.config, 65536, true)?;
    let current = layout.destination.join("current");
    let mut rotating = false;
    if !absent(&current)? {
        let value = super::verify(&current, installation, key, public)?;
        let old: Bundle =
            serde_json::from_slice(&read(&current.join("bundle.json"), 65536, false)?)
                .map_err(|_| "first installation bundle malformed")?;
        match existing_bundle_action(
            old.schema,
            old.build_seq,
            &old.source_reference,
            old.identity_sha256 == hex::encode(Sha256::digest(&identity)),
            build,
            &reference,
        ) {
            ExistingBundle::Keep => return Ok(value),
            ExistingBundle::Preserve => {
                return Err("first installation existing bundle preserved".into())
            }
            ExistingBundle::Rotate => rotating = true,
        }
    }
    if rotating && !absent(&layout.agent.join(ROTATION))? {
        return Err("first installation rotation pending".into());
    }
    if (!rotating && !absent(&layout.destination.join("previous"))?)
        || !absent(&layout.destination.join("staging"))?
    {
        return Err("first installation occupied assets preserved".into());
    }
    // The recovery bootstrap is installed by the agent tick, not by this worker. Check it
    // before creating anything: a missing directory there used to fail after the staging
    // copy with "install package path unavailable", leaving a staging directory behind.
    if absent(&layout.bootstrap.join("rustdesk-recovery-bootstrap.exe"))? {
        return Err("first installation bootstrap not installed".into());
    }
    budget()?;
    if absent(&layout.destination)? {
        workspace::create(&layout.destination)?;
    } else {
        workspace::verify(&layout.destination)?;
    }
    // `previous` and `staging` belong to other flows and are left alone above.
    // `first-install-staging` is this flow's own scratch directory and publication
    // renames it to `current`, so one that still exists belongs to an attempt that did
    // not finish. Refusing it made every retry reject its own leftover, and
    // `rescue/current` was then never published on any machine that had failed once.
    let stage = layout.destination.join("first-install-staging");
    if !absent(&stage)? {
        reclaim_staging(&stage)?;
    }
    workspace::create(&stage)?;
    let package = copy_registered(&package_path, &stage, &name, target, deadline)?;
    write_new(
        &stage.join("approved-manifest.json"),
        &serde_json::to_vec(&approval.manifest)
            .map_err(|_| "first installation manifest encoding")?,
    )?;
    let extracted = match public {
        #[cfg(test)]
        Some(key) => super::msi_assets::extract_fixture(
            &stage,
            &stage.join(&name),
            &manifest,
            &state["target_key"].as_str().ok_or("target missing")?,
            key,
        )?,
        _ => super::msi_assets::extract(&stage, &stage.join(&name), &manifest, key, build)?,
    };
    budget()?;
    let client = capture_client(&extracted.client, &stage, deadline)?;
    if client != bounded_inventory(&layout.client, deadline)? {
        return Err("first installation current client mismatch".into());
    }
    if let Err(error) = fallback_compatible(layout, key, &client) {
        return Ok(fallback_blocked(error));
    }
    let agent = super::copy(&stage, &extracted.agent, "agent.exe", 512 * 1024 * 1024)?;
    let expected = hbb_common::UpdateTarget {
        size: agent.size,
        sha256: agent.sha256.clone(),
        ..Default::default()
    };
    let _agent = package_gate::lock_verified_file(&layout.agent_exe, &expected)?;
    let _bootstrap = package_gate::lock_verified_file(
        &layout.bootstrap.join("rustdesk-recovery-bootstrap.exe"),
        &expected,
    )?;
    let bootstrap = super::copy(
        &stage,
        &layout.bootstrap.join("rustdesk-recovery-bootstrap.exe"),
        "bootstrap.exe",
        512 * 1024 * 1024,
    )?;
    let bootstrap_manifest = super::copy(
        &stage,
        &layout.bootstrap.join("bootstrap.json"),
        "bootstrap.json",
        65536,
    )?;
    write_new(
        &stage.join("identity.dpapi"),
        &super::machine_protect(&identity, false)?,
    )?;
    write_new(
        &stage.join("config.dpapi"),
        &super::machine_protect(&config, false)?,
    )?;
    let identity_sha256 = hex::encode(Sha256::digest(&identity));
    let time = super::now()?;
    let receipt = Receipt {
        schema: 1,
        source_kind: "installed_msi".into(),
        source_id: source_id(installation, &reference, &identity_sha256),
        installation_id: installation.into(),
        target_key: key.into(),
        source_reference: reference.clone(),
        build_seq: build,
        product_code: msi.product_code,
        registered_source: package_path.clone(),
        package: package.clone(),
        identity_sha256: identity_sha256.clone(),
        client_pid,
        agent_pid,
        observed_at: time,
    };
    write_new(
        &stage.join("installation-receipt.json"),
        &serde_json::to_vec(&receipt).map_err(|_| "first installation receipt encoding")?,
    )?;
    let bundle = Bundle {
        schema: 2,
        installation_id: installation.into(),
        source_transaction: String::new(),
        source_kind: Some("installed_msi".into()),
        installation_receipt: Some(super::describe(&stage, "installation-receipt.json", 65536)?),
        source_reference: reference,
        target_key: key.into(),
        version: version.into(),
        build_seq: build,
        verified_at: time,
        package,
        client_manifest: super::describe(&stage, "client/manifest.json", 8 * 1024 * 1024)?,
        agent,
        bootstrap,
        bootstrap_manifest,
        identity: super::describe(&stage, "identity.dpapi", 131072)?,
        config: super::describe(&stage, "config.dpapi", 131072)?,
        identity_sha256: identity_sha256.clone(),
        config_sha256: hex::encode(Sha256::digest(&config)),
    };
    write_new(
        &stage.join("bundle.json"),
        &serde_json::to_vec(&bundle).map_err(|_| "first installation bundle encoding")?,
    )?;
    super::verify_readonly_status(&stage, installation, key, public)?;
    budget()?;
    check_gates(layout, installation, key)?;
    if healthy(layout, key, build)? != (client_pid, agent_pid)
        || registered(&approval.product_code, &name, target)? != package_path
        || read(&sidecar, 262144, false)? != approved
        || read(&layout.identity, 65536, true)? != identity
        || read(&layout.config, 65536, true)? != config
    {
        return Err("first installation live source changed; staging preserved".into());
    }
    if !rotating && !absent(&layout.destination.join("previous"))? {
        return Err("first installation previous appeared; staging preserved".into());
    }
    let fallback = fallback_dir(layout, key);
    let known = known_client_manifests(layout)?;
    // 这里只判定一次：保底目录的整目录哈希不便宜，而它在轮换前后都一样。
    let refresh = rotating && fallback_superseded(&known, &fallback)?;
    if let Err(error) = fallback_compatible_for(&known, &fallback, &client) {
        return Ok(fallback_blocked(error));
    }
    if rotating {
        rotate_staged(
            &layout.destination,
            &layout.agent.join(ROTATION),
            &stage,
            |path| rotation_compatible(path, installation, key, build, &identity_sha256, public),
            None,
        )?;
    } else {
        super::publish(&stage, &current)?;
    }
    // The verified lock denies deletion; release it so the signed copy can go now.
    drop(_package);
    discard_signed_sources(&layout.agent, None);
    // 保底目录换来换去会让旧包清单永远对不上，轮换成功后顺带把它换成同一版。
    let mut refreshed = false;
    if refresh {
        if let Err(error) = refresh_fallback(&fallback, &layout.client, &client) {
            hbb_common::throttled_log!(
                Duration::from_secs(300),
                warn,
                "first installation fallback not refreshed: {error}"
            );
        } else {
            refreshed = true;
        }
    }
    Ok(
        serde_json::json!({"rescue_readiness":"degraded","source_kind":"installed_msi","source_transaction":null,"source_reference":bundle.source_reference,"fallback_refreshed":refreshed,"client":"verified","offline_package":"verified","credentials":"verified","agent_backup":"verified","bootstrap_backup":"verified"}),
    )
}

fn registered(code: &str, name: &str, target: &hbb_common::UpdateTarget) -> Result<PathBuf, String> {
    #[cfg(test)]
    if let Some(result) = WORKER_SCOPE.with(|v| v.borrow().as_ref().map(|s| {
        if s.product_code.eq_ignore_ascii_case(code) && s.source.file_name().and_then(|v|v.to_str()) == Some(name) {
            Ok(s.source.clone())
        } else { Err("isolated registered source mismatch".to_owned()) }
    })) { return result; }
    let (registered_name, source) = msi_metadata::registered_source(code)?;
    if registered_name != name {
        return Err("first installation registered basename mismatch".into());
    }
    match source {
        // Local repair installs from `rescue\current`, which then becomes the registered source;
        // rotation cannot copy from the bundle it is about to move, so it takes the signed copy.
        Some(source) if !inside(&source, &agent_dir().join("rescue")) => Ok(source),
        _ => signed_source(&agent_dir(), code, target),
    }
}

fn inside(path: &Path, root: &Path) -> bool {
    let text = |p: &Path| p.to_string_lossy().replace('/', "\\").to_ascii_lowercase();
    let (path, root) = (text(path), text(root));
    path == root || path.starts_with(&format!("{}\\", root.trim_end_matches('\\')))
}

const SIGNED_SOURCE_PREFIX: &str = "rescue-source-";

fn signed_source_dir(agent: &Path, target: &hbb_common::UpdateTarget) -> Result<PathBuf, String> {
    let key = target
        .sha256
        .get(..16)
        .filter(|key| key.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .ok_or("first installation package hash invalid")?;
    Ok(agent
        .join("release-cache")
        .join(format!("{SIGNED_SOURCE_PREFIX}{}", key.to_ascii_lowercase())))
}

/// The online updater installs from a temporary directory that is deleted afterwards, so the
/// registered source is often gone by the time the rescue bundle is rotated. Fetch the signed
/// package itself (hash- and size-checked against the approved manifest) and accept it only
/// when Windows Installer recorded exactly this package for the installed product.
fn signed_source(agent: &Path, code: &str, target: &hbb_common::UpdateTarget) -> Result<PathBuf, String> {
    let directory = signed_source_dir(agent, target)?;
    discard_signed_sources(agent, Some(&directory));
    let reused = directory.exists();
    let (path, _locked) = match super::super::package_download::download(&directory, target) {
        Ok(package) => package,
        // A previous attempt can leave a partial or foreign file under the reused directory;
        // a fresh download that failed is simply retried on the next round.
        Err(_) if reused => {
            std::fs::remove_dir_all(&directory)
                .map_err(|_| "first installation signed source not reclaimed")?;
            super::super::package_download::download(&directory, target)?
        }
        Err(error) => return Err(error),
    };
    msi_metadata::bind_candidate(code, &path)?;
    Ok(path)
}

/// Signed copies are only needed until the bundle is published; `keep` is the one in use.
fn discard_signed_sources(agent: &Path, keep: Option<&Path>) {
    let Ok(entries) = std::fs::read_dir(agent.join("release-cache")) else {
        return;
    };
    for entry in entries.flatten().take(64) {
        let path = entry.path();
        let ours = entry
            .file_name()
            .to_str()
            .is_some_and(|name| name.starts_with(SIGNED_SOURCE_PREFIX));
        if ours && Some(path.as_path()) != keep {
            let _ = std::fs::remove_dir_all(&path);
        }
    }
}
#[cfg(test)]
pub fn prepare_fixture(agent: &Path, public: &[u8]) -> Result<serde_json::Value, String> {
    prepare(
        &super::fixture(agent),
        Some(public),
        |_, name, _| Ok(agent.join("release-cache").join(name)),
        |_, _, _| Ok((1, 2)),
    )
}

#[cfg(test)]
pub fn prepare_native_resume_source_fixture(public: &[u8]) -> Result<serde_json::Value, String> {
    if !base::lab_guard::is_lab_os("win11") {
        return Err("native source fixture requires Win11".into());
    }
    let mut layout = fixed_layout()?;
    layout.destination = PathBuf::from(r"C:\RustDeskAgentLab\native-resume-source-b");
    prepare(&layout, Some(public), registered, live)
}
#[cfg(test)]
pub fn fallback_change_fixture(
    agent: &Path,
    public: &[u8],
    change: bool,
) -> Result<serde_json::Value, String> {
    let calls = std::cell::Cell::new(0);
    prepare(
        &super::fixture(agent),
        Some(public),
        |_, name, _| Ok(agent.join("release-cache").join(name)),
        |_, _, _| {
            calls.set(calls.get() + 1);
            if change && calls.get() == 2 {
                std::fs::write(
                    agent.join("fallback/librustdesk.dll"),
                    b"changed before publication",
                )
                .map_err(|e| e.to_string())?;
            }
            Ok((1, 2))
        },
    )
}

pub struct Worker {
    sender: mpsc::SyncSender<()>,
}
impl Worker {
    pub fn notify(&self) -> Result<(), String> {
        match self.sender.try_send(()) {
            Ok(()) | Err(mpsc::TrySendError::Full(_)) => Ok(()),
            Err(_) => Err("first installation worker unavailable".into()),
        }
    }
}
fn start_checked(public: Option<Vec<u8>>, #[cfg(test)] scope: Option<WorkerScope>) -> std::io::Result<Worker> {
    let (sender, receiver) = mpsc::sync_channel(1);
    std::thread::Builder::new().name("agent-rescue-first-install".into()).spawn(move||{
        #[cfg(test)]
        WORKER_SCOPE.with(|v| *v.borrow_mut() = scope);
        let mut last=None;
        #[allow(unused_mut, unused_variables)]
        let mut pruned:Option<Instant>=None;
        for () in receiver{
            if last.is_some_and(|time:Instant|time.elapsed()<Duration::from_secs(300)){continue;}last=Some(Instant::now());
            // Finished transactions pile up with every update; past 64 entries approval delivery stops.
            #[cfg(not(test))]
            if pruned.map_or(true,|time:Instant|time.elapsed()>=Duration::from_secs(6*3600)){
                pruned=Some(Instant::now());
                match super::super::transaction_retention::prune(&agent_dir(),std::time::SystemTime::now()){
                    Ok(0)=>{}
                    Ok(count)=>hbb_common::log::info!("finished transactions pruned: {count}"),
                    Err(error)=>hbb_common::throttled_log!(Duration::from_secs(300),warn,"finished transactions not pruned: {error}"),
                }
            }
            let result=(||{let layout=fixed_layout()?;
                #[cfg(not(test))]
                if let Err(error)=finish_interrupted_rotation(&layout){hbb_common::throttled_log!(Duration::from_secs(300),warn,"first installation rotation not finished: {error}");}
                if !absent(&layout.destination.join("current"))? && !current_needs_rotation(&layout){return Ok(());}
                #[cfg(not(test))]
                if let Err(error)=super::super::first_install_approval_delivery::discard_superseded_approval(){hbb_common::throttled_log!(Duration::from_secs(300),warn,"first installation superseded approval check unavailable: {error:?}");}
                #[cfg(not(test))]
                if let Err(error)=reclaim_orphaned_staging(&layout){hbb_common::throttled_log!(Duration::from_secs(300),warn,"first installation orphaned staging not reclaimed: {error}");}
                #[allow(unused_mut)]
                let mut coordination: Option<&'static str> = None;
                #[cfg(not(test))]
                if absent(&layout.agent.join("release-cache/first-install.json"))? {
                    coordination=match super::super::first_install_approval_delivery::coordinate_current_installation(){
                        Ok(outcome)=>outcome.pending_code(),
                        Err(error)=>{hbb_common::throttled_log!(Duration::from_secs(300),warn,"first installation approval unavailable: {error:?}");Some(error.code())}
                    };
                }
                #[cfg(test)]
                if absent(&layout.agent.join("release-cache/first-install.json"))? {
                    WORKER_SCOPE.with(|v| {
                        if let Some(scope) = v.borrow().as_ref() {
                            let outcome = (scope.coordinate)(&scope.root);
                            scope.coordination_results.lock().unwrap().push(outcome);
                        }
                    });
                }
                let report=first_install_report(prepare(&layout,public.as_deref(),registered,live),coordination,agent_build());
                let reports=layout.agent.join("first-install-status");if absent(&reports)?{workspace::create(&reports)?;}workspace::verify(&reports)?;
                super::super::state_store::publish(&reports.join("last-check.json"),&serde_json::to_vec(&report).map_err(|_|"first installation status encoding")?).map_err(|e|e.to_string())?;
                Ok::<(),String>(())})();
            if let Err(error)=result{hbb_common::throttled_log!(Duration::from_secs(300),warn,"first installation preparation unavailable: {error}");}
        }
    })?;
    Ok(Worker { sender })
}
pub fn start() -> std::io::Result<Worker> {
    start_checked(None, #[cfg(test)] None)
}
#[cfg(test)]
pub fn start_fixture(public: &[u8]) -> std::io::Result<Worker> {
    start_checked(Some(public.to_vec()), None)
}

#[cfg(test)]
std::thread_local! { static WORKER_SCOPE: std::cell::RefCell<Option<WorkerScope>> = const { std::cell::RefCell::new(None) }; }

#[cfg(test)]
pub struct WorkerScope {
    pub root: PathBuf,
    pub source: PathBuf,
    pub product_code: String,
    pub health_calls: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    pub coordination_results: std::sync::Arc<std::sync::Mutex<Vec<Result<(), String>>>>,
    pub coordinate: Box<dyn Fn(&Path) -> Result<(), String> + Send>,
}

#[cfg(test)]
pub fn start_scoped_fixture(scope: WorkerScope) -> std::io::Result<Worker> {
    start_checked(None, Some(scope))
}

#[cfg(test)]
mod orphaned_staging_tests {
    use super::{super::fixture, reclaim_orphaned_staging};
    use std::path::PathBuf;

    fn root(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "rd-orphaned-staging-{name}-{}",
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(root.join("rescue/first-install-staging/image")).unwrap();
        std::fs::write(root.join("rescue/first-install-staging/extract.log"), b"x").unwrap();
        root
    }

    // 审批投递只要看到 first-install-staging 就报 Busy，而回收它的 prepare
    // 又要先有审批；失败遗留的 staging 因此把首装救援永久卡死。
    #[test]
    fn a_leftover_staging_without_approval_is_reclaimed() {
        let root = root("reclaim");
        assert_eq!(reclaim_orphaned_staging(&fixture(&root)), Ok(true));
        assert!(!root.join("rescue/first-install-staging").exists());
        assert!(root.join("rescue").exists());
        std::fs::remove_dir_all(&root).unwrap();
    }

    // 有审批时 staging 归 prepare 管，这里不能动它。
    #[test]
    fn a_staging_with_an_approval_is_left_to_prepare() {
        let root = root("approved");
        std::fs::create_dir_all(root.join("release-cache")).unwrap();
        std::fs::write(root.join("release-cache/first-install.json"), b"{}").unwrap();
        assert_eq!(reclaim_orphaned_staging(&fixture(&root)), Ok(false));
        assert!(root
            .join("rescue/first-install-staging/extract.log")
            .exists());
        std::fs::remove_dir_all(&root).unwrap();
    }

    // 本地禁用、停用恢复和维护状态优先于任何清理。
    #[test]
    fn a_local_gate_keeps_the_staging() {
        for gate in ["disable-agent", "disable-recovery", "maintenance.json"] {
            let root = root("gate");
            std::fs::create_dir_all(root.join("state")).unwrap();
            std::fs::write(root.join("state").join(gate), b"").unwrap();
            assert_eq!(reclaim_orphaned_staging(&fixture(&root)), Ok(false));
            assert!(
                root.join("rescue/first-install-staging/extract.log")
                    .exists(),
                "{gate}"
            );
            std::fs::remove_dir_all(&root).unwrap();
        }
    }

    // staging 被换成联接点时拒绝回收，联接点指向的内容必须原样保留。
    #[test]
    fn a_reparse_point_staging_is_refused_and_its_target_kept() {
        let root = root("reparse");
        // mklink does not accept forward slashes.
        let stage = root.join("rescue").join("first-install-staging");
        std::fs::remove_dir_all(&stage).unwrap();
        let target = root.join("elsewhere");
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(target.join("keep.txt"), b"keep").unwrap();
        let made = std::process::Command::new("cmd")
            .args(["/c", "mklink", "/J"])
            .arg(&stage)
            .arg(&target)
            .output()
            .unwrap();
        assert!(made.status.success());
        assert!(reclaim_orphaned_staging(&fixture(&root)).is_err());
        assert!(target.join("keep.txt").exists());
        std::fs::remove_dir(&stage).unwrap();
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn nothing_happens_without_a_staging() {
        let root = root("absent");
        std::fs::remove_dir_all(root.join("rescue/first-install-staging")).unwrap();
        assert_eq!(reclaim_orphaned_staging(&fixture(&root)), Ok(false));
        std::fs::remove_dir_all(&root).unwrap();
    }
}

#[cfg(test)]
mod guard_wait_tests {
    use super::acquire_guard_patiently;
    use std::{cell::Cell, io, time::Duration};

    // 同一轮周期任务里，救援状态、注册重连等线程会和首装线程一起被唤醒，
    // 它们毫秒级的短暂持锁曾让首装每 5 分钟都恰好撞上，永远报 busy。
    #[test]
    fn a_briefly_held_guard_is_acquired_after_waiting() {
        let calls = Cell::new(0);
        let result = acquire_guard_patiently(
            || {
                calls.set(calls.get() + 1);
                Ok(if calls.get() < 3 { None } else { Some(7) })
            },
            Duration::from_secs(2),
            Duration::from_millis(1),
        );
        assert_eq!(result, Ok(7));
        assert_eq!(calls.get(), 3);
    }

    // 真有长期持锁者时，等待有上限，照旧报 busy，不能无限阻塞首装线程。
    #[test]
    fn a_guard_held_past_the_window_still_reports_busy() {
        let result = acquire_guard_patiently::<u8>(
            || Ok(None),
            Duration::from_millis(30),
            Duration::from_millis(5),
        );
        assert_eq!(result, Err("first installation busy".to_owned()));
    }

    // 系统调用失败不是「被占用」，不应重试，直接报不可用。
    #[test]
    fn a_failing_guard_is_not_retried() {
        let calls = Cell::new(0);
        let result = acquire_guard_patiently::<u8>(
            || {
                calls.set(calls.get() + 1);
                Err(io::Error::other("denied"))
            },
            Duration::from_secs(2),
            Duration::from_millis(1),
        );
        assert_eq!(
            result,
            Err("first installation guard unavailable".to_owned())
        );
        assert_eq!(calls.get(), 1);
    }
}

#[cfg(test)]
mod rotation_tests {
    use super::{
        current_superseded, existing_bundle_action, finish_pending_rotation, locally_gated,
        rotate_staged, workspace, ExistingBundle,
    };
    use std::path::{Path, PathBuf};

    // 客户端更新器升级 MSI 后救援包一直停在旧构建：首装线程一看到 current 就退出。
    #[test]
    fn only_an_older_msi_bundle_is_superseded() {
        let msi = "windows-x86_64-msi-standard";
        assert!(current_superseded(Some(2026102503), Some(2026102601), msi));
        assert!(current_superseded(
            Some(1),
            Some(2),
            "windows-x86_64-msi-sos"
        ));
        assert!(!current_superseded(Some(2), Some(2), msi));
        assert!(!current_superseded(Some(3), Some(2), msi));
        assert!(!current_superseded(
            Some(1),
            Some(2),
            "windows-x86_64-exe-standard"
        ));
        assert!(!current_superseded(None, Some(2), msi));
        assert!(!current_superseded(Some(1), None, msi));
    }

    // 构建号相等（或救援包更新、读不到构建号）时，首装线程在投递审批、丢弃旧审批
    // 之前就退出，行为与引入轮换前完全一致。
    #[test]
    fn an_up_to_date_bundle_keeps_the_worker_idle() {
        for key in ["windows-x86_64-msi-standard", "windows-x86_64-msi-sos"] {
            assert!(!current_superseded(Some(2026102601), Some(2026102601), key));
            assert!(!current_superseded(Some(2026102602), Some(2026102601), key));
            assert!(!current_superseded(None, Some(2026102601), key));
        }
    }

    #[test]
    fn an_existing_bundle_is_kept_rotated_or_preserved() {
        let keep = existing_bundle_action(2, 5, "r", true, 5, "r");
        assert_eq!(keep, ExistingBundle::Keep);
        assert_eq!(
            existing_bundle_action(2, 4, "old", true, 5, "r"),
            ExistingBundle::Rotate
        );
        assert_eq!(
            existing_bundle_action(1, 4, "old", true, 5, "r"),
            ExistingBundle::Rotate
        );
        // 旧构建的包一律换成当前运行主程序的构建，身份不一致（重装、换身份后留下的）也一样，
        // 旧包轮换后保留为 previous，不会丢。
        assert_eq!(
            existing_bundle_action(2, 4, "old", false, 5, "r"),
            ExistingBundle::Rotate
        );
        assert_eq!(
            existing_bundle_action(1, 4, "old", false, 5, "r"),
            ExistingBundle::Rotate
        );
        // 同构建或更新构建且身份不一致、降级、同构建不同来源、同构建旧格式：保留原包，不替换。
        assert_eq!(
            existing_bundle_action(2, 5, "r", false, 5, "r"),
            ExistingBundle::Preserve
        );
        assert_eq!(
            existing_bundle_action(2, 6, "new", false, 5, "r"),
            ExistingBundle::Preserve
        );
        assert_eq!(
            existing_bundle_action(2, 6, "new", true, 5, "r"),
            ExistingBundle::Preserve
        );
        assert_eq!(
            existing_bundle_action(2, 5, "other", true, 5, "r"),
            ExistingBundle::Preserve
        );
        assert_eq!(
            existing_bundle_action(1, 5, "r", true, 5, "r"),
            ExistingBundle::Preserve
        );
    }

    // 2603 实测：升级后轮换用新包清单校验上一版留下的保底目录，必然不一致，
    // 救援包停在旧构建、R3 也无法恢复。上一版救援包记录的清单才对得上，应视为自家旧副本。
    #[test]
    fn a_fallback_from_the_previous_bundle_is_superseded_not_preserved() {
        let root = std::env::temp_dir().join(format!(
            "rd-fallback-superseded-{}",
            uuid::Uuid::new_v4().simple()
        ));
        workspace::create(&root).unwrap();
        let fallback = root.join("fallback");
        workspace::create(&fallback).unwrap();
        std::fs::write(fallback.join("RustDesk.exe"), b"previous build").unwrap();
        let previous =
            super::super::directory_backup::capture(&fallback, &root.join("previous-manifest"))
                .unwrap();

        assert_eq!(
            super::fallback_superseded(&[previous.clone()], &fallback),
            Ok(true)
        );
        assert!(super::fallback_compatible_for(&[previous.clone()], &fallback, &previous).is_ok());

        std::fs::remove_dir_all(&root).unwrap();
    }

    // 保底目录不是上一版救援包放的（用户自备、被改过、或根本没有上一版），必须保留不覆盖。
    #[test]
    fn an_unknown_fallback_is_preserved() {
        let root = std::env::temp_dir().join(format!(
            "rd-fallback-unknown-{}",
            uuid::Uuid::new_v4().simple()
        ));
        workspace::create(&root).unwrap();
        let fallback = root.join("fallback");
        workspace::create(&fallback).unwrap();
        std::fs::write(fallback.join("RustDesk.exe"), b"previous build").unwrap();
        let previous =
            super::super::directory_backup::capture(&fallback, &root.join("previous-manifest"))
                .unwrap();

        std::fs::write(fallback.join("RustDesk.exe"), b"user replaced").unwrap();
        assert_eq!(
            super::fallback_superseded(&[previous.clone()], &fallback),
            Ok(false)
        );
        std::fs::remove_file(fallback.join("RustDesk.exe")).unwrap();
        std::fs::write(fallback.join("extra.dll"), b"user file").unwrap();
        assert_eq!(
            super::fallback_superseded(&[previous.clone()], &fallback),
            Ok(false)
        );
        assert_eq!(super::fallback_superseded(&[], &fallback), Ok(false));

        std::fs::remove_dir_all(&root).unwrap();
    }

    // 保底目录不存在时不需要任何判定，直接放行（首发安装就是这种情况）。
    #[test]
    fn an_absent_fallback_needs_no_decision() {
        let root = std::env::temp_dir().join(format!(
            "rd-fallback-absent-{}",
            uuid::Uuid::new_v4().simple()
        ));
        workspace::create(&root).unwrap();
        assert_eq!(
            super::fallback_superseded(&[], &root.join("missing")),
            Ok(false)
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    // 保底目录换成新版：内容逐文件核对一致，旧的换下来的目录不留在磁盘上。
    #[test]
    fn a_superseded_fallback_is_replaced_in_place() {
        let root = std::env::temp_dir().join(format!(
            "rd-fallback-refresh-{}",
            uuid::Uuid::new_v4().simple()
        ));
        workspace::create(&root).unwrap();
        let fallback = root.join("fallback");
        let source = root.join("client");
        for (dir, body) in [(&fallback, &b"old build"[..]), (&source, &b"new build"[..])] {
            workspace::create(dir).unwrap();
            std::fs::create_dir_all(dir.join("data")).unwrap();
            std::fs::write(dir.join("RustDesk.exe"), body).unwrap();
            std::fs::write(dir.join("data/plugin.dll"), body).unwrap();
        }
        let expected =
            super::super::directory_backup::capture(&source, &root.join("expected")).unwrap();

        super::refresh_fallback(&fallback, &source, &expected).unwrap();

        assert!(expected.matches_directory(&fallback).unwrap());
        assert!(!root.join("RustDesk.new").exists());
        assert!(!root.join("RustDesk.old").exists());
        std::fs::remove_dir_all(&root).unwrap();
    }

    // 上一次替换在「原目录已改名、新副本未就位」时中断：RustDesk.old 是保底目录的唯一
    // 副本，必须改回来，不能被当成垃圾删掉。
    #[test]
    fn an_interrupted_replacement_restores_the_retired_directory() {
        let root = std::env::temp_dir().join(format!(
            "rd-fallback-resume-{}",
            uuid::Uuid::new_v4().simple()
        ));
        workspace::create(&root).unwrap();
        let fallback = root.join("fallback");
        let source = root.join("client");
        for (dir, body) in [
            (&root.join("RustDesk.old"), &b"old build"[..]),
            (&source, &b"new build"[..]),
        ] {
            workspace::create(dir).unwrap();
            std::fs::write(dir.join("RustDesk.exe"), body).unwrap();
        }
        let expected =
            super::super::directory_backup::capture(&source, &root.join("expected")).unwrap();

        super::refresh_fallback(&fallback, &source, &expected).unwrap();

        assert!(expected.matches_directory(&fallback).unwrap());
        assert!(!root.join("RustDesk.old").exists());
        std::fs::remove_dir_all(&root).unwrap();
    }

    // 新客户端与救援包记录不一致时不动现有保底目录：报告错误，原目录与内容保持不变。
    #[test]
    fn a_changed_client_keeps_the_existing_fallback() {
        let root = std::env::temp_dir().join(format!(
            "rd-fallback-kept-{}",
            uuid::Uuid::new_v4().simple()
        ));
        workspace::create(&root).unwrap();
        let fallback = root.join("fallback");
        let source = root.join("client");
        for (dir, body) in [(&fallback, &b"old build"[..]), (&source, &b"new build"[..])] {
            workspace::create(dir).unwrap();
            std::fs::write(dir.join("RustDesk.exe"), body).unwrap();
        }
        let expected =
            super::super::directory_backup::capture(&source, &root.join("expected")).unwrap();
        std::fs::write(source.join("RustDesk.exe"), b"changed after approval").unwrap();

        assert!(super::refresh_fallback(&fallback, &source, &expected).is_err());

        assert_eq!(
            std::fs::read(fallback.join("RustDesk.exe")).unwrap(),
            b"old build"
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    struct Fixture {
        root: PathBuf,
    }
    impl Fixture {
        fn new(previous: bool) -> Self {
            let root = std::env::temp_dir().join(format!(
                "rd-rescue-rotation-{}",
                uuid::Uuid::new_v4().simple()
            ));
            workspace::create(&root).unwrap();
            workspace::create(&root.join("rescue")).unwrap();
            let f = Self { root };
            f.bundle(&f.dest().join("current"), 1);
            if previous {
                f.bundle(&f.dest().join("previous"), 0);
            }
            f.bundle(&f.dest().join("first-install-staging"), 2);
            f
        }
        fn dest(&self) -> PathBuf {
            self.root.join("rescue")
        }
        fn rotation(&self) -> PathBuf {
            self.root.join("first-install-rotation")
        }
        fn bundle(&self, path: &Path, build: u64) {
            workspace::create(path).unwrap();
            std::fs::write(
                path.join("bundle.json"),
                format!(r#"{{"schema":2,"source_kind":"installed_msi","build_seq":{build}}}"#),
            )
            .unwrap();
        }
        fn build(&self, name: &str) -> Option<u64> {
            let bytes = std::fs::read(self.dest().join(name).join("bundle.json")).ok()?;
            serde_json::from_slice::<serde_json::Value>(&bytes).ok()?["build_seq"].as_u64()
        }
        fn stage(&self) -> PathBuf {
            self.dest().join("first-install-staging")
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn a_staged_bundle_replaces_current_and_keeps_it_as_previous() {
        let f = Fixture::new(false);
        rotate_staged(&f.dest(), &f.rotation(), &f.stage(), |_| Ok(()), None).unwrap();
        assert_eq!(f.build("current"), Some(2));
        assert_eq!(f.build("previous"), Some(1));
        assert!(!f.stage().exists() && !f.dest().join("staging").exists());
        assert!(!f.rotation().exists());
    }

    #[test]
    fn an_older_previous_is_retired_with_the_rotation() {
        let f = Fixture::new(true);
        rotate_staged(&f.dest(), &f.rotation(), &f.stage(), |_| Ok(()), None).unwrap();
        assert_eq!(f.build("current"), Some(2));
        assert_eq!(f.build("previous"), Some(1));
        assert!(!f.rotation().exists());
    }

    // 校验不通过时不动现有救援包，暂存由收尾回收，不留下阻塞后续轮换的残留。
    #[test]
    fn a_refused_bundle_leaves_current_untouched() {
        let f = Fixture::new(false);
        let refused = rotate_staged(
            &f.dest(),
            &f.rotation(),
            &f.stage(),
            |_| Err("refused".into()),
            None,
        );
        assert!(refused.is_err());
        assert_eq!(f.build("current"), Some(1));
        assert!(finish_pending_rotation(&f.dest(), &f.rotation(), |_| Ok(())).unwrap());
        assert_eq!(f.build("current"), Some(1));
        assert!(!f.dest().join("staging").exists() && !f.rotation().exists());
    }

    // 中途掉电或进程被结束：下次收尾按回执续做到底。
    #[test]
    fn an_interrupted_rotation_is_finished_from_its_receipt() {
        for fault in [
            "after_previous_move",
            "after_current_move",
            "after_publish_move",
        ] {
            let f = Fixture::new(true);
            let result = rotate_staged(
                &f.dest(),
                &f.rotation(),
                &f.stage(),
                |_| Ok(()),
                Some(fault),
            );
            assert!(result.is_err(), "{fault}");
            assert!(
                finish_pending_rotation(&f.dest(), &f.rotation(), |_| Ok(())).unwrap(),
                "{fault}"
            );
            assert_eq!(f.build("current"), Some(2), "{fault}");
            assert_eq!(f.build("previous"), Some(1), "{fault}");
            assert!(!f.rotation().exists(), "{fault}");
        }
    }

    #[test]
    fn a_pending_rotation_blocks_a_new_one() {
        let f = Fixture::new(false);
        workspace::create(&f.rotation()).unwrap();
        assert!(rotate_staged(&f.dest(), &f.rotation(), &f.stage(), |_| Ok(()), None).is_err());
        assert_eq!(f.build("current"), Some(1));
        assert_eq!(f.build("first-install-staging"), Some(2));
    }

    // 没有回执时 rescue/staging 可能属于 Agent 事务轮换：只回收首装自己的暂存。
    #[test]
    fn a_staging_without_receipt_is_reclaimed_only_when_it_is_ours() {
        let f = Fixture::new(false);
        workspace::create(&f.rotation()).unwrap();
        let staging = f.dest().join("staging");
        workspace::create(&staging).unwrap();
        std::fs::write(
            staging.join("bundle.json"),
            br#"{"schema":1,"build_seq":2}"#,
        )
        .unwrap();
        assert!(finish_pending_rotation(&f.dest(), &f.rotation(), |_| Ok(())).is_err());
        assert!(staging.join("bundle.json").exists());
        assert_eq!(f.build("current"), Some(1));
        std::fs::write(
            staging.join("bundle.json"),
            br#"{"schema":2,"source_kind":"installed_msi","build_seq":2}"#,
        )
        .unwrap();
        assert!(finish_pending_rotation(&f.dest(), &f.rotation(), |_| Ok(())).unwrap());
        assert!(!staging.exists() && !f.rotation().exists());
        assert_eq!(f.build("current"), Some(1));
    }

    // 本地禁用、停用恢复和维护状态下，收尾也不移动或删除任何救援资产。
    #[test]
    fn a_local_gate_is_reported_for_rotation_cleanup() {
        for gate in ["disable-agent", "disable-recovery", "maintenance.json"] {
            let root = std::env::temp_dir().join(format!(
                "rd-rotation-gate-{}",
                uuid::Uuid::new_v4().simple()
            ));
            std::fs::create_dir_all(root.join("state")).unwrap();
            assert!(!locally_gated(&root).unwrap());
            std::fs::write(root.join("state").join(gate), b"").unwrap();
            assert!(locally_gated(&root).unwrap(), "{gate}");
            std::fs::remove_dir_all(&root).unwrap();
        }
    }

    #[test]
    fn nothing_is_pending_without_a_rotation_directory() {
        let f = Fixture::new(false);
        assert!(!finish_pending_rotation(&f.dest(), &f.rotation(), |_| Ok(())).unwrap());
        assert_eq!(f.build("current"), Some(1));
    }
}

#[cfg(test)]
mod rotation_image_tests {
    use super::*;

    // 身份不一致的旧构建包要能被轮换退役，否则“旧构建一律轮换”只会每 5 分钟空转一次。
    #[test]
    fn an_older_image_with_another_identity_can_be_retired() {
        assert_eq!(rotation_image_allowed(4, "old-id", 5, "new-id"), Ok(()));
    }

    #[test]
    fn the_running_build_image_must_carry_the_current_identity() {
        assert_eq!(rotation_image_allowed(5, "new-id", 5, "new-id"), Ok(()));
        assert_eq!(
            rotation_image_allowed(5, "old-id", 5, "new-id"),
            Err("first installation rotation identity mismatch".into())
        );
    }

    #[test]
    fn a_newer_image_is_never_accepted() {
        assert_eq!(
            rotation_image_allowed(6, "new-id", 5, "new-id"),
            Err("first installation rotation downgrade refused".into())
        );
    }
}

#[cfg(test)]
mod report_tests {
    use super::*;

    // 批准协调失败只打日志时，后台只能看到「救援包旧」，看不到为什么没换；原因码要落进报告。
    #[test]
    fn a_coordination_failure_is_recorded_beside_the_report() {
        let report = first_install_report(Ok(waiting()), Some("registered_source_mismatch"), None);
        assert_eq!(report["error_code"], "waiting_approval");
        assert_eq!(report["coordination_error"], "registered_source_mismatch");
    }

    #[test]
    fn a_successful_coordination_adds_nothing() {
        let report = first_install_report(Ok(waiting()), None, None);
        assert_eq!(report, waiting());
    }

    #[test]
    fn a_refused_preparation_carries_a_stable_reason() {
        let report = first_install_report(
            Err("first installation sessions active or unknown".into()),
            Some("locally_blocked"),
            None,
        );
        assert_eq!(report["state"], "unavailable");
        assert_eq!(report["error_code"], "preparation_refused");
        assert_eq!(report["reason"], "sessions_active");
        assert_eq!(report["coordination_error"], "locally_blocked");
        assert_eq!(report["ready"], false);
    }

    // 报告带上生成时的 Agent 构建，状态只转发与当前构建对应的原因，升级后不会把上一轮的旧原因当成现在的。
    #[test]
    fn the_report_records_the_agent_build() {
        let report = first_install_report(Ok(waiting()), None, Some(2026103001));
        assert_eq!(report["agent_build"], 2026103001);
    }

    #[test]
    fn refusals_are_classified_into_actionable_reasons() {
        for (error, reason) in [
            (
                "first installation sessions active or unknown",
                "sessions_active",
            ),
            ("first installation busy", "installation_busy"),
            ("first installation guard unavailable", "installation_busy"),
            ("first installation locally blocked", "locally_blocked"),
            ("first installation user stopped", "locally_blocked"),
            ("first installation service not healthy", "client_unhealthy"),
            (
                "first installation bootstrap not installed",
                "bootstrap_missing",
            ),
            (
                "first installation existing bundle preserved",
                "existing_bundle_preserved",
            ),
            ("first installation rotation pending", "rotation_pending"),
            (
                "first installation occupied assets preserved",
                "staging_occupied",
            ),
            (
                "first installation previous appeared; staging preserved",
                "staging_occupied",
            ),
            (
                "first installation registered basename mismatch",
                "registered_source_mismatch",
            ),
            (
                "installed MSI source query failed: 1605",
                "registered_source_mismatch",
            ),
            (
                "first installation MSI binding mismatch",
                "registered_source_mismatch",
            ),
            (
                "first installation package integrity mismatch",
                "package_invalid",
            ),
            (
                "first installation live source changed; staging preserved",
                "source_changed",
            ),
            ("first installation active transaction", "installation_busy"),
            ("health server executable mismatch", "client_unhealthy"),
            (
                "rescue bundle identity mismatch",
                "existing_bundle_preserved",
            ),
            (
                "first installation rotation identity mismatch",
                "existing_bundle_preserved",
            ),
            ("something unexpected", "preparation_refused"),
        ] {
            assert_eq!(refusal_reason(error), reason, "{error}");
        }
    }
}

#[cfg(test)]
mod signed_source_tests {
    use super::*;

    fn target(sha256: &str) -> hbb_common::UpdateTarget {
        hbb_common::UpdateTarget {
            sha256: sha256.into(),
            ..Default::default()
        }
    }

    fn root(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "rd-signed-source-{name}-{}",
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(root.join("release-cache")).unwrap();
        root
    }

    // 下载副本按包哈希分目录，重试复用同一份，不同版本不会互相覆盖。
    #[test]
    fn the_signed_copy_is_keyed_by_the_package_hash() {
        let agent = Path::new(r"C:\agent");
        let dir = signed_source_dir(agent, &target(&"AB".repeat(32))).unwrap();
        assert_eq!(
            dir,
            agent.join("release-cache").join("rescue-source-abababababababab")
        );
        for invalid in ["", "abc", "zzzzzzzzzzzzzzzz0000"] {
            assert_eq!(
                signed_source_dir(agent, &target(invalid)).unwrap_err(),
                "first installation package hash invalid"
            );
        }
    }

    // R4 从救援包里装过之后，登记源就在救援目录里；轮换不能从要被移走的目录复制。
    #[test]
    fn a_registered_source_inside_the_rescue_tree_is_not_used() {
        let rescue = Path::new(r"C:\ProgramData\RustDesk\ManagementAgent\rescue");
        assert!(inside(Path::new(r"C:\ProgramData\RustDesk\ManagementAgent\rescue\current\a.msi"), rescue));
        assert!(inside(Path::new(r"c:/programdata/rustdesk/managementagent/RESCUE/previous/a.msi"), rescue));
        assert!(!inside(Path::new(r"C:\ProgramData\RustDesk\ManagementAgent\rescue-other\a.msi"), rescue));
        assert!(!inside(Path::new(r"C:\Windows\Temp\rd-update\a.msi"), rescue));
    }

    // 只清自己的下载副本：在用的那份和审批文件都要留下。
    #[test]
    fn only_superseded_signed_copies_are_discarded() {
        let root = root("discard");
        let cache = root.join("release-cache");
        for name in ["rescue-source-old", "rescue-source-current", "unrelated"] {
            std::fs::create_dir_all(cache.join(name)).unwrap();
        }
        std::fs::write(cache.join("first-install.json"), b"{}").unwrap();
        discard_signed_sources(&root, Some(&cache.join("rescue-source-current")));
        assert!(!cache.join("rescue-source-old").exists());
        assert!(cache.join("rescue-source-current").exists());
        assert!(cache.join("unrelated").exists());
        assert!(cache.join("first-install.json").exists());
        discard_signed_sources(&root, None);
        assert!(!cache.join("rescue-source-current").exists());
        std::fs::remove_dir_all(&root).unwrap();
    }
}
