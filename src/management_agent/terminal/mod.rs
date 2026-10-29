//! `system.terminal@1` 的 Agent 侧核心：PTY 会话、shell 解析与输出环形缓冲。
//!
//! 字段语义、错误码与超时以 `docs/terminal-protocol.zh.md` 为准。

#[cfg(windows)]
mod console_host;
#[cfg(windows)]
mod job_windows;
mod manager;
mod output_ring;
mod protocol;
#[cfg(windows)]
pub(crate) mod runtime;
#[cfg(windows)]
mod session;
#[cfg(all(test, windows))]
mod session_tests;
mod shell;
mod transport;

use std::time::Duration;

// 会話命令层接入前，部分导出只被测试使用。
#[allow(unused_imports)]
pub use manager::{AuditEvent, CloseReason, OpenRequest, OpenedInfo, SessionManager, TerminalSlot};
#[allow(unused_imports)]
pub use output_ring::{OutputChunk, OutputRing, CHUNK_OVERHEAD};
#[cfg(windows)]
#[allow(unused_imports)]
pub(crate) use session::READ_BUFFER_BYTES;
#[cfg(windows)]
#[allow(unused_imports)]
pub use session::{Expiry, PtySession, SessionPolicy};
#[allow(unused_imports)]
pub use shell::{resolve_shell, ShellCommand, ShellKind, ShellLocations, ShellMode};

/// 当前进程实际会用哪种终端承载。`PipeSession` 与 shell 启动参数必须用同一判定，
/// 否则会给没有控制台的管道进程配上 ConPTY 专用参数。
pub fn shell_mode() -> ShellMode {
    #[cfg(windows)]
    if !console_host::conpty_available() {
        return ShellMode::Pipe;
    }
    ShellMode::Console
}

/// Agent 侧每个会话保留的输出字节上限（协议第 4 节）。
pub const AGENT_RING_BYTES: usize = 4 * 1024 * 1024;
pub const IDLE_TIMEOUT: Duration = Duration::from_secs(30 * 60);
pub const MAX_LIFETIME: Duration = Duration::from_secs(24 * 60 * 60);
pub const MIN_DIMENSION: u16 = 1;
pub const MAX_DIMENSION: u16 = 1000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TerminalSize {
    pub cols: u16,
    pub rows: u16,
}

impl TerminalSize {
    pub fn new(cols: u16, rows: u16) -> Result<Self, TerminalError> {
        let valid = |value| (MIN_DIMENSION..=MAX_DIMENSION).contains(&value);
        if valid(cols) && valid(rows) {
            Ok(Self { cols, rows })
        } else {
            Err(TerminalError::InvalidSize { cols, rows })
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TerminalError {
    ShellUnavailable(ShellKind),
    StartFailed(String),
    GapTooLarge { requested: u64, oldest: u64 },
    SeqAhead { requested: u64, latest: u64 },
    InvalidSize { cols: u16, rows: u16 },
    InvalidArgs(&'static str),
    NotFound,
    SessionLimit,
    LocalGate(&'static str),
    Io(String),
}

impl TerminalError {
    /// 协议第 5 节的错误码。
    pub fn code(&self) -> &'static str {
        match self {
            Self::ShellUnavailable(_) => "terminal_shell_unavailable",
            Self::StartFailed(_) => "terminal_start_failed",
            Self::GapTooLarge { .. } => "terminal_gap_too_large",
            Self::NotFound => "terminal_not_found",
            Self::SessionLimit => "terminal_session_limit",
            Self::LocalGate(_) => "terminal_local_gate",
            Self::SeqAhead { .. }
            | Self::InvalidSize { .. }
            | Self::InvalidArgs(_)
            | Self::Io(_) => "terminal_internal",
        }
    }
}

impl std::fmt::Display for TerminalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ShellUnavailable(kind) => write!(f, "shell {} is not installed", kind.name()),
            Self::StartFailed(reason) => write!(f, "terminal start failed: {reason}"),
            Self::GapTooLarge { requested, oldest } => write!(
                f,
                "output after seq {requested} was discarded; oldest retained seq is {oldest}"
            ),
            Self::SeqAhead { requested, latest } => {
                write!(f, "seq {requested} is ahead of latest seq {latest}")
            }
            Self::InvalidSize { cols, rows } => write!(f, "invalid terminal size {cols}x{rows}"),
            Self::InvalidArgs(reason) => write!(f, "invalid terminal arguments: {reason}"),
            Self::NotFound => write!(f, "terminal session not found"),
            Self::SessionLimit => write!(f, "terminal session limit reached"),
            Self::LocalGate(gate) => write!(f, "terminal refused by local gate: {gate}"),
            Self::Io(reason) => write!(f, "terminal io failed: {reason}"),
        }
    }
}

impl std::error::Error for TerminalError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn size_accepts_bounds_and_rejects_outside() {
        assert!(TerminalSize::new(1, 1).is_ok());
        assert!(TerminalSize::new(1000, 1000).is_ok());
        assert_eq!(
            TerminalSize::new(0, 24),
            Err(TerminalError::InvalidSize { cols: 0, rows: 24 })
        );
        assert_eq!(
            TerminalSize::new(80, 1001),
            Err(TerminalError::InvalidSize {
                cols: 80,
                rows: 1001
            })
        );
    }

    #[test]
    fn error_codes_follow_protocol() {
        assert_eq!(
            TerminalError::ShellUnavailable(ShellKind::Pwsh).code(),
            "terminal_shell_unavailable"
        );
        assert_eq!(
            TerminalError::StartFailed(String::new()).code(),
            "terminal_start_failed"
        );
        assert_eq!(
            TerminalError::GapTooLarge {
                requested: 1,
                oldest: 5
            }
            .code(),
            "terminal_gap_too_large"
        );
        assert_eq!(TerminalError::Io(String::new()).code(), "terminal_internal");
    }
}
