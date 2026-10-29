use super::TerminalError;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShellKind {
    /// PowerShell 7 优先，未安装时用系统自带 Windows PowerShell 5.1。
    PowerShell,
    /// 明确要求 PowerShell 7；未安装时报错，不回退。
    Pwsh,
    Cmd,
}

impl ShellKind {
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
pub struct ShellCommand {
    pub program: PathBuf,
    pub args: Vec<String>,
    /// 由 agent 在本地做行编辑与回显，只把整行交给 shell。
    ///
    /// cmd 在管道输入下是纯行输入模式：不逐字回显，退格/方向键也只当普通字节。
    /// 实测 `wxyz` + 4 个退格 + `hoami` + 回车什么都不执行，所以编辑必须在本地完成。
    /// 配合 `/Q`（关掉 shell 自己的整行回显）才不会在按回车时看到两遍命令。
    pub local_echo: bool,
}

/// 终端承载方式决定启动参数能怎么给。
///
/// ConPTY 是真实伪控制台：shell 有控制台，可以设 `[Console]` 编码，也能用 `>NUL` 这类重定向。
/// 管道没有控制台：`-Command` 载荷会被 CreateProcessW 原样拼进命令行，含引号与 `>` 时
/// cmd 会整串当成命令名执行；且 pipe 下 `/K <cmd>` 会吞掉随后的标准输入，第一条命令直接丢失。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShellMode {
    Console,
    Pipe,
}

/// 只使用系统目录下的绝对路径，SYSTEM 身份下不按 PATH 查找程序。
#[derive(Clone, Debug)]
pub struct ShellLocations {
    pub program_files: PathBuf,
    pub system32: PathBuf,
}

const POWERSHELL_UTF8_ENCODING: &str = "[Console]::InputEncoding = [System.Text.Encoding]::UTF8; [Console]::OutputEncoding = [System.Text.Encoding]::UTF8";

impl ShellLocations {
    #[cfg(windows)]
    pub fn system() -> Self {
        let dirs = super::super::system_dirs::get();
        Self {
            program_files: dirs.program_files.clone(),
            system32: dirs.system32(),
        }
    }

    fn pwsh(&self) -> PathBuf {
        self.program_files.join(r"PowerShell\7\pwsh.exe")
    }

    fn windows_powershell(&self) -> PathBuf {
        self.system32.join(r"WindowsPowerShell\v1.0\powershell.exe")
    }

    fn cmd(&self) -> PathBuf {
        self.system32.join("cmd.exe")
    }

    /// 启动参数里的 chcp 也用绝对路径：cmd 会先在工作目录里找命令。
    fn chcp(&self) -> PathBuf {
        self.system32.join("chcp.com")
    }
}

pub fn resolve_shell(
    kind: ShellKind,
    mode: ShellMode,
    locations: &ShellLocations,
    exists: impl Fn(&Path) -> bool,
) -> Result<ShellCommand, TerminalError> {
    let candidates = match kind {
        ShellKind::PowerShell => vec![locations.pwsh(), locations.windows_powershell()],
        ShellKind::Pwsh => vec![locations.pwsh()],
        ShellKind::Cmd => vec![locations.cmd()],
    };
    let program = candidates
        .into_iter()
        .find(|path| exists(path))
        .ok_or(TerminalError::ShellUnavailable(kind))?;
    let args = match (kind, mode) {
        // 管道下的 cmd 不能用 `/K <payload>`：载荷含 `>` 时 cmd 会把整串当成命令名，
        // 报「不是内部或外部命令」；去掉重定向后它又会吃掉紧随 stdin 的第一条命令。
        // `/Q` 关掉 shell 自己的整行回显：管道输入下它只在回车时把整行一次性吐出，
        // 用户打字期间什么都看不到，按回车又会出现两遍。整行由 agent 本地回显。
        (ShellKind::Cmd, ShellMode::Pipe) => vec!["/Q".to_owned(), "/K".to_owned()],
        (ShellKind::Cmd, ShellMode::Console) => vec![
            "/K".to_owned(),
            format!("{} 65001 >NUL", locations.chcp().display()),
        ],
        // 管道进程没有控制台，[Console]::InputEncoding/OutputEncoding 无处可设。
        (ShellKind::PowerShell | ShellKind::Pwsh, ShellMode::Pipe) => vec![
            "-NoLogo".to_owned(),
            "-NoProfile".to_owned(),
            "-NoExit".to_owned(),
        ],
        (ShellKind::PowerShell | ShellKind::Pwsh, ShellMode::Console) => vec![
            "-NoLogo".to_owned(),
            "-NoProfile".to_owned(),
            "-NoExit".to_owned(),
            "-Command".to_owned(),
            format!(
                "& '{}' 65001 > $null; {POWERSHELL_UTF8_ENCODING}",
                locations.chcp().display()
            ),
        ],
    };
    // PowerShell 在管道输入下逐字回显并处理退格，本地再回显会重复；只有 cmd 需要。
    let local_echo = kind == ShellKind::Cmd && mode == ShellMode::Pipe;
    Ok(ShellCommand {
        program,
        args,
        local_echo,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn locations() -> ShellLocations {
        ShellLocations {
            program_files: PathBuf::from(r"E:\Apps"),
            system32: PathBuf::from(r"E:\OS\System32"),
        }
    }

    fn pwsh() -> PathBuf {
        locations().program_files.join(r"PowerShell\7\pwsh.exe")
    }

    fn windows_powershell() -> PathBuf {
        locations()
            .system32
            .join(r"WindowsPowerShell\v1.0\powershell.exe")
    }

    fn cmd() -> PathBuf {
        locations().system32.join("cmd.exe")
    }

    fn present(paths: &[PathBuf]) -> impl Fn(&Path) -> bool {
        let set: HashSet<PathBuf> = paths.iter().cloned().collect();
        move |path| set.contains(path)
    }

    #[test]
    fn parses_only_known_shell_names() {
        assert_eq!(ShellKind::parse("powershell"), Some(ShellKind::PowerShell));
        assert_eq!(ShellKind::parse("pwsh"), Some(ShellKind::Pwsh));
        assert_eq!(ShellKind::parse("cmd"), Some(ShellKind::Cmd));
        for bad in ["", "bash", "PowerShell", "cmd.exe", "pwsh "] {
            assert_eq!(ShellKind::parse(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn powershell_prefers_powershell_7() {
        let command = resolve_shell(
            ShellKind::PowerShell,
            ShellMode::Pipe,
            &locations(),
            present(&[pwsh(), windows_powershell()]),
        )
        .unwrap();
        assert_eq!(command.program, pwsh());
    }

    #[test]
    fn powershell_falls_back_to_windows_powershell() {
        let command = resolve_shell(
            ShellKind::PowerShell,
            ShellMode::Pipe,
            &locations(),
            present(&[windows_powershell()]),
        )
        .unwrap();
        assert_eq!(command.program, windows_powershell());
    }

    #[test]
    fn pwsh_absent_is_reported_not_substituted() {
        assert_eq!(
            resolve_shell(
                ShellKind::Pwsh,
                ShellMode::Pipe,
                &locations(),
                present(&[windows_powershell(), cmd()])
            ),
            Err(TerminalError::ShellUnavailable(ShellKind::Pwsh))
        );
    }

    #[test]
    fn every_shell_absent_is_reported() {
        for kind in [ShellKind::PowerShell, ShellKind::Pwsh, ShellKind::Cmd] {
            assert_eq!(
                resolve_shell(kind, ShellMode::Pipe, &locations(), present(&[])),
                Err(TerminalError::ShellUnavailable(kind))
            );
        }
    }

    /// 管道下的 cmd 必须关掉 shell 自己的整行回显并由本进程编辑：它逐字不回显、
    /// 退格只当普通字节，只有 `/Q` 才不会在回车时把整行再吐一遍。
    #[test]
    fn cmd_in_pipe_edits_locally_with_echo_off() {
        let command =
            resolve_shell(ShellKind::Cmd, ShellMode::Pipe, &locations(), present(&[cmd()])).unwrap();
        assert_eq!(command.program, cmd());
        assert_eq!(command.args, vec!["/Q".to_owned(), "/K".to_owned()]);
        assert!(command.local_echo);
    }

    #[test]
    fn cmd_on_conpty_keeps_shell_side_echo_off_but_local_echo_disabled() {
        let command =
            resolve_shell(ShellKind::Cmd, ShellMode::Console, &locations(), present(&[cmd()]))
                .unwrap();
        let chcp = locations().system32.join("chcp.com");
        assert_eq!(
            command.args,
            vec!["/K".to_owned(), format!("{} 65001 >NUL", chcp.display())]
        );
        // 真实控制台自己回显，本进程再回显会重复。
        assert!(!command.local_echo);
    }

    /// PowerShell 在管道下自己逐字回显，本进程不能再补一遍。
    #[test]
    fn powershell_never_uses_local_echo() {
        for mode in [ShellMode::Pipe, ShellMode::Console] {
            let command =
                resolve_shell(ShellKind::PowerShell, mode, &locations(), present(&[pwsh()]))
                    .unwrap();
            assert!(!command.local_echo, "{mode:?}");
        }
    }

    /// Console 模式要给 chcp 与 [Console] 编码；三者都是真实控制台才成立的能力。
    #[test]
    fn powershell_runs_without_profile_and_with_utf8_console() {
        let command =
            resolve_shell(ShellKind::Pwsh, ShellMode::Console, &locations(), present(&[pwsh()]))
                .unwrap();
        assert_eq!(
            &command.args[..4],
            ["-NoLogo", "-NoProfile", "-NoExit", "-Command"]
        );
        assert!(
            command.args[4].contains("[Console]::OutputEncoding = [System.Text.Encoding]::UTF8")
        );
        assert!(command.args[4].contains("[Console]::InputEncoding = [System.Text.Encoding]::UTF8"));
        let chcp = locations().system32.join("chcp.com");
        assert!(command.args[4].starts_with(&format!("& '{}' 65001 > $null;", chcp.display())));
    }
}
