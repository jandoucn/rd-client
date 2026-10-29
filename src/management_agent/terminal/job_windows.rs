use super::TerminalError;
use windows::Win32::{
    Foundation::{CloseHandle, HANDLE},
    System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
        SetInformationJobObject, TerminateJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    },
};

/// shell 及其派生的全部子进程都归入此 Job；关闭会话或句柄时整棵树一起结束。
pub struct KillOnCloseJob(HANDLE);

// Job 句柄只用于 Terminate/Close，跨线程使用安全。
unsafe impl Send for KillOnCloseJob {}
unsafe impl Sync for KillOnCloseJob {}

impl KillOnCloseJob {
    pub fn adopt_handle(process: std::os::windows::io::RawHandle) -> Result<Self, TerminalError> {
        let job = unsafe { CreateJobObjectW(None, windows::core::PCWSTR::null()) }
            .map_err(|error| TerminalError::StartFailed(format!("create job: {error}")))?;
        let job = Self(job);
        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        unsafe {
            SetInformationJobObject(
                job.0,
                JobObjectExtendedLimitInformation,
                &limits as *const _ as *const core::ffi::c_void,
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        }
        .map_err(|error| TerminalError::StartFailed(format!("configure job: {error}")))?;
        unsafe { AssignProcessToJobObject(job.0, HANDLE(process as _)) }
            .map_err(|error| TerminalError::StartFailed(format!("assign job: {error}")))?;
        Ok(job)
    }

    pub fn adopt(child: &(dyn portable_pty::Child + Send + Sync)) -> Result<Self, TerminalError> {
        let process = child
            .as_raw_handle()
            .ok_or_else(|| TerminalError::StartFailed("shell process handle unavailable".into()))?;
        Self::adopt_handle(process)
    }

    pub fn terminate(&self) {
        if let Err(error) = unsafe { TerminateJobObject(self.0, 1) } {
            hbb_common::log::warn!("terminate terminal job failed: {error}");
        }
    }
}

impl Drop for KillOnCloseJob {
    fn drop(&mut self) {
        let _ = unsafe { CloseHandle(self.0) };
    }
}
