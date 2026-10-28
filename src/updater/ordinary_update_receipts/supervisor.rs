use super::{workspace, Outbox};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Read,
    path::Path,
    process::{Child, Command},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
#[path = "../../platform/windows_agent_guard.rs"]
pub(crate) mod guard;
#[path = "../../management_agent/handoff.rs"]
mod handoff;
use handoff::{Handoff, ObservedProcess, ReadyReceipt, ReleaseReceipt};

pub(crate) fn validate_package_basename(name: &str) -> Result<String, String> {
    if name.is_empty()
        || name.len() > 180
        || !name.ends_with(".msi")
        || name.contains(['/', '\\', ':', '%', '?', '#'])
        || name.chars().any(char::is_control)
        || name
            .split('.')
            .any(|part| part.is_empty() || part == "." || part == "..")
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._-+".contains(&byte))
    {
        return Err("ordinary package basename invalid".into());
    }
    Ok(name.to_owned())
}

pub(crate) fn approved_package_basename(package: &Path) -> Result<String, String> {
    let name = package
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or("ordinary package basename unavailable")?;
    validate_package_basename(name)
}

fn now() -> Result<u64, String> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|v| v.as_secs())
        .map_err(|e| e.to_string())
}

fn locked_hash(path: &Path) -> Result<(fs::File, String), String> {
    use std::os::windows::fs::OpenOptionsExt;
    workspace::verify_file(path)?;
    let mut file = fs::OpenOptions::new()
        .read(true)
        .share_mode(1)
        .custom_flags(0x00200000)
        .open(path)
        .map_err(|e| e.to_string())?;
    let mut hash = Sha256::new();
    let mut bytes = [0; 65536];
    loop {
        let n = file.read(&mut bytes).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        hash.update(&bytes[..n]);
    }
    Ok((file, hex::encode(hash.finalize())))
}

pub(crate) fn seal_runtime(outbox: &Outbox) -> Result<(), String> {
    let runtime = outbox.directory.join("runtime");
    workspace::verify(&runtime)?;
    let mut inventory = std::collections::BTreeMap::new();
    for entry in fs::read_dir(&runtime).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| "ordinary runtime filename invalid")?;
        if name != "ordinary-msi-writer.exe" && !name.to_ascii_lowercase().ends_with(".dll") {
            return Err("ordinary runtime unexpected file".into());
        }
        inventory.insert(name, locked_hash(&entry.path())?.1);
    }
    if !inventory.contains_key("ordinary-msi-writer.exe") {
        return Err("ordinary writer image absent".into());
    }
    outbox.write_once("runtime-inventory.json", inventory)
}

fn verify_runtime(outbox: &Outbox) -> Result<Vec<fs::File>, String> {
    let inventory: std::collections::BTreeMap<String, String> =
        outbox.read("runtime-inventory.json")?;
    if !inventory.contains_key("ordinary-msi-writer.exe") {
        return Err("ordinary writer inventory absent".into());
    }
    let mut locks = Vec::new();
    for (name, expected) in inventory {
        if name.contains(['/', '\\', ':'])
            || (name != "ordinary-msi-writer.exe" && !name.to_ascii_lowercase().ends_with(".dll"))
        {
            return Err("ordinary runtime path refused".into());
        }
        let (file, hash) = locked_hash(&outbox.directory.join("runtime").join(name))?;
        if hash != expected {
            return Err("ordinary runtime hash mismatch".into());
        }
        locks.push(file);
    }
    Ok(locks)
}

fn wait_record<T: serde::de::DeserializeOwned>(
    outbox: &Outbox,
    name: &str,
    child: Option<&mut Child>,
) -> Result<T, String> {
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut child = child;
    loop {
        if outbox.directory.join(name).exists() {
            return outbox.read(name);
        }
        if let Some(child) = child.as_deref_mut() {
            if child.try_wait().map_err(|e| e.to_string())?.is_some() {
                return Err("ordinary supervisor exited before handshake".into());
            }
        }
        if Instant::now() >= deadline {
            return Err("ordinary supervisor handshake timeout".into());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Called with the parent's existing guard. Release is explicit and bound to a
/// live, actually executed helper; a dead parent is never substitute permission.
pub(crate) fn launch(outbox: &Outbox, parent_guard: guard::InstallGuard) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    let _locks = verify_runtime(outbox)?;
    let digest = outbox.intent_digest()?;
    let mut command = Command::new(outbox.directory.join(r"runtime\ordinary-msi-writer.exe"));
    #[cfg(not(test))]
    command.args([
        "--ordinary-msi-supervisor",
        &outbox.binding.transaction_id,
        &digest,
    ]);
    #[cfg(all(test, not(t02_integration)))]
    command
        .args([
            "--exact",
            "persistence::supervisor_green_child",
            "--ignored",
            "--nocapture",
        ])
        .env(
            "T02_SUPERVISOR_ROOT",
            outbox.directory.parent().ok_or("ordinary root absent")?,
        )
        .env("T02_SUPERVISOR_DIGEST", &digest);
    #[cfg(all(test, t02_integration))]
    command
        .args([
            "--exact",
            "integration::supervisor_child",
            "--ignored",
            "--nocapture",
        ])
        .env(
            "T02_SUPERVISOR_ROOT",
            outbox.directory.parent().ok_or("ordinary root absent")?,
        )
        .env("T02_SUPERVISOR_DIGEST", &digest)
        .env("T02_SUPERVISOR_TX", &outbox.binding.transaction_id);
    let mut child = command
        .current_dir(outbox.directory.join("runtime"))
        .creation_flags(0x08000000)
        .spawn()
        .map_err(|e| e.to_string())?;
    let result = (|| {
        let parent = ObservedProcess::open(std::process::id())?;
        let helper = ObservedProcess::open(child.id())?;
        let mut nonce = [0u8; 32];
        #[link(name = "bcrypt")]
        extern "system" {
            fn BCryptGenRandom(
                algorithm: *mut std::ffi::c_void,
                buffer: *mut u8,
                length: u32,
                flags: u32,
            ) -> i32;
        }
        if unsafe { BCryptGenRandom(std::ptr::null_mut(), nonce.as_mut_ptr(), 32, 2) } < 0 {
            return Err("ordinary handshake randomness failed".into());
        }
        let binding = Handoff::new(
            outbox.binding.transaction_id.clone(),
            digest,
            hex::encode(nonce),
            parent.owner().clone(),
            helper.owner().clone(),
            now()?,
        )?;
        outbox.write_once("supervisor-handoff.json", &binding)?;
        let ready: ReadyReceipt = wait_record(outbox, "writer-ready.json", Some(&mut child))?;
        binding.verify_ready(&ready, now()?)?;
        if !helper.is_alive()? {
            return Err("ordinary executed writer exited".into());
        }
        let release = binding.release_receipt(&ready, now()?)?;
        drop(parent_guard);
        outbox.write_once("supervisor-release.json", release)?;
        let acquired: ReadyReceipt =
            wait_record(outbox, "supervisor-acquired.json", Some(&mut child))?;
        binding.verify_ready(&acquired, now()?)?;
        Ok(())
    })();
    // After release the child owns execution. Never kill a possibly running MSI.
    if result.is_err() && !outbox.directory.join("supervisor-release.json").exists() {
        child
            .kill()
            .map_err(|e| format!("ordinary unreleased helper cleanup: {e}"))?;
        child.wait().map_err(|e| e.to_string())?;
    }
    result
}

pub(crate) fn selfcheck(outbox: &Outbox, digest: &str) -> Result<(), String> {
    if outbox.intent_digest()? != digest {
        return Err("ordinary supervisor intent mismatch".into());
    }
    let actual = fs::canonicalize(std::env::current_exe().map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    let expected = fs::canonicalize(outbox.directory.join(r"runtime\ordinary-msi-writer.exe"))
        .map_err(|e| e.to_string())?;
    if actual != expected {
        return Err("ordinary supervisor image mismatch".into());
    }
    let _locks = verify_runtime(outbox)?;
    let binding: Handoff = wait_record(outbox, "supervisor-handoff.json", None)?;
    let me = ObservedProcess::open(std::process::id())?;
    if binding.helper != *me.owner()
        || binding.plan_sha256 != digest
        || binding.transaction_id != outbox.binding.transaction_id
        || !ObservedProcess::owner_alive(&binding.parent)?
    {
        return Err("ordinary supervisor process binding mismatch".into());
    }
    outbox.write_once("writer-ready.json", binding.ready_receipt(now()?)?)
}

pub(crate) fn acquire_ready(outbox: &Outbox) -> Result<guard::InstallGuard, String> {
    let binding: Handoff = outbox
        .read("supervisor-handoff.json")
        .map_err(|_| "ordinary executed writer selfcheck missing")?;
    let ready: ReadyReceipt = outbox.read("writer-ready.json")?;
    let released: ReleaseReceipt = outbox.read("supervisor-release.json")?;
    let me = ObservedProcess::open(std::process::id())?;
    if binding.plan_sha256 != outbox.intent_digest()?
        || binding.transaction_id != outbox.binding.transaction_id
    {
        return Err("ordinary supervisor intent mismatch".into());
    }
    binding.verify_release(&ready, &released, me.owner(), now()?)?;
    let guard = guard::InstallGuard::try_acquire()
        .map_err(|e| e.to_string())?
        .ok_or("ordinary supervisor install lease busy")?;
    outbox.write_once("supervisor-acquired.json", &ready)?;
    Ok(guard)
}

#[cfg(test)]
pub(crate) fn fixture_run(
    outbox: &Outbox,
    digest: &str,
    run: impl FnOnce(&str, &str) -> Result<(), String>,
) -> Result<(), String> {
    selfcheck(outbox, digest)?;
    let _: ReleaseReceipt = wait_record(outbox, "supervisor-release.json", None)?;
    let _locks = verify_runtime(outbox)?;
    outbox.dispatch(run)
}

#[cfg(test)]
pub(crate) fn fixture_stage(outbox: &Outbox) -> Result<(), String> {
    let runtime = outbox.directory.join("runtime");
    workspace::create(&runtime)?;
    fs::copy(
        std::env::current_exe().map_err(|e| e.to_string())?,
        runtime.join("ordinary-msi-writer.exe"),
    )
    .map_err(|e| e.to_string())?;
    seal_runtime(outbox)
}

#[cfg(test)]
pub(crate) fn fixture_wait_exit(outbox: &Outbox) -> Result<(), String> {
    let ready: ReadyReceipt = outbox.read("writer-ready.json")?;
    let deadline = Instant::now() + Duration::from_secs(20);
    while ObservedProcess::owner_alive(&ready.helper)? {
        if Instant::now() >= deadline {
            return Err("ordinary fixture helper still running".into());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    Ok(())
}

pub(crate) fn run(args: &[String]) -> Result<(), String> {
    if args.len() != 2 {
        return Err("ordinary supervisor arguments invalid".into());
    }
    super::OutcomeWriteRequest::parse(&[args[0].clone(), args[1].clone(), "0".into()])?;
    let outbox = Outbox::open_existing(&args[0])?;
    selfcheck(&outbox, &args[1])?;
    let _: ReleaseReceipt = wait_record(&outbox, "supervisor-release.json", None)?;
    let _runtime_locks = verify_runtime(&outbox)?;
    let package_name = outbox.package_name()?;
    let (package_lock, hash) = locked_hash(&outbox.directory.join(&package_name))?;
    if hash != outbox.binding.package_sha256 {
        return Err("ordinary supervisor package mismatch".into());
    }
    outbox.dispatch(|_, _| {
        #[cfg(not(t02_receipts))]
        super::runtime::install_allowed()?;
        let status = Command::new(
            crate::platform::windows::system_dirs::get()
                .system32()
                .join("msiexec.exe"),
        )
        .arg("/i")
        .arg(outbox.directory.join(&package_name))
        .args([
            "/qn",
            "LAUNCH_TRAY_APP=N",
            "REBOOT=ReallySuppress",
            "/norestart",
        ])
        .status()
        .map_err(|e| e.to_string())?;
        let code = status.code().ok_or("ordinary installer exit unavailable")? as u32;
        let result = super::OutcomeWriteRequest::parse(&[
            args[0].clone(),
            args[1].clone(),
            code.to_string(),
        ])?
        .write();
        drop(package_lock);
        result
    })
}
