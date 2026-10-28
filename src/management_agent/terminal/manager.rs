//! 终端会话注册表与五个操作（open/write/read/resize/close）。
//!
//! 与传输无关：broker↔agent 的 WS 层把帧解析成这里的调用，门禁、会话上限、
//! 审计都在此判定。会话与 PTY 的实际承载由 [`TerminalSlot`] 抽象，便于在非
//! Windows 上单测决策逻辑，真实 PTY 由 [`super::PtySession`] 实现。
//!
//! 字段语义、错误码、超时以 `docs/terminal-protocol.zh.md` 为准。

use super::{OutputChunk, SessionPolicy, ShellKind, TerminalError, TerminalSize};
use std::{
    collections::HashMap,
    time::Instant,
};

/// 本机（单设备）会话上限（协议 §1）。单管理员 8 由 broker 侧计数。
pub const MAX_SESSIONS_PER_DEVICE: usize = 4;
/// `cwd` 长度上限，防畸形超长输入。
const MAX_CWD_CHARS: usize = 32_767;

/// 本机门禁快照；由传输层读取状态文件后构造，便于单测。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GateState {
    pub disable_recovery: bool,
    pub disable_agent: bool,
    pub maintenance: bool,
    pub stop_requested: bool,
}

impl GateState {
    /// 命中则返回具体门禁名；复用 `uac_policy::local_gate_refusal`。
    fn refusal(self) -> Option<&'static str> {
        super::super::uac_policy::local_gate_refusal(
            self.disable_recovery,
            self.disable_agent,
            self.maintenance,
            self.stop_requested,
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenRequest {
    pub session_id: String,
    pub shell: ShellKind,
    pub size: TerminalSize,
    pub cwd: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenedInfo {
    pub session_id: String,
    pub shell: ShellKind,
    pub size: TerminalSize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloseReason {
    User,
    Idle,
    DeviceGone,
    Error,
}

impl CloseReason {
    /// 协议 §2 `closed.reason` 字符串。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Idle => "idle",
            Self::DeviceGone => "device_gone",
            Self::Error => "error",
        }
    }
}

/// 审计事件（协议 §6）：只记元数据，绝不含输入输出内容。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AuditEvent {
    SessionOpened {
        session_id: String,
        shell: ShellKind,
        cols: u16,
        rows: u16,
    },
    SessionClosed {
        session_id: String,
        reason: CloseReason,
        duration_ms: u64,
    },
    GateRefused {
        session_id: Option<String>,
        gate: &'static str,
        op: &'static str,
    },
}

/// PTY 会话承载抽象。真实实现是 [`super::PtySession`]。
pub trait TerminalSlot: Send {
    fn write(&self, data: &[u8]) -> Result<(), TerminalError>;
    fn resize(&self, size: TerminalSize) -> Result<(), TerminalError>;
    fn read_since(
        &self,
        last_seq: u64,
        max_bytes: usize,
    ) -> Result<Vec<OutputChunk>, TerminalError>;
    fn exit_code(&self) -> Option<u32>;
    fn expiry(&self, policy: &SessionPolicy, now: Instant) -> Option<super::Expiry>;
    fn close(self: Box<Self>) -> Result<Option<u32>, TerminalError>;
    /// shell 退出后把 PTY 里剩余的输出全部收进缓冲；阻塞、可重复调用。
    fn finish_output(&self) {}
    /// `finish_output` 是否已执行过；未执行前已退出的会话不回收，避免丢尾部输出。
    fn output_settled(&self) -> bool {
        true
    }
}

struct Entry {
    slot: Box<dyn TerminalSlot>,
    shell: ShellKind,
    size: TerminalSize,
    started: Instant,
}

/// 校验 `open` 参数：未知键、非法 shell、非法 session_id、非法行列、超长 cwd 均拒绝。
pub fn validate_open(args: &serde_json::Value) -> Result<OpenRequest, TerminalError> {
    let object = args
        .as_object()
        .ok_or(TerminalError::InvalidArgs("open_args_not_object"))?;
    for key in object.keys() {
        if !matches!(
            key.as_str(),
            "session_id" | "shell" | "cols" | "rows" | "cwd"
        ) {
            return Err(TerminalError::InvalidArgs("open_unknown_key"));
        }
    }
    let session_id = object
        .get("session_id")
        .and_then(|v| v.as_str())
        .ok_or(TerminalError::InvalidArgs("open_session_id_missing"))?;
    if !is_session_id(session_id) {
        return Err(TerminalError::InvalidArgs("open_session_id_invalid"));
    }
    let shell = object
        .get("shell")
        .and_then(|v| v.as_str())
        .and_then(ShellKind::parse)
        .ok_or(TerminalError::InvalidArgs("open_shell_invalid"))?;
    let cols = dimension(object.get("cols"))?;
    let rows = dimension(object.get("rows"))?;
    let size = TerminalSize::new(cols, rows)?;
    let cwd = match object.get("cwd") {
        None | Some(serde_json::Value::Null) => None,
        Some(serde_json::Value::String(value)) if value.chars().count() <= MAX_CWD_CHARS => {
            Some(value.clone())
        }
        Some(_) => return Err(TerminalError::InvalidArgs("open_cwd_invalid")),
    };
    Ok(OpenRequest {
        session_id: session_id.to_owned(),
        shell,
        size,
        cwd,
    })
}

fn dimension(value: Option<&serde_json::Value>) -> Result<u16, TerminalError> {
    value
        .and_then(serde_json::Value::as_u64)
        .and_then(|value| u16::try_from(value).ok())
        .ok_or(TerminalError::InvalidArgs("open_dimension_invalid"))
}

/// 会话号由 broker 生成（协议 §1）：32 位小写十六进制。
pub fn is_session_id(value: &str) -> bool {
    value.len() == 32
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

pub struct SessionManager {
    sessions: HashMap<String, Entry>,
    policy: SessionPolicy,
    max_sessions: usize,
}

impl Default for SessionManager {
    fn default() -> Self {
        Self::new(SessionPolicy::default(), MAX_SESSIONS_PER_DEVICE)
    }
}

impl SessionManager {
    pub fn new(policy: SessionPolicy, max_sessions: usize) -> Self {
        Self {
            sessions: HashMap::new(),
            policy,
            max_sessions,
        }
    }

    pub fn len(&self) -> usize {
        self.sessions.len()
    }

    pub fn is_empty(&self) -> bool {
        self.sessions.is_empty()
    }

    pub fn contains(&self, session_id: &str) -> bool {
        self.sessions.contains_key(session_id)
    }

    /// 门禁拒绝 `open`：返回门禁名 + GateRefused 审计。会话号重复、超限也在此拒绝。
    pub fn admit_open(
        &self,
        request: &OpenRequest,
        gate: GateState,
    ) -> Result<(), (TerminalError, AuditEvent)> {
        if let Some(name) = gate.refusal() {
            return Err((
                TerminalError::LocalGate(name),
                AuditEvent::GateRefused {
                    session_id: Some(request.session_id.clone()),
                    gate: name,
                    op: "open",
                },
            ));
        }
        if self.sessions.contains_key(&request.session_id) {
            return Err((
                TerminalError::InvalidArgs("open_session_id_duplicate"),
                AuditEvent::GateRefused {
                    session_id: Some(request.session_id.clone()),
                    gate: "duplicate",
                    op: "open",
                },
            ));
        }
        if self.sessions.len() >= self.max_sessions {
            return Err((
                TerminalError::SessionLimit,
                AuditEvent::GateRefused {
                    session_id: Some(request.session_id.clone()),
                    gate: "session_limit",
                    op: "open",
                },
            ));
        }
        Ok(())
    }

    /// 门禁通过、校验通过后登记已拉起的会话，返回 opened 信息 + SessionOpened 审计。
    pub fn register(
        &mut self,
        request: OpenRequest,
        slot: Box<dyn TerminalSlot>,
    ) -> (OpenedInfo, AuditEvent) {
        let OpenRequest {
            session_id,
            shell,
            size,
            ..
        } = request;
        self.sessions.insert(
            session_id.clone(),
            Entry {
                slot,
                shell,
                size,
                started: Instant::now(),
            },
        );
        (
            OpenedInfo {
                session_id: session_id.clone(),
                shell,
                size,
            },
            AuditEvent::SessionOpened {
                session_id,
                shell,
                cols: size.cols,
                rows: size.rows,
            },
        )
    }

    /// 键盘输入：门禁拒绝 `input`（返回 GateRefused 审计），否则写入会话。
    pub fn input(
        &self,
        session_id: &str,
        data: &[u8],
        gate: GateState,
    ) -> Result<(), (TerminalError, Option<AuditEvent>)> {
        if let Some(name) = gate.refusal() {
            return Err((
                TerminalError::LocalGate(name),
                Some(AuditEvent::GateRefused {
                    session_id: Some(session_id.to_owned()),
                    gate: name,
                    op: "input",
                }),
            ));
        }
        let entry = self.entry(session_id).map_err(|e| (e, None))?;
        entry.slot.write(data).map_err(|e| (e, None))
    }

    /// 调整尺寸：不受门禁限制（清理已有会话时仍可用）。
    pub fn resize(&self, session_id: &str, size: TerminalSize) -> Result<(), TerminalError> {
        self.entry(session_id)?.slot.resize(size)
    }

    /// 读取输出：不受门禁限制，便于 attach 续读。
    pub fn read(
        &self,
        session_id: &str,
        last_seq: u64,
        max_bytes: usize,
    ) -> Result<Vec<OutputChunk>, TerminalError> {
        self.entry(session_id)?.slot.read_since(last_seq, max_bytes)
    }

    /// 主动关闭：不受门禁限制。返回 SessionClosed 审计（reason=user）。
    pub fn close(&mut self, session_id: &str) -> Result<AuditEvent, TerminalError> {
        self.remove(session_id, CloseReason::User)
    }

    /// 回收已退出的 shell：返回各自的 exited 退出码与 SessionClosed(error 若非 0)。
    pub fn drain_exited(&mut self) -> Vec<(String, u32, AuditEvent)> {
        let exited: Vec<(String, u32)> = self
            .sessions
            .iter()
            .filter(|(_, entry)| entry.slot.output_settled())
            .filter_map(|(id, entry)| entry.slot.exit_code().map(|code| (id.clone(), code)))
            .collect();
        exited
            .into_iter()
            .filter_map(|(id, code)| {
                let reason = if code == 0 {
                    CloseReason::User
                } else {
                    CloseReason::Error
                };
                self.remove(&id, reason).ok().map(|audit| (id, code, audit))
            })
            .collect()
    }

    /// 对已退出的 shell 收齐剩余输出，返回这些会话号。调用方应先把它们的输出全部推完，
    /// 再调用 [`Self::drain_exited`] 回收。
    pub fn settle_exited(&self) -> Vec<String> {
        self.sessions
            .iter()
            .filter(|(_, entry)| entry.slot.exit_code().is_some())
            .map(|(id, entry)| {
                entry.slot.finish_output();
                id.clone()
            })
            .collect()
    }

    /// 回收空闲/超最长存活的会话：空闲为 reason=idle，到达最长存活为 reason=error（协议 §7）。
    pub fn reap_expired(&mut self, now: Instant) -> Vec<AuditEvent> {
        let expired: Vec<(String, CloseReason)> = self
            .sessions
            .iter()
            .filter_map(|(id, entry)| {
                entry.slot.expiry(&self.policy, now).map(|expiry| {
                    let reason = match expiry {
                        super::Expiry::Idle => CloseReason::Idle,
                        super::Expiry::MaxLifetime => CloseReason::Error,
                    };
                    (id.clone(), reason)
                })
            })
            .collect();
        expired
            .into_iter()
            .filter_map(|(id, reason)| self.remove(&id, reason).ok())
            .collect()
    }

    fn entry(&self, session_id: &str) -> Result<&Entry, TerminalError> {
        self.sessions.get(session_id).ok_or(TerminalError::NotFound)
    }

    fn remove(
        &mut self,
        session_id: &str,
        reason: CloseReason,
    ) -> Result<AuditEvent, TerminalError> {
        let entry = self
            .sessions
            .remove(session_id)
            .ok_or(TerminalError::NotFound)?;
        let duration_ms = entry.started.elapsed().as_millis() as u64;
        if let Err(error) = entry.slot.close() {
            hbb_common::log::warn!("terminal session {session_id} close failed: {error}");
        }
        Ok(AuditEvent::SessionClosed {
            session_id: session_id.to_owned(),
            reason,
            duration_ms,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        atomic::{AtomicBool, AtomicU32, Ordering},
        Arc,
    };

    #[derive(Default)]
    struct FakeSlot {
        exit: AtomicU32,
        exited: AtomicBool,
        expired: AtomicBool,
        lifetime: AtomicBool,
        unsettled: AtomicBool,
        finished: Arc<AtomicBool>,
        closed: Arc<AtomicBool>,
        writes: Arc<std::sync::Mutex<Vec<Vec<u8>>>>,
    }

    impl FakeSlot {
        fn handle() -> (Box<FakeSlot>, Arc<AtomicBool>) {
            let closed = Arc::new(AtomicBool::new(false));
            (
                Box::new(FakeSlot {
                    closed: closed.clone(),
                    ..Default::default()
                }),
                closed,
            )
        }
    }

    impl TerminalSlot for FakeSlot {
        fn write(&self, data: &[u8]) -> Result<(), TerminalError> {
            self.writes.lock().unwrap().push(data.to_vec());
            Ok(())
        }
        fn resize(&self, _size: TerminalSize) -> Result<(), TerminalError> {
            Ok(())
        }
        fn read_since(
            &self,
            _last_seq: u64,
            _max_bytes: usize,
        ) -> Result<Vec<OutputChunk>, TerminalError> {
            Ok(Vec::new())
        }
        fn exit_code(&self) -> Option<u32> {
            self.exited
                .load(Ordering::SeqCst)
                .then(|| self.exit.load(Ordering::SeqCst))
        }
        fn expiry(&self, _policy: &SessionPolicy, _now: Instant) -> Option<super::super::Expiry> {
            if self.lifetime.load(Ordering::SeqCst) {
                return Some(super::super::Expiry::MaxLifetime);
            }
            self.expired
                .load(Ordering::SeqCst)
                .then_some(super::super::Expiry::Idle)
        }
        fn finish_output(&self) {
            self.finished.store(true, Ordering::SeqCst);
        }
        fn output_settled(&self) -> bool {
            !self.unsettled.load(Ordering::SeqCst) || self.finished.load(Ordering::SeqCst)
        }
        fn close(self: Box<Self>) -> Result<Option<u32>, TerminalError> {
            self.closed.store(true, Ordering::SeqCst);
            Ok(None)
        }
    }

    fn sid(n: u8) -> String {
        format!("{:032x}", n)
    }

    fn req(id: &str) -> OpenRequest {
        OpenRequest {
            session_id: id.to_owned(),
            shell: ShellKind::PowerShell,
            size: TerminalSize::new(80, 24).unwrap(),
            cwd: None,
        }
    }

    fn open(manager: &mut SessionManager, id: &str, gate: GateState) -> (OpenedInfo, AuditEvent) {
        let request = req(id);
        manager.admit_open(&request, gate).unwrap();
        let (slot, _) = FakeSlot::handle();
        manager.register(request, slot)
    }

    fn json(raw: &str) -> serde_json::Value {
        serde_json::from_str(raw).unwrap()
    }

    #[test]
    fn validate_open_accepts_minimal_and_full() {
        let full = validate_open(&json(
            r#"{"session_id":"0123456789abcdef0123456789abcdef","shell":"pwsh","cols":120,"rows":30,"cwd":"C:\\Windows"}"#,
        ))
        .unwrap();
        assert_eq!(full.shell, ShellKind::Pwsh);
        assert_eq!(full.size, TerminalSize::new(120, 30).unwrap());
        assert_eq!(full.cwd.as_deref(), Some(r"C:\Windows"));
        let minimal = validate_open(&json(
            r#"{"session_id":"0123456789abcdef0123456789abcdef","shell":"cmd","cols":80,"rows":24}"#,
        ))
        .unwrap();
        assert_eq!(minimal.cwd, None);
    }

    #[test]
    fn validate_open_rejects_bad_inputs() {
        let base = r#""session_id":"0123456789abcdef0123456789abcdef","shell":"powershell","cols":80,"rows":24"#;
        let cases = [
            r#"{"shell":"powershell","cols":80,"rows":24}"#,
            r#"{"session_id":"SHORT","shell":"powershell","cols":80,"rows":24}"#,
            r#"{"session_id":"0123456789ABCDEF0123456789abcdef","shell":"powershell","cols":80,"rows":24}"#,
            r#"{"session_id":"0123456789abcdef0123456789abcdef","shell":"bash","cols":80,"rows":24}"#,
            r#"{"session_id":"0123456789abcdef0123456789abcdef","shell":"powershell","cols":0,"rows":24}"#,
            r#"{"session_id":"0123456789abcdef0123456789abcdef","shell":"powershell","cols":80,"rows":99999}"#,
        ];
        for case in cases {
            assert_eq!(
                validate_open(&json(case)).map(|_| ()).unwrap_err().code(),
                "terminal_internal",
                "{case}"
            );
        }
        // unknown key rejected
        assert!(validate_open(&json(&format!(r#"{{{base},"extra":1}}"#))).is_err());
        // non-object rejected
        assert!(validate_open(&json("[]")).is_err());
    }

    #[test]
    fn open_register_emits_audit_and_counts() {
        let mut manager = SessionManager::default();
        let (opened, audit) = open(&mut manager, &sid(1), GateState::default());
        assert_eq!(opened.session_id, sid(1));
        assert_eq!(
            audit,
            AuditEvent::SessionOpened {
                session_id: sid(1),
                shell: ShellKind::PowerShell,
                cols: 80,
                rows: 24,
            }
        );
        assert_eq!(manager.len(), 1);
        assert!(manager.contains(&sid(1)));
    }

    #[test]
    fn each_gate_refuses_open_with_specific_audit() {
        let gates = [
            (
                GateState {
                    disable_recovery: true,
                    ..Default::default()
                },
                "local_gate_disable_recovery",
            ),
            (
                GateState {
                    disable_agent: true,
                    ..Default::default()
                },
                "local_gate_disable_agent",
            ),
            (
                GateState {
                    maintenance: true,
                    ..Default::default()
                },
                "local_gate_maintenance",
            ),
            (
                GateState {
                    stop_requested: true,
                    ..Default::default()
                },
                "local_gate_stop_requested",
            ),
        ];
        for (gate, name) in gates {
            let manager = SessionManager::default();
            let (error, audit) = manager.admit_open(&req(&sid(1)), gate).unwrap_err();
            assert_eq!(error.code(), "terminal_local_gate");
            assert_eq!(
                audit,
                AuditEvent::GateRefused {
                    session_id: Some(sid(1)),
                    gate: name,
                    op: "open",
                }
            );
        }
    }

    #[test]
    fn gate_refuses_input_but_allows_read_resize_close() {
        let mut manager = SessionManager::default();
        open(&mut manager, &sid(1), GateState::default());
        let gate = GateState {
            disable_agent: true,
            ..Default::default()
        };
        let (error, audit) = manager.input(&sid(1), b"whoami\r", gate).unwrap_err();
        assert_eq!(error.code(), "terminal_local_gate");
        assert_eq!(
            audit,
            Some(AuditEvent::GateRefused {
                session_id: Some(sid(1)),
                gate: "local_gate_disable_agent",
                op: "input",
            })
        );
        // read / resize / close 仍可用，便于清理
        assert!(manager.read(&sid(1), 0, usize::MAX).is_ok());
        assert!(manager
            .resize(&sid(1), TerminalSize::new(100, 40).unwrap())
            .is_ok());
        assert!(manager.close(&sid(1)).is_ok());
    }

    #[test]
    fn session_limit_rejects_fifth_but_not_fourth() {
        let mut manager = SessionManager::default();
        for n in 0..MAX_SESSIONS_PER_DEVICE as u8 {
            open(&mut manager, &sid(n), GateState::default());
        }
        assert_eq!(manager.len(), MAX_SESSIONS_PER_DEVICE);
        let error = manager
            .admit_open(&req(&sid(99)), GateState::default())
            .unwrap_err()
            .0;
        assert_eq!(error.code(), "terminal_session_limit");
    }

    #[test]
    fn duplicate_session_id_rejected() {
        let mut manager = SessionManager::default();
        open(&mut manager, &sid(1), GateState::default());
        assert!(manager
            .admit_open(&req(&sid(1)), GateState::default())
            .is_err());
    }

    #[test]
    fn missing_session_is_not_found() {
        let manager = SessionManager::default();
        assert_eq!(
            manager.read(&sid(7), 0, usize::MAX).unwrap_err().code(),
            "terminal_not_found"
        );
        assert_eq!(
            manager
                .resize(&sid(7), TerminalSize::new(80, 24).unwrap())
                .unwrap_err()
                .code(),
            "terminal_not_found"
        );
    }

    #[test]
    fn input_reaches_slot_and_refreshes_nothing_else() {
        let mut manager = SessionManager::default();
        let request = req(&sid(1));
        manager.admit_open(&request, GateState::default()).unwrap();
        let (slot, _closed) = FakeSlot::handle();
        let writes = slot.writes.clone();
        manager.register(request, slot);
        manager
            .input(&sid(1), b"echo hi\r", GateState::default())
            .unwrap();
        assert_eq!(writes.lock().unwrap().as_slice(), [b"echo hi\r".to_vec()]);
    }

    #[test]
    fn close_runs_slot_close_and_emits_audit() {
        let mut manager = SessionManager::default();
        let request = req(&sid(1));
        manager.admit_open(&request, GateState::default()).unwrap();
        let (slot, closed) = FakeSlot::handle();
        manager.register(request, slot);
        let audit = manager.close(&sid(1)).unwrap();
        assert!(closed.load(Ordering::SeqCst));
        assert!(matches!(
            audit,
            AuditEvent::SessionClosed {
                reason: CloseReason::User,
                ..
            }
        ));
        assert!(manager.is_empty());
    }

    #[test]
    fn drain_exited_reports_code_and_reason() {
        let mut manager = SessionManager::default();
        let request = req(&sid(1));
        manager.admit_open(&request, GateState::default()).unwrap();
        let (slot, _closed) = FakeSlot::handle();
        slot.exit.store(3, Ordering::SeqCst);
        slot.exited.store(true, Ordering::SeqCst);
        manager.register(request, slot);
        let drained = manager.drain_exited();
        assert_eq!(drained.len(), 1);
        assert_eq!(drained[0].1, 3);
        assert!(matches!(
            drained[0].2,
            AuditEvent::SessionClosed {
                reason: CloseReason::Error,
                ..
            }
        ));
        assert!(manager.is_empty());
    }

    #[test]
    fn reap_expired_closes_idle_sessions() {
        let mut manager = SessionManager::default();
        let request = req(&sid(1));
        manager.admit_open(&request, GateState::default()).unwrap();
        let (slot, closed) = FakeSlot::handle();
        slot.expired.store(true, Ordering::SeqCst);
        manager.register(request, slot);
        open(&mut manager, &sid(2), GateState::default());
        let reaped = manager.reap_expired(Instant::now());
        assert_eq!(reaped.len(), 1);
        assert!(matches!(
            reaped[0],
            AuditEvent::SessionClosed {
                reason: CloseReason::Idle,
                ..
            }
        ));
        assert!(closed.load(Ordering::SeqCst));
        assert_eq!(manager.len(), 1);
        assert!(manager.contains(&sid(2)));
    }

    #[test]
    fn exited_session_is_not_drained_before_its_output_settles() {
        let mut manager = SessionManager::default();
        let request = req(&sid(1));
        manager.admit_open(&request, GateState::default()).unwrap();
        let (slot, closed) = FakeSlot::handle();
        slot.exited.store(true, Ordering::SeqCst);
        slot.unsettled.store(true, Ordering::SeqCst);
        let finished = slot.finished.clone();
        manager.register(request, slot);
        assert!(manager.drain_exited().is_empty(), "输出未收齐前不能回收");
        assert!(manager.contains(&sid(1)));
        assert!(!closed.load(Ordering::SeqCst));
        assert_eq!(manager.settle_exited(), vec![sid(1)]);
        assert!(finished.load(Ordering::SeqCst));
        assert_eq!(manager.drain_exited().len(), 1);
        assert!(manager.is_empty());
    }

    #[test]
    fn settle_exited_skips_running_sessions() {
        let mut manager = SessionManager::default();
        let request = req(&sid(1));
        manager.admit_open(&request, GateState::default()).unwrap();
        let (slot, _closed) = FakeSlot::handle();
        slot.unsettled.store(true, Ordering::SeqCst);
        let finished = slot.finished.clone();
        manager.register(request, slot);
        assert!(manager.settle_exited().is_empty());
        assert!(
            !finished.load(Ordering::SeqCst),
            "仍在运行的会话不能关伪控制台"
        );
        assert!(manager.drain_exited().is_empty());
    }

    #[test]
    fn reap_reports_max_lifetime_as_error() {
        let mut manager = SessionManager::default();
        let request = req(&sid(1));
        manager.admit_open(&request, GateState::default()).unwrap();
        let (slot, closed) = FakeSlot::handle();
        slot.lifetime.store(true, Ordering::SeqCst);
        manager.register(request, slot);
        let reaped = manager.reap_expired(Instant::now());
        assert_eq!(reaped.len(), 1);
        assert!(matches!(
            reaped[0],
            AuditEvent::SessionClosed {
                reason: CloseReason::Error,
                ..
            }
        ));
        assert!(closed.load(Ordering::SeqCst));
    }

    #[test]
    fn close_reason_strings_match_protocol() {
        assert_eq!(CloseReason::User.as_str(), "user");
        assert_eq!(CloseReason::Idle.as_str(), "idle");
        assert_eq!(CloseReason::DeviceGone.as_str(), "device_gone");
        assert_eq!(CloseReason::Error.as_str(), "error");
    }
}
