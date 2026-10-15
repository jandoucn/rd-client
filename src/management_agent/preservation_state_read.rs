use super::super::super::workspace;
use std::{
    fs::File,
    os::windows::{
        ffi::OsStrExt,
        fs::{MetadataExt, OpenOptionsExt},
    },
    path::Path,
};
use windows::{
    core::{PCWSTR, PWSTR},
    Win32::{
        Foundation::{LocalFree, HLOCAL},
        Security::{
            Authorization::{ConvertSidToStringSidW, GetNamedSecurityInfoW, SE_FILE_OBJECT},
            GetAce, ACCESS_ALLOWED_ACE, ACE_HEADER, ACL, DACL_SECURITY_INFORMATION,
            OWNER_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID,
        },
    },
};

struct Allocation(*mut std::ffi::c_void);
impl Drop for Allocation {
    fn drop(&mut self) {
        if !self.0.is_null() && !unsafe { LocalFree(Some(HLOCAL(self.0))) }.0.is_null() {
            eprintln!("preservation state descriptor cleanup failed");
        }
    }
}
fn sid(sid: PSID) -> Result<String, String> {
    let mut text = PWSTR::null();
    unsafe { ConvertSidToStringSidW(sid, &mut text) }.map_err(|e| e.to_string())?;
    let _allocation = Allocation(text.0.cast());
    unsafe { text.to_string() }.map_err(|e| e.to_string())
}

fn directory_acl(path: &Path, state: bool) -> Result<(), String> {
    const SYSTEM: &str = "S-1-5-18";
    const ADMINS: &str = "S-1-5-32-544";
    const INSTALLER: &str = "S-1-5-80-956008885-3418522649-1831038044-1853292631-2271478464";
    let wide = path
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let mut owner = PSID::default();
    let mut acl: *mut ACL = std::ptr::null_mut();
    let mut descriptor = PSECURITY_DESCRIPTOR::default();
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
    if status.0 != 0 {
        return Err(format!("preservation state ACL read failed: {}", status.0));
    }
    let _allocation = Allocation(descriptor.0);
    let owner = sid(owner)?;
    if acl.is_null() || ![SYSTEM, ADMINS].contains(&owner.as_str()) && (state || owner != INSTALLER)
    {
        return Err(format!(
            "preservation state owner untrusted: {} {owner}",
            path.display()
        ));
    }
    let mut system = false;
    let mut admins = false;
    for index in 0..u32::from(unsafe { (*acl).AceCount }) {
        let mut pointer = std::ptr::null_mut();
        unsafe { GetAce(acl, index, &mut pointer) }.map_err(|e| e.to_string())?;
        if pointer.is_null() {
            return Err("preservation state ACE missing".into());
        }
        let header = unsafe { &*pointer.cast::<ACE_HEADER>() };
        if header.AceType != 0
            || usize::from(header.AceSize) < std::mem::size_of::<ACCESS_ALLOWED_ACE>()
        {
            return Err("preservation state ACE unsupported".into());
        }
        let ace = unsafe { &*pointer.cast::<ACCESS_ALLOWED_ACE>() };
        let who = sid(PSID((&ace.SidStart as *const u32).cast_mut().cast()))?;
        let flags = header.AceFlags & !0x10;
        if flags & !0x0f != 0 {
            return Err("preservation state ACE flags unknown".into());
        }
        if [SYSTEM, ADMINS, INSTALLER].contains(&who.as_str()) {
            if flags & 8 == 0 && ace.Mask & 0x1f01ff == 0x1f01ff {
                system |= who == SYSTEM;
                admins |= who == ADMINS;
            }
            continue;
        }
        if who == "S-1-3-0" && flags == 0x0b && ace.Mask == 0x10000000 {
            continue;
        }
        if who == "S-1-5-32-545" && flags == 2 && matches!(ace.Mask, 0x116 | 0x110) {
            continue;
        }
        if ace.Mask & !0x1200a9 == 0 && (!state || who == "S-1-5-32-545") {
            continue;
        }
        // C:\ permits new child directories and inherit-only user rights; neither replaces existing protected ancestors.
        if !state
            && path.parent().is_none()
            && who == "S-1-5-11"
            && ((flags == 0 && ace.Mask == 4) || (flags == 0x0b && ace.Mask == 0xe0010000))
        {
            continue;
        }
        return Err(format!(
            "preservation state untrusted directory rights: {}",
            path.display()
        ));
    }
    if !system || !admins {
        return Err("preservation state trusted directory access missing".into());
    }
    Ok(())
}

pub(super) fn verify(state: &Path, intent: &Path) -> Result<Vec<File>, String> {
    let fixed = Path::new(r"C:\ProgramData\RustDesk\ManagementAgent\state");
    #[cfg(not(test))]
    if state != fixed || intent != hbb_common::config::Config2::file() {
        return Err("preservation fixed state path mismatch".into());
    }
    #[cfg(test)]
    if state != fixed
        && state
            != Path::new(
                r"C:\ProgramData\RustDesk\ManagementAgent\preservation-state-read-fixture\state",
            )
    {
        // Existing isolated admission fixtures retain the strict workspace directory contract.
        workspace::verify(state)?;
        for file in [
            state.join("agent.json"),
            state.join("service-recovery-enabled"),
            intent.to_path_buf(),
        ] {
            workspace::verify_restore_target(&file, false)?;
        }
        return Ok(Vec::new());
    }
    let mut locks = Vec::new();
    for path in state.ancestors() {
        let metadata = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
        if !metadata.is_dir() || metadata.file_attributes() & 0x400 != 0 {
            return Err("preservation state ancestor reparse/non-directory".into());
        }
        locks.push(
            std::fs::OpenOptions::new()
                .read(true)
                .share_mode(3)
                .custom_flags(0x02000000 | 0x00200000)
                .open(path)
                .map_err(|e| e.to_string())?,
        );
        directory_acl(path, path == state)?;
    }
    for path in [
        state.join("agent.json"),
        state.join("service-recovery-enabled"),
        intent.to_path_buf(),
    ] {
        workspace::verify_restore_target(&path, path == intent)
            .map_err(|e| format!("preservation file {}: {e}", path.display()))?;
        locks.push(
            std::fs::OpenOptions::new()
                .read(true)
                .share_mode(1)
                .custom_flags(0x00200000)
                .open(path)
                .map_err(|e| e.to_string())?,
        );
    }
    match std::fs::symlink_metadata(state.join("maintenance.json")) {
        Ok(_) => {
            workspace::verify_file(&state.join("maintenance.json"))?;
            locks.push(
                std::fs::OpenOptions::new()
                    .read(true)
                    .share_mode(1)
                    .custom_flags(0x00200000)
                    .open(state.join("maintenance.json"))
                    .map_err(|e| e.to_string())?,
            );
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.to_string()),
    }
    Ok(locks)
}
