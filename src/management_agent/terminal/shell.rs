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
    let args = match kind {
        ShellKind::Cmd => vec![
            "/K".to_owned(),
            format!("{} 65001 >NUL", locations.chcp().display()),
        ],
        ShellKind::PowerShell | ShellKind::Pwsh => vec![
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
    Ok(ShellCommand { program, args })
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
                resolve_shell(kind, &locations(), present(&[])),
                Err(TerminalError::ShellUnavailable(kind))
            );
        }
    }

    #[test]
    fn cmd_uses_system32_and_utf8_codepage() {
        let command = resolve_shell(ShellKind::Cmd, &locations(), present(&[cmd()])).unwrap();
        assert_eq!(command.program, cmd());
        let chcp = locations().system32.join("chcp.com");
        assert_eq!(
            command.args,
            vec!["/K".to_owned(), format!("{} 65001 >NUL", chcp.display())]
        );
    }

    #[test]
    fn powershell_runs_without_profile_and_with_utf8_console() {
        let command = resolve_shell(ShellKind::Pwsh, &locations(), present(&[pwsh()])).unwrap();
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
