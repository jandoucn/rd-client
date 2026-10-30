//! helper 进程在没有 ConPTY 的系统上改用匿名管道承载 shell。
//!
//! 与服务之间的消息协议不变：数据消息写进 shell，尺寸消息忽略（管道没有窗口尺寸）。
//! 服务端输入管道关闭即结束 shell，shell 退出后 helper 随之退出。

use crate::server::{
    terminal_helper::{
        get_default_shell, read_exact_or_eof, MAX_PAYLOAD_SIZE, MSG_HEADER_SIZE, MSG_TYPE_DATA,
        MSG_TYPE_RESIZE,
    },
    terminal_pipe_win::{shell_environment, PipeReader, PipeShell, PipeWriter},
};
use hbb_common::{anyhow::Result, log};
use portable_pty::{Child, ChildKiller, CommandBuilder};
use std::{
    io::{Read, Write},
    thread,
};

pub(super) fn run<I, O>(input: I, output: O) -> Result<()>
where
    I: Read + Send + 'static,
    O: Write + Send + 'static,
{
    let shell = get_default_shell();
    log::debug!("Using shell on pipes (no ConPTY): {}", shell);
    let PipeShell {
        reader,
        writer,
        mut child,
    } = PipeShell::spawn(
        &shell,
        Some(&shell_environment(&CommandBuilder::new(&shell))),
    )?;
    // 输入线程可能一直阻塞在服务端的输入管道上，不 join；helper 退出时随进程结束。
    thread::spawn(move || forward_input(input, writer));
    let output_thread = thread::spawn(move || forward_output(reader, output));
    let exit_status = child.wait();
    log::info!("Shell exited: {:?}", exit_status);
    // shell 退出后它留下的后台子进程仍可能占着 stdout，结束整棵树输出才会 EOF。
    let _ = child.kill();
    let _ = output_thread.join();
    log::info!("Terminal helper exiting");
    Ok(())
}

/// 解析服务发来的消息。返回即释放写端，从而结束 shell。
fn forward_input<I: Read>(mut input: I, mut writer: PipeWriter) {
    let mut header = [0u8; MSG_HEADER_SIZE];
    let mut payload = vec![0u8; 4096];
    loop {
        match read_exact_or_eof(&mut input, &mut header) {
            Ok(true) => {}
            Ok(false) => break,
            Err(e) => {
                log::error!("Input pipe header read error: {}", e);
                break;
            }
        }
        let len = u32::from_le_bytes([header[1], header[2], header[3], header[4]]) as usize;
        if len > MAX_PAYLOAD_SIZE {
            log::error!(
                "Payload too large: {} bytes (max {})",
                len,
                MAX_PAYLOAD_SIZE
            );
            break;
        }
        if payload.len() < len {
            payload.resize(len, 0);
        }
        if len > 0 {
            match read_exact_or_eof(&mut input, &mut payload[..len]) {
                Ok(true) => {}
                Ok(false) => break,
                Err(e) => {
                    log::error!("Input pipe payload read error: {}", e);
                    break;
                }
            }
        }
        match header[0] {
            MSG_TYPE_DATA => {
                if let Err(e) = writer
                    .write_all(&payload[..len])
                    .and_then(|_| writer.flush())
                {
                    log::error!("Pipe shell write error: {}", e);
                    break;
                }
            }
            MSG_TYPE_RESIZE => {}
            other => {
                log::error!("Unknown message type: {}, terminating", other);
                break;
            }
        }
    }
    log::debug!("Input thread exiting");
}

fn forward_output<O: Write>(mut reader: PipeReader, mut output: O) {
    let mut buf = vec![0u8; 4096];
    loop {
        match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                if let Err(e) = output.write_all(&buf[..n]).and_then(|_| output.flush()) {
                    log::error!("Output pipe write error: {}", e);
                    break;
                }
            }
            Err(e) => {
                log::error!("Pipe shell read error: {}", e);
                break;
            }
        }
    }
    log::debug!("Output thread exiting");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::terminal_helper::{encode_helper_message, encode_resize_message};
    use std::{
        sync::mpsc,
        time::{Duration, Instant},
    };

    fn start_helper() -> (
        std::io::PipeWriter,
        mpsc::Receiver<Vec<u8>>,
        mpsc::Receiver<bool>,
    ) {
        let (input_read, input_write) = std::io::pipe().expect("input pipe");
        let (mut output_read, output_write) = std::io::pipe().expect("output pipe");
        let (done_tx, done_rx) = mpsc::channel();
        thread::spawn(move || {
            let _ = done_tx.send(run(input_read, output_write).is_ok());
        });
        let (out_tx, out_rx) = mpsc::channel();
        thread::spawn(move || {
            let mut buf = [0u8; 4096];
            while let Ok(n) = output_read.read(&mut buf) {
                if n == 0 || out_tx.send(buf[..n].to_vec()).is_err() {
                    break;
                }
            }
        });
        (input_write, out_rx, done_rx)
    }

    fn wait_text(out_rx: &mpsc::Receiver<Vec<u8>>, needle: &str, count: usize) {
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut text = Vec::new();
        while String::from_utf8_lossy(&text).matches(needle).count() < count {
            assert!(
                Instant::now() < deadline,
                "output so far: {:?}",
                String::from_utf8_lossy(&text)
            );
            if let Ok(chunk) = out_rx.recv_timeout(Duration::from_millis(100)) {
                text.extend_from_slice(&chunk);
            }
        }
    }

    /// shell 自己 exit，但留下的后台进程还占着 stdout：helper 仍须退出，后台进程一并结束。
    #[test]
    fn helper_exits_when_shell_exits_with_background_child() {
        let (mut input_write, out_rx, done_rx) = start_helper();
        input_write
            .write_all(&encode_helper_message(
                MSG_TYPE_DATA,
                b"Start-Process ping -ArgumentList '-n','600','127.0.0.1' -NoNewWindow\r",
            ))
            .unwrap();
        wait_text(&out_rx, "127.0.0.1", 1);
        input_write
            .write_all(&encode_helper_message(MSG_TYPE_DATA, b"exit\r"))
            .unwrap();
        assert_eq!(
            done_rx.recv_timeout(Duration::from_secs(20)),
            Ok(true),
            "helper must exit after the shell exits"
        );
    }

    /// 服务发数据消息 → shell 执行；尺寸消息被忽略；服务关闭输入管道 → shell 结束、helper 返回。
    #[test]
    fn helper_runs_shell_on_pipes_and_exits_when_input_closes() {
        let (mut input_write, out_rx, done_rx) = start_helper();
        input_write
            .write_all(&encode_resize_message(30, 100))
            .unwrap();
        input_write
            .write_all(&encode_helper_message(
                MSG_TYPE_DATA,
                b"echo helper-pipe-ok\r",
            ))
            .unwrap();
        wait_text(&out_rx, "helper-pipe-ok", 2);
        drop(input_write);
        assert_eq!(
            done_rx.recv_timeout(Duration::from_secs(20)),
            Ok(true),
            "helper must exit after the service closes its input pipe"
        );
    }
}
