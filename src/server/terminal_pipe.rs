//! 没有 ConPTY 的 Windows（Server 2016 等）上客户端远程终端的管道备选：纯逻辑部分。
//!
//! 与 Management Agent 的网页终端（`src/management_agent/terminal/console_host.rs`）同一套
//! 实测结论，但 Agent 代码只编进 Agent 二进制，这里按客户端终端的接口单独实现。
//! Windows API 相关部分在 `terminal_pipe_win.rs`。

use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
};

/// 管道模式下的 shell 启动参数。
#[derive(Debug, PartialEq, Eq)]
pub struct PipeShellArgs {
    pub args: Vec<String>,
    /// 由本进程做行编辑与回显，只把整行交给 shell（cmd 需要）。
    pub local_echo: bool,
}

/// 管道没有控制台：不能用 ConPTY 下设置 `[Console]` 编码或 `chcp >NUL` 的启动命令。
/// cmd 在管道输入下逐字不回显、退格只当普通字节，所以用 `/Q` 关掉它的整行回显、
/// 改由本进程编辑；`/K` 不带载荷，否则会吞掉 stdin 的第一条命令。
pub fn pipe_shell_args(shell: &str) -> PipeShellArgs {
    let name = shell
        .rsplit(['\\', '/'])
        .next()
        .unwrap_or(shell)
        .to_ascii_lowercase();
    match name.as_str() {
        "cmd.exe" | "cmd" => PipeShellArgs {
            args: vec!["/Q".to_owned(), "/K".to_owned()],
            local_echo: true,
        },
        "pwsh.exe" | "pwsh" | "powershell.exe" | "powershell" => PipeShellArgs {
            args: vec!["-NoLogo".to_owned(), "-NoExit".to_owned()],
            local_echo: false,
        },
        _ => PipeShellArgs {
            args: Vec::new(),
            local_echo: false,
        },
    }
}

/// shell 的 TEMP 目录不存在时改用 `%SystemRoot%\Temp`。
///
/// SYSTEM 令牌生成的环境块把 TEMP 指到 `systemprofile\AppData\Local\Temp`，很多机器上
/// 这个目录不存在，下载到 `$env:TEMP` 的命令直接失败。系统 Temp 也不存在时不改。
pub fn temp_dir_override(
    temp: Option<&OsStr>,
    system_root: Option<&OsStr>,
    exists: impl Fn(&Path) -> bool,
) -> Option<PathBuf> {
    if temp.is_some_and(|temp| !temp.is_empty() && exists(Path::new(temp))) {
        return None;
    }
    let fallback = Path::new(system_root?).join("Temp");
    exists(&fallback).then_some(fallback)
}

/// 把裸 CR 补成 CRLF（输入方向）。管道下的 cmd 收到裸 CR 时整条命令不执行。
pub fn normalize_enter(data: &[u8]) -> Vec<u8> {
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

/// 一个字符在终端里占几列。CJK 与全角字符占两列，退格时要擦两格。
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

/// 正在跳过的终端转义序列。
#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Escape {
    #[default]
    None,
    Esc,
    /// `ESC [`，到 0x40..=0x7E 的终止字节结束。
    Csi,
    /// `ESC O`，后面再跟一个字节。
    Ss3,
}

/// cmd 管道会话的本地行编辑：按键先回显，回车时整行交给 shell。
#[derive(Default)]
pub struct LineEditor {
    line: Vec<u8>,
    /// 上一个字节是 CR：紧跟的 LF 要吞掉，避免同一行被提交两次。
    swallow_lf: bool,
    /// 方向键、Home 等发送的转义序列管道下的 cmd 处理不了，整段丢弃，可能跨多次输入。
    escape: Escape,
}

impl LineEditor {
    /// 返回 (写给 shell 的字节, 回显给远端的字节)。
    pub fn feed(&mut self, data: &[u8]) -> (Vec<u8>, Vec<u8>) {
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
                    self.swallow_lf = byte == b'\r';
                    to_shell.extend_from_slice(&self.line);
                    to_shell.extend_from_slice(b"\r\n");
                    echo.extend_from_slice(b"\r\n");
                    self.line.clear();
                }
                0x08 | 0x7f => self.backspace(&mut echo),
                0x03 => {
                    self.line.clear();
                    echo.extend_from_slice(b"^C\r\n");
                    to_shell.push(0x03);
                }
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
        let mut start = self.line.len() - 1;
        while start > 0 && self.line[start] & 0xc0 == 0x80 {
            start -= 1;
        }
        let removed_width = std::str::from_utf8(&self.line[start..])
            .ok()
            .and_then(|text| text.chars().next())
            .map(display_width)
            .unwrap_or(1);
        self.line.truncate(start);
        for _ in 0..removed_width {
            echo.extend_from_slice(b"\x08 \x08");
        }
    }
}

/// 输入方向的 UTF-8 拼接：一条消息末尾不完整的字符先扣住，与下一条拼齐再转码。
#[derive(Default)]
pub struct Utf8Carry {
    pending: Vec<u8>,
}

impl Utf8Carry {
    pub fn push(&mut self, data: &[u8]) -> Vec<u8> {
        let mut buf = std::mem::take(&mut self.pending);
        buf.extend_from_slice(data);
        let keep = incomplete_utf8_tail(&buf);
        self.pending = buf.split_off(buf.len() - keep);
        buf
    }
}

/// 末尾那个尚未收全的 UTF-8 字符占几个字节；非法字节不扣。
fn incomplete_utf8_tail(buf: &[u8]) -> usize {
    for back in 1..=buf.len().min(3) {
        let byte = buf[buf.len() - back];
        if byte & 0xc0 == 0x80 {
            continue;
        }
        let needed = match byte {
            0xc0..=0xdf => 2,
            0xe0..=0xef => 3,
            0xf0..=0xf7 => 4,
            _ => return 0,
        };
        return if needed > back { back } else { 0 };
    }
    0
}

/// 从头成对扫描，返回完整字符的字节数，只把末尾落单的前导字节留下。
/// 不能只看最后一个字节：GBK 尾字节 0x40..=0xFE 与前导字节 0x81..=0xFE 重叠，
/// 「中」= D6 D0 的尾字节 D0 也会被判成前导字节。
pub fn complete_dbcs_prefix(bytes: &[u8], is_lead: impl Fn(u8) -> bool) -> usize {
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

/// 把 shell 输出里孤立的 CR 补成 CRLF，跨块保持状态。
#[derive(Default)]
pub struct CrlfNormalizer {
    pending_cr: bool,
}

impl CrlfNormalizer {
    pub fn normalize(&mut self, chunk: &[u8]) -> Vec<u8> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cmd_in_pipe_turns_off_shell_echo_and_edits_locally() {
        let args = pipe_shell_args(r"C:\Windows\System32\cmd.exe");
        assert_eq!(args.args, vec!["/Q".to_owned(), "/K".to_owned()]);
        assert!(args.local_echo);
    }

    /// 管道没有控制台：不能带设置 [Console] 编码的 -Command，否则 PowerShell 报错。
    #[test]
    fn powershell_in_pipe_has_no_console_encoding_command() {
        for shell in [
            r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe",
            r"C:\Program Files\PowerShell\7\pwsh.exe",
            "pwsh.exe",
        ] {
            let args = pipe_shell_args(shell);
            assert_eq!(args.args, vec!["-NoLogo".to_owned(), "-NoExit".to_owned()]);
            assert!(!args.local_echo, "{shell}");
        }
    }

    #[test]
    fn unknown_shell_gets_no_extra_args() {
        let args = pipe_shell_args(r"C:\tools\bash.exe");
        assert!(args.args.is_empty());
        assert!(!args.local_echo);
    }

    fn exists_only(paths: &[&str]) -> impl Fn(&Path) -> bool {
        let paths: Vec<PathBuf> = paths.iter().map(PathBuf::from).collect();
        move |path| paths.iter().any(|item| item == path)
    }

    /// SYSTEM 的环境块把 TEMP 指到 systemprofile 下，很多机器上这个目录不存在，
    /// `iwr -OutFile $env:TEMP\x` 直接失败。改用系统 Temp。
    #[test]
    fn missing_temp_dir_falls_back_to_system_temp() {
        let temp = r"C:\Windows\system32\config\systemprofile\AppData\Local\Temp";
        assert_eq!(
            temp_dir_override(
                Some(temp.as_ref()),
                Some(r"C:\Windows".as_ref()),
                exists_only(&[r"C:\Windows/Temp", r"C:\Windows\Temp"])
            ),
            Some(Path::new(r"C:\Windows").join("Temp"))
        );
    }

    #[test]
    fn existing_temp_dir_is_left_alone() {
        let temp = r"C:\Users\olly\AppData\Local\Temp";
        assert_eq!(
            temp_dir_override(
                Some(temp.as_ref()),
                Some(r"C:\Windows".as_ref()),
                exists_only(&[temp, r"C:\Windows/Temp", r"C:\Windows\Temp"])
            ),
            None
        );
    }

    #[test]
    fn unset_temp_falls_back_to_system_temp() {
        assert_eq!(
            temp_dir_override(
                None,
                Some(r"D:\Win".as_ref()),
                exists_only(&[r"D:\Win/Temp", r"D:\Win\Temp"])
            ),
            Some(Path::new(r"D:\Win").join("Temp"))
        );
    }

    /// 系统 Temp 也不存在时不改，免得把一个坏路径换成另一个坏路径。
    #[test]
    fn no_override_when_system_temp_is_missing_too() {
        assert_eq!(
            temp_dir_override(
                Some(r"C:\nope".as_ref()),
                Some(r"C:\Windows".as_ref()),
                exists_only(&[])
            ),
            None
        );
        assert_eq!(
            temp_dir_override(Some(r"C:\nope".as_ref()), None, exists_only(&[])),
            None
        );
    }

    #[test]
    fn normalize_enter_turns_bare_cr_into_crlf() {
        assert_eq!(normalize_enter(b"dir\r"), b"dir\r\n");
        assert_eq!(normalize_enter(b"a\r\nb\rc"), b"a\r\nb\r\nc");
        assert_eq!(normalize_enter(b"plain"), b"plain");
    }

    #[test]
    fn line_editor_echoes_typed_bytes_and_submits_on_enter() {
        let mut editor = LineEditor::default();
        let (to_shell, echo) = editor.feed(b"who");
        assert!(to_shell.is_empty());
        assert_eq!(echo, b"who");
        let (to_shell, echo) = editor.feed(b"ami\r");
        assert_eq!(to_shell, b"whoami\r\n");
        assert_eq!(echo, b"ami\r\n");
        assert!(editor.feed(b"x").0.is_empty());
    }

    #[test]
    fn line_editor_swallows_lf_right_after_cr() {
        let mut editor = LineEditor::default();
        editor.feed(b"dir\r");
        let (to_shell, echo) = editor.feed(b"\n");
        assert!(to_shell.is_empty());
        assert!(echo.is_empty());
    }

    #[test]
    fn line_editor_backspace_removes_one_character() {
        let mut editor = LineEditor::default();
        editor.feed(b"whx");
        let (to_shell, echo) = editor.feed(&[0x08]);
        assert!(to_shell.is_empty());
        assert_eq!(echo, b"\x08 \x08");
        assert_eq!(editor.feed(b"o\r").0, b"who\r\n");
    }

    /// 删一个汉字要删掉完整的 UTF-8 序列，并擦两格，否则屏幕留半个字。
    #[test]
    fn line_editor_backspace_removes_whole_wide_character() {
        let mut editor = LineEditor::default();
        editor.feed("中文".as_bytes());
        let (_, echo) = editor.feed(&[0x7f]);
        assert_eq!(echo, b"\x08 \x08\x08 \x08");
        assert_eq!(editor.feed(b"\r").0, "中\r\n".as_bytes());
    }

    #[test]
    fn line_editor_backspace_on_empty_line_is_noop() {
        let mut editor = LineEditor::default();
        let (to_shell, echo) = editor.feed(&[0x08]);
        assert!(to_shell.is_empty());
        assert!(echo.is_empty());
    }

    #[test]
    fn line_editor_ctrl_c_drops_pending_line() {
        let mut editor = LineEditor::default();
        editor.feed(b"rm -rf /");
        let (to_shell, echo) = editor.feed(&[0x03]);
        assert_eq!(to_shell, vec![0x03]);
        assert_eq!(echo, b"^C\r\n");
        assert_eq!(editor.feed(b"\r").0, b"\r\n");
    }

    #[test]
    fn line_editor_passes_other_control_bytes_through() {
        let mut editor = LineEditor::default();
        let (to_shell, echo) = editor.feed(b"\t");
        assert_eq!(to_shell, vec![b'\t']);
        assert!(echo.is_empty());
    }

    /// shell 用孤立 CR 回到行首重画；xterm 收到会覆盖前一段输出，必须补 LF，且跨块成立。
    #[test]
    fn crlf_normalizer_completes_lone_cr_across_chunks() {
        let mut normalizer = CrlfNormalizer::default();
        assert_eq!(normalizer.normalize(b"a\rb"), b"a\r\nb");
        assert_eq!(normalizer.normalize(b"x\r"), b"x\r");
        assert_eq!(normalizer.normalize(b"\ny"), b"\ny");
        assert_eq!(normalizer.normalize(b"\r"), b"\r");
        assert_eq!(normalizer.normalize(b"z"), b"\nz");
    }

    /// 方向键是 `ESC [ A`：管道下的 cmd 无法处理，整段丢弃，不能把 `[A` 混进命令行。
    #[test]
    fn line_editor_drops_csi_and_ss3_sequences() {
        let mut editor = LineEditor::default();
        let (to_shell, echo) = editor.feed(b"di\x1b[Ar\x1bOD\x1b[1;5C\r");
        assert_eq!(to_shell, b"dir\r\n");
        assert_eq!(echo, b"dir\r\n");
    }

    /// 转义序列可能被拆在两次输入里。
    #[test]
    fn line_editor_drops_escape_sequences_split_across_feeds() {
        let mut editor = LineEditor::default();
        assert_eq!(editor.feed(b"a\x1b"), (Vec::new(), b"a".to_vec()));
        assert_eq!(editor.feed(b"["), (Vec::new(), Vec::new()));
        assert_eq!(editor.feed(b"3~b\r").0, b"ab\r\n");
    }

    /// ESC 后面不是 `[`/`O` 时只丢掉 ESC，后面的字符照常处理。
    #[test]
    fn line_editor_drops_lone_escape() {
        let mut editor = LineEditor::default();
        assert_eq!(editor.feed(b"\x1bx\r").0, b"x\r\n");
    }

    fn gbk_lead(byte: u8) -> bool {
        (0x81..=0xfe).contains(&byte)
    }

    /// 「中中」= D6 D0 D6 D0：块尾的 D0 是尾字节，不能被当成前导字节扣住。
    #[test]
    fn dbcs_prefix_keeps_trail_byte_that_looks_like_a_lead_byte() {
        assert_eq!(complete_dbcs_prefix(&[0xd6, 0xd0], gbk_lead), 2);
        assert_eq!(complete_dbcs_prefix(&[0xd6, 0xd0, 0xd6, 0xd0], gbk_lead), 4);
    }

    #[test]
    fn dbcs_prefix_holds_only_an_unpaired_trailing_lead_byte() {
        assert_eq!(complete_dbcs_prefix(&[b'a', 0xd6], gbk_lead), 1);
        assert_eq!(complete_dbcs_prefix(&[0xd6, 0xd0, 0xd6], gbk_lead), 2);
        assert_eq!(complete_dbcs_prefix(&[0xd6], gbk_lead), 0);
        assert_eq!(complete_dbcs_prefix(b"ascii", gbk_lead), 5);
        assert_eq!(complete_dbcs_prefix(&[], gbk_lead), 0);
    }

    /// 一个汉字的 UTF-8 字节被拆在两条消息里时，先扣住前半，凑齐再交出去，
    /// 否则转成 shell 代码页时会变成替换字符。
    #[test]
    fn utf8_carry_holds_incomplete_trailing_character() {
        let bytes = "中文".as_bytes();
        let mut carry = Utf8Carry::default();
        assert_eq!(carry.push(&bytes[..4]), bytes[..3].to_vec());
        assert_eq!(carry.push(&bytes[4..]), bytes[3..].to_vec());
        assert_eq!(carry.push(b"ok"), b"ok".to_vec());
    }

    /// 非法字节不能被无限扣住：只扣末尾最多 3 个可能组成完整字符的字节。
    #[test]
    fn utf8_carry_passes_invalid_bytes_through() {
        let mut carry = Utf8Carry::default();
        assert_eq!(carry.push(&[0xff, b'a']), vec![0xff, b'a']);
        assert_eq!(carry.push(&[0x80, 0x80]), vec![0x80, 0x80]);
    }
}
