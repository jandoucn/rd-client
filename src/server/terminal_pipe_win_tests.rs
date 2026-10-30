//! 用真实 cmd / PowerShell 进程验证管道 shell。win10-gs 有 ConPTY，这里直接调用管道实现，
//! 不经过 `conpty_available()` 分流。

use super::*;
use std::{
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

fn system32(name: &str) -> String {
    let root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".to_owned());
    format!(r"{root}\System32\{name}")
}

fn cmd() -> String {
    system32("cmd.exe")
}

fn powershell() -> String {
    system32(r"WindowsPowerShell\v1.0\powershell.exe")
}

/// 读线程把输出累积起来，测试按需等待某段文字出现。
struct Collector {
    rx: mpsc::Receiver<Vec<u8>>,
    text: Vec<u8>,
}

impl Collector {
    fn start(mut reader: PipeReader) -> Self {
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let mut buf = [0u8; 4096];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        if tx.send(buf[..n].to_vec()).is_err() {
                            break;
                        }
                    }
                }
            }
        });
        Self {
            rx,
            text: Vec::new(),
        }
    }

    fn wait_for(&mut self, needle: &str) -> String {
        self.wait_for_count(needle, 1)
    }

    fn wait_for_count(&mut self, needle: &str, count: usize) -> String {
        let deadline = Instant::now() + Duration::from_secs(20);
        while Instant::now() < deadline {
            if String::from_utf8_lossy(&self.text).matches(needle).count() >= count {
                return String::from_utf8_lossy(&self.text).into_owned();
            }
            if let Ok(chunk) = self.rx.recv_timeout(Duration::from_millis(100)) {
                self.text.extend_from_slice(&chunk);
            }
        }
        panic!(
            "timed out waiting for {needle:?}; output so far: {:?}",
            String::from_utf8_lossy(&self.text)
        );
    }

    /// 读端在 shell 结束后必须返回 EOF，否则会话关闭时 join 读线程会卡死。
    fn wait_eof(&mut self) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            match self.rx.recv_timeout(Duration::from_millis(100)) {
                Ok(chunk) => self.text.extend_from_slice(&chunk),
                Err(mpsc::RecvTimeoutError::Disconnected) => return,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
        }
        panic!("reader did not reach EOF");
    }
}

fn wait_exit(child: &mut PipeChild) -> Option<ExitStatus> {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if let Some(status) = child.try_wait().expect("try_wait") {
            return Some(status);
        }
        thread::sleep(Duration::from_millis(50));
    }
    None
}

#[test]
fn cmd_runs_a_command_typed_with_bare_cr() {
    let shell = PipeShell::spawn(&cmd(), None).expect("spawn cmd on pipes");
    let mut output = Collector::start(shell.reader);
    let mut writer = shell.writer;
    writer.write_all(b"echo pipe-ok-1\r").unwrap();
    // 一次是本地回显，一次是 cmd 真正执行后的输出。
    output.wait_for_count("pipe-ok-1\r\n", 2);
    let mut child = shell.child;
    assert!(child.process_id().unwrap_or(0) > 0);
    drop(writer);
    assert!(
        wait_exit(&mut child).is_some(),
        "dropping the writer must end the shell"
    );
    output.wait_eof();
}

/// cmd 逐字不回显：本端必须回显按键，退格要真的删掉字符。
#[test]
fn cmd_echoes_locally_and_applies_backspace() {
    let shell = PipeShell::spawn(&cmd(), None).expect("spawn cmd on pipes");
    let mut output = Collector::start(shell.reader);
    let mut writer = shell.writer;
    writer.write_all(b"echx").unwrap();
    output.wait_for("echx");
    writer.write_all(&[0x08]).unwrap();
    writer.write_all(b"o back-ok\r").unwrap();
    let text = output.wait_for_count("back-ok\r\n", 2);
    assert!(!text.contains("'echxo'"), "{text}");
}

/// 系统代码页（GBK 等）与 UTF-8 双向转码：中文发进去、原样回来。
#[test]
fn cmd_round_trips_chinese_text() {
    let shell = PipeShell::spawn(&cmd(), None).expect("spawn cmd on pipes");
    let mut output = Collector::start(shell.reader);
    let mut writer = shell.writer;
    writer.write_all("echo 管道中文-ok\r".as_bytes()).unwrap();
    // 第一次出现是本地回显，第二次才是 cmd 经 GBK 往返后的输出。
    output.wait_for_count("管道中文-ok", 2);
}

#[test]
fn powershell_runs_commands_without_console() {
    let shell = PipeShell::spawn(&powershell(), None).expect("spawn powershell on pipes");
    let mut output = Collector::start(shell.reader);
    let mut writer = shell.writer;
    writer
        .write_all(b"Write-Output ('ps-' + 'pipe-ok')\r")
        .unwrap();
    output.wait_for("ps-pipe-ok");
}

/// 传入的完整环境就是 shell 的环境。
#[test]
fn given_environment_reaches_the_shell() {
    let mut builder = CommandBuilder::new(cmd());
    builder.env("TEMP", r"C:\pipe-test-temp");
    let environment = shell_environment(&builder);
    let shell = PipeShell::spawn(&cmd(), Some(&environment)).expect("spawn cmd on pipes");
    let mut output = Collector::start(shell.reader);
    let mut writer = shell.writer;
    writer.write_all(b"echo [%TEMP%] [%SystemRoot%]\r").unwrap();
    // 本地回显里有原样的 `%SystemRoot%`，只看 cmd 展开后的那一行。
    let text = output.wait_for("[C:\\pipe-test-temp] [");
    assert!(
        !text.contains("[C:\\pipe-test-temp] [%SystemRoot%]"),
        "{text}"
    );
}

#[test]
fn kill_ends_the_shell_and_its_children() {
    let shell = PipeShell::spawn(&cmd(), None).expect("spawn cmd on pipes");
    let mut output = Collector::start(shell.reader);
    let mut writer = shell.writer;
    let mut child = shell.child;
    // 派生一个长时间运行的子进程；结束 Job 后整棵树都不能留下。
    writer.write_all(b"ping -n 60 127.0.0.1\r").unwrap();
    output.wait_for_count("127.0.0.1", 2);
    let mut killer = child.clone_killer();
    killer.kill().unwrap();
    assert!(wait_exit(&mut child).is_some());
    drop(writer);
    output.wait_eof();
}

#[test]
fn environment_block_dedups_case_insensitively_and_terminates() {
    let vars = vec![
        (OsString::from("Path"), OsString::from(r"C:\bin")),
        (OsString::from("temp"), OsString::from(r"C:\old")),
        (OsString::from("TEMP"), OsString::from(r"C:\Windows\Temp")),
    ];
    let block = environment_block(&vars);
    assert_eq!(&block[block.len() - 2..], &[0, 0]);
    let text = String::from_utf16_lossy(&block[..block.len() - 1]);
    let vars: Vec<&str> = text.split('\0').filter(|item| !item.is_empty()).collect();
    assert_eq!(vars, vec![r"Path=C:\bin", r"TEMP=C:\Windows\Temp"]);
}

#[test]
fn quotes_arguments_that_contain_spaces() {
    let mut line = Vec::new();
    push_quoted(
        &mut line,
        OsStr::new(r"C:\Program Files\PowerShell\7\pwsh.exe"),
    );
    line.push(b' ' as u16);
    push_quoted(&mut line, OsStr::new(r#"a "b" c\"#));
    assert_eq!(
        String::from_utf16(&line).unwrap(),
        r#""C:\Program Files\PowerShell\7\pwsh.exe" "a \"b\" c\\""#
    );
}

/// SYSTEM 的注册表 TEMP 指向不存在的目录时，换成系统 Temp（TEMP 与 TMP 都换）。
#[test]
fn temp_override_replaces_missing_temp_in_command_builder() {
    let mut builder = CommandBuilder::new(cmd());
    builder.env(
        "TEMP",
        r"C:\Windows\system32\config\systemprofile\no-such-temp-dir",
    );
    apply_temp_override(&mut builder);
    let root = builder
        .get_env("SystemRoot")
        .expect("SystemRoot")
        .to_owned();
    let expected = Path::new(&root).join("Temp");
    assert_eq!(builder.get_env("TEMP"), Some(expected.as_os_str()));
    assert_eq!(builder.get_env("TMP"), Some(expected.as_os_str()));
}

#[test]
fn temp_override_keeps_existing_temp() {
    let mut builder = CommandBuilder::new(cmd());
    let existing = std::env::temp_dir();
    builder.env("TEMP", &existing);
    builder.env("TMP", r"C:\keep-me");
    apply_temp_override(&mut builder);
    assert_eq!(builder.get_env("TEMP"), Some(existing.as_os_str()));
    assert_eq!(builder.get_env("TMP"), Some(OsStr::new(r"C:\keep-me")));
}

/// 环境与 ConPTY 路径同源：CommandBuilder 合并注册表后的变量都在。
#[test]
fn shell_environment_carries_builder_variables() {
    let builder = CommandBuilder::new(cmd());
    let environment = shell_environment(&builder);
    let has = |name: &str| {
        environment
            .iter()
            .any(|(key, _)| key.to_string_lossy().eq_ignore_ascii_case(name))
    };
    assert!(has("Path"));
    assert!(has("SystemRoot"));
}

/// 系统代码页为 936 时，块尾的尾字节不能被扣住，汉字要完整转出。
#[test]
fn ansi_codec_converts_gbk_split_at_character_boundary() {
    let mut codec = AnsiCodec {
        codepage: 936,
        pending: Vec::new(),
    };
    assert_eq!(codec.decode(&[0xd6, 0xd0]), "中".as_bytes());
    assert_eq!(codec.decode(&[0xce]), Vec::<u8>::new());
    assert_eq!(codec.decode(&[0xc4]), "文".as_bytes());
}
