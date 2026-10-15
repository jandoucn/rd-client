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
            if volume_root && principal=="S-1-5-11" && ((flags==0 && ace.Mask==4)||(flags==0x0b && ace.Mask==0xe0010000)) {continue;}
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
    create_at(Path::new(r"C:\ProgramData\RustDesk\ManagementAgent\state"),recheck)
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
