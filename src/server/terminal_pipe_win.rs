//! 没有 ConPTY 的 Windows 上客户端远程终端的管道 shell。
//!
//! 用匿名管道接 shell 的标准输入输出，对外给出与 portable-pty 相同的读端、写端和子进程，
//! 让终端会话其余逻辑不必区分承载方式。实测结论与 Agent 的
//! `src/management_agent/terminal/console_host.rs` 一致：句柄要显式可继承、cmd 本地行编辑、
//! 回车补 CRLF、按系统 ANSI 代码页与 UTF-8 双向转码。没有窗口尺寸，全屏 TUI 不可用。
//!
//! 写端被释放即结束整棵 shell 进程树：会话关闭时先 join 读线程，管道下 shell 不会自己退出，
//! 只有这样读端才能拿到 EOF。

use super::terminal_pipe::{
    complete_dbcs_prefix, normalize_enter, pipe_shell_args, temp_dir_override, CrlfNormalizer,
    LineEditor, Utf8Carry,
};
use hbb_common::{
    anyhow::{anyhow, Context, Result},
    log,
};
use portable_pty::{Child, ChildKiller, CommandBuilder, ExitStatus};
use std::{
    collections::BTreeMap,
    ffi::{OsStr, OsString},
    fs::File,
    io::{Read, Write},
    os::windows::{
        ffi::OsStrExt,
        io::{AsRawHandle, FromRawHandle, OwnedHandle, RawHandle},
    },
    path::Path,
    sync::{
        mpsc::{channel, Receiver, Sender},
        Arc,
    },
};
use windows::{
    core::{PCSTR, PCWSTR, PWSTR},
    Win32::{
        Foundation::{
            CloseHandle, SetHandleInformation, HANDLE, HANDLE_FLAG_INHERIT, STILL_ACTIVE,
        },
        Globalization::{
            GetACP, IsDBCSLeadByteEx, MultiByteToWideChar, WideCharToMultiByte,
            MULTI_BYTE_TO_WIDE_CHAR_FLAGS,
        },
        System::{
            JobObjects::{
                AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
                SetInformationJobObject, TerminateJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
                JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            },
            Pipes::CreatePipe,
            Threading::{
                CreateProcessW, DeleteProcThreadAttributeList, GetExitCodeProcess,
                InitializeProcThreadAttributeList, ResumeThread, TerminateProcess,
                UpdateProcThreadAttribute, WaitForSingleObject, CREATE_NO_WINDOW, CREATE_SUSPENDED,
                CREATE_UNICODE_ENVIRONMENT, EXTENDED_STARTUPINFO_PRESENT, INFINITE,
                LPPROC_THREAD_ATTRIBUTE_LIST, PROCESS_INFORMATION,
                PROC_THREAD_ATTRIBUTE_HANDLE_LIST, STARTF_USESTDHANDLES, STARTUPINFOEXW,
                STARTUPINFOW,
            },
        },
    },
};

const READ_BUFFER_BYTES: usize = 8 * 1024;
const CP_UTF8: u32 = 65001;

/// ConPTY 能否使用，与 portable-pty 打开伪终端时的判定一致：kernel32 必须导出
/// `CreatePseudoConsole`（Windows 10 1809 起才有），之后才会考虑随程序附带的 `conpty.dll`。
pub fn conpty_available() -> bool {
    portable_pty::win::check_support().is_ok()
}

/// shell 将拿到的 TEMP 目录不存在时，把 TEMP/TMP 换成系统 Temp，见 [`temp_dir_override`]。
///
/// 必须看 `CommandBuilder` 里的值：它会用注册表 `HKCU\Environment` 覆盖进程环境，
/// SYSTEM 下展开后是不存在的 `systemprofile\AppData\Local\Temp`，而服务进程自己的 TEMP 是好的。
pub fn apply_temp_override(cmd: &mut CommandBuilder) {
    let Some(temp) =
        temp_dir_override(cmd.get_env("TEMP"), cmd.get_env("SystemRoot"), Path::exists)
    else {
        return;
    };
    cmd.env("TEMP", &temp);
    cmd.env("TMP", &temp);
}

/// 与 ConPTY 路径同一来源的 shell 环境：`CommandBuilder` 合并注册表后的完整变量表。
pub fn shell_environment(cmd: &CommandBuilder) -> Vec<(OsString, OsString)> {
    cmd.iter_full_env_as_str()
        .map(|(name, value)| (OsString::from(name), OsString::from(value)))
        .collect()
}

/// 管道上运行的 shell，三部分分别交给会话的写线程、读线程与子进程管理。
pub struct PipeShell {
    pub reader: PipeReader,
    pub writer: PipeWriter,
    pub child: PipeChild,
}

impl PipeShell {
    /// `environment` 为 `None` 时继承当前进程环境，否则用它作为 shell 的完整环境。
    pub fn spawn(shell: &str, environment: Option<&[(OsString, OsString)]>) -> Result<Self> {
        let shell_args = pipe_shell_args(shell);
        let (stdin_read, stdin_write) = pipe_pair()?;
        let (stdout_read, stdout_write) = pipe_pair()?;
        // stderr 与 stdout 共用写端；句柄列表里同一句柄只能出现一次。
        let child_stdin = raw_handle(stdin_read.as_raw_handle());
        let child_stdout = raw_handle(stdout_write.as_raw_handle());
        make_inheritable(child_stdin)?;
        make_inheritable(child_stdout)?;

        let mut command_line = Vec::new();
        push_quoted(&mut command_line, OsStr::new(shell));
        for arg in &shell_args.args {
            command_line.push(b' ' as u16);
            push_quoted(&mut command_line, OsStr::new(arg));
        }
        command_line.push(0);

        let mut handles = [child_stdin, child_stdout];
        let mut attribute_list = AttributeList::new(1)?;
        attribute_list.set_handles(&mut handles)?;
        let startup = STARTUPINFOEXW {
            StartupInfo: STARTUPINFOW {
                cb: std::mem::size_of::<STARTUPINFOEXW>() as u32,
                dwFlags: STARTF_USESTDHANDLES,
                hStdInput: child_stdin,
                hStdOutput: child_stdout,
                hStdError: child_stdout,
                ..Default::default()
            },
            lpAttributeList: attribute_list.as_raw(),
        };
        let environment = environment.map(environment_block);
        let mut flags = CREATE_NO_WINDOW | CREATE_SUSPENDED | EXTENDED_STARTUPINFO_PRESENT;
        if environment.is_some() {
            flags |= CREATE_UNICODE_ENVIRONMENT;
        }
        let application = wide(OsStr::new(shell));
        let mut process = PROCESS_INFORMATION::default();
        let created = unsafe {
            CreateProcessW(
                PCWSTR(application.as_ptr()),
                Some(PWSTR(command_line.as_mut_ptr())),
                None,
                None,
                true,
                flags,
                environment
                    .as_ref()
                    .map(|block| block.as_ptr() as *const core::ffi::c_void),
                PCWSTR::null(),
                &startup.StartupInfo,
                &mut process,
            )
        };
        drop(stdin_read);
        drop(stdout_write);
        created.with_context(|| format!("Failed to start {shell} on pipes"))?;

        // CreateProcessW 成功后进程与主线程句柄归我们关闭。
        let process_handle = unsafe { OwnedHandle::from_raw_handle(process.hProcess.0 as _) };
        let thread_handle = unsafe { OwnedHandle::from_raw_handle(process.hThread.0 as _) };
        let job = match KillOnCloseJob::adopt(process_handle.as_raw_handle()) {
            Ok(job) => Arc::new(job),
            Err(error) => {
                let _ = unsafe { TerminateProcess(raw_handle(process_handle.as_raw_handle()), 1) };
                let _ = unsafe { ResumeThread(raw_handle(thread_handle.as_raw_handle())) };
                return Err(error);
            }
        };
        if unsafe { ResumeThread(raw_handle(thread_handle.as_raw_handle())) } == u32::MAX {
            let error = std::io::Error::last_os_error();
            job.terminate();
            return Err(anyhow!("Failed to resume shell: {error}"));
        }
        drop(thread_handle);

        let (tx, rx) = channel();
        spawn_output_thread(File::from(stdout_read), tx.clone())?;
        Ok(Self {
            reader: PipeReader {
                rx,
                pending: Vec::new(),
                eof: false,
            },
            writer: PipeWriter {
                stdin: File::from(stdin_write),
                editor: shell_args.local_echo.then(LineEditor::default),
                carry: Utf8Carry::default(),
                codec: AnsiCodec::system(),
                echo: tx,
                job: job.clone(),
            },
            child: PipeChild {
                process: Arc::new(process_handle),
                pid: process.dwProcessId,
                job,
            },
        })
    }
}

enum Chunk {
    Data(Vec<u8>),
    Eof,
}

/// shell 输出（已转成 UTF-8、补齐行尾）与本地回显的合流。
pub struct PipeReader {
    rx: Receiver<Chunk>,
    pending: Vec<u8>,
    eof: bool,
}

impl Read for PipeReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        while self.pending.is_empty() && !self.eof {
            match self.rx.recv() {
                Ok(Chunk::Data(data)) => self.pending = data,
                Ok(Chunk::Eof) | Err(_) => self.eof = true,
            }
        }
        let count = buf.len().min(self.pending.len());
        buf[..count].copy_from_slice(&self.pending[..count]);
        self.pending.drain(..count);
        Ok(count)
    }
}

/// 远端输入（UTF-8）→ shell。cmd 由本端行编辑并回显，PowerShell 自己逐字回显。
pub struct PipeWriter {
    stdin: File,
    editor: Option<LineEditor>,
    /// 一个字符的 UTF-8 字节可能分在两条消息里，凑齐再转成 shell 代码页。
    carry: Utf8Carry,
    codec: AnsiCodec,
    echo: Sender<Chunk>,
    job: Arc<KillOnCloseJob>,
}

impl Write for PipeWriter {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        let (to_shell, echo) = match self.editor.as_mut() {
            Some(editor) => editor.feed(data),
            None => (normalize_enter(data), Vec::new()),
        };
        if !echo.is_empty() {
            let _ = self.echo.send(Chunk::Data(echo));
        }
        let to_shell = self.codec.encode(&self.carry.push(&to_shell));
        if !to_shell.is_empty() {
            self.stdin.write_all(&to_shell)?;
        }
        Ok(data.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.stdin.flush()
    }
}

impl Drop for PipeWriter {
    fn drop(&mut self) {
        self.job.terminate();
        // 别的进程若在句柄可继承的窗口期继承了 shell 的 stdout 写端，管道永远不会 EOF；
        // 直接通知读端结束，会话关闭不依赖管道 EOF。
        let _ = self.echo.send(Chunk::Eof);
    }
}

/// 管道 shell 进程。结束时连同它派生的整棵进程树一起结束。
#[derive(Debug)]
pub struct PipeChild {
    process: Arc<OwnedHandle>,
    pid: u32,
    job: Arc<KillOnCloseJob>,
}

impl PipeChild {
    fn exit_code(&self) -> std::io::Result<Option<u32>> {
        let mut code = 0u32;
        unsafe { GetExitCodeProcess(raw_handle(self.process.as_raw_handle()), &mut code) }
            .map_err(std::io::Error::other)?;
        Ok((code != STILL_ACTIVE.0 as u32).then_some(code))
    }
}

impl ChildKiller for PipeChild {
    fn kill(&mut self) -> std::io::Result<()> {
        self.job.terminate();
        Ok(())
    }

    fn clone_killer(&self) -> Box<dyn ChildKiller + Send + Sync> {
        Box::new(JobKiller(self.job.clone()))
    }
}

impl Child for PipeChild {
    fn try_wait(&mut self) -> std::io::Result<Option<ExitStatus>> {
        Ok(self.exit_code()?.map(ExitStatus::with_exit_code))
    }

    fn wait(&mut self) -> std::io::Result<ExitStatus> {
        unsafe { WaitForSingleObject(raw_handle(self.process.as_raw_handle()), INFINITE) };
        Ok(ExitStatus::with_exit_code(self.exit_code()?.unwrap_or(1)))
    }

    fn process_id(&self) -> Option<u32> {
        Some(self.pid)
    }

    fn as_raw_handle(&self) -> Option<RawHandle> {
        Some(self.process.as_raw_handle())
    }
}

#[derive(Debug)]
struct JobKiller(Arc<KillOnCloseJob>);

impl ChildKiller for JobKiller {
    fn kill(&mut self) -> std::io::Result<()> {
        self.0.terminate();
        Ok(())
    }

    fn clone_killer(&self) -> Box<dyn ChildKiller + Send + Sync> {
        Box::new(JobKiller(self.0.clone()))
    }
}

/// shell 及其派生的全部子进程都归入此 Job；终止或关闭句柄时整棵树一起结束。
#[derive(Debug)]
struct KillOnCloseJob(HANDLE);

// Job 句柄只用于 Terminate/Close，跨线程使用安全。
unsafe impl Send for KillOnCloseJob {}
unsafe impl Sync for KillOnCloseJob {}

impl KillOnCloseJob {
    fn adopt(process: RawHandle) -> Result<Self> {
        let job = unsafe { CreateJobObjectW(None, PCWSTR::null()) }.context("create job")?;
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
        .context("configure job")?;
        unsafe { AssignProcessToJobObject(job.0, raw_handle(process)) }.context("assign job")?;
        Ok(job)
    }

    fn terminate(&self) {
        if let Err(error) = unsafe { TerminateJobObject(self.0, 1) } {
            log::trace!("terminate terminal job failed: {error}");
        }
    }
}

impl Drop for KillOnCloseJob {
    fn drop(&mut self) {
        let _ = unsafe { CloseHandle(self.0) };
    }
}

fn spawn_output_thread(mut stdout: File, tx: Sender<Chunk>) -> Result<()> {
    std::thread::Builder::new()
        .name("terminal-pipe-output".into())
        .spawn(move || {
            let mut buffer = vec![0u8; READ_BUFFER_BYTES];
            let mut codec = AnsiCodec::system();
            let mut normalizer = CrlfNormalizer::default();
            loop {
                match stdout.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(count) => {
                        let decoded = codec.decode(&buffer[..count]);
                        if decoded.is_empty() {
                            continue;
                        }
                        if tx
                            .send(Chunk::Data(normalizer.normalize(&decoded)))
                            .is_err()
                        {
                            break;
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                    // 管道写端关闭（shell 已退出）时是 ERROR_BROKEN_PIPE，按 EOF 处理。
                    Err(_) => break,
                }
            }
            let _ = tx.send(Chunk::Eof);
        })
        .context("Failed to start terminal pipe output thread")?;
    Ok(())
}

/// 生成 CreateProcessW 要的 UTF-16 环境块。变量名按 Windows 规则不区分大小写，
/// 同名时后出现的生效，块内按大写名排序。
fn environment_block(vars: &[(OsString, OsString)]) -> Vec<u16> {
    let key = |name: &OsStr| name.to_string_lossy().to_uppercase();
    let vars: BTreeMap<String, &(OsString, OsString)> =
        vars.iter().map(|var| (key(&var.0), var)).collect();
    let mut block = Vec::new();
    for (name, value) in vars.values() {
        block.extend(name.encode_wide());
        block.push(b'=' as u16);
        block.extend(value.encode_wide());
        block.push(0);
    }
    block.push(0);
    block
}

/// 管道下的 shell 按系统 ANSI 代码页读写（GEP 的 Server 2016 是 936/GBK），远端是 UTF-8。
/// 不能用 `chcp 65001`：它只改输出代码页，stdin 仍按启动代码页解析。
struct AnsiCodec {
    codepage: u32,
    /// 上一块末尾落单的 DBCS 前导字节，留到下一块再拼。
    pending: Vec<u8>,
}

impl AnsiCodec {
    fn system() -> Self {
        Self {
            codepage: unsafe { GetACP() },
            pending: Vec::new(),
        }
    }

    fn is_identity(&self) -> bool {
        self.codepage == 0 || self.codepage == CP_UTF8
    }

    fn encode(&self, data: &[u8]) -> Vec<u8> {
        if self.is_identity() || data.is_empty() {
            return data.to_vec();
        }
        utf8_to_wide(data)
            .and_then(|wide| wide_to_multibyte(self.codepage, &wide))
            .unwrap_or_else(|| data.to_vec())
    }

    fn decode(&mut self, chunk: &[u8]) -> Vec<u8> {
        if self.is_identity() {
            return chunk.to_vec();
        }
        self.pending.extend_from_slice(chunk);
        let split = complete_dbcs_prefix(&self.pending, |byte| is_lead_byte(self.codepage, byte));
        if split == 0 {
            return Vec::new();
        }
        let ready: Vec<u8> = self.pending.drain(..split).collect();
        multibyte_to_wide(self.codepage, &ready)
            .and_then(|wide| wide_to_multibyte(CP_UTF8, &wide))
            .unwrap_or(ready)
    }
}

fn is_lead_byte(codepage: u32, byte: u8) -> bool {
    unsafe { IsDBCSLeadByteEx(codepage, byte).is_ok() }
}

fn utf8_to_wide(data: &[u8]) -> Option<Vec<u16>> {
    multibyte_to_wide(CP_UTF8, data)
}

fn multibyte_to_wide(codepage: u32, data: &[u8]) -> Option<Vec<u16>> {
    let flags = MULTI_BYTE_TO_WIDE_CHAR_FLAGS(0);
    let needed = unsafe { MultiByteToWideChar(codepage, flags, data, None) };
    if needed <= 0 {
        return None;
    }
    let mut wide = vec![0u16; needed as usize];
    let written = unsafe { MultiByteToWideChar(codepage, flags, data, Some(&mut wide)) };
    if written <= 0 {
        return None;
    }
    wide.truncate(written as usize);
    Some(wide)
}

fn wide_to_multibyte(codepage: u32, wide: &[u16]) -> Option<Vec<u8>> {
    let needed = unsafe { WideCharToMultiByte(codepage, 0, wide, None, PCSTR::null(), None) };
    if needed <= 0 {
        return None;
    }
    let mut out = vec![0u8; needed as usize];
    let written =
        unsafe { WideCharToMultiByte(codepage, 0, wide, Some(&mut out), PCSTR::null(), None) };
    if written <= 0 {
        return None;
    }
    out.truncate(written as usize);
    Some(out)
}

fn wide(value: &OsStr) -> Vec<u16> {
    value.encode_wide().chain(Some(0)).collect()
}

/// CreateProcessW 的命令行分词规则：双引号成对，反斜杠只在引号前成对转义。
fn push_quoted(out: &mut Vec<u16>, value: &OsStr) {
    out.push(b'"' as u16);
    let mut backslashes = 0usize;
    for unit in value.encode_wide() {
        if unit == b'\\' as u16 {
            backslashes += 1;
            continue;
        }
        if unit == b'"' as u16 {
            out.extend(std::iter::repeat(b'\\' as u16).take(backslashes * 2 + 1));
            out.push(unit);
            backslashes = 0;
            continue;
        }
        out.extend(std::iter::repeat(b'\\' as u16).take(backslashes));
        out.push(unit);
        backslashes = 0;
    }
    out.extend(std::iter::repeat(b'\\' as u16).take(backslashes * 2));
    out.push(b'"' as u16);
}

struct AttributeList {
    raw: Vec<u8>,
}

impl AttributeList {
    fn new(count: u32) -> Result<Self> {
        let mut size = 0usize;
        let _ = unsafe { InitializeProcThreadAttributeList(None, count, None, &mut size) };
        if size == 0 {
            return Err(anyhow!(
                "attribute list size: {}",
                std::io::Error::last_os_error()
            ));
        }
        let mut raw = vec![0u8; size];
        unsafe {
            InitializeProcThreadAttributeList(
                Some(LPPROC_THREAD_ATTRIBUTE_LIST(raw.as_mut_ptr() as _)),
                count,
                None,
                &mut size,
            )
        }
        .context("attribute list")?;
        Ok(Self { raw })
    }

    fn as_raw(&mut self) -> LPPROC_THREAD_ATTRIBUTE_LIST {
        LPPROC_THREAD_ATTRIBUTE_LIST(self.raw.as_mut_ptr() as _)
    }

    fn set_handles(&mut self, handles: &mut [HANDLE]) -> Result<()> {
        unsafe {
            UpdateProcThreadAttribute(
                self.as_raw(),
                0,
                PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize,
                Some(handles.as_mut_ptr() as *const _),
                std::mem::size_of_val(handles),
                None,
                None,
            )
        }
        .context("handle list")
    }
}

impl Drop for AttributeList {
    fn drop(&mut self) {
        unsafe { DeleteProcThreadAttributeList(self.as_raw()) };
    }
}

fn pipe_pair() -> Result<(OwnedHandle, OwnedHandle)> {
    let mut read = HANDLE::default();
    let mut write = HANDLE::default();
    unsafe { CreatePipe(&mut read, &mut write, None, 0) }.context("Failed to create pipe")?;
    Ok((
        unsafe { OwnedHandle::from_raw_handle(read.0 as _) },
        unsafe { OwnedHandle::from_raw_handle(write.0 as _) },
    ))
}

/// HANDLE_LIST 只接受可继承句柄，CreatePipe 给的是不可继承的。
fn make_inheritable(handle: HANDLE) -> Result<()> {
    unsafe { SetHandleInformation(handle, HANDLE_FLAG_INHERIT.0, HANDLE_FLAG_INHERIT) }
        .context("handle inheritance")
}

fn raw_handle(handle: RawHandle) -> HANDLE {
    HANDLE(handle as _)
}

#[cfg(test)]
#[path = "terminal_pipe_win_tests.rs"]
mod tests;
