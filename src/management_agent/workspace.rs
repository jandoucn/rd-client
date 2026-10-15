use std::{
    os::windows::{ffi::OsStrExt, fs::MetadataExt},
    path::Path,
};
use windows::{
    core::{PCWSTR, PWSTR},
    Win32::{
        Foundation::{LocalFree, HLOCAL},
        Security::{
            Authorization::{
                ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
                GetNamedSecurityInfoW, SDDL_REVISION_1, SE_FILE_OBJECT,
            },
            GetAce, GetSecurityDescriptorControl, SetFileSecurityW, ACCESS_ALLOWED_ACE, ACE_HEADER,
            ACL, DACL_SECURITY_INFORMATION, OWNER_SECURITY_INFORMATION,
            PROTECTED_DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID, SE_DACL_PROTECTED,
        },
    },
};

struct Allocation(*mut std::ffi::c_void);
impl Drop for Allocation {
    fn drop(&mut self) {
        if !self.0.is_null() {
            let result = unsafe { LocalFree(Some(HLOCAL(self.0))) };
            if !result.0.is_null() {
                eprintln!("workspace security descriptor cleanup failed");
            }
        }
    }
}

fn local_path(path: &Path) -> Result<(), String> {
    if !path.is_absolute() {
        return Err("workspace path must be absolute".into());
    }
    for ancestor in path.ancestors() {
        let metadata = std::fs::symlink_metadata(ancestor)
            .map_err(|e| format!("workspace path unavailable: {e}"))?;
        if metadata.file_attributes() & 0x400 != 0 {
            return Err("workspace reparse path refused".into());
        }
    }
    Ok(())
}

fn wide(path: &Path) -> Vec<u16> {
    path.as_os_str().encode_wide().chain(Some(0)).collect()
}

fn sid_text(sid: PSID) -> Result<String, String> {
    if sid.0.is_null() {
        return Err("workspace owner/SID missing".into());
    }
    let mut string = PWSTR::null();
    unsafe { ConvertSidToStringSidW(sid, &mut string) }
        .map_err(|e| format!("workspace SID decode: {e}"))?;
    let _allocation = Allocation(string.0.cast());
    unsafe { string.to_string() }.map_err(|e| e.to_string())
}

fn verify_acl(path: &Path, require_protected: bool, installed_target: bool, local_service: bool) -> Result<(), String> {
    let path_wide = wide(path);
    let mut owner = PSID::default();
    let mut acl: *mut ACL = std::ptr::null_mut();
    let mut descriptor = PSECURITY_DESCRIPTOR::default();
    let status = unsafe {
        GetNamedSecurityInfoW(
            PCWSTR(path_wide.as_ptr()),
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
        return Err(format!("workspace security read: {}", status.0));
    }
    let _allocation = Allocation(descriptor.0);
    if acl.is_null() || descriptor.0.is_null() {
        return Err("workspace DACL missing".into());
    }
    if !["S-1-5-18", "S-1-5-32-544"].contains(&sid_text(owner)?.as_str()) {
        return Err("workspace owner not approved".into());
    }
    if require_protected {
        let mut control = 0u16;
        let mut revision = 0u32;
        unsafe { GetSecurityDescriptorControl(descriptor, &mut control, &mut revision) }
            .map_err(|e| e.to_string())?;
        if control & SE_DACL_PROTECTED.0 == 0 {
            return Err("workspace DACL inheritance not protected".into());
        }
    }
    let mut system = false;
    let mut administrators = false;
    let count = unsafe { (*acl).AceCount };
    for index in 0..u32::from(count) {
        let mut pointer = std::ptr::null_mut();
        unsafe { GetAce(acl, index, &mut pointer) }.map_err(|e| e.to_string())?;
        if pointer.is_null() {
            return Err("workspace ACE missing".into());
        }
        let header = unsafe { &*pointer.cast::<ACE_HEADER>() };
        if header.AceType != 0
            || usize::from(header.AceSize) < std::mem::size_of::<ACCESS_ALLOWED_ACE>()
        {
            return Err("workspace ACE type unsupported".into());
        }
        let allowed = unsafe { &*pointer.cast::<ACCESS_ALLOWED_ACE>() };
        let sid = PSID((&allowed.SidStart as *const u32).cast_mut().cast());
        let name = sid_text(sid)?;
        if !["S-1-5-18", "S-1-5-32-544"].contains(&name.as_str()) {
            if installed_target && ((local_service && name == "S-1-5-19")
                || allowed.Mask & !0x001200a9 == 0) {
                continue;
            }
            return Err("workspace unexpected permission principal".into());
        }
        if allowed.Mask & 0x001f01ff != 0x001f01ff || header.AceFlags & 0x08 != 0 {
            return Err("workspace required full access missing".into());
        }
        if name == "S-1-5-18" {
            system = true;
        } else {
            administrators = true;
        }
    }
    if !system || !administrators {
        return Err("workspace SYSTEM/Administrators access incomplete".into());
    }
    Ok(())
}

pub fn verify(path: &Path) -> Result<(), String> {
    local_path(path)?;
    if !std::fs::metadata(path).map_err(|e| e.to_string())?.is_dir() {
        return Err("workspace is not directory".into());
    }
    verify_acl(path, true, false, false)
}

pub fn verify_file(path: &Path) -> Result<(), String> {
    local_path(path)?;
    if !std::fs::metadata(path)
        .map_err(|e| e.to_string())?
        .is_file()
    {
        return Err("workspace asset is not file".into());
    }
    verify_acl(path, false, false, false)
}

pub fn verify_restore_target(path: &Path, identity_target: bool) -> Result<(), String> {
    local_path(path)?;
    if !std::fs::metadata(path).map_err(|e| e.to_string())?.is_file() {
        return Err("restore target is not a regular file".into());
    }
    verify_acl(path, false, true, identity_target)
}

pub(super) fn verify_agent_install_directory(path: &Path) -> Result<(), String> {
    if path != Path::new(r"C:\ProgramData\RustDesk\ManagementAgent") {
        return Err("agent install directory not fixed".into());
    }
    local_path(path)?;
    if !std::fs::metadata(path).map_err(|e| e.to_string())?.is_dir() {
        return Err("agent install path is not directory".into());
    }
    agent_directory_acl(path, false)
}

// Only the authorized local installation transaction may call this before publication.
pub(super) fn protect_agent_install_directory(
    evidence: &Path,
    before_write: impl Fn() -> Result<(), String>,
) -> Result<(), String> {
    protect_agent_directory_at(
        Path::new(r"C:\ProgramData\RustDesk\ManagementAgent"),
        evidence,
        &before_write,
    )
}

#[cfg(test)]
std::thread_local! { static AFTER_AGENT_SD_SAVE: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = std::cell::RefCell::new(None); }

#[cfg(test)]
pub fn agent_acl_revoke_fixture(
    evidence: &Path,
    after_save: impl FnOnce() + 'static,
    before_write: impl Fn() -> Result<(), String>,
) -> Result<(), String> {
    struct Clear;
    impl Drop for Clear {
        fn drop(&mut self) {
            AFTER_AGENT_SD_SAVE.with(|v| {
                v.borrow_mut().take();
            });
        }
    }
    AFTER_AGENT_SD_SAVE.with(|v| *v.borrow_mut() = Some(Box::new(after_save)));
    let _clear = Clear;
    protect_agent_directory_at(
        Path::new(r"C:\RustDeskAgentLab\total-loss-acl-fixture"),
        evidence,
        &before_write,
    )
}

fn protect_agent_directory_at(
    path: &Path,
    evidence: &Path,
    before_write: &dyn Fn() -> Result<(), String>,
) -> Result<(), String> {
    use std::io::Write;
    verify(evidence)?;
    let mut descriptor = PSECURITY_DESCRIPTOR::default();
    let target = wide(path);
    let status = unsafe {
        GetNamedSecurityInfoW(
            PCWSTR(target.as_ptr()),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            None,
            None,
            None,
            None,
            &mut descriptor,
        )
    };
    if status.0 != 0 {
        return Err(format!("agent original ACL read: {}", status.0));
    }
    let _allocation = Allocation(descriptor.0);
    #[link(name = "advapi32")]
    extern "system" {
        fn GetSecurityDescriptorLength(descriptor: *const std::ffi::c_void) -> u32;
    }
    let size = unsafe { GetSecurityDescriptorLength(descriptor.0) };
    if size == 0 || size > 65536 {
        return Err("agent original ACL size invalid".into());
    }
    let bytes = unsafe { std::slice::from_raw_parts(descriptor.0.cast::<u8>(), size as usize) };
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(evidence.join("agent-directory-original.sd"))
        .map_err(|e| e.to_string())?;
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(|e| e.to_string())?;
    #[cfg(test)]
    AFTER_AGENT_SD_SAVE.with(|v| {
        if let Some(hook) = v.borrow_mut().take() {
            hook();
        }
    });
    agent_directory_acl_checked(path, true, before_write)
}

#[cfg(test)]
pub fn protect_agent_directory_fixture() -> Result<(), String> {
    agent_directory_acl(
        Path::new(r"C:\RustDeskAgentLab\total-loss-acl-fixture"),
        true,
    )
}

#[cfg(test)]
pub fn agent_acl_bytes_fixture() -> Result<Vec<u8>, String> {
    let target = wide(Path::new(r"C:\RustDeskAgentLab\total-loss-acl-fixture"));
    let mut descriptor = PSECURITY_DESCRIPTOR::default();
    let status = unsafe {
        GetNamedSecurityInfoW(
            PCWSTR(target.as_ptr()),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            None,
            None,
            None,
            None,
            &mut descriptor,
        )
    };
    if status.0 != 0 {
        return Err(format!("fixture SD query: {}", status.0));
    }
    let _allocation = Allocation(descriptor.0);
    #[link(name = "advapi32")]
    extern "system" {
        fn GetSecurityDescriptorLength(descriptor: *const std::ffi::c_void) -> u32;
    }
    let size = unsafe { GetSecurityDescriptorLength(descriptor.0) };
    if size == 0 || size > 65536 {
        return Err("fixture SD length invalid".into());
    }
    Ok(unsafe { std::slice::from_raw_parts(descriptor.0.cast::<u8>(), size as usize) }.to_vec())
}

fn agent_directory_acl(path: &Path, repair: bool) -> Result<(), String> {
    agent_directory_acl_checked(path, repair, &|| Ok(()))
}

fn agent_directory_acl_checked(
    path: &Path,
    repair: bool,
    before_write: &dyn Fn() -> Result<(), String>,
) -> Result<(), String> {
    use std::os::windows::fs::OpenOptionsExt;
    local_path(path)?;
    let _directory = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(3)
        .custom_flags(0x02000000 | 0x00200000)
        .open(path)
        .map_err(|e| format!("agent directory identity lock: {e}"))?;
    let path_wide = wide(path);
    let mut owner = PSID::default();
    let mut acl: *mut ACL = std::ptr::null_mut();
    let mut descriptor = PSECURITY_DESCRIPTOR::default();
    let status = unsafe {
        GetNamedSecurityInfoW(
            PCWSTR(path_wide.as_ptr()),
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
        return Err(format!("agent directory ACL read: {}", status.0));
    }
    let _allocation = Allocation(descriptor.0);
    if acl.is_null() || !["S-1-5-18", "S-1-5-32-544"].contains(&sid_text(owner)?.as_str()) {
        return Err("agent directory owner or ACL invalid".into());
    }
    let mut system = false;
    let mut administrators = false;
    for index in 0..u32::from(unsafe { (*acl).AceCount }) {
        let mut pointer = std::ptr::null_mut();
        unsafe { GetAce(acl, index, &mut pointer) }.map_err(|e| e.to_string())?;
        if pointer.is_null() {
            return Err("agent directory ACE missing".into());
        }
        let ace = unsafe { &mut *pointer.cast::<ACCESS_ALLOWED_ACE>() };
        if ace.Header.AceType != 0
            || usize::from(ace.Header.AceSize) < std::mem::size_of::<ACCESS_ALLOWED_ACE>()
        {
            return Err("agent directory ACE unsupported".into());
        }
        let sid = sid_text(PSID((&ace.SidStart as *const u32).cast_mut().cast()))?;
        if sid == "S-1-3-0" && ace.Header.AceFlags & 0x08 != 0 {
            continue;
        }
        if ["S-1-5-18", "S-1-5-32-544"].contains(&sid.as_str()) {
            if ace.Mask & 0x1f01ff != 0x1f01ff || ace.Header.AceFlags & 8 != 0 {
                return Err("agent directory trusted rights incomplete".into());
            }
            system |= sid == "S-1-5-18";
            administrators |= sid == "S-1-5-32-544";
        } else if sid == "S-1-5-32-545"
            && ace.Mask == 0x116
            && ace.Header.AceFlags & !0x10 == 2
            && repair
        {
            ace.Mask &= !6;
        } else if ace.Mask & !0x1200a9 != 0
            && !(sid == "S-1-5-32-545" && ace.Mask == 0x110 && ace.Header.AceFlags & !0x10 == 2)
        {
            return Err("agent directory untrusted mutation rights".into());
        }
    }
    if !system || !administrators {
        return Err("agent directory trusted access missing".into());
    }
    if repair {
        #[link(name = "advapi32")]
        extern "system" {
            fn SetSecurityDescriptorControl(
                descriptor: *mut std::ffi::c_void,
                bits: u16,
                value: u16,
            ) -> i32;
        }
        if unsafe {
            SetSecurityDescriptorControl(descriptor.0, SE_DACL_PROTECTED.0, SE_DACL_PROTECTED.0)
        } == 0
        {
            return Err(format!(
                "agent directory protection flag: {}",
                std::io::Error::last_os_error()
            ));
        }
        before_write()?;
        unsafe {
            SetFileSecurityW(
                PCWSTR(path_wide.as_ptr()),
                DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
                descriptor,
            )
        }
        .ok()
        .map_err(|e| format!("agent directory ACL apply: {e}"))?;
        return agent_directory_acl(path, false);
    }
    let mut control = 0u16;
    let mut revision = 0u32;
    unsafe { GetSecurityDescriptorControl(descriptor, &mut control, &mut revision) }
        .map_err(|e| e.to_string())?;
    if control & SE_DACL_PROTECTED.0 == 0 {
        return Err("agent directory inheritance must be protected".into());
    }
    Ok(())
}

pub fn create(path: &Path) -> Result<(), String> {
    if path.exists() {
        return Err("workspace already exists; refuse reapproval".into());
    }
    local_path(path.parent().ok_or("workspace parent missing")?)?;
    std::fs::create_dir(path).map_err(|e| e.to_string())?;
    let sddl = "O:BAD:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)"
        .encode_utf16()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let mut descriptor = PSECURITY_DESCRIPTOR::default();
    unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            PCWSTR(sddl.as_ptr()),
            SDDL_REVISION_1,
            &mut descriptor,
            None,
        )
    }
    .map_err(|e| format!("workspace ACL construct: {e}"))?;
    let _allocation = Allocation(descriptor.0);
    let path_wide = wide(path);
    let applied = unsafe {
        SetFileSecurityW(
            PCWSTR(path_wide.as_ptr()),
            OWNER_SECURITY_INFORMATION
                | DACL_SECURITY_INFORMATION
                | PROTECTED_DACL_SECURITY_INFORMATION,
            descriptor,
        )
    };
    if !applied.as_bool() {
        return Err(format!(
            "workspace ACL apply: {}",
            std::io::Error::last_os_error()
        ));
    }
    verify(path)
}
