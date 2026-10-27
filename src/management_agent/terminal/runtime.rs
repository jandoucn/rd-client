//! 终端 WS 的 Windows 运行层：专用线程里拨出站 WS、签名握手、驱动 `transport` 的纯逻辑。
//!
//! 只在 Windows 编译。线程内是阻塞式 tungstenite，不创建 Tokio 运行时；PTY 与会话阻塞调用
//! 都在这条线程里完成，不会落在异步线程上。

use super::{
    manager::{AuditEvent, GateState, SessionManager, TerminalSlot},
    protocol,
    transport::{self, Backoff, Dispatch, Liveness, SlotFactory, MAX_FRAME_BYTES},
    OpenRequest, PtySession, ShellLocations, TerminalError, AGENT_RING_BYTES,
};
use crate::management_agent::{
    identity::{self, DeviceIdentity},
    local_events, recovery, AgentPaths, AgentState,
};
use hbb_common::{
    base64::{engine::general_purpose::STANDARD, Engine as _},
    sodiumoxide::crypto::sign,
};
use std::{
    io::ErrorKind,
    net::TcpStream,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
use tungstenite::{
    client::IntoClientRequest, protocol::WebSocketConfig, stream::MaybeTlsStream, Message,
    WebSocket,
};

const POLL_SLICE: Duration = Duration::from_millis(50);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const SERVICE_API: &str = "https://rdapi.yan.life";

/// 与 `system.disable_uac` 同一套四个本地信号。
pub fn read_gate(paths: &AgentPaths) -> GateState {
    GateState {
        disable_recovery: crate::management_agent::purge_marker_path(paths).exists(),
        disable_agent: paths.state.join("disable-agent").exists(),
        maintenance: paths.state.join("maintenance.json").exists(),
        stop_requested: !matches!(
            crate::management_agent::recovery_windows::read_intent(),
            Ok(recovery::Intent::Enabled)
        ),
    }
}

struct PtyFactory;

impl SlotFactory for PtyFactory {
    fn spawn(&mut self, request: &OpenRequest) -> Result<Box<dyn TerminalSlot>, TerminalError> {
        let locations = ShellLocations::system();
        let command = super::resolve_shell(request.shell, &locations, |path| path.exists())?;
        let cwd = request.cwd.as_deref().map(std::path::Path::new);
        let session = PtySession::spawn(&command, cwd, request.size, AGENT_RING_BYTES)?;
        Ok(Box::new(session))
    }
}

/// 每个会话已发往 broker 的输出块序号，用于只推送新增输出。
#[derive(Default)]
struct Cursors(std::collections::HashMap<String, u64>);

impl Cursors {
    fn forget(&mut self, session_id: &str) {
        self.0.remove(session_id);
    }
}

type Socket = WebSocket<MaybeTlsStream<TcpStream>>;

fn connect(
    identity: &DeviceIdentity,
    installation_id: &str,
    generation: u64,
) -> Result<Socket, String> {
    let query = transport::handshake_query(&identity.id, &identity.uuid, installation_id, generation);
    let url = transport::ws_url(SERVICE_API, &query).map_err(str::to_owned)?;
    let timestamp = crate::management_agent::unix_timestamp() as i64;
    let nonce = hex::encode(hbb_common::sodiumoxide::randombytes::randombytes(16));
    let canonical = transport::handshake_canonical(&identity.id, &identity.uuid, timestamp, &nonce);
    let signature = STANDARD.encode(sign::sign_detached(canonical.as_bytes(), &identity.secret).to_bytes());

    let mut request = url.into_client_request().map_err(|e| e.to_string())?;
    let headers = request.headers_mut();
    for (name, value) in [
        ("X-RustDesk-Device-ID", identity.id.clone()),
        ("X-RustDesk-Device-Public-Key", STANDARD.encode(identity.public.as_ref())),
        ("X-RustDesk-Device-Timestamp", timestamp.to_string()),
        ("X-RustDesk-Device-Nonce", nonce),
        ("X-RustDesk-Device-Signature", signature),
    ] {
        headers.insert(
            name,
            value.parse().map_err(|_| "terminal handshake header invalid".to_owned())?,
        );
    }
    let host = request.uri().host().ok_or("terminal ws host missing")?.to_owned();
    let port = request.uri().port_u16().unwrap_or(443);
    let tcp = {
        use std::net::ToSocketAddrs;
        let address = (host.as_str(), port)
            .to_socket_addrs()
            .map_err(|e| e.to_string())?
            .next()
            .ok_or("terminal ws host unresolved")?;
        TcpStream::connect_timeout(&address, CONNECT_TIMEOUT).map_err(|e| e.to_string())?
    };
    let _ = tcp.set_nodelay(true);
    let config = WebSocketConfig::default().max_message_size(Some(MAX_FRAME_BYTES));
    let (socket, response) =
        tungstenite::client_tls_with_config(request, tcp, Some(config), None)
            .map_err(|e| e.to_string())?;
    if response.status().as_u16() != 101 {
        return Err(format!("terminal ws upgrade status {}", response.status()));
    }
    match socket.get_ref() {
        MaybeTlsStream::Plain(stream) => stream.set_read_timeout(Some(POLL_SLICE)),
        MaybeTlsStream::NativeTls(stream) => stream.get_ref().set_read_timeout(Some(POLL_SLICE)),
        _ => Ok(()),
    }
    .map_err(|e| e.to_string())?;
    Ok(socket)
}

fn send(socket: &mut Socket, value: &serde_json::Value) -> Result<(), String> {
    let text = serde_json::to_string(value).map_err(|e| e.to_string())?;
    if text.len() > MAX_FRAME_BYTES {
        return Err("terminal frame exceeds message limit".into());
    }
    socket.send(Message::text(text)).map_err(|e| e.to_string())
}

fn flush_dispatch(socket: &mut Socket, out: Dispatch, audit: &mut dyn FnMut(AuditEvent)) -> Result<(), String> {
    for reply in &out.replies {
        send(socket, reply)?;
    }
    for event in out.audits {
        audit(event);
    }
    Ok(())
}

/// 推送各会话的新增输出、退出与超时回收。
fn pump(
    socket: &mut Socket,
    manager: &mut SessionManager,
    cursors: &mut Cursors,
    audit: &mut dyn FnMut(AuditEvent),
) -> Result<(), String> {
    let ids: Vec<String> = cursors.0.keys().cloned().collect();
    for id in ids {
        let last = cursors.0.get(&id).copied().unwrap_or(0);
        let chunks = match manager.read(&id, last, transport::MAX_OUTPUT_CHUNK_BYTES * 8) {
            Ok(chunks) => chunks,
            Err(TerminalError::NotFound) => {
                cursors.forget(&id);
                continue;
            }
            Err(error) => {
                send(socket, &protocol::error(0, Some(&id), &error))?;
                continue;
            }
        };
        for frame in transport::output_frames(&id, &chunks) {
            send(socket, &frame)?;
        }
        if let Some(chunk) = chunks.last() {
            cursors.0.insert(id.clone(), chunk.seq);
        }
    }
    for (id, code, event) in manager.drain_exited() {
        let last = cursors.0.get(&id).copied().unwrap_or(0);
        send(socket, &protocol::exited(last + 2, &id, code, 0))?;
        send(socket, &protocol::closed(0, &id, super::CloseReason::User))?;
        cursors.forget(&id);
        audit(event);
    }
    for event in manager.reap_expired(Instant::now()) {
        if let AuditEvent::SessionClosed { session_id, .. } = &event {
            send(socket, &protocol::closed(0, session_id, super::CloseReason::Idle))?;
            cursors.forget(session_id);
        }
        audit(event);
    }
    Ok(())
}

fn serve(
    socket: &mut Socket,
    paths: &AgentPaths,
    stop: &AtomicBool,
    audit: &mut dyn FnMut(AuditEvent),
) -> Result<(), String> {
    let mut manager = SessionManager::default();
    let mut factory = PtyFactory;
    let mut cursors = Cursors::default();
    let mut last_frame = Instant::now();
    let mut pinged = false;
    loop {
        if stop.load(Ordering::SeqCst) {
            return Ok(());
        }
        match socket.read() {
            Ok(Message::Text(text)) => {
                last_frame = Instant::now();
                pinged = false;
                let value: serde_json::Value =
                    serde_json::from_str(text.as_str()).map_err(|e| e.to_string())?;
                match protocol::parse_client_frame(&value) {
                    Ok(frame) => {
                        let opened = matches!(frame, protocol::ClientFrame::Open(_));
                        let out = transport::dispatch(
                            &mut manager,
                            &mut factory,
                            frame,
                            read_gate(paths),
                            crate::management_agent::unix_timestamp() * 1000,
                        );
                        if opened {
                            for reply in &out.replies {
                                if reply["type"] == "opened" {
                                    if let Some(id) = reply["session_id"].as_str() {
                                        cursors.0.insert(id.to_owned(), 0);
                                    }
                                }
                            }
                        }
                        flush_dispatch(socket, out, audit)?;
                    }
                    Err(error) => send(socket, &protocol::error(0, None, &error))?,
                }
            }
            Ok(Message::Ping(_)) | Ok(Message::Pong(_)) => {
                last_frame = Instant::now();
                pinged = false;
            }
            Ok(Message::Close(_)) => return Ok(()),
            Ok(_) => {}
            Err(tungstenite::Error::Io(error))
                if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
            Err(error) => return Err(error.to_string()),
        }
        pump(socket, &mut manager, &mut cursors, audit)?;
        match transport::liveness(last_frame, Instant::now()) {
            Liveness::Alive => {}
            Liveness::SendPing if !pinged => {
                pinged = true;
                send(socket, &serde_json::json!({ "type": "ping" }))?;
            }
            Liveness::SendPing => {}
            Liveness::Dead => return Err("terminal ws heartbeat timeout".into()),
        }
    }
}

/// 从本机状态与绑定读出握手所需身份；未就绪返回 `None`，调用方休眠后重试。
pub fn load_handshake_identity(paths: &AgentPaths) -> Option<(DeviceIdentity, String, u64)> {
    let identity = identity::load_existing().ok()?;
    let bytes = std::fs::read(paths.state.join("agent.json")).ok()?;
    let state: AgentState = serde_json::from_slice(&bytes).ok()?;
    if state.agent_installation_id.is_empty() {
        return None;
    }
    let binding_path = paths.state.join("local-events").join("events.json");
    let log = local_events::LocalEvents::open(&binding_path).ok()?;
    let binding = log.verified_binding().filter(|binding| {
        binding.client_id == identity.id
            && binding.client_uuid == identity.uuid
            && binding.agent_installation_id == state.agent_installation_id
            && binding.enrollment_generation > 0
    })?;
    Some((
        identity,
        state.agent_installation_id,
        binding.enrollment_generation,
    ))
}

fn log_audit(event: AuditEvent) {
    match event {
        AuditEvent::SessionOpened {
            session_id,
            shell,
            cols,
            rows,
        } => hbb_common::log::info!(
            "terminal audit open session={session_id} shell={} {cols}x{rows}",
            shell.name()
        ),
        AuditEvent::SessionClosed {
            session_id,
            reason,
            duration_ms,
        } => hbb_common::log::info!(
            "terminal audit close session={session_id} reason={} duration_ms={duration_ms}",
            reason.as_str()
        ),
        AuditEvent::GateRefused {
            session_id,
            gate,
            op,
        } => hbb_common::log::info!(
            "terminal audit gate refused op={op} gate={gate} session={}",
            session_id.as_deref().unwrap_or("-")
        ),
    }
}

/// 常驻线程入口：声明能力且身份就绪时保持一条出站终端 WS，断线指数退避重连。
/// 每次（重）连成功都从全新的 `SessionManager` 开始，上一条连接的会话随其析构被结束（v1 不续会话）。
pub fn run(paths: AgentPaths, stop: Arc<AtomicBool>) {
    let mut backoff = Backoff::default();
    while !stop.load(Ordering::SeqCst) {
        let Some((identity, installation_id, generation)) = load_handshake_identity(&paths) else {
            std::thread::sleep(Duration::from_secs(5));
            continue;
        };
        match connect(&identity, &installation_id, generation) {
            Ok(mut socket) => {
                backoff.reset();
                if let Err(error) = serve(&mut socket, &paths, &stop, &mut log_audit) {
                    hbb_common::throttled_log!(
                        Duration::from_secs(300),
                        warn,
                        "terminal channel dropped: {error}"
                    );
                }
                let _ = socket.close(None);
            }
            Err(error) => hbb_common::throttled_log!(
                Duration::from_secs(300),
                warn,
                "terminal channel connect failed: {error}"
            ),
        }
        let delay = backoff.next_delay(hbb_common::rand::random::<f64>());
        let until = Instant::now() + delay;
        while Instant::now() < until && !stop.load(Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(200));
        }
    }
}
