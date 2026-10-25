use super::{msi_metadata, package_gate, read, workspace, write_new, Artifact, Bundle, Layout};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    sync::mpsc,
    time::{Duration, Instant},
};

const AGENT: &str = r"C:\ProgramData\RustDesk\ManagementAgent";
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
    let boundary = if layout.agent == Path::new(AGENT) {
        Path::new(r"C:\ProgramData\RustDesk")
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
    let agent = PathBuf::from(AGENT);
    let identity = hbb_common::config::Config::file();
    if ![
        r"C:\Windows\ServiceProfiles\LocalService\AppData\Roaming\RustDesk\config\RustDesk.toml",
        r"C:\Windows\System32\config\systemprofile\AppData\Roaming\RustDesk\config\RustDesk.toml",
    ]
    .iter()
    .any(|p| identity.to_string_lossy().eq_ignore_ascii_case(p))
    {
        return Err("first installation identity outside fixed profiles".into());
    }
    Ok(Layout {
        destination: agent.join("rescue"),
        transaction: PathBuf::new(),
        client: r"C:\Program Files\RustDesk".into(),
        agent_exe: agent.join("rustdesk-management-agent.exe"),
        agent,
        config: identity.with_file_name("RustDesk2.toml"),
        identity,
        bootstrap: r"C:\ProgramData\RustDesk\RecoveryBootstrap".into(),
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
            r"C:\Program Files\RustDesk".into(),
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

fn waiting() -> serde_json::Value {
    serde_json::json!({"state":"unavailable","error_code":"waiting_approval","ready":false})
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
    let fallback = Path::new(r"C:\ProgramData\RustDesk\Client")
        .join(if key.ends_with("-sos") {
            "sos"
        } else {
            "standard"
        })
        .join("RustDesk");
    #[cfg(test)]
    let fallback = if layout.agent != Path::new(AGENT) {
        layout.agent.join("fallback")
    } else {
        fallback
    };
    let _ = layout;
    super::r3_existing_fallback(client, &fallback).map(|_| ())
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
    registered: impl Fn(&str, &str) -> Result<PathBuf, String>,
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
    let package_path = registered(&approval.product_code, &name)?;
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
    if !absent(&current)? {
        let value = super::verify(&current, installation, key, public)?;
        let old: Bundle =
            serde_json::from_slice(&read(&current.join("bundle.json"), 65536, false)?)
                .map_err(|_| "first installation bundle malformed")?;
        if old.schema != 2
            || old.source_reference != reference
            || old.identity_sha256 != hex::encode(Sha256::digest(&identity))
        {
            return Err("first installation existing bundle preserved".into());
        }
        return Ok(value);
    }
    if !absent(&layout.destination.join("previous"))?
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
        identity_sha256,
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
        || registered(&approval.product_code, &name)? != package_path
        || read(&sidecar, 262144, false)? != approved
        || read(&layout.identity, 65536, true)? != identity
        || read(&layout.config, 65536, true)? != config
    {
        return Err("first installation live source changed; staging preserved".into());
    }
    if !absent(&layout.destination.join("previous"))? {
        return Err("first installation previous appeared; staging preserved".into());
    }
    if let Err(error) = fallback_compatible(layout, key, &client) {
        return Ok(fallback_blocked(error));
    }
    super::publish(&stage, &current)?;
    Ok(
        serde_json::json!({"rescue_readiness":"degraded","source_kind":"installed_msi","source_transaction":null,"source_reference":bundle.source_reference,"client":"verified","offline_package":"verified","credentials":"verified","agent_backup":"verified","bootstrap_backup":"verified"}),
    )
}

fn registered(code: &str, name: &str) -> Result<PathBuf, String> {
    #[cfg(test)]
    if let Some(result) = WORKER_SCOPE.with(|v| v.borrow().as_ref().map(|s| {
        if s.product_code.eq_ignore_ascii_case(code) && s.source.file_name().and_then(|v|v.to_str()) == Some(name) {
            Ok(s.source.clone())
        } else { Err("isolated registered source mismatch".to_owned()) }
    })) { return result; }
    let source = msi_metadata::installed_source(code)?;
    if source.package_name != name {
        return Err("first installation registered basename mismatch".into());
    }
    Ok(source.source)
}
#[cfg(test)]
pub fn prepare_fixture(agent: &Path, public: &[u8]) -> Result<serde_json::Value, String> {
    prepare(
        &super::fixture(agent),
        Some(public),
        |_, name| Ok(agent.join("release-cache").join(name)),
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
        |_, name| Ok(agent.join("release-cache").join(name)),
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
        for () in receiver{
            if last.is_some_and(|time:Instant|time.elapsed()<Duration::from_secs(300)){continue;}last=Some(Instant::now());
            let result=(||{let layout=fixed_layout()?;if !absent(&layout.destination.join("current"))?{return Ok(());}
                #[cfg(not(test))]
                if let Err(error)=super::super::first_install_approval_delivery::discard_superseded_approval(){hbb_common::throttled_log!(Duration::from_secs(300),warn,"first installation superseded approval check unavailable: {error:?}");}
                #[cfg(not(test))]
                if let Err(error)=reclaim_orphaned_staging(&layout){hbb_common::throttled_log!(Duration::from_secs(300),warn,"first installation orphaned staging not reclaimed: {error}");}
                #[cfg(not(test))]
                if absent(&layout.agent.join("release-cache/first-install.json"))? {
                    if let Err(error)=super::super::first_install_approval_delivery::coordinate_current_installation(){hbb_common::throttled_log!(Duration::from_secs(300),warn,"first installation approval unavailable: {error:?}");}
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
                let report=match prepare(&layout,public.as_deref(),registered,live){Ok(report)=>report,Err(error)=>serde_json::json!({"state":"unavailable","error_code":"preparation_refused","error":error,"ready":false})};
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
