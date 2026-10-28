use super::{package_gate, workspace};
use std::path::Path;

pub fn gate(state: &str, disabled: bool, pid_present: bool) -> Result<(), String> {
    if state != "stopped" || disabled || pid_present {
        return Err("agent restore requires stopped non-disabled service without live PID".into());
    }
    Ok(())
}
fn absent(path: &Path) -> Result<(), String> {
    match std::fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err("agent restore destination unreadable".into()),
        Ok(_) => Err("agent restore destination occupied; preserve evidence".into()),
    }
}
#[cfg(test)]
fn publish(source: &Path, destination: &Path, size: u64, hash: &str) -> Result<(), String> {
    publish_checked(source, destination, size, hash, || Ok(()), || Ok(()))
}
pub(super) fn publish_checked(
    source: &Path,
    destination: &Path,
    size: u64,
    hash: &str,
    before_create: impl FnOnce() -> Result<(), String>,
    before_publish: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    use std::{io::Write, os::windows::ffi::OsStrExt};
    before_create()?;
    absent(destination)?;
    workspace::verify_file(source)?;
    let target = hbb_common::UpdateTarget {
        size,
        sha256: hash.into(),
        ..Default::default()
    };
    let _lock = package_gate::lock_verified_file(source, &target)?;
    let parent = destination
        .parent()
        .ok_or("agent restore destination parent missing")?;
    let staging = parent.join("agent-restore-staging");
    workspace::create(&staging)?;
    let temp = staging.join("agent.exe");
    let mut input = std::fs::File::open(source).map_err(|e| e.to_string())?;
    let mut output = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)
        .map_err(|e| e.to_string())?;
    let copied = std::io::copy(&mut input, &mut output).map_err(|e| e.to_string())?;
    output
        .flush()
        .and_then(|_| output.sync_all())
        .map_err(|e| e.to_string())?;
    drop(output);
    if copied != size {
        return Err("agent restore copied size mismatch".into());
    }
    let verified = package_gate::lock_verified_file(&temp, &target)?;
    drop(verified);
    workspace::verify_file(&temp)?;
    absent(destination)?;
    #[cfg(test)]
    AFTER_COPY.with(|hook| {
        if let Some(hook) = hook.borrow_mut().take() {
            hook();
        }
    });
    before_publish()?;
    #[link(name = "kernel32")]
    extern "system" {
        fn MoveFileExW(source: *const u16, destination: *const u16, flags: u32) -> i32;
    }
    let from = temp
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let to = destination
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    if unsafe { MoveFileExW(from.as_ptr(), to.as_ptr(), 8) } == 0 {
        return Err(format!(
            "agent restore no-overwrite publication failed: {}",
            std::io::Error::last_os_error()
        ));
    }
    package_gate::lock_verified_file(destination, &target)?;
    std::fs::remove_dir(&staging)
        .map_err(|e| format!("agent restore empty staging cleanup failed: {e}"))?;
    Ok(())
}
#[cfg(test)]
pub fn publish_fixture(
    source: &Path,
    destination: &Path,
    size: u64,
    hash: &str,
) -> Result<(), String> {
    publish(source, destination, size, hash)
}
#[cfg(test)]
std::thread_local! {static AFTER_COPY:std::cell::RefCell<Option<Box<dyn FnOnce()>>>=std::cell::RefCell::new(None);}
#[cfg(test)]
pub fn publication_fixture(
    source: &Path,
    destination: &Path,
    size: u64,
    hash: &str,
    before_create: impl FnOnce() -> Result<(), String>,
    before_publish: impl FnOnce() -> Result<(), String>,
    after_copy: impl FnOnce() + 'static,
) -> Result<(), String> {
    struct Clear;
    impl Drop for Clear {
        fn drop(&mut self) {
            AFTER_COPY.with(|hook| {
                hook.borrow_mut().take();
            });
        }
    }
    AFTER_COPY.with(|hook| *hook.borrow_mut() = Some(Box::new(after_copy)));
    let _clear = Clear;
    publish_checked(
        source,
        destination,
        size,
        hash,
        before_create,
        before_publish,
    )
}

pub fn process_present() -> Result<bool, String> {
    use windows::Win32::{
        Foundation::CloseHandle,
        System::Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
            TH32CS_SNAPPROCESS,
        },
    };
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }
        .map_err(|_| "agent restore process scan unavailable")?;
    let result = (|| {
        let mut entry = PROCESSENTRY32W::default();
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        unsafe { Process32FirstW(snapshot, &mut entry) }
            .map_err(|_| "agent restore process list unknown")?;
        loop {
            let n = entry
                .szExeFile
                .iter()
                .position(|&c| c == 0)
                .unwrap_or(entry.szExeFile.len());
            if String::from_utf16_lossy(&entry.szExeFile[..n])
                .eq_ignore_ascii_case("rustdesk-management-agent.exe")
            {
                return Ok(true);
            }
            if let Err(error) = unsafe { Process32NextW(snapshot, &mut entry) } {
                if error.code().0 as u32 != 0x80070012 {
                    return Err("agent restore process scan failed".into());
                }
                break;
            }
        }
        Ok(false)
    })();
    unsafe { CloseHandle(snapshot) }.map_err(|_| "agent restore process handle cleanup failed")?;
    result
}

pub fn restore(
    expected_hash: &str,
    edition: &str,
    kind: &str,
    build: u64,
    public: Option<&[u8]>,
    before_create: impl FnOnce() -> Result<(), String>,
    before_publish: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    let destination = &super::system_dirs::get()
        .agent()
        .join("rustdesk-management-agent.exe");
    absent(destination)?;
    let (source, size) =
        super::rescue_assets::trusted_agent_backup(expected_hash, edition, kind, build, public)?;
    if process_present()? {
        return Err("agent restore live process refused".into());
    }
    publish_checked(
        &source,
        destination,
        size,
        expected_hash,
        before_create,
        before_publish,
    )
}
