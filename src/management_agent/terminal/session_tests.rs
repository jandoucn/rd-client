use super::*;
use std::time::{Duration, Instant};

const WAIT: Duration = Duration::from_secs(30);

fn collected(session: &PtySession) -> Vec<u8> {
    session
        .read_since(0, usize::MAX)
        .expect("test output fits in the ring")
        .into_iter()
        .flat_map(|chunk| chunk.data)
        .collect()
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

fn wait_for(session: &PtySession, needle: &[u8]) -> Vec<u8> {
    let deadline = Instant::now() + WAIT;
    loop {
        let output = collected(session);
        if contains(&output, needle) {
            return output;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {:?}; output so far: {:?}",
            String::from_utf8_lossy(needle),
            String::from_utf8_lossy(&output)
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn wait_exit(session: &PtySession) -> u32 {
    let deadline = Instant::now() + WAIT;
    loop {
        if let Some(code) = session.exit_code() {
            return code;
        }
        assert!(Instant::now() < deadline, "shell did not exit");
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn size(cols: u16, rows: u16) -> TerminalSize {
    TerminalSize::new(cols, rows).unwrap()
}

#[test]
fn policy_reports_idle_and_lifetime_expiry() {
    let policy = SessionPolicy {
        idle: Duration::from_secs(60),
        max_lifetime: Duration::from_secs(600),
    };
    let start = Instant::now();
    assert_eq!(
        policy.expiry(start, start, start + Duration::from_secs(59)),
        None
    );
    assert_eq!(
        policy.expiry(start, start, start + Duration::from_secs(60)),
        Some(Expiry::Idle)
    );
    let active = start + Duration::from_secs(550);
    assert_eq!(
        policy.expiry(start, active, start + Duration::from_secs(599)),
        None
    );
    assert_eq!(
        policy.expiry(start, active, start + Duration::from_secs(600)),
        Some(Expiry::MaxLifetime)
    );
}

#[test]
fn default_policy_follows_protocol_timeouts() {
    let policy = SessionPolicy::default();
    assert_eq!(policy.idle, Duration::from_secs(30 * 60));
    assert_eq!(policy.max_lifetime, Duration::from_secs(24 * 60 * 60));
}

#[test]
fn spawn_of_missing_program_is_start_failed() {
    let command = ShellCommand {
        program: std::env::temp_dir().join("rd-terminal-missing-shell.exe"),
        args: Vec::new(),
        local_echo: false,
    };
    match PtySession::open_conpty(&command, None, size(80, 24), 1024) {
        Err(error) => assert_eq!(error.code(), "terminal_start_failed"),
        Ok(_) => panic!("missing program must not start"),
    }
}

#[cfg(unix)]
mod unix {
    use super::*;

    fn sh() -> PtySession {
        let command = ShellCommand {
            program: "/bin/sh".into(),
            args: Vec::new(),
            local_echo: false,
        };
        PtySession::open_conpty(&command, None, size(80, 24), AGENT_RING_BYTES).unwrap()
    }

    #[test]
    fn input_reaches_shell_and_output_returns() {
        let session = sh();
        assert!(session.process_id().is_some());
        session.write(b"echo RD-$((6*7))\r").unwrap();
        wait_for(&session, b"RD-42");
        session.close().unwrap();
    }

    #[test]
    fn output_bytes_are_returned_unchanged() {
        let session = sh();
        session
            .write(b"printf 'B<\\344\\270\\255\\033[0m>\\n'\r")
            .unwrap();
        wait_for(&session, b"B<\xe4\xb8\xad\x1b[0m>");
        session.close().unwrap();
    }

    #[test]
    fn resize_is_visible_to_the_shell() {
        let session = sh();
        session.resize(size(132, 40)).unwrap();
        session.write(b"stty size\r").unwrap();
        wait_for(&session, b"40 132");
        session.close().unwrap();
    }

    #[test]
    fn shell_exit_code_is_reported() {
        let session = sh();
        session.write(b"exit 3\r").unwrap();
        assert_eq!(wait_exit(&session), 3);
        assert_eq!(session.close().unwrap(), Some(3));
    }

    #[test]
    fn writes_refresh_idle_timer() {
        let session = sh();
        let policy = SessionPolicy {
            idle: Duration::from_secs(60),
            max_lifetime: Duration::from_secs(3600),
        };
        session.write(b"stty -echo; echo QUIET-$((1+1))\r").unwrap();
        wait_for(&session, b"QUIET-2");
        std::thread::sleep(Duration::from_millis(300));
        let before = session.last_activity();
        assert_eq!(
            session.expiry(&policy, before + policy.idle),
            Some(Expiry::Idle)
        );
        std::thread::sleep(Duration::from_millis(20));
        session.write(b"").unwrap();
        assert!(session.last_activity() > before);
        assert_eq!(session.expiry(&policy, before + policy.idle), None);
        session.close().unwrap();
    }

    #[test]
    fn finish_output_collects_tail_after_exit() {
        let session = sh();
        session
            .write(b"i=0; while [ $i -lt 3000 ]; do echo LINE$i; i=$((i+1)); done; echo TAIL-$((700+77)); exit 0\r")
            .unwrap();
        assert_eq!(wait_exit(&session), 0);
        session.finish_output();
        session.finish_output();
        assert!(session.output_settled());
        assert!(contains(&collected(&session), b"TAIL-777"));
        assert_eq!(session.close().unwrap(), Some(0));
    }

    #[test]
    fn sequences_are_continuous_across_reads() {
        let session = sh();
        session.write(b"echo FIRST-$((1+1))\r").unwrap();
        wait_for(&session, b"FIRST-2");
        let first = session.read_since(0, usize::MAX).unwrap();
        let last_seq = first.last().unwrap().seq;
        for (index, chunk) in first.iter().enumerate() {
            assert_eq!(chunk.seq, index as u64 + 1);
        }
        session.write(b"echo SECOND-$((2+1))\r").unwrap();
        let deadline = Instant::now() + WAIT;
        loop {
            let newer = session.read_since(last_seq, usize::MAX).unwrap();
            let bytes: Vec<u8> = newer.iter().flat_map(|c| c.data.clone()).collect();
            if contains(&bytes, b"SECOND-3") {
                assert_eq!(newer[0].seq, last_seq + 1);
                break;
            }
            assert!(Instant::now() < deadline, "second output missing");
            std::thread::sleep(Duration::from_millis(50));
        }
        session.close().unwrap();
    }
}

#[cfg(windows)]
mod windows {
    use super::*;

    fn open(kind: ShellKind) -> Result<PtySession, TerminalError> {
        let command =
            resolve_shell(kind, ShellMode::Console, &ShellLocations::system(), |path| path.exists())?;
        PtySession::open_conpty(&command, None, size(120, 30), AGENT_RING_BYTES)
    }

    fn pid_after(output: &[u8], marker: &str) -> u32 {
        let text = String::from_utf8_lossy(output);
        text.match_indices(marker)
            .filter_map(|(index, _)| {
                let digits: String = text[index + marker.len()..]
                    .chars()
                    .take_while(|c| c.is_ascii_digit())
                    .collect();
                digits.parse().ok()
            })
            .last()
            .expect("pid marker in output")
    }

    fn process_exits_within(pid: u32, timeout: Duration) -> bool {
        use ::windows::Win32::{
            Foundation::{CloseHandle, WAIT_OBJECT_0},
            System::Threading::{
                OpenProcess, WaitForSingleObject, PROCESS_QUERY_LIMITED_INFORMATION,
                PROCESS_SYNCHRONIZE,
            },
        };
        let Ok(handle) = (unsafe {
            OpenProcess(
                PROCESS_SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION,
                false,
                pid,
            )
        }) else {
            return true;
        };
        let result = unsafe { WaitForSingleObject(handle, timeout.as_millis() as u32) };
        let _ = unsafe { CloseHandle(handle) };
        result == WAIT_OBJECT_0
    }

    #[test]
    fn powershell_session_prints_computed_output() {
        let session = open(ShellKind::PowerShell).unwrap();
        session
            .write(b"Write-Output (\"RD-TERM-\" + (6*7))\r")
            .unwrap();
        wait_for(&session, b"RD-TERM-42");
        session.close().unwrap();
    }

    #[test]
    fn cmd_session_prints_computed_output() {
        let session = open(ShellKind::Cmd).unwrap();
        session.write(b"set /a 40000+2\r").unwrap();
        wait_for(&session, b"40002");
        session.close().unwrap();
    }

    #[test]
    fn pwsh_runs_when_installed_and_is_reported_otherwise() {
        let installed = ShellLocations::system()
            .program_files
            .join(r"PowerShell\7\pwsh.exe")
            .exists();
        match open(ShellKind::Pwsh) {
            Ok(session) => {
                assert!(installed);
                session
                    .write(b"Write-Output (\"PWSH-\" + $PSVersionTable.PSVersion.Major)\r")
                    .unwrap();
                wait_for(&session, b"PWSH-7");
                session.close().unwrap();
            }
            Err(error) => {
                assert!(!installed);
                assert_eq!(error, TerminalError::ShellUnavailable(ShellKind::Pwsh));
            }
        }
    }

    #[test]
    fn chinese_output_is_utf8() {
        let session = open(ShellKind::PowerShell).unwrap();
        session
            .write(b"Write-Output (\"RD-\" + [char]0x4E2D + [char]0x6587)\r")
            .unwrap();
        wait_for(&session, "RD-中文".as_bytes());
        session.close().unwrap();
    }

    #[test]
    fn chinese_input_reaches_shell_as_text() {
        let session = open(ShellKind::PowerShell).unwrap();
        session
            .write("Write-Output ('LEN-' + '中文'.Length)\r".as_bytes())
            .unwrap();
        wait_for(&session, b"LEN-2");
        session.close().unwrap();
    }

    #[test]
    fn resize_changes_console_window() {
        let session = open(ShellKind::PowerShell).unwrap();
        session.write(b"Write-Output (\"READY-\" + 1)\r").unwrap();
        wait_for(&session, b"READY-1");
        session.resize(size(132, 40)).unwrap();
        session
            .write(b"$s = $Host.UI.RawUI.WindowSize; Write-Output (\"SIZE=\" + $s.Width + \"x\" + $s.Height)\r")
            .unwrap();
        wait_for(&session, b"SIZE=132x40");
        session.close().unwrap();
    }

    /// V2：类全屏 TUI——VT 备用屏 + 顶/底标记 + 大尺寸 resize 后重绘。
    /// Windows 无 vim/top 时用 ConPTY 对 VT 全屏交互的等价验证。
    #[test]
    fn fullscreen_tui_survives_large_resize_and_redraws() {
        let session = open(ShellKind::PowerShell).unwrap();
        session.write(b"Write-Output READY-TUI\r").unwrap();
        wait_for(&session, b"READY-TUI");

        session.resize(size(160, 40)).unwrap();
        std::thread::sleep(Duration::from_millis(300));
        // 单行：进入备用屏、清屏、顶/底标记、报告窗口尺寸
        session
            .write(
                b"$e=[char]27; Write-Host -NoNewline ($e+'[?1049h'+$e+'[2J'+$e+'[H'+'TUI-TOP-A'); Write-Host -NoNewline ($e+'[40;1H'+'TUI-BOT-A'); $s=$Host.UI.RawUI.WindowSize; Write-Output ('TUI-OK-'+$s.Width+'x'+$s.Height+'-A')\r",
            )
            .unwrap();
        wait_for(&session, b"TUI-TOP-A");
        wait_for(&session, b"TUI-BOT-A");
        let first = wait_for(&session, b"TUI-OK-");
        assert!(
            contains(&first, b"TUI-OK-") && contains(&first, b"-A"),
            "first paint must report size tag A: {:?}",
            String::from_utf8_lossy(&first)
        );

        session.resize(size(100, 28)).unwrap();
        std::thread::sleep(Duration::from_millis(300));
        session
            .write(
                b"$e=[char]27; Write-Host -NoNewline ($e+'[2J'+$e+'[H'+'TUI-TOP-B'); Write-Host -NoNewline ($e+'[28;1H'+'TUI-BOT-B'); $s=$Host.UI.RawUI.WindowSize; Write-Output ('TUI-OK-'+$s.Width+'x'+$s.Height+'-B')\r",
            )
            .unwrap();
        wait_for(&session, b"TUI-TOP-B");
        wait_for(&session, b"TUI-BOT-B");
        let second = wait_for(&session, b"TUI-OK-");
        assert!(
            contains(&second, b"-B"),
            "second paint must report tag B: {:?}",
            String::from_utf8_lossy(&second)
        );

        // 退出备用屏，主缓冲仍可写
        session
            .write(b"$e=[char]27; Write-Host -NoNewline ($e+'[?1049l'); Write-Output TUI-EXIT-OK\r")
            .unwrap();
        wait_for(&session, b"TUI-EXIT-OK");
        session.close().unwrap();
    }

    #[test]
    fn close_terminates_whole_process_tree() {
        let session = open(ShellKind::PowerShell).unwrap();
        let shell_pid = session.process_id().unwrap();
        session
            .write(b"$p = Start-Process ping.exe -ArgumentList '-n','600','127.0.0.1' -PassThru -WindowStyle Hidden; Write-Output (\"CHILD=\" + $p.Id)\r")
            .unwrap();
        let deadline = Instant::now() + WAIT;
        let output = loop {
            let output = collected(&session);
            if pid_digits_present(&output) {
                break output;
            }
            assert!(
                Instant::now() < deadline,
                "child pid missing: {:?}",
                String::from_utf8_lossy(&output)
            );
            std::thread::sleep(Duration::from_millis(50));
        };
        let child_pid = pid_after(&output, "CHILD=");
        assert!(!process_exits_within(child_pid, Duration::from_millis(200)));
        session.close().unwrap();
        assert!(process_exits_within(shell_pid, Duration::from_secs(10)));
        assert!(process_exits_within(child_pid, Duration::from_secs(10)));
    }

    fn start_child(session: &PtySession) -> u32 {
        session
            .write(b"$p = Start-Process ping.exe -ArgumentList '-n','600','127.0.0.1' -PassThru -WindowStyle Hidden; Write-Output (\"CHILD=\" + $p.Id)\r")
            .unwrap();
        let deadline = Instant::now() + WAIT;
        loop {
            let output = collected(session);
            if pid_digits_present(&output) {
                return pid_after(&output, "CHILD=");
            }
            assert!(Instant::now() < deadline, "child pid missing");
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    #[test]
    fn close_after_shell_exit_still_ends_orphaned_children() {
        let session = open(ShellKind::PowerShell).unwrap();
        let child_pid = start_child(&session);
        session.write(b"exit 0\r").unwrap();
        assert_eq!(wait_exit(&session), 0);
        assert!(!process_exits_within(child_pid, Duration::from_millis(200)));
        session.close().unwrap();
        assert!(process_exits_within(child_pid, Duration::from_secs(10)));
    }

    #[test]
    fn dropping_session_ends_process_tree() {
        let session = open(ShellKind::PowerShell).unwrap();
        let shell_pid = session.process_id().unwrap();
        let child_pid = start_child(&session);
        drop(session);
        assert!(process_exits_within(shell_pid, Duration::from_secs(10)));
        assert!(process_exits_within(child_pid, Duration::from_secs(10)));
    }

    fn pid_digits_present(output: &[u8]) -> bool {
        let text = String::from_utf8_lossy(output);
        text.match_indices("CHILD=")
            .any(|(index, _)| text[index + 6..].starts_with(|c: char| c.is_ascii_digit()))
    }

    const BIG_THEN_EXIT: &[u8] =
        b"(for /l %i in (1,1,3000) do @echo LINE%i) & set /a 777000+777 & exit 0\r";

    #[test]
    fn finish_output_collects_tail_after_exit() {
        let session = open(ShellKind::Cmd).unwrap();
        session.write(BIG_THEN_EXIT).unwrap();
        assert_eq!(wait_exit(&session), 0);
        session.finish_output();
        assert!(session.output_settled());
        assert!(contains(&collected(&session), b"777777"));
        session.close().unwrap();
    }

    /// 与 `runtime::pump` 同序：先收尾已退出的会话，已收尾的读完、其余按预算读，最后回收。
    #[test]
    fn pump_order_delivers_tail_output_of_exiting_shell() {
        use crate::management_agent::terminal::{
            manager::OpenRequest,
            transport::{drain_all, next_push, MAX_OUTPUT_CHUNK_BYTES},
        };
        for round in 0..30u32 {
            let session = open(ShellKind::Cmd).unwrap();
            session.write(BIG_THEN_EXIT).unwrap();
            let mut manager = SessionManager::default();
            let id = format!("{:032x}", round + 1);
            manager.register(
                OpenRequest {
                    session_id: id.clone(),
                    shell: ShellKind::Cmd,
                    size: size(120, 30),
                    cwd: None,
                },
                Box::new(session),
            );
            let budget = MAX_OUTPUT_CHUNK_BYTES * 8;
            let mut cursor = 0;
            let mut delivered = Vec::new();
            let deadline = Instant::now() + WAIT;
            loop {
                let settled = manager.settle_exited();
                let batch = if settled.contains(&id) {
                    drain_all(cursor, |from| manager.read(&id, from, budget))
                } else {
                    next_push(cursor, |from| manager.read(&id, from, budget))
                }
                .unwrap();
                for chunk in &batch.chunks {
                    delivered.extend_from_slice(&chunk.data);
                }
                cursor = batch.cursor;
                if !manager.drain_exited().is_empty() {
                    break;
                }
                assert!(Instant::now() < deadline, "shell did not exit");
                std::thread::sleep(Duration::from_millis(50));
            }
            assert!(
                contains(&delivered, b"777777"),
                "round {round}: tail output lost ({} bytes delivered)",
                delivered.len()
            );
        }
    }

    #[test]
    fn shell_exit_code_is_reported() {
        let session = open(ShellKind::PowerShell).unwrap();
        session.write(b"exit 3\r").unwrap();
        assert_eq!(wait_exit(&session), 3);
        assert_eq!(session.close().unwrap(), Some(3));
    }
}
