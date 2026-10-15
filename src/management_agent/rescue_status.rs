use super::{rescue_assets, state_store, workspace};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    io::Read,
    path::{Path, PathBuf},
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
    if record.is_some_and(|r| r.assets["source_kind"] == "installed_msi") {
        components.insert("local_repair".into(),serde_json::json!({"supported":false,"state":"unsupported_source","ready":false,"reason":"installed_msi_local_repair_not_supported"}));
    }
    serde_json::json!({"schema":1,"state":state,"ready":false,"verification_supported":true,"full_recovery_supported":false,
        "freshness":fresh,"error_code":error,"verification_interval_seconds":MAX_AGE,"evidence":"cached_full_verification","current_integrity":"not_rechecked_by_status",
        "checked_at":record.map(|r|r.checked_at),"source_reference":record.and_then(|r|r.source_reference.as_deref()),"source_digest":record.map(|r|r.source_digest.as_str()),
        "components":components,"blocked_reason":"full_recovery_not_confirmed"})
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
    if result["error_code"] == "bundle_missing" {
        if let Ok(bytes) = read(&agent.join("first-install-status/last-check.json"), 32768) {
            if let Ok(report) = serde_json::from_slice::<serde_json::Value>(&bytes) {
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
    result
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
            let agent = PathBuf::from(r"C:\ProgramData\RustDesk\ManagementAgent");
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
