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

#[derive(Clone)]
pub struct Tick {
    pub elapsed: Duration,
    pub enabled: bool,
    pub retry_at: u64,
}

fn normal_missing() -> Result<bool, String> {
    use std::os::windows::fs::MetadataExt;
    let normal = Path::new(r"C:\Program Files\RustDesk");
    match std::fs::symlink_metadata(normal) {
        Ok(metadata) => {
            if metadata.file_attributes() & 0x400 != 0 {
                return Err("automatic local repair normal reparse refused".into());
            }
            Ok(false)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            for parent in normal.ancestors().skip(1) {
                let m = std::fs::symlink_metadata(parent)
                    .map_err(|_| "automatic local repair parent unavailable")?;
                if !m.is_dir() || m.file_attributes() & 0x400 != 0 {
                    return Err("automatic local repair parent invalid".into());
                }
            }
            Ok(true)
        }
        Err(_) => Err("automatic local repair normal state unknown".into()),
    }
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
    let id = attempt_id(a);
    match std::fs::symlink_metadata(transactions.join(&id)) {
        Ok(_) => return Ok(Decision::Existing(id)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.to_string()),
    }
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
        Path::new(r"C:\ProgramData\RustDesk\ManagementAgent\rescue\current\bundle.json"),
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
        let root = Path::new(r"C:\ProgramData\RustDesk\ManagementAgent\rescue\current");
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
    if let Some(source) = &a.installed_source_id {
        return hex::encode(Sha256::digest(format!(
            "local-repair-auto-installed-v1\n{}\n{}\n{}\n{}\n{}",
            a.installation_id, source, a.source_reference, a.target_key, a.package.sha256
        )))[..32]
            .to_owned();
    }
    hex::encode(Sha256::digest(format!(
        "local-repair-auto-v1\n{}\n{}\n{}\n{}\n{}",
        a.installation_id, a.source_transaction, a.source_reference, a.target_key, a.package.sha256
    )))[..32]
        .to_owned()
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
    let transactions = Path::new(r"C:\ProgramData\RustDesk\ManagementAgent\transactions");
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
        return Ok(Decision::Existing(id));
    }
    let fallback = Path::new(r"C:\ProgramData\RustDesk\Client")
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
    // Reserve this exact approval through the existing transaction namespace. Partial directories
    // and terminal plans prevent retries across worker or agent restarts without a new counter.
    let id = attempt_id(&a);
    match std::fs::symlink_metadata(transactions.join(&id)) {
        Ok(_) => return Ok(Decision::Existing(id)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return Err("automatic local repair attempt unavailable".into()),
    }
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
            for tick in rx {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|v| v.as_secs())
                    .unwrap_or(0);
                if !budget.due(now) {
                    continue;
                }
                let outcome = run_once(&tick, public.as_deref());
                let text = match outcome {
                    Ok(value) => format!("{value:?}"),
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
