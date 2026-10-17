#![cfg(windows)]

include!("management_agent_rescue_projection/modules.rs");
#[path = "../src/management_agent/first_install_approval_delivery.rs"]
mod first_install_approval_delivery;

use std::{
    io::{Read, Write},
    path::Path,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

#[test]
#[ignore = "Win10 only; real worker/HTTP/producer/reader, isolated registration and failed health"]
fn t01_real_worker_http_producer_reader_preserves_health_failure_and_waiting() {
    assert_eq!(std::env::var("COMPUTERNAME").as_deref(), Ok("RDLAB-WIN10"));
    assert_eq!(
        std::env::var("RUSTDESK_T01_PRODUCER").as_deref(),
        Ok("red-formal-sos-1501")
    );
    let input = Path::new(r"C:\RustDeskAgentLab\t01-delivery-input");
    let reply: serde_json::Value =
        serde_json::from_slice(&std::fs::read(input.join("resolved.json")).unwrap()).unwrap();
    let key = "windows-x86_64-msi-sos";
    let build = 2026101501;
    let manifest =
        package_gate::bind_resolved(&reply, reply["package_id"].as_str().unwrap(), key, build)
            .unwrap();
    let target = package_gate::verify(&manifest, key, build).unwrap();
    let source = input.join(package_gate::cache_name(&target.primary, "msi").unwrap());
    let _input_lock = package_gate::lock_verified_file(&source, target).unwrap();
    let product = msi_metadata::inspect(&source).unwrap().product_code;
    for approved in [true, false] {
        let root = Path::new(r"C:\RustDeskAgentLab")
            .join(format!("t01-worker-real-{}", uuid::Uuid::new_v4().simple()));
        workspace::create(&root).unwrap();
        for name in ["state", "current", "release-cache", "first-install-status"] {
            workspace::create(&root.join(name)).unwrap();
        }
        let installation = "cccccccccccccccccccccccccccccccc";
        std::fs::write(root.join("state/agent.json"), serde_json::to_vec(&serde_json::json!({
            "agent_installation_id":installation,"target_key":key,"management_agent_enabled":true,"disabled":false
        })).unwrap()).unwrap();
        std::fs::write(root.join("state/service-recovery-enabled"), b"enabled").unwrap();
        hbb_common::sodiumoxide::init().unwrap();
        let (pk, sk) = hbb_common::sodiumoxide::crypto::sign::gen_keypair();
        std::fs::write(
            root.join("current/identity.toml"),
            format!(
                "id='fixture'\nkey_pair=[{:?},{:?}]\n",
                sk.0.to_vec(),
                pk.0.to_vec()
            ),
        )
        .unwrap();
        std::fs::write(
            root.join("current/RustDesk2.toml"),
            b"[options]\nstop-service='N'\n",
        )
        .unwrap();
        let before = [
            "state/agent.json",
            "current/identity.toml",
            "current/RustDesk2.toml",
        ]
        .map(|p| std::fs::read(root.join(p)).unwrap());
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let endpoint = listener.local_addr().unwrap();
        let body = serde_json::to_vec(&reply["manifest"]).unwrap();
        let server = std::thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(10);
            let mut stream = loop {
                match listener.accept() {
                    Ok((s, _)) => break s,
                    Err(e)
                        if e.kind() == std::io::ErrorKind::WouldBlock
                            && Instant::now() < deadline =>
                    {
                        std::thread::sleep(Duration::from_millis(10))
                    }
                    Err(e) => panic!("loopback accept failed: {e}"),
                }
            };
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            stream
                .set_write_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut request = Vec::new();
            while !request.ends_with(b"\r\n\r\n") && request.len() < 8192 {
                let mut byte = [0];
                assert_eq!(stream.read(&mut byte).unwrap(), 1);
                request.push(byte[0]);
            }
            assert!(request.starts_with(b"GET /rd/update/v1/manifest/stable.json HTTP/1.1\r\n"));
            if approved {
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                )
                .unwrap();
                stream.write_all(&body).unwrap();
            } else {
                stream
                    .write_all(
                        b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                    )
                    .unwrap();
            }
        });
        let health = Arc::new(AtomicUsize::new(0));
        let outcomes = Arc::new(Mutex::new(Vec::new()));
        let delivery_source = source.clone();
        let delivery_product = product.clone();
        let worker = rescue_assets::first_install::start_scoped_fixture(
            rescue_assets::first_install::WorkerScope {
                root: root.clone(),
                source: source.clone(),
                product_code: product.clone(),
                health_calls: health.clone(),
                coordination_results: outcomes.clone(),
                coordinate: Box::new(move |root| {
                    use first_install_approval_delivery::*;
                    coordinate_approval(
                        CurrentInstallation {
                            installation_id: installation,
                            target_key: key,
                            current_build: build,
                            product_code: &delivery_product,
                        },
                        || fetch_loopback_fixture(endpoint, Duration::from_secs(2)),
                        |request| {
                            deliver_at(
                                request,
                                TestLayout {
                                    agent: root,
                                    identity: root.join("current/identity.toml"),
                                    config: root.join("current/RustDesk2.toml"),
                                    registered_source: &delivery_source,
                                },
                            )
                        },
                    )
                    .map(|outcome| {
                        assert_eq!(
                            outcome,
                            if approved {
                                CoordinationOutcome::Delivered
                            } else {
                                CoordinationOutcome::WaitingSource
                            }
                        );
                    })
                    .map_err(|e| format!("{e:?}"))
                }),
            },
        )
        .unwrap();
        worker.notify().unwrap();
        let report_path = root.join("first-install-status/last-check.json");
        let deadline = Instant::now() + Duration::from_secs(15);
        while !report_path.exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        drop(worker);
        server.join().unwrap();
        assert!(report_path.exists(), "worker report must arrive");
        let report: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&report_path).unwrap()).unwrap();
        assert!(
            [
                "state/agent.json",
                "current/identity.toml",
                "current/RustDesk2.toml"
            ]
            .map(|p| std::fs::read(root.join(p)).unwrap())
                == before,
            "identity/config/state preserved"
        );
        assert_eq!(*outcomes.lock().unwrap(), vec![Ok(())]);
        assert_eq!(report["ready"], false);
        assert!(!root.join("rescue/current").exists());
        if approved {
            assert!(root.join("release-cache/first-install.json").exists());
            assert_eq!(
                health.load(Ordering::SeqCst),
                1,
                "real reader must reach health after signature/package checks"
            );
            assert_eq!(report["error_code"], "preparation_refused");
            assert_eq!(report["error"], "isolated worker health unavailable");
        } else {
            assert!(!root.join("release-cache/first-install.json").exists());
            assert_eq!(health.load(Ordering::SeqCst), 0);
            assert_eq!(report["error_code"], "waiting_approval");
        }
        println!(
            "T01_REAL_WORKER approved={approved} root={} report={report}",
            root.display()
        );
    }
}
