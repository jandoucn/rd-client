use super::handoff::ObservedProcess;
use hbb_common::tokio::{
    self,
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
};
use parity_tokio_ipc::{Endpoint, Incoming, SecurityAttributes};
use serde::{Deserialize, Serialize};
use std::{
    os::windows::io::AsRawHandle,
    path::Path,
    time::{Duration, Instant},
};

pub const PIPE: &str = r"\\.\pipe\RustDesk\management-registration-v1";
pub const ACTION: &str = "requestRestartRegistration";
#[cfg(not(test))]
#[path = "handoff.rs"]
mod handoff;
#[cfg(not(test))]
#[path = "health.rs"]
mod health;
#[cfg(not(test))]
#[path = "health_windows.rs"]
mod health_windows;
#[cfg(not(test))]
#[path = "../platform/windows_agent_guard.rs"]
mod install_guard;

#[path = "state_store.rs"]
mod budget_store;
#[path = "workspace.rs"]
mod budget_workspace;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct BudgetRecord {
    schema: u32,
    installation: String,
    attempts: u64,
    owner: Owner,
    budget: Budget,
}

fn budget_bytes(path: &Path) -> Result<Option<Vec<u8>>, String> {
    use std::io::Read;
    match std::fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.to_string()),
        Ok(_) => budget_workspace::verify_file(path)?,
    }
    let mut bytes = vec![];
    std::fs::File::open(path)
        .map_err(|e| e.to_string())?
        .take(4097)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > 4096 {
        return Err("R0 budget too large".into());
    }
    Ok(Some(bytes))
}

#[derive(Default)]
pub struct DurableCycle {
    watchdog: Watchdog,
}
impl DurableCycle {
    pub fn invalidate(&mut self) {
        self.watchdog.window = None;
    }

    pub fn step(
        &mut self,
        root: &Path,
        installation: &str,
        o: &Observation,
        mono: u64,
        wall: u64,
        probe: impl FnOnce() -> Result<bool, String>,
        recheck: impl FnOnce() -> Result<(), String>,
        action: impl FnOnce() -> Result<(), String>,
    ) -> Result<Decision, String> {
        let result = self.step_checked(root, installation, o, mono, wall, probe, recheck, action);
        if result.is_err() {
            self.invalidate();
        }
        result
    }

    fn step_checked(
        &mut self,
        root: &Path,
        installation: &str,
        o: &Observation,
        mono: u64,
        wall: u64,
        probe: impl FnOnce() -> Result<bool, String>,
        recheck: impl FnOnce() -> Result<(), String>,
        action: impl FnOnce() -> Result<(), String>,
    ) -> Result<Decision, String> {
        use std::os::windows::fs::OpenOptionsExt;
        if installation.len() != 32
            || !installation
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err("R0 installation invalid".into());
        }
        budget_workspace::verify(root)?;
        let lock_path = root.join("budget.lock");
        match std::fs::symlink_metadata(&lock_path) {
            Ok(_) => budget_workspace::verify_file(&lock_path)?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.to_string()),
        }
        let _writer = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .share_mode(0)
            .open(&lock_path)
            .map_err(|_| "R0 budget writer busy")?;
        budget_workspace::verify_file(&lock_path)?;
        let path = root.join("budget.json");
        let preimage = budget_bytes(&path)?;
        let record = preimage
            .as_ref()
            .map(|bytes| {
                serde_json::from_slice::<BudgetRecord>(bytes).map_err(|_| "R0 budget malformed")
            })
            .transpose()?;
        if let Some(r) = &record {
            if r.schema != 1
                || r.installation != installation
                || r.attempts == 0
                || r.owner.pid == 0
                || r.owner.created_at == 0
                || r.owner.build_seq == 0
                || r.budget.last_attempt == 0
                || r.budget.next_at != r.budget.last_attempt.saturating_add(3600)
            {
                return Err("R0 budget binding invalid".into());
            }
        }
        let mut budget = record
            .as_ref()
            .map(|r| r.budget.clone())
            .unwrap_or_default();
        // Establish continuity without DNS/IPC action on an ineligible tick.
        let pending = Observation {
            owner: o.owner.clone(),
            ready: o.ready,
            registered: o.registered,
            allowed: o.allowed,
            fixed_id_reachable: false,
        };
        let decision = self
            .watchdog
            .observe(&pending, mono, wall, &mut budget, |_| {
                Err("R0 premature persistence".into())
            })?;
        let due = matches!(decision, Decision::Wait)
            && self
                .watchdog
                .window
                .as_ref()
                .is_some_and(|(_, since, _)| mono.saturating_sub(*since) >= 300)
            && o.ready
            && !o.registered
            && o.allowed
            && !self.watchdog.attempted
            && wall >= budget.next_at
            && wall >= budget.last_attempt;
        if !due {
            return Ok(decision);
        }
        if !probe()? {
            return Ok(Decision::Wait);
        }
        recheck()?;
        let approved = Observation {
            fixed_id_reachable: true,
            ..pending
        };
        let attempts = record
            .as_ref()
            .map(|r| r.attempts)
            .unwrap_or(0)
            .checked_add(1)
            .ok_or("R0 attempt overflow")?;
        let result = self
            .watchdog
            .observe(&approved, mono, wall, &mut budget, |next| {
                if budget_bytes(&path)? != preimage {
                    return Err("R0 budget CAS conflict".into());
                }
                let next = BudgetRecord {
                    schema: 1,
                    installation: installation.into(),
                    attempts,
                    owner: o.owner.clone(),
                    budget: next.clone(),
                };
                budget_store::publish(
                    &path,
                    &serde_json::to_vec(&next).map_err(|e| e.to_string())?,
                )
                .map_err(|e| e.to_string())
            })?;
        if result == Decision::Request {
            action()?;
        }
        Ok(result)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Owner {
    pub pid: u32,
    pub created_at: u64,
    pub build_seq: u64,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub schema: u32,
    pub action: String,
    pub nonce: String,
    pub expected_owner: Owner,
}

pub fn dispatch(
    bytes: &[u8],
    owner: &Owner,
    callback: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    if bytes.is_empty() || bytes.len() > 4096 {
        return Err("registration packet length invalid".into());
    }
    let request: Request =
        serde_json::from_slice(bytes).map_err(|_| "registration request malformed")?;
    if request.schema != 1
        || request.action != ACTION
        || !nonce(&request.nonce)
        || &request.expected_owner != owner
        || owner.pid == 0
        || owner.created_at == 0
        || owner.build_seq == 0
    {
        return Err("registration request binding invalid".into());
    }
    callback()
}
fn nonce(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Budget {
    pub last_attempt: u64,
    pub next_at: u64,
}

pub struct Observation {
    pub owner: Owner,
    pub ready: bool,
    pub registered: bool,
    pub allowed: bool,
    pub fixed_id_reachable: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Decision {
    Observe,
    Wait,
    Request,
}

#[derive(Default)]
pub struct Watchdog {
    window: Option<(Owner, u64, u64)>,
    last_wall: Option<u64>,
    attempted: bool,
}
impl Watchdog {
    pub fn observe(
        &mut self,
        o: &Observation,
        monotonic: u64,
        wall: u64,
        budget: &mut Budget,
        persist: impl FnOnce(&Budget) -> Result<(), String>,
    ) -> Result<Decision, String> {
        let backwards = self.last_wall.is_some_and(|old| wall < old) || wall < budget.last_attempt;
        self.last_wall = Some(wall);
        if backwards || wall == 0 || budget.next_at < budget.last_attempt {
            self.window = None;
            return Ok(Decision::Wait);
        }
        if !o.ready
            || !o.allowed
            || o.owner.pid == 0
            || o.owner.created_at == 0
            || o.owner.build_seq == 0
        {
            self.window = None;
            return Ok(Decision::Observe);
        }
        if o.registered {
            self.window = None;
            self.attempted = false;
            return Ok(Decision::Observe);
        }
        let contiguous = self.window.as_ref().is_some_and(|(owner, _, last)| {
            owner == &o.owner && monotonic >= *last && monotonic - *last <= 60
        });
        if !contiguous {
            self.window = Some((o.owner.clone(), monotonic, monotonic));
            return Ok(Decision::Wait);
        }
        let Some((_, since, last)) = &mut self.window else {
            return Ok(Decision::Wait);
        };
        *last = monotonic;
        if monotonic - *since < 300
            || !o.fixed_id_reachable
            || self.attempted
            || wall < budget.next_at
        {
            return Ok(Decision::Wait);
        }
        let next = Budget {
            last_attempt: wall,
            next_at: wall.saturating_add(3600),
        };
        persist(&next)?;
        *budget = next;
        self.attempted = true;
        Ok(Decision::Request)
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Hello {
    schema: u32,
    owner: Owner,
    challenge: String,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Reply {
    pub schema: u32,
    pub nonce: String,
    pub owner: Owner,
    pub accepted: bool,
    pub registration_verified: bool,
    pub error: Option<String>,
}

pub fn bind() -> Result<Incoming, String> {
    bind_at(PIPE)
}
fn bind_at(path: &str) -> Result<Incoming, String> {
    let mut endpoint = Endpoint::new(path.into());
    endpoint.set_security_attributes(
        SecurityAttributes::from_sddl("D:P(A;;GA;;;SY)(A;;GA;;;BA)").map_err(|e| e.to_string())?,
    );
    endpoint.incoming().map_err(|e| e.to_string())
}
async fn receive(stream: &mut (impl AsyncRead + Unpin)) -> Result<Vec<u8>, String> {
    let count = stream.read_u32_le().await.map_err(|e| e.to_string())?;
    if count == 0 || count > 4096 {
        return Err("registration packet length invalid".into());
    }
    let mut bytes = vec![0; count as usize];
    stream
        .read_exact(&mut bytes)
        .await
        .map_err(|e| e.to_string())?;
    Ok(bytes)
}
async fn send(
    stream: &mut (impl AsyncWrite + Unpin),
    value: &impl Serialize,
) -> Result<(), String> {
    let bytes = serde_json::to_vec(value).map_err(|e| e.to_string())?;
    if bytes.len() > 4096 {
        return Err("registration response too large".into());
    }
    stream
        .write_u32_le(bytes.len() as u32)
        .await
        .map_err(|e| e.to_string())?;
    stream.write_all(&bytes).await.map_err(|e| e.to_string())?;
    stream.flush().await.map_err(|e| e.to_string())
}

pub async fn serve(
    listener: Incoming,
    build: u64,
    callback: impl FnMut() -> Result<(), String>,
) -> Result<(), String> {
    serve_bounded(listener, build, callback, None).await
}
async fn serve_bounded(
    mut listener: Incoming,
    build: u64,
    mut callback: impl FnMut() -> Result<(), String>,
    limit: Option<usize>,
) -> Result<(), String> {
    let owner = {
        let process = ObservedProcess::open(std::process::id())?;
        Owner {
            pid: process.owner().pid,
            created_at: process.owner().created_at,
            build_seq: build,
        }
    };
    if build == 0 {
        return Err("registration build unavailable".into());
    }
    hbb_common::sodiumoxide::init().map_err(|_| "registration crypto unavailable")?;
    let mut last_request: Option<Instant> = None;
    let mut count = 0;
    while let Some(connection) = listener.next().await {
        let mut stream = connection.map_err(|e| e.to_string())?;
        let challenge = hex::encode(hbb_common::sodiumoxide::randombytes::randombytes(32));
        let outcome = tokio::time::timeout(Duration::from_secs(2), async {
            send(
                &mut stream,
                &Hello {
                    schema: 1,
                    owner: owner.clone(),
                    challenge: challenge.clone(),
                },
            )
            .await?;
            let bytes = receive(&mut stream).await?;
            let request: Request =
                serde_json::from_slice(&bytes).map_err(|_| "registration request malformed")?;
            let result = dispatch(&bytes, &owner, || {
                if request.nonce != challenge {
                    return Err("registration challenge mismatch".into());
                }
                if last_request.is_some_and(|last| last.elapsed() < Duration::from_secs(300)) {
                    return Err("registration cooldown".into());
                }
                callback()?;
                last_request = Some(Instant::now());
                Ok(())
            });
            send(
                &mut stream,
                &Reply {
                    schema: 1,
                    nonce: request.nonce,
                    owner: owner.clone(),
                    accepted: result.is_ok(),
                    registration_verified: false,
                    error: result.err(),
                },
            )
            .await
        })
        .await;
        if !matches!(outcome, Ok(Ok(()))) {
            hbb_common::log::trace!("registration request refused or timed out");
        }
        count += 1;
        if limit.is_some_and(|limit| count >= limit) {
            return Ok(());
        }
    }
    Err("registration listener ended".into())
}

fn peer(handle: *mut std::ffi::c_void) -> Result<(ObservedProcess, std::path::PathBuf), String> {
    #[link(name = "kernel32")]
    extern "system" {
        fn GetNamedPipeServerProcessId(pipe: *mut std::ffi::c_void, pid: *mut u32) -> i32;
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut std::ffi::c_void;
        fn QueryFullProcessImageNameW(
            process: *mut std::ffi::c_void,
            flags: u32,
            name: *mut u16,
            count: *mut u32,
        ) -> i32;
        fn CloseHandle(handle: *mut std::ffi::c_void) -> i32;
    }
    let mut pid = 0;
    if unsafe { GetNamedPipeServerProcessId(handle, &mut pid) } == 0 {
        return Err("registration peer unavailable".into());
    }
    let process = ObservedProcess::open(pid)?;
    let handle = unsafe { OpenProcess(0x1000, 0, pid) };
    if handle.is_null() {
        return Err("registration peer inaccessible".into());
    }
    let mut buffer = vec![0u16; 32768];
    let mut count = buffer.len() as u32;
    let ok = unsafe { QueryFullProcessImageNameW(handle, 0, buffer.as_mut_ptr(), &mut count) };
    if unsafe { CloseHandle(handle) } == 0 {
        return Err("registration peer handle cleanup failed".into());
    }
    if ok == 0 {
        return Err("registration peer path unavailable".into());
    }
    let path = String::from_utf16(&buffer[..count as usize])
        .map_err(|_| "registration peer path encoding")?;
    Ok((process, path.into()))
}

pub async fn request_restart(expected_path: &Path, owner: &Owner) -> Result<Reply, String> {
    if ![
        r"C:\Program Files\RustDesk\RustDesk.exe",
        r"C:\ProgramData\RustDesk\Client\standard\RustDesk\RustDesk.exe",
        r"C:\ProgramData\RustDesk\Client\sos\RustDesk\RustDesk.exe",
    ]
    .iter()
    .any(|p| expected_path.to_string_lossy().eq_ignore_ascii_case(p))
    {
        return Err("registration target outside fixed paths".into());
    }
    request_at(PIPE, expected_path, owner).await
}
async fn request_at(pipe: &str, expected_path: &Path, owner: &Owner) -> Result<Reply, String> {
    tokio::time::timeout(Duration::from_secs(3), async {
        let mut stream = Endpoint::connect(pipe).await.map_err(|e| e.to_string())?;
        let (process, path) = peer(stream.as_raw_handle())?;
        let expected = std::fs::canonicalize(expected_path).map_err(|e| e.to_string())?;
        let actual = std::fs::canonicalize(path).map_err(|e| e.to_string())?;
        if !actual
            .to_string_lossy()
            .eq_ignore_ascii_case(&expected.to_string_lossy())
            || process.owner().pid != owner.pid
            || process.owner().created_at != owner.created_at
            || !process.is_alive()?
        {
            return Err("registration server identity mismatch".into());
        }
        let hello: Hello = serde_json::from_slice(&receive(&mut stream).await?)
            .map_err(|_| "registration handshake malformed")?;
        if hello.schema != 1
            || &hello.owner != owner
            || !nonce(&hello.challenge)
            || !process.is_alive()?
        {
            return Err("registration handshake binding mismatch".into());
        }
        let request = Request {
            schema: 1,
            action: ACTION.into(),
            nonce: hello.challenge,
            expected_owner: owner.clone(),
        };
        send(&mut stream, &request).await?;
        let reply: Reply = serde_json::from_slice(&receive(&mut stream).await?)
            .map_err(|_| "registration reply malformed")?;
        if reply.schema != 1
            || reply.nonce != request.nonce
            || &reply.owner != owner
            || reply.registration_verified
            || reply.accepted == reply.error.is_some()
            || !process.is_alive()?
        {
            return Err("registration reply binding mismatch".into());
        }
        Ok(reply)
    })
    .await
    .map_err(|_| "registration timeout".to_owned())?
}

#[cfg(test)]
pub fn bind_fixture(path: &str) -> Result<Incoming, String> {
    if !path.starts_with(r"\\.\pipe\RustDeskRegistrationTest-") {
        return Err("fixture pipe refused".into());
    }
    bind_at(path)
}
#[cfg(test)]
pub async fn serve_fixture(
    listener: Incoming,
    build: u64,
    callback: impl FnMut() -> Result<(), String>,
    connections: usize,
) -> Result<(), String> {
    serve_bounded(listener, build, callback, Some(connections)).await
}
#[cfg(test)]
pub async fn request_fixture(pipe: &str, path: &Path, owner: &Owner) -> Result<Reply, String> {
    request_at(pipe, path, owner).await
}

pub struct Tick {
    pub installation: String,
    queued: Instant,
}
pub struct Worker {
    sender: std::sync::mpsc::SyncSender<Tick>,
}
impl Worker {
    pub fn notify(&self, installation: &str) -> Result<(), String> {
        match self.sender.try_send(Tick {
            installation: installation.into(),
            queued: Instant::now(),
        }) {
            Ok(()) | Err(std::sync::mpsc::TrySendError::Full(_)) => Ok(()),
            Err(_) => Err("R0 worker unavailable".into()),
        }
    }
}
fn spawn_worker(
    mut step: impl FnMut(Tick) -> Result<(), String> + Send + 'static,
) -> std::io::Result<Worker> {
    let (sender, receiver) = std::sync::mpsc::sync_channel::<Tick>(1);
    std::thread::Builder::new()
        .name("agent-registration-r0".into())
        .spawn(move || {
            for tick in receiver {
                if tick.queued.elapsed() > Duration::from_secs(30) {
                    continue;
                }
                if let Err(error) = step(tick) {
                    hbb_common::throttled_log!(
                        Duration::from_secs(300),
                        warn,
                        "R0 observation/action refused: {error}"
                    );
                }
            }
        })?;
    Ok(Worker { sender })
}
#[cfg(test)]
pub fn worker_fixture(
    step: impl FnMut(Tick) -> Result<(), String> + Send + 'static,
) -> std::io::Result<Worker> {
    spawn_worker(step)
}

#[cfg(not(test))]
pub fn start() -> std::io::Result<Worker> {
    let mut cycle = DurableCycle::default();
    let started = Instant::now();
    // This dedicated synchronous worker owns its runtime; none is nested in async code.
    let mut runtime = None;
    spawn_worker(move |tick| {
        let result = (|| {
            if runtime.is_none() {
                runtime = Some(
                    tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .map_err(|e| e.to_string())?,
                );
            }
            let rt = runtime.as_ref().ok_or("R0 runtime unavailable")?;
            let (path, observation) = production::observe(&tick.installation)?;
            let root = Path::new(r"C:\ProgramData\RustDesk\ManagementAgent\registration-reconnect");
            if !root.exists() {
                budget_workspace::create(root)?;
            }
            let wall = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|_| "R0 wall clock invalid")?
                .as_secs();
            cycle.step(
                root,
                &tick.installation,
                &observation,
                started.elapsed().as_secs(),
                wall,
                || production::id_reachable(),
                || production::recheck(&tick.installation, &path, &observation.owner),
                || {
                    production::recheck(&tick.installation, &path, &observation.owner)?;
                    let reply = rt.block_on(request_restart(&path, &observation.owner))?;
                    if !reply.accepted {
                        return Err(reply.error.unwrap_or_else(|| "R0 request refused".into()));
                    }
                    Ok(())
                },
            )?;
            Ok(())
        })();
        if result.is_err() {
            cycle.invalidate();
        }
        result
    })
}
#[cfg(test)]
pub fn start() -> std::io::Result<Worker> {
    spawn_worker(|_| Err("R0 production worker disabled in tests".into()))
}

#[cfg(not(test))]
mod production {
    use super::*;
    use std::{io::Read, path::PathBuf};
    use windows_service::{
        service::{ServiceAccess, ServiceStartType, ServiceState, ServiceType},
        service_manager::{ServiceManager, ServiceManagerAccess},
    };

    fn read(path: &Path, identity: bool) -> Result<Vec<u8>, String> {
        budget_workspace::verify_restore_target(path, identity)?;
        let mut bytes = vec![];
        std::fs::File::open(path)
            .map_err(|e| e.to_string())?
            .take(65537)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        if bytes.len() > 65536 {
            return Err("R0 local record too large".into());
        }
        Ok(bytes)
    }
    fn gate(installation: &str) -> Result<(PathBuf, u64), String> {
        let _guard = install_guard::InstallGuard::try_acquire()
            .map_err(|e| e.to_string())?
            .ok_or("R0 installation busy")?;
        let state = Path::new(r"C:\ProgramData\RustDesk\ManagementAgent\state");
        for marker in ["disable-agent", "disable-recovery", "maintenance.json"] {
            match std::fs::symlink_metadata(state.join(marker)) {
                Ok(_) => return Err("R0 locally blocked".into()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e.to_string()),
            }
        }
        budget_workspace::verify_restore_target(&state.join("service-recovery-enabled"), false)?;
        let value: serde_json::Value =
            serde_json::from_slice(&read(&state.join("agent.json"), false)?)
                .map_err(|_| "R0 state malformed")?;
        if value["agent_installation_id"] != installation
            || value["disabled"] != false
            || value["management_agent_enabled"] != true
        {
            return Err("R0 state authorization changed".into());
        }
        let config = hbb_common::config::Config2::file();
        if ![r"C:\Windows\ServiceProfiles\LocalService\AppData\Roaming\RustDesk\config\RustDesk2.toml",r"C:\Windows\System32\config\systemprofile\AppData\Roaming\RustDesk\config\RustDesk2.toml"].iter().any(|p|config.to_string_lossy().eq_ignore_ascii_case(p)){return Err("R0 config path refused".into());}
        let bytes = read(&config, true)?;
        let text = std::str::from_utf8(&bytes).map_err(|_| "R0 intent encoding")?;
        let intent: hbb_common::toml::Value =
            hbb_common::toml::from_str(text).map_err(|_| "R0 intent malformed")?;
        let stop_service = intent
            .get("options")
            .and_then(|v| v.get("stop-service"))
            .and_then(|v| v.as_str());
        if !base::stop_service_intent_enabled(stop_service) {
            return Err("R0 stopped or unknown intent".into());
        }
        let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)
            .map_err(|e| e.to_string())?;
        let service = manager
            .open_service(
                "RustDesk",
                ServiceAccess::QUERY_CONFIG | ServiceAccess::QUERY_STATUS,
            )
            .map_err(|e| e.to_string())?;
        let config = service.query_config().map_err(|e| e.to_string())?;
        let status = service.query_status().map_err(|e| e.to_string())?;
        if config.service_type != ServiceType::OWN_PROCESS
            || !matches!(
                config.start_type,
                ServiceStartType::AutoStart | ServiceStartType::OnDemand
            )
            || status.current_state != ServiceState::Running
            || status.process_id.is_none()
            || !config
                .account_name
                .is_some_and(|v| v.to_string_lossy().eq_ignore_ascii_case("LocalSystem"))
        {
            return Err("R0 service not eligible".into());
        }
        let command = config.executable_path.to_string_lossy();
        let path = [
            r"C:\Program Files\RustDesk\RustDesk.exe",
            r"C:\ProgramData\RustDesk\Client\standard\RustDesk\RustDesk.exe",
            r"C:\ProgramData\RustDesk\Client\sos\RustDesk\RustDesk.exe",
        ]
        .iter()
        .find(|p| command.eq_ignore_ascii_case(&format!("\"{p}\" --service")))
        .ok_or("R0 service path refused")?;
        budget_workspace::verify_restore_target(Path::new(path), false)?;
        let metadata: serde_json::Value = serde_json::from_str(include_str!("../../version.json"))
            .map_err(|_| "R0 build metadata")?;
        Ok((
            PathBuf::from(path),
            metadata["build_seq"].as_u64().ok_or("R0 build absent")?,
        ))
    }
    pub fn observe(installation: &str) -> Result<(PathBuf, Observation), String> {
        let (path, build) = gate(installation)?;
        let health = health_windows::probe_blocking(
            r"\\.\pipe\RustDesk\management-health-v2",
            &path,
            build,
            Duration::from_secs(3),
        )?;
        Ok((
            path,
            Observation {
                owner: Owner {
                    pid: health.pid,
                    created_at: health.process_created_at,
                    build_seq: health.build_seq,
                },
                ready: health.local_responsive,
                registered: health.registered,
                allowed: health.no_active_sessions == Some(true),
                fixed_id_reachable: false,
            },
        ))
    }
    pub fn recheck(installation: &str, path: &Path, owner: &Owner) -> Result<(), String> {
        let (actual, o) = observe(installation)?;
        if actual != path || &o.owner != owner || !o.ready || o.registered || !o.allowed {
            return Err("R0 observation changed before request".into());
        }
        Ok(())
    }
    pub fn id_reachable() -> Result<bool, String> {
        use std::{
            net::{TcpStream, ToSocketAddrs},
            sync::{
                atomic::{AtomicBool, Ordering},
                mpsc,
            },
        };
        static RESOLVING: AtomicBool = AtomicBool::new(false);
        if RESOLVING
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Ok(false);
        }
        let (tx, rx) = mpsc::sync_channel(1);
        let started = Instant::now();
        if let Err(error) = std::thread::Builder::new()
            .name("agent-r0-fixed-dns".into())
            .spawn(move || {
                let result = ("rd.yan.life", 21116)
                    .to_socket_addrs()
                    .map(|v| v.take(8).collect::<Vec<_>>());
                RESOLVING.store(false, Ordering::Release);
                let _ = tx.try_send(result);
            })
        {
            RESOLVING.store(false, Ordering::Release);
            return Err(error.to_string());
        }
        let addresses = match rx.recv_timeout(Duration::from_secs(3)) {
            Ok(Ok(v)) => v,
            _ => return Ok(false),
        };
        for address in addresses {
            let remaining = Duration::from_secs(3).saturating_sub(started.elapsed());
            if remaining.is_zero() {
                return Ok(false);
            }
            if TcpStream::connect_timeout(&address, remaining.min(Duration::from_secs(1))).is_ok() {
                return Ok(true);
            }
        }
        Ok(false)
    }
}
