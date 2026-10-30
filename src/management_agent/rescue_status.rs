use super::{rescue_assets, state_store, workspace};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    io::Read,
    path::Path,
    sync::mpsc,
};

pub const MAX_AGE: u64 = 86400;
pub fn freshness(checked: u64, now: u64) -> &'static str {
    if checked == 0 {
        "unknown"
    } else if now < checked {
        "clock_rollback"
    } else if now - checked >= MAX_AGE {
        "stale"
    } else {
        "fresh"
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    schema: u32,
    installation: String,
    target_key: String,
    checked_at: u64,
    source_digest: String,
    source_reference: Option<String>,
    authority: String,
    state: String,
    error_code: Option<String>,
    assets: serde_json::Value,
}

fn read(path: &Path, limit: u64) -> Result<Vec<u8>, String> {
    use std::os::windows::fs::OpenOptionsExt;
    workspace::verify_file(path)?;
    let mut bytes = Vec::new();
    std::fs::OpenOptions::new()
        .read(true)
        .share_mode(1)
        .custom_flags(0x00200000)
        .open(path)
        .map_err(|_| "report unreadable")?
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "report read failed")?;
    if bytes.len() as u64 > limit {
        return Err("report too large".into());
    }
    Ok(bytes)
}

fn source(bundle: &Path) -> Result<(String, Option<String>), String> {
    match std::fs::symlink_metadata(bundle) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err("bundle_missing".into()),
        Err(_) => return Err("source_unreadable".into()),
        Ok(_) => {}
    }
    workspace::verify(bundle).map_err(|_| "source_untrusted")?;
    for name in ["bundle.json", "approved-manifest.json"] {
        match std::fs::symlink_metadata(bundle.join(name)) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err("bundle_incomplete".into())
            }
            Err(_) => return Err("source_unreadable".into()),
            Ok(_) => {}
        }
    }
    let bundle_bytes = read(&bundle.join("bundle.json"), 65536)?;
    let manifest = read(&bundle.join("approved-manifest.json"), 262144)?;
    let mut hash = Sha256::new();
    hash.update((bundle_bytes.len() as u64).to_le_bytes());
    hash.update(&bundle_bytes);
    hash.update(&manifest);
    let reference = serde_json::from_slice::<serde_json::Value>(&bundle_bytes)
        .ok()
        .and_then(|value| value["source_reference"].as_str().map(str::to_owned));
    Ok((hex::encode(hash.finalize()), reference))
}
fn authority(public: Option<&[u8]>) -> String {
    #[cfg(test)]
    if let Some(public) = public {
        return format!("fixture:{}", hex::encode(Sha256::digest(public)));
    }
    let _ = public;
    "yan-release-2026".into()
}
fn load(reports: &Path) -> Result<Record, String> {
    workspace::verify(reports)?;
    let record: Record = serde_json::from_slice(&read(&reports.join("last-check.json"), 32768)?)
        .map_err(|_| "report malformed")?;
    if record.schema != 1
        || !matches!(
            record.state.as_str(),
            "degraded" | "unavailable" | "unknown"
        )
        || record.checked_at == 0
    {
        return Err("report contract invalid".into());
    }
    Ok(record)
}
fn projection(
    state: &str,
    fresh: &str,
    error: Option<&str>,
    record: Option<&Record>,
) -> serde_json::Value {
    let usable = fresh == "fresh" && state == "degraded";
    let mut components = serde_json::Map::new();
    for name in [
        "client",
        "offline_package",
        "credentials",
        "agent_backup",
        "bootstrap_backup",
    ] {
        components.insert(name.into(),serde_json::json!({"supported":true,"state":if usable{"verified"}else{"unknown"},"ready":false,"evidence":"last_full_verification"}));
    }
    for name in ["service_recovery", "online_source", "recovery_entry"] {
        components.insert(
            name.into(),
            serde_json::json!({"supported":null,"state":"not_assessed","ready":false}),
        );
    }
    if let Some(component) = local_repair_component(record) {
        components.insert("local_repair".into(), component);
    }
    serde_json::json!({"schema":1,"state":state,"ready":false,"verification_supported":true,"full_recovery_supported":false,
        "freshness":fresh,"error_code":error,"verification_interval_seconds":MAX_AGE,"evidence":"cached_full_verification","current_integrity":"not_rechecked_by_status",
        "checked_at":record.map(|r|r.checked_at),"source_reference":record.and_then(|r|r.source_reference.as_deref()),"source_digest":record.map(|r|r.source_digest.as_str()),
        "components":components,"blocked_reason":"full_recovery_not_confirmed"})
}
// The automatic local-repair engine (local_repair_schedule::approval) accepts
// installed_msi sources, so status must not advertise installed_msi local repair
// as unsupported. Status alone cannot confirm the full approval chain
// (installed_source_id, package hash, runtime gates), so it reports not_assessed
// rather than asserting support either way, matching the sibling components above.
// committed_transaction sources carry no local_repair projection, as before.
fn local_repair_component(record: Option<&Record>) -> Option<serde_json::Value> {
    record
        .filter(|r| r.assets["source_kind"] == "installed_msi")
        .map(|_| serde_json::json!({"supported":null,"state":"not_assessed","ready":false}))
}
fn snapshot_at(
    bundle: &Path,
    reports: &Path,
    installation: &str,
    key: &str,
    time: u64,
    expected_authority: Option<&str>,
) -> serde_json::Value {
    let (digest, _) = match source(bundle) {
        Ok(v) => v,
        Err(error) => {
            return projection(
                if matches!(error.as_str(), "bundle_missing" | "bundle_incomplete") {
                    "unavailable"
                } else {
                    "unknown"
                },
                "unverified",
                Some(&error),
                None,
            )
        }
    };
    let record = match load(reports) {
        Ok(record) => record,
        Err(_) => {
            return projection(
                "unknown",
                "unverified",
                Some("verification_unavailable"),
                None,
            )
        }
    };
    if record.installation != installation
        || record.target_key != key
        || expected_authority.is_some_and(|a| a != record.authority)
    {
        return projection(
            "unknown",
            "unverified",
            Some("report_binding_mismatch"),
            None,
        );
    }
    if time < record.checked_at {
        return projection(
            "unknown",
            "clock_rollback",
            Some("clock_rollback"),
            Some(&record),
        );
    }
    if record.source_digest != digest {
        return projection(
            "unknown",
            "source_changed",
            Some("source_changed"),
            Some(&record),
        );
    }
    let fresh = freshness(record.checked_at, time);
    projection(
        if fresh == "fresh" {
            &record.state
        } else {
            "unknown"
        },
        fresh,
        record.error_code.as_deref(),
        Some(&record),
    )
}
fn classify(error: &str) -> (&'static str, &'static str) {
    if error.contains("signature") || error.contains("signing key") {
        ("unavailable", "signature_invalid")
    } else if error.contains("deadline") || error.contains("budget") {
        ("unknown", "verification_budget_exceeded")
    } else if error.contains("mismatch")
        || error.contains("malformed")
        || error.contains("incomplete")
    {
        ("unavailable", "asset_invalid")
    } else {
        ("unknown", "verification_unconfirmed")
    }
}
fn refresh_at(
    bundle: &Path,
    reports: &Path,
    installation: &str,
    key: &str,
    time: u64,
    public: Option<&[u8]>,
) -> Result<bool, String> {
    if time == 0 {
        return Ok(false);
    }
    let observed = source(bundle);
    let auth = authority(public);
    let digest = observed
        .as_ref()
        .map(|v| v.0.clone())
        .unwrap_or_else(|error| format!("error:{error}"));
    if let Ok(previous) = load(reports) {
        if time < previous.checked_at {
            return Ok(false);
        }
        if previous.installation == installation
            && previous.target_key == key
            && previous.authority == auth
            && previous.source_digest == digest
            && freshness(previous.checked_at, time) == "fresh"
        {
            return Ok(false);
        }
    }
    let reference = observed.as_ref().ok().and_then(|v| v.1.clone());
    let result = match observed {
        Ok(_) => rescue_assets::verify_readonly_status(bundle, installation, key, public),
        Err(error) => Err(error),
    };
    let (state, error, assets) = match result {
        Ok(assets) => ("degraded".to_owned(), None, assets),
        Err(error) => {
            let (state, code) = if matches!(error.as_str(), "bundle_missing" | "bundle_incomplete")
            {
                ("unavailable", error.as_str())
            } else {
                classify(&error)
            };
            (state.into(), Some(code.to_owned()), serde_json::Value::Null)
        }
    };
    let after = source(bundle)
        .as_ref()
        .map(|v| v.0.clone())
        .unwrap_or_else(|error| format!("error:{error}"));
    if after != digest {
        return Err("rescue source changed during verification; previous evidence retained".into());
    }
    if !reports.exists() {
        workspace::create(reports)?;
    }
    workspace::verify(reports)?;
    let record = Record {
        schema: 1,
        installation: installation.into(),
        target_key: key.into(),
        checked_at: time,
        source_digest: digest,
        source_reference: reference,
        authority: auth,
        state,
        error_code: error,
        assets,
    };
    let bytes = serde_json::to_vec(&record).map_err(|_| "rescue report encoding")?;
    if record.state == "degraded" {
        state_store::publish(&reports.join("verified.json"), &bytes).map_err(|e| e.to_string())?;
    }
    state_store::publish(&reports.join("last-check.json"), &bytes).map_err(|e| e.to_string())?;
    Ok(true)
}

pub fn snapshot(agent: &Path, installation: &str, key: &str) -> serde_json::Value {
    let time = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|t| t.as_secs())
        .unwrap_or(0);
    let mut result = snapshot_at(
        &agent.join("rescue/current"),
        &agent.join("rescue-status"),
        installation,
        key,
        time,
        Some("yan-release-2026"),
    );
    mark_build_mismatch(
        &mut result,
        bundle_build(&agent.join("rescue/current")),
        agent_build(),
    );
    let first_install = read(&agent.join("first-install-status/last-check.json"), 32768).ok();
    if result["error_code"] == "bundle_missing" {
        if let Some(bytes) = &first_install {
            if let Ok(report) = serde_json::from_slice::<serde_json::Value>(bytes) {
                if report["error_code"] == "waiting_approval" {
                    result["error_code"] = "waiting_approval".into();
                }
                if report["blocked_reason"] == "existing_fallback_incompatible" {
                    result["state"] = "degraded".into();
                    result["error_code"] = "existing_fallback_incompatible".into();
                    result["blocked_reason"] = "existing_fallback_incompatible".into();
                    result["legacy_route_preserved"] = true.into();
                }
            }
        }
    }
    attach_rotation_reason(&mut result, first_install.as_deref(), agent_build());
    result
}

/// When the bundle is missing or older than the agent, forwards why the first-install
/// worker did not (re)build it during the running build. Only short snake_case codes leave the machine; the
/// report's raw error text can name local paths and stays local.
fn attach_rotation_reason(
    result: &mut serde_json::Value,
    report: Option<&[u8]>,
    build: Option<u64>,
) {
    if !matches!(
        result["error_code"].as_str(),
        Some("bundle_build_mismatch" | "bundle_missing" | "waiting_approval")
    ) {
        return;
    }
    let Some(report) =
        report.and_then(|bytes| serde_json::from_slice::<serde_json::Value>(bytes).ok())
    else {
        return;
    };
    // A report left from before the last upgrade describes another build.
    if build.is_none() || report["agent_build"].as_u64() != build {
        return;
    }
    let mut rotation = serde_json::Map::new();
    for field in ["error_code", "reason", "coordination_error"] {
        if let Some(value) = report_code(&report[field]) {
            rotation.insert(field.into(), value.into());
        }
    }
    if !rotation.is_empty() {
        result["rotation"] = rotation.into();
    }
}

fn report_code(value: &serde_json::Value) -> Option<&str> {
    value.as_str().filter(|text| {
        !text.is_empty()
            && text.len() <= 64
            && text
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte == b'_')
    })
}

fn agent_build() -> Option<u64> {
    serde_json::from_str::<serde_json::Value>(include_str!("../../version.json")).ok()?["build_seq"]
        .as_u64()
}
fn bundle_build(bundle: &Path) -> Option<u64> {
    serde_json::from_slice::<serde_json::Value>(&read(&bundle.join("bundle.json"), 65536).ok()?)
        .ok()?["build_seq"]
        .as_u64()
}
/// A client updater upgrade replaces the installed MSI but not the rescue bundle, which
/// still verifies on its own; restoring it would bring back the old build (and its old
/// MSI product code), so it must not be reported as usable.
fn mark_build_mismatch(result: &mut serde_json::Value, bundle: Option<u64>, agent: Option<u64>) {
    if let (Some(bundle), Some(agent)) = (bundle, agent) {
        if bundle != agent && result["error_code"].is_null() {
            result["state"] = "unavailable".into();
            result["error_code"] = "bundle_build_mismatch".into();
        }
    }
}

#[cfg(test)]
pub fn refresh_fixture(
    bundle: &Path,
    reports: &Path,
    installation: &str,
    key: &str,
    time: u64,
    public: Option<&[u8]>,
) -> Result<bool, String> {
    refresh_at(bundle, reports, installation, key, time, public)
}
#[cfg(test)]
pub fn snapshot_fixture(
    bundle: &Path,
    reports: &Path,
    installation: &str,
    key: &str,
    time: u64,
) -> serde_json::Value {
    snapshot_at(bundle, reports, installation, key, time, None)
}

#[derive(Clone)]
pub struct Tick {
    pub installation: String,
    pub target_key: String,
}
pub struct Worker {
    sender: mpsc::SyncSender<Tick>,
}
impl Worker {
    pub fn notify(&self, tick: Tick) -> Result<(), String> {
        match self.sender.try_send(tick) {
            Ok(()) | Err(mpsc::TrySendError::Full(_)) => Ok(()),
            Err(_) => Err("rescue verification worker unavailable".into()),
        }
    }
}
pub fn start() -> std::io::Result<Worker> {
    start_checked(None)
}
#[cfg(test)]
pub fn start_fixture(public: &[u8]) -> std::io::Result<Worker> {
    start_checked(Some(public.to_vec()))
}
fn start_checked(public: Option<Vec<u8>>) -> std::io::Result<Worker> {
    let (sender, receiver) = mpsc::sync_channel::<Tick>(1);
    std::thread::Builder::new()
        .name("agent-rescue-readonly".into())
        .spawn(move || {
            #[link(name = "kernel32")]
            extern "system" {
                fn GetCurrentThread() -> *mut std::ffi::c_void;
                fn SetThreadPriority(thread: *mut std::ffi::c_void, priority: i32) -> i32;
            }
            if unsafe { SetThreadPriority(GetCurrentThread(), -2) } == 0 {
                hbb_common::log::warn!("rescue verifier low priority setup failed");
                return;
            }
            let agent = super::system_dirs::get().agent();
            for tick in receiver {
                let time = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|t| t.as_secs())
                    .unwrap_or(0);
                match refresh_at(
                    &agent.join("rescue/current"),
                    &agent.join("rescue-status"),
                    &tick.installation,
                    &tick.target_key,
                    time,
                    public.as_deref(),
                ) {
                    Ok(_) => {}
                    Err(error) => hbb_common::throttled_log!(
                        std::time::Duration::from_secs(300),
                        warn,
                        "rescue verification report failed: {error}"
                    ),
                }
            }
        })?;
    Ok(Worker { sender })
}

#[cfg(test)]
mod projection_tests {
    use super::*;

    fn record_with(assets: serde_json::Value) -> Record {
        Record {
            schema: 1,
            installation: "c".repeat(32),
            target_key: "windows-x86_64-msi-standard".into(),
            checked_at: 1000,
            source_digest: "d".repeat(64),
            source_reference: Some("ref".into()),
            authority: "yan-release-2026".into(),
            state: "degraded".into(),
            error_code: None,
            assets,
        }
    }

    #[test]
    fn installed_msi_local_repair_is_not_assessed_not_unsupported() {
        let record = record_with(serde_json::json!({"source_kind": "installed_msi"}));
        let component = local_repair_component(Some(&record)).expect("installed_msi has a local_repair projection");
        assert_eq!(component["supported"], serde_json::Value::Null);
        assert_eq!(component["state"], "not_assessed");
        assert_eq!(component["ready"], false);
        assert!(component.get("reason").is_none(), "must not advertise installed_msi_local_repair_not_supported");

        let projected = projection("degraded", "fresh", None, Some(&record));
        assert_eq!(projected["components"]["local_repair"]["state"], "not_assessed");
        assert_ne!(projected["components"]["local_repair"]["state"], "unsupported_source");
        // Contract fields the status layer must still hold constant.
        assert_eq!(projected["ready"], false);
        assert_eq!(projected["full_recovery_supported"], false);
    }

    #[test]
    fn committed_transaction_source_has_no_local_repair_component() {
        let record = record_with(serde_json::json!({"source_kind": "committed_transaction"}));
        assert!(local_repair_component(Some(&record)).is_none());
        let projected = projection("degraded", "fresh", None, Some(&record));
        assert!(projected["components"].get("local_repair").is_none());
    }

    #[test]
    fn absent_record_has_no_local_repair_component() {
        assert!(local_repair_component(None).is_none());
        let projected = projection("unavailable", "unverified", Some("bundle_missing"), None);
        assert!(projected["components"].get("local_repair").is_none());
    }
}

#[cfg(test)]
mod build_mismatch_tests {
    use super::*;

    fn clean() -> serde_json::Value {
        serde_json::json!({"state":"degraded","freshness":"fresh","error_code":null})
    }

    // 客户端更新器升级 MSI 后救援包仍是旧构建，状态却报 fresh 且无错误，
    // 后台显示「可自行恢复」，实际 R3 只能恢复出旧版、R4 直接拒绝。
    #[test]
    fn an_older_bundle_is_reported_as_a_build_mismatch() {
        let mut result = clean();
        mark_build_mismatch(&mut result, Some(2026102503), Some(2026102601));
        assert_eq!(result["state"], "unavailable");
        assert_eq!(result["error_code"], "bundle_build_mismatch");
    }

    #[test]
    fn a_matching_bundle_is_left_alone() {
        let mut result = clean();
        mark_build_mismatch(&mut result, Some(2026102601), Some(2026102601));
        assert_eq!(result, clean());
    }

    // 原结论已有错误时保留原错误，不被覆盖。
    #[test]
    fn an_existing_error_is_not_overwritten() {
        let mut result = serde_json::json!({"state":"unavailable","error_code":"bundle_missing"});
        mark_build_mismatch(&mut result, Some(1), Some(2));
        assert_eq!(result["error_code"], "bundle_missing");
    }

    // 读不到构建号时不臆断。
    #[test]
    fn an_unknown_build_is_not_reported() {
        for (bundle, agent) in [(None, Some(2)), (Some(1), None), (None, None)] {
            let mut result = clean();
            mark_build_mismatch(&mut result, bundle, agent);
            assert_eq!(result, clean());
        }
    }

    #[test]
    fn the_agent_build_comes_from_version_json() {
        let expected: serde_json::Value =
            serde_json::from_str(include_str!("../../version.json")).unwrap();
        assert_eq!(agent_build(), expected["build_seq"].as_u64());
    }
}

#[cfg(test)]
mod rotation_reason_tests {
    use super::*;

    fn mismatch() -> serde_json::Value {
        serde_json::json!({"state":"unavailable","error_code":"bundle_build_mismatch"})
    }

    // 救援包旧时，把首装线程最近一次没能换新的原因一起上报，后台才能给出处理办法。
    #[test]
    fn an_outdated_bundle_reports_why_it_was_not_replaced() {
        let mut result = mismatch();
        attach_rotation_reason(
            &mut result,
            Some(br#"{"state":"unavailable","error_code":"waiting_approval","coordination_error":"msi_location_mismatch","ready":false,"agent_build":7}"#),
            Some(7),
        );
        assert_eq!(
            result["rotation"],
            serde_json::json!({"error_code":"waiting_approval","coordination_error":"msi_location_mismatch"})
        );
        assert_eq!(result["error_code"], "bundle_build_mismatch");
    }

    #[test]
    fn a_refused_preparation_reports_its_reason_but_never_the_raw_error() {
        let mut result = mismatch();
        attach_rotation_reason(
            &mut result,
            Some(br#"{"state":"unavailable","error_code":"preparation_refused","reason":"sessions_active","error":"C:\\secret path","ready":false,"agent_build":7}"#),
            Some(7),
        );
        assert_eq!(
            result["rotation"],
            serde_json::json!({"error_code":"preparation_refused","reason":"sessions_active"})
        );
    }

    #[test]
    fn a_missing_or_unreadable_report_adds_nothing() {
        for report in [None, Some(&b"not json"[..]), Some(&b"[]"[..])] {
            let mut result = mismatch();
            attach_rotation_reason(&mut result, report, Some(7));
            assert_eq!(result, mismatch());
        }
    }

    // 字段只接受短的 snake_case 码，任何别的内容都丢掉。
    #[test]
    fn only_short_codes_are_forwarded() {
        let mut result = mismatch();
        attach_rotation_reason(
            &mut result,
            Some(br#"{"error_code":"Bad Code","reason":"x\\y","coordination_error":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","agent_build":7}"#),
            Some(7),
        );
        assert_eq!(result, mismatch());
    }

    // 救援包正常时不附带任何轮换原因。
    #[test]
    fn a_healthy_bundle_carries_no_rotation_reason() {
        let mut result = serde_json::json!({"state":"degraded","error_code":null});
        attach_rotation_reason(
            &mut result,
            Some(
                br#"{"error_code":"waiting_approval","coordination_error":"busy","agent_build":7}"#,
            ),
            Some(7),
        );
        assert!(result.get("rotation").is_none());
    }

    #[test]
    fn a_missing_bundle_also_reports_the_reason() {
        for code in ["bundle_missing", "waiting_approval"] {
            let mut result = serde_json::json!({"state":"unavailable","error_code":code});
            attach_rotation_reason(
                &mut result,
                Some(br#"{"error_code":"waiting_approval","coordination_error":"msi_product_absent","agent_build":7}"#),
            Some(7),
            );
            assert_eq!(
                result["rotation"]["coordination_error"], "msi_product_absent",
                "{code}"
            );
        }
    }

    // 上一次升级前留下的报告描述的是旧构建，不能当成现在没换新的原因上报。
    #[test]
    fn a_report_from_another_build_is_not_forwarded() {
        for (report, build) in [
            (
                &br#"{"error_code":"waiting_approval","agent_build":6}"#[..],
                Some(7),
            ),
            (&br#"{"error_code":"waiting_approval"}"#[..], Some(7)),
            (
                &br#"{"error_code":"waiting_approval","agent_build":7}"#[..],
                None,
            ),
        ] {
            let mut result = mismatch();
            attach_rotation_reason(&mut result, Some(report), build);
            assert_eq!(result, mismatch());
        }
    }
}
