use std::{fs::{File, OpenOptions}, io::Write, os::windows::{ffi::OsStrExt, fs::{MetadataExt, OpenOptionsExt}, io::{AsRawHandle, FromRawHandle}}, path::Path};
use windows::{core::PWSTR, Win32::{Foundation::{HANDLE, HLOCAL, LocalFree}, Security::{Authorization::{ConvertSidToStringSidW, GetSecurityInfo, SE_FILE_OBJECT}, GetAce, ACCESS_ALLOWED_ACE, ACE_HEADER, ACL, DACL_SECURITY_INFORMATION, OWNER_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID}}};

pub struct HeldMarker {
    _directories: Vec<File>,
    _marker: File,
}

struct Allocation(*mut std::ffi::c_void);
impl Drop for Allocation {
    fn drop(&mut self) {
        if !self.0.is_null() && !unsafe { LocalFree(Some(HLOCAL(self.0))) }.0.is_null() {
            eprintln!("purge marker security allocation release failed");
        }
    }
}
fn sid(value: PSID) -> Result<String, String> {
    let mut text=PWSTR::null();
    unsafe { ConvertSidToStringSidW(value,&mut text) }.map_err(|e| e.to_string())?;
    let _text=Allocation(text.0.cast());
    unsafe { text.to_string() }.map_err(|e|e.to_string())
}

/// Drive-root rights that let users add children without touching existing ones; same set as
/// preservation_state_read. Windows client images grant them to Authenticated Users
/// ((A;;LC;;;AU), inherit-only modify), Windows Server images to Users ((A;CI;LC;;;BU) adds
/// subdirectories, (A;CIIO;DC;;;BU) only applies to new children). `flags` excludes INHERITED_ACE.
fn drive_root_ace_allowed(principal: &str, flags: u8, mask: u32) -> bool {
    match principal {
        "S-1-5-11" => (flags == 0 && mask == 4) || (flags == 0x0b && mask == 0xe0010000),
        "S-1-5-32-545" => (flags == 0x02 && mask == 4) || (flags == 0x0a && mask == 2),
        _ => false,
    }
}

// Same narrow inherited-directory grants as preservation_state_read, but
// checked on pinned handles. This authorizes creation only, never replacement.
fn acl(file: &File, directory: bool, volume_root: bool) -> Result<(), String> {
    let mut owner=PSID::default();let mut dacl: *mut ACL=std::ptr::null_mut();
    let mut descriptor=PSECURITY_DESCRIPTOR::default();
    let status=unsafe { GetSecurityInfo(HANDLE(file.as_raw_handle()),SE_FILE_OBJECT,
        OWNER_SECURITY_INFORMATION|DACL_SECURITY_INFORMATION,Some(&mut owner),None,Some(&mut dacl),None,Some(&mut descriptor)) };
    if status.0!=0 {return Err(format!("purge marker ACL query: {}",status.0));}
    let _descriptor=Allocation(descriptor.0);
    let trusted=["S-1-5-18","S-1-5-32-544"];
    let installer="S-1-5-80-956008885-3418522649-1831038044-1853292631-2271478464";
    let who=sid(owner)?;
    if dacl.is_null() || (!trusted.contains(&who.as_str()) && !(directory && who==installer)) {
        return Err("purge marker owner/DACL untrusted".into());
    }
    let mut full=[false;2];
    for index in 0..u32::from(unsafe {(*dacl).AceCount}) {
        let mut pointer=std::ptr::null_mut();
        unsafe {GetAce(dacl,index,&mut pointer)}.map_err(|e|e.to_string())?;
        if pointer.is_null() {return Err("purge marker ACE missing".into());}
        let header=unsafe {&*pointer.cast::<ACE_HEADER>()};
        if header.AceType!=0 || usize::from(header.AceSize)<std::mem::size_of::<ACCESS_ALLOWED_ACE>() {
            return Err("purge marker ACE unsupported".into());
        }
        let ace=unsafe {&*pointer.cast::<ACCESS_ALLOWED_ACE>()};
        let principal=sid(PSID((&ace.SidStart as *const u32).cast_mut().cast()))?;
        let flags=header.AceFlags & !0x10;
        if flags & !0x0f !=0 {return Err("purge marker ACE flags unknown".into());}
        if let Some(i)=trusted.iter().position(|v|*v==principal) {
            if flags & 8==0 && ace.Mask & 0x1f01ff==0x1f01ff {full[i]=true;}
            continue;
        }
        if directory {
            if principal==installer {continue;}
            if principal=="S-1-3-0" && flags==0x0b && ace.Mask==0x10000000 {continue;}
            // Create-child is not permission to replace an existing child. The
            // new marker receives an explicit protected descriptor atomically.
            if principal=="S-1-5-32-545" && flags==2 && matches!(ace.Mask,0x116|0x110) {continue;}
            if ace.Mask & !0x1200a9 ==0 {continue;}
            if volume_root && drive_root_ace_allowed(&principal,flags,ace.Mask) {continue;}
        }
        return Err("purge marker untrusted rights; replacement refused".into());
    }
    if full!=[true,true] {return Err("purge marker trusted full access missing".into());}
    Ok(())
}

fn pin_directories(state: &Path) -> Result<Vec<File>, String> {
    let mut directories=Vec::new();
    for path in state.ancestors().collect::<Vec<_>>().into_iter().rev() {
        let file=OpenOptions::new().read(true).share_mode(3).custom_flags(0x02000000|0x00200000)
            .open(path).map_err(|e|format!("purge marker ancestor open: {e}"))?;
        let metadata=file.metadata().map_err(|e|e.to_string())?;
        if !metadata.is_dir() || metadata.file_attributes() & 0x400!=0 {
            return Err("purge marker ancestor reparse/non-directory".into());
        }
        acl(&file,true,path.parent().is_none())?;
        directories.push(file);
    }
    Ok(directories)
}

pub fn create(recheck: impl Fn() -> Result<(),String>) -> Result<HeldMarker,String> {
    create_at(&super::system_dirs::get().agent().join("state"),recheck)
}

fn create_at(state: &Path, recheck: impl Fn() -> Result<(),String>) -> Result<HeldMarker,String> {
    let directories=pin_directories(state)?;
    recheck()?;
    let path=state.join("disable-recovery");
    match std::fs::symlink_metadata(&path) {
        Ok(_) => {
            let file=OpenOptions::new().read(true).share_mode(1).custom_flags(0x00200000).open(&path).map_err(|e|e.to_string())?;
            validate_marker(&file)?;
            recheck()?;
            return Ok(HeldMarker {_directories:directories,_marker:file});
        },
        Err(e) if matches!(e.raw_os_error(),Some(2|3)) => {},
        Err(e) => return Err(format!("purge marker observation unknown: {e}")),
    }
    #[repr(C)]
    struct Attributes {length:u32,descriptor:*mut std::ffi::c_void,inherit:i32}
    #[link(name="advapi32")]
    extern "system" {fn ConvertStringSecurityDescriptorToSecurityDescriptorW(text:*const u16,revision:u32,descriptor:*mut *mut std::ffi::c_void,size:*mut u32)->i32;}
    #[link(name="kernel32")]
    extern "system" {fn CreateFileW(name:*const u16,access:u32,share:u32,attributes:*const Attributes,creation:u32,flags:u32,template:*mut std::ffi::c_void)->*mut std::ffi::c_void;}
    let sddl="O:BAD:P(A;;FA;;;SY)(A;;FA;;;BA)".encode_utf16().chain(Some(0)).collect::<Vec<_>>();
    let mut descriptor=std::ptr::null_mut();
    if unsafe {ConvertStringSecurityDescriptorToSecurityDescriptorW(sddl.as_ptr(),1,&mut descriptor,std::ptr::null_mut())}==0 {
        return Err(format!("purge marker descriptor: {}",std::io::Error::last_os_error()));
    }
    let _descriptor=Allocation(descriptor);
    let attributes=Attributes {length:std::mem::size_of::<Attributes>() as u32,descriptor,inherit:0};
    let wide=path.as_os_str().encode_wide().chain(Some(0)).collect::<Vec<_>>();
    recheck()?;
    let handle=unsafe {CreateFileW(wide.as_ptr(),0xC0020000,1,&attributes,1,0x00200000,std::ptr::null_mut())};
    if handle as isize == -1 {return Err(format!("purge marker create_new refused: {}",std::io::Error::last_os_error()));}
    let mut file=unsafe {File::from_raw_handle(handle)};
    validate_marker(&file)?;
    file.write_all(b"disabled\n").and_then(|_|file.sync_all()).map_err(|e|format!("purge marker persist: {e}"))?;
    validate_marker(&file)?;
    Ok(HeldMarker {_directories:directories,_marker:file})
}

fn validate_marker(file:&File)->Result<(),String> {
    let metadata=file.metadata().map_err(|e|e.to_string())?;
    if !metadata.is_file() || metadata.file_attributes() & 0x400!=0 {return Err("purge marker reparse/non-file".into());}
    acl(file,false,false)
}

#[cfg(test)]
pub fn fixture(state:&Path,recheck:impl Fn()->Result<(),String>)->Result<HeldMarker,String> {create_at(state,recheck)}

#[cfg(test)]
mod drive_root_tests {
    use super::drive_root_ace_allowed;

    #[test]
    fn client_and_server_default_drive_roots_are_accepted() {
        assert!(drive_root_ace_allowed("S-1-5-11", 0x00, 0x4));
        assert!(drive_root_ace_allowed("S-1-5-11", 0x0b, 0xe0010000));
        assert!(drive_root_ace_allowed("S-1-5-32-545", 0x02, 0x4));
        assert!(drive_root_ace_allowed("S-1-5-32-545", 0x0a, 0x2));
    }

    // 实验室：RD_ACL_ROOT_LAB 指向一个用 subst 映射、权限与 Windows Server 默认 C:\ 相同的盘符根目录。
    #[test]
    fn a_server_default_drive_root_handle_is_trusted() {
        let Ok(root) = std::env::var("RD_ACL_ROOT_LAB") else {
            return;
        };
        use std::os::windows::fs::OpenOptionsExt;
        let file = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(3)
            .custom_flags(0x02000000 | 0x00200000)
            .open(&root)
            .unwrap();
        assert_eq!(super::acl(&file, true, true), Ok(()));
        assert!(super::acl(&file, true, false).is_err());
    }

    #[test]
    fn near_misses_are_rejected() {
        for (who, flags, mask) in [
            ("S-1-5-32-545", 0x03, 0x4),
            ("S-1-5-32-545", 0x06, 0x4),
            ("S-1-5-32-545", 0x0b, 0x2),
            ("S-1-5-32-545", 0x0e, 0x2),
            ("S-1-5-32-545", 0x02, 0x100004),
            ("S-1-5-32-545", 0x00, 0x4),
            ("S-1-5-11", 0x02, 0x4),
            ("S-1-1-0", 0x02, 0x4),
        ] {
            assert!(!drive_root_ace_allowed(who, flags, mask), "{who} {flags:#x} {mask:#x}");
        }
    }
}
