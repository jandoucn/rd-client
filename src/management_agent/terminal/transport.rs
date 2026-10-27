//! 终端 WS 传输层的纯逻辑：重连退避、握手签名与 URL、心跳判定、帧分发。
//!
//! 不碰 socket 与线程，便于单测；socket 驱动在 `runtime.rs`（仅 Windows）。
//! 约定以 `docs/terminal-protocol.zh.md` §3、§7、§10 为准。

use super::{
    manager::{AuditEvent, GateState, SessionManager, TerminalSlot},
    protocol::{self, ClientFrame},
    CloseReason, TerminalError,
};
use serde_json::Value;
use std::time::{Duration, Instant};

pub const AGENT_WS_PATH: &str = "/rd/update/v1/agent/terminal/ws";
pub const BACKOFF_BASE: Duration = Duration::from_secs(1);
pub const BACKOFF_CAP: Duration = Duration::from_secs(60);
pub const PING_AFTER: Duration = Duration::from_secs(25);
pub const DEAD_AFTER: Duration = Duration::from_secs(60);
pub const MAX_FRAME_BYTES: usize = 256 * 1024;
/// PTY 读线程单次最多读这么多字节（`session.rs`），一个输出块即一帧，base64 后远小于 [`MAX_FRAME_BYTES`]。
pub const MAX_OUTPUT_CHUNK_BYTES: usize = 8 * 1024;

/// 指数退避：1s 起、翻倍、上限 60s；`jitter` 取 `[0,1)`，只向下抖动至多 25%，保证不超上限。
#[derive(Debug, Default)]
pub struct Backoff {
    failures: u32,
}

impl Backoff {
    pub fn next_delay(&mut self, jitter: f64) -> Duration {
        let exponent = self.failures.min(6);
        self.failures = self.failures.saturating_add(1);
        let base = (BACKOFF_BASE * 2u32.pow(exponent)).min(BACKOFF_CAP);
        let jitter = if jitter.is_finite() {
            jitter.clamp(0.0, 1.0)
        } else {
            0.0
        };
        base.mul_f64(1.0 - 0.25 * jitter)
    }

    pub fn reset(&mut self) {
        self.failures = 0;
    }
}

/// 握手 URL 的查询参数（协议 §3 / §10 第 7 条）。
pub fn handshake_query(
    client_id: &str,
    client_uuid: &str,
    installation_id: &str,
    generation: u64,
) -> String {
    format!(
        "client_id={}&client_uuid={}&agent_installation_id={}&enrollment_generation={generation}",
        percent_encode(client_id),
        percent_encode(client_uuid),
        percent_encode(installation_id),
    )
}

fn percent_encode(value: &str) -> String {
    value
        .bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (byte as char).to_string()
            }
            _ => format!("%{byte:02X}"),
        })
        .collect()
}

/// 握手要签名的串：与命令通道同一个 `canonical_request`，GET、空 body、`command_id` 为空。
pub fn handshake_canonical(
    client_id: &str,
    client_uuid: &str,
    timestamp: i64,
    nonce: &str,
) -> String {
    format!(
        "rustdesk-update-auth-v1\nmethod=GET\npath={AGENT_WS_PATH}\nclient_id={client_id}\nclient_uuid={client_uuid}\ntimestamp={timestamp}\nnonce={nonce}\ncommand_id=\nbody_sha256={EMPTY_BODY_SHA256}\n"
    )
}

const EMPTY_BODY_SHA256: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

/// 把 `https://host` 基址换成 `wss://host`（`http` → `ws`），其它 scheme 拒绝。
pub fn ws_url(base: &str, query: &str) -> Result<String, &'static str> {
    let (scheme, rest) = if let Some(rest) = base.strip_prefix("https://") {
        ("wss", rest)
    } else if let Some(rest) = base.strip_prefix("http://") {
        ("ws", rest)
    } else {
        return Err("terminal_ws_base_scheme_unsupported");
    };
    let host = rest.trim_end_matches('/');
    if host.is_empty() || host.contains('/') {
        return Err("terminal_ws_base_invalid");
    }
    Ok(format!("{scheme}://{host}{AGENT_WS_PATH}?{query}"))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Liveness {
    Alive,
    SendPing,
    Dead,
}

/// 25s 无任何帧则发 ping；60s 无帧判死。
pub fn liveness(last_frame: Instant, now: Instant) -> Liveness {
    let idle = now.saturating_duration_since(last_frame);
    if idle >= DEAD_AFTER {
        Liveness::Dead
    } else if idle >= PING_AFTER {
        Liveness::SendPing
    } else {
        Liveness::Alive
    }
}

/// 帧分发的产出：要发给 broker 的 JSON 帧与要写审计的事件。
#[derive(Debug, Default, PartialEq)]
pub struct Dispatch {
    pub replies: Vec<Value>,
    pub audits: Vec<AuditEvent>,
}

/// 拉起会话的抽象，便于单测注入假 slot；真实实现在 `runtime.rs`。
pub trait SlotFactory {
    fn spawn(
        &mut self,
        request: &super::OpenRequest,
    ) -> Result<Box<dyn TerminalSlot>, TerminalError>;
}

/// 处理一条 broker→agent 的帧。`opened` 固定 seq=1；错误、`pong` 与用户主动 `closed` 用 seq=0。
pub fn dispatch(
    manager: &mut SessionManager,
    factory: &mut dyn SlotFactory,
    frame: ClientFrame,
    gate: GateState,
    now_ms: u64,
) -> Dispatch {
    let mut out = Dispatch::default();
    match frame {
        ClientFrame::Open(request) => {
            if let Err((error, audit)) = manager.admit_open(&request, gate) {
                out.replies
                    .push(protocol::error(0, Some(&request.session_id), &error));
                out.audits.push(audit);
                return out;
            }
            match factory.spawn(&request) {
                Ok(slot) => {
                    let (info, audit) = manager.register(request, slot);
                    out.replies.push(protocol::opened(
                        1,
                        &info.session_id,
                        info.shell,
                        info.size,
                        now_ms,
                    ));
                    out.audits.push(audit);
                }
                Err(error) => {
                    out.replies
                        .push(protocol::error(0, Some(&request.session_id), &error))
                }
            }
        }
        ClientFrame::Input { session_id, data } => {
            if let Err((error, audit)) = manager.input(&session_id, &data, gate) {
                out.replies
                    .push(protocol::error(0, Some(&session_id), &error));
                out.audits.extend(audit);
            }
        }
        ClientFrame::Resize { session_id, size } => {
            if let Err(error) = manager.resize(&session_id, size) {
                out.replies
                    .push(protocol::error(0, Some(&session_id), &error));
            }
        }
        ClientFrame::Close { session_id } => match manager.close(&session_id) {
            Ok(audit) => {
                out.replies
                    .push(protocol::closed(0, &session_id, CloseReason::User));
                out.audits.push(audit);
            }
            Err(error) => out
                .replies
                .push(protocol::error(0, Some(&session_id), &error)),
        },
        ClientFrame::Ping => out.replies.push(protocol::pong()),
    }
    out
}

/// 输出块 → `output` 帧。`opened` 占 seq=1，所以线上序号是块序号 + 1，一块一帧。
pub fn output_frames(session_id: &str, chunks: &[super::OutputChunk]) -> Vec<Value> {
    chunks
        .iter()
        .map(|chunk| protocol::output(chunk.seq + 1, session_id, &chunk.data))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::management_agent::terminal::{
        manager::{OpenRequest, TerminalSlot},
        CloseReason, OutputChunk, SessionPolicy, ShellKind, TerminalSize,
    };
    use serde_json::json;

    fn secs(n: u64) -> Duration {
        Duration::from_secs(n)
    }

    #[test]
    fn backoff_doubles_from_one_second_and_caps_at_sixty() {
        let mut backoff = Backoff::default();
        let delays: Vec<_> = (0..9).map(|_| backoff.next_delay(0.0)).collect();
        assert_eq!(
            delays,
            vec![
                secs(1),
                secs(2),
                secs(4),
                secs(8),
                secs(16),
                secs(32),
                secs(60),
                secs(60),
                secs(60)
            ]
        );
    }

    #[test]
    fn backoff_jitter_only_shortens_and_stays_positive() {
        let mut backoff = Backoff::default();
        let delay = backoff.next_delay(0.999);
        assert!(delay < secs(1));
        assert!(delay >= Duration::from_millis(750));
        for _ in 0..20 {
            assert!(backoff.next_delay(0.5) <= BACKOFF_CAP);
        }
    }

    #[test]
    fn backoff_resets_after_success() {
        let mut backoff = Backoff::default();
        for _ in 0..5 {
            backoff.next_delay(0.0);
        }
        backoff.reset();
        assert_eq!(backoff.next_delay(0.0), secs(1));
    }

    #[test]
    fn handshake_query_carries_identity_and_generation_exactly() {
        assert_eq!(
            handshake_query("123456789", "uuid-1", "install-9", 7),
            "client_id=123456789&client_uuid=uuid-1&agent_installation_id=install-9&enrollment_generation=7"
        );
    }

    #[test]
    fn handshake_query_percent_encodes_reserved_characters() {
        let query = handshake_query("a b", "u&v=1", "i/d", 0);
        assert_eq!(
            query,
            "client_id=a%20b&client_uuid=u%26v%3D1&agent_installation_id=i%2Fd&enrollment_generation=0"
        );
    }

    #[test]
    fn handshake_canonical_matches_command_channel_empty_get() {
        let canonical = handshake_canonical("id1", "uuid1", 1_700_000_000, "abcd");
        assert_eq!(
            canonical,
            "rustdesk-update-auth-v1\nmethod=GET\npath=/rd/update/v1/agent/terminal/ws\nclient_id=id1\nclient_uuid=uuid1\ntimestamp=1700000000\nnonce=abcd\ncommand_id=\nbody_sha256=e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855\n"
        );
    }

    #[cfg(windows)]
    #[test]
    fn handshake_canonical_is_byte_identical_to_command_channel() {
        assert_eq!(
            handshake_canonical("id1", "uuid1", 1_700_000_000, "abcd"),
            crate::management_agent::operations::canonical_request(
                "GET",
                AGENT_WS_PATH,
                ("id1", "uuid1"),
                1_700_000_000,
                "abcd",
                "",
                b""
            )
        );
    }

    #[test]
    fn ws_url_maps_schemes_and_rejects_others() {
        assert_eq!(
            ws_url("https://rdapi.yan.life", "a=1").unwrap(),
            "wss://rdapi.yan.life/rd/update/v1/agent/terminal/ws?a=1"
        );
        assert_eq!(
            ws_url("http://127.0.0.1:7000/", "a=1").unwrap(),
            "ws://127.0.0.1:7000/rd/update/v1/agent/terminal/ws?a=1"
        );
        for bad in ["ftp://x", "rdapi.yan.life", "wss://x", ""] {
            assert!(ws_url(bad, "a=1").is_err(), "{bad:?}");
        }
    }

    #[test]
    fn liveness_pings_at_25s_and_dies_at_60s() {
        let start = Instant::now();
        assert_eq!(liveness(start, start + secs(24)), Liveness::Alive);
        assert_eq!(liveness(start, start + secs(25)), Liveness::SendPing);
        assert_eq!(liveness(start, start + secs(59)), Liveness::SendPing);
        assert_eq!(liveness(start, start + secs(60)), Liveness::Dead);
    }

    struct FakeSlot {
        fail_write: bool,
    }

    impl TerminalSlot for FakeSlot {
        fn write(&self, _data: &[u8]) -> Result<(), TerminalError> {
            if self.fail_write {
                Err(TerminalError::Io("pipe".into()))
            } else {
                Ok(())
            }
        }
        fn resize(&self, _size: TerminalSize) -> Result<(), TerminalError> {
            Ok(())
        }
        fn read_since(&self, _l: u64, _m: usize) -> Result<Vec<OutputChunk>, TerminalError> {
            Ok(Vec::new())
        }
        fn exit_code(&self) -> Option<u32> {
            None
        }
        fn expiry(
            &self,
            _p: &SessionPolicy,
            _n: Instant,
        ) -> Option<crate::management_agent::terminal::Expiry> {
            None
        }
        fn close(self: Box<Self>) -> Result<Option<u32>, TerminalError> {
            Ok(Some(0))
        }
    }

    struct Factory {
        result: Option<TerminalError>,
        spawned: usize,
    }

    impl SlotFactory for Factory {
        fn spawn(&mut self, _r: &OpenRequest) -> Result<Box<dyn TerminalSlot>, TerminalError> {
            self.spawned += 1;
            match self.result.clone() {
                Some(error) => Err(error),
                None => Ok(Box::new(FakeSlot { fail_write: false })),
            }
        }
    }

    fn open_frame(id: &str) -> ClientFrame {
        ClientFrame::Open(OpenRequest {
            session_id: id.into(),
            shell: ShellKind::PowerShell,
            size: TerminalSize::new(120, 30).unwrap(),
            cwd: None,
        })
    }

    fn sid(n: char) -> String {
        n.to_string().repeat(32)
    }

    fn run(
        manager: &mut SessionManager,
        factory: &mut Factory,
        frame: ClientFrame,
        gate: GateState,
    ) -> Dispatch {
        dispatch(manager, factory, frame, gate, 1_700_000_000_000)
    }

    #[test]
    fn open_spawns_registers_and_replies_opened_with_seq_one() {
        let mut manager = SessionManager::default();
        let mut factory = Factory {
            result: None,
            spawned: 0,
        };
        let id = sid('a');
        let out = run(
            &mut manager,
            &mut factory,
            open_frame(&id),
            GateState::default(),
        );
        assert_eq!(factory.spawned, 1);
        assert!(manager.contains(&id));
        assert_eq!(out.replies.len(), 1);
        assert_eq!(out.replies[0]["type"], "opened");
        assert_eq!(out.replies[0]["seq"], 1);
        assert_eq!(out.replies[0]["session_id"], id);
        assert_eq!(out.replies[0]["shell"], "powershell");
        assert_eq!(out.audits.len(), 1);
        assert!(matches!(out.audits[0], AuditEvent::SessionOpened { .. }));
    }

    #[test]
    fn gate_refusal_never_spawns_and_replies_local_gate_with_audit() {
        let mut manager = SessionManager::default();
        let mut factory = Factory {
            result: None,
            spawned: 0,
        };
        let gate = GateState {
            disable_agent: true,
            ..GateState::default()
        };
        let id = sid('b');
        let out = run(&mut manager, &mut factory, open_frame(&id), gate);
        assert_eq!(factory.spawned, 0);
        assert!(!manager.contains(&id));
        assert_eq!(out.replies[0]["type"], "error");
        assert_eq!(out.replies[0]["code"], "terminal_local_gate");
        assert_eq!(out.replies[0]["session_id"], id);
        assert_eq!(out.replies[0]["seq"], 0);
        assert!(matches!(
            out.audits[0],
            AuditEvent::GateRefused { op: "open", .. }
        ));
    }

    #[test]
    fn spawn_failure_replies_error_and_leaves_no_session() {
        let mut manager = SessionManager::default();
        let mut factory = Factory {
            result: Some(TerminalError::ShellUnavailable(ShellKind::Pwsh)),
            spawned: 0,
        };
        let id = sid('c');
        let out = run(
            &mut manager,
            &mut factory,
            open_frame(&id),
            GateState::default(),
        );
        assert!(!manager.contains(&id));
        assert_eq!(out.replies[0]["code"], "terminal_shell_unavailable");
        assert!(out.audits.is_empty() || matches!(out.audits[0], AuditEvent::GateRefused { .. }));
    }

    #[test]
    fn fifth_open_hits_session_limit() {
        let mut manager = SessionManager::default();
        let mut factory = Factory {
            result: None,
            spawned: 0,
        };
        for n in ['1', '2', '3', '4'] {
            run(
                &mut manager,
                &mut factory,
                open_frame(&sid(n)),
                GateState::default(),
            );
        }
        let out = run(
            &mut manager,
            &mut factory,
            open_frame(&sid('5')),
            GateState::default(),
        );
        assert_eq!(factory.spawned, 4);
        assert_eq!(out.replies[0]["code"], "terminal_session_limit");
    }

    #[test]
    fn input_under_gate_is_refused_but_close_still_works() {
        let mut manager = SessionManager::default();
        let mut factory = Factory {
            result: None,
            spawned: 0,
        };
        let id = sid('d');
        run(
            &mut manager,
            &mut factory,
            open_frame(&id),
            GateState::default(),
        );
        let gate = GateState {
            maintenance: true,
            ..GateState::default()
        };
        let refused = run(
            &mut manager,
            &mut factory,
            ClientFrame::Input {
                session_id: id.clone(),
                data: b"dir\r".to_vec(),
            },
            gate,
        );
        assert_eq!(refused.replies[0]["code"], "terminal_local_gate");
        assert!(matches!(
            refused.audits[0],
            AuditEvent::GateRefused { op: "input", .. }
        ));
        let closed = run(
            &mut manager,
            &mut factory,
            ClientFrame::Close {
                session_id: id.clone(),
            },
            gate,
        );
        assert!(!manager.contains(&id));
        assert_eq!(closed.replies[0]["type"], "closed");
        assert_eq!(closed.replies[0]["reason"], CloseReason::User.as_str());
        assert!(matches!(closed.audits[0], AuditEvent::SessionClosed { .. }));
    }

    #[test]
    fn unknown_session_replies_not_found_with_session_id() {
        let mut manager = SessionManager::default();
        let mut factory = Factory {
            result: None,
            spawned: 0,
        };
        let id = sid('e');
        for frame in [
            ClientFrame::Input {
                session_id: id.clone(),
                data: vec![1],
            },
            ClientFrame::Resize {
                session_id: id.clone(),
                size: TerminalSize::new(80, 24).unwrap(),
            },
            ClientFrame::Close {
                session_id: id.clone(),
            },
        ] {
            let out = run(&mut manager, &mut factory, frame, GateState::default());
            assert_eq!(out.replies[0]["code"], "terminal_not_found");
            assert_eq!(out.replies[0]["session_id"], id);
        }
    }

    #[test]
    fn ping_replies_pong_with_seq_zero() {
        let mut manager = SessionManager::default();
        let mut factory = Factory {
            result: None,
            spawned: 0,
        };
        let out = run(
            &mut manager,
            &mut factory,
            ClientFrame::Ping,
            GateState::default(),
        );
        assert_eq!(out.replies, vec![json!({"type": "pong", "seq": 0})]);
    }

    #[test]
    fn output_frames_shift_seq_by_one_so_opened_keeps_seq_one() {
        let chunks = vec![
            OutputChunk {
                seq: 1,
                data: b"PS> ".to_vec(),
            },
            OutputChunk {
                seq: 2,
                data: vec![0xe4, 0xb8, 0xad],
            },
        ];
        let frames = output_frames(&sid('f'), &chunks);
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0]["seq"], 2);
        assert_eq!(frames[1]["seq"], 3);
        for frame in &frames {
            assert_eq!(frame["type"], "output");
            assert_eq!(frame["session_id"], sid('f'));
            assert_eq!(frame["stream"], "stdout");
        }
        let decoded = hbb_common::base64::Engine::decode(
            &hbb_common::base64::engine::general_purpose::STANDARD,
            frames[1]["data"].as_str().unwrap(),
        )
        .unwrap();
        assert_eq!(decoded, vec![0xe4, 0xb8, 0xad]);
    }

    #[test]
    fn largest_pty_chunk_fits_one_frame_under_the_message_limit() {
        let chunk = OutputChunk {
            seq: 9,
            data: vec![0xAB; MAX_OUTPUT_CHUNK_BYTES],
        };
        let frames = output_frames(&sid('g'), &[chunk]);
        assert_eq!(frames.len(), 1);
        let text = serde_json::to_string(&frames[0]).unwrap();
        assert!(text.len() < MAX_FRAME_BYTES, "{} bytes", text.len());
    }

    #[test]
    fn pty_read_buffer_never_exceeds_the_documented_chunk_bound() {
        assert!(crate::management_agent::terminal::READ_BUFFER_BYTES <= MAX_OUTPUT_CHUNK_BYTES);
    }
}
