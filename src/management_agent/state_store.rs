use std::{io::Write, path::Path};

// Readers elsewhere in the agent open state files without FILE_SHARE_DELETE and Defender briefly
// scans freshly written files; either makes the replacing rename fail with ERROR_ACCESS_DENIED (5)
// or ERROR_SHARING_VIOLATION (32). Both clear within milliseconds, so retry instead of dropping
// the state update (a dropped update used to abort the whole recovery tick).
#[cfg(windows)]
const REPLACE_ATTEMPTS: u32 = 20;
#[cfg(windows)]
const REPLACE_PAUSE: std::time::Duration = std::time::Duration::from_millis(50);

#[cfg(windows)]
fn transient_replace_error(error: &std::io::Error) -> bool {
    matches!(error.raw_os_error(), Some(5) | Some(32))
}

// Several agent threads publish state files; a name made only of the process id collided between
// them and a failed writer's cleanup could delete another writer's staging file.
#[cfg(windows)]
fn staging_path(path: &Path) -> std::path::PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    path.with_extension(format!(
        "tmp-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
}

#[cfg(windows)]
pub fn publish(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "kernel32")]
    extern "system" {
        fn MoveFileExW(source: *const u16, destination: *const u16, flags: u32) -> i32;
        fn SetFileAttributesW(path: *const u16, attributes: u32) -> i32;
    }
    let temporary = staging_path(path);
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)?;
    let result = (|| {
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        let source: Vec<u16> = temporary.as_os_str().encode_wide().chain(Some(0)).collect();
        let destination: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        if unsafe { SetFileAttributesW(source.as_ptr(), 2 | 4) } == 0 {
            return Err(std::io::Error::last_os_error());
        }
        let mut attempt = 1;
        loop {
            if unsafe { MoveFileExW(source.as_ptr(), destination.as_ptr(), 1 | 8) } != 0 {
                return Ok(());
            }
            let error = std::io::Error::last_os_error();
            if attempt >= REPLACE_ATTEMPTS || !transient_replace_error(&error) {
                return Err(error);
            }
            attempt += 1;
            std::thread::sleep(REPLACE_PAUSE);
        }
    })();
    if let Err(error) = result {
        if let Err(cleanup) = std::fs::remove_file(&temporary) {
            return Err(std::io::Error::new(
                error.kind(),
                format!("{error}; state staging cleanup failed: {cleanup}"),
            ));
        }
        return Err(error);
    }
    Ok(())
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use std::os::windows::fs::OpenOptionsExt;

    fn scratch(name: &str) -> std::path::PathBuf {
        // This file is also compiled into registration_reconnect::budget_store, so both copies of
        // these tests run in the same process; the module path keeps their directories apart.
        let root = std::env::temp_dir().join(format!(
            "rd-state-store-{}-{name}-{}",
            module_path!().replace("::", "-"),
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn publish_waits_out_a_reader_without_delete_sharing() {
        let root = scratch("reader");
        let target = root.join("agent.json");
        publish(&target, b"old").unwrap();
        // Same open mode the agent's readers use: read sharing only, no FILE_SHARE_DELETE.
        let reader = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(1)
            .open(&target)
            .unwrap();
        let release = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(200));
            drop(reader);
        });
        publish(&target, b"new").unwrap();
        release.join().unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"new");
        let leftovers = std::fs::read_dir(&root).unwrap().count();
        assert_eq!(leftovers, 1, "staging files must not be left behind");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn publish_gives_up_on_a_reader_that_never_lets_go_and_keeps_the_old_bytes() {
        let root = scratch("held");
        let target = root.join("agent.json");
        publish(&target, b"old").unwrap();
        let reader = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(1)
            .open(&target)
            .unwrap();
        let error = publish(&target, b"new").unwrap_err();
        assert!(transient_replace_error(&error), "{error:?}");
        drop(reader);
        assert_eq!(std::fs::read(&target).unwrap(), b"old");
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn concurrent_publishers_never_share_a_staging_file() {
        let root = scratch("threads");
        let target = root.join("agent.json");
        let handles = (0..8)
            .map(|index| {
                let target = target.clone();
                std::thread::spawn(move || publish(&target, format!("{index}").as_bytes()))
            })
            .collect::<Vec<_>>();
        for handle in handles {
            handle.join().unwrap().unwrap();
        }
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
        std::fs::remove_dir_all(&root).unwrap();
    }
}
