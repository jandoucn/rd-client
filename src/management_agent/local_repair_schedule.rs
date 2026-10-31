use super::{
    local_repair_plan::LocalRepairPlan,
    transaction::{Phase, Transaction},
    workspace,
};
use sha2::{Digest, Sha256};
use std::{io::Read, path::Path, sync::mpsc, time::Duration};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Decision {
    Observe,
    Deferred,
    Existing(String),
    Prepare,
}

pub fn decide(
    normal_missing: bool,
    allowed: bool,
    due: bool,
    fallback_idle: bool,
    existing: Option<String>,
) -> Decision {
    if !normal_missing {
        return Decision::Observe;
    }
    if !allowed || !due || !fallback_idle {
        return Decision::Deferred;
    }
    if let Some(id) = existing {
        return Decision::Existing(id);
    }
    Decision::Prepare
}

pub fn existing(
    root: &Path,
    installation: &str,
    source: &str,
    key: &str,
    hash: &str,
) -> Result<Option<String>, String> {
    existing_bound(root, installation, source, key, hash, None)
}

#[cfg(test)]
pub fn existing_approval_fixture(
    root: &Path,
    installation: &str,
    source: &str,
    key: &str,
    hash: &str,
    reference: &str,
) -> Result<Option<String>, String> {
    existing_bound(root, installation, source, key, hash, Some(reference))
}

pub const STARTUP_GRACE: Duration = Duration::from_secs(180);

fn read(path: &Path, limit: u64) -> Result<Vec<u8>, String> {
    workspace::verify_file(path)?;
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .map_err(|e| e.to_string())?
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > limit {
        return Err("automatic local repair record too large".into());
    }
    Ok(bytes)
}

fn existing_bound(
    root: &Path,
    installation: &str,
    source: &str,
    key: &str,
    hash: &str,
    reference: Option<&str>,
) -> Result<Option<String>, String> {
    super::local_repair::capacity_admission(root, None, false)?;
    let mut found = None;
    for (index, entry) in std::fs::read_dir(root)
        .map_err(|e| e.to_string())?
        .enumerate()
    {
        if index >= 64 {
            return Err("automatic local repair transaction scan capacity exceeded".into());
        }
        let directory = entry.map_err(|e| e.to_string())?.path();
        workspace::verify(&directory)?;
        let tx = Transaction::decode(&read(&directory.join("transaction.json"), 65536)?)?;
        if directory.file_name().and_then(|n| n.to_str()) != Some(&tx.id) {
            return Err("automatic local repair directory binding mismatch".into());
        }
        if !matches!(tx.phase, Phase::Committed | Phase::RolledBack) {
            if tx.phase == Phase::RollingBack {
                if let Ok(plan) =
                    super::local_repair_plan::total_loss::completed_preservation(&directory)
                {
                    if plan.installation_id == installation
                        && plan.target_key == key
                        && plan.approval_id() != source
                    {
                        continue;
                    }
                }
            }
            return Ok(Some(tx.id));
        }
        match std::fs::symlink_metadata(directory.join("local-repair-plan.json")) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(_) => return Err("automatic local repair plan observation failed".into()),
            Ok(_) => {}
        }
        if let Some(reference) = reference {
            let manifest: serde_json::Value =
                serde_json::from_slice(&read(&directory.join("approved-manifest.json"), 262144)?)
                    .map_err(|_| "automatic local repair previous approval malformed")?;
            if super::package_gate::package_reference(&manifest, key)? != reference {
                continue;
            }
        }
        let plan =
            LocalRepairPlan::decode(&read(&directory.join("local-repair-plan.json"), 65536)?)?;
        plan.bind(&tx)?;
        if plan.installation_id == installation
            && plan.approval_id() == source
            && plan.target_key == key
            && plan.package.sha256 == hash
        {
            found = Some(tx.id);
        }
    }
    if found.is_none() {
        super::local_repair::capacity_admission(root, None, true)?;
    }
    Ok(found)
}

fn now_secs() -> Result<u64, String> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|v| v.as_secs())
        .map_err(|_| "automatic local repair clock invalid".into())
}

#[derive(Clone)]
pub struct Tick {
    pub elapsed: Duration,
    pub enabled: bool,
    pub retry_at: u64,
}

// Same criterion as R3: a missing RustDesk.exe counts even when the directory remains, because
// antivirus usually quarantines only the executable. Disagreeing here left devices on the
// fallback forever: R3 switched, this worker kept observing.
fn normal_missing() -> Result<bool, String> {
    super::rescue_assets::r3_normal_missing(
        &crate::management_agent::system_dirs::get().normal_install(),
    )
    .map_err(|error| format!("automatic local repair normal state: {error}"))
}

/// Windows Installer holds `Global\_MSIExecute` while it runs an installation; a normal directory
/// without RustDesk.exe is then an installation in progress, not damage.
fn windows_installer_busy() -> bool {
    #[link(name = "kernel32")]
    extern "system" {
        fn OpenMutexW(access: u32, inherit: i32, name: *const u16) -> *mut std::ffi::c_void;
        fn CloseHandle(handle: *mut std::ffi::c_void) -> i32;
    }
    let name = "Global\\_MSIExecute"
        .encode_utf16()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let handle = unsafe { OpenMutexW(0x0010_0000, 0, name.as_ptr()) };
    if handle.is_null() {
        // Present but not accessible still means an installation is running.
        return std::io::Error::last_os_error().raw_os_error() == Some(5);
    }
    unsafe { CloseHandle(handle) };
    true
}

/// The repair recreates the normal directory and treats an existing one as foreign evidence. When
/// only part of it is left (RustDesk.exe quarantined by antivirus), move the remainder into the
/// agent's own quarantine on the same volume first. Runs under the install guard and only while no
/// Windows Installer transaction is active, so an upgrade that briefly removes RustDesk.exe is never
/// mistaken for damage. Only the newest remainder is kept.
fn quarantine_damaged_normal(now: u64) -> Result<(), String> {
    use std::os::windows::fs::MetadataExt;
    let dirs = crate::management_agent::system_dirs::get();
    let normal = dirs.normal_install();
    match std::fs::symlink_metadata(&normal) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(_) => return Err("automatic local repair normal state unknown".into()),
        Ok(metadata) => {
            if metadata.file_attributes() & 0x400 != 0 || !metadata.is_dir() {
                return Err("automatic local repair normal remainder refused".into());
            }
        }
    }
    let Some(_guard) = super::install_guard::InstallGuard::try_acquire()
        .map_err(|_| "automatic local repair install guard unavailable")?
    else {
        return Err("automatic local repair installation busy".into());
    };
    if windows_installer_busy() {
        return Err("automatic local repair Windows Installer busy".into());
    }
    if !normal_missing()? {
        return Err("automatic local repair normal install still complete".into());
    }
    let root = dirs.agent().join("quarantine");
    match std::fs::symlink_metadata(&root) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => workspace::create(&root)?,
        Err(_) => return Err("automatic local repair quarantine unavailable".into()),
        Ok(_) => workspace::verify(&root)?,
    }
    let name = format!("normal-{now}");
    std::fs::rename(&normal, root.join(&name))
        .map_err(|e| format!("automatic local repair damaged normal move failed: {e}"))?;
    // Older remainders go only after the newest one is safely moved aside.
    for entry in std::fs::read_dir(&root).map_err(|e| e.to_string())? {
        let path = entry.map_err(|e| e.to_string())?.path();
        let file_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        if file_name.starts_with("normal-") && file_name != name {
            std::fs::remove_dir_all(&path)
                .map_err(|e| format!("automatic local repair old remainder cleanup failed: {e}"))?;
        }
    }
    Ok(())
}

#[derive(serde::Deserialize)]
struct Approval {
    schema: u32,
    installation_id: String,
    #[serde(default)]
    source_transaction: String,
    #[serde(default)]
    source_kind: Option<String>,
    #[serde(default)]
    installation_receipt: Option<super::install_plan::Artifact>,
    #[serde(skip)]
    installed_source_id: Option<String>,
    source_reference: String,
    target_key: String,
    build_seq: u64,
    package: super::install_plan::Artifact,
}

fn run_total_loss(
    a: &Approval,
    transactions: &Path,
    public: Option<&[u8]>,
) -> Result<Decision, String> {
    let id = match choose_attempt(a, transactions)? {
        NextAttempt::New(id) => id,
        NextAttempt::Blocked(id) => return Ok(Decision::Existing(id)),
        NextAttempt::Exhausted => {
            return Err("automatic local repair attempts exhausted for this rescue bundle".into())
        }
    };
    quarantine_damaged_normal(now_secs()?)?;
    let directory = super::local_repair::prepare_total_loss(&id, public)?;
    let prepared =
        LocalRepairPlan::decode(&read(&directory.join("local-repair-plan.json"), 65536)?)?;
    let manifest =
        serde_json::from_slice(&read(&directory.join("approved-manifest.json"), 262144)?)
            .map_err(|_| "total loss prepared manifest malformed")?;
    if prepared.installation_id != a.installation_id
        || prepared.approval_id()
            != a.installed_source_id
                .as_deref()
                .unwrap_or(&a.source_transaction)
        || prepared.target_key != a.target_key
        || prepared.package.sha256 != a.package.sha256
        || super::package_gate::package_reference(&manifest, &a.target_key)? != a.source_reference
    {
        return Err("total loss scheduled approval changed".into());
    }
    let mut child = match public {
        #[cfg(test)]
        Some(public) => super::local_repair::launch_fixture(&directory, public)?,
        _ => super::local_repair::launch(&directory)?,
    };
    let deadline = std::time::Instant::now() + Duration::from_secs(900);
    while child.try_wait().map_err(|e| e.to_string())?.is_none() {
        if std::time::Instant::now() >= deadline {
            return Err("total loss helper pending".into());
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    let tx = Transaction::decode(&read(&directory.join("transaction.json"), 65536)?)?;
    if tx.phase != Phase::Committed {
        return Err(format!(
            "total loss reconciliation required: {:?}",
            tx.phase
        ));
    }
    Ok(Decision::Existing(id))
}
fn approval() -> Result<Approval, String> {
    let mut value: Approval = serde_json::from_slice(&read(
        &crate::management_agent::system_dirs::get()
            .agent()
            .join(r"rescue\current\bundle.json"),
        65536,
    )?)
    .map_err(|_| "automatic local repair approval malformed")?;
    let token = |s: &str, len| {
        s.len() == len
            && s.bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    };
    let source_valid = if value.schema == 2
        && value.source_kind.as_deref() == Some("installed_msi")
        && value.source_transaction.is_empty()
    {
        let artifact = value
            .installation_receipt
            .as_ref()
            .ok_or("automatic local repair receipt absent")?;
        if artifact.relative_path != "installation-receipt.json" || artifact.size > 65536 {
            return Err("automatic local repair receipt reference invalid".into());
        }
        let root = &crate::management_agent::system_dirs::get()
            .agent()
            .join(r"rescue\current");
        artifact.verify(root)?;
        let receipt: serde_json::Value =
            serde_json::from_slice(&read(&root.join(&artifact.relative_path), 65536)?)
                .map_err(|_| "automatic local repair receipt malformed")?;
        let id = receipt["source_id"]
            .as_str()
            .filter(|id| token(id, 64))
            .ok_or("automatic local repair installed source id invalid")?;
        if receipt["source_kind"] != "installed_msi"
            || receipt["installation_id"] != value.installation_id
            || receipt["source_reference"] != value.source_reference
            || receipt["target_key"] != value.target_key
        {
            return Err("automatic local repair receipt binding mismatch".into());
        }
        value.installed_source_id = Some(id.into());
        true
    } else {
        value.schema == 1
            && token(&value.source_transaction, 32)
            && value.source_kind.is_none()
            && value.installation_receipt.is_none()
    };
    if !source_valid
        || !token(&value.installation_id, 32)
        || !token(&value.source_reference, 64)
        || !token(&value.package.sha256, 64)
        || value.build_seq == 0
        || !matches!(
            value.target_key.as_str(),
            "windows-x86_64-msi-sos" | "windows-x86_64-msi-standard"
        )
    {
        return Err("automatic local repair approval identity invalid".into());
    }
    Ok(value)
}
fn attempt_id(a: &Approval) -> String {
    attempt_id_at(a, 0)
}

// Attempt 0 keeps the historical id so transactions written by earlier builds are recognised.
fn attempt_id_at(a: &Approval, attempt: u32) -> String {
    let suffix = if attempt == 0 {
        String::new()
    } else {
        format!("\n#{attempt}")
    };
    if let Some(source) = &a.installed_source_id {
        return hex::encode(Sha256::digest(format!(
            "local-repair-auto-installed-v1\n{}\n{}\n{}\n{}\n{}{suffix}",
            a.installation_id, source, a.source_reference, a.target_key, a.package.sha256
        )))[..32]
            .to_owned();
    }
    hex::encode(Sha256::digest(format!(
        "local-repair-auto-v1\n{}\n{}\n{}\n{}\n{}{suffix}",
        a.installation_id, a.source_transaction, a.source_reference, a.target_key, a.package.sha256
    )))[..32]
        .to_owned()
}

/// Repairs allowed per rescue bundle. Only a committed attempt lets a new one start when the normal
/// install goes missing again; an unfinished or rolled-back attempt stops automatic repair, because
/// repeating a deterministic failure risks a failed rollback that leaves maintenance.json behind.
pub const MAX_ATTEMPTS_PER_BUNDLE: u32 = 5;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttemptState {
    Absent,
    Committed,
    Blocking,
}

#[derive(Debug, PartialEq, Eq)]
pub enum NextAttempt {
    Blocked(String),
    New(String),
    Exhausted,
}

/// Pure choice of the next attempt id given each candidate's state.
pub fn next_attempt(
    ids: &[String],
    state: impl Fn(&str) -> Result<AttemptState, String>,
) -> Result<NextAttempt, String> {
    for id in ids {
        match state(id)? {
            AttemptState::Absent => return Ok(NextAttempt::New(id.clone())),
            AttemptState::Blocking => return Ok(NextAttempt::Blocked(id.clone())),
            AttemptState::Committed => {}
        }
    }
    Ok(NextAttempt::Exhausted)
}

fn attempt_state(directory: &Path) -> Result<AttemptState, String> {
    match std::fs::symlink_metadata(directory) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(AttemptState::Absent),
        Err(_) => return Err("automatic local repair attempt unavailable".into()),
        Ok(_) => {}
    }
    // An unreadable or undecodable transaction is an interrupted preparation: it blocks.
    let committed = read(&directory.join("transaction.json"), 65536)
        .and_then(|bytes| Transaction::decode(&bytes))
        .is_ok_and(|tx| tx.phase == Phase::Committed);
    Ok(if committed {
        AttemptState::Committed
    } else {
        AttemptState::Blocking
    })
}

fn choose_attempt(a: &Approval, transactions: &Path) -> Result<NextAttempt, String> {
    let ids = (0..MAX_ATTEMPTS_PER_BUNDLE)
        .map(|attempt| attempt_id_at(a, attempt))
        .collect::<Vec<_>>();
    next_attempt(&ids, |id| attempt_state(&transactions.join(id)))
}

#[cfg(test)]
pub fn installed_approval_fixture() -> Result<(String, String), String> {
    let value = approval()?;
    Ok((
        value
            .installed_source_id
            .clone()
            .unwrap_or_else(|| value.source_transaction.clone()),
        attempt_id(&value),
    ))
}

fn run_once(tick: &Tick, public: Option<&[u8]>) -> Result<Decision, String> {
    let missing = normal_missing()?;
    if !missing {
        return Ok(Decision::Observe);
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| "automatic local repair clock invalid")?
        .as_secs();
    if tick.elapsed < STARTUP_GRACE || !tick.enabled || now < tick.retry_at {
        return Ok(Decision::Deferred);
    }
    if !super::rescue_assets::r3_cycle_allowed()? {
        return Ok(Decision::Deferred);
    }
    let a = approval()?;
    let transactions = &crate::management_agent::system_dirs::get()
        .agent()
        .join("transactions");
    match std::fs::symlink_metadata(transactions) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            workspace::create(transactions)?;
        }
        Err(e) => return Err(e.to_string()),
        Ok(_) => {}
    }
    let prior = existing_bound(
        transactions,
        &a.installation_id,
        a.installed_source_id
            .as_deref()
            .unwrap_or(&a.source_transaction),
        &a.target_key,
        &a.package.sha256,
        Some(&a.source_reference),
    )?;
    if let Some(id) = prior {
        // Only a committed repair of this bundle lets the attempt numbering below continue; any
        // other transaction (unfinished, rolled back, or another flow's) still blocks.
        if attempt_state(&transactions.join(&id))? != AttemptState::Committed {
            return Ok(Decision::Existing(id));
        }
    }
    let fallback = crate::management_agent::system_dirs::get()
        .client_root()
        .join(if a.target_key.ends_with("-sos") {
            "sos"
        } else {
            "standard"
        })
        .join("RustDesk");
    if matches!(std::fs::symlink_metadata(&fallback), Err(ref e) if e.kind() == std::io::ErrorKind::NotFound)
    {
        return run_total_loss(&a, transactions, public);
    }
    let contract = super::service_control::Contract {
        name: "RustDesk".into(),
        executable: fallback.join("RustDesk.exe"),
        arguments: vec!["--service".into()],
        start_type: windows_service::service::ServiceStartType::AutoStart,
    };
    let service = super::service_control::inspect(&contract)?;
    if service.state != windows_service::service::ServiceState::Running {
        return Ok(Decision::Deferred);
    }
    let response = super::health_windows::probe_blocking(
        r"\\.\pipe\RustDesk\management-health-v2",
        &contract.executable,
        a.build_seq,
        Duration::from_secs(3),
    )?;
    let decision = decide(
        missing,
        true,
        true,
        response.no_active_sessions == Some(true),
        None,
    );
    if decision != Decision::Prepare {
        return Ok(decision);
    }
    // Reserve this approval through the transaction namespace. Unfinished directories prevent
    // retries across worker or agent restarts; finished ones move on to the next attempt number.
    let id = match choose_attempt(&a, transactions)? {
        NextAttempt::New(id) => id,
        NextAttempt::Blocked(id) => return Ok(Decision::Existing(id)),
        NextAttempt::Exhausted => {
            return Err("automatic local repair attempts exhausted for this rescue bundle".into())
        }
    };
    quarantine_damaged_normal(now)?;
    let directory = match public {
        #[cfg(test)]
        Some(public) => super::local_repair::prepare_fixture(&id, public)?,
        _ => super::local_repair::prepare(&id)?,
    };
    let prepared =
        LocalRepairPlan::decode(&read(&directory.join("local-repair-plan.json"), 65536)?)?;
    let prepared_manifest: serde_json::Value =
        serde_json::from_slice(&read(&directory.join("approved-manifest.json"), 262144)?)
            .map_err(|_| "automatic local repair prepared approval malformed")?;
    if prepared.installation_id != a.installation_id
        || prepared.approval_id()
            != a.installed_source_id
                .as_deref()
                .unwrap_or(&a.source_transaction)
        || prepared.target_key != a.target_key
        || prepared.package.sha256 != a.package.sha256
        || super::package_gate::package_reference(&prepared_manifest, &a.target_key)?
            != a.source_reference
    {
        return Err(
            "automatic local repair approval changed before launch; preparation retained".into(),
        );
    }
    let mut child = match public {
        #[cfg(test)]
        Some(public) => super::local_repair::launch_fixture(&directory, public)?,
        _ => super::local_repair::launch(&directory)?,
    };
    let deadline = std::time::Instant::now() + Duration::from_secs(900);
    while child.try_wait().map_err(|e| e.to_string())?.is_none() {
        if std::time::Instant::now() >= deadline {
            return Err("automatic local repair helper pending; transaction retained".into());
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    let tx = Transaction::decode(&read(&directory.join("transaction.json"), 65536)?)?;
    if tx.phase != Phase::Committed {
        return Err(format!(
            "automatic local repair terminal/reconciliation required: {:?}; id={id}",
            tx.phase
        ));
    }
    Ok(Decision::Existing(id))
}

pub struct Worker {
    tx: mpsc::SyncSender<Tick>,
    results: mpsc::Receiver<String>,
}
impl Worker {
    pub fn notify(&self, tick: Tick) -> Result<(), String> {
        match self.tx.try_send(tick) {
            Ok(()) | Err(mpsc::TrySendError::Full(_)) => Ok(()),
            Err(mpsc::TrySendError::Disconnected(_)) => {
                Err("automatic local repair worker disconnected".into())
            }
        }
    }
    pub fn result(&self) -> Option<String> {
        self.results.try_recv().ok()
    }
}
pub fn start() -> std::io::Result<Worker> {
    start_with_key(None)
}
#[cfg(test)]
pub fn start_fixture(public: Vec<u8>) -> std::io::Result<Worker> {
    start_with_key(Some(public))
}
fn start_with_key(public: Option<Vec<u8>>) -> std::io::Result<Worker> {
    let (tx, rx) = mpsc::sync_channel::<Tick>(1);
    let (result_tx, results) = mpsc::sync_channel(1);
    std::thread::Builder::new()
        .name("agent-local-repair".into())
        .spawn(move || {
            let mut budget = super::recovery::RetryBudget::default();
            let mut marker_checked: Option<std::time::Instant> = None;
            for tick in rx {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|v| v.as_secs())
                    .unwrap_or(0);
                // A marker left by a transaction that already ended halts every local recovery.
                if tick.enabled
                    && marker_checked.map_or(true, |at| at.elapsed() >= std::time::Duration::from_secs(60))
                {
                    marker_checked = Some(std::time::Instant::now());
                    match super::maintenance_orphan::release(
                        &crate::management_agent::system_dirs::get().agent().join("state"),
                        now,
                    ) {
                        Ok(Some(phase)) => hbb_common::log::warn!(
                            "maintenance marker released after transaction ended {phase}"
                        ),
                        Ok(None) => {}
                        Err(error) => hbb_common::throttled_log!(
                            std::time::Duration::from_secs(300),
                            warn,
                            "maintenance marker not released: {error}"
                        ),
                    }
                }
                if !budget.due(now) {
                    continue;
                }
                let outcome = run_once(&tick, public.as_deref());
                let text = match outcome {
                    Ok(value) => {
                        // Same 600-second healthy window as the service path: a crash loop or a
                        // repeatedly deleted executable keeps its backoff.
                        budget.observe_health(value == Decision::Observe, now);
                        format!("{value:?}")
                    }
                    Err(error) => {
                        budget.record_failure(now);
                        format!("Error: {error}")
                    }
                };
                let _ = result_tx.try_send(text);
            }
        })?;
    Ok(Worker { tx, results })
}

#[cfg(test)]
mod attempt_tests {
    use super::*;

    fn ids() -> Vec<String> {
        (0..MAX_ATTEMPTS_PER_BUNDLE)
            .map(|n| format!("id{n}"))
            .collect()
    }

    #[test]
    fn first_attempt_uses_the_historical_slot() {
        assert_eq!(
            next_attempt(&ids(), |_| Ok(AttemptState::Absent)),
            Ok(NextAttempt::New("id0".into()))
        );
    }

    #[test]
    fn a_committed_repair_does_not_block_the_next_loss() {
        let state = |id: &str| {
            Ok(if id == "id0" || id == "id1" {
                AttemptState::Committed
            } else {
                AttemptState::Absent
            })
        };
        assert_eq!(
            next_attempt(&ids(), state),
            Ok(NextAttempt::New("id2".into()))
        );
    }

    #[test]
    fn an_unfinished_or_rolled_back_attempt_stops_automatic_repair() {
        let state = |id: &str| {
            Ok(match id {
                "id0" => AttemptState::Committed,
                "id1" => AttemptState::Blocking,
                _ => AttemptState::Absent,
            })
        };
        assert_eq!(
            next_attempt(&ids(), state),
            Ok(NextAttempt::Blocked("id1".into()))
        );
    }

    #[test]
    fn attempts_are_bounded_per_bundle() {
        assert_eq!(
            next_attempt(&ids(), |_| Ok(AttemptState::Committed)),
            Ok(NextAttempt::Exhausted)
        );
    }

    #[test]
    fn unreadable_state_stops_the_choice() {
        assert_eq!(
            next_attempt(&ids(), |_| Err(
                "automatic local repair attempt unavailable".into()
            )),
            Err("automatic local repair attempt unavailable".into())
        );
    }

    #[test]
    fn attempt_state_reads_real_transaction_directories() {
        let root = std::env::temp_dir().join(format!("rd-attempt-state-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        assert_eq!(
            attempt_state(&root.join("absent")).unwrap(),
            AttemptState::Absent
        );
        std::fs::create_dir(root.join("empty")).unwrap();
        assert_eq!(
            attempt_state(&root.join("empty")).unwrap(),
            AttemptState::Blocking
        );
        std::fs::create_dir(root.join("garbage")).unwrap();
        std::fs::write(root.join("garbage").join("transaction.json"), b"not json").unwrap();
        assert_eq!(
            attempt_state(&root.join("garbage")).unwrap(),
            AttemptState::Blocking
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn attempt_zero_keeps_the_id_written_by_earlier_builds() {
        let a = Approval {
            schema: 2,
            installation_id: "a".repeat(32),
            source_transaction: String::new(),
            source_kind: Some("installed_msi".into()),
            installation_receipt: None,
            installed_source_id: Some("b".repeat(64)),
            source_reference: "c".repeat(64),
            target_key: "windows-x86_64-msi-standard".into(),
            build_seq: 2026103002,
            package: super::super::install_plan::Artifact {
                relative_path: "p.msi".into(),
                size: 1,
                sha256: "d".repeat(64),
            },
        };
        let legacy = hex::encode(Sha256::digest(format!(
            "local-repair-auto-installed-v1\n{}\n{}\n{}\n{}\n{}",
            a.installation_id,
            a.installed_source_id.as_deref().unwrap(),
            a.source_reference,
            a.target_key,
            a.package.sha256
        )))[..32]
            .to_owned();
        assert_eq!(attempt_id(&a), legacy);
        assert_ne!(attempt_id_at(&a, 1), legacy);
        assert_ne!(attempt_id_at(&a, 1), attempt_id_at(&a, 2));
    }
}
