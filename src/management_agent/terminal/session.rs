use super::{OutputChunk, OutputRing, ShellCommand, TerminalError, TerminalSize};
use portable_pty::{Child, CommandBuilder, MasterPty, PtySize};
use std::{
    io::{ErrorKind, Read, Write},
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, MutexGuard, PoisonError,
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

pub(crate) const READ_BUFFER_BYTES: usize = 8 * 1024;
const READER_JOIN_TIMEOUT: Duration = Duration::from_secs(2);
/// shell 退出后 conhost 仍可能有没画到输出管道的尾部内容；输出连续静默这么久才关伪控制台。
const EXIT_OUTPUT_QUIET: Duration = Duration::from_millis(200);
const EXIT_OUTPUT_QUIET_MAX: Duration = Duration::from_secs(2);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Expiry {
    Idle,
    MaxLifetime,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SessionPolicy {
    pub idle: Duration,
    pub max_lifetime: Duration,
}

impl Default for SessionPolicy {
    fn default() -> Self {
        Self {
            idle: super::IDLE_TIMEOUT,
            max_lifetime: super::MAX_LIFETIME,
        }
    }
}

impl SessionPolicy {
    pub fn expiry(&self, started: Instant, last_activity: Instant, now: Instant) -> Option<Expiry> {
        if now.saturating_duration_since(started) >= self.max_lifetime {
            Some(Expiry::MaxLifetime)
        } else if now.saturating_duration_since(last_activity) >= self.idle {
            Some(Expiry::Idle)
        } else {
            None
        }
    }
}

/// 一个 PTY + 一棵 shell 进程树。输出由后台线程持续写入环形缓冲。
pub struct PtySession {
    master: Mutex<Option<Box<dyn MasterPty + Send>>>,
    writer: Mutex<Option<Box<dyn Write + Send>>>,
    child: Mutex<Box<dyn Child + Send + Sync>>,
    ring: Arc<Mutex<OutputRing>>,
    last_activity: Arc<Mutex<Instant>>,
    started: Instant,
    reader: Option<JoinHandle<()>>,
    reader_done: Arc<AtomicBool>,
    #[cfg(windows)]
    job: super::job_windows::KillOnCloseJob,
    closed: bool,
    output_settled: AtomicBool,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn pty_size(size: TerminalSize) -> PtySize {
    PtySize {
        rows: size.rows,
        cols: size.cols,
        pixel_width: 0,
        pixel_height: 0,
    }
}

fn start_failed(error: impl std::fmt::Display) -> TerminalError {
    TerminalError::StartFailed(error.to_string())
}

fn io_failed(error: impl std::fmt::Display) -> TerminalError {
    TerminalError::Io(error.to_string())
}

fn closed() -> TerminalError {
    TerminalError::Io("terminal session is closed".to_owned())
}

impl PtySession {
    /// 现有 ConPTY 实现。测试直接用它，避免在没有伪控制台的机器上被备选路径替换。
    pub fn open_conpty(
        command: &ShellCommand,
        cwd: Option<&Path>,
        size: TerminalSize,
        ring_capacity: usize,
    ) -> Result<Self, TerminalError> {
        let pair = portable_pty::native_pty_system()
            .openpty(pty_size(size))
            .map_err(start_failed)?;
        let mut builder = CommandBuilder::new(&command.program);
        builder.args(&command.args);
        if let Some(cwd) = cwd {
            builder.cwd(cwd);
        }
        let mut child = pair.slave.spawn_command(builder).map_err(start_failed)?;
        drop(pair.slave);
        match Self::attach(pair.master, &mut child, ring_capacity) {
            Ok(parts) => Ok(parts.into_session(child)),
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                Err(error)
            }
        }
    }

    pub fn spawn(
        command: &ShellCommand,
        cwd: Option<&Path>,
        size: TerminalSize,
        ring_capacity: usize,
    ) -> Result<Box<dyn super::TerminalSlot>, TerminalError> {
        // ConPTY 从 Windows 10 1809 才有。没有 CreatePseudoConsole 的系统走管道备选；
        // 有该 API 的系统仍只走现有实现，不经过备选。
        #[cfg(windows)]
        if !super::console_host::conpty_available() {
            return Ok(Box::new(super::console_host::PipeSession::spawn(
                command,
                cwd,
                size,
                ring_capacity,
            )?));
        }
        Ok(Box::new(Self::open_conpty(
            command,
            cwd,
            size,
            ring_capacity,
        )?))
    }

    fn attach(
        master: Box<dyn MasterPty + Send>,
        child: &mut Box<dyn Child + Send + Sync>,
        ring_capacity: usize,
    ) -> Result<Parts, TerminalError> {
        // portable-pty 不支持挂起创建，shell 启动到加入 Job 之间派生的进程（如启动参数里
        // 很快退出的 chcp）不在 Job 内；用户输入要等 writer 就绪后才进来，不受影响。
        #[cfg(windows)]
        let job = super::job_windows::KillOnCloseJob::adopt(child.as_ref())?;
        #[cfg(not(windows))]
        let _ = child;
        let reader = master.try_clone_reader().map_err(start_failed)?;
        let writer = master.take_writer().map_err(start_failed)?;
        let ring = Arc::new(Mutex::new(OutputRing::new(ring_capacity)));
        let last_activity = Arc::new(Mutex::new(Instant::now()));
        let reader_done = Arc::new(AtomicBool::new(false));
        let reader = spawn_reader(
            reader,
            ring.clone(),
            last_activity.clone(),
            reader_done.clone(),
        )?;
        Ok(Parts {
            master,
            writer,
            ring,
            last_activity,
            reader,
            reader_done,
            #[cfg(windows)]
            job,
        })
    }

    pub fn write(&self, data: &[u8]) -> Result<(), TerminalError> {
        let mut writer = lock(&self.writer);
        let writer = writer.as_mut().ok_or_else(closed)?;
        writer.write_all(data).map_err(io_failed)?;
        writer.flush().map_err(io_failed)?;
        *lock(&self.last_activity) = Instant::now();
        Ok(())
    }

    pub fn resize(&self, size: TerminalSize) -> Result<(), TerminalError> {
        let master = lock(&self.master);
        master
            .as_ref()
            .ok_or_else(closed)?
            .resize(pty_size(size))
            .map_err(io_failed)
    }

    pub fn read_since(
        &self,
        last_seq: u64,
        max_bytes: usize,
    ) -> Result<Vec<OutputChunk>, TerminalError> {
        lock(&self.ring).since(last_seq, max_bytes)
    }

    pub fn last_activity(&self) -> Instant {
        *lock(&self.last_activity)
    }

    pub fn exit_code(&self) -> Option<u32> {
        lock(&self.child)
            .try_wait()
            .ok()
            .flatten()
            .map(|status| status.exit_code())
    }

    /// shell 退出后收齐 PTY 里剩余的输出：先等输出静默（Windows 上 conhost 异步把屏幕画进
    /// 输出管道，立刻关伪控制台会丢掉最后一段），再关闭伪控制台让读端得到 EOF，最后等读线程
    /// 读完。阻塞至多 [`EXIT_OUTPUT_QUIET_MAX`] + [`READER_JOIN_TIMEOUT`]；可重复调用。
    pub fn finish_output(&self) {
        if self.output_settled.load(Ordering::SeqCst) {
            return;
        }
        let quiet_deadline = Instant::now() + EXIT_OUTPUT_QUIET_MAX;
        while lock(&self.last_activity).elapsed() < EXIT_OUTPUT_QUIET
            && Instant::now() < quiet_deadline
        {
            std::thread::sleep(Duration::from_millis(20));
        }
        lock(&self.writer).take();
        lock(&self.master).take();
        let deadline = Instant::now() + READER_JOIN_TIMEOUT;
        while !self.reader_done.load(Ordering::SeqCst) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        self.output_settled.store(true, Ordering::SeqCst);
    }

    pub fn output_settled(&self) -> bool {
        self.output_settled.load(Ordering::SeqCst)
    }

    pub fn process_id(&self) -> Option<u32> {
        lock(&self.child).process_id()
    }

    pub fn expiry(&self, policy: &SessionPolicy, now: Instant) -> Option<Expiry> {
        policy.expiry(self.started, *lock(&self.last_activity), now)
    }

    /// 结束整棵进程树并释放 PTY，返回 shell 的退出码（被强制结束时为终止码）。
    ///
    /// 阻塞调用（等待进程退出、关闭伪控制台、回收读线程）；丢弃会话同样会执行，
    /// 在异步上下文里必须放进 `spawn_blocking` 或专用线程。
    pub fn close(mut self) -> Result<Option<u32>, TerminalError> {
        self.shutdown()
    }

    fn shutdown(&mut self) -> Result<Option<u32>, TerminalError> {
        if self.closed {
            return Ok(None);
        }
        self.closed = true;
        let exit_code = {
            let mut child = lock(&self.child);
            let exited = matches!(child.try_wait(), Ok(Some(_)));
            #[cfg(windows)]
            self.job.terminate();
            if !exited {
                let _ = child.kill();
            }
            child.wait().map(|status| status.exit_code())
        };
        lock(&self.writer).take();
        lock(&self.master).take();
        self.join_reader();
        exit_code.map(Some).map_err(io_failed)
    }

    fn join_reader(&mut self) {
        let deadline = Instant::now() + READER_JOIN_TIMEOUT;
        while !self.reader_done.load(Ordering::SeqCst) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        if let Some(reader) = self.reader.take() {
            if self.reader_done.load(Ordering::SeqCst) {
                let _ = reader.join();
            } else {
                hbb_common::log::warn!("terminal output reader did not finish after close");
            }
        }
    }
}

impl Drop for PtySession {
    fn drop(&mut self) {
        if let Err(error) = self.shutdown() {
            hbb_common::log::warn!("terminal session teardown failed: {error}");
        }
    }
}

struct Parts {
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    ring: Arc<Mutex<OutputRing>>,
    last_activity: Arc<Mutex<Instant>>,
    reader: JoinHandle<()>,
    reader_done: Arc<AtomicBool>,
    #[cfg(windows)]
    job: super::job_windows::KillOnCloseJob,
}

impl Parts {
    fn into_session(self, child: Box<dyn Child + Send + Sync>) -> PtySession {
        let started = *lock(&self.last_activity);
        PtySession {
            master: Mutex::new(Some(self.master)),
            writer: Mutex::new(Some(self.writer)),
            child: Mutex::new(child),
            ring: self.ring,
            started,
            last_activity: self.last_activity,
            reader: Some(self.reader),
            reader_done: self.reader_done,
            #[cfg(windows)]
            job: self.job,
            closed: false,
            output_settled: AtomicBool::new(false),
        }
    }
}

fn spawn_reader(
    mut reader: Box<dyn Read + Send>,
    ring: Arc<Mutex<OutputRing>>,
    last_activity: Arc<Mutex<Instant>>,
    done: Arc<AtomicBool>,
) -> Result<JoinHandle<()>, TerminalError> {
    std::thread::Builder::new()
        .name("terminal-output".to_owned())
        .spawn(move || {
            let mut buffer = vec![0u8; READ_BUFFER_BYTES];
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(read) => {
                        lock(&ring).push(&buffer[..read]);
                        *lock(&last_activity) = Instant::now();
                    }
                    Err(error) if error.kind() == ErrorKind::Interrupted => continue,
                    Err(error) => {
                        hbb_common::log::warn!("terminal output reader stopped: {error}");
                        break;
                    }
                }
            }
            done.store(true, Ordering::SeqCst);
        })
        .map_err(start_failed)
}

impl super::TerminalSlot for PtySession {
    fn write(&self, data: &[u8]) -> Result<(), TerminalError> {
        PtySession::write(self, data)
    }
    fn resize(&self, size: TerminalSize) -> Result<(), TerminalError> {
        PtySession::resize(self, size)
    }
    fn read_since(
        &self,
        last_seq: u64,
        max_bytes: usize,
    ) -> Result<Vec<OutputChunk>, TerminalError> {
        PtySession::read_since(self, last_seq, max_bytes)
    }
    fn exit_code(&self) -> Option<u32> {
        PtySession::exit_code(self)
    }
    fn expiry(&self, policy: &SessionPolicy, now: Instant) -> Option<Expiry> {
        PtySession::expiry(self, policy, now)
    }
    fn close(self: Box<Self>) -> Result<Option<u32>, TerminalError> {
        (*self).close()
    }
    fn finish_output(&self) {
        PtySession::finish_output(self)
    }
    fn output_settled(&self) -> bool {
        PtySession::output_settled(self)
    }
}
