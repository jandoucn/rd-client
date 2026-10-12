use std::{net::{SocketAddr, TcpStream, ToSocketAddrs}, sync::{atomic::{AtomicBool, Ordering}, mpsc}, time::{Duration, Instant}};

pub fn fixed(timeout: Duration) -> Result<serde_json::Value, String> {
    let started = Instant::now();
    let budget = timeout.min(Duration::from_secs(30));
    let api = http("https://rdapi.yan.life", budget.min(Duration::from_secs(10)));
    let download = http("https://download.yan.life", budget.saturating_sub(started.elapsed()).min(Duration::from_secs(10)));
    let id = tcp("rd.yan.life", 21116, budget.saturating_sub(started.elapsed()).min(Duration::from_secs(10)));
    Ok(serde_json::json!({"api":api,"download":download,"id":id}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{io::{Read, Write}, net::TcpListener, thread, time::Instant};

    #[test]
    fn http_probe_distinguishes_status_and_does_not_follow_redirects() {
        let trap = TcpListener::bind("127.0.0.1:0").unwrap();
        trap.set_nonblocking(true).unwrap();
        for status in [200, 302, 403, 503] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let address = listener.local_addr().unwrap();
            let location = trap.local_addr().unwrap();
            let server = thread::spawn(move || {
                let (mut socket, _) = listener.accept().unwrap();
                socket.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
                let mut request = [0; 4096];
                let count = socket.read(&mut request).unwrap();
                assert!(request[..count].starts_with(b"HEAD /probe "));
                write!(socket, "HTTP/1.1 {status} Fixture\r\nLocation: http://{location}/secret\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
            });
            let result = http(&format!("http://{address}/probe"), Duration::from_secs(2));
            server.join().unwrap();
            assert_eq!(result["reachable"], true);
            assert_eq!(result["status"], status);
            assert_eq!(result["ok"], status == 200);
            assert!(!result.to_string().contains("secret"));
            assert_eq!(trap.accept().unwrap_err().kind(), std::io::ErrorKind::WouldBlock);
        }
    }

    #[test]
    fn probe_timeouts_fail_without_leaking_urls_or_blocking_other_results() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/secret?token=private", listener.local_addr().unwrap());
        let started = Instant::now();
        let result = http(&url, Duration::from_millis(100));
        assert_eq!(result["ok"], false);
        assert_eq!(result["error_code"], "timeout");
        assert!(!result.to_string().contains("private"));
        assert!(started.elapsed() < Duration::from_secs(2));
        let result = fixed(Duration::ZERO).unwrap();
        for key in ["api", "download", "id"] {
            assert_eq!(result[key]["error_code"], "timeout");
        }
    }

    #[test]
    fn tcp_probe_checks_only_connectivity_without_claiming_registration() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let result = tcp("127.0.0.1", address.port(), Duration::from_secs(1));
        assert_eq!(result["ok"], true);
        assert_eq!(result["protocol"], "tcp");
        assert_eq!(result["registration_verified"], false);
        let (socket, _) = listener.accept().unwrap();
        drop(socket);
        drop(listener);
        let started = Instant::now();
        let result = tcp("127.0.0.1", address.port(), Duration::from_secs(1));
        assert_eq!(result["ok"], false);
        assert_eq!(result["reachable"], false);
        assert!(matches!(result["error_code"].as_str(), Some("connect_failed" | "timeout")));
        assert!(started.elapsed() < Duration::from_secs(2));
    }
}

fn http(url: &str, timeout: Duration) -> serde_json::Value {
    if timeout.is_zero() { return failed("https", "timeout"); }
    let client = match reqwest::blocking::Client::builder()
        .connect_timeout(timeout).timeout(timeout)
        .redirect(reqwest::redirect::Policy::none()).build() {
        Ok(client) => client,
        Err(_) => return failed("https", "client_unavailable"),
    };
    match client.head(url).send() {
        Ok(response) => serde_json::json!({"ok":response.status().is_success(),"reachable":true,"status":response.status().as_u16(),"protocol":"https"}),
        Err(error) => failed("https", if error.is_timeout() { "timeout" } else { "request_failed" }),
    }
}

fn failed(protocol: &str, code: &str) -> serde_json::Value {
    serde_json::json!({"ok":false,"reachable":false,"protocol":protocol,"error_code":code})
}

fn tcp(host: &str, port: u16, timeout: Duration) -> serde_json::Value {
    if timeout.is_zero() { return failed("tcp", "timeout"); }
    let started = Instant::now();
    // The system resolver cannot be cancelled. Permit only one outstanding lookup
    // so successive timed-out commands cannot accumulate resolver threads.
    static RESOLVING: AtomicBool = AtomicBool::new(false);
    if RESOLVING.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire).is_err() {
        return failed("tcp", "resolver_busy");
    }
    let (tx, rx) = mpsc::sync_channel(1);
    let host = host.to_owned();
    if std::thread::Builder::new().name("agent-fixed-dns".into()).spawn(move || {
        let addresses = (host.as_str(), port).to_socket_addrs()
            .map(|values| values.take(8).collect::<Vec<SocketAddr>>());
        RESOLVING.store(false, Ordering::Release);
        // Receiver goes away on timeout; no external effect needs retrying.
        let _ = tx.send(addresses);
    }).is_err() {
        RESOLVING.store(false, Ordering::Release);
        return failed("tcp", "resolver_unavailable");
    }
    let addresses = match rx.recv_timeout(timeout) {
        Ok(Ok(addresses)) => addresses,
        Ok(Err(_)) => return failed("tcp", "dns_failed"),
        Err(mpsc::RecvTimeoutError::Timeout) => return failed("tcp", "timeout"),
        Err(_) => return failed("tcp", "resolver_unavailable"),
    };
    for address in addresses {
        let remaining = timeout.saturating_sub(started.elapsed());
        if remaining.is_zero() { return failed("tcp", "timeout"); }
        if TcpStream::connect_timeout(&address, remaining.min(Duration::from_secs(2))).is_ok() {
            return serde_json::json!({"ok":true,"reachable":true,"protocol":"tcp","port":port,"registration_verified":false});
        }
    }
    failed("tcp", if started.elapsed() >= timeout { "timeout" } else { "connect_failed" })
}
