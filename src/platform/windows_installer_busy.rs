//! Read-only Windows Installer observation; not a lock or an installation permit.
//! https://learn.microsoft.com/en-us/windows/win32/msi/-msiexecute-mutex
//! That page identifies SERVICE_ACCEPT_SHUTDOWN in dwControlsAccepted as the
//! Installer activity indication. Running alone does not establish activity.
//! QueryServiceStatusEx reports the last SCM update, which can lag reality:
//! https://learn.microsoft.com/en-us/windows/win32/api/winsvc/nf-winsvc-queryservicestatusex

use std::{ffi::c_void, io, mem::size_of, ptr};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Api {
    OpenManager,
    OpenService,
    QueryStatus,
    CloseService,
    CloseManager,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProbeError {
    pub api: Api,
    pub code: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnknownReason {
    Api(ProbeError),
    UnstableState(u32),
    InconsistentStatus,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Observation {
    Idle,
    Busy,
    Unknown(UnknownReason),
}

impl Observation {
    pub fn permits_attempt(self) -> bool {
        self == Self::Idle
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Status {
    pub service_type: u32,
    pub current_state: u32,
    pub controls_accepted: u32,
    pub win32_exit_code: u32,
    pub service_specific_exit_code: u32,
    pub check_point: u32,
    pub wait_hint: u32,
    pub process_id: u32,
    pub service_flags: u32,
}

pub fn classify(status: Result<Status, ProbeError>) -> Observation {
    let status = match status {
        Ok(status) => status,
        Err(error) => return Observation::Unknown(UnknownReason::Api(error)),
    };
    if !matches!(status.service_type, 0x10 | 0x20) {
        return Observation::Unknown(UnknownReason::InconsistentStatus);
    }
    match status.current_state {
        1 if status.controls_accepted == 0 => Observation::Idle,
        1 => Observation::Unknown(UnknownReason::InconsistentStatus),
        4 if status.process_id == 0 => Observation::Unknown(UnknownReason::InconsistentStatus),
        4 if status.controls_accepted & 0x0004 != 0 => Observation::Busy,
        4 => Observation::Idle,
        state => Observation::Unknown(UnknownReason::UnstableState(state)),
    }
}

pub fn probe() -> Observation {
    classify(query_status())
}

#[link(name = "advapi32")]
extern "system" {
    fn OpenSCManagerW(machine: *const u16, database: *const u16, access: u32) -> *mut c_void;
    fn OpenServiceW(manager: *mut c_void, name: *const u16, access: u32) -> *mut c_void;
    fn QueryServiceStatusEx(
        service: *mut c_void,
        level: u32,
        buffer: *mut u8,
        size: u32,
        needed: *mut u32,
    ) -> i32;
    fn CloseServiceHandle(handle: *mut c_void) -> i32;
}

fn last_error(api: Api) -> ProbeError {
    ProbeError {
        api,
        code: io::Error::last_os_error().raw_os_error().unwrap_or(0) as u32,
    }
}

struct Handle(*mut c_void, Api);

impl Handle {
    fn close(&mut self) -> Result<(), ProbeError> {
        if self.0.is_null() {
            return Ok(());
        }
        if unsafe { CloseServiceHandle(self.0) } == 0 {
            return Err(last_error(self.1));
        }
        self.0 = ptr::null_mut();
        Ok(())
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        if let Err(error) = self.close() {
            eprintln!("Windows Installer observation handle cleanup failed: {error:?}");
        }
    }
}

fn query_status() -> Result<Status, ProbeError> {
    // SC_MANAGER_CONNECT and SERVICE_QUERY_STATUS only; never start the service.
    let raw = unsafe { OpenSCManagerW(ptr::null(), ptr::null(), 0x0001) };
    if raw.is_null() {
        return Err(last_error(Api::OpenManager));
    }
    let mut manager = Handle(raw, Api::CloseManager);
    let name: Vec<u16> = "msiserver".encode_utf16().chain(Some(0)).collect();
    let raw = unsafe { OpenServiceW(manager.0, name.as_ptr(), 0x0004) };
    if raw.is_null() {
        return Err(last_error(Api::OpenService));
    }
    let mut service = Handle(raw, Api::CloseService);
    let mut status = Status::default();
    let mut needed = 0;
    if unsafe {
        QueryServiceStatusEx(
            service.0,
            0,
            (&mut status as *mut Status).cast(),
            size_of::<Status>() as u32,
            &mut needed,
        )
    } == 0
    {
        return Err(last_error(Api::QueryStatus));
    }
    service.close()?;
    manager.close()?;
    Ok(status)
}
