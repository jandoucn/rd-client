//! 本机 SYSTEM 终端的服务端：`_terminal` 是一条独立的服务级通道，每条连接对应一个自己的
//! 终端服务。
//!
//! 身份来源不在客户端而在 `TerminalServiceProxy::new` 的第三个参数：不传 `user_token` 时
//! `handle_open` 就在服务进程内直接开 PTY，也就是服务进程的身份（Windows 上是 LocalSystem）；
//! 传了 token 才会走 helper 以登录用户身份运行。
//!
//! 目录、权限、accept 时授权、连接上限与节流日志都照 `ipc/drm.rs` 的 `_drm` 通道写，区别只在
//! 这条通道要跨平台：Unix 是 0666 socket，Windows 是 `allow_everyone_create` 的命名管道。

use super::*;
use crate::{
    local_terminal::LocalRequest,
    server::terminal_service::{self, TerminalServiceProxy},
};
use base::message_proto::TerminalResponse;
use hbb_common::protobuf::Message as _;

/// 每条连接都会在服务的终端服务表里占一项（表本身另有总量上限），而这条通道对任何本机用户
/// 可达，所以连接数必须在这里封顶。
const MAX_TERMINAL_CONNS: usize = 8;

/// 授权会做一次活跃用户查询与可执行文件比对，代价不便宜；先限住并发量，避免被大量连接拖住。
const MAX_TERMINAL_AUTH_IN_FLIGHT: usize = 4;

/// 单轮最多等这么久就回去推一次输出，与远端终端约 30fps 的读取节奏一致。
const TERMINAL_POLL_MS: u64 = 15;

/// 拒绝日志的节流间隔：这些路径可被任意本机进程触发，不能一条都不限流地写进服务日志。
const REJECT_LOG_INTERVAL: std::time::Duration = std::time::Duration::from_secs(5);

fn terminal_conn_admitted(prev_count: usize) -> bool {
    prev_count < MAX_TERMINAL_CONNS
}

fn terminal_auth_admitted(prev_in_flight: usize) -> bool {
    prev_in_flight < MAX_TERMINAL_AUTH_IN_FLIGHT
}

/// accept 之后的授权，判据与 `_drm` 完全一致，不额外收紧也不再放宽。
fn terminal_peer_authorized(stream: &Connection) -> bool {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        super::ipc_auth::authorize_service_scoped_ipc_connection(
            stream,
            crate::local_terminal::POSTFIX,
        )
    }
    #[cfg(windows)]
    {
        super::ipc_auth::authorize_windows_main_ipc_connection(
            stream,
            crate::local_terminal::POSTFIX,
        )
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    {
        // 本仓库只在这三个平台上构建；别处没有现成的判据，就一律拒绝，宁可不可用也不留一条
        // 能直接拿到 SYSTEM shell 却没有身份校验的通道。
        let _ = stream;
        false
    }
}

/// Bind the `_terminal` listener: any local user may connect, authorization happens per connection.
fn new_terminal_listener() -> ResultType<Incoming> {
    let path = super::terminal_ipc_path();
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        let _ = ensure_secure_ipc_parent_dir(&path, crate::POSTFIX_SERVICE)?;
        // 与 `_drm` 一样走 fd 版本：`unlink(2)` 对目录型占位返回 EISDIR，fd 版本才会按
        // `AT_REMOVEDIR` 处理空目录。
        if let Err(err) = remove_ipc_entry_via_secure_parent_fd(&path) {
            log::warn!(
                "terminal: could not clear a stale entry at {}: {}",
                &path,
                err
            );
        }
    }
    let mut endpoint = Endpoint::new(path.clone());
    endpoint.set_security_attributes(SecurityAttributes::allow_everyone_create()?);
    let incoming = endpoint.incoming()?;
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o666)).map_err(|err| {
        std::fs::remove_file(&path).ok();
        err
    })?;
    log::info!("Started terminal ipc server at path: {}", &path);
    Ok(incoming)
}

/// 一帧 IPC 输入对这条连接的处置。
enum TerminalInput {
    /// 一条可以执行的本机终端请求。
    Request(LocalRequest),
    /// 与本通道无关、解不出、或是不认识的动作：同机进程写进来的，丢掉就行，不必断开。
    Ignore,
    /// 对端关闭或把连接写坏了：这条连接到此为止。
    Closed,
}

fn classify_input(frame: Option<ResultType<Option<Data>>>) -> TerminalInput {
    match frame {
        // 到点没等到东西：这一轮只是没有输入，输出照常推。
        None => TerminalInput::Ignore,
        Some(Err(_)) => TerminalInput::Closed,
        Some(Ok(None)) => TerminalInput::Ignore,
        Some(Ok(Some(Data::LocalTerminal(bytes)))) => match LocalRequest::decode(&bytes) {
            Some(request) => TerminalInput::Request(request),
            // 解不出或不认识的动作一律不执行，但连接要留着：界面的其它动作还能继续用。
            None => TerminalInput::Ignore,
        },
        Some(Ok(Some(_))) => TerminalInput::Ignore,
    }
}

/// 服务表里的这一项必须跟着连接走：正常断开、出错返回、以及 panic 都要清掉，否则每断开
/// 一次就漏一个非持久服务，最后把服务的服务表占满。
struct TerminalServiceGuard(String);

impl Drop for TerminalServiceGuard {
    fn drop(&mut self) {
        terminal_service::drop_service(&self.0);
    }
}

async fn handle_terminal_conn(stream: Connection) -> ResultType<()> {
    use std::sync::atomic::{AtomicUsize, Ordering};

    static AUTH_IN_FLIGHT: AtomicUsize = AtomicUsize::new(0);
    struct AuthGuard;
    impl Drop for AuthGuard {
        fn drop(&mut self) {
            AUTH_IN_FLIGHT.fetch_sub(1, Ordering::SeqCst);
        }
    }

    if !terminal_auth_admitted(AUTH_IN_FLIGHT.fetch_add(1, Ordering::SeqCst)) {
        AUTH_IN_FLIGHT.fetch_sub(1, Ordering::SeqCst);
        // 故意用 `debug`：这条路径对任意本机 uid 可达，能进服务日志的级别就是一个无界的日志
        // 写入原语。
        log::debug!(
            "terminal: too many _terminal authorizations in flight; dropping this connection"
        );
        return Ok(());
    }
    let auth_guard = AuthGuard;
    let (mut conn, authorized) = tokio::task::spawn_blocking(move || {
        let authorized = terminal_peer_authorized(&stream);
        (stream, authorized)
    })
    .await?;
    drop(auth_guard);
    if !authorized {
        // 判据自己已经按节流写过了拒绝原因，这里不再补一行同样会被刷的日志。
        return Ok(());
    }

    static TERMINAL_CONN_COUNT: AtomicUsize = AtomicUsize::new(0);
    struct ConnGuard;
    impl Drop for ConnGuard {
        fn drop(&mut self) {
            TERMINAL_CONN_COUNT.fetch_sub(1, Ordering::SeqCst);
        }
    }
    if !terminal_conn_admitted(TERMINAL_CONN_COUNT.fetch_add(1, Ordering::SeqCst)) {
        TERMINAL_CONN_COUNT.fetch_sub(1, Ordering::SeqCst);
        hbb_common::throttled_log!(
            REJECT_LOG_INTERVAL,
            warn,
            "terminal: too many concurrent _terminal connections (>= {}); rejecting",
            MAX_TERMINAL_CONNS
        );
        return Ok(());
    }
    let _conn_guard = ConnGuard;

    let service_id = terminal_service::generate_service_id();
    if let Err(err) = terminal_service::ensure_service(&service_id, false) {
        hbb_common::throttled_log!(
            REJECT_LOG_INTERVAL,
            warn,
            "terminal: could not create a terminal service for this connection: {}",
            err
        );
        return Ok(());
    }
    let _service_guard = TerminalServiceGuard(service_id.clone());
    // 第三个参数传 `None`：不传用户令牌，`handle_open` 才会在服务进程内直接开 PTY。
    let mut proxy = TerminalServiceProxy::new(service_id, Some(false), None);

    loop {
        match classify_input(conn.next_timeout2(TERMINAL_POLL_MS).await) {
            TerminalInput::Request(request) => {
                match proxy.handle_action(&request.to_action()) {
                    Ok(Some(response)) => {
                        if !send_response(&mut conn, &response).await {
                            break;
                        }
                    }
                    Ok(None) => {}
                    // 单个动作失败不带崩整条连接：界面的其它动作还能继续用。
                    Err(err) => log::debug!("terminal: action failed: {}", err),
                }
            }
            TerminalInput::Ignore => {}
            TerminalInput::Closed => {
                log::debug!("terminal ipc connection closed");
                break;
            }
        }
        for response in proxy.read_outputs() {
            if !send_response(&mut conn, &response).await {
                return Ok(());
            }
        }
    }
    Ok(())
}

/// 发一条响应；返回 `false` 表示对端已经不在了，调用方该收尾。
async fn send_response(conn: &mut Connection, response: &TerminalResponse) -> bool {
    match response.write_to_bytes() {
        Ok(bytes) => match conn.send(&Data::LocalTerminalResponse(bytes)).await {
            Ok(()) => true,
            Err(err) => {
                log::debug!("terminal: could not send a response: {}", err);
                false
            }
        },
        Err(err) => {
            log::warn!("terminal: could not encode a response: {}", err);
            true
        }
    }
}

#[tokio::main(flavor = "current_thread")]
pub async fn start_terminal() {
    match new_terminal_listener() {
        Ok(mut incoming) => loop {
            match incoming.next().await {
                Some(Ok(stream)) => {
                    tokio::spawn(async move {
                        if let Err(err) = handle_terminal_conn(Connection::new(stream)).await {
                            log::info!("terminal ipc connection ended: {}", err);
                        }
                    });
                }
                Some(Err(err)) => log::error!("Couldn't get terminal client: {:?}", err),
                None => {
                    log::error!("terminal ipc listener stream ended; stopping terminal server");
                    break;
                }
            }
        },
        Err(err) => {
            log::error!("Failed to start terminal ipc server: {}", err);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base::message_proto::terminal_response::Union;

    fn open_frame(terminal_id: i32) -> Vec<u8> {
        LocalRequest::Open {
            terminal_id,
            rows: 24,
            cols: 80,
        }
        .encode()
        .unwrap()
    }

    #[test]
    fn admission_bounds_follow_the_drm_shape() {
        assert!(terminal_conn_admitted(0));
        assert!(terminal_conn_admitted(MAX_TERMINAL_CONNS - 1)); // 最后一格
        assert!(!terminal_conn_admitted(MAX_TERMINAL_CONNS));
        assert!(!terminal_conn_admitted(MAX_TERMINAL_CONNS + 5));

        assert!(terminal_auth_admitted(0));
        assert!(terminal_auth_admitted(MAX_TERMINAL_AUTH_IN_FLIGHT - 1));
        assert!(!terminal_auth_admitted(MAX_TERMINAL_AUTH_IN_FLIGHT));
        assert!(
            MAX_TERMINAL_AUTH_IN_FLIGHT <= MAX_TERMINAL_CONNS,
            "预授权闸门不能比连接上限还宽"
        );
    }

    #[test]
    fn a_terminal_request_is_decoded_into_an_action() {
        match classify_input(Some(Ok(Some(Data::LocalTerminal(open_frame(3)))))) {
            TerminalInput::Request(request) => assert_eq!(
                request,
                LocalRequest::Open {
                    terminal_id: 3,
                    rows: 24,
                    cols: 80
                }
            ),
            _ => panic!("a well-formed request must be executed"),
        }
    }

    #[test]
    fn a_broken_frame_is_ignored_instead_of_closing_the_connection() {
        for bytes in [
            b"".to_vec(),
            b"not json".to_vec(),
            br#"{"Unknown":{"terminal_id":1}}"#.to_vec(),
        ] {
            assert!(
                matches!(
                    classify_input(Some(Ok(Some(Data::LocalTerminal(bytes))))),
                    TerminalInput::Ignore
                ),
                "an undecodable request must be dropped, not acted on"
            );
        }
    }

    #[test]
    fn unrelated_ipc_variants_and_empty_frames_are_ignored() {
        // 每条连接是一个独立终端服务，别的聊天变体与这条通道无关。
        assert!(matches!(
            classify_input(Some(Ok(Some(Data::Close)))),
            TerminalInput::Ignore
        ));
        assert!(matches!(
            classify_input(Some(Ok(Some(Data::Test)))),
            TerminalInput::Ignore
        ));
        // 帧完整但不在 `Data` 里。
        assert!(matches!(
            classify_input(Some(Ok(None))),
            TerminalInput::Ignore
        ));
        // 本轮没等到输入。
        assert!(matches!(classify_input(None), TerminalInput::Ignore));
    }

    #[test]
    fn a_peer_close_ends_the_connection() {
        let closed = Some(Err(hbb_common::anyhow::anyhow!("reset by the peer")));
        assert!(matches!(classify_input(closed), TerminalInput::Closed));
    }

    /// 服务端 `write_to_bytes` 编出来的字节就是客户端 `parse_from_bytes` 读的东西，中间没有
    /// 第二层封装；这里把两端钉在一起。
    #[test]
    fn a_response_survives_the_wire_the_client_reads() {
        let mut response = TerminalResponse::new();
        let mut data = base::message_proto::TerminalData::new();
        data.terminal_id = 5;
        data.data = vec![0xe4, 0xb8, 0xad].into();
        response.set_data(data);

        let bytes = response.write_to_bytes().unwrap();
        let decoded = TerminalResponse::parse_from_bytes(&bytes).unwrap();
        match decoded.union {
            Some(Union::Data(d)) => {
                assert_eq!(d.terminal_id, 5);
                assert_eq!(d.data.to_vec(), vec![0xe4, 0xb8, 0xad]);
            }
            other => panic!("expected terminal data, got {other:?}"),
        }
    }

    /// 客户端与服务端必须用同一套路径，否则连不上。Unix 上它落在 `_service` 的目录里
    /// （那是 root 服务与用户进程共享的目录），而不是按当前 uid 分的目录。
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn the_terminal_path_shares_the_service_directory() {
        let terminal = std::path::Path::new(&super::terminal_ipc_path()).to_path_buf();
        let service = Config::ipc_path(crate::POSTFIX_SERVICE);
        assert_eq!(
            terminal.parent(),
            std::path::Path::new(&service).parent(),
            "the user process must be able to reach the service-owned directory"
        );
        assert_eq!(
            terminal.file_name().and_then(|n| n.to_str()),
            Some("ipc_terminal")
        );
        assert_ne!(
            terminal,
            std::path::Path::new(&Config::ipc_path(crate::local_terminal::POSTFIX)),
            "the terminal channel must not collide with the per-uid `_terminal` path"
        );
    }
}
