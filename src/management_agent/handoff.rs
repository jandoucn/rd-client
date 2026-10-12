use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Owner {
    pub pid: u32,
    pub created_at: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Handoff {
    pub schema: u32,
    pub transaction_id: String,
    pub plan_sha256: String,
    pub nonce: String,
    pub parent: Owner,
    pub helper: Owner,
    pub created_at: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadyReceipt {
    pub schema: u32,
    pub transaction_id: String,
    pub plan_sha256: String,
    pub nonce: String,
    pub helper: Owner,
    pub ready_at: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReleaseReceipt {
    pub schema: u32,
    pub transaction_id: String,
    pub plan_sha256: String,
    pub nonce: String,
    pub parent: Owner,
    pub helper: Owner,
    pub released_at: u64,
}

fn token(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

impl Handoff {
    pub fn release_receipt(
        &self,
        ready: &ReadyReceipt,
        now: u64,
    ) -> Result<ReleaseReceipt, String> {
        self.verify_ready(ready, now)?;
        Ok(ReleaseReceipt {
            schema: 1,
            transaction_id: self.transaction_id.clone(),
            plan_sha256: self.plan_sha256.clone(),
            nonce: self.nonce.clone(),
            parent: self.parent.clone(),
            helper: self.helper.clone(),
            released_at: now,
        })
    }

    pub fn verify_release(
        &self,
        ready: &ReadyReceipt,
        released: &ReleaseReceipt,
        observed_helper: &Owner,
        now: u64,
    ) -> Result<(), String> {
        self.verify_ready(ready, now)?;
        if released.schema != 1
            || released.transaction_id != self.transaction_id
            || released.plan_sha256 != self.plan_sha256
            || released.nonce != self.nonce
            || released.parent != self.parent
            || released.helper != self.helper
            || *observed_helper != self.helper
            || released.released_at < ready.ready_at
            || released.released_at > now
        {
            return Err("installation lock release binding mismatch".into());
        }
        Ok(())
    }

    pub fn new(
        transaction_id: String,
        plan_sha256: String,
        nonce: String,
        parent: Owner,
        helper: Owner,
        now: u64,
    ) -> Result<Self, String> {
        let value = Self {
            schema: 1,
            transaction_id,
            plan_sha256,
            nonce,
            parent,
            helper,
            created_at: now,
        };
        value.validate()?;
        Ok(value)
    }

    fn validate(&self) -> Result<(), String> {
        if self.schema != 1
            || !token(&self.transaction_id, 32)
            || !token(&self.plan_sha256, 64)
            || !token(&self.nonce, 64)
            || self.parent.pid == 0
            || self.helper.pid == 0
            || self.parent.pid == self.helper.pid
            || self.parent.created_at == 0
            || self.helper.created_at == 0
            || self.created_at == 0
        {
            return Err("installation handoff identity invalid".into());
        }
        Ok(())
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() > 4096 {
            return Err("installation handoff too large".into());
        }
        let value: Self = serde_json::from_slice(bytes)
            .map_err(|_| "installation handoff malformed".to_owned())?;
        value.validate()?;
        Ok(value)
    }

    pub fn ready_receipt(&self, now: u64) -> Result<ReadyReceipt, String> {
        self.validate()?;
        if now < self.created_at || now.saturating_sub(self.created_at) >= 300 {
            return Err("installation handoff readiness expired".into());
        }
        Ok(ReadyReceipt {
            schema: 1,
            transaction_id: self.transaction_id.clone(),
            plan_sha256: self.plan_sha256.clone(),
            nonce: self.nonce.clone(),
            helper: self.helper.clone(),
            ready_at: now,
        })
    }

    pub fn verify_ready(&self, receipt: &ReadyReceipt, now: u64) -> Result<(), String> {
        self.validate()?;
        if receipt.schema != 1
            || receipt.transaction_id != self.transaction_id
            || receipt.plan_sha256 != self.plan_sha256
            || receipt.nonce != self.nonce
            || receipt.helper != self.helper
            || receipt.ready_at < self.created_at
            || receipt.ready_at > now
            || now < self.created_at
            || now.saturating_sub(self.created_at) >= 300
        {
            return Err("installation helper readiness mismatch or expired".into());
        }
        Ok(())
    }

    pub fn may_take_over(
        &self,
        receipt: &ReadyReceipt,
        observed_helper: &Owner,
        parent_alive: bool,
        now: u64,
    ) -> Result<(), String> {
        self.verify_ready(receipt, now)?;
        if parent_alive || *observed_helper != self.helper {
            return Err("installation handoff owner not released or process changed".into());
        }
        Ok(())
    }
}

#[cfg(windows)]
pub struct ObservedProcess {
    handle: *mut std::ffi::c_void,
    owner: Owner,
}

#[cfg(windows)]
impl ObservedProcess {
    pub fn owner_alive(owner: &Owner) -> Result<bool, String> {
        if owner.pid == 0 || owner.created_at == 0 {
            return Err("installation process identity invalid".into());
        }
        #[link(name = "kernel32")]
        extern "system" {
            fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut std::ffi::c_void;
            fn CloseHandle(handle: *mut std::ffi::c_void) -> i32;
        }
        let handle = unsafe { OpenProcess(0x1000 | 0x00100000, 0, owner.pid) };
        if handle.is_null() {
            let error = std::io::Error::last_os_error();
            return if error.raw_os_error() == Some(87) { Ok(false) }
                else { Err(format!("installation previous owner inaccessible: {error}")) };
        }
        // Retain the first handle while reading identity so an exiting process
        // cannot have its PID reused between these observations.
        let observed = Self::open(owner.pid);
        let result = observed.and_then(|process| {
            if process.owner() != owner { Ok(false) } else { process.is_alive() }
        });
        if unsafe { CloseHandle(handle) } == 0 {
            return Err(format!("installation owner handle cleanup: {}", std::io::Error::last_os_error()));
        }
        result
    }

    pub fn open(pid: u32) -> Result<Self, String> {
        #[repr(C)]
        #[derive(Default)]
        struct FileTime {
            low: u32,
            high: u32,
        }
        #[link(name = "kernel32")]
        extern "system" {
            fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut std::ffi::c_void;
            fn GetProcessTimes(
                handle: *mut std::ffi::c_void,
                created: *mut FileTime,
                exited: *mut FileTime,
                kernel: *mut FileTime,
                user: *mut FileTime,
            ) -> i32;
            fn CloseHandle(handle: *mut std::ffi::c_void) -> i32;
        }
        if pid == 0 {
            return Err("installation process PID invalid".into());
        }
        let handle = unsafe { OpenProcess(0x1000 | 0x00100000, 0, pid) };
        if handle.is_null() {
            return Err(format!(
                "installation process open: {}",
                std::io::Error::last_os_error()
            ));
        }
        let mut created = FileTime::default();
        let mut exited = FileTime::default();
        let mut kernel = FileTime::default();
        let mut user = FileTime::default();
        if unsafe { GetProcessTimes(handle, &mut created, &mut exited, &mut kernel, &mut user) }
            == 0
        {
            let error = std::io::Error::last_os_error();
            unsafe { CloseHandle(handle) };
            return Err(format!("installation process creation time: {error}"));
        }
        // Creation FILETIME is an opaque identity token, separate from Unix deadlines.
        let created_at = (u64::from(created.high) << 32) | u64::from(created.low);
        Ok(Self {
            handle,
            owner: Owner { pid, created_at },
        })
    }

    pub fn owner(&self) -> &Owner {
        &self.owner
    }

    pub fn is_alive(&self) -> Result<bool, String> {
        #[link(name = "kernel32")]
        extern "system" {
            fn WaitForSingleObject(handle: *mut std::ffi::c_void, milliseconds: u32) -> u32;
        }
        match unsafe { WaitForSingleObject(self.handle, 0) } {
            0 => Ok(false),
            0x102 => Ok(true),
            _ => Err(format!(
                "installation process observation: {}",
                std::io::Error::last_os_error()
            )),
        }
    }
}

#[cfg(windows)]
impl Drop for ObservedProcess {
    fn drop(&mut self) {
        #[link(name = "kernel32")]
        extern "system" {
            fn CloseHandle(handle: *mut std::ffi::c_void) -> i32;
        }
        if unsafe { CloseHandle(self.handle) } == 0 {
            eprintln!(
                "installation process handle cleanup: {}",
                std::io::Error::last_os_error()
            );
        }
    }
}
