//! 通过会话管理打开管道终端：输入经会话写线程进 shell，输出经读线程回到会话，
//! 关闭服务时不能卡在 join 读线程上。

use super::*;

fn open_request(terminal_id: i32) -> OpenTerminal {
    let mut open = OpenTerminal::new();
    open.terminal_id = terminal_id;
    open.rows = 24;
    open.cols = 80;
    open
}

fn wait_output(session: &Arc<Mutex<TerminalSession>>, needle: &str, count: usize) -> String {
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut text = Vec::new();
    while Instant::now() < deadline {
        if String::from_utf8_lossy(&text).matches(needle).count() >= count {
            return String::from_utf8_lossy(&text).into_owned();
        }
        let chunk = {
            let session = session.lock().unwrap();
            session
                .output_rx
                .as_ref()
                .and_then(|rx| rx.recv_timeout(Duration::from_millis(100)).ok())
        };
        if let Some(chunk) = chunk {
            text.extend_from_slice(&chunk);
        }
    }
    panic!(
        "timed out waiting for {needle:?}; output so far: {:?}",
        String::from_utf8_lossy(&text)
    );
}

#[test]
fn pipe_session_runs_commands_and_closes_without_hanging() {
    let service_id = format!("pipe-test-{}", std::process::id());
    ensure_service(&service_id, false).expect("create service");
    let service = get_service(&service_id).expect("service registered");
    let proxy = TerminalServiceProxy::new(service_id.clone(), Some(false), None);

    let response = {
        let mut service = service.lock().unwrap();
        proxy
            .handle_open_with_pipe(&mut service, &open_request(7))
            .expect("open terminal on pipes")
            .expect("open response")
    };
    let opened = response.opened();
    assert!(opened.success, "{}", opened.message);
    assert_eq!(opened.terminal_id, 7);
    assert!(opened.pid > 0);

    let session = service
        .lock()
        .unwrap()
        .sessions
        .get(&7)
        .cloned()
        .expect("session stored");
    assert!(session.lock().unwrap().pty_pair.is_none());

    let send = |data: &[u8]| {
        let session = session.lock().unwrap();
        session
            .input_tx
            .as_ref()
            .expect("input channel")
            .send(data.to_vec())
            .expect("send input");
    };
    send(b"echo svc-pipe-ok\r");
    // PowerShell 自己回显一次，执行结果再出现一次；cmd 由本端回显，同样是两次。
    wait_output(&session, "svc-pipe-ok", 2);

    // 关闭服务：写端释放即结束 shell，读线程拿到 EOF 后才能被 join。
    let (done_tx, done_rx) = mpsc::channel();
    let closing = service_id.clone();
    thread::spawn(move || {
        drop_service(&closing);
        let _ = done_tx.send(());
    });
    assert!(
        done_rx.recv_timeout(Duration::from_secs(20)).is_ok(),
        "closing the pipe session must not hang"
    );
    assert!(
        process_gone(opened.pid),
        "shell must have exited after the session closed"
    );
}

/// shell 在跑不读 stdin 的命令时继续灌输入：写线程卡在写管道上。关闭会话仍须及时返回。
#[test]
fn closing_while_writer_is_blocked_does_not_hang() {
    let service_id = format!("pipe-blocked-{}", std::process::id());
    ensure_service(&service_id, false).expect("create service");
    let service = get_service(&service_id).expect("service registered");
    let proxy = TerminalServiceProxy::new(service_id.clone(), Some(false), None);
    let pid = {
        let mut service = service.lock().unwrap();
        let response = proxy
            .handle_open_with_pipe(&mut service, &open_request(3))
            .expect("open terminal on pipes")
            .expect("open response");
        response.opened().pid
    };
    let session = service
        .lock()
        .unwrap()
        .sessions
        .get(&3)
        .cloned()
        .expect("session");
    let input = session
        .lock()
        .unwrap()
        .input_tx
        .clone()
        .expect("input channel");
    // PowerShell 与 cmd 都认得的长时间命令：ping 不读 stdin。
    input.send(b"ping -n 600 127.0.0.1\r".to_vec()).unwrap();
    wait_output(&session, "127.0.0.1", 2);
    // 远超匿名管道缓冲（约 4 KB）的输入，写线程必然卡住。
    for _ in 0..64 {
        input.send(vec![b'x'; 512]).unwrap();
    }
    drop(input);
    thread::sleep(Duration::from_millis(500));

    let (done_tx, done_rx) = mpsc::channel();
    let closing = service_id.clone();
    thread::spawn(move || {
        drop_service(&closing);
        let _ = done_tx.send(());
    });
    assert!(
        done_rx.recv_timeout(Duration::from_secs(20)).is_ok(),
        "closing must not hang on a blocked pipe writer"
    );
    assert!(process_gone(pid), "shell must be gone after closing");
}

/// 进程已结束（或已无法打开）返回 true。
fn process_gone(pid: u32) -> bool {
    use windows::Win32::{
        Foundation::{CloseHandle, STILL_ACTIVE},
        System::Threading::{GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION},
    };
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        let Ok(handle) = (unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) })
        else {
            return true;
        };
        let mut code = 0u32;
        let queried = unsafe { GetExitCodeProcess(handle, &mut code) }.is_ok();
        let _ = unsafe { CloseHandle(handle) };
        if !queried || code != STILL_ACTIVE.0 as u32 {
            return true;
        }
        thread::sleep(Duration::from_millis(100));
    }
    false
}
