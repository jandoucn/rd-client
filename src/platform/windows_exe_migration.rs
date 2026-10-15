use sha2::{Digest, Sha256};
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};
#[path = "../management_agent/exe_assets.rs"]
mod exe_assets;
#[path = "../management_agent/handoff.rs"]
mod handoff;
#[path = "../management_agent/package_gate.rs"]
mod package_gate;
#[path = "../management_agent/workspace.rs"]
mod workspace;

pub fn bounded_status(
    command: &mut Command,
    timeout: Duration,
    limit: usize,
) -> Result<Vec<u8>, String> {
    bounded_status_inner(
        command,
        timeout,
        limit,
        #[cfg(test)]
        &mut |_| Ok(()),
    )
}

#[cfg(test)]
pub fn bounded_status_after_empty(
    command: &mut Command,
    timeout: Duration,
    limit: usize,
    after_empty: &mut dyn FnMut(&mut std::process::Child) -> Result<(), String>,
) -> Result<Vec<u8>, String> {
    bounded_status_inner(command, timeout, limit, after_empty)
}

fn bounded_status_inner(
    command: &mut Command,
    timeout: Duration,
    limit: usize,
    #[cfg(test)] after_empty: &mut dyn FnMut(&mut std::process::Child) -> Result<(), String>,
) -> Result<Vec<u8>, String> {
    use std::os::windows::io::AsRawHandle;
    #[link(name = "kernel32")]
    extern "system" {
        fn PeekNamedPipe(
            pipe: *mut std::ffi::c_void,
            buffer: *mut std::ffi::c_void,
            size: u32,
            read: *mut u32,
            available: *mut u32,
            left: *mut u32,
        ) -> i32;
    }
    let deadline = Instant::now() + timeout;
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| e.to_string())?;
    // Child owns its process handle; PID/creation is retained independently while polling.
    let owner = match handoff::ObservedProcess::open(child.id()) {
        Ok(p) => p,
        Err(e) => {
            child
                .kill()
                .map_err(|k| format!("{e}; status child kill: {k}"))?;
            child.wait().map_err(|k| k.to_string())?;
            return Err(e);
        }
    };
    let result = (|| {
        let mut stdout = child.stdout.take().ok_or("EXE status stdout unavailable")?;
        let mut bytes = Vec::new();
        let mut exited: Option<std::process::ExitStatus> = None;
        loop {
            if Instant::now() >= deadline {
                return Err("EXE readonly status timeout".into());
            }
            let mut available = 0u32;
            let ok = unsafe {
                PeekNamedPipe(
                    stdout.as_raw_handle(),
                    std::ptr::null_mut(),
                    0,
                    std::ptr::null_mut(),
                    &mut available,
                    std::ptr::null_mut(),
                )
            };
            if ok == 0 {
                let e = std::io::Error::last_os_error();
                if e.raw_os_error() != Some(109) {
                    return Err(format!("EXE readonly status pipe: {e}"));
                }
            }
            if available > 0 {
                if bytes.len().saturating_add(available as usize) > limit {
                    return Err("EXE readonly status output limit".into());
                }
                let mut block = [0u8; 8192];
                let count = (available as usize).min(block.len());
                let n = stdout
                    .read(&mut block[..count])
                    .map_err(|e| e.to_string())?;
                if n == 0 {
                    return Err("EXE status unexpected EOF".into());
                }
                bytes.extend_from_slice(&block[..n]);
                continue;
            }
            if let Some(status) = exited {
                if !status.success() {
                    return Err(format!("EXE readonly status exit: {status}"));
                }
                return Ok(bytes);
            }
            #[cfg(test)]
            after_empty(&mut child)?;
            if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
                // The last empty peek may precede the child's final write. Re-peek
                // after confirmed exit; an empty pipe then needs no inherited EOF.
                exited = Some(status);
                continue;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    })();
    if result.is_err() && owner.is_alive()? {
        child
            .kill()
            .map_err(|e| format!("EXE own readonly status termination failed: {e}"))?;
        let reap = Instant::now() + Duration::from_secs(2);
        while child.try_wait().map_err(|e| e.to_string())?.is_none() {
            if Instant::now() >= reap {
                return Err("EXE own readonly child termination unconfirmed".into());
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    result
}

pub fn completed(root: &Path) -> Result<(), String> {
    let inspected: Result<(Vec<u8>, Vec<u8>, Vec<u8>), String> = (|| {
        let plan = read(&root.join("prepared.json"), 65536)?;
        let tx = read(&root.join("transaction.json"), 65536)?;
        let result = read(&root.join("execution-result.json"), 65536)?;
        let binding = read(&root.join("handoff.json"), 4096)?;
        let handoff: serde_json::Value =
            serde_json::from_slice(&binding).map_err(|_| "handoff malformed")?;
        let owner: handoff::Owner = serde_json::from_value(handoff["helper"].clone())
            .map_err(|_| "helper identity malformed")?;
        if handoff::ObservedProcess::owner_alive(&owner)? {
            return Err("owned helper still running".into());
        }
        let cleanup: serde_json::Value =
            serde_json::from_slice(&read(&root.join("cleanup-complete.json"), 65536)?)
                .map_err(|_| "cleanup receipt malformed")?;
        let p: serde_json::Value = serde_json::from_slice(&plan).map_err(|_| "plan malformed")?;
        let r: serde_json::Value =
            serde_json::from_slice(&result).map_err(|_| "result malformed")?;
        if cleanup["schema"] != 1
            || cleanup["cleanup_finished"] != true
            || cleanup["transaction_id"] != p["transaction_id"]
            || cleanup["installation_id"] != p["installation_id"]
            || cleanup["helper"] != handoff["helper"]
            || cleanup["phase"] != r["phase"]
            || cleanup["plan_sha256"] != hex::encode(Sha256::digest(&plan))
            || cleanup["transaction_sha256"] != hex::encode(Sha256::digest(&tx))
            || cleanup["execution_result_sha256"] != hex::encode(Sha256::digest(&result))
            || cleanup["handoff_sha256"] != hex::encode(Sha256::digest(&binding))
            || r["stopped_state_sha256"]
                != hex::encode(Sha256::digest(read(
                    &root.join("stopped-state.json"),
                    65536,
                )?))
        {
            return Err("cleanup completion binding mismatch".into());
        }
        Ok((plan, tx, result))
    })();
    match inspected {
        Ok((p, t, r)) => terminal(&p, &t, &r),
        Err(e) => Err(format!(
            "[update-pending] {e}; transaction={}",
            root.display()
        )),
    }
}

const AGENT_ROOT: &str = r"C:\ProgramData\RustDesk\ManagementAgent";

pub fn approved_manifest(args: &[String]) -> Result<hbb_common::UpdateManifest, String> {
    let bytes = approval_bytes(args)?;
    serde_json::from_slice(&bytes).map_err(|_| "EXE fixed approval malformed".into())
}

fn approval_bytes(args: &[String]) -> Result<Vec<u8>, String> {
    let value = |name: &str| -> Result<&str, String> {
        let positions = args.iter().enumerate().filter(|(_, a)| a.as_str() == name)
            .map(|(i, _)| i).collect::<Vec<_>>();
        if positions.len() != 1 || args.iter().any(|a| a.starts_with(&format!("{name}="))) {
            return Err("EXE fixed approval arguments missing or ambiguous".into());
        }
        args.get(positions[0] + 1).map(String::as_str)
            .ok_or_else(|| "EXE fixed approval arguments missing or ambiguous".into())
    };
    let path = Path::new(value("--management-agent-approved-manifest")?);
    let digest = value("--management-agent-approved-sha256")?;
    if !path.is_absolute() || digest.len() != 64
        || !digest.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("EXE fixed approval path/digest invalid".into());
    }
    let bytes = read_local_approval(path)?;
    if bytes.len() > 262144 || !hex::encode(Sha256::digest(&bytes)).eq_ignore_ascii_case(digest) {
        return Err("EXE fixed approval digest mismatch".into());
    }
    Ok(bytes)
}

pub fn read_local_approval(path: &Path) -> Result<Vec<u8>, String> {
    use std::{os::windows::{ffi::OsStrExt, fs::{MetadataExt, OpenOptionsExt}}, path::{Component, Prefix}};
    let mut components = path.components();
    let drive = match components.next() {
        Some(Component::Prefix(p)) => match p.kind() {
            Prefix::Disk(drive) => drive,
            _ => return Err("EXE approval requires a local disk path".into()),
        },
        _ => return Err("EXE approval requires a local disk path".into()),
    };
    if components.next() != Some(Component::RootDir)
        || !components.all(|c| matches!(c, Component::Normal(v) if !v.to_string_lossy().contains(':') && !v.to_string_lossy().chars().any(char::is_control))) {
        return Err("EXE approval requires a local disk path".into());
    }
    #[link(name = "kernel32")]
    extern "system" { fn GetDriveTypeW(root: *const u16) -> u32; }
    let root = PathBuf::from(format!("{}:\\", drive as char));
    let wide = root.as_os_str().encode_wide().chain(Some(0)).collect::<Vec<_>>();
    if unsafe { GetDriveTypeW(wide.as_ptr()) } != 3 {
        return Err("EXE approval requires a local fixed disk".into());
    }
    // Pin every ancestor without delete sharing before opening the next component.
    let mut pinned = Vec::new();
    for ancestor in path.ancestors().collect::<Vec<_>>().into_iter().rev() {
        let metadata = std::fs::symlink_metadata(ancestor).map_err(|e| e.to_string())?;
        let leaf = ancestor == path;
        if metadata.file_attributes() & 0x400 != 0 || (leaf && !metadata.is_file()) {
            return Err("EXE approval must be a regular non-reparse file".into());
        }
        let mut options = std::fs::OpenOptions::new();
        options.read(true).share_mode(1).custom_flags(0x00200000 | if leaf { 0 } else { 0x02000000 });
        if !leaf { options.access_mode(0); }
        let file = options.open(ancestor).map_err(|e| e.to_string())?;
        let metadata = file.metadata().map_err(|e| e.to_string())?;
        if metadata.file_attributes() & 0x400 != 0 || (leaf && !metadata.is_file()) {
            return Err("EXE approval must be a regular non-reparse file".into());
        }
        pinned.push(file);
    }
    let file = pinned.last_mut().ok_or("EXE approval file absent")?;
    if file.metadata().map_err(|e| e.to_string())?.len() > 262144 {
        return Err("EXE approval exceeds size limit".into());
    }
    let mut bytes = Vec::new();
    file.take(262145).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    if bytes.len() > 262144 { return Err("EXE approval exceeds size limit".into()); }
    Ok(bytes)
}

pub fn outer_argument(args: &[String]) -> Result<PathBuf, String> {
    if args.first().map(String::as_str) != Some("--update") {
        return Err("EXE migration requires update command".into());
    }
    let positions = args
        .iter()
        .enumerate()
        .filter(|(_, a)| a.as_str() == "--management-agent-outer-package")
        .map(|(i, _)| i)
        .collect::<Vec<_>>();
    if positions.len() != 1
        || args
            .iter()
            .any(|a| a.starts_with("--management-agent-outer-package="))
    {
        return Err("EXE outer package missing or ambiguous".into());
    }
    let path = PathBuf::from(args.get(positions[0] + 1).ok_or("EXE outer path missing")?);
    if !path.is_absolute()
        || path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.eq_ignore_ascii_case("exe"))
            != Some(true)
        || path.to_string_lossy().chars().any(char::is_control)
    {
        return Err("EXE outer path invalid".into());
    }
    Ok(path)
}

pub fn bind_target(
    manifest: &hbb_common::UpdateManifest,
    kind: &str,
    edition: &str,
    version: &str,
    build: u64,
    package: &Path,
) -> Result<(), String> {
    bind(manifest, kind, edition, version, build, package, None).map(|_| ())
}
fn bind(
    manifest: &hbb_common::UpdateManifest,
    kind: &str,
    edition: &str,
    version: &str,
    build: u64,
    package: &Path,
    public: Option<&[u8]>,
) -> Result<package_gate::LockedPackage, String> {
    if kind != "exe" || !matches!(edition, "standard" | "sos") {
        return Err("EXE migration installed kind/edition refused".into());
    }
    let key = format!("windows-x86_64-exe-{edition}");
    let target = match public {
        #[cfg(test)]
        Some(public) => {
            package_gate::verify_with_key(manifest, &key, build, "fixture-key", public)?
        }
        _ => {
            let _ = public;
            package_gate::verify(manifest, &key, build)?
        }
    };
    let (actual_version, actual_build, _) =
        base::update::update_target_release(manifest, target).map_err(|e| e.to_string())?;
    if actual_version != version || actual_build != build {
        return Err("EXE catalog target differs from this exact package build".into());
    }
    package_gate::lock_verified_file(package, target)
}
#[cfg(test)]
pub fn bind_fixture(
    manifest: &hbb_common::UpdateManifest,
    kind: &str,
    edition: &str,
    version: &str,
    build: u64,
    package: &Path,
    public: &[u8],
) -> Result<(), String> {
    bind(
        manifest,
        kind,
        edition,
        version,
        build,
        package,
        Some(public),
    )
    .map(|_| ())
}

pub fn terminal(plan: &[u8], transaction: &[u8], receipt: &[u8]) -> Result<(), String> {
    let p: serde_json::Value = serde_json::from_slice(plan).map_err(|_| "EXE plan malformed")?;
    let t: serde_json::Value =
        serde_json::from_slice(transaction).map_err(|_| "EXE transaction malformed")?;
    let r: serde_json::Value =
        serde_json::from_slice(receipt).map_err(|_| "EXE receipt malformed")?;
    if !p["transaction_id"].is_string()
        || !p["installation_id"].is_string()
        || r["schema"] != 1
        || t["id"] != p["transaction_id"]
        || r["transaction_id"] != p["transaction_id"]
        || r["installation_id"] != p["installation_id"]
        || r["plan_sha256"] != hex::encode(Sha256::digest(plan))
        || r["phase"] != t["phase"]
        || r["reboot_required"] != false
    {
        return Err("EXE terminal binding mismatch".into());
    }
    match t["phase"].as_str() {
        Some("committed") if r.get("error") == Some(&serde_json::Value::Null) => Ok(()),
        Some("rolled_back") if r["error"].is_string() => Err(format!(
            "[update-rolled-back] {}",
            r["error"].as_str().unwrap_or("EXE update failed")
        )),
        Some("rollback_failed") => {
            Err("[update-rollback-failed] EXE transaction requires reconciliation".into())
        }
        _ => Err("EXE transaction not durably committed".into()),
    }
}

fn read(path: &Path, limit: u64) -> Result<Vec<u8>, String> {
    workspace::verify_file(path)?;
    let mut bytes = vec![];
    std::fs::File::open(path)
        .map_err(|e| e.to_string())?
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > limit {
        return Err("EXE metadata too large".into());
    }
    Ok(bytes)
}
fn write_new(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| e.to_string())?;
    f.write_all(bytes)
        .and_then(|_| f.sync_all())
        .map_err(|e| e.to_string())?;
    drop(f);
    workspace::verify_file(path)
}

pub fn approval_binding(plan: &[u8], approval: &[u8], id: &str, installation: &str,
    key: &str, version: &str, build: u64) -> Result<Vec<u8>, String> {
    let p: serde_json::Value = serde_json::from_slice(plan).map_err(|_| "EXE approval plan malformed")?;
    let raw: serde_json::Value = serde_json::from_slice(approval).map_err(|_| "EXE approval malformed")?;
    let manifest: hbb_common::UpdateManifest = serde_json::from_value(raw.clone()).map_err(|_| "EXE approval malformed")?;
    let target = manifest.targets.get(key).ok_or("EXE approval target missing")?;
    let (v, b, _) = base::update::update_target_release(&manifest, target).map_err(|e| e.to_string())?;
    if p["transaction_id"] != id || p["installation_id"] != installation || p["target_key"] != key
        || p["target_version"] != version || p["target_build_seq"] != build
        || v != version || b != build || p["package"]["size"] != target.size
        || p["package"]["sha256"] != target.sha256 {
        return Err("EXE approval prepared binding mismatch".into());
    }
    serde_json::to_vec(&serde_json::json!({"schema":1,"transaction_id":id,"installation_id":installation,
        "target_key":key,"version":version,"build_seq":build,"package":p["package"],
        "plan_sha256":hex::encode(Sha256::digest(plan)),"approval_sha256":hex::encode(Sha256::digest(approval)),
        "approved_reference":package_gate::package_reference(&raw,key)?})).map_err(|e| e.to_string())
}
fn protected(path: &Path) -> Result<(), String> {
    if path.exists() {
        workspace::verify(path)
    } else {
        workspace::create(path)
    }
}

fn cli(exe: &Path, command: &str, id: &str, deadline: Instant) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    let mut child = Command::new(exe)
        .args([command, "--transaction-id", id])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(0x08000000)
        .spawn()
        .map_err(|e| format!("EXE CLI launch: {e}"))?;
    loop {
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            return if status.success() {
                Ok(())
            } else {
                Err(format!("EXE CLI {command} failed: {status}"))
            };
        }
        if Instant::now() >= deadline {
            return Err(format!("[update-pending] EXE CLI still active; transaction={id}; reconcile existing transaction"));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

pub fn run(args: &[String]) -> Result<(), String> {
    if args.iter().any(|a| a == "--update-event" || a.starts_with("--update-event=")) {
        return Err("EXE update-event consumer unavailable; transaction not started".into());
    }
    let outer = outer_argument(args)?;
    let approval = approval_bytes(args)?;
    let manifest: hbb_common::UpdateManifest = serde_json::from_slice(&approval)
        .map_err(|_| "EXE fixed approval malformed")?;
    let root = Path::new(AGENT_ROOT);
    let state_path = root.join("state/agent.json");
    workspace::verify_restore_target(&state_path, false)?;
    let state_bytes = std::fs::read(&state_path).map_err(|e| e.to_string())?;
    if state_bytes.len() > 65536 {
        return Err("EXE installed state too large".into());
    }
    let state: serde_json::Value =
        serde_json::from_slice(&state_bytes).map_err(|_| "EXE installed state malformed")?;
    let metadata: serde_json::Value =
        serde_json::from_str(include_str!("../../version.json")).map_err(|e| e.to_string())?;
    let build = metadata["build_seq"]
        .as_u64()
        .ok_or("EXE current package build missing")?;
    let version = metadata["version"].as_str().ok_or("EXE version missing")?;
    let edition = if option_env!("RUSTDESK_SOS") == Some("1") {
        "sos"
    } else {
        "standard"
    };
    if state["package_kind"] != "exe"
        || state["edition"] != edition
        || state["normal_install_path"] != r"C:\Program Files\RustDesk"
        || state["disabled"] != false
        || state["management_agent_enabled"] != true
    {
        return Err("EXE migration fixed installed identity/layout refused".into());
    }
    for marker in ["disable-agent", "disable-recovery", "maintenance.json"] {
        match std::fs::symlink_metadata(root.join("state").join(marker)) {
            Ok(_) => return Err("EXE migration locally blocked".into()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.to_string()),
        }
    }
    workspace::verify_restore_target(&root.join("state/service-recovery-enabled"), false)?;
    let intent_path = hbb_common::config::Config2::file();
    workspace::verify_restore_target(&intent_path, true)?;
    let intent_text = std::fs::read_to_string(&intent_path).map_err(|e| e.to_string())?;
    if intent_text.len() > 65536 {
        return Err("EXE intent too large".into());
    }
    #[derive(serde::Deserialize)]
    struct Intent {
        #[serde(default)]
        options: std::collections::HashMap<String, String>,
    }
    let intent: Intent =
        hbb_common::toml::from_str(&intent_text).map_err(|_| "EXE intent unknown")?;
    if !matches!(
        intent.options.get("stop-service").map(String::as_str),
        None | Some("" | "N")
    ) {
        return Err("EXE user stop intent".into());
    }
    let _lock = bind(&manifest, "exe", edition, version, build, &outer, None)?;
    protected(&root.join("exe-assets"))?;
    let key = format!("windows-x86_64-exe-{edition}");
    let sealed = exe_assets::prepare(
        &root.join("exe-assets"),
        &outer,
        &manifest,
        &key,
        "exe",
        edition,
        build,
    )
    .map_err(|e| format!("EXE signed extraction failed: {e:?}"))?;
    let helper = sealed.root.join("payload/rustdesk-management-agent.exe");
    let target = sealed
        .files
        .iter()
        .find(|f| f.0 == "rustdesk-management-agent.exe")
        .ok_or("EXE signed helper missing")?;
    let helper_target = hbb_common::UpdateTarget {
        size: target.1,
        sha256: target.2.clone(),
        ..Default::default()
    };
    let _helper_lock = package_gate::lock_verified_file(&helper, &helper_target)?;
    // Installed agent reports current build; preparation verifies it against v2 IPC.
    let old = root.join("rustdesk-management-agent.exe");
    workspace::verify_restore_target(&old, false)?;
    let output = bounded_status(
        Command::new(old).args([
            "status",
            "--json",
            "--edition",
            edition,
            "--package-kind",
            "exe",
        ]),
        Duration::from_secs(20),
        262144,
    )?;
    let status: serde_json::Value =
        serde_json::from_slice(&output).map_err(|_| "EXE old agent status malformed")?;
    let old_build = status["application"]["build_seq"]
        .as_u64()
        .ok_or("EXE old application build unknown")?;
    if status["application"]["state"] != "ready"
        || status["agent_installation_id"] != state["agent_installation_id"]
        || old_build > build
    {
        return Err("EXE old installation not eligible".into());
    }
    protected(&root.join("exe-inbox"))?;
    protected(&root.join("transactions"))?;
    let id = uuid::Uuid::new_v4().simple().to_string();
    let inbox = root.join("exe-inbox").join(&id);
    workspace::create(&inbox)?;
    let mut source = std::fs::File::open(&outer).map_err(|e| e.to_string())?;
    let mut destination = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(inbox.join("package.exe"))
        .map_err(|e| e.to_string())?;
    std::io::copy(&mut source, &mut destination).map_err(|e| e.to_string())?;
    destination.sync_all().map_err(|e| e.to_string())?;
    drop(destination);
    bind_target(
        &manifest,
        "exe",
        edition,
        version,
        build,
        &inbox.join("package.exe"),
    )?;
    write_new(
        &inbox.join("approved-manifest.json"),
        &approval,
    )?;
    write_new(
        &inbox.join("prepare.json"),
        &serde_json::to_vec(&serde_json::json!({"current_build":old_build}))
            .map_err(|e| e.to_string())?,
    )?;
    let deadline = Instant::now() + Duration::from_secs(1200);
    cli(&helper, "exe-transaction-prepare", &id, deadline)?;
    let transaction = root.join("transactions").join(&id);
    let prepared = read(&transaction.join("prepared.json"), 65536)?;
    let binding = approval_binding(&prepared, &approval, &id,
        state["agent_installation_id"].as_str().ok_or("EXE installation identity missing")?,
        &key, version, build)?;
    write_new(&transaction.join("online-approved-manifest.json"), &approval)?;
    write_new(&transaction.join("online-approval-binding.json"), &binding)?;
    if read(&transaction.join("prepared.json"),65536)? != prepared
        || read(&transaction.join("online-approved-manifest.json"),262144)? != approval
        || read(&transaction.join("online-approval-binding.json"),65536)? != binding {
        return Err("EXE approval publication changed".into());
    }
    cli(&helper, "exe-transaction-launch", &id, deadline).map_err(|e| {
        format!("[update-pending] {e}; transaction={id}; launch outcome requires reconciliation")
    })?;
    loop {
        if transaction.join("cleanup-complete.json").exists() {
            let result = completed(&transaction);
            if !result
                .as_ref()
                .err()
                .is_some_and(|e| e.contains("[update-pending]"))
            {
                return result;
            }
        }
        if Instant::now() >= deadline {
            return Err(format!("[update-pending] EXE update outcome or cleanup unknown; transaction={}; reconcile recorded transaction",transaction.display()));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}
