use super::{install_plan::Artifact, workspace};
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Write},
    os::windows::{ffi::OsStrExt, fs::OpenOptionsExt},
    path::Path,
};

pub fn restore(root: &Path, source: &Artifact, destination: &Path) -> Result<(), String> {
    source.verify(root)?;
    workspace::verify_file(&root.join(&source.relative_path))?;
    let dirs = crate::management_agent::system_dirs::get();
    let identity_target = [
        dirs.local_service_roaming()
            .join(r"RustDesk\config\RustDesk.toml"),
        dirs.system_profile_roaming()
            .join(r"RustDesk\config\RustDesk.toml"),
    ]
    .iter()
    .any(|path| {
        destination
            .to_string_lossy()
            .eq_ignore_ascii_case(&path.to_string_lossy())
    });
    workspace::verify_restore_target(destination, identity_target)?;
    let temporary = destination.with_extension(format!("restore-{}", std::process::id()));
    let mut input = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(root.join(&source.relative_path))
        .map_err(|e| e.to_string())?;
    let mut output = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|e| e.to_string())?;
    let result = (|| {
        let mut hash = Sha256::new();
        let mut size = 0u64;
        let mut buffer = [0u8; 65536];
        loop {
            let count = input.read(&mut buffer).map_err(|e| e.to_string())?;
            if count == 0 {
                break;
            }
            size = size.saturating_add(count as u64);
            if size > source.size {
                return Err("restored backup size changed".into());
            }
            hash.update(&buffer[..count]);
            output
                .write_all(&buffer[..count])
                .map_err(|e| e.to_string())?;
        }
        output.sync_all().map_err(|e| e.to_string())?;
        drop(output);
        if size != source.size || hex::encode(hash.finalize()) != source.sha256 {
            return Err("restored backup hash/size changed".into());
        }
        workspace::verify_restore_target(&temporary, identity_target)?;
        #[link(name = "kernel32")]
        extern "system" {
            fn ReplaceFileW(
                destination: *const u16,
                replacement: *const u16,
                backup: *const u16,
                flags: u32,
                exclude: *mut std::ffi::c_void,
                reserved: *mut std::ffi::c_void,
            ) -> i32;
        }
        let target = destination
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect::<Vec<_>>();
        let replacement = temporary
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect::<Vec<_>>();
        if unsafe {
            ReplaceFileW(
                target.as_ptr(),
                replacement.as_ptr(),
                std::ptr::null(),
                0,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        } == 0
        {
            return Err(format!(
                "backup file restore publication: {}",
                std::io::Error::last_os_error()
            ));
        }
        workspace::verify_restore_target(destination, identity_target)
    })();
    if let Err(error) = result {
        if temporary.exists() {
            if let Err(cleanup) = std::fs::remove_file(&temporary) {
                return Err(format!(
                    "{error}; backup restore staging cleanup: {cleanup}"
                ));
            }
        }
        return Err(error);
    }
    Ok(())
}
