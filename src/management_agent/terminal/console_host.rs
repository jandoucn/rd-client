//! 没有 ConPTY 的 Windows 上的备选终端。
//!
//! `CreatePseudoConsole` 从 Windows 10 1809 才有。Server 2016 等更早系统调用它会直接失败，
//! 网页上就只看到「设备未能启动终端」。这里用匿名管道接 shell 的标准输入输出，不经过伪控制台。
//! 有 ConPTY 的系统不会进入本模块。
//!
//! 这不是完整伪终端：没有窗口尺寸、没有 VT 全屏。三种 shell 都能输入输出，也仍归入 Job，
//! 结束会话会杀掉整棵进程树。

use super::{
    job_windows::KillOnCloseJob, OutputChunk, OutputRing, ShellCommand, TerminalError, TerminalSize,
};
use std::{
    ffi::{CStr, OsStr},
    io::Write,
    os::windows::{
        ffi::OsStrExt,
        io::{AsRawHandle, FromRawHandle, OwnedHandle, RawHandle},
    },
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, MutexGuard, PoisonError,
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};
use windows::{
    core::{PCSTR, PCWSTR, PWSTR},
    Win32::{
        Foundation::{HANDLE, HANDLE_FLAG_INHERIT, SetHandleInformation, STILL_ACTIVE},
        Globalization::{
            GetACP, IsDBCSLeadByteEx, MultiByteToWideChar, WideCharToMultiByte,
            MULTI_BYTE_TO_WIDE_CHAR_FLAGS,
        },
        Storage::FileSystem::ReadFile,
        System::{
            LibraryLoader::{GetModuleHandleW, GetProcAddress},
            Pipes::CreatePipe,
            Threading::{
                CreateProcessW, DeleteProcThreadAttributeList, GetExitCodeProcess,
                InitializeProcThreadAttributeList, ResumeThread, TerminateProcess,
                UpdateProcThreadAttribute, WaitForSingleObject, CREATE_NO_WINDOW,
                CREATE_SUSPENDED, EXTENDED_STARTUPINFO_PRESENT, LPPROC_THREAD_ATTRIBUTE_LIST,
                PROCESS_INFORMATION, PROC_THREAD_ATTRIBUTE_HANDLE_LIST, STARTF_USESTDHANDLES,
                STARTUPINFOEXW, STARTUPINFOW,
            },
        },
    },
};

const READ_BUFFER_BYTES: usize = 8 * 1024;
const READER_JOIN_TIMEOUT: Duration = Duration::from_secs(2);

struct PipeChild {
    process: OwnedHandle,
    thread: OwnedHandle,
    pid: u32,
}

/// 管道上的 shell。对外实现与 ConPTY 会话相同的 [`super::TerminalSlot`]。
pub struct PipeSession {
    writer: Mutex<Option<std::fs::File>>,
    process: Mutex<PipeChild>,
    ring: Arc<Mutex<OutputRing>>,
    last_activity: Arc<Mutex<Instant>>,
    started: Instant,
    reader: Option<JoinHandle<()>>,
    reader_done: Arc<AtomicBool>,
    job: KillOnCloseJob,
    closed: bool,
    output_settled: AtomicBool,
    /// 由本进程做行编辑与回显（cmd）。PowerShell 自己逐字回显，为 `None`。
    editor: Option<Mutex<LineEditor>>,
    /// 输入方向：浏览器 UTF-8 → shell 的 ANSI 代码页。
    in_codec: AnsiCodec,
    /// 输出方向：shell 的 ANSI 代码页 → 浏览器 UTF-8。读线程持有可变状态。
    out_pending: Arc<Mutex<Vec<u8>>>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn start_failed(error: impl std::fmt::Display) -> TerminalError {
    TerminalError::StartFailed(error.to_string())
}

fn io_failed(error: impl std::fmt::Display) -> TerminalError {
    TerminalError::Io(error.to_string())
}

/// kernel32 导出了 `CreatePseudoConsole` 才走现有 ConPTY。按函数是否存在判断，不按版本号。
pub fn conpty_available() -> bool {
    let kernel = wide(OsStr::new("kernel32.dll"));
    let Ok(module) = (unsafe { GetModuleHandleW(PCWSTR(kernel.as_ptr())) }) else {
        return false;
    };
    let name = CStr::from_bytes_with_nul(b"CreatePseudoConsole\0").expect("static c string");
    unsafe { GetProcAddress(module, PCSTR(name.as_ptr() as *const u8)) }.is_some()
}

fn wide(value: &OsStr) -> Vec<u16> {
    value.encode_wide().chain(Some(0)).collect()
}

/// CreateProcessW 的命令行分词规则：空格分隔，双引号成对，反斜杠只在引号前成对转义。
fn push_quoted(out: &mut Vec<u16>, value: &OsStr) {
    out.push(b'"' as u16);
    let mut backslashes = 0u16;
    for unit in value.encode_wide() {
        if unit == b'\\' as u16 {
            backslashes += 1;
            continue;
        }
        if unit == b'"' as u16 {
            out.extend(std::iter::repeat(b'\\' as u16).take(backslashes as usize * 2 + 1));
            out.push(unit);
            backslashes = 0;
            continue;
        }
        out.extend(std::iter::repeat(b'\\' as u16).take(backslashes as usize));
        out.push(unit);
        backslashes = 0;
    }
    out.extend(std::iter::repeat(b'\\' as u16).take(backslashes as usize * 2));
    out.push(b'"' as u16);
}

struct AttributeList {
    raw: Vec<u8>,
}

impl AttributeList {
    fn new(count: u32) -> Result<Self, String> {
        let mut size = 0usize;
        let _ = unsafe { InitializeProcThreadAttributeList(None, count, None, &mut size) };
        if size == 0 {
            return Err(format!(
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
        .map_err(|error| format!("attribute list: {error}"))?;
        Ok(Self { raw })
    }

    fn as_raw(&mut self) -> LPPROC_THREAD_ATTRIBUTE_LIST {
        LPPROC_THREAD_ATTRIBUTE_LIST(self.raw.as_mut_ptr() as _)
    }

    fn set_handles(&mut self, handles: &mut [HANDLE]) -> Result<(), String> {
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
        .map_err(|error| format!("handle list: {error}"))
    }
}

impl Drop for AttributeList {
    fn drop(&mut self) {
        unsafe { DeleteProcThreadAttributeList(self.as_raw()) };
    }
}

fn pipe_pair() -> Result<(OwnedHandle, OwnedHandle), TerminalError> {
    let mut read = HANDLE::default();
    let mut write = HANDLE::default();
    unsafe { CreatePipe(&mut read, &mut write, None, 0) }.map_err(start_failed)?;
    // CreatePipe 成功后两个句柄都已打开，必须由 OwnedHandle 负责关闭。
    Ok((
        unsafe { OwnedHandle::from_raw_handle(read.0 as _) },
        unsafe { OwnedHandle::from_raw_handle(write.0 as _) },
    ))
}

/// 把句柄标成可继承。`CreateProcessW` 带 bInheritHandles=true 时，HANDLE_LIST 里列出的
/// 句柄必须本身可继承，否则整次创建直接失败（ERROR_INVALID_PARAMETER）。
/// CreatePipe 默认给的是不可继承句柄，所以子进程要用的那两端必须显式打开这个标志。
fn make_inheritable(handle: HANDLE) -> Result<(), TerminalError> {
    unsafe { SetHandleInformation(handle, HANDLE_FLAG_INHERIT.0, HANDLE_FLAG_INHERIT) }
        .map_err(|error| start_failed(format!("handle inheritance: {error}")))
}

impl PipeSession {
    pub fn spawn(
        command: &ShellCommand,
        cwd: Option<&Path>,
        _size: TerminalSize,
        ring_capacity: usize,
    ) -> Result<Self, TerminalError> {        let (stdin_read, stdin_write) = pipe_pair()?;
        let (stdout_read, stdout_write) = pipe_pair()?;
        // stderr 与 stdout 共用写端。句柄列表里同一句柄只能出现一次，所以不另复制。
        let child_stdin = raw_handle(stdin_read.as_raw_handle());
        let child_stdout = raw_handle(stdout_write.as_raw_handle());
        // HANDLE_LIST 只接受可继承句柄；CreatePipe 给的是不可继承的，这里显式打开。
        make_inheritable(child_stdin)?;
        make_inheritable(child_stdout)?;

        let mut command_line = Vec::new();
        push_quoted(&mut command_line, command.program.as_os_str());
        for arg in &command.args {
            command_line.push(b' ' as u16);
            // PowerShell 的 -Command 含空格与引号，必须整段加引号，否则 cmd/powershell 都会拆错参数。
            push_quoted(&mut command_line, OsStr::new(arg));
        }
        command_line.push(0);

        let mut handles = [child_stdin, child_stdout];
        let mut attribute_list = AttributeList::new(1).map_err(start_failed)?;
        attribute_list
            .set_handles(&mut handles)
            .map_err(start_failed)?;
        let mut startup = STARTUPINFOEXW {
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
        let directory = cwd.map(|path| wide(path.as_os_str()));
        let directory = directory
            .as_ref()
            .map(|value| PCWSTR(value.as_ptr()))
            .unwrap_or(PCWSTR::null());
        let application = wide(command.program.as_os_str());
        let mut process = PROCESS_INFORMATION::default();
        let created = unsafe {
            CreateProcessW(
                PCWSTR(application.as_ptr()),
                Some(PWSTR(command_line.as_mut_ptr())),
                None,
                None,
                true,
                CREATE_NO_WINDOW | CREATE_SUSPENDED | EXTENDED_STARTUPINFO_PRESENT,
                None,
                directory,
                &startup.StartupInfo,
                &mut process,
            )
        };
        drop(stdin_read);
        drop(stdout_write);
        created.map_err(start_failed)?;

        // CreateProcessW 成功后进程与主线程句柄归我们关闭。
        let child = PipeChild {
            process: unsafe { OwnedHandle::from_raw_handle(process.hProcess.0 as _) },
            thread: unsafe { OwnedHandle::from_raw_handle(process.hThread.0 as _) },
            pid: process.dwProcessId,
        };
        let _ = child.pid;
        let job = match KillOnCloseJob::adopt_handle(child.process.as_raw_handle()) {
            Ok(job) => job,
            Err(error) => {
                let _ = unsafe { TerminateProcess(raw_handle(child.process.as_raw_handle()), 1) };
                let _ = unsafe { ResumeThread(raw_handle(child.thread.as_raw_handle())) };
                return Err(error);
            }
        };
        if unsafe { ResumeThread(raw_handle(child.thread.as_raw_handle())) } == u32::MAX {
            let error = std::io::Error::last_os_error();
            job.terminate();
            return Err(start_failed(format!("resume shell: {error}")));
        }

        let ring = Arc::new(Mutex::new(OutputRing::new(ring_capacity)));
        let last_activity = Arc::new(Mutex::new(Instant::now()));
        let reader_done = Arc::new(AtomicBool::new(false));
        // 输入输出各用一个编解码器：输出那个活在读线程里，持有 DBCS 跨块残留。
        let out_pending = Arc::new(Mutex::new(Vec::new()));
        let reader = spawn_reader(
            stdout_read,
            ring.clone(),
            last_activity.clone(),
            reader_done.clone(),
            AnsiCodec::system(),
            out_pending.clone(),
        )?;
        Ok(Self {
            writer: Mutex::new(Some(unsafe { std::fs::File::from(stdin_write) })),
            process: Mutex::new(child),
            ring,
            started: Instant::now(),
            last_activity,
            reader: Some(reader),
            reader_done,
            job,
            closed: false,
            output_settled: AtomicBool::new(false),
            editor: command.local_echo.then(|| Mutex::new(LineEditor::default())),
            in_codec: AnsiCodec::system(),
            out_pending,
        })
    }

    pub fn write(&self, data: &[u8]) -> Result<(), TerminalError> {
        let mut writer = lock(&self.writer);
        let writer = writer
            .as_mut()
            .ok_or_else(|| TerminalError::Io("terminal session is closed".into()))?;
        // 本地行编辑的会话（cmd）先回显按键、只把整行交给 shell；
        // 其余会话（PowerShell）直接透传，它自己逐字回显。裸 CR 统一补成 CRLF。
        let (to_shell, echo) = match self.editor.as_ref() {
            Some(editor) => lock(editor).feed(data),
            None => (normalize_enter(data), Vec::new()),
        };
        // 回显在本地生成，始终是 UTF-8，直接进输出环，不经过 shell 的代码页。
        if !echo.is_empty() {
            lock(&self.ring).push(&echo);
        }
        // 真正下发前转成 shell 的代码页：它按系统 ANSI 代码页解析命令行。
        let to_shell = self.in_codec.to_shell(&to_shell);
        if !to_shell.is_empty() {
            writer.write_all(&to_shell).map_err(io_failed)?;
            writer.flush().map_err(io_failed)?;
        }
        *lock(&self.last_activity) = Instant::now();
        Ok(())
    }

    pub fn read_since(
        &self,
        last_seq: u64,
        max_bytes: usize,
    ) -> Result<Vec<OutputChunk>, TerminalError> {
        lock(&self.ring).since(last_seq, max_bytes)
    }

    pub fn exit_code(&self) -> Option<u32> {
        let child = lock(&self.process);
        let mut code = 0u32;
        if unsafe { GetExitCodeProcess(raw_handle(child.process.as_raw_handle()), &mut code) }
            .is_err()
        {
            return None;
        }
        (code != STILL_ACTIVE.0 as u32).then_some(code)
    }

    pub fn finish_output(&self) {
        if self.output_settled.load(Ordering::SeqCst) {
            return;
        }
        lock(&self.writer).take();
        let deadline = Instant::now() + READER_JOIN_TIMEOUT;
        while !self.reader_done.load(Ordering::SeqCst) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        self.output_settled.store(true, Ordering::SeqCst);
    }

    pub fn output_settled(&self) -> bool {
        self.output_settled.load(Ordering::SeqCst)
    }

    fn shutdown(&mut self) -> Result<Option<u32>, TerminalError> {
        if self.closed {
            return Ok(None);
        }
        self.closed = true;
        let already = self.exit_code();
        if already.is_none() {
            self.job.terminate();
            let child = lock(&self.process);
            let _ = unsafe { WaitForSingleObject(raw_handle(child.process.as_raw_handle()), 2000) };
        }
        lock(&self.writer).take();
        self.join_reader();
        Ok(self.exit_code().or(already))
    }

    fn join_reader(&mut self) {
        let deadline = Instant::now() + READER_JOIN_TIMEOUT;
        while !self.reader_done.load(Ordering::SeqCst) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        if let Some(reader) = self.reader.take() {
            if self.reader_done.load(Ordering::SeqCst) {
                let _ = reader.join();
            }
        }
    }
}

impl Drop for PipeSession {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

/// 把裸 CR 补成 CRLF（输入方向）。
///
/// xterm 按回车发送的是裸 `\r`。实测管道下的 cmd 只认 CRLF：收到裸 CR 时整条命令
/// 既不回显也不执行，用户会觉得「根本打不进字」。PowerShell 能执行裸 CR，但输出行尾会缺 LF。
/// 已经是 CRLF 的原样保留。单次按键只写一个 `\r`，所以末尾的 CR 也必须补。
fn normalize_enter(data: &[u8]) -> Vec<u8> {
    if !data.contains(&b'\r') {
        return data.to_vec();
    }
    let mut out = Vec::with_capacity(data.len() + 4);
    for (index, &byte) in data.iter().enumerate() {
        out.push(byte);
        if byte == b'\r' && data.get(index + 1) != Some(&b'\n') {
            out.push(b'\n');
        }
    }
    out
}

/// 管道下的 shell 用系统 ANSI 代码页读写（实测 GEP 的 Server 2016 是 936/GBK），
/// 浏览器是 UTF-8。转码必须在本进程完成，否则中文全是乱码。
///
/// 不能改成 `chcp 65001`：实测它只改输出代码页，cmd 解析 stdin 那条命令行时仍按启动时的
/// 936 处理，UTF-8 中文被拆成非法字节，整条命令直接变成 `More?`。
const CP_UTF8: u32 = 65001;

/// shell 与浏览器之间的字符集桥。
struct AnsiCodec {
    /// 系统 ANSI 代码页。shell 没有控制台，管道读写用的就是它。
    codepage: u32,
    /// 上一块末尾留下的 DBCS 前导字节：双字节字符被管道读切断时留到下一块再拼。
    pending: Vec<u8>,
}

impl AnsiCodec {
    fn system() -> Self {
        let codepage = unsafe { GetACP() };
        Self {
            codepage,
            pending: Vec::new(),
        }
    }

    /// 代码页本身就是 UTF-8 时所有转换都是恒等，不必进出 Win32 API。
    fn is_identity(&self) -> bool {
        self.codepage == 0 || self.codepage == CP_UTF8
    }

    /// 浏览器（UTF-8）→ shell（ANSI）。无法表示的字符退化成 `?`，但不丢整行。
    fn to_shell(&self, data: &[u8]) -> Vec<u8> {
        if self.is_identity() || data.is_empty() {
            return data.to_vec();
        }
        match utf8_to_wide(data).and_then(|wide| wide_to_ansi(self.codepage, &wide)) {
            Some(converted) => converted,
            // 转不动就原样送过去：宁可乱码也不要吞掉用户敲的命令。
            None => data.to_vec(),
        }
    }

    /// shell（ANSI）→ 浏览器（UTF-8）。跨块断开的双字节字符留到下一块处理。
    fn from_shell(&mut self, chunk: &[u8]) -> Vec<u8> {
        if self.is_identity() {
            return chunk.to_vec();
        }
        self.pending.extend_from_slice(chunk);
        // 末尾落单的 DBCS 前导字节，后半个字符还在管道里，先别急着转。
        let codepage = self.codepage;
        let split = complete_dbcs_prefix(&self.pending, |byte| is_lead_byte(codepage, byte));
        if split == 0 {
            return Vec::new();
        }
        let ready: Vec<u8> = self.pending.drain(..split).collect();
        match ansi_to_utf8(self.codepage, &ready) {
            Some(converted) => converted,
            None => ready,
        }
    }
}

/// 从头成对扫描，返回完整字符的字节数，只把末尾落单的前导字节留下。
/// 不能只看最后一个字节：GBK 尾字节 0x40..=0xFE 与前导字节 0x81..=0xFE 重叠，
/// 「中」= D6 D0 的尾字节 D0 也会被判成前导字节。
fn complete_dbcs_prefix(bytes: &[u8], is_lead: impl Fn(u8) -> bool) -> usize {
    let mut index = 0;
    while index < bytes.len() {
        if is_lead(bytes[index]) {
            if index + 1 == bytes.len() {
                return index;
            }
            index += 2;
        } else {
            index += 1;
        }
    }
    bytes.len()
}

fn is_lead_byte(codepage: u32, byte: u8) -> bool {
    unsafe { IsDBCSLeadByteEx(codepage, byte).is_ok() }
}

fn utf8_to_wide(data: &[u8]) -> Option<Vec<u16>> {
    // 不用 MB_ERR_INVALID_CHARS：非法序列替换成 U+FFFD，好过整段失败。
    let needed = unsafe { MultiByteToWideChar(CP_UTF8, MULTI_BYTE_TO_WIDE_CHAR_FLAGS(0), data, None) };
    if needed <= 0 {
        return None;
    }
    let mut wide = vec![0u16; needed as usize];
    let written =
        unsafe { MultiByteToWideChar(CP_UTF8, MULTI_BYTE_TO_WIDE_CHAR_FLAGS(0), data, Some(&mut wide)) };
    if written <= 0 {
        return None;
    }
    wide.truncate(written as usize);
    Some(wide)
}

fn wide_to_utf8(wide: &[u16]) -> Option<Vec<u8>> {
    let needed = unsafe { WideCharToMultiByte(CP_UTF8, 0, wide, None, PCSTR::null(), None) };
    if needed <= 0 {
        return None;
    }
    let mut out = vec![0u8; needed as usize];
    let written =
        unsafe { WideCharToMultiByte(CP_UTF8, 0, wide, Some(&mut out), PCSTR::null(), None) };
    if written <= 0 {
        return None;
    }
    out.truncate(written as usize);
    Some(out)
}

fn wide_to_ansi(codepage: u32, wide: &[u16]) -> Option<Vec<u8>> {
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

fn ansi_to_utf8(codepage: u32, data: &[u8]) -> Option<Vec<u8>> {
    let needed =
        unsafe { MultiByteToWideChar(codepage, MULTI_BYTE_TO_WIDE_CHAR_FLAGS(0), data, None) };
    if needed <= 0 {
        return None;
    }
    let mut wide = vec![0u16; needed as usize];
    let written = unsafe {
        MultiByteToWideChar(
            codepage,
            MULTI_BYTE_TO_WIDE_CHAR_FLAGS(0),
            data,
            Some(&mut wide),
        )
    };
    if written <= 0 {
        return None;
    }
    wide.truncate(written as usize);
    wide_to_utf8(&wide)
}

/// 一个字符在终端里占几列。CJK 与全角字符占两列，退格擦除时必须擦两格，
/// 否则删掉一个汉字会在屏幕上留下半个字的残影。
fn display_width(ch: char) -> usize {
    match ch as u32 {
        0x1100..=0x115F
        | 0x2E80..=0xA4CF
        | 0xAC00..=0xD7A3
        | 0xF900..=0xFAFF
        | 0xFE30..=0xFE6F
        | 0xFF00..=0xFF60
        | 0xFFE0..=0xFFE6
        | 0x1F300..=0x1F64F
        | 0x1F900..=0x1F9FF
        | 0x20000..=0x3FFFD => 2,
        _ => 1,
    }
}

/// cmd 管道会话的本地行编辑。
///
/// 实测 cmd 在管道输入下是纯行输入模式：逐字不回显，退格只当普通字节
/// （`wxyz` + 4 个退格 + `hoami` + 回车什么都不执行）。所以编辑必须在本进程完成：
/// 按键先回显给浏览器，只有整行在回车时才交给 shell。配合启动参数里的 `/Q`
/// 关闭 shell 自己的整行回显，按回车时才不会看到两遍命令。
#[derive(Default)]
struct LineEditor {
    line: Vec<u8>,
    /// 上一个字节是 CR：紧跟的 LF 要吞掉，避免同一行被提交两次。
    swallow_lf: bool,
    /// 方向键、Home 等发送的转义序列管道下的 cmd 处理不了，整段丢弃，可能跨多次输入。
    escape: Escape,
}

#[derive(Default, Clone, Copy, PartialEq)]
enum Escape {
    #[default]
    None,
    /// 刚收到 ESC。
    Esc,
    /// `ESC [`，直到 0x40..=0x7E 的终止字节。
    Csi,
    /// `ESC O`，后面再跟一个字节。
    Ss3,
}

impl LineEditor {
    /// 处理一段输入，返回要写进 shell 的字节；回显内容由调用方推给输出环。
    fn feed(&mut self, data: &[u8]) -> (Vec<u8>, Vec<u8>) {
        let mut to_shell = Vec::new();
        let mut echo = Vec::new();
        for &byte in data {
            if self.swallow_lf {
                self.swallow_lf = false;
                if byte == b'\n' {
                    continue;
                }
            }
            if self.skip_escape(byte) {
                continue;
            }
            match byte {
                b'\r' | b'\n' => {
                    // cmd 只认 CRLF：裸 LF 进来也补成 CRLF 再提交。
                    self.swallow_lf = byte == b'\r';
                    to_shell.extend_from_slice(&self.line);
                    to_shell.extend_from_slice(b"\r\n");
                    echo.extend_from_slice(b"\r\n");
                    self.line.clear();
                }
                // 退格：删掉一个完整字符（可能是多字节 UTF-8），并把光标退回一格。
                0x08 | 0x7f => self.backspace(&mut echo),
                // Ctrl+C：丢弃未提交的行，交给 shell 中断当前命令。
                0x03 => {
                    self.line.clear();
                    echo.extend_from_slice(b"^C\r\n");
                    to_shell.push(0x03);
                }
                // 其余控制字符（Tab、Ctrl+Z 等）不缓冲、不回显，直接透传。
                byte if byte < 0x20 => to_shell.push(byte),
                byte => {
                    self.line.push(byte);
                    echo.push(byte);
                }
            }
        }
        (to_shell, echo)
    }

    /// 返回 true 表示该字节属于转义序列，已被丢弃。
    fn skip_escape(&mut self, byte: u8) -> bool {
        match self.escape {
            Escape::None if byte == 0x1b => {
                self.escape = Escape::Esc;
                true
            }
            Escape::None => false,
            Escape::Esc => {
                self.escape = match byte {
                    b'[' => Escape::Csi,
                    b'O' => Escape::Ss3,
                    _ => Escape::None,
                };
                self.escape != Escape::None
            }
            Escape::Csi => {
                if (0x40..=0x7e).contains(&byte) {
                    self.escape = Escape::None;
                }
                true
            }
            Escape::Ss3 => {
                self.escape = Escape::None;
                true
            }
        }
    }

    fn backspace(&mut self, echo: &mut Vec<u8>) {
        if self.line.is_empty() {
            return;
        }
        // 往回跳过 UTF-8 续字节（0b10xxxxxx），再连前导字节一起删掉，
        // 保证一次退格删掉一个完整字符而不是半个字符。
        let mut start = self.line.len() - 1;
        while start > 0 && self.line[start] & 0xc0 == 0x80 {
            start -= 1;
        }
        // 被删掉的字符可能是全角（中文占两列）：多擦一格，否则屏幕上留半个字。
        let removed_width = std::str::from_utf8(&self.line[start..])
            .ok()
            .and_then(|text| text.chars().next())
            .map(display_width)
            .unwrap_or(1);
        self.line.truncate(start);
        // 退格 + 空格 + 退格，重复到覆盖该字符的显示宽度。
        for _ in 0..removed_width {
            echo.extend_from_slice(b"\x08 \x08");
        }
    }
}

/// 把孤立的 CR 补成 CRLF。
///
/// 管道下的 shell 认为自己在往控制台写：它用 `\r` 回到行首覆盖同一行（提示符重画、
/// 进度刷新）。xterm 收到孤立的 `\r` 只会回到行首、不换行，于是后一段输出直接盖掉
/// 前一段，用户看到的就是「命令输出被覆盖」。这里按字节流做有状态规范化：上一个块末尾
/// 是 CR 而下一块开头不是 LF 时，补一个 LF。
struct CrlfNormalizer {
    pending_cr: bool,
}

impl CrlfNormalizer {
    fn new() -> Self {
        Self { pending_cr: false }
    }

    fn normalize(&mut self, chunk: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(chunk.len() + 8);
        for &byte in chunk {
            if self.pending_cr {
                self.pending_cr = false;
                if byte != b'\n' {
                    out.push(b'\n');
                }
            }
            if byte == b'\r' {
                self.pending_cr = true;
                out.push(b'\r');
                continue;
            }
            out.push(byte);
        }
        out
    }
}

fn spawn_reader(
    read: OwnedHandle,
    ring: Arc<Mutex<OutputRing>>,
    last_activity: Arc<Mutex<Instant>>,
    done: Arc<AtomicBool>,
    mut codec: AnsiCodec,
    out_pending: Arc<Mutex<Vec<u8>>>,
) -> Result<JoinHandle<()>, TerminalError> {
    std::thread::Builder::new()
        .name("terminal-pipe-output".into())
        .spawn(move || {
            let mut buffer = vec![0u8; READ_BUFFER_BYTES];
            let mut normalizer = CrlfNormalizer::new();
            // 跨块的 DBCS 前导字节从共享缓冲接回来：会话关闭后仍可能要继续拼。
            codec.pending = std::mem::take(&mut *lock(&out_pending));
            loop {
                let mut read_count = 0u32;
                let ok = unsafe {
                    ReadFile(
                        raw_handle(read.as_raw_handle()),
                        Some(buffer.as_mut_slice()),
                        Some(&mut read_count),
                        None,
                    )
                };
                match ok {
                    Ok(()) if read_count == 0 => break,
                    Ok(()) => {
                        // 先转成 UTF-8 再规范化行尾：xterm 按 UTF-8 解码。
                        let decoded = codec.from_shell(&buffer[..read_count as usize]);
                        *lock(&out_pending) = codec.pending.clone();
                        if decoded.is_empty() {
                            continue;
                        }
                        let normalized = normalizer.normalize(&decoded);
                        lock(&ring).push(&normalized);
                        *lock(&last_activity) = Instant::now();
                    }
                    Err(error) if error.code().0 as u32 == 109 || error.code().0 as u32 == 232 => {
                        break
                    } // ERROR_BROKEN_PIPE / ERROR_NO_DATA
                    Err(_) => break,
                }
            }
            let _ = read;
            done.store(true, Ordering::SeqCst);
        })
        .map_err(start_failed)
}

fn raw_handle(handle: RawHandle) -> HANDLE {
    HANDLE(handle as _)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 逐字输入期间只回显、不碰 shell；回车才整行交给 shell。
    /// 这是 cmd 管道模式唯一能用的输入方式：它自身逐字不回显、退格也只当普通字节。
    #[test]
    fn line_editor_echoes_typed_bytes_and_submits_on_enter() {
        let mut editor = LineEditor::default();
        let (to_shell, echo) = editor.feed(b"who");
        assert!(to_shell.is_empty());
        assert_eq!(echo, b"who");

        // xterm 的回车是裸 CR，必须补成 cmd 认识的 CRLF。
        let (to_shell, echo) = editor.feed(b"ami\r");
        assert_eq!(to_shell, b"whoami\r\n");
        assert_eq!(echo, b"ami\r\n");

        // 提交后缓冲清空，下一行从头开始。
        assert_eq!(editor.feed(b"x").0, Vec::<u8>::new() as Vec<u8>);
    }

    #[test]
    fn line_editor_swallows_lf_right_after_cr() {
        let mut editor = LineEditor::default();
        editor.feed(b"dir\r");
        // 同一行的 CRLF 不能提交两次。
        let (to_shell, echo) = editor.feed(b"\n");
        assert!(to_shell.is_empty());
        assert!(echo.is_empty());
    }

    /// 退格要删掉一个完整字符，并给终端擦掉一格。
    #[test]
    fn line_editor_backspace_removes_one_character() {
        let mut editor = LineEditor::default();
        editor.feed(b"whx");
        let (to_shell, echo) = editor.feed(&[0x08]);
        assert!(to_shell.is_empty());
        assert_eq!(echo, b"\x08 \x08");
        let (to_shell, _) = editor.feed(b"o\r");
        assert_eq!(to_shell, b"who\r\n");
    }

    /// 多字节 UTF-8 一次退格删一个字符，不能只删掉一个字节留下残缺序列。
    #[test]
    fn line_editor_backspace_removes_whole_utf8_character() {
        let mut editor = LineEditor::default();
        editor.feed("中文".as_bytes());
        editor.feed(&[0x7f]);
        let (to_shell, _) = editor.feed(b"\r");
        assert_eq!(to_shell, "中\r\n".as_bytes());
    }

    /// 行首退格不能删到别的东西，也不能 panic。
    #[test]
    fn line_editor_backspace_on_empty_line_is_noop() {
        let mut editor = LineEditor::default();
        let (to_shell, echo) = editor.feed(&[0x08]);
        assert!(to_shell.is_empty());
        assert!(echo.is_empty());
    }

    /// Ctrl+C 丢弃未提交的行，并让 shell 收到中断。
    #[test]
    fn line_editor_ctrl_c_drops_pending_line() {
        let mut editor = LineEditor::default();
        editor.feed(b"rm -rf /");
        let (to_shell, echo) = editor.feed(&[0x03]);
        assert_eq!(to_shell, vec![0x03]);
        assert_eq!(echo, b"^C\r\n");
        // 被丢弃的内容不会再被提交。
        let (to_shell, _) = editor.feed(b"\r");
        assert_eq!(to_shell, b"\r\n");
    }

    /// Tab 之类的控制字符不缓冲也不回显，直接透传。
    #[test]
    fn line_editor_passes_other_control_bytes_through() {
        let mut editor = LineEditor::default();
        let (to_shell, echo) = editor.feed(&[b'\t']);
        assert_eq!(to_shell, vec![b'\t']);
        assert!(echo.is_empty());
    }

    /// 方向键整段丢弃，不能让 `[A` 混进命令行。
    #[test]
    fn line_editor_drops_csi_sequences() {
        let mut editor = LineEditor::default();
        editor.feed(b"dir");
        let (to_shell, echo) = editor.feed(b"\x1b[A\x1b[1;5C");
        assert!(to_shell.is_empty());
        assert!(echo.is_empty());
        let (to_shell, _) = editor.feed(b"\r");
        assert_eq!(to_shell, b"dir\r\n");
    }

    #[test]
    fn line_editor_drops_ss3_sequences() {
        let mut editor = LineEditor::default();
        editor.feed(b"a");
        let (to_shell, echo) = editor.feed(b"\x1bOH");
        assert!(to_shell.is_empty());
        assert!(echo.is_empty());
        assert_eq!(editor.feed(b"\r").0, b"a\r\n");
    }

    /// 转义序列被拆成多条输入消息时状态要保持。
    #[test]
    fn line_editor_escape_state_survives_split_feeds() {
        let mut editor = LineEditor::default();
        editor.feed(b"x");
        editor.feed(b"\x1b");
        editor.feed(b"[");
        editor.feed(b"1;");
        editor.feed(b"5A");
        assert_eq!(editor.feed(b"\r").0, b"x\r\n");
    }

    /// ESC 后跟其他字节时只丢 ESC，后面的字节照常处理。
    #[test]
    fn line_editor_lone_escape_drops_only_esc() {
        let mut editor = LineEditor::default();
        let (_, echo) = editor.feed(b"\x1bq");
        assert_eq!(echo, b"q");
        assert_eq!(editor.feed(b"\r").0, b"q\r\n");
    }

    fn gbk_lead(byte: u8) -> bool {
        (0x81..=0xfe).contains(&byte)
    }

    /// 尾字节 D0 与前导字节范围重叠：整个「中」都完整，不能扣留。
    #[test]
    fn dbcs_prefix_does_not_hold_trail_byte_that_looks_like_lead() {
        assert_eq!(complete_dbcs_prefix(&[0xd6, 0xd0], gbk_lead), 2);
        assert_eq!(complete_dbcs_prefix(&[0xd6, 0xd0, 0xd6, 0xd0], gbk_lead), 4);
    }

    #[test]
    fn dbcs_prefix_holds_only_a_dangling_lead_byte() {
        assert_eq!(complete_dbcs_prefix(&[b'a', 0xd6], gbk_lead), 1);
        assert_eq!(complete_dbcs_prefix(&[0xd6, 0xd0, 0xd6], gbk_lead), 2);
        assert_eq!(complete_dbcs_prefix(&[0xd6], gbk_lead), 0);
        assert_eq!(complete_dbcs_prefix(b"ascii", gbk_lead), 5);
        assert_eq!(complete_dbcs_prefix(&[], gbk_lead), 0);
    }

    #[test]
    fn quotes_arguments_that_contain_spaces() {
        let mut line = Vec::new();
        push_quoted(&mut line, OsStr::new(r"C:\Windows\System32\cmd.exe"));
        line.push(b' ' as u16);
        push_quoted(
            &mut line,
            OsStr::new(r#"& 'C:\Windows\System32\chcp.com' 65001 > $null"#),
        );
        let text = String::from_utf16(&line).unwrap();
        assert_eq!(
            text,
            r#""C:\Windows\System32\cmd.exe" "& 'C:\Windows\System32\chcp.com' 65001 > $null""#
        );
    }
}

impl super::TerminalSlot for PipeSession {
    fn write(&self, data: &[u8]) -> Result<(), TerminalError> {
        PipeSession::write(self, data)
    }
    fn resize(&self, _size: TerminalSize) -> Result<(), TerminalError> {
        // 管道没有控制台窗口。改尺寸不能失败，否则浏览器会把一次 resize 当成会话错误。
        Ok(())
    }
    fn read_since(
        &self,
        last_seq: u64,
        max_bytes: usize,
    ) -> Result<Vec<OutputChunk>, TerminalError> {
        PipeSession::read_since(self, last_seq, max_bytes)
    }
    fn exit_code(&self) -> Option<u32> {
        PipeSession::exit_code(self)
    }
    fn expiry(&self, policy: &super::SessionPolicy, now: Instant) -> Option<super::Expiry> {
        policy.expiry(self.started, *lock(&self.last_activity), now)
    }
    fn close(mut self: Box<Self>) -> Result<Option<u32>, TerminalError> {
        self.shutdown()
    }
    fn finish_output(&self) {
        PipeSession::finish_output(self)
    }
    fn output_settled(&self) -> bool {
        PipeSession::output_settled(self)
    }
}


