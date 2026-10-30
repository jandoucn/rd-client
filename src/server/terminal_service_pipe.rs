//! 没有 ConPTY 时用匿名管道打开远程终端会话（SYSTEM / 管理员终端等不经 helper 的路径）。
//!
//! 读写线程与会话登记照搬 `handle_open` 的 ConPTY 路径；不同的只是 shell 由
//! [`PipeShell`] 承载，`pty_pair` 留空，调整窗口大小因此不生效。

use crate::server::{
    terminal_pipe_win::{apply_temp_override, shell_environment, PipeShell},
    terminal_service::*,
};

impl TerminalServiceProxy {
    pub(super) fn handle_open_with_pipe(
        &self,
        service: &mut PersistentTerminalService,
        open: &OpenTerminal,
    ) -> Result<Option<TerminalResponse>> {
        let mut response = TerminalResponse::new();
        log::info!(
            "Creating new terminal {} on pipes (no ConPTY) for service {}",
            open.terminal_id,
            service.service_id
        );
        let mut session =
            TerminalSession::new(open.terminal_id, open.rows as u16, open.cols as u16);

        let shell = get_default_shell();
        // 与 ConPTY 路径同一来源的环境，含注册表变量与 TEMP 修复。
        let mut env_source = CommandBuilder::new(&shell);
        apply_temp_override(&mut env_source);
        let environment = shell_environment(&env_source);
        let PipeShell {
            reader,
            writer,
            child,
        } = PipeShell::spawn(&shell, Some(&environment))
            .context("Failed to spawn shell on pipes")?;
        let child: Box<dyn Child + Send + Sync> = Box::new(child);
        session.is_pipe_mode = true;

        session.pid = child.process_id().unwrap_or(0);

        // Create channels for input/output
        let (input_tx, input_rx) = mpsc::sync_channel::<Vec<u8>>(CHANNEL_BUFFER_SIZE);
        let (output_tx, output_rx) = mpsc::sync_channel::<Vec<u8>>(CHANNEL_BUFFER_SIZE);

        // Spawn writer thread
        let terminal_id = open.terminal_id;
        let writer_thread = thread::spawn(move || {
            let mut writer = writer;
            while let Ok(data) = input_rx.recv() {
                if let Err(e) = writer.write_all(&data) {
                    log::error!("Terminal {} write error: {}", terminal_id, e);
                    break;
                }
                if let Err(e) = writer.flush() {
                    log::error!("Terminal {} flush error: {}", terminal_id, e);
                }
            }
            log::debug!("Terminal {} writer thread exiting", terminal_id);
        });

        let exiting = session.exiting.clone();
        // Spawn reader thread
        let terminal_id = open.terminal_id;
        let reader_thread = thread::spawn(move || {
            let mut reader = reader;
            let mut buf = vec![0u8; 4096];
            let mut utf8_chunks = Utf8ChunkAccumulator::default();
            let mut drop_count: u64 = 0;
            // Initialize to > 5s ago so the first drop triggers a warning immediately.
            let mut last_drop_warn = Instant::now() - Duration::from_secs(6);
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => {
                        // EOF
                        // This branch can be reached when the child process exits on macOS.
                        // But not on Linux and Windows in my tests.
                        if let Some(data) = utf8_chunks.finish() {
                            let _ = try_send_output(
                                &output_tx,
                                data,
                                terminal_id,
                                "",
                                &mut drop_count,
                                &mut last_drop_warn,
                            );
                        }
                        break;
                    }
                    Ok(n) => {
                        if exiting.load(Ordering::SeqCst) {
                            break;
                        }
                        let Some(data) = utf8_chunks.push_chunk(buf[..n].to_vec()) else {
                            continue;
                        };
                        // Use try_send to avoid blocking the reader thread when channel is full.
                        // During disconnect, the run loop (sp.ok()) stops and read_outputs() is
                        // no longer called, so the channel won't be drained. Blocking send would
                        // deadlock the reader thread in that case.
                        // Note: data produced during disconnect may be lost if channel fills up,
                        // since output_buffer is only updated in read_outputs(). The buffer will
                        // contain history from before the disconnect, not data produced after it.
                        if try_send_output(
                            &output_tx,
                            data,
                            terminal_id,
                            "",
                            &mut drop_count,
                            &mut last_drop_warn,
                        ) {
                            break;
                        }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        // This branch is not reached in my tests, but we still add `exiting` check to ensure we can exit.
                        if exiting.load(Ordering::SeqCst) {
                            break;
                        }
                        // For non-blocking I/O, sleep briefly
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(e) => {
                        log::error!("Terminal {} read error: {}", terminal_id, e);
                        break;
                    }
                }
            }
            log::debug!("Terminal {} reader thread exiting", terminal_id);
        });

        session.child = Some(child);
        session.input_tx = Some(input_tx);
        session.output_rx = Some(output_rx);
        session.reader_thread = Some(reader_thread);
        session.writer_thread = Some(writer_thread);
        session.state = SessionState::Active {
            pending_buffer: None,
            sigwinch: SigwinchPhase::Idle,
        };

        let mut opened = TerminalOpened::new();
        opened.terminal_id = open.terminal_id;
        opened.success = true;
        opened.message = "Terminal opened".to_string();
        opened.pid = session.pid;
        opened.service_id = service.service_id.clone();
        if service.needs_session_sync {
            if !service.sessions.is_empty() {
                opened.persistent_sessions = service.sessions.keys().cloned().collect();
            }
            service.needs_session_sync = false;
        }
        response.set_opened(opened);

        log::info!(
            "Terminal {} opened successfully with PID {}",
            open.terminal_id,
            session.pid
        );

        // Store the session
        service
            .sessions
            .insert(open.terminal_id, Arc::new(Mutex::new(session)));

        Ok(Some(response))
    }
}

#[cfg(test)]
#[path = "terminal_service_pipe_tests.rs"]
mod tests;
