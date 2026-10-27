//! `system.run`：以 SYSTEM 身份执行操作者给出的命令。
//!
//! 本私人定制项目里 agent 常驻 SYSTEM，用于远程维护自有设备；这里不做命令
//! 白名单，只做参数结构、长度和超时的边界校验，避免畸形输入打崩执行层。
//! 是否允许执行由上层本地门禁与后台权限决定，不在这里判断。

use std::time::Duration;

pub const MAX_TIMEOUT_SECONDS: u64 = 900;
pub const MIN_TIMEOUT_SECONDS: u64 = 1;
pub const MAX_OUTPUT_BYTES: usize = 32 * 1024;
pub const MAX_COMMAND_CHARS: usize = 32_767;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shell {
    /// 系统自带 Windows PowerShell 5.1；装了 PowerShell 7 时优先用 pwsh.exe。
    PowerShell,
    /// 明确要求 PowerShell 7（pwsh.exe）；不存在时报错，不回退。
    Pwsh,
    Cmd,
}

impl Shell {
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "powershell" => Self::PowerShell,
            "pwsh" => Self::Pwsh,
            "cmd" => Self::Cmd,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::PowerShell => "powershell",
            Self::Pwsh => "pwsh",
            Self::Cmd => "cmd",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    pub shell: Shell,
    pub command: String,
    pub timeout: Duration,
    /// 脚本的工作目录；为 None 时用系统目录。
    pub workdir: Option<std::path::PathBuf>,
}

/// 解析并校验 `system.run` 的参数。
///
/// `envelope_timeout` 来自命令信封，作为上限：请求值再大也不能超过它。
pub fn parse_args(
    args: &serde_json::Value,
    envelope_timeout: u64,
) -> Result<Request, &'static str> {
    let Some(object) = args.as_object() else {
        return Err("run_parameters_invalid");
    };
    if object
        .keys()
        .any(|key| !matches!(key.as_str(), "shell" | "command" | "timeout_seconds" | "workdir"))
    {
        return Err("run_parameters_invalid");
    }
    let shell = match object.get("shell").and_then(serde_json::Value::as_str) {
        Some(value) => Shell::parse(value).ok_or("run_shell_unsupported")?,
        None => Shell::PowerShell,
    };
    let command = object
        .get("command")
        .and_then(serde_json::Value::as_str)
        .ok_or("run_command_missing")?
        .trim();
    if command.is_empty() {
        return Err("run_command_missing");
    }
    if command.chars().count() > MAX_COMMAND_CHARS {
        return Err("run_command_too_long");
    }
    if command.chars().any(|c| c == '\0') {
        return Err("run_command_invalid");
    }
    let envelope_timeout = envelope_timeout.clamp(MIN_TIMEOUT_SECONDS, MAX_TIMEOUT_SECONDS);
    let requested = object
        .get("timeout_seconds")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(envelope_timeout);
    let seconds = requested
        .clamp(MIN_TIMEOUT_SECONDS, MAX_TIMEOUT_SECONDS)
        .min(envelope_timeout);
    let workdir = match object.get("workdir") {
        None | Some(serde_json::Value::Null) => None,
        Some(serde_json::Value::String(path)) => {
            let path = path.trim();
            if path.is_empty() || path.chars().any(|c| c == '\0') {
                return Err("run_workdir_invalid");
            }
            Some(std::path::PathBuf::from(path))
        }
        Some(_) => return Err("run_workdir_invalid"),
    };
    Ok(Request {
        shell,
        command: command.to_owned(),
        timeout: Duration::from_secs(seconds),
        workdir,
    })
}

/// 截断到 `MAX_OUTPUT_BYTES`，按字符边界回退，避免切坏 UTF-8。
pub fn truncate_output(bytes: &[u8]) -> (String, bool) {
    let slice = if bytes.len() > MAX_OUTPUT_BYTES {
        &bytes[..MAX_OUTPUT_BYTES]
    } else {
        bytes
    };
    let text = String::from_utf8_lossy(slice).into_owned();
    (text, bytes.len() > MAX_OUTPUT_BYTES)
}

/// 命令结果。字段名与后端展示一致，不随实现变化。
pub fn result_json(
    shell: Shell,
    exit_code: Option<i32>,
    timed_out: bool,
    duration: Duration,
    stdout: &[u8],
    stderr: &[u8],
) -> serde_json::Value {
    let (out, out_cut) = truncate_output(stdout);
    let (err, err_cut) = truncate_output(stderr);
    serde_json::json!({
        "schema": 1,
        "shell": shell.name(),
        "exit_code": exit_code,
        "timed_out": timed_out,
        "duration_ms": duration.as_millis() as u64,
        "stdout": out,
        "stderr": err,
        "stdout_truncated": out_cut,
        "stderr_truncated": err_cut,
    })
}

#[cfg(windows)]
pub fn execute(request: &Request) -> Result<serde_json::Value, String> {
    use std::{
        io::Read,
        os::windows::process::CommandExt,
        process::{Command, Stdio},
        sync::mpsc,
        thread,
        time::Instant,
    };

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    let ps_args = |c: &mut Command| {
        c.args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
        ])
        .arg(&request.command);
    };
    let mut command = match request.shell {
        Shell::PowerShell => {
            // 装了 PowerShell 7 就用它的完整 shell，否则回退系统自带 5.1。
            let mut c = if super::system_dirs::get()
                .program_files
                .join(r"PowerShell\7\pwsh.exe")
                .is_file()
            {
                Command::new("pwsh.exe")
            } else {
                Command::new("powershell.exe")
            };
            ps_args(&mut c);
            c
        }
        Shell::Pwsh => {
            let mut c = Command::new("pwsh.exe");
            ps_args(&mut c);
            c
        }
        Shell::Cmd => {
            let mut c = Command::new("cmd.exe");
            c.arg("/c").arg(&request.command);
            c
        }
    };
    if let Some(dir) = &request.workdir {
        if !dir.is_dir() {
            return Err("run_workdir_missing".into());
        }
        command.current_dir(dir);
    }
    let mut child = command
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("run_spawn_failed:{error}"))?;

    let (sender, receiver) = mpsc::channel();
    for (index, stream) in [
        child.stdout.take().map(|s| Box::new(s) as Box<dyn Read + Send>),
        child.stderr.take().map(|s| Box::new(s) as Box<dyn Read + Send>),
    ]
    .into_iter()
    .enumerate()
    {
        let Some(mut stream) = stream else { continue };
        let sender = sender.clone();
        thread::spawn(move || {
            let mut buffer = Vec::new();
            let mut chunk = [0u8; 8192];
            loop {
                match stream.read(&mut chunk) {
                    Ok(0) | Err(_) => break,
                    Ok(read) => {
                        // 超过上限后继续读，避免子进程因管道写满而卡死。
                        if buffer.len() < MAX_OUTPUT_BYTES * 2 {
                            buffer.extend_from_slice(&chunk[..read]);
                        }
                    }
                }
            }
            let _ = sender.send((index, buffer));
        });
    }
    drop(sender);

    let started = Instant::now();
    let mut timed_out = false;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) => {}
            Err(error) => return Err(format!("run_wait_failed:{error}")),
        }
        if started.elapsed() >= request.timeout {
            timed_out = true;
            let _ = child.kill();
            let _ = child.wait();
            break None;
        }
        thread::sleep(Duration::from_millis(50));
    };

    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(2);
    for _ in 0..2 {
        let remaining = deadline.saturating_duration_since(Instant::now());
        match receiver.recv_timeout(remaining.max(Duration::from_millis(1))) {
            Ok((0, buffer)) => stdout = buffer,
            Ok((_, buffer)) => stderr = buffer,
            Err(_) => {}
        }
    }
    let exit_code = status.and_then(|status| status.code());
    Ok(result_json(
        request.shell,
        exit_code,
        timed_out,
        started.elapsed(),
        &stdout,
        &stderr,
    ))
}

#[cfg(not(windows))]
pub fn execute(_request: &Request) -> Result<serde_json::Value, String> {
    Err("unsupported agent operation".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_to_powershell_and_uses_envelope_timeout() {
        let request = parse_args(
            &serde_json::json!({"command": "Get-Date"}),
            30,
        )
        .unwrap();
        assert_eq!(request.shell, Shell::PowerShell);
        assert_eq!(request.command, "Get-Date");
        assert_eq!(request.timeout, Duration::from_secs(30));
    }

    #[test]
    fn accepts_cmd_and_explicit_timeout_within_the_envelope() {
        let request = parse_args(
            &serde_json::json!({"shell": "cmd", "command": "ipconfig /all", "timeout_seconds": 10}),
            60,
        )
        .unwrap();
        assert_eq!(request.shell, Shell::Cmd);
        assert_eq!(request.timeout, Duration::from_secs(10));
    }

    #[test]
    fn timeout_is_capped_by_the_envelope_and_the_hard_limit() {
        let capped_by_envelope =
            parse_args(&serde_json::json!({"command": "x", "timeout_seconds": 600}), 20).unwrap();
        assert_eq!(capped_by_envelope.timeout, Duration::from_secs(20));
        let capped_by_limit = parse_args(
            &serde_json::json!({"command": "x", "timeout_seconds": 999_999}),
            MAX_TIMEOUT_SECONDS,
        )
        .unwrap();
        assert_eq!(
            capped_by_limit.timeout,
            Duration::from_secs(MAX_TIMEOUT_SECONDS)
        );
        let zero = parse_args(&serde_json::json!({"command": "x", "timeout_seconds": 0}), 60).unwrap();
        assert_eq!(zero.timeout, Duration::from_secs(MIN_TIMEOUT_SECONDS));
    }

    #[test]
    fn rejects_unknown_keys_and_bad_shapes() {
        for args in [
            serde_json::json!({}),
            serde_json::json!([]),
            serde_json::json!("Get-Date"),
            serde_json::json!({"command": "x", "extra": 1}),
            serde_json::json!({"shell": "bash", "command": "x"}),
            serde_json::json!({"command": "   "}),
            serde_json::json!({"command": 42}),
            serde_json::json!({"command": "a\u{0}b"}),
        ] {
            assert!(parse_args(&args, 60).is_err(), "{args}");
        }
        let long = "a".repeat(MAX_COMMAND_CHARS + 1);
        assert_eq!(
            parse_args(&serde_json::json!({"command": long}), 60),
            Err("run_command_too_long")
        );
    }

    #[test]
    fn multiline_scripts_are_accepted_as_is() {
        let script = "Get-Process | Where-Object { $_.Name -like 'RustDesk*' }\nGet-Service RustDesk";
        let request = parse_args(&serde_json::json!({"command": script}), 60).unwrap();
        assert_eq!(request.command, script);
    }

    #[test]
    fn output_is_truncated_and_flagged() {
        let short = truncate_output(b"ok");
        assert_eq!(short, ("ok".to_owned(), false));
        let long = vec![b'a'; MAX_OUTPUT_BYTES + 10];
        let (text, cut) = truncate_output(&long);
        assert!(cut);
        assert_eq!(text.len(), MAX_OUTPUT_BYTES);
    }

    #[test]
    fn result_shape_is_stable() {
        let value = result_json(
            Shell::PowerShell,
            Some(0),
            false,
            Duration::from_millis(1500),
            b"hello",
            b"",
        );
        assert_eq!(value["schema"], 1);
        assert_eq!(value["shell"], "powershell");
        assert_eq!(value["exit_code"], 0);
        assert_eq!(value["timed_out"], false);
        assert_eq!(value["duration_ms"], 1500);
        assert_eq!(value["stdout"], "hello");
        assert_eq!(value["stderr"], "");
        assert_eq!(value["stdout_truncated"], false);
    }

    #[test]
    fn timeout_is_reported_without_an_exit_code() {
        let value = result_json(Shell::Cmd, None, true, Duration::from_secs(5), b"", b"");
        assert_eq!(value["timed_out"], true);
        assert_eq!(value["exit_code"], serde_json::Value::Null);
    }
}
