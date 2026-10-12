use serde::Deserialize;
use std::{io::Read, path::Path};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Marker {
    schema: u32,
    installation_id: String,
    transaction_id: String,
    plan_sha256: String,
    nonce: String,
}

#[derive(Deserialize)]
struct State {
    agent_installation_id: String,
    disabled: bool,
    management_agent_enabled: bool,
}

#[derive(Deserialize)]
struct Intent {
    #[serde(default)]
    options: std::collections::HashMap<String, String>,
}

fn read(path: &Path) -> Result<Vec<u8>, String> {
    use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
    if !path.is_absolute() { return Err("maintenance path invalid".into()); }
    for ancestor in path.ancestors() {
        let metadata = std::fs::symlink_metadata(ancestor).map_err(|_| "maintenance record unavailable")?;
        if metadata.file_attributes() & 0x400 != 0 { return Err("maintenance reparse path refused".into()); }
    }
    let file = std::fs::OpenOptions::new().read(true).share_mode(1).custom_flags(0x00200000)
        .open(path).map_err(|_| "maintenance record unreadable")?;
    let metadata = file.metadata().map_err(|_| "maintenance record unreadable")?;
    if !metadata.is_file() || metadata.file_attributes() & 0x400 != 0 {
        return Err("maintenance record not a regular file".into());
    }
    let mut bytes = Vec::new();
    file.take(65537).read_to_end(&mut bytes).map_err(|_| "maintenance record unreadable")?;
    if bytes.len() > 65536 { return Err("maintenance record too large".into()); }
    Ok(bytes)
}

pub fn verify(
    state: &Path,
    intent: &Path,
    installation: &str,
    transaction: &str,
    plan_sha256: &str,
    nonce: &str,
) -> Result<(), String> {
    for marker in ["disable-agent", "disable-recovery"] {
        match std::fs::symlink_metadata(state.join(marker)) {
            Ok(_) => return Err("installation locally blocked".into()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err("installation local gate unreadable".into()),
        }
    }
    let current: State = serde_json::from_slice(&read(&state.join("agent.json"))?)
        .map_err(|_| "installation state malformed")?;
    if current.disabled || !current.management_agent_enabled || current.agent_installation_id != installation {
        return Err("installation state disabled or identity changed".into());
    }
    let bytes = read(intent)?;
    let text = std::str::from_utf8(&bytes).map_err(|_| "installation intent encoding invalid")?;
    let policy: Intent = hbb_common::toml::from_str(text).map_err(|_| "installation intent malformed")?;
    if !matches!(policy.options.get("stop-service").map(String::as_str), None | Some("" | "N")) {
        return Err("installation user stop intent".into());
    }
    let path = state.join("maintenance.json");
    super::workspace::verify_file(&path)?;
    let marker: Marker = serde_json::from_slice(&read(&path)?).map_err(|_| "maintenance record malformed")?;
    if marker.schema != 1 || marker.installation_id != installation || marker.transaction_id != transaction
        || marker.plan_sha256 != plan_sha256 || marker.nonce != nonce {
        return Err("maintenance binding mismatch".into());
    }
    Ok(())
}

pub fn release_owned(state: &Path, installation: &str, transaction: &str, plan_sha256: &str, nonce: &str) -> Result<(), String> {
    use std::os::windows::{fs::{MetadataExt, OpenOptionsExt}, io::AsRawHandle};
    let path = state.join("maintenance.json");
    super::workspace::verify_file(&path)?;
    let mut file = std::fs::OpenOptions::new().access_mode(0x80000000 | 0x00010000)
        .share_mode(1).custom_flags(0x00200000).open(&path)
        .map_err(|error| format!("maintenance release lock: {error}"))?;
    let metadata = file.metadata().map_err(|error| error.to_string())?;
    if !metadata.is_file() || metadata.file_attributes() & 0x400 != 0 {
        return Err("maintenance record not a regular file".into());
    }
    let mut bytes = Vec::new();
    (&mut file).take(65537).read_to_end(&mut bytes).map_err(|error| error.to_string())?;
    if bytes.len() > 65536 { return Err("maintenance record too large".into()); }
    let marker: Marker = serde_json::from_slice(&bytes).map_err(|_| "maintenance record malformed")?;
    if marker.schema != 1 || marker.installation_id != installation || marker.transaction_id != transaction
        || marker.plan_sha256 != plan_sha256 || marker.nonce != nonce {
        return Err("maintenance binding mismatch".into());
    }
    #[repr(C)]
    struct Disposition { delete_file: u8 }
    #[link(name = "kernel32")]
    extern "system" {
        fn SetFileInformationByHandle(handle: *mut std::ffi::c_void, class: i32, information: *const std::ffi::c_void, size: u32) -> i32;
    }
    let disposition = Disposition { delete_file: 1 };
    if unsafe { SetFileInformationByHandle(file.as_raw_handle(), 4,
        (&disposition as *const Disposition).cast(), std::mem::size_of::<Disposition>() as u32) } == 0 {
        return Err(format!("maintenance release failed: {}", std::io::Error::last_os_error()));
    }
    Ok(())
}

pub fn claim(state: &Path, installation: &str, transaction: &str, plan_sha256: &str, nonce: &str) -> Result<(), String> {
    use std::{io::Write, os::windows::{ffi::OsStrExt, fs::MetadataExt, io::FromRawHandle}};
    let token = |value: &str, length: usize| value.len() == length
        && value.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
    if !token(installation,32) || !token(transaction,32) || !token(plan_sha256,64) || !token(nonce,64) {
        return Err("maintenance claim binding invalid".into());
    }
    if !state.is_absolute() { return Err("maintenance path invalid".into()); }
    for ancestor in state.ancestors() {
        let metadata = std::fs::symlink_metadata(ancestor).map_err(|_| "maintenance directory unavailable")?;
        if !metadata.is_dir() || metadata.file_attributes() & 0x400 != 0 {
            return Err("maintenance directory not a regular directory".into());
        }
    }
    #[repr(C)]
    struct Attributes { length:u32, descriptor:*mut std::ffi::c_void, inherit:i32 }
    #[link(name="advapi32")]
    extern "system" {
        fn ConvertStringSecurityDescriptorToSecurityDescriptorW(text:*const u16,revision:u32,descriptor:*mut *mut std::ffi::c_void,size:*mut u32)->i32;
    }
    #[link(name="kernel32")]
    extern "system" {
        fn CreateFileW(name:*const u16,access:u32,share:u32,attributes:*const Attributes,creation:u32,flags:u32,template:*mut std::ffi::c_void)->*mut std::ffi::c_void;
        fn LocalFree(memory:*mut std::ffi::c_void)->*mut std::ffi::c_void;
    }
    let sddl="O:BAD:P(A;;FA;;;SY)(A;;FA;;;BA)".encode_utf16().chain(Some(0)).collect::<Vec<_>>();
    let mut descriptor=std::ptr::null_mut();
    if unsafe{ConvertStringSecurityDescriptorToSecurityDescriptorW(sddl.as_ptr(),1,&mut descriptor,std::ptr::null_mut())}==0 {
        return Err(format!("maintenance security descriptor: {}",std::io::Error::last_os_error()));
    }
    let attributes=Attributes {length:std::mem::size_of::<Attributes>() as u32,descriptor,inherit:0};
    let path=state.join("maintenance.json");
    let wide=path.as_os_str().encode_wide().chain(Some(0)).collect::<Vec<_>>();
    let handle=unsafe{CreateFileW(wide.as_ptr(),0x40000000,0,&attributes,1,0x00200000,std::ptr::null_mut())};
    let error=std::io::Error::last_os_error();
    let freed=unsafe{LocalFree(descriptor)};
    if !freed.is_null() { eprintln!("maintenance descriptor cleanup failed"); }
    if handle as isize == -1 { return Err(format!("maintenance exclusive creation failed: {error}")); }
    let mut file=unsafe{std::fs::File::from_raw_handle(handle)};
    let bytes=serde_json::to_vec(&serde_json::json!({"schema":1,"installation_id":installation,
        "transaction_id":transaction,"plan_sha256":plan_sha256,"nonce":nonce})).map_err(|e|e.to_string())?;
    file.write_all(&bytes).and_then(|_|file.sync_all()).map_err(|e|format!("maintenance persistence failed; preserve marker: {e}"))?;
    drop(file);
    super::workspace::verify_file(&path)
}
