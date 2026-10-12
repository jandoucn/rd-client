use std::{ffi::c_void, io, ptr};

#[cfg(not(test))]
const MUTEX_NAME: &str = "Global\\RustDeskManagementAgentInstall";
#[cfg(test)]
const MUTEX_NAME: &str = "Global\\RustDeskAgentTestInstallGuard";

#[repr(C)]
struct SecurityAttributes {
    length: u32,
    descriptor: *mut c_void,
    inherit: i32,
}

#[link(name = "kernel32")]
extern "system" {
    fn CreateMutexW(
        attributes: *const SecurityAttributes,
        owner: i32,
        name: *const u16,
    ) -> *mut c_void;
    fn WaitForSingleObject(handle: *mut c_void, timeout: u32) -> u32;
    fn ReleaseMutex(handle: *mut c_void) -> i32;
    fn CloseHandle(handle: *mut c_void) -> i32;
    fn LocalFree(memory: *mut c_void) -> *mut c_void;
}

#[link(name = "advapi32")]
extern "system" {
    fn ConvertStringSecurityDescriptorToSecurityDescriptorW(
        value: *const u16,
        revision: u32,
        descriptor: *mut *mut c_void,
        size: *mut u32,
    ) -> i32;
}

pub struct InstallGuard {
    handle: *mut c_void,
    // Windows mutex ownership is bound to the acquiring thread.
    _thread_bound: std::marker::PhantomData<std::rc::Rc<()>>,
}

impl InstallGuard {
    pub fn try_acquire() -> io::Result<Option<Self>> {
        let sddl = "D:P(A;;GA;;;SY)(A;;GA;;;BA)"
            .encode_utf16()
            .chain(Some(0))
            .collect::<Vec<_>>();
        let name = MUTEX_NAME.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
        let mut descriptor = ptr::null_mut();
        unsafe {
            if ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                1,
                &mut descriptor,
                ptr::null_mut(),
            ) == 0
            {
                return Err(io::Error::last_os_error());
            }
            let attributes = SecurityAttributes {
                length: std::mem::size_of::<SecurityAttributes>() as u32,
                descriptor,
                inherit: 0,
            };
            let handle = CreateMutexW(&attributes, 0, name.as_ptr());
            let error = io::Error::last_os_error();
            LocalFree(descriptor);
            if handle.is_null() {
                return Err(error);
            }
            match WaitForSingleObject(handle, 0) {
                0 => Ok(Some(Self {
                    handle,
                    _thread_bound: std::marker::PhantomData,
                })),
                // An abandoned install is not permission to start another transaction.
                0x80 => {
                    ReleaseMutex(handle);
                    CloseHandle(handle);
                    Err(io::Error::other("agent install mutex abandoned"))
                }
                0x102 => {
                    CloseHandle(handle);
                    Ok(None)
                }
                _ => {
                    let error = io::Error::last_os_error();
                    CloseHandle(handle);
                    Err(error)
                }
            }
        }
    }
}

impl Drop for InstallGuard {
    fn drop(&mut self) {
        unsafe {
            if ReleaseMutex(self.handle) == 0 {
                eprintln!(
                    "agent install mutex release failed: {}",
                    io::Error::last_os_error()
                );
            }
            if CloseHandle(self.handle) == 0 {
                eprintln!(
                    "agent install mutex close failed: {}",
                    io::Error::last_os_error()
                );
            }
        }
    }
}
